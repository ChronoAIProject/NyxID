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
        fields: BTreeMap::new(),
        graph_version: None,
        signup_version: None,
        signup_extras: BTreeMap::new(),
        feature_types: &[],
    };
    if let (Some(managed), Some(credentials)) =
        (adapter.managed_onboarding(), adapter.platform_credentials())
    {
        let row = platform_credential_service::load(&state.db, managed.provider).await?;
        response.available = platform_credential_service::configured(row.as_ref(), &credentials);
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
            serde_json::json!({ "bot_id": created.bot.id, "platform": platform, "credential_source": "platform", "owner_user_id": owner }),
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
