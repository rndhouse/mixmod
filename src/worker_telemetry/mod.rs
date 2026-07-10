//! Worker-backend telemetry adapters.
//!
//! These adapters normalize backend-specific monitoring endpoints into raw
//! Mixmod telemetry. They should not interpret whether a worker is healthy or
//! context-pressured; that judgment belongs to the supervisor.

pub(crate) mod llama_server;

use serde::{Deserialize, Serialize};

/// Raw worker-backend telemetry exposed to live supervisor turns.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerBackendTelemetry {
    /// Backend telemetry provider that produced this packet.
    pub provider: String,
    /// Whether backend telemetry was reachable for this sample.
    pub available: bool,
    /// UTC timestamp when Mixmod captured the telemetry packet.
    pub captured_at: String,
    /// Context size reported by the backend, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx_size: Option<u64>,
    /// Number of requests currently processing, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests_processing: Option<u64>,
    /// Number of requests currently deferred or queued, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests_deferred: Option<u64>,
    /// Backend-reported high watermark of observed context tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_max_observed: Option<u64>,
    /// Active backend slots, reduced to raw fields relevant to worker progress.
    pub active_slots: Vec<WorkerBackendSlotTelemetry>,
    /// Short collection error when no backend telemetry endpoint was reachable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Raw per-slot worker-backend telemetry.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerBackendSlotTelemetry {
    /// Backend slot identifier.
    pub id: u64,
    /// Context size for this slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx_size: Option<u64>,
    /// Whether the slot is processing a request.
    pub is_processing: bool,
    /// Number of decoded tokens reported for the current response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_tokens: Option<u64>,
    /// Number of remaining tokens reported by the backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_tokens: Option<i64>,
}
