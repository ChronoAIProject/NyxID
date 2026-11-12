//! Complete model price revisions. No branch in this module uses legacy rates.
use super::{
    amounts,
    lago_client::{LagoApi, ServicePriceSync},
    model_usage,
    route_context::BillingLane,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES},
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService, PlatformKeyPurpose},
        platform_model::{COLLECTION_NAME as MODELS, PlatformModel},
        platform_model_tariff::*,
        service_billing::{LanePricing, PricingSyncStatus, ServiceBilling},
        usage_meter::CredentialClass,
    },
    services::{
        audit_service::{self, AuditActor},
        platform_model_service, security_readiness_service, transaction_service,
    },
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

pub const READINESS: &str = platform_model_service::BILLING_READINESS;
fn invalid(s: &str) -> AppError {
    AppError::ValidationError(s.into())
}
pub fn unavailable(reason: &str) -> AppError {
    AppError::BillingProviderUnavailable(format!("Model tariff unavailable: {reason}"))
}
fn identifier(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn lane(b: &ServiceBilling, l: TariffLane) -> Option<&LanePricing> {
    match l {
        TariffLane::Byok => b.byok_pricing.as_ref(),
        TariffLane::PlatformKey => b.platform_key_pricing.as_ref(),
    }
}
fn lane_mut(b: &mut ServiceBilling, l: TariffLane) -> &mut Option<LanePricing> {
    match l {
        TariffLane::Byok => &mut b.byok_pricing,
        TariffLane::PlatformKey => &mut b.platform_key_pricing,
    }
}
fn field(l: TariffLane) -> &'static str {
    match l {
        TariffLane::Byok => "byok_pricing",
        TariffLane::PlatformKey => "platform_key_pricing",
    }
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (keys, name, unique) in [
        (
            doc! {"model_id":1,"revision":1},
            "model_tariff_revision",
            true,
        ),
        (doc! {"status":1,"updated_at":1}, "model_tariff_sync", false),
    ] {
        db.collection::<ModelTariff>(COLLECTION_NAME)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(unique)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}

pub fn metric_code(
    slug: &str,
    model: &str,
    lane: TariffLane,
    revision: i64,
    key: &str,
) -> AppResult<String> {
    let id = Uuid::parse_str(model).map_err(|_| invalid("Invalid model ID"))?;
    if !identifier(key, 48)
        || revision <= 0
        || slug.is_empty()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(invalid("Invalid tariff metric identity"));
    }
    let suffix = if lane == TariffLane::Byok {
        "byok"
    } else {
        "pk"
    };
    let code = format!(
        "platform_svc_{slug}_{suffix}_mdl_{}_t{revision}_{key}",
        id.simple()
    );
    if code.len() > 255 {
        return Err(invalid("Model tariff metric code exceeds 255 characters"));
    }
    Ok(code)
}

pub fn validate(t: &mut ModelTariff, slug: &str) -> AppResult<()> {
    if t.variant.len() > 8
        || t.variant.iter().any(|(k, v)| {
            !identifier(k, 40)
                || v.is_empty()
                || v.len() > 80
                || !v
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
        })
    {
        return Err(invalid("Invalid finite tariff variant"));
    }
    if t.source.trim().is_empty() || t.source.len() > 1000 || t.source.chars().any(char::is_control)
    {
        return Err(invalid("Tariff verification source is required"));
    }
    crate::services::platform_model_execution::certification::validate_family_tariff(t)?;
    let selectors = model_usage::selectors(&t.contract)?;
    let b = &t.bounds;
    if !(1..=16).contains(&b.max_candidates)
        || !(1..=64).contains(&b.max_tool_iterations)
        || !(0..=100_000_000).contains(&b.max_input_tokens)
        || !(0..=100_000_000).contains(&b.max_output_tokens)
        || !(0..=100).contains(&b.max_images)
        || !(1..=33_554_432).contains(&b.max_request_bytes)
        || !(1..=33_554_432).contains(&b.max_response_bytes)
        || (t.contract.tokens && b.max_input_tokens == 0)
        || (t.contract.images && b.max_images == 0)
    {
        return Err(invalid("Invalid model request bounds"));
    }
    let input_bound = b
        .max_input_tokens
        .checked_mul(b.max_tool_iterations)
        .ok_or_else(|| invalid("Input bound overflow"))?;
    let output_bound = b
        .max_output_tokens
        .checked_mul(b.max_candidates)
        .and_then(|n| n.checked_mul(b.max_tool_iterations))
        .ok_or_else(|| invalid("Output bound overflow"))?;
    let mut found = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut cost = crate::models::credits::Credits::ZERO;
    if t.components.len() != selectors.len() {
        return Err(invalid(
            "Every usage class requires an explicit price or free component",
        ));
    }
    t.components.sort_by(|a, b| a.key.cmp(&b.key));
    for c in &mut t.components {
        if !found.insert(c.selector)
            || !keys.insert(c.key.clone())
            || !selectors.contains(&c.selector)
            || c.metric != model_usage::metric(c.selector)
            || c.max_quantity <= 0
            || c.max_quantity > 1_000_000_000
        {
            return Err(invalid(
                "Invalid, overlapping or unbounded tariff component",
            ));
        }
        let required = match c.selector {
            Selector::Output
            | Selector::Reasoning
            | Selector::ImageOutput
            | Selector::AudioOutput => output_bound,
            Selector::Images => b.max_images,
            Selector::Requests => 1,
            _ => input_bound,
        };
        if c.max_quantity < required || (c.selector == Selector::Requests && c.max_quantity != 1) {
            return Err(invalid(
                "Component reservation does not cover all candidates and tool iterations",
            ));
        }
        c.credits_per_unit = super::pricing::normalize_price(&c.credits_per_unit)?;
        let pico = amounts::decimal_to_pico(&c.credits_per_unit)
            .ok_or_else(|| invalid("Invalid price"))?;
        if (pico == 0) != c.explicit_free {
            return Err(invalid("Zero components must be explicitly free"));
        }
        cost = cost.checked_add(amounts::cost(i128::from(pico), c.max_quantity)?)?;
        c.lago_metric_code = metric_code(slug, &t.model_id, t.lane, t.revision, &c.key)?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SelectedTariff {
    tariff: ModelTariff,
}
impl SelectedTariff {
    pub fn tariff(&self) -> &ModelTariff {
        &self.tariff
    }
    pub fn paid(&self) -> bool {
        self.tariff.components.iter().any(|c| !c.explicit_free)
    }
    pub fn usage(
        &self,
        e: &model_usage::Evidence,
    ) -> AppResult<crate::models::service_billing::PlatformUsage> {
        model_usage::charge_vector(&self.tariff, e)
    }
}

/// This single selector powers admission and resolve_service_billing. Inputs
/// are internal live registry/catalog records, never a provider-reported model.
#[allow(clippy::too_many_arguments)]
pub fn select(
    service: &DownstreamService,
    model_id: &str,
    class: CredentialClass,
    variant: &BTreeMap<String, String>,
    tariffs: &[ModelTariff],
    rates: &HashMap<String, BillingRateCache>,
    now: DateTime<Utc>,
    ttl: u64,
) -> Result<SelectedTariff, &'static str> {
    if !service.is_active {
        return Err("service_disabled");
    }
    if service
        .platform_key
        .as_ref()
        .is_none_or(|p| p.purpose != PlatformKeyPurpose::ModelOffering)
    {
        return Err("not_model_offering");
    }
    let billing = service.billing.as_ref().ok_or("missing_tariff")?;
    if billing.resale_billable || billing.managed_execution.is_some() {
        return Err("incompatible_billing");
    }
    let selected_lane = match BillingLane::for_credential(class) {
        Some(BillingLane::Byok) => TariffLane::Byok,
        Some(BillingLane::PlatformKey) => TariffLane::PlatformKey,
        None => return Err("credential_ineligible"),
    };
    let binding = lane(billing, selected_lane)
        .ok_or("missing_tariff")?
        .model_prices
        .iter()
        .filter(|b| b.model_id == model_id && &b.variant == variant)
        .collect::<Vec<_>>();
    if binding.len() != 1 {
        return Err("missing_or_ambiguous_variant");
    }
    let id = binding[0]
        .active_tariff_id
        .as_ref()
        .ok_or("tariff_pending")?;
    let t = tariffs
        .iter()
        .find(|t| &t.id == id)
        .ok_or("missing_tariff")?;
    if t.service_id != service.id
        || t.model_id != model_id
        || t.lane != selected_lane
        || &t.variant != variant
        || t.status != TariffStatus::Synced
    {
        return Err("tariff_inactive");
    }
    let mut validated = t.clone();
    if validate(&mut validated, &service.slug).is_err() || validated.components != t.components {
        return Err("invalid_tariff");
    }
    for c in &t.components {
        if c.explicit_free {
            continue;
        }
        let rate = rates.get(&c.lago_metric_code).ok_or("rate_missing")?;
        if rate.model.is_some()
            || rate.lago_metric_code != c.lago_metric_code
            || rate.retired_at.is_some()
            || rate.synced_at > now
            || now.signed_duration_since(rate.synced_at).num_seconds()
                > i64::try_from(ttl).unwrap_or(i64::MAX)
        {
            return Err("rate_stale_or_retired");
        }
        if amounts::rate_pico(rate.credits_per_unit_pico, rate.credits_per_unit_micros)
            != i128::from(amounts::decimal_to_pico(&c.credits_per_unit).ok_or("invalid_price")?)
        {
            return Err("rate_mismatch");
        }
    }
    Ok(SelectedTariff { tariff: t.clone() })
}

pub async fn load_for(
    db: &Database,
    services: &[DownstreamService],
) -> AppResult<Vec<ModelTariff>> {
    let ids: Vec<&str> = services
        .iter()
        .filter_map(|s| s.billing.as_ref())
        .flat_map(|b| [b.byok_pricing.as_ref(), b.platform_key_pricing.as_ref()])
        .flatten()
        .flat_map(|l| &l.model_prices)
        .flat_map(|b| {
            [
                b.active_tariff_id.as_deref(),
                b.pending_tariff_id.as_deref(),
            ]
        })
        .flatten()
        .collect();
    if ids.is_empty() {
        return Ok(vec![]);
    }
    Ok(db
        .collection::<ModelTariff>(COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids}})
        .await?
        .try_collect()
        .await?)
}
pub async fn list(db: &Database, model_id: &str) -> AppResult<Vec<ModelTariff>> {
    Ok(db
        .collection::<ModelTariff>(COLLECTION_NAME)
        .find(doc! {"model_id":model_id})
        .sort(doc! {"revision":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewTariff {
    pub revision: i64,
    pub lane: TariffLane,
    pub variant: BTreeMap<String, String>,
    pub contract: UsageContract,
    pub bounds: RequestBounds,
    pub components: Vec<TariffComponent>,
    pub source: String,
}

pub async fn create(
    db: &Database,
    actor: &AuditActor,
    model_id: &str,
    input: NewTariff,
) -> AppResult<ModelTariff> {
    security_readiness_service::require_activation(db, READINESS).await?;
    if super::super::platform_model_execution::certification::image_contract(&input.contract) {
        security_readiness_service::require_activation(
            db,
            super::super::platform_model_execution::images::READINESS,
        )
        .await?;
    }
    if super::super::platform_model_execution::certification::family_contract(&input.contract) {
        security_readiness_service::require_activation(
            db,
            super::super::platform_model_execution::certification::READINESS,
        )
        .await?;
    }
    create_ready(db, actor, model_id, input).await
}
async fn create_ready(
    db: &Database,
    actor: &AuditActor,
    model_id: &str,
    input: NewTariff,
) -> AppResult<ModelTariff> {
    let model = platform_model_service::get(db, model_id).await?;
    if model.revision != input.revision {
        return Err(AppError::Conflict(
            "Model changed; reload before saving".into(),
        ));
    }
    let document = db
        .collection::<bson::Document>(SERVICES)
        .find_one(doc! {
            "_id": model.billing_service_id.as_deref(),
            "is_active": true,
            "platform_key.purpose": "model_offering",
        })
        .await?
        .ok_or_else(|| invalid("Model requires an active logical billing offering"))?;
    let original_billing = document.get("billing").cloned().unwrap_or(bson::Bson::Null);
    let service: DownstreamService =
        bson::from_document(document).map_err(|e| AppError::Internal(e.to_string()))?;
    if !matches!(
        (model.family, input.contract.family),
        (
            crate::models::platform_model::ModelFamily::Openai,
            UsageFamily::Openai
        ) | (
            crate::models::platform_model::ModelFamily::Claude,
            UsageFamily::Claude
        ) | (
            crate::models::platform_model::ModelFamily::Gemini,
            UsageFamily::Gemini
        ) | (
            crate::models::platform_model::ModelFamily::OpenModel,
            UsageFamily::OpenMaas
        )
    ) {
        return Err(invalid(
            "Usage contract must match the registry model family",
        ));
    }
    let mut billing = service.billing.clone().unwrap_or_default();
    if billing.resale_billable || billing.managed_execution.is_some() {
        return Err(invalid(
            "Model tariffs cannot use resale or managed execution billing",
        ));
    }
    let now = Utc::now();
    let mut tariff = ModelTariff {
        id: Uuid::new_v4().to_string(),
        revision: model
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Revision exhausted"))?,
        service_id: service.id.clone(),
        model_id: model.id.clone(),
        public_model: model.public_id.clone(),
        lane: input.lane,
        variant: input.variant,
        contract: input.contract,
        bounds: input.bounds,
        components: input.components,
        source: input.source,
        status: TariffStatus::Pending,
        sync_error: None,
        created_at: now,
        updated_at: now,
    };
    validate(&mut tariff, &service.slug)?;
    let pricing = lane_mut(&mut billing, tariff.lane).get_or_insert_with(|| LanePricing {
        metric: crate::models::service_billing::BillingMetric::Requests,
        credits_per_unit: "0".into(),
        lago_metric_code: String::new(),
        sync_status: PricingSyncStatus::Pending,
        sync_error: None,
        components: vec![],
        operations: vec![],
        model_prices: vec![],
    });
    if let Some(binding) = pricing
        .model_prices
        .iter_mut()
        .find(|b| b.model_id == tariff.model_id && b.variant == tariff.variant)
    {
        binding.pending_tariff_id = Some(tariff.id.clone());
        for c in &tariff.components {
            if !binding.allowance_metrics.contains(&c.metric) {
                binding.allowance_metrics.push(c.metric);
            }
        }
    } else {
        if pricing.model_prices.len() >= 100 {
            return Err(invalid("Model price binding limit reached"));
        }
        pricing.model_prices.push(ModelPriceBinding {
            allowance_metrics: tariff.components.iter().map(|c| c.metric).collect(),
            model_id: tariff.model_id.clone(),
            variant: tariff.variant.clone(),
            active_tariff_id: None,
            pending_tariff_id: Some(tariff.id.clone()),
        });
    }
    let context = (
        db.clone(),
        actor.clone(),
        model,
        service,
        billing,
        tariff.clone(),
        original_billing,
    );
    let key = audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))?;
    transaction_service::run_db_only(
        db,
        &context,
        |session, (db, actor, model, service, billing, tariff, original_billing)| {
            Box::pin(async move {
                if !crate::services::tools_manager_service::fence_role_authority(
                    db,
                    session,
                    &actor.user_id,
                    None,
                )
                .await?
                {
                    return Err(AppError::Forbidden("Admin authority revoked".into()));
                }
                let model_changed = db
                    .collection::<PlatformModel>(MODELS)
                    .update_one(
                        doc! {
                            "_id": &model.id,
                            "revision": model.revision,
                            "billing_service_id": &service.id,
                        },
                        doc! {"$set": {
                            "revision": tariff.revision,
                            "updated_at": bson::DateTime::from_chrono(tariff.created_at),
                        }},
                    )
                    .session(&mut *session)
                    .await?;
                if model_changed.matched_count != 1 {
                    return Err(AppError::Conflict("Model changed; reload".into()));
                }
                let billing_changed = db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {
                            "_id": &service.id,
                            "billing": original_billing.clone(),
                            "is_active": true,
                            "platform_key.purpose": "model_offering",
                        },
                        doc! {"$set": {
                            "billing": bson::to_bson(billing)
                                .map_err(|e| AppError::Internal(e.to_string()))?,
                        }},
                    )
                    .session(&mut *session)
                    .await?;
                if billing_changed.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "Billing configuration changed; reload".into(),
                    ));
                }
                db.collection::<ModelTariff>(COLLECTION_NAME)
                    .insert_one(tariff)
                    .session(&mut *session)
                    .await?;
                audit_service::log_actor_event_in_session(
                    db,
                    session,
                    key,
                    actor,
                    "platform_model_tariff_created",
                    serde_json::json!({
                        "model_id": model.id,
                        "tariff_id": tariff.id,
                        "revision": tariff.revision,
                        "service_id": service.id,
                    }),
                )
                .await?;
                Ok(())
            })
        },
    )
    .await?;
    if tariff.components.iter().all(|c| c.explicit_free) && publish(db, &tariff).await? {
        tariff.status = TariffStatus::Synced;
    }
    Ok(tariff)
}

/// Revision-qualified codes are never edited/reused. Old charges remain in the
/// plan for unsettled events and billing-period obligations; no destructive GC.
pub async fn sync(
    db: &Database,
    lago: &dyn LagoApi,
    plan: &str,
    t: &ModelTariff,
) -> AppResult<bool> {
    if t.status != TariffStatus::Pending {
        return Ok(false);
    }
    let Some(service) = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&t.service_id})
        .await?
    else {
        // A deleted offering cannot publish. Drain its orphaned revision so
        // bounded oldest-first retries still reach live models.
        db.collection::<ModelTariff>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&t.id,"status":"pending"},
                doc! {"$set":{"status":"retired","updated_at":bson::DateTime::now()}},
            )
            .await?;
        return Ok(false);
    };
    let is_pending = service
        .billing
        .as_ref()
        .and_then(|b| lane(b, t.lane))
        .is_some_and(|l| {
            l.model_prices
                .iter()
                .any(|b| b.pending_tariff_id.as_deref() == Some(&t.id))
        });
    if !is_pending {
        db.collection::<ModelTariff>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&t.id,"status":"pending"},
                doc! {"$set":{"status":"retired","updated_at":bson::DateTime::now()}},
            )
            .await?;
        return Ok(false);
    }
    if !service.is_active {
        db.collection::<ModelTariff>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&t.id,"status":"pending"},
                doc! {"$set":{"updated_at":bson::DateTime::now()}},
            )
            .await?;
        return Ok(false);
    }
    let mut checked = t.clone();
    validate(&mut checked, &service.slug)?;
    if checked.components != t.components {
        return Err(unavailable("invalid_tariff"));
    }
    for c in &t.components {
        if c.explicit_free {
            continue;
        }
        let input = ServicePriceSync {
            metric_code: c.lago_metric_code.clone(),
            metric_name: format!("{} {} t{}", t.public_model, c.key, t.revision),
            metric_description: format!("NyxID model usage: {}", c.metric.label()),
            credits_per_unit: c.credits_per_unit.clone(),
        };
        if super::taxes::sync_charge(db, lago, plan, &input)
            .await
            .is_err()
        {
            db.collection::<ModelTariff>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &t.id, "status": "pending"},
                    doc! {"$set": {
                        "sync_error": "Lago synchronization incomplete; retry pending",
                        "updated_at": bson::DateTime::now(),
                    }},
                )
                .await?;
            return Ok(false);
        }
        let pico = amounts::decimal_to_pico(&c.credits_per_unit)
            .ok_or_else(|| invalid("Invalid price"))?;
        db.collection::<BillingRateCache>(RATES)
            .replace_one(
                doc! {"_id":BillingRateCache::cache_id(&c.lago_metric_code,None)},
                BillingRateCache {
                    id: BillingRateCache::cache_id(&c.lago_metric_code, None),
                    lago_metric_code: c.lago_metric_code.clone(),
                    model: None,
                    credits_per_unit_micros: pico / 1_000_000,
                    credits_per_unit_pico: Some(pico),
                    synced_at: Utc::now(),
                    retired_at: None,
                },
            )
            .upsert(true)
            .await?;
    }
    publish(db, t).await
}
async fn publish(db: &Database, t: &ModelTariff) -> AppResult<bool> {
    let context = (db.clone(), t.clone());
    transaction_service::run_db_only(db, &context, |session, (db, t)| {
        Box::pin(async move {
            for component in &t.components {
                if component.explicit_free {
                    continue;
                }
                let rate = db.collection::<BillingRateCache>(RATES)
                    .find_one(doc! {
                        "_id": BillingRateCache::cache_id(&component.lago_metric_code, None),
                        "retired_at": bson::Bson::Null,
                    })
                    .session(&mut *session).await?
                    .ok_or_else(|| unavailable("rate_missing_during_publication"))?;
                if amounts::rate_pico(rate.credits_per_unit_pico, rate.credits_per_unit_micros)
                    != i128::from(amounts::decimal_to_pico(&component.credits_per_unit)
                        .ok_or_else(|| invalid("Invalid price"))?)
                {
                    return Err(unavailable("rate_changed_during_publication"));
                }
            }
            let path = format!("billing.{}.model_prices", field(t.lane));
            // Publishing and moving the binding commit together. A stale sync cannot
            // activate an abandoned replacement or partially update a live tariff.
            let changed = db.collection::<DownstreamService>(SERVICES)
                .update_one(
                    doc! {
                        "_id": &t.service_id,
                        &path: {"$elemMatch": {"model_id": &t.model_id, "pending_tariff_id": &t.id}},
                        "is_active": true,
                        "platform_key.purpose": "model_offering",
                    },
                    doc! {"$set": {
                        format!("{path}.$.active_tariff_id"): &t.id,
                        format!("{path}.$.pending_tariff_id"): bson::Bson::Null,
                    }},
                )
                .session(&mut *session).await?;
            if changed.matched_count == 0 {
                return Ok(false);
            }
            let tariff_changed = db.collection::<ModelTariff>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &t.id, "status": "pending"},
                    doc! {"$set": {
                        "status": "synced",
                        "sync_error": bson::Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    }},
                )
                .session(&mut *session).await?;
            if tariff_changed.matched_count != 1 {
                return Err(AppError::Conflict("Tariff changed during sync".into()));
            }
            Ok(true)
        })
    }).await
}

pub async fn retry_pending(db: &Database, lago: &dyn LagoApi, plan: &str) -> AppResult<u64> {
    let pending: Vec<ModelTariff> = db
        .collection::<ModelTariff>(COLLECTION_NAME)
        .find(doc! {"status":"pending"})
        .sort(doc! {"updated_at":1})
        .limit(50)
        .await?
        .try_collect()
        .await?;
    let mut count = 0;
    for t in pending {
        count += u64::from(sync(db, lago, plan, &t).await?);
    }
    Ok(count)
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Explanation {
    pub model_id: String,
    pub variant: BTreeMap<String, String>,
    pub lane: TariffLane,
    pub active_tariff_id: Option<String>,
    pub pending_tariff_id: Option<String>,
    pub available: bool,
    pub blocking_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_input: Option<super::super::platform_model_execution::images::PublicLimits>,
    pub components: Vec<TariffComponent>,
}

pub fn explain(
    service: &DownstreamService,
    tariffs: &[ModelTariff],
    rates: &HashMap<String, BillingRateCache>,
    now: DateTime<Utc>,
    ttl: u64,
    billing_enabled: bool,
) -> Vec<Explanation> {
    let mut out = vec![];
    if let Some(b) = &service.billing {
        for (l, class) in [
            (TariffLane::Byok, CredentialClass::UserOwned),
            (TariffLane::PlatformKey, CredentialClass::NyxidManagedMaster),
        ] {
            if let Some(p) = lane(b, l) {
                for binding in &p.model_prices {
                    let selected = select(
                        service,
                        &binding.model_id,
                        class,
                        &binding.variant,
                        tariffs,
                        rates,
                        now,
                        ttl,
                    );
                    let selected = selected.and_then(|t| {
                        super::config::route_context(service, class, false)
                            .with_model_tariff(t.clone())
                            .map(|_| t)
                            .map_err(|_| "incompatible_route")
                    });
                    let reason = match &selected {
                        Err(reason) => Some(*reason),
                        Ok(t) if t.paid() && !billing_enabled => Some("billing_disabled"),
                        _ => None,
                    };
                    out.push(Explanation {
                        model_id: binding.model_id.clone(),
                        variant: binding.variant.clone(),
                        lane: l,
                        active_tariff_id: binding.active_tariff_id.clone(),
                        pending_tariff_id: binding.pending_tariff_id.clone(),
                        available: reason.is_none(),
                        image_input: None,
                        blocking_reason: reason.map(str::to_owned),
                        components: selected.map(|t| t.tariff.components).unwrap_or_default(),
                    });
                }
            }
        }
    }
    out
}

/// Transactional authority fence shared by all model component reservations.
pub(super) async fn fence_admission(
    db: &Database,
    session: &mut mongodb::ClientSession,
    selected: &SelectedTariff,
) -> AppResult<()> {
    let t = selected.tariff();
    let path = format!("billing.{}.model_prices", field(t.lane));
    let changed = db
        .collection::<bson::Document>(SERVICES)
        .update_one(
            doc! {
                "_id": &t.service_id,
                "is_active": true,
                "platform_key.purpose": "model_offering",
                "billing.resale_billable": {"$ne": true},
                path: {"$elemMatch": {"model_id": &t.model_id, "active_tariff_id": &t.id}},
            },
            doc! {"$inc": {"model_price_admission_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if changed.matched_count != 1 {
        return Err(unavailable("binding_changed"));
    }
    if db
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.id,"status":"synced"},
            doc! {"$inc":{"admission_fence":1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(unavailable("tariff_inactive"));
    }
    Ok(())
}

pub async fn retire(
    db: &Database,
    actor: &AuditActor,
    model_id: &str,
    tariff_id: &str,
    revision: i64,
) -> AppResult<()> {
    let key = audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))?;
    let t = db
        .collection::<ModelTariff>(COLLECTION_NAME)
        .find_one(doc! {"_id":tariff_id,"model_id":model_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Tariff not found".into()))?;
    let context = (db.clone(), actor.clone(), t, revision);
    transaction_service::run_db_only(db, &context, |session, (db, actor, t, revision)| {
        Box::pin(async move {
            if !crate::services::tools_manager_service::fence_role_authority(
                db,
                session,
                &actor.user_id,
                None,
            )
            .await?
            {
                return Err(AppError::Forbidden("Admin authority revoked".into()));
            }
            if db
                .collection::<PlatformModel>(MODELS)
                .update_one(
                    doc! {"_id":&t.model_id,"revision":revision},
                    doc! {"$inc":{"revision":1_i64},"$set":{"updated_at":bson::DateTime::now()}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Err(AppError::Conflict("Model changed; reload".into()));
            }
            db.collection::<ModelTariff>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&t.id},
                    doc! {"$set":{"status":"retired","updated_at":bson::DateTime::now()}},
                )
                .session(&mut *session)
                .await?;
            let path = format!("billing.{}.model_prices", field(t.lane));
            for binding in ["active_tariff_id", "pending_tariff_id"] {
                db.collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {"_id":&t.service_id,format!("{path}.{binding}"):&t.id},
                        doc! {"$set":{format!("{path}.$.{binding}"):bson::Bson::Null}},
                    )
                    .session(&mut *session)
                    .await?;
            }
            let codes: Vec<&str> = t
                .components
                .iter()
                .map(|c| c.lago_metric_code.as_str())
                .collect();
            db.collection::<BillingRateCache>(RATES)
                .update_many(
                    doc! {"lago_metric_code":{"$in":codes}},
                    doc! {"$set":{"retired_at":bson::DateTime::now()}},
                )
                .session(&mut *session)
                .await?;
            audit_service::log_actor_event_in_session(
                db,
                session,
                key,
                actor,
                "platform_model_tariff_retired",
                serde_json::json!({"model_id":t.model_id,"tariff_id":t.id}),
            )
            .await?;
            Ok(())
        })
    })
    .await
}

/// Rechecked in the ordinary catalog mutation transaction so a stale form
/// cannot overwrite a concurrently published tariff binding.
pub fn validate_catalog_update(
    current: Option<&ServiceBilling>,
    requested: Option<&ServiceBilling>,
) -> AppResult<()> {
    for l in [TariffLane::Byok, TariffLane::PlatformKey] {
        let before = current
            .and_then(|b| lane(b, l))
            .map(|p| p.model_prices.as_slice())
            .unwrap_or_default();
        let after = requested
            .and_then(|b| lane(b, l))
            .map(|p| p.model_prices.as_slice())
            .unwrap_or_default();
        if before != after {
            return Err(AppError::Conflict(
                "Model tariff bindings changed; reload the service".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "model_tariff_tests.rs"]
pub(crate) mod tests;
