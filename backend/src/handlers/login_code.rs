use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use super::{
    auth_device::{AuthDevicePollResponse, AuthDeviceRequestBody},
    login_client_context::{capture_client_context, resolve_client_ip},
};
use crate::{
    AppState,
    errors::AppResult,
    models::login_code::LoginCodeStatus,
    mw::auth::AuthUser,
    services::{
        api_key_validation, auth_agent_key_login_service as agent, login_code_service as codes,
    },
};

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = LoginCodeMintBody)]
#[serde(tag = "auth_kind", rename_all = "snake_case")]
pub enum MintBody {
    AccountSession,
    AgentKey {
        selection: Box<agent::Selection>,
        credential_expires_at: Option<String>,
    },
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = LoginCodeMintResponse)]
pub struct MintResponse {
    pub request_id: String,
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for MintResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MintResponse").finish_non_exhaustive()
    }
}

impl From<codes::MintedCode> for MintResponse {
    fn from(value: codes::MintedCode) -> Self {
        Self {
            request_id: value.id,
            code: value.code.to_string(),
            expires_at: value.expires_at,
        }
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = LoginCodeRedeemBody)]
pub struct RedeemBody {
    pub code: String,
    pub requested_profile: Option<String>,
    #[serde(flatten)]
    pub context: AuthDeviceRequestBody,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = LoginCodeStatusResponse)]
pub struct StatusResponse {
    pub request_id: String,
    pub status: &'static str,
    pub auth_kind: &'static str,
    pub expires_at: DateTime<Utc>,
    pub redeemed_at: Option<DateTime<Utc>>,
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub client_ip: Option<String>,
    pub client_ip_attribution: &'static str,
    pub can_revoke: bool,
}

#[tracing::instrument(skip_all)]
#[utoipa::path(post, path = "/api/v1/auth/login-code/options",
    responses((status = 200, body = agent::LoginOptions), (status = 403, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse)), security(("bearer_auth" = [])), tag = "Login Codes")]
pub async fn options(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<agent::LoginOptions>> {
    super::login_client_context::require_first_party_human(&user)?;
    codes::limit(
        &state.db,
        "login-code-options",
        &user.user_id.to_string(),
        60,
    )
    .await?;
    Ok(Json(
        agent::options_for_actor(&state.db, &user.user_id.to_string()).await?,
    ))
}

#[tracing::instrument(skip_all)]
#[utoipa::path(post, path = "/api/v1/auth/login-code", request_body = MintBody,
    responses((status = 200, body = MintResponse), (status = 403, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse)), security(("bearer_auth" = [])), tag = "Login Codes")]
pub async fn mint(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<MintBody>,
) -> AppResult<Json<MintResponse>> {
    super::login_client_context::require_first_party_human(&user)?;
    let ip = resolve_client_ip(&headers, addr, &state)?;
    codes::limit(&state.db, "login-code-mint-source", &ip.to_string(), 10).await?;
    let (selection, expiry) = match body {
        MintBody::AccountSession => (None, None),
        MintBody::AgentKey {
            selection,
            credential_expires_at,
        } => (
            Some(*selection),
            credential_expires_at
                .as_deref()
                .filter(|v| !v.is_empty())
                .map(api_key_validation::parse_expires_at)
                .transpose()?,
        ),
    };
    Ok(Json(
        codes::mint(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            &user.user_id.to_string(),
            selection,
            expiry,
        )
        .await?
        .into(),
    ))
}

#[tracing::instrument(skip_all)]
#[utoipa::path(post, path = "/api/v1/auth/login-code/redeem", request_body = RedeemBody,
    responses((status = 200, body = AuthDevicePollResponse), (status = 400, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse), (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse)), tag = "Login Codes")]
pub async fn redeem(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RedeemBody>,
) -> AppResult<Json<AuthDevicePollResponse>> {
    let delivery = codes::redeem(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        &body.code,
        capture_client_context(&headers, addr, &state, body.context)?,
        body.requested_profile.as_deref(),
    )
    .await?;
    Ok(Json(match delivery {
        codes::Delivery::Account(tokens) => AuthDevicePollResponse::AccountSession {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            token_type: "Bearer",
            expires_in: tokens.access_expires_in,
        },
        codes::Delivery::AgentKey(delivery) => AuthDevicePollResponse::AgentKey {
            delivery: Box::new(super::auth_agent_key::DeliveryResponse {
                credential: delivery.credential.to_string(),
                credential_id: delivery.credential_id,
                credential_expires_at: delivery.credential_expires_at,
                label: delivery.label,
                api_key: delivery.api_key,
            }),
        },
    }))
}

#[tracing::instrument(skip_all)]
#[utoipa::path(get, path = "/api/v1/auth/login-code/{id}", params(("id" = String, Path)),
    responses((status = 200, body = StatusResponse), (status = 403, body = crate::errors::ErrorResponse),
        (status = 400, body = crate::errors::ErrorResponse)), security(("bearer_auth" = [])), tag = "Login Codes")]
pub async fn status(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<StatusResponse>> {
    super::login_client_context::require_first_party_human(&user)?;
    let row = codes::status(&state.db, &user.user_id.to_string(), &id).await?;
    let status = match row.status {
        LoginCodeStatus::Pending if row.expires_at <= Utc::now() => "expired",
        LoginCodeStatus::Pending => "pending",
        LoginCodeStatus::Redeemed => "redeemed",
        LoginCodeStatus::Cancelled => "cancelled",
        LoginCodeStatus::Revoked => "revoked",
    };
    let context = row.context.unwrap_or_default();
    Ok(Json(StatusResponse {
        request_id: row.id,
        status,
        auth_kind: if row.selection.is_some() || row.device_request_id.is_some() {
            "agent_key"
        } else {
            "account_session"
        },
        expires_at: row.expires_at,
        redeemed_at: row.redeemed_at,
        client_label: context.client_label,
        client_user_agent: context.client_user_agent,
        client_ip: context.client_ip,
        client_ip_attribution: context.client_ip_attribution.as_str(),
        can_revoke: row.status == LoginCodeStatus::Redeemed,
    }))
}

#[tracing::instrument(skip_all)]
#[utoipa::path(delete, path = "/api/v1/auth/login-code/{id}", params(("id" = String, Path)),
    responses((status = 200, body = super::auth_device::AuthDeviceDecisionResponse), (status = 403, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse)), security(("bearer_auth" = [])), tag = "Login Codes")]
pub async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&user)?;
    codes::cancel(&state.db, &user.user_id.to_string(), &id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[tracing::instrument(skip_all)]
#[utoipa::path(post, path = "/api/v1/auth/login-code/{id}/revoke", params(("id" = String, Path)),
    responses((status = 200, body = super::auth_device::AuthDeviceDecisionResponse), (status = 403, body = crate::errors::ErrorResponse),
        (status = 400, body = crate::errors::ErrorResponse)), security(("bearer_auth" = [])), tag = "Login Codes")]
pub async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&user)?;
    codes::revoke(
        &state.db,
        &user.user_id.to_string(),
        &id,
        Some(&state.mcp_sessions),
    )
    .await?;
    Ok(Json(serde_json::json!({"ok": true})))
}
