use super::*;

#[test]
fn opencode_instruction_includes_local_worker_self_check() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Worker support",
  "instructions": "Make the requested local change.",
  "files": ["src/lib.rs"],
  "tests": ["cargo test"]
}"#,
    )
    .unwrap();
    let (_, task_spec) = read_task_json(&task).unwrap();

    let instruction = build_opencode_instruction(
        DelegationMode::Patch,
        &task_spec,
        &task,
        &state_layout(root).runs().join("example"),
    )
    .unwrap();

    assert!(instruction.contains("## Completion Self-Check"));
    assert!(instruction.contains("Expected repository patch: yes"));
    assert!(instruction.contains("Mixmod-managed state lives outside this repository"));
    assert!(!instruction.contains("Task file:"));
    assert!(!instruction.contains("Artifact directory:"));
    assert!(instruction.contains("Did you complete every edit you intended to make?"));
    assert!(instruction.contains("If you intended checks or verification"));
    assert!(instruction.contains("Do not claim success if intended edits"));
    assert!(instruction.contains("## Output Contract"));
}

#[test]
fn opencode_instruction_honors_no_patch_tasks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Investigate only",
  "instructions": "Explain what changed.",
  "expect_patch": false
}"#,
    )
    .unwrap();
    let (_, task_spec) = read_task_json(&task).unwrap();

    let instruction = build_opencode_instruction(
        DelegationMode::Patch,
        &task_spec,
        &task,
        &state_layout(root).runs().join("example"),
    )
    .unwrap();

    assert!(instruction.contains("Expected repository patch: no"));
}

#[test]
fn as_given_worker_brief_uses_original_task_defaults() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "As-given handoff",
  "instructions": "Fix the checkout bug.",
  "files": ["checkout.py"],
  "tests": ["python -m unittest -q"],
  "constraints": ["Keep patch focused."],
  "acceptance": ["Tests pass."]
}"#,
    )
    .unwrap();

    let brief = json!({"handoff": "as_given"});
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "tests"),
        vec!["python -m unittest -q"]
    );
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["Tests pass."]
    );

    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains("Supervisor message to worker:"));
    assert!(instructions.contains("Proceed from the original task."));
    assert!(instructions.contains("Fix the checkout bug."));
    assert!(!instructions.contains("Objective:"));
    assert!(!instructions.contains("Implementation steps:"));
    assert_eq!(worker_task["context"]["worker_brief"], brief);
}

#[test]
fn worker_brief_prompt_prioritizes_compact_executable_handoff() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    atomic_write(
        &root.join("checkout.py"),
        b"def total(items):\n    return sum(items)\n",
    )
    .unwrap();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Checkout",
  "instructions": "Fix totals.",
  "files": ["checkout.py"],
  "tests": ["python -m unittest -q"]
}"#,
    )
    .unwrap();

    let prompt = supervisor_worker_brief_prompt(
        root,
        &task,
        &WorkerSupervisorGuidance::default(),
        SupervisorInitMode::Compact,
    )
    .unwrap();

    assert!(prompt.contains("minimize supervisor output"));
    assert!(prompt.contains("compact executable worker handoff"));
    assert!(prompt.contains("exact files, edit target, expected behavior, and checks"));
    assert!(prompt.contains(r#"Default to "guided""#));
    assert!(prompt.contains("Guided means terse and executable"));
    assert!(prompt.contains("target <=120 output tokens"));
    assert!(prompt.contains("one command-style message_to_worker"));
    assert!(prompt.contains("usually <=2"));
    assert!(prompt.contains("worker_turn_shape"));
    assert!(prompt.contains("small_patch_slice"));
    assert!(prompt.contains("exact_edits"));
    assert!(prompt.contains("completion_gate"));
    assert!(prompt.contains("preferred worker_turn_shape"));
    assert!(prompt.contains("first patch seed"));
    assert!(prompt.contains("Bad small_patch_slice choices ask for a whole feature"));
    assert!(prompt.contains("immediately executable edit commands"));
    assert!(prompt.contains("concrete repo file paths, not directories"));
    assert!(prompt.contains("omit investigation_summary"));
    assert!(prompt.contains(r#""expect_patch": true"#));
    assert!(prompt.contains("Set false for investigation/no-change handoffs"));
    assert!(prompt.contains("setup rabbit holes"));
    assert!(prompt.contains("already names the relevant files, desired behavior, and checks"));
    assert!(prompt.contains(r#"{"handoff":"as_given"}"#));
    assert!(prompt.contains("omit empty fields"));
}

#[test]
fn investigative_worker_brief_prompt_allows_read_only_file_pass() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Checkout",
  "instructions": "Fix totals.",
  "files": [],
  "tests": []
}"#,
    )
    .unwrap();

    let prompt = supervisor_worker_brief_prompt(
        root,
        &task,
        &WorkerSupervisorGuidance::default(),
        SupervisorInitMode::Investigate,
    )
    .unwrap();

    assert!(prompt.contains("read-only repo investigation"));
    assert!(prompt.contains("rg"));
    assert!(prompt.contains("target <=500 output tokens"));
    assert!(prompt.contains("investigation_summary"));
    assert!(prompt.contains("edit_plan"));
    assert!(prompt.contains("evidence"));
    assert!(prompt.contains("less capable"));
}

#[test]
fn worker_task_surfaces_supervisor_investigation_notes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Investigated handoff",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": []
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "message_to_worker": "Patch checkout totals before running broad tests.",
        "investigation_summary": "Discount total uses pre-tax values in checkout.py.",
        "edit_plan": ["Update calculate_total.", "Add one regression test."],
        "evidence": ["checkout.py:calculate_total has the wrong order."]
    });
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();
    let instructions = get_str(&worker_task, "instructions").unwrap();

    assert!(instructions.contains("Supervisor investigation:"));
    assert!(instructions.contains("Summary: Discount total uses pre-tax values"));
    assert!(instructions.contains("Edit plan:"));
    assert!(instructions.contains("- Update calculate_total."));
    assert!(instructions.contains("Evidence:"));
}

#[test]
fn small_patch_slice_worker_task_uses_noninteractive_diff_gate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Flatten metadata",
  "instructions": "Implement a broad flatten feature and run pytest.",
  "files": ["mashumaro/helper.py", "mashumaro/core/meta/code/builder.py"],
  "tests": ["python -m pytest tests/test_helper.py"],
  "acceptance": ["all tests pass"]
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "small_patch_slice",
        "turn_goal": "Create the first metadata plumbing patch.",
        "files": ["mashumaro/helper.py", "tests/test_helper.py"],
        "exact_edits": [
            "Add flatten: bool = False to field_options.",
            "Add flatten_prefix: Optional[Union[str, bool]] = None.",
            "Add flatten_rename: Optional[dict[str, str]] = None.",
            "Return all three keys in the metadata dict.",
            "Update tests/test_helper.py expectations for those keys."
        ],
        "defer_checks_until_patch_exists": true,
        "deferred_checks": ["python -m pytest tests/test_helper.py"],
        "completion_gate": "git diff --stat must be non-empty",
        "forbidden_actions": ["ask questions", "run tests before editing"]
    });
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(
        get_string_array(&worker_task, "files"),
        vec!["mashumaro/helper.py", "tests/test_helper.py"]
    );
    assert!(get_string_array(&worker_task, "tests").is_empty());
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["git diff --stat must be non-empty"]
    );

    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains("Noninteractive coding task"));
    assert!(instructions.contains("No user will answer questions"));
    assert!(
        instructions
            .contains("Your only goal in this turn is to create a non-empty repository patch.")
    );
    assert!(instructions.contains("Do not ask questions."));
    assert!(instructions.contains("Do not run tests in this turn."));
    assert!(instructions.contains("Do not stop after reading files."));
    assert!(instructions.contains("Patch slice goal: Create the first metadata plumbing patch."));
    assert!(instructions.contains("1. Add flatten: bool = False to field_options."));
    assert!(!instructions.contains("Add flatten_prefix: Optional"));
    assert!(instructions.contains("additional edit(s)"));
    assert!(instructions.contains("Do not do them now."));
    assert!(instructions.contains("Make exactly this first small patch:"));
    assert!(
        instructions.contains("If a listed item is a directory, do not read the whole directory")
    );
    assert!(instructions.contains("git diff --stat"));
    assert!(instructions.contains("Diff non-empty: yes/no"));
    assert!(!instructions.contains("Supervisor handoff JSON"));
    assert!(!instructions.contains("python -m pytest tests/test_helper.py"));
    assert_eq!(worker_task["context"]["worker_brief"], brief);
}

#[test]
fn small_patch_slice_opencode_instruction_uses_patch_only_output_contract() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("worker-task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Small patch slice",
  "instructions": "Noninteractive coding task.\n\nMake exactly this first small patch:\n1. Update helper.py.",
  "expect_patch": true,
  "files": ["helper.py"],
  "tests": [],
  "acceptance": ["git diff --stat must be non-empty"],
  "context": {
    "expect_patch": true,
    "worker_brief": {
      "worker_turn_shape": "small_patch_slice"
    }
  }
}"#,
    )
    .unwrap();
    let (_, task_spec) = read_task_json(&task).unwrap();

    let instruction = build_opencode_instruction(
        DelegationMode::Patch,
        &task_spec,
        &task,
        &state_layout(root).runs().join("example"),
    )
    .unwrap();

    assert!(instruction.contains("Did you modify repository files?"));
    assert!(instruction.contains("Did `git diff --stat` show a non-empty patch?"));
    assert!(instruction.contains("Diff non-empty: yes/no"));
    assert!(instruction.contains("Do not mention tests unless you actually ran one by mistake."));
    assert!(!instruction.contains("Tests run and results"));
    assert!(!instruction.contains("Stop immediately after the requested tests pass"));
}

#[test]
fn revision_small_patch_slice_opencode_instruction_uses_patch_only_output_contract() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("revision-task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Revision small patch slice",
  "instructions": "Noninteractive coding revision.\n\nMake exactly this next small patch:\n1. Update builder.py.",
  "expect_patch": true,
  "files": ["builder.py"],
  "tests": [],
  "acceptance": ["git diff --stat must be non-empty"],
  "context": {
    "expect_patch": true,
    "revision": {
      "worker_turn_shape": "small_patch_slice",
      "message_to_worker": "Add the next narrow source edit.",
      "worker_mode": "continue",
      "patch_decision": "revise_current",
      "focus_files": ["builder.py"],
      "required_checks": []
    }
  }
}"#,
    )
    .unwrap();
    let (_, task_spec) = read_task_json(&task).unwrap();

    let instruction = build_opencode_instruction(
        DelegationMode::Patch,
        &task_spec,
        &task,
        &state_layout(root).runs().join("revision"),
    )
    .unwrap();

    assert!(instruction.contains("Did you modify repository files?"));
    assert!(instruction.contains("Diff non-empty: yes/no"));
    assert!(instruction.contains("Do not mention tests unless you actually ran one by mistake."));
    assert!(!instruction.contains("Tests run and results"));
    assert!(!instruction.contains("Stop immediately after the requested tests pass"));
}

#[test]
fn agent_visible_task_strips_swebench_scoring_metadata() {
    let task = json!({
        "title": "SWE-bench Lite sympy__sympy-20212",
        "instructions": "Fix 0**-oo.",
        "files": [],
        "tests": [],
        "patch": "gold implementation diff",
        "test_patch": "hidden test diff",
        "hints_text": "look in numbers.py",
        "context": {
            "benchmark": "SWE-bench Lite",
            "dataset": "SWE-bench/SWE-bench_Lite",
            "split": "test",
            "instance_id": "sympy__sympy-20212",
            "repo": "sympy/sympy",
            "base_commit": "a106f4782a9dbe7f8fd16030f15401d977e03ae9",
            "version": "1.7",
            "fail_to_pass": "[\"test_zero\"]",
            "pass_to_pass": "[\"test_rational\"]",
            "candidate_pool": "codex-pass-pool-v1",
            "selection_rule": "keep only Codex-pass instances",
            "environment_setup_commit": "setup"
        }
    });

    let visible = agent_visible_task_value(&task);
    let text = serde_json::to_string(&visible).unwrap();

    assert!(text.contains("Fix 0**-oo."));
    assert!(text.contains("sympy__sympy-20212"));
    assert!(text.contains("sympy/sympy"));
    for forbidden in [
        "test_zero",
        "test_rational",
        "gold implementation diff",
        "hidden test diff",
        "look in numbers.py",
        "candidate_pool",
        "selection_rule",
        "environment_setup_commit",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} leaked in {text}");
    }
}

#[test]
fn codex_prompt_uses_agent_visible_task() {
    let task = json!({
        "title": "SWE-bench Lite sympy__sympy-20212",
        "instructions": "Fix 0**-oo.",
        "context": {
            "instance_id": "sympy__sympy-20212",
            "repo": "sympy/sympy",
            "fail_to_pass": "[\"test_zero\"]",
            "pass_to_pass": "[\"test_rational\"]"
        }
    });

    let prompt = codex_only_prompt(Path::new("/tmp/work"), &task).unwrap();

    assert!(prompt.contains("Fix 0**-oo."));
    assert!(prompt.contains("sympy__sympy-20212"));
    assert!(!prompt.contains("test_zero"));
    assert!(!prompt.contains("test_rational"));
    assert!(!prompt.contains("fail_to_pass"));
    assert!(!prompt.contains("pass_to_pass"));
}

#[test]
fn worktree_task_copy_is_agent_visible_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let source = root.join("full-task.json");
    let target = root.join("work/task.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    atomic_write(
        &source,
        br#"{
  "title": "SWE-bench Lite sympy__sympy-20212",
  "instructions": "Fix 0**-oo.",
  "context": {
    "instance_id": "sympy__sympy-20212",
    "repo": "sympy/sympy",
    "fail_to_pass": "[\"test_zero\"]",
    "pass_to_pass": "[\"test_rational\"]"
  }
}"#,
    )
    .unwrap();

    write_agent_visible_task_file(&source, &target).unwrap();
    let written = fs::read_to_string(&target).unwrap();

    assert!(written.contains("Fix 0**-oo."));
    assert!(!written.contains("test_zero"));
    assert!(!written.contains("test_rational"));
}

#[test]
fn worker_task_does_not_link_to_unsanitized_source_task() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "SWE-bench Lite sympy__sympy-20212",
  "instructions": "Fix 0**-oo.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": [],
  "context": {
    "instance_id": "sympy__sympy-20212",
    "repo": "sympy/sympy",
    "fail_to_pass": "[\"test_zero\"]",
    "pass_to_pass": "[\"test_rational\"]"
  }
}"#,
    )
    .unwrap();

    let brief = json!({"handoff": "as_given"});
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();
    let text = serde_json::to_string(&worker_task).unwrap();

    assert!(text.contains("Fix 0**-oo."));
    assert!(!text.contains("test_zero"));
    assert!(!text.contains("test_rational"));
    assert!(worker_task["context"].get("source_task").is_none());
}

#[test]
fn focused_worker_brief_overrides_files_and_adds_supplemental_checks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Focused handoff",
  "instructions": "Fix the checkout bug.",
  "files": ["catalog.py", "checkout.py"],
  "tests": ["python -m unittest -q"],
  "constraints": [],
  "acceptance": ["Tests pass."]
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "focused",
        "expect_patch": false,
        "supplement": "Preserve public return shapes.",
        "focus_files": ["checkout.py"],
        "must_check": ["VIP discount after line discounts"],
        "avoid": ["broad refactor"]
    });
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_bool(&worker_task, "expect_patch"), Some(false));
    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["VIP discount after line discounts"]
    );
    assert!(
        get_string_array(&worker_task, "constraints")
            .contains(&"Avoid: broad refactor".to_string())
    );
    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains("Supervisor message to worker:"));
    assert!(instructions.contains("Preserve public return shapes."));
    assert!(instructions.contains("Files:"));
}

#[test]
fn direct_worker_message_is_preserved() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Direct handoff",
  "instructions": "Fix the checkout bug.",
  "files": ["catalog.py"],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "message_to_worker": "Investigate checkout totals and make the smallest safe fix.",
        "files": ["checkout.py"],
        "checks": ["VIP discount after line discounts"]
    });
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_bool(&worker_task, "expect_patch"), Some(true));
    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["VIP discount after line discounts"]
    );
    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains(
        "Supervisor message to worker:\nInvestigate checkout totals and make the smallest safe fix."
    ));
}

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

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 1).unwrap();
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

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 2).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(get_str(&revision, "worker_mode"), Some("context_focus"));
    assert_eq!(get_string_array(&revision, "files"), vec!["checkout.py"]);
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("worker_mode=context_focus"));
    assert!(instructions.contains("fresh focused worker attempt"));
    assert!(instructions.contains("make the code/test edit first"));
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

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 3).unwrap();
    let revision = read_json_file(&path).unwrap();
    let instructions = get_str(&revision, "instructions").unwrap();

    assert!(instructions.contains("Patch checkpoint decision: revise_previous"));
    assert!(instructions.contains("Recover the previous candidate using the supervisor message"));
    assert!(instructions.contains("Do not read Mixmod artifacts directly."));
    assert!(!instructions.contains("previous-worktree.patch"));
    assert_eq!(
        get_str(&revision["context"], "patch_decision"),
        Some("revise_previous")
    );
}

#[test]
fn small_patch_slice_revision_task_uses_noninteractive_delta_gate() {
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
            worker_turn_shape: Some("small_patch_slice".to_string()),
            turn_goal: Some("nested item discount branch".to_string()),
            exact_edits: vec![
                "In checkout.py, add the branch that applies item discounts inside nested checkout items.".to_string(),
                "In test_checkout.py, add one assertion for a nested discounted item.".to_string(),
            ],
            deferred_checks: vec!["python -m unittest test_checkout.py -q".to_string()],
            defer_checks_until_patch_exists: Some(true),
            completion_gate: Some("git diff --stat must be non-empty".to_string()),
            forbidden_actions: vec!["run broad tests before editing".to_string()],
        },
        focus_files: vec!["checkout.py".to_string(), "test_checkout.py".to_string()],
        required_checks: vec!["python -m unittest test_checkout.py -q".to_string()],
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

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 1).unwrap();
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
    let constraints = get_string_array(&revision, "constraints");
    assert!(
        constraints
            .iter()
            .any(|constraint| constraint.contains("Do not run tests in this revision turn"))
    );
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Noninteractive coding revision"));
    assert!(instructions.contains("Current accumulated patch is useful but not yet accepted"));
    assert!(
        instructions.contains(
            "Your only goal in this revision turn is to add a non-empty repository delta"
        )
    );
    assert!(instructions.contains("Make exactly this next small patch"));
    assert!(instructions.contains("nested item discount branch"));
    assert!(!instructions.contains("add one assertion for a nested discounted item"));
    assert!(instructions.contains("additional edit(s)"));
    assert!(instructions.contains("Do not do them now."));
    assert!(instructions.contains("Do not run broad tests before editing."));
    assert!(instructions.contains("After editing, run exactly: git diff --stat"));
    assert_eq!(
        get_str(&revision["context"]["revision"], "worker_turn_shape"),
        Some("small_patch_slice")
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

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 4).unwrap();
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
