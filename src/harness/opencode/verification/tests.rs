use std::time::Duration;

use serde_json::{Value, json};

use crate::{get_str, get_string_array};

use super::no_delta::SmallPatchNoDeltaIntervention;

fn revision_slice_task() -> Value {
    json!({
        "context": {
            "revision": {
                "delta_expected": true,
                "worker_turn_shape": "small_patch_slice",
                "patch_decision": "revise_current",
                "message_to_worker": "Add the serialization branch.",
                "focus_files": ["builder.py", "test_builder.py"],
                "exact_edits": ["Add only the flatten=True serialization branch."]
            }
        }
    })
}

fn initial_slice_task() -> Value {
    json!({
        "expect_patch": true,
        "files": ["helper.py", "test_helper.py"],
        "context": {
            "worker_brief": {
                "expect_patch": true,
                "worker_turn_shape": "small_patch_slice",
                "message_to_worker": "Add flatten metadata.",
                "files": ["helper.py", "test_helper.py"],
                "exact_edits": ["Add only the flatten metadata field."]
            }
        }
    })
}

#[test]
fn small_patch_no_delta_guard_interrupts_then_aborts_after_thresholds() {
    let mut guard =
        SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 2, 1)
            .unwrap();

    assert!(
        guard
            .maybe_control_for_diff("", Duration::from_secs(1), Duration::from_secs(1))
            .is_none()
    );
    let interrupt = guard
        .maybe_control_for_diff("", Duration::from_secs(2), Duration::from_secs(2))
        .unwrap();

    assert_eq!(get_str(&interrupt, "action"), Some("interrupt_continue"));
    assert_eq!(
        get_str(&interrupt, "source"),
        Some("auto_revision_no_delta")
    );
    assert!(
        get_str(&interrupt, "message_to_worker")
            .unwrap()
            .contains("Add only the flatten=True serialization branch.")
    );
    let abort = guard
        .maybe_control_for_diff("", Duration::from_secs(2), Duration::from_secs(2))
        .unwrap();
    assert_eq!(get_str(&abort, "action"), Some("abort_worker_turn"));
    assert_eq!(
        get_str(&abort, "source"),
        Some("auto_revision_no_delta_abort")
    );
    assert_eq!(get_str(&abort, "risk"), Some("worker_stalled_no_delta"));
    assert!(
        guard
            .maybe_control_for_diff("", Duration::from_secs(3), Duration::from_secs(3))
            .is_none()
    );
}

#[test]
fn small_patch_no_delta_guard_ignores_existing_new_delta() {
    let mut guard =
        SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 1, 1)
            .unwrap();
    let current_diff = "\
diff --git a/builder.py b/builder.py
--- a/builder.py
+++ b/builder.py
@@ -1,1 +1,2 @@
 old
+new
";

    assert!(
        guard
            .maybe_control_for_diff(current_diff, Duration::from_secs(5), Duration::from_secs(5),)
            .is_none()
    );
}

#[test]
fn small_patch_no_delta_guard_waits_during_recent_worker_output() {
    let mut guard =
        SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 2, 1)
            .unwrap();

    assert!(
        guard
            .maybe_control_for_diff("", Duration::from_secs(10), Duration::from_secs(1))
            .is_none()
    );
}

#[test]
fn small_patch_no_delta_guard_does_not_abort_after_recovery_delta() {
    let mut guard =
        SmallPatchNoDeltaIntervention::from_task(&revision_slice_task(), String::new(), 1, 1)
            .unwrap();
    assert!(
        guard
            .maybe_control_for_diff("", Duration::from_secs(1), Duration::from_secs(1))
            .is_some()
    );
    let current_diff = "\
diff --git a/builder.py b/builder.py
--- a/builder.py
+++ b/builder.py
@@ -1,1 +1,2 @@
 old
+new
";

    assert!(
        guard
            .maybe_control_for_diff(current_diff, Duration::from_secs(1), Duration::from_secs(1),)
            .is_none()
    );
}

#[test]
fn small_patch_no_delta_guard_handles_initial_worker_brief() {
    let mut guard =
        SmallPatchNoDeltaIntervention::from_task(&initial_slice_task(), String::new(), 1, 1)
            .unwrap();

    let interrupt = guard
        .maybe_control_for_diff("", Duration::from_secs(1), Duration::from_secs(1))
        .unwrap();

    assert_eq!(get_str(&interrupt, "action"), Some("interrupt_continue"));
    assert_eq!(get_str(&interrupt, "source"), Some("auto_initial_no_delta"));
    assert!(
        get_str(&interrupt, "message_to_worker")
            .unwrap()
            .contains("first worker turn")
    );
    assert!(
        get_str(&interrupt, "message_to_worker")
            .unwrap()
            .contains("Add only the flatten metadata field.")
    );
    assert_eq!(
        get_string_array(&interrupt, "focus_files"),
        vec!["helper.py", "test_helper.py"]
    );
}

#[test]
fn small_patch_no_delta_guard_requires_small_patch_slice() {
    let task = json!({
        "context": {
            "revision": {
                "delta_expected": true,
                "worker_turn_shape": "default"
            }
        }
    });

    assert!(SmallPatchNoDeltaIntervention::from_task(&task, String::new(), 1, 1).is_none());
}
