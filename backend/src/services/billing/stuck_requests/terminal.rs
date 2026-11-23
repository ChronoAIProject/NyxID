//! One atomic terminal decision; all accounting remains ordinary settlement.
use super::policy::{UnknownConsumptionPolicy, unknown_consumption_policy};
use super::*;
use crate::{
    models::usage_meter::{BillingLayer, UsageMeterRow},
    services::{audit_service, transaction_service},
};
use chrono::Utc;

pub(super) enum Outcome {
    Published(Vec<UsageMeterRow>),
    Deferred(&'static str),
}
#[cfg(test)]
pub(crate) async fn publish(
    db: &mongodb::Database,
    request: &str,
) -> AppResult<Vec<UsageMeterRow>> {
    match publish_outcome(db, request).await? {
        Outcome::Published(rows) => Ok(rows),
        Outcome::Deferred(_) => Ok(vec![]),
    }
}
pub(super) async fn publish_outcome(db: &mongodb::Database, request: &str) -> AppResult<Outcome> {
    publish_with_policy(db, request, unknown_consumption_policy()).await
}
pub(super) async fn publish_with_policy(
    db: &mongodb::Database,
    request: &str,
    policy: UnknownConsumptionPolicy,
) -> AppResult<Outcome> {
    let context = (db.clone(), request.to_owned(), policy);
    let rows = transaction_service::run_db_only(db, &context, |tx, (db, request, policy)| {
        Box::pin(in_session(db, tx, request, *policy))
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
) -> AppResult<Outcome> {
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
    let mut raw = read_documents(&mut cursor, tx).await?;
    if raw.is_empty() || raw.len() > 128 {
        return Ok(Outcome::Deferred("request_rows_unavailable"));
    }
    if let Some(reason) = request_blocker(&raw) {
        return Ok(Outcome::Deferred(reason));
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
        return Ok(Outcome::Deferred("no_remaining_hold"));
    }
    voice::read(db, tx, &mut raw).await?;
    // The inventory's sibling projection includes every actor. Give the same
    // predicate the same request-wide actors, including already-final siblings.
    let actors: Vec<_> = raw
        .iter()
        .map(|r| doc! {"actor_user_id":r.get("actor_user_id").cloned().unwrap_or(Bson::Null)})
        .collect();
    for row in &mut raw {
        if voice::is_voice(row) {
            row.insert(
                "request_rows",
                bson::to_bson(&actors).map_err(|_| unavailable())?,
            );
        }
    }
    let mut candidates = Vec::new();
    for value in &raw {
        if value.get_bool("released").ok() == Some(true) || present(value, "quantity") {
            continue;
        }
        let classification = classify(value, Some(&registry))?;
        if !classification.eligible {
            return Ok(Outcome::Deferred(classification.label));
        }
        let Ok(row) = bson::from_document::<UsageMeterRow>(value.clone()) else {
            return Ok(Outcome::Deferred("meter_row_malformed"));
        };
        candidates.push(row);
    }
    let voice_evidence = if let Some(value) = raw.iter().find(|r| voice::is_voice(r)) {
        match voice::decide(value, &registry) {
            voice::Decision::Ready(evidence) => Some(evidence),
            voice::Decision::Deferred(reason) => return Ok(Outcome::Deferred(reason)),
        }
    } else {
        None
    };
    let recorded = voice_evidence.as_ref().and_then(|e| e.usage.as_ref());
    let applied_policy = if recorded.is_some() {
        "recorded_voice_usage"
    } else {
        policy.name()
    };
    let mut rows = Vec::new();
    for row in candidates {
        let quantity = recorded
            .map(|usage| super::super::meter::platform_quantity(row.metric, usage))
            .or_else(|| policy.quantity(&row));
        let Some(quantity) = quantity else {
            return Ok(Outcome::Deferred("policy_quantity_unavailable"));
        };
        if quantity < 0 {
            return Err(unavailable());
        }
        rows.push((row, quantity));
    }
    #[cfg(test)]
    if voice_tests::round2::empty_decision() {
        rows.clear();
    }
    if rows.is_empty() {
        return Ok(Outcome::Deferred("no_unknown_rows"));
    }
    #[cfg(test)]
    test_point(Phase::BeforeTerminal).await;
    // The decision is complete and nonempty. There are no non-publication
    // returns below: every failure aborts the session/window and meter writes.
    if let Some(evidence) = &voice_evidence {
        evidence.fence(db, tx).await?;
    }
    // The primary coordinator is also fenced if it was already finalized.
    // Finalization of siblings must not race an intent on a different row.
    let coordinator = raw.iter().find(|r| {
        r.get_str("transaction_id").ok()
            == Some(
                super::super::meter::transaction_id(request, BillingLayer::Platform, None).as_str(),
            )
    });
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
            voice_evidence
                .as_ref()
                .and_then(|e| e.model.clone())
                .or(row.model.clone()),
            recorded,
            recorded.and_then(|u| u.reported_model.as_deref()),
            None,
            Utc::now(),
        );
        fields.insert("updated_at", "$$NOW");
        fields.insert("finalized_at", "$$NOW");
        fields.insert(
            "stuck_request_exit",
            doc! {"policy":applied_policy,"ended_at":"$$NOW"},
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
            "request_id":request,"policy":applied_policy,"rows":metadata,
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
    Ok(Outcome::Published(final_rows))
}
