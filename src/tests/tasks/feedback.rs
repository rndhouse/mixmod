use super::super::*;

#[test]
fn feedback_reject_is_normalized_to_revise() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "verdict": "reject",
        "hint": "No patch was captured.",
        "focus_files": ["checkout.py"],
        "required_checks": ["python -m unittest -q"]
    }));

    assert_eq!(verdict, "revise");
    assert_eq!(get_str(&feedback, "verdict"), Some("revise"));
    assert_eq!(get_str(&feedback, "raw_verdict"), Some("reject"));
}

#[test]
fn feedback_stop_is_preserved_as_codex_stop() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "stop",
        "message_to_worker": "No further local attempts.",
        "focus_files": [],
        "required_checks": []
    }));

    assert_eq!(verdict, "stop");
    assert_eq!(get_str(&feedback, "verdict"), Some("stop"));
    assert_eq!(get_str(&feedback, "action"), Some("stop"));
}
