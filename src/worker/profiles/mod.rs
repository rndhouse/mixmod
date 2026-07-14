use std::collections::BTreeSet;

use crate::OpenCodeConfig;

mod glm;
mod minimaxm3;
mod qwen;

/// Historical pitfalls for one worker model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerModelProfile {
    /// Canonical worker model label.
    pub(crate) model: String,
    /// Additional model/provider labels that should select this profile.
    pub(crate) aliases: Vec<String>,
    /// Expected changed-line target for one worker turn.
    pub(crate) target_patch_lines: Option<u64>,
    /// Expected changed-line ceiling for one worker turn.
    pub(crate) max_patch_lines: Option<u64>,
    /// Supervisor-only guidance for adapting worker instructions.
    pub(crate) supervisor_guidance: Vec<String>,
    /// Enable Mixmod-owned retry turns after empty worker patches.
    pub(crate) enable_auto_followups: bool,
    /// Enable Mixmod-owned same-session worker self-review cleanup.
    pub(crate) enable_worker_self_review: bool,
    /// Enable Mixmod forcing fresh worker context after observed overflow.
    pub(crate) enable_forced_context_focus: bool,
}

impl WorkerModelProfile {
    pub(crate) fn matches_opencode_worker(&self, config: &OpenCodeConfig) -> bool {
        let profile_names = self
            .identifiers()
            .into_iter()
            .map(normalize_model_identifier)
            .collect::<BTreeSet<_>>();
        config
            .selected_model_identifiers()
            .into_iter()
            .map(|identifier| normalize_model_identifier(&identifier))
            .any(|identifier| profile_names.contains(&identifier))
    }

    fn identifiers(&self) -> Vec<&str> {
        let mut identifiers = vec![self.model.as_str()];
        identifiers.extend(self.aliases.iter().map(String::as_str));
        identifiers
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerSupervisorGuidance {
    pub(crate) model: String,
    pub(crate) target_patch_lines: Option<u64>,
    pub(crate) max_patch_lines: Option<u64>,
    pub(crate) guidance: Vec<String>,
    pub(crate) enable_auto_followups: bool,
    pub(crate) enable_worker_self_review: bool,
    pub(crate) enable_forced_context_focus: bool,
}

impl WorkerSupervisorGuidance {
    pub(crate) fn is_empty(&self) -> bool {
        self.guidance.is_empty()
            && self.target_patch_lines.is_none()
            && self.max_patch_lines.is_none()
            && !self.enable_auto_followups
            && !self.enable_worker_self_review
            && !self.enable_forced_context_focus
    }

    pub(crate) fn with_patch_line_overrides(
        mut self,
        target_patch_lines: Option<u64>,
        max_patch_lines: Option<u64>,
    ) -> Self {
        if target_patch_lines.is_some() {
            self.target_patch_lines = target_patch_lines;
        }
        if max_patch_lines.is_some() {
            self.max_patch_lines = max_patch_lines;
        }
        self
    }

    /// Return whether Mixmod may run automatic no-delta worker follow-ups.
    pub(crate) fn auto_followups_enabled(&self) -> bool {
        self.enable_auto_followups
    }

    /// Return whether Mixmod may run same-session worker self-review.
    pub(crate) fn worker_self_review_enabled(&self) -> bool {
        self.enable_worker_self_review
    }

    /// Return whether Mixmod may force fresh worker context after overflow.
    pub(crate) fn forced_context_focus_enabled(&self) -> bool {
        self.enable_forced_context_focus
    }
}

pub(crate) fn default_worker_model_profiles() -> Vec<WorkerModelProfile> {
    vec![
        qwen::profile(),
        glm::local_flash_profile(),
        glm::openrouter_glm_5_2_profile(),
        minimaxm3::profile(),
    ]
}

fn normalize_model_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
