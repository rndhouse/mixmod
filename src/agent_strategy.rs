use std::path::Path;
use std::time::{Instant, SystemTime};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::json;

use crate::task::{read_task_json, write_agent_visible_task_file};
use crate::{
    FINAL_PATCH, METRICS_JSON, MixmodConfig, ModelOverrides, REPORT_MD, SupervisorCodexSession,
    TASK_JSON, TOOL_OUTPUT_DIR, WORKTREE_PATCH, WorkerSupervisorGuidance, absolutize, atomic_write,
    budgeted_report, display_path, git_diff_with_untracked, load_config, patch_stats, state_layout,
    write_pretty_json,
};

/// Options for the primary Codex-agent strategy.
pub(crate) struct AgentStrategyOptions {
    /// Per-run model choices supplied by CLI flags.
    pub(crate) model_overrides: ModelOverrides,
    /// Disable local-inference verification for worker-tool calls.
    pub(crate) no_require_local: bool,
}

/// Run the primary Mixmod strategy: Codex owns the solve and Mixmod provides
/// low-cost local worker routing as a tool.
pub(crate) fn run_agent_strategy(
    root: &Path,
    task_arg: &Path,
    out_dir: &Path,
    options: AgentStrategyOptions,
) -> Result<()> {
    AgentStrategyRun {
        root,
        task_arg,
        out_dir,
        options,
    }
    .execute()
}

struct AgentStrategyRun<'a> {
    root: &'a Path,
    task_arg: &'a Path,
    out_dir: &'a Path,
    options: AgentStrategyOptions,
}

impl AgentStrategyRun<'_> {
    fn execute(self) -> Result<()> {
        let Self {
            root,
            task_arg,
            out_dir,
            options,
        } = self;
        let run_start = Utc::now();
        let run_start_time = SystemTime::now();
        let start = Instant::now();
        let out_dir = absolutize(root, out_dir);
        fs::create_dir_all(&out_dir).with_context(|| {
            format!("failed to create agent strategy dir {}", out_dir.display())
        })?;

        let mut config = load_config(root)?;
        options.model_overrides.apply_to_config(&mut config)?;
        if options.no_require_local {
            config.opencode.require_local = false;
            config.opencode.local_verification.enabled = false;
        }

        let task_file = out_dir.join(TASK_JSON);
        write_agent_visible_task_file(&absolutize(root, task_arg), &task_file)?;
        let (task_json, _task) = read_task_json(&task_file)?;

        let prompt = agent_strategy_prompt(root, &task_file, &task_json, &config);
        atomic_write(&out_dir.join("agent-prompt.md"), prompt.as_bytes())?;

        let supervisor = config.supervisor.clone();
        let mut supervisor_session = SupervisorCodexSession::start(root, &supervisor, None)?;
        let result = supervisor_session.run_turn(&out_dir, "agent", &prompt)?;

        let patch = git_diff_with_untracked(root).unwrap_or_else(|error| {
            format!("Unable to capture git diff after Codex agent run: {error}\n")
        });
        atomic_write(&out_dir.join(FINAL_PATCH), patch.as_bytes())?;
        atomic_write(&out_dir.join(WORKTREE_PATCH), patch.as_bytes())?;
        let stats = patch_stats(&patch);
        let changed_files = stats.files.clone();
        let success = result.exit_status == Some(0);
        let codex_turn_status = result.turn_status.clone();
        let codex_error_info = result.error_info.clone();
        let codex_error_message = result.error_message.clone();
        let final_status = agent_final_status(success, codex_error_info.as_deref());
        let worker_tool_metrics = worker_tool_proxy_metrics(root, run_start_time);
        let metrics = json!({
            "kind": "mixmod-agent-strategy",
            "recorded_at": Utc::now().to_rfc3339(),
            "start_timestamp": run_start.to_rfc3339(),
            "end_timestamp": Utc::now().to_rfc3339(),
            "wall_clock_ms": start.elapsed().as_millis(),
            "supervisor_model": supervisor.model,
            "supervisor_reasoning_effort": supervisor.reasoning_effort,
            "supervisor_input_tokens": result.usage.input_tokens,
            "supervisor_output_tokens": result.usage.output_tokens,
            "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
            "supervisor_total_tokens": result.usage.total_tokens,
            "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
            "supervisor_input_bytes_fallback": result.input_bytes,
            "supervisor_output_bytes_fallback": result.output_bytes,
            "codex_visible_bytes": result.input_bytes,
            "supervision_turn_count": 1,
            "codex_calls": 1,
            "codex_backend": "app-server-persistent",
            "codex_app_server_thread_ids": [result.thread_id],
            "codex_app_server_turn_ids": [result.turn_id],
            "codex_app_server_thread_count": 1,
            "codex_turn_status": codex_turn_status,
            "codex_error_info": codex_error_info,
            "codex_error_message": codex_error_message,
            "supervisor_token_usage_source": result.token_usage_source,
            "supervisor_session_reused": false,
            "supervisor_resume_count": 0,
            "strategy_phases": ["codex_primary_agent"],
            "mixmod_delegations": worker_tool_metrics.call_count,
            "opencode_calls": worker_tool_metrics.call_count,
            "worker_backend": config.worker.backend.as_str(),
            "opencode_provider": config.opencode.provider,
            "opencode_model": config.opencode.model,
            "opencode_model_arg": format!("{}/{}", config.opencode.provider, config.opencode.model),
            "require_local": config.opencode.require_local,
            "local_inference_verified": worker_tool_metrics.call_count > 0 && config.opencode.require_local,
            "local_worker_stdout_bytes": worker_tool_metrics.stdout_bytes,
            "local_worker_stderr_bytes": worker_tool_metrics.stderr_bytes,
            "local_worker_text_bytes": worker_tool_metrics.stdout_bytes + worker_tool_metrics.stderr_bytes,
            "local_worker_reasoning_trace_bytes": worker_tool_metrics.reasoning_trace_bytes,
            "local_worker_tool_events_bytes": worker_tool_metrics.tool_events_bytes,
            "local_worker_tool_output_artifact_count": worker_tool_metrics.tool_output_artifact_count,
            "local_worker_tool_output_artifact_bytes": worker_tool_metrics.tool_output_artifact_bytes,
            "artifact_byte_sizes": artifact_byte_sizes(&out_dir)?,
            "patch_bytes": patch.len() as u64,
            "changed_files": changed_files,
            "changed_file_count": stats.files.len(),
            "changed_line_count": stats.changed_line_count,
            "final_status": final_status,
            "final_verdict": if success { "completed" } else { "failed" },
            "final_codex_action": if success { "completed" } else { "failed" },
            "terminal_reject": false,
            "needs_worker_revision": false,
            "notes": [
                "Agent strategy runs Codex as the primary task-solving agent.",
                "Mixmod provides a CLI tool that can route bounded command evidence requests to the configured worker.",
                "The worker is a low-cost tool; it is not the owner of the implementation loop."
            ]
        });
        write_pretty_json(
            &out_dir.join(METRICS_JSON),
            &metrics,
            "agent strategy metrics",
        )?;
        atomic_write(
            &out_dir.join(REPORT_MD),
            budgeted_report("exec", &metrics).as_bytes(),
        )?;

        println!(
            "Mixmod exec wrote artifacts to {}",
            display_path(root, &out_dir)
        );
        println!("status: {}", final_status);
        println!("report: {}", display_path(root, &out_dir.join(REPORT_MD)));
        println!("patch: {}", display_path(root, &out_dir.join(FINAL_PATCH)));
        if !success {
            bail!(
                "Codex primary-agent turn failed; artifacts were written to {}",
                out_dir.display()
            );
        }
        Ok(())
    }
}

fn agent_final_status(success: bool, codex_error_info: Option<&str>) -> &'static str {
    if success {
        "completed_by_codex"
    } else if codex_error_info == Some("usageLimitExceeded") {
        "codex_usage_limit_exceeded"
    } else if codex_error_info.is_some() {
        "codex_system_error"
    } else {
        "codex_failed"
    }
}

fn agent_strategy_prompt(
    root: &Path,
    task_file: &Path,
    task_json: &serde_json::Value,
    config: &MixmodConfig,
) -> String {
    let worker_guidance = render_worker_tool_guidance(&config.worker_supervisor_guidance());
    let mixmod_tool_command = env::current_exe()
        .map(|path| shell_quote_path(&path))
        .unwrap_or_else(|_| "mixmod".to_string());
    let title = task_json
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("Mixmod task");
    let instructions = task_json
        .get("instructions")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    format!(
        r#"You are the primary Codex agent for this Mixmod run.

Complete the task in the working repository. You may inspect files, edit files,
run focused checks, and use git commands directly. Do not ask the user for
approval. Do not commit. Leave the final solution as an uncommitted git diff so
Mixmod can capture it.

Mixmod is a tool/router, not a separate supervisor. The configured local worker
is available as a cheap helper through Mixmod's CLI. It is effectively zero
marginal GPT-token cost, so prefer it for bounded repo evidence when that saves
you reading long files or command output directly. When a bounded command can
save GPT tokens, call:

```bash
{mixmod_tool_command} tool run-command --command "git status --short"
```

Use it for low-risk inspection/check evidence such as `rg`, `sed -n`,
`git diff`, `git status`, `go test`, `cargo test`, and similar. Use your own
tools directly when you need exact control, editing, or final judgment.
When routing commands through the local worker, prefer bounded commands that
return compact evidence: narrow paths or globs, `rg --max-count`, targeted
`sed -n` ranges, and package-level checks. Avoid broad repository-wide searches
with many alternations when a smaller probe would answer the question.
For bounded review or investigation that is not naturally one command, call:

```bash
{mixmod_tool_command} tool ask --prompt "Inspect the final diff for missing edge cases in the requested behavior. Do not read the full diff; use targeted hunks or grep. Derive at most three focused probes from the requirements and changed code paths, prefer changed branches or alternate input shapes visible tests may skip, run probes before broad analysis, stop after one concrete issue or after those probes finish, and return compact evidence."
```

For a substantial semantic diff, do not finish solely from visible happy-path
checks. Before final completion, use at least one cheap local-worker call for
failure-oriented post-diff review unless you already performed an equivalent
check yourself. Good final probes ask the worker to inspect the final diff
against the requested behavior, identify missing edge cases, and derive at most
three small behavior probes when the repository has a cheap way to run them.
Probe changed branches and alternate input shapes, especially paths the visible
tests do not exercise. Ask for targeted hunks or grep rather than a full diff,
and ask the worker to run probes before broad analysis. For test probes, prefer
the changed package's full test suite or the exact tests you added/changed;
avoid narrow regexes that can skip new tests unless there is a clear cost
reason. Treat the worker's output as evidence to use or reject; final task
completion is your responsibility.

Each local-worker call prints an artifact directory. Inspect those artifacts
when the compact summary is insufficient; they include the rendered worker
prompt, stdout/stderr logs, reasoning trace when available, tool events, and
patch files.

Worker guidance:
{worker_guidance}

Before finishing, check the resulting diff against the original task. If checks
cannot run, record the blocker in your final response. Your final response should
be concise and include what changed and what verification you performed.

Working repo: {root}
Task artifact: {task_file}

Task title: {title}

Task instructions:
{instructions}
"#,
        root = root.display(),
        task_file = task_file.display(),
    )
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn render_worker_tool_guidance(guidance: &WorkerSupervisorGuidance) -> String {
    if guidance.guidance.is_empty() {
        return "- No worker-specific profile is configured.".to_string();
    }
    let mut lines = vec![format!("- Worker model/profile: {}", guidance.model)];
    lines.extend(guidance.guidance.iter().map(|item| format!("- {item}")));
    lines.join("\n")
}

#[derive(Default)]
struct WorkerToolProxyMetrics {
    call_count: u64,
    stdout_bytes: u64,
    stderr_bytes: u64,
    reasoning_trace_bytes: u64,
    tool_events_bytes: u64,
    tool_output_artifact_count: u64,
    tool_output_artifact_bytes: u64,
}

fn worker_tool_proxy_metrics(root: &Path, since: SystemTime) -> WorkerToolProxyMetrics {
    let mut metrics = WorkerToolProxyMetrics::default();
    let proxy_root = state_layout(root)
        .project_dir()
        .join("supervisor-tool-proxy");
    let Ok(entries) = fs::read_dir(proxy_root) else {
        return metrics;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        for run in walk_dirs(&path) {
            let metrics_path = run.join(METRICS_JSON);
            if metrics_path.exists() && modified_at_or_after(&metrics_path, since) {
                metrics.call_count += 1;
                metrics.stdout_bytes += file_len_or_zero(&run.join("logs/opencode.stdout.txt"));
                metrics.stderr_bytes += file_len_or_zero(&run.join("logs/opencode.stderr.txt"));
                metrics.reasoning_trace_bytes +=
                    file_len_or_zero(&run.join("reasoning-trace.jsonl"));
                metrics.tool_events_bytes += file_len_or_zero(&run.join("tool-events.jsonl"));
                let tool_output = tool_output_artifact_metrics(&run.join(TOOL_OUTPUT_DIR));
                metrics.tool_output_artifact_count += tool_output.count;
                metrics.tool_output_artifact_bytes += tool_output.bytes;
            }
        }
    }
    metrics
}

#[derive(Default)]
struct FileTreeMetrics {
    count: u64,
    bytes: u64,
}

fn tool_output_artifact_metrics(path: &Path) -> FileTreeMetrics {
    let mut metrics = FileTreeMetrics::default();
    if !path.exists() {
        return metrics;
    }
    for file in walk_files(path) {
        metrics.count += 1;
        metrics.bytes += file_len_or_zero(&file);
    }
    metrics
}

fn walk_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = stack.pop() {
        if path.is_file() {
            files.push(path);
            continue;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            stack.push(entry.path());
        }
    }
    files
}

fn walk_dirs(root: &Path) -> Vec<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = Vec::new();
    while let Some(dir) = stack.pop() {
        dirs.push(dir.clone());
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    dirs
}

fn file_len_or_zero(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn modified_at_or_after(path: &Path, since: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| modified >= since)
        .unwrap_or(false)
}

fn artifact_byte_sizes(dir: &Path) -> Result<serde_json::Value> {
    let mut sizes = serde_json::Map::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            sizes.insert(name.to_string(), json!(entry.metadata()?.len()));
        }
    }
    Ok(serde_json::Value::Object(sizes))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn agent_strategy_prompt_makes_codex_primary_and_worker_a_tool() {
        let config = MixmodConfig::default();
        let prompt = agent_strategy_prompt(
            Path::new("/repo"),
            Path::new("/state/task.json"),
            &json!({
                "title": "Test task",
                "instructions": "Change the code."
            }),
            &config,
        );

        assert!(prompt.contains("You are the primary Codex agent"));
        assert!(prompt.contains("Mixmod is a tool/router"));
        assert!(prompt.contains("local worker"));
        assert!(prompt.contains("cheap helper"));
        assert!(prompt.contains("zero marginal GPT-token cost"));
        assert!(prompt.contains("prefer bounded commands"));
        assert!(prompt.contains("rg --max-count"));
        assert!(prompt.contains("failure-oriented post-diff review"));
        assert!(prompt.contains("Derive at most three focused probes"));
        assert!(prompt.contains("changed code paths"));
        assert!(prompt.contains("Probe changed branches"));
        assert!(prompt.contains("targeted hunks or grep"));
        assert!(prompt.contains("run probes before broad analysis"));
        assert!(prompt.contains("changed package's full test"));
        assert!(prompt.contains("suite or the exact tests"));
        assert!(prompt.contains("completion is your responsibility"));
        assert!(prompt.contains("tool run-command"));
        assert!(prompt.contains("tool ask"));
        assert!(!prompt.contains("Return only JSON"));
        assert!(!prompt.contains("\"action\":\"approve|revise|stop\""));
    }

    #[test]
    fn agent_final_status_distinguishes_codex_usage_limits() {
        assert_eq!(agent_final_status(true, None), "completed_by_codex");
        assert_eq!(
            agent_final_status(false, Some("usageLimitExceeded")),
            "codex_usage_limit_exceeded"
        );
        assert_eq!(
            agent_final_status(false, Some("serverError")),
            "codex_system_error"
        );
        assert_eq!(agent_final_status(false, None), "codex_failed");
    }
}
