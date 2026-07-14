use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkerContextSignals {
    pub(crate) context_overflow_count: u64,
    pub(crate) context_overflow_last_message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorkerTokenUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) cache_write_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) reported_cost_usd: f64,
    pub(crate) step_count: u64,
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

pub(crate) fn worker_token_usage(stdout: &[u8]) -> WorkerTokenUsage {
    let mut usage = WorkerTokenUsage::default();
    let stdout = String::from_utf8_lossy(stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(tokens) = event.pointer("/part/tokens") else {
            continue;
        };
        usage.input_tokens += tokens.get("input").and_then(Value::as_u64).unwrap_or(0);
        usage.cached_input_tokens += tokens
            .pointer("/cache/read")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        usage.cache_write_tokens += tokens
            .pointer("/cache/write")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        usage.output_tokens += tokens.get("output").and_then(Value::as_u64).unwrap_or(0);
        usage.reasoning_tokens += tokens.get("reasoning").and_then(Value::as_u64).unwrap_or(0);
        usage.total_tokens += tokens.get("total").and_then(Value::as_u64).unwrap_or(0);
        usage.reported_cost_usd += event
            .pointer("/part/cost")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        usage.step_count += 1;
    }
    usage
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
