use super::super::*;

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
    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
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

    assert!(prompt.contains("minimizing expensive supervisor GPT output tokens"));
    assert!(prompt.contains("Local worker tokens are cheap"));
    assert!(prompt.contains("Use the worker for concrete implementation"));
    assert!(prompt.contains("remain accountable for correctness"));
    assert!(prompt.contains("Candidate repo file contents are not embedded"));
    assert!(prompt.contains("Candidate repo files:"));
    assert!(prompt.contains("checkout.py"));
    assert!(prompt.contains("listed by task"));
    assert!(!prompt.contains("def total(items):"));
    assert!(!prompt.contains("return sum(items)"));
    assert!(prompt.contains("Target <=160 supervisor output tokens"));
    assert!(prompt.contains("Worker shape contract:"));
    assert!(prompt.contains("No worker-specific default shape is selected"));
    assert!(prompt.contains("Choose the cheapest reliable next worker handoff"));
    assert!(prompt.contains(r#"Use {"handoff":"as_given"} only"#));
    assert!(prompt.contains("already gives enough files, behavior, and checks"));
    assert!(prompt.contains(r#"Use "guided" or "focused""#));
    assert!(prompt.contains("message_to_worker is only the short command"));
    assert!(prompt.contains("worker_turn_shape"));
    assert!(prompt.contains("planning_probe"));
    assert!(prompt.contains("patch_request"));
    assert!(prompt.contains("bounded_feature_slice"));
    assert!(prompt.contains("exact_edits"));
    assert!(prompt.contains("completion_gate"));
    assert!(prompt.contains("obey the worker shape contract"));
    assert!(prompt.contains("largest coherent slice"));
    assert!(prompt.contains("If the route is clear"));
    assert!(prompt.contains("concrete source edits"));
    assert!(prompt.contains("Handoff requirements:"));
    assert!(prompt.contains("do not duplicate it in message_to_worker"));
    assert!(prompt.contains("immediately executable edit instructions"));
    assert!(prompt.contains("only anchors or evidence"));
    assert!(prompt.contains("only for acceptance criteria"));
    assert!(prompt.contains("only for task-specific limits"));
    assert!(prompt.contains("concrete repo file paths, not directories"));
    assert!(prompt.contains(r#""expect_patch":true"#));
    assert!(prompt.contains("Ask for a compact proposal, not edits"));
    assert!(prompt.contains(r#"{"handoff":"as_given"}"#));
    assert!(prompt.contains("Omit optional fields"));
    assert!(prompt.contains("JSON shape:"));
}

#[test]
fn investigative_worker_brief_prompt_allows_repo_investigation_pass() {
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

    assert!(prompt.contains("Use the task JSON and candidate repo paths first"));
    assert!(prompt.contains("Inspect repo context only when that is needed"));
    assert!(prompt.contains("Workspace access is for supervision, not implementation"));
    assert!(prompt.contains("rg"));
    assert!(prompt.contains("git status"));
    assert!(prompt.contains("Stop inspecting once you can choose a reliable worker handoff"));
    assert!(prompt.contains("Target <=500 supervisor output tokens"));
    assert!(prompt.contains("investigation_summary"));
    assert!(prompt.contains("edit_plan"));
    assert!(prompt.contains("evidence"));
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
    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();
    let instructions = get_str(&worker_task, "instructions").unwrap();

    assert!(instructions.contains("Supervisor investigation:"));
    assert!(instructions.contains("Summary: Discount total uses pre-tax values"));
    assert!(instructions.contains("Edit plan:"));
    assert!(instructions.contains("- Update calculate_total."));
    assert!(instructions.contains("Evidence:"));
}

#[test]
fn patch_request_worker_task_preserves_explicit_supervisor_gate() {
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
        "worker_turn_shape": "patch_request",
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
    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
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
    assert!(instructions.contains("Do not ask questions."));
    assert!(instructions.contains("Do not run tests before editing."));
    assert!(instructions.contains("Patch slice goal: Create the first metadata plumbing patch."));
    assert!(instructions.contains("1. Add flatten: bool = False to field_options."));
    assert!(instructions.contains("2. Add flatten_prefix: Optional"));
    assert!(instructions.contains("3. Add flatten_rename: Optional"));
    assert!(instructions.contains("4. Return all three keys in the metadata dict."));
    assert!(instructions.contains("5. Update tests/test_helper.py expectations"));
    assert!(instructions.contains("Supervisor-requested patch slice:"));
    assert!(instructions.contains("Worker edit packet:"));
    assert!(instructions.contains("Use the Worker edit packet before reading whole files."));
    assert!(
        instructions.contains("If a listed item is a directory, do not read the whole directory")
    );
    assert!(instructions.contains("Supervisor completion gate:"));
    assert!(instructions.contains("git diff --stat"));
    assert!(!instructions.contains("Diff non-empty: yes/no"));
    assert!(!instructions.contains("Supervisor handoff JSON"));
    assert!(!instructions.contains("python -m pytest tests/test_helper.py"));
    assert_eq!(worker_task["context"]["worker_brief"], brief);
}

#[test]
fn patch_request_worker_task_includes_anchor_source_packet() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("mashumaro")).unwrap();
    atomic_write(
        &root.join("mashumaro/helper.py"),
        b"from typing import Any\n\n\ndef field_options(alias=None):\n    metadata = {}\n    if alias is not None:\n        metadata['alias'] = alias\n    return metadata\n",
    )
    .unwrap();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Flatten metadata",
  "instructions": "Add flatten metadata support.",
  "files": ["mashumaro/helper.py"],
  "tests": []
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "patch_request",
        "turn_goal": "Add the first flatten option to field_options.",
        "files": ["mashumaro/helper.py"],
        "exact_edits": [
            "In mashumaro/helper.py near the line containing \"def field_options(\" add a flatten: bool = False parameter and return it in metadata."
        ],
        "edit_packet": ["field_options is the public metadata helper."],
        "defer_checks_until_patch_exists": true
    });

    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();
    let instructions = get_str(&worker_task, "instructions").unwrap();

    assert!(instructions.contains("Supervisor packet:"));
    assert!(instructions.contains("field_options is the public metadata helper"));
    assert!(
        instructions
            .contains("Source snippet from mashumaro/helper.py around `def field_options(`")
    );
    assert!(instructions.contains("def field_options(alias=None):"));
    assert!(instructions.contains("Do not read an entire large file before the first edit"));
}

#[test]
fn planning_probe_worker_task_is_no_patch_and_proposal_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Plan checkout fix",
  "instructions": "Fix typed checkout totals.",
  "files": ["checkout.py", "types.py"],
  "tests": ["python -m unittest -q"],
  "constraints": [],
  "acceptance": ["typed totals are enforced"]
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "guided",
        "expect_patch": false,
        "worker_turn_shape": "planning_probe",
        "turn_goal": "Propose the next authored-source patch slice.",
        "files": ["checkout.py"],
        "planning_questions": [
            "Which one or two source edits should happen next?",
            "What anchors and expected patch size should GPT approve?"
        ],
        "evidence": ["Totals flow through checkout.total."]
    });

    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_bool(&worker_task, "expect_patch"), Some(false));
    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "tests"),
        Vec::<String>::new()
    );
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        Vec::<String>::new()
    );
    let constraints = get_string_array(&worker_task, "constraints");
    assert!(
        constraints
            .contains(&"Do not edit files; return a compact plan for the supervisor.".to_string())
    );
    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains("Noninteractive planning probe"));
    assert!(instructions.contains("This is a no-patch turn"));
    assert!(instructions.contains("Do not edit files."));
    assert!(instructions.contains("Do not run tests."));
    assert!(instructions.contains("Prefer targeted searches"));
    assert!(instructions.contains("Do not ask the user for more requirements"));
    assert!(instructions.contains("Recommended next slice:"));
    assert!(instructions.contains("Which one or two source edits should happen next?"));
    assert!(instructions.contains("Totals flow through checkout.total."));
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
    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
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
    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
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
