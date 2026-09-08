use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::channel_bots::{
    CreateChannelBotResponse, resolve_adapter, resolve_bot_owner_for_write, resolve_create_owner,
};
use crate::services::{
    audit_service, channel_bot_service, channel_managed, platform_credential_service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
};

#[derive(Debug, Serialize)]
pub struct BootstrapResponse {
    pub available: bool,
    pub flow: Option<&'static str>,
    pub provider_slug: Option<&'static str>,
    pub required_scopes: &'static [&'static str],
    pub authorize_start_url: Option<String>,
    #[serde(flatten)]
    pub fields: BTreeMap<String, String>,
    pub graph_version: Option<&'static str>,
    pub signup_version: Option<&'static str>,
    pub signup_extras: BTreeMap<String, serde_json::Value>,
    pub feature_types: &'static [&'static str],
}

#[derive(Deserialize)]
pub struct CompleteRequest {
    pub label: String,
    pub target_org_id: Option<String>,
    #[serde(flatten)]
    pub input: BTreeMap<String, Zeroizing<String>>,
}

impl std::fmt::Debug for CompleteRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CompleteRequest([REDACTED])")
    }
}

async fn limit(state: &AppState, auth: &AuthUser) -> AppResult<()> {
    let limiter = crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "channel_managed_onboarding",
        5,
        60,
    );
    if !limiter.check_shared(&auth.user_id.to_string()).await? {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

pub async fn bootstrap(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(platform): Path<String>,
) -> AppResult<(HeaderMap, Json<BootstrapResponse>)> {
    let adapter = resolve_adapter(&platform, &state.token_exchange_cache)?;
    let mut response = BootstrapResponse {
        available: false,
        flow: None,
        provider_slug: None,
        required_scopes: &[],
        authorize_start_url: None,
        fields: BTreeMap::new(),
        graph_version: None,
        signup_version: None,
        signup_extras: BTreeMap::new(),
        feature_types: &[],
    };
    if let (Some(managed), Some(credentials)) =
        (adapter.managed_onboarding(), adapter.platform_credentials())
    {
        response.flow = Some(managed.flow);
        let row = platform_credential_service::load(&state.db, managed.provider).await?;
        response.available = platform_credential_service::configured(row.as_ref(), &credentials);
        if let crate::services::channel_platform::CredentialResolution::OAuthConnection {
            provider_slug,
            required_scopes,
        } = adapter.credential_resolution()
        {
            response.provider_slug = Some(provider_slug);
            response.required_scopes = required_scopes;
            let provider = state
                .db
                .collection::<crate::models::provider_config::ProviderConfig>(
                    crate::models::provider_config::COLLECTION_NAME,
                )
                .find_one(bson::doc! { "slug": provider_slug, "is_active": true })
                .await?;
            response.available &= provider.is_some();
            response.authorize_start_url =
                provider.map(|_| format!("/channel-bots/managed-onboarding/{platform}/start"));
        }
        if response.available {
            for field in managed.bootstrap_fields {
                if credentials
                    .fields
                    .iter()
                    .any(|f| f.name == *field && !f.secret)
                    && let Some(value) = row.as_ref().and_then(|row| row.fields.get(*field))
                {
                    response.fields.insert(field.to_string(), value.clone());
                }
            }
            response.graph_version = Some(managed.graph_version);
            response.signup_version = Some(managed.signup_version);
            response.signup_extras = managed
                .feature_types
                .iter()
                .map(|feature| (feature.to_string(), (managed.signup_extras)(feature)))
                .collect();
            response.feature_types = managed.feature_types;
        }
    }
    Ok((
        HeaderMap::from_iter([(
            header::CACHE_CONTROL,
            "no-store".parse().expect("static header"),
        )]),
        Json(response),
    ))
}

pub async fn complete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CompleteRequest>,
) -> AppResult<axum::response::Response> {
    use axum::response::{IntoResponse, Sse, sse::Event};
    limit(&state, &auth).await?;
    if headers
        .get(header::ACCEPT)
        .is_some_and(|v| v == "text/event-stream")
    {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let progress = channel_managed::ManagedProgress(Some(sender.clone()));
        tokio::spawn(async move {
            let event = match complete_inner(&state, &auth, &platform, body, &progress).await {
                Ok((_, Json(result))) => serde_json::json!({ "result": result }),
                Err(error) => {
                    serde_json::json!(error.response_body())
                }
            };
            let _ = sender.send(event);
        });
        let stream = futures::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|value| {
                (
                    Ok::<_, std::convert::Infallible>(Event::default().data(value.to_string())),
                    receiver,
                )
            })
        });
        return Ok((
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::HeaderName::from_static("x-accel-buffering"), "no"),
            ],
            Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()),
        )
            .into_response());
    }
    Ok(complete_inner(
        &state,
        &auth,
        &platform,
        body,
        &channel_managed::ManagedProgress::default(),
    )
    .await?
    .into_response())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub label: String,
    pub target_org_id: Option<String>,
}

#[derive(Serialize)]
pub struct StartResponse {
    pub connection_id: String,
    pub authorization_url: String,
    pub attempt_nonce: Option<String>,
}

impl std::fmt::Debug for StartResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StartResponse([REDACTED])")
    }
}

pub async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(body): Json<StartRequest>,
) -> AppResult<(HeaderMap, Json<StartResponse>)> {
    limit(&state, &auth).await?;
    let actor = auth.user_id.to_string();
    let owner = resolve_create_owner(&state, &actor, body.target_org_id.as_deref()).await?;
    let adapter = resolve_adapter(&platform, &state.token_exchange_cache)?;
    let started = crate::services::channel_credentials::start_connection(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        adapter.as_ref(),
        &actor,
        &owner,
        body.label.trim(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_bot_oauth_started",
        Some(
            serde_json::json!({ "platform": platform, "owner_user_id": owner, "connection_id": started.connection_id }),
        ),
    );
    Ok((
        HeaderMap::from_iter([(
            header::CACHE_CONTROL,
            "no-store".parse().expect("static header"),
        )]),
        Json(StartResponse {
            connection_id: started.connection_id,
            authorization_url: started.authorization_url,
            attempt_nonce: started.attempt_nonce,
        }),
    ))
}

pub(crate) async fn complete_inner(
    state: &AppState,
    auth: &AuthUser,
    platform: &str,
    body: CompleteRequest,
    progress: &channel_managed::ManagedProgress,
) -> AppResult<(StatusCode, Json<CreateChannelBotResponse>)> {
    let adapter = resolve_adapter(platform, &state.token_exchange_cache)?;
    let descriptor = adapter
        .managed_onboarding()
        .ok_or_else(channel_managed::unavailable)?;
    if body
        .input
        .keys()
        .any(|field| !descriptor.completion_fields.contains(&field.as_str()))
    {
        return Err(AppError::ValidationError(
            "Unknown managed onboarding field".to_string(),
        ));
    }
    let owner = resolve_create_owner(
        state,
        &auth.user_id.to_string(),
        body.target_org_id.as_deref(),
    )
    .await?;
    let created = channel_bot_service::create_managed_bot(
        &state.db,
        &state.config,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &owner,
        body.label.trim(),
        &channel_managed::ManagedOnboardingInput(body.input),
        progress,
    )
    .await?;
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{platform}/{}",
        state.config.base_url, created.bot.id
    );
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        "channel_bot_created",
        Some(
            serde_json::json!({ "bot_id": created.bot.id, "platform": platform, "credential_source": created.bot.credential_source, "owner_user_id": owner }),
        ),
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateChannelBotResponse::from_bot(
            created.bot,
            adapter.registration(),
            webhook_url,
            created.webhook_secret,
        )?),
    ))
}

pub async fn reregister(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    limit(&state, &auth).await?;
    let (_, bot) = resolve_bot_owner_for_write(&state, &auth.user_id.to_string(), &id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    channel_bot_service::reregister_managed_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &bot,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_bot_number_reregistered",
        Some(serde_json::json!({ "bot_id": id, "platform": bot.platform })),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconnectRequest {
    pub connection_id: String,
}

pub async fn reconnect(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ReconnectRequest>,
) -> AppResult<StatusCode> {
    limit(&state, &auth).await?;
    let (_, bot) = resolve_bot_owner_for_write(&state, &auth.user_id.to_string(), &id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    channel_bot_service::reconnect_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &bot,
        &body.connection_id,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_bot_reconnected",
        Some(serde_json::json!({ "bot_id": id, "connection_id": body.connection_id })),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn repair(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<super::channel_bots::ManagedSetupResponse>> {
    limit(&state, &auth).await?;
    let (_, bot) = resolve_bot_owner_for_write(&state, &auth.user_id.to_string(), &id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let setup = channel_bot_service::repair_managed_bot(
        &state.db,
        &state.config,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &bot,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_bot_managed_setup_repaired",
        Some(
            serde_json::json!({ "bot_id": id, "platform": bot.platform, "setup": super::channel_bots::ManagedSetupResponse::from(&setup) }),
        ),
    );
    Ok(Json((&setup).into()))
}
