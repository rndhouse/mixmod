use super::super::*;

#[test]
fn feedback_reject_is_normalized_to_revise() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "verdict": "reject",
        "hint": "No patch was captured.",
        "focus_files": ["checkout.py"],
        "required_checks": ["python -m unittest -q"]
    }));

    assert_eq!(verdict, SupervisorVerdict::Revise);
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

    assert_eq!(verdict, SupervisorVerdict::Stop);
    assert_eq!(get_str(&feedback, "verdict"), Some("stop"));
    assert_eq!(get_str(&feedback, "action"), Some("stop"));
}

#[test]
fn feedback_takeover_is_preserved_as_supervisor_takeover() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "take_over",
        "takeover_reason": "Remaining work is localized edge-case repair.",
        "direct_plan": ["Add focused tests", "Fix one runtime branch"]
    }));

    assert_eq!(verdict, SupervisorVerdict::TakeOver);
    assert_eq!(get_str(&feedback, "verdict"), Some("take_over"));
    assert_eq!(get_str(&feedback, "action"), Some("take_over"));
}
