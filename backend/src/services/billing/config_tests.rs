use std::collections::HashMap;

use chrono::{Duration, Utc};

use super::*;
use crate::models::billing_rate_cache::BillingRateCache;
use crate::models::downstream_service::DownstreamService;
use crate::models::service_billing::{
    BillingMetric, LanePriceComponent, LanePricing, OperationPrice, PricingSyncStatus,
    ServiceBilling, ServicePlatformPricing,
};
use crate::models::usage_meter::CredentialClass as C;

fn service(billing: ServiceBilling) -> DownstreamService {
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = "svc-1".into();
    service.slug = "svc".into();
    service.billing = Some(billing);
    service
}

fn lane(metric: BillingMetric, price: &str, code: &str, status: PricingSyncStatus) -> LanePricing {
    LanePricing {
        metric,
        credits_per_unit: price.into(),
        lago_metric_code: code.into(),
        sync_status: status,
        sync_error: (status == PricingSyncStatus::Failed).then(|| "lago down".into()),
        components: Vec::new(),
        operations: Vec::new(),
    }
}

fn component(
    metric: BillingMetric,
    price: &str,
    code: &str,
    status: PricingSyncStatus,
) -> LanePriceComponent {
    LanePriceComponent {
        metric,
        credits_per_unit: price.into(),
        lago_metric_code: code.into(),
        sync_status: status,
        sync_error: None,
    }
}

fn operation(key: &str, price: &str, code: &str, status: PricingSyncStatus) -> OperationPrice {
    OperationPrice {
        operation: key.into(),
        credits_per_unit: price.into(),
        lago_metric_code: code.into(),
        sync_status: status,
        sync_error: None,
    }
}

fn rate(code: &str, pico: i64, age_secs: i64, retired: bool) -> (String, BillingRateCache) {
    (
        code.to_string(),
        BillingRateCache {
            id: BillingRateCache::cache_id(code, None),
            lago_metric_code: code.into(),
            model: None,
            credits_per_unit_micros: pico / 1_000_000,
            credits_per_unit_pico: Some(pico),
            synced_at: Utc::now() - Duration::seconds(age_secs),
            retired_at: retired.then(Utc::now),
        },
    )
}

fn env(rates: &HashMap<String, BillingRateCache>) -> ResolverEnv<'_> {
    ResolverEnv {
        billing_enabled: true,
        resale_enabled: true,
        rate_ttl_secs: 600,
        now: Utc::now(),
        rates,
    }
}

fn class(resolved: &EffectiveServiceBilling, credential: C) -> &EffectiveClassBilling {
    resolved
        .classes
        .iter()
        .find(|group| group.credential_classes.contains(&credential))
        .expect("every credential class is resolved")
}

#[test]
fn synced_lane_prices_its_classes_and_leaves_other_lanes_free() {
    let mut byok = lane(
        BillingMetric::Requests,
        "0.25",
        "svc_byok",
        PricingSyncStatus::Synced,
    );
    byok.components = vec![
        component(
            BillingMetric::Bytes,
            "0.000001",
            "svc_byok_bytes",
            PricingSyncStatus::Synced,
        ),
        component(
            BillingMetric::Images,
            "2",
            "svc_byok_images",
            PricingSyncStatus::Pending,
        ),
    ];
    byok.operations = vec![
        operation(
            "send_dm",
            "0.5",
            "svc_byok_op_send_dm",
            PricingSyncStatus::Synced,
        ),
        operation(
            "search",
            "0.1",
            "svc_byok_op_search",
            PricingSyncStatus::Failed,
        ),
    ];
    let svc = service(ServiceBilling {
        byok_pricing: Some(byok),
        ..Default::default()
    });
    let rates = HashMap::new();
    let resolved = resolve_service_billing(&env(&rates), &svc);

    let own = class(&resolved, C::UserOwned);
    assert_eq!(
        own.credential_classes,
        vec![
            C::NyxidPlatformOauthApp,
            C::UserOwned,
            C::AgentOverrideUserOwned,
            C::NodeManaged
        ]
    );
    assert_eq!(own.lane, Some("byok"));
    assert!(own.platform_charged);
    assert_eq!(own.source, BillingConfigSource::Service);
    assert_eq!(own.reason, EffectiveReason::LaneSynced);
    let charged: Vec<_> = own
        .prices
        .iter()
        .map(|p| {
            (
                p.origin,
                p.lago_metric_code.as_str(),
                p.credits_per_unit.as_deref(),
                p.operation.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        charged,
        vec![
            ("lane_primary", "svc_byok", Some("0.25"), None),
            ("lane_component", "svc_byok_bytes", Some("0.000001"), None),
            (
                "operation",
                "svc_byok_op_send_dm",
                Some("0.5"),
                Some("send_dm")
            ),
        ]
    );
    assert!(
        own.prices
            .iter()
            .all(|p| p.source == BillingConfigSource::Service)
    );
    // Unsynced extras never charge and never fall back.
    let pending: Vec<_> = own
        .pending
        .iter()
        .map(|p| {
            (
                p.origin,
                p.lago_metric_code.as_str(),
                p.sync_status,
                p.source,
            )
        })
        .collect();
    assert_eq!(
        pending,
        vec![
            (
                "lane_component",
                "svc_byok_images",
                Some(PricingSyncStatus::Pending),
                BillingConfigSource::Free
            ),
            (
                "operation",
                "svc_byok_op_search",
                Some(PricingSyncStatus::Failed),
                BillingConfigSource::Free
            ),
        ]
    );

    let master = class(&resolved, C::NyxidManagedMaster);
    assert_eq!(master.reason, EffectiveReason::LaneNotConfigured);
    assert_eq!(master.source, BillingConfigSource::Free);
    assert!(!master.platform_charged && master.prices.is_empty());

    let no_auth = class(&resolved, C::NoAuth);
    assert_eq!(no_auth.reason, EffectiveReason::NoAuth);
    assert_eq!(no_auth.lane, None);
    assert!(!no_auth.platform_charged);
}

#[test]
fn unsynced_primary_charges_the_legacy_price_and_disables_the_whole_lane() {
    let mut pk = lane(
        BillingMetric::Tokens,
        "0.002",
        "svc_pk",
        PricingSyncStatus::Failed,
    );
    pk.components = vec![component(
        BillingMetric::Images,
        "1",
        "svc_pk_images",
        PricingSyncStatus::Synced,
    )];
    let svc = service(ServiceBilling {
        platform_billable: true,
        platform_pricing: Some(ServicePlatformPricing {
            credits_per_unit: "0.75".into(),
            lago_metric_code: "platform_svc_svc".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
        }),
        platform_key_pricing: Some(pk),
        ..Default::default()
    });
    let rates = HashMap::new();
    let resolved = resolve_service_billing(&env(&rates), &svc);
    let master = class(&resolved, C::NyxidManagedMaster);
    assert_eq!(master.reason, EffectiveReason::LaneUnsyncedLegacy);
    assert_eq!(master.source, BillingConfigSource::Legacy);
    assert!(master.platform_charged);
    assert_eq!(
        master.prices.len(),
        1,
        "a synced extra cannot revive an unsynced lane"
    );
    let price = &master.prices[0];
    assert_eq!(price.origin, "service_price");
    assert_eq!(price.source, BillingConfigSource::Legacy);
    assert_eq!(
        price.metric,
        BillingMetric::Requests,
        "legacy HTTP metric, not the lane unit"
    );
    assert_eq!(price.lago_metric_code, "platform_svc_svc");
    assert_eq!(price.credits_per_unit.as_deref(), Some("0.75"));
    let pending: Vec<_> = master
        .pending
        .iter()
        .map(|p| (p.origin, p.lago_metric_code.as_str()))
        .collect();
    assert_eq!(
        pending,
        vec![
            ("lane_primary", "svc_pk"),
            ("lane_component", "svc_pk_images")
        ]
    );
    assert_eq!(master.pending[0].sync_error.as_deref(), Some("lago down"));

    // The BYOK lane is not configured, so own-key traffic is free even with the legacy opt-in.
    assert_eq!(
        class(&resolved, C::UserOwned).reason,
        EffectiveReason::LaneNotConfigured
    );
}

#[test]
fn unsynced_primary_without_legacy_opt_in_is_free() {
    let svc = service(ServiceBilling {
        byok_pricing: Some(lane(
            BillingMetric::Requests,
            "1",
            "svc_byok",
            PricingSyncStatus::Pending,
        )),
        ..Default::default()
    });
    let rates = HashMap::new();
    let own = resolve_service_billing(&env(&rates), &svc);
    let own = class(&own, C::UserOwned);
    assert_eq!(own.reason, EffectiveReason::LaneUnsyncedFree);
    assert_eq!(own.source, BillingConfigSource::Free);
    assert!(!own.platform_charged && own.prices.is_empty());
    assert_eq!(own.pending[0].sync_status, Some(PricingSyncStatus::Pending));
}

#[test]
fn legacy_only_service_uses_the_cached_plan_rate_for_every_class() {
    let svc = service(ServiceBilling {
        platform_billable: true,
        ..Default::default()
    });
    let rates: HashMap<_, _> = [rate("platform_requests", 1_250_000_000_000, 3_600, false)].into();
    let resolved = resolve_service_billing(&env(&rates), &svc);
    // One group per lane: platform key, own-key classes, no credential.
    assert_eq!(
        resolved
            .classes
            .iter()
            .map(|group| group.lane)
            .collect::<Vec<_>>(),
        vec![Some("platform_key"), Some("byok"), None]
    );
    assert_eq!(
        resolved
            .classes
            .iter()
            .flat_map(|group| group.credential_classes.clone())
            .collect::<Vec<_>>(),
        CREDENTIAL_CLASSES.to_vec()
    );
    for all in &resolved.classes {
        assert_eq!(all.reason, EffectiveReason::LegacyPlatformBillable);
        assert_eq!(all.source, BillingConfigSource::Legacy);
        let price = &all.prices[0];
        assert_eq!(price.origin, "legacy_plan");
        assert_eq!(price.lago_metric_code, "platform_requests");
        assert_eq!(price.credits_per_unit.as_deref(), Some("1.25"));
        let cached = price.cached_rate.as_ref().expect("cached");
        assert!(cached.stale, "older than the 600s TTL");
        assert!(!cached.retired);
    }

    let not_billable = service(ServiceBilling::default());
    let resolved = resolve_service_billing(&env(&rates), &not_billable);
    assert!(resolved.classes.iter().all(|group| {
        group.reason == EffectiveReason::NotBillable
            && group.source == BillingConfigSource::Free
            && !group.platform_charged
    }));
}

#[test]
fn pending_legacy_price_keeps_the_plan_metric_and_reports_its_sync_state() {
    let svc = service(ServiceBilling {
        platform_billable: true,
        platform_pricing: Some(ServicePlatformPricing {
            credits_per_unit: "3".into(),
            lago_metric_code: "platform_svc_svc".into(),
            sync_status: PricingSyncStatus::Failed,
            sync_error: Some("plan update rejected".into()),
        }),
        ..Default::default()
    });
    let rates = HashMap::new();
    let resolved = resolve_service_billing(&env(&rates), &svc);
    let price = &resolved.classes[0].prices[0];
    assert_eq!(price.origin, "legacy_plan");
    assert_eq!(price.lago_metric_code, "platform_requests");
    assert_eq!(price.credits_per_unit, None, "no cached plan rate");
    assert_eq!(price.sync_status, Some(PricingSyncStatus::Failed));
    assert_eq!(price.sync_error.as_deref(), Some("plan update rejected"));
}

#[test]
fn nyxid_credentials_only_exempts_own_key_classes_but_not_the_shared_app() {
    let svc = service(ServiceBilling {
        platform_charge_nyxid_credentials_only: true,
        byok_pricing: Some(lane(
            BillingMetric::Requests,
            "1",
            "svc_byok",
            PricingSyncStatus::Synced,
        )),
        ..Default::default()
    });
    let rates = HashMap::new();
    let resolved = resolve_service_billing(&env(&rates), &svc);
    let shared_app = class(&resolved, C::NyxidPlatformOauthApp);
    assert_eq!(
        shared_app.credential_classes,
        vec![C::NyxidPlatformOauthApp]
    );
    assert_eq!(shared_app.reason, EffectiveReason::LaneSynced);
    assert!(shared_app.platform_charged);
    for credential in [C::UserOwned, C::AgentOverrideUserOwned, C::NodeManaged] {
        let own = class(&resolved, credential);
        assert_eq!(own.reason, EffectiveReason::NyxidCredentialsOnly);
        assert!(!own.platform_charged && own.prices.is_empty() && own.pending.is_empty());
    }
}

#[test]
fn operation_prices_apply_only_to_a_synced_requests_lane() {
    let mut byok = lane(
        BillingMetric::Tokens,
        "0.001",
        "svc_byok",
        PricingSyncStatus::Synced,
    );
    byok.operations = vec![operation(
        "send_dm",
        "0.5",
        "svc_op",
        PricingSyncStatus::Synced,
    )];
    let svc = service(ServiceBilling {
        byok_pricing: Some(byok),
        ..Default::default()
    });
    let rates = HashMap::new();
    let resolved = resolve_service_billing(&env(&rates), &svc);
    let own = class(&resolved, C::UserOwned);
    assert!(own.prices.iter().all(|p| p.origin != "operation"));
    assert_eq!(own.pending.len(), 1);
    assert_eq!(own.pending[0].origin, "operation");
    assert_eq!(own.pending[0].operation.as_deref(), Some("send_dm"));
}

#[test]
fn environment_switches_override_every_service_value() {
    let svc = service(ServiceBilling {
        platform_billable: true,
        resale_billable: true,
        resale_metric: BillingMetric::Tokens,
        lago_resale_metric_code: Some("resale_tokens".into()),
        ..Default::default()
    });
    let rates: HashMap<_, _> = [rate("resale_tokens", 5_000_000, 0, true)].into();
    let mut environment = env(&rates);
    let resolved = resolve_service_billing(&environment, &svc);
    let master = class(&resolved, C::NyxidManagedMaster);
    let resale = master
        .prices
        .iter()
        .find(|p| p.layer == "resale")
        .expect("resale layer");
    assert_eq!(resale.source, BillingConfigSource::Service);
    assert_eq!(resale.credits_per_unit.as_deref(), Some("0.000005"));
    assert!(resale.cached_rate.as_ref().unwrap().retired);
    assert!(
        class(&resolved, C::UserOwned)
            .prices
            .iter()
            .all(|p| p.layer != "resale")
    );
    assert_eq!(resolved.resale.source, BillingConfigSource::Service);
    assert!(resolved.resale.applied);

    environment.resale_enabled = false;
    let resolved = resolve_service_billing(&environment, &svc);
    assert!(
        resolved
            .classes
            .iter()
            .all(|g| g.prices.iter().all(|p| p.layer != "resale"))
    );
    assert_eq!(resolved.resale.source, BillingConfigSource::Env);
    assert!(resolved.resale.configured && !resolved.resale.applied);

    environment.resale_enabled = true;
    environment.billing_enabled = false;
    let resolved = resolve_service_billing(&environment, &svc);
    assert!(resolved.classes.iter().all(|g| {
        g.reason == EffectiveReason::BillingDisabled
            && g.source == BillingConfigSource::Env
            && !g.platform_charged
    }));
}

/// The admin view must agree with the charge path for every configuration
/// shape and credential class: same chargeability and the same Lago codes.
#[test]
fn resolver_matches_the_charge_path_for_every_lane_rule() {
    use PricingSyncStatus::{Failed, Pending, Synced};
    let mut shapes = Vec::new();
    for platform_billable in [false, true] {
        for nyxid_only in [false, true] {
            for platform_pricing in [None, Some(Synced), Some(Failed)] {
                for byok in [None, Some(Synced), Some(Pending)] {
                    for pk in [None, Some(Synced), Some(Failed)] {
                        let mut billing = ServiceBilling {
                            platform_billable,
                            platform_charge_nyxid_credentials_only: nyxid_only,
                            ..Default::default()
                        };
                        billing.platform_pricing =
                            platform_pricing.map(|status| ServicePlatformPricing {
                                credits_per_unit: "1".into(),
                                lago_metric_code: "platform_svc_svc".into(),
                                sync_status: status,
                                sync_error: None,
                            });
                        billing.byok_pricing = byok.map(|status| {
                            let mut lane = lane(BillingMetric::Requests, "1", "svc_byok", status);
                            lane.components = vec![component(
                                BillingMetric::Bytes,
                                "1",
                                "svc_byok_bytes",
                                Synced,
                            )];
                            lane.operations = vec![operation("op", "2", "svc_byok_op", Synced)];
                            lane
                        });
                        billing.platform_key_pricing =
                            pk.map(|status| lane(BillingMetric::Tokens, "1", "svc_pk", status));
                        shapes.push(billing);
                    }
                }
            }
        }
    }
    let rates = HashMap::new();
    for billing in shapes {
        let svc = service(billing.clone());
        let resolved = resolve_service_billing(&env(&rates), &svc);
        for credential in CREDENTIAL_CLASSES {
            let ctx = route_context(&svc, credential, true);
            let group = class(&resolved, credential);
            assert_eq!(
                group.platform_charged, ctx.service_platform_billable,
                "{credential:?} {billing:?}"
            );
            let expected: Vec<String> = if ctx.service_platform_billable {
                ctx.platform_specs()
                    .map(|(_, code)| code.to_string())
                    .chain(ctx.operation_prices().iter().map(|(_, code)| code.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            let actual: Vec<String> = group
                .prices
                .iter()
                .filter(|p| p.layer == "platform")
                .map(|p| p.lago_metric_code.clone())
                .collect();
            assert_eq!(actual, expected, "{credential:?} {billing:?}");
        }
    }
}

#[test]
fn global_overdraft_cap_wins_over_the_environment_default() {
    assert_eq!(
        effective_overdraft_cap(None, 4),
        (4, BillingConfigSource::Env)
    );
    let unset = BillingSettings::default();
    assert_eq!(
        effective_overdraft_cap(Some(&unset), 4),
        (4, BillingConfigSource::Env)
    );
    let set = BillingSettings {
        default_overdraft_cap_credits: Some(0),
    };
    assert_eq!(
        effective_overdraft_cap(Some(&set), 4),
        (0, BillingConfigSource::Global)
    );
}

#[test]
fn global_patch_keeps_clears_or_sets_each_field() {
    let stored = BillingSettings {
        default_overdraft_cap_credits: Some(9),
    };
    let keep = GlobalBillingPatch::default();
    assert!(keep.validate().is_err(), "an empty patch is rejected");
    assert_eq!(keep.apply(Some(&stored)), stored);
    let clear = GlobalBillingPatch {
        default_overdraft_cap_credits: Some(None),
    };
    assert_eq!(
        clear.apply(Some(&stored)).default_overdraft_cap_credits,
        None
    );
    let set = GlobalBillingPatch {
        default_overdraft_cap_credits: Some(Some(12)),
    };
    assert_eq!(set.apply(None).default_overdraft_cap_credits, Some(12));
    for bad in [-1, MAX_DEFAULT_OVERDRAFT_CAP_CREDITS + 1] {
        let error = GlobalBillingPatch {
            default_overdraft_cap_credits: Some(Some(bad)),
        }
        .validate()
        .unwrap_err();
        assert!(
            error.to_string().contains(&format!(
                "between 0 and {MAX_DEFAULT_OVERDRAFT_CAP_CREDITS}"
            )),
            "{error}"
        );
    }
    assert!(
        GlobalBillingPatch {
            default_overdraft_cap_credits: Some(Some(MAX_DEFAULT_OVERDRAFT_CAP_CREDITS)),
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn source_vocabulary_is_stable() {
    let names: Vec<String> = [
        BillingConfigSource::Env,
        BillingConfigSource::Global,
        BillingConfigSource::Service,
        BillingConfigSource::Legacy,
        BillingConfigSource::Free,
    ]
    .iter()
    .map(|source| {
        serde_json::to_value(source)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();
    assert_eq!(names, ["env", "global", "service", "legacy", "free"]);
}
