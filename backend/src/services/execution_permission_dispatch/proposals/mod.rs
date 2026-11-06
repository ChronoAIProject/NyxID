//! Assistant-only, human-authored permission requests. No legacy prompt is authority here.
use super::*;
use crate::models::{
    approval_request::{ApprovalRequest, COLLECTION_NAME as REQUESTS},
    assistant_conversation::AssistantConversation,
    execution_proposal::{Choice, ExecutionProposal, PreparedEffect, Proposal, Review},
};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

mod lifecycle;
mod resume;
pub(crate) mod v2;
pub(crate) use lifecycle::{cancel_in_session, sweep};
pub(crate) use resume::{Resume, result_for_human, resume_turn};
pub(crate) const READINESS: &str = "permissions:execution-authoring-v1";
pub(crate) const MAX_PAYLOAD: usize = 256 * 1024;

fn missing() -> AppError {
    AppError::NotFound("Permission request not found".into())
}
fn conflict() -> AppError {
    AppError::Conflict("Permission review is no longer current".into())
}
fn audit_key() -> AppResult<&'static [u8]> {
    super::super::audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))
}
pub(super) fn fingerprint(value: &impl Serialize) -> AppResult<String> {
    super::super::audit_service::keyed_fingerprint(
        Some(audit_key()?),
        b"execution-proposal-v1",
        &serde_json::to_vec(value).map_err(encoding)?,
    )
    .ok_or_else(refused)
}
fn label(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}
fn binary(bytes: Vec<u8>) -> bson::Binary {
    bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    }
}
fn aad(id: &str) -> String {
    format!("execution-proposal:v1:{id}")
}

pub(crate) fn supported_chat(chat: &acks::ChatAuthority) -> bool {
    chat.turn_live
        && !chat.turn_stopped
        && !chat.guest
        && !chat.execution_unsupported
        && !chat.execution_voice_task
        && !chat.platform_thread
        && chat.platform_origin.is_none()
        && chat.channel_adapter_origin.is_none()
        && chat.confirmation_policy.is_none()
        && chat.execution_task.is_some()
        && (chat.org_agent_access.is_none()
            || chat
                .org_agent_access
                .as_ref()
                .is_some_and(|a| a.shared().is_some())
            || chat.collaboration_thread)
}

/// All reusable uses must have this closed transport shape, including requests
/// arriving after authoring. Arbitrary headers and non-JSON bodies cannot be scoped.
pub(super) fn reusable_shape(
    method: &str,
    body: &[u8],
    content_type: Option<&str>,
    headers: &[(String, String)],
) -> bool {
    headers.iter().all(|(k, v)| {
        k.eq_ignore_ascii_case("content-type")
            && v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    }) && if matches!(method, "GET" | "HEAD" | "OPTIONS" | "DELETE") {
        body.is_empty()
    } else {
        content_type.is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        }) && serde_json::from_slice::<serde_json::Value>(body).is_ok()
    }
}

fn retain(effect: &Effect<'_>, context: &RequestContext) -> Result<PreparedEffect, &'static str> {
    if effect.target.target_id.is_some() || effect.target.service.slug == "llm-openai-codex" {
        return Err("destination_not_retainable");
    }
    // Replay enters the REST core with the original prepared path. Do not
    // promise recovery for paths requiring URI encoding or a second decoding
    // pass (typed MCP paths may already contain encoded parameters).
    if !effect.path.is_ascii()
        || effect
            .path
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b == b'%')
    {
        return Err("path_not_retainable");
    }
    if effect.body.len() > MAX_PAYLOAD {
        return Err("payload_too_large");
    }
    if !effect.body.is_empty()
        && (!effect.content_type.is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        }) || serde_json::from_slice::<serde_json::Value>(effect.body).is_err())
    {
        return Err("body_not_retainable");
    }
    if effect.headers.iter().any(|(k, _)| {
        matches!(
            k.to_ascii_lowercase().as_str(),
            "authorization" | "cookie" | "x-api-key" | "proxy-authorization"
        )
    }) {
        return Err("credential_header_not_retainable");
    }
    Ok(PreparedEffect {
        generated_correlation: context.generated_correlation.get().cloned(),
        connection: effect.connection.into(),
        catalog: effect.target.service.id.clone(),
        method: effect.method.into(),
        path: effect.path.into(),
        query: effect.query.map(str::to_owned),
        body: String::from_utf8(effect.body.to_vec()).map_err(|_| "body_not_retainable")?,
        headers: effect.headers.to_vec(),
        endpoint: effect.endpoint.map(str::to_owned),
    })
}
fn scope(
    effect: &Effect<'_>,
    reusable: bool,
) -> AppResult<crate::models::agent_operation_scope::AgentOperationScope> {
    use crate::models::agent_operation_scope::{AgentOperationScope, ScopedOperation};
    if effect.path.contains(['{', '}', '*']) {
        return Err(conflict());
    }
    let path = if effect.endpoint.is_some() {
        CanonicalPath::from_mcp_built(effect.path)?
    } else {
        CanonicalPath::from_rest_decoded(effect.path)?
    };
    let mut query = std::collections::BTreeMap::new();
    for (k, v) in url::form_urlencoded::parse(effect.query.unwrap_or("").as_bytes()) {
        if query
            .insert(
                k.to_string(),
                nyxid_permissions::google::QueryParameter {
                    required: true,
                    rule: nyxid_permissions::ValueRule::Exact { value: json!(v) },
                },
            )
            .is_some()
        {
            return Err(conflict());
        }
    }
    let body = if effect.body.is_empty() {
        None
    } else {
        Some(nyxid_permissions::google::JsonShape::Exact {
            value: serde_json::from_slice(effect.body).map_err(encoding)?,
        })
    };
    let inputs = reusable.then_some(nyxid_permissions::values::InputRules {
        path: Default::default(),
        query: Some(query),
        body,
    });
    if let Some(inputs) = &inputs {
        inputs
            .validate(effect.method, &Default::default())
            .map_err(|_| conflict())?;
        inputs
            .check(
                &Default::default(),
                effect.query,
                effect.body,
                effect.content_type,
            )
            .map_err(|_| conflict())?;
    }
    Ok(AgentOperationScope {
        revision: 1,
        catalog_service_id: Some(effect.target.service.id.clone()),
        operations: vec![ScopedOperation {
            endpoint_id: effect.endpoint.map(str::to_owned),
            rule: crate::models::downstream_service::ProxyOperationRule {
                method: effect.method.into(),
                path_template: path.as_policy_path(),
                ..Default::default()
            },
            inputs,
            ..Default::default()
        }],
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn create(
    db: &Database,
    keys: &Arc<EncryptionKeys>,
    context: &RequestContext,
    chat: &acks::ChatAuthority,
    effect: &Effect<'_>,
    subject: Subject,
    binding: ConnectionBinding,
    audience: Audience,
    digest: String,
    snapshot: &authority::Snapshot,
    gaps: &[contract::Gap],
) -> AppResult<Option<ApprovalRequest>> {
    if !supported_chat(chat) || effect.floors.is_empty() {
        return Ok(None);
    }
    if effect.floors.iter().any(|f| !contract::satisfiable(*f)) {
        return Err(refused());
    }
    let endpoint_digest = if let Some(id) = effect
        .endpoint
        .filter(|id| *id != super::super::mcp_service::GENERIC_PROXY_ENDPOINT_ID)
    {
        let Some(row) = db
            .collection::<Document>("service_endpoints")
            .find_one(doc! {"_id":id,"service_id":&effect.target.service.id})
            .await?
        else {
            tracing::info!(
                reason = "endpoint_not_recoverable",
                "Execution proposal uses legacy prompts"
            );
            return Ok(None);
        };
        Some(endpoint_digest(&row)?)
    } else {
        None
    };
    let retained = match retain(effect, context) {
        Ok(v) => v,
        Err(reason) => {
            tracing::info!(reason, "Execution proposal uses legacy prompts");
            return Ok(None);
        }
    };
    let encoded = Zeroizing::new(serde_json::to_vec(&retained).map_err(encoding)?);
    // Reserve room for the envelope's wrapped key/nonce/tag as well as plaintext.
    if encoded.len() + 2048 > MAX_PAYLOAD {
        tracing::info!(
            reason = "payload_too_large",
            "Execution proposal uses legacy prompts"
        );
        return Ok(None);
    }
    super::super::security_readiness_service::require(db, READINESS).await?;
    let task = chat.execution_task.as_ref().ok_or_else(refused)?.clone();
    if task.actor_user_id != chat.user_id || task.audience != audience {
        return Err(refused());
    }
    let now = Utc::now();
    let end = (now + Duration::minutes(10)).min(task.hard_deadline);
    let id = Uuid::new_v4().to_string();
    let encrypted = keys.encrypt_with_aad(&encoded, aad(&id).as_bytes()).await?;
    if encrypted.len() > MAX_PAYLOAD {
        tracing::info!(
            reason = "encrypted_payload_too_large",
            "Execution proposal uses legacy prompts"
        );
        return Ok(None);
    }
    let mut reusable = reusable_shape(
        effect.method,
        effect.body,
        effect.content_type,
        &scope_headers(
            effect.headers,
            context.generated_correlation.get().map(String::as_str),
        ),
    ) && !sensitive_inputs(effect);
    let compiled = scope(effect, reusable).or_else(|error| {
        if reusable {
            reusable = false;
            scope(effect, false)
        } else {
            Err(error)
        }
    });
    let operations = match compiled {
        Ok(v) => v,
        Err(_) => {
            tracing::info!(
                reason = "inputs_not_representable",
                "Execution proposal uses legacy prompts"
            );
            return Ok(None);
        }
    };
    let contract = ExecutionContract {
        subject,
        binding,
        context: ExecutionContext {
            audience,
            conversation: ConversationScope::Exact {
                conversation_id: chat.conversation_id.clone(),
            },
        },
        operations,
        resource_policy: None,
        validity: Validity::Operation {
            operation_id: context.operation_id.clone(),
            prepared_digest: digest.clone(),
            expires_at: end,
        },
        satisfies: effect.floors.to_vec(),
        revision: 1,
    };
    let authority_digest = snapshot.digest();
    let gap_key = fingerprint(
        &json!({"subject":contract.subject,"binding":contract.binding,"context":contract.context,"task":task.task,"epoch":task.epoch,"deadline":task.hard_deadline,"key":chat.api_key_id,"occurrence_turn":chat.turn_id,"digest":digest,"authority":authority_digest,"floors":effect.floors,"compiler":1,"report_to":chat.report_to}),
    )?;

    let mut choices = vec![("operation", "This operation", contract.validity.clone())];
    if reusable {
        choices.extend([
            (
                "task",
                "This task",
                Validity::Task {
                    task: task.task.clone(),
                    hard_deadline: task.hard_deadline,
                },
            ),
            (
                "window",
                "For one hour",
                Validity::Window {
                    starts_at: now,
                    ends_at: now + Duration::hours(1),
                },
            ),
            (
                "persistent",
                "Until revoked or this account binding changes",
                Validity::Persistent,
            ),
        ]);
    }
    let choices = choices
        .into_iter()
        .map(|(id, label, validity)| {
            Ok(Choice {
                id: id.into(),
                label: label.into(),
                digest: fingerprint(&json!({"scope":contract,"choice":validity}))?,
                validity,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let p = Proposal {
        revision: 1,
        gap_key: gap_key.clone(),
        effect_id: context.operation_id.clone(),
        prepared_digest: digest,
        authority_digest,
        key_id: chat.api_key_id.clone(),
        conversation_id: chat.conversation_id.clone(),
        source_turn_id: chat.turn_id.clone().ok_or_else(refused)?,
        source_credential_id: context.source_credential.get().cloned(),
        endpoint_digest,
        selected_choice_digest: None,
        notification_claimed_at: None,
        wake_at: None,
        maintenance_at: None,
        delivery_abandoned: false,
        independent_request_id: None,
        independent_card_id: None,
        delivered_turn_id: None,
        task,
        report_to: chat.report_to.clone(),
        invocation_id: chat.invocation_id.clone(),
        contract,
        choices,
        review: review(
            db,
            chat,
            effect,
            reusable,
            context.candidates(db, effect.connection).await?,
            gaps,
        )
        .await?,
        payload: Some(binary(encrypted)),
        state: "pending".into(),
        grant_id: None,
        selected_choice: None,
        claim_id: None,
        resume_turn_id: None,
        claimed_at: None,
        result: None,
        failure_code: None,
    };
    let group_binding = super::super::org_group_service::approval_binding(
        db,
        chat.execution_group_id.as_deref(),
        &chat.user_id,
        Some(&chat.api_key_id),
    )
    .await?;
    let request = ApprovalRequest {
        execution_proposal: Some(ExecutionProposal {
            v2: None,
            version: VERSION,
            v1: Some(p),
        }),
        assistant_group: group_binding,
        id,
        user_id: chat.user_id.clone(),
        service_id: effect.target.service.id.clone(),
        service_name: label(&effect.target.service.name),
        service_slug: label(&effect.target.service.slug),
        requester_type: "user".into(),
        requester_id: chat.user_id.clone(),
        requester_label: Some(label(&chat.agent_name)),
        operation_summary: format!("{} {}", effect.method, label(effect.path)),
        action_description: Some("Review an execution permission for this task".into()),
        http_method: Some(effect.method.into()),
        resource: Some(label(effect.path)),
        verb: None,
        grant_scope: Some(LEGACY_SCOPE.into()),
        tool_name: None,
        tool_call_id: None,
        tool_arguments: None,
        is_destructive: None,
        approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
        status: "pending".into(),
        idempotency_key: format!("execution:v1:{gap_key}"),
        notification_channel: None,
        telegram_message_id: None,
        telegram_chat_id: None,
        expires_at: end,
        decided_at: None,
        decision_channel: None,
        decision_idempotency_key: None,
        notify_user_ids: vec![chat.user_id.clone()],
        from_org_policy: false,
        exact_service: None,
        created_at: now,
    };
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db,chat,&request,snapshot),|session,(db,chat,request,snapshot)|Box::pin(async move {
        let result:AppResult<ApprovalRequest>=async {
            require_enrolled(db,&chat.user_id,Some(session)).await?;
            let p=proposal(request)?;
            tasks::fence(db,session,&p.task).await?;
            let (row,_,_)=Box::pin(acks::fence(db,chat,session)).await?;
            super::super::assistant_access_policy::require_same_live_turn(chat,&row)?;
            snapshot.fence(db,session).await?;
            fence_endpoint(db,session,p).await?;
            let collection=db.collection::<ApprovalRequest>(REQUESTS);
            if let Some(existing)=collection.find_one(doc!{"execution_proposal.v1.gap_key":&p.gap_key}).sort(doc!{"created_at":-1}).session(&mut *session).await? {
                // Denial is task-bound. A pending or dispatched effect is never queued twice.
                return Ok(existing);
            }
            if collection.count_documents(doc!{"user_id":&chat.user_id,"execution_proposal.v1.task.task":bson::to_bson(&p.task.task).map_err(encoding)?}).session(&mut *session).await?>=32 {return Err(AppError::Forbidden("Too many permission requests for this task".into()));}
            collection.insert_one(*request).session(&mut *session).await?;
            audit(db,session,&chat.user_id,"execution_permission_requested",&request.id).await?;
            Ok((*request).clone())
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    })).await.map_err(super::super::api_key_mutation_service::map_transaction_error).map(Some)
}

pub(crate) fn proposal(row: &ApprovalRequest) -> AppResult<&Proposal> {
    row.execution_proposal
        .as_ref()
        .filter(|p| p.version == VERSION)
        .and_then(|p| p.v1.as_ref())
        .ok_or_else(conflict)
}
async fn audit(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    event: &str,
    id: &str,
) -> AppResult<()> {
    super::super::audit_service::log_actor_event_in_session(
        db,
        session,
        audit_key()?,
        &super::super::audit_service::AuditActor {
            user_id: actor.into(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        },
        event,
        json!({"request_id":id}),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub approved: bool,
    pub expected_revision: i64,
    pub choice_id: Option<String>,
    pub choice_digest: Option<String>,
    pub idempotency_key: String,
}

/// Human authentication is required by the handler. Live actor/source fences are
/// also applied here, inside the same transaction as the permission and intent.
pub(crate) async fn decide(
    state: &crate::AppState,
    actor: &str,
    id: &str,
    input: &Decision,
) -> AppResult<ApprovalRequest> {
    if input.idempotency_key.is_empty() || input.idempotency_key.len() > 128 {
        return Err(AppError::ValidationError(
            "A bounded decision idempotency key is required".into(),
        ));
    }
    super::super::security_readiness_service::require(&state.db, READINESS).await?;
    super::super::security_readiness_service::require(&state.db, super::READINESS).await?;
    super::super::security_readiness_service::require(
        &state.db,
        super::super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
    )
    .await?;
    let row = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":id,"user_id":actor,"execution_proposal.version":{"$in":[1,2]}})
        .await?
        .ok_or_else(missing)?;
    if row
        .execution_proposal
        .as_ref()
        .is_some_and(|p| p.version == 2)
    {
        return v2::decide(state, actor, id, input).await;
    }
    let p = proposal(&row)?;
    if row.status != "pending" {
        return replay(&row, input);
    }
    if let Err(error) = tasks::check(&state.db, &p.task).await {
        if error.is_forbidden() {
            expire_pending(&state.db, actor, id, "task_unavailable").await?;
        }
        return Err(error);
    }
    let snapshot = match authority::Snapshot::load(
        &state.db,
        &p.contract.binding.connection_id,
        &p.key_id,
        p.source_credential_id.as_deref(),
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if error.is_forbidden() {
                expire_pending(&state.db, actor, id, "authority_unavailable").await?;
            }
            return Err(error);
        }
    };
    if snapshot.digest() != p.authority_digest {
        expire_pending(&state.db, actor, id, "authority_changed").await?;
        return Err(conflict());
    }
    let grant_id = Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((&state.db,actor,id,input,&snapshot,&grant_id),|session,(db,actor,id,input,snapshot,grant_id)|Box::pin(async move {
        let result:AppResult<ApprovalRequest>=async {
            require_enrolled(db,actor,Some(session)).await?;
            let collection=db.collection::<ApprovalRequest>(REQUESTS);
            let mut row=collection.find_one(doc!{"_id":*id,"user_id":*actor}).session(&mut *session).await?.ok_or_else(missing)?;
            let p=proposal(&row)?.clone();
            if row.status!="pending" {
                return replay(&row,input);
            }
            if p.revision!=input.expected_revision||row.expires_at<=Utc::now(){return Err(conflict());}
            tasks::fence(db,session,&p.task).await?;
            snapshot.fence(db,session).await?;
            let conversations=db.collection::<AssistantConversation>("assistant_conversations");
            let source=conversations.find_one(doc!{"_id":&p.conversation_id,"user_id":*actor,"credential_api_key_id":&p.key_id}).session(&mut *session).await?.ok_or_else(missing)?;
            super::super::assistant_agent_credential_service::authority_in_session(db,&source,session).await?;
            fence_behavior(db,session,&source,&p).await?;
            fence_endpoint(db,session,&p).await?;
            if source.collaboration.is_some(){super::super::collaboration_group_service::runtime::fence_decision(db,&source,actor,session).await?;}
            let current_audience=if let Some(group)=source.collaboration.as_ref().map(|b|&b.group_id).or(source.group_id.as_ref()) {
                let group=db.collection::<crate::models::assistant_group::AssistantGroup>("assistant_groups").find_one(doc!{"_id":group}).session(&mut *session).await?.ok_or_else(missing)?;
                group_audience(&group)
            } else {Audience::Private{user_id:actor.to_string()}};
            if current_audience!=p.contract.context.audience{return Err(conflict());}
            let now=Utc::now();
            let mut updated=p.clone();
            if input.approved {
                let choice=p.choices.iter().find(|c|Some(&c.id)==input.choice_id.as_ref()&&Some(&c.digest)==input.choice_digest.as_ref()).ok_or_else(conflict)?;
                let mut contract=p.contract.clone();contract.validity=choice.validity.clone();
                let grant=ApprovalGrant{execution_use:None,execution_review:Some(p.review.clone()),execution_last_used_at:None,execution:Some(ExecutionPermission{version:VERSION,v1:Some(contract.clone())}),id:(*grant_id).clone(),user_id:actor.to_string(),service_id:row.service_id.clone(),service_name:row.service_name.clone(),requester_type:row.requester_type.clone(),requester_id:row.requester_id.clone(),requester_label:row.requester_label.clone(),approval_request_id:row.id.clone(),scope:Some(LEGACY_SCOPE.into()),granted_at:now,expires_at:contract::legacy_expiry(&contract.validity),revoked:false,org_scoped:false};
                db.collection::<ApprovalGrant>("approval_grants").insert_one(grant).session(&mut *session).await?;
                updated.grant_id=Some((*grant_id).clone());updated.selected_choice=input.choice_id.clone();updated.selected_choice_digest=input.choice_digest.clone();updated.state="queued".into();updated.wake_at=Some(now);
            }else{updated.state="denied".into();updated.payload=None;}
            row.status=if input.approved{"approved"}else{"rejected"}.into();row.decided_at=Some(now);row.decision_channel=Some("web".into());row.decision_idempotency_key=Some(input.idempotency_key.clone());row.execution_proposal=Some(ExecutionProposal { v2: None, version:VERSION,v1:Some(updated)});
            if collection.replace_one(doc!{"_id":&row.id,"status":"pending","execution_proposal.v1.revision":p.revision},&row).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
            let mut event=super::super::assistant_team_service::event("permission_decided",if input.approved{"An execution permission was saved. NyxID will execute the retained operation and report its result; do not reconstruct or repeat it."}else{"The person denied the execution permission. Nothing was sent; do not repeat this request."}.into(),None).bound_to(p.report_to.as_deref()).for_invocation(p.invocation_id.as_deref());
            event.execution_task=Some(p.task.clone());event.id=format!("execution-decision:{}",row.id);
            let entry=bson::to_bson(&event).map_err(encoding)?;
            if conversations.update_one(doc!{"_id":&p.conversation_id,"user_id":*actor,"credential_api_key_id":&p.key_id,"pending_events.63":{"$exists":false}},doc!{"$push":{"pending_events":entry},"$inc":{"execution_dispatch_fence":1_i64}}).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
            if source.collaboration.is_some(){super::super::collaboration_group_service::runtime::queue_decision(db,&source,session).await?;}
            audit(db,session,actor,if input.approved{"execution_permission_saved"}else{"execution_permission_denied"},id).await?;
            Ok(row)
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    })).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}

/// Dedicated safe DTO, never serialize the persisted envelope.
#[derive(Clone, Debug, Serialize)]
pub struct ProposalView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub version: u32,
    pub supported: bool,
    pub revision: i64,
    pub state: String,
    pub can_decide: bool,
    pub review_path: String,
    pub review: Option<Review>,
    pub choices: Vec<ChoiceView>,
    pub grant_id: Option<String>,
    pub failure_code: Option<String>,
    pub result_text: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ChoiceView {
    pub id: String,
    pub digest: String,
    pub label: String,
    pub valid_until: Option<String>,
}
pub(crate) fn view(row: &ApprovalRequest) -> Option<ProposalView> {
    let envelope = row.execution_proposal.as_ref()?;
    if envelope.version == 2 {
        return Some(v2::view(row));
    }
    let p = proposal(row).ok();
    Some(ProposalView {
        source: None,
        result_text: None,
        version: envelope.version,
        supported: p.is_some(),
        revision: p.map_or(0, |p| p.revision),
        state: if row.status == "pending" && row.expires_at <= Utc::now() {
            "expired".into()
        } else {
            p.map_or_else(|| "unsupported".into(), |p| p.state.clone())
        },
        can_decide: p.is_some() && row.status == "pending" && row.expires_at > Utc::now(),
        review_path: p.map_or_else(
            || super::super::assistant_links::AssistantPage::Approvals.path(),
            |p| match &p.contract.context.audience {
                Audience::Group { group_id, .. } => {
                    super::super::assistant_links::AssistantPage::Group { group: group_id }.path()
                }
                _ => super::super::assistant_links::AssistantPage::ReviewCard {
                    conversation: &p.conversation_id,
                    card: &row.id,
                }
                .path(),
            },
        ),
        review: p.map(|p| p.review.clone()),
        choices: p.map_or_else(Vec::new, |p| {
            p.choices
                .iter()
                .map(|c| ChoiceView {
                    id: c.id.clone(),
                    digest: c.digest.clone(),
                    label: c.label.clone(),
                    valid_until: (!matches!(c.validity, Validity::Persistent))
                        .then(|| contract::legacy_expiry(&c.validity).to_rfc3339()),
                })
                .collect()
        }),
        grant_id: p.and_then(|p| p.grant_id.clone()),
        failure_code: p.and_then(|p| p.failure_code.clone()),
    })
}

pub(crate) fn legacy_refusal(row: &ApprovalRequest) -> AppResult<()> {
    if row.execution_proposal.is_some() {
        Err(AppError::Conflict(
            "This permission request requires its human review page".into(),
        ))
    } else {
        Ok(())
    }
}

fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "secret",
        "token",
        "authorization",
        "cookie",
        "api_key",
        "apikey",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|part| key.contains(part))
}
fn sensitive_json(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| secret_key(key) || sensitive_json(value)),
        serde_json::Value::Array(values) => values.iter().any(sensitive_json),
        _ => false,
    }
}
fn sensitive_inputs(effect: &Effect<'_>) -> bool {
    effect.body.len() > 8192
        || effect.query.is_some_and(|q| {
            q.len() > 2048
                || url::form_urlencoded::parse(q.as_bytes()).any(|(key, _)| secret_key(&key))
        })
        || serde_json::from_slice::<serde_json::Value>(effect.body)
            .is_ok_and(|v| sensitive_json(&v))
}
fn redact(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if secret_key(key) {
                    *value = json!("[redacted]");
                } else {
                    redact(value);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact),
        _ => {}
    }
}
async fn review(
    db: &Database,
    chat: &acks::ChatAuthority,
    effect: &Effect<'_>,
    reusable: bool,
    grants: &[ApprovalGrant],
    gaps: &[contract::Gap],
) -> AppResult<Review> {
    let connection = db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"_id":effect.connection,"user_id":effect.owner})
        .await?
        .ok_or_else(refused)?;
    let query = url::form_urlencoded::parse(effect.query.unwrap_or("").as_bytes())
        .map(|(k, v)| {
            (
                k.to_string(),
                if secret_key(&k) {
                    "[redacted]".to_owned()
                } else {
                    v.to_string()
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut body =
        serde_json::from_slice::<serde_json::Value>(effect.body).unwrap_or(serde_json::Value::Null);
    redact(&mut body);
    let inputs=serde_json::to_string(&json!({"query":query,"body":body,"headers":effect.headers.iter().map(|(k,v)|(k,if secret_key(k){"[redacted]"}else{v.as_str()})).collect::<std::collections::BTreeMap<_,_>>()})).map_err(encoding)?;
    Ok(Review {
        agent:label(&chat.agent_name),
        connection:format!("{} · {} · {}",label(&connection.slug),if effect.owner==chat.user_id{"your connection"}else{"shared connection"},effect.connection.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>()),
        operation:format!("{} {}",effect.method,label(effect.path)),
        audience:if chat.execution_group_id.is_some(){"This group and its current participants".into()}else{"This private conversation".into()},
        scope:if reusable{"Only this exact path, query and JSON input in this conversation"}else{"Only this retained operation, including its exact inputs and headers"}.into(),
        inputs:inputs.chars().filter(|c| !c.is_control()).take(12_000).collect(),
        existing_permission:existing_permissions(grants,gaps),
        added_permission:effect.floors.iter().map(|f|match f {
            ExecutionFloor::AccessModeRisk=>"access-mode review",
            ExecutionFloor::PersonalServiceApproval=>"personal service approval",
            ExecutionFloor::SharedConnectionConsent=>"your consent to this shared agent using this exact connection",
            _=>"A separate required review (not granted)",
        }).collect::<Vec<_>>().join("; "),
        reusable_unavailable:(!reusable).then(||"Only this operation is available: the headers, sensitive values or input size cannot be represented as a reusable permission".into()),
    })
}
fn replay(row: &ApprovalRequest, input: &Decision) -> AppResult<ApprovalRequest> {
    let p = proposal(row)?;
    if row.decision_idempotency_key.as_deref() == Some(input.idempotency_key.as_str())
        && p.revision == input.expected_revision
        && p.selected_choice == input.choice_id
        && p.selected_choice_digest == input.choice_digest
        && (row.status == "approved") == input.approved
    {
        Ok(row.clone())
    } else {
        Err(conflict())
    }
}

pub(super) fn approve_url(frontend_url: &str) -> String {
    format!(
        "{}{}",
        frontend_url.trim_end_matches('/'),
        super::super::assistant_links::AssistantPage::Approvals.path()
    )
}

/// Matching model retries are receipt lookups, including under a reusable grant.
/// Pending/denied/uncertain effects remain task-bound. A completed receipt can
/// authorize a new occurrence only in a different server-admitted turn after
/// settlement delivered that receipt; retries of either original/resume turn join it.
pub(super) async fn existing_effect(
    db: &Database,
    chat: &acks::ChatAuthority,
    connection: &str,
    digest: &str,
    frontend_url: &str,
) -> AppResult<()> {
    let task = chat.execution_task.as_ref().ok_or_else(refused)?;
    if let Some(row)=db.collection::<ApprovalRequest>(REQUESTS).find_one(doc!{
        "user_id":&chat.user_id,"execution_proposal.v1.key_id":&chat.api_key_id,
        "execution_proposal.v1.conversation_id":&chat.conversation_id,"execution_proposal.v1.task.task":bson::to_bson(&task.task).map_err(encoding)?,
        "execution_proposal.v1.task.epoch":task.epoch,"execution_proposal.v1.contract.binding.connection_id":connection,
        "execution_proposal.v1.prepared_digest":digest,
        "$or":[{"execution_proposal.v1.state":{"$ne":"completed"}},
            {"execution_proposal.v1.delivered_turn_id":null},
            {"execution_proposal.v1.source_turn_id":&chat.turn_id},
            {"execution_proposal.v1.resume_turn_id":&chat.turn_id}]
    }).await? {
        return Err(if row.status=="pending"&&row.expires_at>Utc::now(){AppError::ApprovalRequired{request_id:row.id}}else{
            AppError::ApprovalFailed{request_id:row.id,reason:"This task already has a decision for this operation. Its retained effect will not be repeated; review its status.".into(),approve_url:approve_url(frontend_url)}
        });
    }
    Ok(())
}
async fn fence_behavior(
    db: &Database,
    session: &mut ClientSession,
    source: &AssistantConversation,
    p: &Proposal,
) -> AppResult<()> {
    if let Some(expected) = &p.contract.binding.behavior_digest {
        let agent_id = source.agent_id.as_ref().ok_or_else(refused)?;
        let agent = db
            .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
            .find_one(doc! {"_id":agent_id,"destroyed_at":null})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        let policy = db
            .collection::<crate::models::assistant_sharing::AgentSharing>(
                crate::models::assistant_sharing::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":agent_id,"enabled":true})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        if super::super::shared_agent_consent::behavior_for_agent(&agent, policy.revision)
            != *expected
        {
            return Err(conflict());
        }
        if db
            .collection::<Document>("assistant_agents")
            .update_one(
                doc! {"_id":agent_id},
                doc! {"$inc":{"execution_dispatch_fence":1_i64}},
            )
            .session(session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
    }
    Ok(())
}

fn endpoint_digest(row: &Document) -> AppResult<String> {
    let mut row = row.clone();
    for field in ["updated_at", "created_at", "execution_dispatch_fence"] {
        row.remove(field);
    }
    fingerprint(&row)
}
async fn fence_endpoint(db: &Database, session: &mut ClientSession, p: &Proposal) -> AppResult<()> {
    if let Some(expected) = &p.endpoint_digest {
        let id = p
            .contract
            .operations
            .operations
            .first()
            .and_then(|op| op.endpoint_id.as_ref())
            .ok_or_else(refused)?;
        let row = db
            .collection::<Document>("service_endpoints")
            .find_one(doc! {"_id":id})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        if endpoint_digest(&row)? != *expected {
            return Err(refused());
        }
        if db
            .collection::<Document>("service_endpoints")
            .update_one(
                doc! {"_id":id},
                doc! {"$inc":{"execution_dispatch_fence":1_i64}},
            )
            .session(session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
    }
    Ok(())
}

fn existing_permissions(grants: &[ApprovalGrant], gaps: &[contract::Gap]) -> String {
    let scopes = grants
        .iter()
        .take(8)
        .map(|row| {
            let Some(c) = row
                .execution
                .as_ref()
                .filter(|e| e.version == VERSION)
                .and_then(|e| e.v1.as_ref())
            else {
                return "Unsupported permission version".into();
            };
            let lifetime = match &c.validity {
                Validity::Operation { .. } => "one operation".into(),
                Validity::Task { .. } => "one task".into(),
                Validity::Window { ends_at, .. } => format!("until {}", ends_at.to_rfc3339()),
                Validity::Persistent => "until revoked or binding changes".into(),
            };
            let paths = c
                .operations
                .operations
                .iter()
                .take(4)
                .map(|op| {
                    format!(
                        "{} {}",
                        label(&op.rule.method),
                        label(&op.rule.path_template)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}{} ({lifetime})",
                if row.revoked { "revoked: " } else { "" },
                paths
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let reasons = gaps
        .iter()
        .map(|gap| match gap {
            contract::Gap::MissingPermission => "no saved permission",
            contract::Gap::Subject => "different actor or principal",
            contract::Gap::Binding => "account, credential or behavior binding changed",
            contract::Gap::Context => "different conversation or result audience",
            contract::Gap::Operation => {
                "operation, resource, inputs or request shape outside scope"
            }
            contract::Gap::Validity => "different operation/task or elapsed validity",
            contract::Gap::Floor(_) => "an additional personal decision is required",
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{} Missing coverage: {reasons}. Saved permissions are not enlarged; this decision adds the exact displayed scope.",if grants.is_empty(){"No saved execution permission for this agent and account.".into()}else{format!("Saved permissions ({} total): {scopes}.",grants.len())}).chars().take(4096).collect()
}

async fn expire_pending(db: &Database, actor: &str, id: &str, reason: &str) -> AppResult<()> {
    db.collection::<Document>(REQUESTS).update_one(doc!{"_id":id,"user_id":actor,"status":"pending","execution_proposal.version":VERSION as i32,"execution_proposal.v1.state":"pending"},doc!{"$set":{"status":"expired","execution_proposal.v1.state":"expired","execution_proposal.v1.failure_code":reason},"$unset":{"execution_proposal.v1.payload":""}}).await?;
    Ok(())
}

/// Exclude only a correlation ID generated by this server request. The retained
/// payload and send-time header comparison still preserve its exact bytes.
pub(super) fn scope_headers(
    headers: &[(String, String)],
    generated: Option<&str>,
) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, value)| {
            !(name.eq_ignore_ascii_case("x-request-id") && generated == Some(value.as_str()))
        })
        .cloned()
        .collect()
}
