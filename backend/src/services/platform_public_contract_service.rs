//! Signed, purpose-specific runtime handshake. No personal credentials or routes.
use super::{
    api_key_mutation_service as tx, billing, platform_manager_contract::*,
    platform_public_policy as policy, platform_public_proxy_service as proxy, proxy_service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        platform_public_budget::{BUCKETS, PublicBudgetBucket},
        platform_public_contract::*,
        service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
        usage_meter::CredentialClass,
    },
};
use chrono::{Duration, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

pub const EXECUTION_CONTRACT: &str = "nyxid-public-execution.v1";
pub const MODEL_CONTRACT: &str = "nyxid-public-model.v1";
pub const CLEANUP_CONTRACT: &str = "nyxid-public-cleanup.v1";
pub const TOOL_CONTRACT: &str = "nyxid-public-tools.v1";
const AUDIENCE: &str = "nyxagent-public";
const MAX_BYTES: usize = 128 * 1024;
fn denied() -> AppError {
    policy::unavailable()
}
fn protocol_error() -> AppError {
    AppError::Internal("Public runtime returned an invalid contract response".into())
}
fn hash(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
fn sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn catalog_sha(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(sha)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlAuthority {
    binding: ManagerBinding,
    nyxagent_service_id: String,
    nyxagent_catalog_digest: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAuthority {
    binding: ManagerBinding,
    nyxagent_service_id: String,
    nyxagent_catalog_digest: String,
    probe_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionAuthority {
    binding: ManagerBinding,
    nyxagent_service_id: String,
    nyxagent_catalog_digest: String,
    manifest: RuntimeManifest,
    selected_profile: String,
    policy_digest: String,
    proof_record_id: String,
    credential_sha256: String,
    public_session_id: String,
    turn_id: String,
    receipt_id: String,
    claim_id: String,
    session_expires_at: chrono::DateTime<Utc>,
    turn_expires_at: chrono::DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupAuthority {
    session_expires_at: chrono::DateTime<Utc>,
    binding: ManagerBinding,
    nyxagent_service_id: String,
    nyxagent_catalog_digest: String,
    manifest: RuntimeManifest,
    public_session_id: String,
    policy_digest: String,
}
fn validate_authority(scope: &str, value: &Value) -> AppResult<()> {
    match scope {
        "discovery" | "readiness" => {
            serde_json::from_value::<ControlAuthority>(value.clone()).map_err(|_| denied())?;
        }
        "probe_model" => {
            serde_json::from_value::<ProbeAuthority>(value.clone()).map_err(|_| denied())?;
        }
        "execution" => {
            serde_json::from_value::<ExecutionAuthority>(value.clone()).map_err(|_| denied())?;
        }
        "cleanup" => {
            serde_json::from_value::<CleanupAuthority>(value.clone()).map_err(|_| denied())?;
        }
        _ => return Err(denied()),
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Assertion {
    iss: String,
    aud: String,
    exp: i64,
    iat: i64,
    jti: String,
    token_type: String,
    scope: String,
    authority: Value,
    request_digest: String,
}
pub(crate) fn assertion(
    state: &AppState,
    scope: &str,
    authority: Value,
    body: &[u8],
    expires: chrono::DateTime<Utc>,
) -> AppResult<Zeroizing<String>> {
    if !matches!(
        scope,
        "discovery" | "readiness" | "execution" | "probe_model" | "cleanup"
    ) {
        return Err(denied());
    }
    validate_authority(scope, &authority)?;
    let now = Utc::now();
    if expires <= now || expires > now + Duration::seconds(300) {
        return Err(denied());
    }
    let claims = Assertion {
        iss: state.config.jwt_issuer.clone(),
        aud: AUDIENCE.into(),
        exp: expires.timestamp(),
        iat: now.timestamp(),
        jti: uuid::Uuid::new_v4().to_string(),
        token_type: "nyxid_public_authority".into(),
        scope: scope.into(),
        authority,
        request_digest: hash(body),
    };
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.typ = Some("nyxid-public+jwt".into());
    header.kid = Some(state.jwt_keys.kid.clone());
    Ok(Zeroizing::new(
        jsonwebtoken::encode(&header, &claims, &state.jwt_keys.encoding).map_err(|_| denied())?,
    ))
}
fn verify_assertion(state: &AppState, raw: &str, scope: &str, body: &[u8]) -> AppResult<Value> {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[&state.config.jwt_issuer]);
    validation.leeway = 0;
    let data = jsonwebtoken::decode::<Assertion>(
        raw,
        state.jwt_keys.decoding_key(raw).map_err(|_| denied())?,
        &validation,
    )
    .map_err(|_| denied())?;
    if data.header.typ.as_deref() != Some("nyxid-public+jwt")
        || data.claims.scope != scope
        || data.claims.token_type != "nyxid_public_authority"
        || data.claims.request_digest != hash(body)
        || data.claims.iat > Utc::now().timestamp()
        || data.claims.exp - data.claims.iat > 300
    {
        return Err(denied());
    }
    validate_authority(scope, &data.claims.authority)?;
    Ok(data.claims.authority)
}

pub(crate) fn validate_manifest(m: &RuntimeManifest) -> AppResult<()> {
    if m.contract != EXECUTION_CONTRACT
        || m.model_contract != MODEL_CONTRACT
        || m.cleanup_contract != CLEANUP_CONTRACT
        || m.tool_contract != TOOL_CONTRACT
        || m.build.len() != 40
        || !m.build.bytes().all(|b| b.is_ascii_hexdigit())
        || !sha(&m.runtime_digest)
        || !m.credential_bound
        || !m.callback_only
        || !m.bounded_compaction
        || !m.absolute_expiry
        || !m.cleanup_after_revocation
        || !m.native_tools_disabled
        || !m.provider_tools_disabled
        || m.profiles.is_empty()
        || m.profiles.len() > 16
    {
        return Err(denied());
    }
    let mut ids = std::collections::HashSet::new();
    for p in &m.profiles {
        let l = &p.limits;
        if p.id.is_empty()
            || p.id.len() > 128
            || p.label.len() > 256
            || !ids.insert(&p.id)
            || !sha(&p.profile_digest)
            || p.model_operations.len() != 1
            || l.max_duration_secs == 0
            || l.max_duration_secs > 120
            || l.max_model_calls == 0
            || l.max_model_calls > 4
            || l.max_tool_calls > 8
            || l.max_continuations > 2
            || l.max_input_tokens == 0
            || l.max_input_tokens > 32768
            || l.max_output_tokens == 0
            || l.max_output_tokens > 2048
            || l.max_input_bytes == 0
            || l.max_input_bytes > 8192
            || l.max_context_bytes == 0
            || l.max_context_bytes > 131072
            || l.max_output_bytes == 0
            || l.max_output_bytes > 16384
        {
            return Err(denied());
        }
        for op in &p.model_operations {
            if uuid::Uuid::parse_str(&op.service_id).is_err()
                || op.method != "POST"
                || op.model_id.is_empty()
                || op.model_id.len() > 256
                || !catalog_sha(&op.execution_authority_digest)
                || !sha(&op.endpoint_set_digest)
                || op.endpoint_generation < 0
                || op.max_input_bytes == 0
                || op.max_input_bytes > 8192
                || op.max_input_tokens < op.max_input_bytes
                || op.max_input_tokens > 8192
                || op.max_output_tokens == 0
                || op.max_output_tokens > 512
                || op.reasoning_effort.as_deref().is_some_and(|effort| {
                    op.protocol != "openai_responses"
                        || !matches!(effort, "low" | "medium" | "high" | "xhigh")
                })
                || op.max_response_bytes == 0
                || op.max_response_bytes > 65536
                || u64::from(op.max_input_tokens) * u64::from(l.max_model_calls)
                    > u64::from(l.max_input_tokens)
                || u64::from(op.max_output_tokens) * u64::from(l.max_model_calls)
                    > u64::from(l.max_output_tokens)
            {
                return Err(denied());
            }
            policy::operation(op)?;
        }
    }
    if m.default_profile
        .as_ref()
        .is_some_and(|id| !ids.contains(id))
    {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn fence_deployment(
    db: &Database,
    session: &mut ClientSession,
    e: &DeploymentEvidence,
) -> AppResult<()> {
    let outer = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&e.nyxagent_service_id})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    proxy::require_outer(&outer)?;
    if proxy::catalog_digest(&outer)? != e.nyxagent_catalog_digest {
        return Err(denied());
    }
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&outer.id},
            doc! {"$inc":{"public_dispatch_fence":1_i64}},
        )
        .session(&mut *session)
        .await?;
    let p = e
        .manifest
        .profiles
        .iter()
        .find(|p| p.id == e.selected_profile)
        .ok_or_else(denied)?;
    for op in &p.model_operations {
        let endpoint_digest = &op.endpoint_set_digest;
        let op = policy::operation(op)?;
        let service = db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&op.service_id})
            .session(&mut *session)
            .await?
            .ok_or_else(denied)?;
        proxy::require_model_catalog_for_sponsor(&service, &e.binding.sponsor_owner_user_id, &op)?;
        let endpoints: Vec<ServiceEndpoint> = db
            .collection::<ServiceEndpoint>(ENDPOINTS)
            .find(doc! {"service_id":&service.id})
            .sort(doc! {"_id":1})
            .session(&mut *session)
            .await?
            .stream(&mut *session)
            .try_collect()
            .await?;
        if endpoint_set_digest(&endpoints)? != *endpoint_digest {
            return Err(denied());
        }
        let canonical =
            super::proxy_authorization::CanonicalPath::from_rest_decoded(&op.canonical_path)?;
        let selected = super::tool_publication_service::active_operation(
            &endpoints,
            op.method.as_str(),
            &canonical,
        );
        match selected {
            Some(name) if op.endpoint_generation.is_some() && name == op.operation => {
                if super::tool_publication_service::require_published_operation(
                    &endpoints,
                    op.method.as_str(),
                    &canonical,
                )?
                .as_deref()
                    != Some(name.as_str())
                {
                    return Err(denied());
                }
            }
            None if op.endpoint_generation.is_none()
                && endpoints.is_empty()
                && service.offering_kind
                    == crate::models::downstream_service::OfferingKind::AiService
                && op.operation == proxy::declared_operation(op.protocol) => {}
            _ => return Err(denied()),
        }

        db.collection::<Document>(SERVICES)
            .update_one(
                doc! {"_id":&service.id},
                doc! {"$inc":{"public_dispatch_fence":1_i64}},
            )
            .session(&mut *session)
            .await?;
        if let Some(generation) = op.endpoint_generation
            && db.collection::<ServiceEndpoint>(ENDPOINTS).update_one(doc! {"service_id":&op.service_id,"name":&op.operation,
                "method":op.method.as_str(),"path":format!("/{}",op.canonical_path),"operation_generation":generation,
                "is_active":true,"publication":"published","target_id":bson::Bson::Null},doc! {"$inc":{"public_dispatch_fence":1_i64}})
                .session(&mut *session).await?.matched_count!=1 {
            return Err(denied());
        }
    }
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&e.binding.sponsor_owner_user_id,"is_active":true,"user_type":"org"},
            doc! {"$inc":{"public_dispatch_fence":1_i64}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(denied());
    }
    Ok(())
}

struct SecretTarget(proxy_service::ProxyTarget);
impl Drop for SecretTarget {
    fn drop(&mut self) {
        self.0.credential.zeroize();
    }
}
enum ControlReply {
    Value(Value),
    Unavailable(UnavailableReason),
}
async fn control(
    state: &AppState,
    outer: &DownstreamService,
    scope: &str,
    path: &str,
    authority: Value,
    body: impl Serialize,
) -> AppResult<ControlReply> {
    proxy::require_outer(outer)?;
    let bytes = Zeroizing::new(serde_json::to_vec(&body).map_err(|_| denied())?);
    let token = assertion(
        state,
        scope,
        authority,
        &bytes,
        Utc::now() + Duration::seconds(90),
    )?;
    let mut target = SecretTarget(
        proxy_service::resolve_admin_proxy_target(&state.db, &state.encryption_keys, &outer.id)
            .await?,
    );
    if proxy::catalog_digest(&target.0.service)? != proxy::catalog_digest(outer)? {
        return Err(denied());
    }
    proxy::strip_identity(&mut target.0);
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::Proxy,
        )),
        billing::BillingIngress::Proxy,
    )?;
    let run = async {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "content-type",
            http::HeaderValue::from_static("application/json"),
        );
        let response = proxy_service::forward_request_with_extra_outbound_headers(
            proxy_service::no_redirect_http_client(),
            &target.0,
            http::Method::POST,
            path,
            None,
            headers,
            proxy_service::ProxyBody::Buffered(Some(bytes::Bytes::copy_from_slice(&bytes))),
            vec![],
            vec![],
            None,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            vec![("x-nyxid-public-authority".into(), token.to_string())],
            permit,
        )
        .await
        .map_err(|e| e.into_app_error())?;
        if matches!(response.status().as_u16(), 404 | 501) {
            return Ok(ControlReply::Unavailable(
                UnavailableReason::PublicContractUnsupported,
            ));
        }
        if matches!(response.status().as_u16(), 401 | 403) {
            return Ok(ControlReply::Unavailable(
                UnavailableReason::PublicAuthorityRejected,
            ));
        }
        if let Some(reason) = match response.status().as_u16() {
            402 => Some(UnavailableReason::InsufficientFunds),
            429 => Some(UnavailableReason::CapacityLimited),
            503 => Some(UnavailableReason::EngineUnavailable),
            _ => None,
        } {
            return Ok(ControlReply::Unavailable(reason));
        }
        if !response.status().is_success() {
            return Err(denied());
        }
        let mut stream = response.bytes_stream();
        let mut output = Zeroizing::new(Vec::new());
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| protocol_error())?;
            if output.len() + chunk.len() > MAX_BYTES {
                return Err(protocol_error());
            }
            output.extend_from_slice(&chunk);
        }
        Ok(ControlReply::Value(
            serde_json::from_slice(&output).map_err(|_| protocol_error())?,
        ))
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(if scope == "cleanup" { 20 } else { 90 }),
        run,
    )
    .await
    .map_err(|_| denied())?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Discovery {
    nonce: String,
    binding: ManagerBinding,
    manifest: RuntimeManifest,
}
pub(crate) async fn discover(
    state: &AppState,
    reviewed: &ManagerRuntimePolicy,
) -> AppResult<(
    PublicProfileDiscovery,
    Option<(DownstreamService, RuntimeManifest)>,
)> {
    let outer = proxy::load_outer_catalog(&state.db).await?;
    let binding = policy::binding(reviewed)?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let catalog_bindings = model_bindings(&state.db, &binding.sponsor_owner_user_id).await?;
    let authority = json!({"binding":binding,"nyxagent_service_id":outer.id,"nyxagent_catalog_digest":proxy::catalog_digest(&outer)?});
    let response = control(
        state,
        &outer,
        "discovery",
        "v1/public/profiles",
        authority,
        json!({"contract":EXECUTION_CONTRACT,"nonce":nonce,"binding":binding,"catalog_bindings":catalog_bindings}),
    )
    .await?;
    let value = match response {
        ControlReply::Value(v) => v,
        ControlReply::Unavailable(reason) => {
            return Ok((
                PublicProfileDiscovery {
                    profiles: vec![],
                    default_profile: None,
                    readiness: PublicReadiness::Unavailable(reason),
                },
                None,
            ));
        }
    };
    let reply: Discovery = serde_json::from_value(value).map_err(|_| protocol_error())?;
    if reply.nonce != nonce || reply.binding != binding {
        return Err(protocol_error());
    }
    validate_manifest(&reply.manifest)?;
    let selected = &reviewed
        .public_policy
        .as_ref()
        .ok_or_else(denied)?
        .nyxagent_profile;
    let readiness = if selected.is_empty() {
        PublicReadiness::Unavailable(UnavailableReason::PublicNotReady)
    } else if reply.manifest.profiles.iter().any(|p| p.id == *selected) {
        policy::assess(&state.db, reviewed).await?
    } else {
        PublicReadiness::Unavailable(UnavailableReason::ProfileUnavailable)
    };
    Ok((
        PublicProfileDiscovery {
            profiles: reply
                .manifest
                .profiles
                .iter()
                .map(|p| super::platform_manager_contract::PublicProfile {
                    id: p.id.clone(),
                    label: p.label.clone(),
                })
                .collect(),
            default_profile: reply.manifest.default_profile.clone(),
            readiness,
        },
        Some((outer, reply.manifest)),
    ))
}

fn probe_body(op: &ModelOperation) -> Value {
    match op.protocol.as_str() {
        "openai_responses" => {
            let mut value = json!({"model":op.model_id,"input":"Reply OK.","max_output_tokens":op.max_output_tokens});
            if let Some(effort) = &op.reasoning_effort {
                value["reasoning"] = json!({"effort":effort,"summary":"none"});
            }
            value
        }
        "openai_completions" => {
            json!({"model":op.model_id,"messages":[{"role":"user","content":"Reply OK."}],"max_completion_tokens":op.max_output_tokens})
        }
        _ => {
            json!({"model":op.model_id,"messages":[{"role":"user","content":"Reply OK."}],"max_tokens":op.max_output_tokens})
        }
    }
}
async fn route(
    state: &AppState,
    sponsor: &str,
    id: &str,
    op: &ModelOperation,
) -> AppResult<(
    DownstreamService,
    billing::BillingRouteContext,
    Credits,
    String,
)> {
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&op.service_id})
        .await?
        .ok_or_else(denied)?;
    proxy::require_model_catalog_for_sponsor(&service, sponsor, &policy::operation(op)?)?;
    let metric = billing::metric_resolution::platform_metric_for_request(&service, false);
    let mut route = billing::BillingRouteContext::new(
        billing::BillingIngress::Proxy,
        id.into(),
        sponsor.into(),
        sponsor.into(),
        None,
        None,
        Some(service.id.clone()),
        Some(service.slug.clone()),
        billing::NodeIntent::Direct,
        service.auth_method.clone(),
        CredentialClass::NyxidManagedMaster,
        metric,
        service.billing.as_ref(),
        state.billing.resale_enabled(),
    );
    let priced = state
        .billing
        .http_operation(&route, "POST", &op.canonical_path)
        .await?;
    route = route.with_operation(priced.as_deref());
    route.require_platform_price()?;
    route = route.with_platform_metering(true).with_native_text_bound(
        i64::from(op.max_input_tokens),
        i64::from(op.max_output_tokens),
        i64::from(op.max_response_bytes),
    );
    route.request_bytes = i64::from(op.max_input_bytes);
    route.public_admission = Some(billing::route_context::PublicBillingAdmission {
        call_id: id.into(),
        model_id: op.model_id.clone(),
        expires_at: Utc::now() + Duration::seconds(90),
    });
    let layers = billing::reservation::estimate_layer_reservations(
        &state.db,
        &route,
        state.config.billing_rate_cache_ttl_secs,
    )
    .await?;
    let maximum = layers
        .iter()
        .map(|l| l.reserved_credits)
        .try_fold(Credits::ZERO, Credits::checked_add)?;
    Ok((service, route, maximum, pricing_digest(&layers)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadinessReply {
    contract: String,
    nonce: String,
    binding: ManagerBinding,
    manifest: RuntimeManifest,
    selected_profile: String,
    probe_id: String,
}
#[derive(Serialize)]
struct ReadinessRequest<'a> {
    contract: &'a str,
    nonce: &'a str,
    binding: &'a ManagerBinding,
    manifest: &'a RuntimeManifest,
    selected_profile: &'a str,
    probe_id: &'a str,
    probe_callback: String,
    probe_authorization: &'a str,
    probe_body_utf8: &'a str,
}

fn probe_deadline(now: chrono::DateTime<Utc>) -> AppResult<chrono::DateTime<Utc>> {
    let next_day = now
        .date_naive()
        .succ_opt()
        .ok_or_else(denied)?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(denied)?
        .and_utc();
    Ok((now + Duration::seconds(90)).min(next_day))
}

pub(crate) async fn probe(
    state: &AppState,
    reviewed: &ManagerRuntimePolicy,
    expected_policy: Option<&str>,
) -> AppResult<PublicReadiness> {
    probe_before(state, reviewed, expected_policy, None).await
}

async fn probe_before(
    state: &AppState,
    reviewed: &ManagerRuntimePolicy,
    expected_policy: Option<&str>,
    refresh_deadline: Option<chrono::DateTime<Utc>>,
) -> AppResult<PublicReadiness> {
    let (discovery, deployed) = discover(state, reviewed).await?;
    let Some((outer, manifest)) = deployed else {
        return Ok(discovery.readiness);
    };
    let selection = reviewed.public_policy.as_ref().ok_or_else(denied)?;
    let Some(profile) = manifest
        .profiles
        .iter()
        .find(|p| p.id == selection.nyxagent_profile)
    else {
        return Ok(PublicReadiness::Unavailable(
            UnavailableReason::ProfileUnavailable,
        ));
    };
    if let Some(expected) = expected_policy {
        let candidate = DeploymentEvidence {
            binding: policy::binding(reviewed)?,
            manifest: manifest.clone(),
            selected_profile: profile.id.clone(),
            nyxagent_service_id: outer.id.clone(),
            nyxagent_catalog_digest: proxy::catalog_digest(&outer)?,
            policy_digest: String::new(),
            evidence_digest: String::new(),
            probe_id: String::new(),
            expires_at: Utc::now(),
        };
        if policy::policy_digest(&candidate)? != expected {
            return Err(denied());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let binding = policy::binding(reviewed)?;
    let (_, _, whole, _) = route(
        state,
        &binding.sponsor_owner_user_id,
        &id,
        &profile.model_operations[0],
    )
    .await?;
    let whole = whole.checked_mul(i64::from(profile.limits.max_model_calls))?;
    if whole > selection.sender_daily_credits || whole > selection.total_daily_credits {
        return Ok(PublicReadiness::Unavailable(
            UnavailableReason::PublicQuotaLimited,
        ));
    }
    let mut op = profile.model_operations[0].clone();
    let body = serde_json::to_vec(&probe_body(&op)).map_err(|_| denied())?;
    if body.len() > op.max_input_bytes as usize || body.len() > op.max_input_tokens as usize {
        return Err(denied());
    }
    // A byte upper bound covers this exact fixed text request, including JSON.
    op.max_input_bytes = body.len() as u32;
    op.max_input_tokens = body.len() as u32;
    let (_, mut probe_route, maximum, price_digest) =
        route(state, &binding.sponsor_owner_user_id, &id, &op).await?;
    probe_route.public_probe = true;
    probe_route.public_admission = Some(billing::route_context::PublicBillingAdmission {
        call_id: id.clone(),
        model_id: op.model_id.clone(),
        expires_at: Utc::now() + Duration::seconds(90),
    });
    let now = state
        .db
        .run_command(doc! {"hello":1})
        .await?
        .get_datetime("localTime")
        .map_err(|_| denied())?
        .to_chrono();
    let deadline = probe_deadline(now)?;
    let deadline = refresh_deadline.map_or(deadline, |refresh| deadline.min(refresh));
    if deadline <= now {
        return Err(denied());
    }
    probe_route
        .public_admission
        .as_mut()
        .ok_or_else(denied)?
        .expires_at = deadline;
    let day = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or_else(denied)?
        .and_utc();
    let scope = format!("total:{}:{}", binding.profile_id, day.timestamp());
    match state
        .db
        .collection::<PublicBudgetBucket>(BUCKETS)
        .update_one(
            doc! {"scope_key": &scope},
            doc! {"$setOnInsert": {
                "_id": uuid::Uuid::new_v4().to_string(), "scope_key": &scope,
                "held": Credits::ZERO.decimal(), "spent": Credits::ZERO.decimal(),
            }},
        )
        .upsert(true)
        .await
    {
        Ok(_) => {}
        Err(error) if super::assistant_team_service::is_duplicate(&error) => {}
        Err(error) => return Err(error.into()),
    }
    let bucket = state
        .db
        .collection::<PublicBudgetBucket>(BUCKETS)
        .find_one(doc! {"scope_key":&scope})
        .await?
        .ok_or_else(denied)?;
    let record = PublicProbe {
        activation_digest: expected_policy.map(str::to_owned),
        id: id.clone(),
        binding: binding.clone(),
        selected_profile: profile.id.clone(),
        manifest: manifest.clone(),
        nyxagent_service_id: outer.id.clone(),
        nyxagent_catalog_digest: proxy::catalog_digest(&outer)?,
        operation: op,
        request_digest: hash(&body),
        route_digest: super::platform_public_budget_service::route_digest(&probe_route)?,
        pricing_digest: price_digest,
        meter_row_ids: vec![],
        bucket_id: bucket.id,
        state: "prepared".into(),
        maximum,
        exact_model_observed: false,
        expires_at: deadline,
    };
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (state.clone(), record.clone(), selection.clone()),
            |session, (state, record, selection)| {
                Box::pin(async move {
                    let result: AppResult<()> = async {
                        fence_probe_authority(
                            &state.db,
                            session,
                            &record.binding,
                            record.activation_digest.as_deref(),
                        )
                        .await?;
                        fence_deployment(&state.db, session, &probe_evidence(record)).await?;
                        if !super::coordination_service::RateWindowStore::admit_in_session(
                            &state.db,
                            "public_readiness_probe",
                            &record.binding.profile_id,
                            1,
                            std::time::Duration::from_secs(30),
                            session,
                        )
                        .await?
                        {
                            return Err(AppError::RateLimited);
                        }
                        let held = state
                            .db
                            .collection::<PublicBudgetBucket>(BUCKETS)
                            .update_one(
                                doc! {
                                "_id": &record.bucket_id,
                                "$expr": {"$and": [
                                    {"$gt": [bson::DateTime::from_chrono(record.expires_at), "$$NOW"]},
                                    {"$lte": [{"$add": ["$held", "$spent", record.maximum.decimal()]}, selection.total_daily_credits.decimal()]},
                                ]},
                                },
                                doc! {"$inc": {"held": record.maximum.decimal()}},
                            )
                            .session(&mut *session)
                            .await?;
                        if held.matched_count != 1 {
                            return Err(AppError::RateLimited);
                        }
                        state
                            .db
                            .collection::<PublicProbe>(PROBES)
                            .insert_one(&*record)
                            .session(session)
                            .await?;
                        Ok(())
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let token = assertion(
        state,
        "probe_model",
        json!({"probe_id":id,"binding":binding,"nyxagent_service_id":outer.id,"nyxagent_catalog_digest":record.nyxagent_catalog_digest}),
        &body,
        record.expires_at,
    )?;
    let authority = json!({"binding":binding,"nyxagent_service_id":outer.id,"nyxagent_catalog_digest":record.nyxagent_catalog_digest});
    // Borrow the bearer directly into serialization; no token-bearing JSON Value
    // or intermediate String survives outside the zeroizing request buffer.
    let request = ReadinessRequest {
        contract: EXECUTION_CONTRACT,
        nonce: &nonce,
        binding: &binding,
        manifest: &manifest,
        selected_profile: &profile.id,
        probe_id: &id,
        probe_callback: format!(
            "/api/v1/public-nyxbot/models/{}/{}",
            record.operation.service_id, record.operation.canonical_path
        ),
        probe_authorization: &token,
        probe_body_utf8: std::str::from_utf8(&body).map_err(|_| denied())?,
    };
    let response = control(
        state,
        &outer,
        "readiness",
        "v1/public/readiness",
        authority,
        request,
    )
    .await?;
    let response = match response {
        ControlReply::Value(v) => v,
        ControlReply::Unavailable(reason) => return Ok(PublicReadiness::Unavailable(reason)),
    };
    let reply: ReadinessReply = serde_json::from_value(response).map_err(|_| protocol_error())?;
    if reply.contract != EXECUTION_CONTRACT
        || reply.nonce != nonce
        || reply.binding != binding
        || reply.manifest != manifest
        || reply.selected_profile != profile.id
        || reply.probe_id != id
    {
        return Err(protocol_error());
    }
    let completed = state
        .db
        .collection::<PublicProbe>(PROBES)
        .find_one(doc! {"_id":&id,"state":"settled","exact_model_observed":true})
        .await?
        .ok_or_else(denied)?;
    // Persist and compare exactly the same timestamp precision as BSON. The
    // immediate scoped fence must see the same proof returned by validation.
    let checked = bson::DateTime::now().to_chrono();
    if completed.expires_at <= checked {
        return Err(denied());
    }
    let mut e = DeploymentEvidence {
        binding,
        manifest,
        selected_profile: completed.selected_profile,
        nyxagent_service_id: outer.id,
        nyxagent_catalog_digest: completed.nyxagent_catalog_digest,
        policy_digest: String::new(),
        evidence_digest: String::new(),
        probe_id: id,
        expires_at: checked + Duration::minutes(5),
    };
    e.policy_digest = policy::policy_digest(&e)?;
    e.evidence_digest = policy::evidence_digest(&e)?;
    let record = PublicDeploymentRecord {
        id: uuid::Uuid::new_v4().to_string(),
        profile_id: e.binding.profile_id.clone(),
        public_policy_revision: e.binding.public_policy_revision,
        candidate_digest: e.binding.candidate_digest.clone(),
        status: "ready".into(),
        proof_gaps: vec![],
        evidence: Some(e),
        effect_revision: 0,
        checked_at: checked,
    };
    session
        .start_transaction()
        .and_run(
            (
                state.clone(),
                record.clone(),
                expected_policy.map(str::to_owned),
            ),
            |session, (state, record, activation_digest)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        fence_probe_authority(
                            &state.db,
                            session,
                            &record.evidence.as_ref().ok_or_else(denied)?.binding,
                            activation_digest.as_deref(),
                        )
                        .await?;
                        fence_deployment(
                            &state.db,
                            session,
                            record.evidence.as_ref().ok_or_else(denied)?,
                        )
                        .await?;
                        state
                            .db
                            .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                            .insert_one(&*record)
                            .session(session)
                            .await?;
                        Ok(())
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    policy::assess_record(&state.db, reviewed, &record).await
}

pub(crate) async fn refresh_for_ingress(
    state: &AppState,
    ingress: &PublicIngress,
) -> AppResult<()> {
    let controls =
        super::platform_bot_runtime_control_service::get(&state.db, &ingress.manager.profile_id)
            .await?
            .ok_or_else(denied)?;
    let reviewed = super::platform_bot_runtime_control_service::policy(&controls);
    if reviewed.manager != ingress.manager
        || reviewed.public_policy_revision != ingress.public_policy_revision
        || !reviewed.public_enabled
        || reviewed.emergency_stopped
        || controls.retired
    {
        return Err(denied());
    }
    if has_turn_lifetime(&state.db, &reviewed).await? {
        return Ok(());
    }
    let activation = policy::activation(&state.db, &reviewed).await?;
    // One bounded refresh per activated contract across replicas. The lease
    // coordinates work only; all Ready authority still comes from persisted,
    // transactionally fenced evidence and the live activation.
    use super::coordination_service::LeaseStore;
    use crate::models::coordination::{
        CoordinationHolder, CoordinationLease, LEASE_COLLECTION_NAME,
    };
    let receipt = state
        .db
        .collection::<Document>(crate::models::platform_manager_ingress::COLLECTION_NAME)
        .find_one(doc! {
            "_id": &ingress.ingress.receipt_id, "claim_id": &ingress.ingress.claim_id,
            "profile_id": &ingress.manager.profile_id, "phase": "admitted",
            "$expr": {"$gt": ["$start_before", "$$NOW"]},
        })
        .await?
        .ok_or_else(denied)?;
    let now = state
        .db
        .run_command(doc! {"hello": 1})
        .await?
        .get_datetime("localTime")
        .map_err(|_| denied())?
        .to_chrono();
    let remaining = (receipt
        .get_datetime("start_before")
        .map_err(|_| denied())?
        .to_chrono()
        - now)
        .to_std()
        .map_err(|_| denied())?
        .min(std::time::Duration::from_secs(95));
    let deadline = tokio::time::Instant::now() + remaining;
    let refresh_expires_at = now + Duration::from_std(remaining).map_err(|_| denied())?;
    let name = format!(
        "public-readiness-refresh:{}",
        policy::digest(&(&activation.binding, &activation.policy_digest,))?
    );
    let holder = CoordinationHolder {
        instance_id: uuid::Uuid::new_v4().to_string(),
        generation_id: uuid::Uuid::new_v4().to_string(),
    };
    let token = tokio::time::timeout_at(
        deadline,
        LeaseStore::acquire(
            &state.db,
            &name,
            &holder,
            std::time::Duration::from_secs(100),
        ),
    )
    .await
    .map_err(|_| denied())??;
    if let Some(token) = token {
        let result = tokio::time::timeout_at(deadline, async {
            // A preceding holder may have published between our initial read
            // and lease acquisition. Reuse it before reserving another probe.
            if has_turn_lifetime(&state.db, &reviewed).await? {
                return Ok(());
            }
            match probe_before(
                state,
                &reviewed,
                Some(&activation.policy_digest),
                Some(refresh_expires_at),
            )
            .await?
            {
                PublicReadiness::Ready(_) => Ok(()),
                PublicReadiness::Unavailable(_) => Err(denied()),
            }
        })
        .await
        .map_err(|_| denied())
        .and_then(|result| result);
        if result.is_ok() {
            tokio::time::timeout_at(deadline, LeaseStore::release(&state.db, &token))
                .await
                .map_err(|_| denied())??;
        }
        // A failed/cancelled holder retains its bounded lease. Its signed probe
        // deadline is earlier than lease expiry, so delayed callbacks cannot
        // overlap a replacement refresh's paid effect.
        return result;
    }
    let wait = async {
        let observed = state
            .db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .find_one(doc! {"_id": &name})
            .await?;
        let mut last_assessed = None;
        loop {
            if refreshed_proof_has_lifetime(&state.db, &reviewed, &activation, &mut last_assessed)
                .await?
            {
                return Ok(());
            }
            let active = match &observed {
                Some(lease) => state
                    .db
                    .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
                    .find_one(doc! {"_id": &name, "lease_id": &lease.lease_id,
                    "$expr": {"$gt": ["$expires_at", "$$NOW"]}})
                    .await?
                    .is_some(),
                None => false,
            };
            if !active {
                // Publication commits before release. A failed/expired holder
                // cannot be reported Ready or trigger another paid attempt.
                return if refreshed_proof_has_lifetime(
                    &state.db,
                    &reviewed,
                    &activation,
                    &mut last_assessed,
                )
                .await?
                {
                    Ok(())
                } else {
                    Err(denied())
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    };
    tokio::time::timeout_at(deadline, wait)
        .await
        .map_err(|_| denied())?
}

/// Polling is read-only. A plausible new proof gets one authoritative assessment;
/// its identity and expiry alone never authorize a turn or a funding effect.
async fn refreshed_proof_has_lifetime(
    db: &Database,
    reviewed: &ManagerRuntimePolicy,
    activation: &PublicDeploymentActivation,
    last_assessed: &mut Option<String>,
) -> AppResult<bool> {
    let Some(record) = db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one(doc! {
            "candidate_digest": &activation.binding.candidate_digest,
            "evidence.policy_digest": &activation.policy_digest,
            "status": "ready",
            "$expr": {"$gt": ["$evidence.expires_at", "$$NOW"]},
        })
        .sort(doc! {"checked_at": -1, "_id": -1})
        .await?
    else {
        return Ok(false);
    };
    if last_assessed.as_ref() == Some(&record.id) {
        return Ok(false);
    }
    let Some(evidence) = &record.evidence else {
        return Ok(false);
    };
    let Some(profile) = evidence
        .manifest
        .profiles
        .iter()
        .find(|profile| profile.id == evidence.selected_profile)
    else {
        return Ok(false);
    };
    let required = Duration::seconds(i64::from(profile.limits.max_duration_secs) + 5);
    if evidence.expires_at <= Utc::now() + required {
        return Ok(false);
    }
    *last_assessed = Some(record.id);
    has_turn_lifetime(db, reviewed).await
}

async fn has_turn_lifetime(db: &Database, reviewed: &ManagerRuntimePolicy) -> AppResult<bool> {
    let PublicReadiness::Ready(evidence) = policy::assess_enabled(db, reviewed).await? else {
        return Ok(false);
    };
    let required = Duration::seconds(i64::from(evidence.limits.max_duration_secs) + 5);
    Ok(evidence.expires_at > Utc::now() + required)
}

pub(crate) async fn execution_assertion(
    state: &AppState,
    context: &PublicExecutionContext,
    key: &str,
    body: &[u8],
) -> AppResult<Zeroizing<String>> {
    let record = state
        .db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one(doc! {"_id":&context.proof_record_id})
        .await?
        .ok_or_else(denied)?;
    let e = record.evidence.ok_or_else(denied)?;
    let row = state
        .db
        .collection::<crate::models::platform_public_session::PlatformPublicSession>(
            crate::models::platform_public_session::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&context.public_session_id,"policy_digest":&context.policy_digest})
        .await?
        .ok_or_else(denied)?;
    assertion(
        state,
        "execution",
        json!({"binding":e.binding,"policy_digest":context.policy_digest,"proof_record_id":context.proof_record_id,
        "nyxagent_service_id":e.nyxagent_service_id,"nyxagent_catalog_digest":e.nyxagent_catalog_digest,
        "manifest":e.manifest,"selected_profile":context.nyxagent_profile,"credential_sha256":hash(key.as_bytes()),
        "public_session_id":context.public_session_id,"turn_id":context.turn.turn_id,
        "receipt_id":context.ingress.ingress.receipt_id,"claim_id":context.ingress.ingress.claim_id,
        "session_expires_at":row.expires_at.to_rfc3339(),"turn_expires_at":context.expires_at.to_rfc3339()}),
        body,
        context.expires_at,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupReply {
    contract: String,
    public_session_id: String,
    policy_digest: String,
    erased: bool,
}
pub(crate) async fn cleanup(
    state: &AppState,
    row: &crate::models::platform_public_session::PlatformPublicSession,
) -> AppResult<bool> {
    let record = state
        .db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one(doc! {"evidence.policy_digest":&row.policy_digest,"profile_id":&row.profile_id})
        .await?
        .ok_or_else(denied)?;
    let e = record.evidence.ok_or_else(denied)?;
    validate_manifest(&e.manifest)?;
    if policy::policy_digest(&e)? != row.policy_digest
        || e.binding.sponsor_owner_user_id != row.sponsor_owner_user_id
    {
        return Err(denied());
    }
    let outer = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&e.nyxagent_service_id})
        .await?
        .ok_or_else(denied)?;
    if proxy::catalog_digest(&outer)? != e.nyxagent_catalog_digest {
        return Err(denied());
    }
    let authority = json!({
        "binding": e.binding, "nyxagent_service_id": outer.id,
        "nyxagent_catalog_digest": e.nyxagent_catalog_digest,
        "public_session_id": row.id, "policy_digest": row.policy_digest,
        "manifest": e.manifest, "session_expires_at": row.expires_at.to_rfc3339(),
    });
    let reply = control(
        state,
        &outer,
        "cleanup",
        "v1/public/cleanup",
        authority,
        json!({"contract": CLEANUP_CONTRACT, "public_session_id": row.id, "policy_digest": row.policy_digest}),
    )
    .await?;
    let ControlReply::Value(reply) = reply else {
        return Ok(false);
    };
    let reply: CleanupReply = serde_json::from_value(reply).map_err(|_| protocol_error())?;
    Ok(reply.contract == CLEANUP_CONTRACT
        && reply.public_session_id == row.id
        && reply.policy_digest == row.policy_digest
        && reply.erased)
}

/// Dedicated signed probe callback: no invented ApiKey, sender, receipt or
/// ordinary AuthUser. The one immutable request and sponsor are stored locally.
pub(crate) async fn dispatch_probe(
    state: &AppState,
    raw: &str,
    service_id: &str,
    path: &str,
    body: &[u8],
) -> AppResult<axum::response::Response> {
    let authority = verify_assertion(state, raw, "probe_model", body)
        .map_err(|_| AppError::Unauthorized("Invalid public probe authority".into()))?;
    let authority: ProbeAuthority = serde_json::from_value(authority).map_err(|_| denied())?;
    let id = authority.probe_id.as_str();
    let record = state
        .db
        .collection::<PublicProbe>(PROBES)
        .find_one(doc! {"_id":id,"state":{"$in":["prepared","funded"]},"expires_at":{"$gt":bson::DateTime::now()}})
        .await?
        .ok_or_else(denied)?;
    if authority.binding != record.binding
        || authority.nyxagent_service_id != record.nyxagent_service_id
        || authority.nyxagent_catalog_digest != record.nyxagent_catalog_digest
        || record.operation.service_id != service_id
        || record.operation.canonical_path != path
        || record.request_digest != hash(body)
        || serde_json::from_slice::<Value>(body).map_err(|_| denied())?
            != probe_body(&record.operation)
    {
        return Err(denied());
    }
    let (service, mut route, maximum, price_digest) = route(
        state,
        &record.binding.sponsor_owner_user_id,
        id,
        &record.operation,
    )
    .await?;
    route.public_probe = true;
    route.public_admission = Some(billing::route_context::PublicBillingAdmission {
        call_id: id.into(),
        model_id: record.operation.model_id.clone(),
        expires_at: record.expires_at,
    });
    if maximum != record.maximum
        || price_digest != record.pricing_digest
        || super::platform_public_budget_service::route_digest(&route)? != record.route_digest
    {
        return Err(denied());
    }
    let mut target = SecretTarget(
        proxy_service::resolve_catalog_platform_target(
            &state.db,
            &state.encryption_keys,
            &record.binding.sponsor_owner_user_id,
            service,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(2, 4),
        )
        .await?,
    );
    proxy::strip_identity(&mut target.0);
    let metered = state.billing.open_required(&route).await?;
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (state.clone(), record.clone()),
            |session, (state, record)| {
                Box::pin(async move {
                    let result: AppResult<()> = async {
                        fence_probe_authority(
                            &state.db,
                            session,
                            &record.binding,
                            record.activation_digest.as_deref(),
                        )
                        .await?;
                        fence_deployment(&state.db, session, &probe_evidence(record)).await?;
                        let funded = state
                            .db
                            .collection::<PublicProbe>(PROBES)
                            .find_one(doc! {"_id": &record.id, "state": "funded"})
                            .session(&mut *session)
                            .await?
                            .ok_or_else(denied)?;
                        if funded.meter_row_ids.is_empty() {
                            return Err(denied());
                        }
                        for meter in &funded.meter_row_ids {
                            let forwarded = state
                                .db
                                .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                                .update_one(
                                    doc! {
                                        "_id": meter, "billing_request_id": &record.id,
                                        "billing_owner_id": &record.binding.sponsor_owner_user_id,
                                        "api_key_id": bson::Bson::Null, "status": "reserved",
                                        "released": false, "forwarded": false,
                                    },
                                    doc! {"$set": {"forwarded": true, "status": "forwarded"}},
                                )
                                .session(&mut *session)
                                .await?;
                            if forwarded.matched_count != 1 {
                                return Err(denied());
                            }
                        }
                        let admitted = state
                            .db
                            .collection::<PublicProbe>(PROBES)
                            .update_one(
                                doc! {"_id": &record.id, "state": "funded",
                                "$expr": {"$gt": ["$expires_at", "$$NOW"]}},
                                doc! {"$set": {"state": "dispatch_admitted"}},
                            )
                            .session(&mut *session)
                            .await?;
                        if admitted.matched_count != 1 {
                            return Err(denied());
                        }
                        let spent = state
                            .db
                            .collection::<PublicBudgetBucket>(BUCKETS)
                            .update_one(
                                doc! {"_id": &record.bucket_id},
                                doc! {"$inc": {
                                    "held": record.maximum.checked_neg()?.decimal(),
                                    "spent": record.maximum.decimal(),
                                }},
                            )
                            .session(session)
                            .await?;
                        if spent.matched_count != 1 {
                            return Err(denied());
                        }
                        Ok(())
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::Proxy,
        )),
        billing::BillingIngress::Proxy,
    )?;
    let run = async {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "content-type",
            http::HeaderValue::from_static("application/json"),
        );
        let response = proxy_service::forward_request_with_extra_outbound_headers(
            proxy_service::no_redirect_http_client(),
            &target.0,
            http::Method::POST,
            path,
            None,
            headers,
            proxy_service::ProxyBody::Buffered(Some(bytes::Bytes::copy_from_slice(body))),
            vec![],
            vec![],
            None,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            vec![],
            permit,
        )
        .await
        .map_err(|e| e.into_app_error())?;
        let status = response.status();
        let mut stream = response.bytes_stream();
        let mut output = Zeroizing::new(Vec::new());
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| denied())?;
            if output.len() + chunk.len() > record.operation.max_response_bytes as usize {
                return Err(denied());
            }
            output.extend_from_slice(&chunk);
        }
        let reported =
            super::llm_usage_service::usage_from_body(&output, path, status.is_success())
                .ok_or_else(denied)?;
        let bytes = (body.len() + output.len()) as i64;
        let resale = route.resale.as_ref().and_then(|r| {
            super::proxy_http_response::resale_usage_from_optional_reported(
                r.metric,
                Some(&reported),
                bytes,
            )
        });
        state
            .billing
            .settle(
                &metered,
                super::llm_usage_service::platform_usage(Some(&reported), bytes, false),
                resale,
                Some(record.operation.model_id.clone()),
            )
            .await?;
        proxy::validate_reported_usage(&policy::operation(&record.operation)?, &reported)?;
        let value: Value = serde_json::from_slice(&output).map_err(|_| denied())?;
        let exact = status.is_success()
            && value["model"].as_str() == Some(record.operation.model_id.as_str())
            && probe_has_text(&value);
        complete_probe(&state.db, id, Some(exact)).await?;
        axum::response::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(output.to_vec()))
            .map_err(|_| denied())
    };
    let remaining = (record.expires_at - Utc::now())
        .to_std()
        .map_err(|_| denied())?;
    tokio::time::timeout(remaining, run)
        .await
        .map_err(|_| denied())?
}
fn probe_has_text(value: &Value) -> bool {
    value["output"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["content"].as_array().is_some_and(|parts| {
                parts
                    .iter()
                    .any(|p| p["text"].as_str().is_some_and(|s| !s.trim().is_empty()))
            })
        })
    }) || value["choices"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["message"]["content"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        })
    }) || value["content"].as_array().is_some_and(|parts| {
        parts
            .iter()
            .any(|p| p["text"].as_str().is_some_and(|s| !s.trim().is_empty()))
    })
}

pub(crate) async fn sweep_probes(state: &AppState, limit: u32) -> AppResult<()> {
    let admitted: Vec<PublicProbe> = state
        .db
        .collection::<PublicProbe>(PROBES)
        .find(doc! {"state":"dispatch_admitted","expires_at":{"$lte":bson::DateTime::now()}})
        .limit(i64::from(limit.clamp(1, 100)))
        .await?
        .try_collect()
        .await?;
    for row in admitted {
        complete_probe(&state.db, &row.id, None).await?;
    }
    let expired_consents: Vec<ProbeConsent> = state
        .db
        .collection::<ProbeConsent>(PROBE_CONSENTS)
        .find(doc! {"expires_at":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"expires_at":1,"_id":1})
        .limit(i64::from(limit.clamp(1, 100)))
        .await?
        .try_collect()
        .await?;
    if !expired_consents.is_empty() {
        let ids: Vec<_> = expired_consents
            .iter()
            .map(|consent| consent.id.as_str())
            .collect();
        state
            .db
            .collection::<ProbeConsent>(PROBE_CONSENTS)
            .delete_many(doc! {"_id": {"$in": ids}, "expires_at": {"$lte": bson::DateTime::now()}})
            .await?;
    }
    let rows: Vec<PublicProbe> = state
        .db
        .collection::<PublicProbe>(PROBES)
        .find(
            doc! {"state": {"$in": ["prepared", "funded", "cancelling"]},
            "expires_at": {"$lte": bson::DateTime::now()}},
        )
        .limit(i64::from(limit.clamp(1, 100)))
        .await?
        .try_collect()
        .await?;
    for row in rows {
        if state
            .db
            .collection::<PublicProbe>(PROBES)
            .update_one(
                doc! {"_id":&row.id,"state":{"$in":["prepared","funded"]}},
                doc! {"$set":{"state":"cancelling"}},
            )
            .await?
            .matched_count
            == 0
            && row.state != "cancelling"
        {
            continue;
        }
        billing::reservation::release_unforwarded_rows(
            &state.db,
            &row.id,
            crate::models::usage_meter::UsageStatus::Failed,
            Some("public probe expired before dispatch"),
        )
        .await?;
        let mut session = state.db.client().start_session().await?;
        session
            .start_transaction()
            .and_run((state.clone(), row.clone()), |session, (state, row)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        if state
                            .db
                            .collection::<PublicProbe>(PROBES)
                            .update_one(
                                doc! {"_id":&row.id,"state":"cancelling"},
                                doc! {"$set":{"state":"released"}},
                            )
                            .session(&mut *session)
                            .await?
                            .matched_count
                            == 1
                        {
                            state
                                .db
                                .collection::<PublicBudgetBucket>(BUCKETS)
                                .update_one(
                                    doc! {"_id":&row.bucket_id},
                                    doc! {"$inc":{"held":row.maximum.checked_neg()?.decimal()}},
                                )
                                .session(session)
                                .await?;
                        }
                        Ok(())
                    }
                    .await;
                    tx::transaction_result(result)
                })
            })
            .await
            .map_err(tx::map_transaction_error)?;
    }
    prune_evidence(&state.db, limit).await
}

/// Retain the active deployment's newest proof and every proof still needed
/// by a retained turn or unfinished cleanup. No TTL can erase those authorities.
async fn prune_evidence(db: &Database, limit: u32) -> AppResult<()> {
    use crate::models::{
        platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
        platform_public_turn::{COLLECTION_NAME as TURNS, PlatformPublicTurn},
    };
    let cutoff = bson::DateTime::from_chrono(Utc::now() - Duration::days(7));
    let due = doc! {"$or":[{"retention_check_after":bson::Bson::Null},{"retention_check_after":{"$lte":bson::DateTime::now()}}]};
    let mut filter = due.clone();
    filter.insert("checked_at", doc! {"$lt":cutoff});
    let records: Vec<PublicDeploymentRecord> = db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find(filter)
        .sort(doc! {"checked_at":1,"_id":1})
        .limit(i64::from(limit.clamp(1, 100)))
        .await?
        .try_collect()
        .await?;
    for record in records {
        let active = db.collection::<PublicDeploymentActivation>(ACTIVATIONS)
            .find_one(doc! {"_id": &record.profile_id, "binding.candidate_digest": &record.candidate_digest})
            .await?;
        let live_controls =
            super::platform_bot_runtime_control_service::get(db, &record.profile_id).await?;
        let live_activation = match (&active, &live_controls) {
            (Some(active), Some(controls)) => {
                controls.public_enabled
                    && !controls.retired
                    && !controls.emergency_stopped
                    && policy::binding(&super::platform_bot_runtime_control_service::policy(
                        controls,
                    ))? == active.binding
            }
            _ => false,
        };
        let refresh_authority = if live_activation {
            let active = active.as_ref().ok_or_else(denied)?;
            db.collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .find_one(doc! {"candidate_digest": &active.binding.candidate_digest,
                "evidence.policy_digest": &active.policy_digest, "status": "ready"})
                .sort(doc! {"checked_at": -1, "_id": -1})
                .await?
                .is_some_and(|latest| latest.id == record.id)
        } else {
            false
        };
        let needed_by_turn = db
            .collection::<PlatformPublicTurn>(TURNS)
            .count_documents(doc! {"proof_record_id":&record.id})
            .limit(1)
            .await?
            != 0;
        let needed_by_cleanup = if let Some(e) = &record.evidence {
            let cleanup_authority = db
                .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .find_one(doc! {"evidence.policy_digest":&e.policy_digest,"status":"ready"})
                .sort(doc! {"checked_at":-1,"_id":-1})
                .await?;
            cleanup_authority
                .as_ref()
                .is_some_and(|r| r.id == record.id)
                && db
                    .collection::<PlatformPublicSession>(SESSIONS)
                    .count_documents(
                        doc! {"policy_digest":&e.policy_digest,"cleanup_complete":false},
                    )
                    .limit(1)
                    .await?
                    != 0
        } else {
            false
        };
        if refresh_authority || needed_by_turn || needed_by_cleanup {
            db.collection::<Document>(DEPLOYMENT_RECORDS)
                .update_one(
                    doc! {"_id": &record.id},
                    doc! {"$set": {"retention_check_after": bson::DateTime::from_chrono(Utc::now() + Duration::hours(1))}},
                )
                .await?;
        } else {
            db.collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .delete_one(doc! {"_id":&record.id,"checked_at":{"$lt":cutoff}})
                .await?;
        }
    }
    let mut filter = due;
    filter.insert("expires_at", doc! {"$lt":cutoff});
    filter.insert("state", doc! {"$in":["settled","released"]});
    let probes: Vec<PublicProbe> = db
        .collection::<PublicProbe>(PROBES)
        .find(filter)
        .sort(doc! {"expires_at":1,"_id":1})
        .limit(i64::from(limit.clamp(1, 100)))
        .await?
        .try_collect()
        .await?;
    for probe in probes {
        if db
            .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
            .count_documents(doc! {"evidence.probe_id":&probe.id})
            .limit(1)
            .await?
            != 0
        {
            db.collection::<Document>(PROBES)
                .update_one(
                    doc! {"_id": &probe.id},
                    doc! {"$set": {"retention_check_after": bson::DateTime::from_chrono(Utc::now() + Duration::hours(1))}},
                )
                .await?;
        } else {
            db.collection::<PublicProbe>(PROBES)
                .delete_one(
                    doc! {"_id": &probe.id, "state": {"$in": ["settled", "released"]},
                    "expires_at": {"$lt": cutoff}},
                )
                .await?;
        }
    }
    Ok(())
}

pub(crate) fn endpoint_set_digest(endpoints: &[ServiceEndpoint]) -> AppResult<String> {
    let mut projection: Vec<Value> = endpoints
        .iter()
        .map(|e| {
            let mut value = serde_json::to_value(e).map_err(|_| denied())?;
            if let Some(object) = value.as_object_mut() {
                object.remove("created_at");
                object.remove("updated_at");
            }
            Ok(value)
        })
        .collect::<AppResult<_>>()?;
    projection.sort_by_key(|v| v.to_string());
    policy::digest(&projection)
}

async fn model_bindings(db: &Database, sponsor: &str) -> AppResult<Vec<Value>> {
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! {"is_active": true, "service_type": "http",
        "offering_kind": {"$ne": "tool"}, "inference.model_list": true})
        .limit(65)
        .await?
        .try_collect()
        .await?;
    if services.len() > 64 {
        return Err(denied());
    }
    let mut bindings = vec![];
    for service in services {
        let protocol = match proxy::protocol(&service) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let path = proxy::inference_path(&service, protocol)?;
        let endpoints = super::service_endpoint_service::list_endpoints(db, &service.id).await?;
        let canonical = super::proxy_authorization::CanonicalPath::from_rest_decoded(&path)?;
        let active =
            super::tool_publication_service::active_operation(&endpoints, "POST", &canonical);
        let (operation, generation) = match active {
            Some(name) => {
                if super::tool_publication_service::require_published_operation(
                    &endpoints, "POST", &canonical,
                )
                .is_err()
                {
                    continue;
                }
                let Some(ep) = endpoints.iter().find(|e| {
                    e.name == name && e.operation_generation > 0 && e.target_id.is_none()
                }) else {
                    continue;
                };
                (name, Some(ep.operation_generation))
            }
            None if endpoints.is_empty() => (proxy::declared_operation(protocol).into(), None),
            _ => continue,
        };
        let op = PublicModelOperation {
            reasoning_effort: None,
            service_id: service.id.clone(),
            operation: operation.clone(),
            method: http::Method::POST,
            canonical_path: path.clone(),
            protocol,
            execution_authority_digest: proxy::catalog_digest(&service)?,
            endpoint_generation: generation,
            model_id: "discovery-only".into(),
            max_input_bytes: 8192,
            max_input_tokens: 8192,
            max_output_tokens: 512,
            max_response_bytes: 65536,
        };
        if proxy::require_model_catalog_for_sponsor(&service, sponsor, &op).is_err() {
            continue;
        }
        let protocol = match protocol {
            PublicModelProtocol::OpenAiResponses => "openai_responses",
            PublicModelProtocol::OpenAiCompletions => "openai_completions",
            PublicModelProtocol::AnthropicMessages => "anthropic_messages",
        };
        bindings.push(json!({
            "service_id": service.id, "service_slug": service.slug,
            "operation": operation, "method": "POST", "canonical_path": path, "protocol": protocol,
            "execution_authority_digest": op.execution_authority_digest,
            "endpoint_generation": generation.unwrap_or(0), "endpoint_set_digest": endpoint_set_digest(&endpoints)?,
            "max_input_bytes": op.max_input_bytes, "max_input_tokens": op.max_input_tokens,
            "max_output_tokens": op.max_output_tokens, "max_response_bytes": op.max_response_bytes,
        }));
    }
    Ok(bindings)
}

pub(crate) async fn record_probe_consent(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    reviewed: &ManagerRuntimePolicy,
) -> AppResult<()> {
    let binding = policy::binding(reviewed)?;
    let profile = db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id": &binding.profile_id, "manager_bot_id": &binding.manager_bot_id,
            "deleted_at": bson::Bson::Null},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    let controls = super::platform_bot_runtime_control_service::get_in_session(
        db,
        session,
        &binding.profile_id,
    )
    .await?;
    let consent = ProbeConsent {
        id: uuid::Uuid::new_v4().to_string(),
        binding: binding.clone(),
        actor_user_id: actor.into(),
        profile_generation: profile.generation,
        controls_id: controls.as_ref().map(|r| r.id.clone()),
        manager_authority_epoch: controls.as_ref().map_or(0, |r| r.manager_authority_epoch),
        personal_control_revision: controls.as_ref().map_or(0, |r| r.personal_control_revision),
        public_policy_revision: controls.as_ref().map_or(0, |r| r.public_policy_revision),
        expires_at: Utc::now() + Duration::minutes(5),
    };
    db.collection::<ProbeConsent>(PROBE_CONSENTS)
        .insert_one(consent)
        .session(session)
        .await?;
    Ok(())
}

/// The candidate may be a reviewed future enable, or the exact enabled policy
/// being refreshed. Both paths fence transport and revocation before funding.
async fn fence_probe_authority(
    db: &Database,
    session: &mut ClientSession,
    binding: &ManagerBinding,
    activation_digest: Option<&str>,
) -> AppResult<()> {
    let live = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &binding.profile_id,
        &binding.manager_bot_id,
        &binding.entry,
        None,
    )
    .await?;
    if let Some(expected) = activation_digest {
        let row = live.as_ref().ok_or_else(denied)?;
        let reviewed = super::platform_bot_runtime_control_service::policy(row);
        if !row.retired
            && !row.emergency_stopped
            && row.public_enabled
            && policy::binding(&reviewed)? == *binding
        {
            let active = db.collection::<PublicDeploymentActivation>(ACTIVATIONS)
                .find_one_and_update(doc! {"_id": &binding.profile_id,
                    "binding": bson::to_bson(binding).map_err(|_| denied())?, "policy_digest": expected},
                    doc! {"$inc": {"effect_revision": 1_i64}})
                .session(&mut *session).await?;
            if active.is_some() {
                return Ok(());
            }
        }
        return Err(denied());
    }
    let consent = db
        .collection::<ProbeConsent>(PROBE_CONSENTS)
        .find_one_and_update(
            doc! {
                "binding": bson::to_bson(binding).map_err(|_| denied())?,
                "$expr": {"$gt": ["$expires_at", "$$NOW"]},
            },
            doc! {"$inc": {"effect_revision": 1_i64}},
        )
        .sort(doc! {"expires_at": -1})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    super::platform_bot_config_service::fence_admin(db, &consent.actor_user_id, session).await?;
    let membership = db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(doc! {
            "org_user_id": &binding.sponsor_owner_user_id, "member_user_id": &consent.actor_user_id,
            "role": "admin", "revoked_at": bson::Bson::Null,
        }, doc! {"$inc": {"public_probe_fence": 1_i64}})
        .session(&mut *session).await?;
    if membership.matched_count != 1 {
        return Err(denied());
    }
    let profile = db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one_and_update(
            doc! {
                "_id": &binding.profile_id, "manager_bot_id": &binding.manager_bot_id,
                "generation": consent.profile_generation, "deleted_at": bson::Bson::Null,
            },
            doc! {"$inc": {"public_probe_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    if super::platform_bot_runtime_control_service::profile_identity(&profile)? != binding.entry {
        return Err(denied());
    }
    match &binding.entry {
        super::platform_manager_contract::EntryIdentity::TelegramV1 { .. } => {
            for (collection, filter) in [
                (
                    crate::models::platform_credential::COLLECTION_NAME,
                    doc! {"provider":"telegram-new","fields.manager_bot_id":&binding.manager_bot_id,"fields.webhook_ready":"true"},
                ),
                (
                    crate::models::channel_bot::COLLECTION_NAME,
                    doc! {"_id":&profile.channel_bot_id,"platform_profile_id":&binding.profile_id,"platform_bot_id":&binding.manager_bot_id,"is_active":true},
                ),
            ] {
                if db
                    .collection::<Document>(collection)
                    .update_one(filter, doc! {"$inc":{"public_probe_fence":1_i64}})
                    .session(&mut *session)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(denied());
                }
            }
        }
        super::platform_manager_contract::EntryIdentity::NativeV2 { .. } => {
            if !super::platform_bot_runtime_control_service::native_transport_ready(
                db, session, &profile, true,
            )
            .await?
            {
                return Err(denied());
            }
            if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .update_one(doc! {"_id":&profile.channel_bot_id,"platform_profile_id":&profile.id,"platform_entry_version":2,
                    "platform":super::platform_entry_scope::platform(&binding.entry),"platform_bot_id":&binding.manager_bot_id,"is_active":true,"status":"active","platform_entry_pending":{"$exists":false}},
                    doc! {"$inc":{"public_probe_fence":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(denied());}
        }
    }
    match (live, consent.controls_id) {
        (None, None) => {}
        (Some(row), Some(id))
            if row.id == id
                && !row.retired
                && !row.emergency_stopped
                && row.manager_authority_epoch == consent.manager_authority_epoch
                && row.personal_control_revision == consent.personal_control_revision
                && row.public_policy_revision == consent.public_policy_revision => {}
        _ => return Err(denied()),
    }
    Ok(())
}

/// Called inside the existing billing funding transaction. No second wallet
/// debit or gap between the one-use probe claim, holds and recoverable meters.
pub(crate) async fn fund_probe_in_session(
    db: &Database,
    session: &mut ClientSession,
    route: &billing::BillingRouteContext,
    layers: &[billing::reservation::LayerReservation],
) -> AppResult<()> {
    let gross = layers
        .iter()
        .map(|l| l.reserved_credits)
        .try_fold(Credits::ZERO, Credits::checked_add)?;
    let admission = route.public_admission.as_ref().ok_or_else(denied)?;
    let row = db
        .collection::<PublicProbe>(PROBES)
        .find_one(doc! {"_id":&admission.call_id})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    fence_probe_authority(db, session, &row.binding, row.activation_digest.as_deref()).await?;
    fence_deployment(db, session, &probe_evidence(&row)).await?;
    if row.operation.model_id != admission.model_id
        || row.operation.service_id != route.catalog_service_id.as_deref().unwrap_or("")
        || row.binding.sponsor_owner_user_id != route.billing_owner_id
        || route.api_key_id.is_some()
        || gross != row.maximum
        || pricing_digest(layers)? != row.pricing_digest
    {
        return Err(denied());
    }
    let funded = db
        .collection::<PublicProbe>(PROBES)
        .update_one(
            doc! {
                "_id": &row.id, "state": "prepared",
                "route_digest": super::platform_public_budget_service::route_digest(route)?,
                "$expr": {"$gt": ["$expires_at", "$$NOW"]},
            },
            doc! {"$set": {"state": "funded"}},
        )
        .session(session)
        .await?;
    if funded.matched_count != 1 {
        return Err(denied());
    }
    Ok(())
}

fn pricing_digest(layers: &[billing::reservation::LayerReservation]) -> AppResult<String> {
    policy::digest(
        &layers
            .iter()
            .map(|l| {
                (
                    &l.layer,
                    &l.metric,
                    &l.lago_metric_code,
                    l.estimated_quantity,
                    l.credits_per_unit_micros,
                    l.credits_per_unit_pico,
                    l.reserved_credits.to_string(),
                )
            })
            .collect::<Vec<_>>(),
    )
}
fn probe_evidence(p: &PublicProbe) -> DeploymentEvidence {
    DeploymentEvidence {
        binding: p.binding.clone(),
        manifest: p.manifest.clone(),
        selected_profile: p.selected_profile.clone(),
        nyxagent_service_id: p.nyxagent_service_id.clone(),
        nyxagent_catalog_digest: p.nyxagent_catalog_digest.clone(),
        policy_digest: String::new(),
        evidence_digest: String::new(),
        probe_id: p.id.clone(),
        expires_at: p.expires_at,
    }
}

async fn complete_probe(db: &Database, id: &str, observed: Option<bool>) -> AppResult<()> {
    use crate::models::usage_meter::{COLLECTION_NAME as METERS, UsageMeterRow};
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db.clone(), id.to_owned(), observed),
            |session, (db, id, observed)| {
                Box::pin(async move {
                    let result: AppResult<()> = async {
                        let Some(row) = db.collection::<PublicProbe>(PROBES)
                            .find_one(doc! {"_id": &*id, "state": "dispatch_admitted"})
                            .session(&mut *session).await?
                        else {
                            return Ok(());
                        };
                        if row.meter_row_ids.is_empty() {
                            return Err(denied());
                        }
                        let mut actual = Credits::ZERO;
                        for meter_id in &row.meter_row_ids {
                            let meter = db.collection::<UsageMeterRow>(METERS)
                                .find_one(doc! {
                                    "_id": meter_id, "billing_request_id": &*id,
                                    "billing_owner_id": &row.binding.sponsor_owner_user_id,
                                    "api_key_id": bson::Bson::Null, "forwarded": true,
                                }).session(&mut *session).await?.ok_or_else(denied)?;
                            let Some(funding) = meter.funding.filter(|funding| funding.settled) else {
                                return Ok(());
                            };
                            let Some(cost) = funding.total_charge else {
                                return Ok(());
                            };
                            if cost < Credits::ZERO {
                                return Err(denied());
                            }
                            actual = actual.checked_add(cost)?;
                        }
                        let settled = db.collection::<PublicProbe>(PROBES)
                            .update_one(
                                doc! {"_id": &*id, "state": "dispatch_admitted"},
                                doc! {"$set": {"state": "settled", "exact_model_observed": observed.unwrap_or(false)}},
                            ).session(&mut *session).await?;
                        if settled.matched_count != 1 {
                            return Err(denied());
                        }
                        let reconciled = db.collection::<PublicBudgetBucket>(BUCKETS)
                            .update_one(
                                doc! {"_id": &row.bucket_id},
                                doc! {"$inc": {"spent": actual.checked_sub(row.maximum)?.decimal()}},
                            ).session(session).await?;
                        if reconciled.matched_count != 1 {
                            return Err(denied());
                        }
                        Ok(())
                    }.await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)
}

#[cfg(test)]
mod tests;
