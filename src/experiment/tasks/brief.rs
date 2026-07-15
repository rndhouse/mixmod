use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::*;

use super::edit_packet::{NO_EDIT_PACKET, patch_request_edit_packet_from_value};
use super::format::{
    append_handoff_list, bullet_list, file_list_or_none, hard_rule_from_forbidden_action,
    non_empty_or, numbered_list, optional_bullet_section, optional_numbered_section,
    optional_text_section,
};
use super::types::{WorkerBriefTask, WorkerBriefTaskContext};

pub(crate) fn write_worker_brief_task(
    work_dir: &Path,
    task_path: &Path,
    brief: &Value,
    default_dir: &Path,
) -> Result<PathBuf> {
    let original = read_json_file(task_path)?;
    let original = agent_visible_task_value(&original);
    let typed_brief = WorkerBrief::from_value(brief);
    let handoff = typed_brief.handoff.as_deref().unwrap_or_else(|| {
        if brief_has_legacy_guidance(brief) {
            "guided"
        } else {
            "as_given"
        }
    });
    let explicit_focus_files =
        first_non_empty_string_array(brief, &["files", "focus_files", "target_files"]);
    let target_files = non_empty_or(
        explicit_focus_files.clone(),
        get_string_array(&original, "files"),
    );
    let expect_patch = typed_brief.expect_patch.unwrap_or(handoff != "blocked");
    let worker_turn_shape = get_str(brief, "worker_turn_shape").unwrap_or("");
    let planning_probe = !expect_patch && worker_turn_shape.trim() == "planning_probe";
    let patch_request = expect_patch && worker_turn_shape.trim() == "patch_request";
    let bounded_feature_slice = expect_patch && worker_turn_shape.trim() == "bounded_feature_slice";
    let defer_checks_until_patch_exists =
        get_bool(brief, "defer_checks_until_patch_exists").unwrap_or(patch_request);
    let original_required_tests = non_empty_or(
        merged_string_arrays(brief, &["tests", "required_tests"]),
        get_string_array(&original, "tests"),
    );
    let required_tests = if !expect_patch || (patch_request && defer_checks_until_patch_exists) {
        Vec::new()
    } else {
        original_required_tests.clone()
    };
    let checks = merged_string_arrays(
        brief,
        &[
            "checks",
            "must_check",
            "required_checks",
            "acceptance_checks",
        ],
    );
    let avoid = get_string_array(brief, "avoid");
    let mut constraints = get_string_array(&original, "constraints");
    constraints.extend(
        get_string_array(brief, "constraints")
            .into_iter()
            .map(|constraint| format!("Supervisor constraint: {constraint}")),
    );
    constraints.extend(avoid.iter().map(|item| format!("Avoid: {item}")));
    if patch_request {
        constraints
            .push("Do not ask questions; make a reasonable assumption and edit.".to_string());
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
    } else if planning_probe {
        constraints
            .push("Do not edit files; return a compact plan for the supervisor.".to_string());
        constraints.push(
            "Inspect only the focused files or narrowly anchored references needed for the proposal."
                .to_string(),
        );
    } else {
        constraints.push(
            "Treat the original task JSON as primary; the supervisor handoff is supplemental."
                .to_string(),
        );
    }
    constraints.push("Keep stdout compact.".to_string());
    constraints.sort();
    constraints.dedup();

    let original_instructions = get_str(&original, "instructions").unwrap_or("");
    let brief_json = serde_json::to_string_pretty(brief)
        .context("failed to serialize supervisor worker brief")?;
    let title = get_str(&original, "title").unwrap_or("Mixmod task");
    let codex_message = codex_message_to_worker(brief, handoff);
    let supervisor_investigation = supervisor_investigation_to_worker(brief);
    let supervisor_investigation_section = if supervisor_investigation.is_empty() {
        String::new()
    } else {
        format!("\n\nSupervisor investigation:\n{supervisor_investigation}")
    };
    let explicit_completion_gate = get_str(brief, "completion_gate")
        .map(str::trim)
        .filter(|gate| !gate.is_empty());
    let explicit_stop_condition = get_str(brief, "stop_condition")
        .or_else(|| get_str(brief, "worker_stop_condition"))
        .map(str::trim)
        .filter(|condition| !condition.is_empty());
    let acceptance = if planning_probe {
        Vec::new()
    } else if patch_request {
        explicit_completion_gate
            .map(|gate| vec![gate.to_string()])
            .unwrap_or_default()
    } else {
        non_empty_or(checks.clone(), get_string_array(&original, "acceptance"))
    };
    let instructions = if planning_probe {
        planning_probe_instructions(&original, brief, &target_files, &codex_message)
    } else if patch_request {
        patch_request_instructions(
            work_dir,
            &original,
            brief,
            &target_files,
            explicit_stop_condition,
            explicit_completion_gate,
            &codex_message,
        )
    } else if bounded_feature_slice {
        bounded_feature_slice_instructions(
            &original,
            brief,
            &target_files,
            &codex_message,
            &supervisor_investigation_section,
            &brief_json,
        )
    } else {
        format!(
            "Original task instructions:\n{original_instructions}\n\nSupervisor message to worker:\n{codex_message}{supervisor_investigation_section}\n\nSupervisor handoff JSON:\n{brief_json}"
        )
    };

    let worker_task = WorkerBriefTask {
        title: format!("Mixmod handoff: {title}"),
        instructions,
        expect_patch,
        files: target_files,
        tests: required_tests,
        constraints,
        acceptance,
        context: WorkerBriefTaskContext {
            expect_patch,
            worker_brief: brief,
        },
    };
    let path = default_dir.join(WORKER_TASK_JSON);
    write_pretty_json(&path, &worker_task, "worker task")?;
    Ok(path)
}

fn planning_probe_instructions(
    original: &Value,
    brief: &Value,
    target_files: &[String],
    fallback_message: &str,
) -> String {
    let original_instructions = get_str(original, "instructions").unwrap_or("").trim();
    let turn_goal = get_str(brief, "turn_goal")
        .or_else(|| get_str(brief, "objective"))
        .or_else(|| get_str(brief, "message_to_worker"))
        .or_else(|| get_str(brief, "message"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_message)
        .trim();
    let plan_questions = non_empty_or(
        first_non_empty_string_array(brief, &["planning_questions", "exact_edits", "edit_plan"]),
        vec![
            "Identify the next one or two authored-source patch requests.".to_string(),
            "Name the files, symbols, anchors, expected changed-line range, and likely risk for each slice.".to_string(),
        ],
    );
    let evidence = merged_string_arrays(brief, &["evidence", "edit_packet", "source_snippets"]);
    let evidence_note = if evidence.is_empty() {
        String::new()
    } else {
        format!("\nSupervisor-provided clues:\n{}\n", bullet_list(&evidence))
    };
    let file_list = file_list_or_none(target_files);

    format!(
        r#"Noninteractive planning probe. This is a no-patch turn for the supervisor. No user will answer questions.

Original task instructions:
{original_instructions}

Planning goal:
{turn_goal}
{evidence_note}

Inspect only the focused files and narrowly anchored references needed to answer the planning questions. Prefer targeted searches, headers, nearby anchors, or short snippets over full-file reads. Do not read whole generated or very large files unless the planning question cannot be answered otherwise. Do not edit files. Do not run tests. Do not regenerate generated artifacts. Do not inspect Mixmod state or artifact directories. Do not ask the user for more requirements; use the original task and supervisor clues to propose the best next slice.

Planning questions:
{plan_questions}

Focused files:
{file_list}

Final response format:
Recommended next slice: <one sentence>
Files: <comma-separated repo paths>
Anchors: <short symbol or literal anchors>
Expected patch size: <rough changed-line range>
Risks: <one short risk or none>
"#,
        plan_questions = numbered_list(&plan_questions),
    )
}

fn patch_request_instructions(
    work_dir: &Path,
    original: &Value,
    brief: &Value,
    target_files: &[String],
    stop_condition: Option<&str>,
    completion_gate: Option<&str>,
    fallback_message: &str,
) -> String {
    let title = get_str(original, "title").unwrap_or("the task");
    let turn_goal = get_str(brief, "turn_goal")
        .or_else(|| get_str(brief, "objective"))
        .or_else(|| get_str(brief, "message_to_worker"))
        .or_else(|| get_str(brief, "message"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_message)
        .trim();
    let exact_edits =
        first_non_empty_string_array(brief, &["exact_edits", "edit_plan", "implementation_steps"]);

    let mut hard_rules = Vec::new();
    for action in get_string_array(brief, "forbidden_actions") {
        let rule = hard_rule_from_forbidden_action(&action);
        if !hard_rules.contains(&rule) {
            hard_rules.push(rule);
        }
    }

    let file_list = file_list_or_none(target_files);
    let edit_packet = patch_request_edit_packet_from_value(
        work_dir,
        target_files,
        &exact_edits,
        brief,
        &["edit_packet", "source_snippets", "anchors", "evidence"],
    );
    let hard_rules_note = optional_bullet_section("Supervisor worker limits", &hard_rules);
    let stop_condition_note = optional_text_section("Supervisor stop condition", stop_condition);
    let completion_gate_note = optional_text_section("Supervisor completion gate", completion_gate);
    let exact_edits_note =
        optional_numbered_section("Supervisor-provided edit details", &exact_edits);
    let edit_packet_note = if edit_packet == NO_EDIT_PACKET {
        String::new()
    } else {
        format!(
            r#"
Worker edit packet:
{edit_packet}

Use the Worker edit packet before reading whole files. If the packet contains the needed anchor, edit from that context first.
"#
        )
    };

    format!(
        r#"Noninteractive coding task. This is the full instruction. No user will answer questions.

Original task: {title}

Patch request goal: {turn_goal}
{hard_rules_note}{exact_edits_note}
{edit_packet_note}

Relevant files:
{file_list}

Use concrete files from this list. If a listed item is a directory, do not read the whole directory; choose the one file required by the patch request.
Do not read an entire large file before the first edit unless focused anchor searches are not enough to apply the patch request.
Do not expand beyond this first patch request unless the supervisor request requires it.
If a listed file is missing, continue with the remaining request; create a missing file only when the request requires it.
When editing an existing source function, preserve surrounding control flow and indentation. Do not rewrite the whole function. Do not delete or reindent unrelated branches. Make the smallest local edit that satisfies this request.
{stop_condition_note}
{completion_gate_note}

Final response format:
Changed files: <comma-separated list>
"#,
        hard_rules_note = hard_rules_note,
        exact_edits_note = exact_edits_note,
        edit_packet_note = edit_packet_note,
        stop_condition_note = stop_condition_note,
        completion_gate_note = completion_gate_note,
    )
}

fn bounded_feature_slice_instructions(
    original: &Value,
    brief: &Value,
    target_files: &[String],
    fallback_message: &str,
    supervisor_investigation_section: &str,
    brief_json: &str,
) -> String {
    let original_instructions = get_str(original, "instructions").unwrap_or("").trim();
    let turn_goal = get_str(brief, "turn_goal")
        .or_else(|| get_str(brief, "objective"))
        .or_else(|| get_str(brief, "message_to_worker"))
        .or_else(|| get_str(brief, "message"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_message)
        .trim();
    let plan = non_empty_or(
        merged_string_arrays(brief, &["edit_plan", "exact_edits", "implementation_steps"]),
        vec![turn_goal.to_string()],
    );
    let checks = merged_string_arrays(
        brief,
        &[
            "checks",
            "must_check",
            "required_checks",
            "acceptance_checks",
        ],
    );
    let checks = if checks.is_empty() {
        "Run the narrowest compile or focused test check that is available after editing."
            .to_string()
    } else {
        numbered_list(&checks)
    };
    let file_list = file_list_or_none(target_files);

    format!(
        r#"Noninteractive coding task. This is the full instruction. No user will answer questions.

Original task instructions:
{original_instructions}

Supervisor bounded feature goal:
{turn_goal}
{supervisor_investigation_section}

Complete this bounded feature chunk before finalizing. A bounded chunk may include related source, API, serialization/deserialization, and focused test/check work when those edits belong together. Keep the work inside the relevant files and do not continue into unrelated acceptance criteria after this chunk is complete.

Edit plan:
{plan}

Relevant files:
{file_list}

Rules:
- Do not ask questions.
- Do not stop after only reading files.
- Make the repository edits before running broad checks.
- Preserve surrounding control flow and indentation in large functions.
- Do not rewrite unrelated code.
- Do not inspect Mixmod state or artifact directories.

Checks after a patch exists:
{checks}

Supervisor handoff JSON:
{brief_json}
"#,
        plan = numbered_list(&plan),
    )
}

fn supervisor_investigation_to_worker(brief: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(summary) = get_str(brief, "investigation_summary")
        .or_else(|| get_str(brief, "investigation"))
        .or_else(|| get_str(brief, "analysis_summary"))
        .or_else(|| get_str(brief, "root_cause"))
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Summary: {}", summary.trim()));
    }
    append_handoff_list(
        &mut lines,
        "Edit plan",
        &merged_string_arrays(brief, &["edit_plan", "implementation_plan"]),
    );
    append_handoff_list(
        &mut lines,
        "Evidence",
        &merged_string_arrays(brief, &["evidence", "file_evidence", "clues"]),
    );
    append_handoff_list(
        &mut lines,
        "Unknowns",
        &merged_string_arrays(brief, &["unknowns", "assumptions"]),
    );
    lines.join("\n")
}

fn codex_message_to_worker(brief: &Value, handoff: &str) -> String {
    if let Some(message) = get_str(brief, "message_to_worker")
        .or_else(|| get_str(brief, "message"))
        .filter(|message| !message.trim().is_empty())
    {
        return message.trim().to_string();
    }
    if handoff == "as_given" && !brief_has_legacy_guidance(brief) {
        return "Proceed from the original task.".to_string();
    }

    let mut lines = Vec::new();
    if let Some(supplement) = get_str(brief, "supplement")
        .or_else(|| get_str(brief, "objective"))
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(supplement.trim().to_string());
    }
    append_handoff_list(
        &mut lines,
        "Files",
        &first_non_empty_string_array(brief, &["files", "focus_files", "target_files"]),
    );
    append_handoff_list(
        &mut lines,
        "Checks",
        &merged_string_arrays(
            brief,
            &[
                "checks",
                "must_check",
                "required_checks",
                "acceptance_checks",
            ],
        ),
    );
    append_handoff_list(
        &mut lines,
        "Notes",
        &get_string_array(brief, "implementation_steps"),
    );
    append_handoff_list(&mut lines, "Avoid", &get_string_array(brief, "avoid"));
    if let Some(risk) = get_str(brief, "risk").filter(|value| !value.trim().is_empty()) {
        lines.push(format!("Risk: {}", risk.trim()));
    }
    if lines.is_empty() {
        "Proceed from the original task.".to_string()
    } else {
        lines.join("\n")
    }
}

fn brief_has_legacy_guidance(brief: &Value) -> bool {
    get_str(brief, "message_to_worker").is_some()
        || get_str(brief, "message").is_some()
        || get_str(brief, "supplement").is_some()
        || get_str(brief, "objective").is_some()
        || get_str(brief, "worker_turn_shape").is_some()
        || get_str(brief, "turn_goal").is_some()
        || get_str(brief, "stop_condition").is_some()
        || get_str(brief, "worker_stop_condition").is_some()
        || get_str(brief, "completion_gate").is_some()
        || !get_string_array(brief, "files").is_empty()
        || !get_string_array(brief, "checks").is_empty()
        || !get_string_array(brief, "focus_files").is_empty()
        || !get_string_array(brief, "target_files").is_empty()
        || !get_string_array(brief, "implementation_steps").is_empty()
        || !get_string_array(brief, "edit_plan").is_empty()
        || !get_string_array(brief, "exact_edits").is_empty()
        || !get_string_array(brief, "edit_packet").is_empty()
        || !get_string_array(brief, "source_snippets").is_empty()
        || !get_string_array(brief, "anchors").is_empty()
        || !get_string_array(brief, "evidence").is_empty()
        || !get_string_array(brief, "forbidden_actions").is_empty()
        || !get_string_array(brief, "deferred_checks").is_empty()
        || !get_string_array(brief, "acceptance_checks").is_empty()
        || !get_string_array(brief, "required_checks").is_empty()
        || !get_string_array(brief, "required_tests").is_empty()
        || !get_string_array(brief, "tests").is_empty()
        || !get_string_array(brief, "constraints").is_empty()
        || get_str(brief, "risk").is_some()
}
