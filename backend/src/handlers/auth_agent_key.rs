use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use super::{
    auth_device::{AuthDevicePreviewResponse, AuthDeviceRequestBody, preview_response},
    login_client_context::*,
};
use crate::models::api_key_credential::{ApiKeyCredential, CredentialRevokedReason};
use crate::services::{
    api_key_credential_service as credentials, api_key_validation,
    auth_agent_key_login_service as service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
};

type PrivateJson<T> = (HeaderMap, Json<T>);

fn private_json<T>(value: T) -> PrivateJson<T> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    (headers, Json(value))
}

#[derive(Deserialize)]
pub struct RequestBody {
    #[serde(flatten)]
    pub context: AuthDeviceRequestBody,
    pub requested_profile: Option<String>,
}

#[derive(Serialize)]
pub struct RequestResponse {
    #[serde(flatten)]
    pub request: service::RequestOutput,
    pub verification_uri: String,
}

#[derive(Deserialize)]
pub struct CodeBody {
    pub user_code: String,
}

#[derive(Deserialize)]
pub struct PollBody {
    pub device_code: String,
}

#[derive(Deserialize)]
pub struct ApproveBody {
    pub user_code: String,
    pub selection: service::Selection,
    pub credential_expires_at: Option<String>,
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => { $(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+ };
}
redacted_debug!(
    RequestBody,
    RequestResponse,
    CodeBody,
    PollBody,
    ApproveBody,
    DeliveryResponse
);

#[derive(Serialize)]
pub struct DeliveryResponse {
    pub credential: String,
    pub credential_id: String,
    pub credential_expires_at: Option<String>,
    pub label: String,
    pub api_key: service::KeySummary,
}

#[derive(Debug, Serialize)]
pub struct PreviewKeySummary {
    pub name: String,
    pub owner_type: String,
    pub scopes: String,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub allowed_service_count: usize,
    pub allowed_node_count: usize,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    #[serde(flatten)]
    pub context: AuthDevicePreviewResponse,
    pub requested_profile: Option<String>,
    pub interval: u32,
    pub api_key: Option<PreviewKeySummary>,
}

#[derive(Debug, Serialize)]
pub struct DecisionResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct CredentialResponse {
    pub id: String,
    pub label: String,
    pub secret_prefix: String,
    pub is_active: bool,
    pub revoked_reason: Option<CredentialRevokedReason>,
    pub revoked_at: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

impl From<ApiKeyCredential> for CredentialResponse {
    fn from(row: ApiKeyCredential) -> Self {
        Self {
            id: row.id,
            label: row.label,
            secret_prefix: row.secret_prefix,
            is_active: row.is_active,
            revoked_reason: row.revoked_reason,
            revoked_at: row.revoked_at.map(|d| d.to_rfc3339()),
            expires_at: row.expires_at.map(|d| d.to_rfc3339()),
            last_used_at: row.last_used_at.map(|d| d.to_rfc3339()),
            created_at: row.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CredentialsResponse {
    pub credentials: Vec<CredentialResponse>,
}

#[derive(Debug, Serialize)]
pub struct SelfResponse {
    pub api_key: service::KeySummary,
    pub credential_id: String,
    pub credential_expires_at: Option<String>,
    pub label: String,
}

async fn decision_limit(
    state: &AppState,
    user: &AuthUser,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> AppResult<String> {
    if !matches!(
        user.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken
    ) {
        return Err(AppError::Forbidden(
            "A human account session is required".into(),
        ));
    }
    let ip = resolve_client_ip(headers, addr, state)?;
    if !state
        .auth_agent_key_approve_limiter
        .check_shared(ip)
        .await?
        || !state
            .auth_agent_key_approve_per_user_limiter
            .check_shared(&format!("user:{}", user.user_id))
            .await?
    {
        return Err(AppError::AgentKeyLoginRateLimited);
    }
    Ok(ip.to_string())
}

pub async fn request(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RequestBody>,
) -> AppResult<PrivateJson<RequestResponse>> {
    let ip = resolve_client_ip(&headers, addr, &state)?;
    if !state
        .auth_agent_key_request_limiter
        .check_shared(ip)
        .await?
    {
        return Err(AppError::AgentKeyLoginRateLimited);
    }
    let verification_uri = format!(
        "{}/login/agent-key",
        state.config.frontend_url.trim().trim_end_matches('/')
    );
    let parsed = url::Url::parse(&verification_uri)
        .map_err(|_| AppError::Internal("Agent Key verification URI is invalid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(AppError::Internal(
            "Agent Key verification URI is invalid".into(),
        ));
    }
    let request = service::request(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        capture_client_context(&headers, addr, &state, body.context)?,
        body.requested_profile,
    )
    .await?;
    Ok(private_json(RequestResponse {
        request,
        verification_uri,
    }))
}

pub async fn preview(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> AppResult<PrivateJson<PreviewResponse>> {
    let client = resolve_client_context(&headers, addr, &state)?;
    if !state
        .auth_agent_key_preview_limiter
        .check_shared(client.ip)
        .await?
    {
        return Err(AppError::AgentKeyLoginRateLimited);
    }
    let result = service::preview(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &body.user_code,
        Some(&client.ip.to_string()),
        auth_device_attribution(client.attribution),
    )
    .await?;
    Ok(private_json(PreviewResponse {
        context: preview_response(result.context),
        requested_profile: result.requested_profile,
        interval: result.interval,
        api_key: result.api_key.map(|key| PreviewKeySummary {
            name: key.name,
            owner_type: key.owner_type,
            scopes: key.scopes,
            allow_all_services: key.allow_all_services,
            allow_all_nodes: key.allow_all_nodes,
            allowed_service_count: key.allowed_service_ids.len(),
            allowed_node_count: key.allowed_node_ids.len(),
            expires_at: key.expires_at,
        }),
    }))
}

pub async fn options(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> AppResult<PrivateJson<service::LoginOptions>> {
    decision_limit(&state, &user, &headers, addr).await?;
    Ok(private_json(
        service::options(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            &user.user_id.to_string(),
            &body.user_code,
        )
        .await?,
    ))
}

pub async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ApproveBody>,
) -> AppResult<PrivateJson<DecisionResponse>> {
    let ip = decision_limit(&state, &user, &headers, addr).await?;
    let expiry = body
        .credential_expires_at
        .as_deref()
        .map(api_key_validation::parse_expires_at)
        .transpose()?;
    service::approve(
        &state.db,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        &user.user_id.to_string(),
        &body.user_code,
        body.selection,
        expiry,
        Some(&ip),
    )
    .await?;
    Ok(private_json(DecisionResponse { ok: true }))
}

pub async fn deny(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> AppResult<PrivateJson<DecisionResponse>> {
    decision_limit(&state, &user, &headers, addr).await?;
    service::deny(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &user.user_id.to_string(),
        &body.user_code,
    )
    .await?;
    Ok(private_json(DecisionResponse { ok: true }))
}

pub async fn poll(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    tele: TelemetryContext,
    Json(body): Json<PollBody>,
) -> AppResult<PrivateJson<DeliveryResponse>> {
    let ip = resolve_client_ip(&headers, addr, &state)?;
    if !state.auth_agent_key_poll_limiter.check_shared(ip).await? {
        return Err(AppError::AgentKeyLoginRateLimited);
    }
    let result = service::poll(
        &state.db,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        &body.device_code,
    )
    .await?;
    emit_event(
        state.telemetry.as_deref(),
        &result.api_key.owner_id,
        Some(&result.api_key.id),
        &tele,
        TelemetryEvent::AuthLoggedIn {
            method: "agent_key".into(),
            mfa_required: false,
        },
    );
    Ok(private_json(DeliveryResponse {
        credential: result.credential.to_string(),
        credential_id: result.credential_id,
        credential_expires_at: result.credential_expires_at,
        label: result.label,
        api_key: result.api_key,
    }))
}

fn child_identity(user: &AuthUser) -> AppResult<(&str, &str)> {
    match (
        user.api_key_id.as_deref(),
        user.api_key_credential_id.as_deref(),
    ) {
        (Some(key), Some(credential)) if user.auth_method == AuthMethod::ApiKey => {
            Ok((key, credential))
        }
        _ => Err(AppError::BadRequest(
            "This request must use an Agent Key login credential".into(),
        )),
    }
}

pub async fn get_self(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<PrivateJson<SelfResponse>> {
    let (key, credential) = child_identity(&user)?;
    let (api_key, row) = service::self_metadata(&state.db, key, credential).await?;
    Ok(private_json(SelfResponse {
        api_key,
        credential_id: row.id,
        credential_expires_at: row.expires_at.map(|d| d.to_rfc3339()),
        label: row.label,
    }))
}

pub async fn delete_self(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<PrivateJson<DecisionResponse>> {
    let (_, credential) = child_identity(&user)?;
    credentials::revoke(&state.db, credential, CredentialRevokedReason::Logout).await?;
    Ok(private_json(DecisionResponse { ok: true }))
}

pub async fn list_credentials(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<PrivateJson<CredentialsResponse>> {
    let rows = credentials::list(&state.db, &user.user_id.to_string(), &key_id).await?;
    Ok(private_json(CredentialsResponse {
        credentials: rows.into_iter().map(Into::into).collect(),
    }))
}

pub async fn revoke_credential(
    State(state): State<AppState>,
    user: AuthUser,
    Path((key_id, credential_id)): Path<(String, String)>,
) -> AppResult<PrivateJson<DecisionResponse>> {
    user.ensure_write_scope()?;
    credentials::web_revoke(
        &state.db,
        &user.user_id.to_string(),
        &key_id,
        &credential_id,
    )
    .await?;
    Ok(private_json(DecisionResponse { ok: true }))
}

#[cfg(test)]
mod tests;
