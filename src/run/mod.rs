use crate::*;
use std::sync::Arc;

mod command;
mod context;
mod prompts;
mod recovery;
mod report;
mod session;

pub(crate) use command::shell_command;
pub(crate) use context::{WorkerContextSignals, worker_context_signals, worker_session_token_peak};
pub(crate) use prompts::build_opencode_instruction;
use recovery::{
    EmptyPatchFollowup, EmptyPatchFollowupRequest, RevisionNoopContext, RevisionNoopFollowup,
    RevisionNoopFollowupRequest, WorkerSelfReview, WorkerSelfReviewRequest, merge_worker_outputs,
    run_empty_patch_followup, run_revision_noop_followup, run_worker_self_review,
    should_run_empty_patch_followup, should_run_revision_noop_followup,
    worker_self_review_skip_reason,
};
pub(crate) use report::build_run_summary;
#[cfg(test)]
pub(crate) use report::opencode_exit_status_label;
use report::{RunReportInput, build_run_report};
use session::{build_reasoning_trace_jsonl, build_session_jsonl};

pub fn run_mixmod_task(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn AgentHarness,
) -> Result<Receipt> {
    run_mixmod_task_with_options(root, mode, task_arg, out_arg, runner, false)
}

pub fn run_mixmod_task_with_options(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn AgentHarness,
    require_local: bool,
) -> Result<Receipt> {
    run_mixmod_task_with_session(root, mode, task_arg, out_arg, runner, require_local, None)
}

pub(crate) fn run_mixmod_task_with_session(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn AgentHarness,
    require_local: bool,
    resume_session_id: Option<String>,
) -> Result<Receipt> {
    run_mixmod_task_with_worker_options(
        root,
        mode,
        task_arg,
        out_arg,
        runner,
        require_local,
        WorkerRunOptions {
            resume_session_id,
            ..WorkerRunOptions::default()
        },
    )
}

pub(crate) struct WorkerRunOptions {
    pub(crate) resume_session_id: Option<String>,
    pub(crate) allow_auto_followups: bool,
    pub(crate) worker_self_review: bool,
    pub(crate) supervisor_advisor: Option<Arc<dyn SupervisorAdvisor>>,
}

impl Default for WorkerRunOptions {
    fn default() -> Self {
        Self {
            resume_session_id: None,
            allow_auto_followups: true,
            worker_self_review: false,
            supervisor_advisor: None,
        }
    }
}

pub(crate) fn run_mixmod_task_with_worker_options(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn AgentHarness,
    require_local: bool,
    options: WorkerRunOptions,
) -> Result<Receipt> {
    MixmodRun {
        root,
        mode,
        task_arg,
        out_arg,
        runner,
        require_local,
        resume_session_id: options.resume_session_id,
        allow_auto_followups: options.allow_auto_followups,
        worker_self_review: options.worker_self_review,
        supervisor_advisor: options.supervisor_advisor,
    }
    .execute()
}

fn write_opencode_logs(logs_dir: &Path, stdout: &[u8], stderr: &[u8]) -> Result<()> {
    atomic_write(&logs_dir.join("opencode.stdout.txt"), stdout)?;
    atomic_write(&logs_dir.join("opencode.stderr.txt"), stderr)?;
    let (events, _count) = build_opencode_events_jsonl(stdout)?;
    atomic_write(&logs_dir.join(OPENCODE_EVENTS_JSONL), events.as_bytes())?;
    Ok(())
}

struct CapturedRunPatch {
    worktree_patch: String,
    patch: String,
}

fn capture_run_patch(
    root: &Path,
    before_diff: Option<&str>,
    notes: &mut Vec<String>,
    context: Option<&str>,
) -> CapturedRunPatch {
    let worktree_patch = match git_diff_with_untracked(root) {
        Ok(after_diff) => after_diff,
        Err(error) => {
            if let Some(context) = context {
                notes.push(format!(
                    "Unable to capture git diff after {context}: {error}"
                ));
            } else {
                notes.push(format!("Unable to capture git diff: {error}"));
            }
            String::new()
        }
    };
    let patch = diff_without_unchanged_blocks(&worktree_patch, before_diff.unwrap_or_default());
    CapturedRunPatch {
        worktree_patch,
        patch,
    }
}

fn write_patch_artifacts(
    out_dir: &Path,
    captured_patch: &CapturedRunPatch,
    output: &AgentOutput,
    notes: &mut Vec<String>,
) -> Result<()> {
    atomic_write(
        &out_dir.join(CHANGES_PATCH),
        captured_patch.patch.as_bytes(),
    )?;
    atomic_write(
        &out_dir.join(WORKTREE_PATCH),
        captured_patch.worktree_patch.as_bytes(),
    )?;
    if output.timed_out || output.idle_timed_out {
        atomic_write(
            &out_dir.join(PARTIAL_PATCH),
            captured_patch.patch.as_bytes(),
        )?;
        notes.push(
            "The worker did not finish normally; partial.patch preserves the worktree diff captured after termination."
                .to_string(),
        );
    }
    Ok(())
}

struct MixmodRun<'a> {
    root: &'a Path,
    mode: DelegationMode,
    task_arg: &'a Path,
    out_arg: &'a Path,
    runner: &'a dyn AgentHarness,
    require_local: bool,
    resume_session_id: Option<String>,
    allow_auto_followups: bool,
    worker_self_review: bool,
    supervisor_advisor: Option<Arc<dyn SupervisorAdvisor>>,
}

impl MixmodRun<'_> {
    fn execute(self) -> Result<Receipt> {
        let Self {
            root,
            mode,
            task_arg,
            out_arg,
            runner,
            require_local,
            resume_session_id,
            allow_auto_followups,
            worker_self_review,
            supervisor_advisor,
        } = self;
        let run_id = make_run_id("run");
        let task_path = absolutize(root, task_arg);
        let out_dir = absolutize(root, out_arg);
        let logs_dir = out_dir.join("logs");
        fs::create_dir_all(&logs_dir)
            .with_context(|| format!("failed to create {}", logs_dir.display()))?;

        let (task_value, task_spec) = read_task_json(&task_path)?;
        write_pretty_json(&out_dir.join(TASK_JSON), &task_value, "run task")?;

        let expect_patch = expect_patch_for_run(mode, &task_value);
        let interventions_path = out_dir.join(INTERVENTIONS_JSONL);
        let mut intervention_log = InterventionLog::new();
        let session_id = make_run_id("worker-session");
        let instruction = build_opencode_instruction(mode, &task_spec, &task_path, &out_dir)?;
        let instruction_path = out_dir.join(OPENCODE_INSTRUCTIONS_MD);
        atomic_write(&instruction_path, instruction.as_bytes())?;
        let initial_session_policy = if resume_session_id.is_some() {
            InterventionSessionPolicy::SameSession
        } else {
            InterventionSessionPolicy::FreshSession
        };
        intervention_log.record(
            InterventionEvent::new(
                InterventionKind::WorkerHandoff,
                InterventionPhase::PreWorker,
                InterventionTarget::Worker,
                "initial_worker_instruction",
                "instruction_written",
            )
            .with_session_policy(initial_session_policy)
            .with_artifacts(vec![
                TASK_JSON.to_string(),
                OPENCODE_INSTRUCTIONS_MD.to_string(),
            ])
            .with_details(intervention_details([
                ("mode", json!(mode.to_string())),
                ("expect_patch", json!(expect_patch)),
                ("task", json!(display_path(root, &task_path))),
            ])),
        );

        let request = AgentRequest {
            root: root.to_path_buf(),
            mode,
            task_path: task_path.clone(),
            out_dir: out_dir.clone(),
            instruction_path,
            instruction,
            session_id,
            resume_session_id,
            require_local,
            supervisor_advisor,
        };

        let start_timestamp = Utc::now();
        let start = Instant::now();
        let before_diff = git_diff_with_untracked(root).ok();
        let mut output = runner.run(&request)?;
        let mut end_timestamp = Utc::now();
        let mut wall_clock_ms = start.elapsed().as_millis();
        let mut empty_patch_followup = EmptyPatchFollowup::new();
        let revision_context = RevisionNoopContext::from_task(&task_value);
        let mut revision_noop_followup = RevisionNoopFollowup::new(
            revision_context
                .as_ref()
                .is_some_and(|ctx| ctx.delta_expected),
        );
        let worker_self_review_forced =
            env_bool("MIXMOD_WORKER_SELF_REVIEW_FORCE").unwrap_or(false);
        let mut worker_self_review =
            WorkerSelfReview::new(worker_self_review, worker_self_review_forced);

        write_opencode_logs(&logs_dir, &output.stdout, &output.stderr)?;

        let mut notes = vec![
        "Exact supervisor token telemetry is unavailable to this prototype unless provided manually."
            .to_string(),
        "Worker output is untrusted until the supervisor reviews the compact artifacts and patch."
            .to_string(),
    ];
        if before_diff.as_deref().unwrap_or_default().trim().is_empty() {
            notes.push("No pre-existing git diff was detected before the worker ran.".to_string());
        } else {
            notes.push(
                "A pre-existing git diff was present before the worker ran; changes.patch subtracts unchanged pre-existing diff blocks."
                    .to_string(),
            );
        }
        notes.push(
            "worktree.patch contains the accumulated current repository diff for supervisor review."
                .to_string(),
        );

        let mut captured_patch = capture_run_patch(root, before_diff.as_deref(), &mut notes, None);
        if allow_auto_followups
            && should_run_revision_noop_followup(
                mode,
                expect_patch,
                revision_context.as_ref(),
                &output,
                &captured_patch.patch,
            )
        {
            let revision_context = revision_context
                .as_ref()
                .expect("revision context is present when revision no-op follow-up is needed");
            revision_noop_followup.triggered = true;
            revision_noop_followup.reason = Some(
                "revision worker run expected a new delta but no changes.patch was captured"
                    .to_string(),
            );
            match run_revision_noop_followup(RevisionNoopFollowupRequest {
                root,
                mode,
                task: &task_spec,
                task_path: &task_path,
                out_dir: &out_dir,
                runner,
                require_local,
                original_request: &request,
                output: &output,
                revision: revision_context,
            }) {
                Ok((followup_output, followup_dir)) => {
                    revision_noop_followup.performed = true;
                    revision_noop_followup.run_dir = Some(display_path(root, &followup_dir));
                    notes.push(format!(
                        "Revision no-op follow-up was triggered and ran in {}.",
                        display_path(root, &followup_dir)
                    ));
                    output = merge_worker_outputs(
                        output,
                        followup_output,
                        "revision no-op follow-up",
                        "Revision no-op follow-up output was merged into this run.",
                    );
                    end_timestamp = Utc::now();
                    wall_clock_ms = start.elapsed().as_millis();
                    captured_patch = capture_run_patch(
                        root,
                        before_diff.as_deref(),
                        &mut notes,
                        Some("revision no-op follow-up"),
                    );
                    revision_noop_followup.patch_created = !captured_patch.patch.trim().is_empty();
                    intervention_log.record(
                        InterventionEvent::new(
                            InterventionKind::RevisionNoopFollowup,
                            InterventionPhase::PostWorker,
                            InterventionTarget::Worker,
                            revision_noop_followup
                                .reason
                                .as_deref()
                                .unwrap_or("revision worker run expected a new delta"),
                            if revision_noop_followup.patch_created {
                                "patch_created"
                            } else {
                                "no_patch_created"
                            },
                        )
                        .with_session_policy(InterventionSessionPolicy::SameSession)
                        .with_artifacts(vec![
                            format!("revision-noop-followup/{TASK_JSON}"),
                            format!("revision-noop-followup/{OPENCODE_INSTRUCTIONS_MD}"),
                        ])
                        .with_details(intervention_details([
                            ("patch_created", json!(revision_noop_followup.patch_created)),
                            ("patch_bytes", json!(captured_patch.patch.len() as u64)),
                            ("worker_mode", json!(revision_context.worker_mode)),
                            ("patch_decision", json!(revision_context.patch_decision)),
                        ])),
                    );
                }
                Err(error) => {
                    revision_noop_followup.reason = Some(format!(
                        "revision no-op follow-up was triggered but could not run: {error}"
                    ));
                    notes.push(format!(
                        "Revision no-op follow-up was triggered but could not run: {error}"
                    ));
                    intervention_log.record(
                        InterventionEvent::new(
                            InterventionKind::RevisionNoopFollowup,
                            InterventionPhase::PostWorker,
                            InterventionTarget::Worker,
                            revision_noop_followup
                                .reason
                                .as_deref()
                                .unwrap_or("revision no-op follow-up failed before it could run"),
                            "failed",
                        )
                        .with_session_policy(InterventionSessionPolicy::SameSession)
                        .with_performed(false)
                        .with_artifacts(vec![
                            format!("revision-noop-followup/{TASK_JSON}"),
                            format!("revision-noop-followup/{OPENCODE_INSTRUCTIONS_MD}"),
                        ])
                        .with_details(intervention_details([("error", json!(error.to_string()))])),
                    );
                }
            }
            write_opencode_logs(&logs_dir, &output.stdout, &output.stderr)?;
        } else if allow_auto_followups
            && should_run_empty_patch_followup(mode, expect_patch, &output, &captured_patch.patch)
        {
            empty_patch_followup.triggered = true;
            empty_patch_followup.reason = Some(
                "patch-mode worker run expected a patch but no repository diff was captured"
                    .to_string(),
            );
            match run_empty_patch_followup(EmptyPatchFollowupRequest {
                root,
                mode,
                task: &task_spec,
                task_path: &task_path,
                out_dir: &out_dir,
                runner,
                require_local,
                original_request: &request,
                output: &output,
            }) {
                Ok((followup_output, followup_dir)) => {
                    empty_patch_followup.performed = true;
                    empty_patch_followup.run_dir = Some(display_path(root, &followup_dir));
                    notes.push(format!(
                        "Empty-patch follow-up was triggered and ran in {}.",
                        display_path(root, &followup_dir)
                    ));
                    output = merge_worker_outputs(
                        output,
                        followup_output,
                        "empty-patch follow-up",
                        "Empty-patch follow-up output was merged into this run.",
                    );
                    end_timestamp = Utc::now();
                    wall_clock_ms = start.elapsed().as_millis();
                    captured_patch = capture_run_patch(
                        root,
                        before_diff.as_deref(),
                        &mut notes,
                        Some("empty-patch follow-up"),
                    );
                    empty_patch_followup.patch_created = !captured_patch.patch.trim().is_empty();
                    intervention_log.record(
                        InterventionEvent::new(
                            InterventionKind::EmptyPatchFollowup,
                            InterventionPhase::PostWorker,
                            InterventionTarget::Worker,
                            empty_patch_followup
                                .reason
                                .as_deref()
                                .unwrap_or("patch-mode worker run expected a patch"),
                            if empty_patch_followup.patch_created {
                                "patch_created"
                            } else {
                                "no_patch_created"
                            },
                        )
                        .with_session_policy(InterventionSessionPolicy::SameSession)
                        .with_artifacts(vec![
                            format!("empty-patch-followup/{TASK_JSON}"),
                            format!("empty-patch-followup/{OPENCODE_INSTRUCTIONS_MD}"),
                        ])
                        .with_details(intervention_details([
                            ("patch_created", json!(empty_patch_followup.patch_created)),
                            ("patch_bytes", json!(captured_patch.patch.len() as u64)),
                        ])),
                    );
                }
                Err(error) => {
                    empty_patch_followup.reason = Some(format!(
                        "empty-patch follow-up was triggered but could not run: {error}"
                    ));
                    notes.push(format!(
                        "Empty-patch follow-up was triggered but could not run: {error}"
                    ));
                    intervention_log.record(
                        InterventionEvent::new(
                            InterventionKind::EmptyPatchFollowup,
                            InterventionPhase::PostWorker,
                            InterventionTarget::Worker,
                            empty_patch_followup
                                .reason
                                .as_deref()
                                .unwrap_or("empty-patch follow-up failed before it could run"),
                            "failed",
                        )
                        .with_session_policy(InterventionSessionPolicy::SameSession)
                        .with_performed(false)
                        .with_artifacts(vec![
                            format!("empty-patch-followup/{TASK_JSON}"),
                            format!("empty-patch-followup/{OPENCODE_INSTRUCTIONS_MD}"),
                        ])
                        .with_details(intervention_details([("error", json!(error.to_string()))])),
                    );
                }
            }
            write_opencode_logs(&logs_dir, &output.stdout, &output.stderr)?;
        }
        worker_self_review.reason = worker_self_review_skip_reason(
            worker_self_review.enabled,
            worker_self_review.forced,
            mode,
            expect_patch,
            &output,
            &captured_patch.patch,
        );
        if worker_self_review.reason.is_none() {
            let patch_before_self_review = captured_patch.patch.clone();
            worker_self_review.triggered = true;
            worker_self_review.reason = Some(
                "patch-mode worker run produced a diff eligible for same-session self-review"
                    .to_string(),
            );
            match run_worker_self_review(WorkerSelfReviewRequest {
                root,
                mode,
                task: &task_spec,
                task_path: &task_path,
                review_patch: &captured_patch.patch,
                out_dir: &out_dir,
                runner,
                require_local,
                original_request: &request,
                output: &output,
            }) {
                Ok((review_output, review_dir)) => {
                    worker_self_review.performed = true;
                    worker_self_review.run_dir = Some(display_path(root, &review_dir));
                    notes.push(format!(
                        "Worker self-review was enabled and ran in {}.",
                        display_path(root, &review_dir)
                    ));
                    output = merge_worker_outputs(
                        output,
                        review_output,
                        "worker self-review",
                        "Worker self-review output was merged into this run.",
                    );
                    end_timestamp = Utc::now();
                    wall_clock_ms = start.elapsed().as_millis();
                    captured_patch = capture_run_patch(
                        root,
                        before_diff.as_deref(),
                        &mut notes,
                        Some("worker self-review"),
                    );
                    worker_self_review.patch_changed =
                        captured_patch.patch != patch_before_self_review;
                    intervention_log.record(
                        InterventionEvent::new(
                            InterventionKind::WorkerSelfReview,
                            InterventionPhase::PostWorker,
                            InterventionTarget::Worker,
                            worker_self_review
                                .reason
                                .as_deref()
                                .unwrap_or("worker self-review was enabled"),
                            if worker_self_review.patch_changed {
                                "patch_changed"
                            } else {
                                "no_patch_change"
                            },
                        )
                        .with_session_policy(InterventionSessionPolicy::SameSession)
                        .with_artifacts(vec![
                            format!("worker-self-review/{TASK_JSON}"),
                            format!("worker-self-review/{OPENCODE_INSTRUCTIONS_MD}"),
                            "worker-self-review/worker-session.patch".to_string(),
                        ])
                        .with_details(intervention_details([
                            ("patch_changed", json!(worker_self_review.patch_changed)),
                            ("patch_bytes", json!(captured_patch.patch.len() as u64)),
                            (
                                "review_patch_bytes",
                                json!(patch_before_self_review.len() as u64),
                            ),
                        ])),
                    );
                }
                Err(error) => {
                    worker_self_review.reason = Some(format!(
                        "worker self-review was triggered but could not run: {error}"
                    ));
                    notes.push(format!(
                        "Worker self-review was triggered but could not run: {error}"
                    ));
                    intervention_log.record(
                        InterventionEvent::new(
                            InterventionKind::WorkerSelfReview,
                            InterventionPhase::PostWorker,
                            InterventionTarget::Worker,
                            worker_self_review
                                .reason
                                .as_deref()
                                .unwrap_or("worker self-review failed before it could run"),
                            "failed",
                        )
                        .with_session_policy(InterventionSessionPolicy::SameSession)
                        .with_performed(false)
                        .with_artifacts(vec![
                            format!("worker-self-review/{TASK_JSON}"),
                            format!("worker-self-review/{OPENCODE_INSTRUCTIONS_MD}"),
                            "worker-self-review/worker-session.patch".to_string(),
                        ])
                        .with_details(intervention_details([("error", json!(error.to_string()))])),
                    );
                }
            }
            write_opencode_logs(&logs_dir, &output.stdout, &output.stderr)?;
        }
        write_patch_artifacts(&out_dir, &captured_patch, &output, &mut notes)?;
        let stats = patch_stats(&captured_patch.patch);
        let worktree_stats = patch_stats(&captured_patch.worktree_patch);

        let (reasoning_trace, reasoning_trace_event_count) =
            build_reasoning_trace_jsonl(&output.stdout)?;
        atomic_write(
            &out_dir.join(REASONING_TRACE_JSONL),
            reasoning_trace.as_bytes(),
        )?;
        let (tool_events, tool_event_count) = build_tool_events_jsonl(&output.stdout)?;
        atomic_write(&out_dir.join(TOOL_EVENTS_JSONL), tool_events.as_bytes())?;
        let (_opencode_events, opencode_event_count) = build_opencode_events_jsonl(&output.stdout)?;
        let context_overflow = worker_context_signals(&output.stdout);
        if context_overflow.context_overflow_count > 0 {
            notes.push(format!(
                "Worker stdout reported {} context overflow event(s); prefer worker_mode=context_focus with a smaller next slice if another revision is needed.",
                context_overflow.context_overflow_count
            ));
        }
        let worker_session_token_peak = worker_session_token_peak(&output.stdout);
        if worker_session_token_peak.is_some_and(|tokens| tokens >= 24_000) {
            notes.push(
                "Worker session token peak was high; prefer a smaller next slice or worker_mode=context_focus if another revision is needed."
                    .to_string(),
            );
        }

        let session = build_session_jsonl(&start_timestamp, &end_timestamp, &output)?;
        atomic_write(&out_dir.join(SESSION_JSONL), session.as_bytes())?;

        let needs_supervisor = output.timed_out
            || output.idle_timed_out
            || output.interrupted_by_supervisor
            || (output.success
                && mode == DelegationMode::Patch
                && expect_patch
                && captured_patch.patch.trim().is_empty());
        let status = if needs_supervisor {
            "needs_supervisor"
        } else if output.success {
            "success"
        } else {
            "failed"
        };
        let summary = build_run_summary(status, mode, &output, &stats, &worktree_stats);
        let report = build_run_report(RunReportInput {
            status,
            mode,
            summary: &summary,
            task: &task_spec,
            output: &output,
            stats: &stats,
            worktree_stats: &worktree_stats,
            context_overflow: &context_overflow,
            worker_session_token_peak,
            notes: &notes,
            root,
            out_dir: &out_dir,
        });
        atomic_write(&out_dir.join(REPORT_MD), report.as_bytes())?;
        intervention_log.write_jsonl(&interventions_path)?;

        let compact_artifacts = RUN_COMPACT_ARTIFACTS;
        let report_bytes = file_len(&out_dir.join(REPORT_MD))?;
        let patch_bytes = file_len(&out_dir.join(CHANGES_PATCH))?;
        let worktree_patch_bytes = file_len(&out_dir.join(WORKTREE_PATCH))?;
        let session_bytes = file_len(&out_dir.join(SESSION_JSONL))?;
        let opencode_events_bytes = file_len(&logs_dir.join(OPENCODE_EVENTS_JSONL))?;
        let reasoning_trace_bytes = file_len(&out_dir.join(REASONING_TRACE_JSONL))?;
        let tool_events_bytes = file_len(&out_dir.join(TOOL_EVENTS_JSONL))?;
        let mut metrics = RunMetrics {
            start_timestamp: start_timestamp.to_rfc3339(),
            end_timestamp: end_timestamp.to_rfc3339(),
            wall_clock_ms,
            worker_backend: output.backend.as_str().to_string(),
            opencode_command: output.command_for_metrics.clone(),
            opencode_segments: output.segments.clone(),
            opencode_exit_status: output.exit_status,
            opencode_provider: output.provider.clone(),
            opencode_model: output.model.clone(),
            opencode_model_arg: output.model_arg.clone(),
            opencode_session_label: output.session_label.clone(),
            opencode_session_id: output.session_id.clone(),
            opencode_resume_session_id: output.resume_session_id.clone(),
            worker_session_reused: output.session_reused,
            interrupted_by_supervisor: output.interrupted_by_supervisor,
            supervisor_control_action: output.supervisor_control_action.clone(),
            supervisor_control_events: output.supervisor_control_events.clone(),
            opencode_timed_out: output.timed_out,
            opencode_idle_timed_out: output.idle_timed_out,
            heartbeat_count: output.heartbeat_count,
            expect_patch,
            intervention_count: intervention_log.events().len(),
            intervention_kinds: intervention_log.kind_names(),
            intervention_artifact: display_path(root, &interventions_path),
            empty_patch_followup_triggered: empty_patch_followup.triggered,
            empty_patch_followup_performed: empty_patch_followup.performed,
            empty_patch_followup_patch_created: empty_patch_followup.patch_created,
            empty_patch_followup_reason: empty_patch_followup.reason.clone(),
            empty_patch_followup_run_dir: empty_patch_followup.run_dir.clone(),
            revision_delta_expected: revision_noop_followup.delta_expected,
            revision_delta_bytes: captured_patch.patch.len() as u64,
            revision_noop_followup_triggered: revision_noop_followup.triggered,
            revision_noop_followup_performed: revision_noop_followup.performed,
            revision_noop_followup_patch_created: revision_noop_followup.patch_created,
            revision_noop_followup_reason: revision_noop_followup.reason.clone(),
            revision_noop_followup_run_dir: revision_noop_followup.run_dir.clone(),
            worker_self_review_enabled: worker_self_review.enabled,
            worker_self_review_triggered: worker_self_review.triggered,
            worker_self_review_performed: worker_self_review.performed,
            worker_self_review_patch_changed: worker_self_review.patch_changed,
            worker_self_review_forced: worker_self_review.forced,
            worker_self_review_reason: worker_self_review.reason.clone(),
            worker_self_review_run_dir: worker_self_review.run_dir.clone(),
            require_local: output.require_local,
            local_inference_verified: output.local_inference_verified,
            gpu_activity_observed: output.gpu_activity_observed,
            backend_activity_observed: output.backend_activity_observed,
            verification_notes: output.verification_notes.clone(),
            stdout_bytes: output.stdout.len() as u64,
            stderr_bytes: output.stderr.len() as u64,
            context_overflow_count: context_overflow.context_overflow_count,
            context_overflow_last_message: context_overflow.context_overflow_last_message.clone(),
            worker_session_token_peak,
            reasoning_trace_bytes,
            reasoning_trace_event_count,
            opencode_events_bytes,
            opencode_event_count,
            tool_events_bytes,
            tool_event_count,
            report_bytes,
            patch_bytes,
            worktree_patch_bytes,
            session_bytes,
            changed_file_count: stats.files.len(),
            changed_line_count: stats.changed_line_count,
            codex_token_usage: None,
            approximate_codex_input_bytes: None,
            approximate_codex_output_bytes: None,
            artifact_files_read_by_codex: compact_artifacts
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
            notes,
        };
        write_pretty_json(&out_dir.join(METRICS_JSON), &metrics, "run metrics")?;

        let receipt = Receipt {
            run_id,
            status: status.to_string(),
            mode: mode.to_string(),
            summary,
            changed_files: stats.files.clone(),
            report: display_path(root, &out_dir.join(REPORT_MD)),
            patch: display_path(root, &out_dir.join(CHANGES_PATCH)),
            worktree_patch: display_path(root, &out_dir.join(WORKTREE_PATCH)),
            session: display_path(root, &out_dir.join(SESSION_JSONL)),
            interventions: display_path(root, &interventions_path),
            metrics: display_path(root, &out_dir.join(METRICS_JSON)),
            logs: display_path(root, &logs_dir),
        };
        write_pretty_json(&out_dir.join(RECEIPT_JSON), &receipt, "run receipt")?;
        let compact_total = compact_artifacts
            .iter()
            .filter_map(|name| file_len(&out_dir.join(*name)).ok())
            .sum();
        metrics.approximate_codex_input_bytes = Some(compact_total);
        write_pretty_json(&out_dir.join(METRICS_JSON), &metrics, "run metrics")?;
        let compact_total_after_metrics_update = compact_artifacts
            .iter()
            .filter_map(|name| file_len(&out_dir.join(*name)).ok())
            .sum();
        if compact_total_after_metrics_update != compact_total {
            metrics.approximate_codex_input_bytes = Some(compact_total_after_metrics_update);
            write_pretty_json(&out_dir.join(METRICS_JSON), &metrics, "run metrics")?;
        }

        println!(
            "Mixmod run {} wrote artifacts to {}",
            receipt.run_id,
            out_dir.display()
        );
        println!("status: {}", receipt.status);
        println!("compact artifacts:");
        for &artifact in compact_artifacts {
            println!("  {}", display_path(root, &out_dir.join(artifact)));
        }
        Ok(receipt)
    }
}

fn expect_patch_for_run(mode: DelegationMode, task: &Value) -> bool {
    if mode != DelegationMode::Patch {
        return false;
    }
    get_bool(task, "expect_patch")
        .or_else(|| {
            task.get("context")
                .and_then(|context| get_bool(context, "expect_patch"))
        })
        .or_else(|| {
            task.get("context")
                .and_then(|context| context.get("worker_brief"))
                .and_then(|brief| get_bool(brief, "expect_patch"))
        })
        .unwrap_or(true)
}

fn intervention_details(
    items: impl IntoIterator<Item = (&'static str, Value)>,
) -> serde_json::Map<String, Value> {
    items
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}
