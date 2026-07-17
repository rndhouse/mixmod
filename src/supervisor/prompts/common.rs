use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::*;

pub(super) const DEBUG_PROFILE_FIT_ENV: &str = "MIXMOD_DEBUG_PROFILE_FIT";

pub(super) fn supervisor_worktree_policy() -> &'static str {
    "Workspace access is for supervision, not implementation. You may use git/worktree commands such as `git status`, `git diff`, `git show`, `git grep`, and checkpoint-oriented `git restore` or `git apply` when needed to inspect or manage state. Do not author task-solving source edits, rewrite code, or create the solution patch yourself; the worker owns implementation."
}

pub(super) fn supervisor_implementation_slice_policy() -> &'static str {
    r#"Implementation slice policy:
- Size worker requests by implementation surface, not only by end-user behavior. One user-visible behavior can still be too broad for one worker turn.
- Treat parser/AST, runtime or environment state, executor/VM, API surface, tests or verification, generated outputs, and migration-style updates as separate layers. A request crossing multiple layers is broad unless prior worker evidence and the selected profile's patch-size guidance support combining them.
- Shrink before handoff when the file list is large, files span subsystems, generated or large files are included, source edits are combined with tests/checks, the route requires broad repo inspection, or anchors are unclear.
- When generic task coherence conflicts with selected worker profile guidance, obey the profile by shrinking files, layers, checks, or anchors until the request fits."#
}

pub(super) fn supervisor_worker_shape_contract(
    worker_guidance: &WorkerSupervisorGuidance,
) -> &'static str {
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
        return r#"Patch-request decomposition contract: for expected-patch implementation handoffs, use worker_turn_shape="patch_request". Choose one bounded, reviewable implementation slice expected to fit the worker patch-size guidance; do not treat one end-to-end behavior as one slice when it crosses implementation layers. When the overall task spans multiple independent behaviors, parser/AST, runtime or environment state, executor/VM, API surface, generated outputs, verification steps, or likely exceeds the worker patch budget, decompose it yourself before emitting JSON: hand off the next slice only, not the full task. Use concrete file paths for that slice, normally a small authored-source set; include generated or large files only when they are part of that slice and name the command or boundary. Include a worker-visible stop_condition that tells the worker to stop after the slice has one useful tracked diff and return for supervisor review. Do not use broad or full-task patch_request scope unless the selected worker profile explicitly supports that scope; when it does, include scope_rationale explaining why the request remains within the profile's patch-size guidance, known file boundary, and acceptable worker-session risk. Do not emit worker_turn_shape="bounded_feature_slice" or "default" for expected-patch work. Use planning_probe with expect_patch=false only when bounded worker investigation is cheaper than supervisor investigation."#;
    }
    "No worker-specific default shape is selected. Choose one shape deliberately: planning_probe for no-patch investigation, patch_request for a focused edit, bounded_feature_slice for a coherent larger feature chunk, or default only when the task is already simple."
}

pub(super) fn supervisor_artifact_index(work_dir: &Path, artifact_paths: &[PathBuf]) -> String {
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
        RECEIPT_JSON => "worker-turn status and artifact locations",
        REPORT_MD => "compact worker-turn summary",
        REVIEW_SIGNALS_JSON => "compact review routing signals and conditional artifact paths",
        REASONING_TRACE_JSONL => "worker reasoning events extracted from structured output",
        TOOL_EVENTS_JSONL => "worker tool-call events extracted from structured output",
        WORKTREE_PATCH => "active current repository diff",
        CHANGES_PATCH => "latest worker-turn diff",
        INTERVENTIONS_JSONL => "Mixmod intervention audit log",
        METRICS_JSON => "worker-turn metrics and signals",
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

pub(super) fn render_worker_guidance(worker_guidance: &WorkerSupervisorGuidance) -> String {
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
        rendered.push_str(" changed lines. This is supervisor planning guidance only, not a Mixmod gate; choose an implementation slice expected to fit it and intentionally exceed it only when the selected profile and prior worker evidence justify the larger turn.\n");
    }
    for item in &worker_guidance.guidance {
        rendered.push_str("- ");
        rendered.push_str(item.trim());
        rendered.push('\n');
    }
    rendered
}
