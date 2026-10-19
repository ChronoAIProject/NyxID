use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use utoipa::ToSchema;

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

fn record_client_ip(state: &AppState, ip: IpAddr) {
    let hash = crate::services::auth_device_service::hmac_hex(
        state.auth_device_hmac_key.as_slice(),
        ip.to_string().as_bytes(),
    );
    tracing::Span::current().record("client_ip_hash", hash.as_str());
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn record_key(key: &str, credential: Option<&str>) {
    tracing::Span::current().record("api_key_id", key);
    if let Some(id) = credential {
        tracing::Span::current().record("credential_id", id);
    }
}

fn observe<T>(result: AppResult<T>, success: &str) -> AppResult<T> {
    let outcome = result
        .as_ref()
        .map_or_else(|error| service::error_outcome(error), |_| success);
    tracing::Span::current().record("outcome", outcome);
    tracing::info!(
        outcome,
        error_code = result.as_ref().err().map(AppError::error_code),
        "agent_key_login.handler.outcome"
    );
    result
}

type PrivateJson<T> = (HeaderMap, Json<T>);

fn private_json<T>(value: T) -> PrivateJson<T> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    (headers, Json(value))
}

#[derive(Deserialize, ToSchema)]
#[schema(as = AgentKeyLoginRequestBody)]
pub struct RequestBody {
    #[serde(flatten)]
    pub context: AuthDeviceRequestBody,
}

#[derive(Serialize, ToSchema)]
#[schema(as = AgentKeyLoginRequestResponse)]
pub struct RequestResponse {
    #[serde(flatten)]
    pub request: service::RequestOutput,
    pub verification_uri: String,
}

#[derive(Deserialize, ToSchema)]
#[schema(as = AgentKeyLoginCodeBody)]
pub struct CodeBody {
    pub user_code: String,
}

#[derive(Deserialize, ToSchema)]
#[schema(as = AgentKeyLoginPollBody)]
pub struct PollBody {
    pub device_code: String,
}

#[derive(Deserialize, ToSchema)]
#[schema(as = AgentKeyLoginApproveBody)]
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

#[derive(Serialize, ToSchema)]
#[schema(as = AgentKeyLoginDeliveryResponse)]
pub struct DeliveryResponse {
    pub credential: String,
    pub credential_id: String,
    pub credential_expires_at: Option<String>,
    pub label: String,
    pub api_key: service::KeySummary,
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginPreviewResponse)]
pub struct PreviewResponse {
    #[serde(flatten)]
    pub context: AuthDevicePreviewResponse,
    pub interval: u32,
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginDecisionResponse)]
pub struct DecisionResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginCredentialResponse)]
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

#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginCredentialsResponse)]
pub struct CredentialsResponse {
    pub credentials: Vec<CredentialResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginSelfResponse)]
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
    route: &str,
) -> AppResult<String> {
    super::login_client_context::require_first_party_human(user)?;
    if !matches!(
        user.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken
    ) {
        return Err(AppError::Forbidden(
            "A human account session is required".into(),
        ));
    }
    let ip = resolve_client_ip(headers, addr, state)?;
    record_client_ip(state, ip);
    if !state
        .auth_agent_key_approve_limiter
        .check_shared(ip)
        .await?
        || !state
            .auth_agent_key_approve_per_user_limiter
            .check_shared(&format!("user:{}", user.user_id))
            .await?
    {
        tracing::warn!(
            route,
            outcome = "rate_limit_hit",
            "agent_key_login.rate_limit_hit"
        );
        return Err(AppError::AgentKeyLoginRateLimited);
    }
    Ok(ip.to_string())
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/agent-key/request",
    request_body = RequestBody,
    responses(
        (status = 200, body = RequestResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "request",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn request(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RequestBody>,
) -> AppResult<PrivateJson<RequestResponse>> {
    let result = async {
        let ip = resolve_client_ip(&headers, addr, &state)?;
        record_client_ip(&state, ip);
        if !state
            .auth_agent_key_request_limiter
            .check_shared(ip)
            .await?
        {
            tracing::warn!(outcome = "rate_limit_hit", "agent_key_login.rate_limit_hit");
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
        let requested_profile = body.context.requested_profile.clone();
        let request = service::request(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            capture_client_context(&headers, addr, &state, body.context)?,
            requested_profile,
        )
        .await?;
        Ok(private_json(RequestResponse {
            request,
            verification_uri,
        }))
    }
    .await;
    observe(result, "requested")
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/agent-key/preview",
    request_body = CodeBody,
    responses(
        (status = 200, body = PreviewResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "preview",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn preview(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> AppResult<PrivateJson<PreviewResponse>> {
    let result = async {
        let client = resolve_client_context(&headers, addr, &state)?;
        record_client_ip(&state, client.ip);
        if !state
            .auth_agent_key_preview_limiter
            .check_shared(client.ip)
            .await?
        {
            tracing::warn!(outcome = "rate_limit_hit", "agent_key_login.rate_limit_hit");
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
            interval: result.interval,
        }))
    }
    .await;
    observe(result, "previewed")
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/agent-key/options",
    request_body = CodeBody,
    responses(
        (status = 200, body = service::LoginOptions),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "options",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn options(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> AppResult<PrivateJson<service::LoginOptions>> {
    let result = async {
        decision_limit(&state, &user, &headers, addr, "options").await?;
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
    .await;
    observe(result, "options")
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/agent-key/approve",
    request_body = ApproveBody,
    responses(
        (status = 200, body = DecisionResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 409, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "approve",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ApproveBody>,
) -> AppResult<PrivateJson<DecisionResponse>> {
    let result = async {
        let ip = decision_limit(&state, &user, &headers, addr, "approve").await?;
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
            user_agent(&headers).as_deref(),
        )
        .await?;
        Ok(private_json(DecisionResponse { ok: true }))
    }
    .await;
    observe(result, "approved")
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/agent-key/deny",
    request_body = CodeBody,
    responses(
        (status = 200, body = DecisionResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "deny",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn deny(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> AppResult<PrivateJson<DecisionResponse>> {
    let result = async {
        let ip = decision_limit(&state, &user, &headers, addr, "deny").await?;
        service::deny(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            &user.user_id.to_string(),
            &body.user_code,
            Some(&ip),
            user_agent(&headers).as_deref(),
        )
        .await?;
        Ok(private_json(DecisionResponse { ok: true }))
    }
    .await;
    observe(result, "denied")
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/agent-key/poll",
    request_body = PollBody,
    responses(
        (status = 200, body = DeliveryResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "poll",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn poll(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    tele: TelemetryContext,
    Json(body): Json<PollBody>,
) -> AppResult<PrivateJson<DeliveryResponse>> {
    let result = async {
        let ip = resolve_client_ip(&headers, addr, &state)?;
        record_client_ip(&state, ip);
        if !state.auth_agent_key_poll_limiter.check_shared(ip).await? {
            tracing::warn!(outcome = "rate_limit_hit", "agent_key_login.rate_limit_hit");
            return Err(AppError::AgentKeyLoginRateLimited);
        }
        let result = service::poll(
            &state.db,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            &body.device_code,
            Some(&ip.to_string()),
            user_agent(&headers).as_deref(),
        )
        .await?;
        record_key(&result.api_key.id, Some(&result.credential_id));
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
    .await;
    observe(result, "delivered")
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

#[utoipa::path(
    get,
    path = "/api/v1/auth/agent-key/self",
    responses(
        (status = 200, body = SelfResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "get_self",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn get_self(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<PrivateJson<SelfResponse>> {
    let result = async {
        record_client_ip(&state, resolve_client_ip(&headers, addr, &state)?);
        let (key, credential) = child_identity(&user)?;
        record_key(key, Some(credential));
        let (api_key, row) = service::self_metadata(&state.db, key, credential).await?;
        tracing::Span::current().record("row_id", row.login_request_id.as_str());
        Ok(private_json(SelfResponse {
            api_key,
            credential_id: row.id,
            credential_expires_at: row.expires_at.map(|d| d.to_rfc3339()),
            label: row.label,
        }))
    }
    .await;
    observe(result, "read")
}

#[utoipa::path(
    delete,
    path = "/api/v1/auth/agent-key/self",
    responses(
        (status = 200, body = DecisionResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "delete_self",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn delete_self(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<PrivateJson<DecisionResponse>> {
    let result = async {
        record_client_ip(&state, resolve_client_ip(&headers, addr, &state)?);
        let (key, credential) = child_identity(&user)?;
        record_key(key, Some(credential));
        credentials::revoke(&state.db, credential, CredentialRevokedReason::Logout).await?;
        Ok(private_json(DecisionResponse { ok: true }))
    }
    .await;
    observe(result, "revoked")
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/{key_id}/credentials",
    params(("key_id" = String, Path, description = "Parent key ID")),
    responses(
        (status = 200, body = CredentialsResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "list_credentials",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn list_credentials(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> AppResult<PrivateJson<CredentialsResponse>> {
    let result = async {
        record_client_ip(&state, resolve_client_ip(&headers, addr, &state)?);
        record_key(&key_id, None);
        let rows = credentials::list(&state.db, &user.user_id.to_string(), &key_id).await?;
        Ok(private_json(CredentialsResponse {
            credentials: rows.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    observe(result, "listed")
}

#[utoipa::path(
    delete,
    path = "/api/v1/api-keys/{key_id}/credentials/{credential_id}",
    params(("key_id" = String, Path, description = "Parent key ID"), ("credential_id" = String, Path, description = "Login credential ID")),
    responses(
        (status = 200, body = DecisionResponse),
        (status = 400, body = crate::errors::ErrorResponse),
        (status = 401, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse),
        (status = 404, body = crate::errors::ErrorResponse),
        (status = 410, body = crate::errors::ErrorResponse),
        (status = 429, body = crate::errors::ErrorResponse),
        (status = 500, body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Agent Key Login"
)]
#[tracing::instrument(
    skip_all,
    fields(
        client_ip_hash,
        route = "revoke_credential",
        row_id,
        api_key_id,
        credential_id,
        outcome
    )
)]
pub async fn revoke_credential(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((key_id, credential_id)): Path<(String, String)>,
) -> AppResult<PrivateJson<DecisionResponse>> {
    let result = async {
        record_client_ip(&state, resolve_client_ip(&headers, addr, &state)?);
        record_key(&key_id, Some(&credential_id));
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
    .await;
    observe(result, "revoked")
}

#[cfg(test)]
mod tests;
