use serde_json::Value;

use crate::SupervisorControlEvent;

#[derive(Debug)]
pub(crate) struct VerifiedCommandOutput {
    pub(crate) exit_status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) opencode_segments: Vec<Value>,
    pub(crate) timed_out: bool,
    pub(crate) idle_timed_out: bool,
    pub(crate) interrupted_by_supervisor: bool,
    pub(crate) supervisor_control_action: Option<String>,
    pub(crate) supervisor_control_events: Vec<SupervisorControlEvent>,
    pub(crate) heartbeat_count: u64,
    pub(crate) local_inference_verified: bool,
    pub(crate) gpu_activity_observed: bool,
    pub(crate) backend_activity_observed: bool,
    pub(crate) verification_notes: Vec<String>,
}
