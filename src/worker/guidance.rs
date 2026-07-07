use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_OPENCODE_LOCAL_MODEL, DEFAULT_OPENCODE_MODEL, DEFAULT_OPENCODE_PROVIDER, OpenCodeConfig,
};

/// Historical pitfalls for one worker model.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WorkerModelProfile {
    /// Canonical worker model label.
    pub model: String,
    /// Additional model/provider labels that should select this profile.
    pub aliases: Vec<String>,
    /// Supervisor-only guidance for adapting worker instructions.
    pub supervisor_guidance: Vec<String>,
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
    pub(crate) guidance: Vec<String>,
}

impl WorkerSupervisorGuidance {
    pub(crate) fn is_empty(&self) -> bool {
        self.guidance.is_empty()
    }
}

pub(crate) fn default_worker_model_profiles() -> Vec<WorkerModelProfile> {
    vec![
        WorkerModelProfile {
            model: DEFAULT_OPENCODE_MODEL.to_string(),
            aliases: vec![
                DEFAULT_OPENCODE_MODEL.to_string(),
                DEFAULT_OPENCODE_LOCAL_MODEL.to_string(),
                "qwen/qwen3.6-27b".to_string(),
                format!("{DEFAULT_OPENCODE_PROVIDER}/{DEFAULT_OPENCODE_LOCAL_MODEL}"),
            ],
            supervisor_guidance: vec![
                "This worker can spend a while reasoning before editing; do not assume it is stalled while OpenCode is still producing reasoning, tool, or stdout activity.".to_string(),
                "This worker can struggle with large effective context before an explicit overflow occurs; keep initial handoffs compact, split broad tasks into smaller coherent slices, and avoid asking it to reread many files at once.".to_string(),
                "For broad expected-patch tasks, prefer worker_turn_shape=bounded_feature_slice: one coherent feature chunk, usually one to three source files, related serialization/deserialization or API/test edits together, and a compile or focused test check after the patch exists.".to_string(),
                "For revisions, prefer worker_mode=continue only while the worker context remains useful. If artifacts show context overflow, repeated summary updates, or no new delta after a focused revision, prefer worker_mode=context_focus with a smaller concrete source slice.".to_string(),
                "When tests fail to start because dependencies are missing, keep it focused on repo-level evidence and allowed commands instead of global environment repair.".to_string(),
                "It can create broad or malformed tests when fixture semantics are unclear; ask for the narrowest regression test that matches existing test style.".to_string(),
                "It may try to mutate user or global environments while installing dependencies; prefer existing project commands and avoid global installs unless the task explicitly requires them.".to_string(),
                "Before accepting a turn, check whether the intended repo diff exists and touches the expected source/test files.".to_string(),
            ],
        },
        WorkerModelProfile {
            model: "glm-4.7-flash:Q4_K_M".to_string(),
            aliases: vec![
                "glm-4.7-flash:Q4_K_M".to_string(),
                format!("{DEFAULT_OPENCODE_PROVIDER}/glm-4.7-flash:Q4_K_M"),
            ],
            supervisor_guidance: vec![
                "It tends to act readily, but can rewrite or delete too much when asked for broad source changes.".to_string(),
                "For broad expected-patch tasks, prefer worker_turn_shape=small_patch_slice with one source behavior, one focused file, one exact edit, and no tests before editing.".to_string(),
                "For large functions or code-generation paths, include preservation constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete or reindent unrelated branches, and edit only the focused block.".to_string(),
                "Prefer worker_mode=continue for revisions so it keeps accumulated file context; use context_focus only when the previous worker context is clearly harmful.".to_string(),
                "Before accepting a turn, check whether the diff is too broad or destructive for the requested slice.".to_string(),
            ],
        },
    ]
}

fn normalize_model_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
