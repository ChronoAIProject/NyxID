use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{
        ConnectInfo, Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::ssh_auth_mode::SshAuthMode;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    approval_service, audit_service, node_routing_service, node_service, notification_service,
    operation_descriptor, ssh_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

use super::services_helpers::fetch_service;

#[derive(Debug, Deserialize, ToSchema)]
pub struct IssueSshCertificateRequest {
    pub public_key: String,
    pub principal: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueSshCertificateResponse {
    pub service_id: String,
    pub key_id: String,
    pub principal: String,
    pub certificate: String,
    pub ca_public_key: String,
    pub valid_after: String,
    pub valid_before: String,
}

#[derive(Clone)]
struct TunnelClientMeta {
    ip_address: Option<String>,
    user_agent: Option<String>,
}

const MAX_SSH_BANNER_BYTES: usize = 4 * 1024;

#[utoipa::path(
    post,
    path = "/api/v1/ssh/{service_id}/certificate",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    request_body = IssueSshCertificateRequest,
    responses(
        (status = 200, description = "Issued short-lived SSH certificate", body = IssueSshCertificateResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn issue_ssh_certificate(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(service_id): Path<String>,
    Json(body): Json<IssueSshCertificateRequest>,
) -> AppResult<Json<IssueSshCertificateResponse>> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    // Resolve the catalog slug for telemetry -- the path param is the
    // service UUID, not the slug. Best-effort: cert issuance already
    // passed auth + SSH config validation, so a transient read failure
    // here (or the service getting deleted between auth and this
    // lookup) must never fail the issuance. Fall back to the UUID and
    // accept that the event will be scrubbed to `[UUID_REDACTED]` in
    // that rare case.
    let service_slug = fetch_service(&state, &service_id)
        .await
        .ok()
        .map(|s| s.slug)
        .unwrap_or_else(|| service_id.clone());
    let user_id = auth_user.user_id.to_string();
    let auth_context =
        ssh_service::resolve_ssh_auth_context(&state.db, &user_id, &service_id, &service_slug)
            .await?;
    if auth_context.mode != SshAuthMode::Cert {
        return Err(AppError::SshAuthModeUnsupportedForOperation(
            "SSH certificate issuance is only supported for cert-mode SSH services".to_string(),
        ));
    }
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let issued = ssh_service::issue_certificate(
        &state.encryption_keys,
        &ssh_service,
        &service_id,
        &user_id,
        &user.email,
        &body.public_key,
        body.principal.trim(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "ssh_certificate_issued",
        Some(serde_json::json!({
            "service_id": service_id,
            "key_id": issued.key_id,
            "principal": issued.principal,
            "routed_via": "ssh",
            "valid_after": issued.valid_after,
            "valid_before": issued.valid_before,
        })),
    );

    // Telemetry: ssh.certificate_issued. `ttl_secs` derived from the
    // certificate validity window (not the configured TTL minutes, so it
    // reflects the actual issuance). `service_slug` comes from the
    // resolved downstream service, not the path param.
    let ttl_secs = (issued.valid_before - issued.valid_after).num_seconds();
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::SshCertificateIssued {
            service_slug,
            ttl_secs,
        },
    );

    Ok(Json(IssueSshCertificateResponse {
        service_id,
        key_id: issued.key_id,
        principal: issued.principal,
        certificate: issued.certificate,
        ca_public_key: issued.ca_public_key,
        valid_after: issued.valid_after.to_rfc3339(),
        valid_before: issued.valid_before.to_rfc3339(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/ssh/{service_id}",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 101, description = "Switching protocols to WebSocket for SSH tunnel"),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn ssh_tunnel_ws(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    // Resolve the downstream service so telemetry
    // (`ssh.tunnel_opened` / `_closed`) emits the slug, not the UUID.
    // Best-effort: SSH tunnel establishment is user-facing; a transient
    // telemetry-only lookup failure must never block the tunnel.
    let service_slug = fetch_service(&state, &service_id)
        .await
        .ok()
        .map(|s| s.slug)
        .unwrap_or_else(|| service_id.clone());
    let auth_context = ssh_service::resolve_ssh_auth_context(
        &state.db,
        &auth_user.user_id.to_string(),
        &service_id,
        &service_slug,
    )
    .await?;
    if auth_context.mode == SshAuthMode::NodeKey {
        return Err(AppError::SshAuthModeUnsupportedForOperation(
            "ssh proxy is not supported for node-key SSH services; use ssh exec or terminal"
                .to_string(),
        ));
    }
    validate_runtime_ssh_target(&service_id, &ssh_service).await?;
    let session_guard = state
        .ssh_session_manager
        .try_acquire(&auth_user.user_id.to_string())?;
    let client_meta = TunnelClientMeta {
        ip_address: Some(addr.ip().to_string()),
        user_agent: headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    };

    // Snapshot the request-scoped telemetry context BEFORE the WS upgrade
    // so SshTunnelOpened / SshTunnelClosed (fired from the spawned socket
    // task) can carry the original `X-NyxID-Client` surface. Without this
    // the websocket task falls back to `TelemetryContext::default()` and
    // every SSH session gets recorded as `surface="backend"`.
    let tele = TelemetryContext::from_headers(
        headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
        headers
            .get("x-nyxid-client-version")
            .and_then(|v| v.to_str().ok()),
    );

    Ok(ws
        .on_upgrade(move |socket| async move {
            handle_ssh_socket(
                state,
                auth_user,
                service_id,
                service_slug,
                ssh_service,
                socket,
                session_guard,
                client_meta,
                tele,
            )
            .await;
        })
        .into_response())
}

#[allow(clippy::too_many_arguments)]
async fn handle_ssh_socket(
    state: AppState,
    auth_user: AuthUser,
    service_id: String,
    service_slug: String,
    ssh_service: crate::models::downstream_service::SshServiceConfig,
    mut socket: WebSocket,
    session_guard: ssh_service::SshSessionGuard,
    client_meta: TunnelClientMeta,
    tele: TelemetryContext,
) {
    // Held for Drop-based session count cleanup for the tunnel lifetime.
    let _ = &session_guard;
    let user_id = auth_user.user_id.to_string();
    let session_id = uuid::Uuid::new_v4().to_string();
    let started_at = Instant::now();
    let node_route = match node_routing_service::resolve_node_route(
        &state.db,
        &user_id,
        &service_id,
        &state.node_ws_manager,
    )
    .await
    {
        Ok(route) => route,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Failed to resolve SSH node route");
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Failed to resolve SSH route".into(),
                })))
                .await;
            return;
        }
    };

    if let Some(node_route) = node_route {
        handle_node_ssh_socket(
            state,
            service_id,
            service_slug,
            ssh_service,
            socket,
            user_id,
            auth_user.api_key_id.clone(),
            session_id,
            started_at,
            client_meta,
            node_route,
            tele,
        )
        .await;
        return;
    }

    let connect_target = format!("{}:{}", ssh_service.host, ssh_service.port);
    let mut tcp_stream = match tokio::time::timeout(
        Duration::from_secs(state.config.ssh_connect_timeout_secs),
        tokio::net::TcpStream::connect(&connect_target),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            tracing::warn!(service_id = %service_id, error = %error, "SSH tunnel connect failed");
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Failed to connect downstream SSH target".into(),
                })))
                .await;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "ssh",
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": error.to_string(),
                })),
                client_meta.ip_address,
                client_meta.user_agent,
                None,
                None,
            );
            return;
        }
        Err(_) => {
            tracing::warn!(
                service_id = %service_id,
                timeout_secs = state.config.ssh_connect_timeout_secs,
                "SSH tunnel connect timed out"
            );
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "SSH target connect timed out".into(),
                })))
                .await;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "ssh",
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": "connect_timeout",
                    "timeout_secs": state.config.ssh_connect_timeout_secs,
                })),
                client_meta.ip_address,
                client_meta.user_agent,
                None,
                None,
            );
            return;
        }
    };

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let initial_downstream_bytes = match read_direct_ssh_banner(
        &mut tcp_stream,
        state.config.ssh_connect_timeout_secs,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "SSH tunnel target failed banner validation");
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Downstream target is not a valid SSH server".into(),
                })))
                .await;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "ssh",
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": error.to_string(),
                })),
                client_meta.ip_address,
                client_meta.user_agent,
                None,
                None,
            );
            return;
        }
    };
    if socket
        .send(Message::Binary(initial_downstream_bytes.clone().into()))
        .await
        .is_err()
    {
        audit_service::log_async(
            state.db.clone(),
            Some(user_id.clone()),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "ssh",
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "banner_send_failed",
            })),
            client_meta.ip_address.clone(),
            client_meta.user_agent.clone(),
            None,
            None,
        );
        return;
    }
    to_client_bytes += initial_downstream_bytes.len() as u64;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_tunnel_connected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "ssh",
            "target_host": ssh_service.host,
            "target_port": ssh_service.port,
        })),
        client_meta.ip_address.clone(),
        client_meta.user_agent.clone(),
        None,
        None,
    );

    // Telemetry: ssh.tunnel_opened (direct path).
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::SshTunnelOpened {
            service_slug: service_slug.clone(),
            mode: "tunnel".to_string(),
        },
    );

    let mut read_buf = vec![0_u8; 16 * 1024];
    let tunnel_timeout = tokio::time::sleep(Duration::from_secs(
        state.config.ssh_max_tunnel_duration_secs,
    ));
    tokio::pin!(tunnel_timeout);

    let disconnect_reason = loop {
        tokio::select! {
            _ = &mut tunnel_timeout => {
                let _ = socket
                    .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 1008,
                        reason: "SSH tunnel reached maximum duration".into(),
                    })))
                    .await;
                break Some("max_tunnel_duration_exceeded");
            }
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if tcp_stream.write_all(&bytes).await.is_err() {
                            break Some("downstream_write_failed");
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break Some("client_closed");
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break Some("invalid_client_frame");
                    }
                    Some(Err(_)) => break Some("client_socket_error"),
                }
            }
            tcp_read = tcp_stream.read(&mut read_buf) => {
                match tcp_read {
                    Ok(0) => break Some("downstream_closed"),
                    Ok(n) => {
                        to_client_bytes += n as u64;
                        if socket.send(Message::Binary(read_buf[..n].to_vec().into())).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Err(_) => break Some("downstream_read_failed"),
                }
            }
        }
    };

    let _ = socket.close().await;

    let duration_ms = started_at.elapsed().as_millis() as u64;
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_tunnel_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "ssh",
            "duration_ms": duration_ms,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
            "disconnect_reason": disconnect_reason,
        })),
        client_meta.ip_address,
        client_meta.user_agent,
        None,
        None,
    );

    // Telemetry: ssh.tunnel_closed (direct path).
    // Best-effort: normal close only, abrupt close may miss -- see
    // TELEMETRY.md §6.5.
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::SshTunnelClosed {
            service_slug,
            duration_ms,
        },
    );
}

#[allow(clippy::too_many_arguments)]
async fn handle_node_ssh_socket(
    state: AppState,
    service_id: String,
    service_slug: String,
    ssh_service: crate::models::downstream_service::SshServiceConfig,
    mut socket: WebSocket,
    user_id: String,
    api_key_id: Option<String>,
    session_id: String,
    started_at: Instant,
    client_meta: TunnelClientMeta,
    node_route: crate::services::node_routing_service::NodeRoute,
    tele: TelemetryContext,
) {
    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    let mut tunnel_rx = None;
    let mut selected_node_id = None;

    for node_id in all_node_ids {
        let signing_secret = if state.config.node_hmac_signing_enabled {
            match node_service::get_node_signing_secret(
                &state.db,
                state.encryption_keys.as_ref(),
                node_id,
            )
            .await
            {
                Ok(secret) => Some(secret),
                Err(error) => {
                    tracing::warn!(
                        service_id = %service_id,
                        node_id = %node_id,
                        error = %error,
                        "SSH node tunnel signing secret resolution failed"
                    );
                    continue;
                }
            }
        } else {
            None
        };
        match state
            .node_ws_manager
            .open_ssh_tunnel(
                node_id,
                crate::services::node_ws_manager::NodeSshTunnelRequest {
                    session_id: session_id.clone(),
                    service_id: service_id.clone(),
                    host: ssh_service.host.clone(),
                    port: ssh_service.port,
                },
                signing_secret.as_ref().map(|secret| secret.as_slice()),
            )
            .await
        {
            Ok(rx) => {
                tunnel_rx = Some(rx);
                selected_node_id = Some(node_id.to_string());
                break;
            }
            Err(error) => {
                tracing::warn!(service_id = %service_id, node_id = %node_id, error = %error, "SSH node tunnel open failed");
            }
        }
    }

    let Some(mut tunnel_rx) = tunnel_rx else {
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1011,
                reason: "Failed to connect downstream SSH target via node".into(),
            })))
            .await;
        audit_service::log_async(
            state.db.clone(),
            Some(user_id),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "node",
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "node_connect_failed",
            })),
            client_meta.ip_address,
            client_meta.user_agent,
            None,
            None,
        );
        return;
    };
    let Some(node_id) = selected_node_id else {
        tracing::error!(
            service_id = %service_id,
            session_id = %session_id,
            "Node-routed SSH tunnel opened without a selected node id"
        );
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1011,
                reason: "Failed to bind SSH tunnel to node".into(),
            })))
            .await;
        audit_service::log_async(
            state.db.clone(),
            Some(user_id),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "node",
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "missing_selected_node_id",
            })),
            client_meta.ip_address,
            client_meta.user_agent,
            None,
            None,
        );
        return;
    };

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let initial_downstream_bytes = match read_node_ssh_banner(
        &mut tunnel_rx,
        state.config.ssh_connect_timeout_secs,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(service_id = %service_id, node_id = %node_id, error = %error, "Node-routed SSH tunnel failed banner validation");
            close_node_ssh_tunnel(
                &state,
                &service_id,
                &node_id,
                &session_id,
                "banner_validation_failed",
            );
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Downstream target is not a valid SSH server".into(),
                })))
                .await;
            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "node",
                    "node_id": node_id,
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": error.to_string(),
                })),
                client_meta.ip_address,
                client_meta.user_agent,
                None,
                None,
            );
            return;
        }
    };
    if socket
        .send(Message::Binary(initial_downstream_bytes.clone().into()))
        .await
        .is_err()
    {
        close_node_ssh_tunnel(
            &state,
            &service_id,
            &node_id,
            &session_id,
            "banner_send_failed",
        );
        audit_service::log_async(
            state.db.clone(),
            Some(user_id.clone()),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "node",
                "node_id": node_id,
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "banner_send_failed",
            })),
            client_meta.ip_address.clone(),
            client_meta.user_agent.clone(),
            None,
            None,
        );
        return;
    }
    to_client_bytes += initial_downstream_bytes.len() as u64;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_tunnel_connected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "node",
            "node_id": node_id,
            "target_host": ssh_service.host,
            "target_port": ssh_service.port,
        })),
        client_meta.ip_address.clone(),
        client_meta.user_agent.clone(),
        None,
        None,
    );

    // Telemetry: ssh.tunnel_opened (node path).
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        api_key_id.as_deref(),
        &tele,
        TelemetryEvent::SshTunnelOpened {
            service_slug: service_slug.clone(),
            mode: "tunnel".to_string(),
        },
    );

    let tunnel_timeout = tokio::time::sleep(Duration::from_secs(
        state.config.ssh_max_tunnel_duration_secs,
    ));
    tokio::pin!(tunnel_timeout);

    let disconnect_reason = loop {
        tokio::select! {
            _ = &mut tunnel_timeout => {
                let _ = socket
                    .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 1008,
                        reason: "SSH tunnel reached maximum duration".into(),
                    })))
                    .await;
                break Some("max_tunnel_duration_exceeded");
            }
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if state.node_ws_manager.send_ssh_tunnel_data(&node_id, &session_id, &bytes).is_err() {
                            break Some("node_tunnel_send_failed");
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break Some("client_closed");
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break Some("invalid_client_frame");
                    }
                    Some(Err(_)) => break Some("client_socket_error"),
                }
            }
            tunnel_message = tunnel_rx.recv() => {
                match tunnel_message {
                    Some(crate::services::node_ws_manager::SshTunnelChunk::Data(bytes)) => {
                        to_client_bytes += bytes.len() as u64;
                        if socket.send(Message::Binary(bytes.into())).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(error)) => {
                        break if error.is_some() {
                            Some("node_tunnel_closed_with_error")
                        } else {
                            Some("node_tunnel_closed")
                        };
                    }
                    None => break Some("node_tunnel_channel_closed"),
                }
            }
        }
    };

    close_node_ssh_tunnel(
        &state,
        &service_id,
        &node_id,
        &session_id,
        "session_cleanup",
    );
    let _ = socket.close().await;

    let duration_ms = started_at.elapsed().as_millis() as u64;
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_tunnel_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "node",
            "node_id": node_id,
            "duration_ms": duration_ms,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
            "disconnect_reason": disconnect_reason,
        })),
        client_meta.ip_address,
        client_meta.user_agent,
        None,
        None,
    );

    // Telemetry: ssh.tunnel_closed (node path).
    // Best-effort: normal close only, abrupt close may miss -- see
    // TELEMETRY.md §6.5.
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        api_key_id.as_deref(),
        &tele,
        TelemetryEvent::SshTunnelClosed {
            service_slug,
            duration_ms,
        },
    );
}

pub(crate) async fn authorize_ssh_access(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
) -> AppResult<()> {
    let operation = operation_descriptor::build_ssh_descriptor(
        operation_descriptor::SshOperationKind::Tunnel,
        None,
    );
    authorize_ssh_access_for_operation(state, auth_user, service_id, &operation).await
}

pub(crate) async fn authorize_ssh_access_for_operation(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    operation: &operation_descriptor::OperationDescriptor,
) -> AppResult<()> {
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    let service = fetch_service(state, service_id).await?;
    if !service.is_active {
        return Err(AppError::NotFound("SSH service not found".to_string()));
    }
    ssh_service::ensure_ssh_service(&service)?;

    // SSH services may be org-owned. Resolve the effective owner through the
    // same membership-aware path the HTTP proxy uses, so SSH access stays
    // consistent with proxy access. `find_effective_service_owner` applies the
    // same role (`can_proxy`) and per-service scope filters, returning the
    // effective owner for the resource owner or an in-scope, proxy-capable org
    // member, and `None` otherwise. For a private (org- or personally-owned)
    // service a `None` result means the caller has no claim on it, so we return
    // NotFound rather than falling back to treating the caller as the owner.
    // The service creator and public catalog services (visibility != "private")
    // keep their existing access. NotFound (rather than Forbidden) avoids
    // leaking the existence of a private service the caller cannot see.
    let effective_owner = crate::services::proxy_service::find_effective_service_owner(
        &state.db,
        &approval_owner_user_id,
        None,
        Some(service_id),
    )
    .await?;
    let owner_for_resolution = match effective_owner.as_deref() {
        Some(owner) => owner,
        None => {
            // No personal service and no authorized org membership grants
            // access. Reject for a private (owned) service unless the caller
            // is its creator -- a Direct owner who never provisioned a
            // UserService also resolves to `None` here and must not be locked
            // out of their own service. Public catalog services keep their
            // open behaviour.
            if service.visibility == "private" && service.created_by != approval_owner_user_id {
                return Err(AppError::NotFound("SSH service not found".to_string()));
            }
            &approval_owner_user_id
        }
    };
    let approval_outcome = approval_service::evaluate_and_check(
        &state.db,
        &approval_owner_user_id,
        owner_for_resolution,
        service_id,
        operation,
        auth_user.approval_requester_type(),
        &auth_user.approval_requester_id(),
        auth_user.auth_method == AuthMethod::Session,
    )
    .await?;

    let pending = match approval_outcome {
        approval_service::ApprovalOutcome::Allowed { .. } => return Ok(()),
        approval_service::ApprovalOutcome::Denied => {
            return Err(AppError::Forbidden(
                "Operation denied by approval policy".to_string(),
            ));
        }
        approval_service::ApprovalOutcome::NeedsApproval(pending) => pending,
    };

    let notify_user_ids = approval_service::approval_notification_recipients(
        &state.db,
        &approval_owner_user_id,
        &pending,
    )
    .await?;

    let timeout_recipient = notify_user_ids.first().cloned().ok_or_else(|| {
        AppError::Internal("approval recipient list unexpectedly empty".to_string())
    })?;
    let channel =
        notification_service::get_or_create_channel(&state.db, &timeout_recipient).await?;
    let timeout_secs = channel.approval_timeout_secs;
    let approval_service_slug =
        resolve_ssh_approval_service_slug(&state.db, owner_for_resolution, &service).await?;
    let request_operation = approval_service::ApprovalRequestOperation::from_descriptor(
        operation,
        pending.resolution.grant_scope.clone(),
    );
    let approval_request = approval_service::create_approval_request(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &pending.primary_owner_user_id,
        service_id,
        &service.name,
        &approval_service_slug,
        &pending.requester_type,
        &pending.requester_id,
        None,
        request_operation,
        pending.resolution.mode.clone(),
        timeout_secs,
        notify_user_ids,
        pending.resolution.from_org_policy,
    )
    .await?;

    let req_id = approval_request.id.clone();
    approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
        .await
        .map_err(|error| {
            approval_service::map_wait_for_decision_error(
                error,
                &req_id,
                &state.config.frontend_url,
            )
        })?;

    Ok(())
}

async fn resolve_ssh_approval_service_slug(
    db: &mongodb::Database,
    owner_user_id: &str,
    service: &crate::models::downstream_service::DownstreamService,
) -> AppResult<String> {
    let user_service =
        user_service_service::find_by_catalog_service_id(db, owner_user_id, &service.id).await?;
    Ok(ssh_approval_display_slug(
        user_service.as_ref().map(|svc| svc.slug.as_str()),
        &service.slug,
        &service.name,
    ))
}

fn ssh_approval_display_slug(
    user_service_slug: Option<&str>,
    backing_slug: &str,
    service_name: &str,
) -> String {
    if let Some(slug) = user_service_slug {
        return slug.to_string();
    }
    if !backing_slug.starts_with("_ssh_") {
        return backing_slug.to_string();
    }
    service_name.to_string()
}

async fn read_direct_ssh_banner(
    stream: &mut tokio::net::TcpStream,
    timeout_secs: u64,
) -> AppResult<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut buffer = Vec::with_capacity(256);
        let mut chunk = [0_u8; 512];
        loop {
            let read = stream.read(&mut chunk).await.map_err(|error| {
                AppError::BadRequest(format!(
                    "Failed to read SSH banner from downstream: {error}"
                ))
            })?;
            if read == 0 {
                return Err(AppError::BadRequest(
                    "Downstream target closed before sending an SSH banner".to_string(),
                ));
            }
            buffer.extend_from_slice(&chunk[..read]);
            if ssh_banner_validated(&buffer)? {
                return Ok(buffer);
            }
        }
    })
    .await
    .map_err(|_| {
        AppError::BadRequest("Timed out waiting for SSH banner from downstream".to_string())
    })?
}

async fn read_node_ssh_banner(
    tunnel_rx: &mut tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::SshTunnelChunk>,
    timeout_secs: u64,
) -> AppResult<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut buffer = Vec::with_capacity(256);
        loop {
            match tunnel_rx.recv().await {
                Some(crate::services::node_ws_manager::SshTunnelChunk::Data(bytes)) => {
                    buffer.extend_from_slice(&bytes);
                    if ssh_banner_validated(&buffer)? {
                        return Ok(buffer);
                    }
                }
                Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(Some(error))) => {
                    return Err(AppError::NodeOffline(format!(
                        "Node tunnel closed before SSH banner: {error}"
                    )));
                }
                Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(None)) | None => {
                    return Err(AppError::NodeOffline(
                        "Node tunnel closed before SSH banner".to_string(),
                    ));
                }
            }
        }
    })
    .await
    .map_err(|_| AppError::BadRequest("Timed out waiting for SSH banner from node".to_string()))?
}

fn ssh_banner_validated(buffer: &[u8]) -> AppResult<bool> {
    let mut offset = 0;
    while let Some(relative_end) = buffer[offset..].iter().position(|byte| *byte == b'\n') {
        let end = offset + relative_end + 1;
        let line = &buffer[offset..end];
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);

        if line.starts_with(b"SSH-2.0-") || line.starts_with(b"SSH-1.99-") {
            return Ok(true);
        }
        if line.starts_with(b"SSH-") {
            return Err(AppError::BadRequest(
                "Downstream target returned an unsupported SSH banner".to_string(),
            ));
        }

        offset = end;
    }

    if buffer.len() >= MAX_SSH_BANNER_BYTES {
        return Err(AppError::BadRequest(
            "Downstream target did not present an SSH identification banner".to_string(),
        ));
    }

    Ok(false)
}

async fn validate_runtime_ssh_target(
    service_id: &str,
    ssh_service: &crate::models::downstream_service::SshServiceConfig,
) -> AppResult<()> {
    ssh_service::validate_resolved_ssh_target(&ssh_service.host, ssh_service.port)
        .await
        .map_err(|error| {
            tracing::warn!(
                service_id,
                host = %ssh_service.host,
                port = ssh_service.port,
                error = %error,
                "Rejected invalid SSH target during tunnel setup"
            );
            error
        })
}

fn close_node_ssh_tunnel(
    state: &AppState,
    service_id: &str,
    node_id: &str,
    session_id: &str,
    reason: &str,
) {
    if let Err(error) = state.node_ws_manager.close_ssh_tunnel(node_id, session_id) {
        tracing::warn!(
            service_id,
            node_id,
            session_id,
            reason,
            error = %error,
            "Failed to close node-routed SSH tunnel"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SSH_BANNER_BYTES, ssh_approval_display_slug, ssh_banner_validated,
        validate_runtime_ssh_target,
    };
    use crate::models::downstream_service::{DownstreamService, SshServiceConfig};
    use crate::models::ssh_auth_mode::SshAuthMode;

    #[test]
    fn accepts_valid_ssh_banner_after_preamble() {
        let buffer = b"NOTICE banner\r\nSSH-2.0-OpenSSH_9.7\r\n";
        assert!(ssh_banner_validated(buffer).expect("valid banner"));
    }

    #[test]
    fn rejects_unsupported_ssh_version_banner() {
        let error = ssh_banner_validated(b"SSH-1.5-legacy\r\n").expect_err("unsupported banner");
        assert!(error.to_string().contains("unsupported SSH banner"));
    }

    #[test]
    fn rejects_non_ssh_target_when_banner_limit_is_exceeded() {
        let buffer = vec![b'x'; MAX_SSH_BANNER_BYTES];
        let error = ssh_banner_validated(&buffer).expect_err("missing banner");
        assert!(
            error
                .to_string()
                .contains("did not present an SSH identification banner")
        );
    }

    #[test]
    fn ssh_approval_display_slug_prefers_user_service_slug() {
        assert_eq!(
            ssh_approval_display_slug(Some("my-server"), "_ssh_1234", "Shared Label"),
            "my-server"
        );
    }

    #[test]
    fn ssh_approval_display_slug_keeps_legacy_backing_slug_when_human_readable() {
        assert_eq!(
            ssh_approval_display_slug(None, "legacy-ssh-slug", "Shared Label"),
            "legacy-ssh-slug"
        );
    }

    #[test]
    fn ssh_approval_display_slug_falls_back_to_name_for_internal_backing_slug() {
        assert_eq!(
            ssh_approval_display_slug(None, "_ssh_1234", "Shared Label"),
            "Shared Label"
        );
    }

    #[tokio::test]
    async fn allows_private_ip_ssh_target() {
        validate_runtime_ssh_target(
            "svc-1",
            &SshServiceConfig {
                host: "192.168.1.50".to_string(),
                port: 22,
                ssh_auth_mode: SshAuthMode::ProxyOnly,
                certificate_auth_enabled: false,
                certificate_ttl_minutes: 30,
                allowed_principals: Vec::new(),
                ca_private_key_encrypted: None,
                ca_public_key: None,
            },
        )
        .await
        .expect("private IPs should be allowed for SSH targets");
    }

    #[tokio::test]
    async fn rejects_metadata_ssh_target() {
        let error = validate_runtime_ssh_target(
            "svc-1",
            &SshServiceConfig {
                host: "metadata.google.internal".to_string(),
                port: 22,
                ssh_auth_mode: SshAuthMode::ProxyOnly,
                certificate_auth_enabled: false,
                certificate_ttl_minutes: 30,
                allowed_principals: Vec::new(),
                ca_private_key_encrypted: None,
                ca_public_key: None,
            },
        )
        .await
        .expect_err("metadata endpoint should be blocked");

        assert!(error.to_string().contains("cloud metadata endpoint"));
    }

    // ── authorize_ssh_access: owner / membership access ──────────────────
    // All four SSH endpoints route through authorize_ssh_access. For a private
    // service it should grant access to the service creator and authorized
    // members of the owning org, and deny everyone else -- matching the
    // ownership resolution the HTTP proxy already applies. These two tests
    // cover the deny (non-member) and allow (creator) directions.

    fn ssh_service_row(id: &str, created_by: &str, visibility: &str) -> DownstreamService {
        DownstreamService {
            id: id.to_string(),
            name: "Bastion".to_string(),
            slug: format!("ssh-{id}"),
            description: None,
            base_url: "ssh://10.0.0.5:22".to_string(),
            service_type: "ssh".to_string(),
            visibility: visibility.to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: vec![],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: Some(SshServiceConfig {
                host: "10.0.0.5".to_string(),
                port: 22,
                ssh_auth_mode: SshAuthMode::Cert,
                certificate_auth_enabled: true,
                certificate_ttl_minutes: 30,
                allowed_principals: vec!["ubuntu".to_string()],
                ca_private_key_encrypted: None,
                ca_public_key: None,
            }),
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: created_by.to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: String::new(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: vec![],
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn authorize_ssh_access_denies_non_member_of_private_org_service() {
        use crate::errors::AppError;
        use crate::models::downstream_service::COLLECTION_NAME as DOWNSTREAM_SERVICES;
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};

        let Some(db) = connect_test_database("ssh_authz_nonmember").await else {
            eprintln!("Skipping MongoDB-backed test; no test database available");
            return;
        };

        let org_id = uuid::Uuid::new_v4().to_string();
        let outsider_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org");
        db.collection::<User>(USERS)
            .insert_one(test_user(&outsider_id, UserType::Person))
            .await
            .expect("insert outsider");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(ssh_service_row(&service_id, &org_id, "private"))
            .await
            .expect("insert org-owned private ssh service");

        let state = test_app_state(db);
        let err = super::authorize_ssh_access(&state, &test_auth_user(&outsider_id), &service_id)
            .await
            .expect_err("a non-member must be denied a private org SSH service");
        assert!(
            matches!(err, AppError::NotFound(_)),
            "expected NotFound (no existence leak), got {err:?}"
        );
    }

    #[tokio::test]
    async fn authorize_ssh_access_allows_service_creator() {
        use crate::models::downstream_service::COLLECTION_NAME as DOWNSTREAM_SERVICES;
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};

        let Some(db) = connect_test_database("ssh_authz_creator").await else {
            eprintln!("Skipping MongoDB-backed test; no test database available");
            return;
        };

        let creator_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_one(test_user(&creator_id, UserType::Person))
            .await
            .expect("insert creator");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(ssh_service_row(&service_id, &creator_id, "private"))
            .await
            .expect("insert creator-owned private ssh service");

        let state = test_app_state(db);
        super::authorize_ssh_access(&state, &test_auth_user(&creator_id), &service_id)
            .await
            .expect("the service creator must retain access to their own private SSH service");
    }

    // ── ssh_banner_validated: additional edge cases ──────────────────────

    #[test]
    fn accepts_ssh_2_0_banner_without_preamble() {
        let buffer = b"SSH-2.0-OpenSSH_9.7\r\n";
        assert!(ssh_banner_validated(buffer).expect("valid banner"));
    }

    #[test]
    fn accepts_ssh_1_99_compatibility_banner() {
        let buffer = b"SSH-1.99-OpenSSH_9.7\r\n";
        assert!(ssh_banner_validated(buffer).expect("valid 1.99 banner"));
    }

    #[test]
    fn accepts_banner_with_lf_only_line_endings() {
        let buffer = b"SSH-2.0-dropbear\n";
        assert!(ssh_banner_validated(buffer).expect("valid LF-only banner"));
    }

    #[test]
    fn returns_false_when_buffer_has_no_complete_line() {
        // No newline yet -- banner validation should return Ok(false) to request
        // more data.
        let buffer = b"SSH-2.0-OpenSSH";
        assert!(!ssh_banner_validated(buffer).expect("incomplete line"));
    }

    #[test]
    fn returns_false_for_empty_buffer() {
        assert!(!ssh_banner_validated(b"").expect("empty buffer"));
    }

    #[test]
    fn returns_false_for_preamble_only_no_banner_yet() {
        // Pre-banner lines that are not SSH identification strings. The spec
        // allows servers to send informational lines before the banner.
        let buffer = b"NOTICE: please accept terms\r\n";
        assert!(!ssh_banner_validated(buffer).expect("preamble without banner"));
    }

    #[test]
    fn rejects_ssh_1_0_banner() {
        let error = ssh_banner_validated(b"SSH-1.0-ancient\r\n")
            .expect_err("SSH-1.0 should be unsupported");
        assert!(error.to_string().contains("unsupported SSH banner"));
    }

    #[test]
    fn rejects_ssh_3_0_banner() {
        let error =
            ssh_banner_validated(b"SSH-3.0-future\r\n").expect_err("SSH-3.0 should be unsupported");
        assert!(error.to_string().contains("unsupported SSH banner"));
    }

    #[test]
    fn accepts_banner_after_multiple_preamble_lines() {
        let buffer = b"NOTICE: line1\r\nNOTICE: line2\r\nSSH-2.0-OpenSSH_9.7\r\n";
        assert!(ssh_banner_validated(buffer).expect("banner after multi-line preamble"));
    }

    #[test]
    fn rejects_when_preamble_exactly_at_max_bytes_with_no_banner() {
        // Fill the buffer to exactly MAX_SSH_BANNER_BYTES with non-banner lines.
        let line = b"AAAA\n";
        let count = MAX_SSH_BANNER_BYTES / line.len();
        let mut buffer = Vec::with_capacity(MAX_SSH_BANNER_BYTES);
        for _ in 0..count {
            buffer.extend_from_slice(line);
        }
        // Pad remainder with newline-terminated content to reach the limit.
        while buffer.len() < MAX_SSH_BANNER_BYTES {
            buffer.push(b'X');
        }
        let error = ssh_banner_validated(&buffer).expect_err("should exceed limit");
        assert!(
            error
                .to_string()
                .contains("did not present an SSH identification banner")
        );
    }

    #[test]
    fn banner_validated_just_under_limit_returns_false() {
        // Buffer filled with non-newline bytes just under the limit should
        // return Ok(false) because the banner might still arrive.
        let buffer = vec![b'x'; MAX_SSH_BANNER_BYTES - 1];
        assert!(!ssh_banner_validated(&buffer).expect("under limit, no newline"));
    }

    // ── ssh_approval_display_slug: additional edge cases ────────────────

    #[test]
    fn ssh_approval_display_slug_returns_empty_user_slug_as_is() {
        // An empty user-service slug is still preferred -- the function does not
        // second-guess what the user configured.
        assert_eq!(
            ssh_approval_display_slug(Some(""), "_ssh_abc", "Service Name"),
            ""
        );
    }

    #[test]
    fn ssh_approval_display_slug_with_underscore_ssh_prefix_variations() {
        // Only the exact `_ssh_` prefix triggers the name fallback.
        assert_eq!(
            ssh_approval_display_slug(None, "_sshx_1234", "Name"),
            "_sshx_1234"
        );
        assert_eq!(
            ssh_approval_display_slug(None, "_ssh_", "My Server"),
            "My Server"
        );
    }

    #[test]
    fn ssh_approval_display_slug_falls_back_to_empty_service_name() {
        assert_eq!(ssh_approval_display_slug(None, "_ssh_foo", ""), "");
    }

    // ── Struct serialization / deserialization ───────────────────────────

    #[test]
    fn issue_ssh_certificate_request_deserialization() {
        let json = r#"{"public_key":"ssh-ed25519 AAAA...","principal":"deploy"}"#;
        let req: super::IssueSshCertificateRequest =
            serde_json::from_str(json).expect("deserialize request");
        assert_eq!(req.public_key, "ssh-ed25519 AAAA...");
        assert_eq!(req.principal, "deploy");
    }

    #[test]
    fn issue_ssh_certificate_response_serialization() {
        let resp = super::IssueSshCertificateResponse {
            service_id: "svc-1".to_string(),
            key_id: "key-1".to_string(),
            principal: "deploy".to_string(),
            certificate: "cert-data".to_string(),
            ca_public_key: "ca-pub".to_string(),
            valid_after: "2024-01-01T00:00:00Z".to_string(),
            valid_before: "2024-01-01T01:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("serialize response");
        assert_eq!(json["service_id"], "svc-1");
        assert_eq!(json["key_id"], "key-1");
        assert_eq!(json["principal"], "deploy");
        assert_eq!(json["certificate"], "cert-data");
        assert_eq!(json["ca_public_key"], "ca-pub");
        assert_eq!(json["valid_after"], "2024-01-01T00:00:00Z");
        assert_eq!(json["valid_before"], "2024-01-01T01:00:00Z");
    }

    #[test]
    fn max_ssh_banner_bytes_is_4kb() {
        assert_eq!(MAX_SSH_BANNER_BYTES, 4096);
    }
}
