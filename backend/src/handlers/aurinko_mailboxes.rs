//! Browser-only managed mailbox authorization; ordinary APIs never return tokens.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthUser, OptionalAuthUser},
    services::aurinko_oauth_service as service,
};
use axum::{
    Json,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct ListQuery {
    pub owner_id: Option<String>,
}
#[derive(Serialize)]
pub struct MailboxResponse {
    connection_id: String,
    service_id: String,
    label: String,
    service_type: Option<String>,
    mailbox_address: Option<String>,
    status: String,
    is_active: bool,
    managed_onboarding: &'static str,
}
#[derive(Serialize)]
pub struct MailboxesResponse {
    mailboxes: Vec<MailboxResponse>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<MailboxesResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let rows = service::list_connections(
        &state.db,
        &actor,
        query.owner_id.as_deref().unwrap_or(&actor),
    )
    .await?;
    Ok(Json(MailboxesResponse {
        mailboxes: rows
            .into_iter()
            .map(|(service, key)| MailboxResponse {
                connection_id: key.id,
                service_id: service.id,
                label: key.label,
                service_type: key.aurinko_account.as_ref().map(|a| a.service_type.clone()),
                mailbox_address: key.aurinko_account.map(|a| a.mailbox_address),
                status: key.status,
                is_active: service.is_active,
                managed_onboarding: service::PROTOCOL,
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeRequest {
    pub provider: service::MailProvider,
    pub label: String,
    pub slug: Option<String>,
    pub owner_id: Option<String>,
    /// Existing UserApiKey UUID for reconnect or a pending AI Service.
    pub connection_id: Option<String>,
}
#[derive(Serialize)]
pub struct AuthorizeResponse {
    pub connection_id: String,
    pub service_id: String,
    pub authorization_url: String,
    pub attempt_nonce: String,
}
pub(crate) fn session(auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    auth.session_id.map(|id| id.to_string()).ok_or_else(|| {
        AppError::Unauthorized("Sign in with a browser session to connect a mailbox".into())
    })
}
fn cookie_name(state: &str) -> String {
    format!("__Secure-nyx_aurinko_{}", state.replace('-', ""))
}
pub(crate) fn cookies(state: &str, nonce: &str, age: u32) -> AppResult<HeaderMap> {
    let cookie = format!(
        "{}={nonce}; Path={}; Max-Age={age}; Secure; HttpOnly; SameSite=Lax",
        cookie_name(state),
        service::CALLBACK_PATH
    );
    Ok(HeaderMap::from_iter([
        (
            header::SET_COOKIE,
            cookie
                .parse()
                .map_err(|_| AppError::Internal("Invalid authorization cookie".into()))?,
        ),
        (header::CACHE_CONTROL, "no-store".parse().expect("static")),
        (
            header::REFERRER_POLICY,
            "no-referrer".parse().expect("static"),
        ),
    ]))
}
pub async fn authorize(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<AuthorizeRequest>,
) -> AppResult<(HeaderMap, Json<AuthorizeResponse>)> {
    let session_id = session(&auth)?;
    let actor = auth.user_id.to_string();
    let limiter = crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "aurinko_authorization",
        5,
        60,
    );
    if !limiter.check_shared(&actor).await? {
        return Err(AppError::RateLimited);
    }
    let started = service::start(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        &actor,
        &session_id,
        body.owner_id.as_deref().unwrap_or(&actor),
        body.label.trim(),
        body.provider,
        body.connection_id.as_deref(),
        None,
        body.slug.as_deref(),
    )
    .await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "aurinko_mailbox_authorization_started",
        Some(
            serde_json::json!({"connection_id":started.key_id,"service_type":body.provider.service_type()}),
        ),
    );
    Ok((
        cookies(&started.state_id, &started.browser_nonce, 600)?,
        Json(AuthorizeResponse {
            connection_id: started.key_id,
            service_id: started.service_id,
            authorization_url: started.authorization_url,
            attempt_nonce: crate::services::user_token_service::chat_attempt_nonce_from_state(
                &started.state_id,
            )
            .expect("Aurinko popup state")
            .to_string(),
        }),
    ))
}
#[derive(Deserialize)]
pub struct CallbackQuery {
    pub state: Option<String>,
    pub code: Option<String>,
    pub status: Option<String>,
    pub error: Option<String>,
}
pub async fn callback(
    State(state): State<AppState>,
    auth: OptionalAuthUser,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(state_id) = query.state.as_deref().filter(|s| {
        crate::services::user_token_service::chat_attempt_nonce_from_state(s).is_some()
    }) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid mailbox authorization state",
        )
            .into_response();
    };
    let cookie_name = cookie_name(state_id);
    let nonce = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .find_map(|pair| {
            pair.trim()
                .split_once('=')
                .filter(|(name, _)| *name == cookie_name)
                .map(|(_, value)| value)
        })
        .unwrap_or("");
    let mut hosted_link = None;
    let mut bound_callback = false;
    let result = async {
        let auth = auth.0.as_ref().ok_or_else(|| {
            AppError::Unauthorized("The original browser session is required".into())
        })?;
        let session_id = session(auth)?;
        hosted_link = service::bound_connect_link(
            &state.db,
            state_id,
            &auth.user_id.to_string(),
            &session_id,
            nonce,
        )
        .await?;
        bound_callback = true;
        service::complete(
            &state.db,
            &state.encryption_keys,
            service::CallbackInput {
                state_id,
                browser_nonce: nonce,
                actor: &auth.user_id.to_string(),
                session_id: &session_id,
                code: query.code.as_deref(),
                error: query.error.as_deref(),
                status: query.status.as_deref(),
            },
        )
        .await
    }
    .await;
    if bound_callback && let Some(auth) = auth.0.as_ref() {
        crate::services::audit_service::log_for_user(
            state.db.clone(),
            auth,
            if result.is_ok() {
                "aurinko_mailbox_authorization_completed"
            } else {
                "aurinko_mailbox_authorization_failed"
            },
            Some(serde_json::json!({"provider":"aurinko"})),
        );
    }
    let error_code = if !bound_callback {
        "state_invalid"
    } else if query.error.as_deref() == Some("access_denied")
        || matches!(query.status.as_deref(), Some("cancel" | "cancelled"))
    {
        "access_denied"
    } else {
        "provider_error"
    };
    if let Some((link_id, owner)) = hosted_link {
        if result.is_err() {
            let _ = crate::services::connect_link_service::record_provider_error(
                &state.db,
                &link_id,
                &owner,
                "provider_error",
            )
            .await;
        }
        crate::services::connect_link_service::dispatch_terminal_webhook_if_needed(
            &state.db,
            &state.developer_webhook_dispatcher,
            &link_id,
        )
        .await;
        let redirect = Redirect::to(&format!(
            "{}/connect/return/{}",
            state.config.frontend_url.trim_end_matches('/'),
            link_id
        ));
        return (cookies(state_id, "", 0).unwrap_or_default(), redirect).into_response();
    }
    let status = if result.is_ok() { "complete" } else { "error" };
    let nonce = crate::services::user_token_service::chat_attempt_nonce_from_state(state_id);
    let redirect = super::user_tokens::redirect_to_oauth_completion(
        state.config.frontend_url.trim_end_matches('/'),
        status,
        "cc",
        result.as_ref().err().map(|_| error_code),
        nonce,
    );
    (cookies(state_id, "", 0).unwrap_or_default(), redirect).into_response()
}
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(attempt): Path<String>,
) -> AppResult<(HeaderMap, axum::http::StatusCode)> {
    let session_id = session(&auth)?;
    service::cancel(&state.db, &auth.user_id.to_string(), &session_id, &attempt).await?;
    Ok((
        cookies(&format!("1cc_{attempt}"), "", 0)?,
        axum::http::StatusCode::NO_CONTENT,
    ))
}
/// Operator-owned intermediate redirect for Google/Zoho upstream app setup.
/// These parameters belong to Aurinko, not NyxID's final authorization state.
pub async fn intermediate(RawQuery(raw): RawQuery) -> AppResult<Response> {
    let raw = raw.unwrap_or_default();
    if raw.len() > 16384 {
        return Err(AppError::BadRequest("Invalid provider callback".into()));
    }
    let pairs: Vec<_> = url::form_urlencoded::parse(raw.as_bytes()).collect();
    let mut target = url::Url::parse("https://api.aurinko.io/v1/auth/callback").expect("fixed URL");
    if pairs.len() > 64
        || !pairs
            .iter()
            .any(|(key, value)| key == "state" && !value.is_empty())
    {
        return Err(AppError::BadRequest("Invalid provider callback".into()));
    }
    for (key, value) in pairs {
        if key.is_empty()
            || key.len() > 128
            || value.len() > 8192
            || key.chars().chain(value.chars()).any(char::is_control)
        {
            return Err(AppError::BadRequest("Invalid provider callback".into()));
        }
        target.query_pairs_mut().append_pair(&key, &value);
    }
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Redirect::to(target.as_str()),
    )
        .into_response())
}
