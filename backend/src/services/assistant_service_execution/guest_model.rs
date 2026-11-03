//! Guests may reuse an owner-established model route, never choose one.
use super::model_diagnostic::{Reason, refuse};
use super::*;
use crate::models::{
    assistant_agent::{AssistantAgent, OwnerModelRoute},
    assistant_conversation::AssistantConversation,
};

/// Shared recipients run their own model lane. They are deliberately neither
/// owner-route guests nor owners whose callbacks may establish a route.
pub(crate) fn is_shared_chat(chat: &acks::ChatAuthority) -> bool {
    chat.org_agent_access
        .as_ref()
        .is_some_and(|access| access.shared().is_some())
}

pub(crate) fn owner_model_route_allowed(chat: &acks::ChatAuthority) -> bool {
    !chat.guest && !is_shared_chat(chat)
}

async fn conversation(
    db: &Database,
    chat: &acks::ChatAuthority,
) -> AppResult<AssistantConversation> {
    let row = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,
        "agent_id":&chat.agent_id,"credential_api_key_id":&chat.api_key_id,
        "active_turn.turn_id":&chat.turn_id})
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    if row.guest_turn != chat.guest
        || super::super::assistant_nyxagent::live_turn(&row, Utc::now())
            .is_none_or(|turn| turn.stop_requested)
    {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(row)
}

async fn input(db: &Database, chat: &acks::ChatAuthority) -> AppResult<Option<String>> {
    let mut filter = scope(chat)?;
    filter.insert("kind", "input_admission");
    Ok(db
        .collection::<Document>(COLLECTION)
        .find_one(filter)
        .await?
        .and_then(|row| row.get_str("input_admission").ok().map(str::to_owned)))
}

pub(super) async fn route(
    db: &Database,
    chat: &acks::ChatAuthority,
    profile: &str,
    connection: Option<&str>,
    target: &crate::services::proxy_service::ProxyTarget,
    operation: &Value,
) -> AppResult<OwnerModelRoute> {
    route_for_actor(
        db,
        &chat.user_id,
        Some(&chat.api_key_id),
        profile,
        connection,
        target,
        operation,
    )
    .await
}

pub(crate) async fn route_for_actor(
    db: &Database,
    actor: &str,
    api_key: Option<&str>,
    profile: &str,
    connection: Option<&str>,
    target: &crate::services::proxy_service::ProxyTarget,
    operation: &Value,
) -> AppResult<OwnerModelRoute> {
    // Held execution digests deliberately use a process-local credential MAC.
    // Durable routes instead bind producer-owned credential identities/epochs,
    // which survive replica changes and lazy OAuth token refreshes.
    let credential = if let Some(id) = connection {
        let authority = crate::services::proxy_service::read_connection_authority(db, actor, id)
            .await?
            .ok_or(AppError::GuestModelRouteRequired)?;
        let override_key = if let Some(api_key) = api_key {
            crate::services::proxy_service::read_agent_credential_override_identity(
                db, actor, api_key, id, target,
            )
            .await?
        } else {
            None
        };
        json!({"key":authority.api_key_id,"epoch":authority.credential_epoch,
            "master":authority.master_credential,"revision":authority.master_credential_revision,
            "override":override_key.map(|key| (key.api_key_id, key.credential_epoch))})
    } else {
        // The catalog master credential route is identified by its catalog
        // service (bound below); never derive identity from credential material.
        json!({"master": true})
    };
    Ok(OwnerModelRoute {
        profile: profile.to_owned(),
        catalog_service_id: target.service.id.clone(),
        connection_id: connection.map(str::to_owned),
        operation: selection_fingerprint(operation),
        execution: digest(
            json!({"version":1,"target":execution_projection_digest(target, false),
            "credential":credential}),
        ),
    })
}

/// The admitted input token bounds model rounds to the same server-started
/// turn, just as for owners. Provider payloads necessarily include evolving
/// context/tool results; this is not a claim that raw prompt bytes are equal.
pub(super) async fn require(
    db: &Database,
    chat: &acks::ChatAuthority,
    connection: Option<&str>,
    target: &crate::services::proxy_service::ProxyTarget,
    operation: &mut Value,
) -> AppResult<()> {
    if is_shared_chat(chat) {
        return refuse(db, chat, Reason::SharedChat).await;
    }
    let row = conversation(db, chat).await?;
    let Some(admitted) = input(db, chat).await? else {
        return refuse(db, chat, Reason::NoAdmission).await;
    };
    if operation
        .get("input_admission")
        .is_some_and(|value| value != &json!(admitted))
    {
        return refuse(db, chat, Reason::NoAdmission).await;
    }
    operation["input_admission"] = json!(admitted);
    let Some(agent) = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&chat.agent_id,"user_id":row.agent_owner_id.as_deref().unwrap_or(&chat.user_id),"destroyed_at":null})
        .await?
    else {
        return refuse(db, chat, Reason::AgentUnavailable).await;
    };
    if let Some(bound) = crate::services::assistant_guest_model_setup::recorded_route(
        db,
        &chat.user_id,
        &chat.agent_id,
        &row.model,
    )
    .await?
        && bound == route(db, chat, &row.model, connection, target, operation).await?
    {
        return Ok(());
    }
    if let Some(bound) = row.owner_model_route.as_deref().or_else(|| {
        agent
            .owner_model_route
            .as_deref()
            .filter(|bound| bound.profile == row.model)
    }) {
        let actual = match route(db, chat, &row.model, connection, target, operation).await {
            Ok(route) => route,
            Err(AppError::GuestModelRouteRequired) => {
                return refuse(db, chat, Reason::ConnectionNotPlatform).await;
            }
            Err(error) => return Err(error),
        };
        if bound == &actual {
            return Ok(());
        }
        return refuse(db, chat, Reason::RouteMismatch).await;
    }
    match super::guest_model_defaults::matches(db, chat, &row.model, connection, target, operation)
        .await?
    {
        None => Ok(()),
        Some(reason) => refuse(db, chat, reason).await,
    }
}

/// Only normal owner model admission establishes a route. Each thread retains
/// its exact owner-established route; matching agent profiles also establish
/// the shared default for new threads. Guests never learn from their callback.
pub(super) async fn remember_owner(
    db: &Database,
    chat: &acks::ChatAuthority,
    connection: Option<&str>,
    target: &crate::services::proxy_service::ProxyTarget,
    operation: &Value,
) -> AppResult<()> {
    if !owner_model_route_allowed(chat)
        || !chat.turn_live
        || chat.turn_stopped
        || input(db, chat).await?.is_none()
    {
        return Ok(());
    }
    // Ordinary raw inference and tool calls outside the engine's model attempt
    // retain their behavior but cannot establish the agent's runtime route.
    let mut attempt = scope(chat)?;
    attempt.extend(doc! {"kind":"model_attempt","active":true});
    if db
        .collection::<Document>(COLLECTION)
        .find_one(attempt)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let row = conversation(db, chat).await?;
    let agent_filter = doc! {"_id":&chat.agent_id,"user_id":row.agent_owner_id.as_deref().unwrap_or(&chat.user_id),"destroyed_at":null};
    let Some(agent) = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(agent_filter.clone())
        .await?
    else {
        return Ok(());
    };
    // Preserve deliberate per-thread profiles and legacy channel defaults.
    // A different profile establishes only this thread's route, not the agent's.
    let bound = route(db, chat, &row.model, connection, target, operation).await?;
    if row.owner_model_route.as_deref() == Some(&bound)
        && (agent.model != row.model || agent.owner_model_route.as_deref() == Some(&bound))
    {
        return Ok(());
    }
    let value = bson::to_bson(&bound)
        .map_err(|_| AppError::Internal("Model route binding failed".into()))?;
    let context = (db.clone(), chat.clone(), agent_filter, row.model, value);
    crate::services::transaction_service::run_db_only(db, &context, |tx, (db, chat, agent_filter, profile, value)| {
        Box::pin(async move {
            crate::services::erasure_admission::fence_in_session(db, &chat.user_id, tx).await?;
            let live = db.collection::<Document>("assistant_conversations").update_one(
                doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"agent_id":&chat.agent_id,
                    "credential_api_key_id":&chat.api_key_id,"model":profile,"active_turn.turn_id":&chat.turn_id,
                    "guest_turn":false,"active_turn.stop_requested":false},
                doc! {"$inc":{"owner_model_route_fence":1_i64},"$set":{"owner_model_route":value}}).session(&mut *tx).await?;
            if live.matched_count != 1 { return Err(AppError::AssistantTurnRequired); }
            // An organization member's exact model identity belongs to that
            // acting person, never an org-wide default for other members.
            if agent_filter.get_str("user_id")==Ok(chat.user_id.as_str()) {
                let mut filter = agent_filter.clone();
                filter.insert("model", profile);
                db.collection::<Document>("assistant_agents").update_one(filter,
                    doc! {"$set":{"owner_model_route":value}}).session(tx).await?;
            }
            Ok(())
        })
    }).await
}
