use super::*;

#[test]
fn supervisor_feedback_prompt_explains_worker_session_modes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let prompt = supervisor_feedback_prompt(
        root,
        &[root.join("missing-report.md")],
        "decide",
        &WorkerSupervisorGuidance::default(),
    )
    .unwrap();

    assert!(prompt.contains("worker_mode=continue to keep the same worker session"));
    assert!(prompt.contains("worker_mode=context_focus to start a new worker session"));
    assert!(prompt.contains("worker session as context-saturated"));
    assert!(prompt.contains("prefer worker_mode=context_focus"));
    assert!(prompt.contains("one concrete source edit"));
    assert!(prompt.contains("patch_decision"));
    assert!(prompt.contains("revise_previous"));
    assert!(prompt.contains("summarize the concrete source/test edits to recover"));
    assert!(prompt.contains("Do not ask the worker to inspect Mixmod state"));
    assert!(prompt.contains("previous worker context is discarded"));
    assert!(prompt.contains("Do not implement code. Do not edit files."));
    assert!(prompt.contains("Do not ask the user for approval."));
    assert!(prompt.contains("worker-model guidance as part of the supervisor decision contract"));
    assert!(prompt.contains("a broad revise is the wrong decision"));
    assert!(prompt.contains("use stop with a clear risk instead of sending a broad revision"));
    assert!(
        prompt.contains(
            "Prefer revise after failed, empty, distracted, or incomplete worker attempts"
        )
    );
    assert!(prompt.contains("not merely because the latest worker turn created a non-empty diff"));
    assert!(prompt.contains("worker-brief.json used worker_turn_shape=small_patch_slice"));
    assert!(prompt.contains("set worker_turn_shape=small_patch_slice with the next narrow"));
    assert!(prompt.contains("worker-brief.json used worker_turn_shape=bounded_feature_slice"));
    assert!(prompt.contains("judge whether the previous worker slice was too much"));
    assert!(prompt.contains("keep or enlarge the next slice as bounded_feature_slice"));
    assert!(prompt.contains("exact_edits"));
    assert!(prompt.contains("Make the next slice one behavior only"));
    assert!(prompt.contains("Treat exact_edits as a queue"));
    assert!(prompt.contains("put one source edit first"));
    assert!(prompt.contains("current accumulated worktree.patch"));
    assert!(prompt.contains("Preserve useful existing edits"));
    assert!(prompt.contains("local transformation near one anchor"));
    assert!(prompt.contains("edit_packet"));
    assert!(prompt.contains("source_snippets"));
    assert!(prompt.contains("exact symbols plus a literal nearby code anchor"));
    assert!(prompt.contains("literal nearby code anchor"));
    assert!(prompt.contains("Stop does not permit direct supervisor editing."));
}

#[test]
fn supervisor_feedback_repair_prompt_preserves_accumulated_work() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let artifact = root.join("worktree.patch");
    atomic_write(
        &artifact,
        br#"diff --git a/mashumaro/helper.py b/mashumaro/helper.py
--- a/mashumaro/helper.py
+++ b/mashumaro/helper.py
@@ -1,1 +1,3 @@
+flatten_prefix
+flatten_rename
"#,
    )
    .unwrap();
    let previous = json!({
        "action": "revise",
        "focus_files": ["mashumaro/core/meta/code/builder.py"],
        "exact_edits": [
            "In mashumaro/core/meta/code/builder.py, implement serialization-only flatten support."
        ]
    });

    let prompt = supervisor_feedback_repair_prompt(
        root,
        &[artifact],
        &WorkerSupervisorGuidance::default(),
        &previous,
    )
    .unwrap();

    assert!(prompt.contains("Preserve the previous feedback's intended target behavior"));
    assert!(prompt.contains("do not rewind to an earlier completed slice"));
    assert!(prompt.contains("Treat useful accumulated worktree.patch changes as context to keep"));
    assert!(
        prompt.contains("Write the repaired exact edit from the current accumulated patch state")
    );
    assert!(prompt.contains("Do not say to continue from an earlier file-only slice"));
    assert!(prompt.contains("edit_packet or source_snippets"));
    assert!(prompt.contains("local transformation near one anchor"));
    assert!(prompt.contains("If previous feedback named one focus file"));
    assert!(prompt.contains("exact_edits must be an array with exactly one string item"));
    assert!(prompt.contains("Do not invent a different file/symbol pair"));
}

#[test]
fn supervisor_prompts_include_selected_worker_model_guidance() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let guidance = MixmodConfig::default().worker_supervisor_guidance();
    let feedback_prompt =
        supervisor_feedback_prompt(root, &[root.join("missing-report.md")], "decide", &guidance)
            .unwrap();

    assert!(feedback_prompt.contains("Supervisor-only worker-model guidance"));
    assert!(feedback_prompt.contains("Do not copy every bullet to the worker"));
    assert!(feedback_prompt.contains("Treat applicable bullets as binding"));
    assert!(feedback_prompt.contains("global environments"));

    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Checkout",
  "instructions": "Fix totals.",
  "tests": ["python -m unittest -q"]
}"#,
    )
    .unwrap();

    let brief_prompt =
        supervisor_worker_brief_prompt(root, &task, &guidance, SupervisorInitMode::Compact)
            .unwrap();
    assert!(brief_prompt.contains("Supervisor-only worker-model guidance"));
    assert!(brief_prompt.contains("reasoning before editing"));
    assert!(brief_prompt.contains("large effective context"));
    assert!(brief_prompt.contains("split broad tasks into small concrete source slices"));
    assert!(brief_prompt.contains("worker_turn_shape=small_patch_slice"));
    assert!(brief_prompt.contains("one immediate source edit"));
    assert!(brief_prompt.contains("current accumulated patch"));
    assert!(brief_prompt.contains("one literal anchor plus the smallest local transformation"));
    assert!(brief_prompt.contains("context overflow"));
    assert!(brief_prompt.contains("worker_mode=context_focus"));
    assert!(brief_prompt.contains("Select only relevant points"));
    assert!(brief_prompt.contains("worker-model guidance as handoff constraints"));
}

#[test]
fn supervisor_review_artifacts_include_task_and_handoff_context() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let default_dir = root.join("default");
    let worker_dir = root.join("worker");
    fs::create_dir_all(&default_dir).unwrap();
    fs::create_dir_all(&worker_dir).unwrap();
    for name in [TASK_JSON, WORKER_BRIEF_JSON, WORKER_TASK_JSON] {
        atomic_write(&default_dir.join(name), b"{}").unwrap();
    }
    for name in RUN_COMPACT_ARTIFACTS {
        atomic_write(&worker_dir.join(name), b"{}").unwrap();
    }

    let paths = supervisor_review_artifact_paths(&default_dir, &worker_dir)
        .into_iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        &paths[..3],
        &[TASK_JSON, WORKER_BRIEF_JSON, WORKER_TASK_JSON]
    );
    assert!(paths.contains(&WORKTREE_PATCH.to_string()));
    assert!(paths.contains(&CHANGES_PATCH.to_string()));
}

#[test]
fn checkpoint_detects_lost_focused_patch_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let previous = root.join("previous");
    let current = root.join("current");
    fs::create_dir_all(&previous).unwrap();
    fs::create_dir_all(&current).unwrap();
    atomic_write(
        &previous.join("worktree.patch"),
        br#"diff --git a/src/_pytest/assertion/rewrite.py b/src/_pytest/assertion/rewrite.py
--- a/src/_pytest/assertion/rewrite.py
+++ b/src/_pytest/assertion/rewrite.py
@@ -1,1 +1,1 @@
-old
+new
diff --git a/testing/test_assertrewrite.py b/testing/test_assertrewrite.py
--- a/testing/test_assertrewrite.py
+++ b/testing/test_assertrewrite.py
@@ -1,1 +1,1 @@
-old
+new
"#,
    )
    .unwrap();
    atomic_write(
        &current.join("worktree.patch"),
        br#"diff --git a/AUTHORS b/AUTHORS
--- a/AUTHORS
+++ b/AUTHORS
@@ -1,1 +1,2 @@
 Alice
+Bob
"#,
    )
    .unwrap();
    atomic_write(&current.join("changes.patch"), b"").unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        patch_decision: "accept_current".to_string(),
        hint: "Fix assertion rewrite.".to_string(),
        revision_handoff: RevisionHandoff::default(),
        focus_files: vec![
            "src/_pytest/assertion/rewrite.py".to_string(),
            "testing/test_assertrewrite.py".to_string(),
        ],
        required_checks: vec![],
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
    };

    let comparison = write_patch_checkpoint_comparison(&previous, &current, &decision).unwrap();

    assert!(comparison.degradation_detected);
    assert_eq!(
        comparison.lost_focus_files,
        vec![
            "src/_pytest/assertion/rewrite.py",
            "testing/test_assertrewrite.py"
        ]
    );
    assert!(current.join(PATCH_COMPARISON).exists());
    assert!(current.join(PREVIOUS_WORKTREE_PATCH).exists());

    let mut artifacts = Vec::new();
    append_patch_checkpoint_artifacts(&current, &mut artifacts).unwrap();
    assert!(
        artifacts
            .iter()
            .any(|path| path.ends_with(PATCH_COMPARISON))
    );
    assert!(
        artifacts
            .iter()
            .any(|path| path.ends_with(PREVIOUS_WORKTREE_PATCH))
    );
}

#[test]
fn checkpoint_detects_destructive_small_patch_slice() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let previous = root.join("previous");
    let current = root.join("current");
    fs::create_dir_all(&previous).unwrap();
    fs::create_dir_all(&current).unwrap();
    atomic_write(
        &previous.join(WORKTREE_PATCH),
        br#"diff --git a/src/builder.py b/src/builder.py
--- a/src/builder.py
+++ b/src/builder.py
@@ -1,1 +1,1 @@
-old
+new
"#,
    )
    .unwrap();
    let destructive_patch = format!(
        "diff --git a/src/builder.py b/src/builder.py\n--- a/src/builder.py\n+++ b/src/builder.py\n@@ -1,30 +1,1 @@\n{}\n+replacement\n",
        (0..30)
            .map(|index| format!("-removed_{index}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    atomic_write(&current.join(WORKTREE_PATCH), destructive_patch.as_bytes()).unwrap();
    atomic_write(&current.join(CHANGES_PATCH), destructive_patch.as_bytes()).unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        patch_decision: "revise_current".to_string(),
        hint: "Add one builder branch.".to_string(),
        revision_handoff: RevisionHandoff {
            worker_turn_shape: Some("small_patch_slice".to_string()),
            ..RevisionHandoff::default()
        },
        focus_files: vec!["src/builder.py".to_string()],
        required_checks: vec![],
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
    };

    let comparison = write_patch_checkpoint_comparison(&previous, &current, &decision).unwrap();

    assert!(comparison.degradation_detected);
    assert!(comparison.latest_delta_stats.removed_lines > 25);
    assert!(
        comparison
            .reasons
            .iter()
            .any(|reason| reason.contains("small patch slice removed too many lines"))
    );
    assert!(comparison.supervisor_guidance.contains("revise_previous"));
}

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
fn auto_no_delta_control_preserves_small_patch_slice_revision_shape() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "supervisor_control_events": [{
                "action": "interrupt_continue",
                "worker_mode": "continue",
                "message_to_worker": "Make only this edit now.",
                "focus_files": ["builder.py"],
                "required_checks": [],
                "risk": "no delta",
                "control": {
                    "source": "auto_revision_no_delta",
                    "worker_turn_shape": "small_patch_slice",
                    "turn_goal": "make one recovery edit",
                    "exact_edits": ["Edit builder.py in one place."],
                    "defer_checks_until_patch_exists": true,
                    "completion_gate": "git diff --stat must be non-empty",
                    "forbidden_actions": ["ask questions"]
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
    assert_eq!(decision.worker_mode, "continue");
    assert_eq!(decision.patch_decision, "revise_current");
    assert!(decision.revision_handoff.is_small_patch_slice());
    assert_eq!(
        decision.revision_handoff.exact_edits,
        vec!["Edit builder.py in one place."]
    );
    assert_eq!(
        decision.revision_handoff.completion_gate.as_deref(),
        Some("git diff --stat must be non-empty")
    );
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
fn auto_no_delta_stop_control_classifies_worker_stall() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "supervisor_control_events": [{
                "action": "stop",
                "worker_mode": "continue",
                "message_to_worker": "Worker made no repository delta after no-delta recovery.",
                "focus_files": ["builder.py"],
                "required_checks": [],
                "risk": "worker_stalled_no_delta",
                "control": {
                    "source": "auto_revision_no_delta_stop"
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

    assert_eq!(decision.verdict, "stop");
    assert_eq!(decision.worker_mode, "continue");
    assert_eq!(decision.patch_decision, "accept_current");
    assert_eq!(
        get_str(&decision.feedback, "risk"),
        Some("worker_stalled_no_delta")
    );
}

#[test]
fn subtracts_unchanged_preexisting_diff_blocks() {
    let before = "diff --git a/task.json b/task.json\nnew file mode 100644\n--- /dev/null\n+++ b/task.json\n@@ -0,0 +1,1 @@\n+{}\n";
    let after = format!(
        "{before}diff --git a/src/generated.rs b/src/generated.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/generated.rs\n@@ -0,0 +1,1 @@\n+pub fn generated() {{}}\n"
    );
    let filtered = diff_without_unchanged_blocks(&after, before);
    assert!(!filtered.contains("task.json"));
    assert!(filtered.contains("src/generated.rs"));
}
