use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::config::AppConfig;
use crate::errors::AppResult;
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET};
use crate::models::usage_meter::{COLLECTION_NAME as USAGE_METER, UsageMeterRow};

use super::lago_client::{LagoApi, LagoError, LagoErrorKind, LagoEvent};
use super::reservation;
use super::webhook;

const PUSH_GRACE_SECS: i64 = 30;
const MAX_RECONCILE_PUSH_BATCH: i64 = 100;
const MAX_WALLET_REFRESH_BATCH: i64 = 100;
const ACKED_RETENTION_DAYS: i64 = 30;

#[derive(Clone)]
pub struct BillingReconciler {
    db: mongodb::Database,
    lago: Option<Arc<dyn LagoApi>>,
    config: Arc<AppConfig>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileStats {
    pub pushed: u64,
    pub duplicate_acked: u64,
    pub retried: u64,
    pub dead_lettered: u64,
    pub abandoned: u64,
    pub recovered_settlements: u64,
    pub drift_alerts: u64,
    pub wallet_balance_refreshes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LagoPushDecision {
    Ack,
    Retry,
    DeadLetter(String),
}

impl BillingReconciler {
    pub fn new(
        db: mongodb::Database,
        lago: Option<Arc<dyn LagoApi>>,
        config: Arc<AppConfig>,
    ) -> Self {
        Self { db, lago, config }
    }

    pub async fn run_once(&self) -> AppResult<ReconcileStats> {
        let mut stats = ReconcileStats::default();
        stats.abandoned += self.abandon_unforwarded_reserved().await?;
        if !self.config.billing_enabled {
            return Ok(stats);
        }
        let settlement = reservation::recover_retryable_settlements(&self.db).await?;
        stats.recovered_settlements += settlement.recovered;
        stats.retried += settlement.retried;
        stats.dead_lettered += settlement.dead_lettered;

        let Some(lago) = &self.lago else {
            return Ok(stats);
        };

        self.push_unacked(lago.as_ref(), &mut stats).await?;
        self.compare_finalized_usage(lago.as_ref(), &mut stats)
            .await?;
        self.refresh_wallet_balances(lago.as_ref(), &mut stats)
            .await?;
        Ok(stats)
    }

    /// Pull wallet balances from Lago into the local cache. Webhooks are the
    /// primary balance signal, but deployments Lago cannot reach (or missed
    /// deliveries) would otherwise never converge.
    async fn refresh_wallet_balances(
        &self,
        lago: &dyn LagoApi,
        stats: &mut ReconcileStats,
    ) -> AppResult<()> {
        let wallets: Vec<BillingWallet> = self
            .db
            .collection::<BillingWallet>(BILLING_WALLET)
            .find(doc! {})
            .sort(doc! { "balance_synced_at": 1 })
            .limit(MAX_WALLET_REFRESH_BATCH)
            .await?
            .try_collect()
            .await?;

        for wallet in wallets {
            match lago.wallet_balance(&wallet.lago_customer_id).await {
                Ok(balance_credits) => {
                    // OSS Lago never refreshes credits_ongoing_balance (the
                    // clock job is premium-gated), so the synced balance
                    // ignores usage accrued this period until the invoice
                    // settles. Subtract the period's current_usage ourselves;
                    // cents convert 1:1 to credits (wallet rate_amount is 1)
                    // and partial credits round up against availability.
                    let accrued_credits = match wallet.lago_subscription_id.as_deref() {
                        Some(subscription_id) => lago
                            .current_usage(&wallet.lago_customer_id, subscription_id)
                            .await
                            .ok()
                            .and_then(|usage| {
                                super::lago_client::extract_current_usage_amount_cents(&usage.raw)
                            })
                            .map(|cents| (cents.max(0) + 99) / 100)
                            .unwrap_or(0),
                        None => 0,
                    };
                    if webhook::refresh_wallet_balance(
                        &self.db,
                        &wallet.lago_customer_id,
                        balance_credits.saturating_sub(accrued_credits),
                    )
                    .await?
                    .is_some()
                    {
                        stats.wallet_balance_refreshes += 1;
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        owner_id = %wallet.owner_id,
                        error = %error,
                        "Billing wallet balance refresh from Lago failed"
                    );
                }
            }
        }
        Ok(())
    }

    async fn abandon_unforwarded_reserved(&self) -> AppResult<u64> {
        let cutoff =
            Utc::now() - Duration::seconds(self.config.billing_reservation_abandon_secs as i64);
        reservation::abandon_stale_unforwarded(&self.db, cutoff).await
    }

    async fn push_unacked(&self, lago: &dyn LagoApi, stats: &mut ReconcileStats) -> AppResult<()> {
        let cutoff = Utc::now() - Duration::seconds(PUSH_GRACE_SECS);
        let rows: Vec<UsageMeterRow> = self
            .db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find(doc! {
                "status": "finalized",
                "lago_acked": false,
                "wallet_id": { "$ne": null },
                "quantity": { "$ne": null },
                "updated_at": { "$lt": bson::DateTime::from_chrono(cutoff) },
            })
            .sort(doc! { "updated_at": 1 })
            .limit(MAX_RECONCILE_PUSH_BATCH)
            .await?
            .try_collect()
            .await?;

        // Events pushed without an external_subscription_id are stored by
        // Lago but never attached to a subscription's charges, so usage
        // silently aggregates to zero. Resolve each owner's subscription
        // from the local wallet, memoized for the batch.
        let mut subscriptions: BTreeMap<String, Option<String>> = BTreeMap::new();
        for row in rows {
            let subscription_id = match subscriptions.get(&row.billing_owner_id) {
                Some(cached) => cached.clone(),
                None => {
                    let resolved = self
                        .db
                        .collection::<BillingWallet>(BILLING_WALLET)
                        .find_one(doc! { "owner_id": &row.billing_owner_id })
                        .await?
                        .and_then(|wallet| wallet.lago_subscription_id);
                    subscriptions.insert(row.billing_owner_id.clone(), resolved.clone());
                    resolved
                }
            };
            let Some(event) = LagoEvent::from_usage_row(&row, subscription_id) else {
                mark_dead_letter(
                    &self.db,
                    &row.id,
                    "usage row has no finalized quantity for Lago event",
                )
                .await?;
                stats.dead_lettered += 1;
                continue;
            };

            let result = lago.record_event(&event).await;
            match decide_lago_push(result) {
                LagoPushDecision::Ack => {
                    mark_lago_acked(&self.db, &row.id).await?;
                    stats.pushed += 1;
                }
                LagoPushDecision::Retry => {
                    bump_attempt(&self.db, &row.id, "Lago event push will retry").await?;
                    stats.retried += 1;
                }
                LagoPushDecision::DeadLetter(reason) => {
                    mark_dead_letter(&self.db, &row.id, &reason).await?;
                    stats.dead_lettered += 1;
                }
            }
        }

        Ok(())
    }

    async fn compare_finalized_usage(
        &self,
        lago: &dyn LagoApi,
        stats: &mut ReconcileStats,
    ) -> AppResult<()> {
        let local = finalized_sums_by_owner_metric(&self.db).await?;
        for ((owner_id, metric_code), local_quantity) in local {
            let Ok(usage) = lago.current_usage(&owner_id, "").await else {
                continue;
            };
            let remote_quantity = extract_metric_quantity(&usage.raw, &metric_code);
            if let Some(remote) = remote_quantity
                && remote != local_quantity
            {
                stats.drift_alerts += 1;
                tracing::warn!(
                    owner_id = %owner_id,
                    metric_code = %metric_code,
                    local_quantity,
                    remote_quantity = remote,
                    "Billing Lago usage drift detected"
                );
            }
        }
        Ok(())
    }
}

pub fn spawn_reconcile_worker(reconciler: BillingReconciler, interval_secs: u64) {
    if interval_secs == 0 {
        return;
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = reconciler.run_once().await {
                tracing::warn!(error = %error, "Billing reconcile sweep failed");
            }
        }
    });
}

pub fn decide_lago_push(
    result: Result<super::lago_client::LagoAck, LagoError>,
) -> LagoPushDecision {
    match result {
        Ok(_) => LagoPushDecision::Ack,
        Err(error) => match error.kind {
            LagoErrorKind::Duplicate => LagoPushDecision::Ack,
            LagoErrorKind::DeadLetter => LagoPushDecision::DeadLetter(
                error
                    .code
                    .map(|code| format!("Lago rejected event permanently: {code}"))
                    .unwrap_or(error.message),
            ),
            LagoErrorKind::Retry | LagoErrorKind::Unavailable => LagoPushDecision::Retry,
        },
    }
}

async fn mark_lago_acked(db: &mongodb::Database, row_id: &str) -> AppResult<()> {
    let now = Utc::now();
    let expires_at = now + Duration::days(ACKED_RETENTION_DAYS);
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! { "_id": row_id, "lago_acked": false },
            doc! {
                "$set": {
                    "lago_acked": true,
                    "updated_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(expires_at),
                }
            },
        )
        .await?;
    Ok(())
}

async fn bump_attempt(db: &mongodb::Database, row_id: &str, error: &str) -> AppResult<()> {
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! { "_id": row_id },
            doc! {
                "$inc": { "attempt": 1 },
                "$set": {
                    "last_error": error,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;
    Ok(())
}

async fn mark_dead_letter(db: &mongodb::Database, row_id: &str, reason: &str) -> AppResult<()> {
    let now = Utc::now();
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! {
                "_id": row_id,
                "lago_acked": false,
                "status": { "$ne": "dead_letter" },
            },
            doc! {
                "$set": {
                    "status": "dead_letter",
                    "last_error": reason,
                    "updated_at": bson::DateTime::from_chrono(now),
                    "finalized_at": bson::DateTime::from_chrono(now),
                },
                "$inc": { "attempt": 1 },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(())
}

async fn finalized_sums_by_owner_metric(
    db: &mongodb::Database,
) -> AppResult<BTreeMap<(String, String), i64>> {
    let mut cursor = db
        .collection::<Document>(USAGE_METER)
        .aggregate(vec![
            doc! {
                "$match": {
                    "status": "finalized",
                    "quantity": { "$ne": null },
                }
            },
            doc! {
                "$group": {
                    "_id": {
                        "owner": "$billing_owner_id",
                        "metric": "$lago_metric_code",
                    },
                    "quantity": { "$sum": "$quantity" },
                }
            },
        ])
        .await?;

    let mut out = BTreeMap::new();
    while let Some(doc) = cursor.try_next().await? {
        let Some(id_doc) = doc.get_document("_id").ok() else {
            continue;
        };
        let Some(owner_id) = id_doc.get_str("owner").ok().map(ToString::to_string) else {
            continue;
        };
        let Some(metric_code) = id_doc.get_str("metric").ok().map(ToString::to_string) else {
            continue;
        };
        let quantity = doc_i64(&doc, "quantity").unwrap_or(0);
        out.insert((owner_id, metric_code), quantity);
    }

    Ok(out)
}

pub fn extract_metric_quantity(value: &serde_json::Value, metric_code: &str) -> Option<i64> {
    let mut seen = BTreeSet::new();
    extract_metric_quantity_inner(value, metric_code, &mut seen)
}

fn extract_metric_quantity_inner(
    value: &serde_json::Value,
    metric_code: &str,
    seen: &mut BTreeSet<usize>,
) -> Option<i64> {
    let ptr = value as *const serde_json::Value as usize;
    if !seen.insert(ptr) {
        return None;
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|item| extract_metric_quantity_inner(item, metric_code, seen)),
        serde_json::Value::Object(map) => {
            let matches_metric = ["code", "metric_code", "billable_metric_code"]
                .iter()
                .any(|key| map.get(*key).and_then(serde_json::Value::as_str) == Some(metric_code));
            if matches_metric {
                for key in ["units", "quantity", "amount", "usage"] {
                    if let Some(quantity) = json_i64(map.get(key)) {
                        return Some(quantity);
                    }
                }
            }
            map.values()
                .find_map(|item| extract_metric_quantity_inner(item, metric_code, seen))
        }
        _ => None,
    }
}

fn json_i64(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        serde_json::Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn doc_i64(doc: &Document, key: &str) -> Option<i64> {
    match doc.get(key)? {
        Bson::Int32(v) => Some(i64::from(*v)),
        Bson::Int64(v) => Some(*v),
        Bson::Double(v) => Some(v.round() as i64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
    use reqwest::StatusCode;
    use serde_json::json;
    use uuid::Uuid;

    use crate::models::service_billing::BillingMetric;
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageMeterRow, UsageStatus};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoErrorKind, LagoEvent, LagoUsage, OwnerProvisionInput,
    };
    use crate::test_utils::{connect_test_database, test_app_config};

    use super::{BillingReconciler, LagoPushDecision, decide_lago_push, extract_metric_quantity};

    /// Reconcile pushes usage to Lago only when billing is enabled (the
    /// `if !self.config.billing_enabled { return }` gate in `run_once`).
    /// Tests that exercise the Lago push/ack/retry/dead-letter path must run
    /// with billing enabled so the reconciler reaches `push_unacked`.
    fn billing_enabled_config() -> crate::config::AppConfig {
        crate::config::AppConfig {
            billing_enabled: true,
            ..test_app_config()
        }
    }

    #[derive(Clone)]
    struct StaticLago {
        result: Result<LagoAck, LagoError>,
    }

    #[async_trait]
    impl super::LagoApi for StaticLago {
        async fn ensure_customer(
            &self,
            _owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            Ok("owner".to_string())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            _plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            Ok(customer_id.to_string())
        }

        async fn record_event(&self, _event: &LagoEvent) -> Result<LagoAck, LagoError> {
            self.result.clone()
        }

        async fn record_events_batch(
            &self,
            events: &[LagoEvent],
        ) -> Result<Vec<LagoAck>, LagoError> {
            events.iter().map(|_| self.result.clone()).collect()
        }

        async fn current_usage(
            &self,
            customer_id: &str,
            subscription_id: &str,
        ) -> crate::errors::AppResult<LagoUsage> {
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: json!({}),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
            Ok(0)
        }

        async fn entitlements(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<Vec<Entitlement>> {
            Ok(Vec::new())
        }
    }

    fn finalized_row(transaction_id: &str) -> UsageMeterRow {
        let now = Utc::now() - Duration::seconds(120);
        UsageMeterRow {
            id: Uuid::new_v4().to_string(),
            transaction_id: transaction_id.to_string(),
            billing_request_id: format!("{transaction_id}-request"),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: "owner-1".to_string(),
            wallet_id: Some("wallet-1".to_string()),
            actor_user_id: "actor-1".to_string(),
            api_key_id: None,
            service_id: Some("service-1".to_string()),
            service_slug: Some("service-one".to_string()),
            metric: BillingMetric::Requests,
            lago_metric_code: "platform_requests".to_string(),
            credential_class: CredentialClass::UserOwned,
            model: None,
            reserved_credits: 0,
            quantity: Some(1),
            pending_resale_quantity: None,
            status: UsageStatus::Finalized,
            forwarded: true,
            released: true,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: Some(now),
            expires_at: None,
            last_error: None,
        }
    }

    #[test]
    fn duplicate_lago_push_is_acked_not_dead_lettered() {
        let decision = decide_lago_push(Err(LagoError::duplicate("already exists")));

        assert_eq!(decision, LagoPushDecision::Ack);
    }

    #[test]
    fn permanent_lago_rejection_dead_letters() {
        let decision = decide_lago_push(Err(LagoError {
            status: Some(StatusCode::UNPROCESSABLE_ENTITY),
            code: Some("billable_metric_not_found".to_string()),
            message: "missing metric".to_string(),
            kind: LagoErrorKind::DeadLetter,
        }));

        assert!(matches!(decision, LagoPushDecision::DeadLetter(_)));
    }

    #[test]
    fn transient_lago_failure_retries() {
        let decision = decide_lago_push(Err(LagoError::retry("timeout")));

        assert_eq!(decision, LagoPushDecision::Retry);
    }

    #[test]
    fn successful_lago_push_acks() {
        let decision = decide_lago_push(Ok(LagoAck {
            transaction_id: "tx".to_string(),
        }));

        assert_eq!(decision, LagoPushDecision::Ack);
    }

    #[test]
    fn current_usage_quantity_is_extracted_by_metric_code() {
        let usage = json!({
            "customer_usage": {
                "charges_usage": [
                    {
                        "billable_metric": { "code": "other" },
                        "units": "1"
                    },
                    {
                        "billable_metric_code": "platform_requests",
                        "units": "42"
                    }
                ]
            }
        });

        assert_eq!(
            extract_metric_quantity(&usage, "platform_requests"),
            Some(42)
        );
    }

    #[tokio::test]
    async fn reconcile_duplicate_transaction_marks_lago_acked() {
        let Some(db) = connect_test_database("billing_reconcile_duplicate_ack").await else {
            return;
        };
        let row = finalized_row("tx-duplicate");
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(StaticLago {
                result: Err(LagoError::duplicate("duplicate")),
            })),
            std::sync::Arc::new(billing_enabled_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");
        let saved = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(stats.pushed, 1);
        assert!(saved.lago_acked);
        assert_eq!(saved.status, UsageStatus::Finalized);
    }

    #[tokio::test]
    async fn reconcile_permanent_lago_rejection_dead_letters() {
        let Some(db) = connect_test_database("billing_reconcile_dead_letter").await else {
            return;
        };
        let row = finalized_row("tx-dead-letter");
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(StaticLago {
                result: Err(LagoError::dead_letter(
                    "billable_metric_not_found",
                    "missing metric",
                )),
            })),
            std::sync::Arc::new(billing_enabled_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");
        let saved = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(stats.dead_lettered, 1);
        assert!(!saved.lago_acked);
        assert_eq!(saved.status, UsageStatus::DeadLetter);
    }

    #[tokio::test]
    async fn reconcile_transient_lago_error_leaves_row_retryable() {
        let Some(db) = connect_test_database("billing_reconcile_retry").await else {
            return;
        };
        let row = finalized_row("tx-retry");
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(StaticLago {
                result: Err(LagoError::retry("timeout")),
            })),
            std::sync::Arc::new(billing_enabled_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");
        let saved = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(stats.retried, 1);
        assert_eq!(saved.status, UsageStatus::Finalized);
        assert!(!saved.lago_acked);
        assert_eq!(saved.attempt, 1);
    }

    #[tokio::test]
    async fn reconcile_recovers_a_settlement_stranded_past_its_deadline() {
        let Some(db) = connect_test_database("billing_reconcile_stranded_settlement").await else {
            return;
        };
        let mut row = finalized_row("tx-stranded-settlement");
        row.wallet_id = None;
        row.released = false;
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert stranded row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(billing_enabled_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");
        let saved = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(stats.recovered_settlements, 1);
        assert_eq!(saved.status, UsageStatus::Finalized);
        assert!(saved.released);
    }

    #[tokio::test]
    async fn reconcile_dead_letters_exhausted_settlement_and_excludes_replay() {
        let Some(db) = connect_test_database("billing_reconcile_settlement_dead_letter").await
        else {
            return;
        };
        let mut row = finalized_row("tx-settlement-dead-letter");
        row.status = UsageStatus::Failed;
        row.released = false;
        row.reserved_credits = 1;
        row.settlement_attempts =
            crate::services::billing::reservation::MAX_SETTLEMENT_ATTEMPTS - 1;
        row.settlement_next_retry_at = Some(Utc::now() - Duration::seconds(1));
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert failed row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(billing_enabled_config()),
        );

        let first = reconciler.run_once().await.expect("run first reconcile");
        let second = reconciler.run_once().await.expect("run second reconcile");
        let saved = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(first.dead_lettered, 1);
        assert_eq!(second.dead_lettered, 0);
        assert_eq!(second.retried, 0);
        assert_eq!(saved.status, UsageStatus::DeadLetter);
        assert_eq!(
            saved.settlement_attempts,
            crate::services::billing::reservation::MAX_SETTLEMENT_ATTEMPTS
        );
        assert!(saved.settlement_next_retry_at.is_none());
    }

    #[tokio::test]
    async fn reconcile_abandons_only_never_forwarded_reserved_rows() {
        let Some(db) = connect_test_database("billing_reconcile_abandon").await else {
            return;
        };
        let stale = Utc::now() - Duration::seconds(1_000);
        let mut never_forwarded = finalized_row("tx-never-forwarded");
        never_forwarded.status = UsageStatus::Reserved;
        never_forwarded.forwarded = false;
        never_forwarded.released = false;
        never_forwarded.quantity = None;
        never_forwarded.updated_at = stale;
        let never_forwarded_id = never_forwarded.id.clone();

        let mut forwarded = finalized_row("tx-forwarded");
        forwarded.status = UsageStatus::Forwarded;
        forwarded.forwarded = true;
        forwarded.released = false;
        forwarded.quantity = None;
        forwarded.updated_at = stale;
        let forwarded_id = forwarded.id.clone();

        let collection =
            db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        collection
            .insert_many([never_forwarded, forwarded])
            .await
            .expect("insert rows");
        let reconciler =
            BillingReconciler::new(db.clone(), None, std::sync::Arc::new(test_app_config()));

        let stats = reconciler.run_once().await.expect("run reconcile");
        let abandoned = collection
            .find_one(doc! { "_id": never_forwarded_id })
            .await
            .expect("find never forwarded")
            .expect("row exists");
        let still_forwarded = collection
            .find_one(doc! { "_id": forwarded_id })
            .await
            .expect("find forwarded")
            .expect("row exists");

        assert_eq!(stats.abandoned, 1);
        assert_eq!(abandoned.status, UsageStatus::Abandoned);
        assert_eq!(still_forwarded.status, UsageStatus::Forwarded);
    }

    #[tokio::test]
    async fn reconcile_does_not_push_lago_events_when_billing_disabled() {
        let Some(db) = connect_test_database("billing_reconcile_dark").await else {
            return;
        };
        let row = finalized_row("tx-dark");
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(StaticLago {
                result: Ok(LagoAck {
                    transaction_id: "tx-dark".to_string(),
                }),
            })),
            std::sync::Arc::new(test_app_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");
        let saved = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(stats.pushed, 0);
        assert!(!saved.lago_acked);
    }
}
