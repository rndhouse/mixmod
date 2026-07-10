use super::super::*;

#[test]
fn supervisor_control_metrics_become_revision_decision() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "supervisor_control_events": [{
                "action": "interrupt_context_focus",
                "worker_mode": "context_focus",
                "message_to_worker": "Ignore setup and edit sympy/core/numbers.py first.",
                "focus_files": ["sympy/core/numbers.py"],
                "required_checks": ["python -m pytest sympy/core/tests/test_power.py -q"],
                "risk": "worker was distracted by dependency setup"
            }]
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();

    let decision = supervisor_control_decision_from_metrics(&run_dir)
        .unwrap()
        .unwrap();

    assert_eq!(decision.verdict, "revise");
    assert_eq!(decision.worker_mode, "context_focus");
    assert!(decision.hint.contains("edit sympy/core/numbers.py"));
    assert_eq!(decision.focus_files, vec!["sympy/core/numbers.py"]);
    assert_eq!(
        decision.required_checks,
        vec!["python -m pytest sympy/core/tests/test_power.py -q"]
    );
}

#[test]
fn nonterminal_supervisor_control_with_patch_waits_for_review() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "interrupted_by_supervisor": false,
            "changed_file_count": 1,
            "patch_bytes": 128,
            "supervisor_control_events": [{
                "action": "interrupt_continue",
                "worker_mode": "continue",
                "message_to_worker": "Make only this edit now.",
                "focus_files": ["helper.py"],
                "required_checks": [],
                "risk": "initial no delta"
            }]
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();

    let decision = supervisor_control_decision_from_metrics(&run_dir).unwrap();

    assert!(decision.is_none());
}

#[test]
fn live_supervisor_no_delta_control_becomes_small_patch_revision() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "changed_file_count": 0,
            "patch_bytes": 0,
            "supervisor_control_events": [{
                "action": "interrupt_context_focus",
                "worker_mode": "context_focus",
                "message_to_worker": "In builder.py near `packers = {}`, make one source edit before tests.",
                "focus_files": ["builder.py"],
                "required_checks": [],
                "risk": "context overflow",
                "control": {
                    "source": "codex_live_supervisor"
                }
            }]
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();

    let decision = supervisor_control_decision_from_metrics(&run_dir)
        .unwrap()
        .unwrap();

    assert_eq!(decision.verdict, "revise");
    assert_eq!(decision.worker_mode, "context_focus");
    assert_eq!(decision.patch_decision, "revise_current");
    assert!(decision.revision_handoff.is_small_patch_slice());
    assert_eq!(
        decision.revision_handoff.exact_edits,
        vec!["In builder.py near `packers = {}`, make one source edit before tests."]
    );
}

#[test]
fn abort_control_waits_for_supervisor_review() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "supervisor_control_events": [
                {
                    "action": "interrupt_context_focus",
                    "worker_mode": "context_focus",
                    "message_to_worker": "Patch only builder.py.",
                    "focus_files": ["builder.py"],
                    "required_checks": [],
                    "risk": "context overflow",
                    "control": {
                        "source": "codex_live_supervisor"
                    }
                },
                {
                    "action": "abort_worker_turn",
                    "worker_mode": "continue",
                    "message_to_worker": "Worker made no repository delta after no-delta recovery.",
                    "focus_files": ["builder.py"],
                    "required_checks": [],
                    "risk": "worker_stalled_no_delta",
                    "control": {
                        "source": "codex_live_supervisor"
                    }
                }
            ]
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();

    let decision = supervisor_control_decision_from_metrics(&run_dir).unwrap();

    assert!(decision.is_none());
}
