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

pub(crate) fn supervisor_worker_brief_prompt(
    work_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
) -> Result<String> {
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
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are the supervisor model for a Mixmod worker.
Use the provided file context. Do not edit files. Do not run tests. Do not implement the patch. Do not ask the user for approval.
The worker receives the original task JSON and can inspect, edit, and test the repo.
Use supervisor reasoning freely, but minimize supervisor output.
{worker_guidance}
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
Use exactly {{"handoff":"as_given"}} only when the original task already names the relevant files, desired behavior, and checks clearly enough for the worker.
Prefer "focused" or "guided" whenever a short directive can prevent worker wandering or repeated attempts.
Optional fields; omit empty fields:
{{"expect_patch":true,"message_to_worker":"direct message for the worker","files":["optional paths"],"checks":["optional checks"],"avoid":["optional constraints"],"risk":"optional short risk"}}
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

pub(crate) fn supervisor_feedback_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
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
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are a terse supervisor reviewing a local worker.
Do not implement code. Do not edit files. Do not ask the user for approval.
{worker_guidance}
Return only JSON matching this schema:
{{"action":"approve|revise|stop","worker_mode":"continue|context_focus","patch_decision":"accept_current|revise_current|revise_previous","message_to_worker":"max 60 words","focus_files":[],"required_checks":[],"risk":"max 25 words"}}
Use approve when no more local worker attempts are needed.
Prefer revise after failed, empty, distracted, or incomplete worker attempts, and put the next worker instruction in message_to_worker.
Use worker_mode=continue to keep the same worker session and let the worker continue with its existing context.
Use worker_mode=context_focus to start a new worker session on the same worktree; previous worker context is discarded unless you repeat it in message_to_worker.
When patch-comparison.json is present, choose patch_decision explicitly. Use accept_current when the current worktree.patch should stand, revise_current when the current patch should be edited further, and revise_previous when previous-worktree.patch is the better candidate. Mixmod will not mutate the repo directly from this choice. If you choose revise_previous, summarize the concrete source/test edits to recover in message_to_worker; do not tell the worker to read previous-worktree.patch or any Mixmod artifact.
Put only repo source/test paths in focus_files. Do not put Mixmod artifacts such as revision-task JSON files in focus_files. Do not ask the worker to inspect Mixmod state or artifact directories.
Important artifact semantics: worktree.patch is the accumulated current repository diff and is authoritative for deciding whether the patch exists; changes.patch is only the latest worker run delta and may be empty after a verification-only revision.
Use stop only to record a blocked or inconclusive worker result when no useful worker path remains. Stop does not permit direct supervisor editing.
Working repo: {work_dir}
Instruction: {instruction}
{artifacts}
"#,
        work_dir = work_dir.display(),
    ))
}

fn render_worker_guidance(worker_guidance: &WorkerSupervisorGuidance) -> String {
    if worker_guidance.is_empty() {
        return String::new();
    }
    let mut rendered = format!(
        "Supervisor-only worker-model guidance for {}:\nThese are historical pitfalls for the selected worker model. Treat them as priors when planning the worker handoff or critique. Do not copy every bullet to the worker. Select only relevant points and convert them into short, concrete worker instructions.\n",
        worker_guidance.model
    );
    for item in &worker_guidance.guidance {
        rendered.push_str("- ");
        rendered.push_str(item.trim());
        rendered.push('\n');
    }
    rendered
}
