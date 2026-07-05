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
- Did you modify repository files?
- Did `git diff --stat` show a non-empty patch?
- If the diff is empty, continue editing instead of finalizing.

Do not claim success if the repository diff is empty."#
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
- Diff non-empty: yes/no
- Risks or blocker, if any

Do not mention tests unless you actually ran one by mistake.
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

Before finalizing after an edit, run `git diff --stat` and make sure the current patch differs from the previous candidate.
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
