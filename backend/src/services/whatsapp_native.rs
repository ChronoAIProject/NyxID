//! WhatsApp-only native gateway contract. Transport orchestration owns activation.
use chrono::Utc;
use mongodb::{Database, bson::doc};
use reqwest::Client;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot,
        channel_whatsapp_native::{COLLECTION_NAME, CallbackPhase, CallbackState, NativeWhatsapp},
    },
};

pub const FLAG: &str = "nyxbot:gateway-whatsapp";
pub const MEDIA_UNAVAILABLE: &str = "Media is unavailable on this WhatsApp gateway connection. Please send your message as text. Images, audio, video and files cannot be received or sent here yet.";

/// Native writes and activation are separate, explicit opt-ins on CMA.
pub fn supported(capabilities: &Value) -> bool {
    capabilities["relay_authoring"]["native_whatsapp_writes_enabled"] == true
        && capabilities["relay_authoring"]["native_whatsapp_auto_activation_enabled"] == true
        && capabilities["relay_authoring"]["multi_consumer_writes_enabled"] == true
}

#[derive(serde::Serialize)]
pub struct Credentials {
    app_secret: zeroize::Zeroizing<String>,
    access_token: zeroize::Zeroizing<String>,
    verify_token: zeroize::Zeroizing<String>,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WhatsappCredentials([REDACTED])")
    }
}
impl Credentials {
    /// Enrollment and rotation always supply all three credentials together.
    /// BYO relay Verify Tokens are hash-only: generate a fresh native token;
    /// never treat the relay hash as a usable token or change its stored hash.
    pub fn new(app_secret: String, access_token: String, verify_token: String) -> AppResult<Self> {
        let result = Self {
            app_secret: zeroize::Zeroizing::new(app_secret),
            access_token: zeroize::Zeroizing::new(access_token),
            verify_token: zeroize::Zeroizing::new(verify_token),
        };
        for (value, max) in [
            (&result.app_secret, 256),
            (&result.access_token, 16 * 1024),
            (&result.verify_token, 256),
        ] {
            if !(16..=max).contains(&value.len())
                || !value.bytes().all(|byte| (33..=126).contains(&byte))
            {
                return Err(refusal("native_whatsapp_credential_bundle_invalid"));
            }
        }
        Ok(result)
    }
}

pub fn numeric_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|b| b.is_ascii_digit())
}

pub fn eligibility(bot: &ChannelBot) -> Result<(), &'static str> {
    if bot.platform != "whatsapp" {
        return Err("whatsapp_required");
    }
    if !bot.is_active {
        return Err("whatsapp_bot_inactive");
    }
    if bot.credential_source != "user" {
        return Err("dedicated_byo_app_required");
    }
    if !numeric_id(&bot.platform_bot_id) || bot.app_secret_encrypted.is_none() {
        return Err("whatsapp_credentials_required");
    }
    Ok(())
}

pub fn matches_bot(config: &NativeWhatsapp, bot: &ChannelBot) -> bool {
    eligibility(bot).is_ok()
        && config.bot_id == bot.id
        && config.owner_id == bot.user_id
        && config.ownership_version == bot.ownership_version
        && config.phone_number_id == bot.platform_bot_id
        && bot.app_id.as_deref().is_none_or(|id| id == config.waba_id)
        && config.dedicated_app
        && numeric_id(&config.meta_app_id)
        && numeric_id(&config.waba_id)
}

async fn graph_read(
    http: &Client,
    app_id: &str,
    app_token: &str,
    path: &str,
    query: &[(&str, String)],
) -> AppResult<Value> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        http.get(callback::graph_endpoint(app_id, path))
            .bearer_auth(app_token)
            .query(query)
            .send(),
    )
    .await
    .map_err(|_| refusal("whatsapp_identity_verification_unavailable"))?
    .map_err(|_| refusal("whatsapp_identity_verification_unavailable"))?;
    if !response.status().is_success() {
        return Err(refusal("whatsapp_identity_verification_failed"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| refusal("whatsapp_identity_verification_unavailable"))?;
    if bytes.len() > 256 * 1024 {
        return Err(refusal("whatsapp_identity_verification_failed"));
    }
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| refusal("whatsapp_identity_verification_failed"))
}

/// Verify the dedicated app and token against Meta's live identity before a
/// native source is selected. Only numeric IDs and scope/ownership facts are
/// retained; Graph response bodies never reach logs or persisted state.
pub async fn verify_platform_identity(
    http: &Client,
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    config: &NativeWhatsapp,
) -> AppResult<()> {
    if !matches_bot(config, bot) {
        return Err(refusal("native_whatsapp_identity_changed"));
    }
    let app_secret = keys
        .decrypt(
            bot.app_secret_encrypted
                .as_deref()
                .ok_or_else(|| refusal("whatsapp_credentials_required"))?,
        )
        .await
        .map_err(|_| refusal("whatsapp_credentials_required"))?;
    let app_secret = Zeroizing::new(
        String::from_utf8(app_secret).map_err(|_| refusal("whatsapp_credentials_required"))?,
    );
    let token = Zeroizing::new(
        crate::services::channel_bot_service::decrypt_bot_token(keys, bot)
            .await
            .map_err(|_| refusal("whatsapp_credentials_required"))?,
    );
    let app_token = Zeroizing::new(format!("{}|{}", config.meta_app_id, app_secret.as_str()));
    let debug = graph_read(
        http,
        &config.meta_app_id,
        app_token.as_str(),
        "debug_token",
        &[
            ("input_token", token.to_string()),
            ("fields", "app_id,type,scopes,is_valid".into()),
        ],
    )
    .await?;
    if debug["data"]["is_valid"] != true
        || debug["data"]["app_id"].as_str() != Some(config.meta_app_id.as_str())
        || !debug["data"]["scopes"].as_array().is_some_and(|scopes| {
            [
                "whatsapp_business_messaging",
                "whatsapp_business_management",
            ]
            .iter()
            .all(|required| scopes.iter().any(|scope| scope.as_str() == Some(required)))
        })
    {
        return Err(refusal("whatsapp_token_scope_or_app_mismatch"));
    }
    let phones = graph_read(
        http,
        &config.meta_app_id,
        token.as_str(),
        &format!("{}/phone_numbers", config.waba_id),
        &[("fields", "id".into())],
    )
    .await?;
    if !phones["data"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|row| row["id"].as_str() == Some(config.phone_number_id.as_str()))
    }) {
        return Err(refusal("whatsapp_phone_not_in_waba"));
    }
    Ok(())
}

pub async fn load(db: &Database, bot: &ChannelBot) -> AppResult<Option<NativeWhatsapp>> {
    Ok(db
        .collection::<NativeWhatsapp>(COLLECTION_NAME)
        .find_one(doc! {"_id": &bot.id, "owner_id": &bot.user_id,
        "ownership_version": bot.ownership_version})
        .await?)
}

const ACTIVATION_BACKOFF: chrono::Duration = chrono::Duration::minutes(15);

pub fn activation_backoff_active(config: &NativeWhatsapp) -> bool {
    config
        .callback
        .activation_retry_at
        .is_some_and(|retry_at| retry_at > Utc::now())
}

/// Persist a bounded retry fence after an external callback setup failure.
/// The code is metadata-only; provider response bodies never enter storage.
pub async fn note_activation_failure(db: &Database, bot: &ChannelBot) {
    let _ = db
        .collection::<NativeWhatsapp>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": &bot.id,
                "owner_id": &bot.user_id,
                "ownership_version": bot.ownership_version,
                "callback.phase": {"$in": ["relay", "awaiting_relay"]},
            },
            doc! {
                "$set": {
                    "callback.activation_retry_at":
                        bson::DateTime::from_chrono(Utc::now() + ACTIVATION_BACKOFF),
                    "callback.last_error": "native_whatsapp_activation_failed",
                    "updated_at": bson::DateTime::now(),
                }
            },
        )
        .await;
}

/// Return the native CMA Verify Token, installing one atomically when this is
/// the first management request for a legacy native settings row. Only the
/// envelope ciphertext is persisted; the plaintext exists for this request.
pub async fn native_verify_token(
    db: &Database,
    keys: &EncryptionKeys,
    config: &NativeWhatsapp,
) -> AppResult<Zeroizing<String>> {
    if let Some(ciphertext) = config.native_verify_token_encrypted.as_deref() {
        let plaintext = keys
            .decrypt(ciphertext)
            .await
            .map_err(|_| refusal("native_whatsapp_verify_token_unavailable"))?;
        return Ok(Zeroizing::new(String::from_utf8(plaintext).map_err(
            |_| refusal("native_whatsapp_verify_token_unavailable"),
        )?));
    }
    let generated = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let encrypted = keys
        .encrypt(generated.as_bytes())
        .await
        .map_err(|_| refusal("native_whatsapp_verify_token_unavailable"))?;
    let result = db
        .collection::<NativeWhatsapp>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": &config.bot_id,
                "owner_id": &config.owner_id,
                "ownership_version": config.ownership_version,
                "revision": config.revision,
                "native_verify_token_encrypted": bson::Bson::Null,
            },
            doc! {
                "$set": {
                    "native_verify_token_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: encrypted,
                    },
                    "updated_at": bson::DateTime::now(),
                }
            },
        )
        .await?;
    if result.matched_count == 1 {
        return Ok(generated);
    }
    let Some(current) = db
        .collection::<NativeWhatsapp>(COLLECTION_NAME)
        .find_one(doc! {"_id": &config.bot_id, "revision": config.revision})
        .await?
    else {
        return Err(refusal("native_whatsapp_verify_token_unavailable"));
    };
    let Some(ciphertext) = current.native_verify_token_encrypted.as_deref() else {
        return Err(refusal("native_whatsapp_verify_token_unavailable"));
    };
    let plaintext = keys
        .decrypt(ciphertext)
        .await
        .map_err(|_| refusal("native_whatsapp_verify_token_unavailable"))?;
    Ok(Zeroizing::new(String::from_utf8(plaintext).map_err(
        |_| refusal("native_whatsapp_verify_token_unavailable"),
    )?))
}

/// Only configuration, never a claim that CMA has verified credentials or intake.
pub async fn configure(
    db: &Database,
    bot: &ChannelBot,
    meta_app_id: &str,
    waba_id: &str,
    dedicated_app: bool,
    revision: i64,
) -> AppResult<NativeWhatsapp> {
    eligibility(bot).map_err(refusal)?;
    if !numeric_id(meta_app_id) || !numeric_id(waba_id) {
        return Err(refusal("numeric_meta_app_and_waba_ids_required"));
    }
    if !dedicated_app {
        return Err(refusal("dedicated_byo_app_required"));
    }
    if bot.app_id.as_deref().is_some_and(|id| id != waba_id) {
        return Err(refusal("waba_does_not_match_relay_bot"));
    }
    let config = NativeWhatsapp {
        bot_id: bot.id.clone(),
        owner_id: bot.user_id.clone(),
        ownership_version: bot.ownership_version,
        meta_app_id: meta_app_id.to_owned(),
        waba_id: waba_id.to_owned(),
        phone_number_id: bot.platform_bot_id.clone(),
        dedicated_app,
        native_verify_token_encrypted: None,
        revision: revision
            .checked_add(1)
            .filter(|r| *r > 0)
            .ok_or_else(|| refusal("invalid_revision"))?,
        callback: CallbackState::default(),
        updated_at: Utc::now(),
    };
    let rows = db.collection::<NativeWhatsapp>(COLLECTION_NAME);
    if revision == 0 {
        rows.insert_one(&config).await.map_err(|e| {
            if e.to_string().contains("E11000") {
                refusal("settings_changed_reload")
            } else {
                e.into()
            }
        })?;
    } else {
        let result = rows
            .replace_one(
                doc! {"_id": &bot.id, "owner_id": &bot.user_id,
                "ownership_version": bot.ownership_version, "revision": revision,
                "$or": [{"callback.phase": "relay"}, {"callback": {"$exists": false}}]},
                &config,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(refusal(
                "restore_relay_before_changing_native_settings_or_reload",
            ));
        }
    }
    Ok(config)
}

fn refusal(reason: &str) -> AppError {
    AppError::ValidationError(reason.to_owned())
}

/// Exact source; groups are deliberately absent. Never use bot.app_id as app_id.
pub fn source(config: &NativeWhatsapp, bot: &ChannelBot) -> AppResult<Value> {
    if !matches_bot(config, bot) {
        return Err(refusal("native_whatsapp_identity_changed"));
    }
    Ok(json!({"type": "whatsapp", "app_id": config.meta_app_id,
        "waba_id": config.waba_id, "phone_number_id": config.phone_number_id,
        "admission": {"type": "all"}}))
}

/// CMA calls this actor unknown: it proves a WhatsApp account, not a human owner.
/// Call only after provider authentication and live linking-admin authorization.
/// The return value preserves wa_id exactly; owner link-code proof remains separate.
pub fn verified_account<'a>(
    config: &NativeWhatsapp,
    bot: &ChannelBot,
    channel_id: &str,
    context: &'a Value,
) -> Option<&'a str> {
    if !matches!(
        config.callback.phase,
        CallbackPhase::Gateway | CallbackPhase::AwaitingRelay
    ) || config.callback.gateway_verified_at.is_none()
        || config.callback.relay_observed_at.is_some()
    {
        return None;
    }
    source_account(config, bot, channel_id, context)
}

fn source_account<'a>(
    config: &NativeWhatsapp,
    bot: &ChannelBot,
    channel_id: &str,
    context: &'a Value,
) -> Option<&'a str> {
    let activity = &context["activity"];
    let source = &activity["source"];
    let sender = activity["actor"]["id"].as_str()?;
    (matches_bot(config, bot)
        && config.callback.gateway_channel_id.as_deref() == Some(channel_id)
        && context["channel_id"].as_str() == Some(channel_id)
        && context["event_ref"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 4096)
        && source["type"] == "whatsapp"
        && source["app_id"].as_str() == Some(&config.meta_app_id)
        && source["waba_id"].as_str() == Some(&config.waba_id)
        && source["phone_number_id"].as_str() == Some(&config.phone_number_id)
        && activity["actor"]["kind"] == "unknown"
        && numeric_id(sender)
        && activity["conversation"]["kind"] == "private"
        && activity["conversation"]["id"].as_str() == Some(sender)
        && activity["conversation"]["thread_id"].is_null()
        && activity["kind"]["type"] == "message"
        && activity["event_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && id.len() <= 256))
    .then_some(sender)
}

/// Media is acknowledged explicitly without inspecting or fetching its locators.
pub fn has_unavailable_media(activity: &Value) -> bool {
    activity["kind"]["attachments"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
}

pub mod callback;
pub mod rollback;

/// Verify the Meta subscription returned by CMA and fence the callback phase
/// before the shared transport swap. A failed proof leaves the relay route in
/// place and the remote release machinery retires only this attempt.
pub async fn activate_gateway(
    db: &Database,
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    channel_id: &str,
    webhook_url: &str,
    relay_url: &str,
) -> AppResult<()> {
    let result = activate_gateway_inner(db, keys, bot, channel_id, webhook_url, relay_url).await;
    if result.is_err() {
        // Every failure after CMA has created a channel is a native activation
        // failure. Keep the relay route authoritative and fence the next
        // attempt for 15 minutes, regardless of which proof/setup step failed.
        note_activation_failure(db, bot).await;
    }
    result
}

async fn activate_gateway_inner(
    db: &Database,
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    channel_id: &str,
    webhook_url: &str,
    relay_url: &str,
) -> AppResult<()> {
    if channel_id.is_empty() || webhook_url.is_empty() {
        return Err(refusal("native_whatsapp_webhook_unavailable"));
    }
    let config = load(db, bot)
        .await?
        .ok_or_else(|| refusal("native_whatsapp_not_configured"))?;
    if !matches_bot(&config, bot) || config.callback.phase != CallbackPhase::Relay {
        return Err(refusal("native_whatsapp_callback_state_changed"));
    }
    let secret = keys
        .decrypt(
            bot.app_secret_encrypted
                .as_deref()
                .ok_or_else(|| refusal("whatsapp_credentials_required"))?,
        )
        .await
        .map_err(|_| refusal("whatsapp_credentials_required"))?;
    let secret = Zeroizing::new(
        String::from_utf8(secret).map_err(|_| refusal("whatsapp_credentials_required"))?,
    );
    let system_token = Zeroizing::new(
        crate::services::channel_bot_service::decrypt_bot_token(keys, bot)
            .await
            .map_err(|_| refusal("whatsapp_credentials_required"))?,
    );
    let verify_token = native_verify_token(db, keys, &config).await?;
    callback::configure_subscription(
        &config.meta_app_id,
        secret.as_str(),
        webhook_url,
        verify_token.as_str(),
    )
    .await?;
    let result = async {
        let proof = callback::verify(
            &config.meta_app_id,
            secret.as_str(),
            system_token.as_str(),
            &config.phone_number_id,
            &config.waba_id,
            webhook_url,
        )
        .await?;
        let mut next = config.clone();
        callback::prepare(&mut next, channel_id.to_owned(), webhook_url.to_owned())?;
        callback::confirm_gateway(&mut next, &proof)?;
        next.callback.activation_retry_at = None;
        next.callback.last_error = None;
        next.revision = next
            .revision
            .checked_add(1)
            .filter(|revision| *revision > 0)
            .ok_or_else(|| refusal("invalid_revision"))?;
        let updated = db
            .collection::<NativeWhatsapp>(COLLECTION_NAME)
            .replace_one(
                doc! {
                    "_id": &config.bot_id,
                    "owner_id": &config.owner_id,
                    "ownership_version": config.ownership_version,
                    "revision": config.revision,
                    "callback.phase": "relay",
                },
                &next,
            )
            .await?;
        if updated.matched_count != 1 {
            return Err(refusal("native_whatsapp_callback_state_changed"));
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        // Meta has already accepted the CMA subscription. Fence the local
        // state into rollback and prove the relay callback before the caller
        // retires the provisional gateway channel. If this fails, the phase
        // remains awaiting_relay so the owner can use the replacement-token
        // recovery endpoint.
        let _ = restore_after_activation_failure(db, keys, bot, relay_url).await;
        return Err(error);
    }
    Ok(())
}

/// Restore the relay callback after Meta accepted native setup but a later
/// proof or local activation fence failed. This path is intentionally
/// best-effort: a failed restore remains owner-visible as `awaiting_relay`.
pub async fn restore_after_activation_failure(
    db: &Database,
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    relay_url: &str,
) -> AppResult<()> {
    let config = load(db, bot)
        .await?
        .ok_or_else(|| refusal("native_whatsapp_not_configured"))?;
    match config.callback.phase {
        CallbackPhase::Relay => {
            let now = bson::DateTime::now();
            let updated = db
                .collection::<NativeWhatsapp>(COLLECTION_NAME)
                .update_one(
                    doc! {
                        "_id": &config.bot_id,
                        "owner_id": &config.owner_id,
                        "ownership_version": config.ownership_version,
                        "revision": config.revision,
                        "callback.phase": "relay",
                    },
                    doc! {
                        "$set": {
                            "callback.phase": "awaiting_relay",
                            "callback.transition_started_at": now,
                            "callback.intake_seen_at": bson::Bson::Null,
                            "callback.relay_observed_at": bson::Bson::Null,
                            "updated_at": now,
                        },
                        "$inc": {"revision": 1_i64},
                    },
                )
                .await?;
            if updated.matched_count != 1 {
                return Err(refusal("native_whatsapp_callback_state_changed"));
            }
        }
        CallbackPhase::Gateway | CallbackPhase::AwaitingRelay => {}
        CallbackPhase::AwaitingGateway | CallbackPhase::Unknown => {
            return Err(refusal("native_whatsapp_callback_state_changed"));
        }
    }
    restore_relay(db, keys, bot, relay_url).await
}

/// Verify that Meta now points at NyxID's relay before a flag-off route swap.
/// The shared transport code performs the remote release only after this
/// source-specific proof succeeds.
pub async fn restore_relay(
    db: &Database,
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    relay_url: &str,
) -> AppResult<()> {
    let config = load(db, bot)
        .await?
        .ok_or_else(|| refusal("native_whatsapp_not_configured"))?;
    if !matches_bot(&config, bot)
        || !matches!(
            config.callback.phase,
            CallbackPhase::Gateway | CallbackPhase::AwaitingRelay
        )
    {
        return Err(refusal("native_whatsapp_callback_not_active"));
    }
    let secret = keys
        .decrypt(
            bot.app_secret_encrypted
                .as_deref()
                .ok_or_else(|| refusal("whatsapp_credentials_required"))?,
        )
        .await
        .map_err(|_| refusal("whatsapp_credentials_required"))?;
    let secret = Zeroizing::new(
        String::from_utf8(secret).map_err(|_| refusal("whatsapp_credentials_required"))?,
    );
    let system_token = Zeroizing::new(
        crate::services::channel_bot_service::decrypt_bot_token(keys, bot)
            .await
            .map_err(|_| refusal("whatsapp_credentials_required"))?,
    );
    let replacement = if config.callback.relay_token_replaced_at.is_none() {
        Some(
            rollback::issue_for_restore(db, bot, config.revision)
                .await?
                .verify_token,
        )
    } else {
        None
    };
    let config = load(db, bot)
        .await?
        .ok_or_else(|| refusal("native_whatsapp_not_configured"))?;
    if let Some(token) = replacement.as_deref() {
        callback::configure_subscription(&config.meta_app_id, secret.as_str(), relay_url, token)
            .await
            .inspect_err(|_| {
                let db = db.clone();
                let bot = bot.clone();
                tokio::spawn(async move {
                    let _ = db
                        .collection::<NativeWhatsapp>(COLLECTION_NAME)
                        .update_one(
                            doc! {"_id": &bot.id, "owner_id": &bot.user_id},
                            doc! {"$set": {
                                "callback.last_error": "relay_subscription_write_failed",
                                "updated_at": bson::DateTime::now(),
                            }},
                        )
                        .await;
                });
            })?;
    }
    let proof = callback::verify(
        &config.meta_app_id,
        secret.as_str(),
        system_token.as_str(),
        &config.phone_number_id,
        &config.waba_id,
        relay_url,
    )
    .await?;
    let mut next = config.clone();
    callback::request_restore(&mut next)?;
    callback::confirm_relay(&mut next, &proof, relay_url)?;
    next.callback.gateway_channel_id = None;
    next.callback.gateway_webhook_url = None;
    next.callback.transition_started_at = None;
    next.callback.intake_seen_at = None;
    next.revision = next
        .revision
        .checked_add(1)
        .filter(|revision| *revision > 0)
        .ok_or_else(|| refusal("invalid_revision"))?;
    let updated = db
        .collection::<NativeWhatsapp>(COLLECTION_NAME)
        .replace_one(
            doc! {
                "_id": &config.bot_id,
                "owner_id": &config.owner_id,
                "ownership_version": config.ownership_version,
                "revision": config.revision,
                "callback.phase": {"$in": ["gateway", "awaiting_relay"]},
            },
            &next,
        )
        .await?;
    if updated.matched_count != 1 {
        return Err(refusal("native_whatsapp_callback_state_changed"));
    }
    Ok(())
}

/// Mark a live native callback as awaiting owner-verified relay restoration.
/// This is deliberately a separate fenced write: flag-off must not assume
/// that Meta has moved its callback, and the owner needs this phase to obtain
/// the one-time replacement Verify Token when the old token is unavailable.
pub async fn begin_restore(db: &Database, bot: &ChannelBot) -> AppResult<()> {
    let config = load(db, bot)
        .await?
        .ok_or_else(|| refusal("native_whatsapp_not_configured"))?;
    if !matches_bot(&config, bot) {
        return Err(refusal("native_whatsapp_identity_changed"));
    }
    if config.callback.phase == CallbackPhase::AwaitingRelay {
        return Ok(());
    }
    if config.callback.phase != CallbackPhase::Gateway {
        return Err(refusal("native_whatsapp_callback_not_active"));
    }
    let mut next = config.clone();
    callback::request_restore(&mut next)?;
    next.revision = next
        .revision
        .checked_add(1)
        .filter(|revision| *revision > 0)
        .ok_or_else(|| refusal("invalid_revision"))?;
    let updated = db
        .collection::<NativeWhatsapp>(COLLECTION_NAME)
        .replace_one(
            doc! {
                "_id": &config.bot_id,
                "owner_id": &config.owner_id,
                "ownership_version": config.ownership_version,
                "revision": config.revision,
                "callback.phase": "gateway",
            },
            &next,
        )
        .await?;
    if updated.matched_count != 1 {
        return Err(refusal("native_whatsapp_callback_state_changed"));
    }
    Ok(())
}

/// Native sends cannot fall back to the local adapter for media or templates.
pub fn text_reply(reply: &crate::services::channel_platform::OutboundReply) -> AppResult<&str> {
    if !reply.attachments.is_empty() {
        return Err(AppError::ChannelMediaUnsupported);
    }
    if reply
        .metadata
        .as_ref()
        .is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|v| !v.is_empty()))
    {
        return Err(refusal(
            "native_whatsapp_text_only_templates_and_controls_unavailable",
        ));
    }
    reply
        .text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| refusal("native_whatsapp_text_required"))
}

pub mod delivery;
#[cfg(test)]
mod tests;
