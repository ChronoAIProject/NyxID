//! Guest service approvals never confer reusable or owner authority.
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::Database;

use super::{
    api_key_mutation_service as transactions, approval_service as approvals,
    assistant_acknowledgement_service::ChatAuthority, assistant_team_service as team,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        approval_request::{ApprovalRequest, GuestApprovalContext},
        assistant_agent::AssistantAgent,
        assistant_conversation::AssistantConversation,
        nyxbot_channel::{NyxbotEvent, NyxbotThread},
    },
};

/// Counts all admitted escalations, including rejected and expired requests.
pub const HOURLY_CAP: usize = 20;
use crate::models::assistant_guest_escalation_budget::{
    AdmissionTime, AssistantGuestEscalationBudget as Budget, COLLECTION_NAME as BUDGETS,
    PendingApproval,
};

pub fn display_text(value: &str) -> String {
    value
        .chars()
        .take(160)
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '.' | ',') {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}

pub fn display_label(guest: &GuestApprovalContext) -> String {
    format!(
        "Guest request: {} via {} in {} — specialist {}",
        guest.sender_name, guest.platform, guest.chat_title, guest.agent_name
    )
}

pub async fn require_decider(
    db: &Database,
    request: &ApprovalRequest,
    actor: &str,
) -> AppResult<()> {
    let guest = request
        .guest
        .as_ref()
        .ok_or_else(|| AppError::Forbidden("Guest request required".into()))?;
    let agent = team::maintained_agent(db, actor, &guest.agent_id).await?;
    if agent.user_id != guest.agent_owner_id {
        return Err(AppError::Forbidden(
            "Guest approval authority changed".into(),
        ));
    }
    if request.from_org_policy {
        let access = super::org_service::resolve_owner_access(db, actor, &request.user_id).await?;
        let mut scoped = access.allows_resource(&request.service_id);
        if !scoped {
            scoped = super::user_service_service::user_service_ids_for_catalog(
                db,
                &request.user_id,
                &request.service_id,
            )
            .await?
            .iter()
            .any(|id| access.allows_resource(id));
        }
        if !access.can_write() || !scoped {
            return Err(AppError::Forbidden(
                "Organization approval authority required".into(),
            ));
        }
    }
    Ok(())
}

/// Atomic across replicas; leases expire with the suspended call's approval.
pub async fn reserve(
    db: &Database,
    agent: &str,
    thread: &str,
    timeout: u32,
) -> AppResult<(String, chrono::DateTime<Utc>)> {
    let token = uuid::Uuid::new_v4().to_string();
    let collection = db.collection::<Budget>(BUDGETS);
    if let Err(error) = collection
        .update_one(
            doc! {"_id": agent},
            doc! {"$setOnInsert": {"attempts": [], "pending": []}},
        )
        .upsert(true)
        .await
        && !team::is_duplicate(&error)
    {
        return Err(error.into());
    }
    let database = db.clone();
    let agent = agent.to_owned();
    let thread = thread.to_owned();
    let admission = token.clone();
    let mut session = db.client().start_session().await?;
    let deadline = session
        .start_transaction()
        .and_run2(async move |session| {
            let result = async {
                let now = Utc::now();
                let collection = database.collection::<Budget>(BUDGETS);
                let row = collection
                    .find_one(doc! {"_id": &agent})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::Internal("Guest budget unavailable".into()))?;
                let mut attempts = row.attempts;
                attempts.retain(|at| at.0 > now - Duration::hours(1));
                let mut pending = row.pending;
                pending.retain(|lease| lease.expires_at > now);
                if attempts.len() >= HOURLY_CAP
                    || pending.iter().any(|lease| lease.thread == thread)
                {
                    return Err(AppError::Forbidden("guest_escalation_limited".into()));
                }
                attempts.push(AdmissionTime(now));
                let deadline = bson::DateTime::from_chrono(now + Duration::seconds(i64::from(timeout)));
                pending.push(PendingApproval { thread: thread.clone(), token: admission.clone(), expires_at: deadline.to_chrono() });
                collection
                    .update_one(
                        doc! {"_id": &agent},
                        doc! {"$set": {"attempts": bson::to_bson(&attempts).map_err(|_| AppError::Internal("Guest budget encoding failed".into()))?, "pending": bson::to_bson(&pending).map_err(|_| AppError::Internal("Guest budget encoding failed".into()))?}},
                    )
                    .session(&mut *session)
                    .await?;
                Ok(deadline.to_chrono())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    Ok((token, deadline))
}

pub async fn release(db: &Database, agent: &str, token: &str) -> AppResult<()> {
    db.collection::<Budget>(BUDGETS)
        .update_one(
            doc! {"_id": agent},
            doc! {"$pull": {"pending": {"token": token}}},
        )
        .await?;
    Ok(())
}

pub struct Call<'a> {
    pub service: &'a super::mcp_service::McpToolService,
    pub endpoint: &'a super::mcp_service::McpToolEndpoint,
    pub prepared: &'a super::mcp_service::PreparedProxyCall,
    pub target: &'a super::mcp_approval::McpApprovalTarget,
    pub approval_owner_user_id: &'a str,
    pub requester_type: Option<&'a str>,
    pub requester_id: &'a str,
    pub is_session: bool,
}

pub(crate) fn authorize<'a>(
    state: &'a AppState,
    chat: &'a ChatAuthority,
    call: Call<'a>,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        use crate::models::{
            assistant_agent::GuestEscalation, service_approval_config::ApprovalMode,
        };
        let db = &state.db;
        let agent = team::agent(db, &chat.user_id, &chat.agent_id).await?;
        if !chat.guest_policy_enabled()
            || !agent.guest_policy.enabled
            || agent.is_nyxbot()
            || matches!(
                call.service.source,
                super::mcp_service::McpToolSource::Internal
            )
            || call.prepared.carries_method_override()
            || (!agent.grants.service_ids.contains(&call.service.service_id)
                && !agent
                    .grants
                    .platform_service_ids
                    .contains(&call.service.service_id))
        {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        let metadata = call
            .service
            .durable_endpoint_metadata
            .get(&call.endpoint.endpoint_id)
            .copied()
            .unwrap_or_default();
        let effects = super::mcp_service::operation_effects(call.prepared.method(), metadata);
        let beyond_level = match agent
            .guest_level_for(&call.service.service_id, call.service.source.catalog_id())
        {
            crate::models::assistant_agent::GuestAccess::Read => !(effects.reads && effects.uses),
            crate::models::assistant_agent::GuestAccess::Use => !effects.uses,
            crate::models::assistant_agent::GuestAccess::All => false,
        };
        let descriptor = call.prepared.operation_descriptor();
        if descriptor.protocol == super::operation_descriptor::Protocol::Ssh {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        let (outcome, _) = approvals::evaluate_for_chat(
            db,
            call.approval_owner_user_id,
            &call.target.service_owner_user_id,
            &call.target.service_id,
            &descriptor,
            call.requester_type,
            call.requester_id,
            call.is_session,
            call.target.is_auto_connected,
            Some(chat),
        )
        .await?;
        if matches!(outcome, approvals::ApprovalOutcome::Denied) {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        if !beyond_level
            && matches!(
                outcome,
                approvals::ApprovalOutcome::Allowed { required: false }
            )
        {
            return Ok(());
        }
        if agent.guest_policy.escalation != GuestEscalation::AskOwner {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        let resolution = approvals::resolve_org_aware_approval(
            db,
            call.approval_owner_user_id,
            &call.target.service_owner_user_id,
            &call.target.service_id,
            &descriptor,
            call.target.is_auto_connected,
        )
        .await?;
        let row = live_thread(db, chat).await?;
        let initial_key =
            super::key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
        let service_allowed = initial_key.allow_all_services
            || match &call.service.source {
                super::mcp_service::McpToolSource::UserManaged { .. } => initial_key
                    .allowed_service_ids
                    .contains(&call.service.service_id),
                super::mcp_service::McpToolSource::Platform { .. } => initial_key
                    .allowed_platform_service_ids
                    .contains(&call.service.service_id),
                super::mcp_service::McpToolSource::Internal => false,
            };
        if !service_allowed {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        let authority = authority_digest(state, chat, &call).await?;
        let mut context = context(db, chat, &agent, &row).await?;
        context.endpoint_id.clone_from(&call.endpoint.endpoint_id);
        context.operation_digest = call
            .prepared
            .guest_digest(&call.service.service_id, call.endpoint);
        let mut operation = approvals::ApprovalRequestOperation::from_descriptor(&descriptor, None);
        operation.action_description = Some(descriptor.operation_summary());
        operation.guest = Some(context.clone());
        let candidates = if resolution.from_org_policy {
            super::org_service::list_admin_user_ids(db, &resolution.primary_owner_user_id).await?
        } else if agent.user_id == chat.user_id {
            vec![chat.user_id.clone()]
        } else {
            db.collection::<crate::models::org_membership::OrgMembership>(crate::models::org_membership::COLLECTION_NAME)
            .find(doc! {"org_user_id": &agent.user_id, "revoked_at": bson::Bson::Null, "role": {"$in": ["admin", "member"]}})
            .await?.try_collect::<Vec<_>>().await?.into_iter().map(|m| m.member_user_id).collect()
        };
        let mut recipients = Vec::new();
        for person in candidates {
            if team::maintained_agent(db, &person, &agent.id).await.is_ok() {
                recipients.push(person);
            }
        }
        let recipient = recipients
            .first()
            .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
        let timeout = super::notification_service::get_or_create_channel(db, recipient)
            .await?
            .approval_timeout_secs;
        let (admission, deadline) = match reserve(db, &agent.id, &chat.conversation_id, timeout)
            .await
        {
            Ok(token) => token,
            Err(error) => {
                let outcome = if matches!(&error, AppError::Forbidden(code) if code == "guest_escalation_limited")
                {
                    "limited"
                } else {
                    "unavailable"
                };
                audit(db, chat, &agent.id, outcome).await;
                return Err(error);
            }
        };
        operation.guest_deadline = Some(deadline);
        let mut completion_known = false;
        let result = async {
        let request = approvals::create_approval_request(db, &state.config, &state.http_client,
            state.fcm_auth.as_deref(), state.apns_auth.as_deref(), &resolution.primary_owner_user_id,
            &call.target.service_id, &call.target.service_name, &call.target.service_slug, call.requester_type.unwrap_or("access_token"), call.requester_id,
            Some(&display_label(&context)), operation, ApprovalMode::PerRequest, timeout, recipients, resolution.from_org_policy).await?;
        audit(db, chat, &agent.id, "pending").await;
        if let Err(error) = approvals::wait_for_decision(db, &request.id, timeout).await {
            if matches!(error, AppError::Forbidden(_)) {
                completion_known = true;
                audit(db, chat, &agent.id, "declined").await;
                return Err(AppError::Forbidden("guest_owner_declined".into()));
            }
            return Err(error);
        }
        completion_known = true;
        live_thread(db, chat).await?;
        let live_agent = team::agent(db, &chat.user_id, &agent.id).await?;
        if !live_agent.guest_policy.enabled || live_agent.guest_policy.escalation != GuestEscalation::AskOwner
            || (!live_agent.grants.service_ids.contains(&call.service.service_id) && !live_agent.grants.platform_service_ids.contains(&call.service.service_id)) {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        // The allowlist is independent of guest consent and may narrow while waiting.
        let key = super::key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
        if key.allow_all_services != initial_key.allow_all_services
            || key.allowed_service_ids != initial_key.allowed_service_ids
            || key.allowed_platform_service_ids != initial_key.allowed_platform_service_ids
            || key.allow_all_nodes != initial_key.allow_all_nodes
            || key.allowed_node_ids != initial_key.allowed_node_ids
        {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        call.prepared.authorize_agent_operations(&key.assistant_operation_scopes, call.service, call.endpoint)?;
        call.prepared.authorize_agent_operations(&live_agent.operation_scopes, call.service, call.endpoint)?;
        let catalog = super::mcp_service::load_operation_catalog(db, &state.node_ws_manager,
            &chat.user_id, super::mcp_service::NodeScope::Unrestricted,
            super::mcp_service::ServiceScope::Granted {
                user_service_ids: &live_agent.grants.service_ids,
                platform_service_ids: &live_agent.grants.platform_service_ids,
            }).await?;
        let service = catalog.services.iter().find(|s| s.service_id == call.service.service_id)
            .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
        let endpoint = service.endpoints.iter().find(|e| e.endpoint_id == context.endpoint_id)
            .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
        let live_metadata = service.durable_endpoint_metadata.get(&endpoint.endpoint_id)
            .copied().unwrap_or_default();
        if live_metadata != metadata
            || call.prepared.guest_digest(&service.service_id, endpoint) != context.operation_digest {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        let (live_outcome, _) = approvals::evaluate_for_chat(db, call.approval_owner_user_id,
            &call.target.service_owner_user_id, &call.target.service_id, &descriptor,
            call.requester_type, call.requester_id, call.is_session, call.target.is_auto_connected, Some(chat)).await?;
        if matches!(live_outcome, approvals::ApprovalOutcome::Denied) {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        if authority_digest(state, chat, &call).await? != authority {
            return Err(AppError::Forbidden("owner_only".into()));
        }
        completion_known = false; // A failed consume write may have persisted.
        let updated = db.collection::<Document>(crate::models::approval_request::COLLECTION_NAME)
            .update_one(doc! {"_id": &request.id, "status": "approved", "expires_at": {"$gt": bson::DateTime::now()},
                "guest.consumed": false, "guest.operation_digest": &context.operation_digest, "guest.turn_id": &context.turn_id},
                doc! {"$set": {"guest.consumed": true}}).await?;
        completion_known = true;
        if updated.modified_count != 1 { return Err(AppError::Forbidden("guest_owner_declined".into())); }
        audit(db, chat, &agent.id, "approved").await;
        Ok(())
    }.await;
        if result.is_err() {
            audit(db, chat, &agent.id, "refused").await;
        }
        // A failed insert acknowledgement or poll may leave a live pending row.
        // Retain its lease until the shared deadline unless completion is known.
        if completion_known {
            release(db, &agent.id, &admission).await?;
        }
        result
    })
}

/// Observe authority without refreshing or materializing provider credentials.
async fn authority_digest(
    state: &AppState,
    chat: &ChatAuthority,
    call: &Call<'_>,
) -> AppResult<String> {
    use super::mcp_service::McpToolSource;
    match &call.service.source {
        McpToolSource::UserManaged {
            user_service_id, ..
        } => {
            let mut snapshot =
                super::proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                    &state.db,
                    &state.encryption_keys,
                    &chat.user_id,
                    user_service_id,
                    Some(&call.service.service_slug),
                )
                .await?
                .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
            call.prepared.resolve_destination(&mut snapshot.target)?;
            let owner = snapshot
                .org_routing
                .as_ref()
                .map(|routing| routing.org_user_id.as_str())
                .unwrap_or(&chat.user_id);
            let fallbacks = super::node_routing_service::list_configured_binding_node_ids(
                &state.db,
                owner,
                &snapshot.target.service.id,
            )
            .await?;
            let override_identity = super::proxy_service::read_agent_credential_override_identity(
                &state.db,
                &chat.user_id,
                &chat.api_key_id,
                user_service_id,
                &snapshot.target,
            )
            .await?
            .map(
                |identity| super::execution_authority::OverrideCredentialIdentity {
                    api_key_id: identity.api_key_id,
                    credential_epoch: identity.credential_epoch,
                },
            );
            Ok(super::execution_authority::digest(
                &super::execution_authority::build_projection(
                    &snapshot,
                    override_identity.as_ref(),
                    fallbacks,
                ),
            ))
        }
        McpToolSource::Platform {
            downstream_service_id,
        } => {
            let row = state
                .db
                .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
                .find_one(doc! {"_id":downstream_service_id,"is_active":true})
                .await?
                .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
            use sha2::{Digest, Sha256};
            let value = serde_json::to_value(row)
                .map_err(|_| AppError::Internal("Guest authority encoding failed".into()))?;
            let bytes = serde_json::to_vec(&value)
                .map_err(|_| AppError::Internal("Guest authority encoding failed".into()))?;
            Ok(hex::encode(Sha256::digest(bytes)))
        }
        McpToolSource::Internal => Err(AppError::Forbidden("owner_only".into())),
    }
}

async fn live_thread(db: &Database, chat: &ChatAuthority) -> AppResult<AssistantConversation> {
    let key = super::key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
    if key.expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(AppError::Forbidden("owner_only".into()));
    }
    let live = super::assistant_acknowledgement_service::for_key(
        db,
        &chat.user_id,
        Some(&chat.api_key_id),
    )
    .await?
    .filter(|live| {
        live.guest
            && live.agent_id == chat.agent_id
            && live.turn_id == chat.turn_id
            && live.turn_live
            && !live.turn_stopped
    })
    .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
    super::assistant_invocation_service::revalidate_after_approval(db, &live).await?;
    db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &chat.conversation_id, "user_id": &chat.user_id})
        .await?
        .ok_or_else(|| AppError::Forbidden("owner_only".into()))
}

async fn context(
    db: &Database,
    chat: &ChatAuthority,
    agent: &AssistantAgent,
    row: &AssistantConversation,
) -> AppResult<GuestApprovalContext> {
    let origin = row
        .channel
        .as_ref()
        .ok_or_else(|| AppError::Forbidden("owner_only".into()))?;
    let thread = db.collection::<NyxbotThread>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
        .find_one(doc! {"channel_id": &origin.nyxbot_channel_id, "partition": &origin.partition, "user_id": &chat.user_id}).await?;
    let mut sender = None;
    if let Some(event_id) = row
        .active_turn
        .as_ref()
        .and_then(|t| t.channel_event_id.as_deref())
        && let Some(event) = db
            .collection::<NyxbotEvent>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
            .find_one(doc! {"_id": event_id, "conversation_id": &row.id, "turn_id": &chat.turn_id})
            .await?
        && let Some(message) = event.relay_message_id
        && let Some(message) = db
            .collection::<crate::models::channel_message::ChannelMessage>(
                crate::models::channel_message::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": message, "user_id": &chat.user_id, "direction": "inbound"})
            .await?
    {
        sender = message.sender_display_name;
    }
    Ok(GuestApprovalContext {
        agent_id: agent.id.clone(),
        agent_owner_id: agent.user_id.clone(),
        conversation_id: row.id.clone(),
        turn_id: chat
            .turn_id
            .clone()
            .ok_or_else(|| AppError::Forbidden("owner_only".into()))?,
        endpoint_id: String::new(),
        operation_digest: String::new(),
        platform: display_text(&origin.platform),
        chat_title: display_text(
            thread
                .as_ref()
                .and_then(|t| t.title.as_deref())
                .unwrap_or("Guest chat"),
        ),
        sender_name: display_text(sender.as_deref().unwrap_or("Guest")),
        agent_name: display_text(&agent.name),
        consumed: false,
    })
}

async fn audit(db: &Database, chat: &ChatAuthority, agent: &str, outcome: &str) {
    let _ = super::audit_service::log_actor_event(db.clone(), &super::audit_service::AuditActor {
        user_id: chat.user_id.clone(), ip_address: None, user_agent: None,
        api_key_id: Some(chat.api_key_id.clone()), api_key_name: None,
    }, "assistant_guest_escalation", Some(serde_json::json!({"agent_id": agent, "guest": true, "escalated": true, "outcome": outcome}))).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_labels_bound_and_remove_control_and_markup() {
        let input = "\n<b>@Owner</b> *[click](javascript:bad)\u{202e}\r\t".repeat(40);
        let text = display_text(&input);
        assert!(text.chars().count() <= 80);
        assert!(
            text.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '.' | ','))
        );
        assert!(!text.contains("Owner</"));
    }

    #[tokio::test]
    async fn guest_requests_never_coalesce_or_mint_grants_and_expiry_cannot_be_decided() {
        use crate::models::service_approval_config::ApprovalMode;
        let f = super::super::assistant_authority_tests::fixture("guest_request_fences").await;
        let mut operation = approvals::ApprovalRequestOperation::from_descriptor(
            &super::super::operation_descriptor::build_mcp_descriptor("DELETE", "/one", None),
            None,
        );
        operation.guest = Some(GuestApprovalContext {
            agent_id: f.chat.agent_id.clone(),
            agent_owner_id: f.owner.clone(),
            conversation_id: f.row.id.clone(),
            turn_id: f.chat.turn_id.clone().unwrap(),
            endpoint_id: "endpoint".into(),
            operation_digest: "digest".into(),
            platform: "telegram".into(),
            chat_title: "Guest chat".into(),
            sender_name: "Guest".into(),
            agent_name: "Specialist".into(),
            consumed: false,
        });
        let create = |deadline| {
            let mut operation = operation.clone();
            operation.guest_deadline = deadline;
            approvals::create_approval_request(
                &f.state.db,
                &f.state.config,
                &f.state.http_client,
                None,
                None,
                &f.owner,
                "service",
                "Service",
                "service",
                "api_key",
                &f.chat.api_key_id,
                None,
                operation,
                ApprovalMode::Grant,
                60,
                vec![f.owner.clone()],
                false,
            )
        };
        let deadline = bson::DateTime::from_chrono(Utc::now() + Duration::seconds(30)).to_chrono();
        let first = create(Some(deadline)).await.unwrap();
        let second = create(Some(deadline)).await.unwrap();
        assert_eq!(first.expires_at, deadline);
        assert_eq!(second.expires_at, deadline);
        assert!(create(None).await.is_err());
        assert!(
            create(Some(Utc::now() - Duration::seconds(1)))
                .await
                .is_err()
        );
        assert_ne!(first.id, second.id);
        assert_eq!(first.approval_mode, ApprovalMode::PerRequest);
        require_decider(&f.state.db, &first, &f.owner)
            .await
            .unwrap();
        assert!(
            require_decider(&f.state.db, &first, "outsider")
                .await
                .is_err()
        );
        // Even a malformed or legacy writer's stored grant mode cannot mint a grant.
        f.state
            .db
            .collection::<Document>(crate::models::approval_request::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &first.id},
                doc! {"$set": {"approval_mode":"grant"}},
            )
            .await
            .unwrap();
        assert!(
            approvals::process_decision(
                &f.state.db,
                &f.state.config,
                &f.state.http_client,
                None,
                None,
                &first.id,
                true,
                Some(3600),
                None,
                "web"
            )
            .await
            .is_err()
        );
        approvals::process_decision(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &first.id,
            true,
            None,
            None,
            "web",
        )
        .await
        .unwrap();
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::approval_grant::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        f.state.db.collection::<Document>(crate::models::approval_request::COLLECTION_NAME)
            .update_one(doc! {"_id": &second.id}, doc! {"$set": {"expires_at":bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        assert!(
            approvals::process_decision(
                &f.state.db,
                &f.state.config,
                &f.state.http_client,
                None,
                None,
                &second.id,
                true,
                None,
                None,
                "web"
            )
            .await
            .is_err()
        );
        assert_ne!(
            approvals::get_request(&f.state.db, &second.id)
                .await
                .unwrap()
                .status,
            "approved"
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn guest_limits_are_atomic_per_thread_and_per_specialist_and_recover_expiry() {
        let db = crate::test_utils::connect_transaction_test_database("guest_limit_atomic").await;
        let (cold_a, cold_b) = tokio::join!(
            reserve(&db, "cold", "thread", 60),
            reserve(&db, "cold", "thread", 60)
        );
        assert_ne!(cold_a.is_ok(), cold_b.is_ok());
        let refused = cold_a.err().or_else(|| cold_b.err()).unwrap();
        assert!(matches!(refused, AppError::Forbidden(code) if code == "guest_escalation_limited"));
        let (first, deadline) = reserve(&db, "specialist", "thread", 60).await.unwrap();
        let budget = db
            .collection::<Budget>(BUDGETS)
            .find_one(doc! {"_id":"specialist"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(budget.pending[0].expires_at, deadline);
        let (a, b) = tokio::join!(
            reserve(&db, "specialist", "thread", 60),
            reserve(&db, "specialist", "thread", 60)
        );
        assert!(a.is_err() && b.is_err());
        release(&db, "specialist", &first).await.unwrap();
        for index in 1..HOURLY_CAP {
            let (token, _) = reserve(&db, "specialist", &format!("thread-{index}"), 60)
                .await
                .unwrap();
            release(&db, "specialist", &token).await.unwrap();
        }
        assert!(
            matches!(reserve(&db, "specialist", "another", 60).await, Err(AppError::Forbidden(code)) if code == "guest_escalation_limited")
        );
        assert!(reserve(&db, "other-specialist", "thread", 60).await.is_ok());
        db.collection::<Budget>(BUDGETS).update_one(doc! {"_id":"specialist"}, doc! {"$set": {"attempts": [bson::DateTime::from_chrono(Utc::now() - Duration::hours(2))], "pending": [{"thread":"thread", "token":"abandoned", "expires_at":bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}]}}).await.unwrap();
        assert!(reserve(&db, "specialist", "thread", 60).await.is_ok());
        db.drop().await.unwrap();
    }
}
