use serde_json::Value;

#[cfg(test)]
use crate::get_str;
use crate::get_u64;

/// Token usage reported by Codex.
#[derive(Clone, Default)]
pub(crate) struct CodexUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
}

impl CodexUsage {
    /// Return this cumulative usage with the previous cumulative reading
    /// subtracted, saturating if Codex reports a reset.
    pub(crate) fn delta_since(&self, previous: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_sub(previous.cached_input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            reasoning_tokens: self
                .reasoning_tokens
                .saturating_sub(previous.reasoning_tokens),
            total_tokens: self.total_tokens.saturating_sub(previous.total_tokens),
        }
    }
}

fn codex_usage_from_breakdown(value: &Value) -> CodexUsage {
    CodexUsage {
        input_tokens: get_u64(value, "inputTokens").unwrap_or(0),
        cached_input_tokens: get_u64(value, "cachedInputTokens").unwrap_or(0),
        output_tokens: get_u64(value, "outputTokens").unwrap_or(0),
        reasoning_tokens: get_u64(value, "reasoningOutputTokens").unwrap_or(0),
        total_tokens: get_u64(value, "totalTokens").unwrap_or_else(|| {
            get_u64(value, "inputTokens").unwrap_or(0)
                + get_u64(value, "outputTokens").unwrap_or(0)
                + get_u64(value, "reasoningOutputTokens").unwrap_or(0)
        }),
    }
}

pub(crate) fn codex_app_server_cumulative_usage(token_usage: &Value) -> Option<CodexUsage> {
    token_usage.get("total").map(codex_usage_from_breakdown)
}

pub(super) fn codex_app_server_last_request_usage(token_usage: &Value) -> Option<CodexUsage> {
    token_usage.get("last").map(codex_usage_from_breakdown)
}

#[cfg(test)]
pub(crate) fn codex_usage_from_jsonl(bytes: &[u8]) -> CodexUsage {
    let mut usage = CodexUsage::default();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let Some(next) = codex_usage_from_json_value(&value) {
            usage = next;
        }
    }
    usage
}

#[cfg(test)]
fn codex_usage_from_json_value(value: &Value) -> Option<CodexUsage> {
    let payload = value.get("payload");
    if get_str(value, "type") == Some("event_msg")
        && payload.and_then(|payload| get_str(payload, "type")) == Some("token_count")
    {
        return payload
            .and_then(|payload| payload.get("info"))
            .and_then(|info| info.get("total_token_usage"))
            .map(codex_usage_from_snake_breakdown);
    }
    if get_str(value, "type") == Some("token_count") {
        return value
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
            .map(codex_usage_from_snake_breakdown);
    }
    if get_str(value, "method") == Some("thread/tokenUsage/updated") {
        return value
            .get("params")
            .and_then(|params| params.get("tokenUsage"))
            .and_then(|token_usage| token_usage.get("total").or_else(|| token_usage.get("last")))
            .map(codex_usage_from_breakdown);
    }
    None
}

#[cfg(test)]
fn codex_usage_from_snake_breakdown(value: &Value) -> CodexUsage {
    CodexUsage {
        input_tokens: get_u64(value, "input_tokens").unwrap_or(0),
        cached_input_tokens: get_u64(value, "cached_input_tokens").unwrap_or(0),
        output_tokens: get_u64(value, "output_tokens").unwrap_or(0),
        reasoning_tokens: get_u64(value, "reasoning_output_tokens").unwrap_or(0),
        total_tokens: get_u64(value, "total_tokens").unwrap_or_else(|| {
            get_u64(value, "input_tokens").unwrap_or(0)
                + get_u64(value, "output_tokens").unwrap_or(0)
                + get_u64(value, "reasoning_output_tokens").unwrap_or(0)
        }),
    }
}
