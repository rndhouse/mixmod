use crate::harness::AgentRequest;

use super::config::OpenCodeModelSelection;

pub(super) fn render_opencode_arg(
    arg: &str,
    request: &AgentRequest,
    selection: &OpenCodeModelSelection,
) -> String {
    let resume_session_id = request.resume_session_id.as_deref().unwrap_or_default();
    arg.replace("{instruction}", &request.instruction)
        .replace(
            "{instruction_file}",
            &request.instruction_path.to_string_lossy(),
        )
        .replace("{task_file}", &request.task_path.to_string_lossy())
        .replace("{mode}", &request.mode.to_string())
        .replace("{out_dir}", &request.out_dir.to_string_lossy())
        .replace("{model}", &selection.model)
        .replace("{provider}", &selection.provider)
        .replace("{model_arg}", &selection.model_arg)
        .replace("{session_id}", &request.session_id)
        .replace("{resume_session_id}", resume_session_id)
}

pub(super) fn redact_opencode_arg(
    arg: &str,
    request: &AgentRequest,
    selection: &OpenCodeModelSelection,
) -> String {
    if arg.contains("{instruction}") {
        arg.replace(
            "{instruction}",
            &format!("<instruction:{} bytes>", request.instruction.len()),
        )
        .replace("{model}", &selection.model)
        .replace("{provider}", &selection.provider)
        .replace("{model_arg}", &selection.model_arg)
        .replace("{session_id}", &request.session_id)
        .replace(
            "{resume_session_id}",
            request.resume_session_id.as_deref().unwrap_or_default(),
        )
    } else {
        render_opencode_arg(arg, request, selection)
    }
}

pub(super) fn redact_runtime_opencode_arg(arg: &str, request: &AgentRequest) -> String {
    if arg == request.instruction {
        format!("<instruction:{} bytes>", request.instruction.len())
    } else if arg == request.instruction_path.to_string_lossy().as_ref() {
        "<instruction_file>".to_string()
    } else {
        arg.to_string()
    }
}

pub(crate) fn prepare_opencode_args(
    mut args: Vec<String>,
    resume_session_id: Option<&str>,
) -> Vec<String> {
    let Some(resume_session_id) = resume_session_id else {
        return args;
    };
    args = remove_opencode_title_args(args);
    if has_opencode_session_arg(&args) {
        return args;
    }
    let insert_at = if args.first().map(|arg| arg == "run").unwrap_or(false) {
        1
    } else {
        0
    };
    args.insert(insert_at, "--session".to_string());
    args.insert(insert_at + 1, resume_session_id.to_string());
    args
}

pub(crate) fn prepare_opencode_control_args(
    base_args: &[String],
    request: &AgentRequest,
    resume_session_id: Option<&str>,
    session_label: &str,
    message: &str,
) -> Vec<String> {
    let mut args = remove_opencode_session_args(remove_opencode_title_args(base_args.to_vec()));
    let instruction_file = request.instruction_path.to_string_lossy();
    args.retain(|arg| arg != &request.instruction && arg != instruction_file.as_ref());
    let insert_at = if args.first().map(|arg| arg == "run").unwrap_or(false) {
        1
    } else {
        0
    };
    if let Some(session_id) = resume_session_id {
        args.insert(insert_at, "--session".to_string());
        args.insert(insert_at + 1, session_id.to_string());
    } else {
        args.insert(insert_at, "--title".to_string());
        args.insert(insert_at + 1, session_label.to_string());
    }
    args.push(message.to_string());
    args
}

fn remove_opencode_title_args(args: Vec<String>) -> Vec<String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--title" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--title=") {
            continue;
        }
        filtered.push(arg);
    }
    filtered
}

fn remove_opencode_session_args(args: Vec<String>) -> Vec<String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--session" || arg == "-s" {
            skip_next = true;
            continue;
        }
        if arg == "--continue" || arg == "-c" || arg.starts_with("--session=") {
            continue;
        }
        filtered.push(arg);
    }
    filtered
}

fn has_opencode_session_arg(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "--session" || arg == "-s" || arg.starts_with("--session="))
}
