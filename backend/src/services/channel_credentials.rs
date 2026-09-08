//! Live owner-bound OAuth credentials for channel operations.

use bson::doc;
use zeroize::Zeroizing;

use super::channel_platform::{CredentialResolution, PlatformAdapter};
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::{
    channel_bot::ChannelBot, provider_config::ProviderConfig, user_api_key::UserApiKey,
};

fn reconnect(cause: &str) -> AppError {
    AppError::ValidationError(format!("{cause}. Reconnect the channel account."))
}

pub async fn connection_token(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    owner: &str,
    connection_id: &str,
    provider_slug: &str,
    required_scopes: &[&str],
) -> AppResult<Zeroizing<String>> {
    let collection = db.collection::<UserApiKey>(crate::models::user_api_key::COLLECTION_NAME);
    let load = || collection.find_one(doc! { "_id": connection_id, "user_id": owner });
    let mut key = load().await?.ok_or_else(|| {
        reconnect("Connected OAuth credential was deleted or belongs to another owner")
    })?;
    let provider = db
        .collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
        .find_one(doc! { "slug": provider_slug, "is_active": true })
        .await?
        .ok_or_else(|| reconnect("OAuth provider is unavailable"))?;
    validate_connection(&key, &provider.id, required_scopes)?;
    if key
        .expires_at
        .is_some_and(|expiry| expiry <= chrono::Utc::now() + chrono::Duration::seconds(60))
    {
        if key.refresh_token_encrypted.is_none() {
            return Err(reconnect(
                "Connected OAuth credential expired without refresh access",
            ));
        }
        match super::user_token_service::refresh_user_api_key_in_place(db, keys, &key, None).await {
            Ok(_) => {}
            Err(_) => {
                // The existing refresh machinery classifies permanent failures and fences rotation.
                let current = load()
                    .await?
                    .ok_or_else(|| reconnect("Connected OAuth credential was deleted"))?;
                validate_connection(&current, &provider.id, required_scopes)?;
                return Err(AppError::ChannelPlatformError(
                    "OAuth refresh is temporarily unavailable; retry later".to_string(),
                ));
            }
        }
    }
    // Re-read after refresh so a revoked row can never fall back to a cached token.
    key = load()
        .await?
        .ok_or_else(|| reconnect("Connected OAuth credential was deleted"))?;
    validate_connection(&key, &provider.id, required_scopes)?;
    if key
        .expires_at
        .is_some_and(|expiry| expiry <= chrono::Utc::now())
    {
        return Err(AppError::ChannelPlatformError(
            "OAuth refresh has not produced a live access token; retry later".to_string(),
        ));
    }
    let encrypted = key
        .access_token_encrypted
        .as_deref()
        .ok_or_else(|| reconnect("Connected OAuth credential has no access token"))?;
    let bytes = Zeroizing::new(keys.decrypt(encrypted).await?);
    let token = std::str::from_utf8(&bytes)
        .map_err(|_| reconnect("Connected OAuth credential is invalid"))?;
    if token.is_empty() {
        return Err(reconnect("Connected OAuth credential is empty"));
    }
    Ok(Zeroizing::new(token.to_string()))
}

fn validate_connection(key: &UserApiKey, provider_id: &str, scopes: &[&str]) -> AppResult<()> {
    if key.credential_type != "oauth2" || key.provider_config_id.as_deref() != Some(provider_id) {
        return Err(reconnect("Connection uses a different OAuth provider"));
    }
    if key.status != "active" {
        return Err(reconnect(
            "Connected OAuth credential was revoked, expired, or authorization failed",
        ));
    }
    if key.credential_source.as_deref() == Some("byo")
        || key.user_oauth_client_id_encrypted.is_some()
    {
        return Err(reconnect(
            "Connection must use the platform OAuth application",
        ));
    }
    if scopes.iter().any(|scope| {
        !key.token_scopes
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .any(|granted| granted == *scope)
    }) {
        return Err(reconnect(
            "Connection is missing required permissions; renewed consent is required",
        ));
    }
    Ok(())
}

pub async fn resolve_bot_token(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<Zeroizing<String>> {
    let CredentialResolution::OAuthConnection {
        provider_slug,
        required_scopes,
    } = adapter.credential_resolution()
    else {
        return super::channel_bot_service::decrypt_bot_token(keys, bot)
            .await
            .map(Zeroizing::new);
    };
    let result = if bot.credential_source != "connection" {
        Err(reconnect("Bot is missing its OAuth connection"))
    } else if let Some(connection_id) = &bot.connection_id {
        connection_token(
            db,
            keys,
            &bot.user_id,
            connection_id,
            provider_slug,
            required_scopes,
        )
        .await
    } else {
        Err(reconnect("Bot is missing its OAuth connection"))
    };
    if let Err(AppError::ValidationError(cause)) = &result {
        fail_bot(db, bot, cause).await?;
    }
    result
}

pub async fn fail_bot(db: &mongodb::Database, bot: &ChannelBot, cause: &str) -> AppResult<()> {
    let result = db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(doc! { "_id": &bot.id, "is_active": true, "status": { "$ne": "failed" }, "connection_id": &bot.connection_id, "updated_at": bson::DateTime::from_chrono(bot.updated_at) },
            doc! { "$set": { "status": "failed", "error": cause, "updated_at": bson::DateTime::now() } }).await?;
    if result.modified_count > 0 {
        audit_failure(db, bot, cause).await?;
    }
    Ok(())
}

pub(crate) async fn audit_failure(
    db: &mongodb::Database,
    bot: &ChannelBot,
    cause: &str,
) -> AppResult<()> {
    super::audit_service::log_system_event(db.clone(), "channel_bot_failed", Some(serde_json::json!({
        "bot_id": bot.id, "owner_user_id": bot.user_id, "platform": bot.platform, "cause": cause,
    }))).await?;
    Ok(())
}

pub struct ConnectionStart {
    pub connection_id: String,
    pub authorization_url: String,
    pub attempt_nonce: Option<String>,
}

impl std::fmt::Debug for ConnectionStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionStart([REDACTED])")
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn start_connection(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    base_url: &str,
    adapter: &dyn PlatformAdapter,
    actor: &str,
    owner: &str,
    label: &str,
) -> AppResult<ConnectionStart> {
    let CredentialResolution::OAuthConnection {
        provider_slug,
        required_scopes,
    } = adapter.credential_resolution()
    else {
        return Err(super::channel_managed::unavailable());
    };
    if label.trim().is_empty() || label.len() > 128 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 128 characters".to_string(),
        ));
    }
    let provider = db
        .collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
        .find_one(doc! { "slug": provider_slug, "is_active": true })
        .await?
        .ok_or_else(super::channel_managed::unavailable)?;
    if provider.client_id_encrypted.is_none() || provider.client_secret_encrypted.is_none() {
        return Err(super::channel_managed::unavailable());
    }
    let connection = uuid::Uuid::new_v4().to_string();
    let key = super::user_api_key_service::create_api_key(
        db,
        keys,
        owner,
        super::user_api_key_service::CreateApiKeyParams {
            label,
            credential_type: "oauth2",
            credential: "",
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: Some(&provider.id),
            connection_id: Some(&connection),
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "pending_auth",
            source: Some("user_created"),
            source_id: None,
        },
    )
    .await?;
    // Pin provenance before OAuth initiation so legacy BYO credentials cannot override this app.
    db.collection::<UserApiKey>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(
            doc! { "_id": &key.id, "user_id": owner },
            doc! { "$set": { "credential_source": "platform" } },
        )
        .await?;
    let scopes = required_scopes
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let result = super::user_token_service::initiate_oauth_connect(
        db,
        keys,
        base_url,
        actor,
        &provider.id,
        (actor != owner).then_some(owner),
        None,
        &[],
        Some(&scopes),
        Some(&connection),
        None,
        Some(crate::models::oauth_flow_kind::OAuthFlowKind::ChatConnect),
    )
    .await;
    match result {
        Ok(flow) => Ok(ConnectionStart {
            connection_id: key.id,
            authorization_url: flow.authorization_url,
            attempt_nonce: flow.attempt_nonce,
        }),
        Err(error) => {
            let _ = db
                .collection::<UserApiKey>(crate::models::user_api_key::COLLECTION_NAME)
                .delete_one(doc! { "_id": &key.id, "status": "pending_auth" })
                .await;
            Err(error)
        }
    }
}
