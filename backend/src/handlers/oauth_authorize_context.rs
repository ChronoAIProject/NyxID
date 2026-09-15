use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use crate::services::{
    oauth_authorize_context_service as contexts, oauth_branding_service as branding,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::OptionalAuthUser,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextQuery {
    pub ctx: String,
}

#[derive(Serialize)]
pub struct ContextResponse {
    pub client_name: String,
    pub handoff_blurb: Option<String>,
    pub logo_url: Option<String>,
    pub homepage_url: Option<String>,
    pub verified: bool,
    pub destination: String,
}

async fn limit(state: &AppState, headers: &HeaderMap, addr: SocketAddr) -> AppResult<()> {
    let ip = crate::mw::rate_limit::resolve_client_ip_for_rate_limit(
        headers,
        Some(addr),
        &state.config.trusted_proxy_ips,
    )
    .unwrap_or(addr.ip());
    if !state.authorize_context_limiter.check_shared(ip).await? {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

/// Public branding requires a live signed context, never an unvalidated client ID.
pub async fn get(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ContextQuery>,
) -> AppResult<Response> {
    limit(&state, &headers, addr).await?;
    let (context, client) = contexts::load(&state, &query.ctx).await?;
    let body = ContextResponse {
        logo_url: branding::logo_url(&client),
        verified: branding::is_verified(&client),
        destination: branding::destination(&client, &context.authorize_params.redirect_uri)?,
        client_name: client.client_name,
        handoff_blurb: client.handoff_blurb,
        homepage_url: client.homepage_url,
    };
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Json(body),
    )
        .into_response())
}

/// AuthFlow returns to this same-origin URL after password, MFA, social or device login.
/// Only the persisted validated request is resumed; query parameters cannot replace it.
pub async fn resume(
    State(state): State<AppState>,
    auth: OptionalAuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ContextQuery>,
) -> AppResult<Response> {
    limit(&state, &headers, addr).await?;
    let (context, _) = contexts::load(&state, &query.ctx).await?;
    let Some(auth) = auth.0 else {
        return Ok(super::oauth::redirect_302(&format!(
            "{}/connect/app/start/{}",
            state.config.frontend_url.trim_end_matches('/'),
            query.ctx
        )));
    };
    super::app_connect_links::require_human(&state, &auth).await?;
    if !contexts::login_completed(
        &state.db,
        &context,
        &auth.user_id.to_string(),
        auth.session_id.as_ref().map(|id| id.to_string()).as_deref(),
    )
    .await?
    {
        return Ok(super::oauth::redirect_302(&format!(
            "{}/connect/app/start/{}",
            state.config.frontend_url.trim_end_matches('/'),
            query.ctx
        )));
    }
    let mut params = super::oauth::params_from_validated(&context.authorize_params);
    // Interactive login has completed. Preserve consent/other prompts without
    // sending prompt=login back through the unauthenticated branch indefinitely.
    params.prompt = params
        .prompt
        .map(|p| {
            p.split_whitespace()
                .filter(|p| *p != "login")
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|p| !p.is_empty());
    contexts::consume(&state.db, &context.id).await?;
    super::oauth::authorize_inner(
        &state,
        OptionalAuthUser(Some(auth)),
        &params,
        true,
        context.authorize_params.external_subject.as_ref(),
    )
    .await
}
