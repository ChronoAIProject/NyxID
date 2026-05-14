#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use mongodb::{Database, bson::doc};
use serde::Serialize;
use uuid::Uuid;

use crate::crypto::device_code::{generate_device_code, generate_user_code};
use crate::errors::{AppError, AppResult};
use crate::models::device_code::{
    COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus, UserCodeGen,
};

pub const DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD: u32 = 3;
pub const DEVICE_CODE_LOCKOUT_SECS: i64 = 60 * 60;
pub const DEVICE_CODE_EXPIRES_IN_SECS: i64 = 15 * 60;
pub const DEVICE_CODE_POLL_INTERVAL_SECS: u32 = 5;

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
        refresh_token_hash: None,
        failed_poll_count: 0,
        locked_until: None,
        expires_at: now + Duration::seconds(DEVICE_CODE_EXPIRES_IN_SECS),
        created_at: now,
        last_polled_at: None,
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
}
