use super::super::*;

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
        token_usage_comparable: true,
    };

    let comparison = write_patch_checkpoint_comparison(&previous, &current, &decision).unwrap();

    assert_eq!(
        comparison.observations,
        vec![
            "current patch lost changed file(s): src/_pytest/assertion/rewrite.py, testing/test_assertrewrite.py",
            "current patch lost focused file(s): src/_pytest/assertion/rewrite.py, testing/test_assertrewrite.py",
            "current patch no longer touches any focused files",
            "latest worker delta is empty while accumulated patch shrank from previous candidate",
        ]
    );
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
        !artifacts
            .iter()
            .any(|path| path.ends_with(PREVIOUS_WORKTREE_PATCH))
    );
}

#[test]
fn revise_previous_checkpoint_restores_previous_worktree_patch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let root = root.as_path();
    init_git(root);
    fs::create_dir_all(root.join("src")).unwrap();
    atomic_write(
        root.join("src/lib.rs").as_path(),
        b"pub fn value() -> i32 { 1 }\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "src/lib.rs"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();

    atomic_write(
        root.join("src/lib.rs").as_path(),
        b"pub fn value() -> i32 { 2 }\n",
    )
    .unwrap();
    atomic_write(root.join("new.rs").as_path(), b"pub fn added() {}\n").unwrap();
    atomic_write(root.join(TASK_JSON).as_path(), b"{\"title\":\"keep me\"}\n").unwrap();
    let previous_patch = git_diff_with_untracked(root).unwrap();

    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join(PREVIOUS_WORKTREE_PATCH),
        previous_patch.as_bytes(),
    )
    .unwrap();

    atomic_write(
        root.join("src/lib.rs").as_path(),
        b"pub fn value() -> i32 { 999 }\n",
    )
    .unwrap();
    atomic_write(root.join("bad.rs").as_path(), b"pub fn bad() {}\n").unwrap();

    let receipt = restore_previous_patch_checkpoint(root, &run_dir).unwrap();

    assert_eq!(receipt.status, "restored");
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "pub fn value() -> i32 { 2 }\n"
    );
    assert!(root.join("new.rs").exists());
    assert!(!root.join("bad.rs").exists());
    assert!(root.join(TASK_JSON).exists());
    assert_eq!(git_diff_with_untracked(root).unwrap(), previous_patch);
    assert!(run_dir.join(ROLLBACK_CURRENT_PATCH).exists());
    assert!(run_dir.join(ROLLBACK_RESTORED_PATCH).exists());
    assert!(run_dir.join(PATCH_ROLLBACK_JSON).exists());
}

#[test]
fn checkpoint_records_small_patch_slice_delta_observations() {
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
        token_usage_comparable: true,
    };

    let comparison = write_patch_checkpoint_comparison(&previous, &current, &decision).unwrap();

    assert!(comparison.latest_delta_stats.removed_lines > 25);
    assert!(
        comparison
            .observations
            .iter()
            .any(|observation| observation.contains("small patch slice latest delta removed lines"))
    );
    assert!(
        comparison
            .observations
            .iter()
            .all(|observation| !observation.contains("revise_previous"))
    );
}
