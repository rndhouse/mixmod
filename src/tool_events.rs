//! Structured worker tool-event extraction.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::get_str;

/// Build a JSONL stream containing all structured OpenCode stdout events.
pub(crate) fn build_opencode_events_jsonl(stdout: &[u8]) -> Result<(String, u64)> {
    let mut trace = String::new();
    let mut count = 0_u64;
    let stdout = String::from_utf8_lossy(stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if !event.is_object() {
            continue;
        }
        trace.push_str(&serde_json::to_string(&event).context("failed to serialize event")?);
        trace.push('\n');
        count += 1;
    }
    Ok((trace, count))
}

/// Build a JSONL stream containing structured worker tool-call events.
pub(crate) fn build_tool_events_jsonl(stdout: &[u8]) -> Result<(String, u64)> {
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
        if get_str(&event, "type") != Some("tool_use") {
            continue;
        }
        trace.push_str(&serde_json::to_string(&event).context("failed to serialize tool event")?);
        trace.push('\n');
        count += 1;
    }
    Ok((trace, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_events_extracts_structured_json_lines() {
        let stdout = br#"
--- opencode segment 1: action=initial worker_mode=initial ---
plain output
{"type":"reasoning","part":{"text":"thinking"}}
{"type":"tool_use","part":{"tool":"bash","state":{"status":"completed","input":{"command":"pytest -q"},"output":"failed"}}}
not json {
[1,2,3]
"#;

        let (events, count) = build_opencode_events_jsonl(stdout).unwrap();

        assert_eq!(count, 2);
        assert!(events.contains("\"type\":\"reasoning\""));
        assert!(events.contains("\"type\":\"tool_use\""));
        assert!(events.contains("\"command\":\"pytest -q\""));
        assert!(!events.contains("opencode segment"));
        assert!(!events.contains("plain output"));
        assert!(!events.contains("[1,2,3]"));
    }

    #[test]
    fn tool_events_extracts_structured_tool_calls() {
        let stdout = br#"
plain output
{"type":"reasoning","part":{"text":"thinking"}}
{"type":"tool_use","part":{"tool":"bash","state":{"status":"completed","input":{"command":"pytest -q"},"output":"failed","metadata":{"exit":1}}}}
{"type":"tool_use","part":{"tool":"read","state":{"input":{"filePath":"src/lib.rs"}}}}
"#;

        let (events, count) = build_tool_events_jsonl(stdout).unwrap();

        assert_eq!(count, 2);
        assert!(events.contains("\"tool\":\"bash\""));
        assert!(events.contains("\"command\":\"pytest -q\""));
        assert!(events.contains("\"exit\":1"));
        assert!(events.contains("\"tool\":\"read\""));
        assert!(!events.contains("plain output"));
        assert!(!events.contains("\"type\":\"reasoning\""));
    }
}
