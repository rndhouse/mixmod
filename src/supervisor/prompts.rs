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
    let shape_contract = supervisor_worker_shape_contract(worker_guidance);
    let worker_guidance = render_worker_guidance(worker_guidance);
    let init_instructions = worker_brief_init_instructions(init_mode);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are the Mixmod supervisor for a local worker.

Mission: complete the task while minimizing expensive supervisor GPT output tokens. Local worker tokens are cheap. Use the worker for concrete implementation, focused inspection, and verification whenever it can plausibly make progress. Keep your own visible output compact, but remain accountable for correctness; a false or premature handoff/approval wastes the run.

{init_instructions}
The worker receives the original task JSON and can inspect, edit, and test the repo.
{worktree_policy}
Candidate repo file contents are not embedded. Inspect repo files or git state only when that improves the handoff.

Worker profile:
{worker_guidance}

Worker shape contract:
{shape_contract}

Choose the cheapest reliable next worker handoff:
- Use {{"handoff":"as_given"}} only when the original task already gives enough files, behavior, and checks; still include worker_turn_shape and related boundary fields when the selected worker shape contract requires them.
- Use "guided" or "focused" for normal implementation work.
- Use worker_turn_shape="planning_probe" with expect_patch=false only when a short worker investigation can save supervisor output or avoid a bad implementation handoff. Ask for a compact proposal, not edits.
- For expected-patch implementation handoffs, obey the worker shape contract before choosing field detail.
- Mixmod will not repair or reshape a broad handoff to fit the worker profile. The worker sees your parsed JSON as written.
- Choose the largest coherent request this worker is likely to complete cleanly within the selected worker shape contract. Do not make requests tiny by default; broaden when the worker can bear it, narrow when ambiguity, context risk, or the selected contract calls for decomposition.
- Treat worker profile patch-size guidance as a decomposition budget. If the full task likely crosses that budget or spans independent implementation slices, hand off only the next reviewable slice.
- If the route is clear, hand off concrete source edits instead of spending GPT output explaining the whole solution.
- For generated outputs, keep the request bounded to intentional repo outputs. Ask the worker to leave no transient generator/debug/build sidecars and to report broad unrelated generator churn instead of carrying it forward.

Handoff requirements:
- Emit minified JSON only; no markdown, no explanation.
- Required field: "handoff" = "as_given" | "focused" | "guided" | "blocked".
- Include "expect_patch":true when the worker should edit the repo.
- Use concrete repo file paths, not directories.
- message_to_worker is only the short command the worker should follow.
- exact_edits is optional. Use it only when you already know the precise source edit or when a prior worker turn drifted.
- If exact_edits is present, use immediately executable edit instructions; do not use broad "investigate/understand/design" wording there.
- Put checks in deferred_checks when they should run only after a non-empty diff exists.
- edit_packet/source_snippets are optional and sparse. Omit them by default; include them only when you already have a cheap anchor/snippet and it will likely save more worker exploration than it costs in supervisor output.
- stop_condition is the worker-visible point where this turn should stop and return for supervisor review; include it when the selected worker shape contract requires a reviewable slice boundary.
- completion_gate is only for acceptance criteria not already covered by the patch request or deferred_checks.
- scope_rationale is only for a compact supervisor-visible justification when you intentionally choose a broad or full-task worker scope despite the selected worker contract.
- forbidden_actions is only for task-specific limits beyond normal noninteractive worker behavior.
- Omit optional fields unless they reduce worker confusion or supervisor output.

JSON shape:
{{"handoff":"guided","expect_patch":true,"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"one-turn goal","message_to_worker":"short worker instruction","files":["repo/path"],"exact_edits":["optional concrete edit"],"edit_packet":["optional cost-justified anchor"],"source_snippets":["optional cost-justified snippet"],"edit_plan":["optional short steps"],"checks":["optional checks"],"deferred_checks":["checks after patch"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional limits"],"investigation_summary":"optional short finding","evidence":["optional file/function clues"],"avoid":["optional constraints"],"risk":"optional short risk"}}
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

fn supervisor_worker_shape_contract(worker_guidance: &WorkerSupervisorGuidance) -> &'static str {
    if worker_guidance
        .guidance
        .iter()
        .any(|item| item.contains("worker_turn_shape=bounded_feature_slice"))
    {
        return r#"Profile-selected shape: for expected-patch implementation handoffs, prefer worker_turn_shape="bounded_feature_slice". Use patch_request only when ambiguity, context risk, or prior worker evidence calls for a smaller source slice. Use planning_probe with expect_patch=false only for bounded investigation."#;
    }
    if worker_guidance
        .guidance
        .iter()
        .any(|item| item.contains("worker_turn_shape=patch_request"))
    {
        return r#"Patch-request decomposition contract: for expected-patch implementation handoffs, use worker_turn_shape="patch_request". This is your responsibility before emitting JSON; Mixmod will not run an automatic repair turn. Choose one bounded, reviewable implementation slice expected to fit the worker patch-size guidance. When the overall task spans multiple independent behaviors, layers, generated outputs, verification steps, or likely exceeds the worker patch budget, decompose it yourself before emitting JSON: hand off the next slice only, not the full task. Use concrete file paths for that slice, normally a small authored-source set; include generated or large files only when they are part of that slice and name the command or boundary. Include a worker-visible stop_condition that tells the worker to stop after the slice has one useful tracked diff and return for supervisor review. If you intentionally ask for broad or full-task scope, include scope_rationale explaining why the request remains within the profile's patch-size guidance, known file boundary, and acceptable worker-session risk. Do not emit worker_turn_shape="bounded_feature_slice" or "default" for expected-patch work. Use planning_probe with expect_patch=false only when bounded worker investigation is cheaper than supervisor investigation."#;
    }
    "No worker-specific default shape is selected. Choose one shape deliberately: planning_probe for no-patch investigation, patch_request for a focused edit, bounded_feature_slice for a coherent larger feature chunk, or default only when the task is already simple."
}

fn worker_brief_init_instructions(init_mode: SupervisorInitMode) -> &'static str {
    match init_mode {
        SupervisorInitMode::Compact => {
            r#"Use the task JSON and candidate repo paths first. Do not run tests, install dependencies, implement the patch, or ask the user for approval. Inspect the repo only if that prevents a likely bad handoff. Target <=160 supervisor output tokens for normal tasks."#
        }
        SupervisorInitMode::Investigate => {
            r#"Use the task JSON and candidate repo paths first. Inspect repo context only when that is needed to create a reliable low-output worker handoff. If needed, you may inspect source/test files and run discovery commands such as `rg`, `find`, `ls`, `sed`, `git status`, `git diff`, or `git grep`. Do not run tests, install dependencies, inspect Mixmod state/artifact directories, or ask the user for approval. Stop inspecting once you can choose a reliable worker handoff. Target <=500 supervisor output tokens."#
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
    let shape_contract = supervisor_worker_shape_contract(worker_guidance);
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are a terse supervisor reviewing a local worker.
{worktree_policy}
Do not ask the user for approval.
Inspect the listed artifact files directly before deciding. Do not rely on this prompt as artifact content; it only names where the review evidence lives.
Treat supervisor input tokens as scarce. Inspect only the artifacts needed for the next decision and stop reading once approve, revise, or stop is clear.
For ordinary worker-turn review, start with task context, compact metadata, and changes.patch. Inspect worktree.patch only when considering approval, rollback, or an integration question that depends on the full active diff.
{worker_guidance}
Worker shape contract:
{shape_contract}

If you choose revise, shape the worker request yourself before emitting JSON. Mixmod will not repair or reshape the revision to fit the worker profile.
Return only JSON matching this schema:
{{"action":"approve|revise|stop","expect_patch":true,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"]}}
Use "expect_patch":false with worker_turn_shape="planning_probe" when the next useful worker turn should only inspect bounded repo context and propose the next patch request. After a planning_probe result, approve or trim its proposal by issuing a normal revise implementation turn; do not approve the whole task merely because the plan is reasonable.
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
{{"action":"wait|interrupt_continue|interrupt_context_focus|abort_worker_turn","worker_mode":"continue|context_focus","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
Treat applicable worker-model guidance as context for shaping message_to_worker if you choose to interrupt.
Available actions:
- wait: let the worker continue.
- interrupt_continue: stop the current worker process and resume the same worker session with a new instruction.
- interrupt_context_focus: stop the current worker process and start a fresh worker session on the same worktree with a focused instruction.
- abort_worker_turn: stop only the active worker process and return to the ordinary supervisor review.
Base the action on the live evidence. Do not assume an intervention is required because a risk signal is present.
Use new_delta_bytes, stdout_log_path, stderr_log_path, tool_events_path, context_overflow_count, worker_session_token_peak, worker_backend_telemetry, elapsed time, and last output age only as evidence for worker progress, confusion, or blockage.
If you need detailed stdout, stderr, or tool-call history, inspect stdout_log_path, stderr_log_path, or tool_events_path yourself. Do not pass those artifact paths to the worker.
If you interrupt, keep message_to_worker bounded to worker_instruction_excerpt, the live evidence, and the selected worker guidance. For patch_request workers, keep any interrupt patch-first: focused repo files, one concrete source behavior, deferred checks, and no broad feature instruction.
If the current worker_instruction_excerpt is a planning_probe and you choose interrupt_context_focus, restate enough of the original task goal, focused files, and planning questions in message_to_worker for a fresh worker session to answer without prior context.
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

#[derive(Default)]
struct SupervisorFeedbackPromptSignals {
    artifact_names: BTreeSet<String>,
    worker_turn_shape: Option<String>,
    context_overflow: bool,
    context_pressure: bool,
    supervisor_control_seen: bool,
    patch_request_progress_streak: bool,
}

fn supervisor_feedback_review_context(artifact_paths: &[PathBuf]) -> String {
    let signals = supervisor_feedback_prompt_signals(artifact_paths);
    let mut sections = vec![supervisor_feedback_core_context(&signals)];

    if signals.worker_turn_shape.as_deref() == Some("patch_request") {
        sections.push(
            r#"Patch request context:
- Treat a first non-empty delta as progress, not proof of completion.
- If more work is needed, make the next revision one source behavior with a bounded patch goal; include exact_edits only when precision saves supervisor output.
- When the current active diff is useful incomplete progress and the next turn should not reread it as a patch, use patch_decision=accept_current_baseline with worker_mode=context_focus.
- Otherwise revise from the current worktree and preserve useful existing edits.
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
- If another revision is needed, prefer a smaller next request.
- Use worker_mode=context_focus when continuing would require broad rereading or the previous context appears stale."#
                .to_string(),
        );
    }

    if signals.supervisor_control_seen {
        sections.push(
            r#"Live-control context:
- Supervisor control already intervened during a worker turn.
- Judge whether the previous request was too broad, too vague, or stale before issuing another revision.
- Prefer one focused repair or verification step over adding a new feature concern in the same handoff."#
                .to_string(),
        );
    }

    if signals.patch_request_progress_streak {
        sections.push(
            r#"Slice-sizing context:
- Multiple recent patch-request turns produced non-empty deltas without context overflow.
- If the current source state is coherent but incomplete, the next patch_request may cover one larger anchored source behavior.
- Keep the selected worker profile's preferred shape unless the profile explicitly supports broadening."#
                .to_string(),
        );
    }

    if signals.has_any(&[
        PATCH_COMPARISON,
        PREVIOUS_WORKTREE_PATCH,
        PATCH_BASELINE_JSON,
        BASELINE_ACCEPTED_PATCH,
        BASELINE_ACTIVE_PATCH,
        PATCH_ROLLBACK_JSON,
        ROLLBACK_CURRENT_PATCH,
        ROLLBACK_RESTORED_PATCH,
    ]) {
        sections.push(
            r#"Patch checkpoint context:
- Treat patch-comparison.json as neutral structural telemetry; Mixmod is not judging patch quality.
- Choose patch_decision yourself from the task, current patch, and checkpoint artifacts.
- Use accept_current_baseline when the current active patch is useful incomplete progress and the next worker should start from a clean active diff to avoid cumulative context cost. Mixmod creates an internal checkpoint commit and reconstructs the final benchmark patch from the original base.
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
- worktree.patch is the active current diff; changes.patch is only the latest worker-turn delta. Avoid opening worktree.patch unless approval, rollback, or integration with prior edits depends on it.
{tool_evidence}
- Minimize supervisor input tokens: do not inspect more artifacts, logs, or diff content once the next action is clear.
- For generated-output diffs, inspect authored-source changes and patch stats first. Avoid opening whole generated files; judge whether generated changes are bounded expected outputs and free of transient tool sidecars.
- Approve only when the current source state appears to satisfy the original task and no worker action or check remains. Before approving, inspect task.json and enough source/diff state to verify completion.
- Treat a false approval as a terminal correctness failure. If evidence is missing for the main requested behavior or a likely edge case, choose revise for a targeted verification or repair turn.
- On approve, required_checks and deferred_checks must be empty and completion_gate must be absent or empty.
- Revise when a useful worker path remains; message_to_worker must be concrete and worker-executable.
- Stop only for a blocked or inconclusive worker result when no useful worker path remains.
- The worker owns implementation. Do not author task-solving source changes.
- Put only repo source/test paths in focus_files. Do not ask the worker to inspect Mixmod artifacts.
- worker_mode=continue reuses the current worker session; worker_mode=context_focus starts a fresh worker session on the same source tree.
- worktree.patch is the active diff against the current baseline. After patch_decision=accept_current_baseline, earlier accepted progress is in the source tree rather than the active diff.
- Use patch_decision=accept_current_baseline when useful incomplete progress should become baseline before the next worker turn, especially when cumulative diff visibility would waste worker or supervisor tokens.
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
        self.patch_request_progress_streak |=
            get_u64(&summary, "patch_request_nonempty_delta_streak").unwrap_or(0) >= 2;

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
        WORKTREE_PATCH => "active current repository diff",
        CHANGES_PATCH => "latest worker-turn diff",
        INTERVENTIONS_JSONL => "Mixmod intervention audit log",
        METRICS_JSON => "worker-run metrics and signals",
        PATCH_COMPARISON => "neutral patch checkpoint comparison",
        PREVIOUS_WORKTREE_PATCH => "previous candidate patch available for rollback decisions",
        PATCH_BASELINE_JSON => "baseline receipt for accept_current_baseline",
        BASELINE_ACCEPTED_PATCH => "patch accepted into the internal baseline",
        BASELINE_ACTIVE_PATCH => "active patch after the internal baseline checkpoint",
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
        "Supervisor-only worker-model guidance for {}:\nUse relevant bullets as constraints for handoff shape, patch size, review, and live control. Do not copy the list to the worker; convert only the needed points into short worker-facing instructions.\n",
        worker_guidance.model
    );
    if worker_guidance.target_patch_lines.is_some() || worker_guidance.max_patch_lines.is_some() {
        let target = worker_guidance
            .target_patch_lines
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unspecified".to_string());
        let max = worker_guidance
            .max_patch_lines
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unspecified".to_string());
        rendered.push_str("Worker patch-size guidance: aim for a worker turn expected around ");
        rendered.push_str(&target);
        rendered.push_str(" changed lines, with a soft maximum around ");
        rendered.push_str(&max);
        rendered.push_str(" changed lines. This is supervisor planning guidance only, not a Mixmod gate; choose a coherent slice expected to fit it and intentionally exceed it only when that saves a useful worker turn.\n");
    }
    for item in &worker_guidance.guidance {
        rendered.push_str("- ");
        rendered.push_str(item.trim());
        rendered.push('\n');
    }
    rendered
}
