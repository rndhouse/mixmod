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
For expected-patch tasks where the selected worker guidance prefers "bounded_feature_slice", choose one coherent feature chunk: usually one to three source files, related API/source/test edits that belong together, and focused checks after the patch exists. Use exact_edits or edit_plan as a short ordered plan, not as one-line micromanagement.
For expected-patch tasks with workers that still need very small recovery steps, "worker_turn_shape":"small_patch_slice" means a first patch seed, not the full implementation. Use one or two narrow files when possible, list mechanical exact_edits, defer checks until after a non-empty diff, and include a completion_gate such as "git diff --stat must be non-empty".
Good small_patch_slice choices are repo-generic seed patches: public API/options plumbing plus a narrow test, one parser/config branch plus a narrow test, one validation branch plus a narrow test, or one localized source edit plus a regression test. Bad small_patch_slice choices ask for a whole feature, core algorithm, validation, aliases, optional/default behavior, and full tests in one turn.
For complex source tasks involving generated code, alias/key behavior, validation matrices, serializers/deserializers, pack/unpack paths, parser/compiler behavior, or multiple cross-cutting flows, do not use worker_turn_shape="default" for the initial worker. Prefer bounded_feature_slice for capable workers; use small_patch_slice only when the selected worker guidance asks for it or a previous turn was confused, destructive, or empty.
For small_patch_slice, exact_edits must be immediately executable edit commands. Do not write "locate", "investigate", "understand", or broad algorithm work as an exact edit. The files array must contain concrete repo file paths, not directories; include the file that defines any function, option, flag, or public API named in exact_edits.
For source edits inside large functions or code-generation paths, add structure-preserving constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete/reindent unrelated branches, and edit only the focused block.
Optional fields; omit empty fields:
{{"expect_patch":true,"worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"one-turn goal","message_to_worker":"direct message for the worker","files":["optional paths"],"exact_edits":["concrete edit"],"edit_plan":["optional concrete steps"],"checks":["optional checks"],"deferred_checks":["checks to run after a patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"git diff --stat must be non-empty","forbidden_actions":["ask questions","run tests before editing"],"investigation_summary":"optional short finding","evidence":["optional file/function clues"],"avoid":["optional constraints"],"risk":"optional short risk"}}
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

pub(crate) fn supervisor_worker_brief_repair_prompt(
    work_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
    previous_brief: &Value,
) -> Result<String> {
    let task_value = read_json_file(task_path)?;
    let visible_task = agent_visible_task_value(&task_value);
    let task = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for worker brief repair prompt")?;
    let previous = serde_json::to_string_pretty(previous_brief)
        .context("failed to serialize previous worker brief for repair prompt")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are repairing a Mixmod supervisor handoff before the worker sees it.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
The previous handoff is too broad for this selected worker because it either omitted worker_turn_shape=small_patch_slice or used a small_patch_slice that still bundled too much work.
{worker_guidance}
Return a corrected expected-patch handoff with:
- "handoff":"guided"
- "expect_patch":true
- "worker_turn_shape":"small_patch_slice"
- one turn_goal for the first patch slice only
- <=2 concrete repo file paths when possible
- exact_edits must be an array with exactly one string item; do not use objects
- exactly one source exact_edits item, plus no test edit in exact_edits
- no checks unless listed in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate:"git diff --stat must be non-empty"
- forbidden_actions including "ask questions" and "run tests before editing"
Choose one source behavior only. Do not bundle validation, aliases, prefix, rename, serialization, deserialization, and tests into one slice. If the previous handoff bundled pairs such as pack/unpack, serialize/deserialize, parse/emit, validate/convert, or prefix/rename, choose only the first source half needed to create a useful diff.
Include a concrete symbol and a literal nearby code anchor when possible, such as `near the line containing "..."`.
For large functions or code-generation paths, include preservation constraints in forbidden_actions: "rewrite the whole function", "delete or reindent unrelated branches", and "edit outside the focused block".
Do not invent a different file/symbol pair. If unsure, choose the smallest already-evidenced source file from the task or previous handoff, and omit anchors you cannot justify from provided context.
If file details are uncertain, pick the smallest public API source seed patch; do not ask the worker to investigate broadly.
Working repo: {work_dir}

Task JSON:
```json
{task}
```

Rejected previous handoff:
```json
{previous}
```
"#,
        work_dir = work_dir.display(),
    ))
}

pub(crate) fn supervisor_worker_brief_repair_retry_prompt(
    work_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
    previous_brief: &Value,
    rejected_repair: &Value,
    rejection_reason: &str,
) -> Result<String> {
    let task_value = read_json_file(task_path)?;
    let visible_task = agent_visible_task_value(&task_value);
    let task = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for worker brief repair retry prompt")?;
    let previous = serde_json::to_string_pretty(previous_brief)
        .context("failed to serialize previous worker brief for repair retry prompt")?;
    let rejected = serde_json::to_string_pretty(rejected_repair)
        .context("failed to serialize rejected worker brief repair")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are retrying a Mixmod supervisor handoff repair.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
The previous repair was rejected for this structural reason: {rejection_reason}
{worker_guidance}
Return one corrected expected-patch handoff with:
- "handoff":"guided"
- "expect_patch":true
- "worker_turn_shape":"small_patch_slice"
- one turn_goal for the first patch slice only
- <=2 concrete repo file paths when possible
- exact_edits as an array with exactly one string item; do not use objects
- no required checks; put checks in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate:"git diff --stat must be non-empty"
- forbidden_actions including "ask questions" and "run tests before editing"
Choose one source behavior only. Do not bundle validation, aliases, prefix, rename, serialization, deserialization, and tests into one slice.
Do not invent a different file/symbol pair. If unsure, choose the smallest already-evidenced source file from the task or previous handoff, and omit anchors you cannot justify from provided context.
For large functions or code-generation paths, include preservation constraints in forbidden_actions: "rewrite the whole function", "delete or reindent unrelated branches", and "edit outside the focused block".
Working repo: {work_dir}

Task JSON:
```json
{task}
```

Original broad handoff:
```json
{previous}
```

Rejected repair:
```json
{rejected}
```
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
- when using worker_turn_shape=bounded_feature_slice, give a coherent edit_plan with related edits that should be completed together before checks
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
- when using worker_turn_shape=bounded_feature_slice, group related source/test work into one coherent worker turn and include enough file/function evidence to reduce repeated exploration
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
{{"action":"approve|revise|stop","worker_mode":"continue|context_focus","patch_decision":"accept_current|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"optional next slice goal","exact_edits":["optional concrete edit"],"edit_plan":["optional concrete steps"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
Use approve when no more local worker attempts are needed because the accumulated worktree.patch appears to satisfy the original task, not merely because the latest worker turn created a non-empty diff.
Prefer revise after failed, empty, distracted, or incomplete worker attempts, and put the next worker instruction in message_to_worker.
Use worker_mode=continue to keep the same worker session and let the worker continue with its existing context. Prefer continue for complex tasks because the worker needs accumulated file context.
Use worker_mode=context_focus to start a new worker session on the same worktree; previous worker context is discarded unless you repeat it in message_to_worker. Use context_focus only when the previous worker context is clearly harmful, confused, or stale.
Before choosing the next worker_turn_shape, judge whether the previous worker slice was too much, too little, or about right. If the worker produced a coherent non-destructive delta and the reasoning/report stayed on target, keep or enlarge the next slice as bounded_feature_slice. If the worker wandered, produced no new delta after exit, damaged unrelated code, or misunderstood the target, shrink to small_patch_slice for one recovery turn. If the worker solved the task, approve.
If worker-brief.json used worker_turn_shape=bounded_feature_slice, treat a useful incomplete patch as progress. When more work is needed, return action=revise, patch_decision=revise_current, worker_mode=continue, and set worker_turn_shape=bounded_feature_slice with the next coherent behavior, not one mechanical edit.
If worker-brief.json used worker_turn_shape=small_patch_slice, treat the first non-empty patch as a starter slice. Approve only after comparing worktree.patch to task.json and deciding the original acceptance criteria appear satisfied. If the slice is useful but incomplete, return action=revise, patch_decision=revise_current, usually worker_mode=continue, and set worker_turn_shape=small_patch_slice with the next narrow source/test slice.
For revision bounded_feature_slice, ask for a complete coherent behavior in one turn: related source edits plus a focused test or compile check when appropriate, usually one to three source files. Use exact_edits or edit_plan as a concise ordered plan. Avoid splitting tightly coupled pairs such as serialization/deserialization, prefix/rename, or API/source/test when doing them together is the clearer next unit.
For revision small_patch_slice, make exact_edits immediately executable and concrete; use focus_files for repo source/test paths, defer checks until after another non-empty diff, and include a completion_gate such as "git diff --stat must be non-empty". Treat exact_edits as a queue: the next worker turn executes only the first item, so put one source edit first and leave tests or follow-up edits for later turns. Make the next slice one behavior only: one source file plus at most one focused test file, no validation matrix, no bundled prefix+rename+deserialization work, and no broad "implement the feature" instruction.
For source edits inside large functions or code-generation paths, include structure-preserving constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete/reindent unrelated branches, and edit only the focused block.
When a revision needs source details, inspect repo files read-only before returning JSON and name exact symbols plus a literal nearby code anchor when possible, such as `near the line containing "..."`
in exact_edits. Do not ask the worker to investigate broadly or complete the whole feature in one revision slice.
When patch-comparison.json is present, choose patch_decision explicitly. Use accept_current when the current worktree.patch should stand, revise_current when the current patch should be edited further, and revise_previous when previous-worktree.patch is the better candidate. Mixmod will not mutate the repo directly from this choice. If you choose revise_previous, summarize the concrete source/test edits to recover in message_to_worker; do not tell the worker to read previous-worktree.patch or any Mixmod artifact.
If patch-comparison.json reports destructive or broad small-slice degradation, prefer patch_decision=revise_previous and ask for a smaller structure-preserving recovery edit unless the current patch is clearly better.
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

pub(crate) fn supervisor_feedback_repair_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    worker_guidance: &WorkerSupervisorGuidance,
    previous_feedback: &Value,
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
            truncate_for_report(&text, 4000)
        ));
    }
    let previous = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize previous supervisor feedback for repair prompt")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are repairing a Mixmod supervisor revision decision before the worker sees it.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
The selected worker needs a smaller revision slice than the previous feedback provided.
{worker_guidance}
Return a corrected revise decision with:
- "action":"revise"
- "worker_mode":"continue" unless the previous feedback required context_focus because prior context is harmful
- "patch_decision":"revise_current" unless the previous feedback required revise_previous
- "worker_turn_shape":"small_patch_slice"
- exact_edits must be an array with exactly one string item; do not use objects
- exactly one exact_edits item
- one source file in focus_files, plus at most one already-written focused test file
- no required_checks; put checks in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate mentioning git diff --stat
- forbidden_actions including "ask questions" and "run tests before editing"
The one exact edit must be atomic: one function or branch, one direction, one source behavior. If the previous edit bundles pairs such as pack/unpack, serialize/deserialize, parse/emit, validate/convert, or prefix/rename, choose only the first source half needed to create a useful diff.
Preserve the previous feedback's intended target behavior and source file unless the artifacts prove that target is wrong. Repair the size/shape of that requested next slice; do not rewind to an earlier completed slice.
Treat useful accumulated worktree.patch changes as context to keep. Do not ask the worker to remove already-useful required task options or fields merely because an earlier slice was narrower.
If previous feedback named one focus file, keep that same repo source file in focus_files and exact_edits while making the edit smaller.
For large functions or code-generation paths, include preservation constraints in forbidden_actions: "rewrite the whole function", "delete or reindent unrelated branches", and "edit outside the focused block".
Include an exact symbol and a literal nearby code anchor when possible, for example `near the line containing "..."`.
Do not invent a different file/symbol pair. If unsure, choose the smallest already-evidenced source file from the previous feedback/artifacts, and omit anchors you cannot justify from provided context.
Do not include a test edit in exact_edits. Tests belong in deferred_checks or a later revision.
Working repo: {work_dir}

Previous feedback:
```json
{previous}
```

Artifacts:
{artifacts}
"#,
        work_dir = work_dir.display(),
    ))
}

pub(crate) fn supervisor_feedback_repair_retry_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    worker_guidance: &WorkerSupervisorGuidance,
    previous_feedback: &Value,
    rejected_repair: &Value,
    rejection_reason: &str,
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
            truncate_for_report(&text, 4000)
        ));
    }
    let previous = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize previous supervisor feedback for repair retry prompt")?;
    let rejected = serde_json::to_string_pretty(rejected_repair)
        .context("failed to serialize rejected supervisor feedback repair")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are retrying a Mixmod supervisor revision repair.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
The previous repair was rejected for this structural reason: {rejection_reason}
{worker_guidance}
Return one corrected revise decision with:
- "action":"revise"
- "worker_mode":"continue" unless the previous feedback required context_focus because prior context is harmful
- "patch_decision":"revise_current" unless the previous feedback required revise_previous
- "worker_turn_shape":"small_patch_slice"
- exact_edits as an array with exactly one string item; do not use objects
- one source file in focus_files, plus at most one already-written focused test file
- no required_checks; put checks in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate mentioning git diff --stat
- forbidden_actions including "ask questions" and "run tests before editing"
Repair the size/shape of the previous requested next slice. Preserve the previous feedback's intended target behavior and source file unless the artifacts prove that target is wrong.
Do not rewind to an earlier completed slice. Do not ask the worker to remove already-useful required task options or fields merely because an earlier slice was narrower.
Do not invent a different file/symbol pair. If unsure, choose the smallest already-evidenced source file from the previous feedback/artifacts, and omit anchors you cannot justify from provided context.
For large functions or code-generation paths, include preservation constraints in forbidden_actions: "rewrite the whole function", "delete or reindent unrelated branches", and "edit outside the focused block".
Working repo: {work_dir}

Previous feedback:
```json
{previous}
```

Rejected repair:
```json
{rejected}
```

Artifacts:
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
