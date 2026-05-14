use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};

use crate::crypto::device_code::verify_poll_signature;
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus};

use super::rotation::rotate_user_code_if_needed;
use super::{
    DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS, DEVICE_CODE_POLL_INTERVAL_SECS,
    DEVICE_CODE_TIMESTAMP_SKEW_SECS, DeviceCodePoll, DeviceCodePollInput,
    apply_signature_failure_lockout, is_locked,
};

pub async fn poll(db: &Database, input: DeviceCodePollInput) -> AppResult<DeviceCodePoll> {
    let now = Utc::now();
    let collection = db.collection::<DeviceCode>(DEVICE_CODES);
    let mut row = collection
        .find_one(doc! {
            "device_code_hash": hash_token(&input.device_code),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::device_code_service::DEVICE_CODE_ROTATE_SECS;
    use crate::services::device_code_service::tests_support::{setup_pending_row, sign_poll};
    use chrono::Duration;
    use ed25519_dalek::SigningKey;

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
