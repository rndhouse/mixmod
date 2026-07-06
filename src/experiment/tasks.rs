use crate::*;

use serde::Serialize;

#[derive(Serialize)]
struct WorkerBriefTask<'a> {
    title: String,
    instructions: String,
    expect_patch: bool,
    files: Vec<String>,
    tests: Vec<String>,
    constraints: Vec<String>,
    acceptance: Vec<String>,
    context: WorkerBriefTaskContext<'a>,
}

#[derive(Serialize)]
struct WorkerBriefTaskContext<'a> {
    expect_patch: bool,
    worker_brief: &'a Value,
}

#[derive(Serialize)]
struct RevisionTask<'a> {
    title: String,
    instructions: String,
    expect_patch: bool,
    worker_mode: &'a str,
    files: Vec<String>,
    tests: Vec<String>,
    constraints: Vec<String>,
    acceptance: Vec<String>,
    context: RevisionTaskContext<'a>,
}

#[derive(Serialize)]
struct RevisionTaskContext<'a> {
    expect_patch: bool,
    codex_focus_files: &'a [String],
    repo_focus_files: &'a [String],
    patch_decision: &'a str,
    revision: RevisionTaskDetails<'a>,
    mixmod_artifact_refs: &'a [String],
}

#[derive(Serialize)]
struct RevisionTaskDetails<'a> {
    delta_expected: bool,
    message_to_worker: &'a str,
    worker_mode: &'a str,
    patch_decision: &'a str,
    worker_turn_shape: Option<&'a str>,
    turn_goal: Option<&'a str>,
    exact_edits: &'a [String],
    deferred_checks: &'a [String],
    defer_checks_until_patch_exists: Option<bool>,
    completion_gate: Option<&'a str>,
    forbidden_actions: &'a [String],
    focus_files: &'a [String],
    repo_focus_files: &'a [String],
    required_checks: &'a [String],
}

pub(crate) fn write_worker_brief_task(
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
    let small_patch_slice = expect_patch
        && get_str(brief, "worker_turn_shape")
            .is_some_and(|shape| shape.trim() == "small_patch_slice");
    let defer_checks_until_patch_exists =
        get_bool(brief, "defer_checks_until_patch_exists").unwrap_or(small_patch_slice);
    let original_required_tests = non_empty_or(
        merged_string_arrays(brief, &["tests", "required_tests"]),
        get_string_array(&original, "tests"),
    );
    let required_tests = if small_patch_slice && defer_checks_until_patch_exists {
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
    if small_patch_slice {
        constraints
            .push("Do not ask questions; make a reasonable assumption and edit.".to_string());
        constraints.push(
            "Do not run tests in this worker turn; checks are deferred until a patch exists."
                .to_string(),
        );
        constraints.push("Do not finalize until git diff --stat is non-empty.".to_string());
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
    let completion_gate = get_str(brief, "completion_gate")
        .filter(|gate| !gate.trim().is_empty())
        .unwrap_or("git diff --stat must be non-empty");
    let acceptance = if small_patch_slice {
        vec![completion_gate.to_string()]
    } else {
        non_empty_or(checks.clone(), get_string_array(&original, "acceptance"))
    };
    let instructions = if small_patch_slice {
        small_patch_slice_instructions(
            &original,
            brief,
            &target_files,
            completion_gate,
            &codex_message,
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

fn small_patch_slice_instructions(
    original: &Value,
    brief: &Value,
    target_files: &[String],
    completion_gate: &str,
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
    let exact_edits = if exact_edits.is_empty() {
        vec![turn_goal.to_string()]
    } else {
        exact_edits
    };

    let mut hard_rules = vec![
        "Do not ask questions.".to_string(),
        "Do not run tests in this turn.".to_string(),
        "Do not stop after reading files.".to_string(),
        "Do not inspect unrelated behavior outside this slice.".to_string(),
        "Do not final-answer until repository files are modified.".to_string(),
        "If something is ambiguous, make a reasonable assumption and continue editing.".to_string(),
    ];
    for action in get_string_array(brief, "forbidden_actions") {
        let rule = hard_rule_from_forbidden_action(&action);
        if !hard_rules.contains(&rule) {
            hard_rules.push(rule);
        }
    }

    let file_list = if target_files.is_empty() {
        "- none specified".to_string()
    } else {
        target_files
            .iter()
            .map(|file| format!("- {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"Noninteractive coding task. This is the full instruction. No user will answer questions.

Original task: {title}

Your only goal in this turn is to create a non-empty repository patch.

Hard rules:
{hard_rules}

Patch slice goal: {turn_goal}

Make exactly this first small patch:
{exact_edits}

Relevant files:
{file_list}

Use concrete files from this list. If a listed item is a directory, do not read the whole directory; choose the one file required by the exact edits.
Do not expand beyond this first patch slice unless one of the exact edits requires it.
If a listed file is missing, continue with the remaining exact edits; create a missing file only when an exact edit requires it.
Checks are intentionally deferred until after a non-empty patch exists.

After editing, run exactly: git diff --stat
If git diff --stat is empty, you failed; edit files before finalizing.
Completion gate: {completion_gate}

Final response format:
Changed files: <comma-separated list>
Diff non-empty: yes/no
"#,
        hard_rules = bullet_list(&hard_rules),
        exact_edits = numbered_list(&exact_edits),
    )
}

fn hard_rule_from_forbidden_action(action: &str) -> String {
    let action = action.trim().trim_end_matches('.');
    if action.is_empty() {
        return "Do not ask questions.".to_string();
    }
    if action.to_ascii_lowercase().starts_with("do not ") {
        format!("{action}.")
    } else {
        format!("Do not {action}.")
    }
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

pub(crate) fn write_revision_task(
    task_path: &Path,
    default_dir: &Path,
    experiment_name: &str,
    decision: &SupervisorFeedbackTurn,
    revision_index: u64,
) -> Result<PathBuf> {
    let task_value = read_json_file(task_path)?;
    let task_value = agent_visible_task_value(&task_value);
    let work_dir = task_path.parent().unwrap_or_else(|| Path::new("."));
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
    }
    constraints.sort();
    constraints.dedup();
    let original_instructions = get_str(&task_value, "instructions").unwrap_or("Revise the patch.");
    let patch_decision_note = if decision.patch_decision == "revise_previous" {
        "\nPatch checkpoint decision: revise_previous. The supervisor judged the previous candidate patch better than the current revision. Recover the previous candidate using the supervisor message below, then make the requested focused changes. Do not read Mixmod artifacts directly.\n"
    } else if decision.patch_decision == "revise_current" {
        "\nPatch checkpoint decision: revise_current. Continue from the current worktree patch and fix the issues the supervisor identified.\n"
    } else {
        ""
    };
    let instructions = if small_patch_slice {
        small_patch_slice_revision_instructions(
            &task_value,
            decision,
            &focus_files,
            &focus_note,
            completion_gate,
            patch_decision_note,
            defer_checks_until_patch_exists,
        )
    } else if decision.worker_mode == "context_focus" {
        format!(
            "Original task instructions:\n{original_instructions}\n\nThe supervisor requested worker_mode=context_focus.\nThis starts a new worker session on the current worktree.\nTreat this as a fresh focused worker attempt and ignore previous worker reasoning unless it is repeated here.{patch_decision_note}\nSupervisor message to worker:\n{}\n\n{focus_note}\nRequired checks: {:?}\nIf checks cannot run because of local environment problems, make the code/test edit first and report the blocker compactly.",
            decision.hint, decision.required_checks
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

fn small_patch_slice_revision_instructions(
    original: &Value,
    decision: &SupervisorFeedbackTurn,
    focus_files: &[String],
    focus_note: &str,
    completion_gate: &str,
    patch_decision_note: &str,
    defer_checks_until_patch_exists: bool,
) -> String {
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
    let mut exact_edits = decision.revision_handoff.exact_edits.clone();
    if exact_edits.is_empty() {
        exact_edits.push(turn_goal.to_string());
    }
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

Relevant files:
{file_list}

{focus_note}
Use concrete files from the relevant file list. If a listed item is a directory, do not read the whole directory; choose the one file required by the exact edits.
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
    )
}

fn revision_delta_expected(decision: &SupervisorFeedbackTurn) -> bool {
    decision.verdict == "revise"
        || decision.patch_decision == "revise_current"
        || decision.patch_decision == "revise_previous"
}

fn split_worker_focus_files(
    work_dir: &Path,
    default_dir: &Path,
    requested: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut repo_files = Vec::new();
    let mut artifact_refs = Vec::new();
    for raw in requested {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        match classify_worker_focus_file(work_dir, default_dir, trimmed) {
            WorkerFocusFile::Repo(path) => push_unique(&mut repo_files, path),
            WorkerFocusFile::Artifact(path) => push_unique(&mut artifact_refs, path),
        }
    }
    (repo_files, artifact_refs)
}

enum WorkerFocusFile {
    Repo(String),
    Artifact(String),
}

fn classify_worker_focus_file(work_dir: &Path, default_dir: &Path, raw: &str) -> WorkerFocusFile {
    let normalized = raw.trim_start_matches("./").replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(work_dir) {
            let relative = path_to_repo_string(relative);
            if is_artifact_focus_ref(&relative) {
                WorkerFocusFile::Artifact(normalized)
            } else {
                WorkerFocusFile::Repo(relative)
            }
        } else {
            let _ = path.strip_prefix(default_dir);
            WorkerFocusFile::Artifact(normalized)
        }
    } else if normalized.starts_with("../")
        || normalized.contains("/../")
        || normalized.starts_with(".mixmod/")
        || normalized.starts_with(".codex/")
        || is_artifact_focus_ref(&normalized)
    {
        WorkerFocusFile::Artifact(normalized)
    } else {
        WorkerFocusFile::Repo(normalized)
    }
}

fn is_artifact_focus_ref(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(path);
    is_static_mixmod_artifact_name(file_name)
        || file_name == "revision-task.json"
        || (file_name.starts_with("revision-task-") && file_name.ends_with(".json"))
}

fn path_to_repo_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn push_unique(items: &mut Vec<String>, item: String) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn brief_has_legacy_guidance(brief: &Value) -> bool {
    get_str(brief, "message_to_worker").is_some()
        || get_str(brief, "message").is_some()
        || get_str(brief, "supplement").is_some()
        || get_str(brief, "objective").is_some()
        || get_str(brief, "worker_turn_shape").is_some()
        || get_str(brief, "turn_goal").is_some()
        || get_str(brief, "completion_gate").is_some()
        || !get_string_array(brief, "files").is_empty()
        || !get_string_array(brief, "checks").is_empty()
        || !get_string_array(brief, "focus_files").is_empty()
        || !get_string_array(brief, "target_files").is_empty()
        || !get_string_array(brief, "implementation_steps").is_empty()
        || !get_string_array(brief, "exact_edits").is_empty()
        || !get_string_array(brief, "forbidden_actions").is_empty()
        || !get_string_array(brief, "deferred_checks").is_empty()
        || !get_string_array(brief, "acceptance_checks").is_empty()
        || !get_string_array(brief, "required_checks").is_empty()
        || !get_string_array(brief, "required_tests").is_empty()
        || !get_string_array(brief, "tests").is_empty()
        || !get_string_array(brief, "constraints").is_empty()
        || get_str(brief, "risk").is_some()
}

fn append_handoff_list(lines: &mut Vec<String>, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{label}:"));
    lines.extend(items.iter().map(|item| format!("- {item}")));
}

fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn numbered_list(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn non_empty_or<T>(value: Vec<T>, fallback: Vec<T>) -> Vec<T> {
    if value.is_empty() { fallback } else { value }
}
