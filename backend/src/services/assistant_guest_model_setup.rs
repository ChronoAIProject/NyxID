//! Visitor initialization uses an ordinary fenced owner turn, never a guest's
//! proposed model/connection. Coordination contains identifiers and closed codes.
use super::{
    assistant_nyxagent as engine, assistant_service_execution as execution,
    assistant_team_service as team,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AssistantAgent, OwnerModelRoute},
        assistant_conversation::{AssistantConversation, TurnOrigin},
        assistant_message::GuestSetupStep,
    },
};
use bson::{Document, doc};
use chrono::Utc;
use serde_json::json;
use std::time::Duration;

pub(crate) const KIND: &str = "guest_model_setup";
pub(crate) const INSTRUCTION: &str = "Initialize this assistant's model connection. Reply only OK. Do not use tools, memory, services, or take any other action.";
pub(crate) const WAIT: Duration = Duration::from_secs(60);
const RETRY: i64 = 60;

pub(crate) mod backoff;

pub(crate) async fn is_setup(db: &mongodb::Database, conversation: &str) -> AppResult<bool> {
    Ok(db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {"_id":conversation,"guest_model_setup":true})
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

/// The existing server transport admits only this exact hidden owner turn.
/// Claim expiry, Stop, erasure, revocation and organization demotion serialize
/// with dispatch. Guest/channel transport authority is never reused here.
pub(crate) async fn fence_dispatch(
    db: &mongodb::Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    key: &str,
) -> AppResult<AssistantConversation> {
    let context = (
        db.clone(),
        owner.to_owned(),
        conversation.to_owned(),
        turn.to_owned(),
        key.to_owned(),
    );
    super::transaction_service::run_db_only(db, &context, |tx, (db, owner, conversation, turn, key)| Box::pin(async move {
        super::erasure_admission::fence_in_session(db, owner, tx).await?;
        let live = db.collection::<Document>(execution::COLLECTION).update_one(
            doc!{"kind":KIND,"user_id":owner,"conversation_id":conversation,"api_key_id":key,
                "status":"setting_up","until":{"$gt":bson::DateTime::now()}},
            doc!{"$inc":{"dispatch_fence":1_i64}},
        ).session(&mut *tx).await?;
        if live.matched_count != 1 {return Err(AppError::AssistantTurnRequired);}
        let row = db.collection::<AssistantConversation>("assistant_conversations").find_one(
            doc!{"_id":conversation,"user_id":owner,"credential_api_key_id":key,
                "guest_model_setup":true,"guest_turn":false,"shared_agent_thread":{"$ne":true}}
        ).session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
        engine::live_turn(&row, Utc::now()).filter(|active|
            active.turn_id == *turn && !active.stop_requested && active.origin == TurnOrigin::Event
                && active.platform_origin.is_none()).ok_or(AppError::AssistantTurnRequired)?;
        if let Some(origin) = row.channel.as_ref().filter(|o| o.thread.is_some() && o.org_execution.is_some()) {
            let conversation = super::org_channel_service::follow_conversation(db, &row, origin, Some(&mut *tx)).await?;
            Box::pin(super::channel_thread_follow_service::validate_in_session(db, owner, origin, &conversation, true, tx)).await?;
        }
        let super::assistant_agent_credential_service::KeyAuthority::ModelSetup(agent_owner) =
            super::assistant_agent_credential_service::authority_in_session(db, &row, tx).await? else {
                return Err(AppError::AssistantTurnRequired);
            };
        fence_admin(db, owner, &agent_owner, tx).await?;
        if super::api_key_mutation_service::update_one(db,
            doc!{"_id":key,"user_id":owner,"is_active":true,"$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
            doc!{"$inc":{"guest_setup_dispatch_fence":1_i64}},Some(&mut *tx)).await?.matched_count != 1 {
            return Err(AppError::AssistantTurnRequired);
        }
        db.collection::<Document>("assistant_conversations").update_one(
            doc!{"_id":conversation,"user_id":owner},doc!{"$inc":{"guest_setup_dispatch_fence":1_i64}}
        ).session(tx).await?;
        Ok(row)
    })).await
}

pub(crate) fn failure_fix(reason: &str) -> &'static str {
    match reason {
        "credits_unavailable" => {
            "Add credits to the linking owner's account. Visitor setup retries automatically."
        }
        "setup_capacity" => "Wait for current work to finish. Visitor setup retries automatically.",
        "setup_timeout" => {
            "The model connection timed out during visitor setup. Check its availability; setup retries automatically."
        }
        _ => {
            "Check this assistant's configured model and the linking owner's model connection. Visitor setup retries automatically."
        }
    }
}

fn scope(actor: &str, agent: &str, profile: &str) -> Document {
    doc! {"kind":KIND,"user_id":actor,"agent_id":agent,"profile":profile}
}

pub(crate) async fn record(
    db: &mongodb::Database,
    actor: &str,
    agent: &str,
    profile: &str,
) -> AppResult<Option<Document>> {
    Ok(db
        .collection::<Document>(execution::COLLECTION)
        .find_one({
            let mut filter = scope(actor, agent, profile);
            filter.insert("expires_at", doc! {"$gt":bson::DateTime::now()});
            filter
        })
        .await?)
}

pub(crate) async fn recorded_route(
    db: &mongodb::Database,
    actor: &str,
    agent: &str,
    profile: &str,
) -> AppResult<Option<OwnerModelRoute>> {
    Ok(record(db, actor, agent, profile)
        .await?
        .filter(|r| r.get_str("status") == Ok("ready"))
        .and_then(|r| r.get_document("route").ok().cloned())
        .and_then(|r| bson::from_document(r).ok()))
}

pub(crate) async fn valid_route(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    actor: &str,
    route: &OwnerModelRoute,
) -> AppResult<bool> {
    valid_route_with_key(db, keys, actor, route, None).await
}

pub(crate) async fn valid_thread_route(
    state: &AppState,
    row: &AssistantConversation,
) -> AppResult<bool> {
    let Some(route) = row
        .owner_model_route
        .as_deref()
        .filter(|r| r.profile == row.model)
    else {
        return Ok(false);
    };
    valid_route_with_key(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        route,
        Some(&row.credential_api_key_id),
    )
    .await
}

async fn valid_route_with_key(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    actor: &str,
    route: &OwnerModelRoute,
    api_key: Option<&str>,
) -> AppResult<bool> {
    let target = match execution::readiness::bound_target(db, keys, actor, route).await {
        Ok(Some(target)) => target,
        Err(error @ AppError::DatabaseError(_)) => return Err(error),
        _ => return Ok(false),
    };
    let current = execution::guest_model::route_for_actor(
        db,
        actor,
        api_key,
        &route.profile,
        route.connection_id.as_deref(),
        &target,
        &json!({}),
    )
    .await;
    Ok(current.is_ok_and(|current| {
        current.execution == route.execution
            && current.catalog_service_id == route.catalog_service_id
    }))
}

/// Only a linking person or organization administrator may initialize; shared
/// recipients must continue using their own model lane without owner routes.
pub(crate) async fn eligible(
    db: &mongodb::Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<bool> {
    if agent.destroyed_at.is_some() {
        return Err(AppError::NotFound("Assistant unavailable".into()));
    }
    if actor != agent.user_id {
        super::org_agent_service::require_use(db, actor, agent).await?;
        let owner = db
            .collection::<Document>("users")
            .find_one(doc! {"_id":&agent.user_id,"user_type":"org","is_active":true})
            .await?;
        if owner.is_none() {
            return Ok(false);
        }
        let access = super::org_service::resolve_owner_access(db, actor, &agent.user_id).await?;
        if !matches!(access, super::org_service::OwnerAccess::AsOrgAdmin { .. }) {
            return Err(AppError::Forbidden(
                "Model setup requires the linking administrator".into(),
            ));
        }
    }
    Ok(true)
}

/// Persist the claim and hidden thread together. Updating the agent is the
/// cross-replica serialization fence, so no secondary unique index is needed.
async fn claim(
    state: &AppState,
    actor: &str,
    agent: &AssistantAgent,
    profile: &str,
    force: bool,
    budget: Duration,
    org_origin: Option<&crate::models::assistant_conversation::ChannelOrigin>,
) -> AppResult<Option<(String, AssistantConversation)>> {
    let context = (
        state.db.clone(),
        state.encryption_keys.clone(),
        actor.to_owned(),
        agent.clone(),
        profile.to_owned(),
        force,
        budget,
        org_origin.cloned(),
    );
    super::transaction_service::run_db_only(&state.db,&context,|tx,(db,keys,actor,agent,profile,force,budget,org_origin)|Box::pin(async move {
        super::erasure_admission::fence_in_session(db,actor,tx).await?;
        fence_admin(db, actor, &agent.user_id, tx).await?;
        if let Some(origin)=org_origin {
            let e=origin.org_execution.as_ref().ok_or_else(super::org_channel_service::denied)?;
            if !e.guest || e.actor_id!=*actor {return Err(super::org_channel_service::denied());}
            Box::pin(super::org_channel_service::validate_in_session(db,tx,origin,actor,&agent.id,true)).await?;
        }
        let changed=db.collection::<Document>("assistant_agents").update_one(doc!{"_id":&agent.id,"user_id":&agent.user_id,"destroyed_at":null,"model":&agent.model},doc!{"$inc":{"guest_setup_fence":1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1 { return Err(AppError::Conflict("Assistant changed during setup".into())); }
        let rows=db.collection::<Document>(execution::COLLECTION);
        let existing=rows.find_one(scope(actor,&agent.id,profile)).session(&mut *tx).await?;
        if let Some(old)=&existing {
            let active=old.get_datetime("until").is_ok_and(|at| at.to_chrono()>Utc::now());
            if (active && old.get_str("status")!=Ok("ready")) || (!force && old.get_str("status")==Ok("ready")) {return Ok(None);}
            if let Ok(id)=old.get_str("conversation_id") {
                db.collection::<Document>("assistant_conversations").update_one(doc!{"_id":id,"user_id":actor,"guest_model_setup":true,"active_turn":{"$type":"object"}},doc!{"$set":{"active_turn.stop_requested":true}}).session(&mut *tx).await?;
                if let Ok(key)=old.get_str("api_key_id") {
                    super::api_key_mutation_service::update_one(db,doc!{"_id":key,"user_id":actor},doc!{"$set":{"is_active":false}},Some(&mut *tx)).await?;
                }
            }
        }
        let retry_target = existing.as_ref().and_then(|r| r.get_document("retry_target").ok());
        let retry_config = backoff::fingerprint(db, agent, retry_target).await?;
        let model_failures = existing.as_ref().and_then(|r| r.get_i64("model_failures").ok()).unwrap_or(0);
        let mut row=team::create_automation_thread(db,keys,actor,agent,"Preparing visitors",tx).await?;
        if row.shared_agent_thread {return Err(AppError::Forbidden("Shared setup refused".into()));}
        row.guest_model_setup=true;
        row.channel=org_origin.clone();
        row.model=profile.clone();
        super::assistant_agent_credential_service::apply_authority(db, actor, &row.credential_api_key_id,
            &super::assistant_agent_credential_service::KeyAuthority::ModelSetup(agent.user_id.clone()), tx).await?;
        // Do not publish a pending owner event: an older replica's ordinary
        // wake sweep must never start this restricted initialization thread.
        db.collection::<AssistantConversation>("assistant_conversations").replace_one(doc!{"_id":&row.id},&row).session(&mut *tx).await?;
        let id=existing.as_ref().and_then(|r|r.get_str("_id").ok()).map(str::to_owned).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
        let claim=uuid::Uuid::new_v4().to_string();
        let notice_actor=if let Some(origin)=org_origin {
            db.collection::<Document>("nyxbot_channels").find_one(doc!{"_id":&origin.nyxbot_channel_id}).session(&mut *tx).await?.and_then(|r|r.get_str("user_id").ok().map(str::to_owned))
        } else {None};
        rows.replace_one(doc!{"_id":&id},doc!{"_id":&id,"kind":KIND,"user_id":actor,"agent_id":&agent.id,"profile":profile,
            "notice_actor":notice_actor,"status":"setting_up","claim":&claim,"conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,
            "model_failures":model_failures,"retry_config":retry_config,"retry_target":retry_target,
            "until":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::from_std(*budget).expect("bounded setup budget")+chrono::Duration::seconds(5)),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::days(7))}).upsert(true).session(&mut *tx).await?;
        Ok(Some((claim,row)))
    })).await
}

async fn fence_admin(
    db: &mongodb::Database,
    actor: &str,
    agent_owner: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    if actor != agent_owner && db.collection::<Document>("org_memberships")
        .update_one(doc!{"org_user_id":agent_owner,"member_user_id":actor,"role":"admin","revoked_at":bson::Bson::Null},
            doc!{"$inc":{"guest_setup_fence":1_i64}}).session(session).await?.matched_count != 1
    {
        return Err(AppError::Forbidden("Model setup requires the linking administrator".into()));
    }
    Ok(())
}

/// Same fixed request on every attempt. Guest text/headers/selectors never
/// enter this method. The current configured profile is read from the thread.
pub(crate) async fn kick(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    profile: &str,
    force: bool,
) -> AppResult<bool> {
    kick_bounded(state, actor, agent_id, profile, force, WAIT).await
}

async fn kick_bounded(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    profile: &str,
    force: bool,
    budget: Duration,
) -> AppResult<bool> {
    kick_with_origin(state, actor, agent_id, profile, force, budget, None).await
}

pub(crate) async fn kick_org_channel(
    state: &AppState,
    origin: &crate::models::assistant_conversation::ChannelOrigin,
    profile: &str,
) -> AppResult<bool> {
    let e = origin
        .org_execution
        .as_ref()
        .ok_or_else(super::org_channel_service::denied)?;
    if !e.guest {
        return Err(super::org_channel_service::denied());
    }
    Box::pin(super::org_channel_service::validate(
        &state.db,
        origin,
        &e.actor_id,
        &e.agent_id,
    ))
    .await?;
    kick_with_origin(
        state,
        &e.actor_id,
        &e.agent_id,
        profile,
        false,
        WAIT,
        Some(origin),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn kick_with_origin(
    state: &AppState,
    actor: &str,
    agent_id: &str,
    profile: &str,
    force: bool,
    budget: Duration,
    org_origin: Option<&crate::models::assistant_conversation::ChannelOrigin>,
) -> AppResult<bool> {
    let agent = team::agent(&state.db, actor, agent_id).await?;
    if actor == agent.user_id
        && org_origin.is_none()
        && state
            .db
            .collection::<Document>("users")
            .find_one(doc! {"_id":actor,"user_type":"org"})
            .await?
            .is_some()
    {
        return Err(super::org_channel_service::denied());
    }
    if !eligible(&state.db, actor, &agent).await? {
        return Ok(true);
    }
    // Settle an expired worker before considering another claim. Otherwise a
    // guest racing the sweep could bypass the failed dispatch's backoff.
    expire_wait(state, actor, agent_id, profile).await?;
    if let Some(record) = record(&state.db, actor, agent_id, profile).await? {
        backoff::refresh(&state.db, &agent, &record).await?;
    }
    let record = record(&state.db, actor, agent_id, profile).await?;
    if record.as_ref().is_some_and(|r| {
        r.get_str("status") != Ok("ready")
            && r.get_datetime("until")
                .is_ok_and(|at| at.to_chrono() > Utc::now())
    }) {
        return Ok(false);
    }
    let valid = if let Some(route) = recorded_route(&state.db, actor, agent_id, profile).await? {
        valid_route(&state.db, &state.encryption_keys, actor, &route).await?
    } else {
        false
    };
    if valid && !force {
        return Ok(true);
    }
    if record.is_none()
        && !force
        && let Some(route) = agent
            .owner_model_route
            .as_deref()
            .filter(|r| r.profile == profile)
        && valid_route(&state.db, &state.encryption_keys, actor, route).await?
    {
        return Ok(true);
    }
    if let Some((token, row)) = claim(
        state,
        actor,
        &agent,
        profile,
        force || !valid,
        budget,
        org_origin,
    )
    .await?
    {
        let state = state.clone();
        state.shutdown.clone().spawn(run(state, token, row, budget));
    }
    Ok(false)
}

pub(crate) fn ensure<'a>(
    state: &'a AppState,
    actor: &'a str,
    agent_id: &'a str,
    profile: &'a str,
    force: bool,
    deadline: tokio::time::Instant,
    org_origin: Option<&'a crate::models::assistant_conversation::ChannelOrigin>,
) -> futures::future::BoxFuture<'a, AppResult<bool>> {
    Box::pin(async move {
        if tokio::time::Instant::now() >= deadline {
            expire_wait(state, actor, agent_id, profile).await?;
            return match recorded_route(&state.db, actor, agent_id, profile).await? {
                Some(route) => valid_route(&state.db, &state.encryption_keys, actor, &route).await,
                None => Ok(false),
            };
        }
        let budget = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(WAIT);
        if kick_with_origin(state, actor, agent_id, profile, force, budget, org_origin).await? {
            return Ok(true);
        }
        loop {
            if let Some(status) = record(&state.db, actor, agent_id, profile).await? {
                match status.get_str("status").unwrap_or("") {
                    "ready" => return Ok(true),
                    "failed" => return Ok(false),
                    _ => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                expire_wait(state, actor, agent_id, profile).await?;
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
}

fn run(
    state: AppState,
    token: String,
    row: AssistantConversation,
    budget: Duration,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        let mut step = GuestSetupStep::Capacity;
        let outcome =
            tokio::time::timeout(budget, run_inner(&state, &token, &row, &mut step)).await;
        let (route, reason) = match outcome {
            Ok(Ok(Some(route))) => {
                step = GuestSetupStep::RouteValidation;
                match valid_route(&state.db, &state.encryption_keys, &row.user_id, &route).await {
                    Ok(true) => (Some(route), None),
                    Ok(false) => (None, Some("model_route_unavailable")),
                    Err(_) => (None, Some("model_connection_unavailable")),
                }
            }
            Ok(Ok(None)) => (None, Some("model_route_unavailable")),
            Ok(Err(AppError::InsufficientCredits)) => (None, Some("credits_unavailable")),
            Ok(Err(AppError::AssistantTurnActive)) => (None, Some("setup_capacity")),
            Ok(Err(_)) => (None, Some("model_connection_unavailable")),
            Err(_) => (None, Some("setup_timeout")),
        };
        let _ = engine::request_stop(&state.db, &row.user_id, &row.id).await;
        let filter =
            doc! {"kind":KIND,"claim":token,"conversation_id":&row.id,"status":"setting_up"};
        let updated=state.db.collection::<Document>(execution::COLLECTION).update_one(filter,backoff::terminal(doc!{
        "status":if route.is_some(){"ready"}else{"failed"},"route":bson::to_bson(&route).unwrap_or(bson::Bson::Null),
        "cleanup_thread_id":&row.id,"notice_pending":reason.is_some(),"reason":reason,
        "failure_step":bson::to_bson(&reason.map(|_|step)).unwrap_or(bson::Bson::Null)},
        doc!{"conversation_id":"","api_key_id":""},route.is_some())).await;
        let Ok(updated) = updated else {
            // Retain the hidden thread and claim for crash recovery if the
            // terminal write is unavailable; deletion would erase the outbox.
            return;
        };
        if updated.matched_count == 0 {
            // A timeout or newer generation owns cleanup now.
            return;
        }
        if updated.modified_count == 1 && reason.is_some() {
            let _ = notify(
                state.db.clone(),
                &row.user_id,
                row.agent_id.as_deref().unwrap_or_default(),
                &row.model,
            )
            .await;
        }
        // Hidden initialization has no ordinary reply, delivery, learning or wake-up.
        // The normal deletion path revokes its key and purges its message history.
        if engine::delete(&state.db, &row.user_id, &row.id)
            .await
            .is_ok()
        {
            let _ = state
                .db
                .collection::<Document>(execution::COLLECTION)
                .update_one(
                    doc! {"kind":KIND,"cleanup_thread_id":&row.id},
                    doc! {"$unset":{"cleanup_thread_id":""}},
                )
                .await;
        }
    })
}

async fn run_inner(
    state: &AppState,
    token: &str,
    row: &AssistantConversation,
    step: &mut GuestSetupStep,
) -> AppResult<Option<OwnerModelRoute>> {
    let limit = super::assistant_settings_service::get(&state.db, &row.user_id)
        .await?
        .max_concurrent_subagent_turns
        .max(1) as u32
        + 1;
    let permit = loop {
        if let Some(permit) = state
            .direct_chat_limiter
            .try_acquire_pool("assistant_team", &row.user_id, limit)
            .await?
        {
            break permit;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    if !super::coordination_service::RateWindowStore::admit(
        &state.db,
        "assistant_event_turns",
        &row.user_id,
        team::EVENT_TURNS_PER_HOUR,
        Duration::from_secs(3600),
    )
    .await?
    .allowed
    {
        return Err(AppError::AssistantTurnActive);
    }
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(row.id.clone()),
        agent_id: None,
        text: INSTRUCTION.into(),
        model: None,
        access_mode: None,
    });
    start.origin = TurnOrigin::Event;
    *step = GuestSetupStep::OrgAccess;
    start.org_access = super::org_channel_service::conversation_access(&state.db, row).await?;
    *step = GuestSetupStep::AgentAccess;
    let agent = team::agent_for_conversation(&state.db, row).await?;
    if !eligible(&state.db, &row.user_id, &agent).await? {
        return Err(AppError::Forbidden("Shared setup refused".into()));
    }
    let ids = engine::OwnerTurnIds {
        artifact_budget: Default::default(),
        conversation_id: row.id.clone(),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let context = (
        state.db.clone(),
        state.encryption_keys.clone(),
        row.clone(),
        start,
        agent,
        token.to_owned(),
        ids,
    );
    *step = GuestSetupStep::TurnAdmission;
    let active=super::transaction_service::run_db_only(&state.db,&context,|tx,(db,keys,row,start,agent,token,ids)|Box::pin(async move {
        let live=db.collection::<Document>(execution::COLLECTION).update_one(doc!{"kind":KIND,"claim":token,"conversation_id":&row.id,"status":"setting_up","until":{"$gt":bson::DateTime::now()}},doc!{"$inc":{"admission_fence":1_i64}}).session(&mut *tx).await?;
        if live.matched_count!=1 {return Err(AppError::AssistantTurnRequired);}
        fence_admin(db, &row.user_id, &agent.user_id, tx).await?;
        let event=team::bound_event(KIND,INSTRUCTION.into(),None,None);
        engine::append_events_in_session(db,&row.user_id,&row.id,&[event],tx).await?;
        Ok(engine::begin_owner_turn_in_session(db,tx,keys,&row.user_id,start,ids,agent,None,None,None).await?.into_started()?.conversation)
    })).await?;
    if let Some(turn) = active.active_turn.as_ref() {
        crate::turn_trace::admitted(&turn.turn_id, turn.started_at, turn.retry_of.as_deref());
    }
    *step = GuestSetupStep::CredentialLoad;
    let credential = super::assistant_agent_credential_service::load_for_conversation(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        &row.id,
    )
    .await?
    .ok_or(AppError::AssistantTurnRequired)?;
    *step = GuestSetupStep::ModelExecution;
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let mut execution = Box::pin(super::assistant_nyxagent_runtime::run_owner_turn(
        state.clone(),
        super::assistant_nyxagent_egress::model_setup(state.clone()),
        active,
        INSTRUCTION.into(),
        credential,
        Some(
            super::billing::route_inventory::BillingRoutePolicy::Metered(
                super::billing::route_inventory::BillingIngress::Proxy,
            ),
        ),
        permit,
        super::assistant_nyxagent_runtime::Events {
            checkpoint: None,
            sender,
            cursor: 0,
            reset_notified: false,
        },
    ));
    let result = loop {
        tokio::select! {
            result=&mut execution=>break result,
            ()=tokio::time::sleep(Duration::from_millis(100))=>{
                let accepted=state.db.collection::<Document>(execution::COLLECTION).find_one(doc!{"kind":KIND,"claim":token,"model_accepted":true}).projection(doc!{"_id":1}).await?.is_some();
                if accepted {
                    break stop_while_settling(&state.db, &row.user_id, &row.id, &mut execution).await?;
                }
            }
        }
    };
    let accepted = state
        .db
        .collection::<Document>(execution::COLLECTION)
        .find_one(doc! {"kind":KIND,"claim":token,"model_accepted":true})
        .projection(doc! {"_id":1})
        .await?
        .is_some();
    let Some(result) = result else {
        return Ok(None);
    };
    if let Some(error) = result.error.filter(|_| !accepted) {
        return if error.code == "insufficient_credits" || error.http_status == Some(402) {
            Err(AppError::InsufficientCredits)
        } else {
            Ok(None)
        };
    }
    *step = GuestSetupStep::RouteValidation;
    Ok(engine::get(&state.db, &row.user_id, &row.id)
        .await?
        .owner_model_route
        .map(|r| *r))
}

async fn stop_while_settling<F: std::future::Future>(
    db: &mongodb::Database,
    owner: &str,
    conversation: &str,
    execution: F,
) -> AppResult<F::Output> {
    // Settlement can already hold a Mongo transaction on this conversation.
    // Awaiting Stop alone would suspend the transaction that must commit
    // before that write can complete. Keep polling both futures.
    let (result, stopped) = tokio::join!(execution, engine::request_stop(db, owner, conversation));
    stopped?;
    Ok(result)
}

/// A wait can expire five seconds before the crash lease. Fence the observed
/// generation only after its own sixty-second execution budget has elapsed;
/// an older guest deadline must never cancel a more recent initialization.
pub(crate) async fn expire_wait(
    state: &AppState,
    actor: &str,
    agent: &str,
    profile: &str,
) -> AppResult<()> {
    let mut filter = scope(actor, agent, profile);
    filter.insert("status", "setting_up");
    filter.insert(
        "until",
        doc! {"$lte":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::seconds(5))},
    );
    if let Some(row) = state
        .db
        .collection::<Document>(execution::COLLECTION)
        .find_one_and_update(
            filter,
            backoff::terminal(
                doc! {"status":"failed","reason":"setup_timeout","failure_step":"lease_expired","notice_pending":true},
                doc! {},
                false,
            ),
        )
        .await?
        && let Ok(id) = row.get_str("conversation_id")
    {
        let _ = engine::request_stop(&state.db, actor, id).await;
        // Keep the thread identity for normal cleanup; its key is revoked there.
    }
    notify(state.db.clone(), actor, agent, profile).await
}

/// Reclaim crashed setup claims and hidden transcripts in the existing sweep.
/// Never restart unknown owner work, and never make a guest event an owner event.
pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    use futures::TryStreamExt;
    let stale: Vec<Document> = state
        .db
        .collection(execution::COLLECTION)
        .find(doc! {"kind":KIND,"status":"setting_up","until":{"$lte":bson::DateTime::now()}})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    for claim in stale {
        let Some(_claim) = crate::services::shutdown::process().background() else {
            break;
        };
        let (Ok(actor), Ok(id), Ok(token)) = (
            claim.get_str("user_id"),
            claim.get_str("conversation_id"),
            claim.get_str("claim"),
        ) else {
            continue;
        };
        let _ = engine::request_stop(&state.db, actor, id).await;
        let updated=state.db.collection::<Document>(execution::COLLECTION).update_one(
            doc!{"kind":KIND,"claim":token,"status":"setting_up"},
            backoff::terminal(doc!{"status":"failed","reason":"setup_timeout","failure_step":"lease_expired","notice_pending":true,"cleanup_thread_id":id},doc!{"conversation_id":"","api_key_id":""},false)
        ).await?;
        let _ = updated;
    }
    let notices: Vec<Document> = state
        .db
        .collection(execution::COLLECTION)
        .find(doc! {"kind":KIND,"notice_pending":true})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    for notice in notices {
        let Some(_claim) = crate::services::shutdown::process().background() else {
            break;
        };
        if let (Ok(actor), Ok(agent), Ok(profile)) = (
            notice.get_str("user_id"),
            notice.get_str("agent_id"),
            notice.get_str("profile"),
        ) {
            notify(state.db.clone(), actor, agent, profile).await?;
        }
    }
    let hidden:Vec<AssistantConversation>=state.db.collection("assistant_conversations")
        .find(doc!{"guest_model_setup":true,"created_at":{"$lte":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(70))}})
        .limit(32).await?.try_collect().await?;
    for row in hidden {
        let Some(_claim) = crate::services::shutdown::process().background() else {
            break;
        };
        // The normal delete path revokes keys and purges all content. Completed
        // route metadata is deliberately unbound before deleting its thread.
        state
            .db
            .collection::<Document>(execution::COLLECTION)
            .update_many(
                doc! {"kind":KIND,"conversation_id":&row.id},
                doc! {"$unset":{"conversation_id":"","api_key_id":""}},
            )
            .await?;
        engine::delete(&state.db, &row.user_id, &row.id).await?;
    }
    Ok(())
}

/// Share one deadline between the durable channel wait and a mapping-drift retry.
pub(crate) async fn guest_deadline(
    db: &mongodb::Database,
    row: &AssistantConversation,
) -> AppResult<tokio::time::Instant> {
    let mut wait = WAIT;
    if row.guest_turn
        && let Some(event) = row
            .active_turn
            .as_ref()
            .and_then(|t| t.channel_event_id.as_deref())
        && let Some(event) = db
            .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
            .find_one(doc! {"_id":event,"$or":[{"user_id":&row.user_id},{"delivery.origin.org_execution.actor_id":&row.user_id}]})
            .projection(doc! {"guest_setup_deadline":1})
            .await?
        && let Ok(deadline) = event.get_datetime("guest_setup_deadline")
    {
        wait = (deadline.to_chrono() - Utc::now())
            .to_std()
            .unwrap_or_default()
            .min(WAIT);
    }
    Ok(tokio::time::Instant::now() + wait)
}

/// Fixed visitor text is backed by an actual setup outcome, never inferred from
/// an arbitrary upstream 403. Preserve its closed reason at normal settlement.
pub(crate) async fn failure(
    db: &mongodb::Database,
    row: &AssistantConversation,
) -> engine::TurnError {
    use crate::models::{
        assistant_conversation::ModelRouteDiagnostic,
        assistant_message::{TurnFailureReason, TurnMetadata},
    };
    #[derive(serde::Deserialize, Default)]
    struct Diagnostic {
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        failure_step: Option<GuestSetupStep>,
    }
    let diagnostic = if let Some(agent) = row.agent_id.as_deref() {
        record(db, &row.user_id, agent, &row.model)
            .await
            .ok()
            .flatten()
            .and_then(|record| bson::from_document::<Diagnostic>(record).ok())
            .unwrap_or_default()
    } else {
        Diagnostic::default()
    };
    let reason = diagnostic.reason;
    let mut error = engine::TurnError::internal("guest_model_route_required", "guest_setup");
    if let Some(reason) = reason.filter(|s| failure_code(s)) {
        error = error.with_model_route_reason(Some(&reason));
        error.metadata = Some(Box::new(TurnMetadata {
            artifact_recovery_target: None,
            artifact_delivery: None,
            failure_reason: Some(Box::new(TurnFailureReason {
                guest_setup_step: diagnostic.failure_step,
                source: "nyxid".into(),
                stage: "guest_setup".into(),
                code: reason.clone(),
                http_status: None,
                nyxid_error_code: None,
                model_route: Some(Box::new(ModelRouteDiagnostic {
                    reason,
                    unknown_fields: vec![],
                    attempt_id: None,
                    at: Utc::now(),
                })),
            })),
            ..Default::default()
        }));
    }
    error
}
pub(crate) fn failure_code(code: &str) -> bool {
    matches!(
        code,
        "model_route_unavailable"
            | "model_connection_unavailable"
            | "credits_unavailable"
            | "setup_timeout"
            | "setup_capacity"
    )
}

async fn notify(db: mongodb::Database, actor: &str, agent: &str, profile: &str) -> AppResult<()> {
    if let Some(row) = record(&db, actor, agent, profile).await?
        && row.get_bool("notice_pending") == Ok(true)
        && let (Ok(reason), Ok(token)) = (row.get_str("reason"), row.get_str("claim"))
    {
        let notice_actor = row.get_str("notice_actor").unwrap_or(actor);
        let fix = if notice_actor != actor {
            "Check the organization's model connection and billing wallet. Guest setup retries automatically."
        } else {
            failure_fix(reason)
        };
        let result =
            execution::readiness_notice::enqueue_failure(&db, notice_actor, agent, reason, fix)
                .await;
        match result {
            Ok(())
            | Err(AppError::NotFound(_) | AppError::Forbidden(_) | AppError::ErasurePreparing) => {}
            Err(error) => return Err(error),
        }
        db.collection::<Document>(execution::COLLECTION)
            .update_one(
                doc! {"kind":KIND,"claim":token},
                doc! {"$set":{"notice_pending":false}},
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn guest_setup_2092_transport_requires_a_live_hidden_owner_claim() {
        let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
            "guest_setup_dispatch",
        ))
        .await;
        assert!(
            fence_dispatch(
                &f.state.db,
                &f.owner,
                &f.row.id,
                &f.row.active_turn.as_ref().unwrap().turn_id,
                &f.row.credential_api_key_id
            )
            .await
            .is_err()
        );
        let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        let (_, row) = claim(&f.state, &f.owner, &agent, &agent.model, false, WAIT, None)
            .await
            .unwrap()
            .unwrap();
        let ids = engine::OwnerTurnIds {
            artifact_budget: Default::default(),
            conversation_id: row.id.clone(),
            turn_id: Uuid::new_v4().to_string(),
            message_id: Uuid::new_v4().to_string(),
        };
        assert!(
            fence_dispatch(
                &f.state.db,
                &f.owner,
                &row.id,
                &ids.turn_id,
                &row.credential_api_key_id
            )
            .await
            .is_err(),
            "a claim without owner turn admission cannot dispatch"
        );
        let mut start = engine::TurnStart::from(&engine::TurnRequest {
            conversation_id: Some(row.id.clone()),
            agent_id: None,
            text: INSTRUCTION.into(),
            model: None,
            access_mode: None,
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
        });
        start.origin = TurnOrigin::Event;
        let context = (
            f.state.db.clone(),
            f.state.encryption_keys.clone(),
            row.clone(),
            agent,
            start,
            ids,
        );
        let active = super::super::transaction_service::run_db_only(
            &f.state.db,
            &context,
            |tx, (db, keys, row, agent, start, ids)| {
                Box::pin(async move {
                    engine::append_events_in_session(
                        db,
                        &row.user_id,
                        &row.id,
                        &[team::bound_event(KIND, INSTRUCTION.into(), None, None)],
                        tx,
                    )
                    .await?;
                    Ok(engine::begin_owner_turn_in_session(
                        db,
                        tx,
                        keys,
                        &row.user_id,
                        start,
                        ids,
                        agent,
                        None,
                        None,
                        None,
                    )
                    .await?
                    .into_started()?
                    .conversation)
                })
            },
        )
        .await
        .unwrap();
        let turn = &active.active_turn.as_ref().unwrap().turn_id;
        fence_dispatch(
            &f.state.db,
            &f.owner,
            &row.id,
            turn,
            &row.credential_api_key_id,
        )
        .await
        .unwrap();
        assert!(
            fence_dispatch(
                &f.state.db,
                &f.owner,
                &row.id,
                turn,
                &f.row.credential_api_key_id
            )
            .await
            .is_err()
        );
        engine::request_stop(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap();
        assert!(
            fence_dispatch(
                &f.state.db,
                &f.owner,
                &row.id,
                turn,
                &row.credential_api_key_id
            )
            .await
            .is_err(),
            "Stop fences system model dispatch too"
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn guest_setup_2092_stop_keeps_polling_settlement_transaction() {
        let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
            "guest_setup_stop",
        ))
        .await;
        let (held, mut locked) = tokio::sync::oneshot::channel();
        let mut settlement = Box::pin(async {
            let mut session = f.state.db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$inc":{"settlement_test_fence":1_i64}},
                )
                .session(&mut session)
                .await
                .unwrap();
            held.send(()).unwrap();
            tokio::task::yield_now().await;
            session.commit_transaction().await.unwrap();
        });
        tokio::select! {
            biased;
            result = &mut locked => result.unwrap(),
            () = &mut settlement => panic!("settlement must pause with its transaction held"),
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            stop_while_settling(&f.state.db, &f.owner, &f.row.id, &mut settlement),
        )
        .await
        .expect("Stop must not suspend the transaction it waits on")
        .unwrap();
        assert!(
            engine::get(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .active_turn
                .as_ref()
                .unwrap()
                .stop_requested
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn guest_setup_2092_concurrent_claims_and_hidden_crash_cleanup() {
        let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
            "guest_setup_claim",
        ))
        .await;
        let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        let (first, second) = tokio::join!(
            claim(&f.state, &f.owner, &agent, &agent.model, false, WAIT, None),
            claim(&f.state, &f.owner, &agent, &agent.model, false, WAIT, None)
        );
        let rows = [first.unwrap(), second.unwrap()];
        assert_eq!(rows.iter().filter(|r| r.is_some()).count(), 1);
        let (_, hidden) = rows.into_iter().flatten().next().unwrap();
        assert!(
            hidden.guest_model_setup
                && hidden.pending_events.is_empty()
                && hidden.active_turn.is_none()
        );
        let initial_key = super::super::key_service::get_api_key(
            &f.state.db,
            &f.owner,
            &hidden.credential_api_key_id,
        )
        .await
        .unwrap();
        assert!(
            !initial_key.allow_all_services
                && !initial_key.allow_all_nodes
                && initial_key.allowed_service_ids.is_empty(),
            "the durable claim already has restricted authority"
        );
        assert!(
            engine::list(&f.state.db, &f.owner, 100, None, None)
                .await
                .unwrap()
                .iter()
                .all(|row| row.id != hidden.id)
        );
        assert!(
            team::threads(&f.state.db, &agent, 100)
                .await
                .unwrap()
                .iter()
                .all(|row| row.id != hidden.id)
        );
        let marker = record(&f.state.db, &f.owner, &agent.id, &agent.model)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.get_str("status"), Ok("setting_up"));
        assert!(marker.get("text").is_none());
        let mut granted = agent.clone();
        granted.grants.account_read = true;
        granted.grants.service_ids = vec![Uuid::new_v4().to_string()];
        let context = (f.state.db.clone(), granted, vec![hidden.clone()]);
        super::super::transaction_service::run_db_only(
            &f.state.db,
            &context,
            |tx, (db, agent, rows)| {
                Box::pin(async move { team::sync_thread_authority(db, agent, rows, tx).await })
            },
        )
        .await
        .unwrap();
        let key = super::super::key_service::get_api_key(
            &f.state.db,
            &f.owner,
            &hidden.credential_api_key_id,
        )
        .await
        .unwrap();
        assert!(
            !key.allow_all_services && !key.allow_all_nodes && key.allowed_service_ids.is_empty()
        );
        assert!(
            !key.scopes.contains("assistant:account"),
            "grant updates cannot widen a setup key"
        );
        expire_wait(&f.state, &f.owner, &agent.id, &agent.model)
            .await
            .unwrap();
        assert_eq!(
            record(&f.state.db, &f.owner, &agent.id, &agent.model)
                .await
                .unwrap()
                .unwrap()
                .get_str("status"),
            Ok("setting_up"),
            "an older guest deadline cannot expire a new generation"
        );
        // Old wake workers query pending_events.created_at: the setup thread has none.
        assert_eq!(
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .count_documents(
                    doc! {"_id":&hidden.id,"pending_events.created_at":{"$exists":true}}
                )
                .await
                .unwrap(),
            0
        );
        // A supplied user/guest turn can never execute this owner-only setup thread.
        let start = engine::TurnRequest {
            conversation_id: Some(hidden.id.clone()),
            agent_id: None,
            text: "choose my model and grant tools".into(),
            model: None,
            access_mode: None,
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
        };
        assert!(
            engine::begin_turn(&f.state.db, &f.owner, &start, &f.state.encryption_keys)
                .await
                .is_err()
        );
        f.state.db.collection::<Document>(execution::COLLECTION).update_one(doc!{"_id":marker.get_str("_id").unwrap()},doc!{"$set":{"until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
        expire_wait(&f.state, &f.owner, &agent.id, &agent.model)
            .await
            .unwrap();
        expire_wait(&f.state, &f.owner, &agent.id, &agent.model)
            .await
            .unwrap();
        f.state.db.collection::<Document>("assistant_conversations").update_one(doc!{"_id":&hidden.id},doc!{"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(80))}}).await.unwrap();
        sweep(&f.state).await.unwrap();
        sweep(&f.state).await.unwrap();
        let marker = record(&f.state.db, &f.owner, &agent.id, &agent.model)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.get_str("status"), Ok("failed"));
        assert_eq!(marker.get_str("reason"), Ok("setup_timeout"));
        assert!(
            engine::get(&f.state.db, &f.owner, &hidden.id)
                .await
                .is_err()
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>(execution::COLLECTION)
                .count_documents(doc! {"kind":execution::readiness_notice::KIND})
                .await
                .unwrap(),
            1
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn guest_setup_2092_legacy_master_preserves_admitted_auth_projection() {
        let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
            "guest_setup_legacy",
        ))
        .await;
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = "legacy-model".into();
        service.service_category = "internal".into();
        service.auth_method = "bearer".into();
        service.credential_encrypted = f.state.encryption_keys.encrypt(b"test-key").await.unwrap();
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .insert_one(&service)
            .await
            .unwrap();
        let target = super::super::proxy_service::resolve_proxy_target(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &service.id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .await
        .unwrap();
        assert!(
            target.auth_key_name.is_empty(),
            "legacy execution preserves the catalog header field"
        );
        let route = execution::guest_model::route_for_actor(
            &f.state.db,
            &f.owner,
            None,
            &f.row.model,
            None,
            &target,
            &json!({}),
        )
        .await
        .unwrap();
        assert!(
            valid_route(&f.state.db, &f.state.encryption_keys, &f.owner, &route)
                .await
                .unwrap()
        );
        f.state
            .db
            .collection::<Document>("downstream_services")
            .update_one(
                doc! {"_id":&service.id},
                doc! {"$set":{"auth_method":"none","credential_encrypted":bson::Binary{
                subtype:bson::spec::BinarySubtype::Generic,bytes:Vec::new()}}},
            )
            .await
            .unwrap();
        assert!(
            !valid_route(&f.state.db, &f.state.encryption_keys, &f.owner, &route)
                .await
                .unwrap()
        );
        let target = super::super::proxy_service::resolve_proxy_target(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &service.id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .await
        .unwrap();
        let route = execution::guest_model::route_for_actor(
            &f.state.db,
            &f.owner,
            None,
            &f.row.model,
            None,
            &target,
            &json!({}),
        )
        .await
        .unwrap();
        assert!(
            valid_route(&f.state.db, &f.state.encryption_keys, &f.owner, &route)
                .await
                .unwrap(),
            "a newly owner-admitted no-auth route remains valid; it does not create bootstrap authority"
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn guest_setup_2092_platform_binding_ignores_retained_unused_byok_key() {
        let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
            "guest_setup_platform",
        ))
        .await;
        let id = crate::services::assistant_authority_tests::connected(
            &f.state.db,
            &f.owner,
            "model",
            "https://models.test",
        )
        .await;
        let rows = f.state.db.collection::<Document>("user_services");
        rows.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"credential_binding":"platform","api_key_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
        let first = execution::connection_binding(&f.state.db, &id, &f.chat.api_key_id)
            .await
            .unwrap();
        rows.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"api_key_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
        assert_eq!(
            first,
            execution::connection_binding(&f.state.db, &id, &f.chat.api_key_id)
                .await
                .unwrap()
        );
        rows.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"credential_binding":"user"}},
        )
        .await
        .unwrap();
        assert!(
            execution::connection_binding(&f.state.db, &id, &f.chat.api_key_id)
                .await
                .is_err()
        );
        f.state.db.drop().await.unwrap();
    }
}

#[cfg(test)]
#[tokio::test]
async fn guest_setup_2092_org_setup_uses_linking_admin_not_org_or_member() {
    let f = super::org_agent_tests::Fixture::new("guest_setup_org").await;
    let (agent, _) = f.create().await;
    assert!(eligible(&f.state.db, &f.admin, &agent).await.unwrap());
    assert!(eligible(&f.state.db, &f.member, &agent).await.is_err());
    let (_, hidden) = claim(&f.state, &f.admin, &agent, &agent.model, false, WAIT, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hidden.user_id, f.admin);
    assert_eq!(hidden.agent_owner_id.as_deref(), Some(f.org.as_str()));
    assert!(!hidden.shared_agent_thread);
    assert!(
        record(&f.state.db, &f.org, &agent.id, &agent.model)
            .await
            .unwrap()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

/// NyxAgent's Responses request deliberately has no generation/tool parameters.
/// Bound its real model callback instead, after closed-shape validation and
/// before ordinary owner admission, reservation and provider transmission.
pub(crate) async fn restrict_callback(
    db: &mongodb::Database,
    chat: Option<&super::assistant_acknowledgement_service::ChatAuthority>,
    target: &super::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    body: bytes::Bytes,
) -> AppResult<bytes::Bytes> {
    let Some(chat) = chat else { return Ok(body) };
    if !is_setup(db, &chat.conversation_id).await? {
        return Ok(body);
    }
    if chat.guest || !chat.turn_live || chat.turn_stopped {
        return Err(AppError::AssistantTurnRequired);
    }
    let mut parsed: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    if target.service.inference.is_none()
        || method != "POST"
        || !execution::stateless_model_transport(path, &parsed)
    {
        return Err(AppError::Forbidden(
            "Visitor setup only admits stateless model generation".into(),
        ));
    }
    let path = path
        .trim_matches('/')
        .strip_prefix("v1/")
        .unwrap_or(path.trim_matches('/'));
    if !matches!(path, "responses" | "chat/completions" | "messages") {
        return Err(AppError::Forbidden(
            "Visitor setup requires a generation callback".into(),
        ));
    }
    parsed["tools"] = json!([]);
    parsed["tool_choice"] = if path == "messages" {
        json!({"type":"none"})
    } else {
        json!("none")
    };
    // Token knobs are not route identity. Never modify model, target, semantic
    // headers or prompt; this remains NyxAgent's genuine configured owner call.
    let object = parsed.as_object_mut().expect("validated generation object");
    let output_bound = match path {
        "responses" => "max_output_tokens",
        "chat/completions" if object.contains_key("max_completion_tokens") => {
            "max_completion_tokens"
        }
        _ => "max_tokens",
    };
    for key in ["max_output_tokens", "max_tokens", "max_completion_tokens"] {
        object.remove(key);
    }
    object.insert(output_bound.into(), json!(16));
    serde_json::to_vec(&parsed)
        .map(bytes::Bytes::from)
        .map_err(|_| AppError::Internal("Model setup projection failed".into()))
}

#[cfg(test)]
#[tokio::test]
async fn org_channel_setup_uses_verified_org_payer_and_shared_durable_claim_without_admin_sponsorship()
 {
    let mut f = super::org_channel_service::tests::Fixture::new("org_channel_setup_claim").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let first = f.guest("one").await;
    let second = f.guest("two").await;
    let (a, b) = tokio::join!(
        claim(
            &f.org.state,
            &f.org.org,
            &f.agent,
            &f.agent.model,
            false,
            WAIT,
            Some(&first)
        ),
        claim(
            &f.org.state,
            &f.org.org,
            &f.agent,
            &f.agent.model,
            false,
            WAIT,
            Some(&second)
        )
    );
    let claims = [a.unwrap(), b.unwrap()];
    assert_eq!(claims.iter().filter(|c| c.is_some()).count(), 1);
    let (_, hidden) = claims.into_iter().flatten().next().unwrap();
    assert_eq!(hidden.user_id, f.org.org);
    assert!(hidden.guest_model_setup && !hidden.guest_turn && hidden.pending_events.is_empty());
    let access = super::org_channel_service::conversation_access(&f.org.state.db, &hidden)
        .await
        .unwrap()
        .unwrap();
    assert!(access.channel().unwrap().guest_payer().is_some());
    let key = f.org.key(&hidden).await;
    assert!(!key.allow_all_services && !key.allow_all_nodes && key.allowed_service_ids.is_empty());
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        &f.org.state.db,
        &key,
        None,
        None,
        None,
    ))
    .await
    .unwrap();
    assert_eq!(auth.user_id.to_string(), f.org.org);
    let payer = f
        .org
        .state
        .billing
        .owner_resolver()
        .resolve_for_assistant_execution(
            &f.org.org,
            &f.org.org,
            crate::models::usage_meter::CredentialClass::NyxidManagedMaster,
            auth.org_agent_access.as_deref(),
        )
        .await
        .unwrap();
    assert_eq!(payer.owner_id, f.org.org);
    assert!(
        engine::list(&f.org.state.db, &f.org.org, 100, None, None)
            .await
            .unwrap()
            .iter()
            .all(|r| r.id != hidden.id)
    );
    assert!(
        kick(&f.org.state, &f.org.org, &f.agent.id, &f.agent.model, false)
            .await
            .is_err(),
        "an org id alone cannot authorize setup"
    );
    f.org.revoke(&f.org.admin).await;
    assert!(
        Box::pin(crate::mw::auth::api_key_auth_user(
            &f.org.state.db,
            &key,
            None,
            None,
            None
        ))
        .await
        .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[cfg(test)]
async fn org_follow_setup_revocation(change: &str) {
    let mut f =
        super::org_channel_service::tests::Fixture::new(&format!("org_follow_setup_{change}"))
            .await;
    f.wallet().await;
    f.guests(vec![]).await;
    let guest = match change {
        "flag_off" | "stopped" | "expired" => Box::pin(f.followed_guest_with_policy(change)).await,
        _ => Box::pin(f.followed_guest()).await,
    };
    let origin = guest.channel.as_ref().unwrap();
    let (token, row) = claim(
        &f.org.state,
        &f.org.org,
        &f.agent,
        &f.agent.model,
        false,
        WAIT,
        Some(origin),
    )
    .await
    .unwrap()
    .unwrap();
    let db = &f.org.state.db;
    assert!(row.guest_model_setup && !row.guest_turn);
    assert_eq!(
        super::org_channel_service::follow_conversation(db, &row, origin, None)
            .await
            .unwrap(),
        guest.id
    );
    assert_eq!(
        db.collection::<Document>(crate::models::org_channel::LANES)
            .count_documents(doc! {"conversation_id":&row.id})
            .await
            .unwrap(),
        0
    );
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(row.id.clone()),
        agent_id: None,
        text: INSTRUCTION.into(),
        model: None,
        access_mode: None,
    });
    start.origin = TurnOrigin::Event;
    start.org_access = super::org_channel_service::conversation_access(db, &row)
        .await
        .unwrap();
    let ids = engine::OwnerTurnIds {
        artifact_budget: Default::default(),
        conversation_id: row.id.clone(),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert_eq!(
        super::org_channel_service::follow_conversation(db, &row, origin, Some(&mut tx))
            .await
            .unwrap(),
        guest.id
    );
    let event = team::bound_event(KIND, INSTRUCTION.into(), None, None);
    engine::append_events_in_session(db, &row.user_id, &row.id, &[event], &mut tx)
        .await
        .unwrap();
    let active = Box::pin(engine::begin_owner_turn_in_session(
        db,
        &mut tx,
        &f.org.state.encryption_keys,
        &row.user_id,
        &start,
        &ids,
        &f.agent,
        None,
        None,
        None,
    ))
    .await
    .unwrap()
    .into_started()
    .unwrap()
    .conversation;
    tx.commit_transaction().await.unwrap();
    let turn = active.active_turn.as_ref().unwrap().turn_id.as_str();
    Box::pin(fence_dispatch(
        db,
        &row.user_id,
        &row.id,
        turn,
        &row.credential_api_key_id,
    ))
    .await
    .unwrap();
    let key = f.org.key(&row).await;
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        db, &key, None, None, None,
    ))
    .await
    .unwrap();
    assert_eq!(auth.user_id.to_string(), f.org.org);
    assert!(!key.allow_all_services && !key.allow_all_nodes && key.allowed_service_ids.is_empty());
    match change {
        "none" | "flag_off" | "stopped" | "expired" => {}
        "guests_off" => {
            let channel = super::org_channel_service::management::load(
                db,
                &f.org.admin,
                &origin.nyxbot_channel_id,
                true,
            )
            .await
            .unwrap();
            super::org_channel_service::management::update(
                db,
                &f.org.admin,
                &channel,
                super::org_channel_service::management::GuestSettings {
                    expected_revision: None,
                    enabled: false,
                    service_ids: None,
                    daily_credits: None,
                },
            )
            .await
            .unwrap();
        }
        "binding" => {
            db.collection::<Document>("nyxbot_channels")
                .update_one(
                    doc! {"_id":&origin.nyxbot_channel_id},
                    doc! {"$set":{"org_specialist.revision":uuid::Uuid::new_v4().to_string()}},
                )
                .await
                .unwrap();
        }
        "follow_stopped" => {
            let binding = origin.thread.as_ref().unwrap();
            super::channel_thread_follow_service::stop(
                db,
                &f.org.admin,
                &origin.nyxbot_channel_id,
                "parent",
                &binding.child_id,
            )
            .await
            .unwrap();
        }
        "sender" => {
            db.collection::<Document>("channel_messages")
                .update_one(
                    doc! {"_id":"source"},
                    doc! {"$set":{"sender_platform_id":"different-sender"}},
                )
                .await
                .unwrap();
        }
        "member_link" => {
            f.link(&f.org.member, "fixture-sender").await;
        }
        _ => panic!("unknown test case"),
    }
    let guest_allowed = super::org_channel_service::conversation_access(db, &guest)
        .await
        .is_ok();
    let setup_allowed = super::org_channel_service::conversation_access(db, &row)
        .await
        .is_ok();
    assert_eq!(
        setup_allowed, guest_allowed,
        "setup must have admitted guest authority: {change}"
    );
    if matches!(
        change,
        "none" | "flag_off" | "stopped" | "expired" | "follow_stopped"
    ) {
        assert!(guest_allowed, "{change}");
        Box::pin(fence_dispatch(
            db,
            &row.user_id,
            &row.id,
            turn,
            &row.credential_api_key_id,
        ))
        .await
        .unwrap();
    } else {
        assert!(
            super::org_channel_service::conversation_access(db, &row)
                .await
                .is_err(),
            "{change}"
        );
        assert!(
            Box::pin(fence_dispatch(
                db,
                &row.user_id,
                &row.id,
                turn,
                &row.credential_api_key_id
            ))
            .await
            .is_err(),
            "{change}"
        );
        run(f.org.state.clone(), token, row.clone(), WAIT).await;
        let failed = record(db, &f.org.org, &f.agent.id, &f.agent.model)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.get_str("status"), Ok("failed"));
        assert_eq!(failed.get_str("reason"), Ok("model_connection_unavailable"));
        assert_eq!(failed.get_str("failure_step"), Ok("org_access"));
        let error = failure(db, &guest).await;
        assert_eq!(
            error
                .metadata
                .as_ref()
                .unwrap()
                .failure_reason
                .as_ref()
                .unwrap()
                .guest_setup_step,
            Some(GuestSetupStep::OrgAccess)
        );
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        super::assistant_turn_observability::failed_in_session(
            db,
            &mut tx,
            &guest,
            guest.active_turn.as_ref().unwrap(),
            &error,
            Utc::now(),
        )
        .await
        .unwrap();
        tx.commit_transaction().await.unwrap();
        let audit = db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(
                doc! {"event_type":"assistant_turn_failed","event_data.conversation_id":&guest.id},
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            audit
                .get_document("event_data")
                .unwrap()
                .get_document("failure_reason")
                .unwrap()
                .get_str("guest_setup_step"),
            Ok("org_access")
        );
        assert_eq!(
            db.collection::<Document>("assistant_conversations")
                .count_documents(doc! {"_id":&row.id})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>("assistant_agent_credentials")
                .count_documents(doc! {"conversation_id":&row.id})
                .await
                .unwrap(),
            0
        );
    }
    db.drop().await.unwrap();
}

#[cfg(test)]
#[tokio::test]
async fn org_follow_setup_uses_originating_lane_in_admission_and_dispatch() {
    Box::pin(org_follow_setup_revocation("none")).await;
}
#[cfg(test)]
#[tokio::test]
async fn org_follow_setup_revokes_when_guests_disabled() {
    Box::pin(org_follow_setup_revocation("guests_off")).await;
}
#[cfg(test)]
#[tokio::test]
async fn org_follow_setup_revokes_when_binding_changes() {
    Box::pin(org_follow_setup_revocation("binding")).await;
}
#[cfg(test)]
#[tokio::test]
async fn org_follow_setup_preserves_admitted_guest_authority_when_follow_stops() {
    Box::pin(org_follow_setup_revocation("follow_stopped")).await;
}
#[cfg(test)]
#[tokio::test]
async fn org_follow_setup_revokes_when_sender_changes() {
    Box::pin(org_follow_setup_revocation("sender")).await;
}
#[cfg(test)]
#[tokio::test]
async fn org_follow_setup_revokes_when_sender_becomes_member() {
    Box::pin(org_follow_setup_revocation("member_link")).await;
}

#[cfg(test)]
#[test]
fn guest_setup_step_is_optional_and_unknown_codes_remain_bounded() {
    use crate::models::assistant_message::TurnFailureReason;
    let old = serde_json::json!({"source":"guest_setup","code":"model_connection_unavailable","stage":"guest_setup","http_status":null,"nyxid_error_code":null});
    let reason: TurnFailureReason = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(reason.guest_setup_step, None);
    let mut future = old;
    future["guest_setup_step"] = serde_json::json!("future_closed_step");
    let reason: TurnFailureReason = serde_json::from_value(future).unwrap();
    assert_eq!(reason.guest_setup_step, Some(GuestSetupStep::Unknown));
    assert_eq!(
        serde_json::to_value(reason).unwrap()["guest_setup_step"],
        "unknown"
    );
}

#[cfg(test)]
#[tokio::test]
async fn cold_org_setup_admits_mention_when_thread_follow_flag_is_off() {
    Box::pin(org_follow_setup_revocation("flag_off")).await;
}
#[cfg(test)]
#[tokio::test]
async fn cold_org_setup_admits_all_replies_with_stopped_child() {
    Box::pin(org_follow_setup_revocation("stopped")).await;
}
#[cfg(test)]
#[tokio::test]
async fn cold_org_setup_admits_all_replies_with_expired_child() {
    Box::pin(org_follow_setup_revocation("expired")).await;
}
