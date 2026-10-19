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
