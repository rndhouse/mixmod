use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn mixmod_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mixmod"))
}

fn state_root(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repo");
    root.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{name}-mixmod-state"))
}

fn project_state(root: &Path) -> PathBuf {
    state_root(root).join("projects").join(project_id(root))
}

fn project_id(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_project_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "root".to_string());
    format!(
        "{name}-{:016x}",
        fnv1a64(canonical.to_string_lossy().as_bytes())
    )
}

fn sanitize_project_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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

fn live_e2e_enabled() -> bool {
    if std::env::var("MIXMOD_RUN_LIVE_E2E").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipping live e2e; set MIXMOD_RUN_LIVE_E2E=1 to run it");
    false
}

fn assert_live_e2e_commands_available() {
    assert_command_available("git");
    assert_command_available("codex");
    assert_command_available("opencode");
}

fn create_broken_calculator_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("calculator.py"),
        "def add(a, b):\n    return a - b\n",
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

    temp
}

fn write_calculator_task(root: &Path) {
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
}

fn run_mixmod_exec(root: &Path, exec_args: &[&str]) -> PathBuf {
    let supervisor_model =
        std::env::var("MIXMOD_E2E_SUPERVISOR_MODEL").unwrap_or_else(|_| "gpt-5.5:high".to_string());
    let worker_model = std::env::var("MIXMOD_E2E_WORKER_MODEL")
        .unwrap_or_else(|_| "mixmod-local-ollama/qwen3.6:27b".to_string());
    let mut args = vec![
        "exec".to_string(),
        "--supervisor-model".to_string(),
        supervisor_model,
        "--worker-model".to_string(),
        worker_model,
    ];
    args.extend(exec_args.iter().map(|arg| arg.to_string()));

    let output = Command::new(mixmod_bin())
        .args(&args)
        .current_dir(root)
        .env("MIXMOD_STATE_DIR", state_root(root))
        .output()
        .unwrap_or_else(|error| panic!("failed to run mixmod {args:?}: {error}"));
    assert_success(output);

    latest_exec_run(root)
}

fn latest_exec_run(root: &Path) -> PathBuf {
    let runs_dir = project_state(root).join("runs");
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

fn assert_common_exec_metrics(metrics: &Value) {
    assert_eq!(value_str(metrics, "kind"), "mixmod-default-strategy");
    assert_eq!(value_str(metrics, "final_status"), "approved_by_codex");
    assert!(value_u64(metrics, "supervisor_input_tokens") > 0);
    assert!(value_u64(metrics, "supervisor_output_tokens") > 0);
    assert!(value_u64(metrics, "codex_calls") >= 2);
    assert!(value_u64(metrics, "opencode_calls") >= 1);
    assert!(value_bool(metrics, "require_local"));
    assert!(value_bool(metrics, "local_inference_verified"));
    assert_qwen_worker(metrics);
    assert_array_contains(metrics, "strategy_phases", "codex_worker_brief");
    assert_array_contains(metrics, "strategy_phases", "codex_open_code_decision_loop");
}

fn assert_common_exec_artifacts(run_dir: &Path) {
    assert!(run_dir.join("worker-brief.json").exists());
    assert!(run_dir.join("supervisor-feedback.jsonl").exists());
    assert!(run_dir.join("worker-runs/proposal/metrics.json").exists());
    assert!(run_dir.join("final.patch").exists());
}

fn assert_calculator_fixed(root: &Path) {
    let calculator = fs::read_to_string(root.join("calculator.py")).unwrap();
    assert!(
        calculator.contains("return a + b"),
        "calculator.py was not fixed:\n{calculator}"
    );
    let output = Command::new("python3")
        .args([
            "-c",
            "from calculator import add; assert add(2, 3) == 5; assert add(-1, 1) == 0",
        ])
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("failed to run calculator assertion: {error}"));
    assert_success(output);
}

#[test]
#[ignore = "requires live Codex plus local OpenCode/Qwen inference"]
fn exec_supervises_qwen_worker_end_to_end() {
    if !live_e2e_enabled() {
        return;
    }
    assert_live_e2e_commands_available();

    let temp = create_broken_calculator_repo();
    let root = temp.path();
    write_calculator_task(root);

    let run_dir = run_mixmod_exec(root, &["--task", "task.json"]);
    let metrics = read_json(&run_dir.join("metrics.json"));
    assert_common_exec_metrics(&metrics);
    assert!(metrics.get("test_status").is_none());
    assert_common_exec_artifacts(&run_dir);
    assert_calculator_fixed(root);
}

#[test]
#[ignore = "requires live Codex plus local OpenCode/Qwen inference"]
fn exec_supervises_qwen_worker_from_prompt_end_to_end() {
    if !live_e2e_enabled() {
        return;
    }
    assert_live_e2e_commands_available();

    let temp = create_broken_calculator_repo();
    let root = temp.path();
    let prompt = "In calculator.py, replace the broken subtraction in add(a, b) with addition so it returns a + b. Only edit calculator.py. Verify with: python3 -c 'from calculator import add; assert add(2, 3) == 5; assert add(-1, 1) == 0'";

    let run_dir = run_mixmod_exec(root, &[prompt]);
    let metrics = read_json(&run_dir.join("metrics.json"));
    assert_common_exec_metrics(&metrics);
    assert!(metrics.get("test_status").is_none());
    assert_common_exec_artifacts(&run_dir);
    assert_calculator_fixed(root);

    let run_task = read_json(&run_dir.join("task.json"));
    assert_eq!(value_str(&run_task, "instructions"), prompt);
    assert!(
        project_state(root)
            .join("tasks")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
}
