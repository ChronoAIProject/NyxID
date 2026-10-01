use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::machine_setup::{Choices, MachineSetup},
    mw::auth::AuthUser,
    services::{assistant_acknowledgement_service as acks, machine_setup_service as setup},
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::net::SocketAddr;
use zeroize::Zeroizing;

#[derive(Serialize)]
pub struct SetupInfo {
    id: String,
    choices: Choices,
    status: String,
    hostname: Option<String>,
    os: Option<String>,
    ip: Option<String>,
    conversation_id: Option<String>,
    expires_at: String,
    machine: Option<nyxid_machine::MachineProfile>,
}
impl SetupInfo {
    fn new(row: MachineSetup, machine: Option<nyxid_machine::MachineProfile>) -> Self {
        Self {
            id: row.id,
            choices: row.choices,
            status: row.status,
            hostname: row.hostname,
            os: row.os,
            ip: row.ip,
            conversation_id: row.conversation_id,
            expires_at: row.expires_at.to_rfc3339(),
            machine,
        }
    }
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(choices): Json<Choices>,
) -> AppResult<Json<SetupInfo>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    rate_owner(&state, &owner).await?;
    let row = setup::create_link(&state.db, &owner, None, choices).await?;
    Ok(Json(SetupInfo::new(row, None)))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SetupInfo>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let mut row = setup::get(&state.db, &auth.user_id.to_string(), &id).await?;
    let node = crate::services::node_service::get_node_by_id(&state.db, &id).await?;
    if let Some(node) = &node {
        if node.user_id != row.choices.owner_id.as_deref().unwrap_or(&row.user_id)
            || !crate::services::org_service::resolve_owner_access(
                &state.db,
                &auth.user_id.to_string(),
                &node.user_id,
            )
            .await?
            .can_write()
        {
            return Err(AppError::MachineNotAllowed);
        }
        if let Some(profile) = &node.machine {
            row.status = if node.status != crate::models::node::NodeStatus::Online {
                "offline"
            } else if profile.computer && !profile.computer_ready {
                "permissions_missing"
            } else {
                "connected"
            }
            .into();
        }
    } else if row.expires_at <= chrono::Utc::now()
        && !matches!(row.status.as_str(), "declined" | "failed")
    {
        row.status = "expired".into();
    }
    Ok(Json(SetupInfo::new(row, node.and_then(|n| n.machine))))
}

#[derive(Serialize)]
pub struct TokenResponse {
    token: Zeroizing<String>,
}
impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenResponse { [REDACTED] }")
    }
}
pub async fn mint(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(choices): Json<Choices>,
) -> AppResult<Json<TokenResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    rate_owner(&state, &owner).await?;
    let token = setup::mint(
        &state.db,
        &owner,
        &id,
        Some(choices),
        state.config.node_max_per_user,
        "review",
    )
    .await?;
    Ok(Json(TokenResponse { token }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRequest {
    hostname: String,
    os: String,
    capabilities: Vec<String>,
}
#[derive(Serialize)]
pub struct PairResponse {
    code: String,
    device: Zeroizing<String>,
    url: String,
    expires_in: i64,
    interval: u32,
}
impl std::fmt::Debug for PairResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairResponse { [REDACTED] }")
    }
}
pub async fn request_pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PairRequest>,
) -> AppResult<Json<PairResponse>> {
    let ip = super::login_client_context::resolve_client_ip(&headers, peer, &state)?;
    if !state.auth_device_request_limiter.check_shared(ip).await? {
        return Err(AppError::AuthDeviceCodeRateLimited);
    }
    let pair = setup::initiate(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &body.hostname,
        &body.os,
        &ip.to_string(),
        body.capabilities,
    )
    .await?;
    let url = format!(
        "{}/machines/pair?code={}",
        state.config.frontend_url.trim_end_matches('/'),
        pair.code
    );
    Ok(Json(PairResponse {
        code: pair.code,
        device: pair.device,
        url,
        expires_in: setup::TTL_SECONDS,
        interval: 2,
    }))
}

#[derive(Deserialize)]
pub struct PollRequest {
    device: Zeroizing<String>,
}
#[derive(Serialize)]
pub struct PollResponse {
    status: &'static str,
    token: Option<Zeroizing<String>>,
}
impl std::fmt::Debug for PollResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PollResponse { [REDACTED] }")
    }
}
pub async fn poll(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PollRequest>,
) -> AppResult<Json<PollResponse>> {
    let ip = super::login_client_context::resolve_client_ip(&headers, peer, &state)?;
    if !state.auth_device_poll_limiter.check_shared(ip).await? {
        return Err(AppError::AuthDeviceCodeRateLimited);
    }
    let token = setup::poll(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &body.device,
        state.config.node_max_per_user,
    )
    .await?;
    Ok(Json(PollResponse {
        status: if token.is_some() {
            "approved"
        } else {
            "pending"
        },
        token,
    }))
}

#[derive(Deserialize)]
pub struct CodeRequest {
    code: String,
}
pub async fn preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CodeRequest>,
) -> AppResult<Json<SetupInfo>> {
    super::login_client_context::require_first_party_human(&auth)?;
    rate_owner(&state, &auth.user_id.to_string()).await?;
    let row = setup::by_code(&state.db, state.auth_device_hmac_key.as_slice(), &body.code).await?;
    if row.status != "pending" {
        return Err(AppError::Conflict("Pairing was already decided".into()));
    }
    Ok(Json(SetupInfo::new(row, None)))
}
#[derive(Deserialize)]
pub struct Decision {
    code: String,
    approve: bool,
}
pub async fn decide(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Decision>,
) -> AppResult<Json<SetupInfo>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    rate_owner(&state, &owner).await?;
    let row = setup::by_code(&state.db, state.auth_device_hmac_key.as_slice(), &body.code).await?;
    let row = setup::decide(&state.db, &owner, &row.id, body.approve, None).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "machine_pairing_decided",
        Some(json!({"setup_id":row.id,"approved":body.approve})),
    );
    Ok(Json(SetupInfo::new(row, None)))
}

async fn rate_owner(state: &AppState, owner: &str) -> AppResult<()> {
    if !state
        .auth_device_approve_per_user_limiter
        .check_shared(owner)
        .await?
    {
        return Err(AppError::AuthDeviceCodeRateLimited);
    }
    Ok(())
}

pub async fn link_tool(
    state: &AppState,
    chat: &acks::ChatAuthority,
    args: &Value,
) -> AppResult<(Value, bool)> {
    crate::services::machine_service::caller(chat)?;
    rate_owner(state, &chat.user_id).await?;
    let choices = Choices {
        owner_id: None,
        name: args["name"].as_str().unwrap_or("my-machine").into(),
        location: args["where"].as_str().unwrap_or("vm").into(),
        capabilities: serde_json::from_value(
            args.get("capabilities")
                .cloned()
                .unwrap_or(json!(["shell", "files"])),
        )
        .map_err(|_| AppError::ValidationError("Invalid machine capabilities".into()))?,
        grant_to: args["grant_to"].as_str().map(str::to_owned),
    };
    let row = setup::create_link(
        &state.db,
        &chat.user_id,
        Some(&chat.conversation_id),
        choices,
    )
    .await?;
    Ok((
        json!({"url":format!("{}/machines/new?setup={}",state.config.frontend_url.trim_end_matches('/'),row.id),"choices":row.choices,"note":"Give the owner this link and end your turn. Recommend a VM or container. NyxID wakes this thread when the machine connects; then use nyx__machine_list and a harmless check such as git --version, apply the requested specialist grant, and continue. Setup credentials appear only on the owner's page, never in chat."}),
        false,
    ))
}

pub async fn pair_tool(
    state: &AppState,
    chat: &acks::ChatAuthority,
    args: &Value,
) -> AppResult<(Value, bool)> {
    crate::services::machine_service::caller(chat)?;
    rate_owner(state, &chat.user_id).await?;
    let code = args["code"].as_str().unwrap_or_default();
    let row = setup::by_code(&state.db, state.auth_device_hmac_key.as_slice(), code).await?;
    if row.status != "pending" {
        return Err(AppError::Conflict("Pairing was already decided".into()));
    }
    let mut canonical = args.clone();
    canonical["code"] = json!(setup::normalize_code(code)?);
    canonical["pairing_id"] = json!(row.id);
    if let Some(id) = args["acknowledgement_id"].as_str()
        && acks::consume_action(&state.db, chat, id, "nyxid__machine_pair", &canonical).await?
    {
        setup::decide(
            &state.db,
            &chat.user_id,
            &row.id,
            true,
            Some(&chat.conversation_id),
        )
        .await?;
        return Ok((
            json!({"status":"approved","note":"Pairing approved. End your turn; NyxID wakes this thread when connected. Verify with machine_list and a harmless command."}),
            false,
        ));
    }
    let details = format!(
        "Pair machine {} ({}, IP {}) with capabilities {}. Confirm only if you started this setup. Commands have the machine user's full access; prefer a VM or container.",
        row.hostname.as_deref().unwrap_or_default(),
        row.os.as_deref().unwrap_or_default(),
        row.ip.as_deref().unwrap_or_default(),
        row.choices.capabilities.join(", ")
    );
    let card = acks::request(
        &state.db,
        chat,
        acks::Request {
            kind: "action",
            service: Some((&row.id, &row.id, &row.choices.name)),
            tool: Some("nyxid__machine_pair"),
            arguments: Some(&canonical),
            summary: &details,
            platform: false,
        },
    )
    .await?;
    // Bind the watch before the card is decided, including a decline or expiry.
    setup::watch(
        &state.db,
        &chat.user_id,
        &chat.conversation_id,
        &row.id,
        row.expires_at,
    )
    .await?;
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::machine_setup::COLLECTION_NAME)
        .update_one(
            mongodb::bson::doc! {"_id":&row.id,"status":"pending"},
            mongodb::bson::doc! {"$set":{"acknowledgement_id":&card.id}},
        )
        .await?;
    Ok((acks::refusal(&card), false))
}
