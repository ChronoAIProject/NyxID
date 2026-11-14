use super::*;
use crate::models::{
    billing_tax_profile::{BillingTaxProfile, COLLECTION_NAME as PROFILES},
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    credits::Credits,
    service_billing::BillingMetric,
    usage_meter::CredentialClass,
};
use crate::services::billing::{
    reservation,
    route_context::{BillingRouteContext, NodeIntent},
    route_inventory::BillingIngress,
    taxes,
};

pub(super) async fn customer() -> Json<Value> {
    Json(json!({"customer":{
        "external_id":"owner", "billing_entity_code":"entity", "taxes":[]
    }}))
}

pub(super) async fn entity() -> Json<Value> {
    Json(json!({"billing_entity":{"code":"entity", "taxes":[]}}))
}

pub(super) async fn entitlements() -> Json<Value> {
    Json(json!({"entitlements":[{"code":"all_services"}]}))
}

pub(super) async fn subscription(State(p): State<Arc<Provider>>) -> Json<Value> {
    p.tax_reads.fetch_add(1, Ordering::SeqCst);
    let mut snapshot = plan(p.charges.lock().unwrap().clone())["plan"].clone();
    snapshot["taxes"] = json!([{
        "code":"tax", "name":"Tax", "rate":p.tax_rate.load(Ordering::SeqCst)
    }]);
    for charge in snapshot["charges"].as_array_mut().unwrap() {
        charge["taxes"] = json!([]);
    }
    // Freeze the old provider response, without blocking the plan writer's GET.
    p.tax_gate.pause().await;
    Json(json!({"subscription":{
        "external_id":"sub", "external_customer_id":"owner", "plan":snapshot
    }}))
}

#[tokio::test]
async fn tax_ordering_inline_admission_retries_across_real_plan_writer() {
    for existing in [true, false] {
        let f = Fixture::new("plan_tax_admission").await;
        let service = f.service("a").await;
        crate::billing_integration_tests::create_usage_index(&f.db).await;
        f.db.collection::<Document>(PROFILES)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"owner_id":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        f.db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME,
        )
        .insert_one(crate::billing_integration_tests::rate(
            "platform_requests",
            chrono::Utc::now(),
        ))
        .await
        .unwrap();
        let now = mongodb::bson::DateTime::now();
        f.db.collection::<Document>(WALLETS)
            .insert_one(doc! {
                "_id":"wallet", "owner_id":"owner", "lago_customer_id":"owner",
                "lago_subscription_id":"sub", "plan_kind":"prepaid",
                "collection_state":"good", "suspended":false,
                "balance_credits":Credits::from_whole(100),
                "reserved_credits":Credits::ZERO, "pending_lago_debits":Credits::ZERO,
                "pending_topup_expiry_credits":Credits::ZERO,
                "balance_synced_at":now, "created_at":now, "updated_at":now,
            })
            .await
            .unwrap();
        f.provider.tax_rate.store(5, Ordering::SeqCst);
        if existing {
            let wallet =
                f.db.collection::<BillingWallet>(WALLETS)
                    .find_one(doc! {"_id":"wallet"})
                    .await
                    .unwrap()
                    .unwrap();
            taxes::refresh(&f.db, f.lago.as_ref(), &wallet, 900)
                .await
                .unwrap();
            f.db.collection::<Document>(PROFILES)
                .update_one(
                    doc! {"owner_id":"owner"},
                    doc! {"$set":{
                        "fetched_at":mongodb::bson::DateTime::from_millis(0)
                    }},
                )
                .await
                .unwrap();
        }
        let billing = ServiceBilling {
            platform_billable: true,
            platform_charge_nyxid_credentials_only: false,
            ..Default::default()
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            "owner".into(),
            "owner".into(),
            None,
            None,
            None,
            Some("a".into()),
            NodeIntent::Direct,
            "test".into(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            Some(&billing),
            false,
        )
        .with_platform_metering(true);
        f.provider.tax_reads.store(0, Ordering::SeqCst);
        f.provider.tax_gate.arm();
        let (admission, ()) = tokio::join!(
            reservation::gate_and_reserve(&f.db, Some(f.lago.as_ref()), &ctx, false, 900),
            async {
                f.provider.tax_gate.entered().await;
                // Provider-owned tax config changes while admission holds 5%.
                // The real writer installs its charge and invalidates twice.
                f.provider.tax_rate.store(9, Ordering::SeqCst);
                assert!(f.billing.sync_service_price(&service).await.unwrap());
                assert_eq!(f.status(&service).await, PricingSyncStatus::Synced);
                f.provider.tax_gate.resume();
            }
        );
        let stored =
            f.db.collection::<BillingTaxProfile>(PROFILES)
                .find_one(doc! {"owner_id":"owner"})
                .await
                .unwrap()
                .unwrap();
        let provider_reads = f.provider.tax_reads.load(Ordering::SeqCst);
        let puts = f.provider.updates.lock().unwrap().len();
        f.db.drop().await.unwrap();
        let reserved = admission
            .expect("one real price sync must not fail admission")
            .expect("wallet-funded admission must reserve");
        assert_eq!(provider_reads, 2, "existing profile: {existing}");
        assert_eq!(puts, 1);
        assert!(
            stored.metric_taxes.contains_key("platform_svc_a"),
            "published profile must include the newly installed charge"
        );
        assert_eq!(
            taxes::snapshot(&stored, "platform_requests")
                .unwrap()
                .rate_percent,
            Credits::from_whole(9)
        );
        assert!(stored.refresh_failed_at.is_none());
        assert_eq!(
            reserved.total_reserved_credits,
            taxes::gross_ceiling(Credits::from_whole(1), Credits::from_whole(9)).unwrap()
        );
        println!(
            "PLAN_TAX_ADMISSION existing={existing}: provider_reads={provider_reads}, plan_puts={puts}, reserved=1.09"
        );
    }
}
