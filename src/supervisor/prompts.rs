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
    init_mode: SupervisorInitMode,
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
    let init_instructions = worker_brief_init_instructions(init_mode);
    Ok(format!(
        r#"You are the supervisor model for a Mixmod worker.
{init_instructions}
The worker receives the original task JSON and can inspect, edit, and test the repo.
Use supervisor reasoning freely, but minimize supervisor output.
{worker_guidance}
If the worker-model guidance names a preferred worker_turn_shape, follow it for the initial expected-patch handoff unless the task is already a trivial one-file edit or no patch is needed.
Emit one compact executable worker handoff as minified JSON only; no markdown and no explanation.
Do not restate the original task. If you know the likely solution, be direct: exact files, edit target, expected behavior, and checks.
Required field: "handoff" = "as_given" | "focused" | "guided" | "blocked".
Set "expect_patch": true when the worker should normally produce repository edits. Set false for investigation/no-change handoffs.
Use exactly {{"handoff":"as_given"}} only when the original task already names the relevant files, desired behavior, and checks clearly enough for the worker.
Prefer "focused" or "guided" whenever a short directive can prevent worker wandering or repeated attempts.
For expected-patch tasks with workers prone to delayed edits, prefer "worker_turn_shape":"small_patch_slice": choose a first patch seed, not the full implementation. Use one or two narrow files when possible, list mechanical exact_edits, defer checks until after a non-empty diff, and include a completion_gate such as "git diff --stat must be non-empty".
Good small_patch_slice choices are repo-generic seed patches: public API/options plumbing plus a narrow test, one parser/config branch plus a narrow test, one validation branch plus a narrow test, or one localized source edit plus a regression test. Bad small_patch_slice choices ask for a whole feature, core algorithm, validation, aliases, optional/default behavior, and full tests in one turn.
For small_patch_slice, exact_edits must be immediately executable edit commands. Do not write "locate", "investigate", "understand", or broad algorithm work as an exact edit. The files array must contain concrete repo file paths, not directories; include the file that defines any function, option, flag, or public API named in exact_edits.
Optional fields; omit empty fields:
{{"expect_patch":true,"worker_turn_shape":"small_patch_slice|default","turn_goal":"one-turn goal","message_to_worker":"direct message for the worker","files":["optional paths"],"exact_edits":["concrete edit"],"checks":["optional checks"],"deferred_checks":["checks to run after a patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"git diff --stat must be non-empty","forbidden_actions":["ask questions","run tests before editing"],"investigation_summary":"optional short finding","edit_plan":["optional concrete steps"],"evidence":["optional file/function clues"],"avoid":["optional constraints"],"risk":"optional short risk"}}
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

fn worker_brief_init_instructions(init_mode: SupervisorInitMode) -> &'static str {
    match init_mode {
        SupervisorInitMode::Compact => {
            r#"Use the provided file context. Do not edit files. Do not run tests. Do not implement the patch. Do not ask the user for approval.
Default to "guided". Guided means terse and executable, not advisory:
- target <=120 output tokens for the whole JSON on normal tasks
- one command-style message_to_worker, ideally <=45 words
- files only when useful, usually <=3
- checks only when useful, usually <=2
- when using worker_turn_shape=small_patch_slice, emit exact_edits instead of a broad plan, usually use <=2 files and <=5 mechanical edits, and omit checks unless they are explicitly deferred
- omit investigation_summary, edit_plan, evidence, avoid, and risk unless one short phrase prevents a likely wrong patch
Assume the local worker is capable but prone to setup rabbit holes, broad exploration, and delayed edits."#
        }
        SupervisorInitMode::Investigate => {
            r#"First do a read-only repo investigation before writing the handoff. You may inspect source/test files and run read-only discovery commands such as `rg`, `find`, `ls`, `sed`, or `git grep`. Do not edit files. Do not run tests. Do not install dependencies. Do not inspect Mixmod state/artifact directories. Do not ask the user for approval.
Default to "guided". Guided means concrete enough for a weaker worker to edit without broad exploration:
- target <=500 output tokens for the whole JSON
- one command-style message_to_worker, ideally <=70 words
- files should name the likely source/test targets, usually <=5
- checks should name the narrowest useful commands, usually <=3
- include investigation_summary with the likely root cause or target behavior
- include edit_plan when it can prevent worker wandering, usually <=4 short steps
- include evidence when file/function clues matter, usually <=4 short bullets
- when using worker_turn_shape=small_patch_slice, make the first slice small enough to edit immediately, usually use <=2 files and <=5 mechanical edits, put exact_edits in command form, and defer tests until a non-empty diff exists
Assume the local worker is less capable, prone to setup rabbit holes, broad exploration, delayed edits, and premature final answers."#
        }
    }
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
