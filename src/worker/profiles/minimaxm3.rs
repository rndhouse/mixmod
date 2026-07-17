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
            "MiniMax can handle moderately large implementation slices, but its cost and latency rise sharply when one OpenCode session accumulates many tool outputs. Even discounted cache reads become expensive when each later tool step replays a 70k-100k context.".to_string(),
            "Prefer short, phase-bounded MiniMax turns that create one useful tracked diff and then stop for supervisor review. This lets the supervisor validate direction before MiniMax spends many cheap-but-not-free steps debugging, regenerating, or reading broadly.".to_string(),
            "For the first expected-patch MiniMax turn, normally set worker_turn_shape=patch_request and request one bounded implementation phase rather than the full task. Include a worker-visible stop_condition; if asking for full-task or multi-phase scope, include a compact scope_rationale.".to_string(),
            "A first implementation phase should be a real boundary, such as parser/API source support, runtime enforcement, or focused tests/verification. Do not combine independent phases unless scope_rationale explains why that saves supervisor tokens without creating a long worker session.".to_string(),
            "Use the larger context as safety margin, not as a reason for unbounded files, generated output, broad logs, or long autonomous debugging loops.".to_string(),
            "Prefer human-authored source edits, keep generated outputs out of the normal worker-owned patch, and review changed-file lists before opening large diffs.".to_string(),
            "After a successful bounded patch_request, the supervisor may assign a somewhat larger coherent implementation slice when prior worker evidence shows MiniMax handled the source area cleanly. Keep files, goal, checks, and stopping point bounded.".to_string(),
            "For broad expected-patch tasks, prefer worker_turn_shape=patch_request with one coherent implementation slice, usually focused authored-source files, deferred checks, and optional exact_edits or anchors only when precision saves supervisor output.".to_string(),
            "When route or file choice is unclear, use a planning_probe that asks for candidate files, anchors, expected patch size, and risks; do not let the worker decide task completion.".to_string(),
            "It may still over-read, over-generalize, or produce plausible but incomplete integration patches; before approval, check end-to-end task behavior and likely edge cases against the accumulated diff.".to_string(),
            "It may eagerly regenerate files or keep generator sidecars after a source edit; ask it to leave only intentional tracked outputs and summarize the generator command/result when generated output changes.".to_string(),
            "If the worker starts broad repo reading, generated-file inspection, or test-before-edit behavior without a useful diff, issue worker_edit with a tighter patch_request rather than expanding context further.".to_string(),
            "Prefer worker_mode=context_focus for the next MiniMax turn after a broad investigation phase, large tool-call burst, generated-output cleanup, context overflow, or high worker_session_token_peak. Restate the current patch state and the next focused goal rather than continuing a long worker session.".to_string(),
        ],
        enable_auto_followups: false,
        enable_worker_self_review: false,
        enable_forced_context_focus: false,
        worker_timeout_seconds: Some(0),
        idle_timeout_seconds: None,
        opencode_output_token_limit: Some(4_096),
    }
}
