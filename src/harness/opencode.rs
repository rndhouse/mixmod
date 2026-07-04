use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde_json::{Value, json};

use crate::harness::{AgentBackend, AgentHarness, AgentOutput, AgentRequest};
use crate::{
    DEFAULT_OPENCODE_OLLAMA_MODEL, LIVE_STATUS_FILE, LOCAL_VERIFICATION_JSON, MixmodConfig,
    OpenCodeConfig, SUPERVISOR_CONTROL_FILE, SUPERVISOR_CONTROL_LOG, SupervisorControlEvent,
    append_file, append_jsonl, atomic_write, env_u64, get_str, get_string_array,
    normalize_worker_mode, shell_command, state_layout, write_pretty_json,
};

pub struct ShellOpenCodeRunner {
    config: MixmodConfig,
}

impl ShellOpenCodeRunner {
    pub fn new(config: MixmodConfig) -> Self {
        Self { config }
    }
}

impl AgentHarness for ShellOpenCodeRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        let command = env::var("MIXMOD_OPENCODE_COMMAND")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.config.opencode.command.clone());
        let selection = resolve_opencode_model(
            &command,
            &request.root,
            &self.config.opencode,
            request.require_local,
        )?;
        let args = env::var("MIXMOD_OPENCODE_ARGS")
            .ok()
            .map(|value| {
                value
                    .split_whitespace()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|args| !args.is_empty())
            .unwrap_or_else(|| self.config.opencode.args.clone());

        let rendered_args = args
            .iter()
            .map(|arg| render_opencode_arg(arg, request, &selection))
            .collect::<Vec<_>>();
        let rendered_args =
            prepare_opencode_args(rendered_args, request.resume_session_id.as_deref());

        let mut command_for_metrics = Vec::with_capacity(rendered_args.len() + 1);
        command_for_metrics.push(command.clone());
        let metric_args = args
            .iter()
            .map(|arg| redact_opencode_arg(arg, request, &selection))
            .collect::<Vec<_>>();
        command_for_metrics.extend(prepare_opencode_args(
            metric_args,
            request.resume_session_id.as_deref(),
        ));

        tracing::info!(
            command = %command,
            mode = %request.mode,
            out_dir = %request.out_dir.display(),
                "running OpenCode command"
        );
        let verification = run_with_local_verification(
            &command,
            &rendered_args,
            request,
            &self.config.opencode,
            &selection,
        );

        match verification {
            Ok(mut verification) => {
                let success = verification
                    .exit_status
                    .map(|code| code == 0)
                    .unwrap_or(false);
                let mut actual_session_id = request.resume_session_id.clone();
                if actual_session_id.is_none() {
                    match resolve_opencode_session_id(&command, &request.root, &request.session_id)
                    {
                        Ok(Some(session_id)) => actual_session_id = Some(session_id),
                        Ok(None) => verification.verification_notes.push(format!(
                            "OpenCode session id was not found for title `{}` and directory `{}`.",
                            request.session_id,
                            request.root.display()
                        )),
                        Err(error) => verification
                            .verification_notes
                            .push(format!("OpenCode session id resolution failed: {error}")),
                    }
                }
                Ok(AgentOutput {
                    backend: AgentBackend::OpenCode,
                    command_for_metrics,
                    segments: verification.opencode_segments,
                    exit_status: verification.exit_status,
                    success,
                    stdout: verification.stdout,
                    stderr: verification.stderr,
                    provider: Some(selection.provider),
                    model: Some(selection.model),
                    model_arg: Some(selection.model_arg),
                    session_label: Some(request.session_id.clone()),
                    session_id: actual_session_id,
                    resume_session_id: request.resume_session_id.clone(),
                    session_reused: request.resume_session_id.is_some(),
                    interrupted_by_supervisor: verification.interrupted_by_supervisor,
                    supervisor_control_action: verification.supervisor_control_action,
                    supervisor_control_events: verification.supervisor_control_events,
                    timed_out: verification.timed_out,
                    idle_timed_out: verification.idle_timed_out,
                    heartbeat_count: verification.heartbeat_count,
                    require_local: selection.require_local,
                    local_inference_verified: verification.local_inference_verified,
                    gpu_activity_observed: verification.gpu_activity_observed,
                    backend_activity_observed: verification.backend_activity_observed,
                    verification_notes: verification.verification_notes,
                })
            }
            Err(error) => Ok(AgentOutput {
                backend: AgentBackend::OpenCode,
                command_for_metrics,
                segments: Vec::new(),
                exit_status: None,
                success: false,
                stdout: Vec::new(),
                stderr: format!("failed to run OpenCode command `{command}`: {error}\n")
                    .into_bytes(),
                provider: Some(selection.provider),
                model: Some(selection.model),
                model_arg: Some(selection.model_arg),
                session_label: Some(request.session_id.clone()),
                session_id: None,
                resume_session_id: request.resume_session_id.clone(),
                session_reused: request.resume_session_id.is_some(),
                interrupted_by_supervisor: false,
                supervisor_control_action: None,
                supervisor_control_events: Vec::new(),
                timed_out: false,
                idle_timed_out: false,
                heartbeat_count: 0,
                require_local: selection.require_local,
                local_inference_verified: false,
                gpu_activity_observed: false,
                backend_activity_observed: false,
                verification_notes: vec![format!(
                    "OpenCode execution failed before verification: {error}"
                )],
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeModelSelection {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) model_arg: String,
    pub(crate) require_local: bool,
}

#[derive(Debug)]
struct NextOpenCodeSegment {
    args: Vec<String>,
    label: String,
    resume_session_id: Option<String>,
    action: String,
    worker_mode: String,
    message: String,
}

#[derive(Debug)]
pub(crate) struct VerifiedCommandOutput {
    pub(crate) exit_status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) opencode_segments: Vec<Value>,
    pub(crate) timed_out: bool,
    pub(crate) idle_timed_out: bool,
    pub(crate) interrupted_by_supervisor: bool,
    pub(crate) supervisor_control_action: Option<String>,
    pub(crate) supervisor_control_events: Vec<SupervisorControlEvent>,
    pub(crate) heartbeat_count: u64,
    pub(crate) local_inference_verified: bool,
    pub(crate) gpu_activity_observed: bool,
    pub(crate) backend_activity_observed: bool,
    pub(crate) verification_notes: Vec<String>,
}

fn resolve_opencode_model(
    command: &str,
    root: &Path,
    config: &OpenCodeConfig,
    require_local_override: bool,
) -> Result<OpenCodeModelSelection> {
    let require_local = require_local_override || config.require_local;
    let models = opencode_command(command, root)
        .arg("models")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run `{command} models`"))?;
    if !models.status.success() {
        bail!(
            "`{command} models` failed: {}",
            String::from_utf8_lossy(&models.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&models.stdout);
    let aliases = model_aliases(config);
    let configured_model = config.model.as_str();
    let selected = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find(|line| model_line_matches(line, &config.provider, configured_model, &aliases))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "configured OpenCode model `{}` with aliases {:?} was not found in `{command} models`; update the Mixmod worker model or generated OpenCode config for this project",
                configured_model,
                aliases
            )
        })?;
    let (provider, model) = selected
        .split_once('/')
        .map(|(provider, model)| (provider.to_string(), model.to_string()))
        .unwrap_or_else(|| (config.provider.clone(), selected.clone()));

    if require_local {
        reject_cloud_provider(&provider)?;
        if !is_allowed_local_provider(&provider, config) {
            bail!(
                "OpenCode provider `{provider}` is not configured as local under --require-local"
            );
        }
        if !aliases
            .iter()
            .any(|alias| selected.contains(alias) || model == *alias)
        {
            bail!("selected model `{selected}` does not match configured local Qwen 3.6 aliases");
        }
    }

    Ok(OpenCodeModelSelection {
        provider,
        model,
        model_arg: selected,
        require_local,
    })
}

fn model_aliases(config: &OpenCodeConfig) -> Vec<String> {
    let mut aliases = vec![config.model.clone()];
    if let Some(extra) = config.model_aliases.get(&config.model) {
        aliases.extend(extra.clone());
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn model_line_matches(
    line: &str,
    provider: &str,
    configured_model: &str,
    aliases: &[String],
) -> bool {
    if line == configured_model || line.ends_with(&format!("/{configured_model}")) {
        return true;
    }
    if provider != "local" && !line.starts_with(&format!("{provider}/")) {
        return false;
    }
    aliases
        .iter()
        .any(|alias| line == alias || line.ends_with(&format!("/{alias}")) || line.contains(alias))
}

fn reject_cloud_provider(provider: &str) -> Result<()> {
    let cloud = [
        "openai",
        "anthropic",
        "gemini",
        "openrouter",
        "xai",
        "groq",
        "copilot",
        "opencode-hosted",
        "azure",
        "bedrock",
    ];
    if cloud.iter().any(|item| provider.contains(item)) {
        bail!("cloud OpenCode provider `{provider}` is rejected under --require-local");
    }
    Ok(())
}

fn is_allowed_local_provider(provider: &str, config: &OpenCodeConfig) -> bool {
    config.local_providers.iter().any(|allowed| {
        provider == allowed || provider.contains(allowed) || allowed.contains(provider)
    })
}

pub(crate) fn run_with_local_verification(
    command: &str,
    args: &[String],
    request: &AgentRequest,
    opencode_config: &OpenCodeConfig,
    selection: &OpenCodeModelSelection,
) -> Result<VerifiedCommandOutput> {
    LocalVerificationRun {
        command,
        args,
        request,
        opencode_config,
        selection,
    }
    .execute()
}

struct LocalVerificationRun<'a> {
    command: &'a str,
    args: &'a [String],
    request: &'a AgentRequest,
    opencode_config: &'a OpenCodeConfig,
    selection: &'a OpenCodeModelSelection,
}

impl LocalVerificationRun<'_> {
    fn execute(self) -> Result<VerifiedCommandOutput> {
        let Self {
            command,
            args,
            request,
            opencode_config,
            selection,
        } = self;
        let root = &request.root;
        let out_dir = &request.out_dir;
        let verification_config = &opencode_config.local_verification;
        let logs_dir = out_dir.join("logs");
        fs::create_dir_all(&logs_dir).with_context(|| {
            format!("failed to create OpenCode logs dir {}", logs_dir.display())
        })?;
        let mut notes = Vec::new();
        let stdout_path = logs_dir.join("opencode.stdout.txt");
        let stderr_path = logs_dir.join("opencode.stderr.txt");
        let heartbeat_path = logs_dir.join("heartbeat.jsonl");
        let live_status_path = out_dir.join(LIVE_STATUS_FILE);
        let supervisor_control_path = out_dir.join(SUPERVISOR_CONTROL_FILE);
        let supervisor_control_log_path = out_dir.join(SUPERVISOR_CONTROL_LOG);
        atomic_write(&stdout_path, b"")?;
        atomic_write(&stderr_path, b"")?;
        atomic_write(&heartbeat_path, b"")?;
        atomic_write(&supervisor_control_log_path, b"")?;
        let _ = fs::remove_file(&supervisor_control_path);

        let before_gpu = if verification_config.enabled {
            run_optional_logged_command(
                &verification_config.gpu_command,
                root,
                &logs_dir.join("nvidia-smi-before.txt"),
            )
        } else {
            None
        };

        let stdout_bytes = Arc::new(AtomicU64::new(0));
        let stderr_bytes = Arc::new(AtomicU64::new(0));
        let last_output_at = Arc::new(AtomicU64::new(now_millis()));

        let start = Instant::now();
        let heartbeat_seconds = env_u64("MIXMOD_OPENCODE_HEARTBEAT_SECONDS")
            .unwrap_or(opencode_config.heartbeat_seconds)
            .max(1);
        let worker_timeout_seconds = env_u64("MIXMOD_OPENCODE_WORKER_TIMEOUT_SECONDS")
            .unwrap_or(opencode_config.worker_timeout_seconds);
        let idle_timeout_seconds = env_u64("MIXMOD_OPENCODE_IDLE_TIMEOUT_SECONDS")
            .unwrap_or(opencode_config.idle_timeout_seconds);
        let heartbeat_interval = Duration::from_secs(heartbeat_seconds);
        let worker_timeout =
            (worker_timeout_seconds > 0).then(|| Duration::from_secs(worker_timeout_seconds));
        let idle_timeout =
            (idle_timeout_seconds > 0).then(|| Duration::from_secs(idle_timeout_seconds));
        let mut last_heartbeat = Instant::now()
            .checked_sub(heartbeat_interval)
            .unwrap_or_else(Instant::now);
        let mut heartbeat_count = 0_u64;
        let mut gpu_activity_seen = false;
        let mut backend_activity_seen = false;
        let mut timed_out = false;
        let mut idle_timed_out = false;
        let mut interrupted_by_supervisor = false;
        let mut supervisor_control_action = None;
        let mut supervisor_control_events = Vec::new();
        let mut opencode_segments = Vec::new();
        let mut segment_args = args.to_vec();
        let mut segment_label = request.session_id.clone();
        let mut segment_resume_session_id = request.resume_session_id.clone();
        let mut segment_action = "initial".to_string();
        let mut segment_worker_mode = if segment_resume_session_id.is_some() {
            "continue".to_string()
        } else {
            "initial".to_string()
        };
        let mut segment_message = String::new();
        let mut active_opencode_session_id = request.resume_session_id.clone();
        let mut segment_index = 0_u64;

        let exit_status = 'segments: loop {
            segment_index += 1;
            let segment_started = Utc::now();
            let segment_stdout_start = stdout_bytes.load(Ordering::Relaxed);
            let segment_stderr_start = stderr_bytes.load(Ordering::Relaxed);
            let mut segment_command = vec![command.to_string()];
            segment_command.extend(
                segment_args
                    .iter()
                    .map(|arg| redact_runtime_opencode_arg(arg, request)),
            );
            append_file(
            &stdout_path,
            format!(
                "\n\n--- opencode segment {segment_index}: action={segment_action} worker_mode={segment_worker_mode} ---\n"
            )
            .as_bytes(),
        )?;
            append_file(
            &stderr_path,
            format!(
                "\n\n--- opencode segment {segment_index}: action={segment_action} worker_mode={segment_worker_mode} ---\n"
            )
            .as_bytes(),
        )?;

            let mut process = spawn_opencode_process(SpawnOpenCodeProcess {
                command,
                args: &segment_args,
                root,
                stdout_path: &stdout_path,
                stderr_path: &stderr_path,
                stdout_bytes: Arc::clone(&stdout_bytes),
                stderr_bytes: Arc::clone(&stderr_bytes),
                last_output_at: Arc::clone(&last_output_at),
            })?;
            let mut next_segment: Option<NextOpenCodeSegment> = None;
            let mut segment_timed_out = false;
            let mut segment_idle_timed_out = false;
            let mut segment_stopped_by_supervisor = false;

            let segment_exit_status = loop {
                if let Some(status) = process.child.try_wait()? {
                    break status.code();
                }

                if let Some(timeout) = worker_timeout
                    && start.elapsed() >= timeout
                {
                    timed_out = true;
                    segment_timed_out = true;
                    notes.push(format!(
                        "OpenCode exceeded worker timeout of {} seconds.",
                        worker_timeout_seconds
                    ));
                    let _ = process.child.kill();
                    break process.child.wait().ok().and_then(|status| status.code());
                }

                let last_output_age =
                    now_millis().saturating_sub(last_output_at.load(Ordering::Relaxed));
                if let Some(timeout) = idle_timeout
                    && last_output_age >= timeout.as_millis() as u64
                {
                    idle_timed_out = true;
                    segment_idle_timed_out = true;
                    notes.push(format!(
                    "OpenCode exceeded idle timeout of {} seconds without stdout/stderr activity.",
                    idle_timeout_seconds
                ));
                    let _ = process.child.kill();
                    break process.child.wait().ok().and_then(|status| status.code());
                }

                if last_heartbeat.elapsed() >= heartbeat_interval {
                    heartbeat_count += 1;
                    let mut gpu_sample_written = false;
                    let mut backend_sample_written = false;
                    if verification_config.enabled {
                        if let Some(text) =
                            run_optional_command_text(&verification_config.gpu_command, root)
                        {
                            let sample = format!("\n--- sample {heartbeat_count} ---\n{text}");
                            append_file(
                                &logs_dir.join("nvidia-smi-during.txt"),
                                sample.as_bytes(),
                            )?;
                            gpu_activity_seen |=
                                gpu_activity_observed(before_gpu.as_deref(), Some(&text));
                            gpu_sample_written = true;
                        }
                        if let Some(text) =
                            run_optional_command_text(&verification_config.backend_command, root)
                        {
                            let sample = format!("\n--- sample {heartbeat_count} ---\n{text}");
                            append_file(&logs_dir.join("ollama-ps.txt"), sample.as_bytes())?;
                            backend_activity_seen |=
                                backend_activity_observed(Some(&text), selection);
                            backend_sample_written = true;
                        }
                    }
                    write_pretty_json(
                        &live_status_path,
                        &live_status_json(LiveStatusSnapshot {
                            request,
                            out_dir,
                            start,
                            stdout_bytes: stdout_bytes.load(Ordering::Relaxed),
                            stderr_bytes: stderr_bytes.load(Ordering::Relaxed),
                            last_output_age_ms: last_output_age,
                            gpu_activity_observed: gpu_activity_seen,
                            backend_activity_observed: backend_activity_seen,
                            status: "running",
                        }),
                        "run status",
                    )?;
                    append_jsonl(
                        &heartbeat_path,
                        &json!({
                            "timestamp": Utc::now().to_rfc3339(),
                            "elapsed_ms": start.elapsed().as_millis(),
                            "stdout_bytes": stdout_bytes.load(Ordering::Relaxed),
                            "stderr_bytes": stderr_bytes.load(Ordering::Relaxed),
                            "last_output_age_ms": last_output_age,
                            "gpu_activity_observed": gpu_activity_seen,
                            "backend_activity_observed": backend_activity_seen,
                            "gpu_sample_written": gpu_sample_written,
                            "backend_sample_written": backend_sample_written,
                            "status": "running",
                            "opencode_segment": segment_index,
                        }),
                    )?;
                    last_heartbeat = Instant::now();
                }

                if let Some(control) = read_supervisor_control(&supervisor_control_path) {
                    let action = normalize_supervisor_control_action(get_str(&control, "action"));
                    let consumed_path = out_dir.join("control.consumed.json");
                    let _ = fs::rename(&supervisor_control_path, consumed_path);
                    if action == "wait" {
                        continue;
                    }
                    let worker_mode = normalize_supervisor_control_worker_mode(
                        &action,
                        get_str(&control, "worker_mode"),
                    );
                    let message_to_worker = get_str(&control, "message_to_worker")
                        .or_else(|| get_str(&control, "message"))
                        .or_else(|| get_str(&control, "hint"))
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let message_to_worker = if message_to_worker.is_empty() {
                        "Continue from the current state. Stay focused, make progress, and finish with compact results."
                        .to_string()
                    } else {
                        message_to_worker
                    };
                    let event = SupervisorControlEvent {
                        timestamp: Utc::now().to_rfc3339(),
                        action: action.clone(),
                        worker_mode: worker_mode.clone(),
                        message_to_worker: message_to_worker.clone(),
                        focus_files: get_string_array(&control, "focus_files"),
                        required_checks: get_string_array(&control, "required_checks"),
                        risk: get_str(&control, "risk").unwrap_or("").to_string(),
                        control,
                        stdout_bytes: stdout_bytes.load(Ordering::Relaxed),
                        stderr_bytes: stderr_bytes.load(Ordering::Relaxed),
                        last_output_age_ms: last_output_age,
                        opencode_segment: segment_index,
                    };
                    let event_json = serde_json::to_value(&event)
                        .context("failed to serialize supervisor control event")?;
                    append_jsonl(&supervisor_control_log_path, &event_json)?;
                    append_jsonl(
                        &heartbeat_path,
                        &json!({
                            "timestamp": Utc::now().to_rfc3339(),
                            "elapsed_ms": start.elapsed().as_millis(),
                            "status": "supervisor_control",
                            "action": action.clone(),
                            "worker_mode": worker_mode.clone(),
                            "stdout_bytes": stdout_bytes.load(Ordering::Relaxed),
                            "stderr_bytes": stderr_bytes.load(Ordering::Relaxed),
                            "last_output_age_ms": last_output_age,
                            "opencode_segment": segment_index,
                        }),
                    )?;
                    supervisor_control_events.push(event);
                    supervisor_control_action = Some(action.clone());
                    notes.push(format!(
                        "Supervisor control requested `{action}` for OpenCode."
                    ));
                    let _ = process.child.kill();
                    let status = process.child.wait().ok().and_then(|status| status.code());

                    match action.as_str() {
                        "interrupt_continue" => {
                            let mut session_id = active_opencode_session_id.clone();
                            if session_id.is_none() {
                                match resolve_opencode_session_id(command, root, &segment_label) {
                                Ok(Some(resolved)) => session_id = Some(resolved),
                                Ok(None) => {}
                                Err(error) => notes.push(format!(
                                    "OpenCode session id resolution failed during interrupt_continue: {error}"
                                )),
                            }
                            }
                            if let Some(session_id) = session_id {
                                active_opencode_session_id = Some(session_id.clone());
                                let next_args = prepare_opencode_control_args(
                                    args,
                                    request,
                                    Some(&session_id),
                                    &segment_label,
                                    &message_to_worker,
                                );
                                next_segment = Some(NextOpenCodeSegment {
                                    args: next_args,
                                    label: segment_label.clone(),
                                    resume_session_id: Some(session_id),
                                    action: action.clone(),
                                    worker_mode: worker_mode.to_string(),
                                    message: message_to_worker.clone(),
                                });
                            } else {
                                interrupted_by_supervisor = true;
                                segment_stopped_by_supervisor = true;
                                notes.push(
                                "Supervisor requested interrupt_continue, but Mixmod could not resolve an OpenCode session id to resume."
                                    .to_string(),
                            );
                            }
                        }
                        "interrupt_context_focus" => {
                            active_opencode_session_id = None;
                            let next_args = prepare_opencode_control_args(
                                args,
                                request,
                                None,
                                &request.session_id,
                                &message_to_worker,
                            );
                            next_segment = Some(NextOpenCodeSegment {
                                args: next_args,
                                label: request.session_id.clone(),
                                resume_session_id: None,
                                action: action.clone(),
                                worker_mode: worker_mode.to_string(),
                                message: message_to_worker.clone(),
                            });
                        }
                        "stop" => {
                            interrupted_by_supervisor = true;
                            segment_stopped_by_supervisor = true;
                        }
                        _ => {}
                    }
                    break status;
                }

                std::thread::sleep(Duration::from_millis(500));
            };

            join_pipe_logger(process.stdout_thread, "stdout", &mut notes);
            join_pipe_logger(process.stderr_thread, "stderr", &mut notes);

            let segment_ended = Utc::now();
            let segment_stdout_end = stdout_bytes.load(Ordering::Relaxed);
            let segment_stderr_end = stderr_bytes.load(Ordering::Relaxed);
            opencode_segments.push(json!({
                "index": segment_index,
                "action": segment_action.clone(),
                "worker_mode": segment_worker_mode.clone(),
                "message_to_worker": segment_message.clone(),
                "session_label": segment_label.clone(),
                "resume_session_id": segment_resume_session_id.clone(),
                "command": segment_command,
                "start_timestamp": segment_started.to_rfc3339(),
                "end_timestamp": segment_ended.to_rfc3339(),
                "exit_status": segment_exit_status,
                "stdout_start_bytes": segment_stdout_start,
                "stdout_end_bytes": segment_stdout_end,
                "stdout_delta_bytes": segment_stdout_end.saturating_sub(segment_stdout_start),
                "stderr_start_bytes": segment_stderr_start,
                "stderr_end_bytes": segment_stderr_end,
                "stderr_delta_bytes": segment_stderr_end.saturating_sub(segment_stderr_start),
                "timed_out": segment_timed_out,
                "idle_timed_out": segment_idle_timed_out,
                "stopped_by_supervisor": segment_stopped_by_supervisor,
                "continued": next_segment.is_some(),
            }));

            if let Some(next) = next_segment {
                segment_args = next.args;
                segment_label = next.label;
                segment_resume_session_id = next.resume_session_id;
                segment_action = next.action;
                segment_worker_mode = next.worker_mode;
                segment_message = next.message;
                continue 'segments;
            }

            break 'segments segment_exit_status;
        };

        let final_status = if timed_out {
            "worker_timeout"
        } else if idle_timed_out {
            "idle_timeout"
        } else if interrupted_by_supervisor {
            "supervisor_interrupt"
        } else {
            "finished"
        };
        let final_last_output_age =
            now_millis().saturating_sub(last_output_at.load(Ordering::Relaxed));
        write_pretty_json(
            &live_status_path,
            &live_status_json(LiveStatusSnapshot {
                request,
                out_dir,
                start,
                stdout_bytes: stdout_bytes.load(Ordering::Relaxed),
                stderr_bytes: stderr_bytes.load(Ordering::Relaxed),
                last_output_age_ms: final_last_output_age,
                gpu_activity_observed: gpu_activity_seen,
                backend_activity_observed: backend_activity_seen,
                status: final_status,
            }),
            "run status",
        )?;

        append_jsonl(
            &heartbeat_path,
            &json!({
                "timestamp": Utc::now().to_rfc3339(),
                "elapsed_ms": start.elapsed().as_millis(),
                "stdout_bytes": stdout_bytes.load(Ordering::Relaxed),
                "stderr_bytes": stderr_bytes.load(Ordering::Relaxed),
                "last_output_age_ms": final_last_output_age,
                "gpu_activity_observed": gpu_activity_seen,
                "backend_activity_observed": backend_activity_seen,
                "exit_status": exit_status,
                "timed_out": timed_out,
                "idle_timed_out": idle_timed_out,
                "status": final_status
            }),
        )?;

        let _after_gpu = if verification_config.enabled {
            run_optional_logged_command(
                &verification_config.gpu_command,
                root,
                &logs_dir.join("nvidia-smi-after.txt"),
            )
        } else {
            None
        };

        let stdout = fs::read(&stdout_path).unwrap_or_default();
        let stderr = fs::read(&stderr_path).unwrap_or_default();

        let gpu_activity_observed = gpu_activity_seen;
        let backend_activity_observed = backend_activity_seen;
        if gpu_activity_observed {
            notes.push("GPU activity was observed during OpenCode execution.".to_string());
        } else {
            notes.push(
                "No positive GPU activity evidence was observed during sampling.".to_string(),
            );
        }
        if backend_activity_observed {
            notes.push("Backend command showed the configured Qwen model active.".to_string());
        } else {
            notes
                .push("Backend command did not show the configured Qwen model active.".to_string());
        }
        if before_gpu.is_none() && verification_config.enabled {
            notes.push(format!(
                "GPU verification command `{}` was unavailable or failed.",
                verification_config.gpu_command
            ));
        }
        if heartbeat_count == 0 && verification_config.enabled {
            notes.push(format!(
                "Backend verification command `{}` was unavailable or failed.",
                verification_config.backend_command
            ));
        }
        let local_inference_verified = selection.require_local
            && is_allowed_local_provider(&selection.provider, opencode_config)
            && (gpu_activity_observed || backend_activity_observed);

        if selection.require_local && !local_inference_verified {
            notes.push(
            "Required local inference could not be verified with positive GPU/backend evidence."
                .to_string(),
        );
        }

        let verification_json = json!({
            "opencode_provider": selection.provider,
            "opencode_model": selection.model,
            "opencode_model_arg": selection.model_arg,
            "require_local": selection.require_local,
            "local_inference_verified": local_inference_verified,
            "gpu_activity_observed": gpu_activity_observed,
            "backend_activity_observed": backend_activity_observed,
            "heartbeat_count": heartbeat_count,
            "heartbeat_seconds": heartbeat_seconds,
            "worker_timeout_seconds": worker_timeout_seconds,
            "idle_timeout_seconds": idle_timeout_seconds,
            "timed_out": timed_out,
            "idle_timed_out": idle_timed_out,
            "interrupted_by_supervisor": interrupted_by_supervisor,
            "supervisor_control_action": supervisor_control_action.clone(),
            "supervisor_control_events": supervisor_control_events.clone(),
            "opencode_segments": opencode_segments.clone(),
            "verification_notes": notes,
            "logs": {
                "nvidia_smi_before": "logs/nvidia-smi-before.txt",
                "nvidia_smi_during": "logs/nvidia-smi-during.txt",
                "nvidia_smi_after": "logs/nvidia-smi-after.txt",
                "backend": "logs/ollama-ps.txt",
                "heartbeat": "logs/heartbeat.jsonl"
            }
        });
        write_pretty_json(
            &out_dir.join(LOCAL_VERIFICATION_JSON),
            &verification_json,
            "local verification",
        )?;

        Ok(VerifiedCommandOutput {
            exit_status,
            stdout,
            stderr,
            opencode_segments,
            timed_out,
            idle_timed_out,
            interrupted_by_supervisor,
            supervisor_control_action,
            supervisor_control_events,
            heartbeat_count,
            local_inference_verified,
            gpu_activity_observed,
            backend_activity_observed,
            verification_notes: notes,
        })
    }
}

struct LiveStatusSnapshot<'a> {
    request: &'a AgentRequest,
    out_dir: &'a Path,
    start: Instant,
    stdout_bytes: u64,
    stderr_bytes: u64,
    last_output_age_ms: u64,
    gpu_activity_observed: bool,
    backend_activity_observed: bool,
    status: &'a str,
}

fn live_status_json(snapshot: LiveStatusSnapshot<'_>) -> Value {
    let request = snapshot.request;
    json!({
        "timestamp": Utc::now().to_rfc3339(),
        "status": snapshot.status,
        "mode": request.mode.to_string(),
        "task_file": request.task_path.to_string_lossy().to_string(),
        "out_dir": snapshot.out_dir.to_string_lossy().to_string(),
        "opencode_session_label": request.session_id.as_str(),
        "opencode_resume_session_id": request.resume_session_id.as_deref(),
        "elapsed_ms": snapshot.start.elapsed().as_millis(),
        "stdout_bytes": snapshot.stdout_bytes,
        "stderr_bytes": snapshot.stderr_bytes,
        "last_output_age_ms": snapshot.last_output_age_ms,
        "gpu_activity_observed": snapshot.gpu_activity_observed,
        "backend_activity_observed": snapshot.backend_activity_observed,
        "status_file": LIVE_STATUS_FILE,
        "control_file": SUPERVISOR_CONTROL_FILE,
        "control_log": SUPERVISOR_CONTROL_LOG,
    })
}

fn read_supervisor_control(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn normalize_supervisor_control_action(value: Option<&str>) -> String {
    match value
        .unwrap_or("wait")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "interrupt_continue" | "continue" | "steer" | "revise" => "interrupt_continue".to_string(),
        "interrupt_context_focus" | "context_focus" | "focus" | "fresh" | "reset" => {
            "interrupt_context_focus".to_string()
        }
        "stop" | "halt" => "stop".to_string(),
        _ => "wait".to_string(),
    }
}

pub(crate) fn normalize_supervisor_control_worker_mode(
    action: &str,
    value: Option<&str>,
) -> String {
    match action {
        "interrupt_context_focus" => "context_focus".to_string(),
        "interrupt_continue" => "continue".to_string(),
        _ => normalize_worker_mode(value),
    }
}

pub(crate) fn tail_text(path: &Path, max_bytes: usize) -> String {
    let bytes = fs::read(path).unwrap_or_default();
    let start = bytes.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&bytes[start..]).to_string()
}

struct RunningOpenCodeProcess {
    child: std::process::Child,
    stdout_thread: JoinHandle<Result<()>>,
    stderr_thread: JoinHandle<Result<()>>,
}

struct SpawnOpenCodeProcess<'a> {
    command: &'a str,
    args: &'a [String],
    root: &'a Path,
    stdout_path: &'a Path,
    stderr_path: &'a Path,
    stdout_bytes: Arc<AtomicU64>,
    stderr_bytes: Arc<AtomicU64>,
    last_output_at: Arc<AtomicU64>,
}

fn spawn_opencode_process(config: SpawnOpenCodeProcess<'_>) -> Result<RunningOpenCodeProcess> {
    let mut child = opencode_command(config.command, config.root)
        .args(config.args)
        .current_dir(config.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn OpenCode command `{}`", config.command))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture OpenCode stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture OpenCode stderr"))?;
    let stdout_thread = spawn_pipe_logger(
        stdout,
        config.stdout_path.to_path_buf(),
        config.stdout_bytes,
        Arc::clone(&config.last_output_at),
    );
    let stderr_thread = spawn_pipe_logger(
        stderr,
        config.stderr_path.to_path_buf(),
        config.stderr_bytes,
        config.last_output_at,
    );
    Ok(RunningOpenCodeProcess {
        child,
        stdout_thread,
        stderr_thread,
    })
}

fn spawn_pipe_logger<R: Read + Send + 'static>(
    mut reader: R,
    path: PathBuf,
    counter: Arc<AtomicU64>,
    last_output_at: Arc<AtomicU64>,
) -> JoinHandle<Result<()>> {
    std::thread::spawn(move || {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader
                .read(&mut buffer)
                .with_context(|| format!("failed to read pipe for {}", path.display()))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .with_context(|| format!("failed to write {}", path.display()))?;
            file.flush().ok();
            counter.fetch_add(read as u64, Ordering::Relaxed);
            last_output_at.store(now_millis(), Ordering::Relaxed);
        }
        Ok(())
    })
}

fn join_pipe_logger(handle: JoinHandle<Result<()>>, label: &str, notes: &mut Vec<String>) {
    match handle.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => notes.push(format!("OpenCode {label} log streaming failed: {error}")),
        Err(_) => notes.push(format!("OpenCode {label} log streaming thread panicked")),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn run_optional_logged_command(command: &str, root: &Path, log_path: &Path) -> Option<String> {
    if command.trim().is_empty() {
        let _ = atomic_write(log_path, b"command not configured\n");
        return None;
    }
    let output = shell_command(command).current_dir(root).output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&format!("$ {command}\n"));
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text.push_str(&format!("\nexit_status={:?}\n", output.status.code()));
            let _ = atomic_write(log_path, text.as_bytes());
            output.status.success().then_some(text)
        }
        Err(error) => {
            let text = format!("failed to run `{command}`: {error}\n");
            let _ = atomic_write(log_path, text.as_bytes());
            None
        }
    }
}

fn run_optional_command_text(command: &str, root: &Path) -> Option<String> {
    if command.trim().is_empty() {
        return None;
    }
    let output = shell_command(command).current_dir(root).output().ok()?;
    let mut text = String::new();
    text.push_str(&format!("$ {command}\n"));
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.push_str(&format!("\nexit_status={:?}\n", output.status.code()));
    output.status.success().then_some(text)
}

fn gpu_activity_observed(before: Option<&str>, during: Option<&str>) -> bool {
    let Some(during) = during else {
        return false;
    };
    let before_memory = before.and_then(parse_gpu_memory_mib).unwrap_or(0);
    let during_memory = parse_gpu_memory_mib(during).unwrap_or(0);
    let during_util = parse_gpu_util_percent(during).unwrap_or(0);
    let lower = during.to_ascii_lowercase();
    lower.contains("ollama")
        || lower.contains("vllm")
        || lower.contains("llama")
        || during_util > 0
        || during_memory.saturating_sub(before_memory) > 500
}

fn parse_gpu_memory_mib(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some((left, right)) = line.split_once("MiB /") {
            let value = left.split_whitespace().last()?.trim().parse::<u64>().ok();
            if value.is_some() && right.contains("MiB") {
                return value;
            }
        }
    }
    None
}

fn parse_gpu_util_percent(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some((left, _)) = line.split_once("%")
            && (line.contains("Default") || line.contains("MiB"))
            && let Some(value) = left.split_whitespace().last()
            && let Ok(value) = value.parse::<u64>()
        {
            return Some(value);
        }
    }
    None
}

fn backend_activity_observed(text: Option<&str>, selection: &OpenCodeModelSelection) -> bool {
    let Some(text) = text else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    let model = selection.model.to_ascii_lowercase();
    let model_arg = selection.model_arg.to_ascii_lowercase();
    lower.contains(&model)
        || lower.contains(&model_arg)
        || lower.contains(DEFAULT_OPENCODE_OLLAMA_MODEL)
}
fn render_opencode_arg(
    arg: &str,
    request: &AgentRequest,
    selection: &OpenCodeModelSelection,
) -> String {
    let resume_session_id = request.resume_session_id.as_deref().unwrap_or_default();
    arg.replace("{instruction}", &request.instruction)
        .replace(
            "{instruction_file}",
            &request.instruction_path.to_string_lossy(),
        )
        .replace("{task_file}", &request.task_path.to_string_lossy())
        .replace("{mode}", &request.mode.to_string())
        .replace("{out_dir}", &request.out_dir.to_string_lossy())
        .replace("{model}", &selection.model)
        .replace("{provider}", &selection.provider)
        .replace("{model_arg}", &selection.model_arg)
        .replace("{session_id}", &request.session_id)
        .replace("{resume_session_id}", resume_session_id)
}

fn redact_opencode_arg(
    arg: &str,
    request: &AgentRequest,
    selection: &OpenCodeModelSelection,
) -> String {
    if arg.contains("{instruction}") {
        arg.replace(
            "{instruction}",
            &format!("<instruction:{} bytes>", request.instruction.len()),
        )
        .replace("{model}", &selection.model)
        .replace("{provider}", &selection.provider)
        .replace("{model_arg}", &selection.model_arg)
        .replace("{session_id}", &request.session_id)
        .replace(
            "{resume_session_id}",
            request.resume_session_id.as_deref().unwrap_or_default(),
        )
    } else {
        render_opencode_arg(arg, request, selection)
    }
}

fn redact_runtime_opencode_arg(arg: &str, request: &AgentRequest) -> String {
    if arg == request.instruction {
        format!("<instruction:{} bytes>", request.instruction.len())
    } else if arg == request.instruction_path.to_string_lossy().as_ref() {
        "<instruction_file>".to_string()
    } else {
        arg.to_string()
    }
}

pub(crate) fn prepare_opencode_args(
    mut args: Vec<String>,
    resume_session_id: Option<&str>,
) -> Vec<String> {
    let Some(resume_session_id) = resume_session_id else {
        return args;
    };
    args = remove_opencode_title_args(args);
    if has_opencode_session_arg(&args) {
        return args;
    }
    let insert_at = if args.first().map(|arg| arg == "run").unwrap_or(false) {
        1
    } else {
        0
    };
    args.insert(insert_at, "--session".to_string());
    args.insert(insert_at + 1, resume_session_id.to_string());
    args
}

pub(crate) fn prepare_opencode_control_args(
    base_args: &[String],
    request: &AgentRequest,
    resume_session_id: Option<&str>,
    session_label: &str,
    message: &str,
) -> Vec<String> {
    let mut args = remove_opencode_session_args(remove_opencode_title_args(base_args.to_vec()));
    let instruction_file = request.instruction_path.to_string_lossy();
    args.retain(|arg| arg != &request.instruction && arg != instruction_file.as_ref());
    let insert_at = if args.first().map(|arg| arg == "run").unwrap_or(false) {
        1
    } else {
        0
    };
    if let Some(session_id) = resume_session_id {
        args.insert(insert_at, "--session".to_string());
        args.insert(insert_at + 1, session_id.to_string());
    } else {
        args.insert(insert_at, "--title".to_string());
        args.insert(insert_at + 1, session_label.to_string());
    }
    args.push(message.to_string());
    args
}

fn remove_opencode_title_args(args: Vec<String>) -> Vec<String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--title" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--title=") {
            continue;
        }
        filtered.push(arg);
    }
    filtered
}

fn remove_opencode_session_args(args: Vec<String>) -> Vec<String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--session" || arg == "-s" {
            skip_next = true;
            continue;
        }
        if arg == "--continue" || arg == "-c" || arg.starts_with("--session=") {
            continue;
        }
        filtered.push(arg);
    }
    filtered
}

fn has_opencode_session_arg(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "--session" || arg == "-s" || arg.starts_with("--session="))
}

fn resolve_opencode_session_id(
    command: &str,
    work_dir: &Path,
    title: &str,
) -> Result<Option<String>> {
    let sql = format!(
        "select id from session where title = '{}' and directory = '{}' order by time_updated desc limit 1;",
        sql_string_literal_content(title),
        sql_string_literal_content(&work_dir.to_string_lossy())
    );
    let output = Command::new(command)
        .env("OPENCODE_CONFIG", opencode_config_path(work_dir))
        .args(["db", &sql, "--format", "json"])
        .output()
        .with_context(|| format!("failed to query OpenCode sessions with `{command} db`"))?;
    if !output.status.success() {
        bail!(
            "`{command} db` failed while resolving session id: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .with_context(|| "failed to parse OpenCode session query JSON")?;
    if let Some(rows) = value.as_array() {
        return Ok(rows
            .first()
            .and_then(|row| get_str(row, "id"))
            .map(ToOwned::to_owned));
    }
    if let Some(rows) = value.get("rows").and_then(Value::as_array) {
        return Ok(rows
            .first()
            .and_then(|row| get_str(row, "id"))
            .map(ToOwned::to_owned));
    }
    Ok(None)
}

pub(crate) fn opencode_config_path(root: &Path) -> PathBuf {
    state_layout(root).opencode_config()
}

fn opencode_command(command: &str, root: &Path) -> Command {
    let mut process = Command::new(command);
    process.env("OPENCODE_CONFIG", opencode_config_path(root));
    process
}

fn sql_string_literal_content(value: &str) -> String {
    value.replace('\'', "''")
}
