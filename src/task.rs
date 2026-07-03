//! Task loading and benchmark-hygiene boundaries.
//!
//! `FullBenchmarkTask` data can contain evaluator-only fields such as hidden
//! test selectors, gold patches, or SWE-bench selection metadata. Model-facing
//! code must use [`agent_visible_task_value`] or [`write_agent_visible_task_file`]
//! so those fields cannot leak into prompts or worktree files.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{make_run_id, state_layout, write_pretty_json};

/// Normalized task shape used by Mixmod run and experiment code.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct TaskSpec {
    pub title: String,
    pub instructions: String,
    pub expect_patch: Option<bool>,
    pub files: Vec<String>,
    pub tests: Vec<String>,
    pub constraints: Vec<String>,
    pub acceptance: Vec<String>,
    pub context: Value,
}

impl Default for TaskSpec {
    fn default() -> Self {
        Self {
            title: String::new(),
            instructions: String::new(),
            expect_patch: None,
            files: Vec::new(),
            tests: Vec::new(),
            constraints: Vec::new(),
            acceptance: Vec::new(),
            context: Value::Null,
        }
    }
}

/// Read a task JSON file and return both its raw JSON and normalized struct.
pub(crate) fn read_task_json(path: &Path) -> Result<(Value, TaskSpec)> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read task JSON {}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse task JSON {}", path.display()))?;
    let mut spec: TaskSpec = serde_json::from_value(value.clone())
        .with_context(|| format!("failed to deserialize task JSON {}", path.display()))?;
    if spec.instructions.trim().is_empty() {
        spec.instructions = value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    }
    if spec.instructions.trim().is_empty() {
        spec.instructions = serde_json::to_string_pretty(&value)
            .with_context(|| format!("failed to render fallback task JSON {}", path.display()))?;
    }
    if spec.title.trim().is_empty() {
        spec.title = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("mixmod-task")
            .to_string();
    }
    Ok((value, spec))
}

/// Write a managed task JSON file for a natural-language prompt.
pub(crate) fn write_prompt_task_file(root: &Path, prompt: &str) -> Result<PathBuf> {
    let trimmed = prompt.trim();
    let path = state_layout(root)
        .tasks()
        .join(format!("{}.json", make_run_id("task")));
    let task = json!({
        "title": prompt_title(trimmed),
        "instructions": trimmed
    });
    write_pretty_json(&path, &task, "prompt task")?;
    Ok(path)
}

/// Copy `source` to `target`, stripping evaluator-only metadata first.
pub(crate) fn write_agent_visible_task_file(source: &Path, target: &Path) -> Result<()> {
    let task = read_json_value(source)?;
    let visible_task = agent_visible_task_value(&task);
    write_pretty_json(target, &visible_task, "agent-visible task")
}

/// Ensure an existing task file is safe for an agent to read.
pub(crate) fn ensure_agent_visible_task_file(path: &Path) -> Result<()> {
    let task = read_json_value(path)?;
    let visible_task = agent_visible_task_value(&task);
    if visible_task != task {
        write_pretty_json(path, &visible_task, "agent-visible task")?;
    }
    Ok(())
}

/// Return the subset of a task that may be shown to an agent.
pub(crate) fn agent_visible_task_value(task: &Value) -> Value {
    let Some(object) = task.as_object() else {
        return task.clone();
    };

    let mut visible = serde_json::Map::new();
    for (key, value) in object {
        if is_evaluation_only_task_key(key) {
            continue;
        }
        if key == "context" {
            if let Some(context) = agent_visible_context_value(value) {
                visible.insert(key.clone(), context);
            }
            continue;
        }
        visible.insert(key.clone(), value.clone());
    }
    Value::Object(visible)
}

/// Render a task as the human-readable experiment task markdown.
pub(crate) fn task_markdown_from_json(value: &Value) -> String {
    let title = get_str(value, "title").unwrap_or("Mixmod task");
    let instructions = get_str(value, "instructions").unwrap_or("");
    let files = markdown_list(&get_string_array(value, "files"));
    let tests = markdown_list(&get_string_array(value, "tests"));
    let acceptance = markdown_list(&get_string_array(value, "acceptance"));
    format!(
        r#"# {title}

## Task

{instructions}

## Relevant files

{files}

## Acceptance

{acceptance}

## Tests

{tests}
"#
    )
}

fn read_json_value(path: &Path) -> Result<Value> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read JSON file {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse JSON file {}", path.display()))
}

fn agent_visible_context_value(context: &Value) -> Option<Value> {
    let object = context.as_object()?;
    let mut visible = serde_json::Map::new();
    for (key, value) in object {
        if is_agent_visible_context_key(key) && !is_evaluation_only_task_key(key) {
            visible.insert(key.clone(), value.clone());
        }
    }
    if visible.is_empty() {
        None
    } else {
        Some(Value::Object(visible))
    }
}

fn is_agent_visible_context_key(key: &str) -> bool {
    matches!(
        key,
        "benchmark" | "dataset" | "split" | "instance_id" | "repo" | "base_commit" | "version"
    )
}

fn is_evaluation_only_task_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "fail_to_pass"
            | "pass_to_pass"
            | "fail_to_fail"
            | "pass_to_fail"
            | "patch"
            | "test_patch"
            | "gold_patch"
            | "gold_test_patch"
            | "hints_text"
            | "candidate_pool"
            | "selection_rule"
            | "environment_setup_commit"
            | "gold_patch_bytes_for_ordering_only"
            | "gold_test_patch_bytes_for_ordering_only"
    )
}

fn markdown_list(items: &[String]) -> String {
    if items.is_empty() {
        "- TBD".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn prompt_title(prompt: &str) -> String {
    let first_line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Mixmod task")
        .trim();
    let title = first_line.chars().take(80).collect::<String>();
    if title.is_empty() {
        "Mixmod task".to_string()
    } else {
        title
    }
}

fn get_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn get_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
