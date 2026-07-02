use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn mixmod_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mixmod"))
}

fn read_json(path: &Path) -> Value {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
    assert_success(output);
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

fn assert_command_available(command: &str) {
    let output = Command::new(command).arg("--version").output();
    assert!(
        output.is_ok(),
        "`{command}` is required for this live end-to-end test"
    );
}

fn latest_exec_run(root: &Path) -> PathBuf {
    let runs_dir = root.join(".mixmod/runs");
    let mut runs = fs::read_dir(&runs_dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", runs_dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("run-"))
        })
        .collect::<Vec<_>>();
    runs.sort();
    runs.pop()
        .unwrap_or_else(|| panic!("no exec run directory under {}", runs_dir.display()))
}

fn value_u64(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("missing numeric metrics field `{key}`"))
}

fn value_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string metrics field `{key}`"))
}

fn value_bool(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("missing boolean metrics field `{key}`"))
}

fn assert_array_contains(value: &Value, key: &str, expected: &str) {
    let array = value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing array metrics field `{key}`"));
    assert!(
        array.iter().any(|item| item.as_str() == Some(expected)),
        "`{key}` did not contain `{expected}`: {array:?}"
    );
}

fn assert_qwen_worker(metrics: &Value) {
    let provider = value_str(metrics, "opencode_provider").to_ascii_lowercase();
    let model = value_str(metrics, "opencode_model").to_ascii_lowercase();
    let model_arg = value_str(metrics, "opencode_model_arg").to_ascii_lowercase();
    assert!(
        provider.contains("local") || provider.contains("ollama"),
        "expected a local worker provider, got `{provider}`"
    );
    assert!(
        model.contains("qwen") || model_arg.contains("qwen"),
        "expected a Qwen worker model, got model `{model}` and arg `{model_arg}`"
    );
}

#[test]
#[ignore = "requires live Codex plus local OpenCode/Qwen inference"]
fn exec_supervises_qwen_worker_end_to_end() {
    if std::env::var("MIXMOD_RUN_LIVE_E2E").as_deref() != Ok("1") {
        eprintln!("skipping live e2e; set MIXMOD_RUN_LIVE_E2E=1 to run it");
        return;
    }

    assert_command_available("git");
    assert_command_available("codex");
    assert_command_available("opencode");

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("calculator.py"),
        "def add(a, b):\n    return a - b\n",
    )
    .unwrap();
    fs::write(
        root.join("task.json"),
        r#"{
  "title": "Fix calculator addition",
  "instructions": "Fix calculator.add so it returns the sum of a and b. Edit calculator.py only.",
  "files": ["calculator.py"],
  "tests": ["python3 - <<'PY'\nfrom calculator import add\nassert add(2, 3) == 5\nassert add(-1, 1) == 0\nPY"],
  "acceptance": ["calculator.add returns a + b", "the listed Python check passes"]
}
"#,
    )
    .unwrap();

    run_git(root, &["init", "-b", "main"]);
    run_git(
        root,
        &["config", "user.email", "mixmod-e2e@example.invalid"],
    );
    run_git(root, &["config", "user.name", "Mixmod E2E"]);
    run_git(root, &["add", "calculator.py"]);
    run_git(root, &["commit", "-m", "seed broken calculator"]);

    let supervisor_model =
        std::env::var("MIXMOD_E2E_SUPERVISOR_MODEL").unwrap_or_else(|_| "gpt-5.5:high".to_string());
    let worker_model = std::env::var("MIXMOD_E2E_WORKER_MODEL")
        .unwrap_or_else(|_| "mixmod-local-ollama/qwen3.6:27b".to_string());
    let output = Command::new(mixmod_bin())
        .args([
            "exec",
            "--task",
            "task.json",
            "--supervisor-model",
            &supervisor_model,
            "--worker-model",
            &worker_model,
        ])
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("failed to run mixmod exec: {error}"));
    assert_success(output);

    let run_dir = latest_exec_run(root);
    let metrics = read_json(&run_dir.join("metrics.json"));
    assert_eq!(value_str(&metrics, "kind"), "mixmod-default-strategy");
    assert_eq!(value_str(&metrics, "final_status"), "approved_by_codex");
    assert_eq!(value_str(&metrics, "test_status"), "passed");
    assert!(value_u64(&metrics, "frontier_input_tokens") > 0);
    assert!(value_u64(&metrics, "frontier_output_tokens") > 0);
    assert!(value_u64(&metrics, "codex_calls") >= 2);
    assert!(value_u64(&metrics, "opencode_calls") >= 1);
    assert!(value_bool(&metrics, "require_local"));
    assert!(value_bool(&metrics, "local_inference_verified"));
    assert_qwen_worker(&metrics);
    assert_array_contains(&metrics, "strategy_phases", "codex_worker_brief");
    assert_array_contains(&metrics, "strategy_phases", "codex_open_code_decision_loop");

    assert!(run_dir.join("worker-brief.json").exists());
    assert!(run_dir.join("frontier-feedback.jsonl").exists());
    assert!(run_dir.join("worker-runs/proposal/metrics.json").exists());
    assert!(run_dir.join("final.patch").exists());

    let calculator = fs::read_to_string(root.join("calculator.py")).unwrap();
    assert!(
        calculator.contains("return a + b"),
        "calculator.py was not fixed:\n{calculator}"
    );
}
