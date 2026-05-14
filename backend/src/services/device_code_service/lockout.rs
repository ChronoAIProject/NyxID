use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};

use crate::crypto::token::hash_token;
use crate::errors::AppResult;
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode};
use crate::services::org_service;

use super::{
    DEVICE_CODE_LOCKOUT_SECS, DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
    DeviceCodeLockoutNotification, SignatureFailureLockout, choose_device_label,
};

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

pub async fn claim_lockout_notification(
    db: &Database,
    device_code_raw: &str,
) -> AppResult<Option<DeviceCodeLockoutNotification>> {
    let now = Utc::now();
    let row = db
        .collection::<DeviceCode>(DEVICE_CODES)
        .find_one_and_update(
            doc! {
                "device_code_hash": hash_token(device_code_raw),
                "failed_poll_count": { "$gte": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD) },
                "locked_until": { "$gt": bson::DateTime::from_chrono(now) },
                "lock_alert_sent_at": bson::Bson::Null,
            },
            doc! { "$set": { "lock_alert_sent_at": bson::DateTime::from_chrono(now) } },
        )
        .return_document(ReturnDocument::After)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let Some(locked_until) = row.locked_until else {
        return Ok(None);
    };

    let recipients = lockout_notification_recipients(db, &row).await?;
    let device_label = choose_device_label(&row, None)?;
    Ok(Some(DeviceCodeLockoutNotification {
        recipients,
        device_label,
        hw_id: row.hw_id,
        node_id: row.issued_node_id,
        failed_poll_count: row.failed_poll_count,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::device_code_service::tests_support::setup_pending_row;
    use mongodb::bson::doc;
    use uuid::Uuid;

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

    #[tokio::test]
    async fn claim_lockout_notification_claims_once_and_returns_recipients() {
        let Some((db, response, _key)) = setup_pending_row("device_code_lockout_claim").await
        else {
            return;
        };
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

        let claim = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("claim")
            .expect("claimed");

        assert_eq!(claim.recipients, vec![approved_by]);
        assert_eq!(claim.device_label, "Kitchen cam");
        assert_eq!(claim.hw_id, "esp32-p4-cam-1");
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
}
