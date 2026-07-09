use super::super::*;

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

    assert!(instruction.contains("Did you follow the supervisor's current instruction?"));
    assert!(instruction.contains("Mention checks only if you actually ran one."));
    assert!(!instruction.contains("Diff non-empty: yes/no"));
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

    assert!(instruction.contains("Did you follow the supervisor's current instruction?"));
    assert!(instruction.contains("Mention checks only if you actually ran one."));
    assert!(!instruction.contains("Diff non-empty: yes/no"));
    assert!(!instruction.contains("Tests run and results"));
    assert!(!instruction.contains("Stop immediately after the requested tests pass"));
}
