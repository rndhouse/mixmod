use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub(super) struct WorkerBriefTask<'a> {
    pub(super) title: String,
    pub(super) instructions: String,
    pub(super) expect_patch: bool,
    pub(super) files: Vec<String>,
    pub(super) tests: Vec<String>,
    pub(super) constraints: Vec<String>,
    pub(super) acceptance: Vec<String>,
    pub(super) context: WorkerBriefTaskContext<'a>,
}

#[derive(Serialize)]
pub(super) struct WorkerBriefTaskContext<'a> {
    pub(super) expect_patch: bool,
    pub(super) worker_brief: &'a Value,
}

#[derive(Serialize)]
pub(super) struct RevisionTask<'a> {
    pub(super) title: String,
    pub(super) instructions: String,
    pub(super) expect_patch: bool,
    pub(super) worker_mode: &'a str,
    pub(super) files: Vec<String>,
    pub(super) tests: Vec<String>,
    pub(super) constraints: Vec<String>,
    pub(super) acceptance: Vec<String>,
    pub(super) context: RevisionTaskContext<'a>,
}

#[derive(Serialize)]
pub(super) struct RevisionTaskContext<'a> {
    pub(super) expect_patch: bool,
    pub(super) codex_focus_files: &'a [String],
    pub(super) repo_focus_files: &'a [String],
    pub(super) patch_decision: &'a str,
    pub(super) revision: RevisionTaskDetails<'a>,
    pub(super) mixmod_artifact_refs: &'a [String],
}

#[derive(Serialize)]
pub(super) struct RevisionTaskDetails<'a> {
    pub(super) expect_patch: bool,
    pub(super) delta_expected: bool,
    pub(super) message_to_worker: &'a str,
    pub(super) worker_mode: &'a str,
    pub(super) patch_decision: &'a str,
    pub(super) worker_turn_shape: Option<&'a str>,
    pub(super) turn_goal: Option<&'a str>,
    pub(super) exact_edits: &'a [String],
    pub(super) edit_plan: &'a [String],
    pub(super) deferred_checks: &'a [String],
    pub(super) defer_checks_until_patch_exists: Option<bool>,
    pub(super) completion_gate: Option<&'a str>,
    pub(super) forbidden_actions: &'a [String],
    pub(super) focus_files: &'a [String],
    pub(super) repo_focus_files: &'a [String],
    pub(super) required_checks: &'a [String],
}
