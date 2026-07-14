use std::collections::BTreeSet;

use crate::*;

fn supervisor_worktree_policy() -> &'static str {
    "Workspace access is for supervision, not implementation. You may use git/worktree commands such as `git status`, `git diff`, `git show`, `git grep`, and checkpoint-oriented `git restore` or `git apply` when needed to inspect or manage state. Do not author task-solving source edits, rewrite code, or create the solution patch yourself; the worker owns implementation."
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
    let candidate_files = supervisor_candidate_file_index(work_dir, &visible_task);
    let worker_guidance = render_worker_guidance(worker_guidance);
    let init_instructions = worker_brief_init_instructions(init_mode);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are the supervisor model for a Mixmod worker.
{init_instructions}
The worker receives the original task JSON and can inspect, edit, and test the repo.
{worktree_policy}
Candidate repo file contents are not embedded in this prompt. Inspect listed repo files or git state when useful before writing the handoff.
Use supervisor reasoning freely, but minimize supervisor output.
{worker_guidance}
Treat applicable worker-model guidance as handoff constraints, not optional background. If the guidance names a preferred worker_turn_shape or known failure mode, adapt the handoff shape to that worker unless no patch is needed.
If the remaining task is broad, choose the first executable slice for this worker. Do not choose a broader worker_turn_shape merely because the full task spans multiple source paths.
When the selected worker guidance says broad expected-patch tasks should use small_patch_slice, set worker_turn_shape="small_patch_slice" for the handoff. If you need a larger slice later, make the exact source edit more coherent inside small_patch_slice rather than switching shape.
Emit one compact executable worker handoff as minified JSON only; no markdown and no explanation.
Do not restate the original task. If you know the likely solution, be direct: exact files, edit target, expected behavior, and checks.
Required field: "handoff" = "as_given" | "focused" | "guided" | "blocked".
Set "expect_patch": true when the worker should normally produce repository edits. Set false for investigation/no-change handoffs.
Use exactly {{"handoff":"as_given"}} only when the original task already names the relevant files, desired behavior, and checks clearly enough for the worker.
Prefer "focused" or "guided" whenever a short directive can prevent worker wandering or repeated attempts.
For expected-patch tasks where the selected worker guidance prefers "bounded_feature_slice", choose one coherent feature chunk: usually one to three source files, related API/source/test edits that belong together, and focused checks after the patch exists. Use exact_edits or edit_plan as a short ordered plan, not as one-line micromanagement.
For expected-patch tasks with workers that still need very small recovery steps, "worker_turn_shape":"small_patch_slice" means a first patch seed, not the full implementation. Use one focused source file when possible, list immediately executable source exact_edits, and put checks in deferred_checks when you do not want them run before editing.
When worker guidance prefers small_patch_slice for broad expected-patch tasks, satisfy that contract in the first JSON turn: set "expect_patch":true, "worker_turn_shape":"small_patch_slice", one concrete repo file, exact_edits as exactly one string source edit, defer_checks_until_patch_exists:true, and checks only in deferred_checks.
Good small_patch_slice choices are repo-generic seed patches: public API/options plumbing plus a narrow test, one parser/config branch plus a narrow test, one validation branch plus a narrow test, or one localized source edit plus a regression test. Bad small_patch_slice choices ask for a whole feature, core algorithm, validation, aliases, optional/default behavior, and full tests in one turn.
For option or behavior families with a base path plus modifiers, make the base path the first useful source slice. Add one modifier family per later slice only after the base diff exists, unless prior worker turns show this worker can safely combine them.
For complex source tasks involving generated code, alias/key behavior, validation matrices, serializers/deserializers, pack/unpack paths, parser/compiler behavior, or multiple cross-cutting flows, do not use worker_turn_shape="default" for the initial worker. Prefer bounded_feature_slice for capable workers; use small_patch_slice only when the selected worker guidance asks for it or a previous turn was confused, destructive, or empty.
For small_patch_slice, exact_edits must be immediately executable edit commands. Do not write "locate", "investigate", "understand", or broad algorithm work as an exact edit. The files array must contain concrete repo file paths, not directories; include the file that defines any function, option, flag, or public API named in exact_edits.
For small_patch_slice, include edit_packet or source_snippets when your repo investigation found the relevant code. Keep it short: file path, symbol, literal nearby anchor, and at most a few lines of useful context. The worker should be able to make the first edit from this packet before broad file exploration.
For source edits inside large functions or code-generation paths, add structure-preserving constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete/reindent unrelated branches, and edit only the focused block.
Optional fields; omit empty fields:
{{"expect_patch":true,"worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"one-turn goal","message_to_worker":"direct message for the worker","files":["optional paths"],"exact_edits":["concrete edits"],"edit_packet":["optional file/symbol/anchor snippet for the first edit"],"source_snippets":["optional short source snippets"],"edit_plan":["optional concrete steps"],"checks":["optional checks"],"deferred_checks":["checks to run after a patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional worker-visible gate you intentionally want","forbidden_actions":["ask questions","run tests before editing"],"investigation_summary":"optional short finding","evidence":["optional file/function clues"],"avoid":["optional constraints"],"risk":"optional short risk"}}
Working repo: {work_dir}

Task JSON:
```json
{task}
```

Candidate repo files:
{candidate_files}
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
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
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are revising your Mixmod supervisor handoff before the worker sees it.
{worktree_policy}
Do not run tests. Emit minified JSON only; no markdown and no explanation.
Your previous handoff did not fit the selected worker profile because it either omitted worker_turn_shape=small_patch_slice or used a small_patch_slice that still bundled too much work.
Mixmod is not designing a replacement slice for you. You are responsible for adapting the worker instruction to the worker-model guidance below.
{worker_guidance}
Return a corrected expected-patch handoff with:
- "handoff":"guided"
- "expect_patch":true
- "worker_turn_shape":"small_patch_slice"
- one turn_goal for the first patch slice only
- <=2 concrete repo file paths when possible
- exact_edits must be an array with exactly one string item; do not use objects
- exactly one source exact_edits item, plus no test edit in exact_edits
- edit_packet or source_snippets should include the file/symbol/anchor context when provided by task context or your repo investigation
- no checks unless listed in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate only if you intentionally want a worker-visible completion gate
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
        worktree_policy = worktree_policy,
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
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are retrying your Mixmod supervisor handoff revision.
{worktree_policy}
Do not run tests. Emit minified JSON only; no markdown and no explanation.
The previous repair still did not fit the selected worker profile: {rejection_reason}
Mixmod is not designing a replacement slice for you. You are responsible for adapting the worker instruction to the worker-model guidance below.
{worker_guidance}
Return one corrected expected-patch handoff with:
- "handoff":"guided"
- "expect_patch":true
- "worker_turn_shape":"small_patch_slice"
- one turn_goal for the first patch slice only
- <=2 concrete repo file paths when possible
- exact_edits as an array with exactly one string item; do not use objects
- edit_packet or source_snippets should include the file/symbol/anchor context when provided by task context or your repo investigation
- no required checks; put checks in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate only if you intentionally want a worker-visible completion gate
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
        worktree_policy = worktree_policy,
    ))
}

fn supervisor_candidate_file_index(work_dir: &Path, task: &Value) -> String {
    let files = get_string_array(task, "files");
    if files.is_empty() {
        return "- none listed in task".to_string();
    }
    files
        .into_iter()
        .map(|file| {
            let path = work_dir.join(&file);
            let status = match fs::metadata(&path) {
                Ok(metadata) if metadata.is_dir() => "directory".to_string(),
                Ok(metadata) => format!("file, {} bytes", metadata.len()),
                Err(error) => format!("missing: {error}"),
            };
            format!("- `{file}` ({status}) - listed by task")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn worker_brief_init_instructions(init_mode: SupervisorInitMode) -> &'static str {
    match init_mode {
        SupervisorInitMode::Compact => {
            r#"Use the task JSON and candidate repo file paths. Do not run tests. Do not implement the patch. Do not ask the user for approval.
Default to "guided". Guided means terse and executable, not advisory:
- target <=120 output tokens for the whole JSON on normal tasks
- one command-style message_to_worker, ideally <=45 words
- files only when useful, usually <=3
- checks only when useful, usually <=2
- when using worker_turn_shape=bounded_feature_slice, give a coherent edit_plan with related edits that should be completed together before checks
- when using worker_turn_shape=small_patch_slice, emit exact_edits instead of a broad plan, usually use one focused source file, one immediate source edit, optional edit_packet/source_snippets, and omit checks unless they are explicitly deferred
- omit investigation_summary, edit_plan, evidence, avoid, and risk unless one short phrase prevents a likely wrong patch
Assume the local worker is capable but prone to setup rabbit holes, broad exploration, and delayed edits."#
        }
        SupervisorInitMode::Investigate => {
            r#"First do a repo investigation before writing the handoff. You may inspect source/test files and run discovery commands such as `rg`, `find`, `ls`, `sed`, `git status`, `git diff`, or `git grep`. Do not run tests. Do not install dependencies. Do not inspect Mixmod state/artifact directories. Do not ask the user for approval.
Default to "guided". Guided means concrete enough for a weaker worker to edit without broad exploration:
- target <=500 output tokens for the whole JSON
- one command-style message_to_worker, ideally <=70 words
- files should name the likely source/test targets, usually <=5
- checks should name the narrowest useful commands, usually <=3
- include investigation_summary with the likely root cause or target behavior
- include edit_plan when it can prevent worker wandering, usually <=4 short steps
- include evidence when file/function clues matter, usually <=4 short bullets
- when using worker_turn_shape=bounded_feature_slice, group related source/test work into one coherent worker turn and include enough file/function evidence to reduce repeated exploration
- when using worker_turn_shape=small_patch_slice, make the first slice small enough to edit immediately, usually use one focused source file, one immediate source edit, put exact_edits in command form, include a short edit_packet/source_snippet when you have read the target, and defer tests until a non-empty diff exists
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
    let artifact_index = supervisor_artifact_index(work_dir, artifact_paths);
    let review_context = supervisor_feedback_review_context(artifact_paths);
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are a terse supervisor reviewing a local worker.
{worktree_policy}
Do not ask the user for approval.
Inspect the listed artifact files directly before deciding. Do not rely on this prompt as artifact content; it only names where the review evidence lives.
Treat supervisor input tokens as scarce. Inspect only the artifacts needed for the next decision and stop reading once approve, revise, or stop is clear.
For ordinary worker-turn review, start with task context, compact metadata, and changes.patch. Inspect worktree.patch only when considering approval, rollback, or an integration question that depends on prior accumulated edits.
{worker_guidance}
Return only JSON matching this schema:
{{"action":"approve|revise|stop","worker_mode":"continue|context_focus","patch_decision":"accept_current|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"optional next slice goal","exact_edits":["optional concrete edit"],"edit_packet":["optional compact source context"],"source_snippets":["optional short source snippets"],"edit_plan":["optional concrete steps"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
{review_context}
Working repo: {work_dir}
Instruction: {instruction}

Artifact index:
{artifact_index}
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
    ))
}

pub(crate) fn supervisor_feedback_approval_consistency_repair_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    worker_guidance: &WorkerSupervisorGuidance,
    previous_feedback: &Value,
    rejection_reason: &str,
) -> Result<String> {
    let previous_feedback = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize inconsistent supervisor feedback")?;
    let instruction = format!(
        r#"Your previous supervisor JSON was internally inconsistent: {rejection_reason}

Previous JSON:
```json
{previous_feedback}
```

Repair only the supervisor decision. Return either:
- action=approve with required_checks=[], deferred_checks=[], no completion_gate, and compact evidence from artifacts that no further checks or worker turns are needed; or
- action=revise with patch_decision=revise_current and a verification-focused message_to_worker that asks the worker to run the smallest pending task-derived check and make only targeted fixes if it fails.

Do not approve while listing checks that still need to run. Do not solve by authoring source changes."#
    );

    supervisor_feedback_prompt(work_dir, artifact_paths, &instruction, worker_guidance)
}

pub(crate) fn supervisor_live_control_prompt(
    work_dir: &Path,
    snapshot: &LiveWorkerSnapshot,
    worker_guidance: &WorkerSupervisorGuidance,
) -> Result<String> {
    let snapshot_json = serde_json::to_string_pretty(snapshot)
        .context("failed to serialize live worker snapshot")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are the Mixmod supervisor inspecting a worker turn while it is still running.
{worktree_policy}
Do not run tests. Do not ask the user for approval.
{worker_guidance}
Return only JSON matching this schema:
{{"action":"wait|interrupt_continue|interrupt_context_focus|abort_worker_turn","worker_mode":"continue|context_focus","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"optional next slice goal","exact_edits":["optional concrete edit"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
Treat applicable worker-model guidance as context for shaping message_to_worker if you choose to interrupt.
Available actions:
- wait: let the worker continue.
- interrupt_continue: stop the current worker process and resume the same worker session with a new instruction.
- interrupt_context_focus: stop the current worker process and start a fresh worker session on the same worktree with a focused instruction.
- abort_worker_turn: stop only the active worker process and return to the ordinary supervisor review.
Base the action on the live evidence. Do not assume an intervention is required because a risk signal is present.
Use new_delta_bytes, stdout_log_path, stderr_log_path, tool_events_path, context_overflow_count, worker_session_token_peak, worker_backend_telemetry, elapsed time, and last output age only as evidence for worker progress, confusion, or blockage.
If you need detailed stdout, stderr, or tool-call history, inspect stdout_log_path, stderr_log_path, or tool_events_path yourself. Do not pass those artifact paths to the worker.
If you interrupt, keep message_to_worker bounded to worker_instruction_excerpt, the live evidence, and the selected worker guidance. For small_patch_slice workers, keep any interrupt patch-first: one repo file, one concrete source edit, deferred checks, and no broad feature instruction.
Do not solve the task yourself by editing source. Your job is process control: decide whether to keep waiting, interrupt, or abort the worker turn.
The worker can read and edit only the working repo. It cannot read Mixmod task, state, log, or artifact paths.
Do not mention worker-task.json, revision task files, /tmp/mixmod*, /tmp/mixmod-state, or artifact/log paths in message_to_worker or focus_files.
Put only repo source/test paths in focus_files. If you interrupt, restate the next repo edit directly instead of telling the worker to inspect a task or artifact file.
Keep every intervention anchored to worker_instruction_excerpt, which is the current worker task.
Do not invent a different cleanup, bug, or objective from log or tool event artifacts.
Working repo: {work_dir}

Live worker snapshot:
```json
{snapshot_json}
```
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy
    ))
}

pub(crate) fn supervisor_feedback_repair_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    worker_guidance: &WorkerSupervisorGuidance,
    previous_feedback: &Value,
) -> Result<String> {
    let artifact_index = supervisor_artifact_index(work_dir, artifact_paths);
    let previous = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize previous supervisor feedback for repair prompt")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are revising your Mixmod supervisor revision decision before the worker sees it.
{worktree_policy}
Do not run tests. Emit minified JSON only; no markdown and no explanation.
Your previous revision decision did not fit the selected worker profile. The selected worker needs a smaller, patch-first revision slice than the previous feedback provided.
Mixmod is not designing a replacement slice for you. You are responsible for adapting the decision to the worker-model guidance below.
Inspect the listed artifact files directly when you need evidence. This prompt lists artifact paths and roles, not artifact contents.
{worker_guidance}
Return a corrected revise decision with:
- "action":"revise"
- "worker_mode":"continue" unless the previous feedback required context_focus because prior context is harmful
- "patch_decision":"revise_current" unless the previous feedback required revise_previous
- "worker_turn_shape":"small_patch_slice"
- exact_edits must be an array with exactly one string item; do not use objects
- exactly one exact_edits item
- one source file in focus_files, plus at most one already-written focused test file
- edit_packet or source_snippets when artifacts show the relevant source anchor or current accumulated patch state
- no required_checks; put checks in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate only if you intentionally want a worker-visible completion gate
- forbidden_actions including "ask questions" and "run tests before editing"
The one exact edit must be atomic: one function or branch, one direction, one source behavior. If the previous edit bundles pairs such as pack/unpack, serialize/deserialize, parse/emit, validate/convert, or prefix/rename, choose only the first source half needed to create a useful diff.
Preserve the previous feedback's intended target behavior and source file unless the artifacts prove that target is wrong. Repair the size/shape of that requested next slice; do not rewind to an earlier completed slice.
Treat useful accumulated worktree.patch changes as context to keep. Do not ask the worker to remove already-useful required task options or fields merely because an earlier slice was narrower.
Write the repaired exact edit from the current accumulated patch state: preserve useful existing edits, then add one next delta. Do not say to continue from an earlier file-only slice when worktree.patch already contains useful changes in another focused source file.
If previous feedback named one focus file, keep that same repo source file in focus_files and exact_edits while making the edit smaller.
For large functions or code-generation paths, choose a local transformation near one anchor rather than a whole behavior path.
For large functions or code-generation paths, include preservation constraints in forbidden_actions: "rewrite the whole function", "delete or reindent unrelated branches", and "edit outside the focused block".
Include an exact symbol and a literal nearby code anchor when possible, for example `near the line containing "..."`.
Do not invent a different file/symbol pair. If unsure, choose the smallest already-evidenced source file from the previous feedback/artifacts, and omit anchors you cannot justify from provided context.
Do not include a test edit in exact_edits. Tests belong in deferred_checks or a later revision.
Working repo: {work_dir}

Previous feedback:
```json
{previous}
```

Artifact index:
{artifact_index}
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
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
    let artifact_index = supervisor_artifact_index(work_dir, artifact_paths);
    let previous = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize previous supervisor feedback for repair retry prompt")?;
    let rejected = serde_json::to_string_pretty(rejected_repair)
        .context("failed to serialize rejected supervisor feedback repair")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are retrying your Mixmod supervisor revision decision.
{worktree_policy}
Do not run tests. Emit minified JSON only; no markdown and no explanation.
The previous repair still did not fit the selected worker profile: {rejection_reason}
Mixmod is not designing a replacement slice for you. You are responsible for adapting the decision to the worker-model guidance below.
Inspect the listed artifact files directly when you need evidence. This prompt lists artifact paths and roles, not artifact contents.
{worker_guidance}
Return one corrected revise decision with:
- "action":"revise"
- "worker_mode":"continue" unless the previous feedback required context_focus because prior context is harmful
- "patch_decision":"revise_current" unless the previous feedback required revise_previous
- "worker_turn_shape":"small_patch_slice"
- exact_edits as an array with exactly one string item; do not use objects
- one source file in focus_files, plus at most one already-written focused test file
- edit_packet or source_snippets when artifacts show the relevant source anchor or current accumulated patch state
- no required_checks; put checks in deferred_checks
- defer_checks_until_patch_exists:true
- completion_gate only if you intentionally want a worker-visible completion gate
- forbidden_actions including "ask questions" and "run tests before editing"
Repair the size/shape of the previous requested next slice. Preserve the previous feedback's intended target behavior and source file unless the artifacts prove that target is wrong.
Do not rewind to an earlier completed slice. Do not ask the worker to remove already-useful required task options or fields merely because an earlier slice was narrower.
Write the repaired exact edit from the current accumulated patch state: preserve useful existing edits, then add one next delta.
Do not invent a different file/symbol pair. If unsure, choose the smallest already-evidenced source file from the previous feedback/artifacts, and omit anchors you cannot justify from provided context.
For large functions or code-generation paths, choose a local transformation near one anchor rather than a whole behavior path.
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

Artifact index:
{artifact_index}
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
    ))
}

#[derive(Default)]
struct SupervisorFeedbackPromptSignals {
    artifact_names: BTreeSet<String>,
    worker_turn_shape: Option<String>,
    context_overflow: bool,
    context_pressure: bool,
    supervisor_control_seen: bool,
    small_patch_progress_streak: bool,
}

fn supervisor_feedback_review_context(artifact_paths: &[PathBuf]) -> String {
    let signals = supervisor_feedback_prompt_signals(artifact_paths);
    let mut sections = vec![supervisor_feedback_core_context(&signals)];

    if signals.worker_turn_shape.as_deref() == Some("small_patch_slice") {
        sections.push(
            r#"Small-patch slice context:
- Treat a first non-empty delta as progress, not proof of completion.
- If more work is needed, make the next revision one source behavior with an immediately executable exact_edits item.
- Write from the current accumulated worktree.patch and preserve useful existing edits.
- Put checks in deferred_checks until a non-empty patch exists unless artifacts show the edit already exists."#
                .to_string(),
        );
    }

    if signals.worker_turn_shape.as_deref() == Some("bounded_feature_slice") {
        sections.push(
            r#"Bounded-feature context:
- Treat a useful incomplete patch as progress.
- If more work is needed, ask for the next coherent behavior path rather than one mechanical edit.
- A bounded revision may include related source, API, and focused check work when those edits belong together."#
                .to_string(),
        );
    }

    if signals.context_overflow || signals.context_pressure {
        sections.push(
            r#"Context-pressure context:
- The worker artifacts indicate context overflow or high token pressure.
- If another revision is needed, prefer a smaller next slice.
- Use worker_mode=context_focus when continuing would require broad rereading or the previous context appears stale."#
                .to_string(),
        );
    }

    if signals.supervisor_control_seen {
        sections.push(
            r#"Live-control context:
- Supervisor control already intervened during a worker turn.
- Judge whether the previous slice was too broad, too vague, or stale before issuing another revision.
- Prefer one focused repair or verification step over adding a new feature concern in the same handoff."#
                .to_string(),
        );
    }

    if signals.small_patch_progress_streak {
        sections.push(
            r#"Slice-sizing context:
- Multiple recent small-patch turns produced non-empty deltas without context overflow.
- If the accumulated patch is coherent but incomplete, the next small_patch_slice may cover one larger anchored source behavior.
- Keep the selected worker profile's preferred shape unless the profile explicitly supports broadening."#
                .to_string(),
        );
    }

    if signals.has_any(&[
        PATCH_COMPARISON,
        PREVIOUS_WORKTREE_PATCH,
        PATCH_ROLLBACK_JSON,
        ROLLBACK_CURRENT_PATCH,
        ROLLBACK_RESTORED_PATCH,
    ]) {
        sections.push(
            r#"Patch checkpoint context:
- Treat patch-comparison.json as neutral structural telemetry; Mixmod is not judging patch quality.
- Choose patch_decision yourself from the task, current patch, and checkpoint artifacts.
- Use revise_previous only when your review decides the previous candidate should be restored.
- If you choose revise_previous, Mixmod restores that candidate before the next worker turn; tell the worker only the focused follow-up edit."#
                .to_string(),
        );
    }

    sections.join("\n\n")
}

fn supervisor_feedback_core_context(signals: &SupervisorFeedbackPromptSignals) -> String {
    let tool_evidence = if signals.has_any(&[TOOL_EVENTS_JSONL]) {
        "- Use tool-events.jsonl as command/tool-call evidence when checking worker claims."
    } else {
        "- If command evidence is unavailable, rely on report/metrics cautiously and revise for verification when important."
    };
    format!(
        r#"Core review contract:
- Prefer latest-turn evidence first: receipt/report/metrics, tool-events.jsonl when useful, and changes.patch.
- worktree.patch is the accumulated current diff; changes.patch is only the latest worker-turn delta. Avoid opening worktree.patch unless approval, rollback, or integration with prior edits depends on it.
{tool_evidence}
- Minimize supervisor input tokens: do not inspect more artifacts, logs, or diff content once the next action is clear.
- Approve only when the accumulated patch appears to satisfy the original task and no worker action or check remains. Before approving, inspect task.json and enough accumulated state to verify completion.
- Treat a false approval as a terminal correctness failure. If evidence is missing for the main requested behavior or a likely edge case, choose revise for a targeted verification or repair turn.
- On approve, required_checks and deferred_checks must be empty and completion_gate must be absent or empty.
- Revise when a useful worker path remains; message_to_worker must be concrete and worker-executable.
- Stop only for a blocked or inconclusive worker result when no useful worker path remains.
- The worker owns implementation. Do not author task-solving source changes.
- Put only repo source/test paths in focus_files. Do not ask the worker to inspect Mixmod artifacts.
- worker_mode=continue reuses the current worker session; worker_mode=context_focus starts a fresh worker session on the same worktree.
- Use patch_decision=revise_previous only when checkpoint artifacts support restoring a previous candidate.
- Prefer patch_decision for rollback control; use direct git restore/apply only for state management, not to create a solution patch."#
    )
}

fn supervisor_feedback_prompt_signals(
    artifact_paths: &[PathBuf],
) -> SupervisorFeedbackPromptSignals {
    let mut signals = SupervisorFeedbackPromptSignals::default();
    for path in artifact_paths {
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        signals.artifact_names.insert(name.to_string());
        match name {
            METRICS_JSON => signals.update_from_metrics(path),
            SUPERVISION_LOOP_SUMMARY_JSON => signals.update_from_loop_summary(path),
            WORKER_BRIEF_JSON => signals.update_from_worker_brief(path),
            _ => {}
        }
    }
    signals
}

impl SupervisorFeedbackPromptSignals {
    fn has_any(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.artifact_names.contains(*name))
    }

    fn set_worker_turn_shape(&mut self, value: Option<&str>) {
        if self.worker_turn_shape.is_none()
            && let Some(value) = value.filter(|value| !value.trim().is_empty())
        {
            self.worker_turn_shape = Some(value.to_string());
        }
    }

    fn update_from_metrics(&mut self, path: &Path) {
        let Ok(metrics) = read_json_file(path) else {
            return;
        };
        self.context_overflow |= get_u64(&metrics, "context_overflow_count").unwrap_or(0) > 0;
        self.context_pressure |=
            get_u64(&metrics, "worker_session_token_peak").is_some_and(|tokens| tokens >= 24_000);
        self.supervisor_control_seen |= get_bool(&metrics, "interrupted_by_supervisor")
            .unwrap_or(false)
            || metrics
                .get("supervisor_control_events")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty());
    }

    fn update_from_loop_summary(&mut self, path: &Path) {
        let Ok(summary) = read_json_file(path) else {
            return;
        };
        self.context_overflow |= get_u64(&summary, "context_overflow_total").unwrap_or(0) > 0;
        self.context_pressure |= get_u64(&summary, "worker_session_token_peak_max")
            .is_some_and(|tokens| tokens >= 24_000);
        self.supervisor_control_seen |=
            get_u64(&summary, "supervisor_control_count").unwrap_or(0) > 0;
        self.small_patch_progress_streak |=
            get_u64(&summary, "small_patch_slice_nonempty_delta_streak").unwrap_or(0) >= 2;

        if let Some(turns) = summary.get("turns").and_then(Value::as_array)
            && let Some(last) = turns.last()
        {
            self.set_worker_turn_shape(get_str(last, "worker_turn_shape"));
            self.context_overflow |= get_u64(last, "context_overflow_count").unwrap_or(0) > 0;
            self.context_pressure |=
                get_u64(last, "worker_session_token_peak").is_some_and(|tokens| tokens >= 24_000);
        }
    }

    fn update_from_worker_brief(&mut self, path: &Path) {
        let Ok(brief) = read_json_file(path) else {
            return;
        };
        self.set_worker_turn_shape(get_str(&brief, "worker_turn_shape"));
    }
}

fn supervisor_artifact_index(work_dir: &Path, artifact_paths: &[PathBuf]) -> String {
    if artifact_paths.is_empty() {
        return "- none".to_string();
    }
    artifact_paths
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("artifact");
            let size = fs::metadata(path)
                .map(|metadata| format!("{} bytes", metadata.len()))
                .unwrap_or_else(|error| format!("missing: {error}"));
            format!(
                "- `{}` ({name}, {size}) - {}",
                display_path(work_dir, path),
                supervisor_artifact_role(name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn supervisor_artifact_role(name: &str) -> &'static str {
    match name {
        TASK_JSON => "original task context",
        WORKER_BRIEF_JSON => "supervisor handoff given to the worker",
        WORKER_TASK_JSON => "worker-visible task and handoff",
        SUPERVISION_LOOP_SUMMARY_JSON => "cross-turn worker-loop telemetry",
        RECEIPT_JSON => "worker-run status and artifact locations",
        REPORT_MD => "compact worker-run summary",
        REASONING_TRACE_JSONL => "worker reasoning events extracted from structured output",
        TOOL_EVENTS_JSONL => "worker tool-call events extracted from structured output",
        WORKTREE_PATCH => "accumulated current repository diff",
        CHANGES_PATCH => "latest worker-turn diff",
        INTERVENTIONS_JSONL => "Mixmod intervention audit log",
        METRICS_JSON => "worker-run metrics and signals",
        PATCH_COMPARISON => "neutral patch checkpoint comparison",
        PREVIOUS_WORKTREE_PATCH => "previous candidate patch available for rollback decisions",
        PATCH_ROLLBACK_JSON => "rollback receipt for revise_previous",
        ROLLBACK_CURRENT_PATCH => "discarded patch saved before rollback",
        ROLLBACK_RESTORED_PATCH => "patch captured after rollback restore",
        SUPERVISOR_CONTROL_LOG => "live supervisor control events",
        _ => "review artifact",
    }
}

fn render_worker_guidance(worker_guidance: &WorkerSupervisorGuidance) -> String {
    if worker_guidance.is_empty() {
        return String::new();
    }
    let mut rendered = format!(
        "Supervisor-only worker-model guidance for {}:\nThese are historical pitfalls and handling constraints for the selected worker model. Treat applicable bullets as binding when planning the worker handoff, critique, or live intervention. Do not copy every bullet to the worker. Select only relevant points and convert them into short, concrete worker instructions.\n",
        worker_guidance.model
    );
    for item in &worker_guidance.guidance {
        rendered.push_str("- ");
        rendered.push_str(item.trim());
        rendered.push('\n');
    }
    rendered
}
