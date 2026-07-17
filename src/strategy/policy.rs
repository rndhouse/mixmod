use crate::DefaultStrategyMode;

/// Capability for supervisor-directed surgical direct edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupervisorDirectEditCapability {
    /// The supervisor may review and plan, but must not author solution edits.
    Disabled,
    /// The supervisor may execute a bounded direct edit through a GPT patch turn.
    SurgicalPatch,
}

impl SupervisorDirectEditCapability {
    /// Return whether supervisor review may emit action=supervisor_direct_edit.
    pub(crate) fn allows_direct_edit(self) -> bool {
        matches!(self, Self::SurgicalPatch)
    }
}

/// Prompt and behavior contract for one default strategy mode.
pub(crate) struct DefaultStrategyPolicy {
    /// Stable strategy label for metrics and prompts.
    pub(crate) id: &'static str,
    /// Supervisor-directed direct-edit capability for this strategy.
    pub(crate) supervisor_direct_edit: SupervisorDirectEditCapability,
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
        if self.supervisor_direct_edit.allows_direct_edit() {
            format!(
                r#"{{"action":"approve|worker_edit|worker_inspect|supervisor_direct_edit|stop","expect_patch":true|false,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","approval_state":"not_close|broad_work_remaining|approval_possible_after_verification|ready_to_approve|blocked","approval_blocker":"max 30 words; empty only when ready_to_approve","approval_contract":[{{"requirement":"task-derived behavior or risk","status":"passed|covered_by_existing_test|not_applicable|pending|failed","evidence":"artifact/source/check evidence or reason","next_check":"optional deterministic check or fix"}}],"context_recommendation":{{"action":"continue|compact_now|compact_after_next_worker","reason":"max 20 words"}},"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"],"supervisor_direct_edit_reason":"required for supervisor_direct_edit; max 40 words","direct_plan":["required for supervisor_direct_edit; exact surgical patch target"]{debug_json_field}}}"#
            )
        } else {
            format!(
                r#"{{"action":"approve|worker_edit|worker_inspect|stop","expect_patch":true|false,"worker_mode":"continue|context_focus","patch_decision":"accept_current|accept_current_baseline|revise_current|revise_previous","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","approval_state":"not_close|broad_work_remaining|approval_possible_after_verification|ready_to_approve|blocked","approval_blocker":"max 30 words; empty only when ready_to_approve","approval_contract":[{{"requirement":"task-derived behavior or risk","status":"passed|covered_by_existing_test|not_applicable|pending|failed","evidence":"artifact/source/check evidence or reason","next_check":"optional deterministic check or fix"}}],"context_recommendation":{{"action":"continue|compact_now|compact_after_next_worker","reason":"max 20 words"}},"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit or planning question"],"edit_packet":["optional cost-justified source context"],"source_snippets":["optional cost-justified snippets"],"edit_plan":["optional concrete steps or planning questions"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional patch gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional worker limits"]{debug_json_field}}}"#
            )
        }
    }
}

const SUPERVISED_WORKER_POLICY: DefaultStrategyPolicy = DefaultStrategyPolicy {
    id: "supervised-worker",
    supervisor_direct_edit: SupervisorDirectEditCapability::Disabled,
    review_instruction: "Decide the next worker-loop action. Use approve only when the worker result is acceptable. Use worker_edit when the worker should change repo state, and worker_inspect when it should inspect, verify, or plan without a patch. Use stop only to record a blocked or inconclusive worker result when no useful worker path remains; do not author task-solving source changes.",
    feedback_policy: r#"Strategy mode: supervised-worker.
- The supervisor remains in review/planning mode. Do not use action=supervisor_direct_edit.
- Delegate further implementation, repair, or verification work to the worker when the result is not ready to approve."#,
    metrics_note: "The supervisor controls the worker loop with approve, worker_edit, worker_inspect, or blocked/inconclusive stop decisions; supervisor-authored patching is not part of this strategy.",
    debug_delegation_decision: false,
};

const WORKER_BUILD_SUPERVISOR_FIX_POLICY: DefaultStrategyPolicy = DefaultStrategyPolicy {
    id: "worker-build-supervisor-fix",
    supervisor_direct_edit: SupervisorDirectEditCapability::SurgicalPatch,
    review_instruction: "Decide the next worker-build-supervisor-fix action. Use approve only when the current source is acceptable. Use worker_edit when the next work is worker-scale and should change repo state: broad construction, uncertain investigation plus editing, multi-file exploration, generated-output synchronization, or checks likely to drive implementation. Use worker_inspect when the worker should inspect, verify, or plan without a patch. Use supervisor_direct_edit only when the remaining edit is surgical: the defect, target files, and intended change are already known from current context and can be handed to a fresh GPT patch session. Use stop only when no useful worker or supervisor_direct_edit path remains. Do not author task-solving source changes during this review turn.",
    feedback_policy: r#"Strategy mode: worker-build-supervisor-fix.
- Use the worker for construction: broad implementation slices, new subsystem wiring, generated-output synchronization, or meaningful test clusters that still require exploration plus editing.
- Use action=supervisor_direct_edit only for surgical correction once a usable baseline exists: named residual defects, edge cases inside already-attempted work, error wording, propagation, shadowing, nil/zero-value behavior, formatting, or another small repair where the target files and intended edit are already known. Mixmod will hand direct_plan to a fresh GPT patch session.
- Before action=worker_edit or action=worker_inspect, classify the next request. Choose worker_edit when the next step needs broad search, large context reads, test exploration, generated-file inspection, multi-subsystem reasoning, or a check likely to uncover implementation work and may change repo state. Choose worker_inspect only when no patch should be produced.
- Before action=supervisor_direct_edit, confirm the direct_plan can be executed without broad exploration. If the plan would require broad shell/test commands, reading large or generated files, or discovering where the bug lives, keep the work with the normal worker.
- Corrections can appear before every broad task area is complete. If the current next step is surgical correction, use supervisor_direct_edit now; later broad construction should happen only if the direct edit establishes that another worker-scale slice remains.
- For supervisor_direct_edit, include supervisor_direct_edit_reason and direct_plan. direct_plan must name the exact residual defect and target files when known. Put broad or command-based checks in a later worker_edit or worker_inspect turn."#,
    metrics_note: "In worker-build-supervisor-fix mode, the supervisor may choose supervisor_direct_edit only for surgical corrective work; Mixmod executes that direct_plan in a fresh GPT patch session. Broad or uncertain follow-up work remains delegated to the normal worker.",
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
