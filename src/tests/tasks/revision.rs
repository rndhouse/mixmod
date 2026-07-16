use super::super::*;

#[test]
fn revision_task_preserves_codex_focus_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": ["Do not commit."],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        patch_decision: "accept_current".to_string(),
        hint: "Update the discount code and its test.".to_string(),
        revision_handoff: RevisionHandoff::default(),
        focus_files: vec!["checkout.py".to_string(), "test_checkout.py".to_string()],
        required_checks: vec!["python -m unittest -q".to_string()],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 1).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(
        get_string_array(&revision, "files"),
        vec!["checkout.py", "test_checkout.py"]
    );
    assert_eq!(
        get_string_array(&revision, "acceptance"),
        vec!["python -m unittest -q"]
    );
    let constraints = get_string_array(&revision, "constraints");
    assert!(constraints.contains(&"Do not commit.".to_string()));
    assert!(constraints.contains(&"Keep the revision focused.".to_string()));
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Message to worker: Update the discount code"));
    assert_eq!(
        get_bool(&revision["context"]["revision"], "delta_expected"),
        Some(true)
    );
    assert_eq!(
        get_str(&revision["context"]["revision"], "message_to_worker"),
        Some("Update the discount code and its test.")
    );
    assert_eq!(
        get_string_array(&revision["context"]["revision"], "focus_files"),
        vec!["checkout.py", "test_checkout.py"]
    );
}

#[test]
fn context_focus_revision_task_uses_focused_prompt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        patch_decision: "accept_current".to_string(),
        hint: "Ignore dependency setup and edit the focused files first.".to_string(),
        revision_handoff: RevisionHandoff::default(),
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec!["python -m unittest -q".to_string()],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 2).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(get_str(&revision, "worker_mode"), Some("context_focus"));
    assert_eq!(get_string_array(&revision, "files"), vec!["checkout.py"]);
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("worker_mode=context_focus"));
    assert!(instructions.contains("fresh focused worker attempt"));
    assert!(instructions.contains("make the code/test edit first"));
}

#[test]
fn no_patch_revision_task_is_verification_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": ["python -m unittest -q"],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        patch_decision: "accept_current".to_string(),
        hint: "Verify the supervisor surgical patch.".to_string(),
        revision_handoff: RevisionHandoff {
            expect_patch: Some(false),
            worker_turn_shape: Some("default".to_string()),
            turn_goal: Some("verify supervisor surgical patch".to_string()),
            edit_plan: vec!["Run the focused regression check.".to_string()],
            forbidden_actions: vec!["edit files".to_string()],
            ..RevisionHandoff::default()
        },
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec!["python -m unittest tests.test_checkout".to_string()],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 3).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(get_bool(&revision, "expect_patch"), Some(false));
    assert!(get_string_array(&revision, "tests").is_empty());
    assert!(get_string_array(&revision, "acceptance").is_empty());
    let constraints = get_string_array(&revision, "constraints");
    assert!(constraints.contains(
        &"Do not edit files; run or inspect only the requested verification.".to_string()
    ));
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Noninteractive verification revision"));
    assert!(instructions.contains("Do not edit files"));
    assert!(instructions.contains("Focused checks:"));
    assert!(instructions.contains("python -m unittest tests.test_checkout"));
    assert!(!instructions.contains("make the code/test edit first"));
    assert_eq!(
        get_bool(&revision["context"]["revision"], "delta_expected"),
        Some(false)
    );
}

#[test]
fn planning_probe_revision_task_is_no_patch_and_no_delta_expected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": ["checkout.py", "types.py"],
  "tests": ["python -m unittest -q"],
  "constraints": [],
  "acceptance": ["typed totals are enforced"]
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        patch_decision: "accept_current".to_string(),
        hint: "Inspect the current checkout/type flow and propose the next source slice."
            .to_string(),
        revision_handoff: RevisionHandoff {
            expect_patch: Some(false),
            worker_turn_shape: Some("planning_probe".to_string()),
            turn_goal: Some("propose next typed checkout source slice".to_string()),
            edit_plan: vec![
                "Identify the next authored-source edit from the current worktree.".to_string(),
                "Return files, anchors, expected patch size, and risk.".to_string(),
            ],
            ..RevisionHandoff::default()
        },
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec![],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 2).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(get_bool(&revision, "expect_patch"), Some(false));
    assert_eq!(get_bool(&revision["context"], "expect_patch"), Some(false));
    assert_eq!(
        get_bool(&revision["context"]["revision"], "delta_expected"),
        Some(false)
    );
    assert_eq!(get_string_array(&revision, "tests"), Vec::<String>::new());
    assert_eq!(
        get_string_array(&revision, "acceptance"),
        Vec::<String>::new()
    );
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Noninteractive planning probe"));
    assert!(instructions.contains("This is a no-patch revision turn"));
    assert!(instructions.contains("Do not edit files."));
    assert!(instructions.contains("Do not run tests."));
    assert!(instructions.contains("Prefer targeted searches"));
    assert!(instructions.contains("Do not ask the user for more requirements"));
    assert!(instructions.contains("Recommended next slice:"));
    assert!(instructions.contains("Return files, anchors, expected patch size, and risk."));
}

#[test]
fn revision_task_mentions_revise_previous_checkpoint_decision() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        patch_decision: "revise_previous".to_string(),
        hint: "Recover the earlier source edit and remove unrelated files.".to_string(),
        revision_handoff: RevisionHandoff::default(),
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec![],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 3).unwrap();
    let revision = read_json_file(&path).unwrap();
    let instructions = get_str(&revision, "instructions").unwrap();

    assert!(instructions.contains("Patch checkpoint decision: revise_previous"));
    assert!(instructions.contains("Mixmod has restored the previous candidate patch"));
    assert!(instructions.contains("Apply only the focused follow-up edit"));
    assert!(instructions.contains("Do not read Mixmod artifacts directly."));
    assert!(!instructions.contains("previous-worktree.patch"));
    assert_eq!(
        get_str(&revision["context"], "patch_decision"),
        Some("revise_previous")
    );
}

#[test]
fn revision_task_mentions_accept_current_baseline_checkpoint_decision() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        patch_decision: "accept_current_baseline".to_string(),
        hint: "Add the next focused validation branch.".to_string(),
        revision_handoff: RevisionHandoff {
            worker_turn_shape: Some("patch_request".to_string()),
            ..RevisionHandoff::default()
        },
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec![],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 3).unwrap();
    let revision = read_json_file(&path).unwrap();
    let instructions = get_str(&revision, "instructions").unwrap();

    assert!(instructions.contains("Patch checkpoint decision: accept_current_baseline"));
    assert!(instructions.contains("previous useful patch as the source baseline"));
    assert!(instructions.contains("active diff starts clean relative to that baseline"));
    assert!(instructions.contains("Do not inspect Git history or Mixmod artifacts."));
    assert_eq!(
        get_str(&revision["context"], "patch_decision"),
        Some("accept_current_baseline")
    );
    assert_eq!(
        get_bool(&revision["context"]["revision"], "delta_expected"),
        Some(true)
    );
}

#[test]
fn patch_request_revision_task_preserves_explicit_supervisor_gate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Add nested checkout discounts and update tests.",
  "files": [],
  "tests": ["python -m unittest -q"],
  "constraints": [],
  "acceptance": ["discounts apply to nested checkout items"]
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        patch_decision: "revise_current".to_string(),
        hint: "Add the nested item discount branch and one focused assertion.".to_string(),
        revision_handoff: RevisionHandoff {
            expect_patch: Some(true),
            worker_turn_shape: Some("patch_request".to_string()),
            turn_goal: Some("nested item discount branch".to_string()),
            exact_edits: vec![
                "In checkout.py, add the branch that applies item discounts inside nested checkout items.".to_string(),
                "In test_checkout.py, add one assertion for a nested discounted item.".to_string(),
            ],
            edit_plan: vec![],
            deferred_checks: vec!["python -m unittest test_checkout.py -q".to_string()],
            defer_checks_until_patch_exists: Some(true),
            stop_condition: Some(
                "return after the nested discount branch and one focused assertion".to_string(),
            ),
            completion_gate: Some("git diff --stat must be non-empty".to_string()),
            forbidden_actions: vec!["run broad tests before editing".to_string()],
        },
        focus_files: vec!["checkout.py".to_string(), "test_checkout.py".to_string()],
        required_checks: vec!["python -m unittest test_checkout.py -q".to_string()],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 1).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(
        get_string_array(&revision, "files"),
        vec!["checkout.py", "test_checkout.py"]
    );
    assert_eq!(get_string_array(&revision, "tests"), Vec::<String>::new());
    assert_eq!(
        get_string_array(&revision, "acceptance"),
        vec!["git diff --stat must be non-empty"]
    );
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Noninteractive coding revision"));
    assert!(instructions.contains("Current source state includes useful prior edits"));
    assert!(instructions.contains("Supervisor-provided edit details:"));
    assert!(!instructions.contains("Worker edit packet:"));
    assert!(!instructions.contains("Use the Worker edit packet before reading whole files."));
    assert!(instructions.contains("nested item discount branch"));
    assert!(instructions.contains("add one assertion for a nested discounted item"));
    assert!(instructions.contains("Do not run broad tests before editing."));
    assert!(instructions.contains("Supervisor stop condition:"));
    assert!(instructions.contains("return after the nested discount branch"));
    assert!(!instructions.contains("Return after one useful tracked diff"));
    assert!(instructions.contains("Supervisor completion gate:"));
    assert!(instructions.contains("git diff --stat must be non-empty"));
    assert!(!instructions.contains("After editing, run exactly: git diff --stat"));
    assert_eq!(
        get_str(&revision["context"]["revision"], "worker_turn_shape"),
        Some("patch_request")
    );
    assert_eq!(
        get_string_array(&revision["context"]["revision"], "exact_edits"),
        vec![
            "In checkout.py, add the branch that applies item discounts inside nested checkout items.",
            "In test_checkout.py, add one assertion for a nested discounted item."
        ]
    );
}

#[test]
fn patch_request_revision_task_allows_goal_without_exact_edits() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix checkout totals.",
  "files": ["checkout.py"],
  "tests": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        patch_decision: "revise_current".to_string(),
        hint: "Fix checkout totals.".to_string(),
        revision_handoff: RevisionHandoff {
            expect_patch: Some(true),
            worker_turn_shape: Some("patch_request".to_string()),
            turn_goal: Some("Fix the checkout total calculation.".to_string()),
            exact_edits: vec![],
            edit_plan: vec![],
            deferred_checks: vec![],
            defer_checks_until_patch_exists: Some(true),
            stop_condition: None,
            completion_gate: None,
            forbidden_actions: vec![],
        },
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec![],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 1).unwrap();
    let revision = read_json_file(&path).unwrap();
    let instructions = get_str(&revision, "instructions").unwrap();

    assert!(instructions.contains("Patch request goal: Fix the checkout total calculation."));
    assert!(!instructions.contains("Supervisor-provided edit details:"));
    assert!(instructions.contains("Relevant files:"));
    assert!(instructions.contains("- checkout.py"));
    assert!(instructions.contains("Supervisor stop condition:"));
    assert!(instructions.contains("Return after one useful tracked diff"));
    assert!(instructions.contains("do not continue into another independent slice"));
}

#[test]
fn revision_task_keeps_mixmod_artifacts_out_of_repo_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("work/task.json");
    fs::create_dir_all(task.parent().unwrap()).unwrap();
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = SupervisorFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        patch_decision: "accept_current".to_string(),
        hint: "Use the latest focused task and edit the source file.".to_string(),
        revision_handoff: RevisionHandoff::default(),
        focus_files: vec![
            "revision-task-3.json".to_string(),
            "sympy/core/power.py".to_string(),
            "../default/worker-brief.json".to_string(),
        ],
        required_checks: vec![],
        takeover_reason: None,
        direct_plan: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    };

    let path =
        write_revision_task(root, &task, &root.join("default"), "demo", &decision, 4).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(
        get_string_array(&revision, "files"),
        vec!["sympy/core/power.py"]
    );
    assert_eq!(
        get_string_array(&revision["context"], "mixmod_artifact_refs"),
        vec!["revision-task-3.json", "../default/worker-brief.json"]
    );
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Mixmod artifact references from the supervisor"));
    assert!(instructions.contains("use the current task text and the supervisor message instead"));
    assert!(!instructions.contains("compact artifacts instead"));
    assert!(!instructions.contains("Focus files: [\"revision-task-3.json\""));
}
