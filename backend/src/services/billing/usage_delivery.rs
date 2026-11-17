//! Immutable send snapshots and acknowledgement evidence, outside request paths.
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    options::{IndexOptions, ReturnDocument},
};

use super::lago_client::{LagoEvent, LagoEventProperties};
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_usage_comparison::{BillingUsageComparison, COLLECTION_NAME as COMPARISONS},
        billing_usage_delivery::{
            BillingUsageDelivery, COLLECTION_NAME as DELIVERIES, DeliveryProperties, DeliveryState,
        },
        usage_meter::UsageMeterRow,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UsageTarget {
    pub customer_id: String,
    pub subscription_id: String,
}

pub(super) const RESOLVED_RETENTION_DAYS: i64 = 100;
const PREPARED_RECONCILE_BATCH: i64 = 64;
pub(super) const LEGACY_USAGE_SHAPE_INDEX: &str = "usage_comparison_legacy_shape_due";

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<Document>(DELIVERIES).create_indexes([
        IndexModel::builder().keys(doc! {"transaction_id":1})
            .options(IndexOptions::builder().unique(true).build()).build(),
        IndexModel::builder().keys(doc! {"owner_id":1,"customer_id":1,"subscription_id":1,"acknowledgement_sequence":1}).build(),
        IndexModel::builder().keys(doc! {"state":1,"last_checked_at":1,"_id":1}).build(),
        IndexModel::builder().keys(doc! {"owner_id":1,"customer_id":1,"subscription_id":1,"event_second":1,"acknowledgement_sequence":1}).build(),
        IndexModel::builder().keys(doc! {"owner_id":1,"customer_id":1,"subscription_id":1,"event_second":1})
            .options(IndexOptions::builder().partial_filter_expression(doc! {"state":"prepared"}).build()).build(),
        IndexModel::builder().keys(doc! {"expires_at":1})
            .options(IndexOptions::builder().expire_after(std::time::Duration::ZERO).build()).build(),
    ]).await?;
    db.collection::<Document>(COMPARISONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id":1,"customer_id":1,"subscription_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(COMPARISONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"retired_at":1,"next_visit_at":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(COMPARISONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"next_visit_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name(LEGACY_USAGE_SHAPE_INDEX.to_owned())
                        .partial_filter_expression(
                            doc! {"retired_at":null,"comparison.reason":"unsupported_usage_shape"},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub(super) async fn prepare(
    db: &Database,
    row: &UsageMeterRow,
    customer_id: &str,
    event: LagoEvent,
) -> AppResult<BillingUsageDelivery> {
    let receipt = BillingUsageDelivery {
        id: uuid::Uuid::new_v4().to_string(),
        transaction_id: event.transaction_id,
        owner_id: row.billing_owner_id.clone(),
        wallet_id: row
            .wallet_id
            .clone()
            .ok_or_else(|| invalid("Delivery has no wallet"))?,
        customer_id: customer_id.into(),
        subscription_id: event.external_subscription_id,
        external_customer_id: event.external_customer_id,
        code: event.code,
        event_second: event.timestamp,
        properties: DeliveryProperties {
            quantity: event.properties.quantity,
            model: event.properties.model,
            service_code: event.properties.service_code,
            layer: event.properties.layer,
        },
        state: DeliveryState::Prepared,
        acknowledgement_sequence: None,
        duplicate_acknowledgement: false,
        rejection_count: 0,
        prepared_at: Utc::now(),
        usage_created_at: Some(row.created_at),
        acknowledged_at: None,
        resolved_at: None,
        last_checked_at: None,
        last_rejected_at: None,
        reconciliation_issue: None,
        expires_at: None,
    };
    let insert = bson::to_document(&receipt).map_err(|_| invalid("Cannot encode delivery"))?;
    let mut saved = db
        .collection::<BillingUsageDelivery>(DELIVERIES)
        .find_one_and_update(
            doc! {"transaction_id":&receipt.transaction_id},
            doc! {"$setOnInsert":insert},
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| invalid("Prepared delivery is missing"))?;
    if saved.owner_id != receipt.owner_id || saved.wallet_id != receipt.wallet_id {
        return Err(invalid(
            "Delivery transaction belongs to another billing owner or wallet",
        ));
    }
    if saved.state == DeliveryState::Rejected {
        // An eligible meter row is about to retry. Its previous rejection is
        // history; an unresolved attempt must not expire with that rejection.
        db.collection::<Document>(DELIVERIES).update_one(
            doc! {"_id":&saved.id,"state":"rejected"},
            vec![doc! {"$set":{"state":"prepared","resolved_at":bson::Bson::Null,"expires_at":bson::Bson::Null,
                "rejection_count":{"$max":[{"$ifNull":["$rejection_count",0_i64]},1_i64]},
                "last_rejected_at":{"$ifNull":["$last_rejected_at","$resolved_at"]}}}],
        ).await?;
        saved.state = DeliveryState::Prepared;
        saved.last_rejected_at = saved.last_rejected_at.or(saved.resolved_at);
        saved.rejection_count = saved.rejection_count.max(1);
        saved.resolved_at = None;
        saved.expires_at = None;
    }
    Ok(saved)
}

pub(super) fn event(receipt: &BillingUsageDelivery) -> LagoEvent {
    LagoEvent {
        transaction_id: receipt.transaction_id.clone(),
        external_customer_id: receipt.external_customer_id.clone(),
        external_subscription_id: receipt.subscription_id.clone(),
        code: receipt.code.clone(),
        timestamp: receipt.event_second,
        properties: LagoEventProperties {
            quantity: receipt.properties.quantity.clone(),
            model: receipt.properties.model.clone(),
            service_code: receipt.properties.service_code.clone(),
            layer: receipt.properties.layer.clone(),
        },
    }
}

pub(super) async fn acknowledge(
    db: &Database,
    session: &mut ClientSession,
    receipt: &BillingUsageDelivery,
    duplicate: bool,
) -> AppResult<()> {
    let current = db
        .collection::<Document>(DELIVERIES)
        .find_one(doc! {"_id":&receipt.id})
        .session(&mut *session)
        .await?;
    let Some(current) = current else {
        return mark_capture_unknown(db, Some(session), receipt, "delivery_receipt_missing").await;
    };
    if !same_snapshot_identity(&current, receipt) {
        return record_acknowledgement_issue(db, session, receipt, "delivery_receipt_mismatch")
            .await;
    }
    let state = delivery_state(&current);
    if state == DeliveryState::Acknowledged {
        // A previously captured receipt already owns its sequence. Legacy or
        // repaired acknowledgements without a sequence remain outside capture.
        if current.get_i64("acknowledgement_sequence").is_err() {
            record_acknowledgement_issue(
                db,
                session,
                receipt,
                "delivery_acknowledged_outside_capture",
            )
            .await?;
        }
        return Ok(());
    }
    let now = bson::DateTime::now();
    let expires_at = now.to_chrono() + Duration::days(RESOLVED_RETENTION_DAYS);
    let known_state = matches!(state, DeliveryState::Prepared | DeliveryState::Rejected);
    let sequence = if known_state {
        if let Some(subscription) = receipt
            .subscription_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            let event_time =
                bson::DateTime::from_millis(receipt.event_second.saturating_mul(1_000));
            let uncertain = doc! {"$gt":[event_time,"$$NOW"]};
            let target = db.collection::<Document>(COMPARISONS)
                .find_one_and_update(
                    doc! {"owner_id":&receipt.owner_id,"customer_id":&receipt.customer_id,"subscription_id":subscription},
                    // Future events can outlive their fixed receipt TTL before their period.
                    vec![doc! {"$set":{
                        "_id":{"$ifNull":["$_id",uuid::Uuid::new_v4().to_string()]},
                        "wallet_id":{"$ifNull":["$wallet_id",{"$literal":&receipt.wallet_id}]},
                        "warming_since":{"$ifNull":["$warming_since","$$NOW"]},
                        "last_acknowledged_at":"$$NOW","coverage_verified":false,
                        "acknowledgement_sequence":{"$add":[{"$ifNull":["$acknowledgement_sequence",0_i64]},1_i64]},
                        "coverage_unknown_reason":{"$cond":[
                            &uncertain,
                            "future_delivery_event","$coverage_unknown_reason"]},
                        "uncertainty_revision":{"$add":[{"$ifNull":["$uncertainty_revision",0_i64]},
                            {"$cond":[&uncertain,1_i64,0_i64]}]},
                        "uncertain_through":{"$cond":[
                            &uncertain,
                            {"$max":["$uncertain_through","$$NOW",event_time]},"$uncertain_through"]},
                    }}])
                .upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?;
            target
                .and_then(|target| bson::from_document::<BillingUsageComparison>(target).ok())
                .map(|target| target.acknowledgement_sequence)
        } else {
            None
        }
    } else {
        None
    };
    let issue = if !known_state {
        Some("delivery_receipt_state_unknown")
    } else if sequence.is_none() {
        Some("delivery_receipt_sequence_unknown")
    } else {
        None
    };
    // Delivery was accepted even if its bookkeeping cannot be reconciled.
    // Only sequence-bearing acknowledgements belong to captured totals.
    let mut fields = doc! {"state":"acknowledged","acknowledgement_sequence":sequence,
    "acknowledged_at":now,"resolved_at":now,"expires_at":bson::DateTime::from_chrono(expires_at),
    "duplicate_acknowledgement":duplicate};
    if state == DeliveryState::Rejected {
        fields.insert(
            "rejection_count",
            current.get_i64("rejection_count").unwrap_or(0).max(1),
        );
        if let Ok(rejected_at) = current
            .get_datetime("last_rejected_at")
            .or_else(|_| current.get_datetime("resolved_at"))
        {
            fields.insert("last_rejected_at", *rejected_at);
        }
    }
    if let Some(issue) = issue {
        fields.insert("reconciliation_issue", issue);
        mark_capture_unknown(db, Some(&mut *session), receipt, issue).await?;
    }
    db.collection::<Document>(DELIVERIES)
        .update_one(doc! {"_id":&receipt.id}, doc! {"$set":fields})
        .session(session)
        .await?;
    Ok(())
}

pub(super) async fn reject(db: &Database, receipt: &BillingUsageDelivery) -> AppResult<()> {
    let now = bson::DateTime::now();
    db.collection::<Document>(DELIVERIES).update_one(
        doc! {"_id":&receipt.id,"state":{"$ne":"acknowledged"}},
        vec![doc! {"$set":{"state":"rejected",
            "last_rejected_at":now,
            "rejection_count":{"$add":[{"$ifNull":["$rejection_count",0_i64]},1_i64]},
            "resolved_at":{"$ifNull":["$resolved_at",now]}}},
            doc! {"$set":{"expires_at":{"$add":["$resolved_at",Duration::days(RESOLVED_RETENTION_DAYS).num_milliseconds()]}}}],
    ).await?;
    Ok(())
}

/// Reconcile old-writer acknowledgements without inferring captured quantities.
pub(super) async fn reconcile_prepared(db: &Database) -> AppResult<()> {
    let receipts: Vec<BillingUsageDelivery> = db
        .collection::<BillingUsageDelivery>(DELIVERIES)
        .find(doc! {"state":"prepared"})
        .sort(doc! {"last_checked_at":1,"_id":1})
        .limit(PREPARED_RECONCILE_BATCH)
        .await?
        .try_collect()
        .await?;
    for receipt in receipts {
        if !super::reconcile::continue_reconcile() {
            break;
        }
        let meter = db
            .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! {"transaction_id":&receipt.transaction_id})
            .projection(doc! {"lago_acked":1})
            .await?;
        let now = bson::DateTime::now();
        let issue = match meter {
            None => Some("prepared_receipt_meter_missing"),
            Some(meter) if meter.get_bool("lago_acked").unwrap_or(false) => {
                Some("prepared_receipt_meter_already_acknowledged")
            }
            _ => None,
        };
        let mut fields = doc! {"last_checked_at":now};
        if let Some(issue) = issue {
            // Publish uncertainty first: a crash may leave a prepared receipt
            // for retry, but cannot lose the coverage warning after closing it.
            mark_capture_unknown(db, None, &receipt, issue).await?;
            fields.extend(doc! {"state":"acknowledged","resolved_at":now,
                "acknowledgement_sequence":bson::Bson::Null,
                "expires_at":bson::DateTime::from_chrono(now.to_chrono()+Duration::days(RESOLVED_RETENTION_DAYS)),
                "reconciliation_issue":issue});
        }
        // Move live receipts to the back of the index so a persistent backlog
        // cannot starve the rest of this bounded scan.
        db.collection::<Document>(DELIVERIES)
            .update_one(
                doc! {"_id":&receipt.id,"state":"prepared"},
                doc! {"$set":fields},
            )
            .await?;
    }
    Ok(())
}

fn delivery_state(document: &Document) -> DeliveryState {
    match document.get_str("state").unwrap_or_default() {
        "prepared" => DeliveryState::Prepared,
        "acknowledged" => DeliveryState::Acknowledged,
        "rejected" => DeliveryState::Rejected,
        _ => DeliveryState::Unknown,
    }
}

fn same_snapshot_identity(document: &Document, receipt: &BillingUsageDelivery) -> bool {
    bson::from_document::<BillingUsageDelivery>(document.clone()).is_ok_and(|current| {
        current.owner_id == receipt.owner_id
            && current.wallet_id == receipt.wallet_id
            && current.customer_id == receipt.customer_id
            && event(&current) == event(receipt)
    })
}

pub(super) async fn record_acknowledgement_issue(
    db: &Database,
    session: &mut ClientSession,
    receipt: &BillingUsageDelivery,
    issue: &str,
) -> AppResult<()> {
    db.collection::<Document>(DELIVERIES)
        .update_one(
            doc! {"_id":&receipt.id},
            doc! {"$set":{"reconciliation_issue":issue}},
        )
        .session(&mut *session)
        .await?;
    mark_capture_unknown(db, Some(session), receipt, issue).await
}

async fn mark_capture_unknown(
    db: &Database,
    session: Option<&mut ClientSession>,
    receipt: &BillingUsageDelivery,
    issue: &str,
) -> AppResult<()> {
    let Some(subscription) = receipt
        .subscription_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        return Ok(());
    };
    let comparisons = db.collection::<Document>(COMPARISONS);
    let event_time = bson::DateTime::from_millis(receipt.event_second.saturating_mul(1_000));
    let operation = comparisons.update_one(
        doc! {"owner_id":&receipt.owner_id,"customer_id":&receipt.customer_id,"subscription_id":subscription},
        vec![doc! {"$set":{
            "_id":{"$ifNull":["$_id",uuid::Uuid::new_v4().to_string()]},
            "wallet_id":{"$ifNull":["$wallet_id",{"$literal":&receipt.wallet_id}]},
            "warming_since":{"$ifNull":["$warming_since","$$NOW"]},
            "acknowledgement_sequence":{"$ifNull":["$acknowledgement_sequence",0_i64]},
            "coverage_verified":false,"coverage_unknown_reason":{"$literal":issue},
            "uncertainty_revision":{"$add":[{"$ifNull":["$uncertainty_revision",0_i64]},1_i64]},
            "uncertain_through":{"$max":[{"$ifNull":["$uncertain_through","$$NOW"]},"$$NOW",event_time]},
        }}],
    ).upsert(true);
    if let Some(session) = session {
        operation.session(session).await?;
    } else {
        operation.await?;
    }
    Ok(())
}

fn invalid(message: &str) -> AppError {
    AppError::Internal(message.into())
}
