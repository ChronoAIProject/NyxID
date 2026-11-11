//! Read-only exact allocation totals. No period creation, activation or cleanup.
use crate::{
    errors::AppResult,
    models::{
        credit_grant::{COLLECTION_NAME, CreditGrant},
        credits::Credits,
    },
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Serialize, ToSchema)]
pub struct AllocationSummary {
    pub remaining: Credits,
    pub unreserved: Credits,
    pub held: Credits,
    pub pending_activation: Credits,
    pub pending_reclaim: Credits,
    pub expired_awaiting_cleanup: Credits,
    pub restricted: Credits,
    pub generated_at: DateTime<Utc>,
}

pub async fn allocations(
    db: &mongodb::Database,
    owner: Option<&str>,
    now: DateTime<Utc>,
) -> AppResult<AllocationSummary> {
    let mut filter = doc! { "status": "active" };
    if let Some(owner) = owner {
        filter.insert("recipient_user_id", owner);
    }
    let mut cursor = db
        .collection::<CreditGrant>(COLLECTION_NAME)
        .find(filter)
        // Exclude recipient snapshots, notes and terminal history from this
        // streaming read. Only current financial state and required identity
        // fields are materialized; the cursor bounds memory independently of
        // the number of historical allocations.
        .projection(doc! {
            "_id": 1, "batch_id": 1, "recipient_user_id": 1,
            "target_kind": 1, "amount_credits": 1, "amount": 1, "amount_micros": 1,
            "remaining": 1, "remaining_micros": 1, "reserved": 1, "reserved_micros": 1,
            "scope": 1, "service_limits": 1, "active_settlement": 1,
            "expires_at": 1, "revoke_requested_at": 1, "issued_ledgered_at": 1,
            "status": 1, "granted_by": 1, "created_at": 1, "updated_at": 1,
        })
        .max_time(std::time::Duration::from_secs(10))
        .await?;
    let mut total = AllocationSummary {
        generated_at: now,
        ..Default::default()
    };
    while let Some(grant) = cursor.try_next().await? {
        let held = super::service_limits::held(&grant)?;
        total.held = total.held.checked_add(held)?;
        if grant.revoke_requested_at.is_some() {
            total.pending_reclaim = total
                .pending_reclaim
                .checked_add(grant.remaining.checked_sub(held)?)?;
            total.remaining = total.remaining.checked_add(grant.remaining)?;
        } else if grant.expires_at.is_some_and(|expiry| expiry <= now) {
            total.expired_awaiting_cleanup = total
                .expired_awaiting_cleanup
                .checked_add(grant.remaining.checked_sub(held)?)?;
            total.remaining = total.remaining.checked_add(held)?;
        } else {
            total.remaining = total.remaining.checked_add(grant.remaining)?;
            if grant.issued_ledgered_at.is_none() {
                total.pending_activation = total
                    .pending_activation
                    .checked_add(grant.remaining.checked_sub(held)?)?;
            } else {
                let unreserved = super::service_limits::realizable(&grant)?;
                total.unreserved = total.unreserved.checked_add(unreserved)?;
                if !grant.scope.all_services || !grant.service_limits.is_empty() {
                    total.restricted = total.restricted.checked_add(unreserved)?;
                }
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use mongodb::bson::{self, Document};
    use uuid::Uuid;
    #[tokio::test]
    async fn benefit_preview_totals_all_grants_with_caps_holds_and_ineligible_states() {
        let db = crate::test_utils::connect_transaction_test_database("benefit-preview").await;
        let now = Utc::now();
        let owner = Uuid::new_v4().to_string();
        let grant = |amount: &str| {
            doc! {
                "_id": Uuid::new_v4().to_string(), "batch_id": "batch", "recipient_user_id": &owner,
                "target_kind": "selected_users", "amount_credits": 0_i64, "amount": amount.parse::<Credits>().unwrap(),
                "remaining": amount.parse::<Credits>().unwrap(), "reserved": Credits::ZERO,
                "scope": {"all_services": true, "service_ids": [], "service_slugs": []}, "status": "active",
                "issued_ledgered_at": bson::DateTime::from_chrono(now), "granted_by": "admin",
                "created_at": bson::DateTime::from_chrono(now), "updated_at": bson::DateTime::from_chrono(now),
            }
        };
        let mut rows = (0..55).map(|_| grant("0.000000000001")).collect::<Vec<_>>();
        let mut capped = grant("10");
        capped.insert("reserved", "2".parse::<Credits>().unwrap());
        capped.insert(
            "scope",
            doc! {"all_services": false, "service_ids": ["svc"], "service_slugs": ["service"]},
        );
        capped.insert("service_limits", vec![doc! {"key": "rule", "service_ids": ["svc"], "service_slugs": ["service"], "limit": "5".parse::<Credits>().unwrap(), "consumed": "1".parse::<Credits>().unwrap(), "reserved": "2".parse::<Credits>().unwrap()}]);
        capped.insert("active_settlement", doc! {"operation_id": "extra", "service_limit_key": "rule", "usage_row_id": "usage", "reserved": "2".parse::<Credits>().unwrap(), "consume": "3".parse::<Credits>().unwrap(), "applied": false, "updated_at": bson::DateTime::from_chrono(now)});
        rows.push(capped);
        let mut pending = grant("5");
        pending.remove("issued_ledgered_at");
        rows.push(pending);
        let mut reclaim = grant("7");
        reclaim.insert("revoke_requested_at", bson::DateTime::from_chrono(now));
        rows.push(reclaim);
        let mut expired = grant("4");
        expired.insert("expires_at", bson::DateTime::from_chrono(now));
        rows.push(expired);
        let mut other = grant("100");
        other.insert("recipient_user_id", "other");
        rows.push(other);
        let mut revoked = grant("100");
        revoked.insert("status", "revoked");
        rows.push(revoked);
        db.collection::<Document>(COLLECTION_NAME)
            .insert_many(rows)
            .await
            .unwrap();
        let total = allocations(&db, Some(&owner), now).await.unwrap();
        assert_eq!(total.unreserved.to_string(), "1.000000000055");
        assert_eq!(total.held.to_string(), "3");
        assert_eq!(total.pending_activation.to_string(), "5");
        assert_eq!(total.pending_reclaim.to_string(), "7");
        assert_eq!(total.expired_awaiting_cleanup.to_string(), "4");
        assert_eq!(total.restricted.to_string(), "1");
        let (page, count) = super::super::grants::list_grants(&db, Some(&owner), None, 50, 0)
            .await
            .unwrap();
        assert_eq!(page.len(), 50);
        assert_eq!(count, 60);
    }
}
