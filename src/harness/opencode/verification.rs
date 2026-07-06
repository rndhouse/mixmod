use std::fs;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{Value, json};

use crate::harness::AgentRequest;
use crate::{
    DEFAULT_OPENCODE_OLLAMA_MODEL, LIVE_STATUS_FILE, LOCAL_VERIFICATION_JSON, OpenCodeConfig,
    SUPERVISOR_CONTROL_FILE, SUPERVISOR_CONTROL_LOG, SupervisorControlEvent, append_file,
    append_jsonl, atomic_write, diff_without_unchanged_blocks, env_u64, get_bool, get_str,
    get_string_array, git_diff_with_untracked, write_pretty_json,
};

use super::args::{prepare_opencode_control_args, redact_runtime_opencode_arg};
use super::config::{
    OpenCodeModelSelection, is_allowed_local_provider, resolve_opencode_session_id,
};
use super::control::{
    LiveStatusSnapshot, live_status_json, normalize_supervisor_control_action,
    normalize_supervisor_control_worker_mode, read_supervisor_control,
};
use super::process::{
    SpawnOpenCodeProcess, join_pipe_logger, now_millis, run_optional_command_text,
    run_optional_logged_command, spawn_opencode_process,
};

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

const DEFAULT_SMALL_PATCH_NO_DELTA_INTERRUPT_SECONDS: u64 = 90;
const DEFAULT_SMALL_PATCH_NO_DELTA_MAX_INTERRUPTS: u64 = 1;

#[derive(Debug)]
struct SmallPatchNoDeltaIntervention {
    threshold: Duration,
    baseline_diff: String,
    interrupt_control: Value,
    stop_control: Value,
    interrupt_count: u64,
    max_interrupts: u64,
    stopped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmallPatchNoDeltaKind {
    Initial,
    Revision,
}

#[derive(Debug)]
struct SmallPatchNoDeltaTarget {
    kind: SmallPatchNoDeltaKind,
    focus_files: Vec<String>,
    exact_edits: Vec<String>,
    message_to_worker: String,
}

impl SmallPatchNoDeltaIntervention {
    fn from_request(request: &AgentRequest) -> Option<Self> {
        let threshold_seconds = env_u64("MIXMOD_SMALL_PATCH_NO_DELTA_INTERRUPT_SECONDS")
            .or_else(|| env_u64("MIXMOD_REVISION_NO_DELTA_INTERRUPT_SECONDS"))
            .unwrap_or(DEFAULT_SMALL_PATCH_NO_DELTA_INTERRUPT_SECONDS);
        if threshold_seconds == 0 {
            return None;
        }
        let max_interrupts = env_u64("MIXMOD_SMALL_PATCH_NO_DELTA_MAX_INTERRUPTS")
            .or_else(|| env_u64("MIXMOD_REVISION_NO_DELTA_MAX_INTERRUPTS"))
            .unwrap_or(DEFAULT_SMALL_PATCH_NO_DELTA_MAX_INTERRUPTS);
        let task = fs::read_to_string(&request.task_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())?;
        let baseline_diff = git_diff_with_untracked(&request.root).ok()?;
        Self::from_task(&task, baseline_diff, threshold_seconds, max_interrupts)
    }

    fn from_task(
        task: &Value,
        baseline_diff: String,
        threshold_seconds: u64,
        max_interrupts: u64,
    ) -> Option<Self> {
        if threshold_seconds == 0 {
            return None;
        }
        let target = small_patch_no_delta_target_from_task(task)?;
        Some(Self {
            threshold: Duration::from_secs(threshold_seconds),
            baseline_diff,
            interrupt_control: small_patch_no_delta_interrupt_control(&target),
            stop_control: small_patch_no_delta_stop_control(&target),
            interrupt_count: 0,
            max_interrupts,
            stopped: false,
        })
    }

    fn maybe_control(&mut self, root: &Path, elapsed: Duration) -> Option<Value> {
        let current_diff = git_diff_with_untracked(root).ok()?;
        self.maybe_control_for_diff(&current_diff, elapsed)
    }

    fn maybe_control_for_diff(&mut self, current_diff: &str, elapsed: Duration) -> Option<Value> {
        if self.stopped || elapsed < self.threshold {
            return None;
        }
        let new_delta = diff_without_unchanged_blocks(current_diff, &self.baseline_diff);
        if !new_delta.trim().is_empty() {
            return None;
        }
        if self.interrupt_count < self.max_interrupts {
            self.interrupt_count += 1;
            return Some(self.interrupt_control.clone());
        }
        self.stopped = true;
        Some(self.stop_control.clone())
    }
}

fn small_patch_no_delta_target_from_task(task: &Value) -> Option<SmallPatchNoDeltaTarget> {
    let context = task.get("context")?;
    if let Some(revision) = context.get("revision") {
        let delta_expected = get_bool(revision, "delta_expected").unwrap_or_else(|| {
            let patch_decision = get_str(revision, "patch_decision").unwrap_or("");
            matches!(patch_decision, "revise_current" | "revise_previous")
        });
        let small_patch_slice = get_str(revision, "worker_turn_shape")
            .is_some_and(|shape| shape.trim() == "small_patch_slice");
        if delta_expected && small_patch_slice {
            return Some(SmallPatchNoDeltaTarget {
                kind: SmallPatchNoDeltaKind::Revision,
                focus_files: get_string_array(revision, "focus_files"),
                exact_edits: get_string_array(revision, "exact_edits"),
                message_to_worker: get_str(revision, "message_to_worker")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            });
        }
    }

    let brief = context.get("worker_brief")?;
    let expect_patch = get_bool(task, "expect_patch")
        .or_else(|| get_bool(context, "expect_patch"))
        .or_else(|| get_bool(brief, "expect_patch"))
        .unwrap_or(true);
    let small_patch_slice = get_str(brief, "worker_turn_shape")
        .is_some_and(|shape| shape.trim() == "small_patch_slice");
    if !expect_patch || !small_patch_slice {
        return None;
    }
    let mut focus_files = first_non_empty_string_array(
        brief,
        &["focus_files", "files", "target_files", "repo_focus_files"],
    );
    if focus_files.is_empty() {
        focus_files = get_string_array(task, "files");
    }
    let exact_edits =
        first_non_empty_string_array(brief, &["exact_edits", "edit_plan", "implementation_steps"]);
    Some(SmallPatchNoDeltaTarget {
        kind: SmallPatchNoDeltaKind::Initial,
        focus_files,
        exact_edits,
        message_to_worker: get_str(brief, "message_to_worker")
            .or_else(|| get_str(brief, "message"))
            .unwrap_or("")
            .trim()
            .to_string(),
    })
}

fn first_non_empty_string_array(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| get_string_array(value, key))
        .find(|items| !items.is_empty())
        .unwrap_or_default()
}

fn small_patch_no_delta_interrupt_control(target: &SmallPatchNoDeltaTarget) -> Value {
    let first_edit = target
        .exact_edits
        .first()
        .map(String::as_str)
        .filter(|edit| !edit.trim().is_empty())
        .or_else(|| {
            (!target.message_to_worker.is_empty()).then_some(target.message_to_worker.as_str())
        })
        .unwrap_or("Apply the next requested source edit.")
        .trim();
    let first_file = target
        .focus_files
        .first()
        .map(String::as_str)
        .filter(|file| !file.trim().is_empty())
        .unwrap_or("the focused source file");
    let (source, phase, risk) = match target.kind {
        SmallPatchNoDeltaKind::Initial => (
            "auto_initial_no_delta",
            "first worker turn",
            "initial small-patch worker made no repository delta before the no-delta guard fired",
        ),
        SmallPatchNoDeltaKind::Revision => (
            "auto_revision_no_delta",
            "revision",
            "revision made no new repository delta before the no-delta guard fired",
        ),
    };
    json!({
        "action": "interrupt_continue",
        "worker_mode": "continue",
        "source": source,
        "worker_turn_shape": "small_patch_slice",
        "turn_goal": "make the first no-delta recovery edit",
        "exact_edits": [first_edit],
        "defer_checks_until_patch_exists": true,
        "completion_gate": "git diff --stat must be non-empty",
        "forbidden_actions": ["ask questions", "run tests before editing"],
        "message_to_worker": format!(
            "You have not modified files in this {phase}. Make only this edit now in {first_file}: {first_edit} Then run git diff --stat and stop with Diff non-empty: yes/no. Do not run tests."
        ),
        "focus_files": target.focus_files.clone(),
        "required_checks": [],
        "risk": risk
    })
}

fn small_patch_no_delta_stop_control(target: &SmallPatchNoDeltaTarget) -> Value {
    let first_edit = target
        .exact_edits
        .first()
        .map(String::as_str)
        .filter(|edit| !edit.trim().is_empty())
        .or_else(|| {
            (!target.message_to_worker.is_empty()).then_some(target.message_to_worker.as_str())
        })
        .unwrap_or("the requested small-patch edit")
        .trim();
    let source = match target.kind {
        SmallPatchNoDeltaKind::Initial => "auto_initial_no_delta_stop",
        SmallPatchNoDeltaKind::Revision => "auto_revision_no_delta_stop",
    };
    json!({
        "action": "stop",
        "worker_mode": "continue",
        "source": source,
        "message_to_worker": format!(
            "Worker made no repository delta after no-delta recovery. Stopping after failing to apply: {first_edit}"
        ),
        "focus_files": target.focus_files.clone(),
        "required_checks": [],
        "risk": "worker_stalled_no_delta"
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
        let mut small_patch_no_delta_intervention =
            SmallPatchNoDeltaIntervention::from_request(request);

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
            let segment_started_instant = Instant::now();
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

                if !supervisor_control_path.exists() {
                    if let Some(control) =
                        small_patch_no_delta_intervention
                            .as_mut()
                            .and_then(|intervention| {
                                intervention.maybe_control(root, segment_started_instant.elapsed())
                            })
                    {
                        write_pretty_json(
                            &supervisor_control_path,
                            &control,
                            "revision no-delta supervisor control",
                        )?;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn revision_slice_task() -> Value {
        json!({
            "context": {
                "revision": {
                    "delta_expected": true,
                    "worker_turn_shape": "small_patch_slice",
                    "patch_decision": "revise_current",
                    "message_to_worker": "Add the serialization branch.",
                    "focus_files": ["builder.py", "test_builder.py"],
                    "exact_edits": ["Add only the flatten=True serialization branch."]
                }
            }
        })
    }

    fn initial_slice_task() -> Value {
        json!({
            "expect_patch": true,
            "files": ["helper.py", "test_helper.py"],
            "context": {
                "worker_brief": {
                    "expect_patch": true,
                    "worker_turn_shape": "small_patch_slice",
                    "message_to_worker": "Add flatten metadata.",
                    "files": ["helper.py", "test_helper.py"],
                    "exact_edits": ["Add only the flatten metadata field."]
                }
            }
        })
    }

    #[test]
    fn small_patch_no_delta_guard_interrupts_then_stops_after_thresholds() {
        let mut guard =
            SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 2, 1)
                .unwrap();

        assert!(
            guard
                .maybe_control_for_diff("", Duration::from_secs(1))
                .is_none()
        );
        let interrupt = guard
            .maybe_control_for_diff("", Duration::from_secs(2))
            .unwrap();

        assert_eq!(get_str(&interrupt, "action"), Some("interrupt_continue"));
        assert_eq!(
            get_str(&interrupt, "source"),
            Some("auto_revision_no_delta")
        );
        assert!(
            get_str(&interrupt, "message_to_worker")
                .unwrap()
                .contains("Add only the flatten=True serialization branch.")
        );
        let stop = guard
            .maybe_control_for_diff("", Duration::from_secs(2))
            .unwrap();
        assert_eq!(get_str(&stop, "action"), Some("stop"));
        assert_eq!(
            get_str(&stop, "source"),
            Some("auto_revision_no_delta_stop")
        );
        assert_eq!(get_str(&stop, "risk"), Some("worker_stalled_no_delta"));
        assert!(
            guard
                .maybe_control_for_diff("", Duration::from_secs(3))
                .is_none()
        );
    }

    #[test]
    fn small_patch_no_delta_guard_ignores_existing_new_delta() {
        let mut guard =
            SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 1, 1)
                .unwrap();
        let current_diff = "\
diff --git a/builder.py b/builder.py
--- a/builder.py
+++ b/builder.py
@@ -1,1 +1,2 @@
 old
+new
";

        assert!(
            guard
                .maybe_control_for_diff(current_diff, Duration::from_secs(5))
                .is_none()
        );
    }

    #[test]
    fn small_patch_no_delta_guard_does_not_stop_after_recovery_delta() {
        let mut guard =
            SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 1, 1)
                .unwrap();
        assert!(
            guard
                .maybe_control_for_diff("", Duration::from_secs(1))
                .is_some()
        );
        let current_diff = "\
diff --git a/builder.py b/builder.py
--- a/builder.py
+++ b/builder.py
@@ -1,1 +1,2 @@
 old
+new
";

        assert!(
            guard
                .maybe_control_for_diff(current_diff, Duration::from_secs(1))
                .is_none()
        );
    }

    #[test]
    fn small_patch_no_delta_guard_handles_initial_worker_brief() {
        let mut guard =
            SmallPatchNoDeltaIntervention::from_task(&initial_slice_task(), String::new(), 1, 1)
                .unwrap();

        let interrupt = guard
            .maybe_control_for_diff("", Duration::from_secs(1))
            .unwrap();

        assert_eq!(get_str(&interrupt, "action"), Some("interrupt_continue"));
        assert_eq!(get_str(&interrupt, "source"), Some("auto_initial_no_delta"));
        assert!(
            get_str(&interrupt, "message_to_worker")
                .unwrap()
                .contains("first worker turn")
        );
        assert!(
            get_str(&interrupt, "message_to_worker")
                .unwrap()
                .contains("Add only the flatten metadata field.")
        );
        assert_eq!(
            get_string_array(&interrupt, "focus_files"),
            vec!["helper.py", "test_helper.py"]
        );
    }

    #[test]
    fn small_patch_no_delta_guard_requires_small_patch_slice() {
        let task = json!({
            "context": {
                "revision": {
                    "delta_expected": true,
                    "worker_turn_shape": "default"
                }
            }
        });

        assert!(SmallPatchNoDeltaIntervention::from_task(&task, String::new(), 1, 1).is_none());
    }
}
