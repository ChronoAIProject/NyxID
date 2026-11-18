use super::*;
use crate::models::usage_meter::{UsageMeterRow, UsageStatus};
use crate::services::billing::recovery_batch::{self, Failures};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;

pub(crate) async fn recover_stale(db: &mongodb::Database, now: DateTime<Utc>) -> AppResult<u64> {
    let rows: Vec<bson::Document> = db
        .collection::<bson::Document>(METERS)
        .find(doc! {
            "usage_checkpoint.final_report":true,
            "usage_checkpoint.recovered":false,
            "status":"forwarded", "released":false,
            "usage_checkpoint.recover_after":{"$lte":bson::DateTime::from_chrono(now)},
        })
        .sort(doc! {"usage_checkpoint.recover_after":1,"_id":1})
        .limit(recovery_batch::limit(100))
        .await?
        .try_collect()
        .await?;
    let mut recovered = 0;
    let mut failures = Failures::default();
    let mut seen = std::collections::BTreeSet::new();
    for raw in rows {
        if !super::super::reconcile::continue_reconcile() {
            break;
        }
        if let Some(request) = raw.get_str("billing_request_id").ok()
            && !seen.insert(request.to_owned())
        {
            continue;
        }
        if let Some(n) = failures
            .unit(async {
                let row: UsageMeterRow = recovery_batch::decode(&raw)?;
                recover_request(db, &row.billing_request_id, Some(now)).await
            })
            .await
        {
            recovered += n;
        }
    }
    failures.finish(recovered)
}

/// `None` is an explicit live attempt finish. A recovery cutoff rechecks an
/// active pool's lease inside the same transaction as final quantity writes.
pub(crate) async fn recover_request(
    db: &mongodb::Database,
    request_id: &str,
    cutoff: Option<DateTime<Utc>>,
) -> AppResult<u64> {
    let context = (db.clone(), request_id.to_owned(), cutoff);
    let rows = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |session, (db, request, cutoff)| {
            Box::pin(async move { promote_in_session(session, db, request, cutoff).await })
        },
    )
    .await?;
    #[cfg(test)]
    crate::billing_integration_tests::usage_checkpoints::point(
        crate::billing_integration_tests::usage_checkpoints::Phase::AfterPromotion,
    )
    .await;
    let count = rows.len() as u64;
    // Crash after the transaction is covered by ordinary finalized-row retry;
    // claim_released_and_settle still owns all money and ledger deduplication.
    meter::settle_persisted(db, rows).await?;
    Ok(count)
}

async fn promote_in_session(
    session: &mut mongodb::ClientSession,
    db: &mongodb::Database,
    request: &str,
    cutoff: &Option<DateTime<Utc>>,
) -> AppResult<Vec<UsageMeterRow>> {
    let collection = db.collection::<UsageMeterRow>(METERS);
    let mut cursor = collection
        .find(doc! {"billing_request_id":request})
        .limit((MAX_COMPONENTS + 1) as i64)
        .session(&mut *session)
        .await?;
    let rows: Vec<UsageMeterRow> = cursor.stream(&mut *session).try_collect().await?;
    if rows.len() > MAX_COMPONENTS {
        return Err(unavailable());
    }
    let Some(coordinator) = rows.iter().find(|r| r.usage_checkpoint.is_some()) else {
        return Ok(Vec::new());
    };
    let checkpoint = coordinator
        .usage_checkpoint
        .as_deref()
        .ok_or_else(unavailable)?;
    if !valid(checkpoint) {
        return Err(unavailable());
    }
    if !checkpoint.final_report
        || checkpoint.recovered
        || coordinator.status != UsageStatus::Forwarded
        || coordinator.released
        || rows.iter().any(|r| {
            r.pending_platform_usage.is_some()
                || r.pending_resale_quantity.is_some()
                || r.quantity.is_some()
        })
    {
        return Ok(Vec::new());
    }
    if let Some(cutoff) = cutoff
        && (rows.iter().any(|r| {
            r.pool_attempt
                .as_ref()
                .is_some_and(|a| a.lease_until > *cutoff)
        }) || (coordinator.pool_attempt.is_none() && checkpoint.recover_after > *cutoff))
    {
        return Ok(Vec::new());
    }
    let usage = platform_usage(checkpoint);
    #[cfg(test)]
    crate::billing_integration_tests::usage_checkpoints::point(
        crate::billing_integration_tests::usage_checkpoints::Phase::BeforePromotion,
    )
    .await;
    let mut digest = None;
    if let Some(vector) = &checkpoint.model_charges {
        let platform_rows: Vec<_> = rows
            .iter()
            .filter(|r| r.layer == BillingLayer::Platform)
            .collect();
        if platform_rows.len() != vector.quantities.len()
            || platform_rows.iter().any(|r| {
                r.model_tariff.as_ref().is_none_or(|b| {
                    b.tariff_id != vector.tariff_id
                        || b.public_model != vector.public_model
                        || vector
                            .quantities
                            .get(&r.lago_metric_code)
                            .is_none_or(|q| *q < 0 || *q > b.max_quantity)
                })
            })
        {
            return Err(unavailable());
        }
        use sha2::{Digest, Sha256};
        digest = Some(hex::encode(Sha256::digest(
            serde_json::to_vec(vector).map_err(|_| unavailable())?,
        )));
    } else if rows.iter().any(|r| r.model_tariff.is_some()) {
        return Ok(Vec::new());
    }
    let mut fields = doc! {"usage_checkpoint.recovered":true};
    if let Some(digest) = digest {
        fields.insert("model_tariff.evidence_digest", digest);
        fields.insert(
            "model_tariff.evidence",
            bson::to_bson(&checkpoint.model_charges).map_err(|_| unavailable())?,
        );
    }
    // This real write conflicts with a newer checkpoint, a normal
    // final intent, release, or another recoverer. Retrying the snapshot
    // rechecks every row; no old observation can override the winner.
    if collection.update_one(doc! {
    "_id":&coordinator.id, "status":"forwarded", "released":false,
    "usage_checkpoint.sequence":checkpoint.sequence,
    "usage_checkpoint.recovered":false,
    "pending_platform_usage":bson::Bson::Null, "pending_resale_quantity":bson::Bson::Null,
}, doc! {"$set":fields}).session(&mut *session).await?.matched_count != 1 {
    return Ok(Vec::new());
}
    let mut finalized = Vec::new();
    for row in &rows {
        if row.status != UsageStatus::Forwarded || row.released {
            continue;
        }
        let amount = match &checkpoint.model_charges {
            Some(vector) => vector.quantities.get(&row.lago_metric_code).copied(),
            None => quantity(checkpoint, row.metric),
        };
        let Some(amount) = amount else {
            continue;
        };
        let primary =
            row.transaction_id == meter::transaction_id(request, BillingLayer::Platform, None);
        let fields = meter::finalization_fields(
            amount,
            row.model.clone(),
            primary.then_some(&usage),
            None,
            None,
            checkpoint.observed_at,
        );
        let changed = collection.find_one_and_update(
        doc! {"_id":&row.id,"status":"forwarded","released":false,"quantity":bson::Bson::Null},
        vec![doc! {"$set":fields}],
    ).return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *session).await?.ok_or_else(unavailable)?;
        finalized.push(changed);
    }
    Ok(finalized)
}
