use super::*;
use crate::models::{
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    service_billing::{LanePriceComponent, LanePricing, ManagedExecutionContract, ServiceBilling},
};
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
};
use async_trait::async_trait;
use std::sync::Arc;

struct FakeLago;
#[async_trait]
impl LagoApi for FakeLago {
    async fn tax_config(
        &self,
        _: &str,
        _: &str,
    ) -> crate::errors::AppResult<crate::services::billing::lago_client::LagoTaxConfig> {
        Ok(crate::services::billing::taxes::zero_config())
    }
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        Ok(owner.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, customer: &str, _plan: &str) -> AppResult<String> {
        Ok(customer.into())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        Ok(events
            .iter()
            .map(|event| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: serde_json::json!({}),
        })
    }
    async fn wallet_balance(&self, _customer: &str) -> AppResult<i64> {
        Ok(0)
    }
    async fn entitlements(&self, _subscription: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "*".into(),
            raw: serde_json::json!({}),
        }])
    }
}

fn config() -> ServiceBilling {
    ServiceBilling {
        platform_billable: true,
        managed_execution: Some(ManagedExecutionContract {
            model: "test-fixed-model".into(),
            max_input_tokens: 100,
            max_output_tokens: 20,
            max_request_bytes: 1000,
            max_response_bytes: 4000,
            tariff_version: "test-v1".into(),
            tariff_source: "https://example.test/tariff".into(),
            tariff_verified_at: Utc::now() - Duration::minutes(1),
        }),
        platform_key_pricing: Some(LanePricing {
            metric: BillingMetric::InputTokens,
            credits_per_unit: "0.000000000002".into(),
            lago_metric_code: "test_input".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![
                LanePriceComponent {
                    metric: BillingMetric::OutputTokens,
                    credits_per_unit: "0.000000000003".into(),
                    lago_metric_code: "test_output".into(),
                    sync_status: PricingSyncStatus::Synced,
                    sync_error: None,
                },
                LanePriceComponent {
                    metric: BillingMetric::CacheReadTokens,
                    credits_per_unit: "0.000000000001".into(),
                    lago_metric_code: "test_cache".into(),
                    sync_status: PricingSyncStatus::Synced,
                    sync_error: None,
                },
            ],
            operations: vec![],
        }),
        ..Default::default()
    }
}

fn context(owner: &str, billing: &ServiceBilling) -> BillingRouteContext {
    BillingRouteContext::new(
        BillingIngress::Proxy,
        uuid::Uuid::new_v4().to_string(),
        owner.into(),
        owner.into(),
        None,
        Some("connection".into()),
        Some("catalog".into()),
        Some("telnyx-test".into()),
        NodeIntent::Direct,
        "bearer".into(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::InputTokens,
        Some(billing),
        false,
    )
    .with_request_body(Some(b"{}"))
}

async fn fixture(label: &str) -> (Database, BillingService, BillingRouteContext) {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("billing_migrations")
        .update_one(doc! {"_id":"exact-v2"}, doc! {"$set":{"completed_at":now}})
        .upsert(true)
        .await
        .unwrap();
    let wallet:BillingWallet=bson::from_document(doc!{
        "_id":uuid::Uuid::new_v4().to_string(),"owner_id":&owner,"lago_customer_id":"customer","lago_subscription_id":"subscription","lago_wallet_id":"wallet",
        "plan_kind":"prepaid","balance_credits":Credits::from_pico(10000).unwrap(),"reserved_credits":Credits::ZERO,"collection_state":"good",
        "balance_synced_at":now,"created_at":now,"updated_at":now,
    }).unwrap();
    db.collection::<BillingWallet>(WALLETS)
        .insert_one(wallet)
        .await
        .unwrap();
    for (code, pico) in [("test_input", 2), ("test_output", 3), ("test_cache", 1)] {
        db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME,
        )
        .insert_one(crate::models::billing_rate_cache::BillingRateCache {
            id: format!("{code}:*"),
            lago_metric_code: code.into(),
            model: None,
            credits_per_unit_micros: 0,
            credits_per_unit_pico: Some(pico),
            synced_at: Utc::now(),
            retired_at: None,
        })
        .await
        .unwrap();
    }
    let mut app = crate::test_utils::test_app_config();
    app.billing_enabled = true;
    let billing = BillingService::new_with_lago(db.clone(), Arc::new(app), Arc::new(FakeLago));
    (db, billing, context(&owner, &config()))
}

async fn open(
    db: &Database,
    billing: &BillingService,
    ctx: BillingRouteContext,
) -> ManagedExecutionHandle {
    prepare(db, billing, ctx, &"a".repeat(64), &"b".repeat(64))
        .await
        .unwrap()
}

async fn wallet(db: &Database) -> BillingWallet {
    db.collection::<BillingWallet>(WALLETS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}

async fn assert_settled_wallet(db: &Database) {
    let wallet = wallet(db).await;
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    // Usage debits reduce availability before Lago refreshes the provider balance.
    assert_eq!(wallet.balance_credits, Credits::from_pico(10000).unwrap());
    assert_eq!(wallet.pending_lago_debits, Credits::from_pico(105).unwrap());
    assert_eq!(
        wallet.available_credits().unwrap(),
        Credits::from_pico(9895).unwrap()
    );
    assert_eq!(
        db.collection::<bson::Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .count_documents(doc! {"movement":"usage_settled"})
            .await
            .unwrap(),
        3
    );
}

fn usage() -> PlatformUsage {
    PlatformUsage {
        requests: 1,
        bytes: 100,
        input_tokens: 40,
        output_tokens: 5,
        cache_read_tokens: 10,
        tokens: 55,
        ..Default::default()
    }
}

#[test]
fn managed_tariff_requires_every_component_and_master_context() {
    let billing = config();
    let ctx = context("actor", &billing);
    assert!(validate_route(&ctx).is_ok());
    let mut incomplete = billing.clone();
    incomplete.platform_key_pricing.as_mut().unwrap().components[0].sync_status =
        PricingSyncStatus::Pending;
    assert!(validate_route(&context("actor", &incomplete)).is_err());
    let mut byok = ctx.clone();
    byok.credential_class = CredentialClass::UserOwned;
    assert!(validate_route(&byok).is_err());
    let mut alternate = ctx;
    alternate.ingress = BillingIngress::LlmProvider;
    assert!(validate_route(&alternate).is_err());
}

#[tokio::test]
async fn managed_atomic_admission_rolls_back_all_holds_when_meter_insert_fails() {
    let (db, billing, ctx) = fixture("managed_atomic_rollback").await;
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("credit_grants").insert_one(doc!{
        "_id":uuid::Uuid::new_v4().to_string(),"batch_id":"batch","recipient_user_id":&ctx.actor_user_id,"target_kind":"selected_users",
        "amount_credits":0_i64,"amount":Credits::from_pico(50).unwrap(),"remaining":Credits::from_pico(50).unwrap(),"reserved":Credits::ZERO,
        "scope":{"all_services":true,"service_ids":[],"service_slugs":[]},"status":"active","issued_ledgered_at":now,
        "granted_by":"admin","terminal_amount":Credits::ZERO,"created_at":now,"updated_at":now,
    }).await.unwrap();
    db.collection::<bson::Document>(METERS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"billing_request_id":1})
                .options(
                    IndexOptions::builder()
                        .name("injected_meter_failure".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    assert!(
        prepare(&db, &billing, ctx, &"a".repeat(64), &"b".repeat(64))
            .await
            .is_err()
    );
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    let grant = db
        .collection::<bson::Document>("credit_grants")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.get_decimal128("reserved").unwrap().to_string(), "0");
    assert_eq!(
        db.collection::<UsageMeterRow>(METERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<ManagedExecution>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .state,
        State::Released
    );
}

#[tokio::test]
async fn managed_usage_settles_once_at_admission_prices_after_cache_changes() {
    let (db, billing, ctx) = fixture("managed_rate_snapshot").await;
    let handle = open(&db, &billing, ctx).await;
    assert_eq!(
        wallet(&db).await.reserved_credits,
        Credits::from_pico(360).unwrap()
    );
    admit_dispatch(&db, &handle).await.unwrap();
    assert!(admit_dispatch(&db, &handle).await.is_err());
    db.collection::<bson::Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"credits_per_unit_pico":1000_i64}})
        .await
        .unwrap();
    complete(&db, &billing, &handle, usage(), Some("provider-request-1"))
        .await
        .unwrap();
    complete(&db, &billing, &handle, usage(), Some("provider-request-1"))
        .await
        .unwrap();
    assert_settled_wallet(&db).await;
    assert_eq!(
        db.collection::<ManagedExecution>(COLLECTION_NAME)
            .find_one(doc! {"_id":handle.execution_id()})
            .await
            .unwrap()
            .unwrap()
            .state,
        State::Settled
    );
}

#[tokio::test]
async fn managed_dispatch_and_cancellation_cannot_both_win() {
    let (db, billing, ctx) = fixture("managed_cancel_race").await;
    let handle = open(&db, &billing, ctx).await;
    let (dispatched, cancelled) = tokio::join!(
        admit_dispatch(&db, &handle),
        cancel_unforwarded(&db, &handle)
    );
    cancelled.unwrap();
    let row = db
        .collection::<ManagedExecution>(COLLECTION_NAME)
        .find_one(doc! {"_id":handle.execution_id()})
        .await
        .unwrap()
        .unwrap();
    if dispatched.is_ok() {
        assert_eq!(row.state, State::DispatchAdmitted);
        assert_eq!(
            wallet(&db).await.reserved_credits,
            Credits::from_pico(360).unwrap()
        );
    } else {
        assert_eq!(row.state, State::Released);
        assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    }
}

#[tokio::test]
async fn managed_proven_unsent_refusal_releases_admitted_holds_and_survives_cleanup_interruption() {
    let (db, billing, ctx) = fixture("managed_final_refusal_recovery").await;
    let handle = open(&db, &billing, ctx).await;
    admit_dispatch(&db, &handle).await.unwrap();
    cancel_unforwarded(&db, &handle).await.unwrap();
    assert_eq!(
        wallet(&db).await.reserved_credits,
        Credits::from_pico(360).unwrap()
    );
    // Simulate an interrupted/failed funding cleanup. The independently durable
    // proof must allow ordinary reconciliation to complete it later.
    db.collection::<BillingWallet>(WALLETS)
        .update_one(doc! {}, doc! {"$set":{"reserved_credits":Credits::ZERO}})
        .await
        .unwrap();
    assert!(reject_before_send(&db, &handle).await.is_err());
    let row = db
        .collection::<ManagedExecution>(COLLECTION_NAME)
        .find_one(doc! {"_id":handle.execution_id()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, State::Cancelling);
    assert_eq!(row.pending_reason.as_deref(), Some(REJECTED_BEFORE_SEND));
    assert_eq!(
        db.collection::<UsageMeterRow>(METERS)
            .count_documents(doc! {"forwarded":true,"released":false})
            .await
            .unwrap(),
        3
    );
    db.collection::<BillingWallet>(WALLETS)
        .update_one(
            doc! {},
            doc! {"$set":{"reserved_credits":Credits::from_pico(360).unwrap()}},
        )
        .await
        .unwrap();
    assert_eq!(reconcile(&db).await.unwrap(), 1);
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    assert_eq!(wallet(&db).await.pending_lago_debits, Credits::ZERO);
    assert_eq!(
        db.collection::<UsageMeterRow>(METERS)
            .count_documents(doc! {"forwarded":false,"released":true,"status":"failed"})
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        db.collection::<ManagedExecution>(COLLECTION_NAME)
            .find_one(doc! {"_id":handle.execution_id()})
            .await
            .unwrap()
            .unwrap()
            .state,
        State::Released
    );
}

#[tokio::test]
async fn managed_unknown_outcome_keeps_holds_and_recovery_replays_persisted_evidence() {
    let (db, billing, ctx) = fixture("managed_pending_recovery").await;
    let handle = open(&db, &billing, ctx).await;
    admit_dispatch(&db, &handle).await.unwrap();
    mark_pending(&db, &handle, "transport_error").await.unwrap();
    billing::reservation::abandon_stale_unforwarded(&db, Utc::now() + Duration::days(1))
        .await
        .unwrap();
    cancel_unforwarded(&db, &handle).await.unwrap();
    reconcile(&db).await.unwrap();
    assert_eq!(
        wallet(&db).await.reserved_credits,
        Credits::from_pico(360).unwrap()
    );
    // Crash after the validated evidence write, before the settlement call.
    db.collection::<ManagedExecution>(COLLECTION_NAME).update_one(doc!{"_id":handle.execution_id()},doc!{"$set":{
        "state":"evidence_ready","usage":bson::to_bson(&usage()).unwrap(),"next_reconcile_at":bson::DateTime::now(),
    }}).await.unwrap();
    assert_eq!(reconcile(&db).await.unwrap(), 1);
    assert_settled_wallet(&db).await;
    reconcile(&db).await.unwrap();
    assert_settled_wallet(&db).await;
}

#[tokio::test]
async fn managed_generic_routes_disabled_billing_and_excluded_rollout_fail_closed() {
    let (db, billing, ctx) = fixture("managed_strict_admission").await;
    assert!(billing.open(&ctx).await.is_err());
    assert!(billing.open_required(&ctx).await.is_err());
    let mut app = crate::test_utils::test_app_config();
    app.billing_enabled = false;
    let disabled = BillingService::new(db.clone(), Arc::new(app));
    assert!(
        prepare(
            &db,
            &disabled,
            ctx.clone(),
            &"a".repeat(64),
            &"b".repeat(64)
        )
        .await
        .is_err()
    );
    let mut fresh = ctx.clone();
    fresh.billing_request_id = uuid::Uuid::new_v4().to_string();
    crate::services::feature_flag_service::set_platform_override(
        &db,
        crate::services::feature_flag_service::BILLING_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        false,
        &ctx.actor_user_id,
    )
    .await
    .unwrap();
    assert!(
        prepare(&db, &billing, fresh, &"a".repeat(64), &"b".repeat(64))
            .await
            .is_err()
    );
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        db.collection::<UsageMeterRow>(METERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn managed_status_visibility_follows_actor_and_verified_personal_payer() {
    let db = crate::test_utils::connect_transaction_test_database("managed_visibility").await;
    let billing = config();
    let personal = new_record(
        &context("person", &billing),
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .unwrap();
    let mut service_account_ctx = context("person", &billing);
    service_account_ctx.actor_user_id = "service-account".into();
    let service_account =
        new_record(&service_account_ctx, &"a".repeat(64), &"b".repeat(64)).unwrap();
    let other = new_record(
        &context("other", &billing),
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .unwrap();
    db.collection::<ManagedExecution>(COLLECTION_NAME)
        .insert_many([personal.clone(), service_account.clone(), other.clone()])
        .await
        .unwrap();
    let personal_rows = list_visible(&db, "person", Some("person")).await.unwrap();
    assert_eq!(personal_rows.len(), 2);
    assert!(
        personal_rows
            .iter()
            .all(|row| row.billing_owner_id == "person")
    );
    let service_account_rows = list_visible(&db, "service-account", None).await.unwrap();
    assert_eq!(service_account_rows.len(), 1);
    assert_eq!(service_account_rows[0].id, service_account.id);
    assert!(
        get_visible(&db, "person", Some("person"), &service_account.id)
            .await
            .is_ok()
    );
    assert!(matches!(
        get_visible(&db, "service-account", None, &personal.id).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        get_visible(&db, "person", Some("person"), &other.id).await,
        Err(AppError::NotFound(_))
    ));
}
