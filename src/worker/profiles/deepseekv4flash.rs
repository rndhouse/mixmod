use super::WorkerModelProfile;

pub(super) fn profile() -> WorkerModelProfile {
    WorkerModelProfile {
        model: "openrouter/deepseek/deepseek-v4-flash".to_string(),
        aliases: vec![
            "openrouter/deepseek/deepseek-v4-flash".to_string(),
            "deepseek/deepseek-v4-flash".to_string(),
            "deepseek-v4-flash".to_string(),
        ],
        target_patch_lines: Some(220),
        max_patch_lines: Some(550),
        supervisor_guidance: vec![
            "DeepSeek V4 Flash is a lower-cost OpenRouter worker that should be useful for direct, bounded source edits and compact repo investigation, but it is still not a completion judge.".to_string(),
            "Prefer phase-bounded patch_request turns that create one useful tracked diff and stop for supervisor review. The supervisor should validate direction before asking for another phase.".to_string(),
            "For the first expected-patch turn, normally set worker_turn_shape=patch_request and request one bounded implementation phase rather than the full task. Include a worker-visible stop_condition; if asking for full-task or multi-phase scope, include a compact scope_rationale.".to_string(),
            "This worker is expected to spend less output on reasoning than heavier workers, so favor concise patch-first requests over planning probes when the likely files and behavior are already known.".to_string(),
            "Use the large context as safety margin, not as a reason for unbounded files, generated output, broad logs, or long autonomous debugging loops.".to_string(),
            "Prefer human-authored source edits. Keep generated outputs out of the normal worker-owned patch, and review changed-file lists before opening large diffs.".to_string(),
            "The supervisor may assign a moderately larger coherent patch_request when that is likely to reduce GPT supervisor turns, but should still bound files, goal, checks, and the expected stopping point.".to_string(),
            "When route or file choice is unclear, use a planning_probe that asks for candidate files, anchors, expected patch size, and risks; do not let the worker decide task completion.".to_string(),
            "It may over-read, over-generalize, or produce plausible but incomplete integration patches; before approval, check end-to-end task behavior and likely edge cases against the accumulated diff.".to_string(),
            "If the worker starts broad repo reading, generated-file inspection, or test-before-edit behavior without a useful diff, revise with a tighter patch_request rather than expanding context further.".to_string(),
            "Prefer worker_mode=context_focus after a broad investigation phase, large tool-call burst, generated-output cleanup, context overflow, or high worker_session_token_peak. Restate the current patch state and the next focused goal rather than continuing a long worker session.".to_string(),
        ],
        enable_auto_followups: false,
        enable_worker_self_review: false,
        enable_forced_context_focus: false,
        worker_timeout_seconds: Some(0),
        opencode_output_token_limit: Some(4_096),
    }
}
