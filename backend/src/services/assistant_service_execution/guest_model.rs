//! Guests may reuse an owner-established model route, never choose one.
use super::*;
use crate::models::{
    assistant_agent::{AssistantAgent, OwnerModelRoute},
    assistant_conversation::AssistantConversation,
};

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
    // Held execution digests deliberately use a process-local credential MAC.
    // Durable routes instead bind producer-owned credential identities/epochs,
    // which survive replica changes and lazy OAuth token refreshes.
    let credential = if let Some(id) = connection {
        let authority =
            crate::services::proxy_service::read_connection_authority(db, &chat.user_id, id)
                .await?
                .ok_or(AppError::GuestModelRouteRequired)?;
        let override_key = crate::services::proxy_service::read_agent_credential_override_identity(
            db,
            &chat.user_id,
            &chat.api_key_id,
            id,
            target,
        )
        .await?;
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
    let row = conversation(db, chat).await?;
    let admitted = input(db, chat)
        .await?
        .ok_or(AppError::GuestModelRouteRequired)?;
    if operation
        .get("input_admission")
        .is_some_and(|value| value != &json!(admitted))
    {
        return Err(AppError::GuestModelRouteRequired);
    }
    operation["input_admission"] = json!(admitted);
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&chat.agent_id,"user_id":&chat.user_id,"destroyed_at":null})
        .await?
        .ok_or(AppError::GuestModelRouteRequired)?;
    if let Some(bound) = row.owner_model_route.as_deref().or_else(|| {
        agent
            .owner_model_route
            .as_deref()
            .filter(|bound| bound.profile == row.model)
    }) {
        if bound == &route(db, chat, &row.model, connection, target, operation).await? {
            return Ok(());
        }
    } else if super::guest_model_defaults::matches(
        db, chat, &row.model, connection, target, operation,
    )
    .await?
    {
        return Ok(());
    }
    Err(AppError::GuestModelRouteRequired)
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
    if chat.guest || !chat.turn_live || chat.turn_stopped || input(db, chat).await?.is_none() {
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
    let agent_filter = doc! {"_id":&chat.agent_id,"user_id":&chat.user_id,"destroyed_at":null};
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
            let mut filter = agent_filter.clone();
            filter.insert("model", profile);
            db.collection::<Document>("assistant_agents").update_one(filter,
                doc! {"$set":{"owner_model_route":value}}).session(tx).await?;
            Ok(())
        })
    }).await
}
