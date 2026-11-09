//! Private, funded platform-model executor. Its only entrance is the reserved
//! gateway; there is no generic proxy target or client-selected route.
pub(crate) mod adapters;
pub(crate) mod certification;
pub(crate) mod images;
mod stream;
mod token;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        downstream_service::{DownstreamService, PlatformKeyPurpose},
        platform_model::*,
        service_billing::BillingMetric,
        usage_meter::CredentialClass,
    },
    services::{
        billing::{self, BillingIngress, BillingRouteContext, BillingService, NodeIntent},
        platform_model_service as registry,
    },
};
use async_trait::async_trait;
use bson::{Document, doc};
use chrono::Utc;
use futures::{StreamExt, TryStreamExt};
use rand::distributions::{Distribution, WeightedIndex};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, OnceLock},
    time::Duration,
};
use token::TokenProvider;
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(crate) use adapters::Request;
const ATTEMPTS: &str = "platform_model_attempts";
const DEADLINE: Duration = Duration::from_secs(120);

pub(crate) struct Plan {
    pub model: PlatformModel,
    pub offering: DownstreamService,
    pub tariff: billing::model_tariffs::SelectedTariff,
    routes: Vec<ModelRoute>,
}
impl Plan {
    pub(crate) fn image_limits(&self) -> Option<images::PublicLimits> {
        images::public_limits(&self.model, &self.routes, self.tariff.tariff())
    }
    #[cfg(test)]
    pub(crate) fn test_routes(&self) -> Vec<ModelRoute> {
        self.routes.clone()
    }
    pub async fn load(
        db: &mongodb::Database,
        person: &str,
        public_id: &str,
        scope: Option<&std::collections::HashSet<String>>,
        ttl: u64,
        request: &Request,
    ) -> AppResult<Self> {
        registry::require_execution_ready(db).await?;
        if request.has_images() {
            crate::services::security_readiness_service::require(db, images::READINESS).await?;
        }
        let metadata = registry::resolve_route_metadata(db, person, public_id, scope).await?;
        let (model, routing) = metadata.into_execution_parts();
        let offering = db
            .collection::<DownstreamService>("downstream_services")
            .find_one(
                doc! {"_id":model.billing_service_id.as_deref().ok_or_else(registry::unavailable)?},
            )
            .await?
            .ok_or_else(registry::unavailable)?;
        let tariffs = billing::model_tariffs::load_for(db, std::slice::from_ref(&offering)).await?;
        let rates = billing::config::load_rates_for(db, std::slice::from_ref(&offering)).await?;
        let tariff = billing::model_tariffs::select(
            &offering,
            &model.id,
            CredentialClass::NyxidManagedMaster,
            &BTreeMap::new(),
            &tariffs,
            &rates,
            Utc::now(),
            ttl,
        )
        .map_err(billing::model_tariffs::unavailable)?;
        let mut routes: Vec<_> = routing
            .routes
            .into_iter()
            .filter(|r| r.enabled && adapters::prepare(&model, r, tariff.tariff(), request).is_ok())
            .collect();
        if routes.is_empty() {
            return Err(adapters::unsupported());
        }
        // Priority first, weighted sampling without replacement inside each tier.
        routes.sort_by_key(|r| r.priority);
        let mut ordered = Vec::new();
        while !routes.is_empty() {
            let n = routes
                .iter()
                .take_while(|r| r.priority == routes[0].priority)
                .count();
            let index = WeightedIndex::new(routes[..n].iter().map(|r| r.weight))
                .map_err(|_| registry::unavailable())?
                .sample(&mut rand::thread_rng());
            ordered.push(routes.remove(index));
        }
        Ok(Self {
            model,
            offering,
            tariff,
            routes: ordered,
        })
    }
}

/// Implemented at the HTTP boundary. No token material enters the route planner.
#[async_trait]
pub(crate) trait Authority: Send + Sync {
    async fn check(&self, plan: &Plan) -> AppResult<()>;
    async fn before_send(&self) -> AppResult<()> {
        Ok(())
    }
}

pub(crate) struct Runtime {
    client: reqwest::Client,
    tokens: token::ServiceAccountTokens,
    slots: Mutex<HashMap<String, Arc<Semaphore>>>,
    cooldowns: Mutex<HashMap<String, Instant>>,
    #[cfg(test)]
    endpoint: Option<String>,
}
impl Runtime {
    fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(DEADLINE)
            .build()
            .expect("static model HTTP client");
        Self {
            tokens: token::ServiceAccountTokens::new(client.clone()),
            client,
            slots: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
            #[cfg(test)]
            endpoint: None,
        }
    }
    pub fn shared() -> Arc<Self> {
        static RUNTIME: OnceLock<Arc<Runtime>> = OnceLock::new();
        RUNTIME.get_or_init(|| Arc::new(Self::new())).clone()
    }
    #[cfg(test)]
    pub(crate) fn mocked(endpoint: &str) -> Self {
        let mut runtime = Self::new();
        runtime.tokens = token::ServiceAccountTokens::with_endpoint(
            runtime.client.clone(),
            format!("{endpoint}/token"),
        );
        runtime.endpoint = Some(endpoint.into());
        runtime
    }
    async fn slot(&self, key: &str) -> AppResult<tokio::sync::OwnedSemaphorePermit> {
        let slot = {
            let mut slots = self.slots.lock().await;
            if slots.len() >= 128 && !slots.contains_key(key) {
                let idle = slots
                    .iter()
                    .find(|(_, v)| Arc::strong_count(v) == 1)
                    .map(|(k, _)| k.clone());
                if let Some(idle) = idle {
                    slots.remove(&idle);
                } else {
                    return Err(AppError::ServicePoolCoolingDown);
                }
            }
            slots
                .entry(key.into())
                .or_insert_with(|| Arc::new(Semaphore::new(4)))
                .clone()
        };
        slot.try_acquire_owned()
            .map_err(|_| AppError::ServicePoolCoolingDown)
    }
    async fn cool(&self, key: String, duration: Duration) {
        let mut c = self.cooldowns.lock().await;
        c.retain(|_, until| *until > Instant::now());
        if c.len() < 256 || c.contains_key(&key) {
            c.insert(key, Instant::now() + duration.min(Duration::from_secs(60)));
        }
    }
}

pub(crate) async fn bounded_body(
    response: reqwest::Response,
    maximum: usize,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(Vec::new());
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| adapters::invalid_response())?;
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|n| n > maximum)
        {
            return Err(adapters::invalid_response());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn credential_version(row: &DownstreamService) -> String {
    // Ciphertext hash versions legacy rows too. Neither this key nor IDs are logged.
    let mut hash = Sha256::new();
    hash.update(row.id.as_bytes());
    hash.update(&row.credential_encrypted);
    hash.update(row.updated_at.timestamp_millis().to_be_bytes());
    format!("{:x}", hash.finalize())
}
async fn credential(db: &mongodb::Database, id: &str) -> AppResult<DownstreamService> {
    let row = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! {"_id":id,"is_active":true})
        .await?
        .ok_or_else(registry::unavailable)?;
    if row
        .platform_key
        .as_ref()
        .is_none_or(|c| !c.enabled || c.purpose != PlatformKeyPurpose::ModelRoute)
        || row.credential_encrypted.is_empty()
    {
        return Err(registry::unavailable());
    }
    crate::services::platform_key_service::validate_model_purpose(&row)
        .map_err(|_| registry::unavailable())?;
    Ok(row)
}

pub(crate) struct Context<'a> {
    pub db: &'a mongodb::Database,
    pub keys: &'a EncryptionKeys,
    pub billing: &'a BillingService,
    pub actor: &'a str,
    pub api_key: Option<String>,
    pub request_id: String,
    pub authority: &'a dyn Authority,
    pub cancel: CancellationToken,
    pub runtime: &'a Runtime,
}
pub(crate) struct Accepted {
    response: reqwest::Response,
    native: adapters::Native,
    metered: billing::MeteredProxyContext,
    deadline: Instant,
    _slot: tokio::sync::OwnedSemaphorePermit,
    _leases: Vec<crate::services::service_concurrency_service::Lease>,
}

pub(crate) async fn dispatch(
    ctx: &Context<'_>,
    plan: &Plan,
    request: &Request,
) -> AppResult<Accepted> {
    let deadline = Instant::now() + DEADLINE;
    if request.has_images() {
        crate::services::security_readiness_service::require(ctx.db, images::READINESS).await?;
    }
    let work = async {
        let mut attempts = 0;
        for route in &plan.routes {
            if attempts == 4 {
                break;
            }
            if ctx.cancel.is_cancelled() {
                return Err(AppError::ClientDisconnected);
            }
            ctx.authority.check(plan).await?;
            let prepared = adapters::prepare(&plan.model, route, plan.tariff.tariff(), request)?;
            let row = credential(ctx.db, &route.credential_service_id).await?;
            let version = credential_version(&row);
            let health = format!("{}:{version}", route.id);
            if ctx
                .runtime
                .cooldowns
                .lock()
                .await
                .get(&health)
                .is_some_and(|until| *until > Instant::now())
            {
                continue;
            }
            // Inspect the bounded route set, but spend the attempt budget only
            // on eligible routes. Cooling routes must not hide a healthy backup.
            attempts += 1;
            // Scope concurrency by project/location/model (not by credential), so
            // changing a key cannot evade the local project pressure budget.
            let resource = format!(
                "model:{}",
                format_args!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&route.target).map_err(|_| registry::unavailable())?
                    )
                )
            );
            let mut leases = Vec::new();
            for (person, limit) in [("platform_model_project_capacity", 8), (ctx.actor, 2)] {
                let policy = crate::models::service_concurrency::ServiceConcurrencyPolicy {
                    service_id: resource.clone(),
                    default_limit: Some(limit),
                    users: vec![],
                    orgs: vec![],
                };
                if let Some(lease) = crate::services::service_concurrency_service::acquire_policy(
                    ctx.db,
                    Some(&policy),
                    person,
                )
                .await?
                {
                    lease.cancel_on_loss(ctx.cancel.clone());
                    leases.push(lease);
                }
            }
            if let Some(lease) = crate::services::service_concurrency_service::acquire(
                ctx.db,
                &plan.offering,
                ctx.actor,
            )
            .await?
            {
                lease.cancel_on_loss(ctx.cancel.clone());
                leases.push(lease);
            }
            let slot = ctx.runtime.slot(&resource).await?;
            let raw =
                crate::services::catalog_credential_record_service::text(ctx.keys, &row).await?;
            let bearer = match route.target {
                RouteTarget::DirectProvider { .. } => raw,
                _ => match ctx.runtime.tokens.bearer(&version, &raw).await {
                    Ok(t) => t,
                    Err(_) => {
                        ctx.runtime.cool(health, Duration::from_secs(30)).await;
                        continue;
                    }
                },
            };
            token::validate_bearer(&bearer).map_err(|_| registry::unavailable())?;
            if ctx.cancel.is_cancelled() {
                return Err(AppError::ClientDisconnected);
            }
            ctx.authority.check(plan).await?;
            if credential_version(&credential(ctx.db, &route.credential_service_id).await?)
                != version
            {
                return Err(registry::unavailable());
            }
            let id = uuid::Uuid::new_v4().to_string();
            let route_ctx = BillingRouteContext::new(
                BillingIngress::LlmGateway,
                id.clone(),
                ctx.actor.into(),
                ctx.actor.into(),
                ctx.api_key.clone(),
                None,
                Some(plan.offering.id.clone()),
                Some(plan.offering.slug.clone()),
                NodeIntent::Direct,
                "platform_model".into(),
                CredentialClass::NyxidManagedMaster,
                BillingMetric::Tokens,
                plan.offering.billing.as_ref(),
                false,
            )
            .with_model_tariff(plan.tariff.clone())?;
            let metered = ctx.billing.open(&route_ctx).await?;
            let preflight = async {
                ctx.authority.check(plan).await?;
                if credential_version(&credential(ctx.db, &route.credential_service_id).await?)
                    != version
                {
                    return Err(registry::unavailable());
                }
                if ctx.cancel.is_cancelled() || Instant::now() >= deadline {
                    return Err(AppError::ClientDisconnected);
                }
                ctx.authority.before_send().await?;
                // Durably mark EVERY component before a byte can leave. A crash
                // after this barrier retains holds, never pool unknown-release.
                mark_dispatched(
                    ctx.db,
                    &metered,
                    &id,
                    doc! {
                        "routing_revision_id": &plan.model.routing_revision_id,
                        "route_id": &route.id,
                        "model_revision": plan.model.revision,
                        "credential_version": &version,
                    },
                )
                .await
            }
            .await;
            if let Err(error) = preflight {
                ctx.billing.fail(&metered, "model_pre_send_refused").await?;
                return Err(error);
            }
            #[allow(unused_mut)]
            let mut url = prepared.url;
            #[cfg(test)]
            if let Some(endpoint) = &ctx.runtime.endpoint {
                let parsed = url::Url::parse(&url).map_err(|_| adapters::unsupported())?;
                url = format!(
                    "{endpoint}{}{}",
                    parsed.path(),
                    parsed.query().map(|q| format!("?{q}")).unwrap_or_default()
                );
            }
            let mut send = ctx.runtime.client.post(&url).json(&prepared.body).header(
                "accept",
                if request.streaming {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            );
            if matches!(
                route.target,
                RouteTarget::DirectProvider {
                    provider: DirectProvider::Anthropic,
                    ..
                }
            ) {
                let mut key_header = http::HeaderValue::from_str(bearer.as_str())
                    .map_err(|_| registry::unavailable())?;
                key_header.set_sensitive(true);
                send = send
                    .header("x-api-key", key_header)
                    .header("anthropic-version", "2023-06-01");
            } else {
                send = send.bearer_auth(bearer.as_str());
            }
            match send.send().await {
                Err(error) if error.is_connect() => {
                    ctx.billing
                        .reject_before_send(&metered, "model_connect_unsent", false)
                        .await?;
                    complete_attempt(ctx.db, &id, "unsent").await?;
                    ctx.runtime.cool(health, Duration::from_secs(1)).await;
                }
                Err(_) => {
                    pending(ctx.db, &id, "transport_ambiguous").await?;
                    return Err(adapters::invalid_response());
                }
                Ok(response) if matches!(response.status().as_u16(), 401 | 403 | 429) => {
                    let status = response.status().as_u16();
                    let wait = crate::services::pool_failover::parse_retry_after(
                        response.headers(),
                        Utc::now(),
                    )
                    .unwrap_or(Duration::from_secs(1));
                    let zero = bounded_body(response, 16_384).await.is_ok_and(|body| {
                        adapters::zero_consumption(prepared.native, status, &body)
                    });
                    if !zero {
                        pending(ctx.db, &id, "uncertified_provider_refusal").await?;
                        return Err(adapters::invalid_response());
                    }
                    ctx.billing
                        .reject_before_send(&metered, "model_zero_consumption_refusal", false)
                        .await?;
                    complete_attempt(ctx.db, &id, "zero_consumption").await?;
                    ctx.runtime
                        .cool(
                            health,
                            wait + Duration::from_millis(rand::random::<u64>() % 250),
                        )
                        .await;
                }
                Ok(response) if !response.status().is_success() => {
                    pending(ctx.db, &id, "provider_refusal_ambiguous").await?;
                    return Err(adapters::invalid_response());
                }
                Ok(response) => {
                    let mime = response
                        .headers()
                        .get(http::header::CONTENT_TYPE)
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or("");
                    if !(if request.streaming {
                        mime.starts_with("text/event-stream")
                    } else {
                        mime.starts_with("application/json")
                    }) {
                        pending(ctx.db, &id, "invalid_content_type").await?;
                        return Err(adapters::invalid_response());
                    }
                    return Ok(Accepted {
                        response,
                        native: prepared.native,
                        metered,
                        deadline,
                        _slot: slot,
                        _leases: leases,
                    });
                }
            }
        }
        Err(AppError::ServicePoolCoolingDown)
    };
    tokio::select! {
        biased;
        _=ctx.cancel.cancelled()=>Err(AppError::ClientDisconnected),
        result=tokio::time::timeout_at(deadline,Box::pin(work))=>result.map_err(|_|adapters::invalid_response())?,
    }
}
async fn mark_dispatched(
    db: &mongodb::Database,
    metered: &billing::MeteredProxyContext,
    id: &str,
    private_route: Document,
) -> AppResult<()> {
    let route = metered
        .route
        .as_ref()
        .ok_or_else(adapters::invalid_response)?;
    let count = route
        .model_tariff
        .as_ref()
        .ok_or_else(adapters::invalid_response)?
        .tariff()
        .components
        .len() as u64;
    let tariff_id = &route.model_tariff.as_ref().unwrap().tariff().id;
    crate::services::transaction_service::run_db_only(
        db,
        &(db, id, count, tariff_id, private_route),
        |session, (db, id, count, tariff_id, private_route)| Box::pin(async move {
            let updated = db.collection::<Document>("usage_meter").update_many(
                doc! {
                    "billing_request_id": *id, "status": "reserved",
                    "released": false, "model_tariff.tariff_id": *tariff_id,
                },
                doc! {"$set": {
                    "status": "forwarded", "forwarded": true,
                    "updated_at": bson::DateTime::now(),
                }},
            ).session(&mut *session).await?;
            if updated.matched_count != *count {
                return Err(adapters::invalid_response());
            }
            db.collection::<Document>(ATTEMPTS).insert_one(doc! {
                "_id": *id, "state": "dispatched",
                "private_route": private_route,
                "lease_until": bson::DateTime::from_chrono(Utc::now()+chrono::Duration::seconds(135)),
                "created_at": bson::DateTime::now(),
            }).session(&mut *session).await?;
            Ok(())
        }),
    ).await
}

async fn complete_attempt(db: &mongodb::Database, id: &str, state: &str) -> AppResult<()> {
    db.collection::<Document>(ATTEMPTS)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"state":state,"updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}
async fn pending(db: &mongodb::Database, id: &str, reason: &str) -> AppResult<()> {
    db.collection::<Document>(ATTEMPTS)
        .update_one(
            doc! {"_id": id, "state": "dispatched"},
            doc! {"$set": {
                "state": "pending_review",
                "reason": reason,
                "updated_at": bson::DateTime::now(),
            }},
        )
        .await?;
    db.collection::<Document>("usage_meter")
        .update_many(
            doc! {"billing_request_id":id,"quantity":bson::Bson::Null,"released":false},
            doc! {"$set":{"last_error":"model_usage_pending"}},
        )
        .await?;
    Ok(())
}
fn public_usage(e: &billing::model_usage::Evidence, native: adapters::Native) -> AppResult<Value> {
    let i = e.input.ok_or_else(adapters::invalid_response)?;
    let o = e.output.ok_or_else(adapters::invalid_response)?;
    let i = if native == adapters::Native::Claude {
        i.checked_add(e.cache_read.unwrap_or(0))
            .and_then(|n| n.checked_add(e.cache_write.unwrap_or(0)))
            .ok_or_else(adapters::invalid_response)?
    } else {
        i
    };
    let o = if native == adapters::Native::Gemini {
        o.checked_add(e.reasoning.unwrap_or(0))
            .ok_or_else(adapters::invalid_response)?
    } else {
        o
    };
    Ok(
        json!({"prompt_tokens":i,"completion_tokens":o,"total_tokens":i.checked_add(o).ok_or_else(adapters::invalid_response)?}),
    )
}

/// Runs in the response-owning task. Cancellation drops HTTP immediately but
/// leaves known terminal evidence available for durable settlement.
pub(crate) async fn consume(
    ctx: &Context<'_>,
    plan: &Plan,
    accepted: Accepted,
    sender: Option<&tokio::sync::mpsc::Sender<Result<bytes::Bytes, std::io::Error>>>,
) -> AppResult<Value> {
    let Accepted {
        response,
        native,
        metered,
        deadline,
        _slot,
        _leases,
    } = accepted;
    let id = metered
        .route
        .as_ref()
        .ok_or_else(adapters::invalid_response)?
        .billing_request_id
        .clone();
    let maximum = plan.tariff.tariff().bounds.max_response_bytes as usize;
    let mut decoder = stream::Decoder::new(native);
    let mut buffer = Zeroizing::new(Vec::new());
    let mut size = 0usize;
    let mut clean = false;
    let mut upstream = response.bytes_stream();
    let read = async {
        while let Some(chunk) = upstream.next().await {
            let chunk = chunk.map_err(|_| adapters::invalid_response())?;
            size = size
                .checked_add(chunk.len())
                .filter(|n| *n <= maximum)
                .ok_or_else(adapters::invalid_response)?;
            if let Some(sender) = sender {
                for text in decoder.push(&chunk)? {
                    if !text.is_empty() {
                        sender
                            .send(Ok(adapters::chunk(
                                json!({"content":text}),
                                None,
                                &plan.model.public_id,
                                &ctx.request_id,
                            )))
                            .await
                            .map_err(|_| AppError::ClientDisconnected)?;
                    }
                }
            } else {
                buffer.extend_from_slice(&chunk);
            }
        }
        clean = true;
        Ok::<_, AppError>(())
    };
    let authority = async {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            ctx.authority.check(plan).await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), AppError>(())
    };
    let result = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => Err(AppError::ClientDisconnected),
        r = Box::pin(authority) => r,
        r = tokio::time::timeout_at(deadline, Box::pin(read)) => {
            r.map_err(|_| adapters::invalid_response()).and_then(|r| r)
        }
    };
    drop(upstream);
    let parsed = if sender.is_some() {
        decoder.finish(clean, &ctx.request_id)
    } else if result.is_ok() {
        (|| {
            let body: Value =
                serde_json::from_slice(&buffer).map_err(|_| adapters::invalid_response())?;
            let usage = if native == adapters::Native::Gemini {
                &body["usageMetadata"]
            } else {
                &body["usage"]
            };
            Ok((
                adapters::output(native, &body, &ctx.request_id),
                adapters::evidence(native, usage)?,
            ))
        })()
    } else {
        Err(adapters::invalid_response())
    };
    let (out, mut evidence) = match parsed {
        Ok(v) => v,
        Err(error) => {
            if let Some(partial) = decoder.partial_evidence() {
                ctx.db
                    .collection::<Document>(ATTEMPTS)
                    .update_one(
                        doc! {"_id": &id},
                        doc! {"$set": {
                            "partial_evidence": bson::to_bson(&partial)
                                .map_err(|_| adapters::invalid_response())?,
                        }},
                    )
                    .await?;
            }
            pending(ctx.db, &id, "missing_or_invalid_final_evidence").await?;
            return Err(error);
        }
    };
    if evidence.cache_write == Some(0) {
        for ttl in &plan.tariff.tariff().contract.cache_write_ttls {
            evidence.writes.entry(*ttl).or_insert(0);
        }
    }
    let usage = match plan.tariff.usage(&evidence) {
        Ok(v) => v,
        Err(error) => {
            pending(ctx.db, &id, "invalid_usage_contract").await?;
            return Err(error);
        }
    };
    ctx.billing
        .settle_deferred(&metered, usage, None, Some(plan.model.public_id.clone()))
        .await?;
    complete_attempt(ctx.db, &id, "evidence_saved").await?;
    let public = public_usage(&evidence, native)?;
    result?;
    let out =
        out.map_err(|_| AppError::BillingProviderUnavailable("model_response_unavailable".into()))?;
    if let Some(sender) = sender {
        if !out.calls.is_empty() {
            let calls: Vec<_> = out
                .calls
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let mut c = c.clone();
                    c["index"] = json!(i);
                    c
                })
                .collect();
            sender
                .send(Ok(adapters::chunk(
                    json!({"tool_calls":calls}),
                    None,
                    &plan.model.public_id,
                    &ctx.request_id,
                )))
                .await
                .map_err(|_| AppError::ClientDisconnected)?;
        }
        sender
            .send(Ok(adapters::chunk(
                json!({}),
                Some(out.finish),
                &plan.model.public_id,
                &ctx.request_id,
            )))
            .await
            .map_err(|_| AppError::ClientDisconnected)?;
        sender
            .send(Ok(bytes::Bytes::from(format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({
                    "id": format!("chatcmpl-{}", ctx.request_id),
                    "object": "chat.completion.chunk",
                    "model": plan.model.public_id,
                    "choices": [],
                    "usage": public,
                }),
            ))))
            .await
            .map_err(|_| AppError::ClientDisconnected)?;
        Ok(Value::Null)
    } else {
        Ok(adapters::public_json(
            out,
            &plan.model.public_id,
            &ctx.request_id,
            public,
        ))
    }
}

pub async fn ensure_indexes(db: &mongodb::Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(ATTEMPTS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"state":1,"lease_until":1})
                .build(),
        )
        .await?;
    Ok(())
}
/// No provider in this stateless cohort guarantees a billable usage lookup.
/// One CAS per expired attempt diagnoses the uncertainty; it NEVER releases or
/// fabricates consumption. Existing component-intent recovery finishes known work.
pub(crate) async fn recover(db: &mongodb::Database) -> AppResult<u64> {
    let mut n = 0;
    for _ in 0..100 {
        let token = uuid::Uuid::new_v4().to_string();
        let row = db
            .collection::<Document>(ATTEMPTS)
            .find_one_and_update(
                doc! {
                    "state": {"$in": ["dispatched", "recovering"]},
                    "lease_until": {"$lt": bson::DateTime::now()},
                },
                doc! {"$set": {
                    "state": "recovering",
                    "recovery_token": &token,
                    "lease_until": bson::DateTime::from_chrono(
                        Utc::now() + chrono::Duration::seconds(30)
                    ),
                    "updated_at": bson::DateTime::now(),
                }},
            )
            .await?;
        let Some(row) = row else { break };
        let id = row
            .get_str("_id")
            .map_err(|_| adapters::invalid_response())?;
        // A crash after saving evidence is already recoverable through PR 2.
        let known = db
            .collection::<Document>("usage_meter")
            .find_one(
                doc! {"billing_request_id":id,"model_tariff.evidence_digest":{"$type":"string"}},
            )
            .await?
            .is_some();
        if !known {
            db.collection::<Document>("usage_meter")
                .update_many(
                    doc! {"billing_request_id":id,"quantity":bson::Bson::Null,"released":false},
                    doc! {"$set":{"last_error":"model_usage_pending"}},
                )
                .await?;
        }
        let mut update = doc! {"state":if known {"evidence_saved"} else {"pending_review"},"updated_at":bson::DateTime::now()};
        if !known {
            update.insert("reason", "executor_lost");
        }
        let result = db
            .collection::<Document>(ATTEMPTS)
            .update_one(
                doc! {"_id":id,"state":"recovering","recovery_token":&token},
                doc! {"$set":update,"$unset":{"recovery_token":""}},
            )
            .await?;
        n += result.modified_count;
    }
    Ok(n)
}
pub(crate) async fn diagnostics(
    db: &mongodb::Database,
) -> AppResult<Vec<billing::BillingStartupDiagnostic>> {
    let rows: Vec<Document> = db
        .collection(ATTEMPTS)
        .find(doc! {"state":"pending_review"})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    Ok(vec![billing::BillingStartupDiagnostic {
        code: "platform_model_usage_pending".into(),
        summary: "Platform model consumption needs review".into(),
        detail: format!(
            "At least {} attempts retain prepaid holds without authoritative final evidence",
            rows.len(),
        ),
        remediation: "Recover authoritative evidence or make an explicitly reviewed write-off. Never release unknown consumption or charge reservation estimates.".into(),
        detected_at: Utc::now(),
    }])
}

#[cfg(test)]
pub(crate) mod tests;
