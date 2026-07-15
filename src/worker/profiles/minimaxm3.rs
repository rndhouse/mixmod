use super::WorkerModelProfile;

pub(super) fn profile() -> WorkerModelProfile {
    WorkerModelProfile {
        model: "openrouter/minimax/minimax-m3".to_string(),
        aliases: vec![
            "openrouter/minimax/minimax-m3".to_string(),
            "minimax/minimax-m3".to_string(),
            "minimax-m3".to_string(),
        ],
        target_patch_lines: Some(180),
        max_patch_lines: Some(450),
        supervisor_guidance: vec![
            "This worker has a much larger advertised context window than local Qwen, but do not treat that as permission to hand it unbounded files, generated output, or broad logs.".to_string(),
            "MiniMax cache reads are relatively cheap, but long OpenCode sessions can still dominate worker cost when a large context is replayed across many tool calls. Do not keep one worker session alive just because cached input is discounted.".to_string(),
            "Start from the same caution used for weaker local workers: prefer human-authored source edits, keep generated outputs out of the normal worker-owned patch, and review changed-file lists before opening large diffs.".to_string(),
            "Use the larger context as safety margin: the supervisor may assign a somewhat larger coherent patch_request when that is likely to reduce GPT supervisor turns, but should still bound files, goal, and checks.".to_string(),
            "For broad expected-patch tasks, prefer worker_turn_shape=patch_request with one coherent source behavior, usually focused authored-source files, deferred checks, and optional exact_edits or anchors only when precision saves supervisor output.".to_string(),
            "When route or file choice is unclear, use a planning_probe that asks for candidate files, anchors, expected patch size, and risks; do not let the worker decide task completion.".to_string(),
            "It may still over-read, over-generalize, or produce plausible but incomplete integration patches; before approval, check end-to-end task behavior and likely edge cases against the accumulated diff.".to_string(),
            "It may eagerly regenerate files or keep generator sidecars after a source edit; ask it to leave only intentional tracked outputs and summarize the generator command/result when generated output changes.".to_string(),
            "If the worker starts broad repo reading, generated-file inspection, or test-before-edit behavior without a useful diff, revise with a tighter patch_request rather than expanding context further.".to_string(),
            "Prefer worker_mode=context_focus for the next MiniMax turn after a broad investigation phase, large tool-call burst, generated-output cleanup, context overflow, or high worker_session_token_peak. Restate the current patch state and the next focused goal rather than continuing a long worker session.".to_string(),
        ],
        enable_auto_followups: false,
        enable_worker_self_review: false,
        enable_forced_context_focus: false,
        worker_timeout_seconds: Some(0),
    }
}
