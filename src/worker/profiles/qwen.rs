use crate::{DEFAULT_OPENCODE_LOCAL_MODEL, DEFAULT_OPENCODE_MODEL, DEFAULT_OPENCODE_PROVIDER};

use super::WorkerModelProfile;

pub(super) fn profile() -> WorkerModelProfile {
    WorkerModelProfile {
        model: DEFAULT_OPENCODE_MODEL.to_string(),
        aliases: vec![
            DEFAULT_OPENCODE_MODEL.to_string(),
            DEFAULT_OPENCODE_LOCAL_MODEL.to_string(),
            "qwen/qwen3.6-27b".to_string(),
            format!("{DEFAULT_OPENCODE_PROVIDER}/{DEFAULT_OPENCODE_LOCAL_MODEL}"),
        ],
        target_patch_lines: Some(100),
        max_patch_lines: Some(250),
        supervisor_guidance: vec![
            "This worker is cheap and useful for focused source edits, narrow repo inspection, compact command checks, and proposal turns; avoid broad autonomous design work.".to_string(),
            "It can spend a while reasoning before editing; do not assume it is stalled while OpenCode is still producing reasoning, tool, or stdout activity.".to_string(),
            "It can struggle with large effective context before explicit overflow; avoid asking it to reread many files, and use worker_mode=context_focus after context overflow, stale context, or repeated no-delta turns.".to_string(),
            "Treat the files list as a likely read queue for this worker: it often opens every listed path before editing, so do not list large or generated files unless full-file reading is intended and context-safe.".to_string(),
            "For broad expected-patch tasks, prefer worker_turn_shape=patch_request: choose the largest coherent source behavior it is likely to complete cleanly, usually one to three focused files, a bounded goal, deferred checks, and optional exact_edits or anchors/snippets only when precision saves supervisor output.".to_string(),
            "When route or file choice is unclear, use worker_turn_shape=planning_probe with expect_patch=false: ask it to inspect one to three focused authored-source files or targeted command outputs and propose the next request; do not let it decide task completion.".to_string(),
            "After multiple clean patch_request turns with useful deltas and no context pressure, broaden the next patch_request within the same shape; after messy, broad, or stalled turns, shrink the next slice.".to_string(),
            "Prefer human-authored source edits. Keep generated outputs, vendored files, lockfiles, snapshots, and build outputs out of the worker-owned patch until there is an intentional generated-output step.".to_string(),
            "For generated-code tasks, separate authored-source edits from generated-output updates when that reduces churn: request the human-authored source patch first, then use a later turn to run the generator or inspect generated output after a source diff exists.".to_string(),
            "If generated output must be produced, ask for the repo's generator or focused command, not manual full-file inspection or manual editing of generated files.".to_string(),
            "It may keep transient generator/debug/build sidecars or produce broad generated-output churn; ask it to leave only intentional tracked outputs and report unrelated churn rather than carrying it forward.".to_string(),
            "It may produce directionally useful but messy parser, grammar, generated-code, or broad integration patches; inspect changed-file lists and patch stats before opening large diffs.".to_string(),
            "It may miss end-to-end integration across slices. Before approval, check that helpers, options, parser/generated code, callers, state mutation, and error propagation are wired where the task requires.".to_string(),
            "Do not trust compile success, non-empty diff, or the worker's summary as proof of task completion; require task-derived behavior evidence plus likely negative or edge-case coverage when behavior changed.".to_string(),
            "For option or behavior families with a base path plus modifiers, start with the base behavior and add one modifier family later unless prior worker turns show it can safely combine more.".to_string(),
            "For large functions or code-generation paths, describe the smallest local transformation; include a literal anchor only when it prevents worker wandering without much supervisor output.".to_string(),
            "When tests cannot start because dependencies are missing, keep the worker focused on repo-level evidence and allowed commands instead of global environment repair.".to_string(),
        ],
    }
}
