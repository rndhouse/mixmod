use crate::*;

pub(crate) fn codex_only_prompt(work_dir: &Path, task: &Value) -> Result<String> {
    let visible_task = agent_visible_task_value(task);
    let task_json = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for Codex-only prompt")?;
    Ok(format!(
        r#"You are the Codex-only baseline for a Mixmod experiment.
Solve the task directly in this repo. Edit files as needed, run the requested tests, and keep the final answer compact.
Do not use Mixmod or OpenCode.
Do not commit. Leave the final repository changes as an uncommitted git diff so Mixmod can record the patch.
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
For expected-patch tasks with workers that still need very small recovery steps, "worker_turn_shape":"small_patch_slice" means a first patch seed, not the full implementation. Use one focused source file when possible, list one immediately executable source exact_edit, defer checks until after a non-empty diff, and include a completion_gate such as "git diff --stat must be non-empty".
When worker guidance prefers small_patch_slice for broad expected-patch tasks, satisfy that contract in the first JSON turn: set "expect_patch":true, "worker_turn_shape":"small_patch_slice", one concrete repo file, exact_edits as exactly one string source edit, defer_checks_until_patch_exists:true, and checks only in deferred_checks.
Good small_patch_slice choices are repo-generic seed patches: public API/options plumbing plus a narrow test, one parser/config branch plus a narrow test, one validation branch plus a narrow test, or one localized source edit plus a regression test. Bad small_patch_slice choices ask for a whole feature, core algorithm, validation, aliases, optional/default behavior, and full tests in one turn.
For option or behavior families with a base path plus modifiers, make the base path the first useful source slice. Add one modifier family per later slice only after the base diff exists, unless prior worker turns show this worker can safely combine them.
For complex source tasks involving generated code, alias/key behavior, validation matrices, serializers/deserializers, pack/unpack paths, parser/compiler behavior, or multiple cross-cutting flows, do not use worker_turn_shape="default" for the initial worker. Prefer bounded_feature_slice for capable workers; use small_patch_slice only when the selected worker guidance asks for it or a previous turn was confused, destructive, or empty.
For small_patch_slice, exact_edits must be immediately executable edit commands. Do not write "locate", "investigate", "understand", or broad algorithm work as an exact edit. The files array must contain concrete repo file paths, not directories; include the file that defines any function, option, flag, or public API named in exact_edits.
For small_patch_slice, include edit_packet or source_snippets when your read-only investigation found the relevant code. Keep it short: file path, symbol, literal nearby anchor, and at most a few lines of useful context. The worker should be able to make the first edit from this packet before broad file exploration.
For source edits inside large functions or code-generation paths, add structure-preserving constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete/reindent unrelated branches, and edit only the focused block.
Optional fields; omit empty fields:
{{"expect_patch":true,"worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"one-turn goal","message_to_worker":"direct message for the worker","files":["optional paths"],"exact_edits":["concrete edit"],"edit_packet":["optional file/symbol/anchor snippet for the first edit"],"source_snippets":["optional short source snippets"],"edit_plan":["optional concrete steps"],"checks":["optional checks"],"deferred_checks":["checks to run after a patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"git diff --stat must be non-empty","forbidden_actions":["ask questions","run tests before editing"],"investigation_summary":"optional short finding","evidence":["optional file/function clues"],"avoid":["optional constraints"],"risk":"optional short risk"}}
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
        r#"You are revising your Mixmod supervisor handoff before the worker sees it.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
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
- edit_packet or source_snippets should include the file/symbol/anchor context when provided by task context or your read-only investigation
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
        r#"You are retrying your Mixmod supervisor handoff revision.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
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
- edit_packet or source_snippets should include the file/symbol/anchor context when provided by task context or your read-only investigation
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
- when using worker_turn_shape=small_patch_slice, emit exact_edits instead of a broad plan, usually use one focused source file, one immediate source edit, optional edit_packet/source_snippets, and omit checks unless they are explicitly deferred
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
{{"action":"approve|revise|stop","worker_mode":"continue|context_focus","patch_decision":"accept_current|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"optional next slice goal","exact_edits":["optional concrete edit"],"edit_packet":["optional compact source context"],"source_snippets":["optional short source snippets"],"edit_plan":["optional concrete steps"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
Use approve when no more local worker attempts are needed because the accumulated worktree.patch appears to satisfy the original task, not merely because the latest worker turn created a non-empty diff.
You own final task completeness. The worker owns editing; you must not approve merely because the worker followed the latest slice, compiled, or reported success.
Before approving, classify whether the accumulated patch changes runtime behavior, parsing/compilation, state mutation, persistence, API contracts, error handling, validation, or control flow. For behavior-changing patches, require concrete evidence from focused task-derived checks that exercise the requested behavior and at least one likely negative or edge case. If artifacts do not show such evidence, return revise with a verification-focused worker turn; the worker should run the smallest relevant probes or tests and edit only if they fail.
Prefer revise after failed, empty, distracted, or incomplete worker attempts, and put the next worker instruction in message_to_worker.
For tasks involving generated keys, aliases, field names, serializers, deserializers, or validation, do not approve until the patch appears coherent for raw names and configured aliases across the relevant input and output paths. If artifacts do not prove an alias/key variant, revise with a focused source repair or regression check.
Treat applicable worker-model guidance as part of the supervisor decision contract. If the selected worker guidance says to prefer small_patch_slice, a broad revise is the wrong decision even when the remaining feature is broad. Split the remaining work into the next immediately executable worker slice; if you cannot identify a concrete slice from artifacts or read-only inspection, use stop with a clear risk instead of sending a broad revision.
Use worker_mode=continue to keep the same worker session and let the worker continue with its existing context. Prefer continue for complex tasks because the worker needs accumulated file context.
Use worker_mode=context_focus to start a new worker session on the same worktree; previous worker context is discarded unless you repeat it in message_to_worker. Use context_focus only when the previous worker context is clearly harmful, confused, or stale.
If the report or metrics show worker context overflow, treat the worker session as context-saturated. When overflow is paired with no new delta, repeated summary updates, or broad rereading, prefer worker_mode=context_focus, preserve the current worktree patch, and repeat only the essential task state in the next handoff.
If the report or metrics show a high worker_session_token_peak relative to the worker context window, treat the session as context-pressured even without explicit overflow. Prefer a smaller next slice, or worker_mode=context_focus when another continued turn would require broad rereading or a multi-branch edit.
Before choosing the next worker_turn_shape, judge whether the previous worker slice was too much, too little, or about right for the selected worker model. If the worker produced a coherent non-destructive delta, the reasoning/report stayed on target, and the selected worker guidance allows broad slices, keep or enlarge the next slice as bounded_feature_slice. If the worker guidance prefers small_patch_slice, keep worker_turn_shape=small_patch_slice; when the worker demonstrates progress, broaden only the anchored source behavior inside that shape unless the guidance explicitly supports bounded_feature_slice. If the worker wandered, produced no new delta after exit, damaged unrelated code, or misunderstood the target, shrink to small_patch_slice for one recovery turn. If the worker solved the task, approve.
If there are multiple consecutive clean small_patch_slice revisions with non-empty accurate deltas, no context overflow, and moderate worker_session_token_peak, treat that as evidence the previous slices may now be too small. For worker profiles that prefer small_patch_slice, promotion means one coherent anchored source behavior inside small_patch_slice, not a switch to bounded_feature_slice.
If a nominal small_patch_slice needed supervisor control, hit a high worker_session_token_peak, or required a corrective follow-up, treat the previous slice as too broad or unstable. A large latest_delta_stats/changed_line_count is a sizing signal, not a rollback signal: first inspect which files changed. Generated or checked-in derived files can legitimately expand a focused source change. Judge whether the current worktree.patch matches the requested source behavior before shrinking or rolling back. When shrinking is warranted, use worker_mode=context_focus when context is pressured, and ask for exactly one local source transformation or one focused test update, not a new bundle of validation, optional/default behavior, aliasing, and extra-key handling.
If a modifier-family slice needed a follow-up fix, finish that modifier or repair its regression before adding another modifier family or defensive validation path. A corrective small_patch_slice is recovery, not promotion.
Do not spend many supervisor turns on tiny prerequisite or defensive-validation slices after the worker has shown it can follow anchored source edits. Once API plumbing, parsing, or basic validation exists, prefer the first useful end-to-end behavior path unless artifacts show another prerequisite is blocking progress.
If context overflow appears, also judge whether the previous slice was too large for the worker. Prefer a smaller worker_turn_shape for the next attempt: one concrete source edit, one focused code anchor, and at most one focused test/check after the edit exists.
If worker-brief.json used worker_turn_shape=bounded_feature_slice, treat a useful incomplete patch as progress. When more work is needed, return action=revise, patch_decision=revise_current, worker_mode=continue, and set worker_turn_shape=bounded_feature_slice with the next coherent behavior, not one mechanical edit.
If worker-brief.json used worker_turn_shape=small_patch_slice, treat the first non-empty patch as a starter slice. Approve only after comparing worktree.patch to task.json and deciding the original acceptance criteria appear satisfied. If the slice is useful but incomplete, return action=revise, patch_decision=revise_current, usually worker_mode=continue, and set worker_turn_shape=small_patch_slice with the next narrow source/test slice.
For revision bounded_feature_slice, ask for a complete coherent behavior in one turn: related source edits plus a focused test or compile check when appropriate, usually one to three source files. Use this only when it fits the selected worker guidance and observed worker behavior. Use exact_edits or edit_plan as a concise ordered plan. Avoid splitting tightly coupled pairs such as serialization/deserialization, prefix/rename, or API/source/test when doing them together is the clearer next unit.
For revision small_patch_slice, make exact_edits immediately executable and concrete; use focus_files for repo source/test paths, defer checks until after another non-empty diff, and include a completion_gate such as "git diff --stat must be non-empty". Treat exact_edits as a queue: the next worker turn executes only the first item, so put one source edit first and leave tests or follow-up edits for later turns. Make the next slice one behavior only: one source file plus at most one focused test file, no validation matrix, no bundled prefix+rename+deserialization work, and no broad "implement the feature" instruction.
For revision small_patch_slice, write from the current accumulated worktree.patch. Preserve useful existing edits, then name the next delta to add; if patch_decision=revise_previous, assume Mixmod will restore the earlier candidate before the worker turn and describe only the follow-up edit after rollback.
For large functions or code-generation paths, make the next exact_edit a local transformation near one anchor, such as collecting a field set, adding one branch, or wiring one helper call. If the full behavior requires several branches, ask for the first branch that creates useful progress.
If prior small slices were clean and the next useful step is naturally a behavior path, do not keep splitting it into isolated validation or setup edits. Use bounded_feature_slice only when the selected worker guidance allows that shape; otherwise make the small_patch_slice exact_edit cover that behavior path with clear anchors and exclusions.
If your read-only review found the relevant source lines, put the minimal file/symbol/anchor context in edit_packet or source_snippets so the worker can edit before rereading the whole method.
For source edits inside large functions or code-generation paths, include structure-preserving constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete/reindent unrelated branches, and edit only the focused block.
For syntax failures in string-literal or generated-code logic, do not guess brittle brace or quote replacements unless your provided artifacts prove the exact replacement. Prefer a compile-driven repair slice: preserve the intended generated code, change the smallest local expression, run the focused parser/compile check, and revise again from that check output if needed.
When a revision needs source details, inspect repo files read-only before returning JSON and name exact symbols plus a literal nearby code anchor when possible, such as `near the line containing "..."`
in exact_edits. Do not ask the worker to investigate broadly or complete the whole feature in one revision slice.
When patch-comparison.json is present, treat it as neutral structural telemetry about the previous candidate, current worktree.patch, and latest worker delta. Mixmod does not judge whether the current patch is better or worse. Large generated or checked-in derived diffs can be normal when they follow from a focused source change; judge them by task context and source intent. Choose patch_decision yourself: use accept_current when the current worktree.patch should stand, revise_current when the current patch should be edited further, and revise_previous only when your own review decides the previous candidate should be restored. Mixmod will restore the previous candidate worktree before the next worker turn. If you choose revise_previous, summarize the focused follow-up edit to apply after rollback in message_to_worker; do not tell the worker to read previous-worktree.patch or any Mixmod artifact.
Put only repo source/test paths in focus_files. Do not put Mixmod artifacts such as revision-task JSON files in focus_files. Do not ask the worker to inspect Mixmod state or artifact directories.
Important artifact semantics: worktree.patch is the accumulated current repository diff and is authoritative for deciding whether the patch exists; changes.patch is only the latest worker run delta and may be empty after a verification-only revision.
When supervision-loop-summary.json is present, treat it as observed cross-turn telemetry only: use it to judge whether worker slices are too small, too large, context-pressured, or making progress, but keep responsibility for the next handoff decision yourself.
Use stop only to record a blocked or inconclusive worker result when no useful worker path remains. Stop does not permit direct supervisor editing.
Working repo: {work_dir}
Instruction: {instruction}
{artifacts}
"#,
        work_dir = work_dir.display(),
    ))
}

pub(crate) fn supervisor_live_control_prompt(
    work_dir: &Path,
    snapshot: &LiveWorkerSnapshot,
    worker_guidance: &WorkerSupervisorGuidance,
) -> Result<String> {
    let snapshot_json = serde_json::to_string_pretty(snapshot)
        .context("failed to serialize live worker snapshot")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    Ok(format!(
        r#"You are the Mixmod supervisor inspecting a worker turn while it is still running.
Do not edit files. Do not run tests. Do not ask the user for approval.
{worker_guidance}
Return only JSON matching this schema:
{{"action":"wait|interrupt_continue|interrupt_context_focus|stop","worker_mode":"continue|context_focus","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"small_patch_slice|bounded_feature_slice|default","turn_goal":"optional next slice goal","exact_edits":["optional concrete edit"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
Treat applicable worker-model guidance as process-control constraints. If the selected worker guidance prefers small_patch_slice and the current turn shows no delta, context overflow, or repeated broad reading, any interrupt should be patch-first: one repo file, one concrete source edit, deferred checks, and no broad feature instruction.
Use wait when the worker is making useful progress or when the evidence is ambiguous.
Use interrupt_continue to stop the current worker process and resume the same worker session with a sharper instruction.
Use interrupt_context_focus to stop the current worker process and start a fresh worker session on the same worktree. Prefer this after context overflow, repeated no-delta rereading, or stale/harmful worker context.
Use stop only when the worker loop is clearly blocked and another worker turn is unlikely to help.
Do not solve the task yourself. Your job is process control: decide whether to keep waiting or steer the worker.
The worker can read and edit only the working repo. It cannot read Mixmod task, state, log, or artifact paths.
Do not mention worker-task.json, revision task files, /tmp/mixmod*, /tmp/mixmod-state, or artifact/log paths in message_to_worker or focus_files.
Put only repo source/test paths in focus_files. If you interrupt, restate the next repo edit directly instead of telling the worker to inspect a task or artifact file.
Keep every intervention anchored to worker_instruction_excerpt, which is the current worker task.
Use stdout_tail and recent_tool_events only to judge worker progress or confusion. Do not invent a different cleanup, bug, or objective from code snippets in stdout_tail.
If new_delta_bytes is 0 and recent_tool_events show repeated reads/searches of the same target, prefer an interrupt over waiting unless stdout shows a concrete edit is imminent. Make the message patch-only: one repo file, one concrete source edit, no tests before a diff exists.
If new_delta_bytes is greater than 0, use wait while the worker is actively editing or checking. If it becomes stale after a failed check, interrupt only with a repair-focused instruction that preserves the current patch and asks for the smallest compile-driven fix; do not add a new feature slice in the same intervention.
If context_overflow_count is positive and no new delta exists, prefer interrupt_context_focus with worker_turn_shape="small_patch_slice", one exact_edits item, and a compact restatement of the exact next source edit.
If worker_session_token_peak is high relative to the worker context window and new_delta_bytes is 0, treat the turn as context-pressured; prefer interrupt_context_focus with a smaller patch-first instruction when broad reading is repeating.
If live_control_check_index equals live_control_check_limit, opencode_segment is greater than 1, and new_delta_bytes is still 0 after earlier interventions, prefer stop unless stdout shows an edit is imminent.
Working repo: {work_dir}

Live worker snapshot:
```json
{snapshot_json}
```
"#,
        work_dir = work_dir.display()
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
        r#"You are revising your Mixmod supervisor revision decision before the worker sees it.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
Your previous revision decision did not fit the selected worker profile. The selected worker needs a smaller, patch-first revision slice than the previous feedback provided.
Mixmod is not designing a replacement slice for you. You are responsible for adapting the decision to the worker-model guidance below.
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
- completion_gate mentioning git diff --stat
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
        r#"You are retrying your Mixmod supervisor revision decision.
Do not edit files. Do not run tests. Emit minified JSON only; no markdown and no explanation.
The previous repair still did not fit the selected worker profile: {rejection_reason}
Mixmod is not designing a replacement slice for you. You are responsible for adapting the decision to the worker-model guidance below.
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
- completion_gate mentioning git diff --stat
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
