use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::{OpenCodeConfig, atomic_write, get_str, is_cloud_opencode_provider, state_layout};

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
    ensure_selected_local_model_in_opencode_config(root, config)?;
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
            bail!("selected model `{selected}` does not match configured local model aliases");
        }
    }

    Ok(OpenCodeModelSelection {
        provider,
        model,
        model_arg: selected,
        require_local,
    })
}

fn ensure_selected_local_model_in_opencode_config(
    root: &Path,
    config: &OpenCodeConfig,
) -> Result<()> {
    if is_cloud_opencode_provider(&config.provider) {
        return Ok(());
    }
    if config.model.trim().is_empty() {
        return Ok(());
    }
    let path = opencode_config_path(root);
    if !path.exists() {
        return Ok(());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut value: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(providers) = value.get_mut("provider").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let Some(provider) = providers.get_mut(&config.provider) else {
        return Ok(());
    };
    let Some(provider) = provider.as_object_mut() else {
        return Ok(());
    };
    let models = provider
        .entry("models".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(models) = models.as_object_mut() else {
        return Ok(());
    };
    if models.contains_key(&config.model) {
        return Ok(());
    }
    models.insert(
        config.model.clone(),
        json!({
            "name": local_model_display_name(&config.model),
        }),
    );
    let bytes =
        serde_json::to_vec_pretty(&value).context("failed to serialize updated OpenCode config")?;
    atomic_write(&path, &bytes).with_context(|| format!("failed to update {}", path.display()))?;
    Ok(())
}

fn local_model_display_name(model: &str) -> String {
    format!("{model} (local)")
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
    if is_cloud_opencode_provider(provider) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MixmodConfig, ModelOverrides};
    use tempfile::TempDir;

    fn fake_opencode_with_models(root: &Path, models: &str) -> PathBuf {
        let command = root.join("fake-opencode.sh");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"models\" ]; then\n  printf '%s\\n' '{}'\n  exit 0\nfi\nexit 1\n",
            models
        );
        std::fs::write(&command, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&command).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&command, perms).unwrap();
        }
        command
    }

    #[test]
    fn local_worker_override_is_added_to_opencode_config_before_lookup() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let opencode_config = opencode_config_path(root);
        std::fs::create_dir_all(opencode_config.parent().unwrap()).unwrap();
        std::fs::write(
            &opencode_config,
            serde_json::to_string_pretty(&json!({
                "$schema": "https://opencode.ai/config.json",
                "model": "llama.cpp/qwen/qwen3.6-27b",
                "provider": {
                    "llama.cpp": {
                        "name": "llama.cpp (Mixmod local)",
                        "npm": "@ai-sdk/openai-compatible",
                        "options": {
                            "baseURL": "http://127.0.0.1:8080/v1"
                        },
                        "models": {
                            "qwen/qwen3.6-27b": {
                                "name": "Qwen 3.6 27B (llama.cpp)"
                            }
                        }
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let command = root.join("fake-opencode.sh");
        let script = r#"#!/bin/sh
if [ "$1" = "models" ]; then
  if grep -q 'glm-4.7-flash:Q4_K_M' "$OPENCODE_CONFIG"; then
    echo 'llama.cpp/glm-4.7-flash:Q4_K_M'
  else
    echo 'llama.cpp/qwen/qwen3.6-27b'
  fi
  exit 0
fi
exit 1
"#;
        std::fs::write(&command, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&command).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&command, perms).unwrap();
        }
        let mut config = MixmodConfig::default();
        ModelOverrides::new(None, Some("llama.cpp/glm-4.7-flash:Q4_K_M".to_string()))
            .apply_to_config(&mut config)
            .unwrap();

        let selection =
            resolve_opencode_model(command.to_str().unwrap(), root, &config.opencode, false)
                .unwrap();

        assert_eq!(selection.provider, "llama.cpp");
        assert_eq!(selection.model, "glm-4.7-flash:Q4_K_M");
        assert_eq!(selection.model_arg, "llama.cpp/glm-4.7-flash:Q4_K_M");
        assert!(
            std::fs::read_to_string(opencode_config)
                .unwrap()
                .contains("glm-4.7-flash:Q4_K_M")
        );
    }

    #[test]
    fn openrouter_worker_resolves_when_local_requirement_is_disabled() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let command = fake_opencode_with_models(root, "openrouter/qwen/qwen3.6-flash");
        let mut config = MixmodConfig::default();
        ModelOverrides::new(None, Some("openrouter/qwen/qwen3.6-flash".to_string()))
            .apply_to_config(&mut config)
            .unwrap();

        let selection =
            resolve_opencode_model(command.to_str().unwrap(), root, &config.opencode, false)
                .unwrap();

        assert_eq!(selection.provider, "openrouter");
        assert_eq!(selection.model, "qwen/qwen3.6-flash");
        assert_eq!(selection.model_arg, "openrouter/qwen/qwen3.6-flash");
        assert!(!selection.require_local);
    }

    #[test]
    fn openrouter_worker_is_rejected_when_local_requirement_is_explicit() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let command = fake_opencode_with_models(root, "openrouter/qwen/qwen3.6-flash");
        let mut config = MixmodConfig::default();
        ModelOverrides::new(None, Some("openrouter/qwen/qwen3.6-flash".to_string()))
            .apply_to_config(&mut config)
            .unwrap();

        let error = resolve_opencode_model(command.to_str().unwrap(), root, &config.opencode, true)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("cloud OpenCode provider `openrouter` is rejected")
        );
    }
}
