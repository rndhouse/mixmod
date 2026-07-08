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

pub(super) fn numbered_list(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn non_empty_or<T>(value: Vec<T>, fallback: Vec<T>) -> Vec<T> {
    if value.is_empty() { fallback } else { value }
}

pub(super) fn immediate_small_patch_exact_edits(
    all_exact_edits: &[String],
    turn_goal: &str,
) -> Vec<String> {
    all_exact_edits
        .iter()
        .find(|edit| !edit.trim().is_empty())
        .cloned()
        .or_else(|| {
            let turn_goal = turn_goal.trim();
            (!turn_goal.is_empty()).then(|| turn_goal.to_string())
        })
        .into_iter()
        .collect()
}
