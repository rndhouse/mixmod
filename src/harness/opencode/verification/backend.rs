use std::env;

use crate::DEFAULT_OPENCODE_LOCAL_MODEL;
use crate::harness::opencode::config::OpenCodeModelSelection;

pub(super) fn effective_backend_command(configured_command: &str) -> String {
    let configured = configured_command.trim();
    let base_url = env::var("MIXMOD_OPENCODE_BASE_URL").ok();
    effective_backend_command_for_base_url(configured, base_url.as_deref())
}

pub(crate) fn effective_backend_command_for_base_url(
    configured_command: &str,
    base_url: Option<&str>,
) -> String {
    if let Some(base_url) = base_url
        && !base_url.trim().is_empty()
        && is_default_backend_command(configured_command)
    {
        return format!(
            "curl --noproxy '*' -fsS {}",
            shell_quote(&backend_models_url(base_url.trim()))
        );
    }
    configured_command.to_string()
}

fn is_default_backend_command(command: &str) -> bool {
    command == "curl -fsS http://127.0.0.1:8080/v1/models"
}

fn backend_models_url(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&()*+,;=%".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

pub(super) fn gpu_activity_observed(before: Option<&str>, during: Option<&str>) -> bool {
    let Some(during) = during else {
        return false;
    };
    let before_memory = before.and_then(parse_gpu_memory_mib).unwrap_or(0);
    let during_memory = parse_gpu_memory_mib(during).unwrap_or(0);
    let during_util = parse_gpu_util_percent(during).unwrap_or(0);
    let lower = during.to_ascii_lowercase();
    lower.contains("ollama")
        || lower.contains("vllm")
        || lower.contains("llama")
        || during_util > 0
        || during_memory.saturating_sub(before_memory) > 500
}

fn parse_gpu_memory_mib(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some((left, right)) = line.split_once("MiB /") {
            let value = left.split_whitespace().last()?.trim().parse::<u64>().ok();
            if value.is_some() && right.contains("MiB") {
                return value;
            }
        }
    }
    None
}

fn parse_gpu_util_percent(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some((left, _)) = line.split_once("%")
            && (line.contains("Default") || line.contains("MiB"))
            && let Some(value) = left.split_whitespace().last()
            && let Ok(value) = value.parse::<u64>()
        {
            return Some(value);
        }
    }
    None
}

pub(super) fn backend_activity_observed(
    text: Option<&str>,
    selection: &OpenCodeModelSelection,
) -> bool {
    let Some(text) = text else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    let model = selection.model.to_ascii_lowercase();
    let model_arg = selection.model_arg.to_ascii_lowercase();
    lower.contains(&model)
        || lower.contains(&model_arg)
        || lower.contains(DEFAULT_OPENCODE_LOCAL_MODEL)
}
