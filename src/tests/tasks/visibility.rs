use super::super::*;

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
    let worker_task_path =
        write_worker_brief_task(root, &task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();
    let text = serde_json::to_string(&worker_task).unwrap();

    assert!(text.contains("Fix 0**-oo."));
    assert!(!text.contains("test_zero"));
    assert!(!text.contains("test_rational"));
    assert!(worker_task["context"].get("source_task").is_none());
}
