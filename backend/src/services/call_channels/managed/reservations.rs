//! Durable owner of managed effect part meters reserved before admission.
//! Admission forwards its parts in the same transaction, so a part still
//! reserved and unforwarded was never admitted, and a managed send receipt still
//! prepared never passed its dispatch barrier. Both are provably unsent and are
//! finished here, never by stuck-request recovery (which excludes these meters).
//! A dispatched part whose outcome stays unknown settles on provider evidence,
//! else at zero once the established 72-hour evidence window has passed.
use super::super::{billing, session, sms_delivery};
use crate::{
    AppState,
    errors::AppResult,
    models::{
        channel_adapter::{EVENTS, EventReceipt},
        managed_comms::{ManagedCallConfirmation, ManagedCommsCostEvidence},
        usage_meter::{COLLECTION_NAME as USAGE, UsageStatus},
    },
    services::{audit_service, billing::reservation, transaction_service},
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};

/// Longer than an admission transaction; an overlapping release is serialized
/// by the admission's conditional forward of the same rows.
const UNADMITTED_AFTER: Duration = Duration::minutes(2);
/// Matches the prepared caller-confirmation recovery window.
const PREPARED_AFTER: Duration = Duration::minutes(5);
const BATCH: i64 = 100;

/// The terminal evidence recorded when a part settles without provider evidence.
pub(in crate::services::call_channels) const WAIVED: &str = "provider_evidence_unavailable";

pub async fn sweep(state: &AppState) -> AppResult<()> {
    let now = Utc::now();
    release_unadmitted(state, now - UNADMITTED_AFTER).await?;
    finish_prepared_sends(state, now - PREPARED_AFTER).await?;
    waive_unconfirmed_sends(state, now).await
}

/// Releases reserved parts whose effect was never admitted, for every caller
/// confirmation (managed or owner-funded) and managed send.
pub(super) async fn release_unadmitted(state: &AppState, cutoff: DateTime<Utc>) -> AppResult<u64> {
    let rows: Vec<Document> = state
        .db
        .collection::<Document>(USAGE)
        .find(doc! {
            "$or": [
                {"billing_request_id": {"$regex": "^call-confirmation:"}},
                {"billing_request_id": {"$regex": "^managed-sms-out:"}},
            ],
            "status": "reserved", "forwarded": false, "released": false,
            "created_at": {"$lt": bson::DateTime::from_chrono(cutoff)},
        })
        .projection(doc! {"billing_request_id": 1})
        .limit(BATCH)
        .await?
        .try_collect()
        .await?;
    let mut requests: Vec<String> = rows
        .iter()
        .filter_map(|row| row.get_str("billing_request_id").ok().map(str::to_owned))
        .collect();
    requests.sort();
    requests.dedup();
    let mut released = 0;
    for request in requests {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        match reservation::release_unforwarded_rows(
            &state.db,
            &request,
            UsageStatus::Abandoned,
            Some("managed_unadmitted"),
        )
        .await
        {
            Ok(count) => released += count,
            Err(_) => tracing::warn!("Managed unadmitted reservation release pending"),
        }
    }
    Ok(released)
}

/// Finishes managed sends that never passed their dispatch barrier, including
/// a previous pass interrupted after claiming one.
pub(super) async fn finish_prepared_sends(
    state: &AppState,
    cutoff: DateTime<Utc>,
) -> AppResult<()> {
    let rows: Vec<EventReceipt> = state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find(doc! {
            "event_kind": "sms_send",
            "billing_request_id": {"$regex": "^managed-sms-out:"},
            "billing_finalized": false,
            "$or": [
                {"status": "prepared", "created_at": {"$lt": bson::DateTime::from_chrono(cutoff)}},
                {"status": "rejected_before_dispatch"},
            ],
        })
        .limit(BATCH)
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        if finish_prepared(state, &row).await.is_err() {
            tracing::warn!(send_id = %row.id, "Prepared managed send recovery pending");
        }
    }
    Ok(())
}

async fn finish_prepared(state: &AppState, row: &EventReceipt) -> AppResult<()> {
    let events = state.db.collection::<Document>(EVENTS);
    // Claiming the barrier fences the original writer, which can no longer send.
    if row.status == "prepared"
        && events
            .update_one(
                doc! {"_id": &row.id, "status": "prepared"},
                doc! {"$set": {"status": "rejected_before_dispatch", "delivery": "failed"}},
            )
            .await?
            .matched_count
            != 1
    {
        return Ok(());
    }
    for part in 0..row.reserved_parts.unwrap_or_default() {
        let request = sms_delivery::outbound_request_id(row, part)?;
        reservation::release_unforwarded_rows(
            &state.db,
            &request,
            UsageStatus::Failed,
            Some("managed_send_not_dispatched"),
        )
        .await?;
        billing::settle_admitted(state, &request, &row.user_id, 0, 0).await?;
    }
    events
        .update_one(
            doc! {"_id": &row.id, "status": "rejected_before_dispatch"},
            doc! {"$set": {"billing_finalized": true}},
        )
        .await?;
    sms_delivery::reconcile(state, &row.id).await
}

pub(in crate::services::call_channels) async fn waived(
    db: &mongodb::Database,
    send: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(EVENTS)
        .find_one(doc! {"_id": send, "terminal_evidence": WAIVED})
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

/// Dispatched caller-confirmation (managed and owner-funded) and managed reply
/// parts still without provider part evidence after the evidence window. A
/// known provider message is read once more first; only then is the part
/// settled at zero. The send keeps its consumed slot and is never retried.
pub(super) async fn waive_unconfirmed_sends(state: &AppState, now: DateTime<Utc>) -> AppResult<()> {
    let cutoff =
        bson::DateTime::from_chrono(now - Duration::hours(session::BYOK_EVIDENCE_WINDOW_HOURS));
    let rows: Vec<Document> = state
        .db
        .collection::<Document>(EVENTS)
        .find(doc! {
            "event_kind": "sms_send",
            "billing_finalized": false,
            "observed_parts": bson::Bson::Null,
            "status": {"$nin": ["prepared", "rejected", "rejected_before_dispatch"]},
            "$and": [
                {"$or": [
                    {"billing_request_id": {"$regex": "^call-confirmation:"}},
                    {"billing_request_id": {"$regex": "^managed-sms-out:"}},
                ]},
                {"$or": [
                    {"dispatched_at": {"$lt": cutoff}},
                    {"dispatched_at": {"$exists": false}, "created_at": {"$lt": cutoff}},
                ]},
            ],
        })
        .projection(doc! {"_id": 1})
        .limit(BATCH)
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let Ok(id) = row.get_str("_id") else {
            continue;
        };
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        // Provider evidence first: a known message ID is read once more.
        let _ = sms_delivery::reconcile(state, id).await;
        if waive(state, id).await.is_err() {
            tracing::warn!(send_id = %id, "Unconfirmed SMS part waiver pending");
        }
    }
    Ok(())
}

async fn waive(state: &AppState, id: &str) -> AppResult<()> {
    let db = transaction_service::durable_database(&state.db);
    let key = state.audit_chain_hmac_key.as_ref().as_ref();
    transaction_service::run_db_only(&db, &(&db, id, key), |tx, c| {
        Box::pin(async move {
            let (db, id, key) = *c;
            let open = doc! {
                "_id": id, "billing_finalized": false,
                "observed_parts": bson::Bson::Null,
                "terminal_evidence": {"$exists": false},
            };
            let Some(sent) = db
                .collection::<EventReceipt>(EVENTS)
                .find_one(open.clone())
                .session(&mut *tx)
                .await?
            else {
                return Ok(());
            };
            db.collection::<Document>(EVENTS)
                .update_one(open, doc! {"$set": {"terminal_evidence": WAIVED}})
                .session(&mut *tx)
                .await?;
            // Release completion no longer waits on a settled confirmation.
            db.collection::<Document>(ManagedCallConfirmation::COLLECTION_NAME)
                .update_one(doc! {"_id": id}, doc! {"$set": {"terminal_evidence": WAIVED}})
                .session(&mut *tx)
                .await?;
            db.collection::<Document>(ManagedCommsCostEvidence::COLLECTION_NAME)
                .update_many(
                    doc! {"effect_id": id},
                    doc! {"$set": {"terminal_evidence": WAIVED, "next_attempt_at": bson::DateTime::now()}},
                )
                .session(&mut *tx)
                .await?;
            let confirmation = sent
                .billing_request_id
                .as_deref()
                .is_some_and(|r| r.starts_with("call-confirmation:"));
            audit_service::log_system_event_in_session(
                db,
                tx,
                key,
                "channel_adapter_sms_provider_evidence_unavailable",
                serde_json::json!({
                    "send_id": id,
                    "binding_id": sent.binding_id,
                    "kind": if confirmation { "caller_confirmation" } else { "reply" },
                    "managed": sent.admitted_reference.is_some(),
                    "reserved_parts": sent.reserved_parts,
                    "evidence_window_hours": session::BYOK_EVIDENCE_WINDOW_HOURS,
                }),
            )
            .await?;
            Ok(())
        })
    })
    .await?;
    if let Some(sent) = state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id": id, "terminal_evidence": WAIVED, "billing_finalized": false})
        .await?
    {
        finish_waived(state, &sent).await?;
    }
    Ok(())
}

/// Zero settlement of a waived send; idempotent, and reconcile resumes it. A
/// callback arriving later only records delivery evidence and never charges.
pub(in crate::services::call_channels) async fn finish_waived(
    state: &AppState,
    sent: &EventReceipt,
) -> AppResult<()> {
    for part in 0..sent.reserved_parts.unwrap_or_default() {
        let request = sms_delivery::outbound_request_id(sent, part)?;
        reservation::release_unforwarded_rows(
            &state.db,
            &request,
            UsageStatus::Failed,
            Some(WAIVED),
        )
        .await?;
        billing::settle_admitted(state, &request, &sent.user_id, 0, 0).await?;
    }
    state
        .db
        .collection::<Document>(EVENTS)
        .update_one(
            doc! {"_id": &sent.id, "terminal_evidence": WAIVED},
            doc! {"$set": {"billing_finalized": true}},
        )
        .await?;
    if sent.admitted_reference.is_some() {
        super::sms::recognize_send(state, sent).await?;
    }
    Ok(())
}
