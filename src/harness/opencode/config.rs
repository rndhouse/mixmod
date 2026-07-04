use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use crate::{OpenCodeConfig, get_str, state_layout};

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeModelSelection {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) model_arg: String,
    pub(crate) require_local: bool,
}

pub(super) fn resolve_opencode_model(
    command: &str,
    root: &Path,
    config: &OpenCodeConfig,
    require_local_override: bool,
) -> Result<OpenCodeModelSelection> {
    let require_local = require_local_override || config.require_local;
    let models = opencode_command(command, root)
        .arg("models")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run `{command} models`"))?;
    if !models.status.success() {
        bail!(
            "`{command} models` failed: {}",
            String::from_utf8_lossy(&models.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&models.stdout);
    let aliases = model_aliases(config);
    let configured_model = config.model.as_str();
    let selected = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find(|line| model_line_matches(line, &config.provider, configured_model, &aliases))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "configured OpenCode model `{}` with aliases {:?} was not found in `{command} models`; update the Mixmod worker model or generated OpenCode config for this project",
                configured_model,
                aliases
            )
        })?;
    let (provider, model) = selected
        .split_once('/')
        .map(|(provider, model)| (provider.to_string(), model.to_string()))
        .unwrap_or_else(|| (config.provider.clone(), selected.clone()));

    if require_local {
        reject_cloud_provider(&provider)?;
        if !is_allowed_local_provider(&provider, config) {
            bail!(
                "OpenCode provider `{provider}` is not configured as local under --require-local"
            );
        }
        if !aliases
            .iter()
            .any(|alias| selected.contains(alias) || model == *alias)
        {
            bail!("selected model `{selected}` does not match configured local Qwen 3.6 aliases");
        }
    }

    Ok(OpenCodeModelSelection {
        provider,
        model,
        model_arg: selected,
        require_local,
    })
}

fn model_aliases(config: &OpenCodeConfig) -> Vec<String> {
    let mut aliases = vec![config.model.clone()];
    if let Some(extra) = config.model_aliases.get(&config.model) {
        aliases.extend(extra.clone());
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn model_line_matches(
    line: &str,
    provider: &str,
    configured_model: &str,
    aliases: &[String],
) -> bool {
    if line == configured_model || line.ends_with(&format!("/{configured_model}")) {
        return true;
    }
    if provider != "local" && !line.starts_with(&format!("{provider}/")) {
        return false;
    }
    aliases
        .iter()
        .any(|alias| line == alias || line.ends_with(&format!("/{alias}")) || line.contains(alias))
}

fn reject_cloud_provider(provider: &str) -> Result<()> {
    let cloud = [
        "openai",
        "anthropic",
        "gemini",
        "openrouter",
        "xai",
        "groq",
        "copilot",
        "opencode-hosted",
        "azure",
        "bedrock",
    ];
    if cloud.iter().any(|item| provider.contains(item)) {
        bail!("cloud OpenCode provider `{provider}` is rejected under --require-local");
    }
    Ok(())
}

pub(super) fn is_allowed_local_provider(provider: &str, config: &OpenCodeConfig) -> bool {
    config.local_providers.iter().any(|allowed| {
        provider == allowed || provider.contains(allowed) || allowed.contains(provider)
    })
}

pub(super) fn resolve_opencode_session_id(
    command: &str,
    work_dir: &Path,
    title: &str,
) -> Result<Option<String>> {
    let sql = format!(
        "select id from session where title = '{}' and directory = '{}' order by time_updated desc limit 1;",
        sql_string_literal_content(title),
        sql_string_literal_content(&work_dir.to_string_lossy())
    );
    let output = Command::new(command)
        .env("OPENCODE_CONFIG", opencode_config_path(work_dir))
        .args(["db", &sql, "--format", "json"])
        .output()
        .with_context(|| format!("failed to query OpenCode sessions with `{command} db`"))?;
    if !output.status.success() {
        bail!(
            "`{command} db` failed while resolving session id: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .with_context(|| "failed to parse OpenCode session query JSON")?;
    if let Some(rows) = value.as_array() {
        return Ok(rows
            .first()
            .and_then(|row| get_str(row, "id"))
            .map(ToOwned::to_owned));
    }
    if let Some(rows) = value.get("rows").and_then(Value::as_array) {
        return Ok(rows
            .first()
            .and_then(|row| get_str(row, "id"))
            .map(ToOwned::to_owned));
    }
    Ok(None)
}

pub(crate) fn opencode_config_path(root: &Path) -> PathBuf {
    state_layout(root).opencode_config()
}

pub(super) fn opencode_command(command: &str, root: &Path) -> Command {
    let mut process = Command::new(command);
    process.env("OPENCODE_CONFIG", opencode_config_path(root));
    process
}

fn sql_string_literal_content(value: &str) -> String {
    value.replace('\'', "''")
}
