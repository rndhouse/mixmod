use crate::DefaultStrategyMode;

/// Capability for supervisor-directed surgical takeover patches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TakeoverPatchCapability {
    /// The supervisor may review and plan, but must not author solution edits.
    Disabled,
    /// The supervisor may take over by sending a bounded patch to a GPT worker.
    SurgicalPatch,
}

impl TakeoverPatchCapability {
    /// Return whether supervisor review may emit action=take_over.
    pub(crate) fn allows_takeover(self) -> bool {
        matches!(self, Self::SurgicalPatch)
    }
}

/// Prompt and behavior contract for one default strategy mode.
pub(crate) struct DefaultStrategyPolicy {
    /// Stable strategy label for metrics and prompts.
    pub(crate) id: &'static str,
    /// Supervisor-directed takeover capability for this strategy.
    pub(crate) takeover_patch: TakeoverPatchCapability,
    /// Instruction supplied to the supervisor review turn.
    pub(crate) review_instruction: &'static str,
    /// Strategy policy block embedded in supervisor review prompts.
    pub(crate) feedback_policy: &'static str,
    /// Note stored in metrics/report artifacts.
    pub(crate) metrics_note: &'static str,
    /// Whether debug mode should require a delegation decision explanation.
    pub(crate) debug_delegation_decision: bool,
}

impl DefaultStrategyPolicy {
    /// Build the JSON schema shown in supervisor review prompts.
    pub(crate) fn feedback_action_schema(&self, debug_json_field: &'static str) -> String {
        if self.takeover_patch.allows_takeover() {
            format!(
                r#"{{"action":"approve|revise|take_over|stop","expect_patch":true,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","context_recommendation":{{"action":"continue|compact_now|compact_after_next_worker","reason":"max 20 words"}},"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"],"takeover_reason":"required for take_over; max 40 words","direct_plan":["required for take_over; exact surgical patch target"]{debug_json_field}}}"#
            )
        } else {
            format!(
                r#"{{"action":"approve|revise|stop","expect_patch":true,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","context_recommendation":{{"action":"continue|compact_now|compact_after_next_worker","reason":"max 20 words"}},"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"]{debug_json_field}}}"#
            )
        }
    }
}

const SUPERVISED_WORKER_POLICY: DefaultStrategyPolicy = DefaultStrategyPolicy {
    id: "supervised-worker",
    takeover_patch: TakeoverPatchCapability::Disabled,
    review_instruction: "Decide the next worker-loop action. Use approve only when the worker result is acceptable. Prefer revise after failed or empty worker attempts, with a concrete next instruction. Use stop only to record a blocked or inconclusive worker result when no useful worker path remains; do not author task-solving source changes.",
    feedback_policy: r#"Strategy mode: supervised-worker.
- The supervisor remains in review/planning mode. Do not use action=take_over.
- Delegate further implementation, repair, or verification work to the worker when the result is not ready to approve."#,
    metrics_note: "The supervisor controls the worker loop with approve, revise, or blocked/inconclusive stop decisions; supervisor-authored patching is not part of this strategy.",
    debug_delegation_decision: false,
};

const WORKER_BUILD_SUPERVISOR_FIX_POLICY: DefaultStrategyPolicy = DefaultStrategyPolicy {
    id: "worker-build-supervisor-fix",
    takeover_patch: TakeoverPatchCapability::SurgicalPatch,
    review_instruction: "Decide the next worker-build-supervisor-fix action. Use approve only when the current source is acceptable. Use revise when the next work is worker-scale: broad construction, uncertain investigation, multi-file exploration, generated-output synchronization, or checks likely to drive additional implementation. Use take_over only when the remaining edit is surgical: the defect, target files, and intended change are already known from current context and can be handed to a fresh GPT worker patch session. Use stop only when no useful worker or takeover patch path remains. Do not author task-solving source changes during this review turn.",
    feedback_policy: r#"Strategy mode: worker-build-supervisor-fix.
- Use the worker for construction: broad implementation slices, new subsystem wiring, generated-output synchronization, or meaningful test clusters that still require exploration plus editing.
- Use action=take_over only for surgical correction once a usable baseline exists: named residual defects, edge cases inside already-attempted work, error wording, propagation, shadowing, nil/zero-value behavior, formatting, or another small repair where the target files and intended edit are already known. Mixmod will hand direct_plan to a fresh GPT worker patch session; the supervisor review turn does not edit source.
- Before action=revise, classify the next request. Choose revise when the next step needs broad search, large context reads, test exploration, generated-file inspection, multi-subsystem reasoning, or a check likely to uncover implementation work.
- Before action=take_over, confirm the direct_plan can be executed without broad exploration. If the plan would require broad shell/test commands, reading large or generated files, or discovering where the bug lives, keep the work with the normal worker.
- Corrections can appear before every broad task area is complete. If the current next step is surgical correction, take over now; later broad construction should happen only if the takeover worker patch establishes that another worker-scale slice remains.
- For take_over, include takeover_reason and direct_plan. direct_plan must name the exact residual defect and target files when known. Put broad or command-based checks in a later worker revise/verification turn."#,
    metrics_note: "In worker-build-supervisor-fix mode, the supervisor may choose take_over only for surgical corrective work; Mixmod executes that direct_plan in a fresh GPT worker patch session. Broad or uncertain follow-up work remains delegated to the normal worker.",
    debug_delegation_decision: true,
};

/// Return the complete policy contract for a default strategy.
pub(crate) fn default_strategy_policy(
    strategy: DefaultStrategyMode,
) -> &'static DefaultStrategyPolicy {
    match strategy {
        DefaultStrategyMode::SupervisedWorker => &SUPERVISED_WORKER_POLICY,
        DefaultStrategyMode::WorkerBuildSupervisorFix => &WORKER_BUILD_SUPERVISOR_FIX_POLICY,
    }
}

/// Return the per-strategy instruction for a supervisor review turn.
pub(crate) fn default_strategy_review_instruction(strategy: DefaultStrategyMode) -> &'static str {
    default_strategy_policy(strategy).review_instruction
}

/// Return the strategy-specific policy block for supervisor review prompts.
pub(crate) fn supervisor_feedback_strategy_policy(strategy: DefaultStrategyMode) -> &'static str {
    default_strategy_policy(strategy).feedback_policy
}

/// Build the JSON schema shown in supervisor review prompts.
pub(crate) fn supervisor_feedback_action_schema(
    strategy: DefaultStrategyMode,
    debug_json_field: &'static str,
) -> String {
    default_strategy_policy(strategy).feedback_action_schema(debug_json_field)
}

/// Return the strategy note stored in metrics/report artifacts.
pub(crate) fn default_strategy_note(strategy: DefaultStrategyMode) -> &'static str {
    default_strategy_policy(strategy).metrics_note
}
