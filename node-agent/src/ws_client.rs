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

use crate::config::{NodeConfig, SshConfig};
use crate::credential_store::SharedCredentials;
use crate::error::{Error, Result};
use crate::metrics::NodeMetrics;
use crate::proxy_executor;
use crate::signing::{self, ReplayGuard};

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

const SSH_CONTROL_CHANNEL_SIZE: usize = 256;
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;
const SSH_SHUTDOWN_DRAIN_TIMEOUT_SECS: u64 = 5;
const AGENT_SHUTDOWN_TIMEOUT_SECS: u64 = 30;
const WS_WRITE_CHANNEL_SIZE: usize = 256;

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
    auth_token: String,
    signing_secret: Option<String>,
    credentials: SharedCredentials,
) {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let in_flight_shutdown = in_flight.clone();
    let signing_secret = signing_secret.map(|secret| Arc::new(Zeroizing::new(secret)));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut connection_task = tokio::spawn({
        let in_flight = in_flight.clone();
        async move {
            run_connection_loop(
                &config,
                &auth_token,
                signing_secret,
                &credentials,
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
async fn run_connection_loop(
    config: &NodeConfig,
    auth_token: &str,
    signing_secret: Option<SharedSigningSecret>,
    credentials: &SharedCredentials,
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
            auth_token,
            signing_secret.clone(),
            credentials,
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

/// Single connection lifecycle: connect, authenticate, serve requests.
async fn connect_and_serve(
    config: &NodeConfig,
    auth_token: &str,
    signing_secret: Option<SharedSigningSecret>,
    credentials: &SharedCredentials,
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
    match parsed["type"].as_str() {
        Some("auth_ok") => {
            tracing::info!(node_id = %config.node.id, "Authenticated with NyxID server");
        }
        Some("auth_error") => {
            let msg = parsed["message"].as_str().unwrap_or("unknown");
            return Err(Error::AuthFailed(msg.to_string()));
        }
        _ => {
            return Err(Error::AuthFailed(format!("Unexpected response: {text}")));
        }
    }

    let proxy_http_client = proxy_executor::build_http_client()?;

    // 4. Set up writer channel
    let (tx, mut rx) = mpsc::channel::<String>(WS_WRITE_CHANNEL_SIZE);
    let active_ssh_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
        String,
        ActiveSshTunnelEntry,
    >::new()));
    let active_web_terminals: ActiveWebTerminalMap =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Writer task: forwards messages from the channel to the WS sink
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Shared state for the reader loop
    let metrics = Arc::new(NodeMetrics::new());
    let replay_guard = Arc::new(tokio::sync::Mutex::new(ReplayGuard::new()));

    // 5. Reader loop: process incoming messages from the server
    let shutting_down = loop {
        let Some(msg) = (tokio::select! {
            msg = ws_stream.next() => msg,
            _ = wait_for_shutdown(&mut shutdown) => break true,
        }) else {
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
    writer_task.abort();
    Ok(())
}

async fn handle_ssh_tunnel_open(
    parsed: &serde_json::Value,
    ssh_config: &SshConfig,
    tx: mpsc::Sender<String>,
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
    tx: &mpsc::Sender<String>,
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
    tx: mpsc::Sender<String>,
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
    tx: &mpsc::Sender<String>,
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
    tx: &mpsc::Sender<String>,
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
    tx: mpsc::Sender<String>,
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
    tx: &mpsc::Sender<String>,
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
    tx: mpsc::Sender<String>,
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
    tx: &mpsc::Sender<String>,
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
    tx: &mpsc::Sender<String>,
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

async fn send_ws_message(tx: &mpsc::Sender<String>, message: String) -> bool {
    tx.send(message).await.is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SshConfig, SshTargetConfig};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tokio::net::TcpListener;

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
            serde_json::from_str(&rx.recv().await.expect("opened message")).expect("opened json");
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
            serde_json::from_str(&rx.recv().await.expect("data message")).expect("data json");
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
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
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
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
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

        let mut config = SshConfig::default();
        config.allowed_targets = vec![crate::config::SshTargetConfig {
            host: "10.0.0.1".to_string(),
            port: Some(22),
        }];

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
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
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
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
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
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
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
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
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
