use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::*;

use super::common::{
    DEBUG_PROFILE_FIT_ENV, render_worker_guidance, supervisor_implementation_slice_policy,
    supervisor_worker_shape_contract, supervisor_worktree_policy,
};

pub(crate) fn supervisor_worker_brief_prompt(
    work_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
    init_mode: SupervisorInitMode,
) -> Result<String> {
    supervisor_worker_brief_prompt_inner(
        work_dir,
        task_path,
        worker_guidance,
        init_mode,
        env_bool(DEBUG_PROFILE_FIT_ENV).unwrap_or(false),
    )
}

#[cfg(test)]
pub(crate) fn supervisor_worker_brief_prompt_with_debug_profile_fit(
    work_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
    init_mode: SupervisorInitMode,
) -> Result<String> {
    supervisor_worker_brief_prompt_inner(work_dir, task_path, worker_guidance, init_mode, true)
}

fn supervisor_worker_brief_prompt_inner(
    work_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
    init_mode: SupervisorInitMode,
    debug_profile_fit: bool,
) -> Result<String> {
    let task_value = read_json_file(task_path)?;
    let visible_task = agent_visible_task_value(&task_value);
    let task = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for worker brief prompt")?;
    let candidate_files = supervisor_candidate_file_index(work_dir, &visible_task);
    let shape_contract = supervisor_worker_shape_contract(worker_guidance);
    let profile_fit_debug = supervisor_profile_fit_debug(worker_guidance, debug_profile_fit);
    let worker_guidance = render_worker_guidance(worker_guidance);
    let init_instructions = worker_brief_init_instructions(init_mode);
    let worktree_policy = supervisor_worktree_policy();
    let slice_sizing_policy = supervisor_implementation_slice_policy();
    Ok(format!(
        r#"You are the Mixmod supervisor for a local worker.

Mission: complete the task while minimizing expensive supervisor GPT output tokens. Local worker tokens are cheap. Use the worker for concrete implementation, focused inspection, and verification whenever it can plausibly make progress. Keep your own visible output compact, but remain accountable for correctness; a false or premature handoff/approval wastes the run.

{init_instructions}
The worker receives the original task JSON and can inspect, edit, and test the repo.
{worktree_policy}
Candidate repo file contents are not embedded. Inspect repo files or git state only when that improves the handoff.

Worker profile:
{worker_guidance}

Worker shape contract:
{shape_contract}

{slice_sizing_policy}

Choose the cheapest reliable next worker handoff:
- Use {{"handoff":"as_given"}} only when the original task already gives enough files, behavior, and checks; still include worker_turn_shape and related boundary fields when the selected worker shape contract requires them.
- Use "guided" or "focused" for normal implementation work.
- Use worker_turn_shape="planning_probe" with expect_patch=false only when a short worker investigation can save supervisor output or avoid a bad implementation handoff. Ask for a compact proposal, not edits.
- For expected-patch implementation handoffs, obey the worker shape contract before choosing field detail.
- Choose the smallest reviewable implementation slice that makes meaningful progress within the selected worker shape contract. Do not split into trivial mechanical steps; broaden only when worker evidence shows the selected profile can handle that implementation surface cleanly and the broader request still fits the patch-size guidance.
- Treat worker profile patch-size guidance as a decomposition budget. If the full task likely crosses that budget or spans independent implementation slices, hand off only the next reviewable slice.
- If the route is clear, hand off concrete source edits instead of spending GPT output explaining the whole solution.
- For generated outputs, keep the request bounded to intentional repo outputs. Ask the worker to leave no transient generator/debug/build sidecars and to report broad unrelated generator churn instead of carrying it forward.
{profile_fit_debug_requirements}

Handoff requirements:
- Emit minified JSON only; no markdown, no explanation.
- Required field: "handoff" = "as_given" | "focused" | "guided" | "blocked".
- Include "expect_patch":true when the worker should edit the repo.
- Use concrete repo file paths, not directories.
- message_to_worker is only the short command the worker should follow.
- exact_edits is optional. Use it only when you already know the precise source edit or when a prior worker turn drifted.
- If exact_edits is present, use immediately executable edit instructions; do not use broad "investigate/understand/design" wording there.
- Put checks in deferred_checks when they should run only after a non-empty diff exists.
- edit_packet/source_snippets are optional and sparse. Omit them by default; include them only when you already have a cheap anchor/snippet and it will likely save more worker exploration than it costs in supervisor output.
- stop_condition is the worker-visible point where this turn should stop and return for supervisor review; include it when the selected worker shape contract requires a reviewable slice boundary.
- completion_gate is only for acceptance criteria not already covered by the patch request or deferred_checks.
- forbidden_actions is only for task-specific limits beyond normal noninteractive worker behavior.
- Omit optional fields unless they reduce worker confusion or supervisor output.

JSON shape:
{{"handoff":"guided","expect_patch":true,"worker_turn_shape":"planning_probe|patch_request|bounded_feature_slice|default","turn_goal":"one-turn goal","message_to_worker":"short worker instruction","files":["repo/path"],"exact_edits":["optional concrete edit"],"edit_packet":["optional cost-justified anchor"],"source_snippets":["optional cost-justified snippet"],"edit_plan":["optional short steps"],"checks":["optional checks"],"deferred_checks":["checks after patch"],"defer_checks_until_patch_exists":true,"stop_condition":"worker-visible turn stop when required","completion_gate":"optional gate","scope_rationale":"optional compact broad-scope justification","forbidden_actions":["optional limits"],"investigation_summary":"optional short finding","evidence":["optional file/function clues"],"avoid":["optional constraints"],"risk":"optional short risk"{profile_fit_debug_json_field}}}
Working repo: {work_dir}

Task JSON:
```json
{task}
```

Candidate repo files:
{candidate_files}
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
        profile_fit_debug_requirements = profile_fit_debug.requirements,
        profile_fit_debug_json_field = profile_fit_debug.json_field,
        slice_sizing_policy = slice_sizing_policy,
    ))
}

fn supervisor_candidate_file_index(work_dir: &Path, task: &Value) -> String {
    let files = get_string_array(task, "files");
    if files.is_empty() {
        return "- none listed in task".to_string();
    }
    files
        .into_iter()
        .map(|file| {
            let path = work_dir.join(&file);
            let status = match fs::metadata(&path) {
                Ok(metadata) if metadata.is_dir() => "directory".to_string(),
                Ok(metadata) => format!("file, {} bytes", metadata.len()),
                Err(error) => format!("missing: {error}"),
            };
            format!("- `{file}` ({status}) - listed by task")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct ProfileFitDebugPrompt {
    requirements: &'static str,
    json_field: &'static str,
}

fn supervisor_profile_fit_debug(
    worker_guidance: &WorkerSupervisorGuidance,
    enabled: bool,
) -> ProfileFitDebugPrompt {
    if !enabled || worker_guidance.is_empty() {
        return ProfileFitDebugPrompt {
            requirements: "",
            json_field: "",
        };
    }
    ProfileFitDebugPrompt {
        requirements: r#"
Debug profile-fit audit:
- Include "profile_fit" on expected-patch handoffs.
- profile_fit must name file_count, implementation layers, generated_or_large_files, expected_patch_fit, profile_risk, and scope_adjustment for this exact turn_goal, file list, stop_condition, and checks.
- If the justification would be weak, shrink the handoff to the next reviewable slice before emitting JSON."#,
        json_field: r#","profile_fit":{"file_count":0,"layers":["debug-only"],"generated_or_large_files":false,"expected_patch_fit":"debug-only","profile_risk":"debug-only","scope_adjustment":"debug-only"}"#,
    }
}

fn worker_brief_init_instructions(init_mode: SupervisorInitMode) -> &'static str {
    match init_mode {
        SupervisorInitMode::Compact => {
            r#"Use the task JSON and candidate repo paths first. Do not run tests, install dependencies, implement the patch, or ask the user for approval. Inspect the repo only if that prevents a likely bad handoff. Target <=160 supervisor output tokens for normal tasks."#
        }
        SupervisorInitMode::Investigate => {
            r#"Use the task JSON and candidate repo paths first. Inspect repo context only when that is needed to create a reliable low-output worker handoff. If needed, you may inspect source/test files and run discovery commands such as `rg`, `find`, `ls`, `sed`, `git status`, `git diff`, or `git grep`. Do not run tests, install dependencies, inspect Mixmod state/artifact directories, or ask the user for approval. Stop inspecting once you can choose a reliable worker handoff. Target <=500 supervisor output tokens."#
        }
    }
}
