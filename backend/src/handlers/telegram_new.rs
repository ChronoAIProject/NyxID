use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use crate::models::telegram_bot_request::{TelegramBotRequest, TelegramRequestStatus as Status};
use crate::services::{
    audit_service, telegram_new_api::TelegramApi, telegram_new_service::TelegramNewService,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
};

// Chat delivery must not occupy Telegram's single manager webhook connection.
static MANAGER_CHANNEL_DELIVERIES: Semaphore = Semaphore::const_new(32);

pub(crate) fn service(state: &AppState) -> TelegramNewService<'_> {
    TelegramNewService {
        db: &state.db,
        keys: &state.encryption_keys,
        config: &state.config,
        api: TelegramApi::new(&state.http_client),
    }
}

fn human(auth: &AuthUser) -> AppResult<String> {
    if !matches!(
        auth.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken
    ) || auth.acting_client_id.is_some()
    {
        return Err(AppError::Forbidden(
            "Telegram creation requires a human account".into(),
        ));
    }
    Ok(auth.user_id.to_string())
}

async fn limit(state: &AppState, actor: &str) -> AppResult<()> {
    limit_bucket(state, actor, "telegram_new_creation", 5).await
}

async fn limit_bucket(state: &AppState, actor: &str, bucket: &str, requests: u32) -> AppResult<()> {
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(state.db.clone(), bucket, requests, 60)
        .check_shared(actor)
        .await?
    {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

fn private_headers() -> HeaderMap {
    HeaderMap::from_iter([
        (header::CACHE_CONTROL, "no-store".parse().unwrap()),
        (header::REFERRER_POLICY, "no-referrer".parse().unwrap()),
    ])
}

#[derive(Serialize)]
pub struct RequestResponse {
    pub id: String,
    pub status: Status,
    pub revision: i64,
    pub label: String,
    pub owner_user_id: String,
    pub expires_at: String,
    pub telegram_bot_id: Option<String>,
    pub bot_username: Option<String>,
    pub channel_bot_id: Option<String>,
    pub auto_connect: bool,
    pub connection_error: Option<String>,
}

impl From<TelegramBotRequest> for RequestResponse {
    fn from(request: TelegramBotRequest) -> Self {
        let approved = matches!(
            request.status,
            Status::Ready | Status::Provisioning | Status::Connected | Status::Suspended
        );
        Self {
            auto_connect: request.auto_connect,
            connection_error: request.connection_error,
            channel_bot_id: matches!(
                request.status,
                Status::Provisioning | Status::Connected | Status::Suspended
            )
            .then(|| request.id.clone()),
            id: request.id,
            status: request.status,
            revision: request.revision,
            label: request.label,
            owner_user_id: request.owner_user_id,
            expires_at: request.expires_at.to_rfc3339(),
            telegram_bot_id: request
                .telegram_bot_id
                .filter(|_| approved)
                .map(|id| id.to_string()),
            bot_username: request.bot_username.filter(|_| approved),
        }
    }
}

#[derive(Serialize)]
pub struct ConfigurationResponse {
    pub available: bool,
    pub manager_username: Option<String>,
    pub request: Option<RequestResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginRequest {
    pub label: String,
    pub target_org_id: Option<String>,
    #[serde(default)]
    pub auto_connect: bool,
}

#[derive(Deserialize)]
pub struct ConfigurationQuery {
    pub request_id: Option<uuid::Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmRequest {
    pub telegram_bot_id: String,
    pub revision: i64,
}

#[derive(Serialize)]
pub struct LaunchResponse {
    pub request: RequestResponse,
    pub launch_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimCodeRequest {
    pub code: Zeroizing<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemClaimRequest {
    pub code: Zeroizing<String>,
    pub label: String,
    pub target_org_id: Option<String>,
}

#[derive(Serialize)]
pub struct ClaimPreviewResponse {
    pub bot_username: String,
    pub expires_at: String,
}

pub async fn preview_claim(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ClaimCodeRequest>,
) -> AppResult<(HeaderMap, Json<ClaimPreviewResponse>)> {
    let actor = human(&auth)?;
    limit_bucket(&state, &actor, "telegram_new_claim_preview", 30).await?;
    let claim = service(&state).preview_claim(&actor, &body.code).await?;
    Ok((
        private_headers(),
        Json(ClaimPreviewResponse {
            bot_username: claim.bot_username,
            expires_at: claim.expires_at.to_rfc3339(),
        }),
    ))
}

pub async fn redeem_claim(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<RedeemClaimRequest>,
) -> AppResult<(StatusCode, HeaderMap, Json<RequestResponse>)> {
    let actor = human(&auth)?;
    limit(&state, &actor).await?;
    let owner =
        super::channel_bots::resolve_create_owner(&state, &actor, body.target_org_id.as_deref())
            .await?;
    let request = service(&state)
        .redeem_claim(&actor, &owner, &body.code, &body.label)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        private_headers(),
        Json(request.into()),
    ))
}

pub async fn configuration(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ConfigurationQuery>,
) -> AppResult<(HeaderMap, Json<ConfigurationResponse>)> {
    let actor = human(&auth)?;
    let row = crate::services::platform_credential_service::load(
        &state.db,
        &crate::services::channel_adapters::telegram_new::credential_descriptor(),
    )
    .await?;
    let available = row.as_ref().is_some_and(|row| {
        row.fields.get("webhook_ready").is_some_and(|v| v == "true")
            && row.secrets.contains_key("manager_bot_token")
    });
    let request = match query.request_id {
        Some(id) => Some(service(&state).get(&actor, &id.to_string()).await?),
        None => service(&state).current(&actor).await?,
    };
    Ok((
        private_headers(),
        Json(ConfigurationResponse {
            available,
            manager_username: row.and_then(|row| row.fields.get("manager_username").cloned()),
            request: request.map(Into::into),
        }),
    ))
}

pub async fn begin(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<BeginRequest>,
) -> AppResult<(HeaderMap, Json<LaunchResponse>)> {
    let actor = human(&auth)?;
    limit(&state, &actor).await?;
    let owner =
        super::channel_bots::resolve_create_owner(&state, &actor, body.target_org_id.as_deref())
            .await?;
    let (request, launch_url) = service(&state)
        .begin(&actor, &owner, &body.label, body.auto_connect)
        .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "telegram_bot_creation_started",
        Some(serde_json::json!({"request_id": request.id, "owner_user_id": owner})),
    );
    Ok((
        private_headers(),
        Json(LaunchResponse {
            request: request.into(),
            launch_url,
        }),
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(HeaderMap, Json<RequestResponse>)> {
    let actor = human(&auth)?;
    Ok((
        private_headers(),
        Json(service(&state).get(&actor, &id).await?.into()),
    ))
}

pub async fn launch(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(HeaderMap, Json<LaunchResponse>)> {
    let actor = human(&auth)?;
    limit(&state, &actor).await?;
    let launch_url = service(&state).launch(&actor, &id).await?;
    Ok((
        private_headers(),
        Json(LaunchResponse {
            request: service(&state).get(&actor, &id).await?.into(),
            launch_url,
        }),
    ))
}

pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    service(&state).cancel(&human(&auth)?, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn connect(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ConfirmRequest>,
) -> AppResult<(HeaderMap, Json<RequestResponse>)> {
    let actor = human(&auth)?;
    limit(&state, &actor).await?;
    let bot_id = body
        .telegram_bot_id
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| AppError::ValidationError("Invalid Telegram bot ID".into()))?;
    let bot = service(&state)
        .connect(&actor, &id, bot_id, body.revision)
        .await?;
    let request = service(&state).get(&actor, &id).await?;
    if !request.auto_connect {
        audit_service::log_for_user(
            state.db.clone(),
            &auth,
            "channel_bot_created",
            Some(
                serde_json::json!({"bot_id": bot.id, "platform": bot.platform, "owner_user_id": bot.user_id}),
            ),
        );
    }
    Ok((private_headers(), Json(request.into())))
}

pub async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<StatusCode> {
    webhook_with_service(&state, &service(&state), &headers, &body).await
}

pub(crate) async fn webhook_with_service(
    state: &AppState,
    service: &TelegramNewService<'_>,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<StatusCode> {
    if let Some((bot, update)) = service.webhook(headers, body).await? {
        relay_manager_channel(state.clone(), bot, update).await;
    }
    Ok(StatusCode::OK)
}

pub(crate) async fn relay_manager_channel(
    state: AppState,
    bot: crate::models::channel_bot::ChannelBot,
    update: serde_json::Value,
) {
    let prepared: AppResult<_> = async {
        let adapter =
            super::channel_bots::resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
        let channel_body = serde_json::to_vec(&update).map_err(|_| {
            AppError::Internal("Unable to serialize Telegram channel update".into())
        })?;
        let messages = adapter.parse_inbound(&channel_body).await?;
        Ok((adapter, messages))
    }
    .await;
    let (adapter, messages) = match prepared {
        Ok(prepared) => prepared,
        Err(_) => {
            tracing::warn!(bot_id = %bot.id, "Telegram manager channel parsing failed; ordinary update dropped");
            return;
        }
    };
    if messages.is_empty() {
        return;
    }
    let Ok(permit) = MANAGER_CHANNEL_DELIVERIES.try_acquire() else {
        tracing::warn!(bot_id = %bot.id, "Telegram manager channel delivery capacity reached; message dropped");
        return;
    };
    let deadline = std::time::Duration::from_secs(
        u64::from(state.config.channel_relay_callback_timeout_secs) + 10,
    );
    tokio::spawn(async move {
        let _permit = permit;
        match tokio::time::timeout(
            deadline,
            crate::services::channel_inbound_service::process_inbound_messages(
                (&state).into(),
                &bot,
                adapter.as_ref(),
                &messages,
            ),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => {
                tracing::warn!(bot_id = %bot.id, "Telegram manager channel relay failed");
            }
            Err(_) => {
                tracing::warn!(bot_id = %bot.id, "Telegram manager channel delivery deadline exceeded");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_new_requires_a_person_session_or_access_token() {
        let mut auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        for allowed in [AuthMethod::Session, AuthMethod::AccessToken] {
            auth.auth_method = allowed;
            assert!(human(&auth).is_ok());
        }
        for rejected in [
            AuthMethod::ApiKey,
            AuthMethod::ServiceAccount,
            AuthMethod::Delegated,
            AuthMethod::Relay,
        ] {
            auth.auth_method = rejected;
            assert!(human(&auth).is_err());
        }
        auth.auth_method = AuthMethod::AccessToken;
        auth.acting_client_id = Some("delegating-client".into());
        assert!(human(&auth).is_err());
    }
}
