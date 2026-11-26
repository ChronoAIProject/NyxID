#[cfg(test)]
use crate::models::credits::Credits;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::BillingRateCache;
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET};
use crate::models::usage_meter::{COLLECTION_NAME as USAGE_METER, UsageMeterRow};
use crate::services::coordination_service::{
    ClusterLeaseRuntime, LeaseStore, cluster_lease_runtime,
};

use super::lago_client::{LagoApi, LagoError, LagoErrorKind, LagoEvent};
use super::reconcile_status::Pass;
use super::reservation;
use super::webhook;

pub(crate) const RECONCILE_LEASE: &str = "billing:reconcile";

tokio::task_local! {
    // Scoped to the supervisor future, never inherited by request tasks. Leaf
    // callers outside a pass keep their existing behavior and row fences.
    static CONTINUE_RECONCILE: Cell<bool>;
}

/// Check only BETWEEN recoverable units, never inside a financial write
/// protocol. Once renewal fails, finish the current unit and admit no next one.
pub(crate) fn continue_reconcile() -> bool {
    CONTINUE_RECONCILE
        .try_with(|running| {
            crate::services::job_health::checkpoint(true);
            running.get()
        })
        .unwrap_or(true)
}

#[cfg(test)]
pub(crate) async fn with_drain_for_test<T>(future: impl std::future::Future<Output = T>) -> T {
    CONTINUE_RECONCILE.scope(Cell::new(true), future).await
}
#[cfg(test)]
pub(crate) fn drain_for_test() {
    CONTINUE_RECONCILE.with(|running| running.set(false));
}

#[cfg(test)]
tokio::task_local! {
    static LEASE_LOSS_OBSERVED: std::sync::Arc<tokio::sync::Notify>;
}

const PUSH_GRACE_SECS: i64 = 30;
const MAX_RECONCILE_PUSH_BATCH: i64 = 100;
const MAX_WALLET_REFRESH_BATCH: i64 = 100;
pub(crate) const ACKED_RETENTION_DAYS: i64 = 30;

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
    pub rate_cache_refreshes: u64,
    pub service_price_syncs: u64,
    pub funding_releases_recovered: u64,
    pub grants_expired: u64,
    pub schedule_grants_created: u64,
    pub schedule_grants_already_disbursed: u64,
    pub schedule_periods_completed: u64,
    pub schedule_periods_abandoned: u64,
    pub schedule_budget_exhausted: bool,
    pub grant_ledger_recoveries: u64,
    pub topups_expired: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LagoPushDecision {
    Ack,
    Retry,
    DeadLetter(String),
}

#[cfg(test)]
tokio::task_local! {
    static TEST_PASS_STATS: std::cell::RefCell<Option<ReconcileStats>>;
}

impl BillingReconciler {
    #[cfg(test)]
    pub(crate) async fn run_once_failing_for_test(&self, expected: &[&str]) -> ReconcileStats {
        TEST_PASS_STATS
            .scope(std::cell::RefCell::new(None), async {
                let error = self
                    .run_once()
                    .await
                    .expect_err("injected failure must fail the pass");
                let crate::errors::AppError::Internal(message) = error else {
                    panic!("unexpected pass error: {error}");
                };
                let mut actual: Vec<_> = message
                    .strip_prefix("Billing reconcile failed steps: ")
                    .expect("pass failure summary")
                    .split(", ")
                    .map(|entry| entry.split(':').next().unwrap())
                    .collect();
                let mut expected = expected.to_vec();
                actual.sort();
                expected.sort();
                assert_eq!(actual, expected, "{message}");
                TEST_PASS_STATS.with(|slot| slot.borrow_mut().take().expect("completed pass stats"))
            })
            .await
    }

    #[cfg(test)]
    pub(crate) async fn compare_usage_for_test(
        &self,
        token: &crate::services::coordination_service::LeaseToken,
    ) -> AppResult<ReconcileStats> {
        let mut stats = ReconcileStats::default();
        self.compare_finalized_usage(
            self.lago.as_ref().expect("test Lago client").as_ref(),
            &mut stats,
            token,
        )
        .await?;
        Ok(stats)
    }

    #[cfg(test)]
    pub(crate) async fn push_usage_for_test(&self) -> AppResult<ReconcileStats> {
        let mut stats = ReconcileStats::default();
        self.push_unacked(
            self.lago.as_ref().expect("test Lago client").as_ref(),
            &mut stats,
        )
        .await?;
        Ok(stats)
    }

    #[cfg(test)]
    pub(crate) async fn push_usage_outage_for_test(&self) -> ReconcileStats {
        let mut stats = ReconcileStats::default();
        let error = self
            .push_unacked(
                self.lago.as_ref().expect("test Lago client").as_ref(),
                &mut stats,
            )
            .await
            .expect_err("injected provider outage must fail the step");
        let AppError::BillingRecoveryFailed {
            failed_units,
            source,
        } = error
        else {
            panic!("unexpected step error: {error}");
        };
        assert!(matches!(*source, AppError::BillingProviderUnavailable(_)));
        assert_eq!(failed_units, stats.retried);
        stats
    }

    pub fn new(
        db: mongodb::Database,
        lago: Option<Arc<dyn LagoApi>>,
        config: Arc<AppConfig>,
    ) -> Self {
        Self { db, lago, config }
    }

    pub async fn run_once(&self) -> AppResult<ReconcileStats> {
        self.run_once_with_runtime(cluster_lease_runtime()).await
    }

    async fn run_once_with_runtime(
        &self,
        runtime: &ClusterLeaseRuntime,
    ) -> AppResult<ReconcileStats> {
        self.run_once_with_shutdown(runtime, &crate::services::shutdown::process())
            .await
    }

    async fn run_once_with_shutdown(
        &self,
        runtime: &ClusterLeaseRuntime,
        shutdown: &crate::services::shutdown::Runtime,
    ) -> AppResult<ReconcileStats> {
        let Some(_work) = shutdown.background() else {
            return Ok(ReconcileStats::default());
        };
        let Some((token, checkpoint)) = LeaseStore::acquire_with_checkpoint(
            &self.db,
            RECONCILE_LEASE,
            &runtime.holder,
            runtime.ttl,
        )
        .await?
        else {
            crate::services::job_health::skipped();
            runtime.notify_contender_observed();
            return Ok(ReconcileStats::default());
        };
        let mut report = Pass::new(checkpoint);
        let result = CONTINUE_RECONCILE
            .scope(Cell::new(true), async {
                let pass = self.run_steps(&mut report, &token);
                tokio::pin!(pass);
                // Passing a borrow keeps the pass alive when the house renewal
                // runner drops its operation on loss. Do not drop a multi-write
                // settlement/release between its durable steps.
                let outcome = {
                    let renewed = runtime.run_while_renewed(&self.db, &token, &mut pass);
                    tokio::pin!(renewed);
                    tokio::select! {
                        biased;
                        () = shutdown.intake_stopped() => {
                            CONTINUE_RECONCILE.with(|running| running.set(false));
                            // Keep renewing while the current unit finishes. If the
                            // hard deadline aborts us, retain the token until TTL.
                            renewed.await
                        },
                        result = &mut renewed => result,
                    }
                };
                match outcome {
                    Some(result) => result,
                    None => {
                        CONTINUE_RECONCILE.with(|running| running.set(false));
                        #[cfg(test)]
                        let _ = LEASE_LOSS_OBSERVED.try_with(|notify| notify.notify_one());
                        crate::services::job_health::checkpoint(false);
                        tracing::warn!("Billing reconcile lease lost; finishing current unit");
                        pass.await
                    }
                }
            })
            .await;
        // Publish once, fenced by the still-live token, in the release command.
        // A lost holder may finish its current money protocol but cannot publish
        // status over a successor's pass.
        if let Err(error) = report.release(&self.db, &token).await {
            report.record::<()>("status_publication", Err(error));
            if LeaseStore::release(&self.db, &token).await.is_err() {
                tracing::warn!("Billing reconcile lease release failed; waiting for expiry");
            }
        }
        #[cfg(test)]
        if let Ok(stats) = &result {
            let _ = TEST_PASS_STATS.try_with(|slot| *slot.borrow_mut() = Some(stats.clone()));
        }
        report.result(result)
    }

    #[cfg(test)]
    async fn run_pass(
        &self,
        token: &crate::services::coordination_service::LeaseToken,
    ) -> AppResult<ReconcileStats> {
        let mut report = Pass::new(None);
        let result = self.run_steps(&mut report, token).await;
        report.result(result)
    }

    async fn run_steps(
        &self,
        report: &mut Pass,
        token: &crate::services::coordination_service::LeaseToken,
    ) -> AppResult<ReconcileStats> {
        if !report.observe(
            "exact_readiness",
            super::exact_migration::ready(&self.db).await,
        )? {
            return Ok(ReconcileStats::default());
        }
        let result = self.run_billing_steps(report, token).await;
        if continue_reconcile() {
            report.record(
                "metering_retention",
                super::usage_rollup::retention::backfill(&self.db, Utc::now()).await,
            );
        }
        result
    }

    async fn run_billing_steps(
        &self,
        report: &mut Pass,
        token: &crate::services::coordination_service::LeaseToken,
    ) -> AppResult<ReconcileStats> {
        let mut stats = ReconcileStats::default();
        // Benefit-only estimates can spill into wallet funding at settlement.
        // Refresh before recovery: pending component/voice recovery may return
        // early on a missing profile and must not starve its own refresh.
        if !continue_reconcile() {
            return Ok(stats);
        }
        if self.config.billing_enabled
            && let Some(lago) = &self.lago
        {
            report.record(
                "taxes_before_recovery",
                super::taxes::reconcile(
                    &self.db,
                    lago.as_ref(),
                    self.config.billing_rate_cache_ttl_secs,
                )
                .await,
            );
        }
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "model_recovery",
            crate::services::platform_model_execution::recover(&self.db).await,
        );
        if !continue_reconcile() {
            return Ok(stats);
        }
        let pool = super::pool_attempt::recover_expired(&self.db, Utc::now()).await;
        if let Err(error) = &pool {
            super::pool_attempt::record_recovery_error(&self.db, None, error).await;
        }
        stats.abandoned += report.record("pool_recovery", pool).unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.recovered_settlements += report
            .record("voice_recovery", super::voice::reconcile(&self.db).await)
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.recovered_settlements += report
            .record(
                "managed_recovery",
                crate::services::managed_execution_service::reconcile(&self.db).await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.abandoned += report
            .record(
                "abandon_unforwarded",
                self.abandon_unforwarded_reserved().await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.funding_releases_recovered += report
            .record(
                "funding_releases",
                super::funding::recover_terminal_releases(&self.db).await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.grants_expired += report
            .record(
                "grant_expiry",
                super::grants::expire_due_grants(&self.db, Utc::now()).await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        if let Some(schedule_stats) = report.record(
            "schedules",
            super::schedules::disburse_due(
                &self.db,
                Utc::now(),
                super::schedules::MAX_RECIPIENTS_PER_TICK,
            )
            .await,
        ) {
            stats.schedule_grants_created += schedule_stats.grants_created as u64;
            stats.schedule_grants_already_disbursed += schedule_stats.already_disbursed as u64;
            stats.schedule_periods_completed += schedule_stats.periods_completed as u64;
            stats.schedule_periods_abandoned += schedule_stats.periods_abandoned as u64;
            stats.schedule_budget_exhausted = schedule_stats.budget_exhausted;
        }
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.grant_ledger_recoveries += report
            .record(
                "grant_ledger",
                super::grants::recover_unledgered_events(&self.db, Utc::now()).await,
            )
            .unwrap_or_default();
        // Anchor the billing-ledger head into the audit chain whenever it
        // advanced, so tail truncation of the ledger is detectable. Best
        // effort: an anchor failure must not stall the sweep.
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record("ledger_anchor", super::ledger::anchor_head(&self.db).await);
        if !self.config.billing_enabled {
            return Ok(stats);
        }
        if !continue_reconcile() {
            return Ok(stats);
        }
        if let Some(settlement) = report.record(
            "settlement_recovery",
            reservation::recover_retryable_settlements(&self.db).await,
        ) {
            stats.recovered_settlements += settlement.recovered;
            stats.retried += settlement.retried;
            stats.dead_lettered += settlement.dead_lettered;
        }

        let Some(lago) = &self.lago else {
            return Ok(stats);
        };

        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.topups_expired += report
            .record(
                "topup_expiry",
                super::topup_expiry::expire_purchased_credits(&self.db, lago.as_ref(), Utc::now())
                    .await,
            )
            .unwrap_or_default();

        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.service_price_syncs += report
            .record(
                "service_prices",
                super::pricing::retry_pending_service_prices(
                    &self.db,
                    lago.as_ref(),
                    &self.config.lago_plan_code,
                )
                .await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.service_price_syncs += report
            .record(
                "model_prices",
                super::model_tariffs::retry_pending(
                    &self.db,
                    lago.as_ref(),
                    &self.config.lago_plan_code,
                )
                .await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        let mut runtime_billing = super::BillingService::new(self.db.clone(), self.config.clone());
        runtime_billing.lago = self.lago.clone();
        // CMA configuration work must not prevent ordinary usage emission or
        // tax/balance refresh. Pending/failed cards remain durable retry work;
        // an ambiguous timeout retains the sync lease until its expiry.
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.service_price_syncs += report
            .record(
                "sandbox_prices",
                runtime_billing.retry_sandbox_rate_cards().await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        stats.service_price_syncs += report
            .record(
                "machine_prices",
                runtime_billing.retry_machine_runtime_rates(4).await,
            )
            .unwrap_or_default();
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "plan_repair",
            super::plan_sync::repair_other_plans(
                &self.db,
                lago.as_ref(),
                &self.config.lago_plan_code,
            )
            .await,
        );
        // Price synchronization invalidates affected profiles. Refresh those
        // within this pass; admissions also refresh individually on demand.
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "taxes_after_prices",
            super::taxes::reconcile(
                &self.db,
                lago.as_ref(),
                self.config.billing_rate_cache_ttl_secs,
            )
            .await,
        );
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "usage_push",
            self.push_unacked(lago.as_ref(), &mut stats).await,
        );
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "usage_comparison",
            self.compare_finalized_usage(lago.as_ref(), &mut stats, token)
                .await,
        );
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "wallet_balances",
            self.refresh_wallet_balances(lago.as_ref(), &mut stats)
                .await,
        );
        if !continue_reconcile() {
            return Ok(stats);
        }
        report.record(
            "rate_cache",
            self.refresh_rate_cache(lago.as_ref(), &mut stats).await,
        );
        Ok(stats)
    }

    /// Mirror the Lago plan's standard charges into `billing_rate_cache` so
    /// reservation sizing always has fresh rates. Lago stays the pricing
    /// authority; this cache only sizes holds. Metrics NyxID meters that the
    /// plan does not price refresh to zero (unpriced in Lago means free), so
    /// the gate stays open for them without a manual seed. Fetch failures
    /// and empty plans leave existing rows untouched: the TTL check remains
    /// the backstop against charging on stale prices.
    async fn refresh_rate_cache(
        &self,
        lago: &dyn LagoApi,
        stats: &mut ReconcileStats,
    ) -> AppResult<()> {
        let mut failures = super::recovery_batch::Failures::default();
        let mut captured = super::rate_cache::capture(&self.db).await?;
        let plan = match lago.plan_rates(&self.config.lago_plan_code).await {
            Ok(plan) => plan,
            Err(error) => {
                tracing::warn!(
                    plan_code = %self.config.lago_plan_code,
                    error = %error,
                    "Billing rate cache refresh failed to fetch plan charges"
                );
                failures.note(error);
                return failures.finish(());
            }
        };
        if !continue_reconcile() {
            return Ok(());
        }
        let rejected = plan.rejected;
        let empty = plan.rates.is_empty();
        let mut by_code: BTreeMap<String, (i64, Option<i64>)> = plan
            .rates
            .into_iter()
            .map(|rate| {
                (
                    rate.lago_metric_code,
                    (rate.credits_per_unit_micros, rate.credits_per_unit_pico),
                )
            })
            .collect();
        if !empty {
            for metric_code in [
                super::meter::PLATFORM_REQUESTS_METRIC_CODE,
                super::meter::PLATFORM_BYTES_METRIC_CODE,
                super::meter::PLATFORM_TOKENS_METRIC_CODE,
            ] {
                if !rejected.iter().any(|code| code == metric_code) {
                    by_code
                        .entry(metric_code.to_owned())
                        .or_insert((0, Some(0)));
                }
            }
        }
        let now = Utc::now();
        let rows: Vec<_> = by_code
            .into_iter()
            .map(|(metric_code, (micros, pico))| BillingRateCache {
                id: BillingRateCache::cache_id(&metric_code, None),
                lago_metric_code: metric_code,
                model: None,
                credits_per_unit_micros: micros,
                credits_per_unit_pico: pico,
                synced_at: now,
                retired_at: None,
            })
            .collect();
        let size = super::rate_cache::PUBLICATION_BATCH_SIZE;
        let chunks = rows.len().max(rejected.len()).div_ceil(size).max(1);
        for index in 0..chunks {
            if !continue_reconcile() {
                break;
            }
            let start = index * size;
            let chunk = super::rate_cache::Chunk {
                captured,
                plan: &self.config.lago_plan_code,
                diagnostics: (index == 0).then_some(rejected.as_slice()),
                rejected: &rejected[start.min(rejected.len())..(start + size).min(rejected.len())],
                rows: &rows[start.min(rows.len())..(start + size).min(rows.len())],
            };
            let Some((next, written)) =
                super::rate_cache::publish_isolated(&self.db, chunk, &mut failures).await?
            else {
                // Never reuse this provider response with a newly read epoch.
                return failures.finish(());
            };
            captured = next;
            stats.rate_cache_refreshes += written;
            #[cfg(test)]
            rate_ordering_tests::after_publication(stats.rate_cache_refreshes).await;
        }
        if empty {
            tracing::warn!(
                plan_code = %self.config.lago_plan_code,
                "Billing rate cache refresh found no standard charges; keeping existing rates"
            );
        }
        failures.finish(())
    }

    /// Pull wallet balances from Lago into the local cache. Webhooks are the
    /// primary balance signal, but deployments Lago cannot reach (or missed
    /// deliveries) would otherwise never converge.
    pub(super) async fn refresh_wallet_balances(
        &self,
        lago: &dyn LagoApi,
        stats: &mut ReconcileStats,
    ) -> AppResult<()> {
        let wallets: Vec<Document> = self
            .db
            .collection::<Document>(BILLING_WALLET)
            .find(doc! {})
            .sort(doc! { "balance_synced_at": 1 })
            .limit(MAX_WALLET_REFRESH_BATCH)
            .await?
            .try_collect()
            .await?;

        let mut failures = super::recovery_batch::Failures::default();
        for raw in wallets {
            if !continue_reconcile() {
                break;
            }
            failures
                .unit(async {
                    let wallet: BillingWallet = super::recovery_batch::decode(&raw)?;
                    match webhook::provider_effective_balance(lago, &wallet).await {
                        Ok(balance_credits) => {
                            // No write has begun; a lost pass can discard this read.
                            if !continue_reconcile() {
                                return Ok(());
                            }
                            if webhook::refresh_wallet_balance(&self.db, &wallet, balance_credits)
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
                            return Err(error);
                        }
                    }
                    Ok(())
                })
                .await;
        }
        failures.finish(())
    }

    async fn abandon_unforwarded_reserved(&self) -> AppResult<u64> {
        let cutoff =
            Utc::now() - Duration::seconds(self.config.billing_reservation_abandon_secs as i64);
        reservation::abandon_stale_unforwarded(&self.db, cutoff).await
    }

    async fn push_unacked(&self, lago: &dyn LagoApi, stats: &mut ReconcileStats) -> AppResult<()> {
        // Receipt repair is bounded bookkeeping. A stale or malformed receipt
        // must never prevent the meter rows below from reaching the provider.
        if let Err(error) = super::usage_delivery::reconcile_prepared(&self.db).await {
            tracing::warn!(error = %error, "usage delivery receipt repair deferred");
        }
        let cutoff = Utc::now() - Duration::seconds(PUSH_GRACE_SECS);
        let rows: Vec<Document> = self
            .db
            .collection::<Document>(USAGE_METER)
            .find(doc! {
                "status": "finalized",
                "released": true,
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
        let mut subscriptions: BTreeMap<String, Option<super::usage_delivery::UsageTarget>> =
            BTreeMap::new();
        let mut failures = super::recovery_batch::Failures::default();
        for raw in rows {
            if !continue_reconcile() {
                break;
            }
            failures
                .unit(async {
                    let row: UsageMeterRow = super::recovery_batch::decode(&raw)?;
                    if row.funding.as_ref().is_some_and(|funding| {
                        funding.settled
                            && funding.lago_billable_quantity_micros.unwrap_or_default() <= 0
                    }) {
                        mark_lago_acked(&self.db, &row.id).await?;
                        return Ok(());
                    }
                    // Runtime contracts keep their accepted rate even after retirement.
                    // Emit to the subscription verified on that plan, without a second
                    // mutable wallet lookup or the ordinary service cache.
                    let binding = if super::sandbox_runtime::METRICS.contains(&row.metric) {
                        super::sandbox_runtime::verify_usage_provider_rate(&self.db, lago, &row)
                            .await
                    } else {
                        super::machine_runtime::verify_usage_provider_rate(&self.db, lago, &row)
                            .await
                    };
                    let verified_subscription = match binding {
                        Ok(binding) => binding,
                        Err(error) => {
                            bump_attempt(
                                &self.db,
                                &row.id,
                                "Runtime billing provider binding verification will retry",
                            )
                            .await?;
                            stats.retried += 1;
                            return Err(error);
                        }
                    };
                    let runtime_binding = verified_subscription.is_some();
                    let target = match verified_subscription {
                        Some(verified) => Some(verified),
                        None => match subscriptions.get(&row.billing_owner_id) {
                            Some(cached) => cached.clone(),
                            None => {
                                let resolved = self
                                    .db
                                    .collection::<BillingWallet>(BILLING_WALLET)
                                    .find_one(doc! { "owner_id": &row.billing_owner_id })
                                    .await?
                                    .and_then(|wallet| {
                                        wallet.lago_subscription_id.map(|subscription_id| {
                                            super::usage_delivery::UsageTarget {
                                                customer_id: wallet.lago_customer_id,
                                                subscription_id,
                                            }
                                        })
                                    });
                                subscriptions
                                    .insert(row.billing_owner_id.clone(), resolved.clone());
                                resolved
                            }
                        },
                    };
                    let Some(event) = LagoEvent::from_usage_row(
                        &row,
                        target.as_ref().map(|target| target.subscription_id.clone()),
                    ) else {
                        mark_dead_letter(
                            &self.db,
                            &row.id,
                            "usage row has no finalized quantity for Lago event",
                        )
                        .await?;
                        stats.dead_lettered += 1;
                        return Ok(());
                    };

                    let receipt = super::usage_delivery::prepare(
                        &self.db,
                        &row,
                        target
                            .as_ref()
                            .map_or(row.billing_owner_id.as_str(), |target| {
                                target.customer_id.as_str()
                            }),
                        event,
                    )
                    .await?;
                    if runtime_binding
                        && target.as_ref().is_some_and(|target| {
                            receipt.customer_id != target.customer_id
                                || receipt.subscription_id.as_deref()
                                    != Some(target.subscription_id.as_str())
                        })
                    {
                        bump_attempt(
                            &self.db,
                            &row.id,
                            "Recorded runtime delivery target no longer matches verified binding",
                        )
                        .await?;
                        stats.retried += 1;
                        return Err(AppError::BillingProviderUnavailable(
                            "Recorded runtime delivery target could not be verified".into(),
                        ));
                    }
                    let event = super::usage_delivery::event(&receipt);
                    let result = lago.record_event(&event).await;
                    let duplicate = result
                        .as_ref()
                        .err()
                        .is_some_and(|error| error.kind == LagoErrorKind::Duplicate);
                    // A typed rate-limit response is waiting, not an outage. Transport
                    // failures and server errors retain the same retry write but fail
                    // this maintenance unit after that write is durable.
                    let retry_error = result.as_ref().err().and_then(|error| {
                        matches!(
                            error.kind,
                            LagoErrorKind::Retry | LagoErrorKind::Unavailable
                        )
                        .then(|| {
                            let source = AppError::BillingProviderUnavailable(
                                "Lago event push will retry".into(),
                            );
                            if error.status == Some(reqwest::StatusCode::TOO_MANY_REQUESTS) {
                                super::recovery_batch::waiting(source)
                            } else {
                                source
                            }
                        })
                    });
                    match decide_lago_push(result) {
                        LagoPushDecision::Ack => {
                            mark_delivered(&self.db, &row.id, Some((receipt, duplicate))).await?;
                            stats.pushed += 1;
                        }
                        LagoPushDecision::Retry => {
                            bump_attempt(&self.db, &row.id, "Lago event push will retry").await?;
                            stats.retried += 1;
                            if let Some(error) = retry_error {
                                return Err(error);
                            }
                        }
                        LagoPushDecision::DeadLetter(reason) => {
                            super::usage_delivery::reject(&self.db, &receipt).await?;
                            mark_dead_letter(&self.db, &row.id, &reason).await?;
                            stats.dead_lettered += 1;
                        }
                    }
                    Ok(())
                })
                .await;
        }

        failures.finish(())
    }

    async fn compare_finalized_usage(
        &self,
        lago: &dyn LagoApi,
        stats: &mut ReconcileStats,
        token: &crate::services::coordination_service::LeaseToken,
    ) -> AppResult<()> {
        super::usage_comparison::run(&self.db, lago, token, stats).await
    }
}

pub(crate) fn spawn_reconcile_worker(
    reconciler: BillingReconciler,
    interval_secs: u64,
    job: crate::services::job_health::Job,
) -> Option<tokio::task::JoinHandle<()>> {
    if interval_secs == 0 {
        return None;
    }

    Some(
        crate::services::shutdown::process().spawn_worker(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first tick fires immediately so a fresh deploy has rate-cache
            // rows and wallet balances before the first billable request.
            loop {
                let Some(_work) = job
                    .tick(&crate::services::shutdown::process(), &mut interval)
                    .await
                else {
                    break;
                };
                if let Err(error) = job.observe(reconciler.run_once()).await {
                    tracing::warn!(error = %error, "Billing reconcile sweep failed");
                }
            }
        }),
    )
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

pub(super) async fn mark_lago_acked(db: &mongodb::Database, row_id: &str) -> AppResult<()> {
    mark_delivered(db, row_id, None).await
}

async fn mark_delivered(
    db: &mongodb::Database,
    row_id: &str,
    receipt: Option<(
        crate::models::billing_usage_delivery::BillingUsageDelivery,
        bool,
    )>,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let row_id = row_id.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = ack_in_session(&database, session, &row_id, receipt.as_ref()).await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

async fn ack_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    row_id: &str,
    receipt: Option<&(
        crate::models::billing_usage_delivery::BillingUsageDelivery,
        bool,
    )>,
) -> AppResult<()> {
    let now = Utc::now();
    let expires_at = now + Duration::days(ACKED_RETENTION_DAYS);
    let row = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! { "_id": row_id, "lago_acked": false },
            doc! { "$set": {
                "lago_acked": true,
                "updated_at": bson::DateTime::from_chrono(now),
                "expires_at": bson::DateTime::from_chrono(expires_at),
            } },
        )
        .session(&mut *session)
        .await?;
    if let Some(row) = row {
        if let Some((receipt, _)) = receipt
            && (receipt.transaction_id != row.transaction_id
                || receipt.owner_id != row.billing_owner_id
                || Some(&receipt.wallet_id) != row.wallet_id.as_ref())
        {
            super::usage_delivery::record_acknowledgement_issue(
                db,
                session,
                receipt,
                "delivery_receipt_mismatch",
            )
            .await?;
        }
        if let Some(wallet_id) = &row.wallet_id {
            // The acknowledgement can make pending debits clearable. A
            // provider read started before it cannot decide that clearing.
            db.collection::<BillingWallet>(BILLING_WALLET)
                .update_one(
                    doc! { "_id": wallet_id },
                    doc! { "$inc": { "snapshot_generation": 1_i64 } },
                )
                .session(&mut *session)
                .await?;
        }
        if let Some((receipt, duplicate)) = receipt
            && receipt.transaction_id == row.transaction_id
            && receipt.owner_id == row.billing_owner_id
            && Some(&receipt.wallet_id) == row.wallet_id.as_ref()
        {
            super::usage_delivery::acknowledge(db, session, receipt, *duplicate).await?;
        }
    }
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

#[cfg(test)]
#[path = "rate_ordering_tests.rs"]
pub(super) mod rate_ordering_tests;

#[cfg(test)]
#[path = "reconcile_cma_tests.rs"]
mod cma_tests;

#[cfg(test)]
#[path = "usage_capture_tests.rs"]
mod usage_capture_tests;

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
    use reqwest::StatusCode;
    use serde_json::json;
    use uuid::Uuid;

    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::models::service_billing::BillingMetric;
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageMeterRow, UsageStatus};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoErrorKind, LagoEvent, LagoUsage, OwnerProvisionInput,
    };
    use crate::test_utils::{connect_test_database, test_app_config};

    use super::{BillingReconciler, LagoPushDecision, decide_lago_push};

    /// Reconcile pushes usage to Lago only when billing is enabled (the
    /// `if !self.config.billing_enabled { return }` gate in `run_once`).
    /// Tests that exercise the Lago push/ack/retry/dead-letter path must run
    /// with billing enabled so the reconciler reaches `push_unacked`.
    pub(super) fn billing_enabled_config() -> crate::config::AppConfig {
        crate::config::AppConfig {
            billing_enabled: true,
            billing_allocations_enabled: false,
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
            crate::services::billing::lago_client::require_subscription_id(subscription_id)?;
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

    pub(super) fn finalized_row(transaction_id: &str) -> UsageMeterRow {
        let now = Utc::now() - Duration::seconds(120);
        UsageMeterRow {
            model_tariff: None,
            org_channel_guest: None,
            stuck_request_exit: None,
            rollup_pending: true,
            id: Uuid::new_v4().to_string(),
            transaction_id: transaction_id.to_string(),
            billing_request_id: format!("{transaction_id}-request"),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: "owner-1".to_string(),
            wallet_id: Some("wallet-1".to_string()),
            actor_user_id: "actor-1".to_string(),
            api_key_id: None,
            assistant_source: None,
            user_service_id: None,
            service_id: Some("service-1".to_string()),
            service_slug: Some("service-one".to_string()),
            metric: BillingMetric::Requests,
            lago_metric_code: "platform_requests".to_string(),
            operation: None,
            credential_class: CredentialClass::UserOwned,
            model: None,
            reported_model: None,
            token_breakdown: None,
            audio_tokens: None,
            image_tokens: None,
            reserved_credits: crate::models::credits::Credits::from_whole(0),
            funding: None,
            quantity: Some(1),
            pending_resale_quantity: None,
            pending_platform_usage: None,
            usage_checkpoint: None,
            realtime_session: None,
            pool_attempt: None,
            machine_usage: None,
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
            code_conflict: false,
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

    #[tokio::test]
    async fn reconcile_locally_acks_fully_benefit_funded_usage() {
        let Some(db) = connect_test_database("billing_reconcile_benefit_funded_ack").await else {
            return;
        };
        let mut row = finalized_row("tx-benefit-funded");
        row.funding = Some(crate::models::usage_meter::UsageFunding {
            settled: true,
            wallet_charge_credits: Some(crate::models::credits::Credits::from_whole(0)),
            lago_billable_quantity_micros: Some(0),
            settled_at: Some(Utc::now()),
            ..Default::default()
        });
        let row_id = row.id.clone();
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .expect("insert row");
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(StaticLago {
                result: Err(LagoError::dead_letter(
                    "must_not_push",
                    "fully funded usage must stay local",
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

        assert_eq!(stats.pushed, 0);
        assert_eq!(stats.dead_lettered, 0);
        assert!(saved.lago_acked);
        assert_eq!(saved.status, UsageStatus::Finalized);
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

        let stats = reconciler.run_once_failing_for_test(&["usage_push"]).await;
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
        row.reserved_credits = crate::models::credits::Credits::from_whole(1);
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

    #[derive(Clone, Default)]
    struct RatesLago {
        rates: Vec<super::super::lago_client::PlanRate>,
        rejected: Vec<String>,
        expected_plan_code: Option<String>,
        subscription: Option<super::super::lago_client::RuntimeSubscription>,
        wallet_change_after_proof: Option<mongodb::Database>,
        events: std::sync::Arc<std::sync::Mutex<Vec<LagoEvent>>>,
    }

    #[async_trait]
    impl super::LagoApi for RatesLago {
        async fn ensure_customer(
            &self,
            owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            _plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            Ok(customer_id.to_string())
        }

        async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
        }

        async fn record_events_batch(
            &self,
            _events: &[LagoEvent],
        ) -> Result<Vec<LagoAck>, LagoError> {
            Ok(Vec::new())
        }

        async fn current_usage(
            &self,
            customer_id: &str,
            subscription_id: &str,
        ) -> crate::errors::AppResult<super::super::lago_client::LagoUsage> {
            crate::services::billing::lago_client::require_subscription_id(subscription_id)?;
            Ok(super::super::lago_client::LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: serde_json::Value::Null,
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
            Ok(0)
        }

        async fn entitlements(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<Vec<super::super::lago_client::Entitlement>> {
            Ok(Vec::new())
        }

        async fn plan_rates(
            &self,
            plan_code: &str,
        ) -> crate::errors::AppResult<super::super::lago_client::PlanRates> {
            if self
                .expected_plan_code
                .as_deref()
                .is_some_and(|expected| expected != plan_code)
            {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "test unexpected billing plan".into(),
                ));
            }
            Ok(super::super::lago_client::PlanRates {
                rates: self.rates.clone(),
                rejected: self.rejected.clone(),
            })
        }

        async fn runtime_subscription(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<super::super::lago_client::RuntimeSubscription> {
            if let Some(db) = &self.wallet_change_after_proof {
                db.collection::<mongodb::bson::Document>(super::BILLING_WALLET)
                    .update_one(
                        doc! {"_id":"wallet-1"},
                        doc! {"$set":{"lago_subscription_id":"changed-after-proof"}},
                    )
                    .await?;
            }
            self.subscription.clone().ok_or_else(|| {
                crate::errors::AppError::BillingProviderUnavailable(
                    "test subscription unavailable".into(),
                )
            })
        }
    }

    #[tokio::test]
    async fn machine_provider_rate_drift_retains_usage_and_continues_service_emission() {
        use crate::models::credits::Credits;
        use crate::models::machine_runtime_billing::{MachineRuntimeRate, RATE_COLLECTION};
        use crate::models::service_billing::PricingSyncStatus;
        use crate::models::usage_meter::UsageFunding;

        let db =
            crate::test_utils::connect_transaction_test_database("machine_provider_drift").await;
        let rate = MachineRuntimeRate {
            profile_version: "remote-machine".into(),
            id: "11111111-1111-4111-8111-111111111111".into(),
            metric: BillingMetric::MachineRunningSeconds,
            lago_metric_code: "remote_machine_running_seconds_11111111111141118111111111111111"
                .into(),
            plan_code: "machine-test-plan".into(),
            credits_per_second: Credits::from_pico(40_000_000).unwrap(),
            sync_status: PricingSyncStatus::Synced,
            retired: false,
            verified_at: Some(Utc::now()),
            created_at: Utc::now(),
        };
        db.collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .insert_one(&rate)
            .await
            .unwrap();
        let mut machine = finalized_row("machine-provider-drift");
        machine.metric = BillingMetric::MachineRunningSeconds;
        machine.lago_metric_code = rate.lago_metric_code.clone();
        machine.funding = Some(UsageFunding {
            settlement_rate: Some(rate.credits_per_second),
            settled: true,
            wallet_charge_credits: Some(rate.credits_per_second),
            lago_billable_quantity_micros: Some(1_000_000),
            ..Default::default()
        });
        let service = finalized_row("service-emits-despite-machine-drift");
        let rows = db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        rows.insert_many([&machine, &service]).await.unwrap();
        let lago = RatesLago {
            rates: vec![super::super::lago_client::PlanRate {
                lago_metric_code: rate.lago_metric_code,
                credits_per_unit_micros: 41,
                credits_per_unit_pico: Some(41_000_000),
            }],
            rejected: vec![],
            ..Default::default()
        };
        let reconciler = BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(billing_enabled_config()),
        );
        let mut stats = super::ReconcileStats::default();
        assert!(matches!(
            reconciler.push_unacked(&lago, &mut stats).await,
            Err(crate::errors::AppError::BillingRecoveryFailed {
                failed_units: 1,
                ..
            })
        ));
        assert_eq!(stats.pushed, 1);
        assert_eq!(stats.retried, 1);
        assert_eq!(stats.dead_lettered, 0);
        let saved = rows
            .find_one(doc! { "_id": &machine.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!saved.lago_acked);
        assert_eq!(saved.status, UsageStatus::Finalized);
        assert_eq!(saved.attempt, 1);
        assert!(saved.expires_at.is_none());
        assert_eq!(
            saved.last_error.as_deref(),
            Some("Runtime billing provider binding verification will retry")
        );
        assert!(
            rows.find_one(doc! { "_id": &service.id })
                .await
                .unwrap()
                .unwrap()
                .lago_acked
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn machine_retired_rate_emits_only_at_accepted_provider_price() {
        use crate::models::credits::Credits;
        use crate::models::machine_runtime_billing::{MachineRuntimeRate, RATE_COLLECTION};
        use crate::models::service_billing::PricingSyncStatus;
        use crate::models::usage_meter::UsageFunding;

        let db =
            crate::test_utils::connect_transaction_test_database("machine_provider_retired").await;
        seed_machine_emission_wallet(&db).await;
        let rate = MachineRuntimeRate {
            profile_version: "remote-machine".into(),
            id: "22222222-2222-4222-8222-222222222222".into(),
            metric: BillingMetric::MachineRunningSeconds,
            lago_metric_code: "remote_machine_running_seconds_22222222222242228222222222222222"
                .into(),
            plan_code: "machine-test-plan".into(),
            credits_per_second: Credits::from_pico(40_000_000).unwrap(),
            sync_status: PricingSyncStatus::Synced,
            retired: true,
            verified_at: Some(Utc::now()),
            created_at: Utc::now(),
        };
        db.collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .insert_one(&rate)
            .await
            .unwrap();
        let mut machine = finalized_row("machine-provider-retired");
        machine.metric = BillingMetric::MachineRunningSeconds;
        machine.lago_metric_code = rate.lago_metric_code.clone();
        machine.funding = Some(UsageFunding {
            settlement_rate: Some(rate.credits_per_second),
            settled: true,
            wallet_charge_credits: Some(rate.credits_per_second),
            lago_billable_quantity_micros: Some(1_000_000),
            ..Default::default()
        });
        let rows = db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        rows.insert_one(&machine).await.unwrap();
        let lago = RatesLago {
            rates: vec![
                super::super::lago_client::PlanRate {
                    lago_metric_code: "remote_machine_running_seconds_new".into(),
                    credits_per_unit_micros: 60,
                    credits_per_unit_pico: Some(60_000_000),
                },
                super::super::lago_client::PlanRate {
                    lago_metric_code: rate.lago_metric_code,
                    credits_per_unit_micros: 40,
                    credits_per_unit_pico: Some(40_000_000),
                },
            ],
            rejected: vec![],
            subscription: Some(machine_subscription()),
            expected_plan_code: Some("machine-test-plan".into()),
            ..Default::default()
        };
        let reconciler = BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(billing_enabled_config()),
        );
        let mut stats = super::ReconcileStats::default();
        reconciler.push_unacked(&lago, &mut stats).await.unwrap();
        assert_eq!(stats.pushed, 1);
        assert_eq!(stats.retried, 0);
        assert_eq!(stats.dead_lettered, 0);
        assert!(
            rows.find_one(doc! { "_id": &machine.id })
                .await
                .unwrap()
                .unwrap()
                .lago_acked
        );
        db.drop().await.unwrap();
    }

    fn machine_subscription() -> super::super::lago_client::RuntimeSubscription {
        super::super::lago_client::RuntimeSubscription {
            external_id: "machine-subscription".into(),
            external_customer_id: "machine-customer".into(),
            plan_code: "machine-test-plan".into(),
            status: "active".into(),
        }
    }

    async fn seed_machine_emission_wallet(db: &mongodb::Database) {
        let now = mongodb::bson::DateTime::now();
        db.collection::<mongodb::bson::Document>(super::BILLING_WALLET)
            .insert_one(doc! {
                "_id":"wallet-1","owner_id":"owner-1",
                "lago_customer_id":"machine-customer",
                "lago_wallet_id":"provider-wallet","lago_subscription_id":"machine-subscription",
                "plan_kind":"prepaid","collection_state":"good",
                "created_at":now,"updated_at":now,"balance_synced_at":now,
            })
            .await
            .unwrap();
    }

    async fn machine_emission_fixture(db: &mongodb::Database) -> (UsageMeterRow, RatesLago) {
        use crate::models::credits::Credits;
        use crate::models::machine_runtime_billing::{MachineRuntimeRate, RATE_COLLECTION};
        use crate::models::service_billing::PricingSyncStatus;
        use crate::models::usage_meter::UsageFunding;

        seed_machine_emission_wallet(db).await;
        let id = Uuid::new_v4().to_string();
        let rate = MachineRuntimeRate {
            profile_version: "remote-machine".into(),
            lago_metric_code: format!("remote_machine_running_seconds_{}", id.replace('-', "")),
            id,
            metric: BillingMetric::MachineRunningSeconds,
            plan_code: "machine-test-plan".into(),
            credits_per_second: Credits::from_pico(40_000_000).unwrap(),
            sync_status: PricingSyncStatus::Synced,
            retired: true,
            verified_at: Some(Utc::now()),
            created_at: Utc::now(),
        };
        db.collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .insert_one(&rate)
            .await
            .unwrap();
        let mut row = finalized_row("machine-provider-binding");
        row.metric = BillingMetric::MachineRunningSeconds;
        row.lago_metric_code = rate.lago_metric_code.clone();
        row.funding = Some(UsageFunding {
            settlement_rate: Some(rate.credits_per_second),
            settled: true,
            wallet_charge_credits: Some(rate.credits_per_second),
            lago_billable_quantity_micros: Some(1_000_000),
            ..Default::default()
        });
        let lago = RatesLago {
            rates: vec![super::super::lago_client::PlanRate {
                lago_metric_code: rate.lago_metric_code,
                credits_per_unit_micros: 40,
                credits_per_unit_pico: Some(40_000_000),
            }],
            subscription: Some(machine_subscription()),
            expected_plan_code: Some("machine-test-plan".into()),
            ..Default::default()
        };
        (row, lago)
    }

    #[tokio::test]
    async fn machine_emission_rejects_subscription_drift_without_blocking_services() {
        for mismatch in [
            "id",
            "customer",
            "plan",
            "status",
            "missing",
            "wallet",
            "wallet_replaced",
            "subscription_missing",
            "owner",
        ] {
            let db = crate::test_utils::connect_transaction_test_database(
                "machine_provider_binding_drift",
            )
            .await;
            let (machine, mut lago) = machine_emission_fixture(&db).await;
            let subscription = lago.subscription.as_mut().unwrap();
            match mismatch {
                "id" => subscription.external_id = "other-subscription".into(),
                "customer" => subscription.external_customer_id = "other-customer".into(),
                "plan" => subscription.plan_code = "other-plan".into(),
                "status" => subscription.status = "terminated".into(),
                "missing" => lago.subscription = None,
                "wallet" => {
                    db.collection::<mongodb::bson::Document>(super::BILLING_WALLET)
                        .delete_one(doc! {"_id":"wallet-1"})
                        .await
                        .unwrap();
                }
                "owner" => {
                    db.collection::<mongodb::bson::Document>(super::BILLING_WALLET)
                        .update_one(
                            doc! {"_id":"wallet-1"},
                            doc! {"$set":{"owner_id":"other-owner"}},
                        )
                        .await
                        .unwrap();
                }
                "wallet_replaced" => {
                    let wallets = db.collection::<mongodb::bson::Document>(super::BILLING_WALLET);
                    let mut replacement = wallets
                        .find_one(doc! {"_id":"wallet-1"})
                        .await
                        .unwrap()
                        .unwrap();
                    wallets.delete_one(doc! {"_id":"wallet-1"}).await.unwrap();
                    replacement.insert("_id", "replacement-wallet");
                    wallets.insert_one(replacement).await.unwrap();
                }
                "subscription_missing" => {
                    db.collection::<mongodb::bson::Document>(super::BILLING_WALLET)
                        .update_one(
                            doc! {"_id":"wallet-1"},
                            doc! {"$unset":{"lago_subscription_id":""}},
                        )
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let service = finalized_row("service-despite-subscription-drift");
            let rows = db.collection::<UsageMeterRow>(super::USAGE_METER);
            rows.insert_many([&machine, &service]).await.unwrap();
            let reconciler = BillingReconciler::new(
                db.clone(),
                None,
                std::sync::Arc::new(billing_enabled_config()),
            );
            let mut stats = super::ReconcileStats::default();
            assert!(matches!(
                reconciler.push_unacked(&lago, &mut stats).await,
                Err(crate::errors::AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ));
            assert_eq!(stats.pushed, 1, "{mismatch}");
            assert_eq!(stats.retried, 1, "{mismatch}");
            assert_eq!(stats.dead_lettered, 0, "{mismatch}");
            let emitted = lago.events.lock().unwrap().clone();
            assert_eq!(emitted.len(), 1, "{mismatch}");
            assert_eq!(
                emitted[0].transaction_id, service.transaction_id,
                "{mismatch}"
            );
            let saved = rows
                .find_one(doc! {"_id":&machine.id})
                .await
                .unwrap()
                .unwrap();
            assert!(!saved.lago_acked, "{mismatch}");
            assert_eq!(saved.status, UsageStatus::Finalized, "{mismatch}");
            assert!(saved.expires_at.is_none(), "{mismatch}");
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn usage_capture_runtime_retry_refuses_a_different_verified_target() {
        let db =
            crate::test_utils::connect_transaction_test_database("capture_runtime_rebinding").await;
        let (row, mut lago) = machine_emission_fixture(&db).await;
        db.collection::<UsageMeterRow>(super::USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        super::super::usage_delivery::prepare(
            &db,
            &row,
            "machine-customer",
            LagoEvent::from_usage_row(&row, Some("machine-subscription".into())).unwrap(),
        )
        .await
        .unwrap();
        db.collection::<super::BillingWallet>(super::BILLING_WALLET)
            .update_one(
                doc! {"_id":"wallet-1"},
                doc! {"$set":{"lago_subscription_id":"replacement-subscription"}},
            )
            .await
            .unwrap();
        lago.subscription.as_mut().unwrap().external_id = "replacement-subscription".into();
        let worker = BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(billing_enabled_config()),
        );
        let mut stats = super::ReconcileStats::default();
        assert!(worker.push_unacked(&lago, &mut stats).await.is_err());
        assert_eq!(stats.retried, 1);
        assert!(lago.events.lock().unwrap().is_empty());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn machine_emission_uses_verified_subscription_after_wallet_changes() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_provider_binding_race")
                .await;
        let (machine, mut lago) = machine_emission_fixture(&db).await;
        lago.wallet_change_after_proof = Some(db.clone());
        db.collection::<UsageMeterRow>(super::USAGE_METER)
            .insert_one(&machine)
            .await
            .unwrap();
        let reconciler = BillingReconciler::new(
            db.clone(),
            None,
            std::sync::Arc::new(billing_enabled_config()),
        );
        let mut stats = super::ReconcileStats::default();
        reconciler.push_unacked(&lago, &mut stats).await.unwrap();
        assert_eq!(stats.pushed, 1);
        assert_eq!(stats.retried, 0);
        let emitted = lago.events.lock().unwrap().clone();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].transaction_id, machine.transaction_id);
        assert_eq!(emitted[0].code, machine.lago_metric_code);
        assert_eq!(
            emitted[0].external_subscription_id.as_deref(),
            Some("machine-subscription")
        );
        assert_eq!(emitted[0].external_customer_id, None);
        let receipt = db
            .collection::<mongodb::bson::Document>("billing_usage_deliveries")
            .find_one(doc! {"transaction_id":&machine.transaction_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.get_str("customer_id"), Ok("machine-customer"));
        assert_eq!(
            receipt.get_str("subscription_id"),
            Ok("machine-subscription")
        );
        let wallet = db
            .collection::<super::BillingWallet>(super::BILLING_WALLET)
            .find_one(doc! {"_id":"wallet-1"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            wallet.lago_subscription_id.as_deref(),
            Some("changed-after-proof")
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reconcile_refreshes_rate_cache_from_plan_charges() {
        let Some(db) = connect_test_database("billing_reconcile_rates").await else {
            return;
        };
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(RatesLago {
                rejected: Vec::new(),
                rates: vec![
                    super::super::lago_client::PlanRate {
                        lago_metric_code: "platform_tokens".to_string(),
                        credits_per_unit_micros: 5,
                        credits_per_unit_pico: None,
                    },
                    super::super::lago_client::PlanRate {
                        lago_metric_code: "platform_svc_example_byok_cache_read_tokens".into(),
                        credits_per_unit_micros: 0,
                        credits_per_unit_pico: Some(250001),
                    },
                ],
                ..Default::default()
            })),
            std::sync::Arc::new(billing_enabled_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");

        let collection =
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME);
        let tokens = collection
            .find_one(doc! { "_id": "platform_tokens:*" })
            .await
            .expect("query tokens rate")
            .expect("tokens rate exists");
        let requests = collection
            .find_one(doc! { "_id": "platform_requests:*" })
            .await
            .expect("query requests rate")
            .expect("unpriced platform metric zero-fills");

        let precise = collection
            .find_one(doc! { "lago_metric_code": "platform_svc_example_byok_cache_read_tokens" })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(precise.credits_per_unit_pico, Some(250001));
        assert_eq!(precise.credits_per_unit_micros, 0);
        assert_eq!(stats.rate_cache_refreshes, 4);
        assert_eq!(tokens.credits_per_unit_micros, 5);
        assert_eq!(requests.credits_per_unit_micros, 0);
    }

    #[tokio::test]
    async fn reconcile_keeps_existing_rates_when_plan_has_no_charges() {
        let Some(db) = connect_test_database("billing_reconcile_rates_empty").await else {
            return;
        };
        let collection =
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME);
        collection
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id("platform_tokens", None),
                lago_metric_code: "platform_tokens".to_string(),
                model: None,
                credits_per_unit_micros: 7,
                credits_per_unit_pico: None,
                synced_at: Utc::now(),
                retired_at: None,
            })
            .await
            .expect("seed rate");
        let reconciler = BillingReconciler::new(
            db.clone(),
            Some(std::sync::Arc::new(RatesLago {
                rates: Vec::new(),
                rejected: Vec::new(),
                ..Default::default()
            })),
            std::sync::Arc::new(billing_enabled_config()),
        );

        let stats = reconciler.run_once().await.expect("run reconcile");

        let saved = collection
            .find_one(doc! { "_id": "platform_tokens:*" })
            .await
            .expect("query rate")
            .expect("rate exists");
        assert_eq!(stats.rate_cache_refreshes, 0);
        assert_eq!(saved.credits_per_unit_micros, 7);
    }

    #[tokio::test]
    async fn invalid_metric_loses_cached_rate_without_free_fallback_or_plan_outage() {
        use super::ReconcileStats;
        use mongodb::bson::{Bson, Document};
        use std::sync::Arc;

        let db = connect_test_database("billing_invalid_rate_cache")
            .await
            .unwrap();
        let collection =
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME);
        collection
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id("platform_tokens", None),
                lago_metric_code: "platform_tokens".into(),
                model: None,
                credits_per_unit_micros: 7,
                credits_per_unit_pico: None,
                synced_at: Utc::now(),
                retired_at: None,
            })
            .await
            .unwrap();
        let lago = RatesLago {
            rates: vec![super::super::lago_client::PlanRate {
                lago_metric_code: "platform_requests".into(),
                credits_per_unit_micros: 9,
                credits_per_unit_pico: Some(9_000_000),
            }],
            rejected: vec!["platform_tokens".into()],
            ..Default::default()
        };
        let config = Arc::new(billing_enabled_config());
        let reconciler = BillingReconciler::new(db.clone(), None, config.clone());
        reconciler
            .refresh_rate_cache(&lago, &mut ReconcileStats::default())
            .await
            .unwrap();
        assert!(
            collection
                .find_one(doc! { "_id": "platform_tokens:*" })
                .await
                .unwrap()
                .is_none()
        );
        let valid = collection
            .find_one(doc! { "_id": "platform_requests:*" })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(valid.credits_per_unit_micros, 9);
        let diagnostic = db
            .collection::<Document>("billing_rate_diagnostics")
            .find_one(doc! { "_id": &config.lago_plan_code })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            diagnostic.get_array("rejected_metrics").unwrap(),
            &[Bson::String("platform_tokens".into())]
        );
    }
}

#[cfg(test)]
#[path = "reconcile_provider_isolation_tests.rs"]
mod provider_isolation_tests;
