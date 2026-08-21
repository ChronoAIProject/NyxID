use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::BillingRateCache;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_billing::{PricingSyncStatus, ServiceBilling};

use super::lago_client::{LagoApi, ServicePriceSync};

pub const MAX_PRICE_CREDITS_PER_UNIT_MICROS: i64 = 1_000_000 * 1_000_000;
const MAX_PENDING_SYNC_BATCH: i64 = 100;

pub fn normalize_platform_pricing(
    service_slug: &str,
    current: Option<&ServiceBilling>,
    requested: &mut ServiceBilling,
) -> AppResult<()> {
    let Some(pricing) = requested.platform_pricing.as_mut() else {
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
            part.is_empty() || part.len() > 6 || !part.chars().all(|ch| ch.is_ascii_digit())
        })
    {
        return Err(AppError::ValidationError(
            "billing.platform_pricing.credits_per_unit must be a non-negative decimal with at most 6 fractional digits"
                .to_string(),
        ));
    }
    let micros = super::lago_client::decimal_credits_to_micros(value).ok_or_else(|| {
        AppError::ValidationError(
            "billing.platform_pricing.credits_per_unit is outside the supported range".to_string(),
        )
    })?;
    if micros > MAX_PRICE_CREDITS_PER_UNIT_MICROS {
        return Err(AppError::ValidationError(format!(
            "billing.platform_pricing.credits_per_unit must not exceed {} credits",
            MAX_PRICE_CREDITS_PER_UNIT_MICROS / 1_000_000
        )));
    }
    Ok(format_micros(micros))
}

pub fn metric_code_for_service(service_slug: &str) -> String {
    format!("platform_svc_{service_slug}")
}

pub async fn sync_service_price(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    service: &DownstreamService,
) -> AppResult<bool> {
    let Some(pricing) = service
        .billing
        .as_ref()
        .and_then(|billing| billing.platform_pricing.as_ref())
    else {
        return Ok(false);
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

pub async fn retry_pending_service_prices(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
) -> AppResult<u64> {
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {
            "billing.platform_pricing.sync_status": { "$in": ["pending", "failed"] },
            "billing.platform_pricing.credits_per_unit": { "$type": "string" },
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

fn format_micros(micros: i64) -> String {
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:06}")
        .trim_end_matches('0')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{metric_code_for_service, normalize_price};

    #[test]
    fn price_normalization_is_exact_and_bounded() {
        assert_eq!(normalize_price("0").expect("zero"), "0");
        assert_eq!(normalize_price("001.250000").expect("decimal"), "1.25");
        assert!(normalize_price("-1").is_err());
        assert!(normalize_price("1.0000001").is_err());
        assert!(normalize_price("1000001").is_err());
    }

    #[test]
    fn service_metric_code_uses_stable_vendor_prefix() {
        assert_eq!(
            metric_code_for_service("llm-openai"),
            "platform_svc_llm-openai"
        );
    }
}
