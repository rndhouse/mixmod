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
                "This local Qwen worker is much less capable than the supervisor, but it is effectively zero marginal GPT-token cost; use it as a cheap tool for bounded work, not as the strategic owner.".to_string(),
                "This worker can spend a while reasoning before editing; do not assume it is stalled while OpenCode is still producing reasoning, tool, or stdout activity.".to_string(),
                "This worker can struggle with large effective context before an explicit overflow occurs; keep initial handoffs compact, split broad tasks into small concrete source slices, and avoid asking it to reread many files at once.".to_string(),
                "Treat this worker as a narrow local tool operator. Use it actively for exact file/symbol/line discovery, source maps, targeted code reading, command execution, failure summaries, focused probe design, and concrete bounded edits.".to_string(),
                "Treat Qwen summaries, reviews, and completion claims as fallible assistance rather than authority. They can reduce local evidence-gathering cost, but they are not final approval evidence by themselves.".to_string(),
                "Prefer bounded helper questions over GPT reading long files directly: ask Qwen to localize symbols, summarize one source path, compare two nearby branches, inspect one changed hunk, or propose one focused probe.".to_string(),
                "Qwen is weak at broad open-ended final diff review: it may spend the call reading large diffs or rerunning visible tests instead of finding missing behavior. For final verification, give it concrete commands, probes chosen by the supervisor, or one branch-specific question.".to_string(),
                "For bounded review questions, avoid prompts that invite whole-file reads. Ask for bounded snippets instead: git diff for named files, rg/grep around anchors, or sed ranges around the changed branch.".to_string(),
                "For parser, binding, destructuring, or assignment changes, do not approve from the main happy path alone; check alternate syntax/input shapes such as single target versus multi-target, scalar versus aggregate or multi-value RHS, and relevant scope writes.".to_string(),
                "For those syntax or assignment changes, ordinary package tests are insufficient unless they exercise the relevant alternate shape or the supervisor has direct code evidence that the alternate shape follows the same path.".to_string(),
                "For evaluator-style tasks, prefer temporary probes or uniquely named regression tests; avoid generic top-level helper names that could collide with hidden tests in the same package.".to_string(),
                "Use no-diff roles before and during patching when repo facts would otherwise cost supervisor context: inspect should return exact files, symbols, anchors, source-path summaries, and uncertainty; run_checks should return commands, exit status, and compact failure excerpts.".to_string(),
                "Do not ask this worker to own architecture, broad diagnosis, or final correctness. The supervisor should choose the strategy and use Qwen to gather evidence or execute a narrow edit.".to_string(),
                "When worker_session_token_peak is high for the configured context window, treat the current worker session as context-pressured; shrink the next revision or use worker_mode=context_focus if the next edit would require broad rereading.".to_string(),
                "For broad expected-patch tasks, use worker_turn_shape=small_patch_slice by default with one immediate source edit, one focused source file, a literal nearby anchor when available, no tests before a diff exists, and a compact edit packet/snippet so the worker can patch before broad exploration.".to_string(),
                "When giving a small_patch_slice, tell it to use the provided edit packet first and avoid reading whole large files before the first edit.".to_string(),
                "For revision small_patch_slice turns, make the next instruction executable from the current accumulated patch: preserve useful existing edits, name the one next source delta, and avoid telling the worker to restart from an earlier completed slice.".to_string(),
                "This worker often follows narrow edit instructions but may miss end-to-end semantics across slices; before approval, check the accumulated patch for integration gaps between helpers, callers, options, parser/generated code, state mutation paths, and error propagation.".to_string(),
                "Do not trust this worker's compile success, non-empty diff, or local slice summary as proof of task completion for behavior-changing work; require task-derived probes or focused tests that exercise the main requested behavior plus a likely negative or edge case.".to_string(),
                "Before approving behavior-changing work, state the task contract in your own terms and verify the important success and failure semantics against primary artifacts: command exit status/output snippets, focused probes, diff hunks, or source paths inspected directly by the supervisor.".to_string(),
                "When a sliced implementation adds a helper or option, verify that every relevant entry point actually uses it under the intended conditions; if that evidence is missing, send a verification-focused revision before approving.".to_string(),
                "For large functions or code-generation paths, provide one literal anchor plus the smallest local transformation near that anchor; avoid asking for an entire behavior path when a preparatory branch or helper would create useful progress.".to_string(),
                "For syntax repairs in string-literal or code-generation logic, prefer a compile-driven repair instruction: preserve the intended generated code, change the smallest local expression, run the focused parser/compile check immediately, and do not hand the worker unverified brace or quote substitutions as facts.".to_string(),
                "For alias/key generated-code repairs, hand off one path at a time such as valid-key collection, serialization key mapping, deserialization key mapping, or collision detection; when the source API permits either form, tell the worker to preserve both raw field names and resolved aliases.".to_string(),
                "For option families or behavior families with a base path plus modifiers, ask for the base behavior first and then one modifier family per later small_patch_slice unless prior worker turns show it can safely combine them.".to_string(),
                "After multiple clean small_patch_slice revisions with non-empty accurate deltas, no context overflow, and moderate token peak, consider the previous slices too small; promote within small_patch_slice to one coherent anchored source behavior instead of switching this profile to bounded_feature_slice.".to_string(),
                "If a small_patch_slice required live supervisor control, produced a large line delta, or needed a corrective follow-up, treat the prior slice as too broad; shrink the next revision and do not add another modifier family or validation concern until one clean corrective delta lands.".to_string(),
                "Once API plumbing and basic validation exist, prioritize the first useful behavior path over additional defensive validation slices unless the artifacts show validation is blocking progress.".to_string(),
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
        WorkerModelProfile {
            model: "openrouter/z-ai/glm-5.2".to_string(),
            aliases: vec![
                "openrouter/z-ai/glm-5.2".to_string(),
                "z-ai/glm-5.2".to_string(),
            ],
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
        },
    ]
}

fn normalize_model_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
