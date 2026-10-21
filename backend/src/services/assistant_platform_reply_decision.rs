//! Shared owner reply decisions. Transport-specific authority is admitted by
//! the caller; the decision body can join its binding and ingress transaction.
use mongodb::Database;

use crate::{
    errors::AppResult,
    models::assistant_acknowledgement::AssistantAcknowledgement,
    services::{assistant_acknowledgement_service as acks, assistant_nyxagent as engine},
};

/// Exact provider-authenticated human sender and the route/chat it entered.
/// This is transport evidence, not a substitute for the transaction's live fences.
pub(crate) struct OwnerReplyAuthority {
    channel_id: String,
    user_id: String,
    channel_bot_id: String,
    bot_owner_id: Option<String>,
    platform: String,
    transport: String,
    route_api_key_id: String,
    route_id: Option<String>,
    agent_api_key_id: Option<String>,
    binding_id: Option<String>,
    follow_binding_generation: i64,
    chat_id: String,
    partition: String,
    platform_chat_id: Option<String>,
    platform_thread_id: Option<String>,
    sender_id: String,
}

impl OwnerReplyAuthority {
    pub(crate) fn verified_sender(
        channel: &crate::models::nyxbot_channel::NyxbotChannel,
        chat: &crate::models::nyxbot_channel::NyxbotThread,
        kind: crate::models::channel_thread::ThreadSenderKind,
        owner_eligible: bool,
        sender_id: &str,
    ) -> AppResult<Self> {
        if kind != crate::models::channel_thread::ThreadSenderKind::Human
            || !owner_eligible
            || sender_id.is_empty()
            || !channel.owner_sender_ids.iter().any(|id| id == sender_id)
            || chat.channel_id != channel.id
            || chat.user_id != channel.user_id
        {
            return Err(crate::errors::AppError::Forbidden(
                "Verified channel owner required".into(),
            ));
        }
        Ok(Self {
            channel_id: channel.id.clone(),
            user_id: channel.user_id.clone(),
            channel_bot_id: channel.channel_bot_id.clone(),
            bot_owner_id: channel.bot_owner_id.clone(),
            platform: channel.platform.clone(),
            transport: channel.transport.clone(),
            route_api_key_id: channel.route_api_key_id.clone(),
            route_id: channel.route_id.clone(),
            agent_api_key_id: channel.agent_api_key_id.clone(),
            binding_id: channel.binding_id.clone(),
            follow_binding_generation: channel.follow_binding_generation,
            chat_id: chat.id.clone(),
            partition: chat.partition.clone(),
            platform_chat_id: chat.platform_chat_id.clone(),
            platform_thread_id: chat.platform_thread_id.clone(),
            sender_id: sender_id.to_owned(),
        })
    }

    async fn fence(
        &self,
        db: &Database,
        session: &mut mongodb::ClientSession,
        conversation_id: &str,
    ) -> AppResult<()> {
        use crate::{
            errors::AppError,
            models::{api_key, channel_bot, nyxbot_channel, user},
        };
        use mongodb::bson::{Document, doc};
        let unavailable = || AppError::Forbidden("Channel decision authority changed".into());
        let row = self;
        let bot_owner = row.bot_owner_id.as_deref().unwrap_or(&row.user_id);
        // Exact writes conflict with unlink/rebinding/revocation rather than
        // accepting a cached owner boolean from transport admission.
        let mut channel_filter = doc! {"_id":&row.channel_id,"user_id":&row.user_id,
        "channel_bot_id":&row.channel_bot_id,"bot_owner_id":&row.bot_owner_id,
        "platform":&row.platform,"transport":&row.transport,"status":"active",
        "route_api_key_id":&row.route_api_key_id,"route_id":&row.route_id,
        "agent_api_key_id":&row.agent_api_key_id,"binding_id":&row.binding_id,
        "follow_binding_generation":row.follow_binding_generation,
        "owner_sender_ids":&self.sender_id};
        // Missing generation is the model's legacy zero; later generations
        // require their exact persisted integer.
        if row.follow_binding_generation == 0 {
            channel_filter.remove("follow_binding_generation");
            channel_filter.insert("$or", bson::bson!([{ "follow_binding_generation":0_i64 },{ "follow_binding_generation":{ "$exists":false }}]));
        }
        db.collection::<Document>(nyxbot_channel::COLLECTION_NAME)
            .find_one_and_update(channel_filter, doc! {"$inc":{"decision_fence":1_i64}})
            .session(&mut *session)
            .await?
            .ok_or_else(unavailable)?;
        db.collection::<Document>(nyxbot_channel::THREADS_COLLECTION_NAME)
            .find_one_and_update(doc! {"_id":&self.chat_id,"channel_id":&row.channel_id,"user_id":&row.user_id,
                "partition":&self.partition,"conversation_id":conversation_id,
                "platform_chat_id":&self.platform_chat_id,"platform_thread_id":&self.platform_thread_id},
                doc! {"$inc":{"decision_fence":1_i64}})
            .session(&mut *session).await?.ok_or_else(unavailable)?;
        super::platform_bot_authority::fence_personal_routes(
            db,
            &row.channel_bot_id,
            bot_owner,
            session,
        )
        .await?;
        db.collection::<Document>(channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":&row.channel_bot_id,"user_id":bot_owner,"is_active":true})
            .session(&mut *session)
            .await?
            .ok_or_else(unavailable)?;
        db.collection::<Document>(api_key::COLLECTION_NAME)
            .find_one_and_update(
                doc! {"_id":&row.route_api_key_id,"user_id":bot_owner,"is_active":true,
                "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
                doc! {"$inc":{"decision_fence":1_i64}},
            )
            .session(&mut *session)
            .await?
            .ok_or_else(unavailable)?;
        db.collection::<Document>(user::COLLECTION_NAME)
            .find_one(doc! {"_id":&row.user_id,"user_type":"person","is_active":true})
            .session(&mut *session)
            .await?
            .ok_or_else(unavailable)?;
        if !super::org_service::resolve_owner_access_in_session(
            db,
            session,
            &row.user_id,
            bot_owner,
        )
        .await?
        .can_write()
        {
            return Err(unavailable());
        }
        Ok(())
    }
}

/// Both ordinary relay and gateway/thread-follow routes retain their exact
/// authenticated sender. The route key never becomes the human audit actor.
pub(crate) async fn decide_owner_reply(
    db: &Database,
    runtime: &super::audit_runtime::AuditRuntime,
    authority: &OwnerReplyAuthority,
    conversation_id: &str,
    text: &str,
) -> AppResult<Option<AssistantAcknowledgement>> {
    if acks::parse_reply(text).is_none() {
        return Ok(None);
    }
    let principal = crate::models::audit_outbox::AuditPrincipal::Person {
        id: authority.user_id.clone(),
    };
    let audit = acks::DecisionAudit::owner_channel(runtime, &principal)?;
    let context = (db, authority, conversation_id, text, audit);
    super::audit_runtime::transaction::run(db, &context, |session, context| Box::pin(async move {
        use crate::models::assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES};
        use mongodb::bson::doc;
        let (db, authority, conversation_id, text, audit) = context;
        authority.fence(db, session, conversation_id).await?;
        let owner = &authority.user_id;
        let since = db.collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"user_id":owner,"conversation_id":*conversation_id,"role":"user","steering":bson::Bson::Null,"execution_pending":{"$ne":true}})
            .sort(doc! {"seq":-1}).session(&mut *session).await?.map(|message|message.created_at);
        acks::decide_reply_in_session(db,session,audit,owner,&[conversation_id.to_string()],text,since).await
    })).await
}

/// Original-command eligibility is transport authenticated. This transaction
/// also compares its exact ingress provenance in the receipt producer.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn decide_personal_reply_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    audit: &acks::DecisionAudit<'_>,
    identity: &dyn super::platform_manager_contract::ManagerIdentityProducer,
    ingress: &dyn super::platform_manager_contract::ManagerIngressTurnStore,
    admitted: &super::platform_manager_contract::AdmittedPersonalUpdate,
    conversation_id: &str,
    command: &super::platform_manager_contract::OriginalManagerCommand,
) -> AppResult<super::platform_manager_contract::PlatformDecisionOutcome> {
    use super::platform_manager_contract as contract;
    use crate::errors::AppError;
    use crate::models::{
        assistant_conversation::AssistantConversation, assistant_message::AssistantMessage,
    };
    use mongodb::bson::doc;
    let authority = &admitted.authority;
    let update = command.update();
    if update.chat_kind != contract::ManagerChatKind::Private
        || update.command_eligibility != contract::ManagerCommandEligibility::OriginalPrivateMessage
        || update.profile_id != authority.manager.profile_id
        || update.manager_bot_id != authority.manager.manager_bot_id
        || update.entry != authority.manager.entry
        || update.sender != authority.sender
    {
        return Ok(contract::PlatformDecisionOutcome::Unavailable(
            contract::UnavailableReason::OriginRevoked,
        ));
    }
    if acks::parse_reply(command.text()).is_none() {
        return Ok(contract::PlatformDecisionOutcome::NoPendingDecision);
    }
    if !matches!(
        identity
            .resolve_thread_authority(
                db,
                session,
                &authority.user_id,
                &authority.binding_id,
                authority.binding_generation,
                conversation_id,
            )
            .await?,
        contract::ThreadAuthorityResolution::Personal(resolved) if *resolved == *authority
    ) {
        return Ok(contract::PlatformDecisionOutcome::Unavailable(
            contract::UnavailableReason::OriginRevoked,
        ));
    }
    // Conflicts with concurrent owner input/rotation/deletion while keeping
    // the plain yes/no previous-message boundary in this same snapshot.
    let row=db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one_and_update(doc! {"_id":conversation_id,"user_id":&authority.user_id,
            "platform_thread.binding_id":&authority.binding_id,"platform_thread.binding_generation":authority.binding_generation},
            doc! {"$inc":{"authority_fence":1_i64}}).session(&mut *session).await?.ok_or(AppError::AssistantTurnRequired)?;
    let messages =
        db.collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME);
    let since=messages.find_one(doc! {"conversation_id":conversation_id,"user_id":&authority.user_id,"role":"user","execution_pending":{"$ne":true}})
        .sort(doc! {"seq":-1}).session(&mut *session).await?.map(|message|message.created_at);
    let Some((ack, allowed)) = acks::select_reply_in_session(
        db,
        session,
        &authority.user_id,
        &[conversation_id.to_owned()],
        command.text(),
        since,
    )
    .await?
    else {
        return Ok(contract::PlatformDecisionOutcome::NoPendingDecision);
    };
    let Some(requested_turn_id) = ack.requested_turn_id.as_ref().filter(|id| !id.is_empty()) else {
        return Ok(contract::PlatformDecisionOutcome::NoPendingDecision);
    };
    if ack.authored_skill.is_some() || ack.api_key_id!=row.credential_api_key_id
        || messages.find_one(doc! {"conversation_id":conversation_id,"user_id":&authority.user_id,"turn_id":requested_turn_id})
            .session(&mut *session).await?.is_none() {
        return Ok(contract::PlatformDecisionOutcome::NoPendingDecision);
    }
    let ordinary_continuation = ack.voice_request_id.is_none() && ack.trigger_run_id.is_none();
    if ordinary_continuation && row.pending_events.len() >= engine::MAX_PENDING_EVENTS {
        return Err(AppError::Conflict("Assistant event queue is full".into()));
    }
    let mut decision = contract::PlatformCardDecision {
        acknowledgement_id: ack.id.clone(),
        requested_turn: contract::TurnRef {
            conversation_id: conversation_id.to_owned(),
            turn_id: requested_turn_id.clone(),
        },
        allowed,
        voice_request_id: ack.voice_request_id.clone(),
        continuation_receipt_id: ack.continuation_receipt_id.clone(),
        trigger_run_id: ack.trigger_run_id.clone(),
    };
    ingress
        .claim_personal_decision(db, session, admitted, command, &decision)
        .await?;
    let ack = acks::decide_in_session(
        db,
        session,
        audit,
        &authority.user_id,
        Some(conversation_id),
        &ack.id,
        allowed,
        acks::Decider::User,
        None,
        None,
    )
    .await?;
    if ordinary_continuation {
        let mut event = crate::models::assistant_acknowledgement::bind_continuation(
            ack.routing.as_ref(),
            super::assistant_team_service::event(
                "owner_action_decided",
                format!(
                    "The owner {} acknowledgement {}. {} {}",
                    if allowed { "allowed" } else { "denied" },
                    engine::identifier(&ack.id),
                    if allowed {
                        "Continue the task using this exact approved acknowledgement_id when required."
                    } else {
                        "Do not execute or retry the denied action; explain what was skipped."
                    },
                    acks::DECISION_NARRATION
                ),
                None,
            ),
        );
        event.id = ack.id.clone();
        if !engine::append_events_in_session(
            db,
            &authority.user_id,
            conversation_id,
            std::slice::from_ref(&event),
            session,
        )
        .await?
        {
            return Err(AppError::AssistantTurnRequired);
        }
    }
    decision.continuation_receipt_id = ack.continuation_receipt_id;
    ingress
        .settle_personal_decision(db, session, &admitted.ingress, &decision)
        .await?;
    Ok(contract::PlatformDecisionOutcome::Decided(decision))
}
