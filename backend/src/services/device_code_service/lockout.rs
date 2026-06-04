use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};

use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode};
use crate::models::device_pubkey_lockout::{
    COLLECTION_NAME as DEVICE_PUBKEY_LOCKOUTS, DevicePubkeyLockout,
};
use crate::services::org_service;
use sha2::{Digest, Sha256};

use super::{
    DEVICE_CODE_LOCKOUT_SECS, DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
    DeviceCodeLockoutNotification, SignatureFailureLockout, choose_device_label,
};

pub fn is_locked(locked_until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    locked_until.is_some_and(|until| until > now)
}

pub(super) fn device_pubkey_hash(pubkey: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pubkey);
    hex::encode(hasher.finalize())
}

pub(super) async fn is_pubkey_locked(
    db: &Database,
    pubkey: &[u8],
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let lockout = db
        .collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
        .find_one(doc! { "_id": device_pubkey_hash(pubkey) })
        .await?;
    Ok(lockout.is_some_and(|row| is_locked(row.locked_until, now)))
}

pub(super) async fn record_pubkey_signature_failure(
    db: &Database,
    pubkey: &[u8],
    now: DateTime<Utc>,
) -> AppResult<SignatureFailureLockout> {
    let collection = db.collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS);
    let id = device_pubkey_hash(pubkey);
    let now_bson = bson::DateTime::from_chrono(now);
    let updated = collection
        .find_one_and_update(
            doc! { "_id": &id },
            doc! {
                "$inc": { "failed_poll_count": 1_i64 },
                "$set": { "last_failure_at": now_bson },
                "$setOnInsert": {
                    "locked_until": bson::Bson::Null,
                    "last_lockout_audited_at": bson::Bson::Null,
                },
            },
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Internal("pubkey lockout upsert returned no row".to_string()))?;

    if is_locked(updated.locked_until, now)
        || updated.failed_poll_count < DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD
    {
        return Ok(SignatureFailureLockout {
            failed_poll_count: updated.failed_poll_count,
            locked_until: updated.locked_until,
        });
    }

    let locked_until = now + Duration::seconds(DEVICE_CODE_LOCKOUT_SECS);
    let locked = collection
        .find_one_and_update(
            doc! {
                "_id": &id,
                "$or": [
                    { "locked_until": bson::Bson::Null },
                    { "locked_until": { "$lte": now_bson } },
                ],
            },
            doc! { "$set": { "locked_until": bson::DateTime::from_chrono(locked_until) } },
        )
        .return_document(ReturnDocument::After)
        .await?;
    let locked = match locked {
        Some(locked) => locked,
        None => collection
            .find_one(doc! { "_id": &id })
            .await?
            .ok_or_else(|| AppError::Internal("pubkey lockout row disappeared".to_string()))?,
    };

    Ok(SignatureFailureLockout {
        failed_poll_count: locked.failed_poll_count,
        locked_until: locked.locked_until,
    })
}

pub(super) async fn reset_pubkey_lockout(db: &Database, pubkey: &[u8]) -> AppResult<()> {
    db.collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
        .update_one(
            doc! { "_id": device_pubkey_hash(pubkey) },
            doc! {
                "$set": {
                    "failed_poll_count": 0_i64,
                    "locked_until": bson::Bson::Null,
                    "last_lockout_audited_at": bson::Bson::Null,
                },
            },
        )
        .await?;
    Ok(())
}

pub async fn claim_lockout_notification(
    db: &Database,
    device_code_raw: &str,
) -> AppResult<Option<DeviceCodeLockoutNotification>> {
    let now = Utc::now();
    let row = db
        .collection::<DeviceCode>(DEVICE_CODES)
        .find_one(doc! { "device_code_hash": hash_token(device_code_raw) })
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let lockout = db
        .collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
        .find_one_and_update(
            doc! {
                "_id": device_pubkey_hash(&row.device_pubkey),
                "failed_poll_count": { "$gte": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD) },
                "locked_until": { "$gt": bson::DateTime::from_chrono(now) },
                "$or": [
                    { "last_lockout_audited_at": bson::Bson::Null },
                    { "last_lockout_audited_at": { "$lte": bson::DateTime::from_chrono(now - Duration::hours(24)) } },
                ],
            },
            doc! { "$set": { "last_lockout_audited_at": bson::DateTime::from_chrono(now) } },
        )
        .return_document(ReturnDocument::After)
        .await?;
    let Some(lockout) = lockout else {
        return Ok(None);
    };
    let Some(locked_until) = lockout.locked_until else {
        return Ok(None);
    };

    db.collection::<DeviceCode>(DEVICE_CODES)
        .update_one(
            doc! { "_id": &row.id },
            doc! { "$set": { "lock_alert_sent_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    let recipients = lockout_notification_recipients(db, &row).await?;
    let device_label = choose_device_label(&row, None)?;
    Ok(Some(DeviceCodeLockoutNotification {
        recipients,
        device_label,
        hw_id: row.hw_id,
        node_id: row.issued_node_id,
        device_pubkey_fingerprint: device_pubkey_fingerprint(&row.device_pubkey),
        failed_poll_count: lockout.failed_poll_count,
        locked_until,
    }))
}

async fn lockout_notification_recipients(
    db: &Database,
    row: &DeviceCode,
) -> AppResult<Vec<String>> {
    let mut recipients = if let Some(org_id) = row.approved_org_id.as_deref() {
        org_service::list_admin_user_ids(db, org_id).await?
    } else {
        Vec::new()
    };

    if recipients.is_empty()
        && let Some(user_id) = row.approved_by_user_id.as_ref()
    {
        recipients.push(user_id.clone());
    }

    recipients.sort();
    recipients.dedup();
    Ok(recipients)
}

fn device_pubkey_fingerprint(pubkey: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pubkey);
    let digest = hex::encode(hasher.finalize());
    digest[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::device_code_service::tests_support::{setup_pending_row, sign_poll};
    use crate::services::device_code_service::{DeviceCodePollInput, poll};
    use crate::test_utils::test_encryption_keys;
    use mongodb::bson::doc;
    use uuid::Uuid;

    #[test]
    fn is_locked_only_when_until_is_in_future() {
        let now = Utc::now();

        assert!(is_locked(Some(now + Duration::seconds(1)), now));
        assert!(!is_locked(Some(now), now));
        assert!(!is_locked(Some(now - Duration::seconds(1)), now));
        assert!(!is_locked(None, now));
    }

    #[tokio::test]
    async fn claim_lockout_notification_claims_once_and_returns_recipients() {
        let Some((db, response, key)) = setup_pending_row("device_code_lockout_claim").await else {
            return;
        };
        let pubkey = key.verifying_key().to_bytes();
        let approved_by = Uuid::new_v4().to_string();
        let locked_until = Utc::now() + Duration::hours(1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "failed_poll_count": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD),
                        "locked_until": bson::DateTime::from_chrono(locked_until),
                        "approved_by_user_id": &approved_by,
                    }
                },
            )
            .await
            .expect("lock row");
        seed_pubkey_lockout(
            &db,
            &pubkey,
            DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
            locked_until,
        )
        .await;

        let claim = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("claim")
            .expect("claimed");

        assert_eq!(claim.recipients, vec![approved_by]);
        assert_eq!(claim.device_label, "Kitchen cam");
        assert_eq!(claim.hw_id, "esp32-p4-cam-1");
        assert_eq!(
            claim.device_pubkey_fingerprint,
            device_pubkey_fingerprint(&pubkey)
        );
        assert_eq!(
            claim.failed_poll_count,
            DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD
        );
        assert_eq!(claim.locked_until.timestamp(), locked_until.timestamp());

        let second = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("second claim");
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn successful_poll_clears_claim_state_for_next_lockout_cycle() {
        let Some((db, response, key)) = setup_pending_row("device_code_lockout_claim_cycle").await
        else {
            return;
        };
        let pubkey = key.verifying_key().to_bytes();
        let approved_by = Uuid::new_v4().to_string();
        let locked_until = Utc::now() + Duration::hours(1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "failed_poll_count": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD),
                        "locked_until": bson::DateTime::from_chrono(locked_until),
                        "approved_by_user_id": &approved_by,
                    }
                },
            )
            .await
            .expect("lock row");
        seed_pubkey_lockout(
            &db,
            &pubkey,
            DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
            locked_until,
        )
        .await;

        claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("claim")
            .expect("claimed");
        let duplicate_claim = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("duplicate claim check");
        assert!(duplicate_claim.is_none());

        let expired_until = Utc::now() - Duration::seconds(1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "locked_until": bson::DateTime::from_chrono(expired_until),
                    }
                },
            )
            .await
            .expect("expire lockout");
        db.collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
            .update_one(
                doc! { "_id": device_pubkey_hash(&pubkey) },
                doc! {
                    "$set": {
                        "locked_until": bson::DateTime::from_chrono(expired_until),
                    }
                },
            )
            .await
            .expect("expire pubkey lockout");

        let timestamp = Utc::now().timestamp();
        let encryption_keys = test_encryption_keys();
        poll(
            &db,
            &encryption_keys,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("successful poll clears lockout claim state");

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .expect("load row")
            .expect("row exists");
        assert_eq!(row.failed_poll_count, 0);
        assert!(row.lock_alert_sent_at.is_none());
        let pubkey_lockout = db
            .collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
            .find_one(doc! { "_id": device_pubkey_hash(&pubkey) })
            .await
            .expect("query pubkey lockout")
            .expect("pubkey lockout exists");
        assert_eq!(pubkey_lockout.failed_poll_count, 0);
        assert!(pubkey_lockout.locked_until.is_none());
        assert!(pubkey_lockout.last_lockout_audited_at.is_none());

        let locked_until = Utc::now() + Duration::hours(1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "failed_poll_count": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD),
                        "locked_until": bson::DateTime::from_chrono(locked_until),
                    }
                },
            )
            .await
            .expect("lock row again");
        seed_pubkey_lockout(
            &db,
            &pubkey,
            DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
            locked_until,
        )
        .await;

        let next_claim = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("next cycle claim");
        assert!(next_claim.is_some());
    }

    async fn seed_pubkey_lockout(
        db: &Database,
        pubkey: &[u8],
        failed_poll_count: u32,
        locked_until: DateTime<Utc>,
    ) {
        db.collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
            .update_one(
                doc! { "_id": device_pubkey_hash(pubkey) },
                doc! {
                    "$set": {
                        "failed_poll_count": i64::from(failed_poll_count),
                        "locked_until": bson::DateTime::from_chrono(locked_until),
                        "last_failure_at": bson::DateTime::from_chrono(Utc::now()),
                        "last_lockout_audited_at": bson::Bson::Null,
                    }
                },
            )
            .upsert(true)
            .await
            .expect("seed pubkey lockout");
    }
}
