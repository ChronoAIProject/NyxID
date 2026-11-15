//! One atomic terminal decision; all accounting remains ordinary settlement.
use super::policy::{UnknownConsumptionPolicy, unknown_consumption_policy};
use super::*;
use crate::{
    models::usage_meter::{BillingLayer, UsageMeterRow},
    services::{audit_service, transaction_service},
};
use chrono::Utc;

pub(crate) async fn publish(
    db: &mongodb::Database,
    request: &str,
) -> AppResult<Vec<UsageMeterRow>> {
    let context = (db.clone(), request.to_owned());
    let rows = transaction_service::run_db_only(db, &context, |tx, (db, request)| {
        Box::pin(in_session(db, tx, request, unknown_consumption_policy()))
    })
    .await?;
    #[cfg(test)]
    test_point(Phase::AfterCommit).await;
    Ok(rows)
}
async fn in_session(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    request: &str,
    policy: UnknownConsumptionPolicy,
) -> AppResult<Vec<UsageMeterRow>> {
    let registry = registry(db, tx).await?;
    let collection = db.collection::<Document>(METERS);
    let mut cursor = collection
        .find(doc! {"billing_request_id":request})
        .hint(mongodb::options::Hint::Name(
            "billing_request_id_1".to_owned(),
        ))
        .max_time(std::time::Duration::from_secs(2))
        .limit(129)
        .session(&mut *tx)
        .await?;
    let raw: Vec<Document> = cursor.stream(&mut *tx).try_collect().await?;
    if raw.is_empty() || raw.len() > 128 || request_blocker(&raw).is_some() {
        return Ok(vec![]);
    }
    // Keep the accepted request-wide terminal fence, including zero-hold
    // coordinators/siblings. Never start a terminal decision for a request
    // whose remaining unknown rows have no reservation at all.
    let mut held = false;
    for row in &raw {
        if row.get_bool("released").ok() != Some(true) && !present(row, "quantity") {
            held |= selection::has_reservation(row)?;
        }
    }
    if !held {
        return Ok(vec![]);
    }
    let mut rows = Vec::new();
    for value in &raw {
        if value.get_bool("released").ok() == Some(true) || present(value, "quantity") {
            continue;
        }
        // A partially materialized component request can contain a final row.
        // Every remaining unknown row must be eligible in this same snapshot.
        if !classify(value, Some(&registry))?.eligible {
            return Ok(vec![]);
        }
        let row: UsageMeterRow = match bson::from_document(value.clone()) {
            Ok(row) => row,
            Err(_) => return Ok(vec![]),
        };
        let Some(quantity) = policy.quantity(&row) else {
            return Ok(vec![]);
        };
        rows.push((row, quantity));
    }
    if rows.is_empty() {
        return Ok(vec![]);
    }
    // The primary coordinator is also fenced if it was already finalized.
    // Finalization of siblings must not race an intent on a different row.
    let coordinator = raw.iter().find(|r| {
        r.get_str("transaction_id").ok()
            == Some(
                super::super::meter::transaction_id(request, BillingLayer::Platform, None).as_str(),
            )
    });
    #[cfg(test)]
    test_point(Phase::BeforeTerminal).await;
    if let Some(coordinator) = coordinator
        && !rows
            .iter()
            .any(|(row, _)| coordinator.get_str("_id").ok() == Some(row.id.as_str()))
    {
        collection
            .update_one(
                doc! {"_id":coordinator.get_str("_id").map_err(|_|unavailable())?},
                doc! {"$set":{"stuck_request_fence":uuid::Uuid::new_v4().to_string()}},
            )
            .session(&mut *tx)
            .await?;
    }
    let mut final_rows = Vec::new();
    let mut metadata = Vec::new();
    for (row, quantity) in &rows {
        let mut fields = super::super::meter::finalization_fields(
            *quantity,
            row.model.clone(),
            None,
            None,
            None,
            Utc::now(),
        );
        fields.insert("updated_at", "$$NOW");
        fields.insert("finalized_at", "$$NOW");
        fields.insert(
            "stuck_request_exit",
            doc! {"policy":policy.name(),"ended_at":"$$NOW"},
        );
        let saved = db
            .collection::<UsageMeterRow>(METERS)
            .find_one_and_update(
                doc! {
                    "_id": &row.id, "status": "forwarded", "forwarded": true,
                    "released": false, "quantity": Bson::Null,
                    "pending_platform_usage": Bson::Null,
                    "pending_resale_quantity": Bson::Null,
                },
                vec![doc! {"$set": fields}],
            )
            .return_document(mongodb::options::ReturnDocument::After)
            .session(&mut *tx)
            .await?
            .ok_or_else(unavailable)?;
        let grant = row
            .funding
            .as_ref()
            .map(|f| &f.grant_reservations)
            .into_iter()
            .flatten()
            .try_fold(Credits::ZERO, |s, g| s.checked_add(g.amount))?;
        let allowance = row
            .funding
            .as_ref()
            .map(|f| &f.allowance_reservations)
            .into_iter()
            .flatten()
            .try_fold(0_i64, |s, a| s.checked_add(a.quantity))
            .ok_or_else(unavailable)?;
        metadata.push(serde_json::json!({
            "row_id": row.id, "owner": row.billing_owner_id, "wallet_id": row.wallet_id,
            "layer": row.layer, "metric": row.metric,
            "age_seconds": (registry.now.to_chrono() - row.created_at).num_seconds().max(0),
            "wallet_held_credits": row.reserved_credits.to_string(),
            "grant_held_credits": grant.to_string(), "allowance_held_units": allowance,
            "quantity": quantity,
        }));
        final_rows.push(saved);
    }
    #[cfg(test)]
    test_point(Phase::AfterRows).await;
    let key = audit_service::audit_chain_hmac_key().ok_or_else(unavailable)?;
    audit_service::log_system_event_in_session(
        db,
        tx,
        key,
        "billing_unknown_request_ended",
        serde_json::json!({
            "request_id":request,"policy":policy.name(),"rows":metadata,
        }),
    )
    .await?;
    #[cfg(test)]
    test_point(Phase::AfterAudit).await;
    db.collection::<Document>(STATE)
        .update_one(
            doc! {"_id": STATE_ID},
            vec![doc! {"$set": {
                "ended_requests": {"$add": [{"$ifNull": ["$ended_requests", 0_i64]}, 1_i64]},
                "ended_rows": {"$add": [{"$ifNull": ["$ended_rows", 0_i64]}, rows.len() as i64]},
                "last_ended_at": "$$NOW",
            }}],
        )
        .upsert(true)
        .session(&mut *tx)
        .await?;
    #[cfg(test)]
    test_point(Phase::BeforeCommit).await;
    final_rows.sort_by_key(|r| r.quantity.unwrap_or(0) > 0);
    Ok(final_rows)
}
