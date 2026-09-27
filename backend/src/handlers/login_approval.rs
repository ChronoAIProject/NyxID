use super::{
    auth::{apply_browser_session_cookies, build_cookie, extract_user_agent},
    login_client_context::resolve_client_ip,
};

#[cfg(test)]
mod tests;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        login_approval::{LoginApproval, LoginFlow},
        user::User,
    },
    services::{
        api_key_validation, audit_service, auth_agent_key_login_service as agent,
        auth_device_service as device, auth_service, catalog_service,
        login_approval_service as service, mfa_service,
    },
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use validator::Validate;

type Private<T> = (HeaderMap, Json<T>);
fn private<T>(body: T) -> Private<T> {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    (headers, Json(body))
}
fn cookie_name(id: &str) -> String {
    format!("nyx_approval_{}", id.replace('-', ""))
}
fn origin(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    let expected = url::Url::parse(&state.config.frontend_url)
        .map_err(|_| service::invalid())?
        .origin()
        .ascii_serialization();
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(expected.as_str()) {
        return Err(AppError::Forbidden(
            "Request verification must start from NyxID.".into(),
        ));
    }
    Ok(())
}
async fn limit(
    state: &AppState,
    headers: &HeaderMap,
    addr: SocketAddr,
    actor: Option<&str>,
) -> AppResult<String> {
    let ip = resolve_client_ip(headers, addr, state)?;
    if !state.auth_device_approve_limiter.check_shared(ip).await? {
        return Err(AppError::AuthDeviceCodeRateLimited);
    }
    if let Some(actor) = actor
        && !state
            .auth_device_approve_per_user_limiter
            .check_shared(&format!("user:{actor}"))
            .await?
    {
        return Err(AppError::AuthDeviceCodeRateLimited);
    }
    Ok(ip.to_string())
}
pub(super) async fn load(
    state: &AppState,
    headers: &HeaderMap,
    id: &str,
) -> AppResult<LoginApproval> {
    uuid::Uuid::parse_str(id).map_err(|_| service::invalid())?;
    let name = cookie_name(id);
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut values = cookie
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v);
    let secret = values.next().ok_or_else(service::invalid)?;
    if values.next().is_some() {
        return Err(service::invalid());
    }
    service::load(&state.db, state.auth_device_hmac_key.as_slice(), id, secret).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginBody {
    flow: LoginFlow,
    user_code: String,
    keep_signed_in: bool,
}
#[derive(Serialize)]
pub struct Identity {
    id: String,
    email: String,
    display_name: Option<String>,
}
#[derive(Serialize)]
pub struct ApprovalStatus {
    id: String,
    flow: LoginFlow,
    user_code: String,
    verified: bool,
    mfa_required: bool,
    keep_signed_in: bool,
    expires_at: String,
    user: Option<Identity>,
}
async fn response(state: &AppState, row: &LoginApproval) -> AppResult<ApprovalStatus> {
    let user = if row.verified {
        Some(service::user(&state.db, row).await?)
    } else {
        None
    };
    Ok(ApprovalStatus {
        id: row.id.clone(),
        flow: row.flow,
        user_code: row.user_code.clone(),
        verified: row.verified,
        mfa_required: row.user_id.is_some() && !row.verified,
        keep_signed_in: row.keep_signed_in,
        expires_at: row.expires_at.to_rfc3339(),
        user: user.map(|u| Identity {
            id: u.id,
            email: u.email,
            display_name: u.display_name,
        }),
    })
}
pub async fn begin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<BeginBody>,
) -> AppResult<Private<ApprovalStatus>> {
    origin(&state, &headers)?;
    limit(&state, &headers, addr, None).await?;
    let (row, secret) = service::begin(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        body.flow,
        &body.user_code,
        body.keep_signed_in,
    )
    .await?;
    let (mut headers, body) = private(response(&state, &row).await?);
    headers.append(
        header::SET_COOKIE,
        build_cookie(
            &cookie_name(&row.id),
            &secret,
            (row.expires_at - chrono::Utc::now()).num_seconds().max(1),
            "/api/v1/auth",
            state.config.use_secure_cookies(),
            None,
        )
        .parse()
        .map_err(|_| service::invalid())?,
    );
    Ok((headers, body))
}
pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Private<ApprovalStatus>> {
    let row = load(&state, &headers, &id).await?;
    Ok(private(response(&state, &row).await?))
}

async fn complete(
    state: &AppState,
    row: &LoginApproval,
    mfa: bool,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> AppResult<Private<ApprovalStatus>> {
    service::live(&state.db, state.auth_device_hmac_key.as_slice(), row).await?;
    let actor = service::user(&state.db, row).await?;
    if actor.mfa_enabled && !mfa {
        return Ok(private(response(state, row).await?));
    }
    let ip = resolve_client_ip(headers, addr, state)?.to_string();
    let ua = extract_user_agent(headers);
    let (row, session) = service::verify(&state.db, row, mfa, &ip, ua.as_deref()).await?;
    let (mut result_headers, body) = private(response(state, &row).await?);
    if let Some(session) = session {
        apply_browser_session_cookies(
            &mut result_headers,
            &session.session_token,
            state.config.use_secure_cookies(),
            state.config.cookie_domain(),
        )?;
    }
    audit_service::log_async(
        state.db.clone(),
        Some(actor.id),
        "login_request_identity_verified".into(),
        Some(
            serde_json::json!({"request_id": row.request_id, "keep_signed_in": row.keep_signed_in, "mfa_verified": mfa}),
        ),
        Some(ip),
        ua,
        None,
        None,
    );
    Ok((result_headers, body))
}
pub async fn password(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<super::auth::LoginRequest>,
) -> AppResult<Private<ApprovalStatus>> {
    origin(&state, &headers)?;
    let row = load(&state, &headers, &id).await?;
    limit(&state, &headers, addr, None).await?;
    service::attempt(&state.db, &row).await?;
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;
    let actor = auth_service::authenticate_user(&state.db, &body.email, &body.password).await?;
    let row = service::identify(&state.db, &row, &actor).await?;
    complete(&state, &row, false, &headers, addr).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MfaBody {
    code: String,
}
pub async fn mfa(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<MfaBody>,
) -> AppResult<Private<ApprovalStatus>> {
    origin(&state, &headers)?;
    let row = load(&state, &headers, &id).await?;
    let actor = service::user(&state.db, &row).await?;
    limit(&state, &headers, addr, Some(&actor.id)).await?;
    service::attempt(&state.db, &row).await?;
    if body.code.len() != 6
        || !body.code.bytes().all(|b| b.is_ascii_digit())
        || !mfa_service::verify_totp(&state.db, &state.encryption_keys, &actor.id, &body.code)
            .await?
    {
        return Err(AppError::AuthenticationFailed("Invalid MFA code".into()));
    }
    complete(&state, &row, true, &headers, addr).await
}

#[derive(Serialize)]
pub struct Inventory {
    options: agent::LoginOptions,
    catalog: super::catalog::CatalogListResponse,
}
pub async fn inventory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Private<Inventory>> {
    let row = load(&state, &headers, &id).await?;
    if !row.verified {
        return Err(service::invalid());
    }
    let actor = service::user(&state.db, &row).await?;
    limit(&state, &headers, addr, Some(&actor.id)).await?;
    let options = match row.flow {
        LoginFlow::Device => {
            device::options(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                &actor.id,
                &row.user_code,
            )
            .await?
        }
        LoginFlow::AgentKey => {
            agent::options(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                &actor.id,
                &row.user_code,
            )
            .await?
        }
    };
    let entries = catalog_service::list_catalog_all(&state.db, &state.encryption_keys, &actor.id)
        .await?
        .into_iter()
        .map(|e| super::catalog::catalog_entry_response(&state.config, e))
        .collect();
    service::live(&state.db, state.auth_device_hmac_key.as_slice(), &row).await?;
    Ok(private(Inventory {
        options,
        catalog: super::catalog::CatalogListResponse { entries },
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBody {
    selection: Option<agent::Selection>,
    credential_expires_at: Option<String>,
}
#[derive(Serialize)]
pub struct Decision {
    ok: bool,
}
pub async fn approve(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<DecisionBody>,
) -> AppResult<Private<Decision>> {
    origin(&state, &headers)?;
    let row = load(&state, &headers, &id).await?;
    if !row.verified {
        return Err(service::invalid());
    }
    let actor = service::user(&state.db, &row).await?;
    let ip = limit(&state, &headers, addr, Some(&actor.id)).await?;
    let ua = extract_user_agent(&headers);
    let expiry = body
        .credential_expires_at
        .as_deref()
        .map(api_key_validation::parse_expires_at)
        .transpose()?;
    if body.selection.is_none() && (row.flow == LoginFlow::AgentKey || expiry.is_some()) {
        return Err(AppError::ValidationError(
            "An Agent Key selection is required.".into(),
        ));
    }
    service::close(&state.db, &row).await?;
    let result = match row.flow {
        LoginFlow::AgentKey => {
            agent::approve(
                &state.db,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                &actor.id,
                &row.user_code,
                body.selection.unwrap(),
                expiry,
                Some(&ip),
                ua.as_deref(),
            )
            .await
        }
        LoginFlow::Device => {
            let input = device::ApproveInput {
                user_id: actor.id,
                user_code: row.user_code.clone(),
                approver_ip: Some(ip),
                approver_user_agent: ua,
            };
            if let Some(selection) = body.selection {
                device::approve_with_agent_key(
                    &state.db,
                    &state.encryption_keys,
                    state.auth_device_hmac_key.as_slice(),
                    input,
                    selection,
                    expiry,
                )
                .await
            } else {
                device::approve(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    &state.encryption_keys,
                    state.auth_device_hmac_key.as_slice(),
                    input,
                )
                .await
            }
        }
    };
    if let Err(error) = result {
        service::retry_decision(&state.db, &row).await?;
        return Err(error);
    }
    Ok(private(Decision { ok: true }))
}
pub async fn deny(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Private<Decision>> {
    origin(&state, &headers)?;
    let row = load(&state, &headers, &id).await?;
    if !row.verified {
        return Err(service::invalid());
    }
    let actor = service::user(&state.db, &row).await?;
    let ip = limit(&state, &headers, addr, Some(&actor.id)).await?;
    let ua = extract_user_agent(&headers);
    service::close(&state.db, &row).await?;
    let result = match row.flow {
        LoginFlow::AgentKey => {
            agent::deny(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                &actor.id,
                &row.user_code,
                Some(&ip),
                ua.as_deref(),
            )
            .await
        }
        LoginFlow::Device => {
            device::deny(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                device::DenyInput {
                    user_id: actor.id,
                    user_code: row.user_code.clone(),
                    denier_ip: Some(ip),
                    denier_user_agent: ua,
                },
            )
            .await
        }
    };
    if let Err(error) = result {
        service::retry_decision(&state.db, &row).await?;
        return Err(error);
    }
    Ok(private(Decision { ok: true }))
}
pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Private<Decision>> {
    origin(&state, &headers)?;
    let row = load(&state, &headers, &id).await?;
    service::close(&state.db, &row).await?;
    Ok(private(Decision { ok: true }))
}

pub(super) fn social_id(raw: &str) -> Option<&str> {
    raw.strip_prefix("approval:")
        .and_then(|s| s.split_once(':'))
        .map(|(id, _)| id)
}
pub(super) async fn social_complete(
    state: &AppState,
    raw: &str,
    actor: &User,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> AppResult<(StatusCode, HeaderMap, ())> {
    let id = social_id(raw).ok_or_else(service::invalid)?;
    let row =
        service::consume_social(&state.db, state.auth_device_hmac_key.as_slice(), id, raw).await?;
    let row = service::identify(&state.db, &row, actor).await?;
    let (mut headers, _) = complete(state, &row, false, headers, addr).await?;
    // Identity and scope hints remain in tab-local state. The callback carries no credential.
    let mut url = url::Url::parse(&state.config.frontend_url).map_err(|_| service::invalid())?;
    url.set_path(&format!(
        "/login/{}",
        match row.flow {
            LoginFlow::Device => "device",
            LoginFlow::AgentKey => "agent-key",
        }
    ));
    url.query_pairs_mut()
        .append_pair("user_code", &row.user_code);
    headers.insert(
        header::LOCATION,
        url.as_str().parse().map_err(|_| service::invalid())?,
    );
    Ok((StatusCode::FOUND, headers, ()))
}
