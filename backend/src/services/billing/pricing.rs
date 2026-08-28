use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::BillingRateCache;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_billing::{PricingSyncStatus, ServiceBilling};
use crate::models::{
    platform_operation::{
        COLLECTION_NAME as PLATFORM_OPERATIONS, ConstrainedOp, OperationBilling,
        PlatformOperationKind, PlatformOperationRow,
    },
    service_billing::BillingMetric,
};

use super::lago_client::{LagoApi, ServicePriceSync};

pub const MAX_PRICE_CREDITS_PER_UNIT_MICROS: i64 = 1_000_000 * 1_000_000;
pub const MAX_OPERATION_METRIC_CODE_LEN: usize = 120;
const MAX_PENDING_SYNC_BATCH: i64 = 100;

pub fn normalize_platform_pricing(
    service_slug: &str,
    current: Option<&ServiceBilling>,
    requested: &mut ServiceBilling,
) -> AppResult<()> {
    if requested
        .platform_metric
        .is_some_and(|metric| matches!(metric, BillingMetric::Characters | BillingMetric::Seconds))
        || matches!(
            requested.resale_metric,
            BillingMetric::Characters | BillingMetric::Seconds
        )
    {
        return Err(AppError::ValidationError(
            "characters and seconds billing metrics are reserved for platform operations"
                .to_string(),
        ));
    }
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

pub fn metric_code_for_operation(catalog_slug: &str, kind_key: &str) -> String {
    let identity = format!("{catalog_slug}:{kind_key}");
    let mut slug = String::with_capacity(identity.len());
    let mut separated = false;
    for character in identity.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separated = false;
        } else if !separated && !slug.is_empty() {
            slug.push('_');
            separated = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("operation");
    }
    let mut code = format!("platform_op_{slug}");
    if code.len() <= MAX_OPERATION_METRIC_CODE_LEN {
        return code;
    }

    let digest = hex::encode(Sha256::digest(identity.as_bytes()));
    let suffix = &digest[..16];
    let prefix_len = MAX_OPERATION_METRIC_CODE_LEN - suffix.len() - 1;
    code.truncate(prefix_len);
    while code.ends_with('_') {
        code.pop();
    }
    format!("{code}_{suffix}")
}

pub fn normalize_operation_billing(
    catalog_slug: &str,
    kind_key: &str,
    kind: &PlatformOperationKind,
    _current: Option<&OperationBilling>,
    requested: &mut OperationBilling,
) -> AppResult<()> {
    if !operation_supports_metric(kind, requested.metric) {
        return Err(AppError::ValidationError(format!(
            "billing metric '{}' cannot be measured for this platform operation",
            requested.metric.as_str()
        )));
    }
    requested.price_per_unit = normalize_price(&requested.price_per_unit)?;
    requested.base_fee_per_call = requested
        .base_fee_per_call
        .as_deref()
        .map(normalize_price)
        .transpose()?;
    if requested.base_fee_per_call.is_some() && requested.price_per_unit == "0" {
        return Err(AppError::ValidationError(
            "billing base_fee_per_call requires a non-zero per-unit price".to_string(),
        ));
    }
    requested.lago_metric_code = metric_code_for_operation(catalog_slug, kind_key);
    requested.sync_status = PricingSyncStatus::Pending;
    requested.sync_error = None;
    Ok(())
}

/// Renders an operation price as one human-readable sentence fragment.
///
/// Every surface that shows a platform price -- the admin table, `/keys`, and
/// MCP tool descriptions -- formats through here so a `characters` or
/// `seconds` operation cannot be rendered as if it were priced per call. The
/// caller passes the `billable` decision it already made, because the
/// user-facing surfaces also gate on the billing rollout flag while the admin
/// surface reports the configured price regardless.
pub fn format_operation_price(billing: &OperationBilling, billable: bool) -> String {
    if !billable {
        return "Free".to_string();
    }
    let unit = billing.metric.unit_noun();
    match (&billing.base_fee_per_call, billing.price_per_unit.as_str()) {
        (_, "0") => "Price not set".to_string(),
        (Some(base), price) => {
            format!("{base} credits per call + {price} per {unit}")
        }
        (None, price) => format!("{price} credits per {unit}"),
    }
}

pub fn operation_supports_metric(kind: &PlatformOperationKind, metric: BillingMetric) -> bool {
    match kind {
        PlatformOperationKind::Endpoint { .. } => {
            matches!(metric, BillingMetric::Requests | BillingMetric::Bytes)
        }
        PlatformOperationKind::Constrained {
            op: ConstrainedOp::Speak,
            ..
        } => metric == BillingMetric::Characters,
        PlatformOperationKind::Constrained {
            op: ConstrainedOp::CallAndSay,
            ..
        } => metric == BillingMetric::Seconds,
        PlatformOperationKind::Constrained {
            op: ConstrainedOp::FlightSearch,
            ..
        } => metric == BillingMetric::Requests,
    }
}

pub async fn sync_service_price(
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

pub async fn sync_operation_price(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    row: &PlatformOperationRow,
) -> AppResult<bool> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": &row.catalog_service_id, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::PlatformVendorProvisioningInvalid(
                "The platform operation catalog service is missing or inactive.".to_string(),
            )
        })?;
    let input = ServicePriceSync {
        metric_code: row.billing.lago_metric_code.clone(),
        metric_name: format!("{} platform operation", service.name),
        metric_description: format!(
            "NyxID-managed platform operation {} for catalog service {}",
            row.kind_key, service.slug
        ),
        credits_per_unit: row.billing.price_per_unit.clone(),
    };

    match lago.sync_standard_charge(plan_code, &input).await {
        Ok(()) => {
            let micros = super::lago_client::decimal_credits_to_micros(&row.billing.price_per_unit)
                .ok_or_else(|| {
                    AppError::Internal("stored operation price is invalid".to_string())
                })?;
            if !set_operation_sync_state(db, row, PricingSyncStatus::Synced, None).await? {
                db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
                    .update_one(
                        doc! { "_id": &row.id },
                        doc! { "$set": {
                            "billing.sync_status": "pending",
                            "billing.sync_error": Bson::Null,
                        } },
                    )
                    .await?;
                return Ok(false);
            }
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
                .replace_one(
                    doc! { "_id": BillingRateCache::cache_id(&row.billing.lago_metric_code, None) },
                    BillingRateCache {
                        id: BillingRateCache::cache_id(&row.billing.lago_metric_code, None),
                        lago_metric_code: row.billing.lago_metric_code.clone(),
                        model: None,
                        credits_per_unit_micros: micros,
                        synced_at: Utc::now(),
                    },
                )
                .upsert(true)
                .await?;
            if let Some(cleanup_code) = row
                .billing_cleanup_metric_code
                .as_deref()
                .filter(|code| !code.trim().is_empty())
            {
                if cleanup_code != row.billing.lago_metric_code {
                    if let Err(error) = lago.remove_standard_charge(plan_code, cleanup_code).await {
                        tracing::warn!(
                            operation_id = %row.id,
                            metric_code = cleanup_code,
                            error = %error,
                            "Platform operation price cleanup failed; reconciliation will retry"
                        );
                        return Ok(false);
                    }
                    complete_operation_price_removal(db, &row.id, cleanup_code).await?;
                } else {
                    clear_operation_cleanup_marker(db, &row.id, cleanup_code).await?;
                }
            }
            Ok(true)
        }
        Err(error) => {
            let public_error =
                "Lago operation-price synchronization failed; the reconcile sweep will retry";
            set_operation_sync_state(db, row, PricingSyncStatus::Failed, Some(public_error))
                .await?;
            tracing::warn!(
                operation_id = %row.id,
                metric_code = %row.billing.lago_metric_code,
                error = %error,
                "Platform operation price synchronization failed"
            );
            Ok(false)
        }
    }
}

pub async fn retry_pending_operation_prices(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
) -> AppResult<u64> {
    let rows: Vec<PlatformOperationRow> = db
        .collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .find(doc! {
            "$or": [
                { "billing.sync_status": { "$in": ["pending", "failed"] } },
                {
                    "billing_cleanup_metric_code": {
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
    for row in rows {
        if sync_operation_price(db, lago, plan_code, &row).await? {
            synced += 1;
        }
    }
    Ok(synced)
}

pub(crate) async fn set_operation_sync_state(
    db: &mongodb::Database,
    row: &PlatformOperationRow,
    status: PricingSyncStatus,
    error: Option<&str>,
) -> AppResult<bool> {
    let mut filter = doc! {
        "_id": &row.id,
        "updated_at": bson::DateTime::from_chrono(row.updated_at),
        "billing.metric": bson::to_bson(&row.billing.metric).map_err(|error| {
            AppError::Internal(format!("failed to encode operation billing metric: {error}"))
        })?,
        "billing.price_per_unit": &row.billing.price_per_unit,
        "billing.lago_metric_code": &row.billing.lago_metric_code,
    };
    match row.billing.base_fee_per_call.as_deref() {
        Some(base_fee) => {
            filter.insert("billing.base_fee_per_call", base_fee);
        }
        None => {
            filter.insert("billing.base_fee_per_call", doc! { "$exists": false });
        }
    }
    let result = db
        .collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .update_one(
            filter,
            doc! { "$set": {
                "billing.sync_status": bson::to_bson(&status).map_err(|error| {
                    AppError::Internal(format!("failed to encode operation price sync status: {error}"))
                })?,
                "billing.sync_error": error.map_or(Bson::Null, |value| Bson::String(value.to_string())),
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;
    Ok(result.matched_count == 1)
}

async fn complete_operation_price_removal(
    db: &mongodb::Database,
    operation_id: &str,
    metric_code: &str,
) -> AppResult<()> {
    db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .delete_many(doc! { "lago_metric_code": metric_code })
        .await?;
    clear_operation_cleanup_marker(db, operation_id, metric_code).await
}

async fn clear_operation_cleanup_marker(
    db: &mongodb::Database,
    operation_id: &str,
    metric_code: &str,
) -> AppResult<()> {
    db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .update_one(
            doc! {
                "_id": operation_id,
                "billing_cleanup_metric_code": metric_code,
            },
            doc! { "$unset": { "billing_cleanup_metric_code": "" } },
        )
        .await?;
    Ok(())
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;
    use mongodb::bson::{self, doc};
    use serde_json::json;

    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::test_utils::connect_test_database;

    use crate::models::service_billing::{
        PricingSyncStatus, ServiceBilling, ServicePlatformPricing,
    };
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
    };

    use super::{
        DOWNSTREAM_SERVICES, complete_price_removal, metric_code_for_operation,
        metric_code_for_service, normalize_operation_billing, normalize_platform_pricing,
        normalize_price,
    };

    #[derive(Default)]
    struct OperationPricingLago {
        fail_sync: AtomicBool,
        fail_remove: AtomicBool,
    }

    #[async_trait]
    impl super::LagoApi for OperationPricingLago {
        async fn ensure_customer(
            &self,
            owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            _plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            Ok(customer_id.to_string())
        }

        async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
            Ok(LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
        }

        async fn record_events_batch(
            &self,
            events: &[LagoEvent],
        ) -> Result<Vec<LagoAck>, LagoError> {
            Ok(events
                .iter()
                .map(|event| LagoAck {
                    transaction_id: event.transaction_id.clone(),
                })
                .collect())
        }

        async fn current_usage(
            &self,
            customer_id: &str,
            subscription_id: &str,
        ) -> crate::errors::AppResult<LagoUsage> {
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: json!({}),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
            Ok(0)
        }

        async fn entitlements(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<Vec<Entitlement>> {
            Ok(Vec::new())
        }

        async fn sync_standard_charge(
            &self,
            _plan_code: &str,
            _input: &super::ServicePriceSync,
        ) -> crate::errors::AppResult<()> {
            if self.fail_sync.load(Ordering::SeqCst) {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "injected operation price sync failure".to_string(),
                ));
            }
            Ok(())
        }

        async fn remove_standard_charge(
            &self,
            _plan_code: &str,
            _metric_code: &str,
        ) -> crate::errors::AppResult<()> {
            if self.fail_remove.load(Ordering::SeqCst) {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "injected operation price removal failure".to_string(),
                ));
            }
            Ok(())
        }
    }

    async fn insert_priced_operation(
        db: &mongodb::Database,
    ) -> crate::models::platform_operation::PlatformOperationRow {
        use crate::models::platform_operation::{
            CallAndSayOperationConfig, ConstrainedConfig, ConstrainedOp, OperationBilling,
            OperationLimits, PerRequestCaps, PlatformOperationRow,
        };
        use crate::models::service_billing::BillingMetric;

        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = "api-twilio".to_string();
        service.name = "Twilio".to_string();
        service.auth_method = "basic".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.service_type = "http".to_string();
        service.is_active = true;
        db.collection::<crate::models::downstream_service::DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert priced catalog service");
        let mut row = PlatformOperationRow::new_constrained(
            service.id,
            ConstrainedOp::CallAndSay,
            ConstrainedConfig::CallAndSay(CallAndSayOperationConfig {
                allowed_destination_prefixes: vec!["+65".to_string()],
                voice: "alice".to_string(),
                account_sid: "AC11111111111111111111111111111111".to_string(),
                call_from: "+6512345678".to_string(),
            }),
            OperationLimits {
                per_request: PerRequestCaps::CallAndSay {
                    max_message_chars: 500,
                    max_duration_seconds: 600,
                },
                per_user_per_day: Some(10),
            },
            OperationBilling {
                metric: BillingMetric::Seconds,
                price_per_unit: "0.01".to_string(),
                base_fee_per_call: Some("1.5".to_string()),
                lago_metric_code: "platform_op_api_twilio_constrained_call_and_say".to_string(),
                sync_status: PricingSyncStatus::Pending,
                sync_error: None,
            },
            "admin".to_string(),
        );
        row.enabled = true;
        db.collection::<PlatformOperationRow>(super::PLATFORM_OPERATIONS)
            .insert_one(&row)
            .await
            .expect("insert priced operation");
        row
    }

    #[test]
    fn operation_metric_code_is_stable_bounded_and_collision_resistant() {
        assert_eq!(
            metric_code_for_operation("api-twitter", "endpoint:GET /2/tweets/search/recent"),
            "platform_op_api_twitter_endpoint_get_2_tweets_search_recent"
        );
        let first =
            metric_code_for_operation("duffel", &format!("endpoint:GET /{}a", "x".repeat(200)));
        let second =
            metric_code_for_operation("duffel", &format!("endpoint:GET /{}b", "x".repeat(200)));
        assert!(first.len() <= 120);
        assert!(second.len() <= 120);
        assert_ne!(first, second);
    }

    #[test]
    fn operation_billing_rejects_metrics_without_a_meaningful_adapter() {
        use crate::models::platform_operation::{
            ConstrainedConfig, ConstrainedOp, OperationBilling, PlatformOperationKind,
            SpeakOperationConfig,
        };
        use crate::models::service_billing::{BillingMetric, PricingSyncStatus};

        let kind = PlatformOperationKind::Constrained {
            op: ConstrainedOp::Speak,
            config: ConstrainedConfig::Speak(SpeakOperationConfig {
                allowed_voice_ids: Vec::new(),
                model_id: "eleven_multilingual_v2".to_string(),
                max_calls_per_user_per_day: 50,
            }),
        };
        let mut billing = OperationBilling {
            metric: BillingMetric::Seconds,
            price_per_unit: "0.01".to_string(),
            base_fee_per_call: None,
            lago_metric_code: String::new(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: Some("stale".to_string()),
        };
        assert!(
            normalize_operation_billing(
                "api-elevenlabs",
                "constrained:speak",
                &kind,
                None,
                &mut billing
            )
            .is_err()
        );

        billing.metric = BillingMetric::Characters;
        normalize_operation_billing(
            "api-elevenlabs",
            "constrained:speak",
            &kind,
            None,
            &mut billing,
        )
        .expect("characters is meaningful for speak");
        assert_eq!(billing.price_per_unit, "0.01");
        assert_eq!(
            billing.lago_metric_code,
            "platform_op_api_elevenlabs_constrained_speak"
        );
        assert_eq!(billing.sync_status, PricingSyncStatus::Pending);
        assert!(billing.sync_error.is_none());
    }

    #[tokio::test]
    async fn stale_operation_price_completion_cannot_overwrite_newer_admin_edit() {
        let Some(db) = connect_test_database("operation_price_stale_completion").await else {
            return;
        };
        let row = insert_priced_operation(&db).await;
        let newer_at = row.updated_at + chrono::Duration::seconds(1);
        db.collection::<crate::models::platform_operation::PlatformOperationRow>(
            super::PLATFORM_OPERATIONS,
        )
        .update_one(
            doc! { "_id": &row.id },
            doc! { "$set": {
                "billing.price_per_unit": "0.02",
                "billing.sync_status": "pending",
                "updated_at": bson::DateTime::from_chrono(newer_at),
            } },
        )
        .await
        .expect("apply newer admin edit");

        assert!(
            !super::set_operation_sync_state(&db, &row, PricingSyncStatus::Synced, None,)
                .await
                .expect("attempt stale completion")
        );
        let saved = db
            .collection::<crate::models::platform_operation::PlatformOperationRow>(
                super::PLATFORM_OPERATIONS,
            )
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find operation")
            .expect("operation exists");
        assert_eq!(saved.billing.price_per_unit, "0.02");
        assert_eq!(saved.billing.sync_status, PricingSyncStatus::Pending);
    }

    #[tokio::test]
    async fn failed_operation_price_sync_retries_and_updates_rate_cache() {
        let Some(db) = connect_test_database("operation_price_retry").await else {
            return;
        };
        let row = insert_priced_operation(&db).await;
        let lago = OperationPricingLago::default();
        lago.fail_sync.store(true, Ordering::SeqCst);

        assert!(
            !super::sync_operation_price(&db, &lago, "standard", &row)
                .await
                .expect("failed sync is recorded")
        );
        let failed = db
            .collection::<crate::models::platform_operation::PlatformOperationRow>(
                super::PLATFORM_OPERATIONS,
            )
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find failed operation")
            .expect("failed operation exists");
        assert_eq!(failed.billing.sync_status, PricingSyncStatus::Failed);

        lago.fail_sync.store(false, Ordering::SeqCst);
        assert_eq!(
            super::retry_pending_operation_prices(&db, &lago, "standard")
                .await
                .expect("retry operation price"),
            1
        );
        let saved = db
            .collection::<crate::models::platform_operation::PlatformOperationRow>(
                super::PLATFORM_OPERATIONS,
            )
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find synced operation")
            .expect("synced operation exists");
        assert_eq!(saved.billing.sync_status, PricingSyncStatus::Synced);
        let rate = db
            .collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .find_one(doc! { "lago_metric_code": &row.billing.lago_metric_code })
            .await
            .expect("find operation rate")
            .expect("operation rate exists");
        assert_eq!(rate.credits_per_unit_micros, 10_000);
    }

    #[tokio::test]
    async fn obsolete_operation_metric_marker_survives_until_charge_and_cache_are_removed() {
        let Some(db) = connect_test_database("operation_price_cleanup_retry").await else {
            return;
        };
        let mut row = insert_priced_operation(&db).await;
        let old_metric = "platform_op_obsolete_call_metric";
        row.billing_cleanup_metric_code = Some(old_metric.to_string());
        db.collection::<crate::models::platform_operation::PlatformOperationRow>(
            super::PLATFORM_OPERATIONS,
        )
        .replace_one(doc! { "_id": &row.id }, &row)
        .await
        .expect("set operation cleanup marker");
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id(old_metric, None),
                lago_metric_code: old_metric.to_string(),
                model: None,
                credits_per_unit_micros: 25_000,
                synced_at: Utc::now(),
            })
            .await
            .expect("insert obsolete rate");
        let lago = OperationPricingLago::default();
        lago.fail_remove.store(true, Ordering::SeqCst);

        assert!(
            !super::sync_operation_price(&db, &lago, "standard", &row)
                .await
                .expect("cleanup failure is retryable")
        );
        let pending = db
            .collection::<crate::models::platform_operation::PlatformOperationRow>(
                super::PLATFORM_OPERATIONS,
            )
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find cleanup operation")
            .expect("cleanup operation exists");
        assert_eq!(
            pending.billing_cleanup_metric_code.as_deref(),
            Some(old_metric)
        );
        assert_eq!(
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
                .count_documents(doc! { "lago_metric_code": old_metric })
                .await
                .expect("count obsolete rates"),
            1
        );

        lago.fail_remove.store(false, Ordering::SeqCst);
        assert!(
            super::sync_operation_price(&db, &lago, "standard", &pending)
                .await
                .expect("cleanup retry")
        );
        let saved = db
            .collection::<crate::models::platform_operation::PlatformOperationRow>(
                super::PLATFORM_OPERATIONS,
            )
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find cleaned operation")
            .expect("cleaned operation exists");
        assert!(saved.billing_cleanup_metric_code.is_none());
        assert_eq!(
            db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
                .count_documents(doc! { "lago_metric_code": old_metric })
                .await
                .expect("count obsolete rates"),
            0
        );
    }

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
