//! Human-only live relay. Only control metadata enters MongoDB or audit.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{machine_desktop::MachineDesktop, node::Node},
    mw::auth::AuthUser,
    services::{
        assistant_nyxagent as engine, audit_service, machine_desktop_service as desktop,
        node_service, org_service,
    },
};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use chrono::Utc;
use futures::StreamExt;
use nyxid_machine::{
    Operation, Request,
    binary::{Frame, Kind},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
pub struct DesktopQuery {
    pub conversation_id: Option<String>,
}

#[derive(Serialize)]
pub struct Metadata {
    node_id: String,
    session_id: String,
    conversation_id: Option<String>,
    status: String,
    reason: Option<String>,
}

impl From<MachineDesktop> for Metadata {
    fn from(row: MachineDesktop) -> Self {
        Self {
            node_id: row.node_id,
            session_id: row.session_id,
            conversation_id: row.conversation_id,
            status: row.status,
            reason: row.reason,
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<DesktopQuery>,
) -> AppResult<Json<Vec<Metadata>>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    if let Some(id) = &query.conversation_id {
        engine::get(&state.db, &owner, id).await?;
    }
    Ok(Json(
        desktop::list(&state.db, &owner, query.conversation_id.as_deref())
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

async fn authorized(state: &AppState, owner: &str, node: &str) -> AppResult<Node> {
    let node = node_service::get_node_by_id(&state.db, node)
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Machine not found".into()))?;
    let access = org_service::resolve_owner_access(&state.db, owner, &node.user_id).await?;
    if !access.can_write() {
        return Err(AppError::MachineNotAllowed);
    }
    crate::services::machine_service::capable(&node, Operation::DesktopOpen)?;
    Ok(node)
}

pub async fn upgrade(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(node): Path<String>,
    Query(query): Query<DesktopQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    super::login_client_context::require_first_party_human(&auth)?;
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Forbidden("Desktop requires a browser origin".into()))?;
    let configured = url::Url::parse(&state.config.frontend_url)
        .map_err(|_| AppError::Internal("Frontend origin unavailable".into()))?;
    if origin != configured.origin().ascii_serialization() {
        return Err(AppError::Forbidden("Desktop origin refused".into()));
    }
    let owner = auth.user_id.to_string();
    let node = authorized(&state, &owner, &node).await?;
    if let Some(id) = &query.conversation_id {
        engine::get(&state.db, &owner, id).await?;
    }
    let row = desktop::open(
        &state.db,
        &owner,
        &node.id,
        query.conversation_id.as_deref(),
    )
    .await?;
    let secret =
        node_service::get_node_signing_secret(&state.db, &state.encryption_keys, &node.id).await?;
    Ok(ws
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| {
            relay(
                state,
                node,
                row,
                zeroize::Zeroizing::new(secret.to_vec()),
                socket,
            )
        }))
}

fn signed(node: &str, operation: Operation, parameters: Value, secret: &[u8]) -> Request {
    let mut request = Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        node_id: node.into(),
        operation,
        parameters,
        timestamp: Utc::now().timestamp(),
        nonce: uuid::Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    request.signature = nyxid_machine::signing::sign(&request, secret);
    request
}

async fn command(
    state: &AppState,
    node: &str,
    operation: Operation,
    args: Value,
    secret: &[u8],
) -> AppResult<()> {
    let result = state
        .node_dispatch
        .machine_request(signed(node, operation, args, secret))
        .await?;
    if result.result.get("error").is_some() {
        return Err(AppError::MachineBrowserUnavailable);
    }
    Ok(())
}

async fn send_json(socket: &mut WebSocket, value: Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}

async fn relay(
    state: AppState,
    node: Node,
    mut row: MachineDesktop,
    secret: zeroize::Zeroizing<Vec<u8>>,
    mut socket: WebSocket,
) {
    let viewer = uuid::Uuid::new_v4().to_string();
    let Ok(mut frames) = state
        .node_dispatch
        .open_machine_desktop(&node.id, &row.session_id, &viewer)
        .await
    else {
        return;
    };
    if command(
        &state,
        &node.id,
        Operation::DesktopOpen,
        json!({"session_id":row.session_id,"refresh_frame":true}),
        &secret,
    )
    .await
    .is_err()
    {
        return;
    }
    audit(&state, &row, "session_start");
    if !send_json(
        &mut socket,
        json!({
            "type":"connected",
            "viewer_id":viewer,
            "session_id":row.session_id,
            "controller":row.status,
            "reason":row.reason
        }),
    )
    .await
    {
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut controls = false;
    let mut last_input = 0u64;
    let mut last_refresh = Instant::now() - Duration::from_secs(1);
    let mut quota = (Instant::now(), 0usize);
    loop {
        tokio::select! {
            frame = frames.recv() => match frame {
                Some(bytes) => {
                    let sent = tokio::time::timeout(
                        Duration::from_secs(3),
                        socket.send(Message::Binary(bytes.as_ref().clone().into())),
                    ).await;
                    if !sent.is_ok_and(|result| result.is_ok()) {
                        break;
                    }
                }
                None => break,
            },
            _ = heartbeat.tick() => {
                if authorized(&state, &row.user_id, &node.id).await.is_err() {
                    break;
                }
                let Ok(Some(current)) = desktop::get(&state.db, &node.id).await else {
                    break;
                };
                if current.session_id != row.session_id || current.user_id != row.user_id {
                    break;
                }
                controls = current.status == "owner" && current.controller.as_deref() == Some(viewer.as_str());
                row = current;
                if desktop::touch(&state.db, &row).await.is_err() {
                    break;
                }
                if controls && desktop::refresh(&state.db, &row, &viewer).await.is_err() {
                    break;
                }
                if command(
                    &state,
                    &node.id,
                    Operation::DesktopOpen,
                    json!({"session_id":row.session_id}),
                    &secret,
                )
                .await
                .is_err()
                {
                    break;
                }
                if !send_json(
                    &mut socket,
                    json!({
                        "type":"state",
                        "controller":row.status,
                        "controls":controls,
                        "reason":row.reason
                    }),
                )
                .await
                {
                    break;
                }
            },
            incoming = socket.next() => {
                let Some(Ok(message)) = incoming else {
                    break;
                };
                match message {
                    Message::Binary(bytes) => {
                        if !controls {
                            continue;
                        }
                        let Ok(frame) = Frame::decode(&bytes) else {
                            break;
                        };
                        if frame.kind != Kind::Input
                            || frame.id.to_string() != row.session_id
                            || frame.sequence <= last_input
                        {
                            break;
                        }
                        if quota.0.elapsed() >= Duration::from_secs(1) {
                            quota = (Instant::now(), 0);
                        }
                        quota.1 += 1;
                        if quota.1 > 90 {
                            continue;
                        }
                        last_input = frame.sequence;
                        let Ok(mut parameters) = serde_json::from_slice::<Value>(frame.bytes) else {
                            break;
                        };
                        if !parameters.is_object() {
                            break;
                        }
                        parameters["session_id"] = json!(row.session_id);
                        parameters["viewer_id"] = json!(viewer);
                        parameters["revision"] = json!(row.revision);
                        let request = signed(&node.id, Operation::DesktopInput, parameters, &secret);
                        let Ok(payload) = serde_json::to_vec(&request) else {
                            break;
                        };
                        let Ok(frame) = (Frame {
                            kind: Kind::Input,
                            end: false,
                            id: frame.id,
                            sequence: frame.sequence,
                            bytes: &payload,
                        })
                        .encode() else {
                            break;
                        };
                        if state
                            .node_dispatch
                            .machine_desktop_input(&node.id, &viewer, frame)
                            .is_err()
                        {
                            break;
                        }
                    }
                    Message::Text(text) => {
                        let Ok(input) = serde_json::from_str::<Value>(&text) else {
                            break;
                        };
                        let result: AppResult<()> = async {
                            match input["type"].as_str() {
                                Some("refresh_frame") => {
                                    if last_refresh.elapsed() >= Duration::from_secs(1) {
                                        command(
                                            &state,
                                            &node.id,
                                            Operation::DesktopOpen,
                                            json!({
                                                "session_id": row.session_id,
                                                "refresh_frame": true,
                                            }),
                                            &secret,
                                        )
                                        .await?;
                                        last_refresh = Instant::now();
                                    }
                                }
                                Some("take_control") => {
                                    row = desktop::take(&state.db, &row, &viewer).await?;
                                    if let Some(id) = &row.conversation_id {
                                        engine::request_stop(&state.db, &row.user_id, id).await?;
                                    }
                                    command(
                                        &state,
                                        &node.id,
                                        Operation::DesktopControl,
                                        json!({
                                            "session_id":row.session_id,
                                            "owner":true,
                                            "viewer_id":viewer,
                                            "revision":row.revision
                                        }),
                                        &secret,
                                    )
                                    .await?;
                                    row = desktop::controlled(&state.db, &row, &viewer).await?;
                                    watch(&state, &row).await?;
                                    controls = true;
                                    audit(&state, &row, "owner_control");
                                }
                                Some("hand_back") if controls => {
                                    row = desktop::release(
                                        &state.db,
                                        &row,
                                        &viewer,
                                        input["note"].as_str().unwrap_or_default(),
                                    )
                                    .await?;
                                    command(
                                        &state,
                                        &node.id,
                                        Operation::DesktopControl,
                                        json!({
                                            "session_id":row.session_id,
                                            "owner":false,
                                            "viewer_id":viewer,
                                            "revision":row.revision
                                        }),
                                        &secret,
                                    )
                                    .await?;
                                    row = desktop::returned(&state.db, &row).await?;
                                    controls = false;
                                    audit(&state, &row, "agent_control");
                                    super::nyxbot::process_watches(&state).await?;
                                }
                                Some("stop") => {
                                    if let Some(id) = &row.conversation_id {
                                        engine::request_stop(&state.db, &row.user_id, id).await?;
                                    }
                                }
                                _ => return Err(AppError::MachineNotAllowed),
                            }
                            Ok(())
                        }
                        .await;
                        let value = if result.is_ok() {
                            json!({
                                "type":"state",
                                "controller":row.status,
                                "controls":controls,
                                "reason":row.reason
                            })
                        } else {
                            json!({
                                "type":"error",
                                "message":"The control change could not finish. Refresh the panel and try again; the agent remains paused during an incomplete takeover."
                            })
                        };
                        if !send_json(&mut socket, value).await {
                            break;
                        }
                    }
                    Message::Ping(bytes) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => break,
                }
            }
        }
    }
    audit(&state, &row, "session_end");
}

fn audit(state: &AppState, row: &MachineDesktop, outcome: &str) {
    audit_service::log_async(
        state.db.clone(),
        Some(row.user_id.clone()),
        "machine_desktop".into(),
        Some(json!({
            "node_id":row.node_id,
            "session_id":row.session_id,
            "conversation_id":row.conversation_id,
            "outcome":outcome,
            "reason":row.reason
        })),
        None,
        None,
        None,
        None,
    );
}

pub async fn watch(state: &AppState, row: &MachineDesktop) -> AppResult<()> {
    use mongodb::bson::{self, doc};
    if let Some(conversation) = &row.conversation_id {
        state.db.collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME).update_one(
            doc!{
                "kind":"machine_control",
                "connect_link_id":&row.node_id,
                "conversation_id":conversation,
                "status":"pending"
            },
            doc!{
                "$setOnInsert":{
                    "_id":uuid::Uuid::new_v4().to_string(),
                    "user_id":&row.user_id,
                    "kind":"machine_control",
                    "connect_link_id":&row.node_id,
                    "conversation_id":conversation,
                    "status":"pending",
                    "created_at":bson::DateTime::now(),
                    "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::days(1))
                }
            }
        ).upsert(true).await?;
    }
    Ok(())
}

pub async fn request_control(
    state: &AppState,
    chat: &crate::services::assistant_acknowledgement_service::ChatAuthority,
    node: &Node,
    reason: &str,
) -> AppResult<Value> {
    let row = desktop::open(
        &state.db,
        &chat.user_id,
        &node.id,
        Some(&chat.conversation_id),
    )
    .await?;
    let row = desktop::request(&state.db, &row, reason).await?;
    let secret =
        node_service::get_node_signing_secret(&state.db, &state.encryption_keys, &node.id).await?;
    command(
        state,
        &node.id,
        Operation::DesktopOpen,
        json!({"session_id":row.session_id}),
        &secret,
    )
    .await?;
    // Pause running commands immediately, including when the owner opens the
    // notification later. Waiting never leaves background observers running.
    command(
        state,
        &node.id,
        Operation::DesktopControl,
        json!({
            "session_id":row.session_id,
            "owner":true,
            "viewer_id":"waiting-for-owner",
            "revision":row.revision
        }),
        &secret,
    )
    .await?;
    watch(state, &row).await?;
    let link = format!(
        "{}/machines/{}/desktop?conversation_id={}",
        state.config.frontend_url.trim_end_matches('/'),
        node.id,
        chat.conversation_id
    );
    let message = format!(
        "NyxBot needs you on {}: {reason}\nTake control: {link}",
        node.name
    );
    let conversation = engine::get(&state.db, &chat.user_id, &chat.conversation_id).await?;
    super::nyxbot::deliver_update(state, &conversation, &message).await;
    let _ = crate::services::notification_service::machine_control_requested(
        state,
        &chat.user_id,
        &node.name,
        &link,
    )
    .await;
    engine::request_stop(&state.db, &chat.user_id, &chat.conversation_id).await?;
    audit(state, &row, "control_requested");
    Ok(json!({
        "waiting_for_owner":true,
        "url":link,
        "message":"The owner has been notified. End this turn. NyxID wakes this conversation on hand-back with the owner's note."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Extension, Router, routing::get};
    use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

    async fn endpoint(
        state: State<AppState>,
        Extension(auth): Extension<AuthUser>,
        path: Path<String>,
        query: Query<DesktopQuery>,
        headers: HeaderMap,
        ws: WebSocketUpgrade,
    ) -> AppResult<Response> {
        upgrade(state, auth, path, query, headers, ws).await
    }

    #[tokio::test]
    async fn machine_desktop_upgrade_requires_owner_human_and_same_origin() {
        let f = crate::services::assistant_authority_tests::orchestrator_fixture(
            "machine_desktop_browser_authority",
        )
        .await;
        let node = crate::services::machine_integration_tests::node(&f, &f.owner).await;
        let origin = url::Url::parse(&f.state.config.frontend_url)
            .unwrap()
            .origin()
            .ascii_serialization();
        let mut developer = crate::test_utils::test_auth_user(&f.owner);
        developer.auth_method = crate::mw::auth::AuthMethod::AccessToken;
        developer.oauth_client_id = Some("developer-app".into());
        let mut first_party = crate::test_utils::test_auth_user(&f.owner);
        first_party.auth_method = crate::mw::auth::AuthMethod::AccessToken;
        for (auth, request_origin, expected) in [
            (
                crate::test_utils::test_auth_user(&f.owner),
                origin.clone(),
                101,
            ),
            (
                crate::test_utils::test_auth_user(&f.owner),
                "https://other.example".into(),
                403,
            ),
            (
                crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string()),
                origin.clone(),
                403,
            ),
            (f.auth.clone(), origin.clone(), 403),
            (developer, origin.clone(), 403),
            (first_party, origin, 101),
        ] {
            let app = Router::new()
                .route(
                    "/api/v1/assistant/nyxagent/machines/{node}/desktop",
                    get(endpoint),
                )
                .layer(axum::middleware::from_fn(
                    crate::mw::auth::reject_oauth_client_tokens,
                ))
                .layer(Extension(auth))
                .with_state(f.state.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!(
                "ws://{}/api/v1/assistant/nyxagent/machines/{}/desktop",
                listener.local_addr().unwrap(),
                node.id
            );
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut request = url.into_client_request().unwrap();
            request
                .headers_mut()
                .insert("origin", request_origin.parse().unwrap());
            let status = match connect_async(request).await {
                Ok((mut socket, response)) => {
                    let _ = socket.close(None).await;
                    response.status().as_u16()
                }
                Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                    response.status().as_u16()
                }
                Err(error) => panic!("unexpected desktop handshake failure: {error}"),
            };
            assert_eq!(status, expected);
            server.abort();
        }
        f.state.db.drop().await.unwrap();
    }
}
