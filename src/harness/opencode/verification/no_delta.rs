use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::harness::AgentRequest;
use crate::{
    diff_without_unchanged_blocks, env_u64, get_bool, get_str, get_string_array,
    git_diff_with_untracked,
};

const DEFAULT_SMALL_PATCH_NO_DELTA_INTERRUPT_SECONDS: u64 = 300;
const DEFAULT_SMALL_PATCH_NO_DELTA_MAX_INTERRUPTS: u64 = 1;

#[derive(Debug)]
pub(super) struct SmallPatchNoDeltaIntervention {
    threshold: Duration,
    baseline_diff: String,
    interrupt_control: Value,
    stop_control: Value,
    interrupt_count: u64,
    max_interrupts: u64,
    stopped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmallPatchNoDeltaKind {
    Initial,
    Revision,
}

#[derive(Debug)]
struct SmallPatchNoDeltaTarget {
    kind: SmallPatchNoDeltaKind,
    focus_files: Vec<String>,
    exact_edits: Vec<String>,
    message_to_worker: String,
}

impl SmallPatchNoDeltaIntervention {
    pub(super) fn from_request(request: &AgentRequest) -> Option<Self> {
        let threshold_seconds = env_u64("MIXMOD_SMALL_PATCH_NO_DELTA_INTERRUPT_SECONDS")
            .or_else(|| env_u64("MIXMOD_REVISION_NO_DELTA_INTERRUPT_SECONDS"))
            .unwrap_or(DEFAULT_SMALL_PATCH_NO_DELTA_INTERRUPT_SECONDS);
        if threshold_seconds == 0 {
            return None;
        }
        let max_interrupts = env_u64("MIXMOD_SMALL_PATCH_NO_DELTA_MAX_INTERRUPTS")
            .or_else(|| env_u64("MIXMOD_REVISION_NO_DELTA_MAX_INTERRUPTS"))
            .unwrap_or(DEFAULT_SMALL_PATCH_NO_DELTA_MAX_INTERRUPTS);
        let task = fs::read_to_string(&request.task_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())?;
        let baseline_diff = git_diff_with_untracked(&request.root).ok()?;
        Self::from_task(&task, baseline_diff, threshold_seconds, max_interrupts)
    }

    pub(super) fn from_task(
        task: &Value,
        baseline_diff: String,
        threshold_seconds: u64,
        max_interrupts: u64,
    ) -> Option<Self> {
        if threshold_seconds == 0 {
            return None;
        }
        let target = small_patch_no_delta_target_from_task(task)?;
        Some(Self {
            threshold: Duration::from_secs(threshold_seconds),
            baseline_diff,
            interrupt_control: small_patch_no_delta_interrupt_control(&target),
            stop_control: small_patch_no_delta_stop_control(&target),
            interrupt_count: 0,
            max_interrupts,
            stopped: false,
        })
    }

    pub(super) fn maybe_control(
        &mut self,
        root: &Path,
        elapsed: Duration,
        last_output_age: Duration,
    ) -> Option<Value> {
        let current_diff = git_diff_with_untracked(root).ok()?;
        self.maybe_control_for_diff(&current_diff, elapsed, last_output_age)
    }

    pub(super) fn maybe_control_for_diff(
        &mut self,
        current_diff: &str,
        elapsed: Duration,
        last_output_age: Duration,
    ) -> Option<Value> {
        if self.stopped || elapsed < self.threshold || last_output_age < self.threshold {
            return None;
        }
        let new_delta = diff_without_unchanged_blocks(current_diff, &self.baseline_diff);
        if !new_delta.trim().is_empty() {
            return None;
        }
        if self.interrupt_count < self.max_interrupts {
            self.interrupt_count += 1;
            return Some(self.interrupt_control.clone());
        }
        self.stopped = true;
        Some(self.stop_control.clone())
    }
}

fn small_patch_no_delta_target_from_task(task: &Value) -> Option<SmallPatchNoDeltaTarget> {
    let context = task.get("context")?;
    if let Some(revision) = context.get("revision") {
        let delta_expected = get_bool(revision, "delta_expected").unwrap_or_else(|| {
            let patch_decision = get_str(revision, "patch_decision").unwrap_or("");
            matches!(patch_decision, "revise_current" | "revise_previous")
        });
        let small_patch_slice = get_str(revision, "worker_turn_shape")
            .is_some_and(|shape| shape.trim() == "small_patch_slice");
        if delta_expected && small_patch_slice {
            return Some(SmallPatchNoDeltaTarget {
                kind: SmallPatchNoDeltaKind::Revision,
                focus_files: get_string_array(revision, "focus_files"),
                exact_edits: get_string_array(revision, "exact_edits"),
                message_to_worker: get_str(revision, "message_to_worker")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            });
        }
    }

    let brief = context.get("worker_brief")?;
    let expect_patch = get_bool(task, "expect_patch")
        .or_else(|| get_bool(context, "expect_patch"))
        .or_else(|| get_bool(brief, "expect_patch"))
        .unwrap_or(true);
    let small_patch_slice = get_str(brief, "worker_turn_shape")
        .is_some_and(|shape| shape.trim() == "small_patch_slice");
    if !expect_patch || !small_patch_slice {
        return None;
    }
    let mut focus_files = first_non_empty_string_array(
        brief,
        &["focus_files", "files", "target_files", "repo_focus_files"],
    );
    if focus_files.is_empty() {
        focus_files = get_string_array(task, "files");
    }
    let exact_edits =
        first_non_empty_string_array(brief, &["exact_edits", "edit_plan", "implementation_steps"]);
    Some(SmallPatchNoDeltaTarget {
        kind: SmallPatchNoDeltaKind::Initial,
        focus_files,
        exact_edits,
        message_to_worker: get_str(brief, "message_to_worker")
            .or_else(|| get_str(brief, "message"))
            .unwrap_or("")
            .trim()
            .to_string(),
    })
}

fn first_non_empty_string_array(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| get_string_array(value, key))
        .find(|items| !items.is_empty())
        .unwrap_or_default()
}

fn small_patch_no_delta_interrupt_control(target: &SmallPatchNoDeltaTarget) -> Value {
    let first_edit = target
        .exact_edits
        .first()
        .map(String::as_str)
        .filter(|edit| !edit.trim().is_empty())
        .or_else(|| {
            (!target.message_to_worker.is_empty()).then_some(target.message_to_worker.as_str())
        })
        .unwrap_or("Apply the next requested source edit.")
        .trim();
    let first_file = target
        .focus_files
        .first()
        .map(String::as_str)
        .filter(|file| !file.trim().is_empty())
        .unwrap_or("the focused source file");
    let (source, phase, risk) = match target.kind {
        SmallPatchNoDeltaKind::Initial => (
            "auto_initial_no_delta",
            "first worker turn",
            "initial small-patch worker made no repository delta before the no-delta guard fired",
        ),
        SmallPatchNoDeltaKind::Revision => (
            "auto_revision_no_delta",
            "revision",
            "revision made no new repository delta before the no-delta guard fired",
        ),
    };
    json!({
        "action": "interrupt_continue",
        "worker_mode": "continue",
        "source": source,
        "worker_turn_shape": "small_patch_slice",
        "turn_goal": "make the first no-delta recovery edit",
        "exact_edits": [first_edit],
        "defer_checks_until_patch_exists": true,
        "completion_gate": "git diff --stat must be non-empty",
        "forbidden_actions": ["ask questions", "run tests before editing"],
        "message_to_worker": format!(
            "You have not modified files in this {phase}. Make only this edit now in {first_file}: {first_edit} Then run git diff --stat and stop with Diff non-empty: yes/no. Do not run tests."
        ),
        "focus_files": target.focus_files.clone(),
        "required_checks": [],
        "risk": risk
    })
}

fn small_patch_no_delta_stop_control(target: &SmallPatchNoDeltaTarget) -> Value {
    let first_edit = target
        .exact_edits
        .first()
        .map(String::as_str)
        .filter(|edit| !edit.trim().is_empty())
        .or_else(|| {
            (!target.message_to_worker.is_empty()).then_some(target.message_to_worker.as_str())
        })
        .unwrap_or("the requested small-patch edit")
        .trim();
    let source = match target.kind {
        SmallPatchNoDeltaKind::Initial => "auto_initial_no_delta_stop",
        SmallPatchNoDeltaKind::Revision => "auto_revision_no_delta_stop",
    };
    json!({
        "action": "stop",
        "worker_mode": "continue",
        "source": source,
        "message_to_worker": format!(
            "Worker made no repository delta after no-delta recovery. Stopping after failing to apply: {first_edit}"
        ),
        "focus_files": target.focus_files.clone(),
        "required_checks": [],
        "risk": "worker_stalled_no_delta"
    })
}
