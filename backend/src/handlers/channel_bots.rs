use axum::{
    Json,
    extract::{Path, Query, State},
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
use crate::services::channel_adapters::slack::SlackAdapter;
use crate::services::channel_adapters::telegram::TelegramAdapter;
use crate::services::channel_platform::PlatformAdapter;
use crate::services::{audit_service, channel_bot_service, lark_permission, org_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateChannelBotRequest {
    pub platform: String,
    pub bot_token: String,
    pub label: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    /// When set, create this channel bot under the given org. The
    /// resulting `ChannelBot.user_id` is the org's user id, making
    /// it visible to every org admin and to the org-delete blocker.
    /// Caller must be an admin of the target org.
    #[serde(default)]
    pub target_org_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateChannelBotRequest {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
}

/// Query parameters for `GET /api/v1/channel-bots`. Pass `org_id` to
/// list bots owned by an org (caller must be admin of the target org);
/// omit for the caller's personal bots.
#[derive(Debug, Deserialize, Default)]
pub struct ChannelBotListQuery {
    #[serde(default)]
    pub org_id: Option<String>,
}

impl std::fmt::Debug for CreateChannelBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateChannelBotRequest")
            .field("platform", &self.platform)
            .field("bot_token", &"[REDACTED]")
            .field("label", &self.label)
            .field("app_id", &self.app_id)
            .field(
                "app_secret",
                &self.app_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "verification_token",
                &self.verification_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "encrypt_key",
                &self.encrypt_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("public_key", &self.public_key)
            .field("target_org_id", &self.target_org_id)
            .finish()
    }
}

impl std::fmt::Display for CreateChannelBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::fmt::Debug for UpdateChannelBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateChannelBotRequest")
            .field("label", &self.label)
            .field(
                "verification_token",
                &self.verification_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "encrypt_key",
                &self.encrypt_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("app_id", &self.app_id)
            .field(
                "app_secret",
                &self.app_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl std::fmt::Display for UpdateChannelBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

fn normalize_optional_field(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Truncated SHA-256 of a platform conversation ID, for use in telemetry
/// properties where raw conversation IDs must not be emitted. Returns the
/// first 16 hex chars (8 bytes) of the digest — enough entropy for
/// per-conversation cardinality analysis, short enough to stay ergonomic.
pub(crate) fn hash_conversation_id(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

fn ensure_lark_verify_material_present(
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<()> {
    if matches!(bot.platform.as_str(), "lark" | "feishu")
        && bot.lark_verification_token_encrypted.is_none()
    {
        return Err(AppError::ValidationError(format!(
            "Lark/Feishu bot is missing Verification Token. PATCH /api/v1/channel-bots/{} with verification_token before verify.",
            bot.id
        )));
    }

    Ok(())
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
    /// Effective owner user_id. For personal bots this is the caller's
    /// user id; for org-owned bots this is the org's user id, which also
    /// doubles as the org id clients use in `target_org_id` / `?org_id=`.
    pub user_id: String,
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
    pub app_secret_configured: bool,
    pub lark_verification_token_configured: bool,
    pub lark_encrypt_key_configured: bool,
    pub conversations_count: u64,
    pub created_at: String,
    pub updated_at: String,
    /// Effective owner user_id (see `ChannelBotItem::user_id`).
    pub user_id: String,
    /// Lark/Feishu only: deep link to the developer console permissions
    /// page with the scopes NyxID's adapter needs already pre-selected.
    /// `None` for non-Lark platforms or when the bot has no `app_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_setup_url: Option<String>,
    /// Lark/Feishu only: the scope keys encoded in `permission_setup_url`,
    /// echoed back so the UI can render the list under the link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_setup_scopes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct CreateChannelBotResponse {
    pub id: String,
    pub platform: String,
    pub platform_bot_username: String,
    pub status: String,
    /// Lark/Feishu only: deep link to the developer console permissions
    /// page (see `ChannelBotDetailResponse::permission_setup_url`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_setup_url: Option<String>,
    /// Lark/Feishu only: scopes pre-selected in `permission_setup_url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_setup_scopes: Option<Vec<String>>,
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

/// Resolve the effective owner id for a WRITE (create, delete, verify)
/// operation on a bot. For personal bots, returns the caller's id; for
/// org-owned bots, resolves the caller's access to the org and requires
/// `can_write()` (admin). Returns the bot's `user_id` on success so the
/// caller can pass it to the org-agnostic service layer.
async fn resolve_bot_owner_for_write(
    state: &AppState,
    actor: &str,
    bot_id: &str,
) -> AppResult<(String, crate::models::channel_bot::ChannelBot)> {
    let bot = channel_bot_service::get_bot(&state.db, bot_id).await?;
    let access = org_service::resolve_owner_access(&state.db, actor, &bot.user_id).await?;
    if !access.can_read() {
        return Err(AppError::ChannelBotNotFound(bot_id.to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this channel bot".to_string(),
        ));
    }
    Ok((bot.user_id.clone(), bot))
}

/// Resolve the effective owner id for a READ operation on a bot.
/// Allows any active member of the owning org (including viewers).
async fn resolve_bot_owner_for_read(
    state: &AppState,
    actor: &str,
    bot_id: &str,
) -> AppResult<(String, crate::models::channel_bot::ChannelBot)> {
    let bot = channel_bot_service::get_bot(&state.db, bot_id).await?;
    let access = org_service::resolve_owner_access(&state.db, actor, &bot.user_id).await?;
    if !access.can_read() {
        return Err(AppError::ChannelBotNotFound(bot_id.to_string()));
    }
    Ok((bot.user_id.clone(), bot))
}

/// Resolve the owner id for creation. If `target_org_id` is set, the
/// caller must be an admin of that org; otherwise the owner is the
/// caller's own id.
async fn resolve_create_owner(
    state: &AppState,
    actor: &str,
    target_org_id: Option<&str>,
) -> AppResult<String> {
    if let Some(org_id) = target_org_id {
        let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create a channel bot under it"
                    .to_string(),
            ));
        }
        Ok(org_id.to_string())
    } else {
        Ok(actor.to_string())
    }
}

/// Resolve the owner id for a list operation. If `org_id` is set, the
/// caller must be an admin of that org; otherwise lists personal bots.
async fn resolve_list_owner(
    state: &AppState,
    actor: &str,
    org_id: Option<&str>,
) -> AppResult<String> {
    if let Some(org_id) = org_id {
        let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its channel bots".to_string(),
            ));
        }
        Ok(org_id.to_string())
    } else {
        Ok(actor.to_string())
    }
}

/// Resolve the platform adapter for the given platform identifier.
///
/// Supported platforms: telegram, discord, lark, feishu, slack, openclaw.
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
        "slack" => Ok(Box::new(SlackAdapter)),
        "openclaw" => Ok(Box::new(OpenClawAdapter)),
        other => Err(AppError::ValidationError(format!(
            "unsupported platform: {other}. Supported: telegram, discord, lark, feishu, slack, openclaw"
        ))),
    }
}

/// Derive the Lark/Feishu permission setup URL for a bot, if applicable.
///
/// Returns `(Some(url), Some(scopes))` for Lark/Feishu bots that have an
/// `app_id` configured, and `(None, None)` for every other case so the
/// caller can drop both fields from the response payload uniformly.
fn lark_permission_payload(
    bot: &crate::models::channel_bot::ChannelBot,
) -> (Option<String>, Option<Vec<String>>) {
    let region = match lark_permission::region_for_channel_platform(&bot.platform) {
        Some(r) => r,
        None => return (None, None),
    };
    let app_id = match bot.app_id.as_deref() {
        Some(value) if !value.is_empty() => value,
        _ => return (None, None),
    };
    let scopes = crate::services::channel_adapters::lark::REQUIRED_BOT_SCOPES;
    let url = lark_permission::build_permission_setup_url(region, app_id, scopes);
    let scope_strings = scopes.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    (Some(url), Some(scope_strings))
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
        user_id: bot.user_id.clone(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/channel-bots
pub async fn create_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateChannelBotRequest>,
) -> AppResult<(StatusCode, Json<CreateChannelBotResponse>)> {
    let actor = auth_user.user_id.to_string();

    // Only platforms with working webhook routes can be registered as bots.
    // OpenClaw uses a separate integration path (openclaw_channel handler).
    if !matches!(
        body.platform.as_str(),
        "telegram" | "discord" | "lark" | "feishu" | "slack"
    ) {
        return Err(AppError::ValidationError(format!(
            "unsupported bot platform: {}. Supported: telegram, discord, lark, feishu, slack",
            body.platform
        )));
    }

    let adapter = resolve_adapter(&body.platform, &state.token_exchange_cache)?;

    let label = body.label.trim();
    let bot_token = body.bot_token.trim();
    let app_id = normalize_optional_field(body.app_id.as_deref());
    let app_secret = normalize_optional_field(body.app_secret.as_deref());
    let verification_token = normalize_optional_field(body.verification_token.as_deref());
    let encrypt_key = normalize_optional_field(body.encrypt_key.as_deref());
    let public_key = normalize_optional_field(body.public_key.as_deref());

    // Validate label length (service also validates, but fail fast here)
    if label.is_empty() || label.len() > 128 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 128 characters".to_string(),
        ));
    }
    if bot_token.is_empty() {
        return Err(AppError::ValidationError(
            "Bot token is required".to_string(),
        ));
    }

    if matches!(body.platform.as_str(), "lark" | "feishu") && verification_token.is_none() {
        return Err(AppError::ValidationError(
            "Verification Token is required for Lark/Feishu".to_string(),
        ));
    }
    if matches!(body.platform.as_str(), "lark" | "feishu") && app_id.is_none() {
        return Err(AppError::ValidationError(
            "App ID is required for Lark/Feishu".to_string(),
        ));
    }
    if matches!(body.platform.as_str(), "lark" | "feishu") && app_secret.is_none() {
        return Err(AppError::ValidationError(
            "App Secret is required for Lark/Feishu".to_string(),
        ));
    }
    if body.platform == "discord" && public_key.is_none() {
        return Err(AppError::ValidationError(
            "Public Key is required for Discord".to_string(),
        ));
    }
    if body.platform == "slack" && app_secret.is_none() {
        return Err(AppError::ValidationError(
            "Signing Secret is required for Slack".to_string(),
        ));
    }

    // Resolve the effective owner. When `target_org_id` is set the bot
    // is written under the org's user_id so every admin can manage it
    // and the org-delete blocker treats it as a live org resource.
    let owner_id = resolve_create_owner(&state, &actor, body.target_org_id.as_deref()).await?;

    // Create bot: verify token, encrypt, insert in pending status
    let create_result = channel_bot_service::create_bot(
        &state.db,
        &state.config,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &owner_id,
        bot_token,
        label,
        app_id,
        app_secret,
        public_key,
        verification_token,
        encrypt_key,
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

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::ChannelBotRegistered {
            platform: body.platform.clone(),
        },
    );

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "channel_bot_created",
        Some(serde_json::json!({
            "bot_id": &bot_id,
            "platform": &body.platform,
            "label": label,
            "owner_user_id": &owner_id,
            "target_org_id": body.target_org_id,
        })),
    );

    let (permission_setup_url, permission_setup_scopes) =
        lark_permission_payload(&create_result.bot);

    Ok((
        StatusCode::CREATED,
        Json(CreateChannelBotResponse {
            id: bot_id,
            platform: create_result.bot.platform,
            platform_bot_username: create_result.bot.platform_bot_username,
            status: "active".to_string(),
            permission_setup_url,
            permission_setup_scopes,
        }),
    ))
}

/// PATCH /api/v1/channel-bots/{id}
pub async fn update_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(bot_id): Path<String>,
    Json(body): Json<UpdateChannelBotRequest>,
) -> AppResult<Json<ChannelBotDetailResponse>> {
    let actor = auth_user.user_id.to_string();
    let (owner_id, bot) = resolve_bot_owner_for_write(&state, &actor, &bot_id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;

    let label = match body.label.as_deref() {
        Some(value) if value.trim().is_empty() => {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 128 characters".to_string(),
            ));
        }
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.len() > 128 {
                return Err(AppError::ValidationError(
                    "Label must be between 1 and 128 characters".to_string(),
                ));
            }
            Some(trimmed)
        }
        None => None,
    };

    let verification_token = match body.verification_token.as_deref() {
        Some(value) if value.trim().is_empty() => {
            return Err(AppError::ValidationError(
                "Verification Token cannot be blank; PATCH the bot with a non-empty verification_token".to_string(),
            ));
        }
        Some(value) => Some(value.trim()),
        None => None,
    };

    let encrypt_key = match body.encrypt_key.as_deref() {
        Some(value) if value.trim().is_empty() => {
            crate::services::channel_bot_service::SecretPatch::Clear
        }
        Some(value) => crate::services::channel_bot_service::SecretPatch::Set(value.trim()),
        None => crate::services::channel_bot_service::SecretPatch::Unchanged,
    };

    let app_id = match body.app_id.as_deref() {
        Some(value) if value.trim().is_empty() => {
            return Err(AppError::ValidationError(
                "App ID cannot be blank".to_string(),
            ));
        }
        Some(value) => Some(value.trim()),
        None => None,
    };

    let app_secret = match body.app_secret.as_deref() {
        Some(value) if value.trim().is_empty() => {
            return Err(AppError::ValidationError(
                "App Secret cannot be blank".to_string(),
            ));
        }
        Some(value) => Some(value.trim()),
        None => None,
    };

    match bot.platform.as_str() {
        "lark" | "feishu" => {}
        "slack" => {
            if verification_token.is_some()
                || !matches!(
                    encrypt_key,
                    crate::services::channel_bot_service::SecretPatch::Unchanged
                )
                || app_id.is_some()
            {
                return Err(AppError::ValidationError(
                    "verification_token, encrypt_key, and app_id are only supported for Lark/Feishu bots".to_string(),
                ));
            }
        }
        _ => {
            if verification_token.is_some()
                || !matches!(
                    encrypt_key,
                    crate::services::channel_bot_service::SecretPatch::Unchanged
                )
                || app_id.is_some()
                || app_secret.is_some()
            {
                return Err(AppError::ValidationError(
                    "Only label updates are supported for this bot platform".to_string(),
                ));
            }
        }
    }

    let updated = channel_bot_service::update_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &bot_id,
        &owner_id,
        crate::services::channel_bot_service::UpdateBotParams {
            label,
            verification_token,
            encrypt_key,
            app_id,
            app_secret,
        },
    )
    .await?;

    let conversations_count = state
        .db
        .collection::<mongodb::bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .count_documents(mongodb::bson::doc! {
            "channel_bot_id": &updated.id,
            "is_active": true,
        })
        .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "channel_bot_updated",
        Some(serde_json::json!({
            "bot_id": &updated.id,
            "platform": &updated.platform,
            "owner_user_id": &owner_id,
        })),
    );

    let (permission_setup_url, permission_setup_scopes) = lark_permission_payload(&updated);

    Ok(Json(ChannelBotDetailResponse {
        id: updated.id,
        platform: updated.platform,
        label: updated.label,
        platform_bot_id: updated.platform_bot_id,
        platform_bot_username: updated.platform_bot_username,
        webhook_registered: updated.webhook_registered,
        status: updated.status,
        is_active: updated.is_active,
        app_secret_configured: updated.app_secret_encrypted.is_some(),
        lark_verification_token_configured: updated.lark_verification_token_encrypted.is_some(),
        lark_encrypt_key_configured: updated.lark_encrypt_key_encrypted.is_some(),
        conversations_count,
        created_at: updated.created_at.to_rfc3339(),
        updated_at: updated.updated_at.to_rfc3339(),
        user_id: updated.user_id,
        permission_setup_url,
        permission_setup_scopes,
    }))
}

/// GET /api/v1/channel-bots
pub async fn list_bots(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ChannelBotListQuery>,
) -> AppResult<Json<ChannelBotListResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_list_owner(&state, &actor, query.org_id.as_deref()).await?;
    let bots = channel_bot_service::list_bots(&state.db, &owner_id).await?;
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
    let actor = auth_user.user_id.to_string();
    let (_owner_id, bot) = resolve_bot_owner_for_read(&state, &actor, &bot_id).await?;

    // Count active conversations for this bot
    let conversations_count = state
        .db
        .collection::<mongodb::bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .count_documents(mongodb::bson::doc! {
            "channel_bot_id": &bot.id,
            "is_active": true,
        })
        .await?;

    let (permission_setup_url, permission_setup_scopes) = lark_permission_payload(&bot);

    Ok(Json(ChannelBotDetailResponse {
        id: bot.id,
        platform: bot.platform,
        label: bot.label,
        platform_bot_id: bot.platform_bot_id,
        platform_bot_username: bot.platform_bot_username,
        webhook_registered: bot.webhook_registered,
        status: bot.status,
        is_active: bot.is_active,
        app_secret_configured: bot.app_secret_encrypted.is_some(),
        lark_verification_token_configured: bot.lark_verification_token_encrypted.is_some(),
        lark_encrypt_key_configured: bot.lark_encrypt_key_encrypted.is_some(),
        conversations_count,
        created_at: bot.created_at.to_rfc3339(),
        updated_at: bot.updated_at.to_rfc3339(),
        user_id: bot.user_id,
        permission_setup_url,
        permission_setup_scopes,
    }))
}

/// DELETE /api/v1/channel-bots/{id}
pub async fn delete_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(bot_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();

    // Resolve the effective owner (personal or org via admin access).
    let (owner_id, bot) = resolve_bot_owner_for_write(&state, &actor, &bot_id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;

    channel_bot_service::delete_bot(
        &state.db,
        &state.http_client,
        &state.encryption_keys,
        adapter.as_ref(),
        &bot_id,
        &owner_id,
    )
    .await?;

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::ChannelBotDeleted {
            platform: bot.platform.clone(),
        },
    );

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "channel_bot_deleted",
        Some(serde_json::json!({
            "bot_id": &bot_id,
            "platform": &bot.platform,
            "owner_user_id": &owner_id,
        })),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/channel-bots/{id}/verify
pub async fn verify_bot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(bot_id): Path<String>,
) -> AppResult<Json<VerifyBotResponse>> {
    let actor = auth_user.user_id.to_string();
    let (_owner_id, bot) = resolve_bot_owner_for_write(&state, &actor, &bot_id).await?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;

    // Decrypt the token and verify it is still valid with the platform
    let bot_token = channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?;

    adapter
        .verify_bot_token(&state.http_client, &bot_token)
        .await?;

    ensure_lark_verify_material_present(&bot)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_lark_bot(has_verification_token: bool) -> crate::models::channel_bot::ChannelBot {
        crate::models::channel_bot::ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "lark".to_string(),
            label: "Test Lark Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "cli_test".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "unused".to_string(),
            app_id: Some("cli_test".to_string()),
            app_secret_encrypted: None,
            lark_verification_token_encrypted: has_verification_token.then(|| vec![1, 2, 3]),
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "pending_webhook".to_string(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn verify_requires_lark_verification_token_to_be_configured() {
        let bot = make_lark_bot(false);
        let err = ensure_lark_verify_material_present(&bot).unwrap_err();

        assert!(matches!(err, AppError::ValidationError(_)));
        assert!(err.to_string().contains("missing Verification Token"));
        assert!(err.to_string().contains(&bot.id));
    }

    #[test]
    fn verify_allows_lark_bot_when_verification_token_is_present() {
        let bot = make_lark_bot(true);
        ensure_lark_verify_material_present(&bot)
            .expect("verification token should satisfy verify precondition");
    }

    fn make_telegram_bot() -> crate::models::channel_bot::ChannelBot {
        crate::models::channel_bot::ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "telegram".to_string(),
            label: "TG Bot".to_string(),
            bot_token_encrypted: vec![0; 8],
            platform_bot_id: "123".to_string(),
            platform_bot_username: "tgbot".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "hash".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "pending".to_string(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn lark_permission_payload_for_lark_bot_returns_url_and_scopes() {
        let bot = make_lark_bot(true);
        let (url, scopes) = lark_permission_payload(&bot);
        let url = url.expect("Lark bot with app_id should produce a permission URL");
        assert!(url.starts_with("https://open.larksuite.com/app/cli_test/auth?q="));
        assert!(url.ends_with("&op_from=openapi"));
        assert_eq!(
            scopes.unwrap(),
            vec![
                "im:message".to_string(),
                "im:message:send_as_bot".to_string()
            ]
        );
    }

    #[test]
    fn lark_permission_payload_skips_non_lark_platforms() {
        let bot = make_telegram_bot();
        let (url, scopes) = lark_permission_payload(&bot);
        assert!(url.is_none());
        assert!(scopes.is_none());
    }

    #[test]
    fn lark_permission_payload_skips_lark_bot_without_app_id() {
        let mut bot = make_lark_bot(true);
        bot.app_id = None;
        let (url, scopes) = lark_permission_payload(&bot);
        assert!(url.is_none());
        assert!(scopes.is_none());
    }

    #[test]
    fn lark_permission_payload_uses_feishu_host_for_china_region() {
        let mut bot = make_lark_bot(true);
        bot.platform = "feishu".to_string();
        let (url, _) = lark_permission_payload(&bot);
        let url = url.expect("Feishu bot should produce a permission URL");
        assert!(url.starts_with("https://open.feishu.cn/app/cli_test/auth?q="));
    }

    #[test]
    fn normalize_optional_field_trims_and_filters() {
        assert_eq!(normalize_optional_field(None), None);
        assert_eq!(normalize_optional_field(Some("")), None);
        assert_eq!(normalize_optional_field(Some("  ")), None);
        assert_eq!(normalize_optional_field(Some(" hello ")), Some("hello"));
    }

    #[test]
    fn hash_conversation_id_is_deterministic_and_16_hex() {
        let h1 = hash_conversation_id("oc_chat789");
        let h2 = hash_conversation_id("oc_chat789");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_conversation_id_different_inputs() {
        assert_ne!(hash_conversation_id("a"), hash_conversation_id("b"));
    }

    #[test]
    fn bot_to_item_maps_fields() {
        let bot = make_telegram_bot();
        let item = bot_to_item(&bot);
        assert_eq!(item.platform, "telegram");
        assert_eq!(item.label, "TG Bot");
        assert!(!item.webhook_registered);
    }

    #[test]
    fn create_channel_bot_request_debug_redacts_token() {
        let req = CreateChannelBotRequest {
            platform: "telegram".to_string(),
            bot_token: "secret123".to_string(),
            label: "Test".to_string(),
            app_id: None,
            app_secret: Some("app_secret_val".to_string()),
            verification_token: None,
            encrypt_key: None,
            public_key: None,
            target_org_id: None,
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("secret123"));
        assert!(!debug.contains("app_secret_val"));
    }
}
