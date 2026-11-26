use super::*;
use crate::models::usage_meter::UsageStatus;
use crate::services::billing::recovery_batch::{self, Failures};
use futures::TryStreamExt;

pub(crate) async fn recover_stale(db: &mongodb::Database) -> AppResult<u64> {
    let rows: Vec<bson::Document> = db
        .collection::<bson::Document>(METERS)
        .find(doc! {
            "status":"forwarded", "released":false, "realtime_session.closed":false,
            "$expr":{"$lte":["$realtime_session.recover_after","$$NOW"]},
        })
        .sort(doc! {"realtime_session.recover_after":1,"_id":1})
        .limit(recovery_batch::limit(100))
        .await?
        .try_collect()
        .await?;
    let mut count = 0;
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
                let rows = publish(db, &row.billing_request_id, None).await?;
                let n = rows.len() as u64;
                meter::settle_persisted(db, rows).await?;
                Ok(n)
            })
            .await
        {
            count += n;
        }
    }
    failures.finish(count)
}

pub(super) async fn publish(
    db: &mongodb::Database,
    request: &str,
    close: Option<(&str, PlatformUsage)>,
) -> AppResult<Vec<UsageMeterRow>> {
    let context = (
        db.clone(),
        request.to_owned(),
        close.map(|(t, u)| (t.to_owned(), u)),
    );
    let rows = transaction_service::run_db_only(db, &context, |tx, (db, request, close)| {
        Box::pin(async move { publish_in_session(tx, db, request, close.as_ref()).await })
    })
    .await?;
    #[cfg(test)]
    crate::billing_integration_tests::usage_checkpoints::realtime_ws::point(
        crate::billing_integration_tests::usage_checkpoints::realtime_ws::Phase::AfterPublication,
    )
    .await;
    Ok(rows)
}

async fn publish_in_session(
    tx: &mut mongodb::ClientSession,
    db: &mongodb::Database,
    request: &str,
    close: Option<&(String, PlatformUsage)>,
) -> AppResult<Vec<UsageMeterRow>> {
    let collection = db.collection::<UsageMeterRow>(METERS);
    let mut cursor = collection
        .find(doc! {"billing_request_id":request})
        .limit((MAX_ROWS + 1) as i64)
        .session(&mut *tx)
        .await?;
    let rows: Vec<UsageMeterRow> = cursor.stream(&mut *tx).try_collect().await?;
    if rows.len() > MAX_ROWS {
        return Err(unavailable());
    }
    let Some(coordinator) = rows.iter().find(|row| row.realtime_session.is_some()) else {
        return Ok(Vec::new());
    };
    let checkpoint = coordinator
        .realtime_session
        .as_ref()
        .ok_or_else(unavailable)?;
    if checkpoint.version != VERSION
        || checkpoint.closed
        || rows.iter().any(|row| {
            row.status != UsageStatus::Forwarded
                || row.released
                || row.quantity.is_some()
                || row.pending_platform_usage.is_some()
                || row.pending_resale_quantity.is_some()
        })
    {
        return Ok(Vec::new());
    }
    if close.is_some_and(|(token, _)| *token != checkpoint.token) {
        return Ok(Vec::new());
    }
    let usage = close.map(|(_, usage)| usage.clone()).unwrap_or_else(|| {
        recovered_session_usage(&checkpoint.prefix, checkpoint.use_reported_usage)
    });
    #[cfg(test)]
    crate::billing_integration_tests::usage_checkpoints::realtime_ws::point(
        crate::billing_integration_tests::usage_checkpoints::realtime_ws::Phase::BeforePublication,
    )
    .await;
    let mut finalized = Vec::new();
    // The coordinator update seals the exact prefix and time predicate inside
    // the same snapshot as all quantities. Observation, close and recovery
    // conflict on this document; retries read the winner instead of adding it.
    for row in
        std::iter::once(coordinator).chain(rows.iter().filter(|row| row.id != coordinator.id))
    {
        let primary =
            row.transaction_id == meter::transaction_id(request, BillingLayer::Platform, None);
        let mut fields = meter::finalization_fields(
            quantity(row, &usage),
            row.model.clone(),
            primary.then_some(&usage),
            usage.reported_model.as_deref(),
            None,
            Utc::now(),
        );
        let mut filter =
            doc! {"_id":&row.id,"status":"forwarded","released":false,"quantity":bson::Bson::Null};
        if row.id == coordinator.id {
            filter.insert("realtime_session.token", &checkpoint.token);
            filter.insert("realtime_session.sequence", checkpoint.sequence);
            filter.insert("realtime_session.closed", false);
            if close.is_none() {
                filter.insert(
                    "$expr",
                    doc! {"$lte":["$realtime_session.recover_after","$$NOW"]},
                );
            }
            fields.insert("realtime_session.closed", true);
            fields.insert("realtime_session.recovered", close.is_none());
        }
        let changed = collection
            .find_one_and_update(filter, vec![doc! {"$set":fields}])
            .return_document(mongodb::options::ReturnDocument::After)
            .session(&mut *tx)
            .await?;
        let Some(changed) = changed else {
            if finalized.is_empty() {
                return Ok(Vec::new());
            }
            return Err(unavailable());
        };
        finalized.push(changed);
    }
    #[cfg(test)]
    crate::billing_integration_tests::usage_checkpoints::realtime_ws::point(
        crate::billing_integration_tests::usage_checkpoints::realtime_ws::Phase::BeforeCommit,
    )
    .await;
    // Match ordinary component settlement: return unused component holds
    // before funding nonzero components. This preserves benefit precedence.
    finalized.sort_by_key(|row| row.quantity.unwrap_or(0) > 0);
    Ok(finalized)
}
