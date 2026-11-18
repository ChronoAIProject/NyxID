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
    if !eligible(chat)
        || !args.as_object().is_some_and(|args| args.is_empty())
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
    let target = initiating
        .turn_metadata
        .and_then(|m| m.artifact_recovery_target)
        .ok_or_else(missing)?;
    // The frozen head identifies at most one indexed association. A purged or
    // expired newest result must never fall back to a prior reply.
    db.collection::<Document>(DELIVERIES)
        .find_one(
            doc! {"_id":&target,"user_id":&row.user_id,"conversation_id":&row.id,
            "seq":{"$lt":initiating.seq}},
        )
        .projection(doc! {"_id":1})
        .session(session)
        .await?
        .ok_or_else(missing)?;
    Ok(target)
}

async fn run(
    db: &Database,
    keys: &EncryptionKeys,
    user: &str,
    conversation: &str,
    message: Option<String>,
    chat: Option<ChatAuthority>,
) -> AppResult<()> {
    let reservation = byte_budget::reserve(sandbox::RESERVATION).await?;
    let database = db.clone();
    let user = user.to_owned();
    let conversation = conversation.to_owned();
    let mut session = db.client().start_session().await?;
    // Pass the borrowed keys through the driver's context API instead of
    // capturing them in an and_run2 async closure (mongodb's documented HRTB bug).
    session.start_transaction().and_run(
        (database, keys, user, conversation, message, chat, &reservation),
        |session, context| {
        let (database, keys, user, conversation, message, chat, reservation) = context.clone();
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
            let source = database.collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":&message,"user_id":&user,"conversation_id":&conversation,
                    "role":"assistant","turn_id":&delivery.event_turn_id})
                .session(&mut *session).await?.ok_or_else(missing)?;
            if source.origin != Some(TurnOrigin::Event) || !matches!(source.status.as_str(), "completed" | "failed" | "stopped") {
                return Err(invalid());
            }
            let lineage: Vec<String> = source.attachments.iter()
                .filter(|a| a.artifact.source_watch_id.as_deref() == Some(delivery.watch_id.as_str()))
                .map(|a| a.id.clone()).collect();
            if lineage != delivery.attachment_ids { return Err(invalid()); }
            // Validate present saved metadata before interpreting expiry. A
            // contradictory retained row must never look like a harmless expired peer.
            for attachment in source.attachments.iter()
                .filter(|a| delivery.attachment_ids.contains(&a.id)) {
                if let Some(saved) = database.collection::<Document>(ATTACHMENTS)
                    .find_one(doc! {"_id":&attachment.id}).projection(doc! {"data_encrypted":0})
                    .session(&mut *session).await?
                    && ([
                        ("user_id", user.as_str()),
                        ("conversation_id", conversation.as_str()),
                        ("turn_id", delivery.event_turn_id.as_str()),
                        ("origin", "generated_artifact"),
                        ("source_watch_id", delivery.watch_id.as_str()),
                        ("source_turn_id", delivery.originating_turn_id.as_str()),
                        ("source_operation_id", delivery.operation_id.as_str()),
                        ("source_result_identity", delivery.source_result_identity.as_str()),
                        ("content_type", attachment.content_type.as_str()),
                    ].iter().any(|(key, value)| saved.get_str(key).ok() != Some(*value))
                        || saved.get_i64("size").ok() != Some(attachment.size)
                        || saved.get_str("content_revision").ok() != attachment.artifact.content_revision.as_deref()) {
                    return Err(invalid());
                }
            }
            // A saved expiry can outlive terminal watch TTL. Report that stable
            // owner-visible outcome before requiring retained recovery authority.
            let expired = super::super::assistant_upload_retention::expired_ids(
                &database, &user, &delivery.attachment_ids).await?;
            if (delivery.attachment_ids.is_empty() && delivery.expires_at <= Utc::now())
                || (!delivery.attachment_ids.is_empty()
                    && delivery.attachment_ids.iter().all(|id| expired.contains(id))) {
                return Err(AppError::AssistantAttachmentExpired);
            }
            let w = database.collection::<AsyncServiceOperation>(WATCHES)
                .find_one_and_update(doc! {"_id":&delivery.watch_id,"user_id":&user,"conversation_id":&conversation,
                    "turn_id":&delivery.originating_turn_id,"operation_id":&delivery.operation_id,
                    "artifact_result.source_result_identity":&delivery.source_result_identity,
                    "artifact_event_turn_id":&delivery.event_turn_id,"state":{"$in":["queued","delivered"]}},
                    doc! {"$inc":{"artifact_recovery_generation":1}})
                .session(&mut *session).await?.ok_or_else(missing)?;
            let (attachments, reason) = materialize(&database, keys, &w, session,
                &delivery.event_turn_id, &source.attachments, reservation).await?;
            if matches!(reason, "result_expired" | "retention_expired") {
                return Err(AppError::AssistantAttachmentExpired);
            }
            if reason != "materialized" { return Err(AppError::Conflict(reason.into())); }
            let expiry = attachment_expiry_in_session(&database, session, &w,
                &delivery.event_turn_id, &attachments).await?;
            let mut merged = source.attachments;
            for attachment in attachments {
                if !merged.iter().any(|a| a.id == attachment.id) { merged.push(attachment); }
            }
            if expiry <= Utc::now() { return Err(AppError::AssistantAttachmentExpired); }
            database.collection::<Document>(MESSAGES)
                .update_one(doc! {"_id":&message,"user_id":&user},
                    doc! {"$set":{"attachments":bson::to_bson(&merged).map_err(|_|invalid())?,"turn_metadata.artifact_delivery.state":"materialized","turn_metadata.artifact_delivery.count":merged.iter().filter(|a| a.origin=="generated_artifact").count() as i64}})
                .session(&mut *session).await?;
            let ids: Vec<String> = merged.iter()
                .filter(|a| a.artifact.source_watch_id.as_deref() == Some(w.id.as_str()))
                .map(|a| a.id.clone()).collect();
            database.collection::<Document>(DELIVERIES)
                .update_one(doc! {"_id":&message}, doc! {"$set":{"attachment_ids":ids,
                    "expires_at":bson::DateTime::from_chrono(expiry),"purge_after":bson::DateTime::from_chrono(expiry+Duration::days(30))}})
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

/// Validate retained recovery authority and bytes before audits or action cards.
pub async fn preflight(
    db: &Database,
    user: &str,
    conversation: &str,
    message: &str,
) -> AppResult<()> {
    let delivery = db
        .collection::<Delivery>(DELIVERIES)
        .find_one(doc! {"_id":message,"user_id":user,"conversation_id":conversation})
        .await?
        .ok_or_else(missing)?;
    let source = db.collection::<AssistantMessage>(MESSAGES).find_one(doc!{"_id":message,"user_id":user,"conversation_id":conversation,"turn_id":&delivery.event_turn_id,"role":"assistant","origin":"event"}).await?.ok_or_else(missing)?;
    let lineage: Vec<String> = source
        .attachments
        .iter()
        .filter(|a| a.artifact.source_watch_id.as_deref() == Some(delivery.watch_id.as_str()))
        .map(|a| a.id.clone())
        .collect();
    if lineage != delivery.attachment_ids {
        return Err(invalid());
    }
    if delivery.expires_at <= Utc::now() {
        return Err(AppError::AssistantAttachmentExpired);
    }
    let w = db.collection::<AsyncServiceOperation>(WATCHES).find_one(doc!{"_id":&delivery.watch_id,"user_id":user,"conversation_id":conversation,"turn_id":&delivery.originating_turn_id,"operation_id":&delivery.operation_id,"artifact_event_turn_id":&delivery.event_turn_id,"artifact_result.source_result_identity":&delivery.source_result_identity,"state":{"$in":["queued","delivered"]}}).await?.ok_or_else(missing)?;
    if !admitted(&w) {
        return Err(invalid());
    }
    if !matches!(source.status.as_str(), "completed" | "failed" | "stopped") {
        return Err(invalid());
    }
    let manifest = w.artifact_result.as_ref().ok_or_else(missing)?;
    let mut session = db.client().start_session().await?;
    let rows = pending_rows_in_session(db, &mut session, &w, manifest, false).await?;
    let mut available = false;
    for pending in rows {
        if pending.state == "pending"
            && pending.expires_at > Utc::now()
            && manifest.expires_at > Utc::now()
        {
            let size = db.collection::<Document>(ARTIFACTS).find_one(doc!{"_id":&pending.id,"state":"pending"})
                .projection(doc!{"encrypted_size":{"$cond":[{"$eq":[{"$type":"$data_encrypted"},"binData"]},{"$binarySize":"$data_encrypted"},0]}})
                .await?;
            available |= size.is_some_and(|row| {
                let size = row.get_i32("encrypted_size").unwrap_or_default();
                size > 64 && size as usize <= sandbox::FILE_BYTES + 4096
            });
        } else if pending.state == "materialized" {
            let filter = doc! {"_id":&pending.assistant_attachment_id,"user_id":user,"conversation_id":conversation,"turn_id":&delivery.event_turn_id,"origin":"generated_artifact","source_watch_id":&w.id,"source_result_identity":&manifest.source_result_identity};
            if let Some(saved) = db
                .collection::<Document>(ATTACHMENTS)
                .find_one(doc! {"_id":&pending.assistant_attachment_id})
                .projection(doc! {"data_encrypted":0})
                .await?
            {
                validate_saved_attachment(&saved, &pending, &w, &delivery.event_turn_id)?;
            }
            match super::super::assistant_upload_retention::require_available(db, filter).await {
                Ok(()) => available = true,
                Err(AppError::AssistantAttachmentExpired | AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    if !available {
        return Err(AppError::AssistantAttachmentExpired);
    }
    Ok(())
}
pub async fn preflight_for_chat(
    db: &Database,
    chat: &ChatAuthority,
    args: &serde_json::Value,
) -> AppResult<()> {
    if !eligible(chat) || !args.as_object().is_some_and(|a| a.is_empty()) {
        return Err(invalid());
    }
    chat.ensure_live_turn(db, None, None, "artifact_delivery")
        .await?;
    let row =
        super::super::assistant_nyxagent::get(db, &chat.user_id, &chat.conversation_id).await?;
    let mut session = db.client().start_session().await?;
    let message = select_message(db, &row, chat, &mut session).await?;
    preflight(db, &chat.user_id, &chat.conversation_id, &message).await
}
