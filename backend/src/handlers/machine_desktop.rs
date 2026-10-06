//! Human-only live relay. Only control metadata enters MongoDB or audit.
use crate::services::assistant_links::AssistantPage;
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
use futures::{StreamExt, TryStreamExt};
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
    pub context_id: Option<String>,
    #[serde(default)]
    pub display: nyxid_machine::desktop::Display,
}

#[derive(Serialize)]
pub struct Metadata {
    node_id: String,
    session_id: String,
    conversation_id: Option<String>,
    status: String,
    reason: Option<String>,
    display: nyxid_machine::desktop::Display,
    context_id: Option<String>,
}

#[derive(Serialize)]
pub struct ContextMetadata {
    context_id: String,
    agent_id: String,
    agent_name: String,
    actor_label: String,
    group_id: Option<String>,
    generation: i64,
}

pub async fn contexts(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(node): Path<String>,
) -> AppResult<Json<Vec<ContextMetadata>>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let viewer = auth.user_id.to_string();
    let node = authorized(&state, &viewer, &node).await?;
    if !desktop::supports_context_desktop(&node) {
        return Ok(Json(Vec::new()));
    }
    let owner = node.user_id.clone();
    let mut filter = mongodb::bson::doc! {
        "node_id": &node.id,
        "owner_id": &owner,
        "mode": "separated"
    };
    // On an organization machine a member may see only their own contexts.
    // The personal machine owner retains the physical authority they already
    // have over every browser on that machine.
    if owner != viewer {
        filter.insert("actor_id", &viewer);
    }
    let mut rows = state
        .db
        .collection::<crate::models::machine_access::Context>(
            crate::models::machine_access::CONTEXTS,
        )
        .find(filter)
        .sort(mongodb::bson::doc! {"agent_id": 1, "actor_id": 1, "group_id": 1})
        .limit(128)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let agent_ids = rows
        .iter()
        .map(|row| row.agent_id.clone())
        .collect::<Vec<_>>();
    let agents = if agent_ids.is_empty() {
        Vec::new()
    } else {
        state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(
                crate::models::assistant_agent::COLLECTION_NAME,
            )
            .find(mongodb::bson::doc! {"_id": {"$in": &agent_ids}})
            .limit(128)
            .await?
            .try_collect::<Vec<_>>()
            .await?
    };
    let names = agents
        .into_iter()
        .map(|agent| (agent.id, agent.display_name.unwrap_or(agent.name)))
        .collect::<std::collections::HashMap<_, _>>();
    Ok(Json(
        rows.drain(..)
            .map(|row| ContextMetadata {
                agent_name: names
                    .get(&row.agent_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown agent".into()),
                context_id: row.id,
                agent_id: row.agent_id,
                actor_label: if row.actor_id == owner {
                    "Owner session".into()
                } else {
                    "Group member session".into()
                },
                group_id: row.group_id,
                generation: row.generation,
            })
            .collect(),
    ))
}

impl From<MachineDesktop> for Metadata {
    fn from(row: MachineDesktop) -> Self {
        Self {
            node_id: row.node_id,
            session_id: row.session_id,
            conversation_id: row.conversation_id,
            status: row.status,
            reason: row.reason,
            display: row.display,
            context_id: row.context_id,
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
    let mut rows = desktop::list(&state.db, &owner, query.conversation_id.as_deref()).await?;
    if let Some(context_id) = &query.context_id {
        // A context selector is an authorization-bearing input even though
        // this endpoint normally lists the current conversation's desktops.
        // Resolve the durable context and node before returning a row; a
        // forged id must never turn into a broad desktop listing.
        let context = state
            .db
            .collection::<crate::models::machine_access::Context>(
                crate::models::machine_access::CONTEXTS,
            )
            .find_one(mongodb::bson::doc! {
                "_id": context_id,
                "mode": "separated"
            })
            .await?
            .ok_or(AppError::MachineNotAllowed)?;
        let node = authorized(&state, &owner, &context.node_id).await?;
        authorized_context(&state, &node, &owner, Some(context_id)).await?;
        rows.retain(|row| row.context_id.as_deref() == Some(context_id.as_str()));
    } else {
        rows = desktop::visible_context_rows(&state.db, &owner, rows).await?;
    }
    Ok(Json(rows.into_iter().map(Into::into).collect()))
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

/// Validate a context against the durable row before any desktop operation.
/// Organization writers may use their own context only; the personal machine
/// owner keeps the physical authority over every context on their machine.
async fn authorized_context(
    state: &AppState,
    node: &Node,
    viewer: &str,
    context_id: Option<&str>,
) -> AppResult<()> {
    desktop::authorize_context(&state.db, node, viewer, context_id).await
}

async fn authorized_message(
    state: &AppState,
    node: &Node,
    viewer: &str,
    row: &MachineDesktop,
    input: &Value,
) -> AppResult<()> {
    if input
        .get("context_id")
        .is_some_and(|id| *id != json!(row.context_id))
    {
        return Err(AppError::MachineNotAllowed);
    }
    if row.context_id.is_some() {
        // Recheck live machine ownership/membership as well as the stored
        // context on each control/input message; a connected socket is no grant.
        let current = authorized(state, viewer, &node.id).await?;
        authorized_context(state, &current, viewer, row.context_id.as_deref()).await?;
    }
    Ok(())
}

async fn stop(state: &AppState, row: &MachineDesktop) -> AppResult<()> {
    if let Some(id) = &row.conversation_id {
        super::machine_cancel::conversation(state, &row.user_id, id).await
    } else if row.context_id.is_none() {
        super::machine_cancel::machine(state, &row.node_id).await
    } else {
        // A standalone context viewer has no conversation fence. In particular
        // it must never fall through to stopping other people's machine work.
        Err(AppError::ValidationError(
            "No agent turn is linked to this desktop. Take control to pause work in this context."
                .into(),
        ))
    }
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
    authorized_context(&state, &node, &owner, query.context_id.as_deref()).await?;
    if query.display == nyxid_machine::desktop::Display::Dev
        && !node.machine.as_ref().is_some_and(|m| m.browser_tools)
    {
        return Err(AppError::ValidationError(
            "Update this machine before opening the developer browser display".into(),
        ));
    }
    if let Some(id) = &query.conversation_id {
        engine::get(&state.db, &owner, id).await?;
    }
    let row = desktop::open_display_for_context(
        &state.db,
        &owner,
        &node.id,
        query.conversation_id.as_deref(),
        query.context_id.as_deref(),
        query.display,
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
                owner,
                zeroize::Zeroizing::new(secret.to_vec()),
                socket,
            )
        }))
}

fn signed(node: &str, operation: Operation, parameters: Value, secret: &[u8]) -> Request {
    let mut request = Request {
        version: 1,
        authority: None,
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
    requester: String,
    secret: zeroize::Zeroizing<Vec<u8>>,
    mut socket: WebSocket,
) {
    let viewer = uuid::Uuid::new_v4().to_string();
    if authorized_context(&state, &node, &requester, row.context_id.as_deref())
        .await
        .is_err()
    {
        return;
    }
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
        json!({"display":row.display,"session_id":row.session_id,"context_id":row.context_id,"refresh_frame":true}),
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
            "name":node.name,
            "viewer_id":viewer,
            "display":row.display,"session_id":row.session_id,
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
                let Ok(current_node) = authorized(&state, &requester, &node.id).await else {
                    break;
                };
                if authorized_context(&state, &current_node, &requester, row.context_id.as_deref())
                    .await
                    .is_err() {
                    break;
                }
                let Ok(Some(current)) = desktop::get(&state.db, &row.id).await else {
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
                    json!({"display":row.display,"session_id":row.session_id,"context_id":row.context_id}),
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
                        if authorized_message(&state, &node, &requester, &row, &parameters)
                            .await
                            .is_err()
                        {
                            break;
                        }
                        parameters["display"] = json!(row.display);
                        parameters["session_id"] = json!(row.session_id);
                        parameters["context_id"] = json!(row.context_id);
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
                            authorized_message(
                                &state,
                                &node,
                                &requester,
                                &row,
                                &input,
                            )
                            .await?;
                            match input["type"].as_str() {
                                Some("refresh_frame") => {
                                    if last_refresh.elapsed() >= Duration::from_secs(1) {
                                        command(
                                            &state,
                                            &node.id,
                                            Operation::DesktopOpen,
                                            json!({
                                                "display": row.display,
                                                "session_id": row.session_id,
                                                "context_id": row.context_id,
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
                                            "display":row.display,"session_id":row.session_id,
                                            "context_id":row.context_id,
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
                                            "display":row.display,"session_id":row.session_id,
                                            "context_id":row.context_id,
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
                                    stop(&state, &row).await?;
                                }
                                _ => return Err(AppError::MachineNotAllowed),
                            }
                            Ok(())
                        }
                        .await;
                        let value = match result {
                            Ok(()) => json!({
                                "type":"state",
                                "controller":row.status,
                                "controls":controls,
                                "reason":row.reason
                            }),
                            Err(AppError::ValidationError(message)) => json!({
                                "type":"error", "message":message
                            }),
                            Err(_) => json!({
                                "type":"error",
                                "message":"The control change could not finish. Refresh the panel and try again; the agent remains paused during an incomplete takeover."
                            }),
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
            "display":row.display,"session_id":row.session_id,
            "context_id":row.context_id,
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
                "connect_link_id":&row.id,
                "conversation_id":conversation,
                "status":"pending"
            },
            doc!{
                "$setOnInsert":{
                    "_id":uuid::Uuid::new_v4().to_string(),
                    "user_id":&row.user_id,
                    "kind":"machine_control",
                    "connect_link_id":&row.id,
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
    display: nyxid_machine::desktop::Display,
    context_id: Option<&str>,
) -> AppResult<Value> {
    authorized_context(state, node, &chat.user_id, context_id).await?;
    let row = desktop::open_display_for_context(
        &state.db,
        &chat.user_id,
        &node.id,
        Some(&chat.conversation_id),
        context_id,
        display,
    )
    .await?;
    let row = desktop::request(&state.db, &row, reason).await?;
    let secret =
        node_service::get_node_signing_secret(&state.db, &state.encryption_keys, &node.id).await?;
    command(
        state,
        &node.id,
        Operation::DesktopOpen,
        json!({"display":row.display,"session_id":row.session_id,"context_id":row.context_id}),
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
            "display":row.display,"session_id":row.session_id,
            "context_id":row.context_id,
            "owner":true,
            "viewer_id":"waiting-for-owner",
            "revision":row.revision
        }),
        &secret,
    )
    .await?;
    watch(state, &row).await?;
    let link = AssistantPage::MachineDesktop {
        node: &node.id,
        conversation: Some(&chat.conversation_id),
        context_id: row.context_id.as_deref(),
        display,
    }
    .url(&state.config.frontend_url);
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
    use uuid::Uuid;

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

    async fn context_upgrade_status(
        state: &AppState,
        actor: &str,
        node: &str,
        context: &str,
    ) -> u16 {
        let app = Router::new()
            .route("/machines/{node}/desktop", get(endpoint))
            .layer(Extension(crate::test_utils::test_auth_user(actor)))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut request = format!(
            "ws://{}/machines/{node}/desktop?context_id={context}",
            listener.local_addr().unwrap(),
        )
        .into_client_request()
        .unwrap();
        request.headers_mut().insert(
            "origin",
            url::Url::parse(&state.config.frontend_url)
                .unwrap()
                .origin()
                .ascii_serialization()
                .parse()
                .unwrap(),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let status = match connect_async(request).await {
            Ok((mut socket, response)) => {
                // Closing a viewer only drops this WebSocket; it does not send
                // a context-selectable close command or alter another session.
                let _ = socket.close(None).await;
                response.status().as_u16()
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                response.status().as_u16()
            }
            Err(error) => panic!("unexpected desktop handshake failure: {error}"),
        };
        server.abort();
        status
    }

    async fn enable_context_desktops(state: &AppState, node: &mut Node) {
        let profile = node.machine.as_mut().unwrap();
        profile.authority_versions = vec![2];
        profile.separated = Some(nyxid_machine::context::Support {
            available: true,
            ..Default::default()
        });
        state
            .db
            .collection::<mongodb::bson::Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                mongodb::bson::doc! {"_id": &node.id},
                mongodb::bson::doc! {"$set": {"machine": mongodb::bson::to_bson(profile).unwrap()}},
            )
            .await
            .unwrap();
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

    #[tokio::test]
    async fn separated_context_desktop_access_is_scoped_to_actor_or_personal_owner() {
        use crate::{
            models::{machine_access::Context, user::UserType},
            services::machine_integration_tests,
            test_utils::{test_membership, test_user},
        };

        let f = crate::services::assistant_authority_tests::orchestrator_fixture(
            "machine_desktop_context_acl",
        )
        .await;
        let org = Uuid::new_v4().to_string();
        let actor_x = Uuid::new_v4().to_string();
        let actor_y = Uuid::new_v4().to_string();
        for (id, user_type) in [
            (&org, UserType::Org),
            (&actor_x, UserType::Person),
            (&actor_y, UserType::Person),
        ] {
            f.state
                .db
                .collection(crate::models::user::COLLECTION_NAME)
                .insert_one(test_user(id, user_type))
                .await
                .unwrap();
        }
        let memberships = f
            .state
            .db
            .collection(crate::models::org_membership::COLLECTION_NAME);
        for actor in [&actor_x, &actor_y] {
            memberships
                .insert_one(test_membership(
                    &org,
                    actor,
                    crate::models::org_membership::OrgRole::Admin,
                    None,
                ))
                .await
                .unwrap();
        }
        let mut org_node =
            machine_integration_tests::ungranted_node_named(&f, &org, "org-desktop-contexts").await;
        enable_context_desktops(&f.state, &mut org_node).await;
        // Clone only fixture metadata to create an org-owned specialist. It
        // cannot be found through the caller's personal-agent listing.
        let mut org_agent =
            crate::services::assistant_team_service::ensure_nyxbot(&f.state.db, &actor_x)
                .await
                .unwrap();
        org_agent.id = Uuid::new_v4().to_string();
        org_agent.user_id = org.clone();
        org_agent.kind = crate::models::assistant_agent::AgentKind::Specialist;
        org_agent.name = "org-engineer".into();
        org_agent.display_name = Some("Org Engineer".into());
        f.state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(
                crate::models::assistant_agent::COLLECTION_NAME,
            )
            .insert_one(&org_agent)
            .await
            .unwrap();
        let org_context = Context {
            id: Uuid::new_v4().to_string(),
            node_id: org_node.id.clone(),
            agent_id: org_agent.id.clone(),
            owner_id: org.clone(),
            actor_id: actor_x.clone(),
            group_id: None,
            generation: 1,
            mode: "separated".into(),
        };
        f.state
            .db
            .collection::<Context>(crate::models::machine_access::CONTEXTS)
            .insert_one(&org_context)
            .await
            .unwrap();

        let x_rows = contexts(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&actor_x),
            Path(org_node.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(x_rows.len(), 1, "the context actor can list their context");
        assert_eq!(x_rows[0].agent_name, "Org Engineer");
        let y_rows = contexts(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&actor_y),
            Path(org_node.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert!(y_rows.is_empty(), "other org writers cannot list it");
        assert!(
            desktop::open_display_for_context(
                &f.state.db,
                &actor_y,
                &org_node.id,
                None,
                Some(&org_context.id),
                nyxid_machine::desktop::Display::Secure,
            )
            .await
            .is_err(),
            "even a new desktop must authorize its context before inserting state"
        );
        assert!(
            desktop::get(
                &f.state.db,
                &desktop::desktop_id(
                    &org_node.id,
                    Some(&org_context.id),
                    nyxid_machine::desktop::Display::Secure,
                )
            )
            .await
            .unwrap()
            .is_none()
        );
        for (actor, context, expected) in [
            (&actor_y, org_context.id.as_str(), 403),
            (&actor_x, "missing-context", 403),
            (&actor_x, org_context.id.as_str(), 101),
        ] {
            assert_eq!(
                Box::pin(context_upgrade_status(
                    &f.state,
                    actor,
                    &org_node.id,
                    context
                ))
                .await,
                expected,
            );
        }
        assert!(
            authorized_context(&f.state, &org_node, &actor_x, Some(&org_context.id))
                .await
                .is_ok()
        );
        assert!(
            authorized_context(&f.state, &org_node, &actor_y, Some(&org_context.id))
                .await
                .is_err()
        );
        assert!(
            authorized_context(&f.state, &org_node, &actor_x, Some("forged-context-id"),)
                .await
                .is_err()
        );
        assert!(
            list(
                State(f.state.clone()),
                crate::test_utils::test_auth_user(&actor_y),
                Query(DesktopQuery {
                    conversation_id: None,
                    context_id: Some(org_context.id.clone()),
                    display: nyxid_machine::desktop::Display::Secure,
                }),
            )
            .await
            .is_err(),
            "a forged context selector is refused by the desktop list route"
        );
        let mut y_chat = f.chat.clone();
        y_chat.user_id = actor_y.clone();
        assert!(
            request_control(
                &f.state,
                &y_chat,
                &org_node,
                "forged context control",
                nyxid_machine::desktop::Display::Secure,
                Some(&org_context.id),
            )
            .await
            .is_err(),
            "a different org member cannot request control of the actor's context"
        );

        let mut personal_node = machine_integration_tests::node(&f, &f.owner).await;
        enable_context_desktops(&f.state, &mut personal_node).await;
        let personal_context = Context {
            id: Uuid::new_v4().to_string(),
            node_id: personal_node.id.clone(),
            agent_id: Uuid::new_v4().to_string(),
            owner_id: f.owner.clone(),
            actor_id: actor_x,
            group_id: None,
            generation: 1,
            mode: "separated".into(),
        };
        f.state
            .db
            .collection::<Context>(crate::models::machine_access::CONTEXTS)
            .insert_one(&personal_context)
            .await
            .unwrap();
        assert!(
            authorized_context(
                &f.state,
                &personal_node,
                &f.owner,
                Some(&personal_context.id),
            )
            .await
            .is_ok()
        );
        let actor_row = desktop::open_display_for_context(
            &f.state.db,
            &personal_context.actor_id,
            &personal_node.id,
            None,
            Some(&personal_context.id),
            nyxid_machine::desktop::Display::Secure,
        )
        .await
        .unwrap();
        let owner_row = desktop::open_display_for_context(
            &f.state.db,
            &f.owner,
            &personal_node.id,
            Some(&f.row.id),
            Some(&personal_context.id),
            nyxid_machine::desktop::Display::Secure,
        )
        .await
        .unwrap();
        assert_eq!(owner_row.session_id, actor_row.session_id);
        assert_eq!(owner_row.user_id, personal_context.actor_id);
        assert!(
            owner_row.conversation_id.is_none(),
            "owner observation must not rebind the actor's conversation"
        );
        let taken = desktop::take(&f.state.db, &owner_row, &Uuid::new_v4().to_string())
            .await
            .unwrap();
        assert_eq!(taken.status, "taking");
        assert_eq!(
            contexts(
                State(f.state.clone()),
                crate::test_utils::test_auth_user(&f.owner),
                Path(personal_node.id.clone())
            )
            .await
            .unwrap()
            .0
            .len(),
            1,
        );
        assert_eq!(
            Box::pin(context_upgrade_status(
                &f.state,
                &f.owner,
                &personal_node.id,
                &personal_context.id
            ))
            .await,
            101,
        );
        assert_eq!(
            Box::pin(context_upgrade_status(
                &f.state,
                &org_context.actor_id,
                &org_node.id,
                &personal_context.id
            ))
            .await,
            403,
            "a real context on another machine cannot be substituted",
        );
        let mut old_node = personal_node.clone();
        old_node
            .machine
            .as_mut()
            .unwrap()
            .authority_versions
            .clear();
        assert!(matches!(
            authorized_context(&f.state, &old_node, &f.owner, Some(&personal_context.id)).await,
            Err(AppError::MachineAuthorityUnsupported)
        ));
        // Missing support never turns a context into the shared desktop.
        old_node.machine.as_mut().unwrap().authority_versions = vec![2];
        old_node.machine.as_mut().unwrap().separated = None;
        assert!(matches!(
            authorized_context(&f.state, &old_node, &f.owner, Some(&personal_context.id)).await,
            Err(AppError::MachineAuthorityUnsupported)
        ));
        authorized_context(&f.state, &old_node, &f.owner, None)
            .await
            .unwrap();

        let org_row = desktop::open_display_for_context(
            &f.state.db,
            &org_context.actor_id,
            &org_node.id,
            None,
            Some(&org_context.id),
            nyxid_machine::desktop::Display::Secure,
        )
        .await
        .unwrap();
        assert!(
            stop(&f.state, &org_row).await.is_err(),
            "an unbound context desktop cannot stop the whole machine"
        );
        for operation in [
            "take_control",
            "hand_back",
            "stop",
            "refresh_frame",
            "input",
        ] {
            let message = json!({"type": operation});
            authorized_message(
                &f.state,
                &org_node,
                &org_context.actor_id,
                &org_row,
                &message,
            )
            .await
            .unwrap();
            assert!(
                authorized_message(&f.state, &org_node, &actor_y, &org_row, &message)
                    .await
                    .is_err()
            );
            let forged = json!({"type":operation, "context_id":personal_context.id});
            assert!(
                authorized_message(
                    &f.state,
                    &org_node,
                    &org_context.actor_id,
                    &org_row,
                    &forged
                )
                .await
                .is_err()
            );
        }
        // The connected actor cannot keep controlling a deleted/rebound
        // context. Stale desktop ownership must not leak it in listings either.
        f.state
            .db
            .collection::<mongodb::bson::Document>(crate::models::machine_access::CONTEXTS)
            .update_one(
                mongodb::bson::doc! {"_id": &org_context.id},
                mongodb::bson::doc! {"$set": {"actor_id": &actor_y}},
            )
            .await
            .unwrap();
        for input in [json!({"type":"take_control"}), json!({"type":"input"})] {
            assert!(
                authorized_message(&f.state, &org_node, &org_context.actor_id, &org_row, &input)
                    .await
                    .is_err()
            );
        }
        let listed = list(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&org_context.actor_id),
            Query(DesktopQuery {
                conversation_id: None,
                context_id: None,
                display: nyxid_machine::desktop::Display::Secure,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            !listed
                .iter()
                .any(|row| row.context_id.as_deref() == Some(&org_context.id))
        );
        f.state.db.drop().await.unwrap();
    }
}
