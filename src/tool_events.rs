//! Structured worker tool-event extraction.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;

use crate::get_str;

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

/// Extract OpenCode full-output file paths referenced by tool events.
pub(crate) fn tool_output_paths_from_events(tool_events_jsonl: &str) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for line in tool_events_jsonl.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        collect_output_paths(&event, &mut paths);
    }
    paths.into_iter().map(PathBuf::from).collect()
}

fn collect_output_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "outputPath"
                    && let Some(path) = value.as_str().filter(|path| !path.is_empty())
                {
                    paths.insert(path.to_string());
                    continue;
                }
                collect_output_paths(value, paths);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_output_paths(value, paths);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn output_paths_extracts_unique_nested_paths() {
        let events = r#"
{"type":"tool_use","part":{"state":{"outputPath":"/tmp/opencode/tool-output/a.txt","metadata":{"outputPath":"/tmp/opencode/tool-output/a.txt"}}}}
{"type":"tool_use","part":{"state":{"metadata":{"outputPath":"/tmp/opencode/tool-output/b.txt"}}}}
"#;

        let paths = tool_output_paths_from_events(events);

        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/opencode/tool-output/a.txt"),
                PathBuf::from("/tmp/opencode/tool-output/b.txt")
            ]
        );
    }
}
