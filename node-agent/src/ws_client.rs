use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{NodeConfig, SshConfig};
use crate::credential_store::SharedCredentials;
use crate::error::{Error, Result};
use crate::metrics::NodeMetrics;
use crate::proxy_executor;
use crate::signing::ReplayGuard;

enum SshTunnelControl {
    Data(Vec<u8>),
    Close,
}

const SSH_CONTROL_CHANNEL_SIZE: usize = 256;
const WS_WRITE_CHANNEL_SIZE: usize = 256;

type ActiveSshTunnelMap = Arc<tokio::sync::Mutex<HashMap<String, ActiveSshTunnelEntry>>>;

enum ActiveSshTunnelEntry {
    Opening {
        control_tx: mpsc::Sender<SshTunnelControl>,
    },
    Active {
        control_tx: mpsc::Sender<SshTunnelControl>,
        task_handle: tokio::task::JoinHandle<()>,
    },
}

impl ActiveSshTunnelEntry {
    fn control_tx(&self) -> mpsc::Sender<SshTunnelControl> {
        match self {
            Self::Opening { control_tx } | Self::Active { control_tx, .. } => control_tx.clone(),
        }
    }

    fn abort(self) {
        if let Self::Active { task_handle, .. } = self {
            task_handle.abort();
        }
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

    tokio::select! {
        () = run_connection_loop(&config, &auth_token, signing_secret.as_deref(), &credentials, in_flight) => {},
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received, draining in-flight requests...");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while in_flight_shutdown.load(Ordering::Relaxed) > 0
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let remaining = in_flight_shutdown.load(Ordering::Relaxed);
            if remaining > 0 {
                tracing::warn!(remaining, "Forcing shutdown with in-flight requests");
            }
            tracing::info!("Shutdown complete");
        }
    }
}

/// Main connection loop with exponential backoff reconnection.
async fn run_connection_loop(
    config: &NodeConfig,
    auth_token: &str,
    signing_secret: Option<&str>,
    credentials: &SharedCredentials,
    in_flight: Arc<AtomicUsize>,
) {
    let mut backoff =
        ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(60), 2.0);

    loop {
        match connect_and_serve(
            config,
            auth_token,
            signing_secret,
            credentials,
            in_flight.clone(),
        )
        .await
        {
            Ok(()) => {
                tracing::info!("Disconnected cleanly, reconnecting...");
                backoff.reset();
            }
            Err(e) => {
                let delay = backoff.next_delay();
                tracing::warn!(
                    error = %e,
                    delay_ms = delay.as_millis(),
                    "Connection failed, retrying"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Single connection lifecycle: connect, authenticate, serve requests.
async fn connect_and_serve(
    config: &NodeConfig,
    auth_token: &str,
    signing_secret: Option<&str>,
    credentials: &SharedCredentials,
    in_flight: Arc<AtomicUsize>,
) -> Result<()> {
    // 1. Connect
    let (ws_stream, _) = tokio_tungstenite::connect_async(&config.server.url)
        .await
        .map_err(|e| Error::WebSocket(format!("Failed to connect: {e}")))?;

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // 2. Authenticate
    let auth_msg = serde_json::json!({
        "type": "auth",
        "node_id": config.node.id,
        "token": auth_token,
    });
    ws_sink
        .send(Message::Text(auth_msg.to_string().into()))
        .await
        .map_err(|e| Error::WebSocket(format!("Failed to send auth: {e}")))?;

    // 3. Wait for auth_ok
    let response = tokio::time::timeout(Duration::from_secs(10), ws_stream.next())
        .await
        .map_err(|_| Error::AuthFailed("Timed out waiting for auth response".to_string()))?
        .ok_or_else(|| Error::AuthFailed("Connection closed during auth".to_string()))?
        .map_err(|e| Error::WebSocket(format!("Read error during auth: {e}")))?;

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

    // 4. Set up writer channel
    let (tx, mut rx) = mpsc::channel::<String>(WS_WRITE_CHANNEL_SIZE);
    let active_ssh_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
        String,
        ActiveSshTunnelEntry,
    >::new()));

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
    while let Some(msg) = ws_stream.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => continue,
            Ok(_) => continue,
            Err(e) => {
                tracing::debug!(error = %e, "WebSocket read error");
                break;
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
                    break;
                }
            }
            Some("proxy_request") => {
                let tx_clone = tx.clone();
                let creds = credentials.snapshot();
                let secret = signing_secret.map(String::from);
                let replay = replay_guard.clone();
                let metrics_clone = metrics.clone();
                let in_flight_clone = in_flight.clone();

                in_flight_clone.fetch_add(1, Ordering::Relaxed);

                tokio::spawn(async move {
                    proxy_executor::execute_proxy_request(
                        &parsed,
                        &creds,
                        secret.as_deref(),
                        &replay,
                        &metrics_clone,
                        &tx_clone,
                    )
                    .await;
                    in_flight_clone.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Some("ssh_tunnel_open") => {
                let tx_clone = tx.clone();
                let ssh_config = config.ssh.clone();
                let active_tunnels = active_ssh_tunnels.clone();
                tokio::spawn(async move {
                    handle_ssh_tunnel_open(&parsed, &ssh_config, tx_clone, active_tunnels).await;
                });
            }
            Some("ssh_tunnel_data") => {
                handle_ssh_tunnel_data(&parsed, &tx, &active_ssh_tunnels).await;
            }
            Some("ssh_tunnel_close") => {
                handle_ssh_tunnel_close(&parsed, &active_ssh_tunnels).await;
            }
            Some("error") => {
                let msg = parsed["message"].as_str().unwrap_or("unknown");
                tracing::error!(message = %msg, "Server error");
            }
            other => {
                tracing::debug!(msg_type = ?other, "Unknown message type");
            }
        }
    }

    writer_task.abort();
    Ok(())
}

async fn handle_ssh_tunnel_open(
    parsed: &serde_json::Value,
    ssh_config: &SshConfig,
    tx: mpsc::Sender<String>,
    active_tunnels: ActiveSshTunnelMap,
) {
    let session_id = match parsed["session_id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            tracing::warn!("ssh_tunnel_open missing session_id");
            return;
        }
    };
    let host = match parsed["host"].as_str() {
        Some(host) if !host.is_empty() => host.to_string(),
        _ => {
            tracing::warn!(session_id = %session_id, "ssh_tunnel_open missing host");
            let _ = send_ws_message(
                &tx,
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": "missing_host",
                })
                .to_string(),
            )
            .await;
            return;
        }
    };
    let port = match parsed["port"].as_u64() {
        Some(port) if u16::try_from(port).is_ok() => port as u16,
        _ => {
            tracing::warn!(session_id = %session_id, "ssh_tunnel_open missing or invalid port");
            let _ = send_ws_message(
                &tx,
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": "invalid_port",
                })
                .to_string(),
            )
            .await;
            return;
        }
    };

    if let Err(error) = validate_node_ssh_target(ssh_config, &host, port).await {
        tracing::warn!(session_id = %session_id, host = %host, port, %error, "ssh tunnel target rejected by node policy");
        let _ = send_ws_message(
            &tx,
            serde_json::json!({
                "type": "ssh_tunnel_closed",
                "session_id": session_id,
                "error": format!("target_not_allowed:{error}"),
            })
            .to_string(),
        )
        .await;
        return;
    }

    let (control_tx, mut control_rx) = mpsc::channel(SSH_CONTROL_CHANNEL_SIZE);
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
            guard.insert(
                session_id.clone(),
                ActiveSshTunnelEntry::Opening {
                    control_tx: control_tx.clone(),
                },
            );
            None
        }
    };
    if let Some(message) = open_rejection {
        let _ = send_ws_message(&tx, message).await;
        return;
    }

    let address = format!("{host}:{port}");
    let stream = match TcpStream::connect(&address).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(session_id = %session_id, %address, %error, "failed to open ssh tunnel tcp stream");
            let _ = remove_ssh_tunnel_entry(&active_tunnels, &session_id).await;
            let _ = send_ws_message(
                &tx,
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": format!("connect_failed:{error}"),
                })
                .to_string(),
            )
            .await;
            return;
        }
    };

    let io_timeout = Duration::from_secs(ssh_config.io_timeout_secs);
    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    let task_session_id = session_id.clone();
    let task_tx = tx.clone();
    let task_tunnels = active_tunnels.clone();
    let task_handle = tokio::spawn(async move {
        if start_rx.await.is_err() {
            return;
        }

        run_ssh_tunnel_task(
            task_session_id,
            stream,
            &mut control_rx,
            task_tx,
            task_tunnels,
            io_timeout,
        )
        .await;
    });
    {
        let mut guard = active_tunnels.lock().await;
        let Some(entry) = guard.get_mut(&session_id) else {
            task_handle.abort();
            return;
        };
        *entry = ActiveSshTunnelEntry::Active {
            control_tx: control_tx.clone(),
            task_handle,
        };
    }
    let _ = start_tx.send(());

    let _ = send_ws_message(
        &tx,
        serde_json::json!({
            "type": "ssh_tunnel_opened",
            "session_id": session_id,
        })
        .to_string(),
    )
    .await;
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
        match entry {
            ActiveSshTunnelEntry::Opening { .. } => {}
            ActiveSshTunnelEntry::Active {
                control_tx,
                task_handle,
            } => match control_tx.try_send(SshTunnelControl::Close) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_))
                | Err(mpsc::error::TrySendError::Closed(_)) => task_handle.abort(),
            },
        }
    }
}

async fn run_ssh_tunnel_task(
    session_id: String,
    mut stream: TcpStream,
    control_rx: &mut mpsc::Receiver<SshTunnelControl>,
    tx: mpsc::Sender<String>,
    active_tunnels: ActiveSshTunnelMap,
    io_timeout: Duration,
) {
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
    let _ = send_ws_message(
        tx,
        serde_json::json!({
            "type": "ssh_tunnel_closed",
            "session_id": session_id,
            "error": error,
        })
        .to_string(),
    )
    .await;
}

async fn remove_ssh_tunnel_entry(
    active_tunnels: &ActiveSshTunnelMap,
    session_id: &str,
) -> Option<ActiveSshTunnelEntry> {
    active_tunnels.lock().await.remove(session_id)
}

async fn validate_node_ssh_target(ssh_config: &SshConfig, host: &str, port: u16) -> Result<()> {
    if is_allowlisted_ssh_target(ssh_config, host, port) {
        return Ok(());
    }

    let normalized_host = normalize_target_host(host);
    if matches!(
        normalized_host.as_str(),
        "localhost" | "metadata.google.internal"
    ) {
        return Err(Error::Validation(
            "SSH target must be explicitly allowlisted in the node config".to_string(),
        ));
    }

    let addresses = resolve_target_ips(host, port).await?;
    if addresses.is_empty() {
        return Err(Error::Validation(
            "SSH target did not resolve to any IP addresses".to_string(),
        ));
    }
    if addresses.iter().copied().any(is_private_or_internal_ip) {
        return Err(Error::Validation(
            "SSH target resolves to a private or internal address and must be allowlisted in the node config".to_string(),
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

async fn resolve_target_ips(host: &str, port: u16) -> Result<Vec<std::net::IpAddr>> {
    if let Ok(ip) = parse_target_ip(host) {
        return Ok(vec![ip]);
    }

    let resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| Error::Validation(format!("Failed to resolve SSH target: {error}")))?;
    Ok(resolved.map(|addr| addr.ip()).collect())
}

fn parse_target_ip(host: &str) -> Result<std::net::IpAddr> {
    normalize_target_host(host)
        .parse()
        .map_err(|error| Error::Validation(format!("Invalid SSH target host: {error}")))
}

fn normalize_target_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn is_private_or_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_unspecified()
                || ipv4.is_broadcast()
                || ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254
        }
        std::net::IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_multicast()
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
                || ipv6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private_or_internal_ip(mapped.into()))
        }
    }
}

async fn send_ws_message(tx: &mpsc::Sender<String>, message: String) -> bool {
    tx.send(message).await.is_ok()
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
            &serde_json::json!({
                "session_id": "sess-1",
                "host": "127.0.0.1",
                "port": port,
            }),
            &ssh_config_with_allowed_target("127.0.0.1", port),
            tx.clone(),
            active_tunnels.clone(),
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
    async fn ssh_tunnel_rejects_private_targets_without_allowlist() {
        let (tx, mut rx) = mpsc::channel(8);
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            ActiveSshTunnelEntry,
        >::new()));

        handle_ssh_tunnel_open(
            &serde_json::json!({
                "session_id": "sess-2",
                "host": "127.0.0.1",
                "port": 22,
            }),
            &SshConfig::default(),
            tx,
            active_tunnels.clone(),
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["session_id"], "sess-2");
        assert!(
            closed["error"]
                .as_str()
                .expect("error")
                .starts_with("target_not_allowed:")
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
        active_tunnels.lock().await.insert(
            "existing".to_string(),
            ActiveSshTunnelEntry::Opening {
                control_tx: placeholder_tx,
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
        )
        .await;

        let closed: serde_json::Value =
            serde_json::from_str(&rx.recv().await.expect("close message")).expect("close json");
        assert_eq!(closed["type"], "ssh_tunnel_closed");
        assert_eq!(closed["error"], "too_many_active_tunnels");
        assert_eq!(active_tunnels.lock().await.len(), 1);
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
            ActiveSshTunnelEntry::Active {
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

    #[test]
    fn rejects_ipv6_multicast_targets() {
        assert!(is_private_or_internal_ip(
            "ff02::1".parse().expect("valid multicast IPv6")
        ));
        assert!(!is_private_or_internal_ip(
            "2001:4860:4860::8888".parse().expect("valid public IPv6")
        ));
    }
}
