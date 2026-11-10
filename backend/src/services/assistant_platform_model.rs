//! Actor-bound platform inference. A context is a binding, never bearer authority.
//! Legacy profiles take the no-I/O path through all hooks in this module.
use std::collections::HashSet;

use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    assistant_acknowledgement_service as acks, platform_model_execution as execution,
    platform_model_service as registry,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{assistant_agent::AssistantAgent, assistant_conversation::AssistantConversation},
    mw::auth::{AuthMethod, AuthUser},
};

// Capability discovery forwards through the same classified proxy as legacy
// model discovery. Exempt policies deliberately cannot forward downstream.
pub(crate) const CAPABILITY_POLICY: super::billing::route_inventory::BillingRoutePolicy =
    super::billing::route_inventory::BillingRoutePolicy::Metered(
        super::billing::route_inventory::BillingIngress::Proxy,
    );
pub const READINESS: &str = "assistant:platform-models-v1";
pub const PROTOCOL: &str = "nyxagent-platform-models-v1";
pub const CONTEXT_HEADER: &str = "x-nyxid-model-context";
pub const OCCURRENCE_HEADER: &str = "x-nyxid-model-occurrence";
const COLLECTION: &str = super::assistant_service_execution::COLLECTION;
const VERSION: i32 = 1;

pub fn valid_id(model: &str) -> bool {
    model.strip_prefix("nyxid/").is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 120
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-._".contains(&c))
    })
}
pub fn unavailable() -> AppError {
    AppError::AssistantModelUnavailable
}

pub async fn require_ready(db: &Database) -> AppResult<()> {
    registry::require_execution_ready(db).await?;
    for capability in [READINESS, execution::images::READINESS] {
        if !super::security_readiness_service::CAPABILITIES.contains(&capability)
            || !super::security_readiness_service::report(db, capability)
                .await?
                .ready
        {
            return Err(AppError::SecurityReadinessUnavailable);
        }
    }
    Ok(())
}

/// Exact paired capability; positive deployment metadata is not turn authority.
pub fn bridge_ready(value: &Value) -> bool {
    let cap = &value["platform_models"];
    cap["protocol"] == PROTOCOL
        && cap["version"] == VERSION
        && [
            "text",
            "function_tools",
            "image_input",
            "streaming",
            "compaction",
        ]
        .iter()
        .all(|key| cap[*key] == true)
}

/// The same validator is used by persisted selections and every turn admission.
/// It never widens the specialist's offering grant as a side effect of selection.
pub async fn validate_selection(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    model: &str,
    guest: bool,
) -> AppResult<()> {
    if !model.starts_with("nyxid/") {
        return Ok(());
    }
    if guest || !valid_id(model) || agent.destroyed_at.is_some() {
        return Err(unavailable());
    }
    require_ready(db).await?;
    let public = registry::authorized_model(db, actor, model)
        .await
        .map_err(selection_error)?;
    let offering = public
        .billing_service_id
        .as_deref()
        .ok_or_else(unavailable)?;
    agent_scope(db, actor, agent, offering).await?;
    Ok(())
}
fn selection_error(error: AppError) -> AppError {
    match error {
        AppError::NotFound(_) | AppError::Forbidden(_) => unavailable(),
        other => other,
    }
}
async fn agent_scope(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    offering: &str,
) -> AppResult<()> {
    if !agent.is_nyxbot()
        && !agent
            .grants
            .platform_service_ids
            .iter()
            .any(|s| s == offering)
    {
        return Err(unavailable());
    }
    if agent.user_id != actor {
        let access = super::org_agent_service::resolve_agent_access(db, actor, agent)
            .await?
            .ok_or_else(unavailable)?;
        if let Some(shared) = access.shared()
            && !super::shared_agent_service::permits(
                shared.level(offering),
                super::mcp_service::OperationEffects {
                    reads: true,
                    uses: true,
                    destructive: false,
                },
            )
        {
            return Err(unavailable());
        }
    }
    if !super::agent_operation_scope_service::route_visible(
        &agent.operation_scopes,
        offering,
        Some(offering),
        "POST",
        "/v1/chat/completions",
    ) {
        return Err(unavailable());
    }
    Ok(())
}

/// Kept in existing assistant execution storage and its TTL/deletion lifecycle.
/// No content, credentials or private route identity is persisted here.
pub(crate) use crate::models::assistant_platform_model::Admission;

pub(crate) async fn bind_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &mut AssistantConversation,
) -> AppResult<()> {
    if !row.model.starts_with("nyxid/") {
        return Ok(());
    }
    // Never manufacture an original person for an owner-paid guest or external voice key.
    if row.guest_turn
        || row.guest_model_setup
        || row
            .active_turn
            .as_ref()
            .is_some_and(|t| t.channel_adapter_origin.is_some())
    {
        return Err(unavailable());
    }
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":row.agent_id.as_deref().ok_or_else(unavailable)?,"destroyed_at":null})
        .session(&mut *session)
        .await?
        .ok_or_else(unavailable)?;
    validate_selection(db, &row.user_id, &agent, &row.model, false).await?;
    let model = registry::authorized_model(db, &row.user_id, &row.model).await?;
    let key = db
        .collection::<crate::models::api_key::ApiKey>("api_keys")
        .find_one(doc! {"_id":&row.credential_api_key_id,"user_id":&row.user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(unavailable)?;
    if !key.allow_all_services
        && !key
            .allowed_platform_service_ids
            .contains(model.billing_service_id.as_ref().ok_or_else(unavailable)?)
    {
        return Err(unavailable());
    }
    let turn = row
        .active_turn
        .as_mut()
        .ok_or(AppError::AssistantTurnRequired)?;
    let admission = Admission {
        handle: uuid::Uuid::new_v4().to_string(),
        kind: "platform_model_admission".into(),
        version: VERSION,
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        api_key_id: row.credential_api_key_id.clone(),
        turn_id: turn.turn_id.clone(),
        agent_id: agent.id,
        model: row.model.clone(),
        revision: model.revision,
        authorization_revision: model.authorization_revision,
        offering: model.billing_service_id.ok_or_else(unavailable)?,
        scopes: agent.operation_scopes,
        key_scopes: key.assistant_operation_scopes,
        bridge_ready: false,
        expires_at: Utc::now() + chrono::Duration::hours(24),
    };
    db.collection::<Admission>(COLLECTION)
        .insert_one(&admission)
        .session(session)
        .await?;
    turn.platform_model_context = Some(admission.handle);
    Ok(())
}

pub(crate) async fn live(
    state: &AppState,
    auth: &AuthUser,
    handle: &str,
) -> AppResult<(Admission, AuthUser)> {
    if auth.auth_method != AuthMethod::ApiKey
        || auth.api_key_credential_id.is_some()
        || auth.oauth_client_id.is_some()
        || auth.acting_client_id.is_some()
        || auth.api_key_purpose != crate::models::api_key::ApiKeyPurpose::General
    {
        return Err(registry::unavailable());
    }
    let original = auth
        .assistant_chat
        .as_ref()
        .ok_or_else(registry::unavailable)?;
    if original.guest {
        return Err(unavailable());
    }
    let key =
        super::key_service::get_api_key(&state.db, &original.user_id, &original.api_key_id).await?;
    if !key.is_active || key.expires_at.is_some_and(|t| t <= Utc::now()) {
        return Err(AppError::AssistantTurnRequired);
    }
    let live = Box::pin(crate::mw::auth::api_key_auth_user(
        &state.db, &key, None, None, None,
    ))
    .await?;
    let chat = live
        .assistant_chat
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    if chat.guest
        || chat.user_id != original.user_id
        || chat.conversation_id != original.conversation_id
        || chat.agent_id != original.agent_id
        || chat.turn_id != original.turn_id
        || !chat.turn_live
        || chat.turn_stopped
    {
        return Err(AppError::AssistantTurnRequired);
    }
    chat.ensure_live_turn(&state.db, None, None, "assistant.platform_model")
        .await?;
    let row = state
        .db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,
        "credential_api_key_id":&chat.api_key_id,"active_turn.turn_id":&chat.turn_id,
        "active_turn.platform_model_context":handle})
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    if row.guest_turn
        || row
            .active_turn
            .as_ref()
            .is_some_and(|t| t.channel_adapter_origin.is_some())
    {
        return Err(unavailable());
    }
    if row.collaboration.is_some() {
        super::collaboration_group_service::runtime::refresh(&state.db, &row).await?;
    }
    let admission = state.db.collection::<Admission>(COLLECTION).find_one(doc! {
        "_id":handle,"kind":"platform_model_admission","version":VERSION,"bridge_ready":true,
        "user_id":&chat.user_id,"api_key_id":&chat.api_key_id,"conversation_id":&chat.conversation_id,
        "turn_id":&chat.turn_id,"expires_at":{"$gt":bson::DateTime::now()}
    }).await?.ok_or(AppError::AssistantTurnRequired)?;
    if admission.model != row.model || admission.agent_id != chat.agent_id {
        return Err(unavailable());
    }
    let agent = super::org_agent_service::chat_agent(&state.db, chat).await?;
    validate_selection(&state.db, &chat.user_id, &agent, &admission.model, false).await?;
    let model = registry::authorized_model(&state.db, &chat.user_id, &admission.model).await?;
    if model.revision != admission.revision
        || model.authorization_revision != admission.authorization_revision
        || model.billing_service_id.as_deref() != Some(&admission.offering)
        || (!key.allow_all_services
            && !key
                .allowed_platform_service_ids
                .contains(&admission.offering))
    {
        return Err(unavailable());
    }
    Ok((admission, live))
}

/// Read-only plan: selects certified public bounds without minting tokens or funding.
pub(crate) async fn selection_plan(
    state: &AppState,
    person: &str,
    model: &str,
) -> AppResult<execution::Plan> {
    // This only validates a plan; it never sends the probe. Sixteen is the
    // certified Responses adapter's minimum and is accepted by every supported
    // family. A one-token probe would incorrectly hide valid Responses routes.
    let request = execution::Request::parse_async(http::Method::POST, "chat/completions".into(),
        json!({"model":model,"messages":[{"role":"user","content":"Model capability validation"}],"max_tokens":16}), person).await?;
    let plan = execution::Plan::load(
        &state.db,
        person,
        model,
        None,
        state.config.billing_rate_cache_ttl_secs,
        &request,
    )
    .await?;
    let cap = &plan.model.capabilities;
    if cap.tools != Some(true)
        || cap.streaming != Some(true)
        || cap.context_tokens.is_none()
        || cap.output_tokens.is_none()
    {
        return Err(unavailable());
    }
    Ok(plan)
}

#[derive(Serialize)]
pub(crate) struct Manifest {
    version: i32,
    protocol: &'static str,
    model: String,
    context_tokens: u32,
    max_output_tokens: i64,
    max_request_bytes: i64,
    image_input: Option<execution::images::PublicLimits>,
}
pub(crate) fn manifest(plan: &execution::Plan, ingress_limit: usize) -> AppResult<Manifest> {
    Ok(Manifest {
        version: VERSION,
        protocol: PROTOCOL,
        model: plan.model.public_id.clone(),
        context_tokens: plan
            .model
            .capabilities
            .context_tokens
            .ok_or_else(unavailable)?,
        max_output_tokens: i64::from(
            plan.model
                .capabilities
                .output_tokens
                .ok_or_else(unavailable)?,
        )
        .min(plan.tariff.tariff().bounds.max_output_tokens),
        max_request_bytes: plan
            .tariff
            .tariff()
            .bounds
            .max_request_bytes
            .min(ingress_limit as i64),
        image_input: plan.image_limits(),
    })
}

pub(crate) async fn check_request(
    state: &AppState,
    original: &AuthUser,
    handle: &str,
    plan: &execution::Plan,
    body: &[u8],
) -> AppResult<AuthUser> {
    let (admission, live) = live(state, original, handle).await?;
    if admission.model != plan.model.public_id || admission.revision != plan.model.revision {
        return Err(unavailable());
    }
    let model = registry::authorized_model(&state.db, &admission.user_id, &admission.model).await?;
    if model.routing_revision_id != plan.model.routing_revision_id {
        return Err(unavailable());
    }
    let offering = state.db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! {"_id":&admission.offering,"is_active":true,"platform_key.enabled":true,"platform_key.purpose":"model_offering"})
        .await?.ok_or_else(unavailable)?;
    if offering.updated_at != plan.offering.updated_at {
        return Err(unavailable());
    }
    let canonical =
        super::proxy_authorization::CanonicalPath::from_mcp_built("/v1/chat/completions")?;
    super::proxy_authorization::authorize_proxy_operation(&offering, "POST", &canonical)?;
    for scopes in [
        &admission.scopes,
        &admission.key_scopes,
        &original.assistant_operation_scopes,
        &live.assistant_operation_scopes,
    ] {
        super::agent_operation_scope_service::authorize(
            scopes,
            &admission.offering,
            Some(&admission.offering),
            None,
            "POST",
            &canonical,
            false,
            false,
        )?;
        super::agent_operation_scope_service::check_inputs(
            scopes,
            &admission.offering,
            Some(&admission.offering),
            None,
            "POST",
            &canonical,
            None,
            body,
            Some("application/json"),
        )?;
    }
    let operation = inference_operation(body);
    let approval_owner = approval_owner(&live);
    if super::approval_service::evaluate_deny_only(
        &state.db,
        &admission.user_id,
        &approval_owner,
        &admission.offering,
        &operation,
        false,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            "Operation denied by approval policy".into(),
        ));
    }
    Ok(live)
}

/// Only for the adapter's parsed/certified stateless Chat request. Logical model
/// offerings have no legacy inference connection metadata. Provider-side effects
/// and unknown fields have already been refused by Request/Plan before this call.
pub(crate) fn inference_operation(body: &[u8]) -> super::operation_descriptor::OperationDescriptor {
    let mut operation = super::operation_descriptor::build_llm_descriptor(
        "POST",
        "/v1/chat/completions",
        Some(body),
    );
    operation.verb = crate::models::service_approval_config::ApprovalVerb::Read;
    operation
}

/// Only trusted server code receives this marker. Headers cannot turn off billing.
#[derive(Clone)]
pub(crate) struct Orchestration {
    _private: (),
}
impl Orchestration {
    pub(crate) async fn for_body(
        state: &AppState,
        row: &AssistantConversation,
        body: &Value,
    ) -> AppResult<Option<Self>> {
        if !row.model.starts_with("nyxid/") {
            return Ok(None);
        }
        let handle = row
            .active_turn
            .as_ref()
            .and_then(|t| t.platform_model_context.as_deref())
            .ok_or_else(unavailable)?;
        if super::assistant_nyxagent::live_turn(row, Utc::now()).is_none() {
            return Err(AppError::AssistantTurnRequired);
        }
        if body["nyxid_model_context"]["version"] != VERSION
            || body["nyxid_model_context"]["handle"] != handle
            || body["model"] != row.model
            || row.guest_turn
        {
            return Err(unavailable());
        }
        require_ready(&state.db).await?;
        Ok(Some(Self { _private: () }))
    }
}

pub(crate) fn image_capabilities(
    mut capabilities: Value,
    limits: Option<&execution::images::PublicLimits>,
) -> Value {
    let Some(limits) = limits else {
        capabilities["input_image"] = Value::Null;
        return capabilities;
    };
    for (field, bound) in [
        ("max_images", limits.max_images),
        ("max_image_bytes", limits.max_file_bytes),
        ("max_total_bytes", limits.max_total_file_bytes),
    ] {
        let previous = capabilities["input_image"][field].as_u64().unwrap_or(0);
        capabilities["input_image"][field] = json!(previous.min(bound as u64));
    }
    if let Some(types) = capabilities["input_image"]["content_types"].as_array_mut() {
        types.retain(|v| v.as_str().is_some_and(|s| limits.mime_types.contains(&s)));
    }
    capabilities
}

/// Bind an image read's bytes to this exact admitted turn. No image payload is stored.
pub(crate) async fn record_image_read(
    db: &Database,
    chat: &acks::ChatAuthority,
    id: &str,
    bytes: &[u8],
) -> AppResult<()> {
    let Some(handle) = chat.platform_model_context.as_deref() else {
        return Ok(());
    };
    if chat.guest {
        return Err(unavailable());
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    let proof = format!("{:x}", Sha256::digest(format!("{handle}:{id}:{digest}")));
    db.collection::<Document>(COLLECTION).update_one(doc! {"_id":proof},doc! {"$setOnInsert":{
        "kind":"platform_model_image","context":handle,"attachment_id":id,"digest":digest,
        "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,"turn_id":&chat.turn_id,
        "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(24))
    }}).upsert(true).await?;
    Ok(())
}

pub(crate) fn image_digests(value: &Value) -> AppResult<HashSet<String>> {
    use base64::Engine;
    let mut digests = HashSet::new();
    for part in value["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|p| p["type"] == "image_url")
    {
        let data = part["image_url"]["url"]
            .as_str()
            .and_then(|s| s.split_once(";base64,").map(|(_, s)| s))
            .ok_or_else(unavailable)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| unavailable())?;
        digests.insert(format!("{:x}", Sha256::digest(&bytes)));
    }
    Ok(digests)
}
pub(crate) async fn check_images(
    db: &Database,
    chat: &acks::ChatAuthority,
    handle: &str,
    digests: &HashSet<String>,
) -> AppResult<()> {
    use futures::TryStreamExt;
    if digests.is_empty() {
        return Ok(());
    }
    let rows:Vec<Document> = db.collection(COLLECTION).find(doc! {"kind":"platform_model_image","context":handle,
        "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,"turn_id":&chat.turn_id,
        "digest":{"$in":digests.iter().collect::<Vec<_>>()},"expires_at":{"$gt":bson::DateTime::now()}})
        .await?.try_collect().await?;
    for digest in digests {
        let mut valid = false;
        for proof in rows
            .iter()
            .filter(|p| p.get_str("digest").ok() == Some(digest.as_str()))
        {
            let id = proof.get_str("attachment_id").map_err(|_| unavailable())?;
            match super::assistant_upload_service::for_egress(db, chat, id).await {
                Ok(_) => {
                    valid = true;
                    break;
                }
                Err(
                    AppError::NotFound(_)
                    | AppError::Forbidden(_)
                    | AppError::AssistantAttachmentExpired,
                ) => {}
                Err(error) => return Err(error),
            }
        }
        if !valid {
            return Err(AppError::AssistantAttachmentExpired);
        }
    }
    Ok(())
}

/// A durable occurrence claims at most one executor invocation. Uncertain or lost
/// output never reopens a claim; the bridge reads its own retained output instead.
pub(crate) async fn claim_occurrence(
    db: &Database,
    admission: &Admission,
    occurrence: &str,
    body: &[u8],
) -> AppResult<String> {
    let occurrence = uuid::Uuid::parse_str(occurrence)
        .map_err(|_| AppError::BadRequest("Invalid model occurrence".into()))?;
    let id = format!(
        "{:x}",
        Sha256::digest(format!("{}:{occurrence}", admission.handle))
    );
    let digest = format!("{:x}", Sha256::digest(body));
    let inserted = db.collection::<Document>(COLLECTION).update_one(doc! {"_id":&id},doc! {"$setOnInsert":{
        "kind":"platform_model_occurrence","context":&admission.handle,"digest":&digest,
        "user_id":&admission.user_id,"conversation_id":&admission.conversation_id,"turn_id":&admission.turn_id,
        "expires_at":bson::DateTime::from_chrono(admission.expires_at),"state":"claimed"
    }}).upsert(true).await?;
    if inserted.upserted_id.is_none() {
        return Err(AppError::Conflict(
            "Model occurrence already admitted; recover its existing result, never resend".into(),
        ));
    }
    Ok(id)
}

/// Only the server runtime can activate callbacks after checking the paired bridge.
pub(crate) async fn activate_bridge(
    db: &Database,
    row: &AssistantConversation,
    capability: &Value,
) -> AppResult<()> {
    if !bridge_ready(capability) || row.guest_turn {
        return Err(unavailable());
    }
    let turn = super::assistant_nyxagent::live_turn(row, Utc::now())
        .ok_or(AppError::AssistantTurnRequired)?;
    let handle = turn
        .platform_model_context
        .as_deref()
        .ok_or_else(unavailable)?;
    let result = db
        .collection::<Document>(COLLECTION)
        .update_one(
            doc! {
                "_id":handle,"kind":"platform_model_admission","version":VERSION,
                "user_id":&row.user_id,"conversation_id":&row.id,"turn_id":&turn.turn_id,
                "api_key_id":&row.credential_api_key_id,"model":&row.model,
                "expires_at":{"$gt":bson::DateTime::now()}
            },
            doc! {"$set":{"bridge_ready":true}},
        )
        .await?;
    if result.matched_count != 1 {
        return Err(unavailable());
    }
    Ok(())
}

/// Public saved selections are never an entitlement side channel. Legacy paths
/// perform no extra reads; inaccessible selections remain explicitly unavailable.
pub(crate) async fn public_selection(db: &Database, actor: &str, model: &str) -> AppResult<String> {
    if !model.starts_with("nyxid/") {
        return Ok(model.into());
    }
    match registry::authorized_model(db, actor, model).await {
        Ok(_) => Ok(model.into()),
        Err(
            AppError::NotFound(_) | AppError::Forbidden(_) | AppError::SecurityReadinessUnavailable,
        ) => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Organization specialist policy remains an independent floor. Personal shares
/// never borrow the sharer's approval settings or decisions.
pub(crate) fn approval_owner(auth: &AuthUser) -> String {
    if auth
        .org_agent_access
        .as_deref()
        .is_some_and(|access| access.shared().is_none())
    {
        return auth
            .assistant_agent_owner_id
            .clone()
            .unwrap_or_else(|| auth.user_id.to_string());
    }
    auth.user_id.to_string()
}

#[cfg(test)]
mod billing_tests {
    use super::*;
    use crate::{
        models::{
            service_billing::{BillingMetric, ResaleSpec, ServiceBilling},
            usage_meter::CredentialClass,
        },
        services::billing::{BillingIngress, BillingRouteContext, NodeIntent},
    };
    #[test]
    fn trusted_outer_marker_suppresses_token_copies_but_preserves_orchestration_requests() {
        for metric in [
            BillingMetric::Tokens,
            BillingMetric::InputTokens,
            BillingMetric::ImageInputTokens,
            BillingMetric::Requests,
        ] {
            let billing = ServiceBilling {
                platform_billable: true,
                platform_charge_nyxid_credentials_only: false,
                ..Default::default()
            };
            let mut original = BillingRouteContext::new(
                BillingIngress::Proxy,
                "request".into(),
                "person".into(),
                "person".into(),
                None,
                None,
                None,
                Some("llm-nyx".into()),
                NodeIntent::Direct,
                "none".into(),
                CredentialClass::NoAuth,
                metric,
                Some(&billing),
                false,
            );
            original.platform_components = vec![
                ResaleSpec {
                    metric: BillingMetric::InputTokens,
                    lago_metric_code: "input".into(),
                },
                ResaleSpec {
                    metric: BillingMetric::Requests,
                    lago_metric_code: "orchestration".into(),
                },
            ];
            let adjusted = original
                .clone()
                .with_platform_model_orchestration(&Orchestration { _private: () });
            assert!(
                original.service_platform_billable,
                "legacy context is unchanged"
            );
            assert_eq!(
                adjusted.service_platform_billable,
                metric == BillingMetric::Requests
            );
            assert_eq!(adjusted.platform_components.len(), 1);
            assert_eq!(
                adjusted.platform_components[0].lago_metric_code,
                "orchestration"
            );
            assert_eq!(adjusted.billing_owner_id, "person");
        }
    }
}
