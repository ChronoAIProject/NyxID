use std::time::Instant;

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
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    approval_service, audit_service, node_routing_service, notification_service, ssh_service,
};

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
    Path(service_id): Path<String>,
    Json(body): Json<IssueSshCertificateRequest>,
) -> AppResult<Json<IssueSshCertificateResponse>> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    let user_id = auth_user.user_id.to_string();
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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_certificate_issued".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "key_id": issued.key_id,
            "principal": issued.principal,
            "routed_via": "ssh",
            "valid_after": issued.valid_after,
            "valid_before": issued.valid_before,
        })),
        None,
        None,
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

    Ok(ws
        .on_upgrade(move |socket| async move {
            handle_ssh_socket(
                state,
                auth_user,
                service_id,
                ssh_service,
                socket,
                session_guard,
                client_meta,
            )
            .await;
        })
        .into_response())
}

async fn handle_ssh_socket(
    state: AppState,
    auth_user: AuthUser,
    service_id: String,
    ssh_service: crate::models::downstream_service::SshServiceConfig,
    mut socket: WebSocket,
    _session_guard: ssh_service::SshSessionGuard,
    client_meta: TunnelClientMeta,
) {
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
            ssh_service,
            socket,
            user_id,
            session_id,
            started_at,
            client_meta,
            node_route,
        )
        .await;
        return;
    }

    let connect_target = format!("{}:{}", ssh_service.host, ssh_service.port);
    let mut tcp_stream = match tokio::time::timeout(
        std::time::Duration::from_secs(state.config.ssh_connect_timeout_secs),
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
            );
            return;
        }
    };

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
    );

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let mut read_buf = vec![0_u8; 16 * 1024];

    loop {
        tokio::select! {
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if tcp_stream.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break;
                    }
                    Some(Err(_)) => break,
                }
            }
            tcp_read = tcp_stream.read(&mut read_buf) => {
                match tcp_read {
                    Ok(0) => break,
                    Ok(n) => {
                        to_client_bytes += n as u64;
                        if socket.send(Message::Binary(read_buf[..n].to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    let _ = socket.close().await;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_tunnel_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "ssh",
            "duration_ms": started_at.elapsed().as_millis() as u64,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
        })),
        client_meta.ip_address,
        client_meta.user_agent,
    );
}

#[allow(clippy::too_many_arguments)]
async fn handle_node_ssh_socket(
    state: AppState,
    service_id: String,
    ssh_service: crate::models::downstream_service::SshServiceConfig,
    mut socket: WebSocket,
    user_id: String,
    session_id: String,
    started_at: Instant,
    client_meta: TunnelClientMeta,
    node_route: crate::services::node_routing_service::NodeRoute,
) {
    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    let mut tunnel_rx = None;
    let mut selected_node_id = None;

    for node_id in all_node_ids {
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
        );
        return;
    };
    let node_id = selected_node_id.expect("selected node id");

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
    );

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;

    loop {
        tokio::select! {
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if state.node_ws_manager.send_ssh_tunnel_data(&node_id, &session_id, &bytes).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break;
                    }
                    Some(Err(_)) => break,
                }
            }
            tunnel_message = tunnel_rx.recv() => {
                match tunnel_message {
                    Some(crate::services::node_ws_manager::SshTunnelChunk::Data(bytes)) => {
                        to_client_bytes += bytes.len() as u64;
                        if socket.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(_)) | None => break,
                }
            }
        }
    }

    let _ = state
        .node_ws_manager
        .close_ssh_tunnel(&node_id, &session_id);
    let _ = socket.close().await;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_tunnel_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "node",
            "node_id": node_id,
            "duration_ms": started_at.elapsed().as_millis() as u64,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
        })),
        client_meta.ip_address,
        client_meta.user_agent,
    );
}

async fn authorize_ssh_access(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
) -> AppResult<()> {
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    let service = fetch_service(state, service_id).await?;
    ssh_service::ensure_ssh_service(&service)?;

    let requires_approval = approval_service::requires_approval_for_service(
        &state.db,
        &approval_owner_user_id,
        service_id,
    )
    .await?;

    if requires_approval && auth_user.auth_method != AuthMethod::Session {
        let requester_type = auth_user.approval_requester_type().ok_or_else(|| {
            AppError::Forbidden("Session auth does not require approval".to_string())
        })?;
        let has_grant = approval_service::check_approval(
            &state.db,
            &approval_owner_user_id,
            service_id,
            requester_type,
            &auth_user.approval_requester_id(),
        )
        .await?;

        if !has_grant {
            let channel =
                notification_service::get_or_create_channel(&state.db, &approval_owner_user_id)
                    .await?;
            let timeout_secs = channel.approval_timeout_secs;
            let approval_request = approval_service::create_approval_request(
                &state.db,
                &state.config,
                &state.http_client,
                state.fcm_auth.as_deref(),
                state.apns_auth.as_deref(),
                &approval_owner_user_id,
                service_id,
                &service.name,
                &service.slug,
                requester_type,
                &auth_user.approval_requester_id(),
                None,
                "ssh:tunnel",
                timeout_secs,
            )
            .await?;

            approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
                .await?;
        }
    }

    Ok(())
}
