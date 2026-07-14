pub(super) fn hard_rule_from_forbidden_action(action: &str) -> String {
    let action = action.trim().trim_end_matches('.');
    if action.is_empty() {
        return "Do not ask questions.".to_string();
    }
    if action.to_ascii_lowercase().starts_with("do not ") {
        format!("{action}.")
    } else {
        format!("Do not {action}.")
    }
}

pub(super) fn append_handoff_list(lines: &mut Vec<String>, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{label}:"));
    lines.extend(items.iter().map(|item| format!("- {item}")));
}

pub(super) fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn file_list_or_none(files: &[String]) -> String {
    if files.is_empty() {
        "- none specified".to_string()
    } else {
        bullet_list(files)
    }
}

pub(super) fn numbered_list(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn optional_bullet_section(heading: &str, items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("\n{heading}:\n{}\n", bullet_list(items))
    }
}

pub(super) fn optional_numbered_section(heading: &str, items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("\n{heading}:\n{}\n", numbered_list(items))
    }
}

pub(super) fn optional_text_section(heading: &str, body: Option<&str>) -> String {
    body.map(|body| format!("\n{heading}:\n{body}\n"))
        .unwrap_or_default()
}

pub(super) fn non_empty_or<T>(value: Vec<T>, fallback: Vec<T>) -> Vec<T> {
    if value.is_empty() { fallback } else { value }
}
