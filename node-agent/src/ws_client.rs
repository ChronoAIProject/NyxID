use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::NodeConfig;
use crate::credential_store::SharedCredentials;
use crate::error::{Error, Result};
use crate::metrics::NodeMetrics;
use crate::proxy_executor;
use crate::signing::ReplayGuard;

enum SshTunnelControl {
    Data(Vec<u8>),
    Close,
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
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let active_ssh_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
        String,
        mpsc::UnboundedSender<SshTunnelControl>,
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
                let _ = tx.send(pong.to_string());
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
                let active_tunnels = active_ssh_tunnels.clone();
                tokio::spawn(async move {
                    handle_ssh_tunnel_open(&parsed, tx_clone, active_tunnels).await;
                });
            }
            Some("ssh_tunnel_data") => {
                handle_ssh_tunnel_data(&parsed, &active_ssh_tunnels).await;
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
    tx: mpsc::UnboundedSender<String>,
    active_tunnels: Arc<
        tokio::sync::Mutex<HashMap<String, mpsc::UnboundedSender<SshTunnelControl>>>,
    >,
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
            let _ = tx.send(
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": "missing_host",
                })
                .to_string(),
            );
            return;
        }
    };
    let port = match parsed["port"].as_u64() {
        Some(port) if u16::try_from(port).is_ok() => port as u16,
        _ => {
            tracing::warn!(session_id = %session_id, "ssh_tunnel_open missing or invalid port");
            let _ = tx.send(
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": "invalid_port",
                })
                .to_string(),
            );
            return;
        }
    };

    let address = format!("{host}:{port}");
    let stream = match TcpStream::connect(&address).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(session_id = %session_id, %address, %error, "failed to open ssh tunnel tcp stream");
            let _ = tx.send(
                serde_json::json!({
                    "type": "ssh_tunnel_closed",
                    "session_id": session_id,
                    "error": format!("connect_failed:{error}"),
                })
                .to_string(),
            );
            return;
        }
    };

    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    active_tunnels
        .lock()
        .await
        .insert(session_id.clone(), control_tx);

    let _ = tx.send(
        serde_json::json!({
            "type": "ssh_tunnel_opened",
            "session_id": session_id,
        })
        .to_string(),
    );

    tokio::spawn(async move {
        let mut stream = stream;
        let mut read_buf = [0u8; 16 * 1024];

        loop {
            tokio::select! {
                control = control_rx.recv() => {
                    match control {
                        Some(SshTunnelControl::Data(bytes)) => {
                            if let Err(error) = stream.write_all(&bytes).await {
                                tracing::warn!(session_id = %session_id, %error, "failed to write ssh tunnel bytes");
                                break;
                            }
                        }
                        Some(SshTunnelControl::Close) | None => break,
                    }
                }
                read_result = stream.read(&mut read_buf) => {
                    match read_result {
                        Ok(0) => break,
                        Ok(n) => {
                            let _ = tx.send(
                                serde_json::json!({
                                    "type": "ssh_tunnel_data",
                                    "session_id": session_id,
                                    "data": base64::engine::general_purpose::STANDARD
                                        .encode(&read_buf[..n]),
                                })
                                .to_string(),
                            );
                        }
                        Err(error) => {
                            tracing::warn!(session_id = %session_id, %error, "failed reading ssh tunnel bytes");
                            break;
                        }
                    }
                }
            }
        }

        active_tunnels.lock().await.remove(&session_id);
        let _ = tx.send(
            serde_json::json!({
                "type": "ssh_tunnel_closed",
                "session_id": session_id,
            })
            .to_string(),
        );
    });
}

async fn handle_ssh_tunnel_data(
    parsed: &serde_json::Value,
    active_tunnels: &Arc<
        tokio::sync::Mutex<HashMap<String, mpsc::UnboundedSender<SshTunnelControl>>>,
    >,
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
        guard.get(session_id).cloned()
    };
    if let Some(sender) = sender {
        let _ = sender.send(SshTunnelControl::Data(bytes));
    }
}

async fn handle_ssh_tunnel_close(
    parsed: &serde_json::Value,
    active_tunnels: &Arc<
        tokio::sync::Mutex<HashMap<String, mpsc::UnboundedSender<SshTunnelControl>>>,
    >,
) {
    let Some(session_id) = parsed["session_id"].as_str() else {
        tracing::warn!("ssh_tunnel_close missing session_id");
        return;
    };

    let sender = active_tunnels.lock().await.remove(session_id);
    if let Some(sender) = sender {
        let _ = sender.send(SshTunnelControl::Close);
    }
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
    use tokio::net::TcpListener;

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

        let (tx, mut rx) = mpsc::unbounded_channel();
        let active_tunnels = Arc::new(tokio::sync::Mutex::new(HashMap::<
            String,
            mpsc::UnboundedSender<SshTunnelControl>,
        >::new()));

        handle_ssh_tunnel_open(
            &serde_json::json!({
                "session_id": "sess-1",
                "host": "127.0.0.1",
                "port": port,
            }),
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
}
