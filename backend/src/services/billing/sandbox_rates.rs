//! Prospective, immutable CMA configuration using shared exact prices and Lago sync.
use super::{BillingService, sandbox_runtime::METRICS};
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_rate_cache::BillingRateCache,
        credits::Credits,
        sandbox_billing::SandboxPriceSnapshot,
        sandbox_rate_card::{COLLECTION_NAME, SandboxRateCard},
        service_billing::PricingSyncStatus,
    },
    services::audit_service::{self, AuditActor},
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Longer than the maximum funded window: published prices cannot backdate a hold.
pub const MIN_NOTICE_SECONDS: i64 = 301;

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}
fn unavailable(message: &str) -> AppError {
    AppError::BillingProviderUnavailable(message.into())
}

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    for field in ["version", "effective_from"] {
        db.collection::<Document>(COLLECTION_NAME)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {field:1})
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
    }
    Ok(())
}

fn prices(version: i64, rates: &BTreeMap<String, String>) -> AppResult<Vec<SandboxPriceSnapshot>> {
    if rates.len() != METRICS.len() {
        return Err(invalid("Supply exactly CPU, RAM and retained-disk rates"));
    }
    METRICS
        .iter()
        .map(|metric| {
            let raw = rates
                .get(metric.as_str())
                .ok_or_else(|| invalid("Missing Sandbox resource rate"))?;
            let normalized = super::pricing::normalize_price(raw)?;
            let amount: Credits = normalized.parse()?;
            if amount <= Credits::ZERO {
                return Err(invalid("Prepaid Sandbox rates must be positive"));
            }
            Ok(SandboxPriceSnapshot {
                metric: *metric,
                lago_metric_code: format!("cma_{}_v{version}", metric.as_str()),
                credits_per_unit: amount,
            })
        })
        .collect()
}

pub async fn list(db: &Database) -> AppResult<Vec<SandboxRateCard>> {
    Ok(db
        .collection::<SandboxRateCard>(COLLECTION_NAME)
        .find(doc! {})
        .sort(doc! {"version":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

pub async fn effective(db: &Database, at: DateTime<Utc>) -> AppResult<SandboxRateCard> {
    // Select the configured version before testing sync. Never fall back to an old price.
    db.collection::<SandboxRateCard>(COLLECTION_NAME)
        .find_one(doc! {"effective_from":{"$lte":bson::DateTime::from_chrono(at)}})
        .sort(doc! {"effective_from":-1})
        .await?
        .ok_or_else(|| unavailable("CMA rate card is not configured"))
}

/// Readback, including delayed usage, can discover provider-side drift after
/// a successful sync. Retain the version and let the existing worker repair it.
pub(super) async fn request_resync(db: &Database, rate_id: &str) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(doc! {"_id":rate_id}, doc! {"$set":{"sync_status":"failed"}})
        .await?;
    Ok(())
}

/// Shared write fence makes publication and funded admissions serialize.
pub(super) async fn fence(db: &Database, session: &mut ClientSession) -> AppResult<()> {
    let result = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"version":1},
            doc! {"$inc":{"admission_revision":1_i64}},
        )
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(unavailable("CMA initial rate card is missing"));
    }
    Ok(())
}

pub(super) async fn coverage_limit(
    db: &Database,
    session: &mut ClientSession,
    rate_id: &str,
    from: i64,
    until: i64,
) -> AppResult<i64> {
    fence(db, session).await?;
    let at = bson::DateTime::from_millis(
        from.checked_mul(1000)
            .ok_or_else(|| invalid("Sandbox time overflow"))?,
    );
    let rates = db.collection::<SandboxRateCard>(COLLECTION_NAME);
    let current = rates
        .find_one(doc! {"effective_from":{"$lte":at}})
        .sort(doc! {"effective_from":-1})
        .session(&mut *session)
        .await?
        .ok_or_else(|| unavailable("CMA rate card is not configured"))?;
    if current.id != rate_id {
        return Err(AppError::Conflict("CMA rate changed; finish prior coverage and obtain a fresh quote for a new lifecycle operation".into()));
    }
    if current.sync_status != PricingSyncStatus::Synced {
        return Err(unavailable("CMA rate card is not synchronized"));
    }
    let next = rates
        .find_one(doc! {"effective_from":{"$gt":at}})
        .sort(doc! {"effective_from":1})
        .session(session)
        .await?;
    Ok(next.map_or(until, |next| until.min(next.effective_from.timestamp())))
}

/// Deploy-time, idempotent seed. The approved values exist only in this seed asset.
pub async fn seed(db: &Database, plan: &str, key: &[u8]) -> AppResult<()> {
    ensure_indexes(db).await?;
    let rates: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../../assets/cma-sandbox-rates.json"))
            .map_err(|_| AppError::Internal("Invalid initial CMA rate configuration".into()))?;
    for _ in 0..8 {
        if db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"version":1})
            .await?
            .is_some()
        {
            return Ok(());
        }
        let now = Utc::now();
        let card = SandboxRateCard {
            id: Uuid::new_v4().to_string(),
            version: 1,
            effective_from: DateTime::from_timestamp(now.timestamp(), 0).unwrap(),
            plan_code: plan.into(),
            prices: prices(1, &rates)?,
            sync_status: PricingSyncStatus::Pending,
            created_by: None,
            created_at: now,
        };
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .build(),
            )
            .await?;
        let result: AppResult<()> = async {
            db.collection::<SandboxRateCard>(COLLECTION_NAME).insert_one(&card).session(&mut session).await?;
            audit_service::log_system_event_in_session(db,&mut session,key,"cma_rate_card_seeded",serde_json::json!({"version":1,"rate_card_id":card.id,"source_pr":2151,"effective_from":card.effective_from,"rates":rates})).await?;
            session.commit_transaction().await?;
            Ok(())
        }.await;
        if result.is_ok() {
            return Ok(());
        }
        let _ = session.abort_transaction().await;
        // Another startup may own the unique version. Only accept its durable result.
        if db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"version":1})
            .await?
            .is_some()
        {
            return Ok(());
        }
        if let Err(AppError::DatabaseError(ref error)) = result
            && error.contains_label("TransientTransactionError")
        {
            continue;
        }
        return result;
    }
    Err(unavailable("CMA rate seed contention; retry startup"))
}

pub struct PublishRateCard {
    pub expected_version: i64,
    pub effective_from: DateTime<Utc>,
    pub rates: BTreeMap<String, String>,
}

pub async fn publish(
    db: &Database,
    plan: &str,
    input: PublishRateCard,
    actor: &AuditActor,
    key: &[u8],
) -> AppResult<SandboxRateCard> {
    let now = Utc::now();
    if input.effective_from.timestamp_subsec_nanos() != 0
        || input.effective_from < now + Duration::seconds(MIN_NOTICE_SECONDS)
    {
        return Err(invalid(
            "effective_from must be a whole UTC second at least 301 seconds in the future",
        ));
    }
    let version = input
        .expected_version
        .checked_add(1)
        .filter(|v| *v > 1)
        .ok_or_else(|| invalid("Invalid expected_version"))?;
    let card = SandboxRateCard {
        id: Uuid::new_v4().to_string(),
        version,
        effective_from: input.effective_from,
        plan_code: plan.into(),
        prices: prices(version, &input.rates)?,
        sync_status: PricingSyncStatus::Pending,
        created_by: Some(actor.user_id.clone()),
        created_at: now,
    };
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(
            mongodb::options::WriteConcern::builder()
                .w(mongodb::options::Acknowledgment::Majority)
                .build(),
        )
        .await?;
    fence(db, &mut session).await?;
    let latest = db
        .collection::<SandboxRateCard>(COLLECTION_NAME)
        .find_one(doc! {})
        .sort(doc! {"version":-1})
        .session(&mut session)
        .await?
        .ok_or_else(|| unavailable("CMA initial rate card is missing"))?;
    if latest.version != input.expected_version || latest.effective_from >= card.effective_from {
        return Err(AppError::Conflict("CMA rate history changed or effective_from does not follow the latest version; reload before publishing".into()));
    }
    db.collection::<SandboxRateCard>(COLLECTION_NAME)
        .insert_one(&card)
        .session(&mut session)
        .await?;
    audit_service::log_actor_event_in_session(db,&mut session,key,actor,"cma_rate_card_published",serde_json::json!({"rate_card_id":card.id,"version":version,"previous_version":latest.version,"effective_from":card.effective_from,"rates":input.rates})).await?;
    session.commit_transaction().await?;
    Ok(card)
}

impl BillingService {
    /// Immutable codes make retries safe and retain historical provider charges.
    /// Admission still verifies the effective subscription, never just this marker.
    pub async fn retry_sandbox_rate_cards(&self) -> AppResult<u64> {
        let Some(lago) = self.lago.as_deref().filter(|_| self.config.billing_enabled) else {
            return Ok(0);
        };
        if !crate::services::feature_flag_service::cma_sandbox_rate_sync_enabled(&self.db).await? {
            return Ok(0);
        }
        let now = Utc::now();
        let claim = Uuid::new_v4().to_string();
        let lock = self.db.collection::<Document>(COLLECTION_NAME).find_one_and_update(
            doc! {"version":1,"$or":[{"sync_claim_until":bson::Bson::Null},{"sync_claim_until":{"$lte":bson::DateTime::from_chrono(now)}}]},
            doc! {"$set":{"sync_claim":&claim,"sync_claim_until":bson::DateTime::from_chrono(now+Duration::minutes(2))}},
        ).await?;
        if lock.is_none() {
            return Ok(0);
        }
        // Bound the complete batch below its lease. A crash leaves pending/failed
        // versions available after expiry; no new price is invented on recovery.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            self.sync_sandbox_rate_batch(lago),
        )
        .await;
        if result.is_ok() {
            self.db
                .collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"version":1,"sync_claim":claim},
                    doc! {"$unset":{"sync_claim":"","sync_claim_until":""}},
                )
                .await?;
        } else {
            tracing::warn!(
                plan_code = %self.config.lago_plan_code,
                timeout_secs = 60,
                "CMA rate synchronization timed out; retaining lease and pending versions for retry"
            );
        }
        // An ambiguous timeout keeps the lease through its original expiry.
        result.map_err(|_| unavailable("CMA rate synchronization timed out"))?
    }

    async fn sync_sandbox_rate_batch(
        &self,
        lago: &dyn super::lago_client::LagoApi,
    ) -> AppResult<u64> {
        let cards: Vec<SandboxRateCard> = self
            .db
            .collection::<SandboxRateCard>(COLLECTION_NAME)
            .find(doc! {"sync_status":{"$ne":"synced"},"plan_code":&self.config.lago_plan_code})
            .sort(doc! {"version":1})
            .limit(4)
            .await?
            .try_collect()
            .await?;
        let mut synced = 0;
        for card in cards {
            if !super::reconcile::continue_reconcile() {
                break;
            }
            let result: AppResult<bool> = async {
                for price in &card.prices {
                    if !super::reconcile::continue_reconcile() {
                        return Ok(false);
                    }
                    super::taxes::sync_charge(
                        &self.db,
                        lago,
                        &card.plan_code,
                        &super::lago_client::ServicePriceSync {
                            metric_code: price.lago_metric_code.clone(),
                            metric_name: format!("CMA {} v{}", price.metric.as_str(), card.version),
                            metric_description: format!(
                                "CMA allocated resource usage, rate card {}",
                                card.id
                            ),
                            credits_per_unit: price.credits_per_unit.to_string(),
                        },
                    )
                    .await?;
                    let pico = i64::try_from(price.credits_per_unit.pico())
                        .map_err(|_| invalid("CMA price overflow"))?;
                    let cache = BillingRateCache {
                        id: BillingRateCache::cache_id(&price.lago_metric_code, None),
                        lago_metric_code: price.lago_metric_code.clone(),
                        model: None,
                        credits_per_unit_pico: Some(pico),
                        credits_per_unit_micros: pico / 1_000_000,
                        synced_at: Utc::now(),
                        retired_at: None,
                    };
                    self.db
                        .collection::<BillingRateCache>(
                            crate::models::billing_rate_cache::COLLECTION_NAME,
                        )
                        .replace_one(doc! {"_id":&cache.id}, cache)
                        .upsert(true)
                        .await?;
                }
                Ok(true)
            }
            .await;
            if matches!(result, Ok(false)) {
                // Partial cards stay pending; do not publish a partial sync.
                break;
            }
            if let Err(error) = &result {
                tracing::warn!(
                    rate_card_id = %card.id,
                    version = card.version,
                    plan_code = %card.plan_code,
                    error_code = error.error_code(),
                    "CMA rate card synchronization failed; recording for retry"
                );
            }
            self.db
                .collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&card.id},
                    doc! {"$set":{"sync_status":if result.is_ok() {"synced"} else {"failed"}}},
                )
                .await?;
            if result.is_ok() {
                synced += 1;
            }
        }
        Ok(synced)
    }
}
