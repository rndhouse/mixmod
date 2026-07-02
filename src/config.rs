use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_FRONTIER_MODEL, DEFAULT_FRONTIER_REASONING_EFFORT, DEFAULT_OPENCODE_MODEL,
    DEFAULT_OPENCODE_OLLAMA_MODEL, DEFAULT_OPENCODE_PROVIDER,
};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MixmodConfig {
    pub opencode: OpenCodeConfig,
    pub frontier: FrontierConfig,
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
