use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::*;

use super::edit_packet::small_patch_edit_packet_from_decision;
use super::focus::split_worker_focus_files;
use super::format::{
    bullet_list, hard_rule_from_forbidden_action, immediate_small_patch_exact_edits, non_empty_or,
    numbered_list,
};
use super::types::{RevisionTask, RevisionTaskContext, RevisionTaskDetails};

pub(crate) fn write_revision_task(
    work_dir: &Path,
    task_path: &Path,
    default_dir: &Path,
    experiment_name: &str,
    decision: &SupervisorFeedbackTurn,
    revision_index: u64,
) -> Result<PathBuf> {
    let task_value = read_json_file(task_path)?;
    let task_value = agent_visible_task_value(&task_value);
    let (repo_focus_files, artifact_focus_files) =
        split_worker_focus_files(work_dir, default_dir, &decision.focus_files);
    let focus_files = non_empty_or(
        repo_focus_files.clone(),
        get_string_array(&task_value, "files"),
    );
    let artifact_note = if artifact_focus_files.is_empty() {
        String::new()
    } else {
        format!(
            "\nMixmod artifact references from the supervisor, not repo source files: {:?}\nDo not read these from the repo root; use the current task text and the supervisor message instead.",
            artifact_focus_files
        )
    };
    let focus_note = format!(
        "Repo focus files: {:?}{artifact_note}",
        if repo_focus_files.is_empty() {
            focus_files.clone()
        } else {
            repo_focus_files.clone()
        }
    );
    let small_patch_slice = decision.revision_handoff.is_small_patch_slice();
    let bounded_feature_slice = decision.revision_handoff.is_bounded_feature_slice();
    let defer_checks_until_patch_exists = decision
        .revision_handoff
        .defer_checks_until_patch_exists
        .unwrap_or(small_patch_slice);
    let completion_gate = decision
        .revision_handoff
        .completion_gate
        .as_deref()
        .filter(|gate| !gate.trim().is_empty())
        .unwrap_or("git diff --stat must be non-empty");
    let acceptance = if small_patch_slice {
        vec![completion_gate.to_string()]
    } else {
        non_empty_or(
            decision.required_checks.clone(),
            get_string_array(&task_value, "acceptance"),
        )
    };
    let mut constraints = get_string_array(&task_value, "constraints");
    constraints.push("Keep the revision focused.".to_string());
    constraints.push("Do not paste long logs.".to_string());
    if small_patch_slice {
        constraints
            .push("Do not ask questions; make a reasonable assumption and edit.".to_string());
        if defer_checks_until_patch_exists {
            constraints.push(
                "Do not run tests in this revision turn; checks are deferred until a patch exists."
                    .to_string(),
            );
        }
        constraints.push("Do not finalize until git diff --stat is non-empty.".to_string());
    } else if bounded_feature_slice {
        constraints
            .push("Do not ask questions; make a reasonable assumption and continue.".to_string());
        constraints.push(
            "Complete the bounded feature chunk before running broad checks or finalizing."
                .to_string(),
        );
        constraints.push(
            "Run focused checks after the repository patch exists when checks are available."
                .to_string(),
        );
    }
    constraints.sort();
    constraints.dedup();
    let original_instructions = get_str(&task_value, "instructions").unwrap_or("Revise the patch.");
    let patch_decision_note = if decision.patch_decision == "revise_previous" {
        "\nPatch checkpoint decision: revise_previous. Mixmod has restored the previous candidate patch in the worktree before this turn. Apply only the focused follow-up edit from the supervisor message below. Do not read Mixmod artifacts directly.\n"
    } else if decision.patch_decision == "revise_current" {
        "\nPatch checkpoint decision: revise_current. Continue from the current worktree patch and fix the issues the supervisor identified.\n"
    } else {
        ""
    };
    let instructions = if small_patch_slice {
        small_patch_slice_revision_instructions(SmallPatchSliceRevisionInput {
            work_dir,
            original: &task_value,
            decision,
            focus_files: &focus_files,
            focus_note: &focus_note,
            completion_gate,
            patch_decision_note,
            defer_checks_until_patch_exists,
        })
    } else if decision.worker_mode == "context_focus" {
        format!(
            "Original task instructions:\n{original_instructions}\n\nThe supervisor requested worker_mode=context_focus.\nThis starts a new worker session on the current worktree.\nTreat this as a fresh focused worker attempt and ignore previous worker reasoning unless it is repeated here.{patch_decision_note}\nSupervisor message to worker:\n{}\n\n{focus_note}\nRequired checks: {:?}\nIf checks cannot run because of local environment problems, make the code/test edit first and report the blocker compactly.",
            decision.hint, decision.required_checks
        )
    } else if bounded_feature_slice {
        bounded_feature_slice_revision_instructions(
            &task_value,
            decision,
            &focus_files,
            &focus_note,
            patch_decision_note,
        )
    } else {
        format!(
            "{original_instructions}\n\nSupervisor decision: revise\nWorker mode: continue\nSame worker session should be reused when available.{patch_decision_note}\nMessage to worker: {}\n{focus_note}\nRequired checks: {:?}\nContinue work from the current working tree and return compact artifacts for supervisor review.",
            decision.hint, decision.required_checks
        )
    };
    let revision = RevisionTask {
        title: format!(
            "Revision {}: {}",
            revision_index,
            get_str(&task_value, "title").unwrap_or(experiment_name)
        ),
        instructions,
        expect_patch: true,
        worker_mode: &decision.worker_mode,
        files: focus_files,
        tests: if small_patch_slice && defer_checks_until_patch_exists {
            Vec::new()
        } else {
            get_string_array(&task_value, "tests")
        },
        constraints,
        acceptance,
        context: RevisionTaskContext {
            expect_patch: true,
            codex_focus_files: &decision.focus_files,
            repo_focus_files: &repo_focus_files,
            patch_decision: &decision.patch_decision,
            revision: RevisionTaskDetails {
                delta_expected: revision_delta_expected(decision),
                message_to_worker: &decision.hint,
                worker_mode: &decision.worker_mode,
                patch_decision: &decision.patch_decision,
                worker_turn_shape: decision.revision_handoff.worker_turn_shape.as_deref(),
                turn_goal: decision.revision_handoff.turn_goal.as_deref(),
                exact_edits: &decision.revision_handoff.exact_edits,
                edit_plan: &decision.revision_handoff.edit_plan,
                deferred_checks: &decision.revision_handoff.deferred_checks,
                defer_checks_until_patch_exists: decision
                    .revision_handoff
                    .defer_checks_until_patch_exists,
                completion_gate: decision.revision_handoff.completion_gate.as_deref(),
                forbidden_actions: &decision.revision_handoff.forbidden_actions,
                focus_files: &decision.focus_files,
                repo_focus_files: &repo_focus_files,
                required_checks: &decision.required_checks,
            },
            mixmod_artifact_refs: &artifact_focus_files,
        },
    };
    let path = if revision_index == 1 {
        default_dir.join("revision-task.json")
    } else {
        default_dir.join(format!("revision-task-{revision_index}.json"))
    };
    write_pretty_json(&path, &revision, "revision task")?;
    if revision_index != 1 {
        write_pretty_json(
            &default_dir.join("revision-task.json"),
            &revision,
            "latest revision task",
        )?;
    }
    Ok(path)
}

struct SmallPatchSliceRevisionInput<'a> {
    work_dir: &'a Path,
    original: &'a Value,
    decision: &'a SupervisorFeedbackTurn,
    focus_files: &'a [String],
    focus_note: &'a str,
    completion_gate: &'a str,
    patch_decision_note: &'a str,
    defer_checks_until_patch_exists: bool,
}

fn small_patch_slice_revision_instructions(input: SmallPatchSliceRevisionInput<'_>) -> String {
    let SmallPatchSliceRevisionInput {
        work_dir,
        original,
        decision,
        focus_files,
        focus_note,
        completion_gate,
        patch_decision_note,
        defer_checks_until_patch_exists,
    } = input;
    let title = get_str(original, "title").unwrap_or("the task");
    let original_instructions = get_str(original, "instructions")
        .unwrap_or("")
        .trim()
        .to_string();
    let original_context = if original_instructions.is_empty() {
        String::new()
    } else {
        format!(
            "\nOriginal task context, for alignment only:\n{}\n",
            truncate_for_report(&original_instructions, 1200)
        )
    };
    let fallback_goal = if decision.hint.trim().is_empty() {
        "Apply the supervisor-requested next patch slice."
    } else {
        decision.hint.trim()
    };
    let turn_goal = decision
        .revision_handoff
        .turn_goal
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_goal)
        .trim();
    let all_exact_edits = decision.revision_handoff.exact_edits.clone();
    let exact_edits = immediate_small_patch_exact_edits(&all_exact_edits, turn_goal);
    let deferred_edit_note = if all_exact_edits.len() > exact_edits.len() {
        format!(
            "The supervisor supplied {} additional edit(s); Mixmod is intentionally deferring them to later turns. Do not do them now.\n",
            all_exact_edits.len() - exact_edits.len()
        )
    } else {
        String::new()
    };
    let mut hard_rules = vec![
        "Do not ask questions.".to_string(),
        "Do not stop after reading files.".to_string(),
        "Do not inspect unrelated behavior outside this slice.".to_string(),
        "Do not final-answer until repository files are modified.".to_string(),
        "If something is ambiguous, make a reasonable assumption and continue editing.".to_string(),
    ];
    if defer_checks_until_patch_exists {
        hard_rules.push("Do not run tests in this turn.".to_string());
    } else {
        hard_rules.push("Make the code/test edit before running any check.".to_string());
    }
    for action in &decision.revision_handoff.forbidden_actions {
        let rule = hard_rule_from_forbidden_action(action);
        if !hard_rules.contains(&rule) {
            hard_rules.push(rule);
        }
    }

    let file_list = if focus_files.is_empty() {
        "- none specified".to_string()
    } else {
        focus_files
            .iter()
            .map(|file| format!("- {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let edit_packet =
        small_patch_edit_packet_from_decision(work_dir, focus_files, &exact_edits, decision);
    let checks = non_empty_or(
        decision.revision_handoff.deferred_checks.clone(),
        decision.required_checks.clone(),
    );
    let checks_note = if checks.is_empty() {
        "Deferred checks: none specified.".to_string()
    } else {
        format!(
            "Deferred checks after a non-empty patch exists:\n{}",
            bullet_list(&checks)
        )
    };

    format!(
        r#"Noninteractive coding revision. This is the full instruction. No user will answer questions.

Original task: {title}{original_context}
Current accumulated patch is useful but not yet accepted as the full solution.
Continue from the current working tree; do not revert existing correct edits.{patch_decision_note}

Your only goal in this revision turn is to add a non-empty repository delta for the next small patch slice.

Hard rules:
{hard_rules}

Patch slice goal: {turn_goal}

Make exactly this next small patch:
{exact_edits}
{deferred_edit_note}

Worker edit packet:
{edit_packet}

Relevant files:
{file_list}

{focus_note}
Use the Worker edit packet before reading whole files. If the packet contains the needed anchor, edit from that context first.
Use concrete files from the relevant file list. If a listed item is a directory, do not read the whole directory; choose the one file required by the exact edits.
Do not read an entire large file before the first edit unless the exact edit cannot be applied from the packet and focused anchor searches.
Do not expand beyond this patch slice unless one of the exact edits requires it.
If a listed file is missing, continue with the remaining exact edits; create a missing file only when an exact edit requires it.
{checks_note}

After editing, run exactly: git diff --stat
If git diff --stat is empty, you failed; edit files before finalizing.
Completion gate: {completion_gate}

Final response format:
Changed files: <comma-separated list>
Diff non-empty: yes/no
"#,
        hard_rules = bullet_list(&hard_rules),
        exact_edits = numbered_list(&exact_edits),
        deferred_edit_note = deferred_edit_note,
    )
}

fn bounded_feature_slice_revision_instructions(
    original: &Value,
    decision: &SupervisorFeedbackTurn,
    focus_files: &[String],
    focus_note: &str,
    patch_decision_note: &str,
) -> String {
    let original_instructions = get_str(original, "instructions")
        .unwrap_or("")
        .trim()
        .to_string();
    let original_context = if original_instructions.is_empty() {
        String::new()
    } else {
        format!(
            "\nOriginal task context, for alignment only:\n{}\n",
            truncate_for_report(&original_instructions, 1200)
        )
    };
    let fallback_goal = if decision.hint.trim().is_empty() {
        "Apply the supervisor-requested bounded feature revision."
    } else {
        decision.hint.trim()
    };
    let turn_goal = decision
        .revision_handoff
        .turn_goal
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_goal)
        .trim();
    let plan = non_empty_or(
        non_empty_or(
            decision.revision_handoff.edit_plan.clone(),
            decision.revision_handoff.exact_edits.clone(),
        ),
        vec![turn_goal.to_string()],
    );
    let checks = non_empty_or(
        decision.revision_handoff.deferred_checks.clone(),
        decision.required_checks.clone(),
    );
    let checks = if checks.is_empty() {
        "Run the narrowest compile or focused test check that is available after editing."
            .to_string()
    } else {
        numbered_list(&checks)
    };
    let file_list = if focus_files.is_empty() {
        "- none specified".to_string()
    } else {
        focus_files
            .iter()
            .map(|file| format!("- {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"Noninteractive coding revision. This is the full instruction. No user will answer questions.

Original task:{original_context}
Current accumulated patch is useful but not yet accepted as the full solution.
Continue from the current working tree; do not revert existing correct edits.{patch_decision_note}

Bounded feature revision goal:
{turn_goal}

Complete this coherent next behavior before finalizing. Keep the same OpenCode session context, use what you already learned, and avoid redoing broad investigation. A bounded chunk may include related source, API, serialization/deserialization, and focused test/check work when those edits belong together.

Edit plan:
{plan}

Relevant files:
{file_list}

{focus_note}
Use concrete repo files from the relevant file list. Do not inspect Mixmod state or artifact directories.

Rules:
- Do not ask questions.
- Do not stop after only reading files.
- Make repository edits before running broad checks.
- Preserve surrounding control flow and indentation in large functions.
- Do not rewrite unrelated code.
- Do not commit.

Checks after a patch exists:
{checks}

Final response format:
Summary: <one sentence>
Changed files: <comma-separated list>
Tests/checks: <commands and result, or not run with reason>
"#,
        plan = numbered_list(&plan),
    )
}

fn revision_delta_expected(decision: &SupervisorFeedbackTurn) -> bool {
    decision.verdict == "revise"
        || decision.patch_decision == "revise_current"
        || decision.patch_decision == "revise_previous"
}
