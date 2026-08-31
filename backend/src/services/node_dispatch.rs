use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::errors::{AppError, AppResult};
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeConnectionOwner};
use crate::services::billing::route_inventory::BillingEgressPermit;
use crate::services::internal_auth::InternalAuth;
use crate::services::node_owner_service::{NodeOwnerFence, ReplicaIdentity};
use crate::services::node_ws_manager::{
    CredentialUpdateParams, NodeProxyFailure, NodeProxyRequest, NodeProxyResponse,
    NodeRequestSignature, NodeSshExecRequest, NodeSshExecResult, NodeSshNodeKeyExecRequest,
    NodeSshTunnelRequest, NodeWebTerminalRequest, NodeWsManager, NodeWsProxyRequest,
    NodeWsProxySession, PendingCredentialCiphertextParams, ProxyResponseType, SshTunnelChunk,
    StreamChunk, WebTerminalChunk, WsProxyFrame, base64_bytes, sign_proxy_request,
    sign_ssh_exec_request, sign_ssh_node_exec_request, sign_ssh_tunnel_request,
    sign_web_terminal_request, sign_ws_proxy_request,
};
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

const INTERNAL_PROXY_KIND: &str = "x-nyxid-proxy-kind";
const INTERNAL_PROXY_STATUS: &str = "x-nyxid-proxy-status";
const INTERNAL_PROXY_HEADERS: &str = "x-nyxid-proxy-headers";
const INTERNAL_DISPATCHED: &str = "x-nyxid-dispatched";
const INTERNAL_PATH_PREFIX: &str = "/internal/v1/nodes";

#[derive(Clone)]
pub struct NodeDispatch {
    db: mongodb::Database,
    manager: Arc<NodeWsManager>,
    identity: Arc<ReplicaIdentity>,
    http_client: reqwest::Client,
    auth: InternalAuth,
    internal_message_limit: usize,
    duplex_handshake_timeout: Duration,
    remote_duplex: Arc<DashMap<String, mpsc::Sender<DuplexClientFrame>>>,
}

impl NodeDispatch {
    pub fn new(
        db: mongodb::Database,
        manager: Arc<NodeWsManager>,
        identity: Arc<ReplicaIdentity>,
        http_client: reqwest::Client,
        auth: InternalAuth,
        internal_message_limit: usize,
        duplex_handshake_timeout: Duration,
    ) -> Self {
        Self {
            db,
            manager,
            identity,
            http_client,
            auth,
            internal_message_limit,
            duplex_handshake_timeout,
            remote_duplex: Arc::new(DashMap::new()),
        }
    }

    pub fn local_manager(&self) -> &Arc<NodeWsManager> {
        &self.manager
    }

    pub async fn session_info(
        &self,
        node_id: &str,
    ) -> crate::services::node_ws_manager::NodeSessionInfo {
        let Ok(Some(node)) = self
            .db
            .collection::<Node>(NODES)
            .find_one(mongodb::bson::doc! { "_id": node_id })
            .await
        else {
            return crate::services::node_ws_manager::NodeSessionInfo::disconnected();
        };
        let Some(owner) =
            crate::services::node_owner_service::live_owner(&node, chrono::Utc::now())
        else {
            return crate::services::node_ws_manager::NodeSessionInfo::disconnected();
        };
        crate::services::node_ws_manager::NodeSessionInfo {
            is_connected: true,
            capabilities_resolved: owner.capabilities_resolved,
            capabilities: crate::services::node_ws_manager::NodeCapabilitiesFlags {
                credential_ack_correlation: owner.credential_ack_correlation,
                remote_credential_crypto_v1: owner.remote_credential_crypto_v1,
                proxy_max_body_size: owner.proxy_max_body_size,
            },
        }
    }

    pub async fn await_capability_resolution(&self, node_id: &str, timeout: Duration) {
        let started = tokio::time::Instant::now();
        loop {
            let info = self.session_info(node_id).await;
            if !info.is_connected || info.capabilities_resolved || started.elapsed() >= timeout {
                return;
            }
            tokio::time::sleep(
                Duration::from_millis(25).min(timeout.saturating_sub(started.elapsed())),
            )
            .await;
        }
    }

    async fn owner_target(&self, node_id: &str) -> AppResult<OwnerTarget> {
        let node = self
            .db
            .collection::<Node>(NODES)
            .find_one(mongodb::bson::doc! { "_id": node_id })
            .await?
            .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))?;
        let owner = crate::services::node_owner_service::live_owner(&node, chrono::Utc::now())
            .cloned()
            .ok_or_else(|| AppError::NodeOffline("Node is not connected".to_string()))?;
        let fence = NodeOwnerFence::from_owner(node_id, &owner);
        match route_for_owner(
            &self.identity,
            &owner,
            self.manager.has_connection(node_id, &owner.connection_id),
        ) {
            OwnerRoute::Local => Ok(OwnerTarget::Local { fence }),
            OwnerRoute::Remote => Ok(OwnerTarget::Remote {
                fence,
                base_url: validated_owner_url(&owner.internal_base_url)?,
            }),
            OwnerRoute::Unavailable => {
                Err(AppError::NodeOffline("Node is not connected".to_string()))
            }
        }
    }

    pub(crate) async fn send_proxy_request_classified(
        &self,
        node_id: &str,
        request: NodeProxyRequest,
        signing_secret: Option<&[u8]>,
        permit: BillingEgressPermit,
    ) -> Result<ProxyResponseType, NodeProxyFailure> {
        let signature = signing_secret.map(|secret| sign_proxy_request(secret, &request));
        let target = self
            .owner_target(node_id)
            .await
            .map_err(NodeProxyFailure::before_dispatch)?;
        match target {
            OwnerTarget::Local { fence } => {
                self.manager
                    .send_proxy_request_classified_prepared(
                        node_id,
                        request,
                        signature,
                        Some(&fence.connection_id),
                        permit,
                    )
                    .await
            }
            OwnerTarget::Remote { fence, base_url } => {
                self.remote_proxy(base_url, fence, request, signature).await
            }
        }
    }

    async fn remote_proxy(
        &self,
        base_url: url::Url,
        fence: NodeOwnerFence,
        request: NodeProxyRequest,
        signature: Option<NodeRequestSignature>,
    ) -> Result<ProxyResponseType, NodeProxyFailure> {
        let node_id = fence.node_id.clone();
        let request_id = request.request_id.clone();
        let path = internal_path(&node_id, "proxy");
        let body = serde_json::to_vec(&ProxyEnvelope {
            fence: fence.clone(),
            request,
            signature,
        })
        .map_err(|error| {
            NodeProxyFailure::before_dispatch(AppError::Internal(format!(
                "Failed to encode internal node proxy request: {error}"
            )))
        })?;
        let headers = self.auth.signed_headers("POST", &path, &body);
        let url = join_internal_url(&base_url, &path).map_err(NodeProxyFailure::before_dispatch)?;
        let cancel_path = internal_path(&node_id, "proxy-cancel");
        let cancel_body = serde_json::to_vec(&CancelEnvelope {
            fence,
            request_id: request_id.clone(),
        })
        .map_err(|error| {
            NodeProxyFailure::before_dispatch(AppError::Internal(format!(
                "Failed to encode internal node cancellation: {error}"
            )))
        })?;
        let cancel_url = join_internal_url(&base_url, &cancel_path)
            .map_err(NodeProxyFailure::before_dispatch)?;
        let mut cancellation = RemoteProxyCancellationGuard {
            request: Some(
                self.http_client
                    .post(cancel_url)
                    .headers(self.auth.signed_headers("POST", &cancel_path, &cancel_body))
                    .body(cancel_body),
            ),
        };
        let response = self
            .http_client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|_| {
                NodeProxyFailure::after_dispatch(AppError::NodeOffline(
                    "Node owner replica is unavailable".to_string(),
                ))
            })?;
        cancellation.disarm();
        if !response.status().is_success() {
            return Err(decode_proxy_failure(response).await);
        }
        let kind = response
            .headers()
            .get(INTERNAL_PROXY_KIND)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let status = response
            .headers()
            .get(INTERNAL_PROXY_STATUS)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| {
                NodeProxyFailure::after_dispatch(AppError::NodeOffline(
                    "Invalid response from node owner replica".to_string(),
                ))
            })?;
        let response_headers =
            decode_proxy_headers(response.headers()).map_err(NodeProxyFailure::after_dispatch)?;
        if kind == "complete" {
            let bytes = response.bytes().await.map_err(|_| {
                NodeProxyFailure::after_dispatch(AppError::NodeOffline(
                    "Node owner replica response ended unexpectedly".to_string(),
                ))
            })?;
            return Ok(ProxyResponseType::Complete(NodeProxyResponse {
                request_id,
                status,
                headers: response_headers,
                body: bytes.to_vec(),
            }));
        }
        if kind != "streaming" {
            return Err(NodeProxyFailure::after_dispatch(AppError::NodeOffline(
                "Invalid response from node owner replica".to_string(),
            )));
        }
        let (tx, rx) = mpsc::channel(1024);
        tx.try_send(StreamChunk::Start {
            status,
            headers: response_headers,
        })
        .map_err(|_| {
            NodeProxyFailure::after_dispatch(AppError::Internal(
                "Failed to initialize internal node stream".to_string(),
            ))
        })?;
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            loop {
                tokio::select! {
                    _ = tx.closed() => break,
                    item = stream.next() => match item {
                        Some(Ok(bytes)) => {
                            if tx.send(StreamChunk::Data(bytes.to_vec())).await.is_err() {
                                break;
                            }
                        }
                        Some(Err(_)) => {
                            let _ = tx.send(StreamChunk::Error(
                                "Node owner replica stream failed".to_string(),
                            )).await;
                            break;
                        }
                        None => {
                            let _ = tx.send(StreamChunk::End).await;
                            break;
                        }
                    }
                }
            }
        });
        Ok(ProxyResponseType::Streaming(rx))
    }

    pub(crate) async fn exec_ssh_command(
        &self,
        node_id: &str,
        request: NodeSshExecRequest,
        signing_secret: Option<&[u8]>,
        permit: BillingEgressPermit,
    ) -> AppResult<NodeSshExecResult> {
        let signature = signing_secret.map(|secret| sign_ssh_exec_request(secret, &request));
        self.exec_prepared(node_id, ExecRequest::Cert { request, signature }, permit)
            .await
    }

    pub(crate) async fn open_ssh_tunnel(
        &self,
        node_id: &str,
        request: NodeSshTunnelRequest,
        signing_secret: Option<&[u8]>,
        permit: BillingEgressPermit,
    ) -> AppResult<mpsc::Receiver<SshTunnelChunk>> {
        let signature = signing_secret.map(|secret| sign_ssh_tunnel_request(secret, &request));
        match self.owner_target(node_id).await? {
            OwnerTarget::Local { fence } => {
                self.manager
                    .open_ssh_tunnel_prepared(
                        node_id,
                        request,
                        signature,
                        Some(&fence.connection_id),
                        permit,
                    )
                    .await
            }
            OwnerTarget::Remote { fence, base_url } => {
                let session_id = request.session_id.clone();
                let (sender, mut incoming, _) = self
                    .open_remote_duplex(
                        base_url,
                        node_id,
                        DuplexOpen::SshTunnel { request, signature },
                        fence,
                    )
                    .await?;
                let key = duplex_key("ssh", node_id, &session_id);
                self.remote_duplex.insert(key.clone(), sender.clone());
                let (tx, rx) = mpsc::channel(256);
                let sessions = self.remote_duplex.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tx.closed() => break,
                            event = incoming.recv() => match event {
                                Some(DuplexServerFrame::Data { data }) => {
                                    if tx.send(SshTunnelChunk::Data(data)).await.is_err() { break; }
                                }
                                Some(DuplexServerFrame::Closed { error, .. }) => {
                                    let _ = tx.send(SshTunnelChunk::Closed(error)).await;
                                    break;
                                }
                                Some(DuplexServerFrame::Failed { failure }) => {
                                    let _ = tx.send(SshTunnelChunk::Closed(Some(
                                        decode_wire_failure(failure).to_string(),
                                    ))).await;
                                    break;
                                }
                                Some(_) => {}
                                None => {
                                    let _ = tx.send(SshTunnelChunk::Closed(Some(
                                        "Node owner replica session closed".to_string(),
                                    ))).await;
                                    break;
                                }
                            }
                        }
                    }
                    sessions.remove(&key);
                    let _ = sender.try_send(DuplexClientFrame::Close {
                        code: None,
                        reason: None,
                    });
                });
                Ok(rx)
            }
        }
    }

    pub fn send_ssh_tunnel_data(
        &self,
        node_id: &str,
        session_id: &str,
        data: &[u8],
    ) -> AppResult<()> {
        if let Some(sender) = self
            .remote_duplex
            .get(&duplex_key("ssh", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Data {
                    data: data.to_vec(),
                })
                .map_err(|_| AppError::NodeOffline("Node tunnel is unavailable".to_string()));
        }
        self.manager.send_ssh_tunnel_data(node_id, session_id, data)
    }

    pub fn close_ssh_tunnel(&self, node_id: &str, session_id: &str) -> AppResult<()> {
        if let Some((_, sender)) = self
            .remote_duplex
            .remove(&duplex_key("ssh", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Close {
                    code: None,
                    reason: None,
                })
                .map_err(|_| AppError::NodeOffline("Node tunnel is unavailable".to_string()));
        }
        self.manager.close_ssh_tunnel(node_id, session_id)
    }

    pub(crate) async fn open_web_terminal(
        &self,
        node_id: &str,
        request: NodeWebTerminalRequest,
        signing_secret: Option<&[u8]>,
        permit: BillingEgressPermit,
    ) -> AppResult<mpsc::Receiver<WebTerminalChunk>> {
        let signature = signing_secret.map(|secret| sign_web_terminal_request(secret, &request));
        match self.owner_target(node_id).await? {
            OwnerTarget::Local { fence } => {
                self.manager
                    .open_web_terminal_prepared(
                        node_id,
                        request,
                        signature,
                        Some(&fence.connection_id),
                        permit,
                    )
                    .await
            }
            OwnerTarget::Remote { fence, base_url } => {
                let session_id = request.session_id.clone();
                let (sender, mut incoming, _) = self
                    .open_remote_duplex(
                        base_url,
                        node_id,
                        DuplexOpen::WebTerminal { request, signature },
                        fence,
                    )
                    .await?;
                let key = duplex_key("terminal", node_id, &session_id);
                self.remote_duplex.insert(key.clone(), sender.clone());
                let (tx, rx) = mpsc::channel(256);
                let sessions = self.remote_duplex.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tx.closed() => break,
                            event = incoming.recv() => match event {
                                Some(DuplexServerFrame::Data { data }) => {
                                    if tx.send(WebTerminalChunk::Data(data)).await.is_err() { break; }
                                }
                                Some(DuplexServerFrame::Closed { error, .. }) => {
                                    let _ = tx.send(WebTerminalChunk::Closed(error)).await;
                                    break;
                                }
                                Some(DuplexServerFrame::Failed { failure }) => {
                                    let _ = tx.send(WebTerminalChunk::Closed(Some(
                                        decode_wire_failure(failure).to_string(),
                                    ))).await;
                                    break;
                                }
                                Some(_) => {}
                                None => {
                                    let _ = tx.send(WebTerminalChunk::Closed(Some(
                                        "Node owner replica session closed".to_string(),
                                    ))).await;
                                    break;
                                }
                            }
                        }
                    }
                    sessions.remove(&key);
                    let _ = sender.try_send(DuplexClientFrame::Close {
                        code: None,
                        reason: None,
                    });
                });
                Ok(rx)
            }
        }
    }

    pub fn send_web_terminal_data(
        &self,
        node_id: &str,
        session_id: &str,
        data: &[u8],
    ) -> AppResult<()> {
        if let Some(sender) = self
            .remote_duplex
            .get(&duplex_key("terminal", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Data {
                    data: data.to_vec(),
                })
                .map_err(|_| AppError::NodeOffline("Node terminal is unavailable".to_string()));
        }
        self.manager
            .send_web_terminal_data(node_id, session_id, data)
    }

    pub fn send_web_terminal_resize(
        &self,
        node_id: &str,
        session_id: &str,
        cols: u32,
        rows: u32,
    ) -> AppResult<()> {
        if let Some(sender) = self
            .remote_duplex
            .get(&duplex_key("terminal", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Resize { cols, rows })
                .map_err(|_| AppError::NodeOffline("Node terminal is unavailable".to_string()));
        }
        self.manager
            .send_web_terminal_resize(node_id, session_id, cols, rows)
    }

    pub fn close_web_terminal(&self, node_id: &str, session_id: &str) -> AppResult<()> {
        if let Some((_, sender)) = self
            .remote_duplex
            .remove(&duplex_key("terminal", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Close {
                    code: None,
                    reason: None,
                })
                .map_err(|_| AppError::NodeOffline("Node terminal is unavailable".to_string()));
        }
        self.manager.close_web_terminal(node_id, session_id)
    }

    pub(crate) async fn open_ws_proxy(
        &self,
        node_id: &str,
        request: NodeWsProxyRequest,
        signing_secret: Option<&[u8]>,
        permit: BillingEgressPermit,
    ) -> AppResult<NodeWsProxySession> {
        let signature = signing_secret.map(|secret| sign_ws_proxy_request(secret, &request));
        match self.owner_target(node_id).await? {
            OwnerTarget::Local { fence } => {
                self.manager
                    .open_ws_proxy_prepared(
                        node_id,
                        request,
                        signature,
                        Some(&fence.connection_id),
                        permit,
                    )
                    .await
            }
            OwnerTarget::Remote { fence, base_url } => {
                let session_id = request.session_id.clone();
                let (sender, mut incoming, selected_protocol) = self
                    .open_remote_duplex(
                        base_url,
                        node_id,
                        DuplexOpen::WsProxy { request, signature },
                        fence,
                    )
                    .await?;
                let key = duplex_key("ws", node_id, &session_id);
                self.remote_duplex.insert(key.clone(), sender.clone());
                let (tx, rx) = mpsc::channel(512);
                let sessions = self.remote_duplex.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tx.closed() => break,
                            event = incoming.recv() => match event {
                                Some(DuplexServerFrame::Text { data }) => {
                                    if tx.send(WsProxyFrame::Text(data)).await.is_err() { break; }
                                }
                                Some(DuplexServerFrame::Data { data }) => {
                                    if tx.send(WsProxyFrame::Binary(data)).await.is_err() { break; }
                                }
                                Some(DuplexServerFrame::Injected { trigger_kind, frame_index }) => {
                                    if tx.send(WsProxyFrame::Injected { trigger_kind, frame_index }).await.is_err() { break; }
                                }
                                Some(DuplexServerFrame::Closed { code, reason, error }) => {
                                    let frame = if let Some(message) = error {
                                        WsProxyFrame::Error(message)
                                    } else {
                                        WsProxyFrame::Closed { code, reason }
                                    };
                                    let _ = tx.send(frame).await;
                                    break;
                                }
                                Some(DuplexServerFrame::Failed { failure }) => {
                                    let _ = tx.send(WsProxyFrame::Error(
                                        decode_wire_failure(failure).to_string(),
                                    )).await;
                                    break;
                                }
                                Some(_) => {}
                                None => {
                                    let _ = tx.send(WsProxyFrame::Error(
                                        "Node owner replica session closed".to_string(),
                                    )).await;
                                    break;
                                }
                            }
                        }
                    }
                    sessions.remove(&key);
                    let _ = sender.try_send(DuplexClientFrame::Close {
                        code: None,
                        reason: None,
                    });
                });
                Ok(NodeWsProxySession {
                    frames: rx,
                    selected_protocol,
                })
            }
        }
    }

    pub fn send_ws_proxy_text(&self, node_id: &str, session_id: &str, data: &str) -> AppResult<()> {
        if let Some(sender) = self
            .remote_duplex
            .get(&duplex_key("ws", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Text {
                    data: data.to_string(),
                })
                .map_err(|_| AppError::NodeOffline("Node WebSocket is unavailable".to_string()));
        }
        self.manager.send_ws_proxy_text(node_id, session_id, data)
    }

    pub fn send_ws_proxy_binary(
        &self,
        node_id: &str,
        session_id: &str,
        data: &[u8],
    ) -> AppResult<()> {
        if let Some(sender) = self
            .remote_duplex
            .get(&duplex_key("ws", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Data {
                    data: data.to_vec(),
                })
                .map_err(|_| AppError::NodeOffline("Node WebSocket is unavailable".to_string()));
        }
        self.manager.send_ws_proxy_binary(node_id, session_id, data)
    }

    pub fn send_ws_proxy_close(
        &self,
        node_id: &str,
        session_id: &str,
        code: Option<u16>,
        reason: Option<String>,
    ) -> AppResult<()> {
        if let Some((_, sender)) = self
            .remote_duplex
            .remove(&duplex_key("ws", node_id, session_id))
        {
            return sender
                .try_send(DuplexClientFrame::Close { code, reason })
                .map_err(|_| AppError::NodeOffline("Node WebSocket is unavailable".to_string()));
        }
        self.manager
            .send_ws_proxy_close(node_id, session_id, code, reason)
    }

    async fn open_remote_duplex(
        &self,
        base_url: url::Url,
        node_id: &str,
        operation: DuplexOpen,
        fence: NodeOwnerFence,
    ) -> AppResult<(
        mpsc::Sender<DuplexClientFrame>,
        mpsc::Receiver<DuplexServerFrame>,
        Option<String>,
    )> {
        let path = internal_path(node_id, "duplex");
        let body = serde_json::to_vec(&DuplexEnvelope { fence, operation }).map_err(|error| {
            AppError::Internal(format!("Failed to encode node duplex request: {error}"))
        })?;
        let mut url = join_internal_url(&base_url, &path)?;
        url.set_scheme(if base_url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| AppError::NodeOffline("Node owner replica is unavailable".to_string()))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| AppError::NodeOffline("Node owner replica is unavailable".to_string()))?;
        request
            .headers_mut()
            .extend(self.auth.signed_headers("GET", &path, &body));
        let websocket_config = WebSocketConfig::default()
            .max_message_size(Some(self.internal_message_limit))
            .max_frame_size(Some(self.internal_message_limit));
        let (mut socket, _) =
            tokio_tungstenite::connect_async_with_config(request, Some(websocket_config), false)
                .await
                .map_err(|_| {
                    AppError::NodeOffline("Node owner replica is unavailable".to_string())
                })?;
        socket
            .send(TungsteniteMessage::Text(
                String::from_utf8(body)
                    .expect("serialized JSON is UTF-8")
                    .into(),
            ))
            .await
            .map_err(|_| AppError::NodeOffline("Node owner replica is unavailable".to_string()))?;
        let first = tokio::time::timeout(self.duplex_handshake_timeout, socket.next())
            .await
            .ok()
            .flatten()
            .and_then(Result::ok)
            .and_then(tungstenite_json)
            .ok_or_else(|| {
                AppError::NodeOffline("Node owner replica rejected session".to_string())
            })?;
        let selected_protocol = match first {
            DuplexServerFrame::Opened { selected_protocol } => selected_protocol,
            DuplexServerFrame::Failed { failure } => return Err(decode_wire_failure(failure)),
            _ => {
                return Err(AppError::NodeOffline(
                    "Invalid response from node owner replica".to_string(),
                ));
            }
        };
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<DuplexClientFrame>(256);
        let (incoming_tx, incoming_rx) = mpsc::channel::<DuplexServerFrame>(512);
        let (mut sink, mut stream) = socket.split();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = incoming_tx.closed() => break,
                    frame = outgoing_rx.recv() => match frame {
                        Some(frame) => {
                            let Ok(json) = serde_json::to_string(&frame) else { break; };
                            if sink.send(TungsteniteMessage::Text(json.into())).await.is_err() { break; }
                        }
                        None => break,
                    },
                    frame = stream.next() => match frame {
                        Some(Ok(message)) => {
                            let Some(frame) = tungstenite_json(message) else { break; };
                            if incoming_tx.send(frame).await.is_err() { break; }
                        }
                        _ => break,
                    }
                }
            }
            let _ = sink.close().await;
        });
        Ok((outgoing_tx, incoming_rx, selected_protocol))
    }

    pub(crate) async fn exec_ssh_node_key_command(
        &self,
        node_id: &str,
        request: NodeSshNodeKeyExecRequest,
        signing_secret: Option<&[u8]>,
        permit: BillingEgressPermit,
    ) -> AppResult<NodeSshExecResult> {
        let signature = signing_secret.map(|secret| sign_ssh_node_exec_request(secret, &request));
        self.exec_prepared(node_id, ExecRequest::NodeKey { request, signature }, permit)
            .await
    }

    async fn exec_prepared(
        &self,
        node_id: &str,
        request: ExecRequest,
        permit: BillingEgressPermit,
    ) -> AppResult<NodeSshExecResult> {
        match self.owner_target(node_id).await? {
            OwnerTarget::Local { fence } => match request {
                ExecRequest::Cert { request, signature } => {
                    self.manager
                        .exec_ssh_command_prepared(
                            node_id,
                            request,
                            signature,
                            Some(&fence.connection_id),
                            permit,
                        )
                        .await
                }
                ExecRequest::NodeKey { request, signature } => {
                    self.manager
                        .exec_ssh_node_key_command_prepared(
                            node_id,
                            request,
                            signature,
                            Some(&fence.connection_id),
                            permit,
                        )
                        .await
                }
            },
            OwnerTarget::Remote { fence, base_url } => {
                let path = internal_path(node_id, "exec");
                let body =
                    serde_json::to_vec(&ExecEnvelope { fence, request }).map_err(|error| {
                        AppError::Internal(format!(
                            "Failed to encode internal SSH request: {error}"
                        ))
                    })?;
                let response = self
                    .http_client
                    .post(join_internal_url(&base_url, &path)?)
                    .headers(self.auth.signed_headers("POST", &path, &body))
                    .body(body)
                    .send()
                    .await
                    .map_err(|_| {
                        AppError::NodeOffline("Node owner replica is unavailable".to_string())
                    })?;
                decode_json_response(response).await
            }
        }
    }

    pub async fn send_credential_update(
        &self,
        node_id: &str,
        params: &CredentialUpdateParams,
    ) -> AppResult<()> {
        self.send_command(
            node_id,
            NodeCommand::CredentialUpdate {
                params: params.clone(),
                timeout_ms: None,
            },
        )
        .await
    }

    pub async fn send_credential_update_and_wait(
        &self,
        node_id: &str,
        params: &CredentialUpdateParams,
        timeout: Duration,
    ) -> AppResult<()> {
        self.send_command(
            node_id,
            NodeCommand::CredentialUpdate {
                params: params.clone(),
                timeout_ms: Some(duration_millis(timeout)?),
            },
        )
        .await
    }

    pub async fn send_credential_remove(&self, node_id: &str, service_slug: &str) -> AppResult<()> {
        self.send_command(
            node_id,
            NodeCommand::CredentialRemove {
                service_slug: service_slug.to_string(),
                timeout_ms: None,
            },
        )
        .await
    }

    pub async fn send_credential_remove_and_wait(
        &self,
        node_id: &str,
        service_slug: &str,
        timeout: Duration,
    ) -> AppResult<()> {
        self.send_command(
            node_id,
            NodeCommand::CredentialRemove {
                service_slug: service_slug.to_string(),
                timeout_ms: Some(duration_millis(timeout)?),
            },
        )
        .await
    }

    pub async fn send_pending_credentials_available(&self, node_id: &str) -> AppResult<()> {
        self.send_command(node_id, NodeCommand::PendingCredentialsAvailable)
            .await
    }

    pub async fn send_pending_credential_ciphertext(
        &self,
        node_id: &str,
        params: &PendingCredentialCiphertextParams<'_>,
    ) -> AppResult<()> {
        self.send_command(
            node_id,
            NodeCommand::PendingCredentialCiphertext {
                pending_id: params.pending_id.to_string(),
                version: params.version.to_string(),
                admin_pubkey: params.admin_pubkey.to_string(),
                nonce: params.nonce.to_string(),
                ciphertext: params.ciphertext.to_string(),
            },
        )
        .await
    }

    async fn send_command(&self, node_id: &str, command: NodeCommand) -> AppResult<()> {
        match self.owner_target(node_id).await? {
            OwnerTarget::Local { fence } => {
                execute_local_command(&self.manager, node_id, &fence.connection_id, command).await
            }
            OwnerTarget::Remote { fence, base_url } => {
                let path = internal_path(node_id, "command");
                let body =
                    serde_json::to_vec(&CommandEnvelope { fence, command }).map_err(|error| {
                        AppError::Internal(format!("Failed to encode node command: {error}"))
                    })?;
                let response = self
                    .http_client
                    .post(join_internal_url(&base_url, &path)?)
                    .headers(self.auth.signed_headers("POST", &path, &body))
                    .body(body)
                    .send()
                    .await
                    .map_err(|_| {
                        AppError::NodeOffline("Node owner replica is unavailable".to_string())
                    })?;
                let _: EmptyResponse = decode_json_response(response).await?;
                Ok(())
            }
        }
    }

    pub async fn disconnect(&self, node_id: &str, code: u16, reason: &str) -> AppResult<bool> {
        let Some(owner) =
            crate::services::node_owner_service::invalidate_current(&self.db, node_id).await?
        else {
            return Ok(false);
        };
        let fence = owner.fence;
        if fence.instance_name == self.identity.instance_name
            && fence.generation_id == self.identity.generation_id
        {
            return Ok(self
                .manager
                .disconnect_connection_if(node_id, &fence.connection_id, code, reason)
                .await);
        }
        self.remote_disconnect(
            validated_owner_url(&owner.internal_base_url)?,
            fence,
            code,
            reason,
        )
        .await
    }

    async fn remote_disconnect(
        &self,
        base_url: url::Url,
        fence: NodeOwnerFence,
        code: u16,
        reason: &str,
    ) -> AppResult<bool> {
        let path = internal_path(&fence.node_id, "disconnect");
        let body = serde_json::to_vec(&DisconnectEnvelope {
            fence,
            code,
            reason: reason.to_string(),
        })
        .map_err(|error| AppError::Internal(format!("Failed to encode disconnect: {error}")))?;
        let response = self
            .http_client
            .post(join_internal_url(&base_url, &path)?)
            .headers(self.auth.signed_headers("POST", &path, &body))
            .body(body)
            .send()
            .await
            .map_err(|_| AppError::NodeOffline("Node owner replica is unavailable".to_string()))?;
        let result: DisconnectResponse = decode_json_response(response).await?;
        Ok(result.disconnected)
    }
}

enum OwnerTarget {
    Local {
        fence: NodeOwnerFence,
    },
    Remote {
        fence: NodeOwnerFence,
        base_url: url::Url,
    },
}

#[derive(Serialize, Deserialize)]
struct ProxyEnvelope {
    fence: NodeOwnerFence,
    request: NodeProxyRequest,
    signature: Option<NodeRequestSignature>,
}

#[derive(Serialize, Deserialize)]
struct ExecEnvelope {
    fence: NodeOwnerFence,
    request: ExecRequest,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExecRequest {
    Cert {
        request: NodeSshExecRequest,
        signature: Option<NodeRequestSignature>,
    },
    NodeKey {
        request: NodeSshNodeKeyExecRequest,
        signature: Option<NodeRequestSignature>,
    },
}

#[derive(Serialize, Deserialize)]
struct CommandEnvelope {
    fence: NodeOwnerFence,
    command: NodeCommand,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NodeCommand {
    CredentialUpdate {
        params: CredentialUpdateParams,
        timeout_ms: Option<u64>,
    },
    CredentialRemove {
        service_slug: String,
        timeout_ms: Option<u64>,
    },
    PendingCredentialsAvailable,
    PendingCredentialCiphertext {
        pending_id: String,
        version: String,
        admin_pubkey: String,
        nonce: String,
        ciphertext: String,
    },
}

#[derive(Serialize, Deserialize)]
struct DisconnectEnvelope {
    fence: NodeOwnerFence,
    code: u16,
    reason: String,
}

#[derive(Serialize, Deserialize)]
struct CancelEnvelope {
    fence: NodeOwnerFence,
    request_id: String,
}

#[derive(Serialize, Deserialize)]
struct EmptyResponse {
    ok: bool,
}

#[derive(Serialize, Deserialize)]
struct DisconnectResponse {
    disconnected: bool,
}

#[derive(Serialize, Deserialize)]
struct DuplexEnvelope {
    fence: NodeOwnerFence,
    operation: DuplexOpen,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DuplexOpen {
    SshTunnel {
        request: NodeSshTunnelRequest,
        signature: Option<NodeRequestSignature>,
    },
    WebTerminal {
        request: NodeWebTerminalRequest,
        signature: Option<NodeRequestSignature>,
    },
    WsProxy {
        request: NodeWsProxyRequest,
        signature: Option<NodeRequestSignature>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DuplexClientFrame {
    Data {
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
    },
    Text {
        data: String,
    },
    Resize {
        cols: u32,
        rows: u32,
    },
    Close {
        code: Option<u16>,
        reason: Option<String>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DuplexServerFrame {
    Opened {
        selected_protocol: Option<String>,
    },
    Data {
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
    },
    Text {
        data: String,
    },
    Injected {
        trigger_kind: String,
        frame_index: usize,
    },
    Closed {
        code: Option<u16>,
        reason: Option<String>,
        error: Option<String>,
    },
    Failed {
        failure: WireFailure,
    },
}

#[derive(Serialize, Deserialize)]
struct WireFailure {
    code: WireErrorCode,
    dispatched: bool,
    max_bytes: Option<usize>,
    error_code: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireErrorCode {
    NodeOffline,
    NodeProxyTimeout,
    NodeCredentialMissing,
    RequestBodyTooLarge,
    SshNodeKeyMissing,
    SshHostKeyMismatch,
    SshNodeExecChannelClosed,
    SshAuthModeUnsupported,
    ClientDisconnected,
    Internal,
}

#[derive(Clone, Copy)]
struct InternalDuplexConfig {
    max_message_bytes: usize,
    handshake_timeout: Duration,
}

pub fn internal_router(
    dispatch: Arc<NodeDispatch>,
    max_body_bytes: usize,
    handshake_timeout: Duration,
) -> Router {
    Router::new()
        .route("/internal/v1/nodes/{node_id}/proxy", post(internal_proxy))
        .route(
            "/internal/v1/nodes/{node_id}/proxy-cancel",
            post(internal_proxy_cancel),
        )
        .route("/internal/v1/nodes/{node_id}/exec", post(internal_exec))
        .route(
            "/internal/v1/nodes/{node_id}/command",
            post(internal_command),
        )
        .route(
            "/internal/v1/nodes/{node_id}/disconnect",
            post(internal_disconnect),
        )
        .route("/internal/v1/nodes/{node_id}/duplex", get(internal_duplex))
        .layer(axum::Extension(InternalDuplexConfig {
            max_message_bytes: max_body_bytes,
            handshake_timeout,
        }))
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(dispatch)
}

async fn internal_duplex(
    State(dispatch): State<Arc<NodeDispatch>>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    axum::Extension(config): axum::Extension<InternalDuplexConfig>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let path = internal_path(&node_id, "duplex");
    let Some(authenticated_body) = dispatch
        .auth
        .authenticate_headers(&headers, "GET", &path)
        .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    upgrade
        .max_message_size(config.max_message_bytes)
        .on_upgrade(move |socket| async move {
            serve_internal_duplex(
                dispatch,
                node_id,
                authenticated_body,
                config.handshake_timeout,
                socket,
            )
            .await;
        })
}

async fn serve_internal_duplex(
    dispatch: Arc<NodeDispatch>,
    node_id: String,
    authenticated_body: crate::services::internal_auth::AuthenticatedBody,
    handshake_timeout: Duration,
    mut socket: WebSocket,
) {
    let Ok(Some(Ok(AxumWsMessage::Text(first)))) =
        tokio::time::timeout(handshake_timeout, socket.next()).await
    else {
        return;
    };
    if !authenticated_body.matches(first.as_bytes()) {
        let _ = socket.close().await;
        return;
    }
    let Ok(envelope) = serde_json::from_str::<DuplexEnvelope>(&first) else {
        let _ = socket.close().await;
        return;
    };
    if !authorize_live_local_fence(&dispatch, &node_id, &envelope.fence).await {
        let _ = socket.close().await;
        return;
    }
    let expected_connection_id = envelope.fence.connection_id;
    match envelope.operation {
        DuplexOpen::SshTunnel { request, signature } => {
            let session_id = request.session_id.clone();
            match dispatch
                .manager
                .open_ssh_tunnel_prepared(
                    &node_id,
                    request,
                    signature,
                    Some(&expected_connection_id),
                    crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                )
                .await
            {
                Ok(receiver) => {
                    if send_axum_json(
                        &mut socket,
                        &DuplexServerFrame::Opened {
                            selected_protocol: None,
                        },
                    )
                    .await
                    {
                        bridge_internal_ssh(
                            &dispatch.manager,
                            &node_id,
                            &session_id,
                            receiver,
                            socket,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    let _ = send_axum_json(
                        &mut socket,
                        &DuplexServerFrame::Failed {
                            failure: encode_wire_failure(&error, true),
                        },
                    )
                    .await;
                }
            }
        }
        DuplexOpen::WebTerminal { request, signature } => {
            let session_id = request.session_id.clone();
            match dispatch
                .manager
                .open_web_terminal_prepared(
                    &node_id,
                    request,
                    signature,
                    Some(&expected_connection_id),
                    crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                )
                .await
            {
                Ok(receiver) => {
                    if send_axum_json(
                        &mut socket,
                        &DuplexServerFrame::Opened {
                            selected_protocol: None,
                        },
                    )
                    .await
                    {
                        bridge_internal_terminal(
                            &dispatch.manager,
                            &node_id,
                            &session_id,
                            receiver,
                            socket,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    let _ = send_axum_json(
                        &mut socket,
                        &DuplexServerFrame::Failed {
                            failure: encode_wire_failure(&error, true),
                        },
                    )
                    .await;
                }
            }
        }
        DuplexOpen::WsProxy { request, signature } => {
            let session_id = request.session_id.clone();
            match dispatch
                .manager
                .open_ws_proxy_prepared(
                    &node_id,
                    request,
                    signature,
                    Some(&expected_connection_id),
                    crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                )
                .await
            {
                Ok(session) => {
                    if send_axum_json(
                        &mut socket,
                        &DuplexServerFrame::Opened {
                            selected_protocol: session.selected_protocol,
                        },
                    )
                    .await
                    {
                        bridge_internal_ws_proxy(
                            &dispatch.manager,
                            &node_id,
                            &session_id,
                            session.frames,
                            socket,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    let _ = send_axum_json(
                        &mut socket,
                        &DuplexServerFrame::Failed {
                            failure: encode_wire_failure(&error, true),
                        },
                    )
                    .await;
                }
            }
        }
    }
}

async fn bridge_internal_ssh(
    manager: &NodeWsManager,
    node_id: &str,
    session_id: &str,
    mut receiver: mpsc::Receiver<SshTunnelChunk>,
    mut socket: WebSocket,
) {
    loop {
        tokio::select! {
            inbound = socket.next() => match inbound.and_then(Result::ok).and_then(axum_json) {
                Some(DuplexClientFrame::Data { data }) => {
                    if manager.send_ssh_tunnel_data(node_id, session_id, &data).is_err() { break; }
                }
                Some(DuplexClientFrame::Close { .. }) | None => break,
                _ => {}
            },
            outbound = receiver.recv() => match outbound {
                Some(SshTunnelChunk::Data(data)) => {
                    if !send_axum_json(&mut socket, &DuplexServerFrame::Data { data }).await { break; }
                }
                Some(SshTunnelChunk::Closed(error)) => {
                    let _ = send_axum_json(&mut socket, &DuplexServerFrame::Closed {
                        code: None,
                        reason: None,
                        error,
                    }).await;
                    break;
                }
                None => break,
            }
        }
    }
    let _ = manager.close_ssh_tunnel(node_id, session_id);
    let _ = socket.close().await;
}

async fn bridge_internal_terminal(
    manager: &NodeWsManager,
    node_id: &str,
    session_id: &str,
    mut receiver: mpsc::Receiver<WebTerminalChunk>,
    mut socket: WebSocket,
) {
    loop {
        tokio::select! {
            inbound = socket.next() => match inbound.and_then(Result::ok).and_then(axum_json) {
                Some(DuplexClientFrame::Data { data }) => {
                    if manager.send_web_terminal_data(node_id, session_id, &data).is_err() { break; }
                }
                Some(DuplexClientFrame::Resize { cols, rows }) => {
                    if manager.send_web_terminal_resize(node_id, session_id, cols, rows).is_err() { break; }
                }
                Some(DuplexClientFrame::Close { .. }) | None => break,
                _ => {}
            },
            outbound = receiver.recv() => match outbound {
                Some(WebTerminalChunk::Data(data)) => {
                    if !send_axum_json(&mut socket, &DuplexServerFrame::Data { data }).await { break; }
                }
                Some(WebTerminalChunk::Closed(error)) => {
                    let _ = send_axum_json(&mut socket, &DuplexServerFrame::Closed {
                        code: None,
                        reason: None,
                        error,
                    }).await;
                    break;
                }
                None => break,
            }
        }
    }
    let _ = manager.close_web_terminal(node_id, session_id);
    let _ = socket.close().await;
}

async fn bridge_internal_ws_proxy(
    manager: &NodeWsManager,
    node_id: &str,
    session_id: &str,
    mut receiver: mpsc::Receiver<WsProxyFrame>,
    mut socket: WebSocket,
) {
    let mut close_code = None;
    let mut close_reason = None;
    loop {
        tokio::select! {
            inbound = socket.next() => match inbound.and_then(Result::ok).and_then(axum_json) {
                Some(DuplexClientFrame::Data { data }) => {
                    if manager.send_ws_proxy_binary(node_id, session_id, &data).is_err() { break; }
                }
                Some(DuplexClientFrame::Text { data }) => {
                    if manager.send_ws_proxy_text(node_id, session_id, &data).is_err() { break; }
                }
                Some(DuplexClientFrame::Close { code, reason }) => {
                    close_code = code;
                    close_reason = reason;
                    break;
                }
                None => break,
                _ => {}
            },
            outbound = receiver.recv() => match outbound {
                Some(WsProxyFrame::Text(data)) => {
                    if !send_axum_json(&mut socket, &DuplexServerFrame::Text { data }).await { break; }
                }
                Some(WsProxyFrame::Binary(data)) => {
                    if !send_axum_json(&mut socket, &DuplexServerFrame::Data { data }).await { break; }
                }
                Some(WsProxyFrame::Injected { trigger_kind, frame_index }) => {
                    if !send_axum_json(&mut socket, &DuplexServerFrame::Injected { trigger_kind, frame_index }).await { break; }
                }
                Some(WsProxyFrame::Closed { code, reason }) => {
                    let _ = send_axum_json(&mut socket, &DuplexServerFrame::Closed {
                        code,
                        reason,
                        error: None,
                    }).await;
                    break;
                }
                Some(WsProxyFrame::Error(error)) => {
                    let _ = send_axum_json(&mut socket, &DuplexServerFrame::Closed {
                        code: None,
                        reason: None,
                        error: Some(error),
                    }).await;
                    break;
                }
                None => break,
            }
        }
    }
    let _ = manager.send_ws_proxy_close(node_id, session_id, close_code, close_reason);
    let _ = socket.close().await;
}

async fn send_axum_json<T: Serialize>(socket: &mut WebSocket, value: &T) -> bool {
    let Ok(json) = serde_json::to_string(value) else {
        return false;
    };
    socket.send(AxumWsMessage::Text(json.into())).await.is_ok()
}

fn axum_json<T: for<'de> Deserialize<'de>>(message: AxumWsMessage) -> Option<T> {
    match message {
        AxumWsMessage::Text(text) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

fn tungstenite_json<T: for<'de> Deserialize<'de>>(message: TungsteniteMessage) -> Option<T> {
    match message {
        TungsteniteMessage::Text(text) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

fn duplex_key(kind: &str, node_id: &str, session_id: &str) -> String {
    format!("{kind}:{node_id}:{session_id}")
}

async fn internal_proxy(
    State(dispatch): State<Arc<NodeDispatch>>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = internal_path(&node_id, "proxy");
    if !dispatch
        .auth
        .authenticate(&headers, "POST", &path, &body)
        .await
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(envelope) = serde_json::from_slice::<ProxyEnvelope>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !authorize_live_local_fence(&dispatch, &node_id, &envelope.fence).await {
        return StatusCode::CONFLICT.into_response();
    }
    let request_id = envelope.request.request_id.clone();
    let expected_connection_id = envelope.fence.connection_id;
    let result = dispatch
        .manager
        .send_proxy_request_classified_prepared(
            &node_id,
            envelope.request,
            envelope.signature,
            Some(&expected_connection_id),
            crate::services::billing::route_inventory::internal_node_dispatch_permit(),
        )
        .await;
    match result {
        Ok(ProxyResponseType::Complete(response)) => proxy_response(
            "complete",
            response.status,
            &response.headers,
            Body::from(response.body),
        ),
        Ok(ProxyResponseType::Streaming(mut rx)) => {
            let first = rx.recv().await;
            let Some(StreamChunk::Start { status, headers }) = first else {
                return wire_failure_response(
                    AppError::NodeOffline("Node stream closed before start".to_string()),
                    true,
                );
            };
            let manager = dispatch.manager.clone();
            let stream_node_id = node_id.clone();
            let stream = async_stream::stream! {
                let _guard = ProxyCancellationGuard {
                    manager,
                    node_id: stream_node_id,
                    request_id,
                };
                while let Some(chunk) = rx.recv().await {
                    match chunk {
                        StreamChunk::Data(data) => yield Ok::<Bytes, std::io::Error>(Bytes::from(data)),
                        StreamChunk::End => break,
                        StreamChunk::Error(message) => {
                            yield Err(std::io::Error::other(message));
                            break;
                        }
                        StreamChunk::Start { .. } => {
                            yield Err(std::io::Error::other("duplicate node stream start"));
                            break;
                        }
                    }
                }
            };
            proxy_response("streaming", status, &headers, Body::from_stream(stream))
        }
        Err(failure) => wire_failure_response(failure.error, failure.dispatched),
    }
}

async fn internal_proxy_cancel(
    State(dispatch): State<Arc<NodeDispatch>>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = internal_path(&node_id, "proxy-cancel");
    if !dispatch
        .auth
        .authenticate(&headers, "POST", &path, &body)
        .await
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(envelope) = serde_json::from_slice::<CancelEnvelope>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if envelope.fence.node_id != node_id
        || envelope.fence.instance_name != dispatch.identity.instance_name
        || envelope.fence.generation_id != dispatch.identity.generation_id
    {
        return StatusCode::CONFLICT.into_response();
    }
    match dispatch.manager.cancel_proxy_request_if(
        &node_id,
        &envelope.fence.connection_id,
        &envelope.request_id,
    ) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::CONFLICT.into_response(),
    }
}

async fn internal_exec(
    State(dispatch): State<Arc<NodeDispatch>>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = internal_path(&node_id, "exec");
    if !dispatch
        .auth
        .authenticate(&headers, "POST", &path, &body)
        .await
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(envelope) = serde_json::from_slice::<ExecEnvelope>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !authorize_live_local_fence(&dispatch, &node_id, &envelope.fence).await {
        return StatusCode::CONFLICT.into_response();
    }
    let expected_connection_id = envelope.fence.connection_id;
    let permit = crate::services::billing::route_inventory::internal_node_dispatch_permit();
    let result = match envelope.request {
        ExecRequest::Cert { request, signature } => {
            dispatch
                .manager
                .exec_ssh_command_prepared(
                    &node_id,
                    request,
                    signature,
                    Some(&expected_connection_id),
                    permit,
                )
                .await
        }
        ExecRequest::NodeKey { request, signature } => {
            dispatch
                .manager
                .exec_ssh_node_key_command_prepared(
                    &node_id,
                    request,
                    signature,
                    Some(&expected_connection_id),
                    permit,
                )
                .await
        }
    };
    json_result(result)
}

async fn internal_command(
    State(dispatch): State<Arc<NodeDispatch>>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = internal_path(&node_id, "command");
    if !dispatch
        .auth
        .authenticate(&headers, "POST", &path, &body)
        .await
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(envelope) = serde_json::from_slice::<CommandEnvelope>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !authorize_live_local_fence(&dispatch, &node_id, &envelope.fence).await {
        return StatusCode::CONFLICT.into_response();
    }
    match execute_local_command(
        &dispatch.manager,
        &node_id,
        &envelope.fence.connection_id,
        envelope.command,
    )
    .await
    {
        Ok(()) => axum::Json(EmptyResponse { ok: true }).into_response(),
        Err(error) => wire_failure_response(error, false),
    }
}

async fn internal_disconnect(
    State(dispatch): State<Arc<NodeDispatch>>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = internal_path(&node_id, "disconnect");
    if !dispatch
        .auth
        .authenticate(&headers, "POST", &path, &body)
        .await
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(envelope) = serde_json::from_slice::<DisconnectEnvelope>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if envelope.fence.node_id != node_id
        || envelope.fence.instance_name != dispatch.identity.instance_name
        || envelope.fence.generation_id != dispatch.identity.generation_id
    {
        return StatusCode::CONFLICT.into_response();
    }
    let disconnected = dispatch
        .manager
        .disconnect_connection_if(
            &node_id,
            &envelope.fence.connection_id,
            envelope.code,
            &envelope.reason,
        )
        .await;
    axum::Json(DisconnectResponse { disconnected }).into_response()
}

async fn authorize_live_local_fence(
    dispatch: &NodeDispatch,
    node_id: &str,
    fence: &NodeOwnerFence,
) -> bool {
    fence.node_id == node_id
        && fence.instance_name == dispatch.identity.instance_name
        && fence.generation_id == dispatch.identity.generation_id
        && dispatch
            .manager
            .has_connection(node_id, &fence.connection_id)
        && crate::services::node_owner_service::matches_live_fence(
            &dispatch.db,
            fence,
            chrono::Utc::now(),
        )
        .await
        .unwrap_or(false)
}

async fn execute_local_command(
    manager: &NodeWsManager,
    node_id: &str,
    connection_id: &str,
    command: NodeCommand,
) -> AppResult<()> {
    match command {
        NodeCommand::CredentialUpdate { params, timeout_ms } => match timeout_ms {
            Some(timeout) => {
                manager
                    .send_credential_update_and_wait_if(
                        node_id,
                        &params,
                        Duration::from_millis(timeout),
                        connection_id,
                    )
                    .await
            }
            None => manager.send_credential_update_if(node_id, &params, connection_id),
        },
        NodeCommand::CredentialRemove {
            service_slug,
            timeout_ms,
        } => match timeout_ms {
            Some(timeout) => {
                manager
                    .send_credential_remove_and_wait_if(
                        node_id,
                        &service_slug,
                        Duration::from_millis(timeout),
                        connection_id,
                    )
                    .await
            }
            None => manager.send_credential_remove_if(node_id, &service_slug, connection_id),
        },
        NodeCommand::PendingCredentialsAvailable => {
            manager.send_pending_credentials_available_if(node_id, connection_id)
        }
        NodeCommand::PendingCredentialCiphertext {
            pending_id,
            version,
            admin_pubkey,
            nonce,
            ciphertext,
        } => manager.send_pending_credential_ciphertext_if(
            node_id,
            &PendingCredentialCiphertextParams {
                pending_id: &pending_id,
                version: &version,
                admin_pubkey: &admin_pubkey,
                nonce: &nonce,
                ciphertext: &ciphertext,
            },
            connection_id,
        ),
    }
}

struct ProxyCancellationGuard {
    manager: Arc<NodeWsManager>,
    node_id: String,
    request_id: String,
}

struct RemoteProxyCancellationGuard {
    request: Option<reqwest::RequestBuilder>,
}

impl RemoteProxyCancellationGuard {
    fn disarm(&mut self) {
        self.request = None;
    }
}

impl Drop for RemoteProxyCancellationGuard {
    fn drop(&mut self) {
        let Some(request) = self.request.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = request.send().await;
            });
        }
    }
}

impl Drop for ProxyCancellationGuard {
    fn drop(&mut self) {
        self.manager
            .cancel_proxy_request(&self.node_id, &self.request_id);
    }
}

fn proxy_response(
    kind: &'static str,
    status: u16,
    headers: &[(String, String)],
    body: Body,
) -> Response {
    let encoded_headers = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(headers).expect("proxy response headers serialize"));
    let mut response = Response::new(body);
    response
        .headers_mut()
        .insert(INTERNAL_PROXY_KIND, HeaderValue::from_static(kind));
    if let Ok(value) = HeaderValue::from_str(&status.to_string()) {
        response.headers_mut().insert(INTERNAL_PROXY_STATUS, value);
    }
    match HeaderValue::from_str(&encoded_headers) {
        Ok(value) => {
            response.headers_mut().insert(INTERNAL_PROXY_HEADERS, value);
        }
        Err(_) => {
            return wire_failure_response(
                AppError::Internal("Node response headers are too large".to_string()),
                true,
            );
        }
    }
    response
}

fn decode_proxy_headers(headers: &HeaderMap) -> AppResult<Vec<(String, String)>> {
    let encoded = headers
        .get(INTERNAL_PROXY_HEADERS)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::NodeOffline("Invalid response from node owner replica".to_string())
        })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            AppError::NodeOffline("Invalid response from node owner replica".to_string())
        })?;
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::NodeOffline("Invalid response from node owner replica".to_string()))
}

fn wire_failure_response(error: AppError, dispatched: bool) -> Response {
    let failure = encode_wire_failure(&error, dispatched);
    let mut response = (StatusCode::BAD_GATEWAY, axum::Json(failure)).into_response();
    response.headers_mut().insert(
        HeaderName::from_static(INTERNAL_DISPATCHED),
        HeaderValue::from_static(if dispatched { "true" } else { "false" }),
    );
    response
}

fn json_result<T: Serialize>(result: AppResult<T>) -> Response {
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => wire_failure_response(error, true),
    }
}

fn encode_wire_failure(error: &AppError, dispatched: bool) -> WireFailure {
    let (code, max_bytes, error_code) = match error {
        AppError::NodeOffline(_) => (WireErrorCode::NodeOffline, None, None),
        AppError::NodeProxyTimeout => (WireErrorCode::NodeProxyTimeout, None, None),
        AppError::NodeCredentialMissing(_) => (WireErrorCode::NodeCredentialMissing, None, None),
        AppError::RequestBodyTooLarge { max_bytes, .. } => {
            (WireErrorCode::RequestBodyTooLarge, Some(*max_bytes), None)
        }
        AppError::SshNodeKeyMissing(_) => (WireErrorCode::SshNodeKeyMissing, None, None),
        AppError::SshHostKeyMismatch(_) => (WireErrorCode::SshHostKeyMismatch, None, None),
        AppError::SshNodeExecChannelClosed(_) => {
            (WireErrorCode::SshNodeExecChannelClosed, None, None)
        }
        AppError::SshAuthModeUnsupportedForOperation(_) => {
            (WireErrorCode::SshAuthModeUnsupported, None, None)
        }
        AppError::ClientDisconnected => (WireErrorCode::ClientDisconnected, None, None),
        _ => (WireErrorCode::Internal, None, None),
    };
    WireFailure {
        code,
        dispatched,
        max_bytes,
        error_code,
    }
}

fn decode_wire_failure(failure: WireFailure) -> AppError {
    match failure.code {
        WireErrorCode::NodeOffline => AppError::NodeOffline("Node is unavailable".to_string()),
        WireErrorCode::NodeProxyTimeout => AppError::NodeProxyTimeout,
        WireErrorCode::NodeCredentialMissing => {
            AppError::NodeCredentialMissing("Node credential is unavailable".to_string())
        }
        WireErrorCode::RequestBodyTooLarge => AppError::RequestBodyTooLarge {
            max_bytes: failure.max_bytes.unwrap_or(0),
            context: "Node proxy".to_string(),
        },
        WireErrorCode::SshNodeKeyMissing => {
            AppError::SshNodeKeyMissing("Node SSH key is unavailable".to_string())
        }
        WireErrorCode::SshHostKeyMismatch => {
            AppError::SshHostKeyMismatch("SSH host key mismatch".to_string())
        }
        WireErrorCode::SshNodeExecChannelClosed => {
            AppError::SshNodeExecChannelClosed("Node SSH channel closed".to_string())
        }
        WireErrorCode::SshAuthModeUnsupported => AppError::SshAuthModeUnsupportedForOperation(
            "SSH authentication mode is unsupported".to_string(),
        ),
        WireErrorCode::ClientDisconnected => AppError::ClientDisconnected,
        WireErrorCode::Internal => AppError::Internal("Internal node dispatch failed".to_string()),
    }
}

async fn decode_proxy_failure(response: reqwest::Response) -> NodeProxyFailure {
    let dispatched = response
        .headers()
        .get(INTERNAL_DISPATCHED)
        .and_then(|value| value.to_str().ok())
        == Some("true");
    let error = match response.json::<WireFailure>().await {
        Ok(failure) => decode_wire_failure(failure),
        Err(_) => AppError::NodeOffline("Node owner replica rejected the request".to_string()),
    };
    if dispatched {
        NodeProxyFailure::after_dispatch(error)
    } else {
        NodeProxyFailure::before_dispatch(error)
    }
}

async fn decode_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> AppResult<T> {
    if response.status().is_success() {
        return response.json::<T>().await.map_err(|_| {
            AppError::NodeOffline("Invalid response from node owner replica".to_string())
        });
    }
    match response.json::<WireFailure>().await {
        Ok(failure) => Err(decode_wire_failure(failure)),
        Err(_) => Err(AppError::NodeOffline(
            "Node owner replica rejected the request".to_string(),
        )),
    }
}

fn internal_path(node_id: &str, operation: &str) -> String {
    format!("{INTERNAL_PATH_PREFIX}/{node_id}/{operation}")
}

fn validated_owner_url(raw: &str) -> AppResult<url::Url> {
    let url = url::Url::parse(raw)
        .map_err(|_| AppError::NodeOffline("Node owner replica is unavailable".to_string()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::NodeOffline(
            "Node owner replica is unavailable".to_string(),
        ));
    }
    Ok(url)
}

fn join_internal_url(base: &url::Url, path: &str) -> AppResult<url::Url> {
    base.join(path)
        .map_err(|_| AppError::NodeOffline("Node owner replica is unavailable".to_string()))
}

fn duration_millis(duration: Duration) -> AppResult<u64> {
    u64::try_from(duration.as_millis())
        .map_err(|_| AppError::Internal("Node command timeout is too large".to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerRoute {
    Local,
    Remote,
    Unavailable,
}

pub fn route_for_owner(
    identity: &ReplicaIdentity,
    owner: &NodeConnectionOwner,
    exact_local_connection_exists: bool,
) -> OwnerRoute {
    if owner.instance_name == identity.instance_name
        && owner.generation_id == identity.generation_id
    {
        if exact_local_connection_exists {
            OwnerRoute::Local
        } else {
            OwnerRoute::Unavailable
        }
    } else {
        OwnerRoute::Remote
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::NodeConnectionOwner;
    use chrono::{Duration, Utc};

    fn owner(instance_name: &str, generation_id: &str) -> NodeConnectionOwner {
        let now = Utc::now();
        NodeConnectionOwner {
            instance_name: instance_name.to_string(),
            generation_id: generation_id.to_string(),
            connection_id: "connection-a".to_string(),
            internal_base_url: "http://10.0.0.8:3002".to_string(),
            claimed_at: now,
            renewed_at: now,
            expires_at: now + Duration::seconds(30),
            credential_ack_correlation: true,
            remote_credential_crypto_v1: true,
            proxy_max_body_size: Some(1024),
            capabilities_resolved: true,
        }
    }

    #[test]
    fn routing_requires_the_exact_local_process_and_connection() {
        let identity = crate::services::node_owner_service::ReplicaIdentity {
            instance_name: "backend-a".to_string(),
            generation_id: "generation-a".to_string(),
            internal_base_url: "http://10.0.0.7:3002".to_string(),
        };
        let local = owner("backend-a", "generation-a");
        let stale_process = owner("backend-a", "generation-old");
        let remote = owner("backend-b", "generation-b");

        assert_eq!(route_for_owner(&identity, &local, true), OwnerRoute::Local);
        assert_eq!(
            route_for_owner(&identity, &local, false),
            OwnerRoute::Unavailable
        );
        assert_eq!(
            route_for_owner(&identity, &stale_process, true),
            OwnerRoute::Remote
        );
        assert_eq!(
            route_for_owner(&identity, &remote, false),
            OwnerRoute::Remote
        );
    }
}
