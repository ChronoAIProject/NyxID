#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;
use uuid::Uuid;

use crate::crypto::device_code::{generate_device_code, generate_user_code, verify_poll_signature};
use crate::errors::{AppError, AppResult};
use crate::models::device_code::{
    COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus, UserCodeGen,
};

pub const DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD: u32 = 3;
pub const DEVICE_CODE_LOCKOUT_SECS: i64 = 60 * 60;
pub const DEVICE_CODE_EXPIRES_IN_SECS: i64 = 15 * 60;
pub const DEVICE_CODE_POLL_INTERVAL_SECS: u32 = 5;
pub const DEVICE_CODE_ROTATE_SECS: i64 = 30;
pub const DEVICE_CODE_TIMESTAMP_SKEW_SECS: i64 = 60;
pub const DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug)]
pub struct DeviceCodeInitiateInput {
    pub device_pubkey: [u8; 32],
    pub hw_id: String,
    pub suggested_label: Option<String>,
    pub frontend_url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DeviceCodeInitiate {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub poll_interval: u32,
}

#[derive(Clone, Debug)]
pub struct DeviceCodePollInput {
    pub device_code: String,
    pub timestamp: i64,
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum DeviceCodePoll {
    Pending {
        current_user_code: String,
        interval: u32,
    },
    Approved {
        api_key: String,
        node_id: String,
        refresh_token: String,
        expires_in: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignatureFailureLockout {
    pub failed_poll_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
}

pub fn apply_signature_failure_lockout(
    current_failed_poll_count: u32,
    now: DateTime<Utc>,
) -> SignatureFailureLockout {
    let failed_poll_count = current_failed_poll_count.saturating_add(1);
    let locked_until = (failed_poll_count >= DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD)
        .then_some(now + Duration::seconds(DEVICE_CODE_LOCKOUT_SECS));

    SignatureFailureLockout {
        failed_poll_count,
        locked_until,
    }
}

pub fn is_locked(locked_until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    locked_until.is_some_and(|until| until > now)
}

pub async fn initiate(
    db: &Database,
    input: DeviceCodeInitiateInput,
) -> AppResult<DeviceCodeInitiate> {
    let now = Utc::now();
    let (device_code, device_code_hash) = generate_device_code();
    let user_code = generate_user_code();
    let (verification_uri, verification_uri_complete) =
        build_verification_uris(&input.frontend_url, &user_code)?;

    let row = DeviceCode {
        id: Uuid::new_v4().to_string(),
        device_code_hash,
        device_pubkey: input.device_pubkey.to_vec(),
        hw_id: input.hw_id,
        suggested_label: input.suggested_label,
        user_code_history: vec![UserCodeGen {
            code: user_code.clone(),
            generated_at: now,
        }],
        status: DeviceCodeStatus::Pending,
        approved_by_user_id: None,
        approved_org_id: None,
        issued_api_key_id: None,
        issued_node_id: None,
        delivery_api_key: None,
        delivery_refresh_token: None,
        refresh_token_hash: None,
        failed_poll_count: 0,
        locked_until: None,
        expires_at: now + Duration::seconds(DEVICE_CODE_EXPIRES_IN_SECS),
        created_at: now,
        last_polled_at: None,
        last_poll_timestamp: None,
        last_rotated_at: now,
    };

    db.collection::<DeviceCode>(DEVICE_CODES)
        .insert_one(&row)
        .await?;

    Ok(DeviceCodeInitiate {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete,
        expires_in: DEVICE_CODE_EXPIRES_IN_SECS,
        poll_interval: DEVICE_CODE_POLL_INTERVAL_SECS,
    })
}

pub async fn poll(db: &Database, input: DeviceCodePollInput) -> AppResult<DeviceCodePoll> {
    let now = Utc::now();
    let collection = db.collection::<DeviceCode>(DEVICE_CODES);
    let mut row = collection
        .find_one(doc! {
            "device_code_hash": crate::crypto::token::hash_token(&input.device_code),
        })
        .await?
        .ok_or(AppError::DeviceCodeNotFound)?;

    if row.status == DeviceCodeStatus::Delivered {
        return Err(AppError::DeviceCodeAlreadyDelivered);
    }

    if row.expires_at <= now || row.status == DeviceCodeStatus::Expired {
        collection
            .update_one(
                doc! { "_id": &row.id },
                doc! { "$set": { "status": "expired" } },
            )
            .await?;
        return Err(AppError::DeviceCodeExpired);
    }

    if is_locked(row.locked_until, now) {
        return Err(AppError::DeviceCodeLocked);
    }

    verify_poll_timestamp(&row, input.timestamp, now)?;

    let pubkey: [u8; 32] = row
        .device_pubkey
        .clone()
        .try_into()
        .map_err(|_| AppError::Internal("stored device_pubkey is not 32 bytes".to_string()))?;
    if let Err(error) = verify_poll_signature(
        &pubkey,
        &input.device_code,
        input.timestamp,
        &input.signature,
    ) {
        let transition = apply_signature_failure_lockout(row.failed_poll_count, now);
        let mut set_doc = doc! {
            "failed_poll_count": i64::from(transition.failed_poll_count),
            "last_polled_at": bson::DateTime::from_chrono(now),
        };
        if let Some(locked_until) = transition.locked_until {
            set_doc.insert("locked_until", bson::DateTime::from_chrono(locked_until));
        }
        collection
            .update_one(doc! { "_id": &row.id }, doc! { "$set": set_doc })
            .await?;

        if transition.locked_until.is_some() {
            return Err(AppError::DeviceCodeLocked);
        }
        return Err(error);
    }

    row.failed_poll_count = 0;
    row.last_polled_at = Some(now);
    row.last_poll_timestamp = Some(input.timestamp);

    match row.status {
        DeviceCodeStatus::Pending => {
            let current_user_code = rotate_user_code_if_needed(&mut row, now)?;
            persist_successful_poll(db, &row, None).await?;
            Ok(DeviceCodePoll::Pending {
                current_user_code,
                interval: DEVICE_CODE_POLL_INTERVAL_SECS,
            })
        }
        DeviceCodeStatus::Approved => {
            let api_key = row.delivery_api_key.clone().ok_or_else(|| {
                AppError::Internal("approved device code missing delivery api key".to_string())
            })?;
            let refresh_token = row.delivery_refresh_token.clone().ok_or_else(|| {
                AppError::Internal(
                    "approved device code missing delivery refresh token".to_string(),
                )
            })?;
            let node_id = row.issued_node_id.clone().ok_or_else(|| {
                AppError::Internal("approved device code missing issued node id".to_string())
            })?;

            persist_successful_poll(db, &row, Some(DeviceCodeStatus::Delivered)).await?;
            Ok(DeviceCodePoll::Approved {
                api_key,
                node_id,
                refresh_token,
                expires_in: DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS,
            })
        }
        DeviceCodeStatus::Denied => Err(AppError::Forbidden("Device code denied".to_string())),
        DeviceCodeStatus::Expired => Err(AppError::DeviceCodeExpired),
        DeviceCodeStatus::Delivered => Err(AppError::DeviceCodeAlreadyDelivered),
    }
}

fn verify_poll_timestamp(row: &DeviceCode, timestamp: i64, now: DateTime<Utc>) -> AppResult<()> {
    let skew = (timestamp - now.timestamp()).abs();
    if skew > DEVICE_CODE_TIMESTAMP_SKEW_SECS {
        return Err(AppError::DevicePollSignatureInvalid(
            "poll timestamp outside allowed skew".to_string(),
        ));
    }
    if let Some(last) = row.last_poll_timestamp
        && timestamp <= last
    {
        return Err(AppError::DevicePollSignatureInvalid(
            "poll timestamp replay detected".to_string(),
        ));
    }
    Ok(())
}

fn rotate_user_code_if_needed(row: &mut DeviceCode, now: DateTime<Utc>) -> AppResult<String> {
    if now.signed_duration_since(row.last_rotated_at) > Duration::seconds(DEVICE_CODE_ROTATE_SECS) {
        let new_code = generate_user_code();
        row.user_code_history.insert(
            0,
            UserCodeGen {
                code: new_code.clone(),
                generated_at: now,
            },
        );
        row.user_code_history.truncate(4);
        row.last_rotated_at = now;
        return Ok(new_code);
    }

    row.user_code_history
        .first()
        .map(|generation| generation.code.clone())
        .ok_or_else(|| AppError::Internal("device code has no user code history".to_string()))
}

async fn persist_successful_poll(
    db: &Database,
    row: &DeviceCode,
    status: Option<DeviceCodeStatus>,
) -> AppResult<()> {
    let mut set_doc = doc! {
        "failed_poll_count": 0_i64,
        "last_polled_at": bson::DateTime::from_chrono(row.last_polled_at.expect("set before persist")),
        "last_poll_timestamp": row.last_poll_timestamp.expect("set before persist"),
        "user_code_history": bson::to_bson(&row.user_code_history)
            .map_err(|e| AppError::Internal(format!("serialize user_code_history: {e}")))?,
        "last_rotated_at": bson::DateTime::from_chrono(row.last_rotated_at),
    };
    if let Some(ref status) = status {
        set_doc.insert(
            "status",
            bson::to_bson(&status)
                .map_err(|e| AppError::Internal(format!("serialize status: {e}")))?,
        );
    }

    let update = if matches!(status, Some(DeviceCodeStatus::Delivered)) {
        doc! {
            "$set": set_doc,
            "$unset": {
                "delivery_api_key": "",
                "delivery_refresh_token": "",
            },
        }
    } else {
        doc! { "$set": set_doc }
    };

    db.collection::<DeviceCode>(DEVICE_CODES)
        .update_one(doc! { "_id": &row.id }, update)
        .await?;
    Ok(())
}

fn build_verification_uris(frontend_url: &str, user_code: &str) -> AppResult<(String, String)> {
    let frontend = frontend_url.trim_end_matches('/');
    let verification_uri = format!("{frontend}/settings/devices/bind");
    let mut parsed = url::Url::parse(&verification_uri)
        .map_err(|_| AppError::Internal("FRONTEND_URL is not a valid URL".to_string()))?;
    parsed.query_pairs_mut().append_pair("user_code", user_code);
    Ok((verification_uri, parsed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::token::hash_token;
    use crate::test_utils::connect_test_database;
    use ed25519_dalek::{Signer, SigningKey};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[77u8; 32])
    }

    fn sign_poll(device_code: &str, timestamp: i64, key: &SigningKey) -> [u8; 64] {
        let mut message = Vec::with_capacity(device_code.len() + std::mem::size_of::<i64>());
        message.extend_from_slice(device_code.as_bytes());
        message.extend_from_slice(&timestamp.to_be_bytes());
        key.sign(&message).to_bytes()
    }

    async fn setup_pending_row(prefix: &str) -> Option<(Database, DeviceCodeInitiate, SigningKey)> {
        let db = connect_test_database(prefix).await?;
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let key = signing_key();
        let response = initiate(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: key.verifying_key().to_bytes(),
                hw_id: "esp32-p4-cam-1".to_string(),
                suggested_label: Some("Kitchen cam".to_string()),
                frontend_url: "https://app.example.com".to_string(),
            },
        )
        .await
        .expect("initiate");
        Some((db, response, key))
    }

    #[test]
    fn signature_failures_below_threshold_do_not_lock() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(1, now);

        assert_eq!(transition.failed_poll_count, 2);
        assert_eq!(transition.locked_until, None);
    }

    #[test]
    fn signature_failure_at_threshold_locks_for_one_hour() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(2, now);

        assert_eq!(transition.failed_poll_count, 3);
        assert_eq!(
            transition.locked_until.expect("locked").timestamp(),
            (now + Duration::hours(1)).timestamp()
        );
    }

    #[test]
    fn signature_failure_after_threshold_keeps_locking() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(3, now);

        assert_eq!(transition.failed_poll_count, 4);
        assert!(transition.locked_until.is_some());
    }

    #[test]
    fn is_locked_only_when_until_is_in_future() {
        let now = Utc::now();

        assert!(is_locked(Some(now + Duration::seconds(1)), now));
        assert!(!is_locked(Some(now), now));
        assert!(!is_locked(Some(now - Duration::seconds(1)), now));
        assert!(!is_locked(None, now));
    }

    #[test]
    fn verification_uris_point_to_bind_page_and_include_user_code() {
        let (uri, complete) =
            build_verification_uris("https://app.example.com/", "ABCD-EFGH-JKLM").unwrap();

        assert_eq!(uri, "https://app.example.com/settings/devices/bind");
        assert_eq!(
            complete,
            "https://app.example.com/settings/devices/bind?user_code=ABCD-EFGH-JKLM"
        );
    }

    #[tokio::test]
    async fn initiate_persists_pending_device_code_row() {
        let Some(db) = connect_test_database("device_code_initiate").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");

        let response = initiate(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: [11u8; 32],
                hw_id: "esp32-p4-cam-1".to_string(),
                suggested_label: Some("Kitchen cam".to_string()),
                frontend_url: "https://app.example.com".to_string(),
            },
        )
        .await
        .expect("initiate device code");

        assert_eq!(response.expires_in, DEVICE_CODE_EXPIRES_IN_SECS);
        assert_eq!(response.poll_interval, DEVICE_CODE_POLL_INTERVAL_SECS);
        assert!(response.verification_uri_complete.contains("user_code="));

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .expect("query")
            .expect("row exists");

        assert_eq!(row.status, DeviceCodeStatus::Pending);
        assert_eq!(row.device_pubkey, vec![11u8; 32]);
        assert_eq!(row.hw_id, "esp32-p4-cam-1");
        assert_eq!(row.suggested_label.as_deref(), Some("Kitchen cam"));
        assert_eq!(row.user_code_history.len(), 1);
        assert_eq!(row.user_code_history[0].code, response.user_code);
        assert_eq!(row.failed_poll_count, 0);
        assert!(row.locked_until.is_none());
    }

    #[tokio::test]
    async fn poll_pending_returns_current_user_code() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_pending").await else {
            return;
        };
        let timestamp = Utc::now().timestamp();

        let poll_response = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("poll");

        assert_eq!(
            poll_response,
            DeviceCodePoll::Pending {
                current_user_code: response.user_code,
                interval: DEVICE_CODE_POLL_INTERVAL_SECS,
            }
        );
    }

    #[tokio::test]
    async fn poll_wrong_signature_increments_and_locks_at_threshold() {
        let Some((db, response, _key)) = setup_pending_row("device_code_poll_wrong_sig").await
        else {
            return;
        };
        let wrong_key = SigningKey::from_bytes(&[88u8; 32]);

        for attempt in 0..3 {
            let timestamp = Utc::now().timestamp() + attempt;
            let error = poll(
                &db,
                DeviceCodePollInput {
                    device_code: response.device_code.clone(),
                    timestamp,
                    signature: sign_poll(&response.device_code, timestamp, &wrong_key),
                },
            )
            .await
            .expect_err("wrong signature should fail");

            if attempt < 2 {
                assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
            } else {
                assert!(matches!(error, AppError::DeviceCodeLocked));
            }
        }

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.failed_poll_count, 3);
        assert!(row.locked_until.is_some());
    }

    #[tokio::test]
    async fn poll_rejects_replayed_timestamp() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_replay").await else {
            return;
        };
        let timestamp = Utc::now().timestamp();
        let signature = sign_poll(&response.device_code, timestamp, &key);

        poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature,
            },
        )
        .await
        .expect("first poll");

        let error = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature,
            },
        )
        .await
        .expect_err("replay should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[tokio::test]
    async fn poll_rotates_pending_user_code_after_window() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_rotate").await else {
            return;
        };
        let old_rotated_at = Utc::now() - Duration::seconds(DEVICE_CODE_ROTATE_SECS + 1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! { "$set": { "last_rotated_at": bson::DateTime::from_chrono(old_rotated_at) } },
            )
            .await
            .expect("age row");

        let timestamp = Utc::now().timestamp();
        let poll_response = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("poll");

        let DeviceCodePoll::Pending {
            current_user_code, ..
        } = poll_response
        else {
            panic!("expected pending");
        };
        assert_ne!(current_user_code, response.user_code);

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.user_code_history.len(), 2);
        assert_eq!(row.user_code_history[0].code, current_user_code);
        assert_eq!(row.user_code_history[1].code, response.user_code);
    }

    #[tokio::test]
    async fn poll_expired_code_returns_expired() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_expired").await else {
            return;
        };
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .expect("expire row");
        let timestamp = Utc::now().timestamp();

        let error = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect_err("expired");
        assert!(matches!(error, AppError::DeviceCodeExpired));
    }

    #[tokio::test]
    async fn poll_approved_delivers_once_and_clears_delivery_secrets() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_approved").await else {
            return;
        };
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "status": "approved",
                        "issued_node_id": "node-1",
                        "delivery_api_key": "nyx_secret",
                        "delivery_refresh_token": "refresh_secret",
                        "refresh_token_hash": hash_token("refresh_secret"),
                    }
                },
            )
            .await
            .expect("approve row");
        let timestamp = Utc::now().timestamp();

        let poll_response = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("approved poll");

        assert_eq!(
            poll_response,
            DeviceCodePoll::Approved {
                api_key: "nyx_secret".to_string(),
                node_id: "node-1".to_string(),
                refresh_token: "refresh_secret".to_string(),
                expires_in: DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS,
            }
        );

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, DeviceCodeStatus::Delivered);
        assert!(row.delivery_api_key.is_none());
        assert!(row.delivery_refresh_token.is_none());

        let next_timestamp = timestamp + 1;
        let error = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp: next_timestamp,
                signature: sign_poll(&response.device_code, next_timestamp, &key),
            },
        )
        .await
        .expect_err("delivered");
        assert!(matches!(error, AppError::DeviceCodeAlreadyDelivered));
    }
}
