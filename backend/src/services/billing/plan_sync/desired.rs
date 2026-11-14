//! Rebuild the full managed set, including previously synced and retained
//! immutable prices. Unmanaged Lago charges are never added to this projection.
use super::ServicePriceSync;
use crate::{
    errors::AppResult,
    models::{
        billing_plan_sync::{CHARGE_COLLECTION, PlanCharge},
        billing_rate_cache::COLLECTION_NAME as RATES,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService, PlatformKeyPurpose},
        machine_runtime_billing::{MachineRuntimeRate, RATE_COLLECTION},
        platform_model_tariff::{COLLECTION_NAME as TARIFFS, ModelTariff, TariffStatus},
        sandbox_rate_card::{COLLECTION_NAME as CARDS, SandboxRateCard},
        service_billing::PricingSyncStatus,
    },
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Desired = BTreeMap<String, Option<ServicePriceSync>>;
fn insert(set: &mut Desired, code: &str, price: &str, name: String, description: String) {
    if !code.is_empty() {
        set.insert(
            code.into(),
            Some(ServicePriceSync {
                metric_code: code.into(),
                metric_name: name,
                metric_description: description,
                credits_per_unit: price.into(),
            }),
        );
    }
}
fn remove(set: &mut Desired, code: Option<&str>) {
    if let Some(code) = code.filter(|c| !c.is_empty()) {
        set.insert(code.into(), None);
    }
}
pub(super) struct Snapshot {
    pub charges: Desired,
    pub owners: BTreeMap<String, String>,
    pub cache_codes: BTreeSet<String>,
    pub intents: BTreeMap<String, PlanCharge>,
}

pub(super) async fn load(db: &Database, plan: &str, configured_plan: &str) -> AppResult<Snapshot> {
    let mut set = Desired::new();
    let mut owners = BTreeMap::new();
    let mut cache_codes = BTreeSet::new();
    let mut records = BTreeMap::new();
    let mut intents = db
        .collection::<PlanCharge>(CHARGE_COLLECTION)
        .find(doc! {"plan_code":plan})
        .await?;
    while let Some(intent) = intents.try_next().await? {
        if let Some(owner) = &intent.service_id {
            owners.insert(intent.metric_code.clone(), owner.clone());
        }
        // An attempted positive intent is not evidence of publication. Only
        // synced source rows below may publish prices/cache. Tombstones survive
        // source cleanup and deletion so removed charges cannot reappear.
        if intent.credits_per_unit.is_none() {
            set.insert(intent.metric_code.clone(), None);
            if intent.writes_rate_cache {
                cache_codes.insert(intent.metric_code.clone());
            }
        }
        records.insert(intent.metric_code.clone(), intent);
    }
    if plan == configured_plan {
        let mut services = db
            .collection::<DownstreamService>(SERVICES)
            .find(doc! {"$or":[{"billing":{"$type":"object"}}, {"_id":{"$in":owners.values().collect::<Vec<_>>()}}]})
            .await?;
        while let Some(service) = services.try_next().await? {
            if service
                .platform_key
                .as_ref()
                .is_some_and(|key| key.purpose == PlatformKeyPurpose::ModelOffering)
            {
                continue;
            }
            for (code, owner) in &owners {
                if owner == &service.id {
                    set.insert(code.clone(), None);
                }
            }
            let Some(billing) = service.billing else {
                continue;
            };
            let mut codes = billing.component_cleanup_metric_codes.clone();
            codes.extend(
                [
                    billing.platform_pricing_cleanup_metric_code.clone(),
                    billing.byok_pricing_cleanup_metric_code.clone(),
                    billing.platform_key_pricing_cleanup_metric_code.clone(),
                ]
                .into_iter()
                .flatten(),
            );
            if let Some(price) = &billing.platform_pricing {
                codes.push(price.lago_metric_code.clone());
            }
            for lane in [&billing.byok_pricing, &billing.platform_key_pricing]
                .into_iter()
                .flatten()
            {
                codes.push(lane.lago_metric_code.clone());
                codes.extend(lane.components.iter().map(|c| c.lago_metric_code.clone()));
                codes.extend(lane.operations.iter().map(|c| c.lago_metric_code.clone()));
            }
            for code in codes {
                cache_codes.insert(code.clone());
                owners.insert(code, service.id.clone());
            }
            for code in &billing.component_cleanup_metric_codes {
                remove(&mut set, Some(code));
            }
            remove(
                &mut set,
                billing.platform_pricing_cleanup_metric_code.as_deref(),
            );
            remove(
                &mut set,
                billing.byok_pricing_cleanup_metric_code.as_deref(),
            );
            remove(
                &mut set,
                billing.platform_key_pricing_cleanup_metric_code.as_deref(),
            );
            if let Some(price) = billing.platform_pricing {
                set.remove(&price.lago_metric_code);
                if price.sync_status == PricingSyncStatus::Synced {
                    insert(
                        &mut set,
                        &price.lago_metric_code,
                        &price.credits_per_unit,
                        format!("{} platform usage", service.name),
                        format!(
                            "NyxID-managed platform usage price for catalog service {}",
                            service.slug
                        ),
                    );
                }
            }
            for (field, lane) in [
                ("byok_pricing", billing.byok_pricing),
                ("platform_key_pricing", billing.platform_key_pricing),
            ] {
                let Some(lane) = lane else {
                    continue;
                };
                let description = format!("NyxID {field} usage for {}", service.slug);
                set.remove(&lane.lago_metric_code);
                if lane.sync_status == PricingSyncStatus::Synced {
                    insert(
                        &mut set,
                        &lane.lago_metric_code,
                        &lane.credits_per_unit,
                        format!("{} {field} {}", service.name, lane.metric.label()),
                        description.clone(),
                    );
                }
                for component in lane.components {
                    set.remove(&component.lago_metric_code);
                    if component.sync_status != PricingSyncStatus::Synced {
                        continue;
                    }
                    cache_codes.insert(component.lago_metric_code.clone());
                    insert(
                        &mut set,
                        &component.lago_metric_code,
                        &component.credits_per_unit,
                        format!("{} {field} {}", service.name, component.metric.label()),
                        description.clone(),
                    );
                }
                for operation in lane.operations {
                    set.remove(&operation.lago_metric_code);
                    if operation.sync_status != PricingSyncStatus::Synced {
                        continue;
                    }
                    insert(
                        &mut set,
                        &operation.lago_metric_code,
                        &operation.credits_per_unit,
                        format!("{} {field} {} requests", service.name, operation.operation),
                        description.clone(),
                    );
                }
            }
        }
        let tariffs: Vec<ModelTariff> = db
            .collection::<ModelTariff>(TARIFFS)
            .find(doc! {"status":{"$in":["synced","retired"]}})
            .await?
            .try_collect()
            .await?;
        let retired_codes: Vec<_> = tariffs
            .iter()
            .filter(|t| t.status == TariffStatus::Retired)
            .flat_map(|t| t.components.iter().map(|c| c.lago_metric_code.clone()))
            .collect();
        let installed: Vec<Document> = if retired_codes.is_empty() {
            Vec::new()
        } else {
            db.collection::<Document>(RATES)
                .find(doc! {"lago_metric_code":{"$in":retired_codes}})
                .projection(doc! {"lago_metric_code":1})
                .await?
                .try_collect()
                .await?
        };
        for tariff in tariffs {
            for component in tariff.components {
                if !component.explicit_free
                    && (tariff.status == TariffStatus::Synced
                        || installed.iter().any(|r| {
                            r.get_str("lago_metric_code").ok()
                                == Some(component.lago_metric_code.as_str())
                        }))
                {
                    cache_codes.insert(component.lago_metric_code.clone());
                    insert(
                        &mut set,
                        &component.lago_metric_code,
                        &component.credits_per_unit,
                        format!(
                            "{} {} t{}",
                            tariff.public_model, component.key, tariff.revision
                        ),
                        format!("NyxID model usage: {}", component.metric.label()),
                    );
                }
            }
        }
    }
    let mut cards = db
        .collection::<SandboxRateCard>(CARDS)
        .find(doc! {"plan_code":plan,"sync_status":"synced"})
        .await?;
    while let Some(card) = cards.try_next().await? {
        for price in card.prices {
            cache_codes.insert(price.lago_metric_code.clone());
            insert(
                &mut set,
                &price.lago_metric_code,
                &price.credits_per_unit.to_string(),
                format!("CMA {} v{}", price.metric.as_str(), card.version),
                format!("CMA allocated resource usage, rate card {}", card.id),
            );
        }
    }
    let mut rates = db
        .collection::<MachineRuntimeRate>(RATE_COLLECTION)
        .find(doc! {"plan_code":plan,"sync_status":"synced"})
        .await?;
    while let Some(rate) = rates.try_next().await? {
        insert(
            &mut set,
            &rate.lago_metric_code,
            &rate.credits_per_second.to_string(),
            "Machine running seconds".into(),
            "Fixed Remote Nyx Machine runtime; work disk and paused retention included".into(),
        );
    }
    Ok(Snapshot {
        charges: set,
        owners,
        cache_codes,
        intents: records,
    })
}
