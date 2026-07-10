use std::fs;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;

use crate::harness::AgentRequest;
use crate::{
    LIVE_STATUS_FILE, LOCAL_VERIFICATION_JSON, OpenCodeConfig, SUPERVISOR_CONTROL_FILE,
    SUPERVISOR_CONTROL_LOG, SupervisorControlEvent, TOOL_EVENTS_JSONL, append_file, append_jsonl,
    atomic_write, build_tool_events_jsonl, env_u64, get_str, get_string_array,
    git_diff_with_untracked, write_pretty_json,
};

use crate::harness::opencode::args::{prepare_opencode_control_args, redact_runtime_opencode_arg};
use crate::harness::opencode::config::{
    OpenCodeModelSelection, is_allowed_local_provider, resolve_opencode_session_id,
};
use crate::harness::opencode::control::{
    LiveStatusSnapshot, live_status_json, normalize_supervisor_control_action,
    normalize_supervisor_control_worker_mode, read_supervisor_control,
};
use crate::harness::opencode::process::{
    SpawnOpenCodeProcess, join_pipe_logger, now_millis, run_optional_command_text,
    run_optional_logged_command, spawn_opencode_process,
};

use super::backend::{backend_activity_observed, effective_backend_command, gpu_activity_observed};
use super::live_snapshot::{LiveWorkerSnapshotInput, build_live_worker_snapshot};
use super::types::VerifiedCommandOutput;

#[derive(Debug)]
struct NextOpenCodeSegment {
    args: Vec<String>,
    label: String,
    resume_session_id: Option<String>,
    action: String,
    worker_mode: String,
    message: String,
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
        let backend_command = effective_backend_command(&verification_config.backend_command);
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
        let tool_events_path = out_dir.join(TOOL_EVENTS_JSONL);
        atomic_write(&stdout_path, b"")?;
        atomic_write(&stderr_path, b"")?;
        atomic_write(&heartbeat_path, b"")?;
        atomic_write(&supervisor_control_log_path, b"")?;
        atomic_write(&tool_events_path, b"")?;
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
        let live_supervision_baseline_diff = git_diff_with_untracked(root).unwrap_or_default();

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
                        if let Some(text) = run_optional_command_text(&backend_command, root) {
                            let sample = format!("\n--- sample {heartbeat_count} ---\n{text}");
                            append_file(&logs_dir.join("backend-status.txt"), sample.as_bytes())?;
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

                if !supervisor_control_path.exists()
                    && let Some(advisor) = request.supervisor_advisor.as_ref()
                {
                    let stdout = fs::read(&stdout_path).unwrap_or_default();
                    match build_tool_events_jsonl(&stdout) {
                        Ok((events, _count)) => atomic_write(&tool_events_path, events.as_bytes())?,
                        Err(error) => {
                            notes.push(format!("Unable to update live tool events: {error}"))
                        }
                    }
                    match build_live_worker_snapshot(LiveWorkerSnapshotInput {
                        request,
                        out_dir,
                        tool_events_path: &tool_events_path,
                        stdout_path: &stdout_path,
                        stderr_path: &stderr_path,
                        baseline_diff: &live_supervision_baseline_diff,
                        segment_stdout_start,
                        start,
                        segment_started_instant,
                        segment_index,
                        segment_action: &segment_action,
                        segment_worker_mode: &segment_worker_mode,
                        worker_provider: &selection.provider,
                        segment_label: &segment_label,
                        segment_resume_session_id: segment_resume_session_id.as_deref(),
                        stdout_bytes: stdout_bytes.load(Ordering::Relaxed),
                        stderr_bytes: stderr_bytes.load(Ordering::Relaxed),
                        last_output_age,
                    }) {
                        Ok(snapshot) => match advisor.advise(&snapshot) {
                            Ok(Some(control)) => {
                                write_pretty_json(
                                    &supervisor_control_path,
                                    &control,
                                    "live supervisor control",
                                )?;
                            }
                            Ok(None) => {}
                            Err(error) => notes.push(format!(
                                "Live supervisor check failed during OpenCode heartbeat: {error}"
                            )),
                        },
                        Err(error) => {
                            notes.push(format!("Unable to build live supervisor snapshot: {error}"))
                        }
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
                        "abort_worker_turn" => {
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
        if verification_config.enabled
            && !backend_activity_seen
            && let Some(text) = run_optional_command_text(&backend_command, root)
        {
            let sample = format!("\n--- final sample ---\n{text}");
            append_file(&logs_dir.join("backend-status.txt"), sample.as_bytes())?;
            backend_activity_seen |= backend_activity_observed(Some(&text), selection);
        }

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
            notes.push("Backend command showed the configured worker model active.".to_string());
        } else {
            notes.push(
                "Backend command did not show the configured worker model active.".to_string(),
            );
        }
        if before_gpu.is_none() && verification_config.enabled {
            notes.push(format!(
                "GPU verification command `{}` was unavailable or failed.",
                verification_config.gpu_command
            ));
        }
        if heartbeat_count == 0 && verification_config.enabled {
            notes.push(format!(
                "No heartbeat backend sample was written before OpenCode exited; final probe used `{}`.",
                backend_command
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
                "backend": "logs/backend-status.txt",
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
