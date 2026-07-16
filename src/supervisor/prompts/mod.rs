mod brief;
mod common;
mod live;
mod patch;
mod review;

pub(crate) use brief::supervisor_worker_brief_prompt;
#[cfg(test)]
pub(crate) use brief::supervisor_worker_brief_prompt_with_debug_profile_fit;
pub(crate) use live::supervisor_live_control_prompt;
pub(crate) use patch::supervisor_patch_prompt;
#[cfg(test)]
pub(crate) use review::supervisor_feedback_prompt_with_debug_profile_fit;
pub(crate) use review::{
    supervisor_feedback_approval_consistency_repair_prompt, supervisor_feedback_prompt,
};
