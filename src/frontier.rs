use std::collections::BTreeSet;

use crate::harness::codex::{CodexAppServer, CodexSandbox, CodexTurnResult};
use crate::*;

pub(crate) fn codex_only_prompt(work_dir: &Path, task: &Value) -> Result<String> {
    let visible_task = agent_visible_task_value(task);
    let task_json = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for Codex-only prompt")?;
    Ok(format!(
        r#"You are the Codex-only baseline for a Mixmod experiment.
Solve the task directly in this repo. Edit files as needed, run the requested tests, and keep the final answer compact.
Do not use Mixmod or OpenCode.
Working repo: {work_dir}

Task JSON:
```json
{}
```
"#,
        task_json,
        work_dir = work_dir.display()
    ))
}

pub(crate) struct FrontierFeedbackTurn {
    pub(crate) feedback: Value,
    pub(crate) verdict: String,
    pub(crate) worker_mode: String,
    pub(crate) patch_decision: String,
    pub(crate) hint: String,
    pub(crate) focus_files: Vec<String>,
    pub(crate) required_checks: Vec<String>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Debug)]
pub(crate) struct FrontierBriefTurn {
    pub(crate) record: Value,
    pub(crate) brief: Value,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Clone)]
pub(crate) struct FrontierUsageSample {
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: u64,
    input_bytes: u64,
    output_bytes: u64,
    thread_id: String,
    turn_id: String,
}

impl FrontierFeedbackTurn {
    pub(crate) fn usage_sample(&self) -> FrontierUsageSample {
        FrontierUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
        }
    }
}

impl FrontierBriefTurn {
    pub(crate) fn usage_sample(&self) -> FrontierUsageSample {
        FrontierUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct FrontierUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) turn_count: u64,
    pub(crate) thread_ids: Vec<String>,
    pub(crate) turn_ids: Vec<String>,
}

impl FrontierUsage {
    pub(crate) fn thread_count(&self) -> u64 {
        self.thread_ids
            .iter()
            .filter(|id| !id.is_empty())
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .len() as u64
    }

    pub(crate) fn thread_reuse_count(&self) -> u64 {
        let observed_thread_ids = self.thread_ids.iter().filter(|id| !id.is_empty()).count() as u64;
        let thread_count = self.thread_count();
        observed_thread_ids.saturating_sub(thread_count)
    }

    pub(crate) fn session_reused(&self) -> bool {
        self.thread_reuse_count() > 0
    }
}

pub(crate) fn run_frontier_brief_turn(
    work_dir: &Path,
    default_dir: &Path,
    task_path: &Path,
    frontier: &FrontierConfig,
) -> Result<FrontierBriefTurn> {
    let prompt = frontier_worker_brief_prompt(work_dir, task_path)?;
    let result = run_codex_app_server_turn(
        work_dir,
        default_dir,
        "worker-brief",
        &prompt,
        frontier,
        CodexSandbox::ReadOnly,
    )?;
    let parsed_brief = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "handoff": "blocked",
            "message_to_worker": "Codex did not return parseable handoff JSON.",
            "risk": truncate_for_report(&result.last_message, 160)
        })
    });
    let thread_id = result.thread_id.clone();
    let turn_id = result.turn_id.clone();
    let record = json!({
        "label": "worker-brief",
        "timestamp": Utc::now().to_rfc3339(),
        "brief": parsed_brief,
        "codex_exit_status": result.exit_status,
        "frontier_model": result.model.clone(),
        "frontier_reasoning_effort": result.reasoning_effort.clone(),
        "frontier_input_tokens": result.usage.input_tokens,
        "frontier_output_tokens": result.usage.output_tokens,
        "frontier_reasoning_tokens": result.usage.reasoning_tokens,
        "frontier_total_tokens": result.usage.total_tokens,
        "frontier_cached_input_tokens": result.usage.cached_input_tokens,
        "input_bytes": result.input_bytes,
        "output_bytes": result.output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "codex_app_server_thread_id": thread_id.clone(),
        "codex_app_server_turn_id": turn_id.clone()
    });
    Ok(FrontierBriefTurn {
        record,
        brief: parsed_brief,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id,
        turn_id,
    })
}

pub(crate) fn run_frontier_feedback_turn(
    work_dir: &Path,
    budgeted_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    instruction: &str,
    frontier: &FrontierConfig,
) -> Result<FrontierFeedbackTurn> {
    let prompt = frontier_feedback_prompt(work_dir, artifact_paths, instruction)?;
    let result = run_codex_app_server_turn(
        work_dir,
        budgeted_dir,
        label,
        &prompt,
        frontier,
        CodexSandbox::ReadOnly,
    )?;
    let parsed_feedback = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "action": if result.exit_status == Some(0) { "approve" } else { "revise" },
            "worker_mode": "continue",
            "message_to_worker": truncate_for_report(&result.last_message, 180),
            "focus_files": [],
            "required_checks": [],
            "risk": if result.exit_status == Some(0) { "none recorded" } else { "codex feedback command failed" }
        })
    });
    let (mut parsed_feedback, verdict) = normalize_feedback_value(parsed_feedback);
    let typed_feedback = FrontierFeedback::from_value(&parsed_feedback);
    let worker_mode = normalize_worker_mode(typed_feedback.worker_mode.as_deref());
    let patch_decision = normalize_patch_decision(typed_feedback.patch_decision.as_deref());
    if let Value::Object(map) = &mut parsed_feedback {
        map.insert("worker_mode".to_string(), json!(worker_mode.clone()));
        map.insert("patch_decision".to_string(), json!(patch_decision.clone()));
    }
    let thread_id = result.thread_id.clone();
    let turn_id = result.turn_id.clone();
    let turn = FrontierFeedbackTurn {
        verdict,
        worker_mode,
        patch_decision,
        hint: typed_feedback
            .message_to_worker
            .or(typed_feedback.hint)
            .unwrap_or_default(),
        focus_files: typed_feedback.focus_files,
        required_checks: typed_feedback.required_checks,
        feedback: json!({
            "label": label,
            "timestamp": Utc::now().to_rfc3339(),
            "feedback": parsed_feedback,
            "codex_exit_status": result.exit_status,
            "frontier_model": result.model.clone(),
            "frontier_reasoning_effort": result.reasoning_effort.clone(),
            "frontier_input_tokens": result.usage.input_tokens,
            "frontier_output_tokens": result.usage.output_tokens,
            "frontier_reasoning_tokens": result.usage.reasoning_tokens,
            "frontier_total_tokens": result.usage.total_tokens,
            "frontier_cached_input_tokens": result.usage.cached_input_tokens,
            "input_bytes": result.input_bytes,
            "output_bytes": result.output_bytes,
            "auth_copied_then_removed": result.auth_copied_then_removed,
            "codex_app_server_thread_id": thread_id.clone(),
            "codex_app_server_turn_id": turn_id.clone()
        }),
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id,
        turn_id,
    };
    Ok(turn)
}

pub(crate) fn run_codex_app_server_turn(
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    prompt: &str,
    frontier: &FrontierConfig,
    sandbox: CodexSandbox,
) -> Result<CodexTurnResult> {
    let mut server = CodexAppServer::start(work_dir, frontier, sandbox)?;
    server.run_turn(artifact_dir, label, prompt)
}

pub(crate) fn frontier_worker_brief_prompt(work_dir: &Path, task_path: &Path) -> Result<String> {
    let task_value = read_json_file(task_path)?;
    let visible_task = agent_visible_task_value(&task_value);
    let task = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for worker brief prompt")?;
    let mut file_context = String::new();
    for file in get_string_array(&visible_task, "files") {
        let path = work_dir.join(&file);
        if !path.exists() || path.is_dir() {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_else(|error| format!("missing: {error}"));
        file_context.push_str(&format!(
            "\n## {file}\n\n```text\n{}\n```\n",
            truncate_for_report(&text, 2200)
        ));
    }
    Ok(format!(
        r#"You are Codex supervising a local OpenCode worker.
Use the provided file context. Do not edit files. Do not run tests. Do not implement the patch. Do not ask the user for approval.
OpenCode receives the original task JSON and can inspect, edit, and test the repo.
Use frontier intelligence freely through reading and reasoning, but minimize frontier output.
Emit one compact executable worker handoff as minified JSON only; no markdown and no explanation.
Do not restate the original task. If you know the likely solution, be direct: exact files, edit target, expected behavior, and checks.
Required field: "handoff" = "as_given" | "focused" | "guided" | "blocked".
Default to "guided". Guided means terse and executable, not advisory:
- target <=120 output tokens for the whole JSON on normal tasks
- one command-style message_to_worker, ideally <=45 words
- files only when useful, usually <=3
- checks only when useful, usually <=2
- omit avoid and risk unless one short phrase prevents a likely wrong patch
Assume the local worker is capable but prone to setup rabbit holes, broad exploration, and delayed edits.
Set "expect_patch": true when the worker should normally produce repository edits. Set false for investigation/no-change handoffs.
Use exactly {{"handoff":"as_given"}} only when the original task already names the relevant files, desired behavior, and checks clearly enough for OpenCode.
Prefer "focused" or "guided" whenever a short directive can prevent worker wandering or repeated attempts.
Optional fields; omit empty fields:
{{"expect_patch":true,"message_to_worker":"direct message for OpenCode","files":["optional paths"],"checks":["optional checks"],"avoid":["optional constraints"],"risk":"optional short risk"}}
Working repo: {work_dir}

Task JSON:
```json
{task}
```

File context:
{file_context}
"#,
        work_dir = work_dir.display(),
    ))
}

pub(crate) fn frontier_feedback_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
) -> Result<String> {
    let mut artifacts = String::new();
    for path in artifact_paths {
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("artifact");
        let text = fs::read_to_string(path).unwrap_or_else(|error| format!("missing: {error}"));
        artifacts.push_str(&format!(
            "\n## {name}\n\n```text\n{}\n```\n",
            truncate_for_report(&text, 6000)
        ));
    }
    Ok(format!(
        r#"You are a terse frontier critic supervising a local worker.
Do not implement code. Do not edit files. Do not ask the user for approval.
Return only JSON matching this schema:
{{"action":"approve|revise|stop","worker_mode":"continue|context_focus","patch_decision":"accept_current|revise_current|revise_previous","message_to_worker":"max 60 words","focus_files":[],"required_checks":[],"risk":"max 25 words"}}
Use approve when no more local worker attempts are needed.
Prefer revise after failed, empty, distracted, or incomplete worker attempts, and put the next worker instruction in message_to_worker.
Use worker_mode=continue to keep the same OpenCode session and let the worker continue with its existing context.
Use worker_mode=context_focus to start a new OpenCode session on the same worktree; previous worker context is discarded unless you repeat it in message_to_worker.
When patch-comparison.json is present, choose patch_decision explicitly. Use accept_current when the current worktree.patch should stand, revise_current when the current patch should be edited further, and revise_previous when previous-worktree.patch is the better candidate. Mixmod will not mutate the repo directly from this choice. If you choose revise_previous, summarize the concrete source/test edits to recover in message_to_worker; do not tell the worker to read previous-worktree.patch or any Mixmod artifact.
Put only repo source/test paths in focus_files. Do not put Mixmod artifacts such as revision-task JSON files in focus_files. Do not ask the worker to inspect Mixmod state or artifact directories.
Important artifact semantics: worktree.patch is the accumulated current repository diff and is authoritative for deciding whether the patch exists; changes.patch is only the latest worker run delta and may be empty after a verification-only revision.
Use stop only to record a blocked or inconclusive local-worker result when no useful OpenCode path remains. Stop does not permit direct Codex editing.
Working repo: {work_dir}
Instruction: {instruction}
{artifacts}
"#,
        work_dir = work_dir.display(),
    ))
}

fn parse_feedback_json(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        return Some(value);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

pub(crate) fn normalize_feedback_value(mut value: Value) -> (Value, String) {
    let raw = get_str(&value, "verdict")
        .or_else(|| get_str(&value, "action"))
        .unwrap_or("revise")
        .to_string();
    let verdict = normalize_frontier_verdict(&raw);
    if let Value::Object(map) = &mut value {
        if raw != verdict {
            map.insert("raw_verdict".to_string(), json!(raw));
        }
        map.insert("verdict".to_string(), json!(verdict.clone()));
        map.insert("action".to_string(), json!(verdict.clone()));
    }
    (value, verdict)
}

fn normalize_frontier_verdict(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => "approve".to_string(),
        "stop" | "stopped" | "halt" | "done" | "needs_user" | "needs-user" => "stop".to_string(),
        "revise" | "revision" | "needs_revision" | "needs-review" | "needs_review" | "reject"
        | "rejected" => "revise".to_string(),
        _ => "revise".to_string(),
    }
}

fn normalize_patch_decision(value: Option<&str>) -> String {
    match value
        .unwrap_or("accept_current")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "revise_previous" | "previous" | "keep_previous" | "restore_previous"
        | "recover_previous" => "revise_previous".to_string(),
        "revise_current" | "current_revision" | "continue_current" => "revise_current".to_string(),
        _ => "accept_current".to_string(),
    }
}

pub(crate) fn normalize_worker_mode(value: Option<&str>) -> String {
    match value
        .unwrap_or("continue")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "context_focus" | "focused" | "focus" | "fresh" | "reset" => "context_focus".to_string(),
        _ => "continue".to_string(),
    }
}

pub(crate) fn aggregate_frontier_usage(turns: &[FrontierUsageSample]) -> FrontierUsage {
    let mut usage = FrontierUsage::default();
    for turn in turns {
        usage.input_tokens += turn.input_tokens;
        usage.output_tokens += turn.output_tokens;
        usage.reasoning_tokens += turn.reasoning_tokens;
        usage.total_tokens += turn.total_tokens;
        usage.cached_input_tokens += turn.cached_input_tokens;
        usage.input_bytes += turn.input_bytes;
        usage.output_bytes += turn.output_bytes;
        usage.turn_count += 1;
        usage.thread_ids.push(turn.thread_id.clone());
        usage.turn_ids.push(turn.turn_id.clone());
    }
    usage
}
