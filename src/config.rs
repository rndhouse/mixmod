use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_FRONTIER_MODEL, DEFAULT_FRONTIER_REASONING_EFFORT, DEFAULT_OPENCODE_MODEL,
    DEFAULT_OPENCODE_OLLAMA_MODEL, DEFAULT_OPENCODE_PROVIDER,
};

const REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MixmodConfig {
    pub opencode: OpenCodeConfig,
    pub frontier: FrontierConfig,
}

/// Per-run model choices supplied by CLI flags.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelOverrides {
    /// Codex supervisor model, optionally suffixed with a reasoning effort.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_model: Option<String>,
    /// OpenCode worker model, optionally prefixed with a provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_model: Option<String>,
}

impl ModelOverrides {
    /// Build model overrides from optional CLI flag values.
    pub fn new(supervisor_model: Option<String>, worker_model: Option<String>) -> Self {
        Self {
            supervisor_model,
            worker_model,
        }
    }

    /// Apply the model overrides to a loaded Mixmod configuration.
    pub fn apply_to_config(&self, config: &mut MixmodConfig) -> Result<()> {
        if let Some(value) = self.supervisor_model.as_deref() {
            let (model, reasoning_effort) =
                parse_supervisor_model(value, &config.frontier.reasoning_effort)?;
            config.frontier.model = model;
            config.frontier.reasoning_effort = reasoning_effort;
        }
        if let Some(value) = self.worker_model.as_deref() {
            apply_worker_model_override(&mut config.opencode, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct OpenCodeConfig {
    pub command: String,
    pub args: Vec<String>,
    pub provider: String,
    pub model: String,
    pub require_local: bool,
    pub heartbeat_seconds: u64,
    pub worker_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub local_verification: LocalVerificationConfig,
    pub model_aliases: std::collections::BTreeMap<String, Vec<String>>,
    pub local_providers: Vec<String>,
}

impl Default for OpenCodeConfig {
    fn default() -> Self {
        let mut model_aliases = std::collections::BTreeMap::new();
        model_aliases.insert(
            DEFAULT_OPENCODE_MODEL.to_string(),
            vec![
                DEFAULT_OPENCODE_MODEL.to_string(),
                DEFAULT_OPENCODE_OLLAMA_MODEL.to_string(),
                "qwen/qwen3.6-27b".to_string(),
                "ollama/qwen3.6:27b".to_string(),
                "local-ollama/qwen3.6:27b".to_string(),
                format!("{DEFAULT_OPENCODE_PROVIDER}/qwen3.6:27b"),
            ],
        );
        Self {
            command: "opencode".to_string(),
            args: vec![
                "run".to_string(),
                "--dangerously-skip-permissions".to_string(),
                "--model".to_string(),
                "{model_arg}".to_string(),
                "--title".to_string(),
                "{session_id}".to_string(),
                "{instruction}".to_string(),
            ],
            provider: DEFAULT_OPENCODE_PROVIDER.to_string(),
            model: DEFAULT_OPENCODE_MODEL.to_string(),
            require_local: true,
            heartbeat_seconds: 10,
            worker_timeout_seconds: 600,
            idle_timeout_seconds: 300,
            local_verification: LocalVerificationConfig::default(),
            model_aliases,
            local_providers: vec![
                "local".to_string(),
                DEFAULT_OPENCODE_PROVIDER.to_string(),
                "local-ollama".to_string(),
                "ollama".to_string(),
                "lmstudio".to_string(),
                "llama.cpp".to_string(),
                "vllm".to_string(),
                "localhost".to_string(),
            ],
        }
    }
}

fn parse_supervisor_model(value: &str, default_reasoning_effort: &str) -> Result<(String, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("--supervisor-model must not be empty");
    }
    if let Some((model, effort)) = trimmed.rsplit_once(':') {
        let effort = effort.trim().to_ascii_lowercase();
        if !REASONING_EFFORTS.contains(&effort.as_str()) {
            bail!(
                "unsupported supervisor reasoning effort `{}`; expected one of {}",
                effort,
                REASONING_EFFORTS.join(", ")
            );
        }
        let model = model.trim();
        if model.is_empty() {
            bail!("--supervisor-model must include a model before the reasoning effort");
        }
        return Ok((model.to_string(), effort));
    }
    Ok((
        trimmed.to_string(),
        default_reasoning_effort.trim().to_ascii_lowercase(),
    ))
}

fn apply_worker_model_override(config: &mut OpenCodeConfig, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("--worker-model must not be empty");
    }
    let (provider, model) = match trimmed.split_once('/') {
        Some((provider, model)) => {
            let provider = provider.trim();
            let model = model.trim();
            if provider.is_empty() || model.is_empty() {
                bail!("--worker-model provider/model values must include both parts");
            }
            (Some(provider.to_string()), model.to_string())
        }
        None => (None, trimmed.to_string()),
    };
    if let Some(provider) = provider {
        if !config.local_providers.iter().any(|item| item == &provider) {
            config.local_providers.push(provider.clone());
        }
        config.provider = provider;
    }
    let aliases = config.model_aliases.entry(model.clone()).or_default();
    for alias in [model.as_str(), trimmed] {
        if !aliases.iter().any(|existing| existing == alias) {
            aliases.push(alias.to_string());
        }
    }
    config.model = model;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LocalVerificationConfig {
    pub enabled: bool,
    pub gpu_command: String,
    pub backend_command: String,
}

impl Default for LocalVerificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gpu_command: "nvidia-smi".to_string(),
            backend_command: "ollama ps".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct FrontierConfig {
    pub model: String,
    pub reasoning_effort: String,
}

impl Default for FrontierConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_FRONTIER_MODEL.to_string(),
            reasoning_effort: DEFAULT_FRONTIER_REASONING_EFFORT.to_string(),
        }
    }
}
