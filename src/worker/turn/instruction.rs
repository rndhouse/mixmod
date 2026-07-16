use crate::*;

use super::recovery::RevisionNoopContext;

pub(crate) fn build_worker_turn_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    _task_path: &Path,
    _out_dir: &Path,
) -> Result<String> {
    let patch_request = is_patch_request_task(task);
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
    let completion_self_check = if patch_request {
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
    let output_contract = if patch_request {
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
    if revision.revision_handoff.is_patch_request() {
        return build_patch_request_revision_noop_followup_instruction(mode, task, revision);
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

fn build_patch_request_revision_noop_followup_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    revision: &RevisionNoopContext,
) -> String {
    let files = string_list_or_none(&revision_focus_files(task, revision));
    let request_detail = revision
        .revision_handoff
        .exact_edits
        .first()
        .map(String::as_str)
        .filter(|edit| !edit.trim().is_empty())
        .unwrap_or_else(|| {
            if revision.message_to_worker.trim().is_empty() {
                "Apply the supervisor-requested source patch."
            } else {
                revision.message_to_worker.trim()
            }
        });
    let turn_goal = revision
        .revision_handoff
        .turn_goal
        .as_deref()
        .filter(|goal| !goal.trim().is_empty())
        .unwrap_or(request_detail);
    let completion_gate = revision
        .revision_handoff
        .completion_gate
        .as_deref()
        .map(str::trim)
        .filter(|gate| !gate.is_empty());
    let stop_condition = revision
        .revision_handoff
        .stop_condition
        .as_deref()
        .map(str::trim)
        .filter(|condition| !condition.is_empty());
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
    let stop_condition_note = stop_condition
        .map(|condition| format!("\nSupervisor stop condition:\n{condition}\n"))
        .unwrap_or_default();

    format!(
        r#"# Revision No-Op Follow-Up

Mode: {mode}
Expected repository patch: yes

Mixmod-managed state lives outside this repository. Do not inspect Mixmod state or artifact directories.

Your previous revision turn made no new repository delta. That turn is incomplete.

Continue in the existing worktree and follow the supervisor's requested patch request now.
{hard_rules_note}

Patch request goal: {turn_goal}

Supervisor request:
{request_detail}

Focus files:
{files}

Do not expand beyond this request. Do not implement neighboring behavior, validation, aliases, serialization/deserialization variants, or tests unless the supervisor request explicitly requires it.
{stop_condition_note}
{completion_gate_note}

Final response format:
Changed files: <comma-separated list>
"#,
        mode = mode,
        hard_rules_note = hard_rules_note,
        stop_condition_note = stop_condition_note,
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

pub(super) fn build_worker_self_review_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    review_files: &[String],
    worker_session_patch_path: &str,
) -> String {
    let files = if review_files.is_empty() {
        "- none".to_string()
    } else {
        review_files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"# Worker Self-Review Cleanup

Mode: {mode}
Expected repository patch: yes

Continue in the same worker session and review only the worker-session patch.
This is a cleanup pass before supervisor review, not a new implementation slice.

Allowed Mixmod artifact:
- `{worker_session_patch_path}`

Do:
- Read the allowed worker-session patch if you need the exact review scope.
- Check only the files listed below and only the hunks represented in that patch.
- Remove accidental artifacts, generated byproducts, debug prints, or unrelated edits only when they are part of that worker-session patch.
- Fix obvious syntax or consistency mistakes in files you already touched when the fix is trivial.
- Leave the worktree unchanged if there is no safe cleanup.

Do not:
- Add new feature scope.
- Rewrite the solution.
- Review or modify unrelated worktree changes, even if they are visible in `git diff`.
- Run broad tests.
- Inspect any Mixmod state or artifact path except the single patch file listed above.
- Inspect verifier internals or hidden solutions.
- Commit changes.

Task title: {title}

Files in this worker-session patch:
{files}

Final response format:
Cleanup changed files: <comma-separated list or none>
Concerns: <brief concerns or none>
"#,
        mode = mode,
        worker_session_patch_path = worker_session_patch_path,
        title = task.title,
        files = files,
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

fn is_patch_request_task(task: &TaskSpec) -> bool {
    task.context
        .get("worker_brief")
        .and_then(|brief| get_str(brief, "worker_turn_shape"))
        .or_else(|| {
            task.context
                .get("revision")
                .and_then(|revision| get_str(revision, "worker_turn_shape"))
        })
        .is_some_and(|shape| shape.trim() == "patch_request")
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
    fn revision_noop_followup_keeps_patch_request_shape() {
        let task = TaskSpec {
            title: "Flatten task".to_string(),
            files: vec!["builder.py".to_string(), "test_builder.py".to_string()],
            ..TaskSpec::default()
        };
        let revision = RevisionNoopContext {
            delta_expected: true,
            message_to_worker: "Make one builder edit.".to_string(),
            revision_handoff: RevisionHandoff {
                expect_patch: Some(true),
                worker_turn_shape: Some("patch_request".to_string()),
                turn_goal: Some("first builder edit".to_string()),
                exact_edits: vec![
                    "Edit builder.py around the packer loop only.".to_string(),
                    "Do not add tests.".to_string(),
                ],
                edit_plan: vec![],
                deferred_checks: vec![],
                defer_checks_until_patch_exists: Some(true),
                stop_condition: Some("return after making the requested edit".to_string()),
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

        assert!(instruction.contains("follow the supervisor's requested patch request now"));
        assert!(instruction.contains("Supervisor request:"));
        assert!(instruction.contains("Edit builder.py around the packer loop only."));
        assert!(instruction.contains("Do not run tests before editing."));
        assert!(instruction.contains("Supervisor completion gate:"));
        assert!(instruction.contains("worker-visible gate from supervisor"));
        assert!(!instruction.contains("Required checks:"));
        assert!(!instruction.contains("Tests run and results"));
        assert!(!instruction.contains("Diff non-empty: yes/no"));
    }
}
