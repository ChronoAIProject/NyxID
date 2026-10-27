//! Recovery can select only a server-associated message. It never executes a service.
use super::*;
use crate::models::assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES};
use crate::services::assistant_acknowledgement_service::ChatAuthority;

pub fn recovery_definition() -> super::super::mcp_service::McpToolDefinition {
    super::super::mcp_service::McpToolDefinition {
        name: "nyx__artifact_delivery".into(),
        description: "Recover the verified cloud result attached to the latest server-associated assistant reply before this owner request in this Web conversation. The server selects the saved result; no identifiers or arguments are accepted. Reuses retained bytes without submitting another execution. Only claim delivery after success.".into(),
        input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false,"maxProperties":0}),
    }
}

/// The HTTP path requires an idle owner conversation and an exact message URL.
pub async fn recover(
    db: &Database,
    keys: &EncryptionKeys,
    user: &str,
    conversation: &str,
    message: &str,
) -> AppResult<()> {
    run(db, keys, user, conversation, Some(message.to_owned()), None).await
}

/// Natural-language recovery takes no client/model-selected lineage. The live
/// owner Web turn and latest preceding server-associated assistant message select it.
pub async fn recover_for_chat(
    db: &Database,
    keys: &EncryptionKeys,
    chat: &ChatAuthority,
    args: &serde_json::Value,
) -> AppResult<()> {
    if !args.as_object().is_some_and(|args| args.is_empty())
        || chat.guest
        || chat.platform_thread
        || chat.platform_origin.is_some()
        || chat.invocation_controlled
        || chat.invocation_id.is_some()
        || chat.channel_adapter_origin.is_some()
        || chat.org_agent_access.is_some()
    {
        return Err(invalid());
    }
    chat.ensure_live_turn(db, None, None, "artifact_delivery")
        .await?;
    run(
        db,
        keys,
        &chat.user_id,
        &chat.conversation_id,
        None,
        Some(chat.clone()),
    )
    .await
}

/// Confirmation continuation is selected from the server-written initiating
/// message, so this no-argument action never accepts an acknowledgement ID.
pub async fn recovery_confirmation_arguments(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<serde_json::Value> {
    chat.ensure_live_turn(db, None, None, "artifact_delivery")
        .await?;
    let row =
        super::super::assistant_nyxagent::get(db, &chat.user_id, &chat.conversation_id).await?;
    let mut session = db.client().start_session().await?;
    let (_, acknowledgement) = initiating_message(db, &row, chat, &mut session).await?;
    Ok(match acknowledgement {
        Some(id) => serde_json::json!({"acknowledgement_id":id}),
        None => serde_json::json!({}),
    })
}

async fn initiating_message(
    db: &Database,
    row: &AssistantConversation,
    chat: &ChatAuthority,
    session: &mut ClientSession,
) -> AppResult<(AssistantMessage, Option<String>)> {
    super::super::assistant_access_policy::require_same_live_turn(chat, row)?;
    let turn = row.active_turn.as_ref().ok_or_else(invalid)?;
    if row.credential_api_key_id != chat.api_key_id
        || row.agent_id.as_deref() != Some(chat.agent_id.as_str())
        || row.guest_turn
        || row.channel.is_some()
        || row.reply_channel.is_some()
        || row.group_request_id.is_some()
        || turn.origin != TurnOrigin::User
        // Ordinary owner Web admission stores no platform reply destination;
        // async completion delivery stores an explicit WebOnly destination.
        || !matches!(turn.platform_reply, None | Some(PlatformReplyRouting::WebOnly))
        || turn.platform_origin.is_some()
        || turn.channel_adapter_origin.is_some()
        || turn.invocation_id.is_some()
        || turn.voice_request_id.is_some()
        || turn.trigger_run_id.is_some()
        || turn.asked_from.is_some()
        || !turn.also_deliver.is_empty()
    {
        return Err(invalid());
    }
    let seq = turn.initiating_message_seq.ok_or_else(invalid)?;
    // Authorship is authoritative; a guest/event cannot borrow a user's origin.
    let initiating = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(
            doc! {"user_id":&row.user_id,"conversation_id":&row.id,"seq":seq,
            "turn_id":&turn.turn_id,"role":"user","origin":"user","author":"owner"},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(invalid)?;
    let Some(acknowledgement) = initiating.continuation_of.as_deref() else {
        return Ok((initiating, None));
    };
    use crate::models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKS,
    };
    let card = db.collection::<AssistantAcknowledgement>(ACKS).find_one(doc! {
        "_id":acknowledgement,"user_id":&chat.user_id,"conversation_id":&row.id,
        "api_key_id":&chat.api_key_id,"kind":"action","tool_name":"nyx__artifact_delivery",
        "arguments_digest":super::super::assistant_acknowledgement_service::arguments_digest(&serde_json::json!({})),
        "status":{"$in":["allowed","used"]},"decider":"user","decided_by":"user",
        "invocation_id":null,"expires_at":{"$gt":bson::DateTime::now()},
    }).session(&mut *session).await?.ok_or_else(invalid)?;
    let original = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "user_id":&row.user_id,"conversation_id":&row.id,"role":"user","origin":"user",
            "author":"owner","turn_id":card.requested_turn_id.as_deref().ok_or_else(invalid)?,
            "seq":{"$lt":seq},
        })
        .session(&mut *session)
        .await?
        .ok_or_else(invalid)?;
    Ok((original, Some(acknowledgement.to_owned())))
}

async fn select_message(
    db: &Database,
    row: &AssistantConversation,
    chat: &ChatAuthority,
    session: &mut ClientSession,
) -> AppResult<String> {
    let (initiating, _) = initiating_message(db, row, chat, session).await?;
    // Joining the server-owned association excludes unrelated assistant replies.
    // Do not filter expiry here: an expired newest result must report expiry,
    // never silently select older bytes. Confirmation uses the original request.
    let mut cursor = db.collection::<Document>(MESSAGES).aggregate(vec![
        doc! {"$match":{"user_id":&row.user_id,"conversation_id":&row.id,"role":"assistant","seq":{"$lt":initiating.seq}}},
        doc! {"$sort":{"seq":-1}},
        doc! {"$lookup":{"from":DELIVERIES,"localField":"_id","foreignField":"_id","as":"saved_delivery"}},
        doc! {"$match":{"saved_delivery":{"$elemMatch":{"user_id":&row.user_id,"conversation_id":&row.id}}}},
        doc! {"$limit":1},
        doc! {"$project":{"_id":1}},
    ]).session(&mut *session).await?;
    let source = cursor
        .stream(&mut *session)
        .try_next()
        .await?
        .ok_or_else(missing)?;
    source
        .get_str("_id")
        .map(str::to_owned)
        .map_err(|_| invalid())
}

async fn run(
    db: &Database,
    keys: &EncryptionKeys,
    user: &str,
    conversation: &str,
    message: Option<String>,
    chat: Option<ChatAuthority>,
) -> AppResult<()> {
    let database = db.clone();
    let user = user.to_owned();
    let conversation = conversation.to_owned();
    let mut session = db.client().start_session().await?;
    // Pass the borrowed keys through the driver's context API instead of
    // capturing them in an and_run2 async closure (mongodb's documented HRTB bug).
    session.start_transaction().and_run(
        (database, keys, user, conversation, message, chat),
        |session, context| {
        let (database, keys, user, conversation, message, chat) = context.clone();
        Box::pin(async move {
        let operation: AppResult<()> = Box::pin(async {
            super::super::erasure_admission::fence_in_session(&database, &user, session).await?;
            // This write conflicts with Stop, a new turn, and conversation deletion.
            let row = database.collection::<AssistantConversation>(CONVERSATIONS)
                .find_one_and_update(doc! {"_id":&conversation,"user_id":&user,"group_id":null},
                    doc! {"$inc":{"async_operation_generation":1}})
                .session(&mut *session).await?.ok_or_else(missing)?;
            let message = match &chat {
                Some(chat) => select_message(&database, &row, chat, session).await?,
                None => {
                    if row.active_turn.is_some() { return Err(AppError::AssistantTurnActive); }
                    message.clone().ok_or_else(missing)?
                }
            };
            let delivery = database.collection::<Delivery>(DELIVERIES)
                .find_one(doc! {"_id":&message,"user_id":&user,"conversation_id":&conversation})
                .session(&mut *session).await?.ok_or_else(missing)?;
            if delivery.expires_at <= Utc::now() { return Err(AppError::AssistantAttachmentExpired); }
            let source = database.collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":&message,"user_id":&user,"conversation_id":&conversation,
                    "role":"assistant","turn_id":&delivery.event_turn_id})
                .session(&mut *session).await?.ok_or_else(missing)?;
            if source.origin != Some(TurnOrigin::Event) || !matches!(source.status.as_str(), "completed" | "failed" | "stopped") {
                return Err(invalid());
            }
            let w = database.collection::<AsyncServiceOperation>(WATCHES)
                .find_one_and_update(doc! {"_id":&delivery.watch_id,"user_id":&user,"conversation_id":&conversation,
                    "turn_id":&delivery.originating_turn_id,"operation_id":&delivery.operation_id,
                    "artifact_result.source_result_identity":&delivery.source_result_identity,
                    "artifact_event_turn_id":&delivery.event_turn_id,"state":{"$in":["queued","delivered"]}},
                    doc! {"$inc":{"artifact_recovery_generation":1}})
                .session(&mut *session).await?.ok_or_else(missing)?;
            if w.artifact_result.as_ref().is_none_or(|m| m.expires_at <= Utc::now()) {
                return Err(AppError::AssistantAttachmentExpired);
            }
            let lineage: Vec<String> = source.attachments.iter()
                .filter(|a| a.artifact.source_watch_id.as_deref() == Some(w.id.as_str()))
                .map(|a| a.id.clone()).collect();
            if lineage != delivery.attachment_ids { return Err(invalid()); }
            let (attachments, reason) = materialize(&database, keys, &w, session,
                &delivery.event_turn_id, &source.attachments).await?;
            if reason != "materialized" { return Err(AppError::Conflict(reason.into())); }
            let mut merged = source.attachments;
            for attachment in attachments {
                if !merged.iter().any(|a| a.id == attachment.id) { merged.push(attachment); }
            }
            let expiry = attachment_expiry_in_session(&database, session, &w,
                &delivery.event_turn_id, &merged).await?.min(delivery.expires_at);
            if expiry <= Utc::now() { return Err(AppError::AssistantAttachmentExpired); }
            database.collection::<Document>(MESSAGES)
                .update_one(doc! {"_id":&message,"user_id":&user},
                    doc! {"$set":{"attachments":bson::to_bson(&merged).map_err(|_|invalid())?}})
                .session(&mut *session).await?;
            let ids: Vec<String> = merged.iter()
                .filter(|a| a.artifact.source_watch_id.as_deref() == Some(w.id.as_str()))
                .map(|a| a.id.clone()).collect();
            database.collection::<Document>(DELIVERIES)
                .update_one(doc! {"_id":&message}, doc! {"$set":{"attachment_ids":ids,
                    "expires_at":bson::DateTime::from_chrono(expiry)}})
                .session(&mut *session).await?;
            database.collection::<Document>(WATCHES)
                .update_one(doc! {"_id":&w.id}, doc! {"$set":{"artifact_result.delivery_reason":"materialized"}})
                .session(&mut *session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(operation)
        })
    }).await.map_err(transactions::map_transaction_error)
}
