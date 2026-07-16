use serde_json::Value;

use crate::{get_bool, get_str, get_u64};

/// Aggregated metrics collected across worker turns in the default strategy.
pub(crate) struct WorkerMetricsSummary {
    pub(crate) local_stdout_bytes: u64,
    pub(crate) local_stderr_bytes: u64,
    pub(crate) local_reasoning_trace_bytes: u64,
    pub(crate) local_reasoning_trace_event_count: u64,
    pub(crate) local_tool_events_bytes: u64,
    pub(crate) local_tool_event_count: u64,
    pub(crate) worker_input_tokens: u64,
    pub(crate) worker_cached_input_tokens: u64,
    pub(crate) worker_cache_write_tokens: u64,
    pub(crate) worker_output_tokens: u64,
    pub(crate) worker_reasoning_tokens: u64,
    pub(crate) worker_total_tokens: u64,
    pub(crate) worker_reported_cost_usd: f64,
    pub(crate) worker_token_step_count: u64,
    pub(crate) worker_token_usage_comparable: bool,
    pub(crate) opencode_session_ids: Vec<String>,
    pub(crate) opencode_session_labels: Vec<String>,
    pub(crate) worker_session_reuse_count: u64,
    pub(crate) supervisor_control_count: u64,
    pub(crate) supervisor_control_actions: Vec<String>,
    pub(crate) supervisor_control_risks: Vec<String>,
    pub(crate) supervisor_control_interrupts: u64,
    pub(crate) local_inference_verified: bool,
    pub(crate) gpu_activity_observed: bool,
    pub(crate) backend_activity_observed: bool,
}

impl WorkerMetricsSummary {
    /// Build an aggregate summary from per-worker-turn metrics JSON values.
    pub(crate) fn from_metrics(worker_metrics: &[Value]) -> Self {
        Self {
            local_stdout_bytes: sum_u64(worker_metrics, "stdout_bytes"),
            local_stderr_bytes: sum_u64(worker_metrics, "stderr_bytes"),
            local_reasoning_trace_bytes: sum_u64(worker_metrics, "reasoning_trace_bytes"),
            local_reasoning_trace_event_count: sum_u64(
                worker_metrics,
                "reasoning_trace_event_count",
            ),
            local_tool_events_bytes: sum_u64(worker_metrics, "tool_events_bytes"),
            local_tool_event_count: sum_u64(worker_metrics, "tool_event_count"),
            worker_input_tokens: sum_u64(worker_metrics, "worker_input_tokens"),
            worker_cached_input_tokens: sum_u64(worker_metrics, "worker_cached_input_tokens"),
            worker_cache_write_tokens: sum_u64(worker_metrics, "worker_cache_write_tokens"),
            worker_output_tokens: sum_u64(worker_metrics, "worker_output_tokens"),
            worker_reasoning_tokens: sum_u64(worker_metrics, "worker_reasoning_tokens"),
            worker_total_tokens: sum_u64(worker_metrics, "worker_total_tokens"),
            worker_reported_cost_usd: sum_f64(worker_metrics, "worker_reported_cost_usd"),
            worker_token_step_count: sum_u64(worker_metrics, "worker_token_step_count"),
            worker_token_usage_comparable: !worker_metrics.is_empty()
                && worker_metrics.iter().all(|metrics| {
                    get_bool(metrics, "worker_token_usage_comparable").unwrap_or(false)
                }),
            opencode_session_ids: collect_strings(worker_metrics, "opencode_session_id"),
            opencode_session_labels: collect_strings(worker_metrics, "opencode_session_label"),
            worker_session_reuse_count: worker_metrics
                .iter()
                .filter(|metrics| get_bool(metrics, "worker_session_reused").unwrap_or(false))
                .count() as u64,
            supervisor_control_count: worker_metrics
                .iter()
                .map(supervisor_control_event_count)
                .sum(),
            supervisor_control_actions: collect_strings(
                worker_metrics,
                "supervisor_control_action",
            ),
            supervisor_control_risks: worker_metrics
                .iter()
                .flat_map(supervisor_control_risks)
                .collect(),
            supervisor_control_interrupts: worker_metrics
                .iter()
                .filter(|metrics| get_bool(metrics, "interrupted_by_supervisor").unwrap_or(false))
                .count() as u64,
            local_inference_verified: !worker_metrics.is_empty()
                && worker_metrics
                    .iter()
                    .all(|metrics| get_bool(metrics, "local_inference_verified").unwrap_or(false)),
            gpu_activity_observed: worker_metrics
                .iter()
                .any(|metrics| get_bool(metrics, "gpu_activity_observed").unwrap_or(false)),
            backend_activity_observed: worker_metrics
                .iter()
                .any(|metrics| get_bool(metrics, "backend_activity_observed").unwrap_or(false)),
        }
    }
}

fn sum_u64(worker_metrics: &[Value], key: &str) -> u64 {
    worker_metrics
        .iter()
        .map(|metrics| get_u64(metrics, key).unwrap_or(0))
        .sum()
}

fn sum_f64(worker_metrics: &[Value], key: &str) -> f64 {
    worker_metrics
        .iter()
        .map(|metrics| metrics.get(key).and_then(Value::as_f64).unwrap_or(0.0))
        .sum()
}

fn collect_strings(worker_metrics: &[Value], key: &str) -> Vec<String> {
    worker_metrics
        .iter()
        .filter_map(|metrics| get_str(metrics, key).map(ToOwned::to_owned))
        .collect()
}

fn supervisor_control_event_count(metrics: &Value) -> u64 {
    metrics
        .get("supervisor_control_events")
        .and_then(Value::as_array)
        .map(|items| items.len() as u64)
        .unwrap_or(0)
}

fn supervisor_control_risks(metrics: &Value) -> Vec<String> {
    metrics
        .get("supervisor_control_events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| get_str(event, "risk").map(ToOwned::to_owned))
        .filter(|risk| !risk.trim().is_empty())
        .collect()
}
