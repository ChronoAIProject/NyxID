use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use base64::Engine;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::errors::{AppError, AppResult};
use crate::models::ws_frame_injection::WsFrameInjection;

const STREAM_BUFFER_CAPACITY: usize = 1024;
const SSH_TUNNEL_BUFFER_CAPACITY: usize = 256;
const WEB_TERMINAL_BUFFER_CAPACITY: usize = 256;
const WS_PROXY_BUFFER_CAPACITY: usize = 512;

/// Request sent to a node via WebSocket.
#[derive(Clone)]
pub struct NodeProxyRequest {
    pub request_id: String,
    pub service_id: String,
    pub service_slug: String,
    pub base_url: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    /// Raw bytes (serialized to base64 in WS message)
    pub body: Option<Vec<u8>>,
}

/// Request sent to a node to open an SSH TCP tunnel.
#[derive(Clone)]
pub struct NodeSshTunnelRequest {
    pub session_id: String,
    pub service_id: String,
    pub host: String,
    pub port: u16,
}

/// Response received from a node via WebSocket (non-streaming).
pub struct NodeProxyResponse {
    pub request_id: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A chunk in a streaming proxy response.
#[derive(Debug)]
pub enum StreamChunk {
    /// Beginning of stream: status code and headers
    Start {
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// A chunk of response data
    Data(Vec<u8>),
    /// End of stream
    End,
    /// Stream error
    Error(String),
    /// Node reports that it injected a WS auth frame locally.
    Injected {
        trigger_kind: String,
        frame_index: usize,
    },
}

/// Result of sending a proxy request: either a complete response or a streaming channel.
pub enum ProxyResponseType {
    /// Standard request/response (v1 behavior)
    Complete(NodeProxyResponse),
    /// Streaming response: chunks arrive through the channel
    Streaming(mpsc::Receiver<StreamChunk>),
}

/// A chunk in a node-backed SSH tunnel.
#[derive(Debug)]
pub enum SshTunnelChunk {
    Data(Vec<u8>),
    Closed(Option<String>),
}

/// A chunk in a node-backed web terminal session.
#[derive(Debug)]
pub enum WebTerminalChunk {
    Data(Vec<u8>),
    Closed(Option<String>),
}

pub(crate) enum NodeProxyOutcome {
    Response(ProxyResponseType),
    RetryableFailure {
        message: String,
        /// Machine-readable classifier echoed from the node's
        /// `proxy_error.reason` field. Lets the backend distinguish
        /// "node is up but can't complete the request" (e.g.
        /// `credential_missing`) from "node is genuinely offline".
        /// `None` means the node didn't advertise a reason (older
        /// agents) or the failure originated on the backend.
        reason: Option<String>,
    },
}

/// A pending proxy request that may receive a single response or a stream.
pub(crate) enum PendingRequest {
    /// Waiting for the first correlated response, which may be either complete
    /// or a live streaming receiver.
    Awaiting(oneshot::Sender<NodeProxyOutcome>),
    /// Streaming response: sends chunks through an mpsc channel
    Streaming(mpsc::Sender<StreamChunk>),
}

pub(crate) enum PendingSshTunnel {
    Awaiting(oneshot::Sender<AppResult<mpsc::Receiver<SshTunnelChunk>>>),
    Active(mpsc::Sender<SshTunnelChunk>),
}

/// A frame in a node-backed WS proxy session.
#[derive(Debug)]
pub enum WsProxyFrame {
    /// Text WS frame from downstream.
    Text(String),
    /// Binary WS frame from downstream.
    Binary(Vec<u8>),
    /// Node reports that it injected a WS auth frame locally.
    Injected {
        trigger_kind: String,
        frame_index: usize,
    },
    /// Downstream closed the WS connection.
    Closed {
        code: Option<u16>,
        reason: Option<String>,
    },
    /// Error from downstream.
    Error(String),
}

pub struct NodeWsProxySession {
    pub frames: mpsc::Receiver<WsProxyFrame>,
    pub selected_protocol: Option<String>,
}

pub(crate) enum PendingWsProxy {
    Awaiting(oneshot::Sender<AppResult<NodeWsProxySession>>),
    Active(mpsc::Sender<WsProxyFrame>),
}

/// Request sent to a node to open a WS proxy connection.
#[derive(Clone)]
pub struct NodeWsProxyRequest {
    pub session_id: String,
    pub service_slug: String,
    pub base_url: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub ws_frame_injections: Vec<WsFrameInjection>,
}

pub(crate) enum PendingWebTerminal {
    Awaiting(oneshot::Sender<AppResult<mpsc::Receiver<WebTerminalChunk>>>),
    Active(mpsc::Sender<WebTerminalChunk>),
}

pub(crate) type PendingSshExec = oneshot::Sender<NodeSshExecResult>;

/// Request sent to a node to execute an SSH command.
#[derive(Clone)]
pub struct NodeSshExecRequest {
    pub request_id: String,
    pub host: String,
    pub port: u16,
    pub principal: String,
    pub private_key_pem: String,
    pub certificate_openssh: String,
    pub command: String,
    pub timeout_secs: u32,
}

/// Result received from a node after SSH command execution.
#[derive(Debug)]
pub struct NodeSshExecResult {
    pub request_id: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub error: Option<String>,
}

/// Request sent to a node to open a web terminal session.
#[derive(Clone)]
pub struct NodeWebTerminalRequest {
    pub session_id: String,
    pub service_id: String,
    pub host: String,
    pub port: u16,
    pub principal: String,
    pub private_key_pem: String,
    pub certificate_openssh: String,
    pub cols: u32,
    pub rows: u32,
}

/// Outbound command for a node connection writer task.
#[derive(Clone, Debug)]
pub(crate) enum NodeOutboundMessage {
    Text(String),
    Close { code: u16, reason: String },
}

/// Handle for sending messages to a connected node.
struct NodeConnection {
    /// Bounded channel to send WS messages to the node's write task (H4).
    /// Prevents memory exhaustion from slow/malicious nodes.
    tx: mpsc::Sender<NodeOutboundMessage>,
    /// Pending proxy request correlation map
    pending: Arc<DashMap<String, PendingRequest>>,
    /// Pending and active SSH tunnel sessions keyed by session_id
    ssh_tunnels: Arc<DashMap<String, PendingSshTunnel>>,
    /// Pending and active web terminal sessions keyed by session_id
    web_terminals: Arc<DashMap<String, PendingWebTerminal>>,
    /// Pending SSH exec requests keyed by request_id
    ssh_exec_requests: Arc<DashMap<String, PendingSshExec>>,
    /// Pending and active WS proxy sessions keyed by session_id
    ws_proxies: Arc<DashMap<String, PendingWsProxy>>,
    /// Pending `credential_update` / `credential_remove` acks keyed by
    /// the `request_id` the backend assigned when the frame was sent.
    /// Resolved by `handlers/node_ws::CredentialUpdateAck` when the
    /// node echoes the `request_id` back. Enables strict delivery
    /// semantics: callers that want to gate a DB commit on node-side
    /// persistence await the oneshot with a timeout.
    credential_acks: Arc<DashMap<String, oneshot::Sender<CredentialAckOutcome>>>,
    /// Per-connection capability flags advertised by the node in its
    /// `status_update` message. Strict ack-wait on credential pushes
    /// only runs when the node has advertised
    /// `credential_ack_correlation`; older agents that don't know
    /// about the `request_id` echo fall back to fire-and-forget
    /// delivery (twenty-seventh-round Codex P2). Arc so shallow
    /// clones share writes after the deep auth handshake.
    capabilities: Arc<std::sync::Mutex<NodeCapabilitiesFlags>>,
    /// Set to `true` once the node has sent its first `status_update`
    /// after the WS handshake — whether or not the frame carried a
    /// `capabilities` field. Callers that need to know "has the
    /// capability negotiation finished?" (e.g. strict credential
    /// push) wait on this instead of checking the flag state
    /// directly, avoiding a race where `PUT /keys` lands after auth
    /// but before the first `status_update` arrives and wrongly
    /// treats an upgraded agent as legacy
    /// (twenty-ninth-round Codex P2).
    capabilities_resolved: Arc<AtomicBool>,
    /// Broadcasts capability resolution to anyone awaiting it. Paired
    /// with `capabilities_resolved` so late waiters see the flag
    /// immediately without having to block. Arc + `notify_waiters`
    /// wakes every waiter on state transition.
    capability_notify: Arc<tokio::sync::Notify>,
}

/// Negotiated capability flags. Default is "legacy agent": every
/// feature disabled, preserving pre-migration behavior for nodes that
/// haven't been upgraded yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeCapabilitiesFlags {
    pub credential_ack_correlation: bool,
}

/// In-memory WebSocket connection manager for credential nodes.
pub struct NodeWsManager {
    /// Active connections: node_id -> NodeConnection
    connections: DashMap<String, NodeConnection>,
    /// Proxy request timeout in seconds
    proxy_timeout_secs: u64,
    /// Maximum concurrent WebSocket connections (authenticated + pending auth)
    max_connections: usize,
    /// Counter for connections currently in the auth handshake phase
    pending_auth: AtomicUsize,
}

/// JSON message sent from NyxID to a node for a proxy request.
#[derive(Debug, Serialize)]
struct WsProxyRequest {
    #[serde(rename = "type")]
    msg_type: &'static str,
    request_id: String,
    service_id: String,
    service_slug: String,
    base_url: String,
    method: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    headers: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    /// HMAC signing fields
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

/// JSON message for heartbeat ping.
#[derive(Debug, Serialize)]
struct WsHeartbeatPing {
    #[serde(rename = "type")]
    msg_type: &'static str,
    timestamp: String,
}

#[derive(Debug, Serialize)]
struct WsSshTunnelOpen {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    service_id: String,
    host: String,
    port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

#[derive(Debug, Serialize)]
struct WsSshTunnelData {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct WsSshTunnelClose {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
}

/// JSON ws_proxy_open sent to node.
#[derive(Debug, Serialize)]
struct WsProxyOpen {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    service_slug: String,
    base_url: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    headers: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ws_frame_injections: Vec<WsFrameInjection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

/// JSON ws_proxy_text sent to node.
#[derive(Debug, Serialize)]
struct WsProxyTextMsg {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    data: String,
}

/// JSON ws_proxy_binary sent to node (base64-encoded payload).
#[derive(Debug, Serialize)]
struct WsProxyBinaryMsg {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    data: String,
}

/// JSON ws_proxy_close sent to node.
#[derive(Debug, Serialize)]
struct WsProxyCloseMsg {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct WsSshExec {
    #[serde(rename = "type")]
    msg_type: &'static str,
    request_id: String,
    host: String,
    port: u16,
    principal: String,
    private_key_pem: String,
    certificate_openssh: String,
    command: String,
    timeout_secs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hmac: Option<String>,
}

#[derive(Debug, Serialize)]
struct WsWebTerminalOpen {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    service_id: String,
    host: String,
    port: u16,
    principal: String,
    private_key_pem: String,
    certificate_openssh: String,
    cols: u32,
    rows: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hmac: Option<String>,
}

#[derive(Debug, Serialize)]
struct WsWebTerminalData {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct WsWebTerminalResize {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
    cols: u32,
    rows: u32,
}

#[derive(Debug, Serialize)]
struct WsWebTerminalClose {
    #[serde(rename = "type")]
    msg_type: &'static str,
    session_id: String,
}

/// JSON proxy_response from node.
#[derive(Debug, Deserialize)]
pub struct WsProxyResponseMsg {
    pub request_id: String,
    pub status: u16,
    #[serde(default)]
    pub headers: serde_json::Value,
    #[serde(default)]
    pub body: Option<String>,
}

/// JSON proxy_error from node.
#[derive(Debug, Deserialize)]
pub struct WsProxyErrorMsg {
    pub request_id: String,
    pub error: String,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub retryable: bool,
    /// Optional machine-readable classifier (e.g. `"credential_missing"`).
    /// Older node agents omit this field; the backend falls back to
    /// the generic `NodeOffline` path in that case.
    #[serde(default)]
    pub reason: Option<String>,
}

/// JSON proxy_response_start from node (streaming).
#[derive(Debug, Deserialize)]
pub struct WsProxyResponseStartMsg {
    pub request_id: String,
    pub status: u16,
    #[serde(default)]
    pub headers: serde_json::Value,
}

/// JSON proxy_response_chunk from node (streaming).
#[derive(Debug, Deserialize)]
pub struct WsProxyResponseChunkMsg {
    pub request_id: String,
    #[serde(default)]
    pub data: Option<String>,
}

/// JSON proxy_response_end from node (streaming).
#[derive(Debug, Deserialize)]
pub struct WsProxyResponseEndMsg {
    pub request_id: String,
}

/// JSON ssh_tunnel_opened from node.
#[derive(Debug, Deserialize)]
pub struct WsSshTunnelOpenedMsg {
    pub session_id: String,
}

/// JSON ssh_tunnel_data from node.
#[derive(Debug, Deserialize)]
pub struct WsSshTunnelDataMsg {
    pub session_id: String,
    #[serde(default)]
    pub data: Option<String>,
}

/// JSON ssh_tunnel_closed from node.
#[derive(Debug, Deserialize)]
pub struct WsSshTunnelClosedMsg {
    pub session_id: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// JSON ws_proxy_opened from node.
#[derive(Debug, Deserialize)]
pub struct WsProxyOpenedInbound {
    pub session_id: String,
    #[serde(default)]
    pub selected_protocol: Option<String>,
}

/// JSON ws_proxy_text from node.
#[derive(Debug, Deserialize)]
pub struct WsProxyTextInbound {
    pub session_id: String,
    pub data: String,
}

/// JSON ws_proxy_binary from node (base64-encoded payload).
#[derive(Debug, Deserialize)]
pub struct WsProxyBinaryInbound {
    pub session_id: String,
    pub data: String,
}

/// JSON ws_proxy_closed from node.
#[derive(Debug, Deserialize)]
pub struct WsProxyClosedInbound {
    pub session_id: String,
    #[serde(default)]
    pub code: Option<u16>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// JSON ws_proxy_error from node.
#[derive(Debug, Deserialize)]
pub struct WsProxyErrorInbound {
    pub session_id: String,
    pub error: String,
}

/// JSON ws_frame_injected from node.
#[derive(Debug, Deserialize)]
pub struct WsFrameInjectedInbound {
    #[serde(rename = "request_id", alias = "session_id")]
    pub session_id: String,
    pub trigger_kind: String,
    pub frame_index: usize,
}

/// JSON ssh_exec_result from node.
#[derive(Debug, Deserialize)]
pub struct WsSshExecResultMsg {
    pub request_id: String,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub stdout: Option<String>,
    #[serde(default)]
    pub stderr: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// JSON web_terminal_started from node.
#[derive(Debug, Deserialize)]
pub struct WsWebTerminalStartedMsg {
    pub session_id: String,
}

/// JSON web_terminal_data from node.
#[derive(Debug, Deserialize)]
pub struct WsWebTerminalDataMsg {
    pub session_id: String,
    #[serde(default)]
    pub data: Option<String>,
}

/// JSON web_terminal_closed from node.
#[derive(Debug, Deserialize)]
pub struct WsWebTerminalClosedMsg {
    pub session_id: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// JSON message for pushing a credential update to a node. Includes an
/// optional `request_id` used by callers that want to await a node-
/// side `credential_update_ack` before treating the push as confirmed
/// (see `send_credential_update_and_wait`). Older node agents ignore
/// the field; newer ones echo it back in the ack.
#[derive(Debug, Serialize)]
struct WsCredentialUpdate {
    #[serde(rename = "type")]
    msg_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    service_slug: String,
    injection_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    header_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    header_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    param_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    param_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_url: Option<String>,
}

/// JSON message instructing a node to drop its local credential for a
/// given service slug. Sent when a `UserService` is reassigned from one
/// node to another so the prior node stops holding the secret. Includes
/// an optional `request_id` for ack-wait correlation.
#[derive(Debug, Serialize)]
struct WsCredentialRemove {
    #[serde(rename = "type")]
    msg_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    service_slug: String,
}

/// Outcome of a `credential_update` / `credential_remove` ack from a
/// node agent. `Ok` means the node persisted the change; `Err` carries
/// the node's error message.
#[derive(Debug, Clone)]
pub enum CredentialAckOutcome {
    Ok,
    Err(String),
}

/// Capability flags advertised by a node agent in its `status_update`
/// message. The backend uses these to decide whether to enable
/// features that require node-side cooperation (e.g., `request_id`
/// echo on credential acks). Old agents omit the `capabilities` field
/// entirely; deserialisation sees `None`, so every flag defaults to
/// `false` and the backend falls back to legacy behavior (twenty-
/// seventh-round Codex P2).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NodeCapabilitiesMsg {
    /// Node echoes the `request_id` from a `credential_update` /
    /// `credential_remove` frame back in the resulting
    /// `credential_update_ack`. Required for strict ack-wait on the
    /// `PUT /keys` push path; when absent, the backend falls back to
    /// fire-and-forget delivery.
    #[serde(default)]
    pub credential_ack_correlation: bool,
}

/// Parameters for pushing a credential update to a node.
pub struct CredentialUpdateParams {
    pub service_slug: String,
    pub injection_method: String,
    pub header_name: Option<String>,
    pub header_value: Option<String>,
    pub param_name: Option<String>,
    pub param_value: Option<String>,
    pub target_url: Option<String>,
}

/// Map a retryable proxy failure from a node into the appropriate
/// [`AppError`] variant. A `reason` of `"credential_missing"` means the
/// node itself is reachable and functioning, but doesn't have a local
/// credential for the requested slug — a misconfiguration on the node,
/// not a transient outage. Surfacing that as `NodeCredentialMissing`
/// (HTTP 502 / code 8004) lets clients tell it apart from
/// `NodeOffline` (HTTP 503 / code 8001), which is what issue #418 asks
/// for. Every other reason (or `None` from older agents) still lands
/// in the generic `NodeOffline` bucket so fallback/retry behavior is
/// unchanged.
pub(crate) fn map_retryable_node_failure(message: String, reason: Option<&str>) -> AppError {
    match reason {
        Some("credential_missing") => AppError::NodeCredentialMissing(message),
        _ => AppError::NodeOffline(message),
    }
}

/// Compute HMAC-SHA256 signature for a proxy request.
pub fn compute_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: Option<&[u8]>,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let body_b64 = body
        .map(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(b)
        })
        .unwrap_or_default();

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        timestamp,
        nonce,
        method,
        path,
        query.unwrap_or(""),
        body_b64,
    );

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Compute HMAC-SHA256 signature for an SSH tunnel open request.
pub fn compute_ssh_tunnel_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    session_id: &str,
    service_id: &str,
    host: &str,
    port: u16,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        timestamp, nonce, session_id, service_id, host, port
    );

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Compute HMAC-SHA256 signature for an SSH exec request.
/// Message format matches the node agent's `verify_ssh_exec_signature`:
/// `{timestamp}\n{nonce}\n{request_id}\n{host}\n{port}\n{principal}`
pub fn compute_ssh_exec_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    request_id: &str,
    host: &str,
    port: u16,
    principal: &str,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        timestamp, nonce, request_id, host, port, principal
    );

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Compute HMAC-SHA256 signature for a web terminal open request.
/// Message format matches the node agent's `verify_web_terminal_signature`:
/// `{timestamp}\n{nonce}\n{session_id}\n{host}\n{port}\n{principal}`
pub fn compute_web_terminal_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    session_id: &str,
    host: &str,
    port: u16,
    principal: &str,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        timestamp, nonce, session_id, host, port, principal
    );

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

impl NodeWsManager {
    fn handle_stream_send_result(
        result: Result<(), mpsc::error::TrySendError<StreamChunk>>,
        node_id: &str,
        request_id: &str,
        chunk_kind: &'static str,
    ) -> bool {
        match result {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    node_id = %node_id,
                    request_id = %request_id,
                    chunk_kind,
                    capacity = STREAM_BUFFER_CAPACITY,
                    "Dropping node proxy stream due to full receive buffer"
                );
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    pub fn new(proxy_timeout_secs: u64, max_connections: usize) -> Self {
        Self {
            connections: DashMap::new(),
            proxy_timeout_secs,
            max_connections,
            pending_auth: AtomicUsize::new(0),
        }
    }

    /// Total connections including those still in auth handshake.
    pub fn total_connection_count(&self) -> usize {
        self.connections.len() + self.pending_auth.load(Ordering::Relaxed)
    }

    /// Maximum allowed concurrent connections.
    pub fn max_connections(&self) -> usize {
        self.max_connections
    }

    /// Increment the pending auth counter (called before WS upgrade).
    pub fn increment_pending_auth(&self) {
        self.pending_auth.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the pending auth counter (called after auth completes or fails).
    pub fn decrement_pending_auth(&self) {
        self.pending_auth.fetch_sub(1, Ordering::Relaxed);
    }

    /// Register a new WebSocket connection with a pre-created sender.
    /// Returns the pending request map for the WS reader task to deliver responses.
    pub(crate) fn register_connection(
        &self,
        node_id: &str,
        tx: mpsc::Sender<NodeOutboundMessage>,
    ) -> Arc<DashMap<String, PendingRequest>> {
        let pending = Arc::new(DashMap::new());
        let ssh_tunnels = Arc::new(DashMap::new());
        let web_terminals = Arc::new(DashMap::new());
        let ssh_exec_requests = Arc::new(DashMap::new());
        let ws_proxies = Arc::new(DashMap::new());
        let return_pending = pending.clone();

        self.connections.insert(
            node_id.to_string(),
            NodeConnection {
                tx,
                pending,
                ssh_tunnels,
                web_terminals,
                ssh_exec_requests,
                ws_proxies,
                credential_acks: Arc::new(DashMap::new()),
                capabilities: Arc::new(std::sync::Mutex::new(NodeCapabilitiesFlags::default())),
                capabilities_resolved: Arc::new(AtomicBool::new(false)),
                capability_notify: Arc::new(tokio::sync::Notify::new()),
            },
        );

        return_pending
    }

    /// Remove a node's connection (called on WS close).
    /// Drops all pending request senders so receivers get RecvError.
    pub fn unregister_connection(&self, node_id: &str) {
        if let Some((_, conn)) = self.connections.remove(node_id) {
            conn.pending.clear();
            conn.ssh_tunnels.clear();
            conn.web_terminals.clear();
            conn.ssh_exec_requests.clear();
            conn.ws_proxies.clear();
            // Drop pending credential-ack waiters so any in-flight
            // `send_credential_update_and_wait` / `_remove_and_wait`
            // fails immediately with RecvError (→ our NodeOffline
            // branch) instead of blocking for the full 10-second
            // timeout (twenty-sixth-round Codex P3). Clearing the map
            // drops the `oneshot::Sender`s, which closes the
            // receivers.
            conn.credential_acks.clear();
        }
    }

    /// Force-close a node connection by sending a WebSocket close frame.
    /// Pending requests are dropped before the close is delivered so callers
    /// immediately observe disconnect semantics.
    pub async fn disconnect_connection(&self, node_id: &str, code: u16, reason: &str) -> bool {
        if let Some((_, conn)) = self.connections.remove(node_id) {
            conn.pending.clear();
            conn.ssh_tunnels.clear();
            conn.web_terminals.clear();
            conn.ssh_exec_requests.clear();
            conn.ws_proxies.clear();
            conn.credential_acks.clear();
            let close_msg = NodeOutboundMessage::Close {
                code,
                reason: reason.to_string(),
            };
            match conn.tx.try_send(close_msg.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        conn.tx.send(close_msg),
                    )
                    .await;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
            true
        } else {
            false
        }
    }

    /// Check if a node has an active WebSocket connection.
    pub fn is_connected(&self, node_id: &str) -> bool {
        self.connections.contains_key(node_id)
    }

    /// Send a proxy request to a node and wait for the response.
    /// If `signing_secret` is provided, the request is HMAC-signed.
    /// Returns either a complete response or a streaming channel.
    pub async fn send_proxy_request(
        &self,
        node_id: &str,
        request: NodeProxyRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<ProxyResponseType> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let request_id = request.request_id.clone();

        // Create oneshot channel for response correlation. The response may be a
        // complete payload or a live streaming receiver.
        let (resp_tx, resp_rx) = oneshot::channel();
        conn.pending
            .insert(request_id.clone(), PendingRequest::Awaiting(resp_tx));

        // Build headers as JSON object
        let headers_map: serde_json::Map<String, serde_json::Value> = request
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();

        let body_b64 = request.body.as_ref().map(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(b)
        });

        // Compute HMAC signature if signing secret is provided
        let (timestamp, nonce, signature) = if let Some(secret) = signing_secret {
            let ts = chrono::Utc::now().to_rfc3339();
            let n = uuid::Uuid::new_v4().to_string();
            let sig = compute_hmac_signature(
                secret,
                &ts,
                &n,
                &request.method,
                &request.path,
                request.query.as_deref(),
                request.body.as_deref(),
            );
            (Some(ts), Some(n), Some(sig))
        } else {
            (None, None, None)
        };

        // Build WS message
        let ws_msg = WsProxyRequest {
            msg_type: "proxy_request",
            request_id: request_id.clone(),
            service_id: request.service_id,
            service_slug: request.service_slug,
            base_url: request.base_url,
            method: request.method,
            path: request.path,
            query: request.query,
            headers: serde_json::Value::Object(headers_map),
            body: body_b64,
            timestamp,
            nonce,
            signature,
        };

        let msg_json = serde_json::to_string(&ws_msg).map_err(|e| {
            conn.pending.remove(&request_id);
            AppError::Internal(format!("Failed to serialize proxy request: {e}"))
        })?;

        // H4: Use try_send on bounded channel. If the channel is full, the node
        // is not keeping up (slow or malicious) — treat as offline.
        match conn.tx.try_send(NodeOutboundMessage::Text(msg_json)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                conn.pending.remove(&request_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} write buffer full"
                )));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                conn.pending.remove(&request_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} connection closed"
                )));
            }
        }

        // Drop the connection ref before awaiting
        drop(conn);

        // Wait for response with timeout
        let timeout = std::time::Duration::from_secs(self.proxy_timeout_secs);
        match tokio::time::timeout(timeout, resp_rx).await {
            Ok(Ok(NodeProxyOutcome::Response(response))) => Ok(response),
            Ok(Ok(NodeProxyOutcome::RetryableFailure { message, reason })) => {
                Err(map_retryable_node_failure(message, reason.as_deref()))
            }
            Ok(Err(_)) => Err(AppError::NodeOffline(format!(
                "Node {node_id} disconnected during request"
            ))),
            Err(_) => {
                // Timeout -- clean up pending request
                if let Some(conn) = self.connections.get(node_id) {
                    conn.pending.remove(&request_id);
                }
                Err(AppError::NodeProxyTimeout)
            }
        }
    }

    /// Open an SSH tunnel on a connected node and await the open acknowledgement.
    pub async fn open_ssh_tunnel(
        &self,
        node_id: &str,
        request: NodeSshTunnelRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<mpsc::Receiver<SshTunnelChunk>> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let session_id = request.session_id.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        conn.ssh_tunnels
            .insert(session_id.clone(), PendingSshTunnel::Awaiting(ready_tx));

        let (timestamp, nonce, signature) = if let Some(secret) = signing_secret {
            let ts = chrono::Utc::now().to_rfc3339();
            let n = uuid::Uuid::new_v4().to_string();
            let sig = compute_ssh_tunnel_hmac_signature(
                secret,
                &ts,
                &n,
                &request.session_id,
                &request.service_id,
                &request.host,
                request.port,
            );
            (Some(ts), Some(n), Some(sig))
        } else {
            (None, None, None)
        };

        let msg = serde_json::to_string(&WsSshTunnelOpen {
            msg_type: "ssh_tunnel_open",
            session_id: request.session_id,
            service_id: request.service_id,
            host: request.host,
            port: request.port,
            timestamp,
            nonce,
            signature,
        })
        .map_err(|e| {
            conn.ssh_tunnels.remove(&session_id);
            AppError::Internal(format!("Failed to serialize SSH tunnel open request: {e}"))
        })?;

        match conn.tx.try_send(NodeOutboundMessage::Text(msg)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                conn.ssh_tunnels.remove(&session_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} write buffer full"
                )));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                conn.ssh_tunnels.remove(&session_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} connection closed"
                )));
            }
        }

        drop(conn);

        let timeout = std::time::Duration::from_secs(self.proxy_timeout_secs);
        match tokio::time::timeout(timeout, ready_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppError::NodeOffline(format!(
                "Node {node_id} disconnected during SSH tunnel open"
            ))),
            Err(_) => {
                if let Some(conn) = self.connections.get(node_id) {
                    conn.ssh_tunnels.remove(&session_id);
                }
                Err(AppError::NodeProxyTimeout)
            }
        }
    }

    /// Forward SSH bytes to an active node tunnel session.
    pub fn send_ssh_tunnel_data(
        &self,
        node_id: &str,
        session_id: &str,
        data: &[u8],
    ) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        if !conn.ssh_tunnels.contains_key(session_id) {
            return Err(AppError::NodeOffline(format!(
                "SSH tunnel session {session_id} is not active"
            )));
        }

        let msg = serde_json::to_string(&WsSshTunnelData {
            msg_type: "ssh_tunnel_data",
            session_id: session_id.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(data),
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize SSH tunnel data: {e}")))?;

        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;

        Ok(())
    }

    /// Request closure of an active node SSH tunnel.
    pub fn close_ssh_tunnel(&self, node_id: &str, session_id: &str) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let msg = serde_json::to_string(&WsSshTunnelClose {
            msg_type: "ssh_tunnel_close",
            session_id: session_id.to_string(),
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize SSH tunnel close: {e}")))?;
        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;
        conn.ssh_tunnels.remove(session_id);
        Ok(())
    }

    /// Send a heartbeat ping to a node. Non-blocking.
    pub fn send_heartbeat_ping(&self, node_id: &str) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;

        let ping = WsHeartbeatPing {
            msg_type: "heartbeat_ping",
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let msg = serde_json::to_string(&ping)
            .map_err(|e| AppError::Internal(format!("Failed to serialize heartbeat: {e}")))?;

        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;

        Ok(())
    }

    /// Push a credential update to a connected node. Fire-and-forget.
    /// Returns Ok(()) if the message was queued, Err if node is not connected.
    pub fn send_credential_update(
        &self,
        node_id: &str,
        params: &CredentialUpdateParams,
    ) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;

        let msg = WsCredentialUpdate {
            msg_type: "credential_update",
            request_id: None,
            service_slug: params.service_slug.clone(),
            injection_method: params.injection_method.clone(),
            header_name: params.header_name.clone(),
            header_value: params.header_value.clone(),
            param_name: params.param_name.clone(),
            param_value: params.param_value.clone(),
            target_url: params.target_url.clone(),
        };

        let json = serde_json::to_string(&msg).map_err(|e| {
            AppError::Internal(format!("Failed to serialize credential_update: {e}"))
        })?;

        conn.tx
            .try_send(NodeOutboundMessage::Text(json))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;

        tracing::info!(
            node_id = %node_id,
            service_slug = %params.service_slug,
            "Pushed credential update to node"
        );

        Ok(())
    }

    /// Send a `credential_remove` frame to the node so it drops any
    /// locally-stored credential + target_url for the given service
    /// slug. Used when a `UserService`'s `node_id` changes so the prior
    /// node stops holding the secret.
    pub fn send_credential_remove(&self, node_id: &str, service_slug: &str) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;

        let msg = WsCredentialRemove {
            msg_type: "credential_remove",
            request_id: None,
            service_slug: service_slug.to_string(),
        };
        let json = serde_json::to_string(&msg).map_err(|e| {
            AppError::Internal(format!("Failed to serialize credential_remove: {e}"))
        })?;

        conn.tx
            .try_send(NodeOutboundMessage::Text(json))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;

        tracing::info!(
            node_id = %node_id,
            service_slug = %service_slug,
            "Sent credential_remove to node"
        );

        Ok(())
    }

    /// Strict-wait variant of `send_credential_update`: generates a
    /// `request_id`, registers a pending oneshot waiter, sends the
    /// frame, then awaits the node's `credential_update_ack` with a
    /// timeout. Returns `Ok(())` only when the node acknowledged a
    /// successful apply. Timeout or negative ack returns an error so
    /// the caller can abort the surrounding transaction. Callers that
    /// don't need strict semantics keep using `send_credential_update`.
    pub async fn send_credential_update_and_wait(
        &self,
        node_id: &str,
        params: &CredentialUpdateParams,
        timeout: std::time::Duration,
    ) -> AppResult<()> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx_ack, rx_ack) = oneshot::channel::<CredentialAckOutcome>();
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        conn.credential_acks.insert(request_id.clone(), tx_ack);

        let msg = WsCredentialUpdate {
            msg_type: "credential_update",
            request_id: Some(request_id.clone()),
            service_slug: params.service_slug.clone(),
            injection_method: params.injection_method.clone(),
            header_name: params.header_name.clone(),
            header_value: params.header_value.clone(),
            param_name: params.param_name.clone(),
            param_value: params.param_value.clone(),
            target_url: params.target_url.clone(),
        };
        let json = serde_json::to_string(&msg).map_err(|e| {
            conn.credential_acks.remove(&request_id);
            AppError::Internal(format!("Failed to serialize credential_update: {e}"))
        })?;
        if conn.tx.try_send(NodeOutboundMessage::Text(json)).is_err() {
            conn.credential_acks.remove(&request_id);
            return Err(AppError::NodeOffline(format!(
                "Node {node_id} connection closed or buffer full"
            )));
        }
        let acks_ref = conn.credential_acks.clone();
        drop(conn); // release DashMap ref before awaiting

        match tokio::time::timeout(timeout, rx_ack).await {
            Ok(Ok(CredentialAckOutcome::Ok)) => Ok(()),
            Ok(Ok(CredentialAckOutcome::Err(msg))) => {
                // A negative ack means the node's keyring / local
                // config write failed — the request body was already
                // validated, so surfacing this as 400 would mislead
                // callers into treating it as a client-side error
                // and could stop automation from retrying. Map to
                // `NodeOffline` so it lands in the node-failure class
                // (5xx-shaped) alongside disconnect / timeout
                // outcomes (thirty-first-round Codex P2).
                Err(AppError::NodeOffline(format!(
                    "Node rejected credential update: {msg}"
                )))
            }
            Ok(Err(_)) => {
                acks_ref.remove(&request_id);
                Err(AppError::NodeOffline(format!(
                    "Node {node_id} dropped before acknowledging credential update"
                )))
            }
            Err(_) => {
                acks_ref.remove(&request_id);
                // Timeout with no matching ack. Return an error so the
                // caller (the `PUT /keys` handler) aborts before
                // committing routing/auth mutations. This is the
                // correct safety property: without a confirmed apply,
                // committing the DB side leaves server and node out of
                // sync (twenty-fifth-round Codex P1 walks back the
                // earlier best-effort fallback). CLIs that predate the
                // `request_id` echo need to be upgraded in lockstep —
                // users who hit the timeout see a clear error instead
                // of a silent broken service.
                Err(AppError::NodeOffline(format!(
                    "Timed out waiting for credential_update_ack from node {node_id}"
                )))
            }
        }
    }

    /// Strict-wait variant of `send_credential_remove`. Same semantics
    /// as `send_credential_update_and_wait`: returns `Ok(())` only after
    /// the node's `credential_update_ack` echoes back with status=ok.
    pub async fn send_credential_remove_and_wait(
        &self,
        node_id: &str,
        service_slug: &str,
        timeout: std::time::Duration,
    ) -> AppResult<()> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx_ack, rx_ack) = oneshot::channel::<CredentialAckOutcome>();
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        conn.credential_acks.insert(request_id.clone(), tx_ack);

        let msg = WsCredentialRemove {
            msg_type: "credential_remove",
            request_id: Some(request_id.clone()),
            service_slug: service_slug.to_string(),
        };
        let json = serde_json::to_string(&msg).map_err(|e| {
            conn.credential_acks.remove(&request_id);
            AppError::Internal(format!("Failed to serialize credential_remove: {e}"))
        })?;
        if conn.tx.try_send(NodeOutboundMessage::Text(json)).is_err() {
            conn.credential_acks.remove(&request_id);
            return Err(AppError::NodeOffline(format!(
                "Node {node_id} connection closed or buffer full"
            )));
        }
        let acks_ref = conn.credential_acks.clone();
        drop(conn);

        match tokio::time::timeout(timeout, rx_ack).await {
            Ok(Ok(CredentialAckOutcome::Ok)) => Ok(()),
            Ok(Ok(CredentialAckOutcome::Err(msg))) => Err(AppError::NodeOffline(format!(
                "Node rejected credential remove: {msg}"
            ))),
            Ok(Err(_)) => {
                acks_ref.remove(&request_id);
                Err(AppError::NodeOffline(format!(
                    "Node {node_id} dropped before acknowledging credential remove"
                )))
            }
            Err(_) => {
                acks_ref.remove(&request_id);
                // Timeout with no matching ack. Return an error so the
                // caller (the `PUT /keys` handler) aborts before
                // committing routing/auth mutations. This is the
                // correct safety property: without a confirmed apply,
                // committing the DB side leaves server and node out of
                // sync (twenty-fifth-round Codex P1 walks back the
                // earlier best-effort fallback). CLIs that predate the
                // `request_id` echo need to be upgraded in lockstep —
                // users who hit the timeout see a clear error instead
                // of a silent broken service.
                Err(AppError::NodeOffline(format!(
                    "Timed out waiting for credential_update_ack from node {node_id}"
                )))
            }
        }
    }

    /// Record the capabilities advertised by a node in its
    /// `status_update` message. Called by the WS reader task on each
    /// status_update; stays a no-op for nodes that omit the field
    /// (old agents → `None`).
    pub fn record_capabilities(&self, node_id: &str, caps: &NodeCapabilitiesMsg) {
        if let Some(conn) = self.connections.get(node_id)
            && let Ok(mut flags) = conn.capabilities.lock()
        {
            flags.credential_ack_correlation = caps.credential_ack_correlation;
        }
    }

    /// Mark that the node has sent *some* `status_update` — with or
    /// without a `capabilities` field — so strict-push waiters know the
    /// capability state for this connection is now final. Also wakes
    /// any futures blocked on `await_capability_resolution`. Called by
    /// the WS reader task on every `status_update`, so legacy agents
    /// that ship a status_update without capabilities still release
    /// waiters and fall through to the fire-and-forget branch
    /// immediately (twenty-ninth-round Codex P2).
    pub fn mark_status_update_received(&self, node_id: &str) {
        if let Some(conn) = self.connections.get(node_id) {
            let was_unresolved = !conn.capabilities_resolved.swap(true, Ordering::AcqRel);
            if was_unresolved {
                conn.capability_notify.notify_waiters();
            }
        }
    }

    /// Await the first `status_update` for this connection, up to
    /// `timeout`. Returns immediately if capabilities have already
    /// been resolved (including negative — old agent advertised no
    /// capabilities) or if the node is not connected. Used by
    /// `push_credential_to_node_strict` to avoid the reconnect race
    /// where a `PUT /keys` lands in the short window after auth but
    /// before the node's first `status_update`, which would otherwise
    /// wrongly downgrade an upgraded agent to fire-and-forget
    /// delivery (twenty-ninth-round Codex P2).
    pub async fn await_capability_resolution(&self, node_id: &str, timeout: std::time::Duration) {
        let (resolved, notify) = {
            let Some(conn) = self.connections.get(node_id) else {
                return;
            };
            (
                conn.capabilities_resolved.clone(),
                conn.capability_notify.clone(),
            )
        };
        if resolved.load(Ordering::Acquire) {
            return;
        }
        let notified = notify.notified();
        if resolved.load(Ordering::Acquire) {
            return;
        }
        let _ = tokio::time::timeout(timeout, notified).await;
    }

    /// Whether the connected node has advertised support for
    /// `request_id`-correlated `credential_update_ack` messages. Used
    /// by the strict-push caller to decide whether to await an ack or
    /// fall back to fire-and-forget.
    pub fn supports_credential_ack_correlation(&self, node_id: &str) -> bool {
        self.connections
            .get(node_id)
            .and_then(|conn| {
                conn.capabilities
                    .lock()
                    .ok()
                    .map(|f| f.credential_ack_correlation)
            })
            .unwrap_or(false)
    }

    /// Deliver a node's `credential_update_ack` to the pending waiter.
    /// Called by the WS reader task when a `credential_update_ack`
    /// arrives with a recognized `request_id`.
    pub fn deliver_credential_ack(
        &self,
        node_id: &str,
        request_id: &str,
        outcome: CredentialAckOutcome,
    ) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, sender)) = conn.credential_acks.remove(request_id)
        {
            let _ = sender.send(outcome);
        }
    }

    /// Get the IDs of all currently connected nodes.
    pub fn connected_node_ids(&self) -> Vec<String> {
        self.connections
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Deliver a proxy response from a node. Called by the WS reader task.
    pub fn deliver_proxy_response(&self, node_id: &str, response: NodeProxyResponse) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.pending.remove(&response.request_id)
        {
            match pending {
                PendingRequest::Awaiting(sender) => {
                    let _ = sender.send(NodeProxyOutcome::Response(ProxyResponseType::Complete(
                        response,
                    )));
                }
                PendingRequest::Streaming(tx) => {
                    // Unexpected: got a full response for a streaming request.
                    // Deliver as start + data + end.
                    let NodeProxyResponse {
                        request_id,
                        status,
                        headers,
                        body,
                    } = response;
                    if Self::handle_stream_send_result(
                        tx.try_send(StreamChunk::Start { status, headers }),
                        node_id,
                        &request_id,
                        "start",
                    ) && Self::handle_stream_send_result(
                        tx.try_send(StreamChunk::Data(body)),
                        node_id,
                        &request_id,
                        "data",
                    ) {
                        let _ = Self::handle_stream_send_result(
                            tx.try_send(StreamChunk::End),
                            node_id,
                            &request_id,
                            "end",
                        );
                    }
                }
            }
        }
    }

    /// Deliver a proxy error from a node. Called by the WS reader task.
    ///
    /// `reason` carries the optional machine-readable classifier from
    /// the node's `proxy_error.reason` field (e.g. `credential_missing`).
    /// It's propagated through [`NodeProxyOutcome::RetryableFailure`] so
    /// callers can distinguish specific failure classes from the generic
    /// "node offline" bucket.
    pub fn deliver_proxy_error(
        &self,
        node_id: &str,
        request_id: &str,
        error: &str,
        status: u16,
        retryable: bool,
        reason: Option<&str>,
    ) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.pending.remove(request_id)
        {
            match pending {
                PendingRequest::Awaiting(sender) => {
                    let outcome = if retryable {
                        NodeProxyOutcome::RetryableFailure {
                            message: error.to_string(),
                            reason: reason.map(str::to_string),
                        }
                    } else {
                        NodeProxyOutcome::Response(ProxyResponseType::Complete(NodeProxyResponse {
                            request_id: request_id.to_string(),
                            status,
                            headers: vec![],
                            body: serde_json::json!({ "error": error })
                                .to_string()
                                .into_bytes(),
                        }))
                    };
                    let _ = sender.send(outcome);
                }
                PendingRequest::Streaming(tx) => {
                    let _ = Self::handle_stream_send_result(
                        tx.try_send(StreamChunk::Error(error.to_string())),
                        node_id,
                        request_id,
                        "error",
                    );
                }
            }
        }
    }

    /// Handle a proxy_response_start message: upgrade pending from Awaiting to Streaming.
    pub fn deliver_stream_start(
        &self,
        node_id: &str,
        request_id: &str,
        status: u16,
        headers: Vec<(String, String)>,
    ) -> bool {
        let Some(conn) = self.connections.get(node_id) else {
            return false;
        };

        // Remove the Awaiting entry and replace with Streaming
        let Some((_, old_pending)) = conn.pending.remove(request_id) else {
            return false;
        };

        match old_pending {
            PendingRequest::Awaiting(response_tx) => {
                let (stream_tx, stream_rx) = mpsc::channel(STREAM_BUFFER_CAPACITY);
                if !Self::handle_stream_send_result(
                    stream_tx.try_send(StreamChunk::Start { status, headers }),
                    node_id,
                    request_id,
                    "start",
                ) {
                    return false;
                }
                if response_tx
                    .send(NodeProxyOutcome::Response(ProxyResponseType::Streaming(
                        stream_rx,
                    )))
                    .is_ok()
                {
                    conn.pending
                        .insert(request_id.to_string(), PendingRequest::Streaming(stream_tx));
                    true
                } else {
                    false
                }
            }
            PendingRequest::Streaming(tx) => {
                // Already streaming (duplicate start?). Send the start chunk and re-insert.
                if Self::handle_stream_send_result(
                    tx.try_send(StreamChunk::Start { status, headers }),
                    node_id,
                    request_id,
                    "start",
                ) {
                    conn.pending
                        .insert(request_id.to_string(), PendingRequest::Streaming(tx));
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Deliver a streaming chunk to an active stream.
    pub fn deliver_stream_chunk(&self, node_id: &str, request_id: &str, data: Vec<u8>) {
        if let Some(conn) = self.connections.get(node_id) {
            let send_result = {
                let Some(pending) = conn.pending.get(request_id) else {
                    return;
                };
                let PendingRequest::Streaming(tx) = pending.value() else {
                    return;
                };
                tx.try_send(StreamChunk::Data(data))
            };

            match send_result {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        request_id = %request_id,
                        capacity = STREAM_BUFFER_CAPACITY,
                        "Dropping node proxy stream due to full receive buffer"
                    );
                    conn.pending.remove(request_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    conn.pending.remove(request_id);
                }
            }
        }
    }

    /// Deliver end-of-stream and remove the pending entry.
    pub fn deliver_stream_end(&self, node_id: &str, request_id: &str) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.pending.remove(request_id)
            && let PendingRequest::Streaming(tx) = pending
        {
            let _ = Self::handle_stream_send_result(
                tx.try_send(StreamChunk::End),
                node_id,
                request_id,
                "end",
            );
        }
    }

    pub fn deliver_ssh_tunnel_opened(&self, node_id: &str, session_id: &str) -> bool {
        let Some(conn) = self.connections.get(node_id) else {
            return false;
        };
        let Some((_, pending)) = conn.ssh_tunnels.remove(session_id) else {
            return false;
        };

        match pending {
            PendingSshTunnel::Awaiting(sender) => {
                let (tx, rx) = mpsc::channel(SSH_TUNNEL_BUFFER_CAPACITY);
                let sent = sender.send(Ok(rx)).is_ok();
                if sent {
                    conn.ssh_tunnels
                        .insert(session_id.to_string(), PendingSshTunnel::Active(tx));
                }
                sent
            }
            PendingSshTunnel::Active(tx) => {
                conn.ssh_tunnels
                    .insert(session_id.to_string(), PendingSshTunnel::Active(tx));
                true
            }
        }
    }

    pub fn deliver_ssh_tunnel_data(&self, node_id: &str, session_id: &str, data: Vec<u8>) {
        if let Some(conn) = self.connections.get(node_id) {
            let send_result = {
                let Some(pending) = conn.ssh_tunnels.get(session_id) else {
                    return;
                };
                let PendingSshTunnel::Active(tx) = pending.value() else {
                    return;
                };
                tx.try_send(SshTunnelChunk::Data(data))
            };

            match send_result {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        session_id = %session_id,
                        capacity = SSH_TUNNEL_BUFFER_CAPACITY,
                        "Dropping SSH tunnel due to full receive buffer"
                    );
                    let close_msg = serde_json::to_string(&WsSshTunnelClose {
                        msg_type: "ssh_tunnel_close",
                        session_id: session_id.to_string(),
                    });
                    if let Ok(close_msg) = close_msg {
                        // TODO: This close signal is best-effort because the node write queue is
                        // also bounded. If try_send fails here, the node-side tunnel relies on
                        // its I/O timeout to clean up.
                        let _ = conn.tx.try_send(NodeOutboundMessage::Text(close_msg));
                    }
                    conn.ssh_tunnels.remove(session_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    conn.ssh_tunnels.remove(session_id);
                }
            }
        }
    }

    pub fn deliver_ssh_tunnel_closed(
        &self,
        node_id: &str,
        session_id: &str,
        error: Option<String>,
    ) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.ssh_tunnels.remove(session_id)
        {
            match pending {
                PendingSshTunnel::Awaiting(sender) => {
                    let _ = sender.send(Err(AppError::NodeOffline(
                        error.unwrap_or_else(|| "SSH tunnel closed before opening".to_string()),
                    )));
                }
                PendingSshTunnel::Active(tx) => {
                    let _ = tx.try_send(SshTunnelChunk::Closed(error));
                }
            }
        }
    }

    // ---- SSH exec (non-interactive command execution) ----

    /// Execute an SSH command on a connected node and wait for the result.
    /// If `signing_secret` is provided, the request is HMAC-signed.
    pub async fn exec_ssh_command(
        &self,
        node_id: &str,
        request: NodeSshExecRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<NodeSshExecResult> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let request_id = request.request_id.clone();

        let (resp_tx, resp_rx) = oneshot::channel();
        conn.ssh_exec_requests.insert(request_id.clone(), resp_tx);

        let (timestamp, nonce, hmac) = if let Some(secret) = signing_secret {
            let ts = chrono::Utc::now().to_rfc3339();
            let n = uuid::Uuid::new_v4().to_string();
            let sig = compute_ssh_exec_hmac_signature(
                secret,
                &ts,
                &n,
                &request.request_id,
                &request.host,
                request.port,
                &request.principal,
            );
            (Some(ts), Some(n), Some(sig))
        } else {
            (None, None, None)
        };

        let msg = serde_json::to_string(&WsSshExec {
            msg_type: "ssh_exec",
            request_id: request.request_id,
            host: request.host,
            port: request.port,
            principal: request.principal,
            private_key_pem: request.private_key_pem,
            certificate_openssh: request.certificate_openssh,
            command: request.command,
            timeout_secs: request.timeout_secs,
            timestamp,
            nonce,
            hmac,
        })
        .map_err(|e| {
            conn.ssh_exec_requests.remove(&request_id);
            AppError::Internal(format!("Failed to serialize SSH exec request: {e}"))
        })?;

        match conn.tx.try_send(NodeOutboundMessage::Text(msg)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                conn.ssh_exec_requests.remove(&request_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} write buffer full"
                )));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                conn.ssh_exec_requests.remove(&request_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} connection closed"
                )));
            }
        }

        drop(conn);

        // Use the configured proxy timeout plus the command timeout as the
        // total wait time (the node agent has its own timeout for the SSH
        // process, but we need a server-side deadline too).
        let total_timeout =
            std::time::Duration::from_secs(self.proxy_timeout_secs + request.timeout_secs as u64);
        match tokio::time::timeout(total_timeout, resp_rx).await {
            Ok(Ok(result)) => {
                if let Some(ref error) = result.error {
                    Err(AppError::Internal(format!("Node SSH exec error: {error}")))
                } else {
                    Ok(result)
                }
            }
            Ok(Err(_)) => Err(AppError::NodeOffline(format!(
                "Node {node_id} disconnected during SSH exec"
            ))),
            Err(_) => {
                if let Some(conn) = self.connections.get(node_id) {
                    conn.ssh_exec_requests.remove(&request_id);
                }
                Err(AppError::NodeProxyTimeout)
            }
        }
    }

    /// Deliver an ssh_exec_result from a node. Called by the WS reader task.
    pub fn deliver_ssh_exec_result(&self, node_id: &str, result: NodeSshExecResult) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, sender)) = conn.ssh_exec_requests.remove(&result.request_id)
        {
            let _ = sender.send(result);
        }
    }

    // ---- Web terminal session management ----

    /// Open a web terminal session on a connected node and await the started acknowledgement.
    pub async fn open_web_terminal(
        &self,
        node_id: &str,
        request: NodeWebTerminalRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<mpsc::Receiver<WebTerminalChunk>> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let session_id = request.session_id.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        conn.web_terminals
            .insert(session_id.clone(), PendingWebTerminal::Awaiting(ready_tx));

        let (timestamp, nonce, signature) = if let Some(secret) = signing_secret {
            let ts = chrono::Utc::now().to_rfc3339();
            let n = uuid::Uuid::new_v4().to_string();
            let sig = compute_web_terminal_hmac_signature(
                secret,
                &ts,
                &n,
                &request.session_id,
                &request.host,
                request.port,
                &request.principal,
            );
            (Some(ts), Some(n), Some(sig))
        } else {
            (None, None, None)
        };

        let msg = serde_json::to_string(&WsWebTerminalOpen {
            msg_type: "web_terminal_open",
            session_id: request.session_id,
            service_id: request.service_id,
            host: request.host,
            port: request.port,
            principal: request.principal,
            private_key_pem: request.private_key_pem,
            certificate_openssh: request.certificate_openssh,
            cols: request.cols,
            rows: request.rows,
            timestamp,
            nonce,
            hmac: signature,
        })
        .map_err(|e| {
            conn.web_terminals.remove(&session_id);
            AppError::Internal(format!(
                "Failed to serialize web terminal open request: {e}"
            ))
        })?;

        match conn.tx.try_send(NodeOutboundMessage::Text(msg)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                conn.web_terminals.remove(&session_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} write buffer full"
                )));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                conn.web_terminals.remove(&session_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} connection closed"
                )));
            }
        }

        drop(conn);

        let timeout = std::time::Duration::from_secs(self.proxy_timeout_secs);
        match tokio::time::timeout(timeout, ready_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppError::NodeOffline(format!(
                "Node {node_id} disconnected during web terminal open"
            ))),
            Err(_) => {
                if let Some(conn) = self.connections.get(node_id) {
                    conn.web_terminals.remove(&session_id);
                }
                Err(AppError::NodeProxyTimeout)
            }
        }
    }

    /// Forward terminal input bytes to an active web terminal session on a node.
    pub fn send_web_terminal_data(
        &self,
        node_id: &str,
        session_id: &str,
        data: &[u8],
    ) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        if !conn.web_terminals.contains_key(session_id) {
            return Err(AppError::NodeOffline(format!(
                "Web terminal session {session_id} is not active"
            )));
        }

        let msg = serde_json::to_string(&WsWebTerminalData {
            msg_type: "web_terminal_data",
            session_id: session_id.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(data),
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize web terminal data: {e}")))?;

        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;

        Ok(())
    }

    /// Send a resize event to an active web terminal session on a node.
    pub fn send_web_terminal_resize(
        &self,
        node_id: &str,
        session_id: &str,
        cols: u32,
        rows: u32,
    ) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;

        let msg = serde_json::to_string(&WsWebTerminalResize {
            msg_type: "web_terminal_resize",
            session_id: session_id.to_string(),
            cols,
            rows,
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize web terminal resize: {e}")))?;

        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;

        Ok(())
    }

    /// Request closure of an active web terminal session on a node.
    pub fn close_web_terminal(&self, node_id: &str, session_id: &str) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let msg = serde_json::to_string(&WsWebTerminalClose {
            msg_type: "web_terminal_close",
            session_id: session_id.to_string(),
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize web terminal close: {e}")))?;
        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;
        conn.web_terminals.remove(session_id);
        Ok(())
    }

    /// Deliver a web_terminal_started event from a node.
    pub fn deliver_web_terminal_started(&self, node_id: &str, session_id: &str) -> bool {
        let Some(conn) = self.connections.get(node_id) else {
            return false;
        };
        let Some((_, pending)) = conn.web_terminals.remove(session_id) else {
            return false;
        };

        match pending {
            PendingWebTerminal::Awaiting(sender) => {
                let (tx, rx) = mpsc::channel(WEB_TERMINAL_BUFFER_CAPACITY);
                let sent = sender.send(Ok(rx)).is_ok();
                if sent {
                    conn.web_terminals
                        .insert(session_id.to_string(), PendingWebTerminal::Active(tx));
                }
                sent
            }
            PendingWebTerminal::Active(tx) => {
                conn.web_terminals
                    .insert(session_id.to_string(), PendingWebTerminal::Active(tx));
                true
            }
        }
    }

    /// Deliver web terminal output data from a node.
    pub fn deliver_web_terminal_data(&self, node_id: &str, session_id: &str, data: Vec<u8>) {
        if let Some(conn) = self.connections.get(node_id) {
            let send_result = {
                let Some(pending) = conn.web_terminals.get(session_id) else {
                    return;
                };
                let PendingWebTerminal::Active(tx) = pending.value() else {
                    return;
                };
                tx.try_send(WebTerminalChunk::Data(data))
            };

            match send_result {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        session_id = %session_id,
                        capacity = WEB_TERMINAL_BUFFER_CAPACITY,
                        "Dropping web terminal due to full receive buffer"
                    );
                    let close_msg = serde_json::to_string(&WsWebTerminalClose {
                        msg_type: "web_terminal_close",
                        session_id: session_id.to_string(),
                    });
                    if let Ok(close_msg) = close_msg {
                        let _ = conn.tx.try_send(NodeOutboundMessage::Text(close_msg));
                    }
                    conn.web_terminals.remove(session_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    conn.web_terminals.remove(session_id);
                }
            }
        }
    }

    /// Deliver a web_terminal_closed event from a node.
    pub fn deliver_web_terminal_closed(
        &self,
        node_id: &str,
        session_id: &str,
        error: Option<String>,
    ) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.web_terminals.remove(session_id)
        {
            match pending {
                PendingWebTerminal::Awaiting(sender) => {
                    let _ =
                        sender.send(Err(AppError::NodeOffline(error.unwrap_or_else(|| {
                            "Web terminal closed before starting".to_string()
                        }))));
                }
                PendingWebTerminal::Active(tx) => {
                    let _ = tx.try_send(WebTerminalChunk::Closed(error));
                }
            }
        }
    }

    // ---- WebSocket proxy passthrough ----

    /// Open a WS proxy session through a connected node.
    /// Sends `ws_proxy_open` and waits for `ws_proxy_opened` or error.
    pub async fn open_ws_proxy(
        &self,
        node_id: &str,
        request: NodeWsProxyRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<NodeWsProxySession> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let session_id = request.session_id.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        conn.ws_proxies
            .insert(session_id.clone(), PendingWsProxy::Awaiting(ready_tx));

        let (timestamp, nonce, signature) = if let Some(secret) = signing_secret {
            let ts = chrono::Utc::now().to_rfc3339();
            let n = uuid::Uuid::new_v4().to_string();
            let sig = compute_ws_proxy_hmac_signature(
                secret,
                &ts,
                &n,
                &request.session_id,
                &request.service_slug,
                &request.base_url,
                &request.path,
                request.query.as_deref(),
            );
            (Some(ts), Some(n), Some(sig))
        } else {
            (None, None, None)
        };

        let headers_json: serde_json::Value = request
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect::<serde_json::Map<_, _>>()
            .into();

        let msg = serde_json::to_string(&WsProxyOpen {
            msg_type: "ws_proxy_open",
            session_id: request.session_id,
            service_slug: request.service_slug,
            base_url: request.base_url,
            path: request.path,
            query: request.query,
            headers: headers_json,
            ws_frame_injections: request.ws_frame_injections,
            timestamp,
            nonce,
            signature,
        })
        .map_err(|e| {
            conn.ws_proxies.remove(&session_id);
            AppError::Internal(format!("Failed to serialize ws_proxy_open: {e}"))
        })?;

        match conn.tx.try_send(NodeOutboundMessage::Text(msg)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                conn.ws_proxies.remove(&session_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} write buffer full"
                )));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                conn.ws_proxies.remove(&session_id);
                return Err(AppError::NodeOffline(format!(
                    "Node {node_id} connection closed"
                )));
            }
        }

        drop(conn);

        let timeout = std::time::Duration::from_secs(self.proxy_timeout_secs);
        match tokio::time::timeout(timeout, ready_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppError::NodeOffline(format!(
                "Node {node_id} disconnected during WS proxy open"
            ))),
            Err(_) => {
                if let Some(conn) = self.connections.get(node_id) {
                    conn.ws_proxies.remove(&session_id);
                }
                Err(AppError::NodeProxyTimeout)
            }
        }
    }

    /// Forward a text WS frame to a node-backed WS proxy session.
    pub fn send_ws_proxy_text(&self, node_id: &str, session_id: &str, data: &str) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        if !conn.ws_proxies.contains_key(session_id) {
            return Err(AppError::NodeOffline(format!(
                "WS proxy session {session_id} is not active"
            )));
        }
        let msg = serde_json::to_string(&WsProxyTextMsg {
            msg_type: "ws_proxy_text",
            session_id: session_id.to_string(),
            data: data.to_string(),
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize ws_proxy_text: {e}")))?;
        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;
        Ok(())
    }

    /// Forward a binary WS frame to a node-backed WS proxy session.
    pub fn send_ws_proxy_binary(
        &self,
        node_id: &str,
        session_id: &str,
        data: &[u8],
    ) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        if !conn.ws_proxies.contains_key(session_id) {
            return Err(AppError::NodeOffline(format!(
                "WS proxy session {session_id} is not active"
            )));
        }
        let msg = serde_json::to_string(&WsProxyBinaryMsg {
            msg_type: "ws_proxy_binary",
            session_id: session_id.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(data),
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize ws_proxy_binary: {e}")))?;
        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;
        Ok(())
    }

    /// Request closure of a node-backed WS proxy session.
    pub fn send_ws_proxy_close(
        &self,
        node_id: &str,
        session_id: &str,
        code: Option<u16>,
        reason: Option<String>,
    ) -> AppResult<()> {
        let conn = self
            .connections
            .get(node_id)
            .ok_or_else(|| AppError::NodeOffline(format!("Node {node_id} is not connected")))?;
        let msg = serde_json::to_string(&WsProxyCloseMsg {
            msg_type: "ws_proxy_close",
            session_id: session_id.to_string(),
            code,
            reason,
        })
        .map_err(|e| AppError::Internal(format!("Failed to serialize ws_proxy_close: {e}")))?;
        conn.tx
            .try_send(NodeOutboundMessage::Text(msg))
            .map_err(|_| {
                AppError::NodeOffline(format!("Node {node_id} connection closed or buffer full"))
            })?;
        conn.ws_proxies.remove(session_id);
        Ok(())
    }

    /// Deliver a ws_proxy_opened acknowledgement from a node.
    pub fn deliver_ws_proxy_opened(
        &self,
        node_id: &str,
        session_id: &str,
        selected_protocol: Option<String>,
    ) -> bool {
        let Some(conn) = self.connections.get(node_id) else {
            return false;
        };
        let Some((_, pending)) = conn.ws_proxies.remove(session_id) else {
            return false;
        };

        match pending {
            PendingWsProxy::Awaiting(sender) => {
                let (tx, rx) = mpsc::channel(WS_PROXY_BUFFER_CAPACITY);
                let sent = sender
                    .send(Ok(NodeWsProxySession {
                        frames: rx,
                        selected_protocol,
                    }))
                    .is_ok();
                if sent {
                    conn.ws_proxies
                        .insert(session_id.to_string(), PendingWsProxy::Active(tx));
                }
                sent
            }
            PendingWsProxy::Active(tx) => {
                conn.ws_proxies
                    .insert(session_id.to_string(), PendingWsProxy::Active(tx));
                true
            }
        }
    }

    /// Deliver a text WS frame from the downstream through the node.
    pub fn deliver_ws_proxy_text(&self, node_id: &str, session_id: &str, data: String) {
        if let Some(conn) = self.connections.get(node_id) {
            let send_result = {
                let Some(pending) = conn.ws_proxies.get(session_id) else {
                    tracing::trace!(
                        node_id = %node_id,
                        session_id = %session_id,
                        "WS proxy text frame for unknown session"
                    );
                    return;
                };
                let PendingWsProxy::Active(tx) = pending.value() else {
                    return;
                };
                tx.try_send(WsProxyFrame::Text(data))
            };
            match send_result {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        session_id = %session_id,
                        "Dropping WS proxy due to full receive buffer"
                    );
                    conn.ws_proxies.remove(session_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    conn.ws_proxies.remove(session_id);
                }
            }
        }
    }

    /// Deliver a binary WS frame from the downstream through the node.
    pub fn deliver_ws_proxy_binary(&self, node_id: &str, session_id: &str, data: Vec<u8>) {
        if let Some(conn) = self.connections.get(node_id) {
            let send_result = {
                let Some(pending) = conn.ws_proxies.get(session_id) else {
                    tracing::trace!(
                        node_id = %node_id,
                        session_id = %session_id,
                        "WS proxy binary frame for unknown session"
                    );
                    return;
                };
                let PendingWsProxy::Active(tx) = pending.value() else {
                    return;
                };
                tx.try_send(WsProxyFrame::Binary(data))
            };
            match send_result {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        session_id = %session_id,
                        "Dropping WS proxy due to full receive buffer"
                    );
                    conn.ws_proxies.remove(session_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    conn.ws_proxies.remove(session_id);
                }
            }
        }
    }

    /// Deliver a metadata-only WS frame injection signal from the node.
    pub fn deliver_ws_frame_injected(
        &self,
        node_id: &str,
        session_id: &str,
        trigger_kind: String,
        frame_index: usize,
    ) {
        if let Some(conn) = self.connections.get(node_id) {
            let send_result = {
                let Some(pending) = conn.ws_proxies.get(session_id) else {
                    tracing::trace!(
                        node_id = %node_id,
                        session_id = %session_id,
                        "WS frame injection signal for unknown session"
                    );
                    return;
                };
                let PendingWsProxy::Active(tx) = pending.value() else {
                    return;
                };
                tx.try_send(WsProxyFrame::Injected {
                    trigger_kind,
                    frame_index,
                })
            };
            match send_result {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        session_id = %session_id,
                        "Dropping WS proxy due to full receive buffer"
                    );
                    conn.ws_proxies.remove(session_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    conn.ws_proxies.remove(session_id);
                }
            }
        }
    }

    /// Deliver a ws_proxy_closed from the node (downstream closed).
    pub fn deliver_ws_proxy_closed(
        &self,
        node_id: &str,
        session_id: &str,
        code: Option<u16>,
        reason: Option<String>,
    ) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.ws_proxies.remove(session_id)
        {
            match pending {
                PendingWsProxy::Awaiting(sender) => {
                    let _ = sender.send(Err(AppError::NodeOffline(
                        reason.unwrap_or_else(|| "WS proxy closed before opening".to_string()),
                    )));
                }
                PendingWsProxy::Active(tx) => {
                    let _ = tx.try_send(WsProxyFrame::Closed { code, reason });
                }
            }
        }
    }

    /// Deliver a ws_proxy_error from the node.
    pub fn deliver_ws_proxy_error(&self, node_id: &str, session_id: &str, error: &str) {
        if let Some(conn) = self.connections.get(node_id)
            && let Some((_, pending)) = conn.ws_proxies.remove(session_id)
        {
            match pending {
                PendingWsProxy::Awaiting(sender) => {
                    let _ = sender.send(Err(AppError::Internal(error.to_string())));
                }
                PendingWsProxy::Active(tx) => {
                    let _ = tx.try_send(WsProxyFrame::Error(error.to_string()));
                }
            }
        }
    }
}

/// Compute HMAC-SHA256 signature for a WS proxy open request.
#[allow(clippy::too_many_arguments)]
pub fn compute_ws_proxy_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    session_id: &str,
    service_slug: &str,
    base_url: &str,
    path: &str,
    query: Option<&str>,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        timestamp,
        nonce,
        session_id,
        service_slug,
        base_url,
        path,
        query.unwrap_or("")
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn register_and_check_connected() {
        let mgr = NodeWsManager::new(30, 100);
        assert!(!mgr.is_connected("node-1"));

        let (tx, _rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);
        assert!(mgr.is_connected("node-1"));

        mgr.unregister_connection("node-1");
        assert!(!mgr.is_connected("node-1"));
    }

    #[test]
    fn connected_node_ids_returns_all() {
        let mgr = NodeWsManager::new(30, 100);
        let (tx1, _rx1) = mpsc::channel(256);
        let (tx2, _rx2) = mpsc::channel(256);
        mgr.register_connection("node-a", tx1);
        mgr.register_connection("node-b", tx2);

        let mut ids = mgr.connected_node_ids();
        ids.sort();
        assert_eq!(ids, vec!["node-a", "node-b"]);
    }

    #[test]
    fn heartbeat_ping_fails_for_disconnected_node() {
        let mgr = NodeWsManager::new(30, 100);
        assert!(mgr.send_heartbeat_ping("unknown").is_err());
    }

    #[test]
    fn hmac_signature_is_deterministic() {
        let secret = b"test-secret-key-bytes-here-32byt";
        let sig1 = compute_hmac_signature(
            secret,
            "2026-03-12T10:00:00Z",
            "nonce-123",
            "POST",
            "/v1/chat/completions",
            Some("stream=true"),
            Some(b"hello"),
        );
        let sig2 = compute_hmac_signature(
            secret,
            "2026-03-12T10:00:00Z",
            "nonce-123",
            "POST",
            "/v1/chat/completions",
            Some("stream=true"),
            Some(b"hello"),
        );
        assert_eq!(sig1, sig2);
        assert!(!sig1.is_empty());
    }

    #[test]
    fn hmac_signature_changes_with_different_input() {
        let secret = b"test-secret-key-bytes-here-32byt";
        let sig1 = compute_hmac_signature(
            secret,
            "2026-03-12T10:00:00Z",
            "nonce-123",
            "POST",
            "/v1/chat/completions",
            None,
            None,
        );
        let sig2 = compute_hmac_signature(
            secret,
            "2026-03-12T10:00:00Z",
            "nonce-456",
            "POST",
            "/v1/chat/completions",
            None,
            None,
        );
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn ws_proxy_hmac_signature_changes_with_query() {
        let secret = b"test-secret-key-bytes-here-32byt";
        let sig1 = compute_ws_proxy_hmac_signature(
            secret,
            "2026-03-12T10:00:00Z",
            "nonce-123",
            "sess-1",
            "openclaw",
            "https://gateway.example.com",
            "/socket",
            Some("stream=true"),
        );
        let sig2 = compute_ws_proxy_hmac_signature(
            secret,
            "2026-03-12T10:00:00Z",
            "nonce-123",
            "sess-1",
            "openclaw",
            "https://gateway.example.com",
            "/socket",
            Some("stream=false"),
        );

        assert_ne!(sig1, sig2);
    }

    #[tokio::test]
    async fn send_proxy_request_upgrades_to_streaming() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        let mgr_clone = mgr.clone();
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound proxy request");
            };
            let parsed: Value = serde_json::from_str(&msg).expect("valid json");
            let request_id = parsed["request_id"].as_str().expect("request id");
            assert_eq!(parsed["base_url"].as_str(), Some("https://api.example.com"));

            assert!(mgr_clone.deliver_stream_start(
                "node-1",
                request_id,
                200,
                vec![("content-type".to_string(), "text/event-stream".to_string())],
            ));
            mgr_clone.deliver_stream_chunk("node-1", request_id, b"hello".to_vec());
            mgr_clone.deliver_stream_end("node-1", request_id);
        });

        let response = mgr
            .send_proxy_request(
                "node-1",
                NodeProxyRequest {
                    request_id: "req-1".to_string(),
                    service_id: "svc-1".to_string(),
                    service_slug: "demo".to_string(),
                    base_url: "https://api.example.com".to_string(),
                    method: "GET".to_string(),
                    path: "/stream".to_string(),
                    query: None,
                    headers: vec![],
                    body: None,
                },
                None,
            )
            .await
            .expect("streaming response");

        match response {
            ProxyResponseType::Streaming(mut stream) => {
                match stream.recv().await {
                    Some(StreamChunk::Start { status, .. }) => assert_eq!(status, 200),
                    other => panic!("expected stream start, got {other:?}"),
                }
                match stream.recv().await {
                    Some(StreamChunk::Data(bytes)) => assert_eq!(bytes, b"hello".to_vec()),
                    other => panic!("expected stream data, got {other:?}"),
                }
                assert!(matches!(stream.recv().await, Some(StreamChunk::End)));
            }
            ProxyResponseType::Complete(_) => panic!("expected streaming response"),
        }

        responder.await.expect("responder task");
    }

    #[tokio::test]
    async fn send_proxy_request_drops_stream_when_buffer_fills() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        let mgr_clone = mgr.clone();
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound proxy request");
            };
            let parsed: Value = serde_json::from_str(&msg).expect("valid json");
            let request_id = parsed["request_id"].as_str().expect("request id");

            assert!(mgr_clone.deliver_stream_start(
                "node-1",
                request_id,
                200,
                vec![("content-type".to_string(), "text/event-stream".to_string())],
            ));

            for index in 0..STREAM_BUFFER_CAPACITY {
                mgr_clone.deliver_stream_chunk("node-1", request_id, vec![index as u8]);
            }
            mgr_clone.deliver_stream_end("node-1", request_id);
        });

        let response = mgr
            .send_proxy_request(
                "node-1",
                NodeProxyRequest {
                    request_id: "req-buffer".to_string(),
                    service_id: "svc-1".to_string(),
                    service_slug: "demo".to_string(),
                    base_url: "https://api.example.com".to_string(),
                    method: "GET".to_string(),
                    path: "/stream".to_string(),
                    query: None,
                    headers: vec![],
                    body: None,
                },
                None,
            )
            .await
            .expect("streaming response");

        match response {
            ProxyResponseType::Streaming(mut stream) => {
                assert!(matches!(
                    stream.recv().await,
                    Some(StreamChunk::Start { status: 200, .. })
                ));

                let mut data_chunks = 0usize;
                while let Some(chunk) = stream.recv().await {
                    match chunk {
                        StreamChunk::Data(_) => data_chunks += 1,
                        other => panic!("expected data chunk after start, got {other:?}"),
                    }
                }

                assert_eq!(data_chunks, STREAM_BUFFER_CAPACITY - 1);
            }
            ProxyResponseType::Complete(_) => panic!("expected streaming response"),
        }

        responder.await.expect("responder task");
    }

    #[tokio::test]
    async fn retryable_proxy_error_without_reason_is_returned_as_node_offline() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        let mgr_clone = mgr.clone();
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound proxy request");
            };
            let parsed: Value = serde_json::from_str(&msg).expect("valid json");
            let request_id = parsed["request_id"].as_str().expect("request id");

            mgr_clone.deliver_proxy_error(
                "node-1",
                request_id,
                "Transient downstream failure",
                502,
                true,
                None,
            );
        });

        let err = match mgr
            .send_proxy_request(
                "node-1",
                NodeProxyRequest {
                    request_id: "req-2".to_string(),
                    service_id: "svc-1".to_string(),
                    service_slug: "demo".to_string(),
                    base_url: "https://api.example.com".to_string(),
                    method: "GET".to_string(),
                    path: "/models".to_string(),
                    query: None,
                    headers: vec![],
                    body: None,
                },
                None,
            )
            .await
        {
            Ok(_) => panic!("retryable node proxy error should trigger fallback"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            AppError::NodeOffline(message)
                if message.contains("Transient downstream failure")
        ));

        responder.await.expect("responder task");
    }

    #[tokio::test]
    async fn retryable_proxy_error_with_credential_missing_reason_maps_to_credential_missing() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        let mgr_clone = mgr.clone();
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound proxy request");
            };
            let parsed: Value = serde_json::from_str(&msg).expect("valid json");
            let request_id = parsed["request_id"].as_str().expect("request id");

            mgr_clone.deliver_proxy_error(
                "node-1",
                request_id,
                "No credentials configured for service 'demo'",
                502,
                true,
                Some("credential_missing"),
            );
        });

        let err = match mgr
            .send_proxy_request(
                "node-1",
                NodeProxyRequest {
                    request_id: "req-cred-missing".to_string(),
                    service_id: "svc-1".to_string(),
                    service_slug: "demo".to_string(),
                    base_url: "https://api.example.com".to_string(),
                    method: "GET".to_string(),
                    path: "/models".to_string(),
                    query: None,
                    headers: vec![],
                    body: None,
                },
                None,
            )
            .await
        {
            Ok(_) => panic!("credential_missing proxy error should surface as credential missing"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            AppError::NodeCredentialMissing(message)
                if message.contains("No credentials configured for service 'demo'")
        ));

        responder.await.expect("responder task");
    }

    #[tokio::test]
    async fn disconnect_connection_sends_close_frame() {
        let mgr = NodeWsManager::new(30, 100);
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        assert!(
            mgr.disconnect_connection("node-1", 4000, "admin disconnected node")
                .await
        );
        assert!(!mgr.is_connected("node-1"));

        match rx.recv().await {
            Some(NodeOutboundMessage::Close { code, reason }) => {
                assert_eq!(code, 4000);
                assert_eq!(reason, "admin disconnected node");
            }
            other => panic!("expected close message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn disconnect_connection_closes_active_ssh_tunnels() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        let open = tokio::spawn({
            let mgr = Arc::clone(&mgr);
            async move {
                mgr.open_ssh_tunnel(
                    "node-1",
                    NodeSshTunnelRequest {
                        session_id: "sess-disconnect".to_string(),
                        service_id: "svc-1".to_string(),
                        host: "ssh.internal".to_string(),
                        port: 22,
                    },
                    None,
                )
                .await
            }
        });

        let outbound = rx.recv().await.expect("open message");
        match outbound {
            NodeOutboundMessage::Text(text) => {
                let json: Value = serde_json::from_str(&text).expect("json");
                assert_eq!(json["type"], "ssh_tunnel_open");
                assert_eq!(json["session_id"], "sess-disconnect");
            }
            other => panic!("unexpected outbound message: {other:?}"),
        }

        assert!(mgr.deliver_ssh_tunnel_opened("node-1", "sess-disconnect"));
        let mut tunnel_rx = open.await.expect("join").expect("open tunnel");

        assert!(
            mgr.disconnect_connection("node-1", 4001, "forced disconnect")
                .await
        );

        assert!(tunnel_rx.recv().await.is_none());
        match rx.recv().await {
            Some(NodeOutboundMessage::Close { code, reason }) => {
                assert_eq!(code, 4001);
                assert_eq!(reason, "forced disconnect");
            }
            other => panic!("expected close message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn open_ssh_tunnel_delivers_data_and_close() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);

        let open = tokio::spawn({
            let mgr = Arc::clone(&mgr);
            async move {
                mgr.open_ssh_tunnel(
                    "node-1",
                    NodeSshTunnelRequest {
                        session_id: "sess-1".to_string(),
                        service_id: "svc-1".to_string(),
                        host: "ssh.internal".to_string(),
                        port: 22,
                    },
                    None,
                )
                .await
            }
        });

        let outbound = rx.recv().await.expect("open message");
        match outbound {
            NodeOutboundMessage::Text(text) => {
                let json: Value = serde_json::from_str(&text).expect("json");
                assert_eq!(json["type"], "ssh_tunnel_open");
                assert_eq!(json["session_id"], "sess-1");
            }
            other => panic!("unexpected outbound message: {other:?}"),
        }

        assert!(mgr.deliver_ssh_tunnel_opened("node-1", "sess-1"));
        let mut tunnel_rx = open.await.expect("join").expect("open tunnel");

        mgr.deliver_ssh_tunnel_data("node-1", "sess-1", b"hello".to_vec());
        match tunnel_rx.recv().await.expect("data") {
            SshTunnelChunk::Data(bytes) => assert_eq!(bytes, b"hello"),
            other => panic!("unexpected ssh tunnel chunk: {other:?}"),
        }

        mgr.deliver_ssh_tunnel_closed("node-1", "sess-1", Some("done".to_string()));
        match tunnel_rx.recv().await.expect("close") {
            SshTunnelChunk::Closed(Some(error)) => assert_eq!(error, "done"),
            other => panic!("unexpected close chunk: {other:?}"),
        }
    }

    #[tokio::test]
    async fn open_ssh_tunnel_includes_hmac_fields_when_secret_present() {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);
        let signing_secret = vec![0xabu8; 32];

        let open = tokio::spawn({
            let mgr = Arc::clone(&mgr);
            let signing_secret = signing_secret.clone();
            async move {
                mgr.open_ssh_tunnel(
                    "node-1",
                    NodeSshTunnelRequest {
                        session_id: "sess-signed".to_string(),
                        service_id: "svc-1".to_string(),
                        host: "ssh.internal".to_string(),
                        port: 22,
                    },
                    Some(&signing_secret),
                )
                .await
            }
        });

        let outbound = rx.recv().await.expect("open message");
        match outbound {
            NodeOutboundMessage::Text(text) => {
                let json: Value = serde_json::from_str(&text).expect("json");
                assert_eq!(json["type"], "ssh_tunnel_open");
                assert_eq!(json["session_id"], "sess-signed");
                let timestamp = json["timestamp"].as_str().expect("timestamp");
                let nonce = json["nonce"].as_str().expect("nonce");
                let signature = json["signature"].as_str().expect("signature");
                assert_eq!(
                    signature,
                    compute_ssh_tunnel_hmac_signature(
                        &signing_secret,
                        timestamp,
                        nonce,
                        "sess-signed",
                        "svc-1",
                        "ssh.internal",
                        22,
                    )
                );
            }
            other => panic!("unexpected outbound message: {other:?}"),
        }

        assert!(mgr.deliver_ssh_tunnel_opened("node-1", "sess-signed"));
        let mut tunnel_rx = open.await.expect("join").expect("open tunnel");
        mgr.deliver_ssh_tunnel_closed("node-1", "sess-signed", None);
        match tunnel_rx.recv().await.expect("close") {
            SshTunnelChunk::Closed(None) => {}
            other => panic!("unexpected close chunk: {other:?}"),
        }
    }
}
