use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn mixmod_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mixmod"))
}

fn run_mixmod(root: &Path, args: &[&str]) -> Output {
    Command::new(mixmod_bin())
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("failed to run mixmod {args:?}: {error}"))
}

fn run_mixmod_with_env(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(mixmod_bin());
    command.args(args).current_dir(root);
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run mixmod {args:?}: {error}"))
}

fn assert_success(output: Output) {
    if !output.status.success() {
        panic!(
            "command failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn assert_failure(output: Output) {
    if output.status.success() {
        panic!(
            "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn read_json(path: &Path) -> Value {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

#[test]
fn internal_commands_are_debug_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    assert_failure(run_mixmod(root, &["init"]));
    assert!(!root.join(".mixmod/config.toml").exists());

    assert_success(run_mixmod_with_env(
        root,
        &["init"],
        &[("MIXMOD_DEBUG_COMMANDS", "1")],
    ));
    assert!(root.join(".mixmod/config.toml").exists());
    assert!(root.join(".mixmod/codex-home").exists());
    assert!(root.join(".mixmod/opencode.json").exists());
    assert!(!root.join("opencode.json").exists());
    assert!(!root.join(".codex/mixmod-instructions.md").exists());

    assert_failure(run_mixmod(root, &["status"]));
    assert_success(run_mixmod_with_env(
        root,
        &["status"],
        &[("MIXMOD_DEBUG_COMMANDS", "1")],
    ));
    assert_failure(run_mixmod(root, &["doctor"]));
    assert_failure(run_mixmod(root, &["hook", "session-start"]));
    assert_failure(run_mixmod(root, &["experiment", "init", "demo"]));
    assert_failure(run_mixmod(root, &["uninstall"]));
}

#[test]
fn live_control_writes_control_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    assert_success(run_mixmod(
        root,
        &[
            "live",
            "control",
            "--run",
            ".mixmod/runs/demo",
            "--action",
            "interrupt_context_focus",
            "--message",
            "Focus on the parser.",
            "--focus-file",
            "src/parser.rs",
            "--check",
            "cargo test parser",
            "--risk",
            "context drift",
        ],
    ));

    let control = read_json(&root.join(".mixmod/runs/demo/control.json"));
    assert!(root.join(".mixmod/config.toml").exists());
    assert!(root.join(".mixmod/opencode.json").exists());
    assert!(root.join(".mixmod/codex-home").exists());
    assert!(!root.join("opencode.json").exists());
    assert!(!root.join(".codex").exists());
    assert_eq!(control["action"], "interrupt_context_focus");
    assert_eq!(control["worker_mode"], "context_focus");
    assert_eq!(control["message_to_worker"], "Focus on the parser.");
    assert_eq!(control["focus_files"][0], "src/parser.rs");
    assert_eq!(control["required_checks"][0], "cargo test parser");
    assert_eq!(control["risk"], "context drift");
}

#[test]
fn experiment_codex_only_task_copy_strips_hidden_metadata() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let fixture = root.join("fixture");
    std::fs::create_dir_all(fixture.join("src")).unwrap();
    std::fs::write(fixture.join("src/lib.rs"), "pub fn value() -> i32 { 1 }\n").unwrap();
    std::fs::write(
        fixture.join("task.json"),
        r#"{
  "title": "Hidden metadata",
  "instructions": "Change value to 2.",
  "files": ["src/lib.rs"],
  "tests": ["cargo test"],
  "patch": "gold implementation diff",
  "test_patch": "hidden test diff",
  "hints_text": "secret hint",
  "fail_to_pass": ["hidden::test"],
  "context": {
    "dataset": "swe-bench-lite",
    "instance_id": "demo__demo-1",
    "test_patch": "hidden context patch",
    "gold_patch": "hidden context gold"
  }
}
"#,
    )
    .unwrap();

    assert_success(run_mixmod_with_env(
        root,
        &[
            "experiment",
            "init",
            "demo",
            "--fixture",
            fixture.to_str().unwrap(),
        ],
        &[("MIXMOD_DEBUG_COMMANDS", "1")],
    ));

    let empty_path = TempDir::new().unwrap();
    let output = Command::new(mixmod_bin())
        .args([
            "experiment",
            "record-codex-only",
            "demo",
            "--task",
            ".mixmod/experiments/demo/task.json",
        ])
        .current_dir(root)
        .env("PATH", empty_path.path())
        .env("MIXMOD_DEBUG_COMMANDS", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());

    let task = read_json(&root.join(".mixmod/experiments/demo/work/codex-only/task.json"));
    assert!(task.get("patch").is_none());
    assert!(task.get("test_patch").is_none());
    assert!(task.get("hints_text").is_none());
    assert!(task.get("fail_to_pass").is_none());
    assert_eq!(task["context"]["dataset"], "swe-bench-lite");
    assert_eq!(task["context"]["instance_id"], "demo__demo-1");
    assert!(task["context"].get("test_patch").is_none());
    assert!(task["context"].get("gold_patch").is_none());
}
