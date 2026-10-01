//! Machine-node wire types and security primitives shared by NyxID and its node.

pub mod binary;
pub mod config;
pub mod desktop;
pub mod gateway;
pub mod signing;
pub mod text;

use serde::{Deserialize, Serialize};

pub const CUA_TOOLS: &str = include_str!("../resources/cua-tools.json");

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_RESULT_BYTES: usize = 9_000;
pub const STREAM_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_FRAME_BYTES: usize = 5 * 1024 * 1024;
/// Git receive-pack sends the pack before receiving response headers. The
/// command's own lifetime and stream idle limits still bound this allowance.
pub const GIT_UPLOAD_TIMEOUT_SECS: u64 = 3600;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    #[default]
    None,
    Changes,
    All,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComputerMode {
    #[default]
    Standard,
    Unrestricted,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ComputerPermissions {
    pub screen_recording: Option<bool>,
    pub accessibility: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct MachineProfile {
    pub version: u32,
    /// Changes on daemon restart, remains stable across WebSocket reconnects.
    pub runtime_id: String,
    pub shell: bool,
    pub files: bool,
    pub computer: bool,
    pub os: String,
    pub arch: String,
    pub roots: Vec<String>,
    pub computer_mode: ComputerMode,
    pub cua_version: Option<String>,
    pub computer_tools: Vec<String>,
    pub computer_ready: bool,
    pub computer_permissions: Option<ComputerPermissions>,
    pub browser_isolated: bool,
    pub saved_login_ready: bool,
}

impl MachineProfile {
    pub fn enabled(&self) -> bool {
        self.version == PROTOCOL_VERSION && (self.shell || self.files || self.computer)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    ProxyUpload,
    ServiceCall,
    JobFinished,
    Exec,
    Job,
    JobCancel,
    ListFiles,
    ReadFile,
    WriteFile,
    EditFile,
    SaveAttachment,
    ShareFile,
    Computer,
    DesktopOpen,
    DesktopClose,
    DesktopControl,
    DesktopInput,
    FillLogin,
}

impl Operation {
    pub fn allowed(self, profile: &MachineProfile) -> bool {
        if profile.version != PROTOCOL_VERSION {
            return false;
        }
        match self {
            Self::ProxyUpload => false,
            Self::Exec | Self::Job | Self::JobCancel | Self::ServiceCall | Self::JobFinished => {
                profile.shell
            }
            Self::ListFiles
            | Self::ReadFile
            | Self::WriteFile
            | Self::EditFile
            | Self::SaveAttachment
            | Self::ShareFile => profile.files,
            Self::Computer
            | Self::DesktopOpen
            | Self::DesktopClose
            | Self::DesktopControl
            | Self::DesktopInput
            | Self::FillLogin => profile.computer,
        }
    }
}

/// Parameters may contain a login value, commands, or paths; Debug never does.
#[derive(Clone, Deserialize, Serialize)]
pub struct Request {
    pub request_id: String,
    pub node_id: String,
    pub operation: Operation,
    pub parameters: serde_json::Value,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MachineRequest")
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Failure {
    pub code: u32,
    pub message: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Response {
    pub request_id: String,
    pub result: serde_json::Value,
}
impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MachineResponse { [REDACTED] }")
    }
}
