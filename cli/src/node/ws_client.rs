use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use zeroize::Zeroizing;

use super::config::{NodeConfig, SshConfig};
use super::credential_store::{CredentialStore, SharedCredentials, SharedCredentialsSender};
use super::error::{Error, Result};
use super::metrics::NodeMetrics;
use super::proxy_executor;
use super::secret_backend::SecretBackend;
use super::signing::{self, ReplayGuard};
use super::ws_frame_injector::{
    self, IncomingFrame, InjectorState, WsFrameDirection, WsFrameInjection, WsFrameKind,
};

// ---------------------------------------------------------------------------
// Web terminal types
// ---------------------------------------------------------------------------

const WEB_TERMINAL_PTY_READ_BUF: usize = 16 * 1024;

/// Maximum bytes captured per output stream (stdout / stderr) for ssh_exec.
const SSH_EXEC_MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB

type ActiveWebTerminalMap = Arc<tokio::sync::Mutex<HashMap<String, ActiveWebTerminal>>>;

struct ActiveWebTerminal {
    pty_writer: pty_process::OwnedWritePty,
    child: tokio::process::Child,
    task_handle: tokio::task::JoinHandle<()>,
    _temp_dir: tempfile::TempDir,
}

enum SshTunnelControl {
    Data(Vec<u8>),
    Close,
}

enum WsProxyControl {
    Text(String),
    Binary(Vec<u8>),
    Close {
        code: Option<u16>,
        reason: Option<String>,
    },
}

type ActiveWsProxyMap = Arc<tokio::sync::Mutex<HashMap<String, ActiveWsProxyEntry>>>;

struct ActiveWsProxyEntry {
    control_tx: mpsc::Sender<WsProxyControl>,
    task_handle: tokio::task::JoinHandle<()>,
}

const WS_PROXY_CONTROL_CHANNEL_SIZE: usize = 256;
const WS_PROXY_CONNECT_TIMEOUT_SECS: u64 = 10;
/// Must match server-side WS_PASSTHROUGH_MAX_DURATION_SECS.
const WS_PROXY_MAX_DURATION_SECS: u64 = 3600;
/// Must match server-side WS_PASSTHROUGH_IDLE_TIMEOUT_SECS.
const WS_PROXY_IDLE_TIMEOUT_SECS: u64 = 300;
/// Must match server-side WS_PASSTHROUGH_MAX_MESSAGE_SIZE.
const WS_PROXY_MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

const SSH_CONTROL_CHANNEL_SIZE: usize = 256;
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;
const SSH_SHUTDOWN_DRAIN_TIMEOUT_SECS: u64 = 5;
const AGENT_SHUTDOWN_TIMEOUT_SECS: u64 = 30;
const WS_WRITE_CHANNEL_SIZE: usize = 256;
/// Multiplier applied to the advertised server heartbeat interval to compute
/// the idle watchdog: we allow ~3 missed pings before declaring the socket
/// dead. For the default 30s interval this yields 90s, matching the prior
/// hard-coded behavior while respecting custom server settings.
const WS_READ_IDLE_TIMEOUT_MULTIPLIER: u64 = 3;
/// Absolute floor for the idle watchdog. Prevents pathologically short
/// timeouts if a server advertises a very small heartbeat interval.
const WS_READ_IDLE_TIMEOUT_FLOOR_SECS: u64 = 30;
/// Absolute ceiling for the idle watchdog. Even if a server advertises an
/// extremely long heartbeat interval we cap the watchdog at one hour so a
/// silently dead connection is eventually detected.
const WS_READ_IDLE_TIMEOUT_CEILING_SECS: u64 = 3600;

/// Compute the WebSocket read-idle timeout from the server-advertised
/// heartbeat interval. Returns `None` when the server did not advertise an
/// interval (older backend); in that case the idle watchdog is disabled so
/// we don't regress deployments that run with `NODE_HEARTBEAT_INTERVAL_SECS`
/// larger than our assumed default. Defined as a pure helper so it can be
/// unit-tested.
fn compute_ws_read_idle_timeout_secs(server_heartbeat_interval_secs: Option<u64>) -> Option<u64> {
    server_heartbeat_interval_secs.map(|interval| {
        interval
            .saturating_mul(WS_READ_IDLE_TIMEOUT_MULTIPLIER)
            .clamp(
                WS_READ_IDLE_TIMEOUT_FLOOR_SECS,
                WS_READ_IDLE_TIMEOUT_CEILING_SECS,
            )
    })
}

/// WebSocket message to send: either a JSON text frame or raw binary data.
/// Text frames carry control messages (JSON, human-readable for debugging).
/// Binary frames carry streaming data chunks (zero encoding overhead).
pub enum NodeWsMessage {
    Text(String),
    Binary(Vec<u8>),
}

type SharedSigningSecret = Arc<Zeroizing<String>>;
type ActiveSshTunnelMap = Arc<tokio::sync::Mutex<HashMap<String, ActiveSshTunnelEntry>>>;

struct ActiveSshTunnelEntry {
    control_tx: mpsc::Sender<SshTunnelControl>,
    task_handle: tokio::task::JoinHandle<()>,
}

impl ActiveSshTunnelEntry {
    fn control_tx(&self) -> mpsc::Sender<SshTunnelControl> {
        self.control_tx.clone()
    }

    fn abort(self) {
        self.task_handle.abort();
    }
}

/// Exponential backoff state for reconnection.
struct ExponentialBackoff {
    current: Duration,
    initial: Duration,
    max: Duration,
    multiplier: f64,
}

impl ExponentialBackoff {
    fn new(initial: Duration, max: Duration, multiplier: f64) -> Self {
        Self {
            current: initial,
            initial,
            max,
            multiplier,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        let next_ms = (self.current.as_millis() as f64 * self.multiplier) as u64;
        self.current = Duration::from_millis(next_ms).min(self.max);
        delay
    }

    fn reset(&mut self) {
        self.current = self.initial;
    }
}

/// Register a node using a one-time registration token.
/// Returns (node_id, auth_token, signing_secret).
pub async fn register_node(
    ws_url: &str,
    registration_token: &str,
) -> Result<(String, String, Option<String>)> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| Error::WebSocket(format!("Failed to connect: {e}")))?;

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Send register message
    let register_msg = serde_json::json!({
        "type": "register",
        "token": registration_token,
    });
    ws_sink
        .send(Message::Text(register_msg.to_string().into()))
        .await
        .map_err(|e| Error::WebSocket(format!("Failed to send register message: {e}")))?;

    // Wait for response
    let response = tokio::time::timeout(Duration::from_secs(10), ws_stream.next())
        .await
        .map_err(|_| {
            Error::RegistrationFailed("Timed out waiting for server response".to_string())
        })?
        .ok_or_else(|| Error::RegistrationFailed("Connection closed".to_string()))?
        .map_err(|e| Error::WebSocket(format!("Read error: {e}")))?;

    let text = match response {
        Message::Text(t) => t.to_string(),
        _ => {
            return Err(Error::RegistrationFailed(
                "Unexpected message type".to_string(),
            ));
        }
    };

    let parsed: serde_json::Value = serde_json::from_str(&text)?;

    match parsed["type"].as_str() {
        Some("register_ok") => {
            let node_id = parsed["node_id"]
                .as_str()
                .ok_or_else(|| Error::RegistrationFailed("Missing node_id".to_string()))?
                .to_string();
            let auth_token = parsed["auth_token"]
                .as_str()
                .ok_or_else(|| Error::RegistrationFailed("Missing auth_token".to_string()))?
                .to_string();
            let signing_secret = parsed["signing_secret"].as_str().map(String::from);

            // Close connection cleanly
            let _ = ws_sink.send(Message::Close(None)).await;

            Ok((node_id, auth_token, signing_secret))
        }
        Some("auth_error") => {
            let msg = parsed["message"].as_str().unwrap_or("Unknown error");
            Err(Error::RegistrationFailed(msg.to_string()))
        }
        _ => Err(Error::RegistrationFailed(format!(
            "Unexpected response: {text}"
        ))),
    }
}

/// Run the agent with graceful shutdown on SIGINT/SIGTERM.
pub async fn run_with_shutdown(
    config: NodeConfig,
    config_path: std::path::PathBuf,
    auth_token: String,
    signing_secret: Option<String>,
    credentials: SharedCredentials,
    credential_sender: Arc<SharedCredentialsSender>,
    backend: Arc<SecretBackend>,
) {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let in_flight_shutdown = in_flight.clone();
    let signing_secret = signing_secret.map(|secret| Arc::new(Zeroizing::new(secret)));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let config_dir = config_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let storage_backend = config.storage_backend.clone();
    drop(backend);
    let mut connection_task = tokio::spawn({
        let in_flight = in_flight.clone();
        async move {
            run_connection_loop(
                &config,
                &config_path,
                &config_dir,
                &storage_backend,
                &auth_token,
                signing_secret,
                &credentials,
                &credential_sender,
                in_flight,
                shutdown_rx,
            )
            .await;
        }
    });

    tokio::select! {
        result = &mut connection_task => {
            if let Err(error) = result {
                tracing::error!(%error, "Connection loop terminated unexpectedly");
            }
        }
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received, draining in-flight requests and SSH tunnels...");
            let _ = shutdown_tx.send(true);
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(AGENT_SHUTDOWN_TIMEOUT_SECS);
            while tokio::time::Instant::now() < deadline {
                if connection_task.is_finished() && in_flight_shutdown.load(Ordering::Relaxed) == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let remaining = in_flight_shutdown.load(Ordering::Relaxed);
            if remaining > 0 {
                tracing::warn!(remaining, "Forcing shutdown with in-flight requests");
            }
            if !connection_task.is_finished() {
                tracing::warn!("Forcing shutdown while the connection loop is still running");
                connection_task.abort();
            }
            if let Err(error) = connection_task.await
                && !error.is_cancelled()
            {
                tracing::error!(%error, "Connection loop terminated unexpectedly during shutdown");
            }
            tracing::info!("Shutdown complete");
        }
    }
}

/// Main connection loop with exponential backoff reconnection.
#[allow(clippy::too_many_arguments)]
async fn run_connection_loop(
    config: &NodeConfig,
    config_path: &std::path::Path,
    config_dir: &std::path::Path,
    storage_backend: &str,
    auth_token: &str,
    signing_secret: Option<SharedSigningSecret>,
    credentials: &SharedCredentials,
    credential_sender: &Arc<SharedCredentialsSender>,
    in_flight: Arc<AtomicUsize>,
    shutdown: watch::Receiver<bool>,
) {
    let mut backoff =
        ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(60), 2.0);

    loop {
        if shutdown_requested(&shutdown) {
            break;
        }

        match connect_and_serve(
            config,
            config_path,
            config_dir,
            storage_backend,
            auth_token,
            signing_secret.clone(),
            credentials,
            credential_sender,
            in_flight.clone(),
            shutdown.clone(),
        )
        .await
        {
            Ok(()) => {
                if shutdown_requested(&shutdown) {
                    break;
                }
                tracing::info!("Disconnected cleanly, reconnecting...");
                backoff.reset();
            }
            Err(e) => {
                if shutdown_requested(&shutdown) {
                    break;
                }
                let delay = backoff.next_delay();
                tracing::warn!(
                    error = %e,
                    delay_ms = delay.as_millis(),
                    "Connection failed, retrying"
                );
                let mut shutdown_wait = shutdown.clone();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = wait_for_shutdown(&mut shutdown_wait) => break,
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Single connection lifecycle: connect, authenticate, serve requests.
async fn connect_and_serve(
    config: &NodeConfig,
    config_path: &std::path::Path,
    config_dir: &std::path::Path,
    storage_backend: &str,
    auth_token: &str,
    signing_secret: Option<SharedSigningSecret>,
    credentials: &SharedCredentials,
    credential_sender: &Arc<SharedCredentialsSender>,
    in_flight: Arc<AtomicUsize>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    // 1. Connect
    let connect = tokio_tungstenite::connect_async(&config.server.url);
    tokio::pin!(connect);
    let (ws_stream, _) = tokio::select! {
        result = &mut connect => {
            result.map_err(|e| Error::WebSocket(format!("Failed to connect: {e}")))?
        }
        _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
    };

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // 2. Authenticate
    let auth_msg = serde_json::json!({
        "type": "auth",
        "node_id": config.node.id,
        "token": auth_token,
    });
    tokio::select! {
        result = ws_sink.send(Message::Text(auth_msg.to_string().into())) => {
            result.map_err(|e| Error::WebSocket(format!("Failed to send auth: {e}")))?;
        }
        _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
    }

    // 3. Wait for auth_ok
    let response = tokio::select! {
        response = tokio::time::timeout(Duration::from_secs(10), ws_stream.next()) => {
            response
                .map_err(|_| Error::AuthFailed("Timed out waiting for auth response".to_string()))?
                .ok_or_else(|| Error::AuthFailed("Connection closed during auth".to_string()))?
                .map_err(|e| Error::WebSocket(format!("Read error during auth: {e}")))?
        }
        _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
    };

    let text = match response {
        Message::Text(t) => t.to_string(),
        _ => return Err(Error::AuthFailed("Unexpected message type".to_string())),
    };

    let parsed: serde_json::Value = serde_json::from_str(&text)?;
    let (use_binary_proxy_chunks, server_heartbeat_interval_secs) = match parsed["type"].as_str() {
        Some("auth_ok") => {
            let enabled = parsed["capabilities"]["proxy_binary_chunks"]
                .as_bool()
                .unwrap_or(false);
            // Only accept the interval when the server explicitly advertises
            // it. An older backend that doesn't include `heartbeat_interval_secs`
            // leaves this as None, which disables the idle watchdog below --
            // preserving pre-patch behavior during mixed-version rollouts so
            // deployments that customize NODE_HEARTBEAT_INTERVAL_SECS above
            // our assumed default don't start flapping.
            let interval = parsed["heartbeat_interval_secs"]
                .as_u64()
                .filter(|v| *v > 0);
            tracing::info!(
                node_id = %config.node.id,
                proxy_binary_chunks = enabled,
                heartbeat_interval_secs = ?interval,
                "Authenticated with NyxID server"
            );
            (enabled, interval)
        }
        Some("auth_error") => {
            let msg = parsed["message"].as_str().unwrap_or("unknown");
            return Err(Error::AuthFailed(msg.to_string()));
        }
        _ => {
            return Err(Error::AuthFailed(format!("Unexpected response: {text}")));
        }
    };

    // Derive the idle watchdog from the server's heartbeat cadence so
    // installations that customize NODE_HEARTBEAT_INTERVAL_SECS don't trigger
    // spurious reconnects. None when the server didn't advertise: we leave
    // the read call blocking indefinitely in that case (pre-patch behavior).
    let ws_read_idle_timeout_secs =
        compute_ws_read_idle_timeout_secs(server_heartbeat_interval_secs);

    let proxy_http_client = proxy_executor::build_http_client()?;

    // 4. Set up writer channel
    let (tx, mut rx) = mpsc::channel::<NodeWsMessage>(WS_WRITE_CHANNEL_SIZE);
    let active_ssh_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
        String,
        ActiveSshTunnelEntry,
    >::new()));
    let active_web_terminals: ActiveWebTerminalMap =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let active_ws_proxies: ActiveWsProxyMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Writer task: forwards messages from the channel to the WS sink.
    // Text frames carry JSON control messages; binary frames carry raw data chunks.
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let ws_msg = match msg {
                NodeWsMessage::Text(text) => Message::Text(text.into()),
                NodeWsMessage::Binary(data) => Message::Binary(data.into()),
            };
            if ws_sink.send(ws_msg).await.is_err() {
                break;
            }
        }
    });

    // Advertise supported capabilities to the server immediately after
    // auth so the backend can enable features that require node-side
    // cooperation (e.g., strict ack-wait on `credential_update` via
    // `request_id` echo). Older backends ignore the `capabilities`
    // field; newer backends only enable the feature for nodes that
    // advertise the matching flag. Fire-and-forget — if the channel
    // is full we'll retry on the next status_update / reconnect.
    let caps_msg = serde_json::json!({
        "type": "status_update",
        "agent_version": env!("CARGO_PKG_VERSION"),
        "capabilities": {
            "credential_ack_correlation": true,
        },
    });
    let _ = send_ws_message(&tx, caps_msg.to_string()).await;

    // Shared state for the reader loop
    let metrics = Arc::new(NodeMetrics::new());
    let replay_guard = Arc::new(tokio::sync::Mutex::new(ReplayGuard::new()));

    // 5. Reader loop: process incoming messages from the server
    let shutting_down = loop {
        // Wrap ws_stream.next() with an idle timeout when the server
        // advertised its heartbeat interval. If it did not (older backend),
        // we fall back to blocking indefinitely -- this is the pre-patch
        // behavior and intentionally leaves the silent-hang bug unfixed for
        // mixed-version rollouts rather than risking healthy-node flapping.
        let read_result = match ws_read_idle_timeout_secs {
            Some(secs) => {
                match tokio::select! {
                    result = tokio::time::timeout(Duration::from_secs(secs), ws_stream.next()) => result,
                    _ = wait_for_shutdown(&mut shutdown) => break true,
                } {
                    Ok(msg) => msg,
                    Err(_) => {
                        // No frame of any kind (including server heartbeat_ping)
                        // within the derived idle window -- the underlying TCP
                        // connection is almost certainly dead. Break to force
                        // the outer connection loop to reconnect.
                        tracing::warn!(
                            idle_secs = secs,
                            server_heartbeat_interval_secs = ?server_heartbeat_interval_secs,
                            "No WebSocket frames received within idle timeout; assuming connection is dead, reconnecting"
                        );
                        break false;
                    }
                }
            }
            None => tokio::select! {
                msg = ws_stream.next() => msg,
                _ = wait_for_shutdown(&mut shutdown) => break true,
            },
        };
        let Some(msg) = read_result else {
            break false;
        };
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break false,
            Ok(Message::Ping(_)) => continue,
            Ok(_) => continue,
            Err(e) => {
                tracing::debug!(error = %e, "WebSocket read error");
                break false;
            }
        };

        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "Invalid message from server");
                continue;
            }
        };

        match parsed["type"].as_str() {
            Some("heartbeat_ping") => {
                let pong = serde_json::json!({
                    "type": "heartbeat_pong",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });
                if !send_ws_message(&tx, pong.to_string()).await {
                    break false;
                }
            }
            Some("proxy_request") => {
                let tx_clone = tx.clone();
                let creds = credentials.snapshot();
                let secret = signing_secret.clone();
                let replay = replay_guard.clone();
                let metrics_clone = metrics.clone();
                let http_client = proxy_http_client.clone();
                let in_flight_clone = in_flight.clone();

                in_flight_clone.fetch_add(1, Ordering::Relaxed);

                tokio::spawn(async move {
                    proxy_executor::execute_proxy_request(
                        &parsed,
                        &creds,
                        secret.as_deref().map(|secret| secret.as_str()),
                        &replay,
                        &metrics_clone,
                        &tx_clone,
                        use_binary_proxy_chunks,
                        &http_client,
                    )
                    .await;
                    in_flight_clone.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Some("ssh_tunnel_open") => {
                let tx_clone = tx.clone();
                let ssh_config = config.ssh.clone();
                let active_tunnels = active_ssh_tunnels.clone();
                let secret = signing_secret.clone();
                let replay = replay_guard.clone();
                tokio::spawn(async move {
                    handle_ssh_tunnel_open(
                        &parsed,
                        &ssh_config,
                        tx_clone,
                        active_tunnels,
                        secret,
                        replay,
                    )
                    .await;
                });
            }
            Some("ssh_tunnel_data") => {
                handle_ssh_tunnel_data(&parsed, &tx, &active_ssh_tunnels).await;
            }
            Some("ssh_tunnel_close") => {
                handle_ssh_tunnel_close(&parsed, &active_ssh_tunnels).await;
            }
            Some("web_terminal_open") => {
                let tx_clone = tx.clone();
                let ssh_config = config.ssh.clone();
                let terminals = active_web_terminals.clone();
                let secret = signing_secret.clone();
                let replay = replay_guard.clone();
                tokio::spawn(async move {
                    handle_web_terminal_open(
                        &parsed,
                        &ssh_config,
                        tx_clone,
                        terminals,
                        secret,
                        replay,
                    )
                    .await;
                });
            }
            Some("web_terminal_data") => {
                handle_web_terminal_data(&parsed, &active_web_terminals).await;
            }
            Some("web_terminal_resize") => {
                handle_web_terminal_resize(&parsed, &active_web_terminals).await;
            }
            Some("web_terminal_close") => {
                handle_web_terminal_close(&parsed, &tx, &active_web_terminals).await;
            }
            Some("ssh_exec") => {
                let tx_clone = tx.clone();
                let ssh_config = config.ssh.clone();
                let secret = signing_secret.clone();
                let replay = replay_guard.clone();
                tokio::spawn(async move {
                    handle_ssh_exec(&parsed, &ssh_config, tx_clone, secret, replay).await;
                });
            }
            Some("credential_update") => {
                // Process credential update synchronously (SecretBackend is
                // not Send/Sync, so we reconstruct it within this block and
                // ensure it's dropped before any .await).
                let ack_msg = {
                    match SecretBackend::from_storage_backend_str(storage_backend, config_dir) {
                        Ok(be) => {
                            process_credential_update(&parsed, credential_sender, config_path, &be)
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to init secret backend for credential update");
                            None
                        }
                    }
                };
                // Send ack after backend is dropped (so we can .await)
                if let Some(ack) = ack_msg {
                    let _ = send_ws_message(&tx, ack).await;
                }
            }
            Some("credential_remove") => {
                // Drop the locally-stored credential for this slug. Sent
                // when a service's `node_id` is reassigned so the prior
                // node stops holding the secret. SecretBackend is not
                // Send/Sync, so we scope its lifetime here and do any
                // awaits after.
                let ack_msg = {
                    match SecretBackend::from_storage_backend_str(storage_backend, config_dir) {
                        Ok(be) => {
                            process_credential_remove(&parsed, credential_sender, config_path, &be)
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to init secret backend for credential remove");
                            None
                        }
                    }
                };
                if let Some(ack) = ack_msg {
                    let _ = send_ws_message(&tx, ack).await;
                }
            }
            Some("ws_proxy_open") => {
                let tx_clone = tx.clone();
                let creds = credentials.snapshot();
                let secret = signing_secret.clone();
                let replay = replay_guard.clone();
                let ws_proxies = active_ws_proxies.clone();
                tokio::spawn(async move {
                    handle_ws_proxy_open(&parsed, &creds, tx_clone, ws_proxies, secret, replay)
                        .await;
                });
            }
            Some("ws_proxy_text") => {
                handle_ws_proxy_text(&parsed, &tx, &active_ws_proxies).await;
            }
            Some("ws_proxy_binary") => {
                handle_ws_proxy_binary(&parsed, &tx, &active_ws_proxies).await;
            }
            Some("ws_proxy_close") => {
                handle_ws_proxy_close(&parsed, &tx, &active_ws_proxies).await;
            }
            Some("error") => {
                let msg = parsed["message"].as_str().unwrap_or("unknown");
                tracing::error!(message = %msg, "Server error");
            }
            other => {
                tracing::debug!(msg_type = ?other, "Unknown message type");
            }
        }
    };

    if shutting_down {
        close_active_ssh_tunnels(
            &active_ssh_tunnels,
            Duration::from_secs(SSH_SHUTDOWN_DRAIN_TIMEOUT_SECS),
        )
        .await;
    } else {
        drain_active_ssh_tunnels(&active_ssh_tunnels).await;
    }
    drain_active_web_terminals(&active_web_terminals).await;
    drain_active_ws_proxies(&active_ws_proxies).await;
    writer_task.abort();
    Ok(())
}

async fn handle_ssh_tunnel_open(
    parsed: &serde_json::Value,
    ssh_config: &SshConfig,
    tx: mpsc::Sender<NodeWsMessage>,
    active_tunnels: ActiveSshTunnelMap,
    signing_secret: Option<SharedSigningSecret>,
    replay_guard: Arc<tokio::sync::Mutex<ReplayGuard>>,
) {
    let session_id = match parsed["session_id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            tracing::warn!("ssh_tunnel_open missing session_id");
            return;
        }
    };

    if let Some(secret) = signing_secret.as_deref()
        && let Err(error) =
            verify_signed_ssh_tunnel_open(parsed, &session_id, secret.as_str(), &replay_guard).await
    {
        let _ = send_ssh_tunnel_closed(&tx, &session_id, Some(error)).await;
        return;
    }

    let host = match parsed["host"].as_str() {
        Some(host) if !host.is_empty() => host.to_string(),
        _ => {
            tracing::warn!(session_id = %session_id, "ssh_tunnel_open missing host");
            let _ = send_ssh_tunnel_closed(&tx, &session_id, Some("missing_host")).await;
            return;
        }
    };
    let port = match parsed["port"].as_u64() {
        Some(port) if u16::try_from(port).is_ok() => port as u16,
        _ => {
            tracing::warn!(session_id = %session_id, "ssh_tunnel_open missing or invalid port");
            let _ = send_ssh_tunnel_closed(&tx, &session_id, Some("invalid_port")).await;
            return;
        }
    };

    if let Err(error) = validate_node_ssh_target(ssh_config, &host, port).await {
        tracing::warn!(session_id = %session_id, host = %host, port, %error, "ssh tunnel target rejected by node policy");
        let error = format!("target_not_allowed:{error}");
        let _ = send_ssh_tunnel_closed(&tx, &session_id, Some(error.as_str())).await;
        return;
    }

    let (control_tx, control_rx) = mpsc::channel(SSH_CONTROL_CHANNEL_SIZE);
    let address = format!("{host}:{port}");
    let io_timeout = Duration::from_secs(ssh_config.io_timeout_secs);
    let open_rejection = {
        let mut guard = active_tunnels.lock().await;
        if guard.contains_key(&session_id) {
            Some(
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": "duplicate_session_id",
                })
                .to_string(),
            )
        } else if guard.len() >= ssh_config.max_tunnels {
            Some(
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": "too_many_active_tunnels",
                })
                .to_string(),
            )
        } else {
            let task_session_id = session_id.clone();
            let task_address = address.clone();
            let task_tx = tx.clone();
            let task_tunnels = active_tunnels.clone();
            let task_handle = tokio::spawn(async move {
                run_ssh_tunnel_session(
                    task_session_id,
                    task_address,
                    control_rx,
                    task_tx,
                    task_tunnels,
                    io_timeout,
                )
                .await;
            });
            guard.insert(
                session_id.clone(),
                ActiveSshTunnelEntry {
                    control_tx: control_tx.clone(),
                    task_handle,
                },
            );
            None
        }
    };
    if let Some(message) = open_rejection {
        let _ = send_ws_message(&tx, message).await;
    }
}

async fn verify_signed_ssh_tunnel_open(
    parsed: &serde_json::Value,
    session_id: &str,
    signing_secret: &str,
    replay_guard: &Arc<tokio::sync::Mutex<ReplayGuard>>,
) -> std::result::Result<(), &'static str> {
    let timestamp = parsed["timestamp"].as_str();
    let nonce = parsed["nonce"].as_str();
    let signature = parsed["signature"].as_str();

    let (Some(timestamp), Some(nonce), Some(signature)) = (timestamp, nonce, signature) else {
        tracing::warn!(session_id = %session_id, "ssh_tunnel_open missing HMAC fields");
        return Err("missing_hmac_fields");
    };

    if !signing::verify_ssh_tunnel_signature(parsed, signing_secret, signature) {
        tracing::warn!(session_id = %session_id, "ssh_tunnel_open HMAC verification failed");
        return Err("invalid_hmac_signature");
    }

    let mut guard = replay_guard.lock().await;
    if !guard.check(timestamp, nonce) {
        tracing::warn!(session_id = %session_id, "ssh_tunnel_open rejected by replay guard");
        return Err("replay_or_expired_timestamp");
    }

    Ok(())
}

async fn handle_ssh_tunnel_data(
    parsed: &serde_json::Value,
    tx: &mpsc::Sender<NodeWsMessage>,
    active_tunnels: &ActiveSshTunnelMap,
) {
    let Some(session_id) = parsed["session_id"].as_str() else {
        tracing::warn!("ssh_tunnel_data missing session_id");
        return;
    };
    let Some(encoded_data) = parsed["data"].as_str() else {
        tracing::warn!(session_id, "ssh_tunnel_data missing data");
        return;
    };

    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded_data) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(session_id, %error, "invalid base64 in ssh_tunnel_data");
            return;
        }
    };

    let sender = {
        let guard = active_tunnels.lock().await;
        guard.get(session_id).map(ActiveSshTunnelEntry::control_tx)
    };
    if let Some(sender) = sender {
        match sender.try_send(SshTunnelControl::Data(bytes)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    session_id,
                    capacity = SSH_CONTROL_CHANNEL_SIZE,
                    "ssh tunnel control buffer full"
                );
                abort_ssh_tunnel(active_tunnels, tx, session_id, Some("control_buffer_full")).await;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                abort_ssh_tunnel(
                    active_tunnels,
                    tx,
                    session_id,
                    Some("control_channel_closed"),
                )
                .await;
            }
        }
    }
}

async fn handle_ssh_tunnel_close(parsed: &serde_json::Value, active_tunnels: &ActiveSshTunnelMap) {
    let Some(session_id) = parsed["session_id"].as_str() else {
        tracing::warn!("ssh_tunnel_close missing session_id");
        return;
    };

    if let Some(entry) = remove_ssh_tunnel_entry(active_tunnels, session_id).await {
        let ActiveSshTunnelEntry {
            control_tx,
            task_handle,
        } = entry;
        match control_tx.try_send(SshTunnelControl::Close) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) | Err(mpsc::error::TrySendError::Closed(_)) => {
                task_handle.abort()
            }
        }
    }
}

async fn run_ssh_tunnel_session(
    session_id: String,
    address: String,
    mut control_rx: mpsc::Receiver<SshTunnelControl>,
    tx: mpsc::Sender<NodeWsMessage>,
    active_tunnels: ActiveSshTunnelMap,
    io_timeout: Duration,
) {
    let connect = tokio::time::timeout(
        Duration::from_secs(SSH_CONNECT_TIMEOUT_SECS),
        TcpStream::connect(&address),
    );
    tokio::pin!(connect);
    let mut stream = loop {
        tokio::select! {
            control = control_rx.recv() => {
                match control {
                    Some(SshTunnelControl::Close) | None => {
                        let _ = remove_ssh_tunnel_entry(&active_tunnels, &session_id).await;
                        let _ = send_ssh_tunnel_closed(&tx, &session_id, None).await;
                        return;
                    }
                    Some(SshTunnelControl::Data(_)) => {
                        tracing::warn!(
                            session_id = %session_id,
                            "received ssh tunnel data before tunnel connect completed"
                        );
                    }
                }
            }
            connect_result = &mut connect => {
                match connect_result {
                    Ok(Ok(stream)) => break stream,
                    Ok(Err(error)) => {
                        tracing::warn!(session_id = %session_id, %address, %error, "failed to open ssh tunnel tcp stream");
                        let _ = remove_ssh_tunnel_entry(&active_tunnels, &session_id).await;
                        let error = format!("connect_failed:{error}");
                        let _ = send_ssh_tunnel_closed(&tx, &session_id, Some(error.as_str())).await;
                        return;
                    }
                    Err(_) => {
                        tracing::warn!(
                            session_id = %session_id,
                            %address,
                            timeout_secs = SSH_CONNECT_TIMEOUT_SECS,
                            "timed out opening ssh tunnel tcp stream"
                        );
                        let _ = remove_ssh_tunnel_entry(&active_tunnels, &session_id).await;
                        let _ = send_ssh_tunnel_closed(&tx, &session_id, Some("connect_timeout")).await;
                        return;
                    }
                }
            }
        }
    };

    if !send_ws_message(
        &tx,
        serde_json::json!({
            "type": "ssh_tunnel_opened",
            "session_id": session_id,
        })
        .to_string(),
    )
    .await
    {
        let _ = remove_ssh_tunnel_entry(&active_tunnels, &session_id).await;
        return;
    }

    let mut read_buf = [0u8; 16 * 1024];

    loop {
        tokio::select! {
            control = control_rx.recv() => {
                match control {
                    Some(SshTunnelControl::Data(bytes)) => {
                        if let Err(error) = write_ssh_tunnel_stream(&mut stream, &bytes, io_timeout).await {
                            tracing::warn!(session_id = %session_id, %error, "failed to write ssh tunnel bytes");
                            break;
                        }
                    }
                    Some(SshTunnelControl::Close) | None => break,
                }
            }
            read_result = read_ssh_tunnel_stream(&mut stream, &mut read_buf, io_timeout) => {
                match read_result {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = send_ws_message(
                            &tx,
                            serde_json::json!({
                                "type": "ssh_tunnel_data",
                                "session_id": session_id,
                                "data": base64::engine::general_purpose::STANDARD
                                    .encode(&read_buf[..n]),
                            })
                            .to_string(),
                        )
                        .await;
                    }
                    Err(error) => {
                        tracing::warn!(session_id = %session_id, %error, "failed reading ssh tunnel bytes");
                        break;
                    }
                }
            }
        }
    }

    let _ = remove_ssh_tunnel_entry(&active_tunnels, &session_id).await;
    let _ = send_ws_message(
        &tx,
        serde_json::json!({
            "type": "ssh_tunnel_closed",
            "session_id": session_id,
        })
        .to_string(),
    )
    .await;
}

async fn read_ssh_tunnel_stream<T>(
    stream: &mut T,
    buf: &mut [u8],
    io_timeout: Duration,
) -> Result<usize>
where
    T: AsyncRead + Unpin,
{
    tokio::time::timeout(io_timeout, stream.read(buf))
        .await
        .map_err(|_| Error::Io(ssh_tunnel_timeout_error("read", io_timeout)))?
        .map_err(Error::Io)
}

async fn write_ssh_tunnel_stream<T>(
    stream: &mut T,
    bytes: &[u8],
    io_timeout: Duration,
) -> Result<()>
where
    T: AsyncWrite + Unpin,
{
    tokio::time::timeout(io_timeout, stream.write_all(bytes))
        .await
        .map_err(|_| Error::Io(ssh_tunnel_timeout_error("write", io_timeout)))?
        .map_err(Error::Io)
}

fn ssh_tunnel_timeout_error(operation: &str, io_timeout: Duration) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "SSH tunnel {operation} timed out after {}ms",
            io_timeout.as_millis()
        ),
    )
}

async fn abort_ssh_tunnel(
    active_tunnels: &ActiveSshTunnelMap,
    tx: &mpsc::Sender<NodeWsMessage>,
    session_id: &str,
    error: Option<&str>,
) {
    let Some(entry) = remove_ssh_tunnel_entry(active_tunnels, session_id).await else {
        return;
    };

    entry.abort();
    let _ = send_ssh_tunnel_closed(tx, session_id, error).await;
}

async fn remove_ssh_tunnel_entry(
    active_tunnels: &ActiveSshTunnelMap,
    session_id: &str,
) -> Option<ActiveSshTunnelEntry> {
    active_tunnels.lock().await.remove(session_id)
}

async fn drain_active_ssh_tunnels(active_tunnels: &ActiveSshTunnelMap) {
    let entries = {
        let mut guard = active_tunnels.lock().await;
        guard.drain().map(|(_, entry)| entry).collect::<Vec<_>>()
    };

    for entry in entries {
        entry.abort();
    }
}

async fn close_active_ssh_tunnels(active_tunnels: &ActiveSshTunnelMap, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let entries = {
            let guard = active_tunnels.lock().await;
            if guard.is_empty() {
                return;
            }
            guard
                .values()
                .map(ActiveSshTunnelEntry::control_tx)
                .collect::<Vec<_>>()
        };

        for control_tx in entries {
            let _ = control_tx.try_send(SshTunnelControl::Close);
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    drain_active_ssh_tunnels(active_tunnels).await;
}

async fn send_ssh_tunnel_closed(
    tx: &mpsc::Sender<NodeWsMessage>,
    session_id: &str,
    error: Option<&str>,
) -> bool {
    send_ws_message(
        tx,
        serde_json::json!({
            "type": "ssh_tunnel_closed",
            "session_id": session_id,
            "error": error,
        })
        .to_string(),
    )
    .await
}

async fn validate_node_ssh_target(ssh_config: &SshConfig, host: &str, _port: u16) -> Result<()> {
    // SSH services inherently target private/internal hosts -- that's the
    // whole point of tunneling through a node agent. Only block cloud
    // metadata endpoints (SSRF risk). If an allowlist is configured,
    // require the target to be listed; otherwise allow all targets.
    let normalized_host = normalize_target_host(host);
    if normalized_host == "metadata.google.internal" {
        return Err(Error::Validation(
            "SSH target must not point to a cloud metadata endpoint".to_string(),
        ));
    }

    if !ssh_config.allowed_targets.is_empty() && !is_allowlisted_ssh_target(ssh_config, host, _port)
    {
        return Err(Error::Validation(
            "SSH target is not in the node's allowed_targets list".to_string(),
        ));
    }

    Ok(())
}

fn is_allowlisted_ssh_target(ssh_config: &SshConfig, host: &str, port: u16) -> bool {
    let normalized_host = normalize_target_host(host);
    ssh_config.allowed_targets.iter().any(|target| {
        normalize_target_host(&target.host) == normalized_host
            && target.port.is_none_or(|allowed_port| allowed_port == port)
    })
}

fn normalize_target_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// SSH exec handler
// ---------------------------------------------------------------------------

async fn handle_ssh_exec(
    parsed: &serde_json::Value,
    ssh_config: &SshConfig,
    tx: mpsc::Sender<NodeWsMessage>,
    signing_secret: Option<SharedSigningSecret>,
    replay_guard: Arc<tokio::sync::Mutex<ReplayGuard>>,
) {
    let request_id = match parsed["request_id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            tracing::warn!("ssh_exec missing request_id");
            return;
        }
    };

    // Verify HMAC signature if signing is enabled
    if let Some(secret) = signing_secret.as_deref()
        && let Err(error) =
            verify_signed_ssh_exec(parsed, &request_id, secret.as_str(), &replay_guard).await
    {
        let _ = send_ssh_exec_result(&tx, &request_id, -1, &[], &[], 0, false, Some(error)).await;
        return;
    }

    let host = match parsed["host"].as_str() {
        Some(h) if !h.is_empty() => h.to_string(),
        _ => {
            tracing::warn!(request_id = %request_id, "ssh_exec missing host");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("missing_host"),
            )
            .await;
            return;
        }
    };
    let port = match parsed["port"].as_u64() {
        Some(p) if u16::try_from(p).is_ok() => p as u16,
        _ => {
            tracing::warn!(request_id = %request_id, "ssh_exec missing or invalid port");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("invalid_port"),
            )
            .await;
            return;
        }
    };
    let principal = match parsed["principal"].as_str() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            tracing::warn!(request_id = %request_id, "ssh_exec missing principal");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("missing_principal"),
            )
            .await;
            return;
        }
    };
    let private_key_pem = match parsed["private_key_pem"].as_str() {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => {
            tracing::warn!(request_id = %request_id, "ssh_exec missing private_key_pem");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("missing_private_key"),
            )
            .await;
            return;
        }
    };
    let certificate_openssh = parsed["certificate_openssh"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    let command = match parsed["command"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            tracing::warn!(request_id = %request_id, "ssh_exec missing command");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("missing_command"),
            )
            .await;
            return;
        }
    };
    let timeout_secs = parsed["timeout_secs"].as_u64().unwrap_or(30).clamp(1, 300);

    // Validate target against SSH config policy
    if let Err(error) = validate_node_ssh_target(ssh_config, &host, port).await {
        tracing::warn!(
            request_id = %request_id,
            host = %host,
            port,
            %error,
            "ssh exec target rejected by node policy"
        );
        let error = format!("target_not_allowed:{error}");
        let _ = send_ssh_exec_result(&tx, &request_id, -1, &[], &[], 0, false, Some(&error)).await;
        return;
    }

    // Write key + cert to temp files
    let temp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(error) => {
            tracing::error!(request_id = %request_id, %error, "failed to create temp dir for SSH exec");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("internal_error"),
            )
            .await;
            return;
        }
    };

    let key_path = temp_dir.path().join("id_key");
    if let Err(error) = write_temp_key_file(&key_path, &private_key_pem) {
        tracing::error!(request_id = %request_id, %error, "failed to write temp SSH key for exec");
        let _ = send_ssh_exec_result(
            &tx,
            &request_id,
            -1,
            &[],
            &[],
            0,
            false,
            Some("internal_error"),
        )
        .await;
        return;
    }

    let cert_path = certificate_openssh.as_ref().map(|cert| {
        let path = temp_dir.path().join("id_key-cert.pub");
        (path, cert.clone())
    });
    if let Some((ref path, ref cert_content)) = cert_path
        && let Err(error) = std::fs::write(path, cert_content)
    {
        tracing::error!(request_id = %request_id, %error, "failed to write temp SSH certificate for exec");
        let _ = send_ssh_exec_result(
            &tx,
            &request_id,
            -1,
            &[],
            &[],
            0,
            false,
            Some("internal_error"),
        )
        .await;
        return;
    }

    // Build SSH command (no PTY needed for exec)
    let identity_file_opt = format!("IdentityFile={}", key_path.display());
    let port_str = port.to_string();
    let user_host = format!("{principal}@{host}");
    let cert_file_opt = cert_path
        .as_ref()
        .map(|(path, _)| format!("CertificateFile={}", path.display()));

    let mut ssh_args: Vec<&str> = vec![
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        &identity_file_opt,
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "LogLevel=FATAL",
        "-o",
        "RequestTTY=no",
    ];
    if let Some(ref cert_opt) = cert_file_opt {
        ssh_args.extend_from_slice(&["-o", cert_opt.as_str()]);
    }
    ssh_args.extend_from_slice(&["-p", &port_str, &user_host]);
    ssh_args.push(&command);

    let started_at = std::time::Instant::now();

    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args(&ssh_args);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(error) => {
            tracing::error!(request_id = %request_id, %error, "failed to spawn ssh for exec");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                0,
                false,
                Some("ssh_spawn_failed"),
            )
            .await;
            return;
        }
    };

    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    let read_and_wait = async {
        let stdout_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut out) = child_stdout {
                use tokio::io::AsyncReadExt;
                // Cap output to prevent memory exhaustion
                let _ = (&mut out)
                    .take(SSH_EXEC_MAX_OUTPUT_BYTES as u64)
                    .read_to_end(&mut buf)
                    .await;
            }
            buf
        });
        let stderr_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut err) = child_stderr {
                use tokio::io::AsyncReadExt;
                let _ = (&mut err)
                    .take(SSH_EXEC_MAX_OUTPUT_BYTES as u64)
                    .read_to_end(&mut buf)
                    .await;
            }
            buf
        });

        let status = child.wait().await;
        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();
        (status, stdout_bytes, stderr_bytes)
    };

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), read_and_wait).await;

    let duration_ms = started_at.elapsed().as_millis() as u64;

    // temp_dir is dropped here, cleaning up key files

    match result {
        Ok((Ok(status), stdout_bytes, stderr_bytes)) => {
            let exit_code = status.code().unwrap_or(-1);
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                exit_code,
                &stdout_bytes,
                &stderr_bytes,
                duration_ms,
                false,
                None,
            )
            .await;
            tracing::info!(
                request_id = %request_id,
                host = %host,
                port,
                principal = %principal,
                exit_code,
                duration_ms,
                "ssh exec completed"
            );
        }
        Ok((Err(error), _, _)) => {
            tracing::error!(request_id = %request_id, %error, "ssh exec process failed");
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                &[],
                duration_ms,
                false,
                Some("ssh_process_failed"),
            )
            .await;
        }
        Err(_) => {
            // Timeout: child is dropped (killed) by the async block drop
            tracing::warn!(
                request_id = %request_id,
                timeout_secs,
                "ssh exec timed out"
            );
            let _ = send_ssh_exec_result(
                &tx,
                &request_id,
                -1,
                &[],
                b"Command execution timed out",
                duration_ms,
                true,
                None,
            )
            .await;
        }
    }
}

async fn verify_signed_ssh_exec(
    parsed: &serde_json::Value,
    request_id: &str,
    signing_secret: &str,
    replay_guard: &Arc<tokio::sync::Mutex<ReplayGuard>>,
) -> std::result::Result<(), &'static str> {
    let timestamp = parsed["timestamp"].as_str();
    let nonce = parsed["nonce"].as_str();
    let signature = parsed["hmac"].as_str();

    let (Some(timestamp), Some(nonce), Some(signature)) = (timestamp, nonce, signature) else {
        tracing::warn!(request_id = %request_id, "ssh_exec missing HMAC fields");
        return Err("missing_hmac_fields");
    };

    if !signing::verify_ssh_exec_signature(parsed, signing_secret, signature) {
        tracing::warn!(request_id = %request_id, "ssh_exec HMAC verification failed");
        return Err("invalid_hmac_signature");
    }

    let mut guard = replay_guard.lock().await;
    if !guard.check(timestamp, nonce) {
        tracing::warn!(request_id = %request_id, "ssh_exec rejected by replay guard");
        return Err("replay_or_expired_timestamp");
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_ssh_exec_result(
    tx: &mpsc::Sender<NodeWsMessage>,
    request_id: &str,
    exit_code: i32,
    stdout: &[u8],
    stderr: &[u8],
    duration_ms: u64,
    timed_out: bool,
    error: Option<&str>,
) -> bool {
    let stdout_b64 = base64::engine::general_purpose::STANDARD.encode(stdout);
    let stderr_b64 = base64::engine::general_purpose::STANDARD.encode(stderr);

    send_ws_message(
        tx,
        serde_json::json!({
            "type": "ssh_exec_result",
            "request_id": request_id,
            "exit_code": exit_code,
            "stdout": stdout_b64,
            "stderr": stderr_b64,
            "duration_ms": duration_ms,
            "timed_out": timed_out,
            "error": error,
        })
        .to_string(),
    )
    .await
}

// ---------------------------------------------------------------------------
// Web terminal handlers
// ---------------------------------------------------------------------------

async fn handle_web_terminal_open(
    parsed: &serde_json::Value,
    ssh_config: &SshConfig,
    tx: mpsc::Sender<NodeWsMessage>,
    active_terminals: ActiveWebTerminalMap,
    signing_secret: Option<SharedSigningSecret>,
    replay_guard: Arc<tokio::sync::Mutex<ReplayGuard>>,
) {
    let session_id = match parsed["session_id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            tracing::warn!("web_terminal_open missing session_id");
            return;
        }
    };

    // Verify HMAC signature if signing is enabled
    if let Some(secret) = signing_secret.as_deref()
        && let Err(error) =
            verify_signed_web_terminal_open(parsed, &session_id, secret.as_str(), &replay_guard)
                .await
    {
        let _ = send_web_terminal_closed(&tx, &session_id, Some(error)).await;
        return;
    }

    let host = match parsed["host"].as_str() {
        Some(h) if !h.is_empty() => h.to_string(),
        _ => {
            tracing::warn!(session_id = %session_id, "web_terminal_open missing host");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("missing_host")).await;
            return;
        }
    };
    let port = match parsed["port"].as_u64() {
        Some(p) if u16::try_from(p).is_ok() => p as u16,
        _ => {
            tracing::warn!(session_id = %session_id, "web_terminal_open missing or invalid port");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("invalid_port")).await;
            return;
        }
    };
    let principal = match parsed["principal"].as_str() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            tracing::warn!(session_id = %session_id, "web_terminal_open missing principal");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("missing_principal")).await;
            return;
        }
    };
    let private_key_pem = match parsed["private_key_pem"].as_str() {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => {
            tracing::warn!(session_id = %session_id, "web_terminal_open missing private_key_pem");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("missing_private_key")).await;
            return;
        }
    };
    let certificate_openssh = parsed["certificate_openssh"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);

    let cols = parsed["cols"].as_u64().unwrap_or(80) as u16;
    let rows = parsed["rows"].as_u64().unwrap_or(24) as u16;

    // Validate target against SSH config policy
    if let Err(error) = validate_node_ssh_target(ssh_config, &host, port).await {
        tracing::warn!(
            session_id = %session_id,
            host = %host,
            port,
            %error,
            "web terminal target rejected by node policy"
        );
        let error = format!("target_not_allowed:{error}");
        let _ = send_web_terminal_closed(&tx, &session_id, Some(error.as_str())).await;
        return;
    }

    // Check for duplicate or capacity limit
    {
        let guard = active_terminals.lock().await;
        if guard.contains_key(&session_id) {
            let _ = send_web_terminal_closed(&tx, &session_id, Some("duplicate_session_id")).await;
            return;
        }
        if guard.len() >= ssh_config.max_tunnels {
            let _ =
                send_web_terminal_closed(&tx, &session_id, Some("too_many_active_terminals")).await;
            return;
        }
    }

    // Write SSH key material to temp files
    let temp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(error) => {
            tracing::error!(session_id = %session_id, %error, "failed to create temp dir for SSH keys");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("internal_error")).await;
            return;
        }
    };

    let key_path = temp_dir.path().join("id_key");
    if let Err(error) = write_temp_key_file(&key_path, &private_key_pem) {
        tracing::error!(session_id = %session_id, %error, "failed to write temp SSH key");
        let _ = send_web_terminal_closed(&tx, &session_id, Some("internal_error")).await;
        return;
    }

    let cert_path = certificate_openssh.as_ref().map(|cert| {
        // OpenSSH expects the certificate file next to the key, named <key>-cert.pub
        let path = temp_dir.path().join("id_key-cert.pub");
        (path, cert.clone())
    });
    if let Some((ref path, ref cert_content)) = cert_path
        && let Err(error) = std::fs::write(path, cert_content)
    {
        tracing::error!(session_id = %session_id, %error, "failed to write temp SSH certificate");
        let _ = send_web_terminal_closed(&tx, &session_id, Some("internal_error")).await;
        return;
    }

    // Open PTY and spawn SSH
    let (pty, pts) = match pty_process::open() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(session_id = %session_id, %error, "failed to open PTY");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("pty_open_failed")).await;
            return;
        }
    };

    if let Err(error) = pty.resize(pty_process::Size::new(rows, cols)) {
        tracing::error!(session_id = %session_id, %error, "failed to resize PTY");
        let _ = send_web_terminal_closed(&tx, &session_id, Some("pty_resize_failed")).await;
        return;
    }

    // Set PTY to raw mode so control characters (Ctrl+C, Ctrl+Z, etc.) and
    // special keys (for top, vim, etc.) pass through to the remote shell.
    {
        use std::os::fd::AsFd;
        if let Ok(mut termios) = nix::sys::termios::tcgetattr(pty.as_fd()) {
            nix::sys::termios::cfmakeraw(&mut termios);
            let _ = nix::sys::termios::tcsetattr(
                pty.as_fd(),
                nix::sys::termios::SetArg::TCSANOW,
                &termios,
            );
        }
    }

    let identity_file_opt = format!("IdentityFile={}", key_path.display());
    let port_str = port.to_string();
    let user_host = format!("{principal}@{host}");
    let cert_file_opt = cert_path
        .as_ref()
        .map(|(path, _)| format!("CertificateFile={}", path.display()));

    let mut ssh_args: Vec<&str> = vec![
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        &identity_file_opt,
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "RequestTTY=no",
        "-o",
        "LogLevel=FATAL",
    ];
    if let Some(ref cert_opt) = cert_file_opt {
        ssh_args.extend_from_slice(&["-o", cert_opt.as_str()]);
    }
    let remote_cmd = format!(
        "export TERM=xterm-256color COLUMNS={cols} LINES={rows}; \
         script -q /dev/null sh -c 'stty cols {cols} rows {rows} 2>/dev/null; exec $SHELL -il' 2>/dev/null \
         || exec $SHELL -il"
    );
    ssh_args.extend_from_slice(&["-p", &port_str, &user_host]);
    ssh_args.push(&remote_cmd);

    let cmd = pty_process::Command::new("ssh");
    let child = match cmd.args(&ssh_args).spawn(pts) {
        Ok(c) => c,
        Err(error) => {
            tracing::error!(session_id = %session_id, %error, "failed to spawn SSH in PTY");
            let _ = send_web_terminal_closed(&tx, &session_id, Some("ssh_spawn_failed")).await;
            return;
        }
    };

    // Send started notification
    if !send_ws_message(
        &tx,
        serde_json::json!({
            "type": "web_terminal_started",
            "session_id": session_id,
        })
        .to_string(),
    )
    .await
    {
        return;
    }

    // Split PTY and spawn reader task
    let (mut pty_reader, pty_writer) = pty.into_split();
    let reader_session_id = session_id.clone();
    let reader_tx = tx.clone();
    let reader_terminals = active_terminals.clone();
    let task_handle = tokio::spawn(async move {
        let mut buf = [0u8; WEB_TERMINAL_PTY_READ_BUF];
        loop {
            match pty_reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if !send_ws_message(
                        &reader_tx,
                        serde_json::json!({
                            "type": "web_terminal_data",
                            "session_id": reader_session_id,
                            "data": encoded,
                        })
                        .to_string(),
                    )
                    .await
                    {
                        break;
                    }
                }
                Err(error) => {
                    // EIO is expected when the child process exits on the PTY
                    if error.raw_os_error() != Some(nix::libc::EIO) {
                        tracing::debug!(
                            session_id = %reader_session_id,
                            %error,
                            "PTY read error"
                        );
                    }
                    break;
                }
            }
        }

        // Reader finished: clean up entry and notify server
        let _ = remove_web_terminal_entry(&reader_terminals, &reader_session_id).await;
        let _ = send_web_terminal_closed(&reader_tx, &reader_session_id, None).await;
    });

    // Store the terminal entry
    active_terminals.lock().await.insert(
        session_id.clone(),
        ActiveWebTerminal {
            pty_writer,
            child,
            task_handle,
            _temp_dir: temp_dir,
        },
    );

    tracing::info!(
        session_id = %session_id,
        host = %host,
        port,
        principal = %principal,
        "web terminal session started"
    );
}

async fn verify_signed_web_terminal_open(
    parsed: &serde_json::Value,
    session_id: &str,
    signing_secret: &str,
    replay_guard: &Arc<tokio::sync::Mutex<ReplayGuard>>,
) -> std::result::Result<(), &'static str> {
    let timestamp = parsed["timestamp"].as_str();
    let nonce = parsed["nonce"].as_str();
    let signature = parsed["hmac"].as_str();

    let (Some(timestamp), Some(nonce), Some(signature)) = (timestamp, nonce, signature) else {
        tracing::warn!(session_id = %session_id, "web_terminal_open missing HMAC fields");
        return Err("missing_hmac_fields");
    };

    if !signing::verify_web_terminal_signature(parsed, signing_secret, signature) {
        tracing::warn!(session_id = %session_id, "web_terminal_open HMAC verification failed");
        return Err("invalid_hmac_signature");
    }

    let mut guard = replay_guard.lock().await;
    if !guard.check(timestamp, nonce) {
        tracing::warn!(session_id = %session_id, "web_terminal_open rejected by replay guard");
        return Err("replay_or_expired_timestamp");
    }

    Ok(())
}

async fn handle_web_terminal_data(
    parsed: &serde_json::Value,
    active_terminals: &ActiveWebTerminalMap,
) {
    let Some(session_id) = parsed["session_id"].as_str() else {
        tracing::warn!("web_terminal_data missing session_id");
        return;
    };
    let Some(encoded_data) = parsed["data"].as_str() else {
        tracing::warn!(session_id, "web_terminal_data missing data");
        return;
    };

    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded_data) {
        Ok(b) => b,
        Err(error) => {
            tracing::warn!(session_id, %error, "invalid base64 in web_terminal_data");
            return;
        }
    };

    let mut guard = active_terminals.lock().await;
    if let Some(terminal) = guard.get_mut(session_id)
        && let Err(error) = terminal.pty_writer.write_all(&bytes).await
    {
        tracing::warn!(session_id, %error, "failed to write to PTY");
    }
}

async fn handle_web_terminal_resize(
    parsed: &serde_json::Value,
    active_terminals: &ActiveWebTerminalMap,
) {
    let Some(session_id) = parsed["session_id"].as_str() else {
        tracing::warn!("web_terminal_resize missing session_id");
        return;
    };
    let cols = match parsed["cols"].as_u64() {
        Some(c) if u16::try_from(c).is_ok() => c as u16,
        _ => {
            tracing::warn!(session_id, "web_terminal_resize missing or invalid cols");
            return;
        }
    };
    let rows = match parsed["rows"].as_u64() {
        Some(r) if u16::try_from(r).is_ok() => r as u16,
        _ => {
            tracing::warn!(session_id, "web_terminal_resize missing or invalid rows");
            return;
        }
    };

    let guard = active_terminals.lock().await;
    if let Some(terminal) = guard.get(session_id)
        && let Err(error) = terminal
            .pty_writer
            .resize(pty_process::Size::new(rows, cols))
    {
        tracing::warn!(session_id, %error, "failed to resize PTY");
    }
}

async fn handle_web_terminal_close(
    parsed: &serde_json::Value,
    tx: &mpsc::Sender<NodeWsMessage>,
    active_terminals: &ActiveWebTerminalMap,
) {
    let Some(session_id) = parsed["session_id"].as_str() else {
        tracing::warn!("web_terminal_close missing session_id");
        return;
    };

    if let Some(mut terminal) = remove_web_terminal_entry(active_terminals, session_id).await {
        let _ = terminal.child.kill().await;
        terminal.task_handle.abort();
        let _ = send_web_terminal_closed(tx, session_id, None).await;
        tracing::info!(session_id, "web terminal session closed by server");
    }
}

async fn remove_web_terminal_entry(
    active_terminals: &ActiveWebTerminalMap,
    session_id: &str,
) -> Option<ActiveWebTerminal> {
    active_terminals.lock().await.remove(session_id)
}

async fn drain_active_web_terminals(active_terminals: &ActiveWebTerminalMap) {
    let entries = {
        let mut guard = active_terminals.lock().await;
        guard.drain().collect::<Vec<_>>()
    };

    for (session_id, mut terminal) in entries {
        let _ = terminal.child.kill().await;
        terminal.task_handle.abort();
        tracing::debug!(session_id = %session_id, "web terminal drained on disconnect");
    }
}

async fn send_web_terminal_closed(
    tx: &mpsc::Sender<NodeWsMessage>,
    session_id: &str,
    error: Option<&str>,
) -> bool {
    send_ws_message(
        tx,
        serde_json::json!({
            "type": "web_terminal_closed",
            "session_id": session_id,
            "error": error,
        })
        .to_string(),
    )
    .await
}

/// Write key material to a temp file with 0600 permissions.
fn write_temp_key_file(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Credential update handlers (server push)
// ---------------------------------------------------------------------------

/// Process a credential update synchronously. Returns an optional JSON ack
/// message to send over the WebSocket (avoids holding `&SecretBackend` across
/// an .await, since `SecretBackend` is not `Send`/`Sync`).
fn process_credential_update(
    parsed: &serde_json::Value,
    credential_sender: &Arc<SharedCredentialsSender>,
    config_path: &std::path::Path,
    backend: &SecretBackend,
) -> Option<String> {
    // Echo back `request_id` so strict-push callers on the backend can
    // correlate the `credential_update_ack` to their pending waiter.
    // Omitted/None preserves backward compatibility with older servers
    // that don't attach a request id.
    let request_id = parsed["request_id"].as_str();
    let service_slug = match parsed["service_slug"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!("credential_update missing service_slug");
            return None;
        }
    };

    let injection_method = parsed["injection_method"].as_str().unwrap_or("header");

    let result = match injection_method {
        "header" => {
            let header_name = parsed["header_name"].as_str().unwrap_or("Authorization");
            let header_value = match parsed["header_value"].as_str() {
                Some(v) => v,
                None => {
                    tracing::warn!(slug = %service_slug, "credential_update missing header_value");
                    return Some(build_credential_ack(
                        request_id,
                        service_slug,
                        "error",
                        Some("missing header_value"),
                    ));
                }
            };
            let target_url = parsed["target_url"].as_str();

            update_header_credential(
                service_slug,
                header_name,
                header_value,
                target_url,
                credential_sender,
                config_path,
                backend,
            )
        }
        "query_param" => {
            let param_name = match parsed["param_name"].as_str() {
                Some(n) => n,
                None => {
                    tracing::warn!(slug = %service_slug, "credential_update missing param_name");
                    return Some(build_credential_ack(
                        request_id,
                        service_slug,
                        "error",
                        Some("missing param_name"),
                    ));
                }
            };
            let param_value = match parsed["param_value"].as_str() {
                Some(v) => v,
                None => {
                    tracing::warn!(slug = %service_slug, "credential_update missing param_value");
                    return Some(build_credential_ack(
                        request_id,
                        service_slug,
                        "error",
                        Some("missing param_value"),
                    ));
                }
            };
            let target_url = parsed["target_url"].as_str();

            update_query_param_credential(
                service_slug,
                param_name,
                param_value,
                target_url,
                credential_sender,
                config_path,
                backend,
            )
        }
        "path_prefix" => {
            let prefix = parsed["header_name"].as_str().unwrap_or("bot");
            let credential = match parsed["header_value"].as_str() {
                Some(v) => v,
                None => {
                    tracing::warn!(slug = %service_slug, "credential_update missing header_value for path_prefix");
                    return Some(build_credential_ack(
                        request_id,
                        service_slug,
                        "error",
                        Some("missing header_value"),
                    ));
                }
            };
            let target_url = parsed["target_url"].as_str();

            update_path_prefix_credential(
                service_slug,
                prefix,
                credential,
                target_url,
                credential_sender,
                config_path,
                backend,
            )
        }
        "none" => {
            // No-auth placeholder push: drops any stored secret for
            // this slug but preserves `target_url`, so the proxy
            // executor can still resolve the downstream without
            // 502ing on a missing entry. Used when NyxID downgrades
            // `auth_method` to `none` on a server-held service
            // (thirty-third-round Codex P1).
            let target_url = parsed["target_url"].as_str();
            update_no_auth_credential(
                service_slug,
                target_url,
                credential_sender,
                config_path,
                backend,
            )
        }
        other => {
            tracing::warn!(method = %other, "Unknown injection_method in credential_update");
            return Some(build_credential_ack(
                request_id,
                service_slug,
                "error",
                Some("unknown injection_method"),
            ));
        }
    };

    match result {
        Ok(()) => {
            tracing::info!(slug = %service_slug, "Credential updated via server push");
            Some(build_credential_ack(request_id, service_slug, "ok", None))
        }
        Err(e) => {
            tracing::error!(slug = %service_slug, error = %e, "Failed to update credential");
            Some(build_credential_ack(
                request_id,
                service_slug,
                "error",
                Some(&e.to_string()),
            ))
        }
    }
}

fn process_credential_remove(
    parsed: &serde_json::Value,
    credential_sender: &Arc<SharedCredentialsSender>,
    config_path: &std::path::Path,
    backend: &SecretBackend,
) -> Option<String> {
    let request_id = parsed["request_id"].as_str();
    let service_slug = match parsed["service_slug"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!("credential_remove missing service_slug");
            return None;
        }
    };

    let result = (|| -> Result<()> {
        let mut config = NodeConfig::load(config_path)?;
        // Check presence *before* calling `remove_credential_via` so
        // we can distinguish "nothing to remove" (slug absent — this
        // node never held the credential, which is fine on a
        // reassignment) from a real backend failure (keyring /
        // filesystem deletion error). Swallowing real errors would
        // leave the old node still serving the secret while NyxID
        // thinks it's gone (eighteenth-round Codex P2).
        if !config.credentials.contains_key(service_slug) {
            return Ok(());
        }
        config.remove_credential_via(service_slug, backend)?;
        config.save(config_path)?;
        let new_store = CredentialStore::from_config_with_backend(&config, backend)?;
        credential_sender.update(new_store);
        Ok(())
    })();

    match result {
        Ok(()) => {
            tracing::info!(slug = %service_slug, "Removed credential via server push");
            Some(build_credential_ack(request_id, service_slug, "ok", None))
        }
        Err(e) => {
            tracing::error!(slug = %service_slug, error = %e, "Failed to remove credential");
            Some(build_credential_ack(
                request_id,
                service_slug,
                "error",
                Some(&e.to_string()),
            ))
        }
    }
}

fn update_header_credential(
    service_slug: &str,
    header_name: &str,
    header_value: &str,
    target_url: Option<&str>,
    credential_sender: &Arc<SharedCredentialsSender>,
    config_path: &std::path::Path,
    backend: &SecretBackend,
) -> Result<()> {
    // 1. Update config on disk
    let mut config = NodeConfig::load(config_path)?;
    config.add_header_credential_via(
        service_slug,
        header_name,
        header_value,
        target_url,
        backend,
    )?;
    config.save(config_path)?;

    // 2. Rebuild credential store from updated config and push to watch channel
    let new_store = CredentialStore::from_config_with_backend(&config, backend)?;
    credential_sender.update(new_store);

    Ok(())
}

fn update_query_param_credential(
    service_slug: &str,
    param_name: &str,
    param_value: &str,
    target_url: Option<&str>,
    credential_sender: &Arc<SharedCredentialsSender>,
    config_path: &std::path::Path,
    backend: &SecretBackend,
) -> Result<()> {
    // 1. Update config on disk
    let mut config = NodeConfig::load(config_path)?;
    config.add_query_param_credential_via(
        service_slug,
        param_name,
        param_value,
        target_url,
        backend,
    )?;
    config.save(config_path)?;

    // 2. Rebuild credential store from updated config and push to watch channel
    let new_store = CredentialStore::from_config_with_backend(&config, backend)?;
    credential_sender.update(new_store);

    Ok(())
}

fn update_path_prefix_credential(
    service_slug: &str,
    prefix: &str,
    credential: &str,
    target_url: Option<&str>,
    credential_sender: &Arc<SharedCredentialsSender>,
    config_path: &std::path::Path,
    backend: &SecretBackend,
) -> Result<()> {
    let mut config = NodeConfig::load(config_path)?;
    config.add_path_prefix_credential_via(service_slug, prefix, credential, target_url, backend)?;
    config.save(config_path)?;

    let new_store = CredentialStore::from_config_with_backend(&config, backend)?;
    credential_sender.update(new_store);

    Ok(())
}

fn update_no_auth_credential(
    service_slug: &str,
    target_url: Option<&str>,
    credential_sender: &Arc<SharedCredentialsSender>,
    config_path: &std::path::Path,
    backend: &SecretBackend,
) -> Result<()> {
    let mut config = NodeConfig::load(config_path)?;
    config.set_no_auth_via(service_slug, target_url, backend)?;
    config.save(config_path)?;

    let new_store = CredentialStore::from_config_with_backend(&config, backend)?;
    credential_sender.update(new_store);

    Ok(())
}

fn build_credential_ack(
    request_id: Option<&str>,
    service_slug: &str,
    status: &str,
    error: Option<&str>,
) -> String {
    let mut ack = serde_json::json!({
        "type": "credential_update_ack",
        "service_slug": service_slug,
        "status": status,
    });
    if let Some(rid) = request_id {
        ack["request_id"] = serde_json::Value::String(rid.to_string());
    }
    if let Some(e) = error {
        ack["error"] = serde_json::Value::String(e.to_string());
    }
    ack.to_string()
}

async fn send_ws_message(tx: &mpsc::Sender<NodeWsMessage>, message: String) -> bool {
    tx.send(NodeWsMessage::Text(message)).await.is_ok()
}

fn shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if shutdown_requested(shutdown) {
        return;
    }
    let _ = shutdown.changed().await;
}

/// Wait for SIGINT or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}

// ---------------------------------------------------------------------------
// WebSocket proxy passthrough handlers
// ---------------------------------------------------------------------------

async fn handle_ws_proxy_open(
    parsed: &serde_json::Value,
    credentials: &CredentialStore,
    tx: mpsc::Sender<NodeWsMessage>,
    active_ws_proxies: ActiveWsProxyMap,
    signing_secret: Option<SharedSigningSecret>,
    replay_guard: Arc<tokio::sync::Mutex<ReplayGuard>>,
) {
    let session_id = match parsed["session_id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            tracing::warn!("ws_proxy_open missing session_id");
            return;
        }
    };

    // Verify HMAC if signing is enabled.
    if let Some(secret) = signing_secret.as_deref()
        && let Err(error) =
            verify_signed_ws_proxy_open(parsed, &session_id, secret.as_str(), &replay_guard).await
    {
        let _ = send_ws_proxy_error(&tx, &session_id, error).await;
        return;
    }

    let service_slug = match parsed["service_slug"].as_str() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            let _ = send_ws_proxy_error(&tx, &session_id, "missing_service_slug").await;
            return;
        }
    };

    let base_url = parsed["base_url"].as_str().unwrap_or("");
    let path = parsed["path"].as_str().unwrap_or("");
    let query = parsed["query"].as_str();
    let ws_frame_injections = match parsed.get("ws_frame_injections") {
        Some(value) if !value.is_null() => {
            match serde_json::from_value::<Vec<WsFrameInjection>>(value.clone()) {
                Ok(rules) => rules,
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %error,
                        "Invalid ws_frame_injections in ws_proxy_open"
                    );
                    let _ =
                        send_ws_proxy_error(&tx, &session_id, "invalid_ws_frame_injections").await;
                    return;
                }
            }
        }
        _ => Vec::new(),
    };

    // Resolve credentials.
    let cred = match credentials.get(&service_slug) {
        Some(c) => c,
        None => {
            let _ = send_ws_proxy_error(
                &tx,
                &session_id,
                &format!("No credentials configured for service '{service_slug}'"),
            )
            .await;
            return;
        }
    };
    let raw_credential = cred.raw_credential().map(str::to_string);

    // Resolve effective base URL.
    let effective_base_url = if base_url.is_empty() {
        match cred.target_url() {
            Some(url) => url.to_string(),
            None => {
                let _ = send_ws_proxy_error(
                    &tx,
                    &session_id,
                    &format!("No target URL configured for service '{service_slug}'"),
                )
                .await;
                return;
            }
        }
    } else {
        base_url.to_string()
    };

    // Build WS URL: convert http(s) to ws(s).
    let base = effective_base_url.trim_end_matches('/');
    let ws_base = if base.starts_with("https://") {
        base.replacen("https://", "wss://", 1)
    } else if base.starts_with("http://") {
        base.replacen("http://", "ws://", 1)
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        let _ = send_ws_proxy_error(&tx, &session_id, "unsupported_base_url_scheme").await;
        return;
    };

    let normalized_path = if path.is_empty() {
        String::new()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    // Path-prefix injection: prepend /{prefix}{credential} to the URL path
    let final_path = if let Some((prefix, credential)) = cred.path_prefix() {
        format!("/{prefix}{credential}{normalized_path}")
    } else {
        normalized_path
    };

    let mut url = format!("{ws_base}{final_path}");
    if let Some(q) = query
        && !q.is_empty()
    {
        url = format!("{url}?{q}");
    }

    // Append query-param credential.
    if let Some((param_name, param_value)) = cred.query_param() {
        url = proxy_executor::append_query_param(&url, param_name, param_value);
    }

    // Build WS connect request with header injection.
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut ws_request = match url.into_client_request() {
        Ok(r) => r,
        Err(e) => {
            let _ = send_ws_proxy_error(
                &tx,
                &session_id,
                &format!("Failed to build WS request: {e}"),
            )
            .await;
            return;
        }
    };

    // Forward headers from the proxy message.
    if let Some(headers) = parsed["headers"].as_object() {
        for (name, value) in headers {
            if let Some(v) = value.as_str()
                && let (Ok(hn), Ok(hv)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                    reqwest::header::HeaderValue::from_str(v),
                )
            {
                ws_request.headers_mut().insert(hn, hv);
            }
        }
    }

    // Inject header credential.
    if let Some((hdr_name, hdr_value)) = cred.header()
        && let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(hdr_name.as_bytes()),
            reqwest::header::HeaderValue::from_str(hdr_value),
        )
    {
        ws_request.headers_mut().insert(hn, hv);
    }

    // Connect to downstream WS with timeout and message size limits
    // matching the server-side WS_PASSTHROUGH_MAX_MESSAGE_SIZE.
    let mut ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    ws_config.max_message_size = Some(WS_PROXY_MAX_MESSAGE_SIZE);
    ws_config.max_frame_size = Some(WS_PROXY_MAX_MESSAGE_SIZE);
    let connect_result = tokio::time::timeout(
        Duration::from_secs(WS_PROXY_CONNECT_TIMEOUT_SECS),
        tokio_tungstenite::connect_async_with_config(ws_request, Some(ws_config), false),
    )
    .await;

    let (downstream_ws, selected_protocol) = match connect_result {
        Ok(Ok((ws, response))) => {
            let selected_protocol = response
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            (ws, selected_protocol)
        }
        Ok(Err(e)) => {
            let _ = send_ws_proxy_error(
                &tx,
                &session_id,
                &format!("Downstream WS connection failed: {e}"),
            )
            .await;
            return;
        }
        Err(_) => {
            let _ =
                send_ws_proxy_error(&tx, &session_id, "Downstream WS connection timed out").await;
            return;
        }
    };

    // Register and spawn relay task.
    let (control_tx, control_rx) = mpsc::channel(WS_PROXY_CONTROL_CHANNEL_SIZE);

    let open_rejection = {
        let mut guard = active_ws_proxies.lock().await;
        if guard.contains_key(&session_id) {
            Some("duplicate_session_id")
        } else {
            let task_session_id = session_id.clone();
            let task_tx = tx.clone();
            let task_proxies = active_ws_proxies.clone();
            let task_handle = tokio::spawn(async move {
                run_ws_proxy_session(
                    task_session_id,
                    downstream_ws,
                    control_rx,
                    task_tx,
                    task_proxies,
                    ws_frame_injections,
                    raw_credential,
                )
                .await;
            });
            guard.insert(
                session_id.clone(),
                ActiveWsProxyEntry {
                    control_tx,
                    task_handle,
                },
            );
            None
        }
    };

    if let Some(error) = open_rejection {
        let _ = send_ws_proxy_error(&tx, &session_id, error).await;
        return;
    }

    // Send ws_proxy_opened acknowledgement.
    let mut opened = serde_json::json!({
        "type": "ws_proxy_opened",
        "session_id": session_id,
    });
    if let Some(protocol) = selected_protocol {
        opened["selected_protocol"] = serde_json::Value::String(protocol);
    }
    let _ = send_ws_message(&tx, opened.to_string()).await;
}

/// Relay loop: bridges frames between the management WS (via control channel)
/// and the downstream WS connection.
async fn run_ws_proxy_session(
    session_id: String,
    downstream_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    control_rx: mpsc::Receiver<WsProxyControl>,
    tx: mpsc::Sender<NodeWsMessage>,
    active_ws_proxies: ActiveWsProxyMap,
    ws_frame_injections: Vec<WsFrameInjection>,
    raw_credential: Option<String>,
) {
    run_ws_proxy_session_with_limits(
        session_id,
        downstream_ws,
        control_rx,
        tx,
        active_ws_proxies,
        ws_frame_injections,
        raw_credential,
        Duration::from_secs(WS_PROXY_MAX_DURATION_SECS),
        Duration::from_secs(WS_PROXY_IDLE_TIMEOUT_SECS),
    )
    .await;
}

async fn send_ws_proxy_control_to_downstream<S>(
    downstream_sink: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<S>,
        Message,
    >,
    control: WsProxyControl,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match control {
        WsProxyControl::Text(data) => downstream_sink
            .send(Message::Text(data.into()))
            .await
            .is_ok(),
        WsProxyControl::Binary(data) => downstream_sink
            .send(Message::Binary(data.into()))
            .await
            .is_ok(),
        WsProxyControl::Close { code, reason } => {
            let close_frame = code.map(|c| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(c),
                reason: reason.unwrap_or_default().into(),
            });
            downstream_sink
                .send(Message::Close(close_frame))
                .await
                .is_ok()
        }
    }
}

fn injection_frame_to_control(frame: ws_frame_injector::WsFrame) -> WsProxyControl {
    match frame.kind {
        WsFrameKind::Text => {
            let text = String::from_utf8(frame.payload).unwrap_or_default();
            WsProxyControl::Text(text)
        }
        WsFrameKind::Binary => WsProxyControl::Binary(frame.payload),
    }
}

async fn send_ws_frame_injected(
    tx: &mpsc::Sender<NodeWsMessage>,
    session_id: &str,
    action: &ws_frame_injector::InjectionAction,
) -> bool {
    let json = serde_json::json!({
        "type": "ws_frame_injected",
        "request_id": session_id,
        "trigger_kind": action.trigger_kind,
        "frame_index": action.frame_index_in,
    });
    send_ws_message(tx, json.to_string()).await
}

#[allow(clippy::too_many_arguments)]
async fn run_ws_proxy_session_with_limits<S>(
    session_id: String,
    downstream_ws: tokio_tungstenite::WebSocketStream<S>,
    mut control_rx: mpsc::Receiver<WsProxyControl>,
    tx: mpsc::Sender<NodeWsMessage>,
    active_ws_proxies: ActiveWsProxyMap,
    ws_frame_injections: Vec<WsFrameInjection>,
    raw_credential: Option<String>,
    max_duration_limit: Duration,
    idle_timeout_limit: Duration,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut downstream_sink, mut downstream_stream) = downstream_ws.split();
    let mut injector_state = InjectorState::default();

    let max_duration = tokio::time::sleep(max_duration_limit);
    tokio::pin!(max_duration);

    let idle_timeout = tokio::time::sleep(idle_timeout_limit);
    tokio::pin!(idle_timeout);

    let reset_idle = || tokio::time::Instant::now() + idle_timeout_limit;

    loop {
        tokio::select! {
            _ = &mut max_duration => {
                tracing::info!(session_id = %session_id, "WS proxy max duration reached");
                let json = serde_json::json!({
                    "type": "ws_proxy_closed",
                    "session_id": session_id,
                    "reason": "max duration reached",
                });
                let _ = send_ws_message(&tx, json.to_string()).await;
                break;
            }
            _ = &mut idle_timeout => {
                tracing::info!(session_id = %session_id, "WS proxy idle timeout reached");
                let json = serde_json::json!({
                    "type": "ws_proxy_closed",
                    "session_id": session_id,
                    "reason": "idle timeout",
                });
                let _ = send_ws_message(&tx, json.to_string()).await;
                break;
            }
            // Server -> downstream (via control channel)
            ctrl = control_rx.recv() => {
                match ctrl {
                    Some(control @ WsProxyControl::Text(_)) | Some(control @ WsProxyControl::Binary(_)) => {
                        idle_timeout.as_mut().reset(reset_idle());
                        if !send_ws_proxy_control_to_downstream(&mut downstream_sink, control).await {
                            break;
                        }
                    }
                    Some(control @ WsProxyControl::Close { .. }) => {
                        let _ = send_ws_proxy_control_to_downstream(&mut downstream_sink, control).await;
                        break;
                    }
                    None => break, // Server disconnected
                }
            }
            // Downstream -> server
            msg = downstream_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        idle_timeout.as_mut().reset(reset_idle());
                        if let Some(credential) = raw_credential.as_deref() {
                            let frame = IncomingFrame {
                                direction: WsFrameDirection::Downstream,
                                kind: WsFrameKind::Text,
                                payload: t.to_string().into_bytes(),
                            };
                            if let Some(action) = ws_frame_injector::evaluate(
                                &ws_frame_injections,
                                &mut injector_state,
                                &frame,
                                credential,
                            ) {
                                let control = injection_frame_to_control(action.send_frame.clone());
                                if !send_ws_proxy_control_to_downstream(&mut downstream_sink, control).await {
                                    break;
                                }
                                if !send_ws_frame_injected(&tx, &session_id, &action).await {
                                    break;
                                }
                                if !action.forward_original {
                                    continue;
                                }
                            }
                        }
                        let json = serde_json::json!({
                            "type": "ws_proxy_text",
                            "session_id": session_id,
                            "data": t.to_string(),
                        });
                        if !send_ws_message(&tx, json.to_string()).await {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        idle_timeout.as_mut().reset(reset_idle());
                        if let Some(credential) = raw_credential.as_deref() {
                            let frame = IncomingFrame {
                                direction: WsFrameDirection::Downstream,
                                kind: WsFrameKind::Binary,
                                payload: b.to_vec(),
                            };
                            if let Some(action) = ws_frame_injector::evaluate(
                                &ws_frame_injections,
                                &mut injector_state,
                                &frame,
                                credential,
                            ) {
                                let control = injection_frame_to_control(action.send_frame.clone());
                                if !send_ws_proxy_control_to_downstream(&mut downstream_sink, control).await {
                                    break;
                                }
                                if !send_ws_frame_injected(&tx, &session_id, &action).await {
                                    break;
                                }
                                if !action.forward_original {
                                    continue;
                                }
                            }
                        }
                        let json = serde_json::json!({
                            "type": "ws_proxy_binary",
                            "session_id": session_id,
                            "data": base64::engine::general_purpose::STANDARD.encode(&b),
                        });
                        if !send_ws_message(&tx, json.to_string()).await {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let (code, reason) = frame
                            .map(|f| (Some(u16::from(f.code)), Some(f.reason.to_string())))
                            .unwrap_or((None, None));
                        let json = serde_json::json!({
                            "type": "ws_proxy_closed",
                            "session_id": session_id,
                            "code": code,
                            "reason": reason,
                        });
                        let _ = send_ws_message(&tx, json.to_string()).await;
                        break;
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                        // Handled at protocol level by tungstenite
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(e)) => {
                        tracing::debug!(
                            session_id = %session_id,
                            error = %e,
                            "Downstream WS error in node proxy"
                        );
                        let json = serde_json::json!({
                            "type": "ws_proxy_closed",
                            "session_id": session_id,
                            "reason": format!("downstream error: {e}"),
                        });
                        let _ = send_ws_message(&tx, json.to_string()).await;
                        break;
                    }
                    None => {
                        let json = serde_json::json!({
                            "type": "ws_proxy_closed",
                            "session_id": session_id,
                        });
                        let _ = send_ws_message(&tx, json.to_string()).await;
                        break;
                    }
                }
            }
        }
    }

    let _ = downstream_sink.close().await;

    // Remove from active map.
    let mut guard = active_ws_proxies.lock().await;
    guard.remove(&session_id);
    tracing::debug!(session_id = %session_id, "WS proxy session ended");
}

async fn verify_signed_ws_proxy_open(
    parsed: &serde_json::Value,
    session_id: &str,
    signing_secret: &str,
    replay_guard: &Arc<tokio::sync::Mutex<ReplayGuard>>,
) -> std::result::Result<(), &'static str> {
    let timestamp = parsed["timestamp"].as_str();
    let nonce = parsed["nonce"].as_str();
    let signature = parsed["signature"].as_str();

    let (Some(timestamp), Some(nonce), Some(signature)) = (timestamp, nonce, signature) else {
        tracing::warn!(session_id = %session_id, "ws_proxy_open missing HMAC fields");
        return Err("missing_hmac_fields");
    };

    if !signing::verify_ws_proxy_signature(parsed, signing_secret, signature) {
        tracing::warn!(session_id = %session_id, "ws_proxy_open HMAC verification failed");
        return Err("invalid_hmac_signature");
    }

    let mut guard = replay_guard.lock().await;
    if !guard.check(timestamp, nonce) {
        tracing::warn!(session_id = %session_id, "ws_proxy_open rejected by replay guard");
        return Err("replay_detected");
    }

    Ok(())
}

async fn fail_ws_proxy_session(
    active_ws_proxies: &ActiveWsProxyMap,
    tx: &mpsc::Sender<NodeWsMessage>,
    session_id: &str,
    error: &str,
) {
    let removed = {
        let mut guard = active_ws_proxies.lock().await;
        guard.remove(session_id)
    };
    if let Some(entry) = removed {
        entry.task_handle.abort();
    }
    let _ = send_ws_proxy_error(tx, session_id, error).await;
}

async fn handle_ws_proxy_text(
    parsed: &serde_json::Value,
    tx: &mpsc::Sender<NodeWsMessage>,
    active_ws_proxies: &ActiveWsProxyMap,
) {
    let session_id = parsed["session_id"].as_str().unwrap_or("");
    let data = parsed["data"].as_str().unwrap_or("");
    let send_result = {
        let guard = active_ws_proxies.lock().await;
        guard.get(session_id).map(|entry| {
            entry
                .control_tx
                .try_send(WsProxyControl::Text(data.to_string()))
        })
    };
    match send_result {
        Some(Err(mpsc::error::TrySendError::Full(_))) => {
            fail_ws_proxy_session(
                active_ws_proxies,
                tx,
                session_id,
                "ws_proxy_control_buffer_full",
            )
            .await;
        }
        Some(Err(mpsc::error::TrySendError::Closed(_))) => {
            fail_ws_proxy_session(active_ws_proxies, tx, session_id, "ws_proxy_session_closed")
                .await;
        }
        _ => {}
    }
}

async fn handle_ws_proxy_binary(
    parsed: &serde_json::Value,
    tx: &mpsc::Sender<NodeWsMessage>,
    active_ws_proxies: &ActiveWsProxyMap,
) {
    let session_id = parsed["session_id"].as_str().unwrap_or("");
    let data_b64 = parsed["data"].as_str().unwrap_or("");
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data_b64) else {
        fail_ws_proxy_session(active_ws_proxies, tx, session_id, "invalid_ws_proxy_binary").await;
        return;
    };
    let send_result = {
        let guard = active_ws_proxies.lock().await;
        guard
            .get(session_id)
            .map(|entry| entry.control_tx.try_send(WsProxyControl::Binary(bytes)))
    };
    match send_result {
        Some(Err(mpsc::error::TrySendError::Full(_))) => {
            fail_ws_proxy_session(
                active_ws_proxies,
                tx,
                session_id,
                "ws_proxy_control_buffer_full",
            )
            .await;
        }
        Some(Err(mpsc::error::TrySendError::Closed(_))) => {
            fail_ws_proxy_session(active_ws_proxies, tx, session_id, "ws_proxy_session_closed")
                .await;
        }
        _ => {}
    }
}

async fn handle_ws_proxy_close(
    parsed: &serde_json::Value,
    tx: &mpsc::Sender<NodeWsMessage>,
    active_ws_proxies: &ActiveWsProxyMap,
) {
    let session_id = parsed["session_id"].as_str().unwrap_or("").to_string();
    let code = parsed["code"].as_u64().map(|c| c as u16);
    let reason = parsed["reason"].as_str().map(|s| s.to_string());
    let send_result = {
        let guard = active_ws_proxies.lock().await;
        guard.get(&session_id).map(|entry| {
            entry
                .control_tx
                .try_send(WsProxyControl::Close { code, reason })
        })
    };
    match send_result {
        Some(Err(mpsc::error::TrySendError::Full(_))) => {
            fail_ws_proxy_session(
                active_ws_proxies,
                tx,
                &session_id,
                "ws_proxy_control_buffer_full",
            )
            .await;
        }
        Some(Err(mpsc::error::TrySendError::Closed(_))) => {
            fail_ws_proxy_session(
                active_ws_proxies,
                tx,
                &session_id,
                "ws_proxy_session_closed",
            )
            .await;
        }
        _ => {}
    }
}

async fn send_ws_proxy_error(
    tx: &mpsc::Sender<NodeWsMessage>,
    session_id: &str,
    error: &str,
) -> bool {
    send_ws_message(
        tx,
        serde_json::json!({
            "type": "ws_proxy_error",
            "session_id": session_id,
            "error": error,
        })
        .to_string(),
    )
    .await
}

async fn drain_active_ws_proxies(active_ws_proxies: &ActiveWsProxyMap) {
    let entries = {
        let mut guard = active_ws_proxies.lock().await;
        guard.drain().collect::<Vec<_>>()
    };
    for (session_id, entry) in entries {
        entry.task_handle.abort();
        tracing::debug!(session_id = %session_id, "WS proxy drained on disconnect");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::config::{SshConfig, SshTargetConfig};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

    /// Unwrap a NodeWsMessage::Text variant, panicking if it's Binary.
    fn unwrap_text(msg: NodeWsMessage) -> String {
        match msg {
            NodeWsMessage::Text(s) => s,
            NodeWsMessage::Binary(_) => panic!("expected Text message, got Binary"),
        }
    }

    fn ssh_config_with_allowed_target(host: &str, port: u16) -> SshConfig {
        SshConfig {
            max_tunnels: 10,
            io_timeout_secs: 3600,
            allowed_targets: vec![SshTargetConfig {
                host: host.to_string(),
                port: Some(port),
            }],
        }
    }

    fn replay_guard() -> Arc<tokio::sync::Mutex<ReplayGuard>> {
        Arc::new(tokio::sync::Mutex::new(ReplayGuard::new()))
    }

    fn shared_signing_secret(secret: &str) -> SharedSigningSecret {
        Arc::new(Zeroizing::new(secret.to_string()))
    }

    fn signed_ssh_tunnel_open_request(
        session_id: &str,
        service_id: &str,
        host: &str,
        port: u16,
        secret_hex: &str,
    ) -> serde_json::Value {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let nonce = uuid::Uuid::new_v4().to_string();
        let message = format!("{timestamp}\n{nonce}\n{session_id}\n{service_id}\n{host}\n{port}");
        let secret = hex::decode(secret_hex).expect("secret hex");
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret).expect("hmac");
        mac.update(message.as_bytes());

        serde_json::json!({
            "session_id": session_id,
            "service_id": service_id,
            "host": host,
            "port": port,
            "timestamp": timestamp,
            "nonce": nonce,
            "signature": hex::encode(mac.finalize().into_bytes()),
        })
    }

    #[tokio::test]
    async fn ws_proxy_text_backpressure_reports_error_and_removes_session() {
        let active_ws_proxies: ActiveWsProxyMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (control_tx, _control_rx) = mpsc::channel(1);
        control_tx
            .try_send(WsProxyControl::Text("queued".to_string()))
            .expect("fill control buffer");

        let task_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });

        active_ws_proxies.lock().await.insert(
            "sess-1".to_string(),
            ActiveWsProxyEntry {
                control_tx,
                task_handle,
            },
        );

        let (tx, mut rx) = mpsc::channel(4);
        let parsed = serde_json::json!({
            "session_id": "sess-1",
            "data": "hello",
        });

        handle_ws_proxy_text(&parsed, &tx, &active_ws_proxies).await;

        let msg = unwrap_text(rx.recv().await.expect("ws_proxy_error"));
        let payload: serde_json::Value =
            serde_json::from_str(&msg).expect("valid ws_proxy_error payload");
        assert_eq!(payload["type"], "ws_proxy_error");
        assert_eq!(payload["session_id"], "sess-1");
        assert_eq!(payload["error"], "ws_proxy_control_buffer_full");
        assert!(active_ws_proxies.lock().await.is_empty());
    }

    async fn duplex_websocket_pair() -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(1024);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        (client, server)
    }

    #[tokio::test]
    async fn ws_proxy_session_reports_idle_timeout() {
        let active_ws_proxies: ActiveWsProxyMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (downstream_ws, _peer_ws) = duplex_websocket_pair().await;
        let (control_tx, control_rx) = mpsc::channel(4);
        let (tx, mut rx) = mpsc::channel(4);

        let relay = tokio::spawn(run_ws_proxy_session_with_limits(
            "sess-idle".to_string(),
            downstream_ws,
            control_rx,
            tx,
            active_ws_proxies,
            Vec::new(),
            None,
            Duration::from_millis(200),
            Duration::from_millis(25),
        ));

        let msg = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("idle timeout message arrives")
            .expect("ws_proxy_closed message");
        let payload: serde_json::Value =
            serde_json::from_str(&unwrap_text(msg)).expect("valid ws_proxy_closed payload");

        assert_eq!(payload["type"], "ws_proxy_closed");
        assert_eq!(payload["session_id"], "sess-idle");
        assert_eq!(payload["reason"], "idle timeout");

        relay.await.expect("relay task completes");
        drop(control_tx);
    }

    #[tokio::test]
    async fn ws_proxy_session_reports_max_duration() {
        let active_ws_proxies: ActiveWsProxyMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (downstream_ws, _peer_ws) = duplex_websocket_pair().await;
        let (control_tx, control_rx) = mpsc::channel(4);
        let (tx, mut rx) = mpsc::channel(4);

        let relay = tokio::spawn(run_ws_proxy_session_with_limits(
            "sess-max".to_string(),
            downstream_ws,
            control_rx,
            tx,
            active_ws_proxies,
            Vec::new(),
            None,
            Duration::from_millis(25),
            Duration::from_millis(200),
        ));

        let msg = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("max duration message arrives")
            .expect("ws_proxy_closed message");
        let payload: serde_json::Value =
            serde_json::from_str(&unwrap_text(msg)).expect("valid ws_proxy_closed payload");

        assert_eq!(payload["type"], "ws_proxy_closed");
        assert_eq!(payload["session_id"], "sess-max");
        assert_eq!(payload["reason"], "max duration reached");

        relay.await.expect("relay task completes");
        drop(control_tx);
    }

    #[test]
    fn exponential_backoff_increases() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(60), 2.0);

        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
        assert_eq!(backoff.next_delay(), Duration::from_millis(200));
        assert_eq!(backoff.next_delay(), Duration::from_millis(400));
        assert_eq!(backoff.next_delay(), Duration::from_millis(800));
    }

    #[test]
    fn exponential_backoff_caps_at_max() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_secs(30), Duration::from_secs(60), 2.0);

        assert_eq!(backoff.next_delay(), Duration::from_secs(30));
        assert_eq!(backoff.next_delay(), Duration::from_secs(60));
        assert_eq!(backoff.next_delay(), Duration::from_secs(60));
    }

    #[test]
    fn exponential_backoff_resets() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(60), 2.0);

        backoff.next_delay();
        backoff.next_delay();
        backoff.reset();

        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    }

    #[test]
    fn ws_read_idle_timeout_matches_default_for_default_interval() {
        // Unchanged behavior against an unmodified server (30s interval -> 90s).
        assert_eq!(compute_ws_read_idle_timeout_secs(Some(30)), Some(90));
    }

    #[test]
    fn ws_read_idle_timeout_scales_with_server_interval() {
        // A server configured with a 120s heartbeat must not trigger the
        // client's watchdog at 90s; the timeout scales to 360s.
        assert_eq!(compute_ws_read_idle_timeout_secs(Some(120)), Some(360));
    }

    #[test]
    fn ws_read_idle_timeout_respects_floor() {
        // Pathologically small server intervals are floored so we don't
        // reconnect aggressively.
        assert_eq!(compute_ws_read_idle_timeout_secs(Some(1)), Some(30));
        assert_eq!(compute_ws_read_idle_timeout_secs(Some(5)), Some(30));
    }

    #[test]
    fn ws_read_idle_timeout_respects_ceiling() {
        // Absurdly large intervals are capped at the ceiling so silently dead
        // connections are still detected eventually.
        assert_eq!(compute_ws_read_idle_timeout_secs(Some(10_000)), Some(3600));
        assert_eq!(
            compute_ws_read_idle_timeout_secs(Some(u64::MAX)),
            Some(3600)
        );
    }

    #[test]
    fn ws_read_idle_timeout_is_disabled_when_server_does_not_advertise() {
        // Mixed-version rollout against an older backend: the client must
        // skip the idle watchdog entirely rather than guess the server's
        // interval, to avoid flapping deployments that customize
        // NODE_HEARTBEAT_INTERVAL_SECS above 90.
        assert_eq!(compute_ws_read_idle_timeout_secs(None), None);
    }

    #[tokio::test]
    async fn ssh_tunnel_handlers_bridge_data_and_close() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).await.expect("read");
            assert_eq!(&buf, b"hello");
            stream.write_all(b"world").await.expect("write");
        });

        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));

        handle_ssh_tunnel_open(
            &signed_ssh_tunnel_open_request("sess-1", "svc-1", "127.0.0.1", port, &"ab".repeat(32)),
            &ssh_config_with_allowed_target("127.0.0.1", port),
            tx.clone(),
            active_tunnels.clone(),
            Some(shared_signing_secret(&"ab".repeat(32))),
            replay_guard(),
        )
        .await;

        let opened: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("opened message")))
                .expect("opened json");
        assert_eq!(opened["type"], "ssh_tunnel_opened");
        assert_eq!(opened["session_id"], "sess-1");

        handle_ssh_tunnel_data(
            &serde_json::json!({
                "session_id": "sess-1",
                "data": base64::engine::general_purpose::STANDARD.encode(b"hello"),
            }),
            &tx,
            &active_tunnels,
        )
        .await;

        let tunneled: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("data message")))
                .expect("data json");
        assert_eq!(tunneled["type"], "ssh_tunnel_data");
        assert_eq!(tunneled["session_id"], "sess-1");
        let payload = base64::engine::general_purpose::STANDARD
            .decode(tunneled["data"].as_str().expect("data b64"))
            .expect("decode");
        assert_eq!(payload, b"world");

        handle_ssh_tunnel_close(
            &serde_json::json!({ "session_id": "sess-1" }),
            &active_tunnels,
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("close message")))
                .expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["session_id"], "sess-1");

        server.await.expect("server join");
    }

    #[tokio::test]
    async fn ssh_tunnel_rejects_metadata_endpoint() {
        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));

        handle_ssh_tunnel_open(
            &serde_json::json!({
                "session_id": "sess-meta",
                "host": "metadata.google.internal",
                "port": 22,
            }),
            &SshConfig::default(),
            tx,
            active_tunnels.clone(),
            None,
            replay_guard(),
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("close message")))
                .expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["session_id"], "sess-meta");
        assert!(
            closed["error"]
                .as_str()
                .expect("error")
                .contains("metadata"),
        );
        assert!(active_tunnels.lock().await.is_empty());
    }

    #[tokio::test]
    async fn ssh_tunnel_rejects_target_not_in_allowlist() {
        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));

        let config = SshConfig {
            allowed_targets: vec![SshTargetConfig {
                host: "10.0.0.1".to_string(),
                port: Some(22),
            }],
            ..Default::default()
        };

        handle_ssh_tunnel_open(
            &serde_json::json!({
                "session_id": "sess-deny",
                "host": "10.0.0.99",
                "port": 22,
            }),
            &config,
            tx,
            active_tunnels.clone(),
            None,
            replay_guard(),
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("close message")))
                .expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["session_id"], "sess-deny");
        assert!(
            closed["error"]
                .as_str()
                .expect("error")
                .contains("allowed_targets"),
        );
        assert!(active_tunnels.lock().await.is_empty());
    }

    #[tokio::test]
    async fn ssh_tunnel_rejects_when_max_tunnels_reached() {
        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));
        let (placeholder_tx, _placeholder_rx) = mpsc::channel(SSH_CONTROL_CHANNEL_SIZE);
        let placeholder_task = tokio::spawn(async {
            futures::future::pending::<()>().await;
        });
        active_tunnels.lock().await.insert(
            "existing".to_string(),
            ActiveSshTunnelEntry {
                control_tx: placeholder_tx,
                task_handle: placeholder_task,
            },
        );

        handle_ssh_tunnel_open(
            &serde_json::json!({
                "session_id": "sess-3",
                "host": "ssh.example.com",
                "port": 22,
            }),
            &SshConfig {
                max_tunnels: 1,
                io_timeout_secs: 3600,
                allowed_targets: vec![SshTargetConfig {
                    host: "ssh.example.com".to_string(),
                    port: Some(22),
                }],
            },
            tx,
            active_tunnels.clone(),
            None,
            replay_guard(),
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("close message")))
                .expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["error"], "too_many_active_tunnels");
        assert_eq!(active_tunnels.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn ssh_tunnel_rejects_invalid_hmac_signature() {
        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));

        let mut request = signed_ssh_tunnel_open_request(
            "sess-bad-sig",
            "svc-1",
            "ssh.example.com",
            22,
            &"ab".repeat(32),
        );
        request["host"] = serde_json::Value::String("tampered.example.com".to_string());

        handle_ssh_tunnel_open(
            &request,
            &ssh_config_with_allowed_target("ssh.example.com", 22),
            tx,
            active_tunnels.clone(),
            Some(shared_signing_secret(&"ab".repeat(32))),
            replay_guard(),
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("close message")))
                .expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["session_id"], "sess-bad-sig");
        assert_eq!(closed["error"], "invalid_hmac_signature");
        assert!(active_tunnels.lock().await.is_empty());
    }

    #[tokio::test]
    async fn read_ssh_tunnel_stream_times_out() {
        let (mut stream, _peer) = tokio::io::duplex(64);
        let mut buffer = [0_u8; 8];

        let error = read_ssh_tunnel_stream(&mut stream, &mut buffer, Duration::from_millis(10))
            .await
            .expect_err("timeout");

        match error {
            Error::Io(io_error) => assert_eq!(io_error.kind(), std::io::ErrorKind::TimedOut),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_ssh_tunnel_stream_times_out() {
        let (mut stream, _peer) = tokio::io::duplex(1);

        let error = write_ssh_tunnel_stream(&mut stream, b"ab", Duration::from_millis(10))
            .await
            .expect_err("timeout");

        match error {
            Error::Io(io_error) => assert_eq!(io_error.kind(), std::io::ErrorKind::TimedOut),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ssh_tunnel_buffer_overflow_aborts_active_task() {
        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));
        let (control_tx, _control_rx) = mpsc::channel(1);
        control_tx
            .try_send(SshTunnelControl::Data(vec![1]))
            .expect("fill control buffer");
        let task_handle = tokio::spawn(async {
            futures::future::pending::<()>().await;
        });
        let abort_handle = task_handle.abort_handle();

        active_tunnels.lock().await.insert(
            "sess-4".to_string(),
            ActiveSshTunnelEntry {
                control_tx,
                task_handle,
            },
        );

        handle_ssh_tunnel_data(
            &serde_json::json!({
                "session_id": "sess-4",
                "data": base64::engine::general_purpose::STANDARD.encode(b"hello"),
            }),
            &tx,
            &active_tunnels,
        )
        .await;

        tokio::task::yield_now().await;

        let closed: serde_json::Value =
            serde_json::from_str(&unwrap_text(rx.recv().await.expect("close message")))
                .expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["session_id"], "sess-4");
        assert_eq!(closed["error"], "control_buffer_full");
        assert!(active_tunnels.lock().await.is_empty());
        assert!(abort_handle.is_finished());
    }

    #[tokio::test]
    async fn drain_active_ssh_tunnels_aborts_active_entries() {
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));
        let (control_tx, _control_rx) = mpsc::channel(1);
        let task_handle = tokio::spawn(async {
            futures::future::pending::<()>().await;
        });
        let abort_handle = task_handle.abort_handle();

        active_tunnels.lock().await.insert(
            "sess-5".to_string(),
            ActiveSshTunnelEntry {
                control_tx,
                task_handle,
            },
        );

        drain_active_ssh_tunnels(&active_tunnels).await;
        tokio::task::yield_now().await;

        assert!(active_tunnels.lock().await.is_empty());
        assert!(abort_handle.is_finished());
    }

    #[tokio::test]
    async fn close_active_ssh_tunnels_requests_graceful_close() {
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        let tunnels = active_tunnels.clone();
        let task_handle = tokio::spawn(async move {
            if matches!(control_rx.recv().await, Some(SshTunnelControl::Close)) {
                let _ = remove_ssh_tunnel_entry(&tunnels, "sess-6").await;
                let _ = closed_tx.send(());
            }
        });

        active_tunnels.lock().await.insert(
            "sess-6".to_string(),
            ActiveSshTunnelEntry {
                control_tx,
                task_handle,
            },
        );

        close_active_ssh_tunnels(&active_tunnels, Duration::from_millis(200)).await;

        closed_rx.await.expect("close signal");
        assert!(active_tunnels.lock().await.is_empty());
    }

    #[tokio::test]
    async fn validate_node_ssh_target_blocks_metadata() {
        let config = SshConfig::default();
        assert!(
            validate_node_ssh_target(&config, "metadata.google.internal", 22)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_node_ssh_target_allows_private_ips() {
        let config = SshConfig::default();
        assert!(
            validate_node_ssh_target(&config, "192.168.1.50", 22)
                .await
                .is_ok()
        );
        assert!(
            validate_node_ssh_target(&config, "100.64.0.10", 22)
                .await
                .is_ok()
        );
        assert!(
            validate_node_ssh_target(&config, "127.0.0.1", 22)
                .await
                .is_ok()
        );
    }
}
