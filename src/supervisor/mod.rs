mod codex;
mod live;
mod normalize;
mod prompts;
#[cfg(test)]
mod tests;
mod turns;
mod types;

pub(crate) use codex::SupervisorCodexSession;
pub(crate) use live::LiveSupervisorAdvisor;
#[cfg(test)]
pub(crate) use normalize::normalize_feedback_value;
pub(crate) use normalize::normalize_worker_mode;
#[cfg(test)]
pub(crate) use prompts::{
    supervisor_feedback_prompt, supervisor_feedback_prompt_with_debug_profile_fit,
    supervisor_live_control_prompt, supervisor_patch_prompt, supervisor_worker_brief_prompt,
    supervisor_worker_brief_prompt_with_debug_profile_fit,
};
pub(crate) use turns::{
    run_supervisor_brief_turn, run_supervisor_compaction, run_supervisor_feedback_turn,
    run_supervisor_patch_turn,
};
pub(crate) use types::{
    PatchDecision, RevisionHandoff, SupervisorBriefTurn, SupervisorCompactionTurn,
    SupervisorContextTelemetry, SupervisorFeedbackTurn, SupervisorPatchTurn, SupervisorUsageSample,
    SupervisorVerdict, WorkerMode, aggregate_supervisor_usage,
};
