use serde_json::{Value, json};

use crate::get_str;

pub(super) fn parse_feedback_json(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        return Some(normalize_supervisor_json_value(value));
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end])
        .ok()
        .map(normalize_supervisor_json_value)
}

fn normalize_supervisor_json_value(mut value: Value) -> Value {
    for key in ["exact_edits", "edit_plan", "implementation_steps"] {
        normalize_mixed_instruction_array(&mut value, key);
    }
    value
}

fn normalize_mixed_instruction_array(value: &mut Value, key: &str) {
    let Some(field) = value.get_mut(key) else {
        return;
    };
    let normalized = match &*field {
        Value::Array(items) => items
            .iter()
            .filter_map(mixed_instruction_item_to_string)
            .collect::<Vec<_>>(),
        other => mixed_instruction_item_to_string(other)
            .into_iter()
            .collect(),
    };
    if !normalized.is_empty() {
        *field = json!(normalized);
    }
}

fn mixed_instruction_item_to_string(value: &Value) -> Option<String> {
    if let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }

    let object = value.as_object()?;
    let instruction = first_object_string(
        object,
        &["instruction", "edit", "description", "message", "action"],
    );
    let file = first_object_string(object, &["file", "path", "target_file"]);
    let symbol = first_object_string(object, &["symbol", "function", "method", "target"]);

    match (file, symbol, instruction) {
        (Some(file), Some(symbol), Some(instruction)) => Some(format!(
            "In {file}, update {symbol}: {instruction}",
            file = file.trim(),
            symbol = symbol.trim(),
            instruction = instruction.trim()
        )),
        (Some(file), None, Some(instruction)) => Some(format!(
            "In {file}: {instruction}",
            file = file.trim(),
            instruction = instruction.trim()
        )),
        (None, Some(symbol), Some(instruction)) => Some(format!(
            "Update {symbol}: {instruction}",
            symbol = symbol.trim(),
            instruction = instruction.trim()
        )),
        (None, None, Some(instruction)) => Some(instruction.trim().to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}

fn first_object_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
}

pub(crate) fn normalize_feedback_value(mut value: Value) -> (Value, String) {
    let raw = get_str(&value, "verdict")
        .or_else(|| get_str(&value, "action"))
        .unwrap_or("revise")
        .to_string();
    let verdict = normalize_supervisor_verdict(&raw);
    if let Value::Object(map) = &mut value {
        if raw != verdict {
            map.insert("raw_verdict".to_string(), json!(raw));
        }
        map.insert("verdict".to_string(), json!(verdict.clone()));
        map.insert("action".to_string(), json!(verdict.clone()));
    }
    (value, verdict)
}

pub(super) fn normalize_supervisor_verdict(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => "approve".to_string(),
        "stop" | "stopped" | "halt" | "done" | "needs_user" | "needs-user" => "stop".to_string(),
        "revise" | "revision" | "needs_revision" | "needs-review" | "needs_review" | "reject"
        | "rejected" => "revise".to_string(),
        _ => "revise".to_string(),
    }
}

pub(super) fn normalize_patch_decision(value: Option<&str>) -> String {
    match value
        .unwrap_or("accept_current")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "revise_previous" | "previous" | "keep_previous" | "restore_previous"
        | "recover_previous" => "revise_previous".to_string(),
        "revise_current" | "current_revision" | "continue_current" => "revise_current".to_string(),
        _ => "accept_current".to_string(),
    }
}

pub(crate) fn normalize_worker_mode(value: Option<&str>) -> String {
    match value
        .unwrap_or("continue")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "context_focus" | "focused" | "focus" | "fresh" | "reset" => "context_focus".to_string(),
        _ => "continue".to_string(),
    }
}
