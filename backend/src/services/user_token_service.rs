use std::collections::HashMap;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::oauth_state::{COLLECTION_NAME as OAUTH_STATES, OAuthState};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_provider_token::{COLLECTION_NAME, UserProviderToken};
use crate::services::oauth_flow;
use crate::services::user_credentials_service;

/// Decrypted token ready for injection.
#[derive(Debug)]
pub struct DecryptedProviderToken {
    pub token_type: String,
    pub access_token: Option<String>,
    pub api_key: Option<String>,
}

/// Outcome of an OAuth/device-code callback, regardless of write path.
///
/// `connection_id` is `Some` when the multi-connection write path was
/// taken (token landed on a `UserApiKey` row by connection_id, bypassing
/// `user_provider_tokens`). `None` means the legacy write path ran:
/// a new `UserProviderToken` was inserted and the caller may want to
/// run the legacy `sync_provider_token_to_api_keys` fanout afterwards.
pub struct OAuthCallbackOutcome {
    pub user_id: String,
    /// The provider this OAuth flow targeted. Carried so callers don't
    /// need to refetch from the route or the (now-consumed) `OAuthState`.
    /// Currently unused by in-tree callers (they have the route param)
    /// but kept for future audit/log emission and downstream callers.
    #[allow(dead_code)]
    pub provider_config_id: String,
    pub connection_id: Option<String>,
}

/// Summary for listing (no decrypted tokens).
#[derive(Debug, serde::Serialize)]
pub struct UserProviderTokenSummary {
    pub provider_config_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub provider_type: String,
    pub token_type: String,
    pub status: String,
    pub label: Option<String>,
    pub gateway_url: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
    pub metadata: Option<HashMap<String, String>>,
}

const OAUTH_PROVIDER_NOT_CONFIGURED_MESSAGE: &str =
    "This provider is not configured for OAuth yet. Please contact your admin.";

/// Maximum number of user-supplied additional scopes per OAuth initiate request.
const MAX_ADDITIONAL_SCOPES: usize = 32;
/// Maximum length of a single scope string.
const MAX_SCOPE_LENGTH: usize = 256;

/// Parse a user-supplied scope string into a list of individual scopes.
///
/// Accepts comma- or whitespace-separated scopes and trims empty entries.
/// Returns `Ok(vec![])` when `raw` is empty or `None`, which is indistinguishable
/// from "no additional scopes" for the caller — the merged result will fall back
/// to `provider.default_scopes`.
///
/// Validation:
/// - At most [`MAX_ADDITIONAL_SCOPES`] entries.
/// - Each scope is at most [`MAX_SCOPE_LENGTH`] characters.
/// - Each scope must match `[A-Za-z0-9._:/~+*=-]+` (RFC 6749 §3.3 permits
///   a broader set, but this covers every known OAuth scope format including
///   Google (`https://.../auth/drive.readonly`), Lark (`contact:contact.base:readonly`),
///   GitHub (`repo`, `read:org`), Atlassian (`read:jira-work`), etc.).
pub fn parse_additional_scopes(raw: Option<&str>) -> AppResult<Vec<String>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let scopes: Vec<String> = raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    if scopes.len() > MAX_ADDITIONAL_SCOPES {
        return Err(AppError::ValidationError(format!(
            "Too many additional scopes (max {MAX_ADDITIONAL_SCOPES})"
        )));
    }

    for scope in &scopes {
        if scope.len() > MAX_SCOPE_LENGTH {
            return Err(AppError::ValidationError(format!(
                "OAuth scope exceeds {MAX_SCOPE_LENGTH} characters"
            )));
        }
        if !scope.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '_' | ':' | '/' | '~' | '+' | '*' | '=' | '-')
        }) {
            return Err(AppError::ValidationError(format!(
                "OAuth scope contains invalid characters: {scope}"
            )));
        }
    }

    Ok(scopes)
}

/// Merge a provider's default scopes with user-supplied additional scopes.
///
/// Preserves the order of `default_scopes` first, then appends any additional
/// scope not already present. Deduplication is case-sensitive (OAuth scopes are
/// case-sensitive per RFC 6749 §3.3).
fn merge_scopes(default_scopes: Option<&Vec<String>>, additional_scopes: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = default_scopes.cloned().unwrap_or_default();
    for scope in additional_scopes {
        if !merged.iter().any(|existing| existing == scope) {
            merged.push(scope.clone());
        }
    }
    merged
}

/// Resolve the space-joined `scope` value to send to the provider, or `None`
/// to omit the parameter entirely. Single source of truth shared by the OAuth
/// and device-code initiate paths (NyxID#917).
///
/// Precedence:
/// 1. `scope_override` present (from the scope-picker UI) → use it verbatim,
///    bypassing the default merge so the user can drop a default. An empty
///    override returns `None` (omit the param; let the provider apply its own
///    minimum) rather than emitting a malformed empty `scope`.
/// 2. No override, no additional scopes → the provider's `default_scopes`
///    verbatim (emitting an empty string when defaults are `Some(vec![])`, to
///    stay byte-identical with the pre-feature behavior).
/// 3. No override, with additional scopes → default-merge + dedupe (legacy
///    additive `--scope` / `scope=` callers).
fn resolve_scope_param(
    default_scopes: Option<&Vec<String>>,
    additional_scopes: &[String],
    scope_override: Option<&[String]>,
) -> Option<String> {
    if let Some(ov) = scope_override {
        if ov.is_empty() {
            None
        } else {
            Some(ov.join(" "))
        }
    } else if additional_scopes.is_empty() {
        default_scopes.map(|scopes| scopes.join(" "))
    } else {
        Some(merge_scopes(default_scopes, additional_scopes).join(" "))
    }
}

/// Validate that a given provider supports user-supplied additional scopes.
///
/// Only providers that need/accept scopes should receive them:
/// - `oauth2` providers always accept scopes (RFC 6749 §3.3).
/// - `device_code` providers using `rfc8628` format accept scopes.
/// - `device_code` providers using `openai` format do **not** accept a `scope`
///   parameter — scopes are baked into the client registration (e.g., Codex).
///   Forwarding a scope value here would turn a previously working connect
///   into a provider-side failure.
///
/// An empty `additional_scopes` slice is always allowed, so existing default
/// behavior is preserved on every code path.
fn ensure_additional_scopes_supported(
    provider: &ProviderConfig,
    additional_scopes: &[String],
) -> AppResult<()> {
    if additional_scopes.is_empty() {
        return Ok(());
    }

    match provider.provider_type.as_str() {
        "oauth2" => Ok(()),
        "device_code" => {
            if provider.device_code_format == "openai" {
                Err(AppError::ValidationError(
                    "This provider's device code endpoint does not accept additional OAuth scopes \
                     (OpenAI-format device code providers ignore the `scope` parameter). \
                     Remove the extra scopes and try again."
                        .to_string(),
                ))
            } else {
                Ok(())
            }
        }
        other => Err(AppError::ValidationError(format!(
            "Provider type '{other}' does not support OAuth scopes"
        ))),
    }
}

fn build_telegram_identity_metadata(
    data: &crate::crypto::telegram::TelegramLoginData,
) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    metadata.insert("telegram_user_id".to_string(), data.id.to_string());
    metadata.insert("first_name".to_string(), data.first_name.clone());
    if let Some(ref ln) = data.last_name {
        metadata.insert("last_name".to_string(), ln.clone());
    }
    if let Some(ref un) = data.username {
        metadata.insert("username".to_string(), un.clone());
    }
    if let Some(ref pu) = data.photo_url {
        metadata.insert("photo_url".to_string(), pu.clone());
    }
    metadata
}

fn build_telegram_identity_update_doc(
    metadata: &HashMap<String, String>,
    now: chrono::DateTime<Utc>,
) -> AppResult<bson::Document> {
    let metadata_bson = bson::to_bson(metadata)
        .map_err(|e| AppError::Internal(format!("Failed to serialize Telegram metadata: {e}")))?;

    Ok(doc! {
        "status": "active",
        "error_message": bson::Bson::Null,
        "metadata": metadata_bson,
        "updated_at": bson::DateTime::from_chrono(now),
    })
}

fn normalize_telegram_bot_api_key(raw: &str) -> AppResult<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Err(AppError::ValidationError(
            "Telegram bot token must not be empty".to_string(),
        ));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(AppError::ValidationError(
            "Telegram bot token must not contain whitespace".to_string(),
        ));
    }
    if normalized.contains('/')
        || normalized.contains('\\')
        || normalized.contains('?')
        || normalized.contains('#')
        || normalized.contains('\0')
        || normalized.contains('%')
        || normalized.contains("..")
    {
        return Err(AppError::ValidationError(
            "Telegram bot token contains invalid characters".to_string(),
        ));
    }

    Ok(normalized.to_string())
}

async fn get_active_telegram_widget_provider(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<ProviderConfig> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "telegram_widget" {
        return Err(AppError::BadRequest(
            "This provider requires Telegram Login Widget connection".to_string(),
        ));
    }

    Ok(provider)
}

fn ensure_oauth_provider_configured(provider: &ProviderConfig) -> AppResult<()> {
    // URLs are always required regardless of credential mode
    if provider.authorization_url.is_none() || provider.token_url.is_none() {
        return Err(AppError::BadRequest(
            OAUTH_PROVIDER_NOT_CONFIGURED_MESSAGE.to_string(),
        ));
    }

    // For "user" or "both" modes, URLs alone are sufficient (users bring their own credentials)
    // For "admin" (default), admin-level credentials are also required
    if provider.credential_mode != "user"
        && provider.credential_mode != "both"
        && (provider.client_id_encrypted.is_none() || provider.client_secret_encrypted.is_none())
    {
        return Err(AppError::BadRequest(
            OAUTH_PROVIDER_NOT_CONFIGURED_MESSAGE.to_string(),
        ));
    }

    Ok(())
}

/// Store an API key for a provider.
pub async fn store_api_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
    gateway_url: Option<&str>,
) -> AppResult<UserProviderToken> {
    // Verify provider exists and is active
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "api_key" {
        return Err(AppError::BadRequest(
            "This provider requires OAuth connection, not an API key".to_string(),
        ));
    }

    let api_key_to_store = if provider.slug == "telegram-bot" {
        normalize_telegram_bot_api_key(api_key)?
    } else {
        if api_key.is_empty() {
            return Err(AppError::ValidationError(
                "API key must not be empty".to_string(),
            ));
        }
        api_key.to_string()
    };

    // Check if user already has a token for this provider (including revoked)
    let existing = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .await?;

    let now = Utc::now();
    let encrypted = encryption_keys.encrypt(api_key_to_store.as_bytes()).await?;

    if let Some(existing_token) = existing {
        // Update existing token
        let mut set_doc = doc! {
            "api_key_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: encrypted,
            },
            "status": "active",
            "label": label,
            "error_message": bson::Bson::Null,
            "updated_at": bson::DateTime::from_chrono(now),
        };
        match gateway_url {
            Some(url) => {
                set_doc.insert("gateway_url", url);
            }
            None => {
                set_doc.insert("gateway_url", bson::Bson::Null);
            }
        }
        db.collection::<UserProviderToken>(COLLECTION_NAME)
            .update_one(doc! { "_id": &existing_token.id }, doc! { "$set": set_doc })
            .await?;

        let updated = db
            .collection::<UserProviderToken>(COLLECTION_NAME)
            .find_one(doc! { "_id": &existing_token.id })
            .await?
            .ok_or_else(|| AppError::Internal("Token disappeared after update".to_string()))?;

        return Ok(updated);
    }

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        connection_id: None,
        credential_user_id: None,
        token_type: "api_key".to_string(),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        token_scopes: None,
        expires_at: None,
        api_key_encrypted: Some(encrypted),
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: label.map(String::from),
        metadata: None,
        gateway_url: gateway_url.map(String::from),
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "API key stored for provider"
    );

    Ok(token)
}

/// Return the Telegram bot username needed to render the Login Widget.
///
/// Verifies that the provider is an active Telegram Login Widget provider and
/// that the required bot configuration exists.
pub async fn get_telegram_connect_bot_username(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<String> {
    let provider = get_active_telegram_widget_provider(db, provider_id).await?;

    let bot_username = provider
        .client_id_param_name
        .as_deref()
        .ok_or_else(|| {
            AppError::BadRequest(
                "Telegram bot username not configured for this provider".to_string(),
            )
        })
        .and_then(crate::services::provider_service::normalize_telegram_bot_username)?;
    if provider.client_secret_encrypted.is_none() {
        return Err(AppError::BadRequest(
            "Telegram bot token not configured for this provider".to_string(),
        ));
    }

    Ok(bot_username)
}

/// Verify Telegram Login Widget callback data and persist the resulting
/// identity metadata for the user.
pub async fn connect_telegram_widget(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
    data: &crate::crypto::telegram::TelegramLoginData,
) -> AppResult<UserProviderToken> {
    let provider = get_active_telegram_widget_provider(db, provider_id).await?;

    let bot_token_enc = provider.client_secret_encrypted.ok_or_else(|| {
        AppError::BadRequest("Telegram bot token not configured for this provider".to_string())
    })?;
    let bot_token_bytes = Zeroizing::new(encryption_keys.decrypt(&bot_token_enc).await?);
    let bot_token = std::str::from_utf8(bot_token_bytes.as_slice())
        .map_err(|e| AppError::Internal(format!("Failed to decode bot token: {e}")))?;

    crate::crypto::telegram::verify_telegram_login(bot_token, data)?;

    store_telegram_identity(db, user_id, provider_id, data).await
}

/// Store a verified Telegram identity for a user.
///
/// Called only after the Telegram Login Widget callback has already been
/// verified. No tokens are stored — only the verified identity metadata.
async fn store_telegram_identity(
    db: &mongodb::Database,
    user_id: &str,
    provider_id: &str,
    data: &crate::crypto::telegram::TelegramLoginData,
) -> AppResult<UserProviderToken> {
    // Check if user already has a token for this provider
    let existing = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
            "status": { "$ne": "revoked" },
        })
        .await?;

    let now = Utc::now();

    let metadata = build_telegram_identity_metadata(data);

    if let Some(existing_token) = existing {
        // Replace the full metadata object so stale optional fields do not survive reconnects.
        let set_doc = build_telegram_identity_update_doc(&metadata, now)?;

        db.collection::<UserProviderToken>(COLLECTION_NAME)
            .update_one(doc! { "_id": &existing_token.id }, doc! { "$set": set_doc })
            .await?;

        let updated = db
            .collection::<UserProviderToken>(COLLECTION_NAME)
            .find_one(doc! { "_id": &existing_token.id })
            .await?
            .ok_or_else(|| AppError::Internal("Token disappeared after update".to_string()))?;

        return Ok(updated);
    }

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        connection_id: None,
        credential_user_id: None,
        token_type: "telegram_identity".to_string(),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        token_scopes: None,
        expires_at: None,
        api_key_encrypted: None,
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: None,
        metadata: Some(metadata),
        gateway_url: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        telegram_user_id = %data.id,
        "Telegram identity stored for provider"
    );

    Ok(token)
}

/// Initiate an OAuth2 connection flow. Returns the authorization URL.
///
/// When `on_behalf_of` is `Some(sa_id)`, the flow stores tokens under the SA's
/// ID instead of the initiating user. `redirect_path` overrides the default
/// frontend callback path for the post-OAuth redirect.
///
/// `additional_scopes` are merged (deduped, order-preserving) on top of the
/// provider's `default_scopes`. Pass an empty slice to preserve the original
/// default-scopes-only behavior.
#[allow(clippy::too_many_arguments)]
/// Initiate an OAuth2 authorization-code flow.
///
/// `connection_id` (multi-connection rollout): when `Some`, the flow is
/// part of a fresh multi-connection add — the callback will write the
/// resulting token directly to the `UserApiKey` row carrying this
/// `connection_id` (bypassing `user_provider_tokens`). When `None`, the
/// callback takes the legacy single-tenant path (writing to
/// `user_provider_tokens` keyed by `(user_id, provider_config_id)`).
pub async fn initiate_oauth_connect(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    base_url: &str,
    user_id: &str,
    provider_id: &str,
    on_behalf_of: Option<&str>,
    redirect_path: Option<&str>,
    additional_scopes: &[String],
    scope_override: Option<&[String]>,
    connection_id: Option<&str>,
) -> AppResult<String> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "oauth2" {
        return Err(AppError::BadRequest(
            "This provider uses API keys, not OAuth".to_string(),
        ));
    }

    ensure_oauth_provider_configured(&provider)?;
    ensure_additional_scopes_supported(&provider, additional_scopes)?;
    // A non-empty override is still subject to the same provider-type guard
    // (e.g. an `openai`-format device-code provider would reject it). An empty
    // override is the user clearing every scope — allowed; the provider then
    // decides at consent time.
    if let Some(ov) = scope_override {
        ensure_additional_scopes_supported(&provider, ov)?;
    }

    let authorization_url = provider
        .authorization_url
        .as_ref()
        .expect("OAuth provider configuration checked above");

    // Multi-connection: if the caller threaded a `connection_id`, look for
    // BYO Custom App credentials on that connection's `UserApiKey` first.
    // When present they replace the single-row `user_provider_credentials`
    // lookup — required for multi-Custom-App Lark / Feishu, since the
    // legacy table only holds one (client_id, secret) pair per
    // `(user, provider)`. Falls through to `resolve_oauth_credentials`
    // for legacy connections, codex-style provider-owned device-code
    // flows, and "both"-mode adds without BYO.
    let resolved = if let Some(conn_id) = connection_id
        && let Some(conn_creds) = user_credentials_service::resolve_connection_oauth_credentials(
            db,
            encryption_keys,
            conn_id,
        )
        .await?
    {
        conn_creds
    } else {
        user_credentials_service::resolve_oauth_credentials(db, encryption_keys, &provider, user_id)
            .await?
    };
    let client_id = resolved.client_id;

    // Create state for CSRF protection
    let state_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + Duration::minutes(10);

    // Generate PKCE code verifier if supported
    let code_verifier = if provider.supports_pkce {
        Some(oauth_flow::generate_code_verifier())
    } else {
        None
    };

    // SEC-M2: Encrypt code_verifier before storing
    let encrypted_verifier = match code_verifier.as_ref() {
        Some(v) => {
            let encrypted = encryption_keys.encrypt(v.as_bytes()).await?;
            Some(hex::encode(encrypted))
        }
        None => None,
    };

    let oauth_state = OAuthState {
        id: state_id.clone(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        code_verifier: encrypted_verifier,
        device_code_encrypted: None,
        user_code_encrypted: None,
        poll_interval: None,
        target_user_id: on_behalf_of.map(String::from),
        credential_user_id: resolved.credential_user_id.clone(),
        redirect_path: redirect_path.map(String::from),
        connection_id: connection_id.map(String::from),
        consumed: false,
        expires_at,
        created_at: now,
    };

    db.collection::<OAuthState>(OAUTH_STATES)
        .insert_one(&oauth_state)
        .await?;

    // Use the generic callback URL (matches the route registered for the callback)
    let callback_url = format!(
        "{}/api/v1/providers/callback",
        base_url.trim_end_matches('/')
    );

    let cid_param = oauth_flow::client_id_param_name(&provider);
    let mut auth_url = format!(
        "{}?{}={}&redirect_uri={}&response_type=code&state={}",
        authorization_url,
        urlencoding::encode(cid_param),
        urlencoding::encode(&client_id),
        urlencoding::encode(&callback_url),
        urlencoding::encode(&state_id),
    );

    // Scope resolution (NyxID#917) — see `resolve_scope_param`. `None` means
    // omit `scope` entirely; a `Some("")` is only produced for an admin-seeded
    // `default_scopes: Some(vec![])`, preserving the byte-identical
    // pre-feature URL.
    if let Some(scope_str) = resolve_scope_param(
        provider.default_scopes.as_ref(),
        additional_scopes,
        scope_override,
    ) {
        auth_url.push_str(&format!("&scope={}", urlencoding::encode(&scope_str)));
    }

    if let Some(ref verifier) = code_verifier {
        let challenge = oauth_flow::generate_code_challenge(verifier);
        auth_url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            urlencoding::encode(&challenge)
        ));
    }

    // Append provider-specific extra auth params (blocklist enforced)
    if let Some(ref extra) = provider.extra_auth_params {
        const BLOCKLIST: &[&str] = &[
            "client_id",
            "client_secret",
            "redirect_uri",
            "response_type",
            "state",
            "code",
            "code_challenge",
            "code_challenge_method",
            "scope",
            "grant_type",
            "nonce",
        ];
        for (key, value) in extra {
            if !BLOCKLIST.contains(&key.as_str()) && key != cid_param {
                auth_url.push_str(&format!(
                    "&{}={}",
                    urlencoding::encode(key),
                    urlencoding::encode(value)
                ));
            }
        }
    }

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        on_behalf_of = ?on_behalf_of,
        "OAuth connect flow initiated"
    );

    Ok(auth_url)
}

/// Result from requesting a device code (RFC 8628 step 1).
pub struct DeviceCodeInitiateResult {
    pub user_code: String,
    pub verification_uri: String,
    pub state: String,
    pub expires_in: i64,
    pub interval: i32,
}

/// Result from polling device code status (RFC 8628 step 3).
pub struct DeviceCodePollResult {
    pub status: String,
    pub interval: Option<i32>,
    pub effective_user_id: Option<String>,
}

/// Step 1: Request a device code from the provider.
///
/// Calls the provider's device_code_url to get a device_auth_id + user_code,
/// stores the encrypted identifiers in an oauth_state, and returns the
/// user_code and verification_uri for the frontend to display.
///
/// When `on_behalf_of` is `Some(sa_id)`, the resulting tokens will be stored
/// under the SA's ID instead of the initiating user.
///
/// `additional_scopes` are merged on top of `provider.default_scopes` and sent
/// in the RFC 8628 device code request. Pass an empty slice to preserve the
/// original default-scopes-only behavior.
///
/// `connection_id` (multi-connection rollout): when `Some`, the eventual
/// poll-completion will write the resulting token directly to the
/// `UserApiKey` row carrying this `connection_id` (bypassing
/// `user_provider_tokens`). When `None`, the completion takes the legacy
/// single-tenant path (writing to `user_provider_tokens` keyed by
/// `(user_id, provider_config_id)`). Mirrors the `connection_id`
/// semantics of [`initiate_oauth_connect`].
#[allow(clippy::too_many_arguments)]
pub async fn request_device_code(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
    on_behalf_of: Option<&str>,
    additional_scopes: &[String],
    scope_override: Option<&[String]>,
    connection_id: Option<&str>,
) -> AppResult<DeviceCodeInitiateResult> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "device_code" {
        return Err(AppError::BadRequest(
            "This provider does not use the device code flow".to_string(),
        ));
    }

    ensure_additional_scopes_supported(&provider, additional_scopes)?;
    if let Some(ov) = scope_override {
        ensure_additional_scopes_supported(&provider, ov)?;
    }

    let device_code_url = provider.device_code_url.as_ref().ok_or_else(|| {
        AppError::Internal("Device code provider missing device_code_url".to_string())
    })?;

    // Multi-connection: same precedence as `initiate_oauth_connect`. Codex
    // (the only `device_code` provider today) is provider-owned, so the
    // BYO path won't actually fire, but the branch is here so a future
    // BYO `device_code` provider works without a second patch.
    let resolved = if let Some(conn_id) = connection_id
        && let Some(conn_creds) = user_credentials_service::resolve_connection_oauth_credentials(
            db,
            encryption_keys,
            conn_id,
        )
        .await?
    {
        conn_creds
    } else {
        user_credentials_service::resolve_oauth_credentials(db, encryption_keys, &provider, user_id)
            .await?
    };
    let client_id = resolved.client_id;

    // Branch on device_code_format: "openai" uses JSON, "rfc8628" uses form-urlencoded
    let response = if provider.device_code_format == "openai" {
        // OpenAI's device code endpoint does not accept a `scope` field
        // (scopes are baked into the client registration, e.g. Codex). We
        // enforce this by rejecting `additional_scopes` for openai-format
        // providers above, so the request body here is unchanged from the
        // pre-scope-feature implementation.
        let mut body = serde_json::Map::new();
        body.insert(
            oauth_flow::client_id_param_name(&provider).to_string(),
            serde_json::Value::String(client_id.clone()),
        );
        oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(device_code_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Device code request failed: {e}")))?
    } else {
        // RFC 8628: form-urlencoded with client_id and optional scope.
        //
        // Backward-compat: when there are no user-supplied additional scopes
        // we take the exact pre-feature code path so the request body is
        // byte-identical (an admin-seeded provider with empty default_scopes
        // still skips the `scope` form field, matching the old behavior).
        let mut params = vec![oauth_flow::client_id_form_field(&provider, &client_id)];
        // Scope resolution shared with `initiate_oauth_connect` (NyxID#917).
        if let Some(scope_str) = resolve_scope_param(
            provider.default_scopes.as_ref(),
            additional_scopes,
            scope_override,
        ) {
            params.push(("scope".to_string(), scope_str));
        }
        oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(device_code_url))
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Device code request failed: {e}")))?
    };

    if !response.status().is_success() {
        let status = response.status();
        let resp_body = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        tracing::error!(
            provider_id = %provider_id,
            status = %status,
            "Device code request returned error"
        );
        return Err(AppError::Internal(format!(
            "Device code request failed with status {status}: {}",
            resp_body.chars().take(200).collect::<String>()
        )));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse device code response: {e}")))?;

    // OpenAI returns `device_auth_id`; standard RFC 8628 returns `device_code`
    let device_auth_id = data["device_auth_id"]
        .as_str()
        .or_else(|| data["device_code"].as_str())
        .ok_or_else(|| {
            AppError::Internal("Missing device_auth_id/device_code in response".to_string())
        })?;

    let user_code = data["user_code"]
        .as_str()
        .or_else(|| data["usercode"].as_str())
        .ok_or_else(|| AppError::Internal("Missing user_code in response".to_string()))?;

    // Verification URI: try response first, then provider config
    let verification_uri = data["verification_uri"]
        .as_str()
        .or_else(|| data["verification_url"].as_str())
        .map(String::from)
        .or_else(|| provider.device_verification_url.clone())
        .ok_or_else(|| {
            AppError::Internal("No verification URI in response or provider config".to_string())
        })?;

    // OpenAI returns interval as a string; handle both string and number
    let interval = data["interval"]
        .as_i64()
        .or_else(|| data["interval"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(5) as i32;

    // OpenAI returns expires_at (ISO timestamp); fall back to expires_in (seconds)
    let expires_in = if let Some(expires_at_str) = data["expires_at"].as_str() {
        chrono::DateTime::parse_from_rfc3339(expires_at_str)
            .map(|dt| (dt.timestamp() - Utc::now().timestamp()).max(60))
            .unwrap_or(900)
    } else {
        data["expires_in"].as_i64().unwrap_or(900)
    };

    // Encrypt device_auth_id and user_code before storing
    let device_code_encrypted =
        hex::encode(encryption_keys.encrypt(device_auth_id.as_bytes()).await?);
    let user_code_encrypted = hex::encode(encryption_keys.encrypt(user_code.as_bytes()).await?);

    // Create state document
    let state_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + Duration::seconds(expires_in);

    let oauth_state = OAuthState {
        id: state_id.clone(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        code_verifier: None,
        device_code_encrypted: Some(device_code_encrypted),
        user_code_encrypted: Some(user_code_encrypted),
        poll_interval: Some(interval),
        target_user_id: on_behalf_of.map(String::from),
        credential_user_id: resolved.credential_user_id.clone(),
        redirect_path: None,
        connection_id: connection_id.map(String::from),
        consumed: false,
        expires_at,
        created_at: now,
    };

    db.collection::<OAuthState>(OAUTH_STATES)
        .insert_one(&oauth_state)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        on_behalf_of = ?on_behalf_of,
        "Device code flow initiated"
    );

    Ok(DeviceCodeInitiateResult {
        user_code: user_code.to_string(),
        verification_uri,
        state: state_id,
        expires_in,
        interval,
    })
}

/// Step 3: Poll the provider's device token endpoint.
///
/// OpenAI-style: sends device_auth_id + user_code as JSON, checks HTTP status.
/// On 403/404 = still pending, on 2xx = success with authorization_code + PKCE,
/// then exchanges authorization_code at token_url for actual tokens.
pub async fn poll_device_code(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
    state: &str,
) -> AppResult<DeviceCodePollResult> {
    let now = Utc::now();

    // Look up state without deleting (we need it for multiple polls)
    let oauth_state = db
        .collection::<OAuthState>(OAUTH_STATES)
        .find_one(doc! { "_id": state })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired device code state".to_string()))?;

    if oauth_state.expires_at < now {
        db.collection::<OAuthState>(OAUTH_STATES)
            .delete_one(doc! { "_id": state })
            .await?;
        return Ok(DeviceCodePollResult {
            status: "expired".to_string(),
            interval: None,
            effective_user_id: None,
        });
    }

    if oauth_state.provider_config_id != provider_id {
        return Err(AppError::BadRequest(
            "Device code state provider mismatch".to_string(),
        ));
    }

    if oauth_state.user_id != user_id {
        return Err(AppError::BadRequest(
            "Device code state user mismatch".to_string(),
        ));
    }

    // When admin-on-behalf flow, store tokens under the target SA's ID
    let effective_user_id = oauth_state.target_user_id.as_deref().unwrap_or(user_id);

    // Decrypt device_auth_id
    let device_code_hex = oauth_state
        .device_code_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("OAuth state missing device_auth_id".to_string()))?;
    let dc_bytes = hex::decode(device_code_hex).map_err(|e| {
        AppError::Internal(format!("Failed to decode encrypted device_auth_id: {e}"))
    })?;
    let decrypted_dc = Zeroizing::new(encryption_keys.decrypt(&dc_bytes).await?);
    let device_auth_id = String::from_utf8((*decrypted_dc).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode device_auth_id: {e}")))?;

    // Decrypt user_code
    let user_code_hex = oauth_state
        .user_code_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("OAuth state missing user_code".to_string()))?;
    let uc_bytes = hex::decode(user_code_hex)
        .map_err(|e| AppError::Internal(format!("Failed to decode encrypted user_code: {e}")))?;
    let decrypted_uc = Zeroizing::new(encryption_keys.decrypt(&uc_bytes).await?);
    let user_code = String::from_utf8((*decrypted_uc).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode user_code: {e}")))?;

    // Load provider config
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))?;

    let device_token_url = provider.device_token_url.as_ref().ok_or_else(|| {
        AppError::Internal("Device code provider missing device_token_url".to_string())
    })?;

    // Multi-connection: when the device-code flow was initiated against a
    // connection (`OAuthState.connection_id`), poll-time client credentials
    // come from THAT connection's `UserApiKey` rather than the
    // single-row `user_provider_credentials` table. Falls back to the
    // legacy resolution (credential_user_id-keyed) for connection-less
    // flows.
    let resolved = if let Some(conn_id) = oauth_state.connection_id.as_deref()
        && let Some(conn_creds) = user_credentials_service::resolve_connection_oauth_credentials(
            db,
            encryption_keys,
            conn_id,
        )
        .await?
    {
        conn_creds
    } else {
        user_credentials_service::resolve_token_oauth_credentials(
            db,
            encryption_keys,
            &provider,
            oauth_state.credential_user_id.as_deref(),
        )
        .await?
    };
    let poll_client_id = resolved.client_id;

    // Branch on device_code_format
    let is_openai = provider.device_code_format == "openai";

    let response = if is_openai {
        // OpenAI-style poll: send device_auth_id + user_code as JSON
        let poll_body = serde_json::json!({
            "device_auth_id": &device_auth_id,
            "user_code": &user_code,
        });
        oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(device_token_url))
            .json(&poll_body)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Device code poll failed: {e}")))?
    } else {
        // RFC 8628: form-urlencoded with grant_type, device_code, client_id
        let mut params = vec![
            (
                "grant_type".to_string(),
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("device_code".to_string(), device_auth_id.clone()),
        ];
        params.push(oauth_flow::client_id_form_field(&provider, &poll_client_id));
        oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(device_token_url))
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Device code poll failed: {e}")))?
    };

    let status_code = response.status();

    // OpenAI: 403/404 = authorization pending
    if is_openai
        && (status_code == reqwest::StatusCode::FORBIDDEN
            || status_code == reqwest::StatusCode::NOT_FOUND)
    {
        return Ok(DeviceCodePollResult {
            status: "pending".to_string(),
            interval: oauth_state.poll_interval,
            effective_user_id: None,
        });
    }

    if !status_code.is_success() {
        let raw_body = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());

        match classify_device_poll_failure(status_code, &raw_body) {
            Ok(DevicePollFlow::Pending) => {
                return Ok(DeviceCodePollResult {
                    status: "pending".to_string(),
                    interval: oauth_state.poll_interval,
                    effective_user_id: None,
                });
            }
            Ok(DevicePollFlow::SlowDown) => {
                let new_interval = oauth_state.poll_interval.unwrap_or(5) + 5;
                db.collection::<OAuthState>(OAUTH_STATES)
                    .update_one(
                        doc! { "_id": state },
                        doc! { "$set": { "poll_interval": new_interval } },
                    )
                    .await?;
                return Ok(DeviceCodePollResult {
                    status: "slow_down".to_string(),
                    interval: Some(new_interval),
                    effective_user_id: None,
                });
            }
            Ok(DevicePollFlow::Expired) => {
                db.collection::<OAuthState>(OAUTH_STATES)
                    .delete_one(doc! { "_id": state })
                    .await?;
                return Ok(DeviceCodePollResult {
                    status: "expired".to_string(),
                    interval: None,
                    effective_user_id: None,
                });
            }
            Ok(DevicePollFlow::Denied) => {
                db.collection::<OAuthState>(OAUTH_STATES)
                    .delete_one(doc! { "_id": state })
                    .await?;
                return Ok(DeviceCodePollResult {
                    status: "denied".to_string(),
                    interval: None,
                    effective_user_id: None,
                });
            }
            Err(err) => {
                tracing::error!(
                    provider_id = %provider_id,
                    status = %status_code,
                    body = %raw_body,
                    "Device code poll failed with provider error"
                );
                return Err(err);
            }
        }
    }

    // Success (2xx): parse response
    let resp_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse poll response: {e}")))?;

    // OpenAI returns authorization_code + PKCE for a second exchange step
    if let Some(authorization_code) = resp_data["authorization_code"].as_str() {
        let code_verifier = resp_data["code_verifier"].as_str().ok_or_else(|| {
            AppError::Internal("Missing code_verifier in device poll response".to_string())
        })?;

        let token_url = provider.token_url.as_ref().ok_or_else(|| {
            AppError::Internal("Provider missing token_url for code exchange".to_string())
        })?;

        // Exchange authorization_code at token_url with PKCE
        // Codex CLI uses form-urlencoded (NOT JSON) and redirect_uri = {issuer}/deviceauth/callback
        let issuer = device_token_url
            .find("/api/accounts/")
            .map(|idx| &device_token_url[..idx])
            .unwrap_or("https://auth.openai.com");
        let redirect_uri = format!("{issuer}/deviceauth/callback");

        let mut token_params = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), authorization_code.to_string()),
            ("redirect_uri".to_string(), redirect_uri),
            ("code_verifier".to_string(), code_verifier.to_string()),
        ];
        token_params.push(oauth_flow::client_id_form_field(&provider, &poll_client_id));

        let token_response =
            oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(token_url))
                .form(&token_params)
                .send()
                .await
                .map_err(|e| {
                    AppError::Internal(format!("Device code token exchange failed: {e}"))
                })?;

        let status = token_response.status();
        let raw_body = token_response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());

        let token_data = match parse_token_exchange_response(status, &raw_body) {
            Ok(value) => value,
            Err(err) => {
                tracing::error!(
                    provider_id = %provider_id,
                    status = %status,
                    body = %raw_body,
                    "Device code token exchange returned error"
                );
                return Err(err);
            }
        };

        return store_device_code_tokens(
            db,
            encryption_keys,
            effective_user_id,
            provider_id,
            state,
            resolved.credential_user_id.as_deref(),
            oauth_state.connection_id.as_deref(),
            &token_data,
            now,
        )
        .await;
    }

    // Standard flow: access_token directly in poll response
    store_device_code_tokens(
        db,
        encryption_keys,
        effective_user_id,
        provider_id,
        state,
        oauth_state.credential_user_id.as_deref(),
        oauth_state.connection_id.as_deref(),
        &resp_data,
        now,
    )
    .await
}

/// Store tokens from a device code flow response (either direct or after code exchange).
///
/// `connection_id` (multi-connection rollout): when `Some`, the tokens
/// are written directly to the matching `UserApiKey` row (via
/// [`user_api_key_service::write_oauth_tokens_to_key`]), bypassing
/// `user_provider_tokens`. When `None`, the legacy single-tenant path
/// runs (`delete_many` + `insert_one` on `user_provider_tokens`).
#[allow(clippy::too_many_arguments)]
async fn store_device_code_tokens(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
    state: &str,
    credential_user_id: Option<&str>,
    connection_id: Option<&str>,
    token_data: &serde_json::Value,
    now: chrono::DateTime<Utc>,
) -> AppResult<DeviceCodePollResult> {
    let access_token = token_data["access_token"]
        .as_str()
        .ok_or_else(|| AppError::Internal("Missing access_token in token response".to_string()))?;

    let refresh_token = token_data["refresh_token"].as_str();
    let expires_in = token_data["expires_in"].as_i64();
    let scope = token_data["scope"].as_str();

    let token_expires_at = expires_in.map(|secs| now + Duration::seconds(secs));

    // Multi-connection path: write tokens directly to the UserApiKey
    // identified by connection_id, then delete the OAuth state. State
    // deletion happens AFTER the token write so reconcile's "no live
    // state ⇒ abandoned" inference can never observe an in-flight
    // window where the new credential isn't yet visible (issue #653
    // race fix parity with `handle_oauth_callback`).
    if let Some(ref conn_id) = connection_id {
        crate::services::user_api_key_service::write_oauth_tokens_to_key(
            db,
            encryption_keys,
            conn_id,
            access_token,
            refresh_token,
            scope,
            token_expires_at,
        )
        .await
        .inspect_err(|e| {
            tracing::warn!(
                user_id = %user_id,
                provider_id = %provider_id,
                connection_id = %conn_id,
                error = %e,
                "multi-connection device-code write failed; OAuthState left in place (TTL will sweep)"
            );
        })?;

        let _ = db
            .collection::<OAuthState>(OAUTH_STATES)
            .delete_one(doc! { "_id": state })
            .await;

        tracing::info!(
            user_id = %user_id,
            provider_id = %provider_id,
            connection_id = %conn_id,
            "Device code tokens written to UserApiKey (multi-connection path)"
        );

        return Ok(DeviceCodePollResult {
            status: "complete".to_string(),
            interval: None,
            effective_user_id: Some(user_id.to_string()),
        });
    }

    // ── Legacy single-tenant path ──
    let access_enc = encryption_keys.encrypt(access_token.as_bytes()).await?;
    let refresh_enc = match refresh_token {
        Some(rt) => Some(encryption_keys.encrypt(rt.as_bytes()).await?),
        None => None,
    };

    // Delete the oauth_state (flow complete). Pre-existing ordering:
    // state-delete first, then token upsert below. This leaves a small
    // window between state-delete and token-insert during which
    // `reconcile_pending_oauth_placeholder` Pass 2 (triggered by
    // `GET /keys/{id}` wizard polling) could observe a missing state +
    // missing token and mark a `pending_auth` placeholder as failed.
    // The window is very tight (two adjacent Mongo round-trips) and
    // pre-dates this multi-connection work — addressing it would be a
    // separate refactor. Multi-connection callers go through the
    // `write_oauth_tokens_to_key` branch above and avoid this entirely.
    db.collection::<OAuthState>(OAUTH_STATES)
        .delete_one(doc! { "_id": state })
        .await?;

    // Upsert: remove existing token for this user+provider, insert new
    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .delete_many(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .await?;

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        connection_id: None,
        credential_user_id: credential_user_id.map(String::from),
        token_type: "oauth2".to_string(),
        access_token_encrypted: Some(access_enc),
        refresh_token_encrypted: refresh_enc,
        token_scopes: scope.map(String::from),
        expires_at: token_expires_at,
        api_key_encrypted: None,
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: None,
        metadata: None,
        gateway_url: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "Device code OAuth token stored"
    );

    Ok(DeviceCodePollResult {
        status: "complete".to_string(),
        interval: None,
        effective_user_id: Some(user_id.to_string()),
    })
}

/// Peek at an OAuth state without consuming it (for the generic callback handler).
pub async fn peek_oauth_state(db: &mongodb::Database, state_id: &str) -> AppResult<OAuthState> {
    db.collection::<OAuthState>(OAUTH_STATES)
        .find_one(doc! { "_id": state_id })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired OAuth state".to_string()))
}

/// Handle the OAuth2 callback after user authorizes.
///
/// Uses a dedicated no-redirect HTTP client (SEC-H2) for the token exchange.
///
/// Returns an [`OAuthCallbackOutcome`]. Two write paths are possible:
///
/// - **Multi-connection** (`OAuthState.connection_id.is_some()`): tokens
///   are written directly to the matching `UserApiKey` row via
///   [`user_api_key_service::write_oauth_tokens_to_key`]. The
///   `user_provider_tokens` collection is **not** touched. The outcome
///   carries `connection_id: Some(...)` so the caller can skip the
///   legacy fan-out sync.
///
/// - **Legacy** (`connection_id.is_none()`): existing behavior —
///   `delete_many({user, provider})` followed by `insert_one(new token)`
///   on `user_provider_tokens`. The outcome carries
///   `connection_id: None` and the caller typically follows with
///   `sync_provider_token_to_api_keys` to fan tokens out to legacy keys.
pub async fn handle_oauth_callback(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    base_url: &str,
    provider_id: &str,
    code: &str,
    state: &str,
) -> AppResult<OAuthCallbackOutcome> {
    // Atomic-claim the state: flip `consumed` from false→true, returning
    // the document. A concurrent callback (replay) loses the race because
    // the filter requires `consumed: { $ne: true }`.
    //
    // Critically, we do NOT delete the state here (as the previous
    // implementation did). Deleting up-front opened a race window
    // [state-deleted, token-inserted] of ~1+s during which
    // `reconcile_pending_oauth_placeholder`'s "no live OAuth state ⇒
    // abandoned ⇒ fail placeholder" inference would prematurely mark the
    // pending placeholder as `failed` for an in-flight successful OAuth
    // (issue #653 race regression caught in PR #723 review). Keeping the
    // row alive (with `consumed=true`) closes that window. The state is
    // deleted at the end of this function, after the new token is in.
    let now = Utc::now();
    let oauth_state = db
        .collection::<OAuthState>(OAUTH_STATES)
        .find_one_and_update(
            doc! { "_id": state, "consumed": { "$ne": true } },
            doc! { "$set": { "consumed": true } },
        )
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired OAuth state".to_string()))?;

    if oauth_state.expires_at < now {
        // Best-effort cleanup of the just-claimed-but-expired state so it
        // doesn't sit in the collection until natural expiry sweeps.
        let _ = db
            .collection::<OAuthState>(OAUTH_STATES)
            .delete_one(doc! { "_id": state })
            .await;
        return Err(AppError::BadRequest("OAuth state has expired".to_string()));
    }

    if oauth_state.provider_config_id != provider_id {
        return Err(AppError::BadRequest(
            "OAuth state provider mismatch".to_string(),
        ));
    }

    // When admin-on-behalf flow, store tokens under the target SA's ID
    let effective_user_id = oauth_state
        .target_user_id
        .as_deref()
        .unwrap_or(&oauth_state.user_id);
    let user_id = effective_user_id;

    // Load provider config
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))?;

    ensure_oauth_provider_configured(&provider)?;

    let token_url = provider
        .token_url
        .as_ref()
        .expect("OAuth provider configuration checked above");

    // Reuse the same OAuth client that was selected during initiation.
    //
    // Multi-connection: the initiate path resolved client credentials from
    // the connection's own `UserApiKey` when `connection_id` was set, so
    // the exchange must use the same source — otherwise the authorize
    // URL would have been signed with the Custom App's client_id but the
    // code-exchange would carry whatever (`user_provider_credentials` or
    // `ProviderConfig`) happened to be present, and Lark would reject
    // the exchange with `redirect_uri_mismatch` / `invalid_client`.
    let resolved = if let Some(conn_id) = oauth_state.connection_id.as_deref()
        && let Some(conn_creds) = user_credentials_service::resolve_connection_oauth_credentials(
            db,
            encryption_keys,
            conn_id,
        )
        .await?
    {
        conn_creds
    } else {
        user_credentials_service::resolve_token_oauth_credentials(
            db,
            encryption_keys,
            &provider,
            oauth_state.credential_user_id.as_deref(),
        )
        .await?
    };

    // Use the generic callback URL (must match what was sent in initiate)
    let callback_url = format!(
        "{}/api/v1/providers/callback",
        base_url.trim_end_matches('/')
    );

    // Exchange code for tokens
    let use_basic_auth = provider.token_endpoint_auth_method == "client_secret_basic";
    let mut params = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("code".to_string(), code.to_string()),
        ("redirect_uri".to_string(), callback_url),
    ];

    if use_basic_auth {
        // client_id still needed in body for some providers even with Basic Auth
        // but credentials go in the Authorization header
    } else {
        params.push(oauth_flow::client_id_form_field(
            &provider,
            &resolved.client_id,
        ));
        if let Some(ref secret) = resolved.client_secret {
            params.push(("client_secret".to_string(), secret.clone()));
        }
    }

    // SEC-M2: Decrypt code_verifier from stored state
    if let Some(ref encrypted_verifier) = oauth_state.code_verifier {
        let verifier_bytes = hex::decode(encrypted_verifier)
            .map_err(|e| AppError::Internal(format!("Failed to decode encrypted verifier: {e}")))?;
        let decrypted = Zeroizing::new(encryption_keys.decrypt(&verifier_bytes).await?);
        let verifier = String::from_utf8((*decrypted).clone())
            .map_err(|e| AppError::Internal(format!("Failed to decode verifier: {e}")))?;
        params.push(("code_verifier".to_string(), verifier));
    }

    // SEC-H2: Use no-redirect client for token exchange
    let mut request =
        oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(token_url));
    request = if uses_json_oauth_token_exchange(&provider) {
        request.json(&params_to_json_body(&params))
    } else {
        request.form(&params)
    };
    if use_basic_auth {
        request = request.basic_auth(&resolved.client_id, resolved.client_secret.as_deref());
    }
    let token_response = request
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("OAuth token exchange failed: {e}")))?;

    let status = token_response.status();
    // Read the body once as text so we can both (a) parse provider-shaped
    // error envelopes that come back with HTTP 200 (Lark / Feishu return
    // `{"code": <non-zero>, "msg": "..."}` with a 200 status) and (b) keep
    // the full body for the server-side audit log without consuming the
    // response twice.
    let raw_body = token_response
        .text()
        .await
        .unwrap_or_else(|_| "unknown".to_string());

    let token_data = match parse_token_exchange_response(status, &raw_body) {
        Ok(value) => value,
        Err(err) => {
            // Log the full status + raw body server-side; `err` only carries
            // the provider's own returned error text (see
            // `parse_token_exchange_response`), never internal/DB details.
            tracing::error!(
                provider_id = %provider_id,
                status = %status,
                body = %raw_body,
                "OAuth token exchange returned error"
            );
            return Err(err);
        }
    };

    let token_payload = oauth_token_payload(&token_data);

    let access_token = match token_payload["access_token"].as_str() {
        Some(token) => token,
        None => {
            tracing::error!(
                provider_id = %provider_id,
                status = %status,
                body = %raw_body,
                "OAuth token exchange response missing access_token"
            );
            // Surface any provider-returned error text rather than a generic
            // internal error so the wizard shows something actionable.
            return Err(
                token_exchange_provider_error(&token_data).unwrap_or_else(|| {
                    AppError::BadRequest(
                        "Identity provider did not return an access token. \
                     Re-check the app credentials and try connecting again."
                            .to_string(),
                    )
                }),
            );
        }
    };

    let refresh_token = token_payload["refresh_token"].as_str();
    let expires_in = token_payload["expires_in"].as_i64();
    let scope = token_payload["scope"].as_str();

    let access_enc = encryption_keys.encrypt(access_token.as_bytes()).await?;
    let refresh_enc = match refresh_token {
        Some(rt) => Some(encryption_keys.encrypt(rt.as_bytes()).await?),
        None => None,
    };

    let token_expires_at = expires_in.map(|secs| now + Duration::seconds(secs));

    // Branch on connection_id (set by `initiate_oauth_connect` when the
    // flow is part of a fresh multi-connection add). When present, the
    // new tokens land directly on the matching `UserApiKey` row and the
    // `user_provider_tokens` collection is untouched. Otherwise: legacy
    // single-tenant path — delete + insert into `user_provider_tokens`.
    if let Some(ref conn_id) = oauth_state.connection_id {
        // The pre-encrypted blobs computed above are discarded; the
        // helper owns encryption end-to-end (encrypts from plaintext).
        // Letting them drop naturally at end-of-scope is functionally
        // identical to dropping them explicitly.
        crate::services::user_api_key_service::write_oauth_tokens_to_key(
            db,
            encryption_keys,
            conn_id,
            access_token,
            refresh_token,
            scope,
            token_expires_at,
        )
        .await
        .inspect_err(|e| {
            // Multi-connection write failed (e.g. UserApiKey was
            // deleted mid-flow). The OAuth state row is still alive
            // with `consumed: true` and will be cleaned up by TTL.
            // Logging here so the rare race is visible to ops without
            // requiring a heavier audit-log emission.
            tracing::warn!(
                user_id = %user_id,
                provider_id = %provider_id,
                connection_id = %conn_id,
                error = %e,
                "multi-connection write failed; OAuthState left consumed=true (TTL will sweep)"
            );
        })?;

        // Best-effort cleanup of the consumed OAuth state. Identical
        // ordering to the legacy branch (done last so reconcile's "no
        // live state => abandoned" inference can never observe an
        // in-flight window where the new token isn't yet visible).
        let _ = db
            .collection::<OAuthState>(OAUTH_STATES)
            .delete_one(doc! { "_id": state })
            .await;

        tracing::info!(
            user_id = %user_id,
            provider_id = %provider_id,
            connection_id = %conn_id,
            "OAuth tokens written to UserApiKey (multi-connection callback path)"
        );

        return Ok(OAuthCallbackOutcome {
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            connection_id: Some(conn_id.clone()),
        });
    }

    // ── Legacy single-tenant path ──
    // Upsert: remove existing token for this user+provider, insert new.
    //
    // NyxID#917: before deleting, snapshot the existing token's
    // refresh_token_encrypted and token_scopes. Many providers omit
    // `refresh_token` on subsequent authorization-code exchanges (it was
    // issued once at initial grant) and `scope` is optional in OAuth token
    // responses. Without this preservation, a manage-scopes re-auth on a
    // legacy `connection_id: null` connection would advance
    // `last_authorized_at` (so the wizard reports success) while wiping
    // the central UserProviderToken's refresh token / recorded scopes —
    // making `sync_provider_token_to_api_keys_after_authorization` fan
    // those nulls back out to the legacy UserApiKey rows.
    let existing_token = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .await?;

    let preserved_refresh_enc = refresh_enc.or_else(|| {
        existing_token
            .as_ref()
            .and_then(|t| t.refresh_token_encrypted.clone())
    });
    let preserved_scope = scope
        .map(String::from)
        .or_else(|| existing_token.as_ref().and_then(|t| t.token_scopes.clone()));

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .delete_many(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .await?;

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        connection_id: None,
        credential_user_id: resolved.credential_user_id.clone(),
        token_type: "oauth2".to_string(),
        access_token_encrypted: Some(access_enc),
        refresh_token_encrypted: preserved_refresh_enc,
        token_scopes: preserved_scope,
        expires_at: token_expires_at,
        api_key_encrypted: None,
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: None,
        metadata: None,
        gateway_url: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    // Now that the new token is durable, delete the consumed OAuth state.
    // Best-effort: an error here is harmless since `expires_at` will sweep
    // it later. Done last so reconcile's "no live state ⇒ abandoned"
    // inference can never observe the in-flight window where the new
    // token isn't yet visible (issue #653 race fix).
    let _ = db
        .collection::<OAuthState>(OAUTH_STATES)
        .delete_one(doc! { "_id": state })
        .await;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "OAuth token stored for provider"
    );

    Ok(OAuthCallbackOutcome {
        user_id: token.user_id,
        provider_config_id: token.provider_config_id,
        connection_id: None,
    })
}

fn uses_json_oauth_token_exchange(provider: &ProviderConfig) -> bool {
    matches!(provider.slug.as_str(), "lark" | "feishu")
        || provider.token_url.as_deref().is_some_and(|url| {
            url.contains("/open-apis/authen/v2/oauth/token")
                && (url.contains("open.larksuite.com") || url.contains("open.feishu.cn"))
        })
}

fn params_to_json_body(params: &[(String, String)]) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    for (key, value) in params {
        body.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    serde_json::Value::Object(body)
}

fn oauth_token_payload(token_data: &serde_json::Value) -> &serde_json::Value {
    if token_data
        .get("access_token")
        .and_then(|value| value.as_str())
        .is_some()
    {
        return token_data;
    }
    token_data
        .get("data")
        .filter(|data| {
            data.get("access_token")
                .and_then(|value| value.as_str())
                .is_some()
        })
        .unwrap_or(token_data)
}

/// Extract a user-surfaceable error from an OAuth token-exchange response
/// body, if it carries one.
///
/// Two provider error shapes are recognized:
///
/// - **Lark / Feishu**: HTTP 200 with `{"code": <non-zero>, "msg": "..."}`.
///   These providers do NOT use the OAuth-standard error envelope and do
///   NOT signal failure via the HTTP status, so a generic parser that only
///   inspects the status or looks for `access_token` would otherwise miss
///   the real cause (issue #694).
/// - **RFC 6749 §5.2**: `{"error": "...", "error_description": "..."}`.
///
/// Returns a [`AppError::BadRequest`] carrying ONLY the provider's own
/// returned `code`/`msg`/`error` text. This is intentionally a surfaceable
/// variant (not `Internal`/`DatabaseError`) so `safe_error_message` passes
/// it through to the user. The raw body and HTTP status are logged by the
/// caller for ops; they are never embedded in the returned error.
fn token_exchange_provider_error(token_data: &serde_json::Value) -> Option<AppError> {
    // Lark / Feishu envelope: non-zero integer `code` means failure.
    if let Some(code) = token_data.get("code").and_then(serde_json::Value::as_i64)
        && code != 0
    {
        let msg = token_data
            .get("msg")
            .and_then(serde_json::Value::as_str)
            .filter(|m| !m.trim().is_empty())
            .unwrap_or("unknown error");
        return Some(AppError::BadRequest(format!(
            "Identity provider rejected the authorization (code {code}): {msg}"
        )));
    }

    // RFC 6749 §5.2 error envelope.
    if let Some(error) = token_data
        .get("error")
        .and_then(serde_json::Value::as_str)
        .filter(|e| !e.trim().is_empty())
    {
        let description = token_data
            .get("error_description")
            .and_then(serde_json::Value::as_str)
            .filter(|d| !d.trim().is_empty());
        let message = match description {
            Some(desc) => format!("Identity provider rejected the authorization: {error} ({desc})"),
            None => format!("Identity provider rejected the authorization: {error}"),
        };
        return Some(AppError::BadRequest(message));
    }

    None
}

/// Evaluate a raw OAuth token-exchange response body and return the parsed
/// JSON value on success, or a user-surfaceable [`AppError`] on failure.
///
/// Failure handling never leaks internal/transport details to the returned
/// error: only provider-returned error text (via
/// [`token_exchange_provider_error`]) or a generic-but-actionable message is
/// surfaced. The caller logs the full status + raw body for ops.
fn parse_token_exchange_response(
    status: reqwest::StatusCode,
    raw_body: &str,
) -> AppResult<serde_json::Value> {
    let parsed: Option<serde_json::Value> = serde_json::from_str(raw_body).ok();

    // A provider-shaped error can appear regardless of HTTP status (Lark
    // returns it with 200). Prefer it whenever present so the user sees the
    // provider's own message.
    if let Some(ref value) = parsed
        && let Some(provider_err) = token_exchange_provider_error(value)
    {
        return Err(provider_err);
    }

    if !status.is_success() {
        // Non-2xx with no recognizable provider error envelope. Surface the
        // status (RFC-style "the provider returned an error") without echoing
        // the raw body, which may contain HTML or sensitive transport noise.
        return Err(AppError::BadRequest(format!(
            "Identity provider returned an error during token exchange (HTTP {}). \
             Re-check the app credentials and try connecting again.",
            status.as_u16()
        )));
    }

    // 2xx: require a JSON object we can read tokens out of.
    match parsed {
        Some(value @ serde_json::Value::Object(_)) => Ok(value),
        _ => Err(AppError::BadRequest(
            "Identity provider returned an unreadable token response. \
             Re-check the app credentials and try connecting again."
                .to_string(),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePollFlow {
    Pending,
    SlowDown,
    Expired,
    Denied,
}

/// Classify a non-2xx device-code poll response body. Returns:
/// - `Ok(DevicePollFlow)` for the four RFC 8628 flow-control states the
///   caller maps to "pending"/"slow_down"/"expired"/"denied",
/// - `Err(AppError::BadRequest)` for any other recognizable provider error
///   (RFC 6749 / Lark envelope) or an opaque non-2xx. The error carries only
///   the provider's own message or a generic-but-actionable hint — never the
///   raw body.
fn classify_device_poll_failure(
    status: reqwest::StatusCode,
    raw_body: &str,
) -> AppResult<DevicePollFlow> {
    let parsed: Option<serde_json::Value> = serde_json::from_str(raw_body).ok();

    if let Some(ref value) = parsed {
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            match error {
                "authorization_pending" => return Ok(DevicePollFlow::Pending),
                "slow_down" => return Ok(DevicePollFlow::SlowDown),
                "expired_token" => return Ok(DevicePollFlow::Expired),
                "access_denied" => return Ok(DevicePollFlow::Denied),
                _ => {}
            }
        }

        if let Some(provider_err) = token_exchange_provider_error(value) {
            return Err(provider_err);
        }
    }

    Err(AppError::BadRequest(format!(
        "Identity provider returned an error during device authorization (HTTP {}). Re-check the app credentials and try again.",
        status.as_u16()
    )))
}

/// Multi-connection OAuth refresh path: refresh an access token using the
/// `refresh_token` stored on a `UserApiKey` row, write the new tokens back
/// to the same row, and return the refreshed key.
///
/// Mirrors [`oauth_flow::refresh_oauth_token`] (which operates on
/// `UserProviderToken`) but for keys minted via the multi-connection
/// add path. Crucially:
///
/// - **OAuth client credentials**: if the key carries user-provided BYO
///   creds (`user_oauth_client_id_encrypted` set — the Lark / Feishu
///   case), they are decrypted from the key itself. Otherwise the
///   `ProviderConfig` client_id (and optional secret) is used (the
///   codex / device-code case where NyxID owns the OAuth client). This
///   avoids consulting `user_provider_credentials`, which is single-tenant
///   per `(user, provider)` and can't represent two different Lark
///   Custom Apps owned by the same user.
///
/// - **Token storage**: success writes new `access_token_encrypted`,
///   `refresh_token_encrypted` (if returned), `expires_at`,
///   `last_used_at`, `status: "active"`, and clears `error_message`
///   directly on the `UserApiKey` row by `_id`. No write to
///   `user_provider_tokens`.
///
/// - **Failure**: writes `status: "failed"` (intentional — `auth-flow-
///   polling.ts` treats `failed` as terminal; using `refresh_failed`
///   would silently leave the wizard polling until timeout) and a
///   truncated error message (200 chars, SEC-M5 parity). Then returns
///   `AppError::Internal`. Lark / Feishu return HTTP 200 with a non-zero
///   `code` on failure (rather than a 4xx body); both shapes are handled
///   and both write `status: "failed"`.
///
/// Caller (`proxy_service::maybe_refresh_provider_backed_api_key`)
/// reaches this path only when `api_key.connection_id.is_some()`.
///
/// Concurrency: this function is read-modify-write on the `UserApiKey`
/// row without a database-level lock. Two simultaneous refreshes for the
/// same `_id` will both call the IdP; the loser's response is discarded
/// (last-write-wins), and if the provider rotates the refresh_token, the
/// loser may end up persisting an already-invalidated value. Acceptable
/// per the design intent — the next refresh attempt would fail and the
/// row would be marked `status: "failed"`. Callers should not invoke
/// this function concurrently for the same key.
/// Fire-and-forget: emit a `key_refresh_failed` audit event so dashboards
/// and operators can detect silently-broken multi-connection refreshes
/// without waiting on a user-facing 401. Includes `connection_id`,
/// `provider_config_id`, `api_key_id`, and a truncated error message
/// so the root cause is visible without a second DB read.
fn emit_key_refresh_failed_audit(
    db: &mongodb::Database,
    api_key: &UserApiKey,
    truncated_error: &str,
) {
    crate::services::audit_service::log_async(
        db.clone(),
        Some(api_key.user_id.clone()),
        "key_refresh_failed".to_string(),
        Some(serde_json::json!({
            "api_key_id": &api_key.id,
            "provider_config_id": api_key.provider_config_id.as_deref(),
            "connection_id": api_key.connection_id.as_deref(),
            "error": truncated_error,
        })),
        None,
        None,
        None,
        None,
    );
}

pub async fn refresh_user_api_key_in_place(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    api_key: &UserApiKey,
) -> AppResult<UserApiKey> {
    let provider_id = api_key.provider_config_id.as_deref().ok_or_else(|| {
        AppError::Internal(
            "refresh_user_api_key_in_place: UserApiKey missing provider_config_id".to_string(),
        )
    })?;
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id })
        .await?
        .ok_or_else(|| AppError::Internal("Provider config not found for refresh".to_string()))?;

    let token_url = provider.token_url.as_ref().ok_or_else(|| {
        AppError::Internal("OAuth provider missing token_url for refresh".to_string())
    })?;

    // Resolve OAuth client credentials. BYO (Lark) lives on the key
    // itself; provider-owned (codex) lives on ProviderConfig.
    let (client_id, client_secret) = if let Some(enc_cid) =
        api_key.user_oauth_client_id_encrypted.as_ref()
    {
        let dec_cid = Zeroizing::new(encryption_keys.decrypt(enc_cid).await?);
        let cid = String::from_utf8((*dec_cid).clone())
            .map_err(|e| AppError::Internal(format!("Failed to decode key client_id: {e}")))?;
        let secret = if let Some(enc_sec) = api_key.user_oauth_client_secret_encrypted.as_ref() {
            let dec_sec = Zeroizing::new(encryption_keys.decrypt(enc_sec).await?);
            Some(String::from_utf8((*dec_sec).clone()).map_err(|e| {
                AppError::Internal(format!("Failed to decode key client_secret: {e}"))
            })?)
        } else {
            None
        };
        (cid, secret)
    } else {
        let enc_cid = provider.client_id_encrypted.as_ref().ok_or_else(|| {
            AppError::Internal(format!(
                "Provider {} missing client_id_encrypted",
                provider.slug
            ))
        })?;
        let dec_cid = Zeroizing::new(encryption_keys.decrypt(enc_cid).await?);
        let cid = String::from_utf8((*dec_cid).clone())
            .map_err(|e| AppError::Internal(format!("Failed to decode provider client_id: {e}")))?;
        let secret = if let Some(enc_sec) = provider.client_secret_encrypted.as_ref() {
            let dec_sec = Zeroizing::new(encryption_keys.decrypt(enc_sec).await?);
            Some(String::from_utf8((*dec_sec).clone()).map_err(|e| {
                AppError::Internal(format!("Failed to decode provider client_secret: {e}"))
            })?)
        } else {
            None
        };
        (cid, secret)
    };

    let enc_refresh = api_key.refresh_token_encrypted.as_ref().ok_or_else(|| {
        AppError::Internal("UserApiKey missing refresh_token for refresh".to_string())
    })?;
    let dec_refresh = Zeroizing::new(encryption_keys.decrypt(enc_refresh).await?);
    let refresh_token = String::from_utf8((*dec_refresh).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode refresh_token: {e}")))?;

    let use_basic_auth = provider.token_endpoint_auth_method == "client_secret_basic";
    let mut params = vec![
        ("grant_type".to_string(), "refresh_token".to_string()),
        ("refresh_token".to_string(), refresh_token.clone()),
    ];
    if !use_basic_auth {
        params.push(oauth_flow::client_id_form_field(&provider, &client_id));
        if let Some(ref secret) = client_secret {
            params.push(("client_secret".to_string(), secret.clone()));
        }
    }

    let mut request =
        oauth_flow::expect_json_response(oauth_flow::token_exchange_client().post(token_url));
    request = if uses_json_oauth_token_exchange(&provider) {
        request.json(&params_to_json_body(&params))
    } else {
        request.form(&params)
    };
    if use_basic_auth {
        request = request.basic_auth(&client_id, client_secret.as_deref());
    }

    let response = request
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Token refresh request failed: {e}")))?;

    if !response.status().is_success() {
        let now = Utc::now();
        let status = response.status();

        // Transient vs terminal classification (OAuth 2.0 §5.2 error
        // semantics): a 5xx from the IdP or a 429 rate-limit means the
        // refresh token is still valid — the token endpoint is just
        // unavailable right now. Marking the credential `failed` here
        // would force the user to re-authorize over a momentary blip.
        // The proactive refresh sweep makes this worse: it touches many
        // keys on a timer, so a brief provider outage overlapping a sweep
        // tick could mark a whole cohort `failed` at once. Leave the row
        // `active` and return a transient error so the next sweep tick or
        // proxy request retries (the sweep interval is itself the backoff).
        // Only 4xx client errors (invalid_grant / invalid_client) below
        // are treated as terminal and written `status: "failed"`.
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tracing::warn!(
                api_key_id = %api_key.id,
                connection_id = ?api_key.connection_id,
                %status,
                "Token refresh hit a transient IdP error; leaving key active for retry"
            );
            return Err(AppError::Internal(format!(
                "Token refresh transiently failed with status {status}; left active for retry"
            )));
        }

        let body = response.text().await.unwrap_or_default();
        // Chinese error strings from Lark / Feishu (the providers most
        // likely to hit this branch) are multi-byte UTF-8 — a naive
        // `&body[..200]` slice panics whenever a code point straddles
        // the boundary. Truncate by character count instead.
        let truncated: String = body.chars().take(200).collect();

        // Compare-and-set guard: a concurrent successful refresh on the
        // same key races us. If it landed first, `updated_at` has moved
        // off `api_key.updated_at` and we must NOT clobber the
        // freshly-active row with `failed`. The `status` predicate
        // additionally refuses to resurrect a row the user has revoked
        // out from under the refresh (or that a sibling already marked
        // `failed` — same outcome, redundant write avoided).
        let snapshot_updated_at = bson::DateTime::from_chrono(api_key.updated_at);
        let write = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! {
                    "_id": &api_key.id,
                    "updated_at": &snapshot_updated_at,
                    "status": { "$nin": ["revoked", "failed"] },
                },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": format!("Refresh failed: {status} {truncated}"),
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        if write.matched_count > 0 {
            // Surface refresh failure as an audit event so dashboards /
            // operators can detect silently-broken connections without
            // waiting for the user to complain about a 401. Includes
            // `connection_id` and the truncated provider response so the
            // root cause (revoked grant, rotated client_secret, etc.) is
            // visible without a separate DB read.
            emit_key_refresh_failed_audit(
                db,
                api_key,
                &format!("Refresh failed: {status} {truncated}"),
            );
        } else {
            // Lost the race to a concurrent write. Either a sibling
            // refresh succeeded (and the live row is active with a
            // fresh token), or the user revoked the key, or another
            // failure write got there first. In all three cases the
            // live state is more correct than ours.
            tracing::info!(
                api_key_id = %api_key.id,
                connection_id = ?api_key.connection_id,
                "Refresh failure write lost CAS — live row already updated by a concurrent operation"
            );
        }

        return Err(AppError::Internal(format!(
            "Token refresh failed with status {status}"
        )));
    }

    let token_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse refresh response: {e}")))?;

    // Lark / Feishu return HTTP 200 with a non-zero `code` field on
    // refresh failure (e.g. `{code: 99991663, msg: "invalid refresh
    // token"}`). Treat this as a refresh failure: write `status:
    // "failed"` so the wizard polling exits and the user knows the
    // refresh didn't succeed. Without this branch the function would
    // fall through to the missing-access_token error below and leave
    // the row in `active` with a stale token (the exact silent-failure
    // mode the design doc set out to avoid).
    if token_data
        .get("code")
        .and_then(|value| value.as_i64())
        .is_some_and(|code| code != 0)
    {
        let now = Utc::now();
        let msg = token_data
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("provider returned non-zero code");
        // Same UTF-8 safety concern as the HTTP-error branch above —
        // Lark / Feishu `msg` fields are commonly Chinese.
        let truncated: String = msg.chars().take(200).collect();

        // Same CAS guard as the HTTP-error branch — see that branch's
        // comment for the race description.
        let snapshot_updated_at = bson::DateTime::from_chrono(api_key.updated_at);
        let write = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! {
                    "_id": &api_key.id,
                    "updated_at": &snapshot_updated_at,
                    "status": { "$nin": ["revoked", "failed"] },
                },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": format!("Refresh failed: {truncated}"),
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        if write.matched_count > 0 {
            emit_key_refresh_failed_audit(db, api_key, &format!("Refresh failed: {truncated}"));
        } else {
            tracing::info!(
                api_key_id = %api_key.id,
                connection_id = ?api_key.connection_id,
                "Refresh failure write lost CAS — live row already updated by a concurrent operation"
            );
        }

        return Err(AppError::Internal(
            "Token refresh failed (provider returned non-zero code)".to_string(),
        ));
    }

    let payload = oauth_token_payload(&token_data);
    let new_access_token = payload["access_token"].as_str().ok_or_else(|| {
        AppError::Internal("Missing access_token in refresh response".to_string())
    })?;
    let new_refresh_token = payload["refresh_token"].as_str();
    let expires_in = payload["expires_in"].as_i64();
    let new_scope = payload["scope"].as_str();
    let now = Utc::now();

    let access_enc = encryption_keys.encrypt(new_access_token.as_bytes()).await?;
    let mut set_doc = doc! {
        "access_token_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: access_enc,
        },
        "status": "active",
        "error_message": bson::Bson::Null,
        "last_used_at": bson::DateTime::from_chrono(now),
        "updated_at": bson::DateTime::from_chrono(now),
    };
    if let Some(exp) = expires_in {
        let new_expires = now + Duration::seconds(exp);
        set_doc.insert("expires_at", bson::DateTime::from_chrono(new_expires));
    }
    if let Some(rt) = new_refresh_token {
        let rt_enc = encryption_keys.encrypt(rt.as_bytes()).await?;
        set_doc.insert(
            "refresh_token_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: rt_enc,
            },
        );
    }
    if let Some(scope) = new_scope {
        set_doc.insert("token_scopes", scope);
    }

    // Status predicate refuses to resurrect a row a sibling write has
    // moved to a terminal state (`revoked` or `failed`). Without it, a
    // concurrent revoke could be overwritten by this success write,
    // re-activating a credential the user just told us to drop.
    // Concurrent successful refreshes keep last-write-wins (see the
    // function-level rustdoc) — both writes have valid token material,
    // so a later one overwriting an earlier one is fine.
    db.collection::<UserApiKey>(USER_API_KEYS)
        .update_one(
            doc! {
                "_id": &api_key.id,
                "status": { "$nin": ["revoked", "failed"] },
            },
            doc! { "$set": set_doc },
        )
        .await?;

    let refreshed = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": &api_key.id })
        .await?
        .ok_or_else(|| {
            AppError::Internal(
                "UserApiKey disappeared after refresh_user_api_key_in_place".to_string(),
            )
        })?;

    tracing::info!(
        user_id = %api_key.user_id,
        connection_id = ?api_key.connection_id,
        provider_id = %provider_id,
        "UserApiKey OAuth tokens refreshed in place (multi-connection path)"
    );

    Ok(refreshed)
}

/// Tally of a single proactive refresh sweep, returned for logging/tests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshSweepReport {
    /// Candidate keys that matched the expiring-soon query.
    pub considered: usize,
    /// Keys whose access token was refreshed successfully.
    pub refreshed: usize,
    /// Keys whose refresh attempt failed (already marked failed in place
    /// by `refresh_user_api_key_in_place`; counted here for visibility).
    pub failed: usize,
}

/// Proactively refresh multi-connection OAuth access tokens that expire
/// within `window`. Runs on a timer from `main.rs` so a token stays warm
/// even for services that aren't proxied frequently, and so a dead
/// refresh token (e.g. a revoked Google grant) surfaces as `status:
/// "failed"` promptly instead of on the next user proxy attempt.
///
/// Scope is intentionally limited to rows that:
///   - are `credential_type == "oauth2"` and `status == "active"`
///   - carry both a `refresh_token` and a `connection_id` (the
///     multi-connection path `refresh_user_api_key_in_place` handles;
///     legacy `connection_id: null` rows still refresh lazily at proxy
///     time and are left alone here to avoid the provider-token sync
///     dance)
///   - have an `expires_at` at or before `now + window`
///
/// This does NOT extend refresh-token lifetime: a Google app left in
/// "Testing" publishing status still expires its refresh tokens after 7
/// days, and a connection authorized before refresh tokens were issued
/// has nothing to refresh. Both cases land as `status: "failed"` here.
///
/// Concurrency: `refresh_user_api_key_in_place` is not lock-serialized
/// against the proxy-time lazy-refresh path (it documents "callers should
/// not invoke concurrently for the same key"). The expiry-window filter
/// makes a collision unlikely — a key the proxy just refreshed has a
/// ~1h expiry and won't match the window — and for non-rotating providers
/// (Google, LinkedIn) a duplicate refresh is merely a wasted call. If a
/// rotating-refresh-token provider (Auth0 / Okta / GitHub App style) is
/// ever added as a BYO catalog entry, a per-credential single-flight lock
/// shared with the proxy path should be introduced first to avoid a
/// reuse-detection `invalid_grant` from a sweep+proxy race. Tracked as a
/// follow-up; not a concern for the current Google / Lark use cases.
pub async fn refresh_expiring_oauth_keys(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    window: Duration,
) -> AppResult<RefreshSweepReport> {
    let deadline = Utc::now() + window;
    let candidates: Vec<UserApiKey> = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find(doc! {
            "credential_type": "oauth2",
            "status": "active",
            "connection_id": { "$ne": null },
            "refresh_token_encrypted": { "$ne": null },
            "expires_at": { "$ne": null, "$lte": bson::DateTime::from_chrono(deadline) },
        })
        .await?
        .try_collect()
        .await?;

    let mut report = RefreshSweepReport {
        considered: candidates.len(),
        ..Default::default()
    };

    for key in &candidates {
        match refresh_user_api_key_in_place(db, encryption_keys, key).await {
            Ok(_) => report.refreshed += 1,
            Err(error) => {
                // `refresh_user_api_key_in_place` already persists a
                // `status: "failed"` + audit event for provider-side
                // (4xx/Lark-code) failures. Internal errors (decrypt,
                // network) leave the row active so a later sweep or
                // proxy-time refresh can retry. Either way, don't let one
                // bad key abort the rest of the sweep.
                report.failed += 1;
                tracing::warn!(
                    user_id = %key.user_id,
                    connection_id = ?key.connection_id,
                    error = %error,
                    "proactive OAuth refresh failed for key; continuing sweep"
                );
            }
        }
    }

    if report.considered > 0 {
        tracing::info!(
            considered = report.considered,
            refreshed = report.refreshed,
            failed = report.failed,
            "proactive OAuth refresh sweep complete"
        );
    }

    Ok(report)
}

/// Get a user's decrypted token for a provider, with lazy refresh for OAuth tokens.
pub async fn get_active_token(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
) -> AppResult<DecryptedProviderToken> {
    let token = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
            "status": { "$in": ["active", "expired"] },
        })
        .await?
        .ok_or_else(|| AppError::NotFound("No active token found for this provider".to_string()))?;

    // Update last_used_at
    let now = Utc::now();
    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &token.id },
            doc! { "$set": { "last_used_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    match token.token_type.as_str() {
        "api_key" => {
            let encrypted = token.api_key_encrypted.ok_or_else(|| {
                AppError::Internal("API key token missing encrypted key".to_string())
            })?;
            let decrypted_bytes = Zeroizing::new(encryption_keys.decrypt(&encrypted).await?);
            let decrypted = String::from_utf8((*decrypted_bytes).clone())
                .map_err(|e| AppError::Internal(format!("Failed to decode API key: {e}")))?;

            Ok(DecryptedProviderToken {
                token_type: "api_key".to_string(),
                access_token: None,
                api_key: Some(decrypted),
            })
        }
        "oauth2" => {
            // Check if token needs refresh (5-minute buffer)
            let needs_refresh = token
                .expires_at
                .is_some_and(|exp| exp <= now + Duration::minutes(5));

            if needs_refresh && token.refresh_token_encrypted.is_some() {
                match oauth_flow::refresh_oauth_token(db, encryption_keys, &token).await {
                    Ok(new_access_token) => {
                        return Ok(DecryptedProviderToken {
                            token_type: "oauth2".to_string(),
                            access_token: Some(new_access_token),
                            api_key: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            user_id = %user_id,
                            provider_id = %provider_id,
                            error = %e,
                            "Token refresh failed, attempting to use existing token"
                        );
                        // Fall through to return existing token
                    }
                }
            }

            let encrypted = token.access_token_encrypted.ok_or_else(|| {
                AppError::Internal("OAuth token missing encrypted access_token".to_string())
            })?;
            let decrypted_bytes = Zeroizing::new(encryption_keys.decrypt(&encrypted).await?);
            let decrypted = String::from_utf8((*decrypted_bytes).clone())
                .map_err(|e| AppError::Internal(format!("Failed to decode access token: {e}")))?;

            Ok(DecryptedProviderToken {
                token_type: "oauth2".to_string(),
                access_token: Some(decrypted),
                api_key: None,
            })
        }
        other => Err(AppError::Internal(format!("Unknown token type: {other}"))),
    }
}

/// Revoke and delete a user's stored token for a provider.
///
/// Attempts best-effort remote token revocation before clearing local state.
pub async fn disconnect_provider(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_id: &str,
) -> AppResult<()> {
    let now = Utc::now();

    // Load the token before marking as revoked (for remote revocation)
    let token = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
            "status": { "$ne": "revoked" },
        })
        .await?;

    // Best-effort remote revocation for OAuth2 tokens
    if let Some(ref tok) = token
        && tok.token_type == "oauth2"
    {
        let provider = db
            .collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find_one(doc! { "_id": provider_id })
            .await?;
        if let Some(ref provider) = provider
            && provider.revocation_url.is_some()
        {
            let _ = try_revoke_token_remote(db, encryption_keys, provider, tok).await;
        }
    }

    let result = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .update_one(
            doc! {
                "user_id": user_id,
                "provider_config_id": provider_id,
                "status": { "$ne": "revoked" },
            },
            doc! { "$set": {
                "status": "revoked",
                "api_key_encrypted": bson::Bson::Null,
                "access_token_encrypted": bson::Bson::Null,
                "refresh_token_encrypted": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "No active token found for this provider".to_string(),
        ));
    }

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "Provider disconnected"
    );

    Ok(())
}

/// Best-effort remote token revocation (RFC 7009).
///
/// Resolves OAuth client credentials so the revocation request includes proper
/// client authentication (`client_secret_basic` or `client_secret_post`).
/// If credential resolution fails, revocation is silently skipped.
async fn try_revoke_token_remote(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    token: &UserProviderToken,
) {
    let revocation_url = match provider.revocation_url.as_deref() {
        Some(url) => url,
        None => return,
    };

    // Resolve the same OAuth credentials that were used to mint this token.
    // If resolution fails (e.g. credentials deleted), skip revocation silently.
    let creds = match user_credentials_service::resolve_token_oauth_credentials(
        db,
        encryption_keys,
        provider,
        token.credential_user_id.as_deref(),
    )
    .await
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let use_basic_auth = provider.token_endpoint_auth_method == "client_secret_basic";

    // Try revoking access token
    if let Some(ref enc) = token.access_token_encrypted
        && let Ok(decrypted) = encryption_keys.decrypt(enc).await
        && let Ok(access_token) = String::from_utf8(decrypted)
    {
        let _ = send_revocation_request(
            revocation_url,
            &access_token,
            "access_token",
            &creds.client_id,
            creds.client_secret.as_deref(),
            use_basic_auth,
            oauth_flow::client_id_param_name(provider),
        )
        .await;
    }

    // Try revoking refresh token
    if let Some(ref enc) = token.refresh_token_encrypted
        && let Ok(decrypted) = encryption_keys.decrypt(enc).await
        && let Ok(refresh_token) = String::from_utf8(decrypted)
    {
        let _ = send_revocation_request(
            revocation_url,
            &refresh_token,
            "refresh_token",
            &creds.client_id,
            creds.client_secret.as_deref(),
            use_basic_auth,
            oauth_flow::client_id_param_name(provider),
        )
        .await;
    }
}

/// Send a single RFC 7009 revocation request with client authentication.
async fn send_revocation_request(
    revocation_url: &str,
    token_value: &str,
    token_type_hint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    use_basic_auth: bool,
    client_id_param_name: &str,
) -> Result<(), ()> {
    let client = oauth_flow::token_exchange_client();

    let mut request = client.post(revocation_url);

    if use_basic_auth {
        request = request.basic_auth(client_id, client_secret);
        request = request.form(&[("token", token_value), ("token_type_hint", token_type_hint)]);
    } else {
        let mut params = vec![
            ("token".to_string(), token_value.to_string()),
            ("token_type_hint".to_string(), token_type_hint.to_string()),
            (client_id_param_name.to_string(), client_id.to_string()),
        ];
        if let Some(secret) = client_secret {
            params.push(("client_secret".to_string(), secret.to_string()));
        }
        request = request.form(&params);
    }

    let _ = request.send().await;
    Ok(())
}

fn build_user_token_summary(
    token: &UserProviderToken,
    provider: Option<&ProviderConfig>,
) -> UserProviderTokenSummary {
    let (provider_name, provider_slug, provider_type) = match provider {
        Some(p) => (p.name.clone(), p.slug.clone(), p.provider_type.clone()),
        None => (
            "Unknown".to_string(),
            "unknown".to_string(),
            token.token_type.clone(),
        ),
    };

    UserProviderTokenSummary {
        provider_config_id: token.provider_config_id.clone(),
        provider_name,
        provider_slug,
        provider_type,
        token_type: token.token_type.clone(),
        status: token.status.clone(),
        label: token.label.clone(),
        gateway_url: token.gateway_url.clone(),
        expires_at: token.expires_at.map(|dt| dt.to_rfc3339()),
        last_used_at: token.last_used_at.map(|dt| dt.to_rfc3339()),
        connected_at: token.created_at.to_rfc3339(),
        metadata: token.metadata.clone(),
    }
}

/// List all providers the user has connected to, with status.
///
/// Uses a single batch query for provider lookups (CR-4/5/6: fix N+1).
pub async fn list_user_tokens(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<UserProviderTokenSummary>> {
    let tokens: Vec<UserProviderToken> = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "status": { "$ne": "revoked" } })
        .await?
        .try_collect()
        .await?;

    if tokens.is_empty() {
        return Ok(vec![]);
    }

    // Batch fetch all providers in a single query
    let provider_ids: Vec<&str> = tokens
        .iter()
        .map(|t| t.provider_config_id.as_str())
        .collect();
    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find(doc! { "_id": { "$in": &provider_ids } })
        .await?
        .try_collect()
        .await?;
    let provider_map: HashMap<&str, &ProviderConfig> =
        providers.iter().map(|p| (p.id.as_str(), p)).collect();

    let summaries = tokens
        .iter()
        .map(|token| {
            build_user_token_summary(
                token,
                provider_map.get(token.provider_config_id.as_str()).copied(),
            )
        })
        .collect();

    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::{
        DevicePollFlow, build_telegram_identity_metadata, build_telegram_identity_update_doc,
        build_user_token_summary, classify_device_poll_failure, ensure_additional_scopes_supported,
        merge_scopes, normalize_telegram_bot_api_key, oauth_token_payload, params_to_json_body,
        parse_additional_scopes, parse_token_exchange_response, resolve_scope_param,
        token_exchange_provider_error, uses_json_oauth_token_exchange,
    };
    use crate::crypto::telegram::TelegramLoginData;
    use crate::errors::AppError;
    use crate::models::provider_config::ProviderConfig;
    use crate::models::user_provider_token::UserProviderToken;
    use chrono::Utc;
    use mongodb::bson::Bson;
    use std::collections::HashMap;

    fn make_provider(provider_type: &str) -> ProviderConfig {
        ProviderConfig {
            id: "provider-1".to_string(),
            slug: "telegram".to_string(),
            name: "Telegram".to_string(),
            description: None,
            provider_type: provider_type.to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: Some(vec![1, 2, 3]),
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: Some("NyxIdBot".to_string()),
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_token(token_type: &str) -> UserProviderToken {
        let mut metadata = HashMap::new();
        metadata.insert("username".to_string(), "nyx_user".to_string());

        UserProviderToken {
            id: "token-1".to_string(),
            user_id: "user-1".to_string(),
            provider_config_id: "provider-1".to_string(),
            connection_id: None,
            credential_user_id: None,
            token_type: token_type.to_string(),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: Some(metadata),
            gateway_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn lark_and_feishu_token_exchange_use_json_body() {
        let mut provider = make_provider("oauth2");
        provider.slug = "lark".to_string();
        assert!(uses_json_oauth_token_exchange(&provider));

        provider.slug = "feishu".to_string();
        assert!(uses_json_oauth_token_exchange(&provider));
    }

    #[test]
    fn lark_token_exchange_detection_matches_known_endpoint_urls() {
        let mut provider = make_provider("oauth2");
        provider.slug = "custom-lark".to_string();
        provider.token_url =
            Some("https://open.larksuite.com/open-apis/authen/v2/oauth/token".to_string());
        assert!(uses_json_oauth_token_exchange(&provider));

        provider.token_url =
            Some("https://open.feishu.cn/open-apis/authen/v2/oauth/token".to_string());
        assert!(uses_json_oauth_token_exchange(&provider));
    }

    #[test]
    fn standard_oauth_token_exchange_uses_form_body() {
        let provider = make_provider("oauth2");
        assert!(!uses_json_oauth_token_exchange(&provider));
    }

    #[test]
    fn params_to_json_body_preserves_token_exchange_fields() {
        let params = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), "abc123".to_string()),
            (
                "redirect_uri".to_string(),
                "http://localhost/cb".to_string(),
            ),
        ];

        let body = params_to_json_body(&params);

        assert_eq!(body["grant_type"], "authorization_code");
        assert_eq!(body["code"], "abc123");
        assert_eq!(body["redirect_uri"], "http://localhost/cb");
    }

    #[test]
    fn oauth_token_payload_supports_standard_and_lark_shapes() {
        let standard = serde_json::json!({
            "access_token": "standard-access",
            "refresh_token": "standard-refresh",
        });
        assert_eq!(
            oauth_token_payload(&standard)["access_token"],
            "standard-access"
        );

        let lark = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "access_token": "lark-access",
                "refresh_token": "lark-refresh",
                "expires_in": 7200,
            }
        });
        let payload = oauth_token_payload(&lark);
        assert_eq!(payload["access_token"], "lark-access");
        assert_eq!(payload["refresh_token"], "lark-refresh");
        assert_eq!(payload["expires_in"], 7200);
    }

    /// Regression for issue #694: Lark / Feishu return HTTP 200 with a
    /// non-zero `code` + `msg` instead of an OAuth error envelope. Before the
    /// fix this surfaced as `AppError::Internal`, which `safe_error_message`
    /// flattens to the generic "An internal error occurred" string, leaving
    /// the wizard with no clue why the credential landed in `failed`. The
    /// parsed error must now be a surfaceable variant carrying the provider's
    /// own `code`/`msg`.
    #[test]
    fn lark_style_200_with_nonzero_code_surfaces_provider_message() {
        // Lark sends HTTP 200 even on failure.
        let body = r#"{"code": 99991663, "msg": "app ticket invalid"}"#;
        let err = parse_token_exchange_response(reqwest::StatusCode::OK, body)
            .expect_err("non-zero Lark code must be an error");

        // Must NOT be Internal/DatabaseError, otherwise safe_error_message
        // would hide it behind the generic string.
        assert!(
            matches!(err, AppError::BadRequest(_)),
            "expected surfaceable BadRequest, got {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("99991663"), "missing provider code: {msg}");
        assert!(
            msg.contains("app ticket invalid"),
            "missing provider msg: {msg}"
        );
    }

    /// `safe_error_message` (in `handlers/user_tokens.rs`) only flattens
    /// `Internal`/`DatabaseError`. Verify the Lark error is none of those, so
    /// the actionable text reaches the user-facing redirect.
    #[test]
    fn lark_error_passes_through_safe_error_filter() {
        let body = r#"{"code": 20029, "msg": "redirect_uri mismatch"}"#;
        let err = parse_token_exchange_response(reqwest::StatusCode::OK, body)
            .expect_err("non-zero Lark code must be an error");
        // Mirror safe_error_message's filter: only Internal/DatabaseError are masked.
        let masked = matches!(err, AppError::Internal(_) | AppError::DatabaseError(_));
        assert!(!masked, "Lark error should not be masked as internal");
        assert!(err.to_string().contains("redirect_uri mismatch"));
    }

    #[test]
    fn missing_access_token_without_provider_error_is_actionable_bad_request() {
        // 200 OK, valid JSON, but no access_token and no provider error code.
        let body = r#"{"token_type": "bearer"}"#;
        let value = parse_token_exchange_response(reqwest::StatusCode::OK, body)
            .expect("body with no provider error parses as Ok value");
        // Caller path: no access_token -> token_exchange_provider_error is None
        // -> falls back to the generic-but-actionable BadRequest.
        assert!(token_exchange_provider_error(&value).is_none());
    }

    #[test]
    fn standard_oauth_error_envelope_surfaces_description() {
        let body = r#"{"error": "invalid_grant", "error_description": "code expired"}"#;
        let err = parse_token_exchange_response(reqwest::StatusCode::BAD_REQUEST, body)
            .expect_err("OAuth error envelope must be an error");
        assert!(matches!(err, AppError::BadRequest(_)));
        let msg = err.to_string();
        assert!(msg.contains("invalid_grant"), "msg: {msg}");
        assert!(msg.contains("code expired"), "msg: {msg}");
    }

    #[test]
    fn non_success_without_envelope_does_not_leak_raw_body() {
        // A 500 with an HTML/transport body and no recognizable envelope must
        // NOT echo the raw body to the user.
        let body = "<html><body>internal proxy stack trace: secret.db.host</body></html>";
        let err = parse_token_exchange_response(reqwest::StatusCode::INTERNAL_SERVER_ERROR, body)
            .expect_err("non-2xx without envelope must be an error");
        assert!(matches!(err, AppError::BadRequest(_)));
        let msg = err.to_string();
        assert!(
            !msg.contains("secret.db.host") && !msg.contains("stack trace"),
            "raw body must not leak into user message: {msg}"
        );
        assert!(msg.contains("500"), "status code should be surfaced: {msg}");
    }

    #[test]
    fn successful_standard_response_parses_through() {
        let body = r#"{"access_token": "abc", "token_type": "bearer", "expires_in": 3600}"#;
        let value = parse_token_exchange_response(reqwest::StatusCode::OK, body)
            .expect("valid token response parses");
        assert_eq!(value["access_token"], "abc");
    }

    #[test]
    fn classify_device_poll_failure_handles_flow_control() {
        let status = reqwest::StatusCode::BAD_REQUEST;

        // Pending
        let res =
            classify_device_poll_failure(status, r#"{"error":"authorization_pending"}"#).unwrap();
        assert_eq!(res, DevicePollFlow::Pending);

        // Slow Down
        let res = classify_device_poll_failure(status, r#"{"error":"slow_down"}"#).unwrap();
        assert_eq!(res, DevicePollFlow::SlowDown);

        // Expired
        let res = classify_device_poll_failure(status, r#"{"error":"expired_token"}"#).unwrap();
        assert_eq!(res, DevicePollFlow::Expired);

        // Denied
        let res = classify_device_poll_failure(status, r#"{"error":"access_denied"}"#).unwrap();
        assert_eq!(res, DevicePollFlow::Denied);
    }

    #[test]
    fn classify_device_poll_failure_surfaces_provider_error() {
        let status = reqwest::StatusCode::BAD_REQUEST;

        // Standard OAuth provider error (RFC 6749)
        let body = r#"{"error":"invalid_client","error_description":"client secret mismatch"}"#;
        let err = classify_device_poll_failure(status, body).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        let msg = err.to_string();
        assert!(msg.contains("invalid_client"));
        assert!(msg.contains("client secret mismatch"));

        // Lark-style non-zero code error envelope
        let body = r#"{"code": 20029, "msg": "redirect_uri mismatch"}"#;
        let err = classify_device_poll_failure(status, body).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        let msg = err.to_string();
        assert!(msg.contains("20029"));
        assert!(msg.contains("redirect_uri mismatch"));
    }

    #[test]
    fn classify_device_poll_failure_handles_opaque_failures_without_leak() {
        let status = reqwest::StatusCode::INTERNAL_SERVER_ERROR;
        let body = "<html><body>sensitive raw response stacktrace with secrets</body></html>";
        let err = classify_device_poll_failure(status, body).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        let msg = err.to_string();
        assert!(msg.contains("500"));
        assert!(!msg.contains("sensitive"));
        assert!(!msg.contains("secrets"));
        assert!(!msg.contains("stacktrace"));
    }

    #[test]
    fn parse_additional_scopes_none_and_empty() {
        assert!(parse_additional_scopes(None).unwrap().is_empty());
        assert!(parse_additional_scopes(Some("")).unwrap().is_empty());
        assert!(parse_additional_scopes(Some("   ")).unwrap().is_empty());
        assert!(parse_additional_scopes(Some(", ,")).unwrap().is_empty());
    }

    #[test]
    fn parse_additional_scopes_splits_comma_and_whitespace() {
        let scopes = parse_additional_scopes(Some(
            "contact:contact.base:readonly, contact:department.base:readonly attendance:record:read",
        ))
        .unwrap();
        assert_eq!(
            scopes,
            vec![
                "contact:contact.base:readonly".to_string(),
                "contact:department.base:readonly".to_string(),
                "attendance:record:read".to_string(),
            ]
        );
    }

    #[test]
    fn parse_additional_scopes_accepts_google_style_urls() {
        let scopes =
            parse_additional_scopes(Some("https://www.googleapis.com/auth/drive.readonly"))
                .unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0], "https://www.googleapis.com/auth/drive.readonly");
    }

    #[test]
    fn parse_additional_scopes_rejects_invalid_chars() {
        let err = parse_additional_scopes(Some("ok,bad<scope>")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid characters"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn parse_additional_scopes_rejects_too_many() {
        let many = (0..100)
            .map(|i| format!("scope{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_additional_scopes(Some(&many)).is_err());
    }

    #[test]
    fn parse_additional_scopes_rejects_overlong_scope() {
        let huge = "a".repeat(257);
        assert!(parse_additional_scopes(Some(&huge)).is_err());
    }

    #[test]
    fn resolve_scope_param_override_replaces_defaults() {
        // The picker sends a complete set that drops a default (`email`).
        let defaults = vec!["openid".to_string(), "email".to_string()];
        let override_set = vec!["openid".to_string(), "tweet.read".to_string()];
        let resolved = resolve_scope_param(Some(&defaults), &[], Some(&override_set));
        assert_eq!(resolved, Some("openid tweet.read".to_string()));
    }

    #[test]
    fn resolve_scope_param_empty_override_omits_param() {
        // User cleared every scope -> omit `scope` rather than emit empty.
        let defaults = vec!["openid".to_string()];
        let resolved = resolve_scope_param(Some(&defaults), &[], Some(&[]));
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_scope_param_override_wins_over_additive() {
        // If both are somehow supplied, override takes precedence.
        let defaults = vec!["openid".to_string()];
        let additional = vec!["should_be_ignored".to_string()];
        let override_set = vec!["only".to_string(), "these".to_string()];
        let resolved = resolve_scope_param(Some(&defaults), &additional, Some(&override_set));
        assert_eq!(resolved, Some("only these".to_string()));
    }

    #[test]
    fn resolve_scope_param_no_override_no_additional_uses_defaults() {
        let defaults = vec!["openid".to_string(), "email".to_string()];
        assert_eq!(
            resolve_scope_param(Some(&defaults), &[], None),
            Some("openid email".to_string())
        );
        // Byte-identical pre-feature behavior: Some(vec![]) -> emit empty.
        assert_eq!(
            resolve_scope_param(Some(&vec![]), &[], None),
            Some(String::new())
        );
        // No defaults at all -> omit the param.
        assert_eq!(resolve_scope_param(None, &[], None), None);
    }

    #[test]
    fn resolve_scope_param_no_override_with_additional_merges() {
        let defaults = vec!["openid".to_string()];
        let additional = vec!["profile".to_string(), "openid".to_string()];
        // Merge + dedupe, defaults first.
        assert_eq!(
            resolve_scope_param(Some(&defaults), &additional, None),
            Some("openid profile".to_string())
        );
    }

    #[test]
    fn merge_scopes_preserves_defaults_and_appends_extras() {
        let defaults = vec!["openid".to_string(), "email".to_string()];
        let extras = vec![
            "profile".to_string(),
            "email".to_string(), // duplicate
            "offline_access".to_string(),
        ];
        let merged = merge_scopes(Some(&defaults), &extras);
        assert_eq!(
            merged,
            vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
                "offline_access".to_string(),
            ]
        );
    }

    #[test]
    fn merge_scopes_handles_no_defaults() {
        let extras = vec!["scope-a".to_string(), "scope-b".to_string()];
        let merged = merge_scopes(None, &extras);
        assert_eq!(merged, extras);
    }

    #[test]
    fn merge_scopes_handles_no_extras() {
        let defaults = vec!["openid".to_string()];
        let merged = merge_scopes(Some(&defaults), &[]);
        assert_eq!(merged, defaults);
    }

    #[test]
    fn ensure_additional_scopes_supported_allows_oauth2() {
        let provider = make_provider("oauth2");
        assert!(ensure_additional_scopes_supported(&provider, &["scope-a".to_string()]).is_ok());
    }

    #[test]
    fn ensure_additional_scopes_supported_allows_rfc8628_device_code() {
        let mut provider = make_provider("device_code");
        provider.device_code_format = "rfc8628".to_string();
        assert!(ensure_additional_scopes_supported(&provider, &["scope-a".to_string()]).is_ok());
    }

    #[test]
    fn ensure_additional_scopes_supported_rejects_openai_device_code() {
        let mut provider = make_provider("device_code");
        provider.device_code_format = "openai".to_string();
        let err = ensure_additional_scopes_supported(&provider, &["foo".to_string()])
            .expect_err("openai device_code must reject additional scopes");
        let msg = err.to_string();
        assert!(
            msg.contains("does not accept additional OAuth scopes"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn ensure_additional_scopes_supported_allows_empty_even_for_openai() {
        // Backwards-compatible: never fail when no extras were provided, even
        // on providers that otherwise reject scope forwarding.
        let mut provider = make_provider("device_code");
        provider.device_code_format = "openai".to_string();
        assert!(ensure_additional_scopes_supported(&provider, &[]).is_ok());
    }

    #[test]
    fn ensure_additional_scopes_supported_rejects_api_key_provider() {
        let provider = make_provider("api_key");
        let err = ensure_additional_scopes_supported(&provider, &["foo".to_string()])
            .expect_err("api_key providers must reject scopes");
        let msg = err.to_string();
        assert!(
            msg.contains("does not support OAuth scopes"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn summary_uses_provider_type_and_preserves_metadata() {
        let provider = make_provider("telegram_widget");
        let token = make_token("telegram_identity");

        let summary = build_user_token_summary(&token, Some(&provider));

        assert_eq!(summary.provider_type, "telegram_widget");
        assert_eq!(summary.token_type, "telegram_identity");
        assert_eq!(
            summary
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("username")),
            Some(&"nyx_user".to_string())
        );
    }

    #[test]
    fn summary_falls_back_to_token_type_when_provider_is_missing() {
        let token = make_token("api_key");

        let summary = build_user_token_summary(&token, None);

        assert_eq!(summary.provider_type, "api_key");
        assert_eq!(summary.provider_name, "Unknown");
        assert_eq!(summary.provider_slug, "unknown");
    }

    #[test]
    fn telegram_identity_metadata_omits_missing_optional_fields() {
        let data = TelegramLoginData {
            id: 12345,
            first_name: "Nyx".to_string(),
            last_name: None,
            username: None,
            photo_url: None,
            auth_date: Utc::now().timestamp(),
            hash: "hash".to_string(),
        };

        let metadata = build_telegram_identity_metadata(&data);

        assert_eq!(metadata.get("telegram_user_id"), Some(&"12345".to_string()));
        assert_eq!(metadata.get("first_name"), Some(&"Nyx".to_string()));
        assert!(!metadata.contains_key("last_name"));
        assert!(!metadata.contains_key("username"));
        assert!(!metadata.contains_key("photo_url"));
    }

    #[test]
    fn telegram_identity_update_replaces_metadata_document() {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_user_id".to_string(), "12345".to_string());
        metadata.insert("first_name".to_string(), "Nyx".to_string());

        let update = build_telegram_identity_update_doc(&metadata, Utc::now()).expect("update doc");

        assert_eq!(update.get_str("status").unwrap(), "active");
        assert_eq!(update.get("error_message"), Some(&Bson::Null));
        assert!(update.get("metadata.username").is_none());
        assert_eq!(
            update
                .get_document("metadata")
                .unwrap()
                .get_str("first_name")
                .unwrap(),
            "Nyx"
        );
    }

    #[test]
    fn normalize_telegram_bot_api_key_trims_surrounding_whitespace() {
        let normalized = normalize_telegram_bot_api_key(" 123456:ABC-DEF123 \n")
            .expect("token should normalize");

        assert_eq!(normalized, "123456:ABC-DEF123");
    }

    #[test]
    fn normalize_telegram_bot_api_key_rejects_whitespace_and_path_breakers() {
        let whitespace = normalize_telegram_bot_api_key("123456:ABC DEF")
            .expect_err("whitespace should be rejected");
        assert!(whitespace.to_string().contains("whitespace"));

        let slash =
            normalize_telegram_bot_api_key("123456:ABC/DEF").expect_err("slash should be rejected");
        assert!(slash.to_string().contains("invalid characters"));

        let percent = normalize_telegram_bot_api_key("123456:ABC%2FDEF")
            .expect_err("percent-encoded slash should be rejected");
        assert!(percent.to_string().contains("invalid characters"));
    }

    // ───────────────────────────────────────────────────────────────────
    // refresh_user_api_key_in_place tests (multi-connection refresh path)
    // ───────────────────────────────────────────────────────────────────

    use super::{Duration, PROVIDER_CONFIGS};
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::test_utils::{connect_test_database, test_encryption_keys};
    use mongodb::bson::doc;
    use uuid::Uuid;

    async fn spawn_token_server(
        response: serde_json::Value,
        status: axum::http::StatusCode,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(move || {
                let resp = response.clone();
                async move { (status, axum::Json(resp)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/token"), handle)
    }

    fn make_test_provider(
        id: &str,
        token_url: &str,
        client_id_encrypted: Option<Vec<u8>>,
        client_secret_encrypted: Option<Vec<u8>>,
    ) -> ProviderConfig {
        ProviderConfig {
            id: id.to_string(),
            slug: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://example.com/authorize".to_string()),
            token_url: Some(token_url.to_string()),
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted,
            client_secret_encrypted,
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    async fn insert_pending_user_api_key(
        db: &mongodb::Database,
        encryption_keys: &crate::crypto::aes::EncryptionKeys,
        provider_config_id: &str,
        user_oauth_client_id: Option<&str>,
        user_oauth_client_secret: Option<&str>,
    ) -> UserApiKey {
        let connection_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();
        let refresh_enc = encryption_keys
            .encrypt(b"stored-refresh-token")
            .await
            .unwrap();
        let access_enc = encryption_keys.encrypt(b"stale-access").await.unwrap();
        let user_client_id_enc = match user_oauth_client_id {
            Some(cid) => Some(encryption_keys.encrypt(cid.as_bytes()).await.unwrap()),
            None => None,
        };
        let user_client_secret_enc = match user_oauth_client_secret {
            Some(s) => Some(encryption_keys.encrypt(s.as_bytes()).await.unwrap()),
            None => None,
        };
        let now = Utc::now();
        let key = UserApiKey {
            id: key_id,
            user_id: Uuid::new_v4().to_string(),
            label: "test-key".to_string(),
            credential_type: "oauth2".to_string(),
            credential_encrypted: None,
            access_token_encrypted: Some(access_enc),
            refresh_token_encrypted: Some(refresh_enc),
            token_scopes: Some("openid".to_string()),
            expires_at: Some(now - Duration::minutes(1)),
            provider_config_id: Some(provider_config_id.to_string()),
            connection_id: Some(connection_id),
            user_oauth_client_id_encrypted: user_client_id_enc,
            user_oauth_client_secret_encrypted: user_client_secret_enc,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        key
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_provider_owned_creds() {
        let Some(db) = connect_test_database("refresh_in_place_provider_creds").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({
                "access_token": "fresh-access-token",
                "refresh_token": "fresh-refresh-token",
                "expires_in": 3600,
                "scope": "openid profile",
            }),
            axum::http::StatusCode::OK,
        )
        .await;

        // Provider-owned creds (codex/OpenAI scenario): provider has
        // client_id_encrypted, the UserApiKey does NOT carry BYO creds.
        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(&provider_id, &token_url, Some(admin_cid_enc), None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let key =
            insert_pending_user_api_key(&db, &encryption_keys, &provider_id, None, None).await;

        let refreshed = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect("refresh should succeed");

        assert_eq!(refreshed.status, "active");
        assert!(refreshed.error_message.is_none());
        // Access token updated to the mock's value (decrypt to verify).
        let bytes = encryption_keys
            .decrypt(refreshed.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "fresh-access-token");
        assert_eq!(refreshed.token_scopes.as_deref(), Some("openid profile"));
        // expires_at advanced past now.
        assert!(refreshed.expires_at.unwrap() > Utc::now());
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_byo_creds_lark() {
        let Some(db) = connect_test_database("refresh_in_place_byo_creds").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({
                "access_token": "byo-access-token",
                "refresh_token": "byo-refresh-token",
                "expires_in": 7200,
            }),
            axum::http::StatusCode::OK,
        )
        .await;

        // Lark scenario: provider has NO admin client_id; the UserApiKey
        // carries the user-provided OAuth client_id/secret.
        let provider_id = Uuid::new_v4().to_string();
        let provider = make_test_provider(&provider_id, &token_url, None, None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let key = insert_pending_user_api_key(
            &db,
            &encryption_keys,
            &provider_id,
            Some("byo-client-id"),
            Some("byo-client-secret"),
        )
        .await;

        let refreshed = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect("BYO refresh should succeed");

        assert_eq!(refreshed.status, "active");
        let bytes = encryption_keys
            .decrypt(refreshed.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "byo-access-token");
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_writes_failed_on_4xx() {
        let Some(db) = connect_test_database("refresh_in_place_failure").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({"error": "invalid_grant"}),
            axum::http::StatusCode::UNAUTHORIZED,
        )
        .await;

        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(&provider_id, &token_url, Some(admin_cid_enc), None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let key =
            insert_pending_user_api_key(&db, &encryption_keys, &provider_id, None, None).await;

        let err = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect_err("expected Err on 4xx refresh");
        assert!(matches!(err, AppError::Internal(_)));

        // Status persisted as "failed" (not "refresh_failed" — see doc §4.7).
        let updated = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "failed");
        assert!(
            updated
                .error_message
                .as_deref()
                .is_some_and(|m| m.contains("401"))
        );
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_leaves_key_active_on_5xx() {
        // A transient IdP 5xx must NOT mark the credential terminally
        // failed — the refresh token is still valid and the next attempt
        // should retry. Otherwise a momentary provider outage (amplified
        // by the proactive sweep) would force the user to re-authorize.
        let Some(db) = connect_test_database("refresh_in_place_5xx_transient").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({"error": "temporarily_unavailable"}),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;

        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(&provider_id, &token_url, Some(admin_cid_enc), None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let key =
            insert_pending_user_api_key(&db, &encryption_keys, &provider_id, None, None).await;

        let err = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect_err("expected transient Err on 5xx");
        assert!(matches!(err, AppError::Internal(_)));

        // Key must remain "active" (not "failed") so a later retry recovers.
        let updated = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.status, "active",
            "a transient 5xx must not terminally fail the credential"
        );
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_handles_lark_json_body_and_code_error() {
        // Lark / Feishu specifics:
        //   1. Refresh request must be JSON body, not form-encoded.
        //   2. Provider returns HTTP 200 with `{code: <non-zero>, msg: ...}`
        //      on failure (not the standard 4xx body).
        // Verify the function correctly writes `status: "failed"` instead
        // of falling through to a missing-access_token error.
        let Some(db) = connect_test_database("refresh_in_place_lark_code_err").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        // Lark-flavored error: HTTP 200 + nonzero code.
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({
                "code": 99991663,
                "msg": "invalid refresh token",
                "data": null,
            }),
            axum::http::StatusCode::OK,
        )
        .await;

        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"lark-client-id").await.unwrap();
        let admin_sec_enc = encryption_keys
            .encrypt(b"lark-client-secret")
            .await
            .unwrap();
        // slug = "lark" triggers `uses_json_oauth_token_exchange`, exercising
        // the JSON-body code path that legacy `refresh_oauth_token` lacks.
        let mut provider = make_test_provider(
            &provider_id,
            &token_url,
            Some(admin_cid_enc),
            Some(admin_sec_enc),
        );
        provider.slug = "lark".to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let key =
            insert_pending_user_api_key(&db, &encryption_keys, &provider_id, None, None).await;

        let err = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect_err("Lark non-zero code response should error");
        assert!(matches!(err, AppError::Internal(_)));

        let updated = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.status, "failed",
            "Lark code-error must mark the key as failed so the wizard exits"
        );
        assert!(
            updated
                .error_message
                .as_deref()
                .is_some_and(|m| m.contains("invalid refresh token")),
            "error_message should include the Lark msg, got {:?}",
            updated.error_message
        );
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_preserves_refresh_token_when_provider_omits_it() {
        // Some providers don't rotate refresh_tokens on every refresh
        // (e.g. they only issue a new one when the old one is near
        // expiry). Verify the function keeps the existing refresh_token
        // in that case instead of nulling it out.
        let Some(db) = connect_test_database("refresh_in_place_keeps_old_refresh").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_url, _server) = spawn_token_server(
            // Response omits refresh_token entirely — standard OAuth behavior.
            serde_json::json!({
                "access_token": "new-access",
                "expires_in": 3600,
            }),
            axum::http::StatusCode::OK,
        )
        .await;

        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(&provider_id, &token_url, Some(admin_cid_enc), None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let key =
            insert_pending_user_api_key(&db, &encryption_keys, &provider_id, None, None).await;
        let original_rt_encrypted = key.refresh_token_encrypted.clone().unwrap();

        let refreshed = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect("refresh should succeed");

        // Access token rotated to the new value.
        let access_bytes = encryption_keys
            .decrypt(refreshed.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(access_bytes).unwrap(), "new-access");

        // Refresh token preserved (provider didn't issue a new one).
        let restored_rt = refreshed.refresh_token_encrypted.clone().unwrap();
        let stored_plain = encryption_keys
            .decrypt(&original_rt_encrypted)
            .await
            .unwrap();
        let restored_plain = encryption_keys.decrypt(&restored_rt).await.unwrap();
        assert_eq!(
            stored_plain, restored_plain,
            "refresh_token must be preserved when provider omits it from the response"
        );
    }

    #[tokio::test]
    async fn refresh_user_api_key_in_place_errors_when_refresh_token_missing() {
        let Some(db) = connect_test_database("refresh_in_place_no_refresh").await else {
            eprintln!("skipping refresh_user_api_key_in_place test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(
            &provider_id,
            "http://127.0.0.1:0/token",
            Some(admin_cid_enc),
            None,
        );
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let now = Utc::now();
        let key = UserApiKey {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            label: "no-refresh-token".to_string(),
            credential_type: "oauth2".to_string(),
            credential_encrypted: None,
            access_token_encrypted: Some(encryption_keys.encrypt(b"a").await.unwrap()),
            refresh_token_encrypted: None, // ← missing on purpose
            token_scopes: None,
            expires_at: None,
            provider_config_id: Some(provider_id.clone()),
            connection_id: Some(Uuid::new_v4().to_string()),
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();

        let err = super::refresh_user_api_key_in_place(&db, &encryption_keys, &key)
            .await
            .expect_err("missing refresh_token should error");
        assert!(matches!(err, AppError::Internal(ref m) if m.contains("refresh_token")));
    }

    #[test]
    fn telegram_identity_metadata_includes_all_optional_fields_when_present() {
        let data = TelegramLoginData {
            id: 99999,
            first_name: "Alice".to_string(),
            last_name: Some("Smith".to_string()),
            username: Some("alice_bot".to_string()),
            photo_url: Some("https://t.me/photo.jpg".to_string()),
            auth_date: Utc::now().timestamp(),
            hash: "abcdef".to_string(),
        };
        let metadata = build_telegram_identity_metadata(&data);
        assert_eq!(metadata.get("telegram_user_id"), Some(&"99999".to_string()));
        assert_eq!(metadata.get("first_name"), Some(&"Alice".to_string()));
        assert_eq!(metadata.get("last_name"), Some(&"Smith".to_string()));
        assert_eq!(metadata.get("username"), Some(&"alice_bot".to_string()));
        assert_eq!(
            metadata.get("photo_url"),
            Some(&"https://t.me/photo.jpg".to_string())
        );
    }

    #[test]
    fn normalize_telegram_bot_api_key_rejects_empty() {
        let err = normalize_telegram_bot_api_key("").expect_err("empty should be rejected");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn normalize_telegram_bot_api_key_rejects_double_dots() {
        let err = normalize_telegram_bot_api_key("123:ABC..DEF")
            .expect_err("double dots should be rejected");
        assert!(err.to_string().contains("invalid characters"));
    }

    #[test]
    fn ensure_additional_scopes_supported_allows_empty_for_api_key() {
        let provider = make_provider("api_key");
        assert!(ensure_additional_scopes_supported(&provider, &[]).is_ok());
    }

    #[test]
    fn parse_additional_scopes_accepts_tilde_and_star() {
        let scopes = parse_additional_scopes(Some("read~all,write*")).unwrap();
        assert_eq!(scopes, vec!["read~all".to_string(), "write*".to_string()]);
    }

    #[test]
    fn merge_scopes_dedup_is_case_sensitive() {
        let defaults = vec!["OpenID".to_string()];
        let extras = vec!["openid".to_string()];
        let merged = merge_scopes(Some(&defaults), &extras);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn build_telegram_identity_update_doc_sets_status_to_active() {
        let metadata = HashMap::new();
        let doc = build_telegram_identity_update_doc(&metadata, Utc::now()).unwrap();
        assert_eq!(doc.get_str("status").unwrap(), "active");
        assert!(doc.get("updated_at").is_some());
    }

    #[test]
    fn ensure_additional_scopes_supported_rejects_telegram_widget() {
        let provider = make_provider("telegram_widget");
        let err = ensure_additional_scopes_supported(&provider, &["scope".to_string()])
            .expect_err("telegram_widget should reject scopes");
        assert!(err.to_string().contains("does not support OAuth scopes"));
    }

    // ───────────────────────────────────────────────────────────────────
    // Integration tests: get_active_token, list_user_tokens,
    // disconnect_provider, store_api_key (additional coverage)
    // ───────────────────────────────────────────────────────────────────
    use crate::models::user_provider_token::COLLECTION_NAME as USER_PROVIDER_TOKENS;

    /// Helper: insert a UserProviderToken directly into the DB for test setup.
    async fn insert_test_token(db: &mongodb::Database, token: &UserProviderToken) {
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(token)
            .await
            .expect("insert test token");
    }

    /// Helper: build an api_key-type token with encrypted key, ready for DB insert.
    async fn make_api_key_token(
        enc: &crate::crypto::aes::EncryptionKeys,
        user_id: &str,
        provider_id: &str,
        api_key_value: &str,
        status: &str,
    ) -> UserProviderToken {
        let now = Utc::now();
        let encrypted = enc.encrypt(api_key_value.as_bytes()).await.unwrap();
        UserProviderToken {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            connection_id: None,
            credential_user_id: None,
            token_type: "api_key".to_string(),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: Some(encrypted),
            status: status.to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: Some("test-api-key".to_string()),
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Helper: build an oauth2-type token with encrypted access token.
    async fn make_oauth_token(
        enc: &crate::crypto::aes::EncryptionKeys,
        user_id: &str,
        provider_id: &str,
        access_token: &str,
        status: &str,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> UserProviderToken {
        let now = Utc::now();
        let access_enc = enc.encrypt(access_token.as_bytes()).await.unwrap();
        UserProviderToken {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(access_enc),
            refresh_token_encrypted: None,
            token_scopes: Some("openid".to_string()),
            expires_at,
            api_key_encrypted: None,
            status: status.to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    // --- get_active_token tests ---

    #[tokio::test]
    async fn get_active_token_returns_decrypted_api_key() {
        let Some(db) = connect_test_database("ut_svc_get_active_api_key").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token =
            make_api_key_token(&enc, &user_id, &provider_id, "sk-secret-123", "active").await;
        insert_test_token(&db, &token).await;

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id)
            .await
            .expect("should return decrypted token");

        assert_eq!(result.token_type, "api_key");
        assert_eq!(result.api_key.as_deref(), Some("sk-secret-123"));
        assert!(result.access_token.is_none());
    }

    #[tokio::test]
    async fn get_active_token_returns_decrypted_oauth2_token() {
        let Some(db) = connect_test_database("ut_svc_get_active_oauth2").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        // Token that expires far in the future (no refresh needed)
        let far_future = Utc::now() + Duration::hours(24);
        let token = make_oauth_token(
            &enc,
            &user_id,
            &provider_id,
            "access-token-xyz",
            "active",
            Some(far_future),
        )
        .await;
        insert_test_token(&db, &token).await;

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id)
            .await
            .expect("should return decrypted token");

        assert_eq!(result.token_type, "oauth2");
        assert_eq!(result.access_token.as_deref(), Some("access-token-xyz"));
        assert!(result.api_key.is_none());
    }

    #[tokio::test]
    async fn get_active_token_updates_last_used_at() {
        let Some(db) = connect_test_database("ut_svc_get_active_last_used").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token = make_api_key_token(&enc, &user_id, &provider_id, "key-abc", "active").await;
        let token_id = token.id.clone();
        assert!(token.last_used_at.is_none());
        insert_test_token(&db, &token).await;

        let _ = super::get_active_token(&db, &enc, &user_id, &provider_id)
            .await
            .unwrap();

        // Verify last_used_at was set in the DB
        let updated = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token_id })
            .await
            .unwrap()
            .unwrap();
        assert!(
            updated.last_used_at.is_some(),
            "last_used_at should be set after get_active_token"
        );
    }

    #[tokio::test]
    async fn get_active_token_finds_expired_status_tokens() {
        // get_active_token queries for status in ["active", "expired"]
        let Some(db) = connect_test_database("ut_svc_get_active_expired_status").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token =
            make_api_key_token(&enc, &user_id, &provider_id, "key-expired", "expired").await;
        insert_test_token(&db, &token).await;

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id)
            .await
            .expect("should find expired-status token");
        assert_eq!(result.api_key.as_deref(), Some("key-expired"));
    }

    #[tokio::test]
    async fn get_active_token_not_found_for_revoked() {
        let Some(db) = connect_test_database("ut_svc_get_active_revoked").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token =
            make_api_key_token(&enc, &user_id, &provider_id, "key-revoked", "revoked").await;
        insert_test_token(&db, &token).await;

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id).await;
        assert!(result.is_err(), "should not find revoked token");
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_active_token_not_found_when_empty() {
        let Some(db) = connect_test_database("ut_svc_get_active_empty").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id).await;
        assert!(result.is_err(), "should return NotFound");
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_active_token_rejects_unknown_token_type() {
        let Some(db) = connect_test_database("ut_svc_get_active_unknown_type").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let now = Utc::now();
        let token = UserProviderToken {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            provider_config_id: provider_id.clone(),
            connection_id: None,
            credential_user_id: None,
            token_type: "unknown_type".to_string(),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        };
        insert_test_token(&db, &token).await;

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id).await;
        assert!(result.is_err(), "unknown token_type should error");
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
        assert!(err.to_string().contains("Unknown token type"));
    }

    // --- list_user_tokens tests ---

    #[tokio::test]
    async fn list_user_tokens_returns_empty_for_new_user() {
        let Some(db) = connect_test_database("ut_svc_list_empty").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let result = super::list_user_tokens(&db, &user_id).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_user_tokens_returns_active_tokens_with_provider_info() {
        let Some(db) = connect_test_database("ut_svc_list_with_provider").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        // Insert a provider config
        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        provider.name = "Anthropic".to_string();
        provider.slug = "anthropic".to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        // Insert an active token
        let token = make_api_key_token(&enc, &user_id, &provider_id, "key-123", "active").await;
        insert_test_token(&db, &token).await;

        let result = super::list_user_tokens(&db, &user_id).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].provider_name, "Anthropic");
        assert_eq!(result[0].provider_slug, "anthropic");
        assert_eq!(result[0].status, "active");
        assert_eq!(result[0].token_type, "api_key");
        assert_eq!(result[0].label.as_deref(), Some("test-api-key"));
    }

    #[tokio::test]
    async fn list_user_tokens_excludes_revoked_tokens() {
        let Some(db) = connect_test_database("ut_svc_list_excludes_revoked").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let active_token =
            make_api_key_token(&enc, &user_id, &provider_id, "active-key", "active").await;
        insert_test_token(&db, &active_token).await;

        let provider_id_2 = Uuid::new_v4().to_string();
        let revoked_token =
            make_api_key_token(&enc, &user_id, &provider_id_2, "revoked-key", "revoked").await;
        insert_test_token(&db, &revoked_token).await;

        let result = super::list_user_tokens(&db, &user_id).await.unwrap();
        assert_eq!(result.len(), 1, "revoked token should be excluded");
        assert_eq!(result[0].provider_config_id, provider_id);
    }

    #[tokio::test]
    async fn list_user_tokens_handles_missing_provider_gracefully() {
        let Some(db) = connect_test_database("ut_svc_list_missing_provider").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        // Insert token but NO provider config
        let token = make_api_key_token(&enc, &user_id, &provider_id, "orphan-key", "active").await;
        insert_test_token(&db, &token).await;

        let result = super::list_user_tokens(&db, &user_id).await.unwrap();
        assert_eq!(result.len(), 1);
        // Falls back to "Unknown" when provider is missing
        assert_eq!(result[0].provider_name, "Unknown");
        assert_eq!(result[0].provider_slug, "unknown");
    }

    #[tokio::test]
    async fn list_user_tokens_multiple_providers_batch_resolved() {
        let Some(db) = connect_test_database("ut_svc_list_multi_providers").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        // Create two providers
        let pid1 = Uuid::new_v4().to_string();
        let mut p1 = make_provider("api_key");
        p1.id = pid1.clone();
        p1.slug = "provider-alpha".to_string();
        p1.name = "Alpha".to_string();

        let pid2 = Uuid::new_v4().to_string();
        let mut p2 = make_provider("api_key");
        p2.id = pid2.clone();
        p2.slug = "provider-beta".to_string();
        p2.name = "Beta".to_string();

        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&p1)
            .await
            .unwrap();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&p2)
            .await
            .unwrap();

        let t1 = make_api_key_token(&enc, &user_id, &pid1, "k1", "active").await;
        let t2 = make_api_key_token(&enc, &user_id, &pid2, "k2", "active").await;
        insert_test_token(&db, &t1).await;
        insert_test_token(&db, &t2).await;

        let result = super::list_user_tokens(&db, &user_id).await.unwrap();
        assert_eq!(result.len(), 2);
        let slugs: Vec<&str> = result.iter().map(|s| s.provider_slug.as_str()).collect();
        assert!(slugs.contains(&"provider-alpha"));
        assert!(slugs.contains(&"provider-beta"));
    }

    // --- disconnect_provider tests ---

    #[tokio::test]
    async fn disconnect_provider_marks_token_as_revoked() {
        let Some(db) = connect_test_database("ut_svc_disconnect_revoke").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token =
            make_api_key_token(&enc, &user_id, &provider_id, "key-to-revoke", "active").await;
        let token_id = token.id.clone();
        insert_test_token(&db, &token).await;

        super::disconnect_provider(&db, &enc, &user_id, &provider_id)
            .await
            .expect("disconnect should succeed");

        // Verify token status is now "revoked" and credentials are cleared
        let updated = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "revoked");
        assert!(
            updated.api_key_encrypted.is_none(),
            "encrypted key should be cleared"
        );
        assert!(updated.access_token_encrypted.is_none());
        assert!(updated.refresh_token_encrypted.is_none());
    }

    #[tokio::test]
    async fn disconnect_provider_not_found_when_no_token() {
        let Some(db) = connect_test_database("ut_svc_disconnect_not_found").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let err = super::disconnect_provider(&db, &enc, &user_id, &provider_id)
            .await
            .expect_err("should return NotFound");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn disconnect_provider_not_found_when_already_revoked() {
        let Some(db) = connect_test_database("ut_svc_disconnect_already_revoked").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token = make_api_key_token(&enc, &user_id, &provider_id, "key", "revoked").await;
        insert_test_token(&db, &token).await;

        let err = super::disconnect_provider(&db, &enc, &user_id, &provider_id)
            .await
            .expect_err("should return NotFound for already-revoked token");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn disconnect_provider_then_get_active_returns_not_found() {
        // End-to-end: disconnect, then verify get_active_token fails
        let Some(db) = connect_test_database("ut_svc_disconnect_then_get").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token = make_api_key_token(&enc, &user_id, &provider_id, "key-e2e", "active").await;
        insert_test_token(&db, &token).await;

        // Confirm we can get it
        let result = super::get_active_token(&db, &enc, &user_id, &provider_id).await;
        assert!(result.is_ok());

        // Disconnect
        super::disconnect_provider(&db, &enc, &user_id, &provider_id)
            .await
            .unwrap();

        // Now get_active_token should fail
        let result = super::get_active_token(&db, &enc, &user_id, &provider_id).await;
        assert!(result.is_err(), "should not find token after disconnect");
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn disconnect_provider_then_list_excludes_it() {
        let Some(db) = connect_test_database("ut_svc_disconnect_then_list").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let token = make_api_key_token(&enc, &user_id, &provider_id, "listed-key", "active").await;
        insert_test_token(&db, &token).await;

        assert_eq!(
            super::list_user_tokens(&db, &user_id).await.unwrap().len(),
            1
        );

        super::disconnect_provider(&db, &enc, &user_id, &provider_id)
            .await
            .unwrap();

        let after = super::list_user_tokens(&db, &user_id).await.unwrap();
        assert!(after.is_empty(), "revoked token should not appear in list");
    }

    // --- store_api_key + get_active_token round-trip ---

    #[tokio::test]
    async fn store_api_key_then_get_active_token_roundtrip() {
        let Some(db) = connect_test_database("ut_svc_store_then_get").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        super::store_api_key(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "roundtrip-key",
            Some("RT"),
            None,
        )
        .await
        .unwrap();

        let result = super::get_active_token(&db, &enc, &user_id, &provider_id)
            .await
            .unwrap();
        assert_eq!(result.token_type, "api_key");
        assert_eq!(result.api_key.as_deref(), Some("roundtrip-key"));
    }

    #[tokio::test]
    async fn store_api_key_with_gateway_url_roundtrip() {
        let Some(db) = connect_test_database("ut_svc_store_gateway_url").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        provider.requires_gateway_url = true;
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let token = super::store_api_key(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "gw-key",
            Some("OpenClaw"),
            Some("http://localhost:18789"),
        )
        .await
        .unwrap();

        assert_eq!(token.gateway_url.as_deref(), Some("http://localhost:18789"));
        assert_eq!(token.label.as_deref(), Some("OpenClaw"));
    }

    #[tokio::test]
    async fn store_api_key_rejects_empty_key() {
        let Some(db) = connect_test_database("ut_svc_store_empty_key").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let err = super::store_api_key(&db, &enc, &user_id, &provider_id, "", None, None)
            .await
            .expect_err("empty key should be rejected");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn store_api_key_rejects_inactive_provider() {
        let Some(db) = connect_test_database("ut_svc_store_inactive_provider").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();

        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        provider.is_active = false;
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let err = super::store_api_key(&db, &enc, &user_id, &provider_id, "key", None, None)
            .await
            .expect_err("inactive provider should be rejected");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // --- peek_oauth_state tests ---

    #[tokio::test]
    async fn peek_oauth_state_returns_not_found_for_missing() {
        let Some(db) = connect_test_database("ut_svc_peek_state_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let err = super::peek_oauth_state(&db, "nonexistent-state-id")
            .await
            .expect_err("should return error for missing state");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn store_api_key_creates_new_token_for_api_key_provider() {
        let Some(db) = connect_test_database("user_token_ext_store").await else {
            return;
        };
        let enc = test_encryption_keys();
        let provider_id = Uuid::new_v4().to_string();
        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        let user_id = Uuid::new_v4().to_string();
        let token = super::store_api_key(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "test-key-value",
            Some("My Key"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(token.status, "active");
        assert_eq!(token.token_type, "api_key");
        assert!(token.api_key_encrypted.is_some());
    }

    #[tokio::test]
    async fn store_api_key_rejects_oauth_provider_type() {
        let Some(db) = connect_test_database("user_token_ext_store_oauth").await else {
            return;
        };
        let enc = test_encryption_keys();
        let provider_id = Uuid::new_v4().to_string();
        let mut provider = make_provider("oauth2");
        provider.id = provider_id.clone();
        provider.authorization_url = Some("https://auth.example.com".to_string());
        provider.token_url = Some("https://auth.example.com/token".to_string());
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        let user_id = Uuid::new_v4().to_string();
        let err = super::store_api_key(&db, &enc, &user_id, &provider_id, "test-key", None, None)
            .await
            .expect_err("oauth provider should reject api_key store");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn store_api_key_updates_existing_token() {
        let Some(db) = connect_test_database("user_token_ext_store_update").await else {
            return;
        };
        let enc = test_encryption_keys();
        let provider_id = Uuid::new_v4().to_string();
        let mut provider = make_provider("api_key");
        provider.id = provider_id.clone();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        let user_id = Uuid::new_v4().to_string();
        let first =
            super::store_api_key(&db, &enc, &user_id, &provider_id, "first-key", None, None)
                .await
                .unwrap();
        let second = super::store_api_key(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "second-key",
            Some("Updated"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.label.as_deref(), Some("Updated"));
    }

    /// Insert a UserApiKey with arbitrary `expires_at`, `connection_id`,
    /// and `credential_type` so the sweep query can be exercised against
    /// matching and non-matching rows.
    #[allow(clippy::too_many_arguments)]
    async fn insert_oauth_key_with(
        db: &mongodb::Database,
        encryption_keys: &crate::crypto::aes::EncryptionKeys,
        provider_config_id: &str,
        credential_type: &str,
        status: &str,
        connection_id: Option<String>,
        expires_at: Option<chrono::DateTime<Utc>>,
        with_refresh_token: bool,
    ) -> UserApiKey {
        let now = Utc::now();
        let refresh_enc = if with_refresh_token {
            Some(encryption_keys.encrypt(b"stored-refresh").await.unwrap())
        } else {
            None
        };
        let key = UserApiKey {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            label: "sweep-key".to_string(),
            credential_type: credential_type.to_string(),
            credential_encrypted: None,
            access_token_encrypted: Some(encryption_keys.encrypt(b"stale").await.unwrap()),
            refresh_token_encrypted: refresh_enc,
            token_scopes: None,
            expires_at,
            provider_config_id: Some(provider_config_id.to_string()),
            connection_id,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: status.to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        key
    }

    #[tokio::test]
    async fn refresh_sweep_refreshes_only_expiring_matching_keys() {
        let Some(db) = connect_test_database("refresh_sweep_matches").await else {
            eprintln!("skipping refresh sweep test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({
                "access_token": "fresh-access-token",
                "expires_in": 3600,
            }),
            axum::http::StatusCode::OK,
        )
        .await;

        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(&provider_id, &token_url, Some(admin_cid_enc), None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let now = Utc::now();
        // Matches: expires in 5 min, well inside a 15-min window.
        let expiring = insert_oauth_key_with(
            &db,
            &encryption_keys,
            &provider_id,
            "oauth2",
            "active",
            Some(Uuid::new_v4().to_string()),
            Some(now + Duration::minutes(5)),
            true,
        )
        .await;
        // Skipped: expires in 2 hours, outside the window.
        let far_future = insert_oauth_key_with(
            &db,
            &encryption_keys,
            &provider_id,
            "oauth2",
            "active",
            Some(Uuid::new_v4().to_string()),
            Some(now + Duration::hours(2)),
            true,
        )
        .await;
        // Skipped: legacy row with no connection_id (lazy path handles it).
        let legacy = insert_oauth_key_with(
            &db,
            &encryption_keys,
            &provider_id,
            "oauth2",
            "active",
            None,
            Some(now - Duration::minutes(1)),
            true,
        )
        .await;
        // Skipped: non-oauth2 credential.
        let api_key_row = insert_oauth_key_with(
            &db,
            &encryption_keys,
            &provider_id,
            "api_key",
            "active",
            Some(Uuid::new_v4().to_string()),
            Some(now - Duration::minutes(1)),
            true,
        )
        .await;

        let report =
            super::refresh_expiring_oauth_keys(&db, &encryption_keys, Duration::minutes(15))
                .await
                .expect("sweep should succeed");
        assert_eq!(
            report.considered, 1,
            "only the in-window oauth2 key with a connection_id should be considered"
        );
        assert_eq!(report.refreshed, 1);
        assert_eq!(report.failed, 0);

        // The expiring key's access token was rotated.
        let updated = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &expiring.id })
            .await
            .unwrap()
            .unwrap();
        let bytes = encryption_keys
            .decrypt(updated.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "fresh-access-token");

        // The other three were left untouched (still "stale").
        for id in [&far_future.id, &legacy.id, &api_key_row.id] {
            let row = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! { "_id": id })
                .await
                .unwrap()
                .unwrap();
            let bytes = encryption_keys
                .decrypt(row.access_token_encrypted.as_ref().unwrap())
                .await
                .unwrap();
            assert_eq!(String::from_utf8(bytes).unwrap(), "stale");
        }
    }

    #[tokio::test]
    async fn refresh_sweep_counts_failures_and_continues() {
        let Some(db) = connect_test_database("refresh_sweep_failures").await else {
            eprintln!("skipping refresh sweep test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        // Provider returns 401 → every refresh fails.
        let (token_url, _server) = spawn_token_server(
            serde_json::json!({"error": "invalid_grant"}),
            axum::http::StatusCode::UNAUTHORIZED,
        )
        .await;

        let provider_id = Uuid::new_v4().to_string();
        let admin_cid_enc = encryption_keys.encrypt(b"admin-client-id").await.unwrap();
        let provider = make_test_provider(&provider_id, &token_url, Some(admin_cid_enc), None);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let now = Utc::now();
        let key_a = insert_oauth_key_with(
            &db,
            &encryption_keys,
            &provider_id,
            "oauth2",
            "active",
            Some(Uuid::new_v4().to_string()),
            Some(now - Duration::minutes(1)),
            true,
        )
        .await;
        let key_b = insert_oauth_key_with(
            &db,
            &encryption_keys,
            &provider_id,
            "oauth2",
            "active",
            Some(Uuid::new_v4().to_string()),
            Some(now - Duration::minutes(1)),
            true,
        )
        .await;

        let report =
            super::refresh_expiring_oauth_keys(&db, &encryption_keys, Duration::minutes(15))
                .await
                .expect("sweep itself should not error even when keys fail");
        assert_eq!(report.considered, 2);
        assert_eq!(report.refreshed, 0);
        assert_eq!(report.failed, 2, "both keys should be counted as failed");

        // Both keys persisted as failed (so they won't be retried forever
        // and the user can see they need to re-auth).
        for id in [&key_a.id, &key_b.id] {
            let row = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! { "_id": id })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, "failed");
        }
    }
}
