use crate::DEFAULT_OPENCODE_PROVIDER;

use super::WorkerModelProfile;

pub(super) fn local_flash_profile() -> WorkerModelProfile {
    WorkerModelProfile {
        model: "glm-4.7-flash:Q4_K_M".to_string(),
        aliases: vec![
            "glm-4.7-flash:Q4_K_M".to_string(),
            format!("{DEFAULT_OPENCODE_PROVIDER}/glm-4.7-flash:Q4_K_M"),
        ],
        target_patch_lines: Some(180),
        max_patch_lines: Some(400),
        supervisor_guidance: vec![
            "It tends to act readily, but can rewrite or delete too much when asked for broad source changes.".to_string(),
            "For broad expected-patch tasks, prefer worker_turn_shape=patch_request with one source behavior, one focused file, a bounded goal, optional exact_edits when precision is needed, and no tests before editing.".to_string(),
            "For large functions or code-generation paths, include preservation constraints: preserve existing control flow and indentation, do not rewrite the whole function, do not delete or reindent unrelated branches, and edit only the focused block.".to_string(),
            "Prefer worker_mode=continue for revisions so it keeps accumulated file context; use context_focus only when the previous worker context is clearly harmful.".to_string(),
            "Before accepting a turn, check whether the diff is too broad or destructive for the requested patch.".to_string(),
        ],
    }
}

pub(super) fn openrouter_glm_5_2_profile() -> WorkerModelProfile {
    WorkerModelProfile {
        model: "openrouter/z-ai/glm-5.2".to_string(),
        aliases: vec![
            "openrouter/z-ai/glm-5.2".to_string(),
            "z-ai/glm-5.2".to_string(),
        ],
        target_patch_lines: Some(300),
        max_patch_lines: Some(800),
        supervisor_guidance: vec![
            "This worker is capable, but may over-investigate when the handoff contains an apparent implementation constraint conflict or an unresolved toolchain choice.".to_string(),
            "For generated-code, parser/compiler, toolchain, or similar trap-prone tasks, resolve the implementation route in the supervisor handoff before invoking the worker; do not ask the worker to discover whether the obvious route is viable.".to_string(),
            "When the supervisor has selected a route, tell the worker to trust that route unless a direct compile, test, or command result proves it impossible.".to_string(),
            "For broad expected-patch tasks, prefer worker_turn_shape=bounded_feature_slice with one concrete implementation path, one to three focused files, and the first reversible source edit named explicitly.".to_string(),
            "Make the initial handoff patch-first: include the chosen strategy, the exact next behavior slice, the files to touch, and deferred checks; avoid leaving design forks for the worker to resolve before editing.".to_string(),
            "If the worker starts toolchain archaeology, scratch-file probing, broad repo reading, or test-before-edit behavior without a diff, use live control to restate the chosen implementation route and request an immediate focused source edit.".to_string(),
            "For revisions, anchor the next instruction to the current accumulated patch, preserve useful existing edits, and name the next missing behavior instead of restarting discovery.".to_string(),
            "Before approval, check that the accumulated patch implements the requested end-to-end behavior, not just the first structural field or helper, and require focused behavior evidence for the main path plus likely invalid or edge case.".to_string(),
        ],
    }
}
