use super::super::*;

#[test]
fn feedback_reject_is_normalized_to_worker_edit() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "verdict": "reject",
        "hint": "No patch was captured.",
        "focus_files": ["checkout.py"],
        "required_checks": ["python -m unittest -q"]
    }));

    assert_eq!(verdict, SupervisorVerdict::WorkerEdit);
    assert_eq!(get_str(&feedback, "verdict"), Some("worker_edit"));
    assert_eq!(get_str(&feedback, "action"), Some("worker_edit"));
    assert_eq!(get_bool(&feedback, "expect_patch"), Some(true));
    assert_eq!(get_str(&feedback, "raw_verdict"), Some("reject"));
}

#[test]
fn feedback_worker_inspect_defaults_to_no_patch() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "worker_inspect",
        "message_to_worker": "Inspect the current parser path and propose the next edit."
    }));

    assert_eq!(verdict, SupervisorVerdict::WorkerInspect);
    assert_eq!(get_str(&feedback, "verdict"), Some("worker_inspect"));
    assert_eq!(get_str(&feedback, "action"), Some("worker_inspect"));
    assert_eq!(get_bool(&feedback, "expect_patch"), Some(false));
}

#[test]
fn feedback_legacy_no_patch_revise_normalizes_to_worker_inspect() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "revise",
        "expect_patch": false,
        "worker_turn_shape": "planning_probe",
        "message_to_worker": "Inspect only."
    }));

    assert_eq!(verdict, SupervisorVerdict::WorkerInspect);
    assert_eq!(get_str(&feedback, "verdict"), Some("worker_inspect"));
    assert_eq!(get_str(&feedback, "action"), Some("worker_inspect"));
    assert_eq!(get_str(&feedback, "raw_verdict"), Some("revise"));
    assert_eq!(get_bool(&feedback, "expect_patch"), Some(false));
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
fn feedback_supervisor_direct_edit_is_preserved_as_supervisor_direct_edit() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "supervisor_direct_edit",
        "supervisor_direct_edit_reason": "Remaining work is localized edge-case repair.",
        "direct_plan": ["Add focused tests", "Fix one runtime branch"]
    }));

    assert_eq!(verdict, SupervisorVerdict::SupervisorDirectEdit);
    assert_eq!(
        get_str(&feedback, "verdict"),
        Some("supervisor_direct_edit")
    );
    assert_eq!(get_str(&feedback, "action"), Some("supervisor_direct_edit"));
}

#[test]
fn feedback_legacy_take_over_normalizes_to_supervisor_direct_edit() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "take_over",
        "takeover_reason": "Remaining work is localized edge-case repair.",
        "direct_plan": ["Add focused tests", "Fix one runtime branch"]
    }));

    assert_eq!(verdict, SupervisorVerdict::SupervisorDirectEdit);
    assert_eq!(
        get_str(&feedback, "verdict"),
        Some("supervisor_direct_edit")
    );
    assert_eq!(get_str(&feedback, "action"), Some("supervisor_direct_edit"));
    assert_eq!(get_str(&feedback, "raw_verdict"), Some("take_over"));
    assert_eq!(
        SupervisorFeedback::from_value(&feedback)
            .supervisor_direct_edit_reason
            .as_deref(),
        Some("Remaining work is localized edge-case repair.")
    );
}
