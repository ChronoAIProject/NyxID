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
pub use crate::services::channel_adapters::resolve_adapter;
use crate::services::channel_platform::{BotCredentials, PlatformAdapter, RegistrationValues};
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
    #[serde(default)]
    pub phone_number_id: Option<String>,
    #[serde(default)]
    pub waba_id: Option<String>,
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
    pub bot_token: Option<String>,
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
            .field("phone_number_id", &self.phone_number_id)
            .field("waba_id", &self.waba_id)
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
            .field("bot_token", &self.bot_token.as_ref().map(|_| "[REDACTED]"))
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

fn ensure_verify_material_present(
    bot: &crate::models::channel_bot::ChannelBot,
    adapter: &dyn PlatformAdapter,
) -> AppResult<()> {
    adapter.validate_stored_verification(bot)
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ChannelBotItem {
    pub credential_source: String,
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
    pub credential_source: String,
    pub managed_setup: Option<ManagedSetupResponse>,
    #[serde(flatten)]
    pub platform_config: std::collections::BTreeMap<String, String>,
    pub webhook_url: String,
    pub webhook_secret_label: Option<&'static str>,
    pub setup_instructions: &'static [&'static str],
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

#[derive(Serialize)]
pub struct CreateChannelBotResponse {
    pub credential_source: String,
    pub managed_setup: Option<ManagedSetupResponse>,
    #[serde(flatten)]
    pub platform_config: std::collections::BTreeMap<String, String>,
    pub webhook_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    pub webhook_secret_label: Option<&'static str>,
    pub setup_instructions: &'static [&'static str],
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

#[derive(Debug, Serialize)]
pub struct ManagedSetupResponse {
    pub subscription: String,
    pub webhook_override: String,
    pub registration: String,
    pub business_id: Option<String>,
    pub coexistence: bool,
    pub coexistence_sync: std::collections::BTreeMap<String, String>,
}

impl From<&crate::models::channel_bot::ManagedBotSetup> for ManagedSetupResponse {
    fn from(value: &crate::models::channel_bot::ManagedBotSetup) -> Self {
        Self {
            subscription: value.subscription.clone(),
            webhook_override: value.webhook_override.clone(),
            registration: value.registration.clone(),
            business_id: value.business_id.clone(),
            coexistence: value.coexistence,
            coexistence_sync: value.coexistence_sync.clone(),
        }
    }
}

impl std::fmt::Debug for CreateChannelBotResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateChannelBotResponse")
            .field("id", &self.id)
            .field("platform", &self.platform)
            .field("webhook_secret", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl CreateChannelBotResponse {
    pub(crate) fn from_bot(
        bot: crate::models::channel_bot::ChannelBot,
        descriptor: crate::services::channel_platform::RegistrationDescriptor,
        webhook_url: String,
        webhook_secret: String,
    ) -> AppResult<Self> {
        let (permission_setup_url, permission_setup_scopes) = lark_permission_payload(&bot);
        Ok(Self {
            credential_source: bot.credential_source.clone(),
            managed_setup: bot.managed_setup.as_ref().map(Into::into),
            platform_config: descriptor.configuration(&bot)?,
            webhook_url,
            webhook_secret: descriptor
                .webhook_secret_label
                .filter(|_| bot.credential_source != "platform")
                .map(|_| webhook_secret),
            webhook_secret_label: descriptor.webhook_secret_label,
            setup_instructions: if bot.credential_source == "platform" {
                &[]
            } else {
                descriptor.setup_instructions
            },
            id: bot.id,
            platform: bot.platform,
            platform_bot_username: bot.platform_bot_username,
            status: descriptor.create_response_status.to_string(),
            permission_setup_url,
            permission_setup_scopes,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the effective owner id for a WRITE (create, delete, verify)
/// operation on a bot. For personal bots, returns the caller's id; for
/// org-owned bots, resolves the caller's access to the org and requires
/// `can_write()` (admin). Returns the bot's `user_id` on success so the
/// caller can pass it to the org-agnostic service layer.
pub(crate) async fn resolve_bot_owner_for_write(
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
pub(crate) async fn resolve_create_owner(
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
        credential_source: bot.credential_source.clone(),
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

    let adapter = resolve_adapter(&body.platform, &state.token_exchange_cache)?;
    let descriptor = adapter.registration();
    let label = body.label.trim();
    if label.is_empty() || label.len() > 128 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 128 characters".to_string(),
        ));
    }
    let fields = RegistrationValues(
        [
            ("bot_token", Some(body.bot_token.as_str())),
            ("app_id", body.app_id.as_deref()),
            ("app_secret", body.app_secret.as_deref()),
            ("verification_token", body.verification_token.as_deref()),
            ("encrypt_key", body.encrypt_key.as_deref()),
            ("public_key", body.public_key.as_deref()),
            ("phone_number_id", body.phone_number_id.as_deref()),
            ("waba_id", body.waba_id.as_deref()),
        ]
        .into_iter()
        .filter_map(|(key, value)| normalize_optional_field(value).map(|value| (key, value)))
        .collect(),
    );
    descriptor.validate(&fields, false)?;

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
        label,
        &fields,
    )
    .await?;

    let bot_id = create_result.bot.id.clone();
    let webhook_secret = create_result.webhook_secret;

    // Build the per-bot webhook URL (platform-specific path)
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        state.config.base_url, body.platform, bot_id
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

    Ok((
        StatusCode::CREATED,
        Json(CreateChannelBotResponse::from_bot(
            create_result.bot,
            descriptor,
            webhook_url,
            webhook_secret,
        )?),
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

    let updated = channel_bot_service::update_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        adapter.as_ref(),
        &bot_id,
        &owner_id,
        crate::services::channel_bot_service::UpdateBotParams {
            bot_token: body.bot_token.as_deref().map(str::trim),
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
        credential_source: updated.credential_source.clone(),
        managed_setup: updated.managed_setup.as_ref().map(Into::into),
        platform_config: adapter.registration().configuration(&updated)?,
        webhook_url: format!(
            "{}/api/v1/webhooks/channel/{}/{}",
            state.config.base_url, updated.platform, updated.id
        ),
        webhook_secret_label: adapter.registration().webhook_secret_label,
        setup_instructions: if updated.credential_source == "platform" {
            &[]
        } else {
            adapter.registration().setup_instructions
        },
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
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;

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
        credential_source: bot.credential_source.clone(),
        managed_setup: bot.managed_setup.as_ref().map(Into::into),
        platform_config: adapter.registration().configuration(&bot)?,
        webhook_url: format!(
            "{}/api/v1/webhooks/channel/{}/{}",
            state.config.base_url, bot.platform, bot.id
        ),
        webhook_secret_label: adapter.registration().webhook_secret_label,
        setup_instructions: if bot.credential_source == "platform" {
            &[]
        } else {
            adapter.registration().setup_instructions
        },
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
    let platform_secrets = if bot.credential_source == "platform" {
        Some(
            crate::services::channel_managed::build_verify_secrets(
                &state.db,
                &state.encryption_keys,
                adapter.as_ref(),
                &bot,
            )
            .await?,
        )
    } else {
        None
    };

    adapter
        .verify_bot_token(
            &state.http_client,
            &BotCredentials {
                token: &bot_token,
                platform_bot_id: Some(&bot.platform_bot_id),
                platform_secrets: platform_secrets.as_ref(),
            },
        )
        .await?;

    ensure_verify_material_present(&bot, adapter.as_ref())?;

    // Some subscription protocols bind the dashboard to the original secret.
    if adapter.registration().preserve_subscription_on_verify {
        return Ok(Json(VerifyBotResponse {
            id: bot.id,
            status: bot.status,
            webhook_registered: bot.webhook_registered,
        }));
    }

    // Re-register webhook with a fresh secret. The original raw secret is not
    // stored (only its SHA-256 hash), so we generate a new one and update the
    // stored hash accordingly.
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        state.config.base_url, bot.platform, bot.id
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

    #[test]
    fn create_response_only_exposes_dashboard_verification_secrets() {
        let cache = std::sync::Arc::new(
            crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
        );
        for platform in ["telegram", "whatsapp"] {
            let mut bot = make_telegram_bot();
            bot.platform = platform.to_string();
            let descriptor = resolve_adapter(platform, &cache).unwrap().registration();
            let response = CreateChannelBotResponse::from_bot(
                bot,
                descriptor,
                "https://nyxid.example/webhook".to_string(),
                "generated-secret".to_string(),
            )
            .unwrap();
            let json = serde_json::to_value(response).unwrap();
            if platform == "telegram" {
                assert!(json.get("webhook_secret").is_none());
                assert!(json["webhook_secret_label"].is_null());
            } else {
                assert_eq!(json["webhook_secret"], "generated-secret");
                assert_eq!(json["webhook_secret_label"], "Verify Token");
            }
        }
    }
    use chrono::Utc;

    fn make_lark_bot(has_verification_token: bool) -> crate::models::channel_bot::ChannelBot {
        crate::models::channel_bot::ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "lark".to_string(),
            label: "Test Lark Bot".to_string(),
            credential_source: "user".to_string(),
            registration_pin_encrypted: None,
            managed_setup: None,
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
        let err = ensure_verify_material_present(
            &bot,
            resolve_adapter(
                &bot.platform,
                &std::sync::Arc::new(
                    crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
                ),
            )
            .unwrap()
            .as_ref(),
        )
        .unwrap_err();

        assert!(matches!(err, AppError::ValidationError(_)));
        assert!(err.to_string().contains("missing Verification Token"));
        assert!(err.to_string().contains(&bot.id));
    }

    #[test]
    fn verify_allows_lark_bot_when_verification_token_is_present() {
        let bot = make_lark_bot(true);
        ensure_verify_material_present(
            &bot,
            resolve_adapter(
                &bot.platform,
                &std::sync::Arc::new(
                    crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
                ),
            )
            .unwrap()
            .as_ref(),
        )
        .expect("verification token should satisfy verify precondition");
    }

    fn make_telegram_bot() -> crate::models::channel_bot::ChannelBot {
        crate::models::channel_bot::ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "telegram".to_string(),
            label: "TG Bot".to_string(),
            credential_source: "user".to_string(),
            registration_pin_encrypted: None,
            managed_setup: None,
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
            phone_number_id: None,
            waba_id: None,
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
