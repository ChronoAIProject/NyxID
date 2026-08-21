use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_billing::{BillingMetric, PlatformUsage, ResaleUsage};
use crate::models::usage_meter::{
    BillingLayer, COLLECTION_NAME as USAGE_METER, UsageFunding, UsageMeterRow, UsageStatus,
};

use super::reservation::{self, BillingReservation};
use super::route_context::BillingRouteContext;

pub const PLATFORM_REQUESTS_METRIC_CODE: &str = "platform_requests";
pub const PLATFORM_BYTES_METRIC_CODE: &str = "platform_bytes";
pub const PLATFORM_TOKENS_METRIC_CODE: &str = "platform_tokens";
const SETTLEMENT_INTENT_RECOVERY_BATCH_SIZE: i64 = 100;

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

    pub(crate) fn from_route(route: &BillingRouteContext) -> Self {
        Self {
            route: Some(route.clone()),
        }
    }
}

pub(crate) async fn has_complete_meter(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
) -> AppResult<bool> {
    let mut transaction_ids = Vec::with_capacity(2);
    if ctx.platform_metered() {
        transaction_ids.push(transaction_id(
            &ctx.billing_request_id,
            BillingLayer::Platform,
            None,
        ));
    }
    if ctx.resale.is_some() {
        transaction_ids.push(transaction_id(
            &ctx.billing_request_id,
            BillingLayer::Resale,
            None,
        ));
    }
    if transaction_ids.is_empty() {
        return Ok(false);
    }
    let count = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .count_documents(doc! { "transaction_id": { "$in": &transaction_ids } })
        .await?;
    Ok(count == transaction_ids.len() as u64)
}

pub async fn open(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    reservation: Option<&BillingReservation>,
) -> AppResult<MeteredProxyContext> {
    if !ctx.is_metered() {
        return Ok(MeteredProxyContext::disabled());
    }

    let mut inserted_row_ids = Vec::with_capacity(2);
    let result: AppResult<()> = async {
        if ctx.platform_metered() {
            let inserted = insert_reserved_row(
                db,
                ctx,
                BillingLayer::Platform,
                ctx.platform_metric,
                ctx.platform_lago_metric_code.clone(),
                reservation,
                None,
            )
            .await?;
            match inserted {
                Some(row_id) => inserted_row_ids.push(row_id),
                None if reservation.is_some() => {
                    return Err(AppError::Conflict(
                        "billing request is already being metered".to_string(),
                    ));
                }
                None => {}
            }
        }

        if let Some(resale) = &ctx.resale {
            let inserted = insert_reserved_row(
                db,
                ctx,
                BillingLayer::Resale,
                resale.metric,
                resale.lago_metric_code.clone(),
                reservation,
                None,
            )
            .await?;
            match inserted {
                Some(row_id) => inserted_row_ids.push(row_id),
                None if reservation.is_some() => {
                    return Err(AppError::Conflict(
                        "billing request is already being metered".to_string(),
                    ));
                }
                None => {}
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        // The caller releases this attempt's reservation. Delete only rows
        // inserted by this attempt: a broad request-id cleanup could erase an
        // earlier idempotent attempt and release its funding twice.
        if !inserted_row_ids.is_empty()
            && let Err(cleanup_error) = db
                .collection::<UsageMeterRow>(USAGE_METER)
                .delete_many(doc! {
                    "_id": { "$in": &inserted_row_ids },
                    "status": "reserved",
                    "forwarded": false,
                    "released": false,
                })
                .await
        {
            tracing::error!(
                billing_request_id = %ctx.billing_request_id,
                error = %cleanup_error,
                "failed to remove partial usage-meter setup"
            );
        }
        return Err(error);
    }

    Ok(MeteredProxyContext::from_route(ctx))
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
    let persisted = persist_settlement_intent(db, metered, platform, resale, model).await?;
    settle_persisted(db, persisted).await
}

pub(super) async fn persist_settlement_intent(
    db: &mongodb::Database,
    metered: &MeteredProxyContext,
    platform: PlatformUsage,
    resale: Option<ResaleUsage>,
    model: Option<String>,
) -> AppResult<Vec<UsageMeterRow>> {
    let Some(ctx) = &metered.route else {
        return Ok(Vec::new());
    };

    let mut finalized_rows = Vec::new();
    let platform_quantity = ctx
        .platform_metered()
        .then(|| platform_quantity(ctx.platform_metric, &platform));
    let resale_quantity = resale
        .filter(|_| ctx.resale.is_some())
        .map(|usage| usage.quantity.max(0));
    let finalized_at = Utc::now();

    if let (Some(platform_quantity), Some(resale_quantity)) = (platform_quantity, resale_quantity) {
        let coordinator = finalize_layer(
            db,
            &ctx.billing_request_id,
            BillingLayer::Platform,
            platform_quantity,
            model.clone(),
            platform.token_breakdown.as_ref(),
            Some(resale_quantity),
            finalized_at,
        )
        .await?;
        if let Some(row) = coordinator.as_ref() {
            finalized_rows.push(row.clone());
        }
        let coordinator = match coordinator {
            Some(row) => Some(row),
            None => {
                db.collection::<UsageMeterRow>(USAGE_METER)
                    .find_one(doc! {
                        "billing_request_id": &ctx.billing_request_id,
                        "layer": "platform",
                        "pending_resale_quantity": { "$exists": true, "$ne": Bson::Null },
                    })
                    .await?
            }
        };
        if let Some(coordinator) = coordinator
            && let Some(row) = materialize_pending_resale_intent(db, &coordinator).await?
        {
            finalized_rows.push(row);
        }
        return Ok(finalized_rows);
    }

    if let Some(platform_quantity) = platform_quantity
        && let Some(row) = finalize_layer(
            db,
            &ctx.billing_request_id,
            BillingLayer::Platform,
            platform_quantity,
            model.clone(),
            platform.token_breakdown.as_ref(),
            None,
            finalized_at,
        )
        .await?
    {
        finalized_rows.push(row);
    }

    if let Some(resale_quantity) = resale_quantity
        && let Some(row) = finalize_layer(
            db,
            &ctx.billing_request_id,
            BillingLayer::Resale,
            resale_quantity,
            model,
            None,
            None,
            finalized_at,
        )
        .await?
    {
        finalized_rows.push(row);
    }

    Ok(finalized_rows)
}

pub(super) async fn recover_pending_resale_intents(db: &mongodb::Database) -> AppResult<u64> {
    let coordinators: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {
            "layer": "platform",
            "pending_resale_quantity": { "$exists": true, "$ne": Bson::Null },
        })
        .limit(SETTLEMENT_INTENT_RECOVERY_BATCH_SIZE)
        .await?
        .try_collect()
        .await?;

    let mut recovered = 0;
    for coordinator in coordinators {
        if materialize_pending_resale_intent(db, &coordinator)
            .await?
            .is_some()
        {
            recovered += 1;
        }
    }

    Ok(recovered)
}

pub(super) async fn settle_persisted(
    db: &mongodb::Database,
    finalized_rows: Vec<UsageMeterRow>,
) -> AppResult<()> {
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
) -> AppResult<Option<String>> {
    let now = Utc::now();
    let transaction_id = transaction_id(&ctx.billing_request_id, layer, flush_seq);
    let wallet_id = reservation.map(|reservation| reservation.wallet_id.clone());
    let reserved_credits = reservation
        .map(|reservation| reservation.reserved_for(layer))
        .unwrap_or(0);
    let funding = reservation.map(|reservation| {
        let layer = reservation.layers.iter().find(|item| item.layer == layer);
        UsageFunding {
            credits_per_unit_micros: layer
                .map(|item| item.credits_per_unit_micros)
                .unwrap_or_default(),
            allowance_reservations: layer
                .map(|item| item.allowance_reservations.clone())
                .unwrap_or_default(),
            grant_reservations: layer
                .map(|item| item.grant_reservations.clone())
                .unwrap_or_default(),
            ..Default::default()
        }
    });
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
        token_breakdown: None,
        reserved_credits,
        funding,
        quantity: None,
        pending_resale_quantity: None,
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

    let inserted = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&row)
        .await
        .map(|_| true)
        .or_else(|error| {
            if is_duplicate_key_error(&error) {
                Ok(false)
            } else {
                Err(error)
            }
        })?;
    Ok(inserted.then_some(row.id))
}

#[allow(clippy::too_many_arguments)]
async fn finalize_layer(
    db: &mongodb::Database,
    billing_request_id: &str,
    layer: BillingLayer,
    quantity: i64,
    model: Option<String>,
    token_breakdown: Option<&crate::models::service_billing::TokenBreakdown>,
    pending_resale_quantity: Option<i64>,
    finalized_at: chrono::DateTime<Utc>,
) -> AppResult<Option<UsageMeterRow>> {
    let model_for_row = model.clone();
    let mut set = doc! {
        "status": "finalized",
        "quantity": quantity,
        "released": false,
        "model": model_for_row,
        "updated_at": bson::DateTime::from_chrono(finalized_at),
        "finalized_at": bson::DateTime::from_chrono(finalized_at),
    };
    if let Some(breakdown) = token_breakdown
        && let Ok(breakdown) = bson::to_bson(breakdown)
    {
        set.insert("token_breakdown", breakdown);
    }
    if let Some(resale_quantity) = pending_resale_quantity {
        set.insert("pending_resale_quantity", resale_quantity);
    }
    let collection = db.collection::<UsageMeterRow>(USAGE_METER);
    let claimed = collection
        .find_one_and_update(
            doc! {
                "billing_request_id": billing_request_id,
                "layer": layer.as_transaction_suffix(),
                "status": "forwarded",
            },
            doc! { "$set": set },
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

async fn materialize_pending_resale_intent(
    db: &mongodb::Database,
    coordinator: &UsageMeterRow,
) -> AppResult<Option<UsageMeterRow>> {
    let Some(resale_quantity) = coordinator.pending_resale_quantity else {
        return Ok(None);
    };
    let finalized_at = coordinator.finalized_at.unwrap_or(coordinator.updated_at);
    let materialized = finalize_layer(
        db,
        &coordinator.billing_request_id,
        BillingLayer::Resale,
        resale_quantity,
        coordinator.model.clone(),
        None,
        None,
        finalized_at,
    )
    .await?;
    clear_pending_resale_intent(
        db,
        &coordinator.id,
        &coordinator.billing_request_id,
        resale_quantity,
        coordinator.model.as_deref(),
    )
    .await?;
    Ok(materialized)
}

async fn clear_pending_resale_intent(
    db: &mongodb::Database,
    coordinator_id: &str,
    billing_request_id: &str,
    resale_quantity: i64,
    model: Option<&str>,
) -> AppResult<()> {
    let mut resale_filter = doc! {
        "billing_request_id": billing_request_id,
        "layer": "resale",
        "status": { "$in": ["finalized", "failed", "dead_letter"] },
        "quantity": resale_quantity,
    };
    resale_filter.insert(
        "model",
        model
            .map(|value| Bson::String(value.to_string()))
            .unwrap_or(Bson::Null),
    );
    let resale_materialized = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .count_documents(resale_filter)
        .await?
        > 0;
    if !resale_materialized {
        return Err(AppError::Internal(format!(
            "billing resale settlement intent was not materialized for {billing_request_id}"
        )));
    }

    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": coordinator_id,
                "pending_resale_quantity": resale_quantity,
            },
            doc! { "$unset": { "pending_resale_quantity": "" } },
        )
        .await?;
    Ok(())
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
    use crate::services::billing::route_inventory::BillingIngress;
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
            platform_billable: true,
            platform_metric: None,
            platform_pricing: None,
            resale_billable: true,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: Some("resale_tokens".to_string()),
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
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
            BillingIngress::LlmProvider,
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
                estimated_quantity: 1,
                credits_per_unit_micros: 5_000_000,
                reserved_credits: 5,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
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
    async fn persisted_settlement_intent_recovers_when_live_apply_never_starts() {
        let Some(db) = connect_test_database("billing_settle_pre_detachment_intent").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        let owner_id = "owner-pre-detachment-intent";
        insert_wallet(&db, owner_id, 10, 0).await;
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 5)
            .await
            .expect("reserve")
            .expect("reserved");

        let ctx = platform_context("billing-pre-detachment-intent", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: format!("wallet-{owner_id}"),
            total_reserved_credits: 5,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                estimated_quantity: 1,
                credits_per_unit_micros: 5_000_000,
                reserved_credits: 5,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
            }],
        };
        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");

        let persisted = super::persist_settlement_intent(
            &db,
            &metered,
            PlatformUsage::single_request(1),
            None,
            Some("model-before-detach".to_string()),
        )
        .await
        .expect("persist settlement intent");
        assert_eq!(persisted.len(), 1);

        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-pre-detachment-intent" })
            .await
            .expect("find persisted row")
            .expect("persisted row exists");
        assert_eq!(row.status, UsageStatus::Finalized);
        assert_eq!(row.quantity, Some(1));
        assert_eq!(row.model.as_deref(), Some("model-before-detach"));
        assert!(!row.released);

        let recovered = crate::services::billing::reservation::recover_retryable_settlements_at(
            &db,
            row.finalized_at.expect("finalized deadline") + chrono::Duration::seconds(31),
        )
        .await
        .expect("recover persisted settlement intent");
        assert_eq!(recovered.recovered, 1);

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, 5);
    }

    #[tokio::test]
    async fn pending_resale_intent_recovers_a_crash_between_layer_writes() {
        let Some(db) = connect_test_database("billing_settle_multi_layer_intent").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        let billing = ServiceBilling {
            platform_billable: true,
            platform_metric: None,
            platform_pricing: None,
            resale_billable: true,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: Some("resale_tokens".to_string()),
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            "billing-multi-layer-intent".to_string(),
            "owner-multi-layer-intent".to_string(),
            "actor-1".to_string(),
            None,
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
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        let finalized_at = Utc::now();

        // Simulate process loss after the atomic coordinator write and before
        // the resale row is materialized.
        let platform = super::finalize_layer(
            &db,
            &ctx.billing_request_id,
            BillingLayer::Platform,
            42,
            Some("model-before-detach".to_string()),
            None,
            Some(17),
            finalized_at,
        )
        .await
        .expect("persist complete multi-layer intent")
        .expect("platform coordinator claimed");
        assert_eq!(platform.quantity, Some(42));
        assert_eq!(platform.pending_resale_quantity, Some(17));

        let collection =
            db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        let resale_before = collection
            .find_one(doc! {
                "billing_request_id": &ctx.billing_request_id,
                "layer": "resale",
            })
            .await
            .expect("find resale row")
            .expect("resale row exists");
        assert_eq!(resale_before.quantity, None);

        let recovered = crate::services::billing::reservation::recover_retryable_settlements_at(
            &db,
            finalized_at + chrono::Duration::seconds(31),
        )
        .await
        .expect("recover multi-layer settlement intent");
        assert_eq!(recovered.recovered, 2);

        let rows: Vec<UsageMeterRow> = collection
            .find(doc! { "billing_request_id": &ctx.billing_request_id })
            .await
            .expect("find recovered rows")
            .try_collect()
            .await
            .expect("collect recovered rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.released));
        assert!(rows.iter().any(|row| {
            row.layer == BillingLayer::Platform
                && row.quantity == Some(42)
                && row.pending_resale_quantity.is_none()
        }));
        assert!(rows.iter().any(|row| {
            row.layer == BillingLayer::Resale
                && row.quantity == Some(17)
                && row.model.as_deref() == Some("model-before-detach")
        }));
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
                estimated_quantity: 1,
                credits_per_unit_micros: 5_000_000,
                reserved_credits: 5,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
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
                estimated_quantity: 1,
                credits_per_unit_micros: 4_000_000,
                reserved_credits: 4,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
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
                estimated_quantity: 1,
                credits_per_unit_micros: 5_000_000,
                reserved_credits: 5,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
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

    /// A settlement first-apply appends exactly one tamper-evident ledger
    /// entry; replays append nothing, and the resulting chain verifies.
    #[tokio::test]
    async fn settle_first_apply_appends_exactly_one_ledger_entry() {
        let Some(db) = connect_test_database("billing_ledger_settle_hook").await else {
            return;
        };
        crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        create_usage_transaction_index(&db).await;
        insert_rate(&db, "platform_requests", 5).await;
        let owner_id = "owner-ledger-hook";
        insert_wallet(&db, owner_id, 10, 0).await;
        crate::services::billing::reservation::try_reserve_prepaid(&db, owner_id, 5)
            .await
            .expect("reserve")
            .expect("reserved");

        let ctx = platform_context("billing-ledger-hook", owner_id);
        let reservation = BillingReservation {
            owner_id: owner_id.to_string(),
            wallet_id: format!("wallet-{owner_id}"),
            total_reserved_credits: 5,
            layers: vec![crate::services::billing::reservation::LayerReservation {
                layer: BillingLayer::Platform,
                estimated_quantity: 1,
                credits_per_unit_micros: 5_000_000,
                reserved_credits: 5,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
            }],
        };
        let metered = open(&db, &ctx, Some(&reservation)).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");
        settle(
            &db,
            &metered,
            crate::models::service_billing::PlatformUsage::single_request(64),
            None,
            None,
        )
        .await
        .expect("settle");

        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-ledger-hook" })
            .await
            .expect("find row")
            .expect("row exists");
        assert!(row.released);

        // A replay of the settlement claim must not append a second entry.
        crate::services::billing::reservation::claim_released_and_settle(&db, &row)
            .await
            .expect("settle replay");

        // The append is spawned off the settlement path; wait for it.
        let ledger = db.collection::<crate::models::billing_ledger::BillingLedgerEntry>(
            crate::models::billing_ledger::COLLECTION_NAME,
        );
        let mut entries: Vec<crate::models::billing_ledger::BillingLedgerEntry> = Vec::new();
        for _ in 0..100 {
            entries = ledger
                .find(doc! { "owner_id": owner_id })
                .await
                .expect("find ledger entries")
                .try_collect()
                .await
                .expect("collect ledger entries");
            if !entries.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(entries.len(), 1, "one charge, one ledger entry");
        let entry = &entries[0];
        assert_eq!(
            entry.event_type,
            crate::models::billing_ledger::BillingLedgerEventType::UsageSettled
        );
        assert_eq!(entry.reference_id, row.id);
        assert_eq!(entry.quantity, Some(1));
        assert_eq!(entry.amount_credits, Some(5));

        // Free metered traffic (no wallet) settles without a ledger entry.
        let free_ctx = platform_context("billing-ledger-free", "owner-ledger-free");
        let free_metered = open(&db, &free_ctx, None).await.expect("open free");
        mark_forwarded(&db, &free_metered)
            .await
            .expect("mark free forwarded");
        settle(
            &db,
            &free_metered,
            crate::models::service_billing::PlatformUsage::single_request(64),
            None,
            None,
        )
        .await
        .expect("settle free");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let free_entries = ledger
            .count_documents(doc! { "owner_id": "owner-ledger-free" })
            .await
            .expect("count free entries");
        assert_eq!(free_entries, 0, "free traffic moves no money");

        let report = crate::services::billing::ledger::verify_chain(
            &db,
            &crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
            None,
            None,
            None,
        )
        .await
        .expect("verify ledger");
        assert_eq!(
            report.status,
            crate::services::billing::ledger::BillingLedgerStatus::Ok
        );
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
                estimated_quantity: 1,
                credits_per_unit_micros: 5_000_000,
                reserved_credits: 5,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
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
                pending_topup_expiry_credits: 0,
                has_payment_instrument: false,
                overdraft_cap_credits,
                suspended: false,
                collection_state: CollectionState::Good,
                topup_expiry_checked_at: None,
                active_topup_expiry: None,
                balance_synced_at: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert wallet");
    }

    /// The provider-reported token breakdown persists onto the finalized
    /// platform row and never onto resale rows.
    #[tokio::test]
    async fn settle_persists_token_breakdown_on_platform_row() {
        let Some(db) = connect_test_database("billing_token_breakdown").await else {
            return;
        };
        create_usage_transaction_index(&db).await;
        let ctx = platform_context("billing-breakdown", "owner-breakdown");
        let metered = open(&db, &ctx, None).await.expect("open");
        mark_forwarded(&db, &metered).await.expect("mark forwarded");

        let breakdown = crate::models::service_billing::TokenBreakdown {
            prompt_tokens: 120,
            completion_tokens: 40,
            cached_tokens: 100,
            cache_creation_tokens: 30,
        };
        settle(
            &db,
            &metered,
            PlatformUsage::llm_completion(640, 160).with_token_breakdown(Some(breakdown)),
            None,
            Some("test-model".to_string()),
        )
        .await
        .expect("settle");

        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_request_id": "billing-breakdown" })
            .await
            .expect("find row")
            .expect("row exists");
        assert_eq!(row.token_breakdown, Some(breakdown));

        // An empty breakdown is dropped instead of stored as zeros.
        let empty = PlatformUsage::llm_completion(64, 1).with_token_breakdown(Some(
            crate::models::service_billing::TokenBreakdown::default(),
        ));
        assert!(empty.token_breakdown.is_none());
    }

    fn platform_context(request_id: &str, owner_id: &str) -> BillingRouteContext {
        BillingRouteContext::new(
            BillingIngress::Proxy,
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
