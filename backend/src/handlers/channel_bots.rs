use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::channel_adapters::discord::DiscordAdapter;
use crate::services::channel_adapters::lark::LarkFamilyAdapter;
use crate::services::channel_adapters::openclaw::OpenClawAdapter;
use crate::services::channel_adapters::telegram::TelegramAdapter;
use crate::services::channel_platform::PlatformAdapter;
use crate::services::{audit_service, channel_bot_service};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateChannelBotRequest {
    pub platform: String,
    pub bot_token: String,
    pub label: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
}

// Redact bot_token in Debug output to prevent credential leakage.
impl std::fmt::Display for CreateChannelBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CreateChannelBotRequest {{ platform: {}, label: {}, bot_token: [REDACTED] }}",
            self.platform, self.label
        )
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ChannelBotItem {
    pub id: String,
    pub platform: String,
    pub label: String,
    pub platform_bot_username: String,
    pub webhook_registered: bool,
    pub status: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ChannelBotListResponse {
    pub bots: Vec<ChannelBotItem>,
    pub total: u64,
}

#[derive(Debug, Serialize)]
pub struct ChannelBotDetailResponse {
    pub id: String,
    pub platform: String,
    pub label: String,
    pub platform_bot_id: String,
    pub platform_bot_username: String,
    pub webhook_registered: bool,
    pub status: String,
    pub is_active: bool,
    pub conversations_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct CreateChannelBotResponse {
    pub id: String,
    pub platform: String,
    pub platform_bot_username: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyBotResponse {
    pub id: String,
    pub status: String,
    pub webhook_registered: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the platform adapter for the given platform identifier.
///
/// Supported platforms: telegram, discord, lark, feishu, openclaw.
///
/// The Lark and Feishu adapters share a process-wide
/// [`TokenExchangeCache`] that is also used by the proxy's
/// `token_exchange` auth method. Callers pass the cache from `AppState`
/// so both code paths deduplicate token exchanges for the same Lark app.
pub fn resolve_adapter(
    platform: &str,
    token_exchange_cache: &std::sync::Arc<
        crate::services::provider_token_exchange_service::TokenExchangeCache,
    >,
) -> AppResult<Box<dyn PlatformAdapter>> {
    match platform {
        "telegram" => Ok(Box::new(TelegramAdapter)),
        "discord" => Ok(Box::new(DiscordAdapter)),
        "lark" => Ok(Box::new(LarkFamilyAdapter::lark(
            token_exchange_cache.clone(),
        ))),
        "feishu" => Ok(Box::new(LarkFamilyAdapter::feishu(
            token_exchange_cache.clone(),
        ))),
        "openclaw" => Ok(Box::new(OpenClawAdapter)),
        other => Err(AppError::ValidationError(format!(
            "unsupported platform: {other}. Supported: telegram, discord, lark, feishu, openclaw"
        ))),
    }
}

fn bot_to_item(bot: &crate::models::channel_bot::ChannelBot) -> ChannelBotItem {
    ChannelBotItem {
        id: bot.id.clone(),
        platform: bot.platform.clone(),
        label: bot.label.clone(),
        platform_bot_username: bot.platform_bot_username.clone(),
        webhook_registered: bot.webhook_registered,
        status: bot.status.clone(),
        is_active: bot.is_active,
        created_at: bot.created_at.to_rfc3339(),
        updated_at: bot.updated_at.to_rfc3339(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/channel-bots
pub async fn create_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateChannelBotRequest>,
) -> AppResult<(StatusCode, Json<CreateChannelBotResponse>)> {
    let user_id_str = auth_user.user_id.to_string();

    // Only platforms with working webhook routes can be registered as bots.
    // OpenClaw uses a separate integration path (openclaw_channel handler).
    if !matches!(
        body.platform.as_str(),
        "telegram" | "discord" | "lark" | "feishu"
    ) {
        return Err(AppError::ValidationError(format!(
            "unsupported bot platform: {}. Supported: telegram, discord, lark, feishu",
            body.platform
        )));
    }

    let adapter = resolve_adapter(&body.platform, &state.token_exchange_cache)?;

    // Validate label length (service also validates, but fail fast here)
    if body.label.is_empty() || body.label.len() > 128 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 128 characters".to_string(),
        ));
    }

    // Create bot: verify token, encrypt, insert in pending status
    let create_result = channel_bot_service::create_bot(
        &state.db,
        &state.config,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &user_id_str,
        &body.bot_token,
        &body.label,
        body.app_id.as_deref(),
        body.app_secret.as_deref(),
        body.public_key.as_deref(),
    )
    .await?;

    let bot_id = create_result.bot.id.clone();
    let webhook_secret = create_result.webhook_secret;

    // Build the per-bot webhook URL (platform-specific path)
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        state.config.base_url, &body.platform, &bot_id
    );

    // Register the webhook with the platform
    let reg_result = channel_bot_service::register_webhook(
        &state.db,
        &state.http_client,
        adapter.as_ref(),
        &bot_id,
        &body.bot_token,
        &webhook_url,
        &webhook_secret,
    )
    .await;

    if let Err(e) = reg_result {
        // Webhook registration failed: mark the bot as failed and return error
        let _ = channel_bot_service::mark_bot_failed(&state.db, &bot_id).await;
        return Err(AppError::BadRequest(format!(
            "Webhook registration failed: {e}"
        )));
    }

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "channel_bot_created".to_string(),
        Some(serde_json::json!({
            "bot_id": &bot_id,
            "platform": &body.platform,
            "label": &body.label,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateChannelBotResponse {
            id: bot_id,
            platform: create_result.bot.platform,
            platform_bot_username: create_result.bot.platform_bot_username,
            status: "active".to_string(),
        }),
    ))
}

/// GET /api/v1/channel-bots
pub async fn list_bots(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ChannelBotListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let bots = channel_bot_service::list_bots(&state.db, &user_id_str).await?;
    let total = bots.len() as u64;
    let items = bots.iter().map(bot_to_item).collect();
    Ok(Json(ChannelBotListResponse { bots: items, total }))
}

/// GET /api/v1/channel-bots/{id}
pub async fn get_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(bot_id): Path<String>,
) -> AppResult<Json<ChannelBotDetailResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let bot = channel_bot_service::get_bot_for_user(&state.db, &bot_id, &user_id_str).await?;

    // Count active conversations for this bot
    let conversations_count = state
        .db
        .collection::<mongodb::bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .count_documents(mongodb::bson::doc! {
            "channel_bot_id": &bot.id,
            "is_active": true,
        })
        .await?;

    Ok(Json(ChannelBotDetailResponse {
        id: bot.id,
        platform: bot.platform,
        label: bot.label,
        platform_bot_id: bot.platform_bot_id,
        platform_bot_username: bot.platform_bot_username,
        webhook_registered: bot.webhook_registered,
        status: bot.status,
        is_active: bot.is_active,
        conversations_count,
        created_at: bot.created_at.to_rfc3339(),
        updated_at: bot.updated_at.to_rfc3339(),
    }))
}

/// DELETE /api/v1/channel-bots/{id}
pub async fn delete_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(bot_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();

    // Fetch the bot to determine platform for adapter resolution
    let bot = channel_bot_service::get_bot_for_user(&state.db, &bot_id, &user_id_str).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;

    channel_bot_service::delete_bot(
        &state.db,
        &state.http_client,
        &state.encryption_keys,
        adapter.as_ref(),
        &bot_id,
        &user_id_str,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "channel_bot_deleted".to_string(),
        Some(serde_json::json!({
            "bot_id": &bot_id,
            "platform": &bot.platform,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/channel-bots/{id}/verify
pub async fn verify_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(bot_id): Path<String>,
) -> AppResult<Json<VerifyBotResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let bot = channel_bot_service::get_bot_for_user(&state.db, &bot_id, &user_id_str).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;

    // Decrypt the token and verify it is still valid with the platform
    let bot_token = channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?;

    adapter
        .verify_bot_token(&state.http_client, &bot_token)
        .await?;

    // Re-register webhook with a fresh secret. The original raw secret is not
    // stored (only its SHA-256 hash), so we generate a new one and update the
    // stored hash accordingly.
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        state.config.base_url, &bot.platform, &bot.id
    );

    let raw_secret = {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    };
    let new_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(raw_secret.as_bytes());
        hex::encode(hasher.finalize())
    };

    let reg_result = channel_bot_service::register_webhook(
        &state.db,
        &state.http_client,
        adapter.as_ref(),
        &bot.id,
        &bot_token,
        &webhook_url,
        &raw_secret,
    )
    .await;

    // Update the stored hash to match the new secret
    if reg_result.is_ok() {
        let _ = state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .update_one(
                mongodb::bson::doc! { "_id": &bot.id },
                mongodb::bson::doc! { "$set": { "webhook_secret_hash": &new_hash } },
            )
            .await;
    }

    let (status, webhook_registered) = match reg_result {
        Ok(()) => ("active".to_string(), true),
        Err(_) => {
            let _ = channel_bot_service::mark_bot_failed(&state.db, &bot.id).await;
            ("failed".to_string(), false)
        }
    };

    Ok(Json(VerifyBotResponse {
        id: bot.id,
        status,
        webhook_registered,
    }))
}
