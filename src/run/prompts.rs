use crate::*;

use super::recovery::RevisionNoopContext;

pub(crate) fn build_opencode_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    _task_path: &Path,
    _out_dir: &Path,
) -> Result<String> {
    let small_patch_slice = is_small_patch_slice_task(task);
    let files = if task.files.is_empty() {
        "- none specified".to_string()
    } else {
        task.files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tests = if task.tests.is_empty() {
        "- none specified".to_string()
    } else {
        task.tests
            .iter()
            .map(|test| format!("- `{test}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let constraints = if task.constraints.is_empty() {
        "- Keep work bounded to the task.\n- Return concise findings and avoid long pasted logs."
            .to_string()
    } else {
        task.constraints
            .iter()
            .map(|constraint| format!("- {constraint}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let acceptance = if task.acceptance.is_empty() {
        "- State whether the task appears complete.".to_string()
    } else {
        task.acceptance
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let completion_self_check = if small_patch_slice {
        r#"Before finalizing, do a completion self-check:
- Did you follow the supervisor's current instruction?
- If any requested edit or check remains incomplete, say exactly what remains incomplete.

Do not claim success if requested work remains incomplete."#
    } else {
        r#"Before finalizing, do a completion self-check:
- Did you complete every edit you intended to make?
- If you intended checks or verification, did you complete them?
- If any intended edit or check remains incomplete, say exactly what remains incomplete.

Do not claim success if intended edits or intended checks are incomplete."#
    };
    let output_contract = if small_patch_slice {
        r#"Keep the final stdout response compact and include:
- Changed files
- Risks or blocker, if any

Mention checks only if you actually ran one.
Do not paste long logs. Mixmod captures stdout, stderr, patch, metrics, and session artifacts on disk."#
    } else {
        r#"Keep the final stdout response compact and include:
- Summary
- Changed files
- Tests run and results
- Risks or supervisor questions

Stop immediately after the requested tests pass. Do not keep exploring after a passing test run.
Do not paste long logs. Mixmod captures stdout, stderr, patch, metrics, and session artifacts on disk."#
    };

    Ok(format!(
        r#"# Mixmod Local Worker Task

You are the Mixmod worker supervised by the supervisor model.
The supervisor remains the final authority. Treat your own output as a draft artifact for review.

Mode: {mode}
Expected repository patch: {expected_patch}

Mixmod-managed state lives outside this repository. Do not inspect Mixmod state or artifact directories. The task content you need is embedded below.

## Task

Title: {title}

{instructions}

## Relevant Files

{files}

## Requested Tests

{tests}

## Constraints

{constraints}

## Acceptance

{acceptance}

## Completion Self-Check

{completion_self_check}

## Output Contract

{output_contract}
"#,
        mode = mode,
        expected_patch = if expected_patch_for_instruction(mode, task) {
            "yes"
        } else {
            "no"
        },
        title = task.title,
        instructions = task.instructions,
        files = files,
        tests = tests,
        constraints = constraints,
        acceptance = acceptance,
        completion_self_check = completion_self_check,
        output_contract = output_contract,
    ))
}

pub(super) fn build_revision_noop_followup_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    revision: &RevisionNoopContext,
) -> String {
    if revision.revision_handoff.is_small_patch_slice() {
        return build_small_patch_slice_revision_noop_followup_instruction(mode, task, revision);
    }
    let files = string_list_or_none(&revision_focus_files(task, revision));
    let checks = string_list_or_none(&revision.required_checks);
    let message = if revision.message_to_worker.trim().is_empty() {
        "Apply the supervisor-requested revision from the current task context.".to_string()
    } else {
        revision.message_to_worker.trim().to_string()
    };
    format!(
        r#"# Revision No-Op Follow-Up

Mode: {mode}
Expected repository patch: yes

Mixmod-managed state lives outside this repository. Do not inspect Mixmod state or artifact directories. The task content you need is embedded below.

Your previous revision turn made no repository changes. The supervisor requested a revision, so that turn is incomplete.

Apply the requested revision now in the existing worktree, or return exactly `BLOCKED: <reason>` if you cannot make the edit.

Do not only inspect files. Do not restate the plan. Do not finalize unless you have changed the repository or returned `BLOCKED`.

Required revision:
{message}

Patch decision: {patch_decision}
Worker mode: {worker_mode}

Focus files:
{files}

Required checks:
{checks}

Keep the final response compact.
"#,
        mode = mode,
        message = message,
        patch_decision = if revision.patch_decision.is_empty() {
            "revise_current"
        } else {
            &revision.patch_decision
        },
        worker_mode = revision.worker_mode,
        files = files,
        checks = checks,
    )
}

fn build_small_patch_slice_revision_noop_followup_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    revision: &RevisionNoopContext,
) -> String {
    let files = string_list_or_none(&revision_focus_files(task, revision));
    let first_edit = revision
        .revision_handoff
        .exact_edits
        .first()
        .map(String::as_str)
        .filter(|edit| !edit.trim().is_empty())
        .unwrap_or_else(|| {
            if revision.message_to_worker.trim().is_empty() {
                "Apply the supervisor-requested source edit."
            } else {
                revision.message_to_worker.trim()
            }
        });
    let turn_goal = revision
        .revision_handoff
        .turn_goal
        .as_deref()
        .filter(|goal| !goal.trim().is_empty())
        .unwrap_or(first_edit);
    let completion_gate = revision
        .revision_handoff
        .completion_gate
        .as_deref()
        .map(str::trim)
        .filter(|gate| !gate.is_empty());
    let mut hard_rules = Vec::new();
    for action in &revision.revision_handoff.forbidden_actions {
        let action = action.trim().trim_end_matches('.');
        if action.is_empty() {
            continue;
        }
        let rule = if action.to_ascii_lowercase().starts_with("do not ") {
            format!("{action}.")
        } else {
            format!("Do not {action}.")
        };
        if !hard_rules.contains(&rule) {
            hard_rules.push(rule);
        }
    }
    let hard_rules_note = if hard_rules.is_empty() {
        String::new()
    } else {
        format!(
            "\nSupervisor worker limits:\n{}\n",
            plain_string_list_or_none(&hard_rules)
        )
    };
    let completion_gate_note = completion_gate
        .map(|gate| format!("\nSupervisor completion gate:\n{gate}\n"))
        .unwrap_or_default();

    format!(
        r#"# Revision No-Op Follow-Up

Mode: {mode}
Expected repository patch: yes

Mixmod-managed state lives outside this repository. Do not inspect Mixmod state or artifact directories.

Your previous revision turn made no new repository delta. That turn is incomplete.

Continue in the existing worktree and follow the supervisor's requested patch slice now.
{hard_rules_note}

Patch slice goal: {turn_goal}

Supervisor-requested edit:
1. {first_edit}

Focus files:
{files}

Do not expand beyond this one edit. Do not implement neighboring behavior, validation, aliases, serialization/deserialization variants, or tests unless that exact edit explicitly requires it.
{completion_gate_note}

Final response format:
Changed files: <comma-separated list>
"#,
        mode = mode,
        hard_rules_note = hard_rules_note,
        completion_gate_note = completion_gate_note,
    )
}

pub(super) fn revision_focus_files(task: &TaskSpec, revision: &RevisionNoopContext) -> Vec<String> {
    if revision.focus_files.is_empty() {
        task.files.clone()
    } else {
        revision.focus_files.clone()
    }
}

pub(super) fn build_empty_patch_followup_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    _task_path: &Path,
    _out_dir: &Path,
) -> String {
    let files = if task.files.is_empty() {
        "- none specified".to_string()
    } else {
        task.files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tests = if task.tests.is_empty() {
        "- none specified".to_string()
    } else {
        task.tests
            .iter()
            .map(|test| format!("- `{test}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"# Empty-Patch Follow-Up

Mode: {mode}
Expected repository patch: yes

Mixmod-managed state lives outside this repository. Do not inspect Mixmod state or artifact directories. The task content you need is embedded below.

The previous local-worker run exited successfully, but Mixmod captured no repository diff.

Confirm one of:
1. No patch is actually needed. Explain briefly why.
2. A patch is needed. Make the intended edits now, then finalize.
3. You are blocked. Explain the blocker briefly.

If you intended edits, do not finalize until they are made.

Relevant files:
{files}

Requested tests:
{tests}

Keep the final response compact.
"#,
        mode = mode,
        files = files,
        tests = tests,
    )
}

fn expected_patch_for_instruction(mode: DelegationMode, task: &TaskSpec) -> bool {
    if mode != DelegationMode::Patch {
        return false;
    }
    task.expect_patch
        .or_else(|| get_bool(&task.context, "expect_patch"))
        .or_else(|| {
            task.context
                .get("worker_brief")
                .and_then(|brief| get_bool(brief, "expect_patch"))
        })
        .unwrap_or(true)
}

fn is_small_patch_slice_task(task: &TaskSpec) -> bool {
    task.context
        .get("worker_brief")
        .and_then(|brief| get_str(brief, "worker_turn_shape"))
        .or_else(|| {
            task.context
                .get("revision")
                .and_then(|revision| get_str(revision, "worker_turn_shape"))
        })
        .is_some_and(|shape| shape.trim() == "small_patch_slice")
}

fn string_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "- none specified".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- `{item}`"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn plain_string_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "- none specified".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_noop_followup_keeps_small_patch_slice_shape() {
        let task = TaskSpec {
            title: "Flatten task".to_string(),
            files: vec!["builder.py".to_string(), "test_builder.py".to_string()],
            ..TaskSpec::default()
        };
        let revision = RevisionNoopContext {
            delta_expected: true,
            message_to_worker: "Make one builder edit.".to_string(),
            revision_handoff: RevisionHandoff {
                worker_turn_shape: Some("small_patch_slice".to_string()),
                worker_role: Some("patch_slice".to_string()),
                turn_goal: Some("first builder edit".to_string()),
                exact_edits: vec![
                    "Edit builder.py around the packer loop only.".to_string(),
                    "Do not add tests.".to_string(),
                ],
                edit_plan: vec![],
                deferred_checks: vec![],
                defer_checks_until_patch_exists: Some(true),
                completion_gate: Some("worker-visible gate from supervisor".to_string()),
                forbidden_actions: vec!["run tests before editing".to_string()],
            },
            focus_files: vec!["builder.py".to_string()],
            required_checks: vec!["pytest test_builder.py".to_string()],
            worker_mode: "continue".to_string(),
            patch_decision: "revise_current".to_string(),
        };

        let instruction =
            build_revision_noop_followup_instruction(DelegationMode::Patch, &task, &revision);

        assert!(instruction.contains("follow the supervisor's requested patch slice now"));
        assert!(instruction.contains("Supervisor-requested edit:"));
        assert!(instruction.contains("Edit builder.py around the packer loop only."));
        assert!(instruction.contains("Do not run tests before editing."));
        assert!(instruction.contains("Supervisor completion gate:"));
        assert!(instruction.contains("worker-visible gate from supervisor"));
        assert!(!instruction.contains("Required checks:"));
        assert!(!instruction.contains("Tests run and results"));
        assert!(!instruction.contains("Diff non-empty: yes/no"));
    }
}
