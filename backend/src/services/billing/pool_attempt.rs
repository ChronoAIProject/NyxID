//! Recovery for pool attempts whose provider quantity was never established.
//! Wallet release and the meter's release marker commit together. Benefit
//! releases use the existing durable, idempotent funding recovery protocol.
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS};
use crate::models::credits::Credits;
use crate::models::usage_meter::{COLLECTION_NAME as METERS, PoolAttemptOutcome, UsageMeterRow};

/// Finish a dropped/failed attempt before another member reserves funds. A
/// persisted settlement intent always takes precedence over unknown cleanup.
pub async fn finish(
    db: &mongodb::Database,
    request_id: &str,
    outcome: PoolAttemptOutcome,
) -> AppResult<u64> {
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(METERS)
        .find(doc! { "billing_request_id": request_id, "pool_attempt": { "$type": "object" } })
        .await?
        .try_collect()
        .await?;
    for row in &rows {
        if row.pending_platform_usage.is_some() {
            let materialized =
                Box::pin(super::meter::materialize_component_intent(db, row)).await?;
            Box::pin(super::meter::settle_persisted(db, materialized)).await?;
        }
        if row.pending_resale_quantity.is_some()
            && let Some(materialized) =
                Box::pin(super::meter::materialize_pending_resale_intent(db, row)).await?
        {
            Box::pin(super::meter::settle_persisted(db, vec![materialized])).await?;
        }
        if row.quantity.is_some() && !row.released {
            Box::pin(super::meter::settle_persisted(db, vec![row.clone()])).await?;
        }
    }
    let released = Box::pin(release_unknown_request(db, request_id, outcome, None)).await?;
    for row in rows {
        // Includes a release committed by a prior worker that died before
        // completing its benefit operations.
        if let Some(fresh) = db
            .collection::<UsageMeterRow>(METERS)
            .find_one(doc! { "_id": &row.id, "released": true, "quantity": bson::Bson::Null })
            .await?
        {
            super::funding::release_usage_reservations(db, &fresh).await?;
        }
    }
    Ok(released)
}

/// Record the transport result before settlement. Recovery preserves this field
/// while independently deciding reported versus unknown accounting.
pub async fn record_completion(
    db: &mongodb::Database,
    request_id: &str,
    cause: crate::models::usage_meter::PoolCompletionCause,
) -> AppResult<()> {
    db.collection::<UsageMeterRow>(METERS).update_many(
        doc! {"billing_request_id":request_id,"pool_attempt":{"$type":"object"}, "pool_attempt.completion_cause": bson::Bson::Null},
        doc! {"$set":{"pool_attempt.completion_cause":bson::to_bson(&cause).map_err(|e| AppError::Internal(e.to_string()))?}},
    ).await?;
    Ok(())
}

async fn release_unknown_request(
    db: &mongodb::Database,
    request_id: &str,
    outcome: PoolAttemptOutcome,
    expired_at: Option<DateTime<Utc>>,
) -> AppResult<u64> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let request_id = request_id.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<u64> = async {
            let now = bson::DateTime::from_chrono(Utc::now());
            let meters = db.collection::<UsageMeterRow>(METERS);
            let mut cursor = meters.find(doc! { "billing_request_id": &request_id,
                "pool_attempt": { "$type": "object" },
            }).session(&mut *session).await?;
            let rows: Vec<UsageMeterRow> = cursor.stream(&mut *session).try_collect().await?;
            // Revalidate the scan's cutoff in the same transaction that releases
            // holds. A concurrent renewal conflicts on these very meter rows.
            if rows.iter().any(|row| !row.released && expired_at.is_some_and(|cutoff| {
                row.pool_attempt.as_ref().is_none_or(|attempt| attempt.lease_until > cutoff)
            })) {
                return Ok(0);
            }
            if rows.iter().any(|row| row.pending_platform_usage.is_some() || row.pending_resale_quantity.is_some()) {
                return Ok(0);
            }
            let mut released = 0;
            for row in rows {
                if row.released || row.quantity.is_some() { continue; }
                if !matches!(row.status, crate::models::usage_meter::UsageStatus::Reserved | crate::models::usage_meter::UsageStatus::Forwarded) { continue; }
                if row.reserved_credits > Credits::ZERO {
                    let wallet = db.collection::<BillingWallet>(WALLETS).update_one(
                        doc! { "owner_id": &row.billing_owner_id,
                            "$expr": { "$gte": ["$reserved_credits", row.reserved_credits] } },
                        doc! { "$inc": { "reserved_credits": -row.reserved_credits },
                            "$set": { "updated_at": now } },
                    ).session(&mut *session).await?;
                    if wallet.modified_count != 1 {
                        return Err(AppError::Internal("Pool reservation wallet release failed".into()));
                    }
                }
                // Every unresolved row, including the platform coordinator,
                // transitions in this transaction. Intent persistence writes
                // that coordinator and therefore cannot write-skew against a
                // sibling's release.
                meters.update_one(doc! { "_id": &row.id }, doc! { "$set": {
                    "status": "abandoned", "released": true,
                    "pool_attempt.outcome": bson::to_bson(&outcome).map_err(|e| AppError::Internal(e.to_string()))?,
                    "last_error": match outcome {
                        PoolAttemptOutcome::Unsent => "pool_attempt_unsent",
                        PoolAttemptOutcome::Rejected => "pool_attempt_rejected",
                        _ => "pool_attempt_outcome_unknown",
                    },
                    "updated_at": now, "finalized_at": now,
                }}).session(&mut *session).await?;
                released += 1;
            }
            Ok(released)
        }.await;
        crate::services::api_key_mutation_service::transaction_result(operation)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

/// Only newly tagged pool attempts are eligible. Ordinary historical forwarded
/// rows retain the existing reconciliation contract.
pub async fn recover_expired(db: &mongodb::Database, now: DateTime<Utc>) -> AppResult<u64> {
    use crate::models::usage_meter::{POOL_RECOVERY_COLLECTION_NAME, PoolRecoveryCursor};
    let cursors = db.collection::<PoolRecoveryCursor>(POOL_RECOVERY_COLLECTION_NAME);
    let cursor = cursors
        .find_one(doc! { "name": "expired_attempts" })
        .await?;
    let mut filter = doc! {
        "released": false, "pool_attempt": { "$type": "object" },
        "pool_attempt.lease_until": { "$lte": bson::DateTime::from_chrono(now) },
    };
    if let Some(cursor) = &cursor
        && let (Some(lease), Some(row_id)) = (cursor.lease_until, cursor.row_id.as_ref())
    {
        filter.insert("$or", vec![
            doc! { "pool_attempt.lease_until": { "$gt": bson::DateTime::from_chrono(lease) } },
            doc! { "pool_attempt.lease_until": bson::DateTime::from_chrono(lease), "_id": { "$gt": row_id } },
        ]);
    }
    // Page over the recovery index before reading coordinator state. Known
    // intent/quantity rows advance this cursor too, so they cannot starve the
    // next page. No grouping, unbounded sort, or correlated aggregation.
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(METERS)
        .find(filter)
        .sort(doc! { "pool_attempt.lease_until": 1, "_id": 1 })
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let mut recovered = 0;
    let mut requests = std::collections::HashSet::new();
    for row in &rows {
        if row.quantity.is_none() && requests.insert(&row.billing_request_id) {
            match Box::pin(release_unknown_request(
                db,
                &row.billing_request_id,
                PoolAttemptOutcome::Unknown,
                Some(now),
            ))
            .await
            {
                Ok(count) => recovered += count,
                Err(error) => {
                    record_recovery_error(db, Some(&row.billing_request_id), &error).await
                }
            }
        }
    }
    let last = rows.last();
    cursors.update_one(doc! { "name": "expired_attempts" }, doc! {
        "$setOnInsert": { "_id": uuid::Uuid::new_v4().to_string(), "name": "expired_attempts" },
        "$set": {
            "lease_until": last.and_then(|row| row.pool_attempt.as_ref()).map(|attempt| bson::DateTime::from_chrono(attempt.lease_until)),
            "row_id": last.map(|row| row.id.as_str()),
            "updated_at": bson::DateTime::from_chrono(now),
        },
    }).upsert(true).await?;
    Ok(recovered)
}

pub async fn record_recovery_error(
    db: &mongodb::Database,
    request_id: Option<&str>,
    error: &AppError,
) {
    use crate::models::pool_recovery_diagnostic::{COLLECTION_NAME, PoolRecoveryFailure};
    tracing::error!(request_id, error = %error, "Pool recovery failed; reservation retained and sweep continues");
    let failure = PoolRecoveryFailure {
        request_id: request_id.map(str::to_owned),
        detail: "Release failed; the existing reservation was retained".into(),
    };
    let result: AppResult<()> = async {
        db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc! {"name":"pool_recovery"}, doc! {
            "$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(), "name":"pool_recovery"},
            "$set":{"updated_at":bson::DateTime::from_chrono(Utc::now())},
            "$inc":{"failures":1_i64},
            "$push":{"samples":{"$each":[bson::to_bson(&failure).map_err(|e| AppError::Internal(e.to_string()))?],"$slice":-5}},
        }).upsert(true).await?;
        Ok(())
    }.await;
    if let Err(error) = result {
        tracing::error!(error = %error, "Could not persist pool recovery Integrity diagnostic");
    }
}

pub async fn renew(
    db: &mongodb::Database,
    request_id: &str,
    until: DateTime<Utc>,
    expected_rows: u64,
) -> AppResult<bool> {
    if expected_rows == 0 {
        return Ok(true);
    }
    let result = db
        .collection::<UsageMeterRow>(METERS)
        .update_many(
            doc! { "billing_request_id": request_id, "released": false,
            "pool_attempt.lease_until": { "$gt": bson::DateTime::from_chrono(Utc::now()) } },
            doc! { "$max": { "pool_attempt.lease_until": bson::DateTime::from_chrono(until) } },
        )
        .await?;
    Ok(result.matched_count == expected_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::service_billing::{BillingMetric, ResaleSpec};
    use crate::models::usage_meter::{CredentialClass, PoolAttemptAccounting};
    use crate::services::billing::reservation::{BillingReservation, LayerReservation};
    use crate::services::billing::{BillingIngress, BillingRouteContext, NodeIntent};

    async fn fixture(
        label: &str,
        components: bool,
    ) -> (mongodb::Database, super::super::MeteredProxyContext) {
        let db = crate::test_utils::connect_test_database(label)
            .await
            .expect("MongoDB required");
        let mut ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            uuid::Uuid::new_v4().to_string(),
            "pool-owner".into(),
            "pool-actor".into(),
            None,
            Some("member".into()),
            Some("catalog".into()),
            Some("pool-test".into()),
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::Tokens,
            None,
            false,
        )
        .with_platform_metering(true);
        ctx.pool_attempt = Some(PoolAttemptAccounting {
            pool_id: "pool".into(),
            member_id: "member".into(),
            attempt: 1,
            lease_until: Utc::now() - chrono::Duration::minutes(1),
            outcome: None,
            completion_cause: None,
        });
        if components {
            ctx.platform_components.push(ResaleSpec {
                metric: BillingMetric::Images,
                lago_metric_code: "images".into(),
            });
        }
        let layers: Vec<_> = ctx
            .platform_specs()
            .map(|(metric, code)| LayerReservation {
                metric,
                lago_metric_code: code.into(),
                layer: crate::models::usage_meter::BillingLayer::Platform,
                estimated_quantity: 1,
                credits_per_unit_micros: 1_000_000,
                credits_per_unit_pico: None,
                reserved_credits: Credits::from_whole(1),
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
            })
            .collect();
        let total = Credits::from_whole(layers.len() as i64);
        db.collection::<bson::Document>(WALLETS)
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(), "owner_id": "pool-owner",
                "reserved_credits": total, "balance_credits": Credits::from_whole(10),
            })
            .await
            .unwrap();
        let reservation = BillingReservation {
            owner_id: "pool-owner".into(),
            wallet_id: "wallet".into(),
            total_reserved_credits: total,
            layers,
        };
        let metered = super::super::meter::open(&db, &ctx, Some(&reservation))
            .await
            .unwrap();
        super::super::meter::mark_forwarded(&db, &metered)
            .await
            .unwrap();
        (db, metered)
    }

    async fn held(db: &mongodb::Database) -> Credits {
        let wallet = db
            .collection::<bson::Document>(WALLETS)
            .find_one(doc! { "owner_id": "pool-owner" })
            .await
            .unwrap()
            .unwrap();
        Credits::from_bson(wallet.get("reserved_credits").unwrap().clone(), 1).unwrap()
    }

    #[tokio::test]
    async fn pool_recovery_bad_hold_isolated_and_later_reconciliation_continues() {
        let (db, _) = fixture("pool_recovery_bad_hold", false).await;
        crate::db::ensure_indexes(&db).await.unwrap();
        let rows = db.collection::<UsageMeterRow>(METERS);
        let good = rows.find_one(doc! {}).await.unwrap().unwrap();
        let mut bad = good.clone();
        bad.id = uuid::Uuid::new_v4().to_string();
        bad.billing_request_id = "bad-hold".into();
        bad.transaction_id = "bad-hold:platform".into();
        bad.reserved_credits = Credits::from_whole(100);
        bad.pool_attempt.as_mut().unwrap().lease_until -= chrono::Duration::seconds(1);
        rows.insert_one(&bad).await.unwrap();
        let mut stale = good.clone();
        stale.id = uuid::Uuid::new_v4().to_string();
        stale.billing_request_id = "ordinary-stale".into();
        stale.transaction_id = "ordinary-stale:platform".into();
        stale.pool_attempt = None;
        stale.reserved_credits = Credits::ZERO;
        stale.wallet_id = None;
        stale.status = crate::models::usage_meter::UsageStatus::Reserved;
        stale.forwarded = false;
        stale.updated_at = Utc::now() - chrono::Duration::days(1);
        rows.insert_one(&stale).await.unwrap();
        db.collection::<bson::Document>("billing_migrations")
            .update_one(
                doc! {"_id":super::super::exact_migration::BILLING_MARKER},
                doc! {"$set":{"completed_at":bson::DateTime::now()}},
            )
            .upsert(true)
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let reconciler = super::super::reconcile::BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(state.config),
        );
        let stats = reconciler.run_once().await.unwrap();
        assert_eq!(
            stats.abandoned, 2,
            "valid pool row and ordinary stale row must both recover"
        );
        assert!(
            !rows
                .find_one(doc! {"_id":&bad.id})
                .await
                .unwrap()
                .unwrap()
                .released
        );
        assert!(
            rows.find_one(doc! {"_id":&good.id})
                .await
                .unwrap()
                .unwrap()
                .released
        );
        assert!(
            rows.find_one(doc! {"_id":&stale.id})
                .await
                .unwrap()
                .unwrap()
                .released
        );
        assert_eq!(
            held(&db).await,
            Credits::ZERO,
            "only the valid one-credit hold was released"
        );
        let diagnostics = super::super::startup_diagnostics(&db).await.unwrap();
        let diagnostic = diagnostics
            .iter()
            .find(|d| d.code == "billing_pool_recovery")
            .unwrap();
        assert!(diagnostic.detail.contains("bad-hold"));
        assert!(!diagnostic.detail.contains("Mongo") && !diagnostic.detail.contains("100"));
        // All errors on a page still advance the cursor and the singleton stays bounded.
        for _ in 0..8 {
            record_recovery_error(
                &db,
                Some("bad-hold"),
                &AppError::Internal("sensitive DB diagnostic".into()),
            )
            .await;
        }
        let diagnostic = db
            .collection::<crate::models::pool_recovery_diagnostic::PoolRecoveryDiagnostic>(
                crate::models::pool_recovery_diagnostic::COLLECTION_NAME,
            )
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(diagnostic.samples.len(), 5);
        assert_eq!(
            db.collection::<bson::Document>(
                crate::models::pool_recovery_diagnostic::COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<bson::Document>("billing_ledger")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pool_attempt_restart_recovery_releases_unknown_once_and_leaves_historical_rows() {
        let (db, metered) = fixture("pool_attempt_restart", false).await;
        let mut old = db
            .collection::<UsageMeterRow>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        old.id = uuid::Uuid::new_v4().to_string();
        old.billing_request_id = "historical".into();
        old.transaction_id = "historical:platform".into();
        old.pool_attempt = None;
        old.reserved_credits = Credits::ZERO;
        db.collection::<UsageMeterRow>(METERS)
            .insert_one(old)
            .await
            .unwrap();
        assert_eq!(recover_expired(&db, Utc::now()).await.unwrap(), 1);
        assert_eq!(held(&db).await, Credits::ZERO);
        assert_eq!(recover_expired(&db, Utc::now()).await.unwrap(), 0);
        let row = db
            .collection::<UsageMeterRow>(METERS)
            .find_one(doc! { "billing_request_id": &metered.route.unwrap().billing_request_id })
            .await
            .unwrap()
            .unwrap();
        assert!(row.quantity.is_none());
        assert!(row.forwarded && row.released);
        assert_eq!(
            row.pool_attempt.unwrap().outcome,
            Some(PoolAttemptOutcome::Unknown)
        );
        assert_eq!(db.collection::<UsageMeterRow>(METERS).count_documents(doc! { "billing_request_id": "historical", "status": "forwarded", "released": false }).await.unwrap(), 1);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pool_attempt_recovery_pages_past_known_usage_without_releasing_it() {
        let (db, _) = fixture("pool_recovery_pages", false).await;
        let rows = db.collection::<UsageMeterRow>(METERS);
        let unknown = rows.find_one(doc! {}).await.unwrap().unwrap();
        let mut known = Vec::new();
        for index in 0..100 {
            let mut row = unknown.clone();
            row.id = uuid::Uuid::new_v4().to_string();
            row.billing_request_id = format!("known-{index}");
            row.transaction_id = format!("known-{index}:platform");
            row.quantity = Some(1);
            row.reserved_credits = Credits::ZERO;
            row.pool_attempt.as_mut().unwrap().lease_until -= chrono::Duration::seconds(1);
            known.push(row);
        }
        rows.insert_many(known).await.unwrap();
        assert_eq!(recover_expired(&db, Utc::now()).await.unwrap(), 0);
        assert_eq!(held(&db).await, Credits::from_whole(1));
        assert_eq!(recover_expired(&db, Utc::now()).await.unwrap(), 1);
        assert_eq!(held(&db).await, Credits::ZERO);
        assert_eq!(
            rows.count_documents(doc! {"quantity":1,"released":false})
                .await
                .unwrap(),
            100
        );
        let recovered = rows
            .find_one(doc! {"_id":unknown.id})
            .await
            .unwrap()
            .unwrap();
        assert!(recovered.released && recovered.quantity.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pool_attempt_stale_expiry_scan_cannot_release_a_renewed_lease() {
        let (db, metered) = fixture("pool_attempt_renew", false).await;
        let request = &metered.route.unwrap().billing_request_id;
        let scan_cutoff = Utc::now() + chrono::Duration::seconds(40);
        db.collection::<UsageMeterRow>(METERS).update_many(doc! {}, doc! { "$set": { "pool_attempt.lease_until": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(30)) } }).await.unwrap();
        renew(&db, request, scan_cutoff + chrono::Duration::minutes(5), 1)
            .await
            .unwrap();
        assert_eq!(
            Box::pin(release_unknown_request(
                &db,
                request,
                PoolAttemptOutcome::Unknown,
                Some(scan_cutoff)
            ))
            .await
            .unwrap(),
            0
        );
        assert_eq!(held(&db).await, Credits::from_whole(1));
        assert_eq!(
            Box::pin(release_unknown_request(
                &db,
                request,
                PoolAttemptOutcome::Unknown,
                Some(scan_cutoff + chrono::Duration::minutes(6))
            ))
            .await
            .unwrap(),
            1
        );
        assert_eq!(held(&db).await, Credits::ZERO);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pool_attempt_coordinator_intent_and_all_component_releases_serialize() {
        for iteration in 0..8 {
            let (db, metered) =
                fixture(&format!("pool_attempt_coordinator_{iteration}"), true).await;
            let request = &metered.route.unwrap().billing_request_id;
            let rows = db.collection::<UsageMeterRow>(METERS);
            let intent = doc! { "$set": { "pending_platform_usage": bson::to_bson(&crate::models::service_billing::PlatformUsage::single_request(1)).unwrap() } };
            let (released, accepted) = tokio::join!(
                Box::pin(release_unknown_request(
                    &db,
                    request,
                    PoolAttemptOutcome::Unknown,
                    Some(Utc::now())
                )),
                rows.update_one(
                    doc! { "transaction_id": format!("{request}:platform"), "status": "forwarded" },
                    intent
                )
                .into_future(),
            );
            let released = released.unwrap();
            let accepted = accepted.unwrap().matched_count;
            if accepted == 1 {
                assert_eq!(released, 0);
                assert_eq!(held(&db).await, Credits::from_whole(2));
                assert_eq!(
                    rows.count_documents(doc! { "released": true })
                        .await
                        .unwrap(),
                    0
                );
            } else {
                assert_eq!(released, 2);
                assert_eq!(held(&db).await, Credits::ZERO);
            }
            db.drop().await.unwrap();
        }
    }
}
