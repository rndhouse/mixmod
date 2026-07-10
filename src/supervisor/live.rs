use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde_json::{Value, json};

use crate::*;

use super::codex::SupervisorCodexSession;
use super::normalize::parse_feedback_json;
use super::prompts::supervisor_live_control_prompt;
use super::types::SupervisorUsageSample;

pub(crate) struct LiveSupervisorAdvisor {
    work_dir: PathBuf,
    artifact_dir: PathBuf,
    feedback_path: PathBuf,
    supervisor_session: Arc<Mutex<SupervisorCodexSession>>,
    worker_guidance: WorkerSupervisorGuidance,
    config: LiveSupervisionConfig,
    state: Mutex<LiveSupervisorAdvisorState>,
}

#[derive(Default)]
struct LiveSupervisorAdvisorState {
    check_count: u64,
    last_check_elapsed_ms: Option<u64>,
    usage_samples: Vec<SupervisorUsageSample>,
}

impl LiveSupervisorAdvisor {
    pub(crate) fn new(
        work_dir: &Path,
        artifact_dir: &Path,
        feedback_path: &Path,
        supervisor_session: Arc<Mutex<SupervisorCodexSession>>,
        worker_guidance: WorkerSupervisorGuidance,
        config: LiveSupervisionConfig,
    ) -> Self {
        Self {
            work_dir: work_dir.to_path_buf(),
            artifact_dir: artifact_dir.to_path_buf(),
            feedback_path: feedback_path.to_path_buf(),
            supervisor_session,
            worker_guidance,
            config,
            state: Mutex::new(LiveSupervisorAdvisorState::default()),
        }
    }

    pub(crate) fn drain_usage_samples(&self) -> Vec<SupervisorUsageSample> {
        std::mem::take(
            &mut self
                .state
                .lock()
                .expect("live supervisor advisor state lock poisoned")
                .usage_samples,
        )
    }

    fn reserve_check(&self, snapshot: &LiveWorkerSnapshot) -> Option<u64> {
        if !live_supervision_snapshot_should_check(snapshot, &self.config) {
            return None;
        }
        let mut state = self
            .state
            .lock()
            .expect("live supervisor advisor state lock poisoned");
        if state.check_count >= self.config.max_checks_per_worker {
            return None;
        }
        if let Some(last_check) = state.last_check_elapsed_ms {
            let elapsed_since_check = snapshot.elapsed_ms.saturating_sub(last_check);
            if elapsed_since_check < self.config.check_interval_seconds.saturating_mul(1000) {
                return None;
            }
        }
        state.check_count += 1;
        state.last_check_elapsed_ms = Some(snapshot.elapsed_ms);
        Some(state.check_count)
    }
}

impl SupervisorAdvisor for LiveSupervisorAdvisor {
    fn advise(&self, snapshot: &LiveWorkerSnapshot) -> Result<Option<Value>> {
        let Some(check_index) = self.reserve_check(snapshot) else {
            return Ok(None);
        };
        let label = format!("live-control-{check_index}");
        let mut bounded_snapshot = snapshot_for_live_supervisor_prompt(snapshot);
        bounded_snapshot.live_control_check_index = check_index;
        bounded_snapshot.live_control_check_limit = self.config.max_checks_per_worker;
        bounded_snapshot.stdout_tail =
            truncate_for_report(&bounded_snapshot.stdout_tail, self.config.stdout_tail_bytes);
        bounded_snapshot.stderr_tail =
            truncate_for_report(&bounded_snapshot.stderr_tail, self.config.stderr_tail_bytes);
        let prompt = supervisor_live_control_prompt(
            &self.work_dir,
            &bounded_snapshot,
            &self.worker_guidance,
        )?;
        let result = self
            .supervisor_session
            .lock()
            .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?
            .run_turn(&self.artifact_dir, &label, &prompt)?;
        let raw_parsed = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
            json!({
                "action": "wait",
                "risk": "live supervisor response was not parseable JSON"
            })
        });
        let mut parsed = raw_parsed.clone();
        let action = normalize_supervisor_control_action(
            get_str(&parsed, "action").or_else(|| get_str(&parsed, "verdict")),
        );
        let worker_mode =
            normalize_supervisor_control_worker_mode(&action, get_str(&parsed, "worker_mode"));
        let raw_message_to_worker = get_str(&parsed, "message_to_worker")
            .or_else(|| get_str(&parsed, "message"))
            .or_else(|| get_str(&parsed, "hint"))
            .unwrap_or("")
            .trim()
            .to_string();
        let focus_files = normalize_live_control_focus_files(
            &self.work_dir,
            get_string_array(&parsed, "focus_files"),
        );
        let message_to_worker = sanitize_live_control_message(&raw_message_to_worker, &focus_files);
        let required_checks = get_string_array(&parsed, "required_checks");
        let worker_turn_shape = get_str(&parsed, "worker_turn_shape").map(ToOwned::to_owned);
        let turn_goal = get_str(&parsed, "turn_goal").map(ToOwned::to_owned);
        let exact_edits = get_string_array(&parsed, "exact_edits");
        let edit_plan = get_string_array(&parsed, "edit_plan");
        let deferred_checks = get_string_array(&parsed, "deferred_checks");
        let defer_checks_until_patch_exists = get_bool(&parsed, "defer_checks_until_patch_exists");
        let completion_gate = get_str(&parsed, "completion_gate").map(ToOwned::to_owned);
        let forbidden_actions = get_string_array(&parsed, "forbidden_actions");
        if let Some(object) = parsed.as_object_mut() {
            object.insert("action".to_string(), json!(action.clone()));
            object.insert("worker_mode".to_string(), json!(worker_mode.clone()));
            object.insert(
                "message_to_worker".to_string(),
                json!(message_to_worker.clone()),
            );
            object.insert("focus_files".to_string(), json!(focus_files.clone()));
        }
        let risk = get_str(&parsed, "risk").unwrap_or("").trim().to_string();
        let feedback_record = json!({
            "label": label,
            "timestamp": Utc::now().to_rfc3339(),
            "live_control": parsed,
            "raw_live_control": raw_parsed,
            "snapshot": {
                "elapsed_ms": bounded_snapshot.elapsed_ms,
                "segment_elapsed_ms": bounded_snapshot.segment_elapsed_ms,
                "stdout_bytes": bounded_snapshot.stdout_bytes,
                "stderr_bytes": bounded_snapshot.stderr_bytes,
                "last_output_age_ms": bounded_snapshot.last_output_age_ms,
                "new_delta_bytes": bounded_snapshot.new_delta_bytes,
                "new_delta_files": bounded_snapshot.new_delta_files,
                "context_overflow_count": bounded_snapshot.context_overflow_count,
                "worker_session_token_peak": bounded_snapshot.worker_session_token_peak,
                "repeated_read_signature": bounded_snapshot.repeated_read_signature,
                "repeated_read_count": bounded_snapshot.repeated_read_count,
                "live_control_check_index": bounded_snapshot.live_control_check_index,
                "live_control_check_limit": bounded_snapshot.live_control_check_limit,
                "recent_tool_events": bounded_snapshot.recent_tool_events,
            },
            "codex_exit_status": result.exit_status,
            "supervisor_model": result.model.clone(),
            "supervisor_reasoning_effort": result.reasoning_effort.clone(),
            "supervisor_input_tokens": result.usage.input_tokens,
            "supervisor_output_tokens": result.usage.output_tokens,
            "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
            "supervisor_total_tokens": result.usage.total_tokens,
            "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
            "input_bytes": result.input_bytes,
            "output_bytes": result.output_bytes,
            "auth_copied_then_removed": result.auth_copied_then_removed,
            "codex_app_server_thread_id": result.thread_id.clone(),
            "codex_app_server_turn_id": result.turn_id.clone(),
        });
        append_jsonl(&self.feedback_path, &feedback_record)?;
        self.state
            .lock()
            .expect("live supervisor advisor state lock poisoned")
            .usage_samples
            .push(SupervisorUsageSample {
                input_tokens: result.usage.input_tokens,
                output_tokens: result.usage.output_tokens,
                reasoning_tokens: result.usage.reasoning_tokens,
                total_tokens: result.usage.total_tokens,
                cached_input_tokens: result.usage.cached_input_tokens,
                input_bytes: result.input_bytes,
                output_bytes: result.output_bytes,
                thread_id: result.thread_id,
                turn_id: result.turn_id,
            });

        if action == "wait" {
            return Ok(None);
        }

        let control = SupervisorControlCommand {
            timestamp: Utc::now().to_rfc3339(),
            action,
            worker_mode,
            message_to_worker,
            focus_files,
            required_checks,
            risk,
            source: "codex_live_supervisor".to_string(),
            worker_turn_shape,
            turn_goal,
            exact_edits,
            edit_plan,
            deferred_checks,
            defer_checks_until_patch_exists,
            completion_gate,
            forbidden_actions,
        };
        serde_json::to_value(control)
            .map(Some)
            .context("failed to serialize live supervisor control")
    }
}

pub(super) fn snapshot_for_live_supervisor_prompt(
    snapshot: &LiveWorkerSnapshot,
) -> LiveWorkerSnapshot {
    let mut snapshot = snapshot.clone();
    snapshot.out_dir = "[redacted: Mixmod artifact directory]".to_string();
    snapshot.task_path = "[redacted: Mixmod worker task artifact]".to_string();
    snapshot
}

pub(super) fn normalize_live_control_focus_files(
    root: &Path,
    focus_files: Vec<String>,
) -> Vec<String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for focus_file in focus_files {
        let Some(path) = normalize_live_control_focus_file(&root, &focus_file) else {
            continue;
        };
        if seen.insert(path.clone()) {
            normalized.push(path);
        }
    }
    normalized
}

fn normalize_live_control_focus_file(root: &Path, focus_file: &str) -> Option<String> {
    let trimmed = focus_file.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(trimmed);
    let relative = if path.is_absolute() {
        let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        absolute.strip_prefix(root).ok()?.to_path_buf()
    } else {
        repo_relative_path(path)?
    };
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative.to_string_lossy().replace('\\', "/"))
}

fn repo_relative_path(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(relative)
}

pub(super) fn sanitize_live_control_message(message: &str, focus_files: &[String]) -> String {
    let message = message.trim();
    if message.is_empty() || live_control_message_mentions_mixmod_artifact(message) {
        return fallback_live_control_message(focus_files);
    }
    message.to_string()
}

fn live_control_message_mentions_mixmod_artifact(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "worker-task.json",
        "revision-task.json",
        "/tmp/mixmod",
        "/tmp/mixmod-state",
        "mixmod-state",
        "opencode-instructions.md",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn fallback_live_control_message(focus_files: &[String]) -> String {
    if focus_files.is_empty() {
        "Continue from the current repo state. Make one focused source edit for the requested behavior."
            .to_string()
    } else {
        let mut displayed_files = focus_files.iter().take(3).cloned().collect::<Vec<_>>();
        if focus_files.len() > displayed_files.len() {
            displayed_files.push("...".to_string());
        }
        format!(
            "Focus on {}. Make one focused source edit for the requested behavior.",
            displayed_files.join(", ")
        )
    }
}

pub(super) fn live_supervision_snapshot_should_check(
    snapshot: &LiveWorkerSnapshot,
    config: &LiveSupervisionConfig,
) -> bool {
    if !config.enabled || config.max_checks_per_worker == 0 {
        return false;
    }
    if snapshot.elapsed_ms < config.min_elapsed_seconds.saturating_mul(1000) {
        return false;
    }
    if snapshot.context_overflow_count > 0 {
        return true;
    }
    let stale_with_output = snapshot.last_output_age_ms
        >= config.stale_after_seconds.saturating_mul(1000)
        && snapshot.stdout_bytes > 0;
    if snapshot.repeated_read_count >= config.repeated_read_threshold
        && (snapshot.new_delta_bytes == 0 || stale_with_output)
    {
        return true;
    }
    stale_with_output
}
