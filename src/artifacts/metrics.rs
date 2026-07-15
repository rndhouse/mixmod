use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::receipt::Receipt;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DefaultStrategyMetrics {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
    pub final_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_receipt: Option<Receipt>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug)]
pub struct ExperimentReportInputs {
    pub codex_metrics: Value,
    pub default_metrics: Value,
    pub default_source: String,
    pub default_metrics_path: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct PatchStats {
    pub files: Vec<String>,
    pub changed_line_count: usize,
    pub added_lines: usize,
    pub removed_lines: usize,
}
