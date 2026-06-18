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
use crate::models::auth_device_code::{
    AuthDeviceCode, AuthDeviceCodeStatus, COLLECTION_NAME as AUTH_DEVICE_CODES,
};
use crate::services::{audit_service, token_service};

type HmacSha256 = Hmac<sha2::Sha256>;

const AUTH_DEVICE_CODE_PREFIX: &str = "nyx_adc_";
const AUTH_DEVICE_EXPIRES_IN_SECS: i64 = 10 * 60;
const AUTH_DEVICE_POLL_INTERVAL_SECS: u32 = 5;
const AUTH_DEVICE_SLOW_DOWN_INCREMENT_SECS: i64 = 5;
const AUTH_DEVICE_USER_CODE_LEN: usize = 8;
const AUTH_DEVICE_USER_CODE_WRITE_RETRIES: usize = 5;
const AUTH_DEVICE_USER_CODE_ALPHABET: &[u8] = b"123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitiateInput {
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub client_ip: Option<String>,
}

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
    Ready {
        encrypted_access: Vec<u8>,
        encrypted_refresh: Vec<u8>,
        expires_in: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewOutput {
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub initiated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: AuthDeviceCodeStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApproveInput {
    pub user_id: String,
    pub user_code: String,
    pub approver_ip: Option<String>,
    pub approver_user_agent: Option<String>,
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
    let client_label = sanitize_optional(input.client_label, 64);
    let client_user_agent = sanitize_optional(input.client_user_agent, 256);
    tracing::Span::current().record(
        "client_label_len",
        client_label.as_ref().map(|label| label.len()).unwrap_or(0),
    );

    initiate_with_user_code_generator(
        db,
        hmac_key,
        client_label,
        client_user_agent,
        input.client_ip,
        generate_user_code,
    )
    .await
}

async fn initiate_with_user_code_generator<F>(
    db: &Database,
    hmac_key: &[u8],
    client_label: Option<String>,
    client_user_agent: Option<String>,
    client_ip: Option<String>,
    mut user_code_generator: F,
) -> AppResult<InitiateOutput>
where
    F: FnMut() -> String,
{
    for attempt in 0..=AUTH_DEVICE_USER_CODE_WRITE_RETRIES {
        let now = Utc::now();
        let device_code = generate_device_code();
        let user_code_normalized = user_code_generator();
        let user_code = format_user_code(&user_code_normalized);

        let row = AuthDeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hmac: hmac_hex(hmac_key, device_code.as_bytes()),
            user_code_hmac: hmac_hex(hmac_key, user_code_normalized.as_bytes()),
            status: AuthDeviceCodeStatus::Pending,
            poll_interval_secs: AUTH_DEVICE_POLL_INTERVAL_SECS,
            slow_down_increments: 0,
            client_label: client_label.clone(),
            client_user_agent: client_user_agent.clone(),
            client_ip_hmac: client_ip
                .as_deref()
                .map(|client_ip| hmac_hex(hmac_key, client_ip.as_bytes())),
            last_polled_at: None,
            approved_user_id: None,
            approved_session_id: None,
            approver_ip_hmac: None,
            delivery_access_token_encrypted: None,
            delivery_refresh_token_encrypted: None,
            delivery_access_token_expires_in: None,
            created_at: now,
            approved_at: None,
            delivered_at: None,
            denied_at: None,
            expires_at: now + Duration::seconds(AUTH_DEVICE_EXPIRES_IN_SECS),
        };

        match collection(db).insert_one(&row).await {
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
    let collection = collection(db);
    let now = Utc::now();
    let device_code_hmac = hmac_hex(hmac_key, device_code.as_bytes());
    let row = collection
        .find_one(doc! { "device_code_hmac": device_code_hmac })
        .await?
        .ok_or(AppError::AuthDeviceCodeNotFound)?;

    tracing::Span::current().record("row_id", row.id.as_str());

    if row.expires_at < now {
        mark_expired(&collection, &row.id, now).await?;
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
        AuthDeviceCodeStatus::Approved => deliver_approved_claim(&collection, &row, now).await?,
    };

    record_poll_outcome(&row.id, poll_claim_outcome(&outcome));
    Ok(outcome)
}

#[tracing::instrument(name = "auth_device.preview", skip_all, fields(row_id))]
pub async fn preview(db: &Database, hmac_key: &[u8], user_code: &str) -> AppResult<PreviewOutput> {
    let normalized = normalize_user_code(user_code)?;
    let user_code_hmac = hmac_hex(hmac_key, normalized.as_bytes());
    let row = collection(db)
        .find_one(doc! { "user_code_hmac": user_code_hmac })
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;

    tracing::Span::current().record("row_id", row.id.as_str());

    Ok(PreviewOutput {
        client_label: row.client_label,
        client_user_agent: row.client_user_agent,
        initiated_at: row.created_at,
        expires_at: row.expires_at,
        status: row.status,
    })
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
    let collection = collection(db);
    let now = Utc::now();

    let row = collection
        .find_one(doc! { "user_code_hmac": user_code_hmac })
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;

    tracing::Span::current().record("row_id", row.id.as_str());

    if row.expires_at < now {
        return Err(AppError::AuthDeviceCodeExpired);
    }

    if row.status != AuthDeviceCodeStatus::Pending {
        return Err(non_pending_approve_error(row.status));
    }

    let user_agent = approve_session_user_agent(input.approver_user_agent.as_deref());
    let tokens = token_service::create_session_and_issue_tokens(
        db,
        config,
        jwt_keys,
        &input.user_id,
        input.approver_ip.as_deref(),
        Some(user_agent.as_str()),
    )
    .await?;
    tracing::Span::current().record("session_id", tokens.session_id.as_str());
    let session_id = tokens.session_id.clone();

    let access_plaintext = Zeroizing::new(tokens.access_token.into_bytes());
    let refresh_plaintext = Zeroizing::new(tokens.refresh_token.into_bytes());
    let encrypted_access = match encryption_keys.encrypt(access_plaintext.as_slice()).await {
        Ok(encrypted) => encrypted,
        Err(error) => {
            cleanup_issued_session(db, &session_id).await;
            return Err(error);
        }
    };
    let encrypted_refresh = match encryption_keys.encrypt(refresh_plaintext.as_slice()).await {
        Ok(encrypted) => encrypted,
        Err(error) => {
            cleanup_issued_session(db, &session_id).await;
            return Err(error);
        }
    };

    let approved_status = bson::to_bson(&AuthDeviceCodeStatus::Approved)
        .map_err(|e| AppError::Internal(format!("serialize auth device status: {e}")))?;
    let approved_at = Utc::now();
    let delivery_expires_at = approved_at + Duration::seconds(60);
    let approver_ip_hmac = input
        .approver_ip
        .as_deref()
        .map(|ip| hmac_hex(hmac_key, ip.as_bytes()));

    let mut set_doc = doc! {
        "status": approved_status,
        "approved_user_id": &input.user_id,
        "approved_session_id": &tokens.session_id,
        "approved_at": bson::DateTime::from_chrono(approved_at),
        "delivery_access_token_encrypted": Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: encrypted_access,
        }),
        "delivery_refresh_token_encrypted": Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: encrypted_refresh,
        }),
        "delivery_access_token_expires_in": tokens.access_expires_in,
        "expires_at": bson::DateTime::from_chrono(delivery_expires_at),
    };
    match approver_ip_hmac {
        Some(ip_hmac) => {
            set_doc.insert("approver_ip_hmac", ip_hmac);
        }
        None => {
            set_doc.insert("approver_ip_hmac", Bson::Null);
        }
    }

    let updated = collection
        .find_one_and_update(
            doc! { "_id": &row.id, "status": "pending" },
            doc! { "$set": set_doc },
        )
        .return_document(ReturnDocument::After)
        .await?;

    if updated.is_none() {
        cleanup_issued_session(db, &session_id).await;
        return Err(AppError::AuthDeviceCodeAlreadyDelivered);
    }

    audit_service::log_async(
        db.clone(),
        Some(input.user_id.clone()),
        "auth_device_code_approved".to_string(),
        Some(serde_json::json!({
            "session_id": session_id,
            "user_code_redacted": redact_user_code(&normalized),
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

    if normalized.len() == AUTH_DEVICE_USER_CODE_LEN {
        Ok(normalized)
    } else {
        Err(AppError::AuthDeviceUserCodeInvalid)
    }
}

pub fn format_user_code(normalized: &str) -> String {
    if normalized.len() <= 4 {
        return normalized.to_string();
    }
    format!("{}-{}", &normalized[..4], &normalized[4..])
}

fn collection(db: &Database) -> Collection<AuthDeviceCode> {
    db.collection::<AuthDeviceCode>(AUTH_DEVICE_CODES)
}

fn generate_device_code() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{AUTH_DEVICE_CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn generate_user_code() -> String {
    let mut rng = OsRng;
    (0..AUTH_DEVICE_USER_CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..AUTH_DEVICE_USER_CODE_ALPHABET.len());
            AUTH_DEVICE_USER_CODE_ALPHABET[idx] as char
        })
        .collect()
}

fn hmac_hex(hmac_key: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC-SHA256 accepts any key length");
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

fn sanitize_optional(value: Option<String>, max_len: usize) -> Option<String> {
    let value = value?;
    let sanitized: String = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_len)
        .collect();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
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
            doc! { "_id": row_id, "status": "approved" },
            doc! {
                "$set": {
                    "status": delivered_status,
                    "delivered_at": bson::DateTime::from_chrono(now),
                    "last_polled_at": bson::DateTime::from_chrono(now),
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
) -> AppResult<PollClaim> {
    match claim_approved_delivery(collection, &row.id, now).await? {
        Some(claimed) => {
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
            tracing::info!("auth_device.deliver");
            Ok(PollClaim::Ready {
                encrypted_access,
                encrypted_refresh,
                expires_in,
            })
        }
        None => Ok(PollClaim::AlreadyDelivered),
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
            Some("workstation".to_string()),
            Some("nyxid-cli/0.8.0".to_string()),
            None,
            || "ABCD1234".to_string(),
        )
        .await
        .expect("initiate");

        let preview = preview(&db, TEST_HMAC_KEY, &output.user_code)
            .await
            .expect("preview");

        assert_eq!(preview.client_label.as_deref(), Some("workstation"));
        assert_eq!(
            preview.client_user_agent.as_deref(),
            Some("nyxid-cli/0.8.0")
        );
        assert_eq!(preview.status, AuthDeviceCodeStatus::Pending);
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
            Some("workstation".to_string()),
            Some("nyxid-cli/0.8.0".to_string()),
            None,
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
                .and_then(|data| data.get("user_code_redacted"))
                .and_then(serde_json::Value::as_str),
            Some("AB****GH")
        );
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
        let row = AuthDeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hmac: hmac_hex(TEST_HMAC_KEY, b"device-code"),
            user_code_hmac: hmac_hex(TEST_HMAC_KEY, b"ABCD1234"),
            status,
            poll_interval_secs: AUTH_DEVICE_POLL_INTERVAL_SECS,
            slow_down_increments: 0,
            client_label: None,
            client_user_agent: None,
            client_ip_hmac: None,
            last_polled_at: None,
            approved_user_id: None,
            approved_session_id: None,
            approver_ip_hmac: None,
            delivery_access_token_encrypted: Some(b"encrypted-access".to_vec()),
            delivery_refresh_token_encrypted: Some(b"encrypted-refresh".to_vec()),
            delivery_access_token_expires_in: Some(900),
            created_at: now,
            approved_at: None,
            delivered_at: None,
            denied_at: None,
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
        InitiateInput {
            client_label: None,
            client_user_agent: None,
            client_ip: None,
        }
    }

    fn make_debug_row() -> AuthDeviceCode {
        let now = Utc::now();
        AuthDeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hmac: "abc123ff".repeat(8),
            user_code_hmac: "def456aa".repeat(8),
            status: AuthDeviceCodeStatus::Pending,
            poll_interval_secs: 5,
            slow_down_increments: 0,
            client_label: Some("wsl-calvin".to_string()),
            client_user_agent: Some("nyxid-cli/0.8.0".to_string()),
            client_ip_hmac: Some("11112222".repeat(8)),
            last_polled_at: Some(now),
            approved_user_id: Some(Uuid::new_v4().to_string()),
            approved_session_id: Some(Uuid::new_v4().to_string()),
            approver_ip_hmac: Some("33334444".repeat(8)),
            delivery_access_token_encrypted: Some(vec![0xab, 0xcd, 0xef]),
            delivery_refresh_token_encrypted: Some(vec![0x12, 0x34, 0x56]),
            delivery_access_token_expires_in: Some(900),
            created_at: now,
            approved_at: Some(now),
            delivered_at: Some(now),
            denied_at: None,
            expires_at: now + Duration::minutes(10),
        }
    }
}
