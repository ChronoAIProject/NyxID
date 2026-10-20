//! Central billing configuration: global defaults, their audit trail, and the
//! read-only effective-price resolver used by the admin billing surface.
//!
//! The resolver never decides a charge. It builds the same
//! [`BillingRouteContext`] the charge path builds for every credential class
//! and only explains the result, so the admin view cannot drift from billing.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Bson, doc};
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES};
use crate::models::downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService};
use crate::models::platform_settings::{
    BillingSettings, COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID, PlatformSettings,
};
use crate::models::service_billing::{BillingMetric, LanePricing, PricingSyncStatus};
use crate::models::usage_meter::CredentialClass;
use crate::services::audit_service::AuditActor;

use super::route_context::{BillingLane, BillingRouteContext, NodeIntent};
use super::route_inventory::BillingIngress;

/// Largest accepted global overdraft cap, in whole credits. Kept below
/// JavaScript's safe-integer range so the admin form round-trips it exactly.
pub const MAX_DEFAULT_OVERDRAFT_CAP_CREDITS: i64 = 1_000_000_000_000;

/// Every credential class, in the order the admin view lists them.
pub const CREDENTIAL_CLASSES: [CredentialClass; 6] = [
    CredentialClass::NyxidManagedMaster,
    CredentialClass::NyxidPlatformOauthApp,
    CredentialClass::UserOwned,
    CredentialClass::AgentOverrideUserOwned,
    CredentialClass::NodeManaged,
    CredentialClass::NoAuth,
];

/// Where an effective value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingConfigSource {
    /// A deployment environment variable (read-only in the admin UI).
    Env,
    /// `PlatformSettings.billing`, edited on the central billing page.
    Global,
    /// The service's own billing lane configuration.
    Service,
    /// Legacy per-service platform billing (`platform_billable`, `platform_pricing`
    /// or the Lago-authored plan price).
    Legacy,
    /// Nothing charges this value.
    Free,
}

pub async fn load_settings(db: &mongodb::Database) -> AppResult<Option<BillingSettings>> {
    Ok(
        crate::services::platform_settings_service::load_settings(db)
            .await?
            .billing,
    )
}

/// The global overdraft cap applied to newly provisioned wallets: the stored
/// global default wins, otherwise `BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS`.
pub fn effective_overdraft_cap(
    global: Option<&BillingSettings>,
    env_default: i64,
) -> (i64, BillingConfigSource) {
    match global.and_then(|settings| settings.default_overdraft_cap_credits) {
        Some(value) => (value, BillingConfigSource::Global),
        None => (env_default, BillingConfigSource::Env),
    }
}

/// Per-field patch for the global defaults: `None` keeps the stored value,
/// `Some(None)` clears it back to the environment default, `Some(Some(v))` sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobalBillingPatch {
    pub default_overdraft_cap_credits: Option<Option<i64>>,
}

impl GlobalBillingPatch {
    pub fn is_empty(&self) -> bool {
        self.default_overdraft_cap_credits.is_none()
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.is_empty() {
            return Err(AppError::ValidationError(
                "No billing configuration field was supplied".into(),
            ));
        }
        if let Some(Some(value)) = self.default_overdraft_cap_credits
            && !(0..=MAX_DEFAULT_OVERDRAFT_CAP_CREDITS).contains(&value)
        {
            return Err(AppError::ValidationError(format!(
                "default_overdraft_cap_credits must be between 0 and {MAX_DEFAULT_OVERDRAFT_CAP_CREDITS}"
            )));
        }
        Ok(())
    }

    fn apply(&self, current: Option<&BillingSettings>) -> BillingSettings {
        let mut next = current.cloned().unwrap_or_default();
        if let Some(value) = self.default_overdraft_cap_credits {
            next.default_overdraft_cap_credits = value;
        }
        next
    }
}

/// Applies a validated patch and appends its audit row in one transaction:
/// either both commit or neither does.
pub async fn update_global_defaults(
    db: &mongodb::Database,
    patch: &GlobalBillingPatch,
    actor: &AuditActor,
    key: &[u8],
) -> AppResult<BillingSettings> {
    patch.validate()?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let settings = db.collection::<PlatformSettings>(SETTINGS);
    let current = settings
        .find_one(doc! {"_id": PLATFORM_SETTINGS_ID})
        .session(&mut session)
        .await?
        .and_then(|row| row.billing);
    let next = patch.apply(current.as_ref());
    let value =
        mongodb::bson::to_bson(&next).map_err(|e| AppError::Internal(format!("serialize: {e}")))?;
    settings
        .update_one(
            doc! {"_id": PLATFORM_SETTINGS_ID},
            doc! {"$set": {"billing": value}, "$setOnInsert": {"_id": PLATFORM_SETTINGS_ID}},
        )
        .upsert(true)
        .session(&mut session)
        .await?;
    crate::services::audit_service::log_actor_event_in_session(
        db,
        &mut session,
        key,
        actor,
        "billing_config_updated",
        serde_json::json!({
            "default_overdraft_cap_credits": {
                "before": current.as_ref().and_then(|s| s.default_overdraft_cap_credits),
                "after": next.default_overdraft_cap_credits,
            },
        }),
    )
    .await?;
    session.commit_transaction().await?;
    Ok(next)
}

/// Result of an admin-triggered price resync.
#[derive(Clone, Debug)]
pub struct ResyncOutcome {
    pub synced: bool,
    pub service: DownstreamService,
}

/// Re-runs the Lago price sync for one catalog service. The audit row is
/// best-effort: once Lago has applied the change the request must not fail.
pub async fn resync_service(
    db: &mongodb::Database,
    billing: &super::BillingService,
    actor: &AuditActor,
    service_id: &str,
) -> AppResult<ResyncOutcome> {
    let service = load_service(db, service_id)
        .await?
        .ok_or_else(|| AppError::NotFound("service not found".into()))?;
    let synced = billing.sync_service_price(&service).await?;
    if let Err(error) = crate::services::audit_service::log_actor_event(
        db.clone(),
        actor,
        "billing_service_resynced",
        Some(serde_json::json!({"service_id": service_id, "synced": synced})),
    )
    .await
    {
        tracing::error!(%error, service_id, "billing resync audit append failed");
    }
    let service = load_service(db, service_id)
        .await?
        .ok_or_else(|| AppError::NotFound("service not found".into()))?;
    Ok(ResyncOutcome { synced, service })
}

pub async fn load_rates(db: &mongodb::Database) -> AppResult<Vec<BillingRateCache>> {
    Ok(db
        .collection::<BillingRateCache>(RATES)
        .find(doc! {})
        .sort(doc! {"_id": 1})
        .await?
        .try_collect()
        .await?)
}

/// Catalog services that carry a billing block, ordered by slug.
pub async fn load_billed_services(db: &mongodb::Database) -> AppResult<Vec<DownstreamService>> {
    Ok(db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! {"billing": {"$ne": Bson::Null}})
        .sort(doc! {"slug": 1})
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

/// Cached Lago rates for the model-agnostic rows of the given metric codes.
pub async fn load_rates_for(
    db: &mongodb::Database,
    services: &[DownstreamService],
) -> AppResult<HashMap<String, BillingRateCache>> {
    let mut ids: Vec<String> = services
        .iter()
        .flat_map(referenced_metric_codes)
        .map(|code| BillingRateCache::cache_id(&code, None))
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<BillingRateCache> = db
        .collection::<BillingRateCache>(RATES)
        .find(doc! {"_id": {"$in": ids}})
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.lago_metric_code.clone(), row))
        .collect())
}

fn referenced_metric_codes(service: &DownstreamService) -> Vec<String> {
    let mut codes = Vec::new();
    for class in CREDENTIAL_CLASSES {
        let ctx = route_context(service, class, true);
        codes.extend(ctx.platform_specs().map(|(_, code)| code.to_string()));
        codes.extend(ctx.operation_prices().iter().map(|(_, code)| code.clone()));
        if let Some(resale) = &ctx.resale {
            codes.push(resale.lago_metric_code.clone());
        }
    }
    codes
}

/// Why a credential class is (or is not) charged, as a stable code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveReason {
    /// `BILLING_ENABLED=false`: nothing is charged anywhere.
    BillingDisabled,
    /// `platform_charge_nyxid_credentials_only` exempts this credential class.
    NyxidCredentialsOnly,
    /// The class's lane primary is synced; the lane price applies.
    LaneSynced,
    /// The lane primary is not synced; legacy platform billing applies meanwhile.
    LaneUnsyncedLegacy,
    /// The lane primary is not synced and legacy platform billing is off.
    LaneUnsyncedFree,
    /// Another lane is configured but this class's lane is not.
    LaneNotConfigured,
    /// No-auth traffic is never priced once lanes are configured.
    NoAuth,
    /// No lanes; legacy `platform_billable` charges the legacy price.
    LegacyPlatformBillable,
    /// No lanes and legacy platform billing is off.
    NotBillable,
}

impl EffectiveReason {
    /// Whether the platform layer charges for this reason.
    pub fn platform_charged(self) -> bool {
        matches!(
            self,
            Self::LaneSynced | Self::LaneUnsyncedLegacy | Self::LegacyPlatformBillable
        )
    }

    fn source(self) -> BillingConfigSource {
        match self {
            Self::BillingDisabled => BillingConfigSource::Env,
            Self::LaneSynced => BillingConfigSource::Service,
            Self::LaneUnsyncedLegacy | Self::LegacyPlatformBillable => BillingConfigSource::Legacy,
            Self::NyxidCredentialsOnly
            | Self::LaneUnsyncedFree
            | Self::LaneNotConfigured
            | Self::NoAuth
            | Self::NotBillable => BillingConfigSource::Free,
        }
    }
}

/// A cached Lago rate behind a metric code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CachedRate {
    /// Exact decimal credits per unit.
    pub credits_per_unit: String,
    pub synced_at: DateTime<Utc>,
    pub stale: bool,
    pub retired: bool,
}

/// One priced value with its provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectivePrice {
    /// `platform` or `resale`.
    pub layer: &'static str,
    pub metric: BillingMetric,
    /// Operation key when an operation price replaces the Requests rate.
    pub operation: Option<String>,
    pub lago_metric_code: String,
    /// Exact decimal credits per unit: the NyxID-authored price, or the
    /// cached Lago rate for Lago-authored prices; `None` when not yet cached.
    pub credits_per_unit: Option<String>,
    pub source: BillingConfigSource,
    /// `lane_primary`, `lane_component`, `operation`, `service_price`,
    /// `legacy_plan` or `resale_plan`.
    pub origin: &'static str,
    pub sync_status: Option<PricingSyncStatus>,
    pub sync_error: Option<String>,
    pub cached_rate: Option<CachedRate>,
}

/// Effective billing for one or more credential classes that resolve identically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveClassBilling {
    pub credential_classes: Vec<CredentialClass>,
    /// `byok`, `platform_key`, or `None` for no-auth traffic.
    pub lane: Option<&'static str>,
    /// Whether the platform layer charges this class.
    pub platform_charged: bool,
    pub source: BillingConfigSource,
    pub reason: EffectiveReason,
    /// Charged values: platform primary, components, operations and resale.
    pub prices: Vec<EffectivePrice>,
    /// Configured values of the class's lane that do not charge until synced.
    pub pending: Vec<EffectivePrice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveResale {
    pub configured: bool,
    /// Resale applies to NyxID master-credential traffic only.
    pub applied: bool,
    pub source: BillingConfigSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveServiceBilling {
    /// The metric HTTP traffic meters when no synced lane overrides it.
    pub fallback_metric: BillingMetric,
    pub classes: Vec<EffectiveClassBilling>,
    pub resale: EffectiveResale,
}

/// Inputs shared by every service in one resolution pass.
pub struct ResolverEnv<'a> {
    pub billing_enabled: bool,
    pub resale_enabled: bool,
    pub rate_ttl_secs: i64,
    pub now: DateTime<Utc>,
    pub rates: &'a HashMap<String, BillingRateCache>,
}

fn route_context(
    service: &DownstreamService,
    class: CredentialClass,
    resale_enabled: bool,
) -> BillingRouteContext {
    BillingRouteContext::new(
        BillingIngress::Proxy,
        String::new(),
        String::new(),
        String::new(),
        None,
        None,
        Some(service.id.clone()),
        Some(service.slug.clone()),
        NodeIntent::Direct,
        service.auth_method.clone(),
        class,
        super::metric_resolution::platform_metric_for_request(service, false),
        service.billing.as_ref(),
        resale_enabled,
    )
}

fn class_lane(service: &DownstreamService, class: CredentialClass) -> Option<&LanePricing> {
    let billing = service.billing.as_ref()?;
    match BillingLane::for_credential(class)? {
        BillingLane::Byok => billing.byok_pricing.as_ref(),
        BillingLane::PlatformKey => billing.platform_key_pricing.as_ref(),
    }
}

/// The reason for a class, following `BillingRouteContext::new` branch by branch.
fn reason_for(service: &DownstreamService, class: CredentialClass) -> EffectiveReason {
    let Some(billing) = service.billing.as_ref() else {
        return EffectiveReason::NotBillable;
    };
    if billing.platform_charge_nyxid_credentials_only
        && !matches!(
            class,
            CredentialClass::NyxidManagedMaster | CredentialClass::NyxidPlatformOauthApp
        )
    {
        return EffectiveReason::NyxidCredentialsOnly;
    }
    if billing.byok_pricing.is_some() || billing.platform_key_pricing.is_some() {
        return match class_lane(service, class) {
            None if class == CredentialClass::NoAuth => EffectiveReason::NoAuth,
            None => EffectiveReason::LaneNotConfigured,
            Some(lane) if lane.sync_status == PricingSyncStatus::Synced => {
                EffectiveReason::LaneSynced
            }
            Some(_) if billing.platform_billable => EffectiveReason::LaneUnsyncedLegacy,
            Some(_) => EffectiveReason::LaneUnsyncedFree,
        };
    }
    if billing.platform_billable {
        EffectiveReason::LegacyPlatformBillable
    } else {
        EffectiveReason::NotBillable
    }
}

fn cached_rate(env: &ResolverEnv<'_>, code: &str) -> Option<CachedRate> {
    let rate = env.rates.get(code)?;
    let pico = super::amounts::rate_pico(rate.credits_per_unit_pico, rate.credits_per_unit_micros);
    let credits = crate::models::credits::Credits::from_pico(pico).ok()?;
    Some(CachedRate {
        credits_per_unit: credits.to_string(),
        synced_at: rate.synced_at,
        stale: rate.synced_at + chrono::Duration::seconds(env.rate_ttl_secs) < env.now,
        retired: rate.retired_at.is_some(),
    })
}

fn platform_price(
    env: &ResolverEnv<'_>,
    service: &DownstreamService,
    class: CredentialClass,
    metric: BillingMetric,
    code: &str,
) -> EffectivePrice {
    let lane = class_lane(service, class).filter(|l| l.sync_status == PricingSyncStatus::Synced);
    let cached = cached_rate(env, code);
    let base = |origin, source, price: Option<String>, status, error| EffectivePrice {
        layer: "platform",
        metric,
        operation: None,
        lago_metric_code: code.to_string(),
        credits_per_unit: price.or_else(|| cached.as_ref().map(|r| r.credits_per_unit.clone())),
        source,
        origin,
        sync_status: status,
        sync_error: error,
        cached_rate: cached.clone(),
    };
    if let Some(lane) = lane {
        if lane.lago_metric_code == code {
            return base(
                "lane_primary",
                BillingConfigSource::Service,
                Some(lane.credits_per_unit.clone()),
                Some(lane.sync_status),
                lane.sync_error.clone(),
            );
        }
        if let Some(component) = lane.components.iter().find(|c| c.lago_metric_code == code) {
            return base(
                "lane_component",
                BillingConfigSource::Service,
                Some(component.credits_per_unit.clone()),
                Some(component.sync_status),
                component.sync_error.clone(),
            );
        }
    }
    if let Some(pricing) = service
        .billing
        .as_ref()
        .and_then(|b| b.platform_pricing.as_ref())
        .filter(|p| p.sync_status == PricingSyncStatus::Synced && p.lago_metric_code == code)
    {
        return base(
            "service_price",
            BillingConfigSource::Legacy,
            Some(pricing.credits_per_unit.clone()),
            Some(pricing.sync_status),
            pricing.sync_error.clone(),
        );
    }
    // A Lago-authored plan price. A pending/failed legacy NyxID price is
    // reported here because traffic keeps the plan metric until it syncs.
    let pending = service
        .billing
        .as_ref()
        .and_then(|b| b.platform_pricing.as_ref())
        .filter(|p| p.sync_status != PricingSyncStatus::Synced);
    base(
        "legacy_plan",
        BillingConfigSource::Legacy,
        None,
        pending.map(|p| p.sync_status),
        pending.and_then(|p| p.sync_error.clone()),
    )
}

fn pending_prices(
    env: &ResolverEnv<'_>,
    service: &DownstreamService,
    class: CredentialClass,
    reason: EffectiveReason,
) -> Vec<EffectivePrice> {
    let Some(lane) = class_lane(service, class) else {
        return Vec::new();
    };
    if reason == EffectiveReason::NyxidCredentialsOnly {
        return Vec::new();
    }
    let entry = |origin,
                 metric,
                 operation: Option<&str>,
                 code: &str,
                 price: &str,
                 status,
                 error: &Option<String>| EffectivePrice {
        layer: "platform",
        metric,
        operation: operation.map(str::to_string),
        lago_metric_code: code.to_string(),
        credits_per_unit: Some(price.to_string()),
        source: BillingConfigSource::Free,
        origin,
        sync_status: Some(status),
        sync_error: error.clone(),
        cached_rate: cached_rate(env, code),
    };
    let mut out = Vec::new();
    if lane.sync_status != PricingSyncStatus::Synced {
        // An unsynced primary disables the whole lane, extras included.
        out.push(entry(
            "lane_primary",
            lane.metric,
            None,
            &lane.lago_metric_code,
            &lane.credits_per_unit,
            lane.sync_status,
            &lane.sync_error,
        ));
        out.extend(lane.components.iter().map(|c| {
            entry(
                "lane_component",
                c.metric,
                None,
                &c.lago_metric_code,
                &c.credits_per_unit,
                c.sync_status,
                &c.sync_error,
            )
        }));
        out.extend(lane.operations.iter().map(|o| {
            entry(
                "operation",
                BillingMetric::Requests,
                Some(&o.operation),
                &o.lago_metric_code,
                &o.credits_per_unit,
                o.sync_status,
                &o.sync_error,
            )
        }));
        return out;
    }
    out.extend(
        lane.components
            .iter()
            .filter(|c| c.sync_status != PricingSyncStatus::Synced)
            .map(|c| {
                entry(
                    "lane_component",
                    c.metric,
                    None,
                    &c.lago_metric_code,
                    &c.credits_per_unit,
                    c.sync_status,
                    &c.sync_error,
                )
            }),
    );
    // Operation prices only exist on a Requests lane; on any other unit they are inert.
    out.extend(
        lane.operations
            .iter()
            .filter(|o| {
                lane.metric != BillingMetric::Requests || o.sync_status != PricingSyncStatus::Synced
            })
            .map(|o| {
                entry(
                    "operation",
                    BillingMetric::Requests,
                    Some(&o.operation),
                    &o.lago_metric_code,
                    &o.credits_per_unit,
                    o.sync_status,
                    &o.sync_error,
                )
            }),
    );
    out
}

fn resolve_class(
    env: &ResolverEnv<'_>,
    service: &DownstreamService,
    class: CredentialClass,
) -> EffectiveClassBilling {
    let ctx = route_context(service, class, env.resale_enabled);
    let reason = reason_for(service, class);
    debug_assert_eq!(reason.platform_charged(), ctx.service_platform_billable);
    let mut prices = Vec::new();
    if ctx.service_platform_billable {
        for (metric, code) in ctx.platform_specs() {
            prices.push(platform_price(env, service, class, metric, code));
        }
        let lane = class_lane(service, class);
        for (operation, code) in ctx.operation_prices() {
            let configured =
                lane.and_then(|l| l.operations.iter().find(|o| &o.operation == operation));
            let cached = cached_rate(env, code);
            prices.push(EffectivePrice {
                layer: "platform",
                metric: BillingMetric::Requests,
                operation: Some(operation.clone()),
                lago_metric_code: code.clone(),
                credits_per_unit: configured.map(|o| o.credits_per_unit.clone()),
                source: BillingConfigSource::Service,
                origin: "operation",
                sync_status: configured.map(|o| o.sync_status),
                sync_error: configured.and_then(|o| o.sync_error.clone()),
                cached_rate: cached,
            });
        }
    }
    if let Some(resale) = &ctx.resale {
        let cached = cached_rate(env, &resale.lago_metric_code);
        prices.push(EffectivePrice {
            layer: "resale",
            metric: resale.metric,
            operation: None,
            lago_metric_code: resale.lago_metric_code.clone(),
            credits_per_unit: cached.as_ref().map(|r| r.credits_per_unit.clone()),
            source: BillingConfigSource::Service,
            origin: "resale_plan",
            sync_status: None,
            sync_error: None,
            cached_rate: cached,
        });
    }
    let (platform_charged, source, reason) = if env.billing_enabled {
        (ctx.service_platform_billable, reason.source(), reason)
    } else {
        (
            false,
            BillingConfigSource::Env,
            EffectiveReason::BillingDisabled,
        )
    };
    EffectiveClassBilling {
        credential_classes: vec![class],
        lane: BillingLane::for_credential(class).map(|lane| match lane {
            BillingLane::Byok => "byok",
            BillingLane::PlatformKey => "platform_key",
        }),
        platform_charged,
        source,
        reason,
        prices,
        pending: pending_prices(env, service, class, reason_for(service, class)),
    }
}

/// Effective billing for every credential class of one catalog service.
/// Classes that resolve identically are grouped, preserving class order.
pub fn resolve_service_billing(
    env: &ResolverEnv<'_>,
    service: &DownstreamService,
) -> EffectiveServiceBilling {
    let mut classes: Vec<EffectiveClassBilling> = Vec::new();
    for class in CREDENTIAL_CLASSES {
        let resolved = resolve_class(env, service, class);
        let same = |group: &EffectiveClassBilling| {
            group.lane == resolved.lane
                && group.platform_charged == resolved.platform_charged
                && group.source == resolved.source
                && group.reason == resolved.reason
                && group.prices == resolved.prices
                && group.pending == resolved.pending
        };
        match classes.iter_mut().find(|group| same(group)) {
            Some(group) => group.credential_classes.push(class),
            None => classes.push(resolved),
        }
    }
    let configured = service
        .billing
        .as_ref()
        .is_some_and(|b| b.active_resale_spec().is_some());
    EffectiveServiceBilling {
        fallback_metric: super::metric_resolution::platform_metric_for_request(service, false),
        classes,
        resale: EffectiveResale {
            configured,
            applied: configured && env.billing_enabled && env.resale_enabled,
            source: if !configured {
                BillingConfigSource::Free
            } else if env.billing_enabled && env.resale_enabled {
                BillingConfigSource::Service
            } else {
                BillingConfigSource::Env
            },
        },
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
