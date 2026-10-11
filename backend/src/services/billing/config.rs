use crate::errors::AppResult;
use crate::models::platform_settings::BillingSettings;
use crate::models::service_billing::ServiceBilling;
use crate::models::{
    billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES},
    downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
    platform_settings::{COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID, PlatformSettings},
};
use futures::TryStreamExt;
use mongodb::bson::doc;

pub async fn load_settings(db: &mongodb::Database) -> AppResult<Option<BillingSettings>> {
    Ok(db
        .collection::<PlatformSettings>(SETTINGS)
        .find_one(doc! {"_id": PLATFORM_SETTINGS_ID})
        .await?
        .and_then(|s| s.billing))
}
pub async fn save_settings(
    db: &mongodb::Database,
    billing: Option<BillingSettings>,
) -> AppResult<()> {
    let value = mongodb::bson::to_bson(&billing).map_err(|e| {
        crate::errors::AppError::Internal(format!("serialize billing settings: {e}"))
    })?;
    db.collection::<PlatformSettings>(SETTINGS)
        .update_one(
            doc! {"_id": PLATFORM_SETTINGS_ID},
            doc! {"$set": {"billing": value}, "$setOnInsert": {"_id": PLATFORM_SETTINGS_ID}},
        )
        .upsert(true)
        .await?;
    Ok(())
}
pub async fn load_rates(db: &mongodb::Database) -> AppResult<Vec<BillingRateCache>> {
    Ok(db
        .collection::<BillingRateCache>(RATES)
        .find(doc! {})
        .await?
        .try_collect()
        .await?)
}
pub async fn load_services(db: &mongodb::Database) -> AppResult<Vec<DownstreamService>> {
    Ok(db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! {})
        .await?
        .try_collect()
        .await?)
}
pub async fn load_service(
    db: &mongodb::Database,
    id: &str,
) -> AppResult<Option<DownstreamService>> {
    Ok(db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": id})
        .await?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingConfigSource {
    Env,
    Global,
    Service,
    Legacy,
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBillingConfig {
    pub lane: Option<String>,
    pub overdraft_cap_credits: Option<i64>,
    pub lane_source: BillingConfigSource,
    pub overdraft_source: BillingConfigSource,
}

/// Resolves display/configuration precedence without participating in billing
/// admission, pricing, or settlement: service override, global setting, then
/// deployment fallback. `None` is explicitly represented as free.
pub fn effective_config(
    service: Option<&ServiceBilling>,
    global: Option<&BillingSettings>,
    env_lane: Option<&str>,
    env_overdraft: Option<i64>,
) -> EffectiveBillingConfig {
    let service_lane = service
        .and_then(|b| b.platform_pricing.as_ref())
        .map(|_| "platform".to_string());
    let (lane, lane_source) = match service_lane {
        Some(v) => (Some(v), BillingConfigSource::Service),
        None => match global.and_then(|g| g.default_lane.clone()) {
            Some(v) => (Some(v), BillingConfigSource::Global),
            None => match env_lane {
                Some(v) => (Some(v.to_string()), BillingConfigSource::Env),
                None => (None, BillingConfigSource::Free),
            },
        },
    };
    let (overdraft_cap_credits, overdraft_source) = match global
        .map(|g| g.default_overdraft_cap_credits)
        .unwrap_or(None)
    {
        Some(v) => (Some(v), BillingConfigSource::Global),
        None => match env_overdraft {
            Some(v) => (Some(v), BillingConfigSource::Env),
            None => (None, BillingConfigSource::Legacy),
        },
    };
    EffectiveBillingConfig {
        lane,
        overdraft_cap_credits,
        lane_source,
        overdraft_source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_is_service_global_env_free() {
        let global = BillingSettings {
            default_lane: Some("byok".into()),
            default_overdraft_cap_credits: Some(3),
        };
        let effective = effective_config(None, Some(&global), Some("platform"), Some(5));
        assert_eq!(effective.lane_source, BillingConfigSource::Global);
        assert_eq!(effective.overdraft_cap_credits, Some(3));
        let free = effective_config(None, None, None, None);
        assert_eq!(free.lane_source, BillingConfigSource::Free);
    }

    #[test]
    fn service_override_wins_over_global_and_env() {
        let service = ServiceBilling {
            platform_pricing: Some(crate::models::service_billing::ServicePlatformPricing {
                credits_per_unit: "1".into(),
                lago_metric_code: String::new(),
                sync_status: Default::default(),
                sync_error: None,
            }),
            ..Default::default()
        };
        let global = BillingSettings {
            default_lane: Some("byok".into()),
            default_overdraft_cap_credits: Some(3),
        };
        let effective = effective_config(Some(&service), Some(&global), Some("legacy"), Some(5));
        assert_eq!(effective.lane_source, BillingConfigSource::Service);
    }

    #[test]
    fn every_source_is_representable() {
        assert_eq!(format!("{:?}", BillingConfigSource::Env), "Env");
        assert_eq!(format!("{:?}", BillingConfigSource::Global), "Global");
        assert_eq!(format!("{:?}", BillingConfigSource::Service), "Service");
        assert_eq!(format!("{:?}", BillingConfigSource::Legacy), "Legacy");
        assert_eq!(format!("{:?}", BillingConfigSource::Free), "Free");
    }
}
