//! Shared owner reply decisions. Transport-specific authority is admitted by
//! the caller; the decision body can join its binding and ingress transaction.
use mongodb::Database;

use crate::{
    errors::AppResult,
    models::assistant_acknowledgement::AssistantAcknowledgement,
    services::{
        api_key_mutation_service as transactions, assistant_acknowledgement_service as acks,
        assistant_nyxagent as engine, audit_service,
    },
};

/// Ordinary channel wrapper: preserve same-thread previous-message selection,
/// decision effects and metadata-only audit without constructing HTTP auth.
pub(crate) async fn decide_owner_reply(
    db: &Database,
    owner: &str,
    conversation_id: &str,
    text: &str,
) -> AppResult<Option<AssistantAcknowledgement>> {
    if acks::parse_reply(text).is_none() {
        return Ok(None);
    }
    let since = engine::previous_user_message(db, owner, conversation_id, None)
        .await?
        .map(|message| message.created_at);
    let mut session = db.client().start_session().await?;
    let decision_db = db.clone();
    let decision_owner = owner.to_owned();
    let conversation_ids = vec![conversation_id.to_owned()];
    let text = text.to_owned();
    let decided = session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(acks::decide_reply_in_session(
                &decision_db,
                session,
                &decision_owner,
                &conversation_ids,
                &text,
                since,
            ))
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(decided) = &decided {
        acks::audit_decision(
            db,
            &audit_service::AuditActor {
                user_id: owner.to_owned(),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
            },
            decided,
        )
        .await;
    }
    Ok(decided)
}

/// Original-command eligibility is transport authenticated. This transaction
/// also compares its exact ingress provenance in the receipt producer.
pub(crate) async fn decide_personal_reply_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
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
        || update.telegram_bot_id != authority.manager.telegram_bot_id
        || update.sender_id != authority.sender_id
        || update.chat_id != authority.chat_id
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
        contract::ThreadAuthorityResolution::Personal(resolved) if resolved == *authority
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
        let queued = db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(doc! {"_id":conversation_id,"user_id":&authority.user_id,
                "platform_thread.binding_id":&authority.binding_id,"platform_thread.binding_generation":authority.binding_generation,
                "pending_events.id":{"$ne":&event.id}},
                doc! {"$push":{"pending_events":mongodb::bson::to_bson(&event)
                    .map_err(|_| AppError::Internal("Assistant event serialization failed".into()))?}})
            .session(&mut *session).await?;
        if queued.matched_count != 1 {
            return Err(AppError::AssistantTurnRequired);
        }
    }
    decision.continuation_receipt_id = ack.continuation_receipt_id;
    ingress
        .settle_personal_decision(db, session, &admitted.ingress, &decision)
        .await?;
    Ok(contract::PlatformDecisionOutcome::Decided(decision))
}
