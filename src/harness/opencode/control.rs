use std::fs;
use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use serde_json::{Value, json};

use crate::harness::AgentRequest;
use crate::{
    LIVE_STATUS_FILE, SUPERVISOR_CONTROL_FILE, SUPERVISOR_CONTROL_LOG, normalize_worker_mode,
};

pub(super) struct LiveStatusSnapshot<'a> {
    pub(super) request: &'a AgentRequest,
    pub(super) out_dir: &'a Path,
    pub(super) start: Instant,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    pub(super) last_output_age_ms: u64,
    pub(super) gpu_activity_observed: bool,
    pub(super) backend_activity_observed: bool,
    pub(super) status: &'a str,
}

pub(super) fn live_status_json(snapshot: LiveStatusSnapshot<'_>) -> Value {
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

pub(super) fn read_supervisor_control(path: &Path) -> Option<Value> {
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
        "abort_worker_turn" | "abort" | "stop_worker" | "stop" | "halt" => {
            "abort_worker_turn".to_string()
        }
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
