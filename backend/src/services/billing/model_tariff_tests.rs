use super::*;
use crate::models::{
    downstream_service::{PlatformKeyConfig, test_helpers::dummy_service},
    service_billing::BillingMetric,
};

pub(crate) fn fixture() -> (
    DownstreamService,
    ModelTariff,
    HashMap<String, BillingRateCache>,
) {
    let now = Utc::now();
    let mut service = dummy_service();
    service.id = "service".into();
    service.slug = "models".into();
    service.is_active = true;
    service.platform_key = Some(PlatformKeyConfig {
        purpose: PlatformKeyPurpose::ModelOffering,
        ..Default::default()
    });
    let mut tariff = ModelTariff {
        id: Uuid::new_v4().to_string(),
        revision: 2,
        service_id: service.id.clone(),
        model_id: "f8da2b33-164a-4401-9eee-dc48ce40a729".into(),
        public_model: "nyxid/gpt-test".into(),
        lane: TariffLane::PlatformKey,
        variant: BTreeMap::new(),
        contract: UsageContract {
            version: "openai.responses.v1".into(),
            fixture_sha256: "a".repeat(64),
            family: UsageFamily::Openai,
            tokens: true,
            cache_read: true,
            cache_write_ttls: vec![CacheTtl::ThirtyMinutes, CacheTtl::TwelveHours],
            split_reasoning: false,
            input_modalities: vec![],
            output_modalities: vec![],
            bill_tool_input: false,
            images: false,
            requests: false,
        },
        bounds: RequestBounds {
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_candidates: 1,
            max_tool_iterations: 1,
            max_images: 0,
            max_request_bytes: 1024,
            max_response_bytes: 1024,
        },
        components: vec![],
        source: "Certified fixture, retail price authored by admin".into(),
        status: TariffStatus::Synced,
        sync_error: None,
        created_at: now,
        updated_at: now,
    };
    for selector in model_usage::selectors(&tariff.contract).unwrap() {
        let key = match selector {
            Selector::Input => "input",
            Selector::Output => "output",
            Selector::CacheRead => "read",
            Selector::CacheWrite {
                ttl: CacheTtl::ThirtyMinutes,
            } => "write30",
            Selector::CacheWrite { .. } => "write12",
            _ => unreachable!(),
        };
        tariff.components.push(TariffComponent {
            key: key.into(),
            metric: model_usage::metric(selector),
            selector,
            credits_per_unit: "0.000001000001".into(),
            explicit_free: false,
            max_quantity: 100,
            lago_metric_code: String::new(),
        });
    }
    validate(&mut tariff, &service.slug).unwrap();
    service.billing = Some(ServiceBilling {
        platform_billable: true,
        platform_key_pricing: Some(LanePricing {
            metric: BillingMetric::Requests,
            credits_per_unit: "999".into(),
            lago_metric_code: "legacy-base".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![],
            operations: vec![],
            model_prices: vec![ModelPriceBinding {
                model_id: tariff.model_id.clone(),
                variant: tariff.variant.clone(),
                allowance_metrics: tariff.components.iter().map(|c| c.metric).collect(),
                active_tariff_id: Some(tariff.id.clone()),
                pending_tariff_id: None,
            }],
        }),
        ..Default::default()
    });
    let rates = tariff
        .components
        .iter()
        .map(|c| {
            let rate = BillingRateCache {
                id: BillingRateCache::cache_id(&c.lago_metric_code, None),
                lago_metric_code: c.lago_metric_code.clone(),
                model: None,
                credits_per_unit_micros: 1,
                credits_per_unit_pico: Some(1_000_001),
                synced_at: now,
                retired_at: None,
            };
            (c.lago_metric_code.clone(), rate)
        })
        .collect();
    (service, tariff, rates)
}
fn selected(
    s: &DownstreamService,
    t: &ModelTariff,
    r: &HashMap<String, BillingRateCache>,
) -> Result<SelectedTariff, &'static str> {
    select(
        s,
        &t.model_id,
        CredentialClass::NyxidManagedMaster,
        &t.variant,
        std::slice::from_ref(t),
        r,
        Utc::now(),
        900,
    )
}
#[test]
fn revision_codes_and_whole_lane_replacement_are_exact() {
    let (s, t, r) = fixture();
    let selected = selected(&s, &t, &r).unwrap();
    let ctx = super::super::config::model_test_route(&s, CredentialClass::NyxidManagedMaster)
        .with_model_tariff(selected)
        .unwrap();
    assert_eq!(
        ctx.platform_specs().map(|(_, c)| c).collect::<Vec<_>>(),
        t.components
            .iter()
            .map(|c| c.lago_metric_code.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        ctx.platform_specs()
            .all(|(_, c)| c.contains("_t2_") && !c.contains("legacy"))
    );
    assert_ne!(
        metric_code(&s.slug, &t.model_id, t.lane, 2, "input").unwrap(),
        metric_code(&s.slug, &t.model_id, t.lane, 3, "input").unwrap()
    );
    assert!(metric_code(&"a".repeat(255), &t.model_id, t.lane, 2, "input").is_err());
}
#[test]
fn selection_fails_closed_for_every_missing_failed_retired_or_changed_component() {
    let (s, t, r) = fixture();
    assert!(selected(&s, &t, &r).is_ok());
    for c in &t.components {
        let mut rates = r.clone();
        rates.remove(&c.lago_metric_code);
        assert_eq!(selected(&s, &t, &rates).unwrap_err(), "rate_missing");
        let mut rates = r.clone();
        rates
            .get_mut(&c.lago_metric_code)
            .unwrap()
            .credits_per_unit_pico = Some(1);
        assert_eq!(selected(&s, &t, &rates).unwrap_err(), "rate_mismatch");
        let mut rates = r.clone();
        rates.get_mut(&c.lago_metric_code).unwrap().retired_at = Some(Utc::now());
        assert!(selected(&s, &t, &rates).is_err());
        let mut rates = r.clone();
        rates.get_mut(&c.lago_metric_code).unwrap().synced_at =
            Utc::now() - chrono::Duration::seconds(901);
        assert!(selected(&s, &t, &rates).is_err());
    }
    for status in [TariffStatus::Pending, TariffStatus::Retired] {
        let mut t = t.clone();
        t.status = status;
        assert!(selected(&s, &t, &r).is_err());
    }
}
#[test]
fn pending_replacement_does_not_mix_revisions_and_unknown_variants_never_fallback() {
    let (mut s, t, r) = fixture();
    s.billing
        .as_mut()
        .unwrap()
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices[0]
        .pending_tariff_id = Some("replacement".into());
    assert!(selected(&s, &t, &r).is_ok());
    assert!(
        select(
            &s,
            &t.model_id,
            CredentialClass::NyxidManagedMaster,
            &BTreeMap::from([("tier".into(), "unknown".into())]),
            std::slice::from_ref(&t),
            &r,
            Utc::now(),
            900
        )
        .is_err()
    );
    for class in [
        CredentialClass::NoAuth,
        CredentialClass::UserOwned,
        CredentialClass::NodeManaged,
        CredentialClass::NyxidPlatformOauthApp,
        CredentialClass::AgentOverrideUserOwned,
    ] {
        assert!(
            select(
                &s,
                &t.model_id,
                class,
                &t.variant,
                std::slice::from_ref(&t),
                &r,
                Utc::now(),
                900
            )
            .is_err()
        );
    }
}
#[test]
fn complete_explicit_free_partition_and_bound_validation() {
    let (_, mut t, _) = fixture();
    t.components.pop();
    assert!(validate(&mut t, "models").is_err());
    let (_, mut t, _) = fixture();
    t.components[0].credits_per_unit = "0".into();
    assert!(validate(&mut t, "models").is_err());
    t.components[0].explicit_free = true;
    assert!(validate(&mut t, "models").is_ok());
    t.components[0].max_quantity = 0;
    assert!(validate(&mut t, "models").is_err());
    let (_, mut t, _) = fixture();
    t.components[1].selector = t.components[0].selector;
    assert!(validate(&mut t, "models").is_err());
}
#[test]
fn explain_and_execution_use_identical_component_prices_and_blocking_reasons() {
    let (s, t, r) = fixture();
    for missing in [false, true] {
        let mut rates = r.clone();
        if missing {
            rates.remove(&t.components[0].lago_metric_code);
        }
        let env = super::super::config::ResolverEnv {
            model_tariffs: std::slice::from_ref(&t),
            billing_enabled: true,
            resale_enabled: true,
            rate_ttl_secs: 900,
            now: Utc::now(),
            rates: &rates,
        };
        let view = super::super::config::resolve_service_billing(&env, &s);
        let result = selected(&s, &t, &rates);
        let explained = &view.model_prices[0];
        assert_eq!(explained.available, result.is_ok());
        match result {
            Ok(selected) => assert_eq!(explained.components, selected.tariff().components),
            Err(reason) => assert_eq!(explained.blocking_reason.as_deref(), Some(reason)),
        }
    }
}
#[test]
fn legacy_forms_cannot_clear_or_forge_model_bindings_and_plain_lanes_stay_compatible() {
    let (s, _, _) = fixture();
    let old = s.billing.unwrap();
    let mut request = old.clone();
    request
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices
        .clear();
    super::super::pricing::normalize_lane_pricing("models", Some(&old), &mut request).unwrap();
    assert_eq!(
        request.platform_key_pricing.as_ref().unwrap().model_prices,
        old.platform_key_pricing.as_ref().unwrap().model_prices
    );
    request.platform_key_pricing = None;
    assert!(
        super::super::pricing::normalize_lane_pricing("models", Some(&old), &mut request).is_err()
    );
    let mut request = old.clone();
    request.platform_key_pricing.as_mut().unwrap().model_prices[0].active_tariff_id =
        Some("forged".into());
    assert!(
        super::super::pricing::normalize_lane_pricing("models", Some(&old), &mut request).is_err()
    );
    let mut plain = old;
    plain
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices
        .clear();
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("model_prices"));
}
#[test]
fn model_charge_vector_has_distinct_same_metric_ttls_and_public_identity_only() {
    let (s, t, r) = fixture();
    let selected = selected(&s, &t, &r).unwrap();
    let e = model_usage::Evidence {
        input: Some(50),
        output: Some(10),
        cache_read: Some(10),
        cache_write: Some(20),
        writes: BTreeMap::from([(CacheTtl::ThirtyMinutes, 5), (CacheTtl::TwelveHours, 15)]),
        ..Default::default()
    };
    let usage = selected.usage(&e).unwrap();
    let q = &usage.model_charges.as_ref().unwrap().quantities;
    assert_eq!(
        q[&t.components
            .iter()
            .find(|c| c.key == "write30")
            .unwrap()
            .lago_metric_code],
        5
    );
    assert_eq!(
        q[&t.components
            .iter()
            .find(|c| c.key == "write12")
            .unwrap()
            .lago_metric_code],
        15
    );
    assert_eq!(usage.tokens, 60);
    assert!(usage.reported_model.is_none());
    let mut unbounded = e;
    unbounded.output = Some(101);
    assert!(selected.usage(&unbounded).is_err());
}

#[tokio::test]
async fn authoring_is_readiness_gated_cas_fenced_admin_only_and_retirement_keeps_history() {
    let db = crate::test_utils::connect_transaction_test_database("model_tariff_authoring").await;
    platform_model_service::ensure_indexes(&db).await.unwrap();
    ensure_indexes(&db).await.unwrap();
    crate::test_utils::init_test_audit_chain_hmac_key();
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    let id = Uuid::new_v4().to_string();
    let mut user = crate::test_utils::test_user(&id, crate::models::user::UserType::Person);
    user.role_ids.push(roles.admin);
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(user)
        .await
        .unwrap();
    let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&id));
    let model = platform_model_service::create(
        &db,
        &actor,
        platform_model_service::NewModel {
            public_id: "nyxid/gpt-test".into(),
            display_name: "GPT test".into(),
            description: String::new(),
            family: crate::models::platform_model::ModelFamily::Openai,
            capabilities: Default::default(),
        },
    )
    .await
    .unwrap();
    let (mut service, t, _) = fixture();
    service.billing = None;
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    db.collection::<PlatformModel>(MODELS)
        .update_one(
            doc! {"_id":&model.id},
            doc! {"$set":{"billing_service_id":&service.id}},
        )
        .await
        .unwrap();
    let input = NewTariff {
        revision: model.revision,
        lane: t.lane,
        variant: t.variant,
        contract: t.contract,
        bounds: t.bounds,
        components: t.components,
        source: t.source,
    };
    assert!(matches!(
        create(&db, &actor, &model.id, input.clone()).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    security_readiness_service::ready_test_replica(&db).await;
    let created = create(&db, &actor, &model.id, input.clone()).await.unwrap();
    assert_eq!(created.status, TariffStatus::Pending);
    assert_eq!(created.public_model, model.public_id);
    assert!(create(&db, &actor, &model.id, input.clone()).await.is_err());
    let plain_id = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &plain_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let plain = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&plain_id));
    let mut next = input;
    next.revision = created.revision;
    assert!(matches!(
        create(&db, &plain, &model.id, next).await,
        Err(AppError::Forbidden(_))
    ));
    retire(&db, &actor, &model.id, &created.id, created.revision)
        .await
        .unwrap();
    let history = list(&db, &model.id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, TariffStatus::Retired);
    assert_eq!(history[0].components, created.components);
    let live = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&service.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        live.billing
            .unwrap()
            .platform_key_pricing
            .unwrap()
            .model_prices[0]
            .pending_tariff_id
            .is_none()
    );
    let audit: Vec<bson::Document> = db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(audit.iter().all(|r| r.get("entry_hash").is_some()));
    db.drop().await.unwrap();
}
#[test]
fn stale_catalog_update_cannot_erase_a_concurrently_published_binding() {
    let (s, _, _) = fixture();
    let mut stale = s.billing.clone().unwrap();
    stale
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices
        .clear();
    assert!(validate_catalog_update(s.billing.as_ref(), Some(&stale)).is_err());
    assert!(validate_catalog_update(s.billing.as_ref(), None).is_err());
    assert!(validate_catalog_update(s.billing.as_ref(), s.billing.as_ref()).is_ok());
    assert!(validate_catalog_update(None, None).is_ok());
}

#[test]
fn explicit_free_classes_need_no_provider_rate_but_missing_prices_are_never_free() {
    let (s, mut t, _) = fixture();
    for c in &mut t.components {
        c.credits_per_unit = "0".into();
        c.explicit_free = true;
    }
    validate(&mut t, &s.slug).unwrap();
    let selected = selected(&s, &t, &HashMap::new()).unwrap();
    assert!(!selected.paid());
    t.components[0].explicit_free = false;
    assert!(
        select(
            &s,
            &t.model_id,
            CredentialClass::NyxidManagedMaster,
            &t.variant,
            &[t.clone()],
            &HashMap::new(),
            Utc::now(),
            900
        )
        .is_err()
    );
}
#[test]
fn candidate_and_tool_iteration_bounds_must_cover_all_charges() {
    let (_, mut t, _) = fixture();
    t.bounds.max_candidates = 2;
    assert!(validate(&mut t, "models").is_err());
    t.components
        .iter_mut()
        .find(|c| c.selector == Selector::Output)
        .unwrap()
        .max_quantity = 200;
    assert!(validate(&mut t, "models").is_ok());
    t.bounds.max_tool_iterations = 2;
    assert!(validate(&mut t, "models").is_err());
}

#[test]
fn aggregate_context_bound_cannot_be_evaded_by_distributing_tokens_across_components() {
    let (_, t, _) = fixture();
    let e = model_usage::Evidence {
        input: Some(150),
        output: Some(10),
        cache_read: Some(80),
        cache_write: Some(20),
        writes: BTreeMap::from([(CacheTtl::ThirtyMinutes, 5), (CacheTtl::TwelveHours, 15)]),
        ..Default::default()
    };
    assert!(model_usage::normalize(&t.contract, &e).is_ok());
    assert!(model_usage::charge_vector(&t, &e).is_err());
}

#[test]
fn explain_parity_covers_every_credential_class_and_explicit_free_state() {
    for class in [
        CredentialClass::UserOwned,
        CredentialClass::AgentOverrideUserOwned,
        CredentialClass::NodeManaged,
        CredentialClass::NyxidPlatformOauthApp,
        CredentialClass::NyxidManagedMaster,
    ] {
        for free in [false, true] {
            let (mut s, mut t, _) = fixture();
            let byok = BillingLane::for_credential(class) == Some(BillingLane::Byok);
            t.lane = if byok {
                TariffLane::Byok
            } else {
                TariffLane::PlatformKey
            };
            if free {
                for c in &mut t.components {
                    c.credits_per_unit = "0".into();
                    c.explicit_free = true;
                }
            }
            validate(&mut t, &s.slug).unwrap();
            let b = s.billing.as_mut().unwrap();
            if byok {
                b.byok_pricing = b.platform_key_pricing.take();
            }
            let rates: HashMap<_, _> = t
                .components
                .iter()
                .filter(|c| !c.explicit_free)
                .map(|c| {
                    (
                        c.lago_metric_code.clone(),
                        BillingRateCache {
                            id: BillingRateCache::cache_id(&c.lago_metric_code, None),
                            lago_metric_code: c.lago_metric_code.clone(),
                            model: None,
                            credits_per_unit_micros: 1,
                            credits_per_unit_pico: Some(1_000_001),
                            synced_at: Utc::now(),
                            retired_at: None,
                        },
                    )
                })
                .collect();
            let admitted = select(
                &s,
                &t.model_id,
                class,
                &t.variant,
                std::slice::from_ref(&t),
                &rates,
                Utc::now(),
                900,
            )
            .unwrap();
            let ctx = super::super::config::model_test_route(&s, class)
                .with_model_tariff(admitted)
                .unwrap();
            let explanation = explain(&s, std::slice::from_ref(&t), &rates, Utc::now(), 900, true);
            assert_eq!(explanation.len(), 1);
            assert!(explanation[0].available);
            assert_eq!(explanation[0].components, t.components);
            assert_eq!(
                ctx.platform_specs().collect::<Vec<_>>(),
                t.components
                    .iter()
                    .map(|c| (c.metric, c.lago_metric_code.as_str()))
                    .collect::<Vec<_>>()
            );
            assert_eq!(ctx.service_platform_billable, !free);
            let disabled = explain(&s, std::slice::from_ref(&t), &rates, Utc::now(), 900, false);
            assert_eq!(disabled[0].available, free);
            assert_eq!(
                disabled[0].blocking_reason.as_deref(),
                if free { None } else { Some("billing_disabled") }
            );
            assert!(
                select(
                    &s,
                    &t.model_id,
                    CredentialClass::NoAuth,
                    &t.variant,
                    std::slice::from_ref(&t),
                    &rates,
                    Utc::now(),
                    900
                )
                .is_err()
            );
        }
    }
}

#[test]
fn charge_vector_preserves_native_breakdown_and_modality_observability() {
    let (_, t, _) = fixture();
    let e = model_usage::Evidence {
        input: Some(50),
        output: Some(10),
        cache_read: Some(10),
        cache_write: Some(20),
        writes: BTreeMap::from([(CacheTtl::ThirtyMinutes, 5), (CacheTtl::TwelveHours, 15)]),
        input_image: Some(3),
        input_audio: Some(4),
        ..Default::default()
    };
    let usage = model_usage::charge_vector(&t, &e).unwrap();
    assert_eq!(usage.token_breakdown.unwrap().prompt_tokens, 50);
    assert_eq!(usage.image_tokens.unwrap().input_tokens, 3);
    assert_eq!(usage.audio_tokens.unwrap().input_tokens, 4);
    let mut claude = t;
    claude.contract.family = UsageFamily::Claude;
    claude.contract.version = "anthropic.messages.v1".into();
    claude.contract.cache_write_ttls = vec![CacheTtl::FiveMinutes, CacheTtl::OneHour];
    for c in &mut claude.components {
        if let Selector::CacheWrite { ttl } = &mut c.selector {
            *ttl = if *ttl == CacheTtl::ThirtyMinutes {
                CacheTtl::FiveMinutes
            } else {
                CacheTtl::OneHour
            };
        }
    }
    let evidence = model_usage::Evidence {
        input: Some(50),
        output: Some(10),
        cache_read: Some(10),
        cache_write: Some(20),
        writes: BTreeMap::from([(CacheTtl::FiveMinutes, 5), (CacheTtl::OneHour, 15)]),
        ..Default::default()
    };
    let usage = model_usage::charge_vector(&claude, &evidence).unwrap();
    assert_eq!(usage.token_breakdown.unwrap().prompt_tokens, 50);
    assert_eq!(usage.tokens, 90);
    assert_eq!(
        usage
            .model_charges
            .unwrap()
            .quantities
            .values()
            .sum::<i64>(),
        90
    );
}

#[tokio::test]
async fn plain_image_and_family_tariff_publication_serves_overlap_and_refuses_unsupported_or_unrecorded_capabilities()
 {
    use crate::models::{security_capability_activation as activation, security_replica};
    use crate::services::platform_model_execution::{certification, images};
    for kind in ["plain", "image", "family"] {
        let required = match kind {
            "plain" => vec![READINESS],
            "image" => vec![READINESS, images::READINESS],
            _ => vec![READINESS, images::READINESS, certification::READINESS],
        };
        for capability in required {
            let db =
                crate::test_utils::connect_transaction_test_database("tariff_overlap_publication")
                    .await;
            platform_model_service::ensure_indexes(&db).await.unwrap();
            ensure_indexes(&db).await.unwrap();
            crate::services::role_service::seed_system_roles(&db)
                .await
                .unwrap();
            let roles = crate::services::role_service::get_platform_role_ids(&db)
                .await
                .unwrap();
            let id = Uuid::new_v4().to_string();
            let mut user = crate::test_utils::test_user(&id, crate::models::user::UserType::Person);
            user.role_ids.push(roles.admin);
            db.collection(crate::models::user::COLLECTION_NAME)
                .insert_one(user)
                .await
                .unwrap();
            let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&id));
            let model = platform_model_service::create(
                &db,
                &actor,
                platform_model_service::NewModel {
                    public_id: "nyxid/gpt-test".into(),
                    display_name: "Test".into(),
                    description: String::new(),
                    family: crate::models::platform_model::ModelFamily::Openai,
                    capabilities: Default::default(),
                },
            )
            .await
            .unwrap();
            let (mut service, mut t, _) = fixture();
            service.billing = None;
            db.collection::<DownstreamService>(SERVICES)
                .insert_one(&service)
                .await
                .unwrap();
            db.collection::<PlatformModel>(MODELS)
                .update_one(
                    doc! {"_id":&model.id},
                    doc! {"$set":{"billing_service_id":&service.id}},
                )
                .await
                .unwrap();
            if kind == "image" {
                t.contract.fixture_sha256 = fixture_digest(include_str!(
                    "../platform_model_execution/fixtures/responses-images-v1.json"
                ));
            }
            if kind == "family" {
                t.contract.version = "openai.responses.v2".into();
                t.contract.fixture_sha256 =
                    fixture_digest(certification::family_fixture(t.contract.family).unwrap());
                t.source = format!(
                    "contract-attested-v1:{} https://provider.example/verified",
                    model.public_id
                );
            }
            let mut input = NewTariff {
                revision: model.revision,
                lane: t.lane,
                variant: t.variant,
                contract: t.contract,
                bounds: t.bounds,
                components: t.components,
                source: t.source,
            };
            security_readiness_service::ready_test_replica(&db).await;
            security_readiness_service::stop_test_replica(&db).await;
            let sibling = security_readiness_service::overlapping_test_replica(&db).await;
            let created = create(&db, &actor, &model.id, input.clone()).await.unwrap();
            input.revision = created.revision;
            security_readiness_service::remove_test_replica_capability(&db, &sibling, capability)
                .await;
            assert!(
                matches!(
                    create(&db, &actor, &model.id, input.clone()).await,
                    Err(AppError::SecurityReadinessUnavailable)
                ),
                "{kind}/{capability}"
            );
            db.collection::<security_replica::SecurityReplica>(security_replica::COLLECTION_NAME)
                .replace_one(doc! {"_id":&sibling.id}, &sibling)
                .await
                .unwrap();
            db.collection::<bson::Document>(activation::COLLECTION_NAME)
                .delete_one(doc! {"record_kind":activation::RECORD_KIND,"capability":capability})
                .await
                .unwrap();
            assert!(
                matches!(
                    create(&db, &actor, &model.id, input.clone()).await,
                    Err(AppError::SecurityReadinessUnavailable)
                ),
                "{kind}/{capability}"
            );
        }
    }
}

fn fixture_digest(value: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(value.as_bytes()))
}
