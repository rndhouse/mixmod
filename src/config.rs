use anyhow::{Result, bail};
use clap::ValueEnum;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_OPENCODE_LOCAL_MODEL, DEFAULT_OPENCODE_MODEL, DEFAULT_OPENCODE_PROVIDER,
    DEFAULT_SUPERVISOR_MODEL, DEFAULT_SUPERVISOR_REASONING_EFFORT, MIXMOD_OPENCODE_AGENT,
    WorkerModelProfile, WorkerSupervisorGuidance, default_worker_model_profiles,
};

const REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];
const CLOUD_OPENCODE_PROVIDER_MARKERS: &[&str] = &[
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

/// Return whether an OpenCode provider name identifies a cloud inference
/// backend.
pub(crate) fn is_cloud_opencode_provider(provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    CLOUD_OPENCODE_PROVIDER_MARKERS
        .iter()
        .any(|marker| provider.contains(marker))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct MixmodConfig {
    /// Default strategy behavior.
    pub strategy: StrategyConfig,
    /// Worker backend selection.
    pub worker: WorkerConfig,
    /// OpenCode worker configuration.
    pub opencode: OpenCodeConfig,
    /// Codex worker configuration.
    pub codex_worker: SupervisorConfig,
    /// Supervisor model configuration.
    pub supervisor: SupervisorConfig,
    /// Historical guidance profiles keyed by worker model.
    #[serde(default = "default_worker_model_profiles")]
    pub worker_model_profiles: Vec<WorkerModelProfile>,
}

impl Default for MixmodConfig {
    fn default() -> Self {
        Self {
            strategy: StrategyConfig::default(),
            worker: WorkerConfig::default(),
            opencode: OpenCodeConfig::default(),
            codex_worker: SupervisorConfig::default(),
            supervisor: SupervisorConfig::default(),
            worker_model_profiles: default_worker_model_profiles(),
        }
    }
}

impl MixmodConfig {
    /// Return supervisor-only guidance for the currently selected worker.
    pub(crate) fn worker_supervisor_guidance(&self) -> WorkerSupervisorGuidance {
        match self.worker.backend {
            WorkerBackend::OpenCode => self
                .worker_model_profiles
                .iter()
                .find(|profile| profile.matches_opencode_worker(&self.opencode))
                .map(|profile| WorkerSupervisorGuidance {
                    model: profile.model.clone(),
                    guidance: profile.supervisor_guidance.clone(),
                })
                .unwrap_or_default(),
            WorkerBackend::Codex => WorkerSupervisorGuidance::default(),
        }
    }
}

/// How much the supervisor should investigate before briefing the worker.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SupervisorInitMode {
    /// Keep the initial supervisor handoff compact.
    #[default]
    #[value(name = "compact")]
    Compact,
    /// Let the supervisor inspect files read-only and pass focused findings to the worker.
    #[value(name = "investigate")]
    Investigate,
}

impl SupervisorInitMode {
    /// Stable configuration and CLI label for this mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Investigate => "investigate",
        }
    }
}

/// Strategy-level defaults for supervisor/worker orchestration.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct StrategyConfig {
    /// Initial supervisor briefing style.
    pub supervisor_init: SupervisorInitMode,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            supervisor_init: SupervisorInitMode::Compact,
        }
    }
}

/// Worker backend selected for repository-editing agent turns.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
pub enum WorkerBackend {
    /// Use OpenCode as the worker harness.
    #[default]
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
    /// Supervisor model, optionally suffixed with a reasoning effort.
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
                parse_supervisor_model(value, &config.supervisor.reasoning_effort)?;
            config.supervisor.model = model;
            config.supervisor.reasoning_effort = reasoning_effort;
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
    pub model_aliases: BTreeMap<String, Vec<String>>,
    pub local_providers: Vec<String>,
}

impl Default for OpenCodeConfig {
    fn default() -> Self {
        let mut model_aliases = BTreeMap::new();
        model_aliases.insert(
            DEFAULT_OPENCODE_MODEL.to_string(),
            vec![
                DEFAULT_OPENCODE_MODEL.to_string(),
                DEFAULT_OPENCODE_LOCAL_MODEL.to_string(),
                "qwen/qwen3.6-27b".to_string(),
                format!("{DEFAULT_OPENCODE_PROVIDER}/{DEFAULT_OPENCODE_LOCAL_MODEL}"),
            ],
        );
        Self {
            command: "opencode".to_string(),
            args: vec![
                "run".to_string(),
                "--agent".to_string(),
                MIXMOD_OPENCODE_AGENT.to_string(),
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
                "lmstudio".to_string(),
                "llama.cpp".to_string(),
                "vllm".to_string(),
                "localhost".to_string(),
            ],
        }
    }
}

impl OpenCodeConfig {
    pub(crate) fn selected_model_identifiers(&self) -> Vec<String> {
        let mut identifiers = vec![
            self.model.clone(),
            format!("{}/{}", self.provider, self.model),
        ];
        if let Some(aliases) = self.model_aliases.get(&self.model) {
            identifiers.extend(aliases.iter().cloned());
        }
        identifiers
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
        if is_cloud_opencode_provider(&provider) {
            config.require_local = false;
            config.local_verification.enabled = false;
        } else if !config.local_providers.iter().any(|item| item == &provider) {
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
            backend_command: "curl -fsS http://127.0.0.1:8080/v1/models".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SupervisorConfig {
    pub model: String,
    pub reasoning_effort: String,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_SUPERVISOR_MODEL.to_string(),
            reasoning_effort: DEFAULT_SUPERVISOR_REASONING_EFFORT.to_string(),
        }
    }
}
