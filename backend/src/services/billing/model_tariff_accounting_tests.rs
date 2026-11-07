//! Same production funding/settlement path, new immutable per-code quantities.
use super::super::{model_tariffs, model_usage};
use super::*;
use crate::models::{
    platform_model_tariff::{CacheTtl, ModelTariff},
    service_billing::BillingMetric,
    usage_meter::CredentialClass,
};

const OWNER: &str = "00000000-0000-4000-8000-000000000001";

async fn fixture(
    name: &str,
    benefits: bool,
) -> (
    mongodb::Database,
    ModelTariff,
    BillingRouteContext,
    meter::MeteredProxyContext,
) {
    fixture_with_free_component(name, benefits, false).await
}

async fn fixture_with_free_component(
    name: &str,
    benefits: bool,
    free_write: bool,
) -> (
    mongodb::Database,
    ModelTariff,
    BillingRouteContext,
    meter::MeteredProxyContext,
) {
    let db = database(name).await;
    // Admin usage uses the production covering indexes even before the first fold.
    usage_rollup::ensure_indexes(&db).await.unwrap();
    let mut wallet = old_wallet(10_000_000);
    wallet.insert("owner_id", OWNER);
    wallet.insert("lago_customer_id", OWNER);
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            OWNER,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    db.collection::<Document>("billing_wallet")
        .insert_one(wallet)
        .await
        .unwrap();
    if benefits {
        let mut grant = old_grant(9);
        grant.insert("recipient_user_id", OWNER);
        db.collection::<Document>("credit_grants")
            .insert_one(grant)
            .await
            .unwrap();
    }
    exact_migration::run(&db).await.unwrap();
    let (service, mut tariff, mut rates) = model_tariffs::tests::fixture();
    if free_write {
        let c = tariff
            .components
            .iter_mut()
            .find(|c| c.key == "write12")
            .unwrap();
        c.credits_per_unit = "0".into();
        c.explicit_free = true;
        rates.remove(&c.lago_metric_code);
        model_tariffs::validate(&mut tariff, &service.slug).unwrap();
    }
    db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&service)
        .await
        .unwrap();
    db.collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .insert_one(&tariff)
        .await
        .unwrap();
    db.collection::<crate::models::billing_rate_cache::BillingRateCache>("billing_rate_cache")
        .insert_many(rates.values())
        .await
        .unwrap();
    if benefits {
        let now = Utc::now();
        db.collection::<crate::models::usage_allowance::UsageAllowance>("usage_allowances")
            .insert_one(crate::models::usage_allowance::UsageAllowance {
                id: "allowance".into(),
                bundle_id: None,
                service_id: service.id.clone(),
                service_slug: service.slug.clone(),
                metric: BillingMetric::CacheWriteTokens,
                quantity: 10,
                recurrence: crate::models::usage_allowance::AllowanceRecurrence::Daily,
                target_kind: crate::models::billing_target::BillingTargetKind::AllUsers,
                target_user_ids: vec![],
                target_org_ids: vec![],
                target_group_ids: vec![],
                is_active: true,
                created_by: "admin".into(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }
    let selected = model_tariffs::select(
        &service,
        &tariff.model_id,
        CredentialClass::NyxidManagedMaster,
        &tariff.variant,
        std::slice::from_ref(&tariff),
        &rates,
        Utc::now(),
        900,
    )
    .unwrap();
    let ctx = BillingRouteContext::new(
        BillingIngress::LlmGateway,
        uuid::Uuid::new_v4().to_string(),
        OWNER.into(),
        OWNER.into(),
        None,
        None,
        Some(service.id),
        Some(service.slug),
        NodeIntent::Direct,
        "bearer".into(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::Tokens,
        service.billing.as_ref(),
        false,
    )
    .with_model_tariff(selected)
    .unwrap();
    let reservation = reservation::gate_and_reserve(&db, Some(&Entitled), &ctx, false, 900)
        .await
        .unwrap()
        .unwrap();
    let metered = admitted_meter(&db, &ctx, &reservation).await;
    meter::mark_forwarded(&db, &metered).await.unwrap();
    (db, tariff, ctx, metered)
}
fn evidence() -> model_usage::Evidence {
    model_usage::Evidence {
        input: Some(50),
        output: Some(10),
        cache_read: Some(10),
        cache_write: Some(20),
        writes: std::collections::BTreeMap::from([
            (CacheTtl::ThirtyMinutes, 5),
            (CacheTtl::TwelveHours, 15),
        ]),
        ..Default::default()
    }
}
#[tokio::test]
async fn model_tariff_shared_allowance_then_exact_grant_wallet_and_ledger_once() {
    Box::pin(async {
        let (db,t,_ctx,metered)=fixture("model_tariff_funding",true).await;
        let usage=model_usage::charge_vector(&t,&evidence()).unwrap();
        meter::settle(&db,&metered,usage.clone(),None,None).await.unwrap();
        meter::settle(&db,&metered,usage,None,None).await.unwrap();
        let rows:Vec<UsageMeterRow>=db.collection("usage_meter").find(doc!{}).await.unwrap().try_collect().await.unwrap();assert_eq!(rows.len(),5);
        let mut allowance=Credits::ZERO;let mut grant=Credits::ZERO;let mut wallet=Credits::ZERO;
        for row in &rows {let f=row.funding.as_ref().unwrap();assert!(f.settled);allowance=allowance.checked_add(f.allowance_funded.unwrap()).unwrap();grant=grant.checked_add(f.grant_funded.unwrap()).unwrap();wallet=wallet.checked_add(f.wallet_funded.unwrap()).unwrap();assert_eq!(row.model.as_deref(),Some(t.public_model.as_str()));assert!(row.reported_model.is_none());}
        assert_eq!(allowance,Credits::from_pico(10_000_010).unwrap());assert_eq!(grant,Credits::from_micros(9));assert_eq!(wallet,Credits::from_pico(41_000_050).unwrap());
        assert_eq!(rows.iter().filter(|r|r.token_breakdown.is_some()).count(),1);
        assert_eq!(rows.iter().filter(|r|r.metric==BillingMetric::CacheWriteTokens).map(|r|r.quantity.unwrap()).sum::<i64>(),20);
        let count=db.collection::<Document>("billing_ledger").count_documents(doc!{}).await.unwrap();
        meter::recover_pending_resale_intents(&db).await.unwrap();assert_eq!(db.collection::<Document>("billing_ledger").count_documents(doc!{}).await.unwrap(),count);
        assert!(ledger::verify_chain(&db,&ledger::TEST_BILLING_LEDGER_HMAC_KEY,None,None,None).await.unwrap().break_info.is_none());assert_accounts(&db).await;
        assert_usage_readers(&db,&t).await;
        // Existing metric/code/model dimensions keep TTL rows distinct. New
        // tariff units require no financial backfill or additional measures.
        usage_rollup::ensure_indexes(&db).await.unwrap();let end=usage_rollup::hour(Utc::now());
        db.collection::<Document>("usage_meter").update_many(doc!{},doc!{"$set":{"created_at":bson::DateTime::from_chrono(end-chrono::Duration::hours(2)),"lago_acked":true}}).await.unwrap();
        while usage_rollup::fold_once(&db,end).await.unwrap()>0{}
        for collection in [crate::models::usage_rollup_hourly::COLLECTION_NAME,crate::models::usage_rollup_daily::COLLECTION_NAME] {let rows:Vec<Document>=db.collection(collection).find(doc!{}).await.unwrap().try_collect().await.unwrap();assert_eq!(rows.len(),5);assert!(rows.iter().all(|r|r.get_str("model")==Ok(t.public_model.as_str())));}
        assert_usage_readers(&db,&t).await;
        db.drop().await.unwrap();
    }).await
}
#[tokio::test]
async fn model_tariff_atomic_intent_recovers_frozen_rate_and_changed_evidence_conflicts() {
    let (db, t, ctx, metered) = fixture("model_tariff_recovery", false).await;
    let usage = model_usage::charge_vector(&t, &evidence()).unwrap();
    // Crash at the first durable write, before any component materialization.
    // Evidence and recoverable usage must already be present together.
    let coordinator = meter::save_component_intent(&db, &ctx, &usage, None, Some(&t.public_model))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(coordinator.pending_platform_usage.as_ref(), Some(&usage));
    assert!(coordinator.model_tariff.unwrap().evidence_digest.is_some());
    let rows: Vec<UsageMeterRow> = db
        .collection("usage_meter")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|r| r.quantity.is_none() && !r.released));
    // Admission rate remains authoritative even if the external cache changes.
    db.collection::<Document>("billing_rate_cache")
        .update_many(doc! {}, doc! {"$set":{"credits_per_unit_pico":99_i64}})
        .await
        .unwrap();
    reservation::recover_retryable_settlements(&db)
        .await
        .unwrap();
    meter::settle(&db, &metered, usage, None, None)
        .await
        .unwrap();
    let rows: Vec<UsageMeterRow> = db
        .collection("usage_meter")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(rows.iter().all(|r| r.funding.as_ref().unwrap().settled));
    assert_eq!(
        rows.iter()
            .map(|r| r.funding.as_ref().unwrap().wallet_funded.unwrap())
            .try_fold(Credits::ZERO, Credits::checked_add)
            .unwrap(),
        Credits::from_pico(60_000_060).unwrap()
    );
    let mut changed = evidence();
    changed.output = Some(11);
    assert!(
        meter::settle(
            &db,
            &metered,
            model_usage::charge_vector(&t, &changed).unwrap(),
            None,
            None
        )
        .await
        .is_err()
    );
    assert_accounts(&db).await;
    db.drop().await.unwrap();
}
#[tokio::test]
async fn model_tariff_missing_usage_keeps_hold_and_invalid_vector_cannot_finalize() {
    let (db, t, ctx, metered) = fixture("model_tariff_uncertain", false).await;
    assert!(
        meter::settle(&db, &metered, Default::default(), None, None)
            .await
            .is_err()
    );
    let mut usage = model_usage::charge_vector(&t, &evidence()).unwrap();
    usage.model_charges.as_mut().unwrap().quantities.clear();
    assert!(
        meter::settle(&db, &metered, usage, None, None)
            .await
            .is_err()
    );
    reservation::abandon_stale_unforwarded(&db, Utc::now() + chrono::Duration::days(2))
        .await
        .unwrap();
    let rows: Vec<UsageMeterRow> = db
        .collection("usage_meter")
        .find(doc! {"billing_request_id":&ctx.billing_request_id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        rows.iter()
            .all(|r| r.forwarded && !r.released && r.quantity.is_none())
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn model_tariff_no_overdraft_and_retirement_fence_leave_no_partial_reservations() {
    let (db, _, mut ctx, _) = fixture("model_tariff_admission_fence", false).await;
    ctx.billing_request_id = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>("billing_wallet").update_many(doc!{},doc!{"$set":{"balance_credits":Credits::ZERO,"has_payment_instrument":true,"plan_kind":"postpaid","overdraft_cap_credits":Credits::from_whole(100)}}).await.unwrap();
    assert!(
        reservation::gate_and_reserve(&db, Some(&Entitled), &ctx, false, 900)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        5
    );
    db.collection::<Document>("billing_wallet")
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(10)}},
        )
        .await
        .unwrap();
    db.collection::<Document>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"status":"retired"}})
        .await
        .unwrap();
    assert!(
        reservation::gate_and_reserve(&db, Some(&Entitled), &ctx, false, 900)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        5
    );
    db.drop().await.unwrap();
}

#[derive(Default)]
struct ModelLago {
    events: std::sync::Mutex<std::collections::BTreeMap<String, LagoEvent>>,
    fail_next: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl LagoApi for ModelLago {
    async fn tax_config(&self, _: &str, _: &str) -> crate::errors::AppResult<LagoTaxConfig> {
        Ok(taxes::zero_config())
    }
    async fn ensure_customer(
        &self,
        owner: &OwnerProvisionInput,
    ) -> crate::errors::AppResult<String> {
        Ok(owner.external_customer_id.clone())
    }
    async fn ensure_subscription(
        &self,
        customer: &str,
        _: &str,
    ) -> crate::errors::AppResult<String> {
        Ok(customer.into())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        if self
            .fail_next
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(LagoError::retry("fixture outage"));
        }
        let mut events = self.events.lock().unwrap();
        if let Some(previous) = events.get(&event.transaction_id) {
            assert_eq!(previous, event);
            return Err(LagoError::duplicate("already recorded"));
        }
        events.insert(event.transaction_id.clone(), event.clone());
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        let mut acks = vec![];
        for event in events {
            acks.push(self.record_event(event).await?);
        }
        Ok(acks)
    }
    async fn current_usage(
        &self,
        customer: &str,
        subscription: &str,
    ) -> crate::errors::AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: serde_json::json!({"charges":self.events.lock().unwrap().values().map(|e|serde_json::json!({"code":e.code,"units":e.properties.quantity})).collect::<Vec<_>>()}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> crate::errors::AppResult<i64> {
        Err(crate::errors::AppError::BillingProviderUnavailable(
            "Balance refresh outside fixture".into(),
        ))
    }
    async fn entitlements(&self, _: &str) -> crate::errors::AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "*".into(),
            raw: serde_json::json!({}),
        }])
    }
}
#[tokio::test]
async fn model_tariff_lago_retry_emits_wallet_only_quantities_once_and_reconciles_without_drift() {
    Box::pin(async {
        let (db,t,_,metered)=fixture("model_tariff_lago",true).await;meter::settle(&db,&metered,model_usage::charge_vector(&t,&evidence()).unwrap(),None,None).await.unwrap();
        let lago=std::sync::Arc::new(ModelLago::default());lago.fail_next.store(true,std::sync::atomic::Ordering::SeqCst);
        let mut config=crate::test_utils::test_app_config();config.billing_enabled=true;let worker=reconcile::BillingReconciler::new(db.clone(),Some(lago.clone()),std::sync::Arc::new(config));
        let ledger_before=db.collection::<Document>("billing_ledger").count_documents(doc!{}).await.unwrap();
        db.collection::<Document>("usage_meter").update_many(doc!{},doc!{"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(60))}}).await.unwrap();
        let first=worker.run_once().await.unwrap();assert_eq!(first.retried,1);
        db.collection::<Document>("usage_meter").update_many(doc!{},doc!{"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(60))}}).await.unwrap();
        let second=worker.run_once().await.unwrap();assert_eq!(second.pushed,1);assert_eq!(second.drift_alerts,0);assert_eq!(worker.run_once().await.unwrap().pushed,0);
        let rows:Vec<UsageMeterRow>=db.collection("usage_meter").find(doc!{}).await.unwrap().try_collect().await.unwrap();
        {let events=lago.events.lock().unwrap();assert_eq!(events.len(),5);for row in &rows {assert!(row.lago_acked);let event=&events[&row.transaction_id];assert_eq!(event,LagoEvent::from_usage_row(row,Some("subscription".into())).as_ref().unwrap());assert_eq!(event.properties.model.as_deref(),Some(t.public_model.as_str()));}}
        assert_eq!(db.collection::<Document>("billing_ledger").count_documents(doc!{}).await.unwrap(),ledger_before);assert_accounts(&db).await;db.drop().await.unwrap();
    }).await
}

async fn assert_usage_readers(db: &mongodb::Database, t: &ModelTariff) {
    let state = crate::test_utils::test_app_state(db.clone());
    let usage = crate::handlers::billing::get_usage(
        axum::extract::State(state),
        crate::test_utils::test_auth_user(OWNER),
        axum::extract::Query(crate::handlers::billing::UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: None,
            bucket: None,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(usage.rows.len(), 5);
    // The billing view reports billed request units; this token-only tariff
    // has none. Admin usage below still counts the invocation exactly once.
    assert_eq!(usage.rows.iter().map(|r| r.requests).sum::<i64>(), 0);
    assert_eq!(usage.rows.iter().map(|r| r.quantity).sum::<i64>(), 60);
    assert_eq!(
        usage.totals.estimated_credits,
        Some(Credits::from_pico(60_000_060).unwrap())
    );
    assert_eq!(
        usage.totals.wallet_credits,
        Some(Credits::from_pico(41_000_050).unwrap())
    );
    assert_eq!(usage.totals.grant_credits, Some(Credits::from_micros(9)));
    assert_eq!(
        usage.totals.allowance_credits,
        Some(Credits::from_pico(10_000_010).unwrap())
    );
    assert!(
        usage
            .rows
            .iter()
            .all(|r| r.model.as_deref() == Some(t.public_model.as_str()))
    );
    let now = Utc::now();
    let admin = crate::services::admin_usage_service::get_usage(
        db,
        crate::services::admin_usage_service::AdminUsageQuery {
            from: Some((now - chrono::Duration::days(1)).to_rfc3339()),
            to: Some((now + chrono::Duration::minutes(1)).to_rfc3339()),
            ..Default::default()
        }
        .validate(now + chrono::Duration::minutes(1))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(admin.totals.requests, 1);
    assert_eq!(admin.totals.quantities["cache_write_tokens"], 20);
}

#[tokio::test]
async fn model_tariff_paid_admission_never_downgrades_when_billing_is_disabled() {
    let (db, _, mut ctx, _) = fixture("model_tariff_disabled", false).await;
    ctx.billing_request_id = uuid::Uuid::new_v4().to_string();
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = false;
    let billing = BillingService::new_with_lago(
        db.clone(),
        std::sync::Arc::new(config),
        std::sync::Arc::new(Entitled),
    );
    assert!(matches!(
        billing.open(&ctx).await,
        Err(crate::errors::AppError::BillingProviderUnavailable(_))
    ));
    assert_eq!(
        db.collection::<Document>("usage_meter")
            .count_documents(doc! {"billing_request_id":&ctx.billing_request_id})
            .await
            .unwrap(),
        0
    );
    ctx.model_tariff = None;
    ctx.platform_components.clear();
    ctx.platform_metric = BillingMetric::ReasoningTokens;
    assert!(matches!(
        billing.open(&ctx).await,
        Err(crate::errors::AppError::BillingNotConfigured(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn model_tariff_explicit_free_metering_never_consumes_benefits() {
    let (db, mut t, mut ctx, _) = fixture("model_tariff_free", true).await;
    ctx.billing_request_id = uuid::Uuid::new_v4().to_string();
    for c in &mut t.components {
        c.credits_per_unit = "0".into();
        c.explicit_free = true;
    }
    model_tariffs::validate(&mut t, "models").unwrap();
    let service = db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! {"_id":&t.service_id})
        .await
        .unwrap()
        .unwrap();
    db.collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .replace_one(doc! {"_id":&t.id}, &t)
        .await
        .unwrap();
    let selected = model_tariffs::select(
        &service,
        &t.model_id,
        ctx.credential_class,
        &t.variant,
        std::slice::from_ref(&t),
        &Default::default(),
        Utc::now(),
        900,
    )
    .unwrap();
    ctx = ctx.with_model_tariff(selected).unwrap();
    let before: Vec<Document> = db
        .collection("usage_allowance_periods")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = false;
    let billing = BillingService::new_with_lago(
        db.clone(),
        std::sync::Arc::new(config),
        std::sync::Arc::new(Entitled),
    );
    let metered = billing.open(&ctx).await.unwrap();
    meter::mark_forwarded(&db, &metered).await.unwrap();
    meter::settle(
        &db,
        &metered,
        model_usage::charge_vector(&t, &evidence()).unwrap(),
        None,
        None,
    )
    .await
    .unwrap();
    let rows: Vec<UsageMeterRow> = db
        .collection("usage_meter")
        .find(doc! {"billing_request_id":&ctx.billing_request_id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter()
            .all(|r| r.wallet_id.is_none() && r.funding.is_none())
    );
    let after: Vec<Document> = db
        .collection("usage_allowance_periods")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(before, after);
    // A free snapshot cannot create new rows after its tariff is retired.
    ctx.billing_request_id = uuid::Uuid::new_v4().to_string();
    db.collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .update_one(doc! {"_id":&t.id}, doc! {"$set":{"status":"retired"}})
        .await
        .unwrap();
    assert!(billing.open(&ctx).await.is_err());
    assert_eq!(
        db.collection::<Document>("usage_meter")
            .count_documents(doc! {"billing_request_id":&ctx.billing_request_id})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn model_tariff_free_component_preserves_allowance_and_emits_no_lago_event() {
    Box::pin(async {
        let (db, t, _, metered) = fixture_with_free_component("model_tariff_mixed_free", true, true).await;
        meter::settle(&db, &metered, model_usage::charge_vector(&t, &evidence()).unwrap(), None, None).await.unwrap();
        let free_code = &t.components.iter().find(|c|c.explicit_free).unwrap().lago_metric_code;
        let rows: Vec<UsageMeterRow> = db.collection("usage_meter").find(doc! {}).await.unwrap().try_collect().await.unwrap();
        let free = rows.iter().find(|r|&r.lago_metric_code == free_code).unwrap();
        let funding = free.funding.as_ref().unwrap();
        assert!(funding.settled);
        assert_eq!(funding.total_charge, Some(Credits::ZERO));
        assert_eq!(funding.lago_billable_quantity_micros, Some(0));
        assert!(funding.allowance_consumptions.is_empty());
        assert!(funding.grant_consumptions.is_empty());
        assert_eq!(rows.iter().filter_map(|r|r.funding.as_ref()).map(|f|f.allowance_funded_quantity.unwrap_or(0)).sum::<i64>(), 5);
        let lago = std::sync::Arc::new(ModelLago::default());
        let mut config = crate::test_utils::test_app_config(); config.billing_enabled=true;
        let worker = reconcile::BillingReconciler::new(db.clone(), Some(lago.clone()), std::sync::Arc::new(config));
        db.collection::<Document>("usage_meter").update_many(doc! {}, doc! {"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(60))}}).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap().drift_alerts, 0);
        // The free write class and the fully allowance-funded paid write class
        // both have zero wallet-funded quantity and must emit no Lago event.
        let allowance_code = &t.components.iter().find(|c| c.key == "write30").unwrap().lago_metric_code;
        { let events=lago.events.lock().unwrap(); assert_eq!(events.len(), 3); assert!(events.values().all(|e|&e.code != free_code && &e.code != allowance_code)); }
        assert_accounts(&db).await;
        db.drop().await.unwrap();
    }).await
}
