use super::*;
use crate::models::{
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    platform_entry_gateway::{COLLECTION_NAME as GATEWAYS, PlatformEntryGateway},
    usage_meter::COLLECTION_NAME as METERS,
};
use crate::services::{
    platform_bot_runtime_control_service as controls,
    platform_entry_admin_service::NativeConversionOutcome,
    platform_entry_gateway_service::tests::setup::SetupFixture,
};
use wiremock::MockServer;

#[tokio::test]
async fn initial_native_gateway_probe_funding_rechecks_transport_without_existing_controls() {
    for drift in ["none", "retired", "provider_key", "route_key", "pending"] {
        let gateway = SetupFixture::new(0).await;
        let NativeConversionOutcome::Converted(profile) = gateway.convert(None).await.unwrap()
        else {
            panic!("fresh gateway conversion");
        };
        let mut state = gateway.state().clone();
        let db = &state.db.clone();
        assert!(controls::get(db, &profile.id).await.unwrap().is_none());
        let reviewed = controls::reviewed_policy(
            db,
            gateway.actor(),
            &profile.id,
            &RuntimePatch {
                public_deployment_digest: None,
                initialize: true,
                expected_profile_generation: Some(profile.generation),
                expected: ExpectedRuntimeRevisions {
                    manager_authority_epoch: 0,
                    personal_control_revision: 0,
                    public_policy_revision: 0,
                },
                personal_enabled: Some(false),
                public_enabled: Some(true),
                emergency_stopped: None,
                public_policy: PolicyPatch::Update(PublicPolicyPatch {
                    sponsor_owner_user_id: Some(gateway.funding().into()),
                    nyxagent_profile: Some(manifest().profiles[0].id.clone()),
                    total_daily_credits: Some(Credits::from_whole(10)),
                    sender_daily_credits: Some("0.5".parse().unwrap()),
                }),
            },
        )
        .await
        .unwrap();
        let consent = db
            .collection::<ProbeConsent>(PROBE_CONSENTS)
            .find_one(doc! {"binding":bson::to_bson(&policy::binding(&reviewed).unwrap()).unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert!(consent.controls_id.is_none());
        let row = db
            .collection::<PlatformEntryGateway>(GATEWAYS)
            .find_one(doc! {"profile_id":&profile.id,"state":"active"})
            .await
            .unwrap()
            .unwrap();

        // Fund through the actual billing admission transaction with a local
        // fake Lago and counted local provider. No model request is needed to
        // establish whether this exact initial consent can reserve sponsor money.
        let provider = MockServer::start().await;
        crate::services::channel_x_tests::billing::enable_billing_with_entitlement(
            &mut state,
            gateway.funding(),
            "chrono-llm-public",
        )
        .await;
        let outer = outer();
        let mut model = model();
        model.base_url = provider.uri();
        model.billing = Some(
            serde_json::from_value(json!({
                "platform_key_pricing": {
                    "metric":"requests", "credits_per_unit":"0.01",
                    "lago_metric_code":"native_probe_request", "sync_status":"synced"
                }
            }))
            .unwrap(),
        );
        db.collection::<DownstreamService>(SERVICES)
            .insert_many([&outer, &model])
            .await
            .unwrap();
        cache_rates(db, &[("native_probe_request", "0.01")]).await;
        let mut deployment = evidence(&outer, &model, &[]);
        deployment.binding = policy::binding(&reviewed).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let operation = deployment.manifest.profiles[0].model_operations[0].clone();
        let (_, mut route, maximum, prices) = route(&state, gateway.funding(), &id, &operation)
            .await
            .unwrap();
        route.public_probe = true;
        let probe = PublicProbe {
            id: id.clone(),
            binding: deployment.binding,
            activation_digest: None,
            selected_profile: deployment.selected_profile,
            manifest: deployment.manifest,
            nyxagent_service_id: outer.id,
            nyxagent_catalog_digest: deployment.nyxagent_catalog_digest,
            operation,
            request_digest: "funding-only-local-fixture".into(),
            route_digest: crate::services::platform_public_budget_service::route_digest(&route)
                .unwrap(),
            pricing_digest: prices,
            meter_row_ids: vec![],
            bucket_id: uuid::Uuid::new_v4().to_string(),
            state: "prepared".into(),
            maximum,
            exact_model_observed: false,
            expires_at: Utc::now() + Duration::seconds(90),
        };
        assert_eq!(maximum, "0.01".parse().unwrap());
        db.collection::<PublicProbe>(PROBES)
            .insert_one(&probe)
            .await
            .unwrap();
        match drift {
            "retired" => {
                db.collection::<Document>(GATEWAYS)
                    .update_one(doc! {"_id":&row.id}, doc! {"$set":{"state":"retired"}})
                    .await
                    .unwrap();
            }
            "provider_key" | "route_key" => {
                let key = if drift == "provider_key" {
                    &row.provider_api_key_id
                } else {
                    &row.route_api_key_id
                };
                tx::update_one(
                    db,
                    doc! {"_id":key},
                    doc! {"$set":{"is_active":false}},
                    None,
                )
                .await
                .unwrap();
            }
            "pending" => {
                db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                    .update_one(doc! {"_id":&profile.channel_bot_id},
                        doc! {"$set":{"platform_entry_pending":{"attempt_id":uuid::Uuid::new_v4().to_string()}}})
                    .await.unwrap();
            }
            "none" => {}
            _ => unreachable!(),
        }
        let lago = state.billing.lago_client().unwrap();
        let result = billing::reservation::gate_and_reserve(
            db,
            Some(lago.as_ref()),
            &route,
            false,
            state.config.billing_rate_cache_ttl_secs,
        )
        .await;
        let stored = db
            .collection::<PublicProbe>(PROBES)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        let wallet = db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id":gateway.funding()})
            .await
            .unwrap()
            .unwrap();
        let meters = db
            .collection::<Document>(METERS)
            .count_documents(doc! {"billing_request_id":&id})
            .await
            .unwrap();
        if drift == "none" {
            assert_eq!(result.unwrap().unwrap().total_reserved_credits, maximum);
            assert_eq!(stored.state, "funded");
            assert_eq!(wallet.reserved_credits, maximum);
            assert_eq!(meters, 1);
            assert_eq!(stored.meter_row_ids.len(), 1);
        } else {
            assert!(
                result.is_err(),
                "{drift} must refuse actual sponsor funding"
            );
            assert_eq!(stored.state, "prepared");
            assert!(stored.meter_row_ids.is_empty());
            assert_eq!(wallet.reserved_credits, Credits::ZERO);
            assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
            assert_eq!(meters, 0);
        }
        assert!(provider.received_requests().await.unwrap().is_empty());
        assert!(controls::get(db, &profile.id).await.unwrap().is_none());
        gateway.close().await;
    }
}
