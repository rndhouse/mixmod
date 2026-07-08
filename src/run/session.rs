use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::get_str;
use crate::harness::AgentOutput;

pub(super) fn build_session_jsonl(
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
    output: &AgentOutput,
) -> Result<String> {
    let events = [
        json!({
            "event": "started",
            "timestamp": start.to_rfc3339(),
            "command": output.command_for_metrics,
            "session_label": output.session_label,
            "session_id": output.session_id,
            "resume_session_id": output.resume_session_id,
            "worker_session_reused": output.session_reused,
            "interrupted_by_supervisor": output.interrupted_by_supervisor,
            "supervisor_control_action": output.supervisor_control_action,
            "opencode_segments": output.segments.clone(),
        }),
        json!({
            "event": "opencode_stdout",
            "timestamp": end.to_rfc3339(),
            "bytes": output.stdout.len(),
            "text": String::from_utf8_lossy(&output.stdout),
        }),
        json!({
            "event": "opencode_stderr",
            "timestamp": end.to_rfc3339(),
            "bytes": output.stderr.len(),
            "text": String::from_utf8_lossy(&output.stderr),
        }),
        json!({
            "event": "finished",
            "timestamp": end.to_rfc3339(),
            "exit_status": output.exit_status,
            "success": output.success,
            "timed_out": output.timed_out,
            "idle_timed_out": output.idle_timed_out,
            "heartbeat_count": output.heartbeat_count,
            "interrupted_by_supervisor": output.interrupted_by_supervisor,
            "supervisor_control_action": output.supervisor_control_action,
        }),
    ];
    let mut session = String::new();
    for event in events {
        session.push_str(
            &serde_json::to_string(&event).context("failed to serialize session JSONL event")?,
        );
        session.push('\n');
    }
    Ok(session)
}

pub(super) fn build_reasoning_trace_jsonl(stdout: &[u8]) -> Result<(String, u64)> {
    let mut trace = String::new();
    let mut count = 0_u64;
    let stdout = String::from_utf8_lossy(stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if get_str(&event, "type") != Some("reasoning") {
            continue;
        }
        trace.push_str(
            &serde_json::to_string(&event).context("failed to serialize reasoning trace event")?,
        );
        trace.push('\n');
        count += 1;
    }
    Ok((trace, count))
}
