//! Durable admission for bounded, platform-funded provider operations.
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        managed_execution::{COLLECTION_NAME, ManagedExecution, ManagedExecutionState as State},
        service_billing::{BillingMetric, PlatformUsage, PricingSyncStatus},
        usage_meter::{COLLECTION_NAME as METERS, CredentialClass, UsageMeterRow},
    },
    services::{
        api_key_mutation_service as tx,
        billing::{
            self, BillingIngress, BillingRouteContext, BillingService, MeteredProxyContext,
            NodeIntent,
        },
    },
};

const ADMISSION_SECONDS: i64 = 120;
const RECOVERY_BATCH: i64 = 100;

#[derive(Clone)]
pub(crate) struct ManagedExecutionHandle {
    id: String,
    route_digest: String,
}

impl ManagedExecutionHandle {
    pub(crate) fn execution_id(&self) -> &str {
        &self.id
    }
}

fn unavailable() -> AppError {
    AppError::BillingNotConfigured(
        "Managed execution requires a complete synced tariff and bounded platform admission".into(),
    )
}

pub(crate) fn validate_route(ctx: &BillingRouteContext) -> AppResult<()> {
    let billing = ctx.managed_billing.as_ref().ok_or_else(unavailable)?;
    let contract = billing.managed_execution.as_ref().ok_or_else(unavailable)?;
    let lane = billing
        .platform_key_pricing
        .as_ref()
        .ok_or_else(unavailable)?;
    if ctx.credential_class != CredentialClass::NyxidManagedMaster
        || ctx.node_intent != NodeIntent::Direct
        || ctx.auth_method != "bearer"
        || !matches!(ctx.ingress, BillingIngress::Proxy | BillingIngress::Mcp)
        || ctx.pool_attempt.is_some()
        || ctx.public_admission.is_some()
        || ctx.resale.is_some()
        || billing.resale_billable
        || ctx.catalog_service_id.as_deref().is_none_or(str::is_empty)
        || lane.metric != BillingMetric::InputTokens
        || lane.sync_status != PricingSyncStatus::Synced
        || !lane.operations.is_empty()
        || lane.components.len() != 2
        || contract.model.is_empty()
        || contract.tariff_version.is_empty()
        || contract.tariff_source.is_empty()
        || contract.tariff_verified_at > Utc::now()
        || [
            contract.max_input_tokens,
            contract.max_output_tokens,
            contract.max_request_bytes,
            contract.max_response_bytes,
        ]
        .into_iter()
        .any(|value| value == 0 || value > i64::MAX as u64)
        || ctx.request_bytes < 0
        || ctx.request_bytes as u64 > contract.max_request_bytes
        || ctx.platform_metric != BillingMetric::InputTokens
        || ctx.platform_components.len() != 2
    {
        return Err(unavailable());
    }
    for metric in [BillingMetric::OutputTokens, BillingMetric::CacheReadTokens] {
        if lane
            .components
            .iter()
            .filter(|c| c.metric == metric && c.sync_status == PricingSyncStatus::Synced)
            .count()
            != 1
        {
            return Err(unavailable());
        }
    }
    if lane.lago_metric_code.is_empty()
        || lane
            .credits_per_unit
            .parse::<Credits>()
            .map_err(|_| unavailable())?
            <= Credits::ZERO
    {
        return Err(unavailable());
    }
    for component in &lane.components {
        let price = component
            .credits_per_unit
            .parse::<Credits>()
            .map_err(|_| unavailable())?;
        if component.lago_metric_code.is_empty()
            || price < Credits::ZERO
            || (component.metric == BillingMetric::OutputTokens && price == Credits::ZERO)
        {
            return Err(unavailable());
        }
    }
    ctx.require_platform_price()
}

pub(crate) fn validate_reserved_rates(
    ctx: &BillingRouteContext,
    layers: &[billing::reservation::LayerReservation],
) -> AppResult<()> {
    validate_route(ctx)?;
    let lane = ctx
        .managed_billing
        .as_ref()
        .and_then(|b| b.platform_key_pricing.as_ref())
        .ok_or_else(unavailable)?;
    let expected = std::iter::once((
        lane.metric,
        lane.lago_metric_code.as_str(),
        lane.credits_per_unit.as_str(),
    ))
    .chain(lane.components.iter().map(|c| {
        (
            c.metric,
            c.lago_metric_code.as_str(),
            c.credits_per_unit.as_str(),
        )
    }));
    if layers.len() != 3 {
        return Err(unavailable());
    }
    for (metric, code, rate) in expected {
        let layer = layers
            .iter()
            .find(|layer| layer.metric == metric && layer.lago_metric_code == code)
            .ok_or_else(unavailable)?;
        let configured = rate.parse::<Credits>().map_err(|_| unavailable())?;
        if configured.pico()
            != billing::amounts::rate_pico(
                layer.credits_per_unit_pico,
                layer.credits_per_unit_micros,
            )
            || layer.estimated_quantity != ctx.estimated_quantity(metric)
        {
            return Err(unavailable());
        }
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn new_record(
    ctx: &BillingRouteContext,
    request_digest: &str,
    catalog_digest: &str,
) -> AppResult<ManagedExecution> {
    validate_route(ctx)?;
    if !is_digest(request_digest) || !is_digest(catalog_digest) {
        return Err(AppError::ValidationError(
            "Invalid managed request identity".into(),
        ));
    }
    let now = Utc::now();
    let billing = ctx.managed_billing.clone().ok_or_else(unavailable)?;
    let route_digest = super::mcp_service::canonical_sha256(serde_json::json!([
        "managed-execution.v1",
        ctx.billing_request_id,
        ctx.billing_owner_id,
        ctx.actor_user_id,
        ctx.api_key_id,
        ctx.user_service_id,
        ctx.catalog_service_id,
        ctx.service_slug,
        ctx.ingress.as_str(),
        ctx.request_bytes,
        request_digest,
        catalog_digest,
        billing,
    ]));
    Ok(ManagedExecution {
        id: uuid::Uuid::new_v4().to_string(),
        billing_request_id: ctx.billing_request_id.clone(),
        billing_owner_id: ctx.billing_owner_id.clone(),
        actor_user_id: ctx.actor_user_id.clone(),
        api_key_id: ctx.api_key_id.clone(),
        user_service_id: ctx.user_service_id.clone(),
        catalog_service_id: ctx.catalog_service_id.clone().ok_or_else(unavailable)?,
        service_slug: ctx.service_slug.clone(),
        ingress: ctx.ingress.as_str().into(),
        request_bytes: ctx.request_bytes,
        request_digest: request_digest.into(),
        catalog_digest: catalog_digest.into(),
        route_digest,
        billing,
        state: State::Prepared,
        maximum: Credits::ZERO,
        meter_row_ids: vec![],
        usage: None,
        provider_request_id: None,
        pending_reason: None,
        expires_at: now + Duration::seconds(ADMISSION_SECONDS),
        next_reconcile_at: now + Duration::seconds(ADMISSION_SECONDS),
        created_at: now,
        updated_at: now,
    })
}

fn route(row: &ManagedExecution) -> AppResult<BillingRouteContext> {
    let contract = row
        .billing
        .managed_execution
        .as_ref()
        .ok_or_else(unavailable)?;
    let ingress = match row.ingress.as_str() {
        "proxy" => BillingIngress::Proxy,
        "mcp" => BillingIngress::Mcp,
        _ => return Err(unavailable()),
    };
    let mut ctx = BillingRouteContext::new(
        ingress,
        row.billing_request_id.clone(),
        row.billing_owner_id.clone(),
        row.actor_user_id.clone(),
        row.api_key_id.clone(),
        row.user_service_id.clone(),
        Some(row.catalog_service_id.clone()),
        row.service_slug.clone(),
        NodeIntent::Direct,
        "bearer".into(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::InputTokens,
        Some(&row.billing),
        false,
    )
    .with_platform_metering(true)
    .with_native_text_bound(
        contract.max_input_tokens as i64,
        contract.max_output_tokens as i64,
        contract.max_response_bytes as i64,
    );
    ctx.request_bytes = row.request_bytes;
    ctx.managed_admission = Some(billing::route_context::ManagedBillingAdmission {
        execution_id: row.id.clone(),
        route_digest: row.route_digest.clone(),
        expires_at: row.expires_at,
    });
    Ok(ctx)
}

pub(crate) async fn prepare(
    db: &Database,
    billing: &BillingService,
    ctx: BillingRouteContext,
    request_digest: &str,
    catalog_digest: &str,
) -> AppResult<ManagedExecutionHandle> {
    let row = new_record(&ctx, request_digest, catalog_digest)?;
    db.collection::<ManagedExecution>(COLLECTION_NAME)
        .insert_one(&row)
        .await?;
    let ctx = route(&row)?;
    if let Err(error) = billing.open_managed(&ctx).await {
        cancel(db, &row.id).await?;
        return Err(error);
    }
    Ok(ManagedExecutionHandle {
        id: row.id,
        route_digest: row.route_digest,
    })
}

pub(crate) async fn admit_dispatch(
    db: &Database,
    handle: &ManagedExecutionHandle,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db.clone(), handle.id.clone(), handle.route_digest.clone()), |session, (db,id,digest)| Box::pin(async move {
        let result: AppResult<()> = async {
            let now = Utc::now();
            let calls = db.collection::<ManagedExecution>(COLLECTION_NAME);
            let row = calls.find_one(doc! {"_id":id,"route_digest":digest,"state":"funded","expires_at":{"$gt":bson::DateTime::from_chrono(now)}})
                .session(&mut *session).await?.ok_or_else(|| AppError::Conflict("Managed execution is no longer dispatchable".into()))?;
            if row.meter_row_ids.len() != 3 { return Err(unavailable()); }
            for id in &row.meter_row_ids {
                if db.collection::<UsageMeterRow>(METERS).update_one(doc! {
                    "_id":id,"managed_execution_id":&row.id,"billing_request_id":&row.billing_request_id,
                    "billing_owner_id":&row.billing_owner_id,"status":"reserved","released":false,"forwarded":false,
                },doc! {"$set":{"forwarded":true,"status":"forwarded","updated_at":bson::DateTime::from_chrono(now)}})
                    .session(&mut *session).await?.matched_count != 1 {
                    return Err(AppError::Conflict("Managed reservation was released".into()));
                }
            }
            if calls.update_one(doc! {"_id":&row.id,"state":"funded"},doc! {"$set":{"state":"dispatch_admitted","updated_at":bson::DateTime::from_chrono(now)}})
                .session(&mut *session).await?.matched_count != 1 { return Err(unavailable()); }
            Ok(())
        }.await;
        tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error)
}

pub(crate) async fn mark_pending(
    db: &Database,
    handle: &ManagedExecutionHandle,
    reason: &str,
) -> AppResult<()> {
    // Reasons are classifications, never provider responses, URLs, or error strings.
    let reason = match reason {
        "timeout"
        | "transport_error"
        | "missing_usage"
        | "invalid_usage"
        | "response_too_large"
        | "provider_error"
        | "caller_disconnected"
        | "worker_interrupted" => reason,
        _ => "outcome_unknown",
    };
    db.collection::<ManagedExecution>(COLLECTION_NAME).update_one(doc! {
        "_id":&handle.id,"route_digest":&handle.route_digest,"state":{"$in":["dispatch_admitted","pending"]},
    },doc! {"$set":{"state":"pending","pending_reason":reason,"updated_at":bson::DateTime::now()}}).await?;
    Ok(())
}

fn validate_usage(row: &ManagedExecution, usage: &PlatformUsage) -> AppResult<()> {
    let contract = row
        .billing
        .managed_execution
        .as_ref()
        .ok_or_else(unavailable)?;
    let total = usage
        .input_tokens
        .checked_add(usage.cache_read_tokens)
        .and_then(|input| input.checked_add(usage.output_tokens));
    if usage.requests != 1
        || usage.bytes < 0
        || usage.bytes as u64
            > contract
                .max_request_bytes
                .saturating_add(contract.max_response_bytes)
        || usage.tokens < 0
        || total != Some(usage.tokens)
        || total.is_none_or(|n| n as u64 > contract.max_input_tokens)
        || usage.input_tokens < 0
        || usage.output_tokens < 0
        || usage.cache_read_tokens < 0
        || usage.cache_write_tokens != 0
        || usage.images != 0
        || usage.voice_seconds != 0
        || usage
            .input_tokens
            .checked_add(usage.cache_read_tokens)
            .is_none_or(|n| n as u64 > contract.max_input_tokens)
        || usage.output_tokens as u64 > contract.max_output_tokens
        || usage.audio_tokens.as_ref().is_some_and(|v| !v.is_empty())
        || usage.image_tokens.as_ref().is_some_and(|v| !v.is_empty())
    {
        return Err(AppError::ValidationError(
            "Managed usage is outside the admitted bounds".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn complete(
    db: &Database,
    _billing: &BillingService,
    handle: &ManagedExecutionHandle,
    usage: PlatformUsage,
    provider_request_id: Option<&str>,
) -> AppResult<()> {
    let calls = db.collection::<ManagedExecution>(COLLECTION_NAME);
    let row = calls
        .find_one(doc! {"_id":&handle.id,"route_digest":&handle.route_digest})
        .await?
        .ok_or_else(|| AppError::NotFound("Managed execution not found".into()))?;
    validate_usage(&row, &usage)?;
    let provider_request_id = provider_request_id.filter(|value| {
        !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    });
    let encoded = bson::to_bson(&usage)
        .map_err(|_| AppError::Internal("Managed usage encoding failed".into()))?;
    let changed = calls.update_one(doc! {"_id":&row.id,"state":{"$in":["dispatch_admitted","pending"]},"usage":bson::Bson::Null},
        doc! {"$set":{"state":"evidence_ready","usage":encoded,"provider_request_id":provider_request_id,
            "pending_reason":bson::Bson::Null,"next_reconcile_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}}).await?;
    if changed.matched_count != 1 {
        let existing = calls
            .find_one(doc! {"_id":&row.id})
            .await?
            .ok_or_else(unavailable)?;
        if !matches!(existing.state, State::EvidenceReady | State::Settled)
            || existing.usage.as_ref() != Some(&usage)
        {
            return Err(AppError::Conflict(
                "Managed usage already finalized or execution not dispatched".into(),
            ));
        }
    }
    finish_evidence(db, &row.id).await
}

async fn finish_evidence(db: &Database, id: &str) -> AppResult<()> {
    let calls = db.collection::<ManagedExecution>(COLLECTION_NAME);
    let row = calls
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)?;
    if row.state == State::Settled {
        return Ok(());
    }
    if row.state != State::EvidenceReady {
        return Err(unavailable());
    }
    let usage = row.usage.clone().ok_or_else(unavailable)?;
    validate_usage(&row, &usage)?;
    let ctx = route(&row)?;
    let model = row
        .billing
        .managed_execution
        .as_ref()
        .ok_or_else(unavailable)?
        .model
        .clone();
    billing::meter::settle(
        db,
        &MeteredProxyContext::from_route(&ctx),
        usage,
        None,
        Some(model),
    )
    .await?;
    let settled = db.collection::<UsageMeterRow>(METERS).count_documents(doc! {
        "_id":{"$in":&row.meter_row_ids},"managed_execution_id":&row.id,"released":true,"funding.settled":true,
    }).await?;
    if settled != row.meter_row_ids.len() as u64 || settled != 3 {
        return Err(AppError::Conflict(
            "Managed settlement is still pending".into(),
        ));
    }
    calls
        .update_one(
            doc! {"_id":id,"state":"evidence_ready"},
            doc! {"$set":{"state":"settled","updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

pub(crate) async fn cancel_unforwarded(
    db: &Database,
    handle: &ManagedExecutionHandle,
) -> AppResult<()> {
    cancel(db, &handle.id).await
}

async fn cancel(db: &Database, id: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db.clone(),id.to_owned()),|session,(db,id)|Box::pin(async move {
        let result: AppResult<()> = async {
            let calls = db.collection::<ManagedExecution>(COLLECTION_NAME);
            let Some(row) = calls.find_one(doc! {"_id":id,"state":{"$in":["prepared","funded","cancelling"]}})
                .session(&mut *session).await? else { return Ok(()); };
            // The execution document conflicts with the dispatch transaction.
            calls.update_one(doc! {"_id":&row.id},doc! {"$set":{"state":"cancelling","updated_at":bson::DateTime::now()}})
                .session(&mut *session).await?;
            for id in &row.meter_row_ids {
                let meter = db.collection::<UsageMeterRow>(METERS).find_one(doc! {"_id":id,"managed_execution_id":&row.id})
                    .session(&mut *session).await?.ok_or_else(unavailable)?;
                if meter.released { continue; }
                if meter.forwarded || meter.status != crate::models::usage_meter::UsageStatus::Reserved { return Err(unavailable()); }
                if meter.reserved_credits > Credits::ZERO {
                    let wallet_id = meter.wallet_id.as_deref().ok_or_else(unavailable)?;
                    if db.collection::<bson::Document>(crate::models::billing_wallet::COLLECTION_NAME).update_one(doc! {
                        "_id":wallet_id,"owner_id":&row.billing_owner_id,"reserved_credits":{"$gte":meter.reserved_credits.decimal()},
                    },doc! {"$inc":{"reserved_credits":meter.reserved_credits.checked_neg()?.decimal()},"$set":{"updated_at":bson::DateTime::now()}})
                        .session(&mut *session).await?.matched_count != 1 { return Err(unavailable()); }
                }
                if db.collection::<UsageMeterRow>(METERS).update_one(doc! {"_id":id,"released":false,"forwarded":false,"status":"reserved"},
                    doc! {"$set":{"released":true,"status":"failed","last_error":"Managed execution cancelled before dispatch",
                        "updated_at":bson::DateTime::now(),"finalized_at":bson::DateTime::now()}})
                    .session(&mut *session).await?.matched_count != 1 { return Err(unavailable()); }
            }
            Ok(())
        }.await;
        tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error)?;
    let calls = db.collection::<ManagedExecution>(COLLECTION_NAME);
    let Some(row) = calls.find_one(doc! {"_id":id,"state":"cancelling"}).await? else {
        return Ok(());
    };
    let meters: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(METERS)
        .find(doc! {"_id":{"$in":&row.meter_row_ids}})
        .await?
        .try_collect()
        .await?;
    for meter in &meters {
        billing::funding::release_usage_reservations(db, meter).await?;
    }
    calls
        .update_one(
            doc! {"_id":id,"state":"cancelling"},
            doc! {"$set":{"state":"released","updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

pub(crate) async fn reconcile(db: &Database) -> AppResult<u64> {
    let now = Utc::now();
    let calls = db.collection::<ManagedExecution>(COLLECTION_NAME);
    let rows: Vec<ManagedExecution> = calls.find(doc! {"next_reconcile_at":{"$lte":bson::DateTime::from_chrono(now)},
        "state":{"$in":["prepared","funded","cancelling","dispatch_admitted","evidence_ready"]}})
        .sort(doc! {"next_reconcile_at":1,"_id":1}).limit(RECOVERY_BATCH).await?.try_collect().await?;
    let mut count = 0;
    for row in rows {
        let result = match row.state {
            State::Prepared|State::Funded if row.expires_at <= now => cancel(db,&row.id).await,
            State::Cancelling => cancel(db,&row.id).await,
            State::EvidenceReady => finish_evidence(db,&row.id).await,
            State::DispatchAdmitted if row.expires_at <= now => calls.update_one(doc! {"_id":&row.id,"state":"dispatch_admitted"},
                doc! {"$set":{"state":"pending","pending_reason":"worker_interrupted","updated_at":bson::DateTime::now()}}).await.map(|_|()).map_err(AppError::from),
            _ => Ok(()),
        };
        if result.is_err() {
            tracing::warn!(execution_id=%row.id,"Managed execution recovery will retry");
            calls.update_one(doc! {"_id":&row.id},doc! {"$set":{"next_reconcile_at":bson::DateTime::from_chrono(now+Duration::seconds(30))}}).await?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}

fn visible_filter(actor_user_id: &str, wallet_owner_id: Option<&str>) -> bson::Document {
    match wallet_owner_id {
        Some(owner) => doc! {"$or":[{"actor_user_id":actor_user_id},{"billing_owner_id":owner}]},
        None => doc! {"actor_user_id":actor_user_id},
    }
}

/// The caller supplies a verified personal wallet owner, never an arbitrary query owner.
pub(crate) async fn list_visible(
    db: &Database,
    actor_user_id: &str,
    wallet_owner_id: Option<&str>,
) -> AppResult<Vec<ManagedExecution>> {
    db.collection::<ManagedExecution>(COLLECTION_NAME)
        .find(visible_filter(actor_user_id, wallet_owner_id))
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(50)
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub(crate) async fn get_visible(
    db: &Database,
    actor_user_id: &str,
    wallet_owner_id: Option<&str>,
    id: &str,
) -> AppResult<ManagedExecution> {
    uuid::Uuid::parse_str(id).map_err(|_| AppError::NotFound("Execution not found".into()))?;
    let mut filter = visible_filter(actor_user_id, wallet_owner_id);
    filter.insert("_id", id);
    db.collection::<ManagedExecution>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Execution not found".into()))
}

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<ManagedExecution>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"billing_request_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("managed_billing_request".to_owned())
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"state":1,"next_reconcile_at":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"actor_user_id":1,"created_at":-1,"_id":-1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"billing_owner_id":1,"created_at":-1,"_id":-1})
                .build(),
        ])
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
