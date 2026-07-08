use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkerContextSignals {
    pub(crate) context_overflow_count: u64,
    pub(crate) context_overflow_last_message: Option<String>,
}

pub(crate) fn worker_context_signals(stdout: &[u8]) -> WorkerContextSignals {
    let mut signals = WorkerContextSignals::default();
    let stdout = String::from_utf8_lossy(stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !is_context_overflow_line(trimmed) {
            continue;
        }
        signals.context_overflow_count += 1;
        signals.context_overflow_last_message = Some(extract_context_overflow_message(trimmed));
    }
    signals
}

pub(crate) fn worker_session_token_peak(stdout: &[u8]) -> Option<u64> {
    let mut peak = None;
    let stdout = String::from_utf8_lossy(stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(total) = event.pointer("/part/tokens/total").and_then(Value::as_u64) else {
            continue;
        };
        peak = Some(peak.map_or(total, |current: u64| current.max(total)));
    }
    peak
}

fn is_context_overflow_line(line: &str) -> bool {
    line.contains("ContextOverflowError") || line.contains("exceeds the available context size")
}

fn extract_context_overflow_message(line: &str) -> String {
    if let Ok(event) = serde_json::from_str::<Value>(line) {
        let name = event
            .pointer("/error/name")
            .and_then(Value::as_str)
            .unwrap_or("ContextOverflowError");
        let message = event
            .pointer("/error/data/message")
            .and_then(Value::as_str)
            .unwrap_or("context overflow");
        return truncate_context_overflow_message(&format!("{name}: {message}"));
    }
    truncate_context_overflow_message(line)
}

fn truncate_context_overflow_message(value: &str) -> String {
    const LIMIT: usize = 500;
    if value.chars().count() <= LIMIT {
        return value.to_string();
    }
    let mut truncated = value.chars().take(LIMIT).collect::<String>();
    truncated.push_str("...");
    truncated
}
