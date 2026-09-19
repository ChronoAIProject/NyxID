use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::BillingRateCache;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_billing::{LanePriceComponent, PricingSyncStatus, ServiceBilling};

use super::lago_client::{LagoApi, ServicePriceSync};

use super::amounts::{MAX_PRICE_PICO, PRICE_FRACTIONAL_DIGITS, decimal_to_pico, format_pico};
const MAX_PENDING_SYNC_BATCH: i64 = 100;

pub fn normalize_platform_pricing(
    service_slug: &str,
    current: Option<&ServiceBilling>,
    requested: &mut ServiceBilling,
) -> AppResult<()> {
    let Some(pricing) = requested.platform_pricing.as_mut() else {
        requested.platform_pricing_cleanup_metric_code = current
            .and_then(|billing| billing.platform_pricing.as_ref())
            .map(|pricing| pricing.lago_metric_code.clone())
            .filter(|code| !code.trim().is_empty())
            .or_else(|| {
                current.and_then(|billing| billing.platform_pricing_cleanup_metric_code.clone())
            });
        return Ok(());
    };

    pricing.credits_per_unit = normalize_price(&pricing.credits_per_unit)?;
    pricing.lago_metric_code = current
        .and_then(|billing| billing.platform_pricing.as_ref())
        .map(|pricing| pricing.lago_metric_code.clone())
        .filter(|code| !code.trim().is_empty())
        .unwrap_or_else(|| metric_code_for_service(service_slug));
    pricing.sync_status = PricingSyncStatus::Pending;
    pricing.sync_error = None;
    requested.platform_pricing_cleanup_metric_code = None;
    Ok(())
}

pub fn normalize_price(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(AppError::ValidationError(
            "billing.platform_pricing.credits_per_unit is required".to_string(),
        ));
    }
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|ch| ch.is_ascii_digit())
        || fraction.is_some_and(|part| {
            part.is_empty()
                || part.len() > PRICE_FRACTIONAL_DIGITS
                || !part.chars().all(|ch| ch.is_ascii_digit())
        })
    {
        return Err(AppError::ValidationError(
            "billing.platform_pricing.credits_per_unit must be a non-negative decimal with at most 12 fractional digits"
                .to_string(),
        ));
    }
    // Syntax is already validated; parse overflow also means above the price cap.
    let pico = decimal_to_pico(value)
        .filter(|pico| *pico <= MAX_PRICE_PICO)
        .ok_or_else(|| {
            AppError::ValidationError(
                "billing.platform_pricing.credits_per_unit must not exceed 1,000,000 credits"
                    .to_string(),
            )
        })?;
    Ok(format_pico(pico))
}

pub fn metric_code_for_service(service_slug: &str) -> String {
    format!("platform_svc_{service_slug}")
}

/// Sync all independently authored charges. Keep the legacy path unchanged.
pub async fn sync_service_price(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    service: &DownstreamService,
) -> AppResult<bool> {
    let mut changed = cleanup_components(db, lago, plan_code, service).await?;
    changed |= sync_legacy_price(db, lago, plan_code, service).await?;
    for field in ["byok_pricing", "platform_key_pricing"] {
        changed |= sync_lane_price(db, lago, plan_code, service, field).await?;
    }
    Ok(changed)
}

pub fn normalize_lane_pricing(
    slug: &str,
    current: Option<&ServiceBilling>,
    requested: &mut ServiceBilling,
) -> AppResult<()> {
    requested.component_cleanup_metric_codes = current
        .map(|b| b.component_cleanup_metric_codes.clone())
        .unwrap_or_default();
    if requested
        .platform_metric
        .is_some_and(|metric| !metric.is_legacy())
        || !requested.resale_metric.is_legacy()
    {
        return Err(AppError::ValidationError(
            "Legacy platform_metric and resale_metric support only tokens, requests and bytes"
                .into(),
        ));
    }
    for (lane, cleanup, previous, previous_cleanup, suffix) in [
        (
            &mut requested.byok_pricing,
            &mut requested.byok_pricing_cleanup_metric_code,
            current.and_then(|b| b.byok_pricing.as_ref()),
            current.and_then(|b| b.byok_pricing_cleanup_metric_code.as_ref()),
            "byok",
        ),
        (
            &mut requested.platform_key_pricing,
            &mut requested.platform_key_pricing_cleanup_metric_code,
            current.and_then(|b| b.platform_key_pricing.as_ref()),
            current.and_then(|b| b.platform_key_pricing_cleanup_metric_code.as_ref()),
            "pk",
        ),
    ] {
        if let Some(lane) = lane {
            lane.credits_per_unit = normalize_price(&lane.credits_per_unit)?;
            if let Some(previous) = previous.filter(|p| {
                p.metric == lane.metric
                    && p.credits_per_unit == lane.credits_per_unit
                    && !p.lago_metric_code.is_empty()
            }) {
                lane.lago_metric_code = previous.lago_metric_code.clone();
                lane.sync_status = previous.sync_status;
                lane.sync_error = previous.sync_error.clone();
            } else {
                lane.lago_metric_code = previous
                    .map(|p| p.lago_metric_code.clone())
                    .filter(|code| !code.is_empty())
                    .unwrap_or_else(|| format!("platform_svc_{slug}_{suffix}"));
                lane.sync_status = PricingSyncStatus::Pending;
                lane.sync_error = None;
            }
            let mut metrics = vec![lane.metric];
            for component in &mut lane.components {
                if metrics.contains(&component.metric) {
                    return Err(AppError::ValidationError(
                        "Billing metrics must be unique within each lane".into(),
                    ));
                }
                metrics.push(component.metric);
                component.credits_per_unit = normalize_price(&component.credits_per_unit)?;
                let old = previous
                    .and_then(|p| p.components.iter().find(|c| c.metric == component.metric));
                component.lago_metric_code =
                    format!("platform_svc_{slug}_{suffix}_{}", component.metric.as_str());
                component.sync_status = PricingSyncStatus::Pending;
                component.sync_error = None;
                if let Some(old) = old.filter(|p| {
                    p.credits_per_unit == component.credits_per_unit
                        && p.lago_metric_code == component.lago_metric_code
                }) {
                    *component = old.clone();
                }
            }
            *cleanup = None;
        } else {
            *cleanup = previous
                .map(|p| &p.lago_metric_code)
                .filter(|c| !c.is_empty())
                .or(previous_cleanup)
                .cloned();
        }
        if let Some(previous) = previous {
            for component in &previous.components {
                if !lane.as_ref().is_some_and(|l| {
                    l.components
                        .iter()
                        .any(|c| c.lago_metric_code == component.lago_metric_code)
                }) && !requested
                    .component_cleanup_metric_codes
                    .contains(&component.lago_metric_code)
                {
                    requested
                        .component_cleanup_metric_codes
                        .push(component.lago_metric_code.clone());
                }
            }
        }
    }
    Ok(())
}

async fn sync_lane_price(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    service: &DownstreamService,
    field: &str,
) -> AppResult<bool> {
    let Some(billing) = &service.billing else {
        return Ok(false);
    };
    let (lane, cleanup) = match field {
        "byok_pricing" => (
            &billing.byok_pricing,
            &billing.byok_pricing_cleanup_metric_code,
        ),
        _ => (
            &billing.platform_key_pricing,
            &billing.platform_key_pricing_cleanup_metric_code,
        ),
    };
    let path = format!("billing.{field}");
    let cleanup_path = format!("{path}_cleanup_metric_code");
    let collection = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let Some(lane) = lane else {
        let Some(code) = cleanup.as_deref().filter(|c| !c.is_empty()) else {
            return Ok(false);
        };
        if lago.remove_standard_charge(plan_code, code).await.is_err() {
            return Ok(false);
        }
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .delete_many(doc! { "lago_metric_code": code })
            .await?;
        // A concurrent re-add must be synchronized again after this removal.
        collection
            .update_one(
                doc! { "_id": &service.id, format!("{path}.lago_metric_code"): code },
                doc! { "$set": { format!("{path}.sync_status"): "pending" } },
            )
            .await?;
        collection
            .update_one(
                doc! { "_id": &service.id, &cleanup_path: code, &path: bson::Bson::Null },
                doc! { "$unset": { &cleanup_path: "" } },
            )
            .await?;
        return Ok(true);
    };
    let primary = LanePriceComponent {
        metric: lane.metric,
        credits_per_unit: lane.credits_per_unit.clone(),
        lago_metric_code: lane.lago_metric_code.clone(),
        sync_status: lane.sync_status,
        sync_error: lane.sync_error.clone(),
    };
    let mut changed =
        sync_lane_component(db, lago, plan_code, service, field, &path, &primary).await?;
    for (index, component) in lane.components.iter().enumerate() {
        changed |= sync_lane_component(
            db,
            lago,
            plan_code,
            service,
            field,
            &format!("{path}.components.{index}"),
            component,
        )
        .await?;
    }
    Ok(changed)
}

#[allow(clippy::too_many_arguments)]
async fn sync_lane_component(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    service: &DownstreamService,
    field: &str,
    path: &str,
    lane: &LanePriceComponent,
) -> AppResult<bool> {
    let collection = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let input = ServicePriceSync {
        metric_code: lane.lago_metric_code.clone(),
        metric_name: format!("{} {field} {}", service.name, lane.metric.label()),
        metric_description: format!("NyxID {field} usage for {}", service.slug),
        credits_per_unit: lane.credits_per_unit.clone(),
    };
    let synced = lago.sync_standard_charge(plan_code, &input).await.is_ok();
    // Include the unit: identical prices in a different unit are different charges.
    let filter = doc! { "_id": &service.id,
        format!("{path}.lago_metric_code"): &lane.lago_metric_code,
        format!("{path}.credits_per_unit"): &lane.credits_per_unit,
        format!("{path}.metric"): bson::to_bson(&lane.metric).expect("metric serialization"),
    };
    if synced {
        let micros = super::lago_client::decimal_credits_to_micros(&lane.credits_per_unit)
            .ok_or_else(|| AppError::Internal("stored lane price is invalid".to_string()))?;
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .replace_one(
                doc! { "_id": BillingRateCache::cache_id(&lane.lago_metric_code, None) },
                BillingRateCache {
                    id: BillingRateCache::cache_id(&lane.lago_metric_code, None),
                    lago_metric_code: lane.lago_metric_code.clone(),
                    model: None,
                    credits_per_unit_micros: micros,
                    credits_per_unit_pico: decimal_to_pico(&lane.credits_per_unit),
                    synced_at: Utc::now(),
                },
            )
            .upsert(true)
            .await?;
    }
    let result = collection.update_one(filter, doc! { "$set": {
        format!("{path}.sync_status"): if synced { "synced" } else { "failed" },
        format!("{path}.sync_error"): if synced { bson::Bson::Null } else { bson::Bson::String("Lago price synchronization failed; reconciliation will retry".to_string()) },
    } }).await?;
    if result.matched_count == 0 {
        // A stale upstream write cannot activate a newer price. If removed, keep
        // cleanup durable even when a previous cleanup completed during sync.
        if path.contains(".components.") {
            collection.update_one(doc! { "_id": &service.id,
                format!("billing.{field}.components"): { "$not": { "$elemMatch": { "lago_metric_code": &lane.lago_metric_code } } },
            }, doc! { "$addToSet": { "billing.component_cleanup_metric_codes": &lane.lago_metric_code } }).await?;
        } else {
            collection.update_one(doc! { "_id": &service.id, path: bson::Bson::Null },
                doc! { "$set": { format!("{path}_cleanup_metric_code"): &lane.lago_metric_code } }).await?;
        }
        mark_live_price_pending(db, &service.id, &lane.lago_metric_code).await?;
        return Ok(false);
    }
    Ok(synced)
}

async fn mark_live_price_pending(
    db: &mongodb::Database,
    service_id: &str,
    code: &str,
) -> AppResult<()> {
    let collection = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    for field in ["byok_pricing", "platform_key_pricing"] {
        collection
            .update_one(
                doc! { "_id": service_id, format!("billing.{field}.lago_metric_code"): code },
                doc! { "$set": { format!("billing.{field}.sync_status"): "pending" } },
            )
            .await?;
        collection.update_one(doc! { "_id": service_id, format!("billing.{field}.components.lago_metric_code"): code },
            doc! { "$set": { format!("billing.{field}.components.$[component].sync_status"): "pending" } })
            .array_filters(vec![doc! { "component.lago_metric_code": code }]).await?;
    }
    Ok(())
}

async fn cleanup_components(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    service: &DownstreamService,
) -> AppResult<bool> {
    let Some(billing) = &service.billing else {
        return Ok(false);
    };
    let mut changed = false;
    for code in &billing.component_cleanup_metric_codes {
        if lago.remove_standard_charge(plan_code, code).await.is_err() {
            continue;
        }
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .delete_many(doc! { "lago_metric_code": code })
            .await?;
        mark_live_price_pending(db, &service.id, code).await?;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$pull": { "billing.component_cleanup_metric_codes": code } },
            )
            .await?;
        changed = true;
    }
    Ok(changed)
}

async fn sync_legacy_price(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    service: &DownstreamService,
) -> AppResult<bool> {
    let Some(billing) = service.billing.as_ref() else {
        return Ok(false);
    };
    let Some(pricing) = billing.platform_pricing.as_ref() else {
        let Some(metric_code) = billing
            .platform_pricing_cleanup_metric_code
            .as_deref()
            .filter(|code| !code.trim().is_empty())
        else {
            return Ok(false);
        };
        match lago.remove_standard_charge(plan_code, metric_code).await {
            Ok(()) => {
                complete_price_removal(db, &service.id, metric_code).await?;
                return Ok(true);
            }
            Err(error) => {
                tracing::warn!(
                    service_id = %service.id,
                    service_slug = %service.slug,
                    metric_code,
                    error = %error,
                    "Service price removal failed; reconciliation will retry"
                );
                return Ok(false);
            }
        }
    };
    let input = ServicePriceSync {
        metric_code: pricing.lago_metric_code.clone(),
        metric_name: format!("{} platform usage", service.name),
        metric_description: format!(
            "NyxID-managed platform usage price for catalog service {}",
            service.slug
        ),
        credits_per_unit: pricing.credits_per_unit.clone(),
    };

    match lago.sync_standard_charge(plan_code, &input).await {
        Ok(()) => {
            let micros = super::lago_client::decimal_credits_to_micros(&pricing.credits_per_unit)
                .ok_or_else(|| {
                AppError::Internal("stored service price is invalid".to_string())
            })?;
            if !set_sync_state(
                db,
                &service.id,
                &pricing.lago_metric_code,
                &pricing.credits_per_unit,
                PricingSyncStatus::Synced,
                None,
            )
            .await?
            {
                // A newer admin save won the race while this Lago request
                // was in flight. The older provider write may have landed
                // last, so force the current value back through reconcile.
                db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                    .update_one(
                        doc! {
                            "_id": &service.id,
                            "billing.platform_pricing.lago_metric_code": &pricing.lago_metric_code,
                        },
                        doc! { "$set": {
                            "billing.platform_pricing.sync_status": "pending",
                            "billing.platform_pricing.sync_error": bson::Bson::Null,
                            "updated_at": bson::DateTime::from_chrono(Utc::now()),
                        } },
                    )
                    .await?;
                return Ok(false);
            }
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
                .replace_one(
                    doc! { "_id": BillingRateCache::cache_id(&pricing.lago_metric_code, None) },
                    BillingRateCache {
                        id: BillingRateCache::cache_id(&pricing.lago_metric_code, None),
                        lago_metric_code: pricing.lago_metric_code.clone(),
                        model: None,
                        credits_per_unit_micros: micros,
                        credits_per_unit_pico: decimal_to_pico(&pricing.credits_per_unit),
                        synced_at: Utc::now(),
                    },
                )
                .upsert(true)
                .await?;
            Ok(true)
        }
        Err(error) => {
            let public_error = "Lago price synchronization failed; the reconcile sweep will retry";
            set_sync_state(
                db,
                &service.id,
                &pricing.lago_metric_code,
                &pricing.credits_per_unit,
                PricingSyncStatus::Failed,
                Some(public_error),
            )
            .await?;
            tracing::warn!(
                service_id = %service.id,
                service_slug = %service.slug,
                metric_code = %pricing.lago_metric_code,
                error = %error,
                "Service price synchronization failed"
            );
            Ok(false)
        }
    }
}

async fn complete_price_removal(
    db: &mongodb::Database,
    service_id: &str,
    metric_code: &str,
) -> AppResult<()> {
    // Keep the cleanup marker until after the cache delete. A crash between
    // these writes makes reconciliation repeat an idempotent Lago removal and
    // cache delete instead of permanently orphaning the local rate.
    db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .delete_many(doc! { "lago_metric_code": metric_code })
        .await?;
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! {
                "_id": service_id,
                "billing.platform_pricing_cleanup_metric_code": metric_code,
            },
            doc! { "$unset": {
                "billing.platform_pricing_cleanup_metric_code": "",
            } },
        )
        .await?;
    Ok(())
}

pub async fn retry_pending_service_prices(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
) -> AppResult<u64> {
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {
            "$or": [
                { "billing.component_cleanup_metric_codes.0": { "$exists": true } },
                { "billing.byok_pricing.components.sync_status": { "$in": ["pending", "failed"] } },
                { "billing.platform_key_pricing.components.sync_status": { "$in": ["pending", "failed"] } },
                { "billing.byok_pricing.sync_status": { "$in": ["pending", "failed"] } },
                { "billing.platform_key_pricing.sync_status": { "$in": ["pending", "failed"] } },
                { "billing.byok_pricing_cleanup_metric_code": { "$type": "string", "$ne": "" } },
                { "billing.platform_key_pricing_cleanup_metric_code": { "$type": "string", "$ne": "" } },
                {
                    "billing.platform_pricing.sync_status": { "$in": ["pending", "failed"] },
                    "billing.platform_pricing.credits_per_unit": { "$type": "string" },
                },
                {
                    "billing.platform_pricing_cleanup_metric_code": {
                        "$type": "string",
                        "$ne": "",
                    },
                },
            ],
        })
        .limit(MAX_PENDING_SYNC_BATCH)
        .await?
        .try_collect()
        .await?;
    let mut synced = 0;
    for service in services {
        if sync_service_price(db, lago, plan_code, &service).await? {
            synced += 1;
        }
    }
    Ok(synced)
}

pub(crate) async fn set_sync_state(
    db: &mongodb::Database,
    service_id: &str,
    metric_code: &str,
    credits_per_unit: &str,
    status: PricingSyncStatus,
    error: Option<&str>,
) -> AppResult<bool> {
    let mut set = doc! {
        "billing.platform_pricing.sync_status": bson::to_bson(&status).map_err(|err| {
            AppError::Internal(format!("failed to encode price sync status: {err}"))
        })?,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    set.insert(
        "billing.platform_pricing.sync_error",
        error.map_or(bson::Bson::Null, |value| {
            bson::Bson::String(value.to_string())
        }),
    );
    let result = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! {
                "_id": service_id,
                "billing.platform_pricing.lago_metric_code": metric_code,
                "billing.platform_pricing.credits_per_unit": credits_per_unit,
            },
            doc! { "$set": set },
        )
        .await?;
    Ok(result.matched_count == 1)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use mongodb::bson::doc;

    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::test_utils::connect_test_database;

    use crate::models::service_billing::{
        PricingSyncStatus, ServiceBilling, ServicePlatformPricing,
    };

    use super::{
        DOWNSTREAM_SERVICES, complete_price_removal, metric_code_for_service,
        normalize_platform_pricing, normalize_price,
    };

    #[test]
    fn prices_above_the_cap_have_one_validation_message_even_on_overflow() {
        for price in [
            "1000000.000000000001",
            "1000001",
            "9223373",
            "9223372036854775808",
            "999999999999999999999999999999999999999",
        ] {
            let error = normalize_price(price).unwrap_err();
            assert!(
                matches!(error, crate::errors::AppError::ValidationError(ref message)
                if message == "billing.platform_pricing.credits_per_unit must not exceed 1,000,000 credits")
            );
        }
        assert_eq!(normalize_price("1000000.000000000000").unwrap(), "1000000");
    }

    #[test]
    fn price_normalization_is_exact_and_bounded() {
        assert_eq!(normalize_price("0").expect("zero"), "0");
        assert_eq!(normalize_price("001.250000").expect("decimal"), "1.25");
        assert!(normalize_price("-1").is_err());
        assert_eq!(normalize_price("1.000000000001").unwrap(), "1.000000000001");
        assert!(normalize_price("1.0000000000001").is_err());
        assert!(normalize_price("1000001").is_err());
    }

    #[test]
    fn service_metric_code_uses_stable_vendor_prefix() {
        assert_eq!(
            metric_code_for_service("llm-openai"),
            "platform_svc_llm-openai"
        );
    }

    #[test]
    fn clearing_price_persists_metric_cleanup_marker() {
        let current = ServiceBilling {
            platform_pricing: Some(ServicePlatformPricing {
                credits_per_unit: "0.125".to_string(),
                lago_metric_code: "platform_svc_llm-openai".to_string(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
            }),
            ..Default::default()
        };
        let mut requested = ServiceBilling::default();

        normalize_platform_pricing("llm-openai", Some(&current), &mut requested)
            .expect("normalize clear");

        assert!(requested.platform_pricing.is_none());
        assert_eq!(
            requested.platform_pricing_cleanup_metric_code.as_deref(),
            Some("platform_svc_llm-openai")
        );
    }

    #[tokio::test]
    async fn completed_price_removal_deletes_rate_and_cleanup_marker() {
        let Some(db) = connect_test_database("service_price_cleanup").await else {
            return;
        };
        let metric_code = "platform_svc_llm-openai";
        db.collection::<mongodb::bson::Document>(DOWNSTREAM_SERVICES)
            .insert_one(doc! {
                "_id": "service-1",
                "billing": {
                    "platform_pricing_cleanup_metric_code": metric_code,
                },
            })
            .await
            .expect("insert service cleanup marker");
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id(metric_code, None),
                lago_metric_code: metric_code.to_string(),
                model: None,
                credits_per_unit_micros: 125_000,
                credits_per_unit_pico: None,
                synced_at: Utc::now(),
            })
            .await
            .expect("insert stale service rate");

        complete_price_removal(&db, "service-1", metric_code)
            .await
            .expect("complete price removal");

        let service = db
            .collection::<mongodb::bson::Document>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": "service-1" })
            .await
            .expect("find service")
            .expect("service exists");
        assert!(
            service
                .get_document("billing")
                .expect("billing")
                .get("platform_pricing_cleanup_metric_code")
                .is_none()
        );
        assert_eq!(
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME,)
                .count_documents(doc! { "lago_metric_code": metric_code })
                .await
                .expect("count rates"),
            0
        );
    }
}
