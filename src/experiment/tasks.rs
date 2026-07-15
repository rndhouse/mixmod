mod brief;
mod edit_packet;
mod focus;
mod format;
mod revision;
mod types;

const PATCH_REQUEST_DEFAULT_STOP_CONDITION: &str = "Return after one useful tracked diff for this patch request exists; do not continue into another independent slice, broad verification, or unrelated cleanup.";

pub(crate) use brief::write_worker_brief_task;
pub(crate) use revision::write_revision_task;
