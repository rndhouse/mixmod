use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    DelegationMode, METRICS_JSON, MixmodConfig, OPENCODE_INSTRUCTIONS_MD, REPORT_MD,
    ShellOpenCodeRunner, WORKTREE_PATCH, WorkerRunOptions, absolutize, display_path, get_str,
    load_config, read_json_file, run_mixmod_task_with_worker_options, state_layout,
    write_pretty_json,
};

const HOOKS_JSON: &str = "hooks.json";
const CONFIG_SNAPSHOT_JSON: &str = "supervisor-tool-proxy-config.json";
const PAYLOAD_DIR: &str = "supervisor-tool-proxy-payloads";

/// Install or remove the supervisor-scoped Codex hook in Mixmod's `CODEX_HOME`.
pub(crate) fn prepare_supervisor_tool_proxy_home(
    code_home: &Path,
    config: Option<&MixmodConfig>,
) -> Result<SupervisorToolProxySetup> {
    let Some(config) = config else {
        return Ok(SupervisorToolProxySetup::disabled());
    };
    if !config.strategy.supervisor_tool_proxy.enabled {
        remove_managed_hook_files(code_home)?;
        return Ok(SupervisorToolProxySetup::disabled());
    }

    let exe = env::current_exe().context("failed to locate current mixmod executable")?;
    let config_path = code_home.join(CONFIG_SNAPSHOT_JSON);
    write_pretty_json(&config_path, config, "supervisor tool proxy config")?;

    let hooks = json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "^Bash$",
                    "hooks": [
                        {
                            "type": "command",
                            "command": format!("{} codex-hook pre-tool-use", shell_quote_path(&exe)),
                            "timeout": 30,
                            "statusMessage": "Checking Mixmod supervisor tool proxy"
                        }
                    ]
                }
            ]
        }
    });
    write_pretty_json(&code_home.join(HOOKS_JSON), &hooks, "Codex hooks")?;
    Ok(SupervisorToolProxySetup {
        enabled: true,
        config_path: Some(config_path),
    })
}

/// Runtime data needed when starting Codex with the scoped hook enabled.
#[derive(Clone, Debug, Default)]
pub(crate) struct SupervisorToolProxySetup {
    /// Whether a hook was installed for this supervisor Codex home.
    pub(crate) enabled: bool,
    /// Effective Mixmod config snapshot used by the hook proxy process.
    pub(crate) config_path: Option<PathBuf>,
}

impl SupervisorToolProxySetup {
    fn disabled() -> Self {
        Self {
            enabled: false,
            config_path: None,
        }
    }
}

/// Handle Codex `PreToolUse` input for the supervisor-scoped proxy hook.
pub(crate) fn codex_hook_pre_tool_use() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read Codex hook input")?;
    let event: Value = serde_json::from_str(&input).context("failed to parse Codex hook JSON")?;
    let Some(command) = bash_command_from_pre_tool_use(&event) else {
        return Ok(());
    };
    if !should_proxy_bash_command(command) {
        return Ok(());
    }

    let code_home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("CODEX_HOME was not set for supervisor tool proxy hook"))?;
    let config_path = env::var_os("MIXMOD_SUPERVISOR_TOOL_PROXY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| code_home.join(CONFIG_SNAPSHOT_JSON));
    let payload = SupervisorToolProxyPayload::from_event(command, &event, config_path);
    let payload_path = payload_path(&code_home, &payload)?;
    write_pretty_json(&payload_path, &payload, "supervisor tool proxy payload")?;

    let exe = env::current_exe().context("failed to locate current mixmod executable")?;
    let replacement = format!(
        "{} codex-hook run-tool-proxy --payload {}",
        shell_quote_path(&exe),
        shell_quote_path(&payload_path)
    );
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "command": replacement
            }
        }
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

/// Execute a worker-backed proxy payload and print a compact result for Codex.
pub(crate) fn run_supervisor_tool_proxy(payload_path: &Path, invocation_cwd: &Path) -> Result<()> {
    let payload: SupervisorToolProxyPayload = serde_json::from_value(read_json_file(payload_path)?)
        .with_context(|| format!("failed to parse payload {}", payload_path.display()))?;
    let root = payload
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| invocation_cwd.to_path_buf())
        .canonicalize()
        .unwrap_or_else(|_| absolutize(invocation_cwd, Path::new(".")));
    let config = load_tool_proxy_config(&payload.config_path, &root)?;
    let runner = ShellOpenCodeRunner::new(config.clone());
    let out_dir = state_layout(&root)
        .project_dir()
        .join("supervisor-tool-proxy")
        .join(sanitize_id(payload.turn_id.as_deref().unwrap_or("turn")))
        .join(sanitize_id(
            payload.tool_use_id.as_deref().unwrap_or("tool"),
        ));
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let task_path = out_dir.join("tool-task.json");
    let task = tool_proxy_task(&payload);
    write_pretty_json(&task_path, &task, "supervisor tool proxy task")?;

    let receipt = run_mixmod_task_with_worker_options(
        &root,
        DelegationMode::Explore,
        &task_path,
        &out_dir,
        &runner,
        config.opencode.require_local,
        WorkerRunOptions {
            allow_auto_followups: false,
            ..WorkerRunOptions::default()
        },
    )?;

    let worker_text = extract_last_text(&out_dir.join("logs/opencode.stdout.txt"))
        .unwrap_or_else(|| "No compact worker text was captured.".to_string());
    let metrics = read_json_file(&out_dir.join(METRICS_JSON)).unwrap_or_else(|_| json!({}));
    println!("Mixmod supervisor tool proxy");
    println!("original_command: {}", payload.command.trim());
    println!("worker_status: {}", receipt.status);
    if let Some(exit_status) = metrics.get("opencode_exit_status").and_then(Value::as_u64) {
        println!("worker_exit_status: {exit_status}");
    }
    println!("artifacts: {}", display_path(&root, &out_dir));
    println!("prompt_artifact: {}", OPENCODE_INSTRUCTIONS_MD);
    println!("report_artifact: {}", REPORT_MD);
    println!("worktree_patch_artifact: {}", WORKTREE_PATCH);
    println!();
    println!("worker_summary:");
    println!("{}", compact_text(&worker_text, 6_000));
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct SupervisorToolProxyPayload {
    command: String,
    cwd: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    tool_use_id: Option<String>,
    model: Option<String>,
    config_path: PathBuf,
    created_at: String,
}

impl SupervisorToolProxyPayload {
    fn from_event(command: &str, event: &Value, config_path: PathBuf) -> Self {
        Self {
            command: command.to_string(),
            cwd: get_str(event, "cwd").map(ToOwned::to_owned),
            session_id: get_str(event, "session_id").map(ToOwned::to_owned),
            turn_id: get_str(event, "turn_id").map(ToOwned::to_owned),
            tool_use_id: get_str(event, "tool_use_id").map(ToOwned::to_owned),
            model: get_str(event, "model").map(ToOwned::to_owned),
            config_path,
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

fn bash_command_from_pre_tool_use(event: &Value) -> Option<&str> {
    if get_str(event, "hook_event_name") != Some("PreToolUse") {
        return None;
    }
    if get_str(event, "tool_name") != Some("Bash") {
        return None;
    }
    event
        .get("tool_input")
        .and_then(|input| get_str(input, "command"))
}

fn payload_path(code_home: &Path, payload: &SupervisorToolProxyPayload) -> Result<PathBuf> {
    let turn = sanitize_id(payload.turn_id.as_deref().unwrap_or("turn"));
    let tool = sanitize_id(payload.tool_use_id.as_deref().unwrap_or("tool"));
    let dir = code_home.join(PAYLOAD_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir.join(format!("{turn}-{tool}.json")))
}

fn load_tool_proxy_config(path: &Path, root: &Path) -> Result<MixmodConfig> {
    if path.exists() {
        let value = read_json_file(path)?;
        return serde_json::from_value(value)
            .with_context(|| format!("failed to parse {}", path.display()));
    }
    load_config(root)
}

fn tool_proxy_task(payload: &SupervisorToolProxyPayload) -> Value {
    let role = worker_role_for_command(&payload.command);
    json!({
        "title": format!("Supervisor tool proxy: {}", payload.command.trim()),
        "instructions": format!(
            "A GPT supervisor requested this Bash command:\n\n```bash\n{}\n```\n\nRun exactly that command from the current repository context. Do not edit files, do not commit, and do not run unrelated exploratory commands. Return only the useful minimal result for the supervisor: command, exit status, pass/fail when applicable, and the smallest relevant excerpt or summary. For git diff/status, summarize changed files and notable hunks instead of pasting a long diff.",
            payload.command.trim()
        ),
        "expect_patch": false,
        "tests": [payload.command.trim()],
        "constraints": [
            "Do not edit files.",
            "Do not commit changes.",
            "Do not inspect /solution or verifier internals.",
            "Keep stdout compact.",
            "If the command produces long output, summarize and include only the most relevant failing lines or diff facts."
        ],
        "context": {
            "worker_role": role,
            "expect_patch": false,
            "delegated_from": "codex_pre_tool_use_hook",
            "original_command": payload.command.trim()
        }
    })
}

fn worker_role_for_command(command: &str) -> &'static str {
    let command = command.trim();
    if command.starts_with("git diff") || command.starts_with("git status") {
        "diff_review"
    } else if is_test_or_build_command(command) {
        "run_checks"
    } else {
        "inspect"
    }
}

pub(crate) fn should_proxy_bash_command(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty()
        || command.contains('\n')
        || command.contains(" codex-hook ")
        || command.starts_with("mixmod ")
        || command.starts_with("codex ")
        || command.starts_with("opencode ")
    {
        return false;
    }
    if contains_shell_control(command) || contains_obvious_mutation(command) {
        return false;
    }
    is_inspection_command(command) || is_test_or_build_command(command)
}

fn is_inspection_command(command: &str) -> bool {
    [
        "git diff",
        "git status",
        "git show",
        "git log",
        "rg ",
        "grep ",
        "sed -n ",
        "cat ",
        "ls ",
        "find ",
    ]
    .iter()
    .any(|prefix| command == prefix.trim() || command.starts_with(prefix))
}

fn is_test_or_build_command(command: &str) -> bool {
    [
        "go test",
        "cargo test",
        "cargo check",
        "pytest",
        "python -m pytest",
        "npm test",
        "pnpm test",
        "yarn test",
        "make test",
    ]
    .iter()
    .any(|prefix| command == *prefix || command.starts_with(&format!("{prefix} ")))
}

fn contains_shell_control(command: &str) -> bool {
    ["&&", "||", ";", "|", ">", "<", "`", "$("]
        .iter()
        .any(|token| command.contains(token))
}

fn contains_obvious_mutation(command: &str) -> bool {
    [
        "rm ",
        "mv ",
        "cp ",
        "touch ",
        "chmod ",
        "chown ",
        "git add",
        "git commit",
        "git checkout",
        "git reset",
        "git clean",
        "git apply",
        "gofmt ",
        "ruff --fix",
    ]
    .iter()
    .any(|token| command.starts_with(token) || command.contains(&format!(" {token}")))
}

fn extract_last_text(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| get_str(event, "type") == Some("text"))
        .filter_map(|event| {
            event
                .get("part")
                .and_then(|part| get_str(part, "text"))
                .map(ToOwned::to_owned)
        })
        .last()
}

fn compact_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.trim().to_string();
    }
    let start = text.len().saturating_sub(max_bytes);
    format!("<truncated>\n{}", text[start..].trim())
}

fn remove_managed_hook_files(code_home: &Path) -> Result<()> {
    for path in [
        code_home.join(HOOKS_JSON),
        code_home.join(CONFIG_SNAPSHOT_JSON),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn sanitize_id(value: &str) -> String {
    let sanitized = value
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
        .to_string();
    if sanitized.is_empty() {
        "item".to_string()
    } else {
        sanitized
    }
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxies_evidence_commands() {
        for command in [
            "git diff --stat",
            "git status --short",
            "rg TypeBinding",
            "go test ./vm -run TestVar",
            "cargo test run_writes_full_artifact_bundle",
        ] {
            assert!(should_proxy_bash_command(command), "{command}");
        }
    }

    #[test]
    fn does_not_proxy_mutating_or_complex_commands() {
        for command in [
            "gofmt -w vm/vm.go",
            "git add .",
            "go test ./... && git diff",
            "mixmod codex-hook run-tool-proxy --payload x",
            "opencode run hi",
        ] {
            assert!(!should_proxy_bash_command(command), "{command}");
        }
    }

    #[test]
    fn supervisor_tool_proxy_writes_scoped_hook_files() {
        let temp = tempfile::tempdir().unwrap();
        let code_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&code_home).unwrap();
        let config = MixmodConfig::default();

        let setup = prepare_supervisor_tool_proxy_home(&code_home, Some(&config)).unwrap();

        assert!(setup.enabled);
        assert!(code_home.join(HOOKS_JSON).exists());
        assert!(code_home.join(CONFIG_SNAPSHOT_JSON).exists());
        let hooks = read_json_file(&code_home.join(HOOKS_JSON)).unwrap();
        assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], json!("^Bash$"));
    }
}
