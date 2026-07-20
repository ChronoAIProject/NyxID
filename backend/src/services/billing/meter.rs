use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::AppResult;
use crate::models::service_billing::{BillingMetric, PlatformUsage, ResaleUsage};
use crate::models::usage_meter::{
    BillingLayer, COLLECTION_NAME as USAGE_METER, UsageMeterRow, UsageStatus,
};

use super::reservation::{self, BillingReservation};
use super::route_context::BillingRouteContext;

pub const PLATFORM_REQUESTS_METRIC_CODE: &str = "platform_requests";
pub const PLATFORM_BYTES_METRIC_CODE: &str = "platform_bytes";
pub const PLATFORM_TOKENS_METRIC_CODE: &str = "platform_tokens";

#[derive(Clone, Debug, Default)]
pub struct MeteredProxyContext {
    pub route: Option<BillingRouteContext>,
}

impl MeteredProxyContext {
    pub fn disabled() -> Self {
        Self { route: None }
    }

    pub fn is_enabled(&self) -> bool {
        self.route.is_some()
    }
}

pub async fn open(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    reservation: Option<&BillingReservation>,
) -> AppResult<MeteredProxyContext> {
    if !ctx.is_metered() {
        return Ok(MeteredProxyContext::disabled());
    }

    if ctx.platform_metered() {
        insert_reserved_row(
            db,
            ctx,
            BillingLayer::Platform,
            ctx.platform_metric,
            platform_metric_code(ctx.platform_metric).to_string(),
            reservation,
            None,
        )
        .await?;
    }

    if let Some(resale) = &ctx.resale {
        insert_reserved_row(
            db,
            ctx,
            BillingLayer::Resale,
            resale.metric,
            resale.lago_metric_code.clone(),
            reservation,
            None,
        )
        .await?;
    }

    Ok(MeteredProxyContext {
        route: Some(ctx.clone()),
    })
}

pub async fn mark_forwarded(
    db: &mongodb::Database,
    metered: &MeteredProxyContext,
) -> AppResult<()> {
    let Some(ctx) = &metered.route else {
        return Ok(());
    };

    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_many(
            doc! {
                "billing_request_id": &ctx.billing_request_id,
                "status": "reserved",
            },
            doc! {
                "$set": {
                    "status": "forwarded",
                    "forwarded": true,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;
    Ok(())
}

pub async fn settle(
    db: &mongodb::Database,
    metered: &MeteredProxyContext,
    platform: PlatformUsage,
    resale: Option<ResaleUsage>,
    model: Option<String>,
) -> AppResult<()> {
    let Some(ctx) = &metered.route else {
        return Ok(());
    };

    let mut finalized_rows = Vec::new();
    if ctx.platform_metered()
        && let Some(row) = finalize_layer(
            db,
            ctx,
            BillingLayer::Platform,
            platform_quantity(ctx.platform_metric, &platform),
            model.clone(),
        )
        .await?
    {
        finalized_rows.push(row);
    }

    if let Some(resale_usage) = resale
        && ctx.resale.is_some()
        && let Some(row) = finalize_layer(
            db,
            ctx,
            BillingLayer::Resale,
            resale_usage.quantity.max(0),
            model,
        )
        .await?
    {
        finalized_rows.push(row);
    }

    let mut first_error = None;
    for row in finalized_rows {
        if let Err(error) = reservation::claim_released_and_settle(db, &row).await {
            reservation::record_settlement_failure(db, &row, Utc::now()).await?;
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(())
}

pub async fn fail(
    db: &mongodb::Database,
    metered: &MeteredProxyContext,
    reason: &str,
) -> AppResult<()> {
    let Some(ctx) = &metered.route else {
        return Ok(());
    };

    reservation::release_unforwarded_rows(
        db,
        &ctx.billing_request_id,
        UsageStatus::Failed,
        Some(reason),
    )
    .await?;
    Ok(())
}

async fn insert_reserved_row(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    layer: BillingLayer,
    metric: BillingMetric,
    lago_metric_code: String,
    reservation: Option<&BillingReservation>,
    flush_seq: Option<i64>,
) -> AppResult<()> {
    let now = Utc::now();
    let transaction_id = transaction_id(&ctx.billing_request_id, layer, flush_seq);
    let wallet_id = reservation.map(|reservation| reservation.wallet_id.clone());
    let reserved_credits = reservation
        .map(|reservation| reservation.reserved_for(layer))
        .unwrap_or(0);
    let row = UsageMeterRow {
        id: Uuid::new_v4().to_string(),
        transaction_id,
        billing_request_id: ctx.billing_request_id.clone(),
        layer,
        flush_seq,
        billing_owner_id: ctx.billing_owner_id.clone(),
        wallet_id,
        actor_user_id: ctx.actor_user_id.clone(),
        api_key_id: ctx.api_key_id.clone(),
        service_id: ctx
            .catalog_service_id
            .clone()
            .or_else(|| ctx.user_service_id.clone()),
        service_slug: ctx.service_slug.clone(),
        metric,
        lago_metric_code,
        credential_class: ctx.credential_class,
        model: None,
        reserved_credits,
        quantity: None,
        status: UsageStatus::Reserved,
        forwarded: false,
        released: false,
        lago_acked: false,
        attempt: 0,
        settlement_attempts: 0,
        settlement_next_retry_at: None,
        created_at: now,
        updated_at: now,
        finalized_at: None,
        expires_at: None,
        last_error: None,
    };

    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .map(|_| ())
        .or_else(|error| {
            if is_duplicate_key_error(&error) {
                Ok(())
            } else {
                Err(error)
            }
        })?;
    Ok(())
}

async fn finalize_layer(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    layer: BillingLayer,
    quantity: i64,
    model: Option<String>,
) -> AppResult<Option<UsageMeterRow>> {
    let now = Utc::now();
    let model_for_row = model.clone();
    let collection = db.collection::<UsageMeterRow>(USAGE_METER);
    let claimed = collection
        .find_one_and_update(
            doc! {
                "billing_request_id": &ctx.billing_request_id,
                "layer": layer.as_transaction_suffix(),
                "status": { "$in": ["reserved", "forwarded"] },
            },
            doc! {
                "$set": {
                    "status": "finalized",
                    "forwarded": true,
                    "quantity": quantity,
                    "released": false,
                    "model": model_for_row,
                    "updated_at": bson::DateTime::from_chrono(now),
                    "finalized_at": bson::DateTime::from_chrono(now),
                }
            },
        )
        .with_options(
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;

    let Some(claimed) = claimed else {
        return Ok(None);
    };

    Ok(Some(claimed))
}

pub(crate) fn transaction_id(
    billing_request_id: &str,
    layer: BillingLayer,
    flush_seq: Option<i64>,
) -> String {
    match flush_seq {
        Some(seq) => format!(
            "{}:{}:{}",
            billing_request_id,
            layer.as_transaction_suffix(),
            seq
        ),
        None => format!("{}:{}", billing_request_id, layer.as_transaction_suffix()),
    }
}

pub(crate) fn platform_metric_code(metric: BillingMetric) -> &'static str {
    match metric {
        BillingMetric::Requests => PLATFORM_REQUESTS_METRIC_CODE,
        BillingMetric::Bytes => PLATFORM_BYTES_METRIC_CODE,
        BillingMetric::Tokens => PLATFORM_TOKENS_METRIC_CODE,
    }
}

fn platform_quantity(metric: BillingMetric, usage: &PlatformUsage) -> i64 {
    match metric {
        BillingMetric::Bytes => usage.bytes.max(0),
        BillingMetric::Requests => usage.requests.max(0),
        BillingMetric::Tokens => usage.tokens.max(0),
    }
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("duplicate key")
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::IndexOptions;
    use uuid::Uuid;

    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::service_billing::{BillingMetric, PlatformUsage, ServiceBilling};
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageMeterRow, UsageStatus};
    use crate::services::billing::meter::{
        PLATFORM_TOKENS_METRIC_CODE, mark_forwarded, open, settle,
    };
    use crate::services::billing::reservation::BillingReservation;
    use crate::services::billing::route_context::{BillingRouteContext, NodeIntent};
    use crate::test_utils::connect_test_database;

    #[tokio::test]
    async fn ledger_open_mark_and_settle_are_durable_and_idempotent() {
        let Some(db) = connect_test_database("usage_meter_ledger").await else {
            return;
        };
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! { "transaction_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create transaction id index");

        let billing = ServiceBilling {
            resale_billable: true,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: Some("resale_tokens".to_string()),
        };
        let ctx = BillingRouteContext::new(
            "billing-request-1".to_string(),
            "owner-1".to_string(),
            "actor-1".to_string(),
            Some("api-key-1".to_string()),
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("llm-test".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Bytes,
            Some(&billing),
            true,
        )
        .with_platform_metering(true);

        let metered = open(&db, &ctx, None).await.expect("open meter");
        open(&db, &ctx, None).await.expect("idempotent open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        settle(
            &db,
            &metered,
            PlatformUsage::single_request(42),
            Some(crate::models::service_billing::ResaleUsage {
                metric: BillingMetric::Tokens,
                quantity: 17,
            }),
            Some("test-model".to_string()),
        )
        .await
        .expect("settle");

        let rows: Vec<UsageMeterRow> = db
            .collection(crate::models::usage_meter::COLLECTION_NAME)
            .find(doc! { "billing_request_id": "billing-request-1" })
            .await
            .expect("find rows")
            .try_collect()
            .await
            .expect("collect rows");

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| {
            row.layer == BillingLayer::Platform
                && row.transaction_id == "billing-request-1:platform"
                && row.metric == BillingMetric::Bytes
                && row.quantity == Some(42)
                && row.status == UsageStatus::Finalized
                && row.forwarded
        }));
        assert!(rows.iter().any(|row| {
            row.layer == BillingLayer::Resale
                && row.transaction_id == "billing-request-1:resale"
                && row.metric == BillingMetric::Tokens
                && row.quantity == Some(17)
                && row.credential_class == CredentialClass::NyxidManagedMaster
        }));
    }

    #[tokio::test]
    async fn platform_tokens_settle_as_token_quantity() {
        let Some(db) = connect_test_database("usage_meter_platform_tokens").await else {
            return;
        };
        create_usage_transaction_index(&db).await;

        let ctx = BillingRouteContext::new(
            "billing-token-request-1".to_string(),
            "owner-1".to_string(),
            "actor-1".to_string(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("llm-openai".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Tokens,
            None::<&ServiceBilling>,
            false,
        )
        .with_platform_metering(true);

        let metered = open(&db, &ctx, None).await.expect("open token meter");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        settle(
            &db,
            &metered,
            PlatformUsage::llm_completion(128, 37),
            None,
            Some("gpt-test".to_string()),
        )
        .await
        .expect("settle tokens");

        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-token-request-1" })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(row.layer, BillingLayer::Platform);
        assert_eq!(row.metric, BillingMetric::Tokens);
        assert_eq!(row.lago_metric_code, PLATFORM_TOKENS_METRIC_CODE);
        assert_eq!(row.quantity, Some(37));
        assert_eq!(row.status, UsageStatus::Finalized);
    }

    #[test]
    fn transaction_id_is_per_layer_and_flush() {
        assert_eq!(
            super::transaction_id("req", BillingLayer::Platform, None),
            "req:platform"
        );
        assert_eq!(
            super::transaction_id("req", BillingLayer::Resale, None),
            "req:resale"
        );
        assert_eq!(
            super::transaction_id("req", BillingLayer::Platform, Some(7)),
            "req:platform:7"
        );
    }

    #[tokio::test]
    async fn settle_moves_wallet_once_and_blocks_double_spend_before_lago_sync() {
        let Some(db) = connect_test_database("billing_settle_wallet_once").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        insert_rate(&db, "platform_requests", 5).await;
        let owner_id = "owner-wallet-settle";
        insert_wallet(&db, owner_id, 10, 5).await;

        let ctx = platform_context("billing-wallet-1", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: "wallet-owner-wallet-settle".to_string(),
            total_reserved_credits: 5,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                reserved_credits: 5,
            }],
        };
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 5)
            .await
            .expect("reserve")
            .expect("reserved");

        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        settle(&db, &metered, PlatformUsage::single_request(1), None, None)
            .await
            .expect("settle first time");
        settle(&db, &metered, PlatformUsage::single_request(1), None, None)
            .await
            .expect("settle replay");

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, 5);
        assert_eq!(wallet.available_credits(), 5);

        let second_reservation =
            crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 6)
                .await
                .expect("second reserve query");
        assert!(
            second_reservation.is_none(),
            "pending_lago_debits must reduce availability before Lago sync"
        );
    }

    #[tokio::test]
    async fn recovery_after_settle_debit_gap_does_not_debit_wallet_twice() {
        let Some(db) = connect_test_database("billing_settle_recovery_overlap").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        insert_rate(&db, "platform_requests", 5).await;
        let owner_id = "owner-wallet-recovery";
        insert_wallet(&db, owner_id, 10, 0).await;

        let ctx = platform_context("billing-wallet-recovery", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: "wallet-owner-wallet-recovery".to_string(),
            total_reserved_credits: 5,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                reserved_credits: 5,
            }],
        };
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 5)
            .await
            .expect("reserve")
            .expect("reserved");

        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        settle(&db, &metered, PlatformUsage::single_request(1), None, None)
            .await
            .expect("settle");

        let settled_row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-wallet-recovery" })
            .await
            .expect("find settled row")
            .expect("row exists");
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .update_one(
                doc! { "_id": &settled_row.id },
                doc! {
                    "$set": {
                        "released": false,
                        "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await
            .expect("simulate missing release marker after wallet debit");
        db.collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .update_one(
                doc! { "owner_id": owner_id },
                doc! {
                    "$set": {
                        "active_settlement": {
                            "row_id": &settled_row.id,
                            "reserved_credits": 5_i64,
                            "actual_credits": 5_i64,
                            "applied": true,
                            "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
                        },
                        "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await
            .expect("simulate applied bounded settlement lock");

        let recovered = crate::services::billing::reservation::recover_retryable_settlements_at(
            &db,
            Utc::now() + chrono::Duration::seconds(31),
        )
        .await
        .expect("recover unreleased");
        assert_eq!(recovered.recovered, 1);

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        let wallet_doc = db
            .collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet document")
            .expect("wallet document exists");
        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-wallet-recovery" })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, 5);
        assert!(!wallet_doc.contains_key("active_settlement"));
        assert!(!wallet_doc.contains_key("settled_usage_row_ids"));
        assert!(row.released);
    }

    #[tokio::test]
    async fn fail_releases_only_never_forwarded_wallet_hold() {
        let Some(db) = connect_test_database("billing_fail_releases_hold").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        let owner_id = "owner-fail-release";
        insert_wallet(&db, owner_id, 10, 0).await;

        let ctx = platform_context("billing-wallet-fail", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: "wallet-owner-fail-release".to_string(),
            total_reserved_credits: 4,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                reserved_credits: 4,
            }],
        };
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 4)
            .await
            .expect("reserve")
            .expect("reserved");

        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        super::fail(&db, &metered, "before send")
            .await
            .expect("fail");

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-wallet-fail" })
            .await
            .expect("find row")
            .expect("row exists");

        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, 0);
        assert_eq!(row.status, UsageStatus::Failed);
        assert!(row.released);
    }

    /// ChronoAIProject/NyxID#1023 — the settle path and the reconciler's
    /// `recover_retryable_settlements` sweep can race the SAME finalized row
    /// while it sits in the gap state `{status:finalized, released:false,
    /// wallet_id:set}`. The wallet debit MUST be atomic with the `released`
    /// transition, so the customer is charged exactly once no matter how many
    /// settle replays or recovery sweeps touch the row concurrently.
    ///
    /// This drives `settle` to produce the gap state, then runs the settle
    /// path's `claim_released_and_settle` AND `recover_retryable_settlements`
    /// concurrently against that row, and asserts a single debit
    /// (`reserved_credits` lands at 0, never negative; `pending_lago_debits`
    /// is the single-debit amount, never doubled).
    #[tokio::test]
    async fn settle_and_recovery_sweep_debit_wallet_exactly_once() {
        let Some(db) = connect_test_database("billing_settle_recovery_once").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        // rate = 5 credits/unit, reserve 5; a single debit moves
        // reserved_credits 5 -> 0 and pending_lago_debits 0 -> 5. A double
        // debit would drive reserved_credits to -5 and pending_lago_debits to
        // 10, so the assertions below catch the #1023 regression directly.
        insert_rate(&db, "platform_requests", 5).await;
        let owner_id = "owner-1023-race";
        insert_wallet(&db, owner_id, 10, 0).await;

        let ctx = platform_context("billing-1023", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: format!("wallet-{owner_id}"),
            total_reserved_credits: 5,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                reserved_credits: 5,
            }],
        };
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 5)
            .await
            .expect("reserve")
            .expect("reserved");

        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");

        // Put the row into the exact gap state the bug exploits: finalized,
        // wallet still attached, but NOT yet released — i.e. a crash or pause
        // landed between `finalize_layer`'s finalize claim and its release.
        let collection =
            db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        collection
            .update_one(
                doc! { "billing_request_id": "billing-1023" },
                doc! {
                    "$set": {
                        "status": "finalized",
                        "forwarded": true,
                        "released": false,
                        "quantity": 1_i64,
                    }
                },
            )
            .await
            .expect("force gap state");

        let gap_row = collection
            .find_one(doc! { "billing_request_id": "billing-1023" })
            .await
            .expect("find gap row")
            .expect("gap row exists");
        assert_eq!(gap_row.status, UsageStatus::Finalized);
        assert!(!gap_row.released);
        assert!(gap_row.wallet_id.is_some());

        // Race the settle path's atomic claim+debit against the reconciler
        // recovery sweep. Exactly one of them must win the released CAS and
        // perform the single debit; the other must observe `released:true` and
        // debit nothing.
        let settle_db = db.clone();
        let settle_row = gap_row.clone();
        let settle_task = tokio::spawn(async move {
            crate::services::billing::reservation::claim_released_and_settle(
                &settle_db,
                &settle_row,
            )
            .await
            .expect("settle claim")
        });
        let recover_db = db.clone();
        let recover_task = tokio::spawn(async move {
            crate::services::billing::reservation::recover_retryable_settlements_at(
                &recover_db,
                Utc::now() + chrono::Duration::seconds(31),
            )
            .await
            .expect("recovery sweep")
            .recovered
        });
        let settled_won = settle_task.await.expect("settle join");
        let recovered = recover_task.await.expect("recover join");

        // Run both a second time to prove idempotency under retry: neither a
        // settle replay nor a later sweep re-debits an already-released row.
        let settled_again =
            crate::services::billing::reservation::claim_released_and_settle(&db, &gap_row)
                .await
                .expect("settle replay");
        let recovered_again =
            crate::services::billing::reservation::recover_retryable_settlements_at(
                &db,
                Utc::now() + chrono::Duration::seconds(31),
            )
            .await
            .expect("recovery replay")
            .recovered;

        // Exactly one debit total across all four attempts.
        let winners = usize::from(settled_won) + recovered as usize + usize::from(settled_again);
        assert_eq!(
            winners, 1,
            "exactly one of (settle, recovery, settle-replay) may debit the row"
        );
        assert_eq!(recovered_again, 0, "released row is never re-recovered");

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(
            wallet.reserved_credits, 0,
            "reserved_credits must not go negative (double-debit guard)"
        );
        assert_eq!(
            wallet.pending_lago_debits, 5,
            "wallet must be debited exactly once, never twice"
        );

        let final_row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-1023" })
            .await
            .expect("find final row")
            .expect("row exists");
        assert!(final_row.released, "row must end released exactly once");
        assert_eq!(final_row.status, UsageStatus::Finalized);
    }

    #[tokio::test]
    async fn failed_live_settlement_is_durable_and_recovers_without_double_debit() {
        let Some(db) = connect_test_database("billing_settle_outbox_recovery").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        let owner_id = "owner-settle-outbox";
        insert_wallet(&db, owner_id, 10, 0).await;
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 5)
            .await
            .expect("reserve")
            .expect("reserved");

        let ctx = platform_context("billing-settle-outbox", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: format!("wallet-{owner_id}"),
            total_reserved_credits: 5,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                reserved_credits: 5,
            }],
        };
        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        db.collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .update_one(
                doc! { "owner_id": owner_id },
                doc! { "$set": { "active_settlement": "malformed" } },
            )
            .await
            .expect("inject transient settlement failure");

        settle(&db, &metered, PlatformUsage::single_request(1), None, None)
            .await
            .expect_err("malformed lock must fail live settlement");
        let collection =
            db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        let failed = collection
            .find_one(doc! { "billing_request_id": "billing-settle-outbox" })
            .await
            .expect("find failed row")
            .expect("failed row exists");
        assert_eq!(failed.status, UsageStatus::Failed);
        assert_eq!(failed.settlement_attempts, 1);
        assert!(failed.settlement_next_retry_at.is_some());
        assert!(!failed.released);

        db.collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .update_one(
                doc! { "owner_id": owner_id },
                doc! { "$unset": { "active_settlement": "" } },
            )
            .await
            .expect("repair transient settlement failure");
        let retry_at = failed.settlement_next_retry_at.expect("retry deadline");
        let recovered = crate::services::billing::reservation::recover_retryable_settlements_at(
            &db,
            retry_at + chrono::Duration::seconds(1),
        )
        .await
        .expect("recover failed settlement");
        assert_eq!(recovered.recovered, 1);

        let replay = crate::services::billing::reservation::recover_retryable_settlements_at(
            &db,
            retry_at + chrono::Duration::minutes(10),
        )
        .await
        .expect("replay recovery sweep");
        assert_eq!(replay.recovered, 0);

        let saved = collection
            .find_one(doc! { "_id": &failed.id })
            .await
            .expect("find recovered row")
            .expect("recovered row exists");
        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(saved.status, UsageStatus::Finalized);
        assert!(saved.released);
        assert!(saved.settlement_next_retry_at.is_none());
        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, 5);
    }

    async fn create_usage_transaction_index(db: &mongodb::Database) {
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! { "transaction_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create transaction id index");
    }

    async fn insert_rate(db: &mongodb::Database, metric_code: &str, credits: i64) {
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id(metric_code, None),
                lago_metric_code: metric_code.to_string(),
                model: None,
                credits_per_unit_micros: credits * 1_000_000,
                synced_at: Utc::now(),
            })
            .await
            .expect("insert rate");
    }

    async fn insert_wallet(
        db: &mongodb::Database,
        owner_id: &str,
        balance_credits: i64,
        overdraft_cap_credits: i64,
    ) {
        let now = Utc::now();
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(BillingWallet {
                id: format!("wallet-{owner_id}"),
                owner_id: owner_id.to_string(),
                lago_customer_id: owner_id.to_string(),
                lago_wallet_id: Some(format!("{owner_id}:wallet")),
                lago_subscription_id: Some(format!("{owner_id}:plan")),
                plan_kind: PlanKind::Prepaid,
                balance_credits,
                reserved_credits: 0,
                pending_lago_debits: 0,
                has_payment_instrument: false,
                overdraft_cap_credits,
                suspended: false,
                collection_state: CollectionState::Good,
                balance_synced_at: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert wallet");
    }

    fn platform_context(request_id: &str, owner_id: &str) -> BillingRouteContext {
        BillingRouteContext::new(
            request_id.to_string(),
            owner_id.to_string(),
            "actor-1".to_string(),
            Some(Uuid::new_v4().to_string()),
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            None::<&ServiceBilling>,
            false,
        )
        .with_platform_metering(true)
    }
}
