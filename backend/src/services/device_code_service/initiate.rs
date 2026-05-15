use chrono::{Duration, Utc};
use mongodb::Database;
use uuid::Uuid;

use crate::crypto::device_code::{generate_device_code, generate_user_code};
use crate::errors::{AppError, AppResult};
use crate::models::device_code::{
    COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus, UserCodeGen,
};

use super::{
    DEVICE_CODE_EXPIRES_IN_SECS, DEVICE_CODE_POLL_INTERVAL_SECS,
    DEVICE_CODE_USER_CODE_WRITE_RETRIES, DeviceCodeInitiate, DeviceCodeInitiateInput,
    is_duplicate_key_error, is_pubkey_locked,
};

pub async fn initiate(
    db: &Database,
    input: DeviceCodeInitiateInput,
) -> AppResult<DeviceCodeInitiate> {
    initiate_with_user_code_generator(db, input, generate_user_code).await
}

async fn initiate_with_user_code_generator<F>(
    db: &Database,
    input: DeviceCodeInitiateInput,
    mut user_code_generator: F,
) -> AppResult<DeviceCodeInitiate>
where
    F: FnMut() -> String,
{
    let DeviceCodeInitiateInput {
        device_pubkey,
        hw_id,
        suggested_label,
        frontend_url,
    } = input;

    if is_pubkey_locked(db, &device_pubkey, Utc::now()).await? {
        return Err(AppError::DeviceCodeLocked);
    }

    for attempt in 0..=DEVICE_CODE_USER_CODE_WRITE_RETRIES {
        let now = Utc::now();
        let (device_code, device_code_hash) = generate_device_code();
        let user_code = user_code_generator();
        let (verification_uri, verification_uri_complete) =
            build_verification_uris(&frontend_url, &user_code)?;

        let row = DeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hash,
            device_pubkey: device_pubkey.to_vec(),
            hw_id: hw_id.clone(),
            suggested_label: suggested_label.clone(),
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
            lock_alert_sent_at: None,
            expires_at: now + Duration::seconds(DEVICE_CODE_EXPIRES_IN_SECS),
            created_at: now,
            last_polled_at: None,
            last_poll_timestamp: None,
            last_rotated_at: now,
        };

        match db
            .collection::<DeviceCode>(DEVICE_CODES)
            .insert_one(&row)
            .await
        {
            Ok(_) => {
                return Ok(DeviceCodeInitiate {
                    device_code,
                    user_code,
                    verification_uri,
                    verification_uri_complete,
                    expires_in: DEVICE_CODE_EXPIRES_IN_SECS,
                    poll_interval: DEVICE_CODE_POLL_INTERVAL_SECS,
                });
            }
            Err(error)
                if is_duplicate_key_error(&error)
                    && attempt < DEVICE_CODE_USER_CODE_WRITE_RETRIES =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(AppError::Internal(
        "device-code user_code collision retry limit exceeded".to_string(),
    ))
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
    use crate::models::device_pubkey_lockout::{
        COLLECTION_NAME as DEVICE_PUBKEY_LOCKOUTS, DevicePubkeyLockout,
    };
    use crate::services::device_code_service::DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD;
    use crate::test_utils::connect_test_database;
    use mongodb::bson::doc;
    use sha2::{Digest, Sha256};

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
    async fn initiate_retries_duplicate_pending_current_user_code() {
        let Some(db) = connect_test_database("device_code_initiate_duplicate_retry").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let duplicate_code = "AAAA-BBBB-CCCC".to_string();
        let unique_code = "DDDD-EEEE-FFFF".to_string();

        initiate_with_user_code_generator(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: [21u8; 32],
                hw_id: "esp32-existing".to_string(),
                suggested_label: None,
                frontend_url: "https://app.example.com".to_string(),
            },
            || duplicate_code.clone(),
        )
        .await
        .expect("seed duplicate code");

        let mut calls = 0;
        let response = initiate_with_user_code_generator(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: [22u8; 32],
                hw_id: "esp32-retry".to_string(),
                suggested_label: None,
                frontend_url: "https://app.example.com".to_string(),
            },
            || {
                calls += 1;
                if calls == 1 {
                    duplicate_code.clone()
                } else {
                    unique_code.clone()
                }
            },
        )
        .await
        .expect("retry after duplicate user code");

        assert_eq!(calls, 2);
        assert_eq!(response.user_code, unique_code);
        assert_eq!(
            db.collection::<DeviceCode>(DEVICE_CODES)
                .count_documents(doc! { "user_code_history.0.code": &duplicate_code })
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn initiate_rejects_currently_locked_pubkey() {
        let Some(db) = connect_test_database("device_code_initiate_locked_pubkey").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let pubkey = [31u8; 32];
        db.collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
            .insert_one(DevicePubkeyLockout {
                id: test_pubkey_hash(&pubkey),
                failed_poll_count: DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
                locked_until: Some(Utc::now() + Duration::hours(1)),
                last_failure_at: Utc::now(),
                last_lockout_audited_at: None,
            })
            .await
            .expect("seed pubkey lockout");

        let error = initiate(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: pubkey,
                hw_id: "esp32-locked".to_string(),
                suggested_label: None,
                frontend_url: "https://app.example.com".to_string(),
            },
        )
        .await
        .expect_err("locked pubkey should not get a new device code");

        assert!(matches!(error, AppError::DeviceCodeLocked));
    }

    fn test_pubkey_hash(pubkey: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(pubkey);
        hex::encode(hasher.finalize())
    }
}
