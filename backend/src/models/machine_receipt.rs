//! Bounded machine-operation metadata. Never commands, paths, URLs or output.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineReceipt {
    pub operation_id: String,
    pub node_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub context_mode: Option<String>,
    pub action: String,
    pub status: String,
    pub job_id: Option<String>,
    pub exit_code: Option<i64>,
    pub bytes: Option<u64>,
    pub duration_ms: Option<u64>,
    pub error_code: Option<u32>,
    pub screenshot_id: Option<String>,
    pub preview_id: Option<String>,
    pub preview_enabled: bool,
}
