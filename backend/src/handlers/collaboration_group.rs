//! Public collaboration surfaces and requester-scoped scheduling. No hidden
//! execution IDs or private tool streams are delivered to other participants.
use super::assistant_team::{Pool, acquire, owner_auth, team_pool_limit};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_conversation::AssistantConversation,
    services::{assistant_nyxagent as engine, collaboration_group_service as groups},
};

// The owned boxed boundary breaks the scheduler -> worker -> settlement cycle
// and keeps spawned work independent of a request handler's borrowed state.
pub(crate) fn advance(
    state: &AppState,
    group: Option<&str>,
) -> futures::future::BoxFuture<'static, AppResult<()>> {
    let state = state.clone();
    let group = group.map(str::to_owned);
    Box::pin(async move {
        for request in groups::requests::pending(&state.db, group.as_deref()).await? {
            for agent in &request.pending_agent_ids {
                let owner = request.actor_user_id.as_str();
                let guest_owner = request
                    .collaboration
                    .as_ref()
                    .and_then(|b| b.authorities.get(agent))
                    .filter(|a| a.lane == model::Lane::Guest)
                    .map(|a| a.owner_id.as_str());
                let pool = match guest_owner {
                    Some(owner) => Pool::Channel { owner },
                    None => Pool::Team {
                        owner,
                        limit: team_pool_limit(&state, owner).await + 1,
                    },
                };
                let Some(permit) = acquire(&state, pool).await? else {
                    if guest_owner.is_some() {
                        groups::requests::busy_queued(&state.db, &request, agent).await?;
                        continue;
                    }
                    break;
                };
                let started = match Box::pin(groups::runtime::begin(
                    &state.db,
                    &state.encryption_keys,
                    owner,
                    &request.group_id,
                    &request.id,
                    agent,
                ))
                .await
                {
                    Ok(started) => started,
                    Err(AppError::AssistantTurnActive) => {
                        if guest_owner.is_some() {
                            groups::requests::busy_queued(&state.db, &request, agent).await?;
                        }
                        continue;
                    }
                    Err(error)
                        if error.is_forbidden()
                            || matches!(
                                error,
                                AppError::NotFound(_)
                                    | AppError::Conflict(_)
                                    | AppError::ValidationError(_)
                            ) =>
                    {
                        groups::requests::refuse_queued(&state.db, &request, agent).await?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let authority = &started.admission.slot().authority;
                let audit_result=crate::services::audit_service::log_actor_event(state.db.clone(),
                &crate::services::audit_service::AuditActor {user_id:started.admission.requester().into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},
                "assistant_collaboration_turn_admitted",Some(serde_json::json!({
                    "actor_user_id":started.admission.requester(),"agent_owner_id":authority.owner_id,
                    "execution_user_id":started.claim.conversation.user_id,"execution_mode":authority.lane,
                    "group_id":request.group_id,"request_id":started.admission.request_id(),"agent_id":agent,
                    "billing_policy":match authority.lane {model::Lane::Shared=>"caller",model::Lane::Guest=>"owner",model::Lane::OrgMember=>"org_byok_person_platform",model::Lane::Owner=>"existing_owner"}
                }))).await;
                if audit_result.is_err() {
                    tracing::warn!("Collaboration admission audit deferred");
                }
                started.claim.log_reclaimed();
                let auth = owner_auth(&started.claim.conversation.user_id)?;
                let (sender, _) = tokio::sync::broadcast::channel(256);
                tokio::spawn(super::assistant_nyxagent::run_turn(
                    state.clone(),
                    auth,
                    started.claim.conversation,
                    started.input,
                    started.claim.credential,
                    Some(super::assistant_nyxagent::SERVER_TURN_POLICY),
                    permit,
                    crate::services::assistant_nyxagent_runtime::Events {
                        checkpoint: None,
                        sender,
                        cursor: 0,
                        reset_notified: false,
                    },
                ));
            }
        }
        Ok(())
    })
}

pub(crate) async fn settled(
    state: &AppState,
    row: &AssistantConversation,
    text: &str,
    error: Option<&engine::TurnError>,
) {
    let Some(binding) = &row.collaboration else {
        return;
    };
    if error.is_none() && !text.trim().is_empty() {
        let result: AppResult<()> = async {
            let admission = groups::requests::settled_admission(&state.db, binding).await?;
            let roster =
                groups::roster(&state.db, &binding.original_requester_id, &binding.group_id)
                    .await?;
            // UUIDs are authoritative; ambiguous textual names are never guessed.
            for target in crate::services::assistant_group_service::mentions(text, &roster) {
                groups::requests::handoff(&state.db, &admission, &target).await?;
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            tracing::debug!("Collaboration handoff declined");
        }
    }
    if Box::pin(advance(state, Some(&binding.group_id)))
        .await
        .is_err()
    {
        tracing::debug!("Collaboration admission deferred");
    }
}

pub(crate) async fn sweep(state: &AppState) {
    if groups::cleanup::sweep(&state.db).await.is_err() {
        tracing::debug!("Collaboration cleanup deferred");
    }
    if Box::pin(advance(state, None)).await.is_err() {
        tracing::debug!("Collaboration queue deferred");
    }
}

use super::assistant_group::{GroupMemberResponse, GroupResponse};
use crate::{
    models::{assistant_group::AssistantGroup, assistant_group_collaboration as model},
    mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Deserialize;
use serde_json::{Value, json};

async fn human(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    Ok(actor)
}

pub(crate) async fn response(
    state: &AppState,
    actor: &str,
    group: AssistantGroup,
) -> AppResult<GroupResponse> {
    let group = groups::get(&state.db, actor, &group.id).await?;
    let access = groups::identity::load(&state.db, actor, &group.id, None)
        .await
        .ok();
    let agents = groups::roster(&state.db, actor, &group.id).await?;
    let owners:Vec<crate::models::user::User>=state.db.collection(crate::models::user::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":agents.iter().map(|a|&a.user_id).collect::<Vec<_>>()},"is_active":true}).limit(8).await?.try_collect().await?;
    let threads = groups::threads(&state.db, &group.id).await?;
    let working: Vec<String> = threads
        .iter()
        .filter(|r| engine::live_turn(r, Utc::now()).is_some_and(|t| !t.stop_requested))
        .filter_map(|r| r.agent_id.clone())
        .collect();
    let pending = groups::requests::pending(&state.db, Some(&group.id)).await?;
    let own_threads: Vec<_> = threads
        .iter()
        .filter(|r| {
            r.collaboration
                .as_ref()
                .is_some_and(|b| b.original_requester_id == actor)
        })
        .map(|r| &r.id)
        .collect();
    let mut waiting = std::collections::HashSet::new();
    for (collection, field, filter) in [
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            "conversation_id",
            doc! {"conversation_id":{"$in":&own_threads},"status":{"$in":["pending","allowed"]},"expires_at":{"$gt":bson::DateTime::now()}},
        ),
        (
            crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME,
            "conversation_id",
            doc! {"conversation_id":{"$in":&own_threads},"decision":{"$in":["pending","once"]},"expires_at":{"$gt":bson::DateTime::now()}},
        ),
        (
            crate::models::approval_request::COLLECTION_NAME,
            "assistant_group.conversation_id",
            doc! {"assistant_group.group_id":&group.id,"assistant_group.conversation_id":{"$in":&own_threads},"status":"pending","expires_at":{"$gt":bson::DateTime::now()}},
        ),
    ] {
        let ids = state
            .db
            .collection::<bson::Document>(collection)
            .distinct(field, filter)
            .await?;
        waiting.extend(
            ids.into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned)),
        );
    }
    let active_requests: Vec<_> = pending
        .iter()
        .filter(|r| r.actor_user_id == actor)
        .map(|r| r.id.clone())
        .chain(
            threads
                .iter()
                .filter(|r| r.active_turn.is_some() || waiting.contains(&r.id))
                .filter_map(|r| r.collaboration.as_ref())
                .filter(|b| b.original_requester_id == actor)
                .map(|b| b.request_id.clone()),
        )
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let policies: Vec<model::AgentAccess> = state
        .db
        .collection(model::AGENT_ACCESS)
        .find(doc! {"group_id":&group.id})
        .limit(8)
        .await?
        .try_collect()
        .await?;
    let creator = group.user_id == actor;
    let guest_settings:Vec<_>=policies.iter().map(|p|json!({"agent_id":p.agent_id,"enabled":p.guest_enabled,"revision":p.revision,
        "can_enable":p.owner_id==actor,"can_disable":p.owner_id==actor||creator,
        "lane":access.as_ref().and_then(|a|a.agent(&p.agent_id)).and_then(|a|a.authority()).map(|a|a.lane)})).collect();
    let transfers: Option<model::Transfer> = state
        .db
        .collection(model::TRANSFERS)
        .find_one(doc! {"_id":&group.id,"expires_at":{"$gt":bson::DateTime::now()}})
        .await?;
    let proposals:Vec<model::AgentProposal>=state.db.collection(model::PROPOSALS).find(doc! {"group_id":&group.id,"membership_revision":group.collaboration.as_ref().expect("validated").membership_revision,"expires_at":{"$gt":bson::DateTime::now()}}).limit(128).await?.try_collect().await?;
    let invites: Vec<model::Invite> = if creator {
        state.db.collection(model::INVITES).find(doc! {"group_id":&group.id,"accepted_at":null,"cancelled_at":null,"expires_at":{"$gt":bson::DateTime::now()}}).limit(32).await?.try_collect().await?
    } else {
        vec![]
    };
    let available: Vec<_> = group
        .member_agent_ids
        .iter()
        .filter(|id| {
            access
                .as_ref()
                .and_then(|a| a.agent(id))
                .and_then(|a| a.authority())
                .is_some()
        })
        .cloned()
        .collect();
    Ok(GroupResponse {
        collaboration: Some(
            json!({"version":1,"creator_user_id":group.user_id,"membership_revision":group.collaboration.as_ref().expect("validated").membership_revision,
            "history_disclosure":groups::HISTORY_DISCLOSURE,"guest_access":guest_settings,"your_request_ids":active_requests,
            "can_manage":creator,"transfer_to_user_id":transfers.map(|t|t.to_user_id),
            "proposals":proposals.iter().map(|p|json!({"id":p.id,"agent_id":p.agent_id,"proposed_by":p.proposed_by})).collect::<Vec<_>>(),
            "invites":invites.iter().map(|i|json!({"id":i.id,"expires_at":i.expires_at})).collect::<Vec<_>>()}),
        ),
        owner: None,
        participants: Some(groups::participants(&state.db, &group).await?),
        your_role: Some(if creator { "creator" } else { "participant" }.into()),
        members: group
            .member_agent_ids
            .iter()
            .map(|id| {
                let agent = agents.iter().find(|a| &a.id == id);
                GroupMemberResponse {
                    id: id.clone(),
                    name: agent
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| "Unavailable agent".into()),
                    display_name: agent.and_then(|a| a.display_name.clone()),
                    kind: crate::models::assistant_agent::AgentKind::Specialist,
                    destroyed: agent.is_none_or(|a| a.destroyed_at.is_some()),
                    unavailable: !available.contains(id),
                    shared: access
                        .as_ref()
                        .and_then(|a| a.agent(id))
                        .and_then(|a| a.authority())
                        .is_some_and(|a| a.lane == model::Lane::Shared),
                    owner_name: agent
                        .and_then(|a| owners.iter().find(|o| o.id == a.user_id))
                        .and_then(|o| o.display_name.clone()),
                    conversation_id: None,
                    working: working.contains(id),
                }
            })
            .collect(),
        id: group.id,
        name: group.name,
        lead_agent_id: group.lead_agent_id.clone(),
        default_recipient_ids: crate::services::assistant_group_service::default_recipients(
            group
                .last_recipient_ids_by_actor
                .get(actor)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            &available,
            &group.lead_agent_id,
        ),
        working_agent_ids: working,
        message_count: group.message_count,
        last_message_at: group.last_message_at,
        created_at: group.created_at,
    })
}

pub(crate) async fn messages(
    state: &AppState,
    actor: &str,
    id: &str,
    before: Option<i64>,
) -> AppResult<Value> {
    let group = groups::get(&state.db, actor, id).await?;
    let rows = groups::messages(&state.db, actor, id, before).await?;
    let before_seq = (rows.len() == 100).then(|| rows[0].seq);
    let agents = groups::roster(&state.db, actor, id).await?;
    let participants = groups::participants(&state.db, &group).await?;
    let ids: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
    let feedback =
        crate::services::assistant_feedback_service::states_for_messages(&state.db, actor, &ids)
            .await?;
    let mut messages: Vec<_> = rows
        .into_iter()
        .map(|mut r| {
            if let Some(author) = r.author_user_id.as_deref() {
                r.author_display_name = participants
                    .iter()
                    .find(|p| p.id == author)
                    .map(|p| p.display_name.clone());
            }
            super::assistant_group::message_response(r, &agents)
        })
        .collect();
    super::assistant_feedback::attach(&mut messages, feedback, |m| {
        (m.id.as_str(), &mut m.feedback)
    });
    Ok(
        json!({"group":response(state,actor,group).await?,"messages":messages,"pending_actions":[],
        "collaboration_decisions":decisions(state,actor,id).await?,"before_seq":before_seq}),
    )
}

pub(crate) async fn post(
    state: &AppState,
    actor: &str,
    id: &str,
    text: &str,
    recipients: Option<&[String]>,
    uploads: &[String],
) -> AppResult<Value> {
    let request =
        groups::requests::enqueue_with_uploads(&state.db, actor, id, text, recipients, uploads)
            .await?;
    let message = groups::messages(&state.db, actor, id, Some(request.message_seq + 1))
        .await?
        .into_iter()
        .find(|m| m.seq == request.message_seq)
        .ok_or_else(|| AppError::NotFound("Group message not found".into()))?;
    let agents = groups::roster(&state.db, actor, id).await?;
    let selected = message.recipient_agent_ids.clone();
    Box::pin(advance(state, Some(id))).await?;
    Ok(
        json!({"message":super::assistant_group::message_response(message,&agents),"addressed_agent_ids":selected,"request_id":request.id}),
    )
}

#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Invite {
        #[serde(default)]
        intended_user_id: Option<String>,
    },
    CancelInvite {
        invite_id: String,
    },
    QuickAdd {
        person_id: String,
        history_disclosed: bool,
    },
    Remove {
        person_id: String,
    },
    Leave,
    Delete,
    ProposeTransfer {
        person_id: String,
    },
    AcceptTransfer,
    AddAgent {
        agent_id: String,
    },
    ProposeAgent {
        agent_id: String,
    },
    AcceptAgent {
        proposal_id: String,
    },
    RemoveAgent {
        agent_id: String,
    },
    GuestAccess {
        agent_id: String,
        enabled: bool,
        guest_revision: i64,
    },
    Rename {
        name: String,
    },
    Lead {
        agent_id: String,
    },
}
impl Operation {
    pub(crate) fn into_change(self, actor: &str) -> groups::membership::Change {
        use groups::membership::Change as C;
        match self {
            Self::Invite { intended_user_id } => C::Invite { intended_user_id },
            Self::CancelInvite { invite_id } => C::CancelInvite { invite_id },
            Self::QuickAdd {
                person_id,
                history_disclosed,
            } => C::QuickAdd {
                person_id,
                history_disclosed,
            },
            Self::Remove { person_id } => C::Remove { person_id },
            Self::Leave => C::Leave,
            Self::Delete => C::Delete,
            Self::ProposeTransfer { person_id } => C::ProposeTransfer { person_id },
            Self::AcceptTransfer => C::AcceptTransfer,
            Self::AddAgent { agent_id } => C::AddAgent {
                agent_id,
                proposed_by: actor.into(),
            },
            Self::ProposeAgent { agent_id } => C::ProposeAgent { agent_id },
            Self::AcceptAgent { proposal_id } => C::AcceptAgent { proposal_id },
            Self::RemoveAgent { agent_id } => C::RemoveAgent { agent_id },
            Self::GuestAccess {
                agent_id,
                enabled,
                guest_revision,
            } => C::GuestAccess {
                agent_id,
                enabled,
                expected_revision: guest_revision,
            },
            Self::Rename { name } => C::Rename { name },
            Self::Lead { agent_id } => C::Lead { agent_id },
        }
    }
}
#[derive(Deserialize)]
pub struct MembershipInput {
    pub expected_revision: i64,
    #[serde(flatten)]
    pub operation: Operation,
}
pub(crate) async fn apply(
    state: &AppState,
    actor: &str,
    id: &str,
    input: MembershipInput,
    approved: bool,
) -> AppResult<Value> {
    let changed = groups::membership::apply(
        &state.db,
        actor,
        id,
        input.expected_revision,
        input.operation.into_change(actor),
        approved,
    )
    .await?;
    let invite=changed.invite.map(|i|json!({"id":i.id,"token":i.token.as_str(),"expires_at":i.expires_at,"history_disclosure":i.history_disclosure}));
    let group = match changed.group {
        Some(g) if g.participant_user_ids.iter().any(|p| p == actor) => {
            Some(response(state, actor, g).await?)
        }
        _ => None,
    };
    Ok(json!({"group":group,"invite":invite}))
}
pub async fn membership(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<MembershipInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(apply(&state, &actor, &id, input, true).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteInput {
    pub token: String,
    #[serde(default)]
    pub history_disclosed: bool,
}
pub async fn preview_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<InviteInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    groups::require_available(&state.db, &actor).await?;
    let preview = groups::membership::preview_invite(&state.db, &actor, &input.token).await?;
    Ok(Json(
        json!({"group_id":preview.group_id,"group_name":preview.group_name,"creator_name":preview.creator_name,"participants":preview.participants,"expires_at":preview.expires_at,"history_disclosure":preview.history_disclosure}),
    ))
}
pub async fn accept_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<InviteInput>,
) -> AppResult<Json<GroupResponse>> {
    let actor = human(&state, &auth).await?;
    let group =
        groups::membership::accept_invite(&state.db, &actor, &input.token, input.history_disclosed)
            .await?;
    Ok(Json(response(&state, &actor, group).await?))
}
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, request)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let actor = human(&state, &auth).await?;
    groups::requests::cancel(&state.db, &actor, &id, &request).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Safe waiting rows for everyone; exact details and decision handles only for
/// the original requester. There is never a hidden-thread identifier here.
async fn decisions(state: &AppState, actor: &str, id: &str) -> AppResult<Vec<Value>> {
    groups::get(&state.db, actor, id).await?;
    let threads = groups::runtime::decision_threads(&state.db, actor, id).await?;
    let ids: Vec<_> = threads
        .iter()
        .filter(|t| !t.guest_turn)
        .map(|t| &t.id)
        .collect();
    let acks:Vec<crate::models::assistant_acknowledgement::AssistantAcknowledgement>=state.db.collection(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find(doc! {"conversation_id":{"$in":&ids},"kind":"action","status":"pending","expires_at":{"$gt":bson::DateTime::now()}}).limit(4096).await?.try_collect().await?;
    let consents:Vec<crate::models::assistant_sharing::AgentConsent>=state.db.collection(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
        .find(doc! {"conversation_id":{"$in":&ids},"decision":"pending","expires_at":{"$gt":bson::DateTime::now()}}).limit(4096).await?.try_collect().await?;
    let approvals:Vec<crate::models::approval_request::ApprovalRequest>=state.db.collection(crate::models::approval_request::COLLECTION_NAME)
        .find(doc! {"assistant_group.group_id":id,"assistant_group.conversation_id":{"$in":&ids},"status":"pending","expires_at":{"$gt":bson::DateTime::now()}}).limit(4096).await?.try_collect().await?;
    let connections:Vec<crate::models::user_service::UserService>=state.db.collection(crate::models::user_service::COLLECTION_NAME)
        .find(doc! {"user_id":actor,"_id":{"$in":consents.iter().filter(|c|c.actor_user_id==actor).map(|c|&c.connection_id).collect::<Vec<_>>()}}).limit(4096).await?.try_collect().await?;
    let mut result = Vec::new();
    for thread in threads {
        let Some(binding) = thread.collaboration.as_ref() else {
            continue;
        };
        let acks: Vec<_> = acks
            .iter()
            .filter(|a| a.conversation_id == thread.id)
            .collect();
        let consents: Vec<_> = consents
            .iter()
            .filter(|c| c.conversation_id == thread.id)
            .collect();
        let approvals: Vec<_> = approvals
            .iter()
            .filter(|a| {
                a.assistant_group.as_ref().is_some_and(|b| {
                    b.conversation_id == thread.id && b.request_id == binding.request_id
                })
            })
            .collect();
        if (acks.is_empty() && consents.is_empty() && approvals.is_empty()) || thread.guest_turn {
            continue;
        }
        let own = binding.original_requester_id == actor;
        if !own {
            if !acks.is_empty() || !consents.is_empty() || !approvals.is_empty() {
                result.push(json!({"requester_id":binding.original_requester_id,"agent_id":binding.agent_id,"can_decide":false,"summary":"Waiting for participant approval"}));
            }
            continue;
        }
        for approval in approvals {
            result.push(json!({"id":approval.id,"kind":"approval","requester_id":actor,"agent_id":binding.agent_id,"can_decide":true,"summary":approval.action_description.as_ref().unwrap_or(&approval.operation_summary),"audience":"Everyone in this group can see the reply."}));
        }
        for ack in acks {
            result.push(json!({"id":ack.id,"kind":"action","requester_id":actor,"agent_id":binding.agent_id,"can_decide":true,"summary":ack.summary,"audience":"Everyone in this group can see the reply."}));
        }
        for consent in consents {
            result.push(json!({"id":consent.id,"kind":"consent","requester_id":actor,"agent_id":binding.agent_id,"can_decide":true,"summary":consent.operation_summary,"connection_id":consent.connection_id,"connection_name":connections.iter().find(|c|c.id==consent.connection_id).map(|c|&c.slug),"audience":"Everyone in this group can see the reply. Consent is invalidated when group membership changes."}));
        }
    }
    Ok(result)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionInput {
    pub kind: String,
    pub decision: String,
}
pub async fn decide(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((group, id)): Path<(String, String)>,
    Json(input): Json<DecisionInput>,
) -> AppResult<StatusCode> {
    let actor = human(&state, &auth).await?;
    groups::get(&state.db, &actor, &group).await?;
    match input.kind.as_str() {
        "action" => {
            let ack = state
                .db
                .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                    crate::models::assistant_acknowledgement::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&id,"user_id":&actor,"kind":"action"})
                .await?
                .ok_or_else(|| AppError::NotFound("Decision not found".into()))?;
            groups::runtime::actor_thread(&state.db, &actor, &group, &ack.conversation_id).await?;
            if !matches!(input.decision.as_str(), "allow" | "deny") {
                return Err(AppError::ValidationError("Choose allow or deny".into()));
            }
            let row = crate::services::assistant_acknowledgement_service::decide(
                &state.db,
                &actor,
                &ack.conversation_id,
                &id,
                input.decision == "allow",
            )
            .await?;
            crate::services::assistant_acknowledgement_service::audit_decision(
                &state.db,
                &crate::services::audit_service::AuditActor::from_auth_user(&auth),
                &row,
            )
            .await;
        }
        "consent" => {
            let card = state
                .db
                .collection::<crate::models::assistant_sharing::AgentConsent>(
                    crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&id,"actor_user_id":&actor})
                .await?
                .ok_or_else(|| AppError::NotFound("Decision not found".into()))?;
            groups::runtime::actor_thread(&state.db, &actor, &group, &card.conversation_id).await?;
            let decision = serde_json::from_value(json!(input.decision))
                .map_err(|_| AppError::ValidationError("Choose once, always or skip".into()))?;
            crate::services::shared_agent_consent::decide(&state.db, &actor, &id, decision).await?;
        }
        _ => return Err(AppError::ValidationError("Unknown decision kind".into())),
    }
    Box::pin(advance(&state, Some(&group))).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn guest_widening(state: &AppState, actor: &str, args: &Value) -> AppResult<bool> {
    if args["change"]["action"] != "guest_access" || args["change"]["enabled"] != true {
        return Ok(false);
    }
    let policy=state.db.collection::<model::AgentAccess>(model::AGENT_ACCESS).find_one(doc! {
        "group_id":args["group"].as_str().unwrap_or_default(),"agent_id":args["change"]["agent_id"].as_str().unwrap_or_default(),"owner_id":actor,
        "revision":args["change"]["guest_revision"].as_i64().unwrap_or_default()}).await?.ok_or_else(||AppError::NotFound("Group agent not found".into()))?;
    Ok(!policy.guest_enabled)
}
pub(crate) async fn tool(
    state: &AppState,
    actor: &str,
    name: &str,
    args: &Value,
    approved: bool,
) -> AppResult<Value> {
    // Services gate new work. Retained reads and revocation remain available
    // during rollback, with the same rules as the human REST paths.
    match name {
        "create_collaboration_group" => {
            let ids: Vec<String> = serde_json::from_value(args["member_agent_ids"].clone())
                .map_err(|_| AppError::ValidationError("Choose specialist IDs".into()))?;
            let group = groups::membership::create(
                &state.db,
                actor,
                args["name"].as_str().unwrap_or_default(),
                &ids,
            )
            .await?;
            Ok(json!(response(state, actor, group).await?))
        }
        "list_collaboration_groups" => {
            let mut rows = Vec::new();
            for g in groups::list(&state.db, actor).await? {
                rows.push(response(state, actor, g).await?);
            }
            Ok(json!({"groups":rows}))
        }
        "manage_collaboration_group" => {
            let operation: Operation = serde_json::from_value(args["change"].clone())
                .map_err(|_| AppError::ValidationError("Invalid membership change".into()))?;
            if matches!(
                operation,
                Operation::ProposeTransfer { .. } | Operation::AcceptTransfer
            ) {
                return Err(AppError::Forbidden(
                    "Management transfer requires a human decision".into(),
                ));
            }
            apply(
                state,
                actor,
                args["group"].as_str().unwrap_or_default(),
                MembershipInput {
                    expected_revision: args["expected_revision"].as_i64().unwrap_or_default(),
                    operation,
                },
                approved,
            )
            .await
        }
        "post_to_collaboration_group" => {
            let ids: Option<Vec<String>> = args
                .get("recipient_agent_ids")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|_| AppError::ValidationError("Choose recipient IDs".into()))?;
            post(
                state,
                actor,
                args["group"].as_str().unwrap_or_default(),
                args["text"].as_str().unwrap_or_default(),
                ids.as_deref(),
                &[],
            )
            .await
        }
        _ => Err(AppError::NotFound("Tool not found".into())),
    }
}

pub async fn availability(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        json!({"available":groups::require_available(&state.db,&actor).await.is_ok()}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionInput {
    pub expected_revision: i64,
    #[serde(default)]
    pub history_disclosed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestInput {
    pub expected_revision: i64,
    pub guest_revision: i64,
    pub enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueInviteInput {
    pub expected_revision: i64,
    #[serde(default)]
    pub intended_user_id: Option<String>,
}
pub async fn invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<IssueInviteInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        apply(
            &state,
            &actor,
            &id,
            MembershipInput {
                expected_revision: input.expected_revision,
                operation: Operation::Invite {
                    intended_user_id: input.intended_user_id,
                },
            },
            true,
        )
        .await?,
    ))
}
pub async fn cancel_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, invite_id)): Path<(String, String)>,
    axum::extract::Query(input): axum::extract::Query<RevisionInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        apply(
            &state,
            &actor,
            &id,
            MembershipInput {
                expected_revision: input.expected_revision,
                operation: Operation::CancelInvite { invite_id },
            },
            true,
        )
        .await?,
    ))
}
pub async fn quick_add(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, person_id)): Path<(String, String)>,
    Json(input): Json<RevisionInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        apply(
            &state,
            &actor,
            &id,
            MembershipInput {
                expected_revision: input.expected_revision,
                operation: Operation::QuickAdd {
                    person_id,
                    history_disclosed: input.history_disclosed,
                },
            },
            true,
        )
        .await?,
    ))
}
pub async fn remove_participant(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, person_id)): Path<(String, String)>,
    axum::extract::Query(input): axum::extract::Query<RevisionInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        apply(
            &state,
            &actor,
            &id,
            MembershipInput {
                expected_revision: input.expected_revision,
                operation: Operation::Remove { person_id },
            },
            true,
        )
        .await?,
    ))
}
pub async fn leave(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<RevisionInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        apply(
            &state,
            &actor,
            &id,
            MembershipInput {
                expected_revision: input.expected_revision,
                operation: Operation::Leave,
            },
            true,
        )
        .await?,
    ))
}
pub async fn guest_access(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, agent_id)): Path<(String, String)>,
    Json(input): Json<GuestInput>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    Ok(Json(
        apply(
            &state,
            &actor,
            &id,
            MembershipInput {
                expected_revision: input.expected_revision,
                operation: Operation::GuestAccess {
                    agent_id,
                    enabled: input.enabled,
                    guest_revision: input.guest_revision,
                },
            },
            true,
        )
        .await?,
    ))
}

/// An agent owner may manage only that agent's guest policy without acquiring
/// participant or transcript access. Return identifiers and policy, no roster.
pub async fn owned_guest_groups(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent): Path<String>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let owned = state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&agent,"user_id":&actor,"kind":"specialist","destroyed_at":null})
        .await?
        .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;
    let settings: Vec<model::AgentAccess> = state
        .db
        .collection(model::AGENT_ACCESS)
        .find(doc! {"agent_id":&owned.id,"owner_id":&actor})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let ids: Vec<_> = settings.iter().map(|p| &p.group_id).collect();
    let current: Vec<AssistantGroup> = state
        .db
        .collection(crate::models::assistant_group::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids},"collaboration.version":1,"member_agent_ids":&agent})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    Ok(Json(
        json!({"groups":settings.iter().filter_map(|p|current.iter().find(|g|g.id==p.group_id).map(|g|json!({"group_id":g.id,"membership_revision":g.collaboration.as_ref().expect("filtered").membership_revision,"guest_revision":p.revision,"enabled":p.guest_enabled}))).collect::<Vec<_>>()}),
    ))
}
