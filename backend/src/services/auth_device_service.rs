#![allow(dead_code)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use mongodb::{
    Collection, Database,
    bson::{self, Binary, Bson, doc, spec::BinarySubtype},
    options::ReturnDocument,
};
use rand::{Rng, RngCore, rngs::OsRng};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::crypto::{aes::EncryptionKeys, jwt::JwtKeys};
use crate::errors::{AppError, AppResult};
#[cfg(test)]
use crate::models::auth_device_code::AuthDeviceInitiatingOriginStatus;
use crate::models::auth_device_code::{
    AuthDeviceClientIpAttribution, AuthDeviceCode, AuthDeviceCodeStatus,
    COLLECTION_NAME as AUTH_DEVICE_CODES, V2_COLLECTION_NAME,
};
#[cfg(test)]
use crate::services::login_client_context::CLIENT_DISPLAY_MAX_LEN;
use crate::services::{audit_service, token_service};

#[cfg(test)]
mod grant_tests;
mod grants;
pub use grants::{approve_with_agent_key, options, poll_agent_key, sweep_expired};

type HmacSha256 = Hmac<sha2::Sha256>;

const AUTH_DEVICE_CODE_PREFIX: &str = "nyx_adc_";
const AUTH_DEVICE_EXPIRES_IN_SECS: i64 = 10 * 60;
const AUTH_DEVICE_POLL_INTERVAL_SECS: u32 = 5;
const AUTH_DEVICE_SLOW_DOWN_INCREMENT_SECS: i64 = 5;
const AUTH_DEVICE_USER_CODE_LEN: usize = 8;
const AUTH_DEVICE_USER_CODE_WRITE_RETRIES: usize = 5;
const AUTH_DEVICE_USER_CODE_ALPHABET: &[u8] = b"123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub(crate) use super::login_client_context::*;
pub use crate::models::login_client_context::LoginClientContext as InitiateInput;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitiateOutput {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollClaim {
    Pending,
    SlowDown,
    Denied,
    Expired,
    AlreadyDelivered,
    AgentKey,
    Account(Box<AccountDelivery>),
    Ready {
        encrypted_access: Vec<u8>,
        encrypted_refresh: Vec<u8>,
        expires_in: i64,
        approved_user_id: String,
        approved_session_id: String,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub struct AccountDelivery {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub user_id: String,
    pub session_id: String,
}

impl std::fmt::Debug for AccountDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountDelivery").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApproveInput {
    pub user_id: String,
    pub user_code: String,
    pub approver_ip: Option<String>,
    pub approver_user_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DenyInput {
    pub user_id: String,
    pub user_code: String,
    pub denier_ip: Option<String>,
    pub denier_user_agent: Option<String>,
}

#[tracing::instrument(
    name = "auth_device.initiate",
    skip_all,
    fields(row_id, client_label_len)
)]
pub async fn initiate(
    db: &Database,
    hmac_key: &[u8],
    input: InitiateInput,
) -> AppResult<InitiateOutput> {
    let input = sanitize_context(input);
    tracing::Span::current().record(
        "client_label_len",
        input
            .client_label
            .as_ref()
            .map(|label| label.len())
            .unwrap_or(0),
    );

    initiate_with_user_code_generator(db, hmac_key, input, generate_user_code).await
}

/// New grants live outside the legacy TTL and delivery protocol during rollout.
pub async fn initiate_v2(
    db: &Database,
    hmac_key: &[u8],
    input: InitiateInput,
) -> AppResult<InitiateOutput> {
    initiate_with_user_code_generator(db, hmac_key, sanitize_context(input), || {
        format!("2{}", generate_user_code())
    })
    .await
}

async fn initiate_with_user_code_generator<F>(
    db: &Database,
    hmac_key: &[u8],
    input: InitiateInput,
    mut user_code_generator: F,
) -> AppResult<InitiateOutput>
where
    F: FnMut() -> String,
{
    for attempt in 0..=AUTH_DEVICE_USER_CODE_WRITE_RETRIES {
        let now = Utc::now();
        let user_code_normalized = user_code_generator();
        let supports_grant_choice = is_v2_user_code(&user_code_normalized);
        let device_code = if supports_grant_choice {
            generate_device_code().replacen(AUTH_DEVICE_CODE_PREFIX, "nyx_adc2_", 1)
        } else {
            generate_device_code()
        };
        let collection = collection_for_user_code(db, &user_code_normalized);
        let user_code = format_user_code(&user_code_normalized);
        let user_code_hmac = hmac_hex(hmac_key, user_code_normalized.as_bytes());
        if collection
            .find_one(doc! {"user_code_hmac": &user_code_hmac})
            .await?
            .is_some()
        {
            continue;
        }

        let row = AuthDeviceCode {
            supports_grant_choice,
            id: Uuid::new_v4().to_string(),
            device_code_hmac: hmac_hex(hmac_key, device_code.as_bytes()),
            user_code_hmac: user_code_hmac.clone(),
            user_code_reservation_hmac: Some(user_code_hmac),
            status: AuthDeviceCodeStatus::Pending,
            poll_interval_secs: AUTH_DEVICE_POLL_INTERVAL_SECS,
            slow_down_increments: 0,
            client_label: input.client_label.clone(),
            requested_profile: input.requested_profile.clone(),
            client_user_agent: input.client_user_agent.clone(),
            client_ip: input.client_ip.clone(),
            client_ip_attribution: input.client_ip_attribution,
            client_country: input.client_country.clone(),
            client_city: input.client_city.clone(),
            client_region: input.client_region.clone(),
            client_continent: input.client_continent.clone(),
            client_ip_timezone: input.client_ip_timezone.clone(),
            initiating_origin: input.initiating_origin.clone(),
            initiating_origin_status: input.initiating_origin_status,
            client_app: input.client_app.clone(),
            client_platform: input.client_platform.clone(),
            client_model: input.client_model.clone(),
            client_form_factor: input.client_form_factor.clone(),
            client_timezone: input.client_timezone.clone(),
            client_locale: input.client_locale.clone(),
            client_screen_width: input.client_screen_width,
            client_screen_height: input.client_screen_height,
            client_device_pixel_ratio: input.client_device_pixel_ratio,
            client_hardware_concurrency: input.client_hardware_concurrency,
            client_device_memory: input.client_device_memory,
            client_ip_hmac: input
                .client_ip
                .as_deref()
                .map(|client_ip| hmac_hex(hmac_key, client_ip.as_bytes())),
            last_polled_at: None,
            approved_user_id: None,
            approved_session_id: None,
            agent_key_grant: None,
            purge_at: None,
            approver_ip_hmac: None,
            delivery_access_token_encrypted: None,
            delivery_refresh_token_encrypted: None,
            delivery_access_token_expires_in: None,
            created_at: now,
            approved_at: None,
            delivered_at: None,
            denied_at: None,
            denied_by_user_id: None,
            expires_at: now + Duration::seconds(AUTH_DEVICE_EXPIRES_IN_SECS),
        };

        match collection.insert_one(&row).await {
            Ok(_) => {
                tracing::Span::current().record("row_id", row.id.as_str());
                tracing::info!(row_id = %row.id, "auth_device.initiate");
                return Ok(InitiateOutput {
                    device_code,
                    user_code,
                    expires_in: AUTH_DEVICE_EXPIRES_IN_SECS,
                    interval: AUTH_DEVICE_POLL_INTERVAL_SECS,
                });
            }
            Err(error)
                if is_duplicate_key_error(&error)
                    && attempt < AUTH_DEVICE_USER_CODE_WRITE_RETRIES =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(AppError::Internal(
        "auth-device user_code collision retry limit exceeded".to_string(),
    ))
}

#[tracing::instrument(name = "auth_device.poll.outcome", skip_all, fields(row_id, outcome))]
pub async fn poll_and_claim(
    db: &Database,
    hmac_key: &[u8],
    device_code: &str,
) -> AppResult<PollClaim> {
    poll_internal(db, hmac_key, device_code, None, true).await
}

pub async fn poll_and_prepare(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    device_code: &str,
) -> AppResult<PollClaim> {
    poll_internal(db, hmac_key, device_code, Some(encryption), true).await
}

/// Convert an approved account grant to a browser session in the same commit
/// that consumes delivery. Cookie headers must be prepared before this call.
pub async fn poll_for_browser(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    device_code: &str,
    cookie_hash: &str,
    ip: &str,
    user_agent: Option<&str>,
) -> AppResult<PollClaim> {
    use crate::models::{
        refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken},
        session::{COLLECTION_NAME as SESSIONS, Session},
    };
    use crate::services::api_key_mutation_service as mutations;
    let claim = poll_internal(db, hmac_key, device_code, Some(encryption), false).await?;
    let PollClaim::Account(mut delivery) = claim else {
        return Ok(claim);
    };
    let now = Utc::now();
    let browser = Session {
        id: Uuid::new_v4().to_string(),
        user_id: delivery.user_id.clone(),
        token_hash: cookie_hash.to_owned(),
        ip_address: Some(ip.to_owned()),
        user_agent: user_agent.map(str::to_owned),
        expires_at: now + Duration::seconds(token_service::SESSION_TTL_SECS),
        revoked: false,
        created_at: now,
        last_active_at: now,
    };
    let browser_id = browser.id.clone();
    let original_id = delivery.session_id.clone();
    let device_hash = hmac_hex(hmac_key, device_code.as_bytes());
    let codes = collection_for_device_code(db, device_code);
    let request_id = codes
        .find_one(doc! {"device_code_hmac": &device_hash})
        .await?
        .ok_or(AppError::AuthDeviceCodeNotFound)?
        .id;
    let db_owned = db.clone();
    let transaction_codes = codes.clone();
    let mut transaction = db.client().start_session().await?;
    let result = transaction.start_transaction().and_run2(async move |session| {
        let db = &db_owned;
        let result: AppResult<()> = async {
            let now = Utc::now();
            let claimed = transaction_codes.find_one_and_update(doc! {
                "device_code_hmac": &device_hash, "status": "approved", "agent_key_grant": Bson::Null,
                "expires_at": {"$gt": bson::DateTime::from_chrono(now)}
            }, doc! {"$set": {"status": "delivered", "delivered_at": bson::DateTime::from_chrono(now),
                "purge_at": bson::DateTime::from_chrono(now + Duration::days(1))},
                "$unset": {"delivery_access_token_encrypted": "", "delivery_refresh_token_encrypted": ""}})
                .session(&mut *session).await?;
            if claimed.is_none() { return Err(AppError::AuthDeviceCodeAlreadyDelivered); }
            db.collection::<Session>(SESSIONS).update_one(doc! {"_id": &original_id},
                doc! {"$set": {"revoked": true}}).session(&mut *session).await?;
            db.collection::<RefreshToken>(REFRESH_TOKENS).update_many(doc! {"session_id": &original_id},
                doc! {"$set": {"revoked": true}}).session(&mut *session).await?;
            db.collection::<Session>(SESSIONS).insert_one(&browser).session(&mut *session).await?;
            Ok(())
        }.await;
        mutations::transaction_result(result)
    }).await.map_err(mutations::map_transaction_error);
    if matches!(result, Err(AppError::AuthDeviceCodeAlreadyDelivered)) {
        return current_poll_outcome(&codes, &request_id).await;
    }
    result?;
    delivery.session_id = browser_id;
    Ok(PollClaim::Account(delivery))
}

async fn poll_internal(
    db: &Database,
    hmac_key: &[u8],
    device_code: &str,
    encryption: Option<&EncryptionKeys>,
    consume: bool,
) -> AppResult<PollClaim> {
    let collection = collection_for_device_code(db, device_code);
    let now = Utc::now();
    let device_code_hmac = hmac_hex(hmac_key, device_code.as_bytes());
    let row = collection
        .find_one(doc! { "device_code_hmac": device_code_hmac })
        .await?
        .ok_or(AppError::AuthDeviceCodeNotFound)?;

    tracing::Span::current().record("row_id", row.id.as_str());

    if row.expires_at <= now
        && !matches!(
            row.status,
            AuthDeviceCodeStatus::Delivered | AuthDeviceCodeStatus::Denied
        )
    {
        grants::expire(db, &row).await?;
        record_poll_outcome(&row.id, "expired");
        return Ok(PollClaim::Expired);
    }

    if row.status == AuthDeviceCodeStatus::Pending && should_slow_down(&row, now) {
        collection
            .update_one(
                doc! { "_id": &row.id },
                doc! {
                    "$inc": { "slow_down_increments": 1_i64 },
                    "$set": { "last_polled_at": bson::DateTime::from_chrono(now) },
                },
            )
            .await?;
        record_poll_outcome(&row.id, "slow_down");
        return Ok(PollClaim::SlowDown);
    }

    collection
        .update_one(
            doc! { "_id": &row.id },
            doc! { "$set": { "last_polled_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    let outcome = match row.status {
        AuthDeviceCodeStatus::Pending => PollClaim::Pending,
        AuthDeviceCodeStatus::Denied => PollClaim::Denied,
        AuthDeviceCodeStatus::Expired => PollClaim::Expired,
        AuthDeviceCodeStatus::Delivered => PollClaim::AlreadyDelivered,
        AuthDeviceCodeStatus::Approved => {
            deliver_approved_claim(&collection, &row, now, encryption, consume).await?
        }
    };

    record_poll_outcome(&row.id, poll_claim_outcome(&outcome));
    Ok(outcome)
}

#[tracing::instrument(name = "auth_device.preview", skip_all, fields(row_id))]
pub async fn preview(
    db: &Database,
    hmac_key: &[u8],
    user_code: &str,
    viewer_ip: Option<&str>,
    viewer_ip_attribution: AuthDeviceClientIpAttribution,
) -> AppResult<PreviewOutput> {
    let normalized = normalize_user_code(user_code)?;
    let user_code_hmac = hmac_hex(hmac_key, normalized.as_bytes());
    let row = collection_for_user_code(db, &normalized)
        .find_one(doc! { "user_code_hmac": user_code_hmac })
        .sort(doc! {"created_at": -1})
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;

    tracing::Span::current().record("row_id", row.id.as_str());
    let context = InitiateInput {
        requested_profile: row.requested_profile,
        client_label: row.client_label,
        client_user_agent: row.client_user_agent,
        client_ip: row.client_ip,
        client_ip_attribution: row.client_ip_attribution,
        client_country: row.client_country,
        client_city: row.client_city,
        client_region: row.client_region,
        client_continent: row.client_continent,
        client_ip_timezone: row.client_ip_timezone,
        initiating_origin: row.initiating_origin,
        initiating_origin_status: row.initiating_origin_status,
        client_app: row.client_app,
        client_platform: row.client_platform,
        client_model: row.client_model,
        client_form_factor: row.client_form_factor,
        client_timezone: row.client_timezone,
        client_locale: row.client_locale,
        client_screen_width: row.client_screen_width,
        client_screen_height: row.client_screen_height,
        client_device_pixel_ratio: row.client_device_pixel_ratio,
        client_hardware_concurrency: row.client_hardware_concurrency,
        client_device_memory: row.client_device_memory,
    };
    Ok(context_preview(
        context,
        row.created_at,
        row.expires_at,
        row.status,
        viewer_ip,
        viewer_ip_attribution,
    ))
}

#[tracing::instrument(
    name = "auth_device.approve",
    skip_all,
    fields(row_id, user_id = %input.user_id, session_id)
)]
pub async fn approve(
    db: &Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    encryption_keys: &EncryptionKeys,
    hmac_key: &[u8],
    input: ApproveInput,
) -> AppResult<()> {
    let started_at = std::time::Instant::now();
    let normalized = normalize_user_code(&input.user_code)?;
    let user_code_hmac = hmac_hex(hmac_key, normalized.as_bytes());
    let collection = collection_for_user_code(db, &normalized);
    let now = Utc::now();

    let row = collection
        .find_one(doc! { "user_code_hmac": user_code_hmac })
        .sort(doc! {"created_at": -1})
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;

    tracing::Span::current().record("row_id", row.id.as_str());

    if row.status != AuthDeviceCodeStatus::Pending {
        return Err(non_pending_approve_error(row.status));
    }
    if row.expires_at <= now {
        return Err(AppError::AuthDeviceCodeExpired);
    }

    let mut transaction = db.client().start_session().await?;
    let user_agent = approve_session_user_agent(input.approver_user_agent.as_deref());
    let mut prepared = token_service::prepare_session_tokens(
        db,
        config,
        jwt_keys,
        &input.user_id,
        input.approver_ip.as_deref(),
        Some(&user_agent),
    )
    .await?;
    let access = Zeroizing::new(std::mem::take(&mut prepared.tokens.access_token));
    let refresh = Zeroizing::new(std::mem::take(&mut prepared.tokens.refresh_token));
    let encrypted_access = encryption_keys.encrypt(access.as_bytes()).await?;
    let encrypted_refresh = encryption_keys.encrypt(refresh.as_bytes()).await?;
    let approver_ip_hmac = input
        .approver_ip
        .as_deref()
        .map(|ip| hmac_hex(hmac_key, ip.as_bytes()));
    let session_id = {
        let db_owned = db.clone();
        let input = input.clone();
        let row = row.clone();
        let collection = collection.clone();
        transaction.start_transaction().and_run2(async move |session| {
    let db = &db_owned;
    let operation: AppResult<String> = async {
    let claimed = collection.find_one_and_update(
        doc! {"_id": &row.id, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
        doc! {"$set": {"status": "approved"}},
    ).session(&mut *session).await?;
    if claimed.is_none() {
        return Err(current_decision_error(&collection, &row.id).await?);
    }
    token_service::insert_prepared_session(db, &prepared, &mut *session).await?;
    let tokens = &prepared.tokens;
    tracing::Span::current().record("session_id", tokens.session_id.as_str());
    let session_id = tokens.session_id.clone();

    let approved_status = bson::to_bson(&AuthDeviceCodeStatus::Approved)
        .map_err(|e| AppError::Internal(format!("serialize auth device status: {e}")))?;
    let approved_at = Utc::now();
    if row.expires_at <= approved_at { return Err(AppError::AuthDeviceCodeExpired); }
    let delivery_expires_at = approved_at + Duration::seconds(60);
    let mut set_doc = doc! {
        "status": approved_status,
        "approved_user_id": &input.user_id,
        "approved_session_id": &tokens.session_id,
        "approved_at": bson::DateTime::from_chrono(approved_at),
        "delivery_access_token_encrypted": Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: encrypted_access.clone(),
        }),
        "delivery_refresh_token_encrypted": Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: encrypted_refresh.clone(),
        }),
        "delivery_access_token_expires_in": tokens.access_expires_in,
        "expires_at": bson::DateTime::from_chrono(delivery_expires_at),
    };
    match &approver_ip_hmac {
        Some(ip_hmac) => {
            set_doc.insert("approver_ip_hmac", ip_hmac);
        }
        None => {
            set_doc.insert("approver_ip_hmac", Bson::Null);
        }
    }

    collection
        .find_one_and_update(
            doc! { "_id": &row.id, "status": "approved" },
            doc! { "$set": set_doc },
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?;
    Ok(session_id)
    }.await;
    crate::services::api_key_mutation_service::transaction_result(operation)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)?
    };

    audit_service::log_async(
        db.clone(),
        Some(input.user_id.clone()),
        "auth_device_code_approved".to_string(),
        Some(serde_json::json!({
            "session_id": session_id,
            "request_id": row.id,
        })),
        input.approver_ip.clone(),
        input.approver_user_agent.clone(),
        None,
        None,
    );

    tracing::info!(
        row_id = %row.id,
        user_id = %input.user_id,
        session_id = %session_id,
        latency_ms = started_at.elapsed().as_millis() as u64,
        audit_logged = true,
        "auth_device.approve"
    );

    Ok(())
}

#[tracing::instrument(
    name = "auth_device.deny",
    skip_all,
    fields(row_id, user_id = %input.user_id)
)]
pub async fn deny(db: &Database, hmac_key: &[u8], input: DenyInput) -> AppResult<()> {
    let normalized = normalize_user_code(&input.user_code)?;
    let user_code_hmac = hmac_hex(hmac_key, normalized.as_bytes());
    let collection = collection_for_user_code(db, &normalized);
    let now = Utc::now();

    let row = collection
        .find_one(doc! { "user_code_hmac": user_code_hmac })
        .sort(doc! {"created_at": -1})
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;

    tracing::Span::current().record("row_id", row.id.as_str());

    if row.status != AuthDeviceCodeStatus::Pending {
        return Err(non_pending_approve_error(row.status));
    }
    if row.expires_at <= now {
        return Err(AppError::AuthDeviceCodeExpired);
    }

    let denied_status = bson::to_bson(&AuthDeviceCodeStatus::Denied)
        .map_err(|e| AppError::Internal(format!("serialize auth device status: {e}")))?;
    let updated = collection
        .find_one_and_update(
            doc! { "_id": &row.id, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(now)} },
            doc! {
                "$set": {
                    "status": denied_status,
                    "denied_at": bson::DateTime::from_chrono(now),
                    "denied_by_user_id": &input.user_id,
                    "purge_at": bson::DateTime::from_chrono(now + Duration::days(1)),
                }
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    if updated.is_none() {
        return Err(current_decision_error(&collection, &row.id).await?);
    }

    audit_service::log_async(
        db.clone(),
        Some(input.user_id.clone()),
        "auth_device_code_denied".to_string(),
        Some(serde_json::json!({
            "request_id": row.id,
        })),
        input.denier_ip,
        input.denier_user_agent,
        None,
        None,
    );

    tracing::info!(
        row_id = %row.id,
        user_id = %input.user_id,
        audit_logged = true,
        "auth_device.deny"
    );

    Ok(())
}

pub async fn decrypt_tokens(
    encryption_keys: &EncryptionKeys,
    encrypted_access: &[u8],
    encrypted_refresh: &[u8],
) -> AppResult<(String, String)> {
    let access_plaintext = Zeroizing::new(encryption_keys.decrypt(encrypted_access).await?);
    let refresh_plaintext = Zeroizing::new(encryption_keys.decrypt(encrypted_refresh).await?);

    let access_token = String::from_utf8(access_plaintext.to_vec()).map_err(|_| {
        AppError::Internal("auth-device delivery access token is not valid UTF-8".to_string())
    })?;
    let refresh_token = String::from_utf8(refresh_plaintext.to_vec()).map_err(|_| {
        AppError::Internal("auth-device delivery refresh token is not valid UTF-8".to_string())
    })?;

    Ok((access_token, refresh_token))
}

pub fn normalize_user_code(raw: &str) -> Result<String, AppError> {
    let mut normalized = String::with_capacity(AUTH_DEVICE_USER_CODE_LEN);
    for ch in raw.chars() {
        let ch = match ch {
            '-' | ' ' | '\t' => continue,
            ch => ch.to_ascii_uppercase(),
        };
        let ch = match ch {
            'I' | 'L' => '1',
            'O' => '0',
            'U' => 'V',
            ch => ch,
        };
        if !is_valid_normalized_user_code_char(ch) {
            return Err(AppError::AuthDeviceUserCodeInvalid);
        }
        normalized.push(ch);
    }

    if normalized.len() == AUTH_DEVICE_USER_CODE_LEN || is_v2_user_code(&normalized) {
        Ok(normalized)
    } else {
        Err(AppError::AuthDeviceUserCodeInvalid)
    }
}

pub fn format_user_code(normalized: &str) -> String {
    if is_v2_user_code(normalized) {
        return format!("2-{}-{}", &normalized[1..5], &normalized[5..]);
    }
    if normalized.len() <= 4 {
        return normalized.to_string();
    }
    format!("{}-{}", &normalized[..4], &normalized[4..])
}

fn collection(db: &Database) -> Collection<AuthDeviceCode> {
    db.collection::<AuthDeviceCode>(AUTH_DEVICE_CODES)
}

pub fn is_v2_user_code(normalized: &str) -> bool {
    normalized.len() == AUTH_DEVICE_USER_CODE_LEN + 1 && normalized.starts_with('2')
}

fn collection_for_user_code(db: &Database, normalized: &str) -> Collection<AuthDeviceCode> {
    collection_for_protocol(db, is_v2_user_code(normalized))
}

pub(super) fn collection_for_device_code(db: &Database, code: &str) -> Collection<AuthDeviceCode> {
    collection_for_protocol(db, code.starts_with("nyx_adc2_"))
}

fn collection_for_protocol(
    db: &Database,
    supports_grant_choice: bool,
) -> Collection<AuthDeviceCode> {
    db.collection(if supports_grant_choice {
        V2_COLLECTION_NAME
    } else {
        AUTH_DEVICE_CODES
    })
}

fn generate_device_code() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{AUTH_DEVICE_CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn generate_user_code() -> String {
    let mut rng = OsRng;
    (0..AUTH_DEVICE_USER_CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..AUTH_DEVICE_USER_CODE_ALPHABET.len());
            AUTH_DEVICE_USER_CODE_ALPHABET[idx] as char
        })
        .collect()
}

pub(crate) fn hmac_hex(hmac_key: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC-SHA256 accepts any key length");
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

fn approve_session_user_agent(approver_user_agent: Option<&str>) -> String {
    match approver_user_agent {
        Some(user_agent) if user_agent.starts_with("nyxid-cli/") => user_agent.to_string(),
        _ => "nyxid-cli (device-code)".to_string(),
    }
}

async fn cleanup_issued_session(db: &Database, session_id: &str) {
    if let Err(error) = token_service::revoke_session(db, session_id, None).await {
        tracing::error!(
            session_id = %session_id,
            error = %error,
            "failed to revoke auth-device session after approve failure"
        );
    }
}

fn non_pending_approve_error(status: AuthDeviceCodeStatus) -> AppError {
    match status {
        AuthDeviceCodeStatus::Pending => AppError::AuthDeviceCodePending,
        AuthDeviceCodeStatus::Denied => AppError::AuthDeviceCodeDenied,
        AuthDeviceCodeStatus::Expired => AppError::AuthDeviceCodeExpired,
        AuthDeviceCodeStatus::Approved | AuthDeviceCodeStatus::Delivered => {
            AppError::AuthDeviceCodeAlreadyDelivered
        }
    }
}

async fn current_decision_error(
    collection: &Collection<AuthDeviceCode>,
    row_id: &str,
) -> AppResult<AppError> {
    let row = collection
        .find_one(doc! { "_id": row_id })
        .await?
        .ok_or(AppError::AuthDeviceCodeAlreadyDelivered)?;
    if matches!(
        row.status,
        AuthDeviceCodeStatus::Delivered | AuthDeviceCodeStatus::Denied
    ) {
        return Ok(non_pending_approve_error(row.status));
    }
    if row.expires_at <= Utc::now() {
        return Ok(AppError::AuthDeviceCodeExpired);
    }
    Ok(non_pending_approve_error(row.status))
}

async fn current_poll_outcome(
    collection: &Collection<AuthDeviceCode>,
    row_id: &str,
) -> AppResult<PollClaim> {
    match current_decision_error(collection, row_id).await? {
        AppError::AuthDeviceCodeExpired => Ok(PollClaim::Expired),
        AppError::AuthDeviceCodeDenied => Ok(PollClaim::Denied),
        AppError::AuthDeviceCodeAlreadyDelivered => Ok(PollClaim::AlreadyDelivered),
        error => Err(error),
    }
}

fn redact_user_code(normalized: &str) -> String {
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() <= 4 {
        return "*".repeat(chars.len());
    }
    format!(
        "{}{}****{}{}",
        chars[0],
        chars[1],
        chars[chars.len() - 2],
        chars[chars.len() - 1]
    )
}

fn should_slow_down(row: &AuthDeviceCode, now: DateTime<Utc>) -> bool {
    let Some(last_polled_at) = row.last_polled_at else {
        return false;
    };
    let interval_secs = row.poll_interval_secs as i64
        + (row.slow_down_increments as i64 * AUTH_DEVICE_SLOW_DOWN_INCREMENT_SECS);
    now - last_polled_at < Duration::seconds(interval_secs)
}

async fn mark_expired(
    collection: &Collection<AuthDeviceCode>,
    row_id: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let expired_status = bson::to_bson(&AuthDeviceCodeStatus::Expired)
        .map_err(|e| AppError::Internal(format!("serialize auth device status: {e}")))?;
    collection
        .update_one(
            doc! { "_id": row_id, "status": { "$ne": "delivered" } },
            doc! {
                "$set": {
                    "status": expired_status,
                    "last_polled_at": bson::DateTime::from_chrono(now),
                }
            },
        )
        .await?;
    Ok(())
}

async fn claim_approved_delivery(
    collection: &Collection<AuthDeviceCode>,
    row_id: &str,
    now: DateTime<Utc>,
) -> AppResult<Option<AuthDeviceCode>> {
    let delivered_status = bson::to_bson(&AuthDeviceCodeStatus::Delivered)
        .map_err(|e| AppError::Internal(format!("serialize auth device status: {e}")))?;

    let claimed = collection
        .find_one_and_update(
            doc! { "_id": row_id, "status": "approved", "agent_key_grant": Bson::Null, "expires_at": {"$gt": bson::DateTime::from_chrono(now)} },
            doc! {
                "$set": {
                    "status": delivered_status,
                    "delivered_at": bson::DateTime::from_chrono(now),
                    "last_polled_at": bson::DateTime::from_chrono(now),
                    "purge_at": bson::DateTime::from_chrono(now + Duration::days(1)),
                },
                "$unset": {
                    "delivery_access_token_encrypted": "",
                    "delivery_refresh_token_encrypted": "",
                },
            },
        )
        .return_document(ReturnDocument::Before)
        .await?;

    Ok(claimed)
}

#[tracing::instrument(name = "auth_device.deliver", skip_all, fields(row_id = %row.id, latency_ms = (now - row.created_at).num_milliseconds()))]
async fn deliver_approved_claim(
    collection: &Collection<AuthDeviceCode>,
    row: &AuthDeviceCode,
    now: DateTime<Utc>,
    encryption: Option<&EncryptionKeys>,
    consume: bool,
) -> AppResult<PollClaim> {
    if row.agent_key_grant.is_some() {
        return Ok(PollClaim::AgentKey);
    }
    let prepared = if let Some(encryption) = encryption {
        let (access_token, refresh_token) = decrypt_tokens(
            encryption,
            row.delivery_access_token_encrypted
                .as_deref()
                .ok_or_else(|| AppError::Internal("Missing account delivery".into()))?,
            row.delivery_refresh_token_encrypted
                .as_deref()
                .ok_or_else(|| AppError::Internal("Missing account delivery".into()))?,
        )
        .await?;
        Some(AccountDelivery {
            access_token,
            refresh_token,
            expires_in: row
                .delivery_access_token_expires_in
                .ok_or_else(|| AppError::Internal("Missing account expiry".into()))?,
            user_id: row
                .approved_user_id
                .clone()
                .ok_or_else(|| AppError::Internal("Missing account owner".into()))?,
            session_id: row
                .approved_session_id
                .clone()
                .ok_or_else(|| AppError::Internal("Missing account session".into()))?,
        })
    } else {
        None
    };
    if !consume {
        return prepared
            .map(|delivery| PollClaim::Account(Box::new(delivery)))
            .ok_or_else(|| AppError::Internal("Missing prepared delivery".into()));
    }
    match claim_approved_delivery(collection, &row.id, Utc::now()).await? {
        Some(claimed) => {
            if let Some(delivery) = prepared {
                return Ok(PollClaim::Account(Box::new(delivery)));
            }
            let encrypted_access = claimed.delivery_access_token_encrypted.ok_or_else(|| {
                AppError::Internal(
                    "approved auth-device row missing encrypted access token".to_string(),
                )
            })?;
            let encrypted_refresh = claimed.delivery_refresh_token_encrypted.ok_or_else(|| {
                AppError::Internal(
                    "approved auth-device row missing encrypted refresh token".to_string(),
                )
            })?;
            let expires_in = claimed.delivery_access_token_expires_in.ok_or_else(|| {
                AppError::Internal(
                    "approved auth-device row missing access token expiry".to_string(),
                )
            })?;
            let approved_user_id = claimed.approved_user_id.ok_or_else(|| {
                AppError::Internal("approved auth-device row missing approved user id".to_string())
            })?;
            let approved_session_id = claimed.approved_session_id.ok_or_else(|| {
                AppError::Internal(
                    "approved auth-device row missing approved session id".to_string(),
                )
            })?;
            tracing::info!("auth_device.deliver");
            Ok(PollClaim::Ready {
                encrypted_access,
                encrypted_refresh,
                expires_in,
                approved_user_id,
                approved_session_id,
            })
        }
        None => current_poll_outcome(collection, &row.id).await,
    }
}

fn record_poll_outcome(row_id: &str, outcome: &str) {
    tracing::Span::current().record("outcome", outcome);
    tracing::info!(row_id = %row_id, outcome, "auth_device.poll.outcome");
}

fn poll_claim_outcome(outcome: &PollClaim) -> &'static str {
    match outcome {
        PollClaim::Pending => "pending",
        PollClaim::SlowDown => "slow_down",
        PollClaim::Denied => "denied",
        PollClaim::Expired => "expired",
        PollClaim::AlreadyDelivered => "already_delivered",
        PollClaim::AgentKey => "agent_key_ready",
        PollClaim::Account(_) => "delivered",
        PollClaim::Ready { .. } => "delivered",
    }
}

fn is_valid_normalized_user_code_char(ch: char) -> bool {
    matches!(ch, '0'..='9' | 'A'..='Z') && !matches!(ch, 'I' | 'L' | 'U')
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::hmac_keys::derive_hmac_key;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::session::COLLECTION_NAME as SESSIONS;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::test_utils::{
        cached_test_jwt_keys, connect_test_database, test_app_config, test_encryption_keys,
        test_user,
    };

    const TEST_HMAC_KEY: &[u8] = b"auth-device-test-hmac-key-32-bytes";

    #[test]
    fn client_country_normalization_accepts_only_real_iso_codes() {
        for (raw, expected) in [
            (Some("sg"), Some("SG")),
            (Some(" US "), Some("US")),
            (Some("XX"), None),
            (Some("T1"), None),
            (Some("USA"), None),
            (Some("1A"), None),
            (Some("S\0"), None),
            (Some(""), None),
            (None, None),
        ] {
            assert_eq!(
                normalize_client_country(raw.map(str::to_string)),
                expected.map(str::to_string),
                "raw country: {raw:?}"
            );
        }
    }

    #[test]
    fn initiating_origin_classification_distinguishes_all_security_states() {
        let frontend_url = "https://nyxid.dev/login";
        let cases = [
            (None, None, "absent"),
            (
                Some("https://nyxid.dev"),
                Some("https://nyxid.dev"),
                "matched",
            ),
            (
                Some("https://login-copy.example"),
                Some("https://login-copy.example"),
                "mismatched",
            ),
            (Some("not a url"), Some("not a url"), "malformed"),
            (
                Some("file:///tmp/login.html"),
                Some("file:///tmp/login.html"),
                "non_http",
            ),
        ];

        for (raw, expected_origin, expected_status) in cases {
            let classified = classify_initiating_origin(raw, frontend_url);
            assert_eq!(
                classified.origin.as_deref(),
                expected_origin,
                "raw: {raw:?}"
            );
            assert_eq!(classified.status.as_str(), expected_status, "raw: {raw:?}");
        }
    }

    #[test]
    fn initiating_origin_is_normalized_bounded_and_rejects_non_origin_urls() {
        let normalized =
            classify_initiating_origin(Some(" HTTPS://NYXID.DEV:443/ "), "https://nyxid.dev");
        assert_eq!(normalized.origin.as_deref(), Some("https://nyxid.dev"));
        assert_eq!(normalized.status.as_str(), "matched");

        for malformed in [
            "https://nyxid.dev/login",
            "https://nyxid.dev?source=copy",
            "https://user@nyxid.dev",
            "null",
        ] {
            assert_eq!(
                classify_initiating_origin(Some(malformed), "https://nyxid.dev")
                    .status
                    .as_str(),
                "malformed",
                "origin: {malformed}"
            );
        }

        let hostile = format!("https://{}example.com\0", "a".repeat(400));
        let classified = classify_initiating_origin(Some(&hostile), "https://nyxid.dev");
        assert_eq!(classified.status.as_str(), "malformed");
        assert!(classified.origin.is_some_and(|value| {
            value.chars().count() <= INITIATING_ORIGIN_MAX_LEN
                && !value.chars().any(char::is_control)
        }));
    }

    #[test]
    fn requester_metadata_normalizers_bound_hostile_values() {
        assert_eq!(
            normalize_client_timezone(Some(" Asia/Singapore ".to_string())).as_deref(),
            Some("Asia/Singapore")
        );
        assert_eq!(
            normalize_client_locale(Some("en-SG".to_string())).as_deref(),
            Some("en-SG")
        );
        assert_eq!(
            normalize_client_form_factor(Some("desktop".to_string())).as_deref(),
            Some("desktop")
        );
        assert!(normalize_client_timezone(Some("Europe/Moscow\0oops".to_string())).is_none());
        assert!(normalize_client_timezone(Some("x".repeat(100))).is_none());
        assert!(normalize_client_locale(Some("en SG".to_string())).is_none());
        assert!(normalize_client_form_factor(Some("watch".to_string())).is_none());
        assert_eq!(normalize_screen_dimension(Some(2560)), Some(2560));
        assert_eq!(normalize_screen_dimension(Some(0)), None);
        assert_eq!(normalize_screen_dimension(Some(100_000)), None);
        assert_eq!(normalize_device_pixel_ratio(Some(2.0)), Some(2.0));
        assert_eq!(normalize_device_pixel_ratio(Some(f64::NAN)), None);
        assert_eq!(normalize_hardware_concurrency(Some(16)), Some(16));
        assert_eq!(normalize_hardware_concurrency(Some(0)), None);
        assert_eq!(normalize_device_memory(Some(8.0)), Some(8.0));
        assert_eq!(normalize_device_memory(Some(f64::INFINITY)), None);
    }

    #[test]
    fn verified_and_reported_timezones_are_compared_only_when_both_are_valid() {
        assert_eq!(
            timezones_match(Some("Asia/Singapore"), Some("Asia/Singapore")),
            Some(true)
        );
        assert_eq!(
            timezones_match(Some("Asia/Singapore"), Some("Europe/Moscow")),
            Some(false)
        );
        assert_eq!(timezones_match(Some("Asia/Singapore"), None), None);
        assert_eq!(timezones_match(None, Some("Asia/Singapore")), None);
    }

    #[test]
    fn verified_network_relation_uses_ipv4_24_and_ipv6_48_prefixes() {
        let verified = AuthDeviceClientIpAttribution::Verified;
        let cases = [
            ("8.8.8.8", "8.8.8.8", Some("same_ip")),
            ("8.8.8.8", "8.8.8.200", Some("same_network")),
            ("8.8.8.8", "8.8.9.8", Some("different_network")),
            (
                "2001:4860:4860::8888",
                "2001:4860:4860::8888",
                Some("same_ip"),
            ),
            (
                "2001:4860:4860::8888",
                "2001:4860:4860:ffff::1",
                Some("same_network"),
            ),
            (
                "2001:4860:4860::8888",
                "2001:4860:4861::1",
                Some("different_network"),
            ),
            ("::ffff:8.8.8.8", "8.8.8.42", Some("same_network")),
        ];

        for (requester, viewer, expected) in cases {
            assert_eq!(
                network_relation(Some(requester), verified, Some(viewer), verified,),
                expected,
                "requester={requester}, viewer={viewer}"
            );
        }

        assert_eq!(
            network_relation(
                Some("8.8.8.8"),
                verified,
                Some("8.8.8.9"),
                AuthDeviceClientIpAttribution::Unavailable,
            ),
            None
        );
    }

    #[test]
    fn client_user_agent_parser_covers_supported_requesters() {
        struct Case {
            ua: &'static str,
            kind: &'static str,
            app: Option<&'static str>,
            platform: Option<&'static str>,
        }

        let cases = [
            Case {
                ua: "nyxid-cli/1.4.2 (macos; aarch64)",
                kind: "cli",
                app: Some("NyxID CLI 1.4.2"),
                platform: Some("macOS (aarch64)"),
            },
            Case {
                ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
                kind: "browser",
                app: Some("Chrome 131"),
                platform: Some("Windows (x86_64)"),
            },
            Case {
                ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
                kind: "browser",
                app: Some("Edge 131"),
                platform: Some("Windows (x86_64)"),
            },
            Case {
                ua: "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0",
                kind: "browser",
                app: Some("Firefox 133"),
                platform: Some("Linux (x86_64)"),
            },
            Case {
                ua: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
                kind: "browser",
                app: Some("Safari 18"),
                platform: Some("macOS (x86_64)"),
            },
            Case {
                ua: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
                kind: "browser",
                app: Some("Safari 18"),
                platform: Some("iOS"),
            },
            Case {
                ua: "Mozilla/5.0 (Linux; Android 15; Pixel 9 Pro) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.81 Mobile Safari/537.36",
                kind: "browser",
                app: Some("Chrome 131"),
                platform: Some("Android"),
            },
            Case {
                ua: "nyxid-mobile/2.3.1 (ios; arm64)",
                kind: "mobile",
                app: Some("NyxID Mobile 2.3.1"),
                platform: Some("iOS (arm64)"),
            },
        ];

        for case in cases {
            let parsed = parse_client_user_agent(Some(case.ua));
            assert_eq!(parsed.kind, case.kind, "UA: {}", case.ua);
            assert_eq!(parsed.app.as_deref(), case.app, "UA: {}", case.ua);
            assert_eq!(parsed.platform.as_deref(), case.platform, "UA: {}", case.ua);
        }
    }

    #[test]
    fn client_user_agent_parser_bounds_and_rejects_hostile_or_junk_input() {
        for ua in ["", "   ", "curl/8.10.1", "\0\n\r\t"] {
            let parsed = parse_client_user_agent(Some(ua));
            assert_eq!(parsed.kind, "unknown", "UA: {ua:?}");
            assert!(parsed.app.is_none(), "UA: {ua:?}");
            assert!(parsed.platform.is_none(), "UA: {ua:?}");
        }

        let hostile = format!("nyxid-cli/1.4.2\0\n (macos; {})", "aarch64".repeat(200));
        let parsed = parse_client_user_agent(Some(&hostile));
        assert!(parsed.app.as_ref().is_none_or(|value| {
            value.len() <= CLIENT_DISPLAY_MAX_LEN && !value.chars().any(char::is_control)
        }));
        assert!(parsed.platform.as_ref().is_none_or(|value| {
            value.len() <= CLIENT_DISPLAY_MAX_LEN && !value.chars().any(char::is_control)
        }));
    }

    #[test]
    fn preview_ip_comparison_and_remaining_time_are_pure_and_clamped() {
        assert_eq!(
            same_ip_as_viewer(
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Verified,
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Verified,
            ),
            Some(true)
        );
        assert_eq!(
            same_ip_as_viewer(
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Verified,
                Some("9.9.9.9"),
                AuthDeviceClientIpAttribution::Verified,
            ),
            Some(false)
        );
        assert_eq!(
            same_ip_as_viewer(
                Some("::ffff:8.8.8.8"),
                AuthDeviceClientIpAttribution::Verified,
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Verified,
            ),
            Some(true)
        );
        assert_eq!(
            same_ip_as_viewer(
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Verified,
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Unverified,
            ),
            None
        );
        assert_eq!(
            same_ip_as_viewer(
                Some("10.2.10.22"),
                AuthDeviceClientIpAttribution::Unavailable,
                Some("10.2.10.22"),
                AuthDeviceClientIpAttribution::Unavailable,
            ),
            None
        );
        assert_eq!(
            effective_client_ip_attribution(
                Some("10.2.10.22"),
                AuthDeviceClientIpAttribution::Verified,
            ),
            AuthDeviceClientIpAttribution::Unavailable
        );
        assert_eq!(
            effective_client_ip_attribution(
                Some("8.8.8.8"),
                AuthDeviceClientIpAttribution::Unverified,
            ),
            AuthDeviceClientIpAttribution::Unverified
        );

        let now = Utc::now();
        assert_eq!(
            seconds_remaining_at(now + Duration::milliseconds(1500), now),
            2
        );
        assert_eq!(seconds_remaining_at(now, now), 0);
        assert_eq!(seconds_remaining_at(now - Duration::seconds(5), now), 0);
    }

    #[test]
    fn normalize_user_code_accepts_roundtrip_vectors() {
        for (raw, expected) in [
            ("abcd1234", "ABCD1234"),
            ("AbCd-1234", "ABCD1234"),
            ("ab cd\t12-34", "ABCD1234"),
            ("iLoU2345", "110V2345"),
            ("zzzzzzzz", "ZZZZZZZZ"),
        ] {
            assert_eq!(normalize_user_code(raw).unwrap(), expected);
        }
    }

    #[test]
    fn normalize_user_code_rejects_invalid_inputs() {
        for raw in ["", "ABC1234", "ABCDE12345", "ABC_DEF1", "ABC\nDEF1"] {
            assert!(matches!(
                normalize_user_code(raw),
                Err(AppError::AuthDeviceUserCodeInvalid)
            ));
        }
    }

    #[test]
    fn format_user_code_adds_midpoint_dash() {
        assert_eq!(format_user_code("ABCDEFGH"), "ABCD-EFGH");
    }

    #[test]
    fn redact_user_code_keeps_only_edges() {
        assert_eq!(redact_user_code("ABCDEFGH"), "AB****GH");
    }

    #[test]
    fn auth_device_hmac_label_is_domain_separated() {
        let encryption_key = [0x42_u8; 32];
        let jwt_private_pem = [0x99_u8; 512];
        let cli = derive_hmac_key("cli-pairing", Some(&encryption_key), &jwt_private_pem);
        let auth = derive_hmac_key("auth-device", Some(&encryption_key), &jwt_private_pem);

        assert_ne!(cli.as_slice(), auth.as_slice());
    }

    #[tokio::test]
    async fn initiate_persists_sanitized_pending_row() {
        let Some(db) = connect_test_database("auth_device_initiate").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");

        let output = initiate(
            &db,
            TEST_HMAC_KEY,
            InitiateInput {
                client_label: Some("  label\u{0000}with-control  ".to_string()),
                client_user_agent: Some(format!("  {}  ", "a".repeat(300))),
                client_ip: Some("203.0.113.10".to_string()),
                client_ip_attribution: AuthDeviceClientIpAttribution::Unverified,
                client_country: None,
                client_app: Some("Chrome 151.0.7922.174".to_string()),
                client_platform: Some("macOS 26.5.2 (arm64)".to_string()),
                client_form_factor: Some("desktop".to_string()),
                client_timezone: Some("Asia/Singapore".to_string()),
                client_locale: Some("en-US".to_string()),
                client_screen_width: Some(1512),
                client_screen_height: Some(982),
                client_device_pixel_ratio: Some(2.0),
                client_hardware_concurrency: Some(12),
                client_device_memory: Some(16.0),
                ..Default::default()
            },
        )
        .await
        .expect("initiate");

        assert!(output.device_code.starts_with(AUTH_DEVICE_CODE_PREFIX));
        assert_eq!(output.user_code.len(), 9);
        assert_eq!(output.expires_in, AUTH_DEVICE_EXPIRES_IN_SECS);
        assert_eq!(output.interval, AUTH_DEVICE_POLL_INTERVAL_SECS);

        let row = collection(&db)
            .find_one(doc! {
                "device_code_hmac": hmac_hex(TEST_HMAC_KEY, output.device_code.as_bytes())
            })
            .await
            .expect("query")
            .expect("row exists");

        assert_eq!(row.status, AuthDeviceCodeStatus::Pending);
        assert_eq!(row.client_label.as_deref(), Some("labelwith-control"));
        assert_eq!(row.client_user_agent.as_ref().unwrap().len(), 256);
        assert_eq!(row.client_ip.as_deref(), Some("203.0.113.10"));
        assert_eq!(row.client_app.as_deref(), Some("Chrome 151.0.7922.174"));
        assert_eq!(row.client_platform.as_deref(), Some("macOS 26.5.2 (arm64)"));
        assert_eq!(row.client_form_factor.as_deref(), Some("desktop"));
        assert_eq!(row.client_timezone.as_deref(), Some("Asia/Singapore"));
        assert_eq!(row.client_locale.as_deref(), Some("en-US"));
        assert_eq!(row.client_screen_width, Some(1512));
        assert_eq!(row.client_device_pixel_ratio, Some(2.0));
        assert_eq!(row.client_hardware_concurrency, Some(12));
        assert_eq!(row.client_device_memory, Some(16.0));
        assert_eq!(
            row.client_ip_hmac.as_deref(),
            Some(hmac_hex(TEST_HMAC_KEY, b"203.0.113.10").as_str())
        );
    }

    #[tokio::test]
    async fn preview_returns_safe_display_fields() {
        let Some(db) = connect_test_database("auth_device_preview").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let output = initiate_with_user_code_generator(
            &db,
            TEST_HMAC_KEY,
            InitiateInput {
                client_label: Some("workstation".to_string()),
                client_user_agent: Some("nyxid-cli/0.8.0".to_string()),
                client_ip: Some("203.0.113.10".to_string()),
                client_ip_attribution: AuthDeviceClientIpAttribution::Unverified,
                client_country: None,
                ..Default::default()
            },
            || "ABCD1234".to_string(),
        )
        .await
        .expect("initiate");

        let preview = preview(
            &db,
            TEST_HMAC_KEY,
            &output.user_code,
            None,
            AuthDeviceClientIpAttribution::Unavailable,
        )
        .await
        .expect("preview");

        assert_eq!(preview.client_label.as_deref(), Some("workstation"));
        assert_eq!(
            preview.client_user_agent.as_deref(),
            Some("nyxid-cli/0.8.0")
        );
        assert_eq!(preview.client_ip.as_deref(), Some("203.0.113.10"));
        assert_eq!(preview.initiating_origin_status, "absent");
        assert!(preview.initiating_origin.is_none());
        assert!(preview.client_timezone.is_none());
        assert!(preview.client_ip_timezone.is_none());
        assert!(preview.client_timezone_matches_ip.is_none());
        assert!(preview.network_relation.is_none());
        assert_eq!(preview.status, AuthDeviceCodeStatus::Pending);
    }

    #[tokio::test]
    async fn preview_legacy_row_without_client_ip_returns_none() {
        let Some(db) = connect_test_database("auth_device_preview_legacy_ip").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() + Duration::minutes(10),
        )
        .await;
        db.collection::<bson::Document>(AUTH_DEVICE_CODES)
            .update_one(
                doc! { "_id": &row.id },
                doc! { "$unset": { "client_ip": "" } },
            )
            .await
            .expect("remove legacy field");

        let output = preview(
            &db,
            TEST_HMAC_KEY,
            "ABCD-1234",
            None,
            AuthDeviceClientIpAttribution::Unavailable,
        )
        .await
        .expect("preview legacy row");

        assert!(output.client_ip.is_none());
    }

    #[tokio::test]
    async fn deny_pending_row_sets_terminal_fields_and_audits() {
        let Some(db) = connect_test_database("auth_device_deny_happy").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() + Duration::minutes(10),
        )
        .await;
        let audit_written =
            audit_service::notify_on_audit_write_for_user("auth_device_code_denied", &user_id);

        deny(
            &db,
            TEST_HMAC_KEY,
            DenyInput {
                user_id: user_id.clone(),
                user_code: "ABCD-1234".to_string(),
                denier_ip: Some("203.0.113.88".to_string()),
                denier_user_agent: Some("nyxid-mobile/1.0".to_string()),
            },
        )
        .await
        .expect("deny");

        let denied = row_by_id(&db, &row.id).await;
        assert_eq!(denied.status, AuthDeviceCodeStatus::Denied);
        assert!(denied.denied_at.is_some());
        assert_eq!(denied.denied_by_user_id.as_deref(), Some(user_id.as_str()));
        assert!(denied.approved_user_id.is_none());
        assert!(denied.approved_session_id.is_none());

        let audit_id = tokio::time::timeout(Duration::seconds(2).to_std().unwrap(), audit_written)
            .await
            .expect("audit write timed out")
            .expect("audit watcher");
        let audit = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! { "_id": audit_id })
            .await
            .expect("audit query")
            .expect("audit row");
        assert_eq!(audit.event_type, "auth_device_code_denied");
        assert_eq!(audit.user_id.as_deref(), Some(user_id.as_str()));
        assert_eq!(
            audit
                .event_data
                .as_ref()
                .and_then(|data| data.get("request_id"))
                .and_then(serde_json::Value::as_str),
            Some(row.id.as_str())
        );
        assert!(
            !audit
                .event_data
                .as_ref()
                .is_some_and(|data| data.to_string().contains("ABCD1234"))
        );
    }

    #[tokio::test]
    async fn deny_wrong_user_code_returns_invalid_without_mutation() {
        let Some(db) = connect_test_database("auth_device_deny_wrong_code").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let result = deny(
            &db,
            TEST_HMAC_KEY,
            DenyInput {
                user_id: Uuid::new_v4().to_string(),
                user_code: "WXYZ-9999".to_string(),
                denier_ip: None,
                denier_user_agent: None,
            },
        )
        .await;

        assert!(matches!(result, Err(AppError::AuthDeviceUserCodeInvalid)));
        assert_eq!(
            row_by_id(&db, &row.id).await.status,
            AuthDeviceCodeStatus::Pending
        );
    }

    #[tokio::test]
    async fn deny_non_pending_rows_use_approve_error_mapping() {
        let Some(db) = connect_test_database("auth_device_deny_non_pending").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");

        for (status, expected_error_code) in [
            (AuthDeviceCodeStatus::Approved, 11205),
            (AuthDeviceCodeStatus::Denied, 11204),
            (AuthDeviceCodeStatus::Expired, 11201),
        ] {
            collection(&db)
                .delete_many(doc! {})
                .await
                .expect("clear rows");
            seed_row(&db, status, Utc::now() + Duration::minutes(10)).await;

            let error = deny(
                &db,
                TEST_HMAC_KEY,
                DenyInput {
                    user_id: Uuid::new_v4().to_string(),
                    user_code: "ABCD-1234".to_string(),
                    denier_ip: None,
                    denier_user_agent: None,
                },
            )
            .await
            .expect_err("non-pending deny must fail");

            assert_eq!(error.error_code(), expected_error_code);
        }
    }

    #[tokio::test]
    async fn deny_after_expiry_returns_expired_without_transition() {
        let Some(db) = connect_test_database("auth_device_deny_expired_at").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() - Duration::seconds(1),
        )
        .await;

        let result = deny(
            &db,
            TEST_HMAC_KEY,
            DenyInput {
                user_id: Uuid::new_v4().to_string(),
                user_code: "ABCD-1234".to_string(),
                denier_ip: None,
                denier_user_agent: None,
            },
        )
        .await;

        assert!(matches!(result, Err(AppError::AuthDeviceCodeExpired)));
        assert_eq!(
            row_by_id(&db, &row.id).await.status,
            AuthDeviceCodeStatus::Pending
        );
    }

    #[tokio::test]
    async fn concurrent_approve_and_deny_have_one_winner_and_no_orphan_session() {
        let Some(db) = connect_test_database("auth_device_approve_deny_race").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let (approve_result, deny_result) = tokio::join!(
            approve(
                &db,
                &config,
                &jwt_keys,
                &encryption_keys,
                TEST_HMAC_KEY,
                ApproveInput {
                    user_id: user_id.clone(),
                    user_code: "ABCD-1234".to_string(),
                    approver_ip: None,
                    approver_user_agent: None,
                },
            ),
            deny(
                &db,
                TEST_HMAC_KEY,
                DenyInput {
                    user_id: user_id.clone(),
                    user_code: "ABCD-1234".to_string(),
                    denier_ip: None,
                    denier_user_agent: None,
                },
            )
        );

        assert_eq!(
            [approve_result.is_ok(), deny_result.is_ok()]
                .into_iter()
                .filter(|won| *won)
                .count(),
            1,
            "approve={approve_result:?} deny={deny_result:?}"
        );

        let decided = row_by_id(&db, &row.id).await;
        let live_sessions = db
            .collection::<bson::Document>(SESSIONS)
            .count_documents(doc! { "revoked": false })
            .await
            .expect("live session count");
        match decided.status {
            AuthDeviceCodeStatus::Approved => {
                assert!(approve_result.is_ok());
                assert!(matches!(
                    deny_result,
                    Err(AppError::AuthDeviceCodeAlreadyDelivered)
                ));
                assert_eq!(live_sessions, 1);
            }
            AuthDeviceCodeStatus::Denied => {
                assert!(deny_result.is_ok());
                assert!(matches!(
                    approve_result,
                    Err(AppError::AuthDeviceCodeDenied)
                ));
                assert_eq!(live_sessions, 0);
                assert!(decided.approved_session_id.is_none());
            }
            status => panic!("unexpected race terminal status: {status:?}"),
        }
    }

    #[tokio::test]
    async fn approve_pending_row_encrypts_tokens_shortens_expiry_and_audits() {
        let Some(db) = connect_test_database("auth_device_approve_happy").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let output = initiate_with_user_code_generator(
            &db,
            TEST_HMAC_KEY,
            InitiateInput {
                client_label: Some("workstation".to_string()),
                client_user_agent: Some("nyxid-cli/0.8.0".to_string()),
                client_ip: None,
                client_ip_attribution: AuthDeviceClientIpAttribution::Unavailable,
                client_country: None,
                ..Default::default()
            },
            || "ABCDEFGH".to_string(),
        )
        .await
        .expect("initiate");
        let original = row_by_user_code(&db, "ABCDEFGH").await;
        let audit_written =
            audit_service::notify_on_audit_write_for_user("auth_device_code_approved", &user_id);

        approve(
            &db,
            &config,
            &jwt_keys,
            &encryption_keys,
            TEST_HMAC_KEY,
            ApproveInput {
                user_id: user_id.clone(),
                user_code: output.user_code,
                approver_ip: Some("203.0.113.77".to_string()),
                approver_user_agent: Some("nyxid-cli/0.8.0".to_string()),
            },
        )
        .await
        .expect("approve");

        let updated = row_by_id(&db, &original.id).await;
        assert_eq!(updated.status, AuthDeviceCodeStatus::Approved);
        assert_eq!(updated.approved_user_id.as_deref(), Some(user_id.as_str()));
        assert!(updated.approved_session_id.is_some());
        assert!(updated.approved_at.is_some());
        assert_eq!(
            updated.approver_ip_hmac.as_deref(),
            Some(hmac_hex(TEST_HMAC_KEY, b"203.0.113.77").as_str())
        );
        assert!(updated.delivery_access_token_encrypted.is_some());
        assert!(updated.delivery_refresh_token_encrypted.is_some());
        assert_eq!(
            updated.delivery_access_token_expires_in,
            Some(config.jwt_access_ttl_secs)
        );
        assert!(updated.expires_at < original.expires_at);
        assert!(updated.expires_at <= Utc::now() + Duration::seconds(70));

        let (access, refresh) = decrypt_tokens(
            &encryption_keys,
            updated.delivery_access_token_encrypted.as_deref().unwrap(),
            updated.delivery_refresh_token_encrypted.as_deref().unwrap(),
        )
        .await
        .expect("decrypt");
        assert_eq!(
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &access)
                .expect("access token")
                .sub,
            user_id
        );
        assert_eq!(
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refresh)
                .expect("refresh token")
                .sub,
            user_id
        );

        let audit_id = tokio::time::timeout(Duration::seconds(2).to_std().unwrap(), audit_written)
            .await
            .expect("audit write timed out")
            .expect("audit watcher");
        let audit = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! { "_id": audit_id })
            .await
            .expect("audit query")
            .expect("audit row");
        assert_eq!(audit.event_type, "auth_device_code_approved");
        assert_eq!(audit.user_id.as_deref(), Some(user_id.as_str()));
        assert!(audit.api_key_id.is_none());
        assert!(audit.api_key_name.is_none());
        assert_eq!(
            audit
                .event_data
                .as_ref()
                .and_then(|data| data.get("request_id"))
                .and_then(serde_json::Value::as_str),
            Some(updated.id.as_str())
        );
        let audit_data = audit.event_data.as_ref().unwrap().to_string();
        for forbidden in [
            "ABCDEFGH",
            "user_code",
            updated.user_code_hmac.as_str(),
            updated.device_code_hmac.as_str(),
        ] {
            assert!(!audit_data.contains(forbidden));
        }
        assert_eq!(
            audit
                .event_data
                .as_ref()
                .and_then(|data| data.get("session_id"))
                .and_then(serde_json::Value::as_str),
            updated.approved_session_id.as_deref()
        );
    }

    #[tokio::test]
    async fn approve_wrong_user_code_returns_invalid_without_mutation() {
        let Some(db) = connect_test_database("auth_device_approve_wrong_code").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let result = approve(
            &db,
            &config,
            &jwt_keys,
            &encryption_keys,
            TEST_HMAC_KEY,
            ApproveInput {
                user_id,
                user_code: "WXYZ-9999".to_string(),
                approver_ip: None,
                approver_user_agent: None,
            },
        )
        .await;

        assert!(matches!(result, Err(AppError::AuthDeviceUserCodeInvalid)));
        let unchanged = row_by_id(&db, &row.id).await;
        assert_eq!(unchanged.status, AuthDeviceCodeStatus::Pending);
        assert!(unchanged.approved_user_id.is_none());
        assert!(unchanged.approved_session_id.is_none());
    }

    #[tokio::test]
    async fn approve_already_approved_row_rejects_before_token_mint() {
        let Some(db) = connect_test_database("auth_device_approve_already_approved").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        seed_row(
            &db,
            AuthDeviceCodeStatus::Approved,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let result = approve(
            &db,
            &config,
            &jwt_keys,
            &encryption_keys,
            TEST_HMAC_KEY,
            ApproveInput {
                user_id,
                user_code: "ABCD-1234".to_string(),
                approver_ip: None,
                approver_user_agent: None,
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(AppError::AuthDeviceCodeAlreadyDelivered)
        ));
        assert_eq!(
            db.collection::<bson::Document>(SESSIONS)
                .count_documents(doc! {})
                .await
                .expect("session count"),
            0
        );
    }

    #[tokio::test]
    async fn approve_expired_row_returns_expired_before_token_mint() {
        let Some(db) = connect_test_database("auth_device_approve_expired").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() - Duration::seconds(1),
        )
        .await;

        let result = approve(
            &db,
            &config,
            &jwt_keys,
            &encryption_keys,
            TEST_HMAC_KEY,
            ApproveInput {
                user_id,
                user_code: "ABCD-1234".to_string(),
                approver_ip: None,
                approver_user_agent: None,
            },
        )
        .await;

        assert!(matches!(result, Err(AppError::AuthDeviceCodeExpired)));
        assert_eq!(
            row_by_id(&db, &row.id).await.status,
            AuthDeviceCodeStatus::Pending
        );
        assert_eq!(
            db.collection::<bson::Document>(SESSIONS)
                .count_documents(doc! {})
                .await
                .expect("session count"),
            0
        );
    }

    #[tokio::test]
    async fn approve_loser_of_concurrent_race_leaves_no_usable_session() {
        // Two approvers race the same pending row. Exactly one wins the atomic
        // update; the loser either short-circuits before minting (sees a
        // non-pending row) or mints and then hits `updated.is_none()` and must
        // revoke its just-minted session. Either way the invariant that must
        // hold is: no usable (non-revoked) session exists beyond the winner's,
        // and that session is the one recorded on the approved row.
        let Some(db) = connect_test_database("auth_device_approve_race_rollback").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let approve_input = || ApproveInput {
            user_id: user_id.clone(),
            user_code: "ABCD-1234".to_string(),
            approver_ip: None,
            approver_user_agent: None,
        };
        let (left, right) = tokio::join!(
            approve(
                &db,
                &config,
                &jwt_keys,
                &encryption_keys,
                TEST_HMAC_KEY,
                approve_input(),
            ),
            approve(
                &db,
                &config,
                &jwt_keys,
                &encryption_keys,
                TEST_HMAC_KEY,
                approve_input(),
            )
        );

        let ok_count = [&left, &right].iter().filter(|r| r.is_ok()).count();
        let already_delivered_count = [&left, &right]
            .iter()
            .filter(|r| matches!(r, Err(AppError::AuthDeviceCodeAlreadyDelivered)))
            .count();
        assert_eq!(ok_count, 1, "left={left:?} right={right:?}");
        assert_eq!(already_delivered_count, 1, "left={left:?} right={right:?}");

        let approved = row_by_id(&db, &row.id).await;
        assert_eq!(approved.status, AuthDeviceCodeStatus::Approved);
        let winner_session = approved
            .approved_session_id
            .clone()
            .expect("approved session id");

        // No orphaned usable session: exactly one non-revoked session, and it is
        // the winner's. Any session the loser minted before the failed update
        // must have been revoked by the rollback path.
        let live_sessions = db
            .collection::<bson::Document>(SESSIONS)
            .count_documents(doc! { "revoked": false })
            .await
            .expect("live session count");
        assert_eq!(live_sessions, 1, "exactly one usable session must survive");
        let winner_revoked = db
            .collection::<crate::models::session::Session>(SESSIONS)
            .find_one(doc! { "_id": &winner_session })
            .await
            .expect("winner session query")
            .expect("winner session row")
            .revoked;
        assert!(!winner_revoked, "the delivered session must remain usable");
    }

    #[tokio::test]
    async fn decrypt_tokens_roundtrips_encrypted_jwt_bytes() {
        let encryption_keys = test_encryption_keys();
        let access = "eyJ.access.jwt";
        let refresh = "eyJ.refresh.jwt";
        let encrypted_access = encryption_keys.encrypt(access.as_bytes()).await.unwrap();
        let encrypted_refresh = encryption_keys.encrypt(refresh.as_bytes()).await.unwrap();

        let (decrypted_access, decrypted_refresh) =
            decrypt_tokens(&encryption_keys, &encrypted_access, &encrypted_refresh)
                .await
                .unwrap();

        assert_eq!(decrypted_access, access);
        assert_eq!(decrypted_refresh, refresh);
    }

    #[tokio::test]
    async fn slow_down_repoll_increments_counter() {
        let Some(db) = connect_test_database("auth_device_slow_down").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let output = initiate(&db, TEST_HMAC_KEY, empty_input())
            .await
            .expect("initiate");

        assert_eq!(
            poll_and_claim(&db, TEST_HMAC_KEY, &output.device_code)
                .await
                .expect("first poll"),
            PollClaim::Pending
        );
        assert_eq!(
            poll_and_claim(&db, TEST_HMAC_KEY, &output.device_code)
                .await
                .expect("second poll"),
            PollClaim::SlowDown
        );

        let row = collection(&db)
            .find_one(doc! {
                "device_code_hmac": hmac_hex(TEST_HMAC_KEY, output.device_code.as_bytes())
            })
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.slow_down_increments, 1);
    }

    #[tokio::test]
    async fn expired_poll_marks_expired() {
        let Some(db) = connect_test_database("auth_device_expired").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Pending,
            Utc::now() - Duration::seconds(1),
        )
        .await;

        assert_eq!(
            poll_and_claim(&db, TEST_HMAC_KEY, "device-code")
                .await
                .expect("poll"),
            PollClaim::Expired
        );
        let updated = collection(&db)
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(updated.status, AuthDeviceCodeStatus::Expired);
    }

    #[tokio::test]
    async fn concurrent_approved_claim_has_exactly_one_ready_winner() {
        let Some(db) = connect_test_database("auth_device_concurrent_claim").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        seed_row(
            &db,
            AuthDeviceCodeStatus::Approved,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let (left, right) = tokio::join!(
            poll_and_claim(&db, TEST_HMAC_KEY, "device-code"),
            poll_and_claim(&db, TEST_HMAC_KEY, "device-code")
        );
        let outcomes = [left.expect("left poll"), right.expect("right poll")];

        let ready_count = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PollClaim::Ready { .. }))
            .count();
        let already_delivered_count = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PollClaim::AlreadyDelivered))
            .count();

        assert_eq!(ready_count, 1, "{outcomes:?}");
        assert_eq!(already_delivered_count, 1, "{outcomes:?}");
    }

    #[tokio::test]
    async fn successful_claim_removes_ciphertext_fields_from_db() {
        let Some(db) = connect_test_database("auth_device_claim_unsets_ciphertext").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let row = seed_row(
            &db,
            AuthDeviceCodeStatus::Approved,
            Utc::now() + Duration::minutes(10),
        )
        .await;

        let claim = poll_and_claim(&db, TEST_HMAC_KEY, "device-code")
            .await
            .expect("poll");
        assert_eq!(
            claim,
            PollClaim::Ready {
                encrypted_access: b"encrypted-access".to_vec(),
                encrypted_refresh: b"encrypted-refresh".to_vec(),
                expires_in: 900,
                approved_user_id: "approved-user-id".to_string(),
                approved_session_id: "approved-session-id".to_string(),
            }
        );

        let raw = db
            .collection::<bson::Document>(AUTH_DEVICE_CODES)
            .find_one(doc! { "_id": row.id })
            .await
            .expect("query")
            .expect("row exists");
        assert!(!raw.contains_key("delivery_access_token_encrypted"));
        assert!(!raw.contains_key("delivery_refresh_token_encrypted"));
        assert!(raw.contains_key("delivery_access_token_expires_in"));
    }

    #[test]
    fn auth_device_debug_redaction_still_hides_hashes_and_ciphertext() {
        let row = make_debug_row();
        let debug = format!("{row:?}");

        for secret in [
            row.device_code_hmac.as_str(),
            row.user_code_hmac.as_str(),
            row.client_ip_hmac.as_deref().unwrap(),
            row.approver_ip_hmac.as_deref().unwrap(),
            "abcdef",
            "123456",
        ] {
            assert!(!debug.contains(secret), "{secret} leaked in {debug}");
        }

        assert!(debug.contains("Pending"));
        assert!(debug.contains("created_at"));
        assert!(debug.contains("expires_at"));
    }

    async fn seed_row(
        db: &Database,
        status: AuthDeviceCodeStatus,
        expires_at: DateTime<Utc>,
    ) -> AuthDeviceCode {
        let now = Utc::now();
        let has_approval = matches!(
            status,
            AuthDeviceCodeStatus::Approved | AuthDeviceCodeStatus::Delivered
        );
        let row = AuthDeviceCode {
            supports_grant_choice: false,
            id: Uuid::new_v4().to_string(),
            device_code_hmac: hmac_hex(TEST_HMAC_KEY, b"device-code"),
            user_code_hmac: hmac_hex(TEST_HMAC_KEY, b"ABCD1234"),
            requested_profile: None,
            user_code_reservation_hmac: None,
            status,
            poll_interval_secs: AUTH_DEVICE_POLL_INTERVAL_SECS,
            slow_down_increments: 0,
            client_label: None,
            client_user_agent: None,
            client_ip: None,
            client_ip_attribution: AuthDeviceClientIpAttribution::Unavailable,
            client_country: None,
            client_city: None,
            client_region: None,
            client_continent: None,
            client_ip_timezone: None,
            initiating_origin: None,
            initiating_origin_status: AuthDeviceInitiatingOriginStatus::Absent,
            client_app: None,
            client_platform: None,
            client_model: None,
            client_form_factor: None,
            client_timezone: None,
            client_locale: None,
            client_screen_width: None,
            client_screen_height: None,
            client_device_pixel_ratio: None,
            client_hardware_concurrency: None,
            client_device_memory: None,
            client_ip_hmac: None,
            last_polled_at: None,
            approved_user_id: has_approval.then(|| "approved-user-id".to_string()),
            approved_session_id: has_approval.then(|| "approved-session-id".to_string()),
            agent_key_grant: None,
            purge_at: None,
            approver_ip_hmac: None,
            delivery_access_token_encrypted: Some(b"encrypted-access".to_vec()),
            delivery_refresh_token_encrypted: Some(b"encrypted-refresh".to_vec()),
            delivery_access_token_expires_in: Some(900),
            created_at: now,
            approved_at: None,
            delivered_at: None,
            denied_at: None,
            denied_by_user_id: None,
            expires_at,
        };
        collection(db).insert_one(&row).await.expect("seed row");
        row
    }

    async fn seed_user(db: &Database, user_id: &str) {
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(user_id, UserType::Person))
            .await
            .expect("seed user");
    }

    async fn row_by_user_code(db: &Database, normalized_user_code: &str) -> AuthDeviceCode {
        collection(db)
            .find_one(doc! {
                "user_code_hmac": hmac_hex(TEST_HMAC_KEY, normalized_user_code.as_bytes())
            })
            .await
            .expect("query by user code")
            .expect("row exists")
    }

    async fn row_by_id(db: &Database, row_id: &str) -> AuthDeviceCode {
        collection(db)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("query by id")
            .expect("row exists")
    }

    fn empty_input() -> InitiateInput {
        InitiateInput::default()
    }

    fn make_debug_row() -> AuthDeviceCode {
        let now = Utc::now();
        AuthDeviceCode {
            supports_grant_choice: false,
            id: Uuid::new_v4().to_string(),
            device_code_hmac: "abc123ff".repeat(8),
            user_code_hmac: "def456aa".repeat(8),
            requested_profile: None,
            user_code_reservation_hmac: None,
            status: AuthDeviceCodeStatus::Pending,
            poll_interval_secs: 5,
            slow_down_increments: 0,
            client_label: Some("wsl-calvin".to_string()),
            client_user_agent: Some("nyxid-cli/0.8.0".to_string()),
            client_ip: Some("203.0.113.10".to_string()),
            client_ip_attribution: AuthDeviceClientIpAttribution::Unverified,
            client_country: None,
            client_city: None,
            client_region: None,
            client_continent: None,
            client_ip_timezone: None,
            initiating_origin: None,
            initiating_origin_status: AuthDeviceInitiatingOriginStatus::Absent,
            client_app: None,
            client_platform: None,
            client_model: None,
            client_form_factor: None,
            client_timezone: None,
            client_locale: None,
            client_screen_width: None,
            client_screen_height: None,
            client_device_pixel_ratio: None,
            client_hardware_concurrency: None,
            client_device_memory: None,
            client_ip_hmac: Some("11112222".repeat(8)),
            last_polled_at: Some(now),
            approved_user_id: Some(Uuid::new_v4().to_string()),
            approved_session_id: Some(Uuid::new_v4().to_string()),
            agent_key_grant: None,
            purge_at: None,
            approver_ip_hmac: Some("33334444".repeat(8)),
            delivery_access_token_encrypted: Some(vec![0xab, 0xcd, 0xef]),
            delivery_refresh_token_encrypted: Some(vec![0x12, 0x34, 0x56]),
            delivery_access_token_expires_in: Some(900),
            created_at: now,
            approved_at: Some(now),
            delivered_at: Some(now),
            denied_at: None,
            denied_by_user_id: None,
            expires_at: now + Duration::minutes(10),
        }
    }
}
