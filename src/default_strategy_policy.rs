use crate::DefaultStrategyMode;

/// Return the per-strategy instruction for a supervisor review turn.
pub(crate) fn default_strategy_review_instruction(strategy: DefaultStrategyMode) -> &'static str {
    match strategy {
        DefaultStrategyMode::SupervisedWorker => {
            "Decide the next worker-loop action. Use approve only when the worker result is acceptable. Prefer revise after failed or empty worker attempts, with a concrete next instruction. Use stop only to record a blocked or inconclusive worker result when no useful worker path remains; do not author task-solving source changes."
        }
        DefaultStrategyMode::WorkerBootstrap => {
            "Decide the next worker-bootstrap action. Use approve only when the current source is acceptable. Use revise when the next work is still a substantial separable worker implementation slice. Use take_over when the current patch is a useful baseline and the remaining work is localized edge cases, focused tests, formatting, or debugging that you already understand well enough to finish directly. Use stop only when no useful worker or direct-supervisor path remains. Do not author task-solving source changes during this review turn."
        }
        DefaultStrategyMode::WorkerBuildSupervisorFix => {
            "Decide the next worker-build-supervisor-fix action. Use approve only when the current source is acceptable. Use revise only when the next work is broad worker-scale construction. Use take_over when the next work is corrective: named residual defects, edge cases, error wording, propagation, shadowing, formatting, targeted verification, or other small repairs after a usable baseline exists. Use stop only when no useful worker or direct-supervisor path remains. Do not author task-solving source changes during this review turn."
        }
    }
}

/// Return the strategy-specific policy block for supervisor review prompts.
pub(crate) fn supervisor_feedback_strategy_policy(strategy: DefaultStrategyMode) -> &'static str {
    match strategy {
        DefaultStrategyMode::SupervisedWorker => {
            r#"Strategy mode: supervised-worker.
- The supervisor remains in review/planning mode. Do not use action=take_over.
- Delegate further implementation, repair, or verification work to the worker when the result is not ready to approve."#
        }
        DefaultStrategyMode::WorkerBootstrap => {
            r#"Strategy mode: worker-bootstrap.
- Use the worker as much as possible while the next request is a substantial separable implementation slice with a clear file boundary.
- Choose action=take_over only when the current patch is a useful baseline and the remaining work is localized edge cases, focused tests, formatting, or debugging that you already understand well enough to finish directly.
- Takeover signals include: you would otherwise ask for small tweaks, semantic edge-case repairs, focused test additions, or verification-driven fixes; recent worker deltas are small; another worker message would mostly restate details already clear from artifacts.
- Do not take over merely because progress exists, because the worker made one mistake, or because broad implementation work remains separable into a useful worker slice.
- For take_over, include takeover_reason and direct_plan. Keep direct_plan focused on the exact tests/repairs you will perform after Mixmod compacts the supervisor context."#
        }
        DefaultStrategyMode::WorkerBuildSupervisorFix => {
            r#"Strategy mode: worker-build-supervisor-fix.
- Use the worker for construction: broad implementation slices, new subsystem wiring, generated-output synchronization, or meaningful test clusters that still require exploration plus editing.
- Use action=take_over for correction once a usable baseline exists: named residual defects, edge cases inside already-attempted work, error wording, propagation, shadowing, nil/zero-value behavior, formatting, targeted hidden-test-style fixes, or final verification.
- Before action=revise, classify the next request. If the worker message would list specific defects to repair, it is correction and should normally be take_over. Choose revise only when you can name a remaining broad construction slice, not merely another tight repair.
- Corrections can appear before every broad task area is complete. If the current next step is correction, take over now; later broad construction should happen only if direct supervisor work establishes that another worker-scale slice remains.
- For take_over, include takeover_reason and direct_plan. direct_plan must cover the exact residual defects and targeted checks you will use before approval."#
        }
    }
}

/// Build the JSON schema shown in supervisor review prompts.
pub(crate) fn supervisor_feedback_action_schema(
    strategy: DefaultStrategyMode,
    debug_json_field: &'static str,
) -> String {
    match strategy {
        DefaultStrategyMode::SupervisedWorker => {
            format!(
                r#"{{"action":"approve|revise|stop","expect_patch":true,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","context_recommendation":{{"action":"continue|compact_now|compact_after_next_worker","reason":"max 20 words"}},"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"]{debug_json_field}}}"#
            )
        }
        DefaultStrategyMode::WorkerBootstrap | DefaultStrategyMode::WorkerBuildSupervisorFix => {
            format!(
                r#"{{"action":"approve|revise|take_over|stop","expect_patch":true,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","context_recommendation":{{"action":"continue|compact_now|compact_after_next_worker","reason":"max 20 words"}},"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"],"takeover_reason":"required for take_over; max 40 words","direct_plan":["required for take_over; focused direct supervisor edits/checks"]{debug_json_field}}}"#
            )
        }
    }
}

/// Return the direct-finish policy text for a strategy.
pub(crate) fn supervisor_direct_finish_policy(strategy: DefaultStrategyMode) -> &'static str {
    match strategy {
        DefaultStrategyMode::WorkerBuildSupervisorFix => {
            "Use the worker's current patch as the baseline. Preserve useful worker work. Finish the corrective work identified at takeover. Re-check each named residual defect yourself before approving; do not treat worker-reported checks as enough when your prior review named semantic risks. Do not rewrite broad subsystems unless the current patch is clearly unusable."
        }
        DefaultStrategyMode::WorkerBootstrap | DefaultStrategyMode::SupervisedWorker => {
            "Use the worker's current patch as the baseline. Preserve useful worker work. Finish only the localized remaining work you identified at takeover: edge cases, focused tests, formatting, or small semantic repairs. Do not rewrite broad subsystems unless the current patch is clearly unusable."
        }
    }
}

/// Return the strategy note stored in metrics/report artifacts.
pub(crate) fn default_strategy_note(strategy: DefaultStrategyMode) -> &'static str {
    if strategy.allows_supervisor_takeover() {
        match strategy {
            DefaultStrategyMode::WorkerBuildSupervisorFix => {
                "In worker-build-supervisor-fix mode, the supervisor may choose take_over when the next step is correction rather than broad worker-scale construction."
            }
            _ => {
                "In worker-bootstrap mode, the supervisor may choose take_over when the worker has produced a useful baseline and the remaining work is localized direct-finish work."
            }
        }
    } else {
        "The supervisor controls the worker loop with approve, revise, or blocked/inconclusive stop decisions; direct supervisor editing is not part of this strategy."
    }
}
