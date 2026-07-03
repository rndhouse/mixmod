use anyhow::{Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_FRONTIER_MODEL, DEFAULT_FRONTIER_REASONING_EFFORT, DEFAULT_OPENCODE_MODEL,
    DEFAULT_OPENCODE_OLLAMA_MODEL, DEFAULT_OPENCODE_PROVIDER,
};

const REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MixmodConfig {
    pub worker: WorkerConfig,
    pub opencode: OpenCodeConfig,
    pub codex_worker: FrontierConfig,
    pub frontier: FrontierConfig,
}

/// Worker backend selected for repository-editing agent turns.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
pub enum WorkerBackend {
    /// Use OpenCode as the worker harness.
    #[serde(rename = "opencode")]
    #[value(name = "opencode")]
    OpenCode,
    /// Use Codex app-server as the worker harness.
    #[serde(rename = "codex")]
    #[value(name = "codex")]
    Codex,
}

impl WorkerBackend {
    /// Stable configuration and CLI label for this backend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::Codex => "codex",
        }
    }
}

impl Default for WorkerBackend {
    fn default() -> Self {
        Self::OpenCode
    }
}

/// Backend selection for worker turns.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkerConfig {
    pub backend: WorkerBackend,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            backend: WorkerBackend::OpenCode,
        }
    }
}

/// Per-run model choices supplied by CLI flags.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelOverrides {
    /// Codex supervisor model, optionally suffixed with a reasoning effort.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_model: Option<String>,
    /// Worker model override, interpreted by the selected worker backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_model: Option<String>,
    /// Worker backend override for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_backend: Option<WorkerBackend>,
}

impl ModelOverrides {
    /// Build model overrides from optional CLI flag values.
    pub fn new(supervisor_model: Option<String>, worker_model: Option<String>) -> Self {
        Self {
            supervisor_model,
            worker_model,
            worker_backend: None,
        }
    }

    /// Add a worker backend override to this set of per-run choices.
    pub fn with_worker_backend(mut self, worker_backend: Option<WorkerBackend>) -> Self {
        self.worker_backend = worker_backend;
        self
    }

    /// Apply the model overrides to a loaded Mixmod configuration.
    pub fn apply_to_config(&self, config: &mut MixmodConfig) -> Result<()> {
        if let Some(worker_backend) = self.worker_backend {
            config.worker.backend = worker_backend;
        }
        if let Some(value) = self.supervisor_model.as_deref() {
            let (model, reasoning_effort) =
                parse_supervisor_model(value, &config.frontier.reasoning_effort)?;
            config.frontier.model = model;
            config.frontier.reasoning_effort = reasoning_effort;
        }
        if let Some(value) = self.worker_model.as_deref() {
            match config.worker.backend {
                WorkerBackend::OpenCode => {
                    apply_worker_model_override(&mut config.opencode, value)?
                }
                WorkerBackend::Codex => {
                    let (model, reasoning_effort) =
                        parse_supervisor_model(value, &config.codex_worker.reasoning_effort)?;
                    config.codex_worker.model = model;
                    config.codex_worker.reasoning_effort = reasoning_effort;
                }
            }
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
