//! Single-send delivery of retained personal replies and ephemeral command output.
use super::{
    api_key_mutation_service as tx, platform_bot_runtime_control_service as controls,
    platform_manager_contract::*, platform_personal_turn_service as personal,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        platform_credential::PlatformCredential,
        platform_manager_delivery::{
            COLLECTION_NAME, DeliveryReference, DeliveryState, PlatformManagerDelivery as Delivery,
        },
        platform_manager_ingress::{
            COLLECTION_NAME as INGRESS, IngressAudience, IngressPhase, PlatformManagerIngress,
        },
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

static PERSONAL_SENDS: Semaphore = Semaphore::const_new(16);
static STATUS_SENDS: Semaphore = Semaphore::const_new(4);

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Delivery>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"dedupe_key":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Delivery>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"state":1,"retry_at":1,"expires_at":1})
                .build(),
        )
        .await?;
    db.collection::<Delivery>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"ingress_id":1,"reference.reply_id":1})
                .options(
                    IndexOptions::builder()
                        .name("manager_delivery_reference".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

fn refused() -> AppError {
    AppError::Conflict("Telegram reply authority is unavailable".into())
}

pub(crate) fn fixed_text(reason: UnavailableReason) -> &'static str {
    match reason {
        UnavailableReason::PersonalDisabled => {
            "Personal NyxBot is paused for this manager. Your saved thread remains in NyxID."
        }
        UnavailableReason::OwnerUnavailable | UnavailableReason::OriginRevoked => {
            "This personal link is unavailable. Open NyxID to review your account and links."
        }
        UnavailableReason::InsufficientFunds => {
            "Your NyxBot could not start with the available credits. Review Billing in NyxID."
        }
        UnavailableReason::CapacityLimited => {
            "Your NyxBot is busy. Wait for the current work or open NyxID."
        }
        UnavailableReason::EmergencyStopped
        | UnavailableReason::ManagerUnavailable
        | UnavailableReason::TransportUnavailable => {
            "This manager is currently unavailable. Open NyxID to continue."
        }
        UnavailableReason::PublicDisabled
        | UnavailableReason::PublicNotReady
        | UnavailableReason::RuntimeUninitialized => {
            "Public chat is unavailable. Send /link in this private chat to use your own NyxBot when personal linking is enabled."
        }
        _ => {
            "This request is unavailable. Open NyxID to continue, or send an original /link command in the manager's private chat."
        }
    }
}

pub(crate) fn render_saved(message: &AssistantMessage, frontend: &str) -> String {
    let mut text = message.text.clone();
    if !message.attachments.is_empty() {
        text.push_str("\n\nImages or files are available in your NyxID conversation.");
    }
    if text.trim().is_empty() {
        text = "Your NyxBot turn finished. Open the conversation in NyxID for details.".into();
    }
    // One Telegram operation per durable claim. Oversized output stays in the app.
    if text.encode_utf16().count() > 3900 {
        text =
            "Your NyxBot reply is ready in NyxID; it is too long for one Telegram message.".into();
    }
    if let Ok(mut url) = reqwest::Url::parse(frontend) {
        url.set_path("/assistant");
        url.set_query(None);
        url.set_fragment(None);
        text.push_str(&format!("\n\nOpen NyxID: {url}"));
    }
    text
}

fn receipt_claim(
    destination: &ReplyDestination,
    delivery: &DeliveryRef,
) -> AppResult<IngressClaim> {
    match (&destination.authority, delivery) {
        (ReplyAuthority::Personal { origin, turn }, DeliveryRef::Reply(reply))
            if *turn == reply.turn =>
        {
            match &origin.source {
                PlatformOriginSource::Ingress(claim) => Ok(claim.clone()),
                _ => Err(refused()),
            }
        }
        (ReplyAuthority::Fixed { ingress }, DeliveryRef::Fixed { ingress: other })
            if ingress == other =>
        {
            Ok(ingress.clone())
        }
        _ => Err(refused()),
    }
}

async fn enqueue(
    state: &AppState,
    destination: &ReplyDestination,
    reference: DeliveryReference,
    ingress: &IngressClaim,
    origin: Option<crate::models::assistant_conversation::PlatformTurnOrigin>,
) -> AppResult<Delivery> {
    let identity = match &reference {
        DeliveryReference::Reply { reply_id, .. } => format!("reply:{reply_id}"),
        DeliveryReference::Command => "command".into(),
        _ => "fixed".into(),
    };
    let dedupe_key = hex::encode(Sha256::digest(
        format!(
            "manager-send:v1:{}:{}:{}:{identity}",
            destination.manager.profile_id, destination.manager.telegram_bot_id, ingress.receipt_id
        )
        .as_bytes(),
    ));
    let now = Utc::now();
    let row = Delivery {
        id: uuid::Uuid::new_v4().to_string(),
        dedupe_key: dedupe_key.clone(),
        profile_id: destination.manager.profile_id.clone(),
        manager_bot_id: destination.manager.manager_bot_id.clone(),
        telegram_bot_id: destination.manager.telegram_bot_id,
        manager_authority_epoch: destination.manager.manager_authority_epoch,
        sender_id: destination.sender_id,
        chat_id: destination.chat_id,
        ingress_id: ingress.receipt_id.clone(),
        ingress_claim_id: ingress.claim_id.clone(),
        origin,
        reference,
        state: DeliveryState::Pending,
        claim_id: None,
        telegram_message_ids: vec![],
        expires_at: destination.expires_at,
        retry_at: now,
        updated_at: now,
    };
    let inserted = state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .update_one(
            doc! {"dedupe_key":&dedupe_key},
            doc! {"$setOnInsert":bson::to_document(&row).map_err(|_|refused())?},
        )
        .upsert(true)
        .await;
    if let Err(error) = inserted {
        if !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(failure) if matches!(failure, mongodb::error::WriteFailure::WriteError(write) if write.code == 11000))
        {
            return Err(error.into());
        }
    }
    state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .find_one(doc! {"dedupe_key":dedupe_key})
        .await?
        .ok_or_else(refused)
}

async fn validate_in_session(
    state: &AppState,
    identity: &dyn ManagerIdentityProducer,
    session: &mut mongodb::ClientSession,
    row: &Delivery,
) -> AppResult<()> {
    if row.expires_at <= Utc::now() {
        return Err(refused());
    }
    let receipt = state.db.collection::<PlatformManagerIngress>(INGRESS).find_one(doc! {
        "_id":&row.ingress_id,"claim_id":&row.ingress_claim_id,"profile_id":&row.profile_id,
        "manager_bot_id":&row.manager_bot_id,"telegram_bot_id":row.telegram_bot_id,
        "manager_authority_epoch":row.manager_authority_epoch,"sender_id":row.sender_id,"chat_id":row.chat_id,
    }).session(&mut *session).await?.ok_or_else(refused)?;
    let controls = controls::fence_manager_in_session(
        &state.db,
        session,
        &row.profile_id,
        &row.manager_bot_id,
        row.telegram_bot_id,
        Some(&receipt.observation_id),
    )
    .await?
    .ok_or_else(refused)?;
    if controls.manager_authority_epoch != row.manager_authority_epoch || controls.retired {
        return Err(refused());
    }
    match &row.reference {
        DeliveryReference::Reply {
            conversation_id,
            turn_id,
            reply_id,
        } => {
            let origin = row.origin.as_ref().ok_or_else(refused)?;
            let origin = personal::contract_origin(origin)?;
            if origin.authority.manager != controls::policy(&controls).manager
                || origin.authority.sender_id != row.sender_id
                || origin.authority.chat_id != row.chat_id
                || row.sender_id != row.chat_id
            {
                return Err(refused());
            }
            let IngressAudience::Personal {
                user_id,
                binding_id,
                binding_generation,
                personal_control_revision,
            } = &receipt.audience
            else {
                return Err(refused());
            };
            if *user_id != origin.authority.user_id
                || *binding_id != origin.authority.binding_id
                || *binding_generation != origin.authority.binding_generation
                || *personal_control_revision != origin.authority.personal_control_revision
                || receipt.phase != IngressPhase::Settled
                || receipt.reply_id.as_ref() != Some(reply_id)
            {
                return Err(refused());
            }
            let execution = receipt.execution.as_ref().ok_or_else(refused)?;
            if execution.conversation_id != *conversation_id || execution.turn_id != *turn_id {
                return Err(refused());
            }
            if !matches!(
                identity
                    .fence_personal_origin(&state.db, session, &origin.authority)
                    .await?,
                FenceOutcome::Authorized
            ) {
                return Err(refused());
            }
            let message=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"_id":reply_id,"conversation_id":conversation_id,"turn_id":turn_id,"user_id":user_id,"role":"assistant"}).session(&mut *session).await?.ok_or_else(refused)?;
            if message.platform_origin.as_ref() != row.origin.as_ref() {
                return Err(refused());
            }
            let thread = state
                .db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"_id":conversation_id,"user_id":user_id})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            if thread.platform_thread.as_ref() != Some(&personal::model_origin(&origin).thread) {
                return Err(refused());
            }
        }
        DeliveryReference::Command => {
            if receipt.audience != IngressAudience::Control
                || !receipt.original_private_command
                || receipt.phase != IngressPhase::Settled
                || row.sender_id != row.chat_id
            {
                return Err(refused());
            }
        }
        DeliveryReference::Fixed { .. } | DeliveryReference::Handoff => {
            // Fixed responses cannot expose account-specific or model text.
            if receipt.audience == IngressAudience::Control {
                return Err(refused());
            }
        }
    }
    Ok(())
}

async fn send(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    row: Delivery,
    body: Zeroizing<String>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    if row.state != DeliveryState::Pending {
        return Ok(DeliveryOutcome::Duplicate);
    }
    let personal = matches!(
        row.reference,
        DeliveryReference::Reply { .. } | DeliveryReference::Command
    );
    let permits = if personal {
        &PERSONAL_SENDS
    } else {
        &STATUS_SENDS
    };
    let pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        if personal {
            "manager_personal_send"
        } else {
            "manager_status_send"
        },
        &format!("{}:{}", row.manager_bot_id, row.sender_id),
        1,
        std::time::Duration::from_secs(1),
    )
    .await?;
    let permit = permits.try_acquire();
    if !pacing.allowed || permit.is_err() {
        if matches!(row.reference, DeliveryReference::Command) {
            state
                .db
                .collection::<Delivery>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.id,"state":"pending"},
                    doc! {"$set":{"state":"refused","updated_at":bson::DateTime::now()}},
                )
                .await?;
            return Ok(DeliveryOutcome::Refused(UnavailableReason::CapacityLimited));
        }
        let retry_at = Utc::now() + Duration::seconds(1);
        state
            .db
            .collection::<Delivery>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id,"state":"pending"},
                doc! {"$set":{"retry_at":bson::DateTime::from_chrono(retry_at)}},
            )
            .await?;
        return Ok(DeliveryOutcome::Deferred { retry_at });
    }
    let _permit = permit.map_err(|_| refused())?;
    let claim_id = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    let captured = state.clone();
    let delivery = row.clone();
    let worker_claim = claim_id.clone();
    let result=session.start_transaction().and_run((captured,identity,delivery,worker_claim),|session,(state,identity,row,claim)|Box::pin(async move {
        let result:AppResult<_>=async {
            validate_in_session(state,identity.as_ref(),session,row).await?;
            if state.db.collection::<Delivery>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"state":"pending","expires_at":{"$gt":bson::DateTime::now()}},doc! {"$set":{"state":"dispatching","claim_id":claim,"updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1{return Ok(None);}
            Ok(state.db.collection::<PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME).find_one(doc! {"provider":"telegram-new","fields.manager_bot_id":&row.manager_bot_id,"fields.webhook_ready":"true"}).session(&mut *session).await?)
        }.await;
        tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error);
    let credential = match result {
        Ok(Some(value)) => value,
        Ok(None) => return Ok(DeliveryOutcome::Duplicate),
        Err(error) => {
            if matches!(error, AppError::Conflict(_) | AppError::NotFound(_)) {
                state
                    .db
                    .collection::<Delivery>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&row.id,"state":"pending"},
                        doc! {"$set":{"state":"refused","updated_at":bson::DateTime::now()}},
                    )
                    .await?;
                return Ok(DeliveryOutcome::Refused(UnavailableReason::OriginRevoked));
            }
            return Err(error);
        }
    };
    let values = super::platform_credential_service::decrypt_snapshot(
        &state.encryption_keys,
        Some(&credential),
    )
    .await?;
    let token = values
        .get(super::channel_adapters::telegram_new::MANAGER_TOKEN)
        .ok_or_else(refused)?;
    if token
        .split(':')
        .next()
        .and_then(|id| id.parse::<i64>().ok())
        != Some(row.telegram_bot_id)
    {
        finish(state, &row, &claim_id, "refused", vec![], None).await?;
        return Ok(DeliveryOutcome::Refused(
            UnavailableReason::TransportUnavailable,
        ));
    }
    // No retries after an ambiguous provider effect. Telegram 429 is a proven
    // rejection and may defer retained replies, but never a one-time bearer.
    let result=api.http.post(format!("{}/bot{token}/sendMessage",api.base_url))
        .timeout(std::time::Duration::from_secs(20))
        .json(&serde_json::json!({"chat_id":row.chat_id,"text":body.as_str(),"protect_content":true,"link_preview_options":{"is_disabled":true}}))
        .send().await;
    let envelope = match result {
        Ok(mut response) => {
            let mut bytes = Zeroizing::new(Vec::new());
            let mut complete = true;
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) if bytes.len() + chunk.len() <= 65536 => {
                        bytes.extend_from_slice(&chunk)
                    }
                    Ok(None) => break,
                    _ => {
                        complete = false;
                        break;
                    }
                }
            }
            if complete {
                serde_json::from_slice::<serde_json::Value>(&bytes).ok()
            } else {
                None
            }
        }
        Err(_) => None,
    };
    if let Some(envelope) = envelope {
        if envelope["ok"] == true
            && let Some(id) = envelope["result"]["message_id"]
                .as_i64()
                .filter(|id| *id > 0)
        {
            finish(state, &row, &claim_id, "sent", vec![id], None).await?;
            return Ok(DeliveryOutcome::Sent {
                telegram_message_ids: vec![id],
            });
        }
        if envelope["ok"] == false
            && envelope["error_code"] == 429
            && !matches!(row.reference, DeliveryReference::Command)
            && let Some(delay) = envelope["parameters"]["retry_after"]
                .as_i64()
                .filter(|value| *value > 0 && *value <= 86400)
        {
            let retry_at = Utc::now() + Duration::seconds(delay);
            finish(state, &row, &claim_id, "pending", vec![], Some(retry_at)).await?;
            return Ok(DeliveryOutcome::Deferred { retry_at });
        }
        if envelope["ok"] == false {
            finish(state, &row, &claim_id, "refused", vec![], None).await?;
            return Ok(DeliveryOutcome::Refused(
                UnavailableReason::TransportUnavailable,
            ));
        }
    }
    finish(state, &row, &claim_id, "uncertain", vec![], None).await?;
    Ok(DeliveryOutcome::Uncertain)
}

async fn finish(
    state: &AppState,
    row: &Delivery,
    claim: &str,
    status: &str,
    ids: Vec<i64>,
    retry: Option<chrono::DateTime<Utc>>,
) -> AppResult<()> {
    let mut set =
        doc! {"state":status,"telegram_message_ids":ids,"updated_at":bson::DateTime::now()};
    if let Some(at) = retry {
        set.insert("retry_at", bson::DateTime::from_chrono(at));
    }
    state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id,"state":"dispatching","claim_id":claim},
            doc! {"$set":set},
        )
        .await?;
    Ok(())
}

pub async fn deliver(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    destination: &ReplyDestination,
    reference: &DeliveryRef,
    content: ReplyContent,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    let claim = receipt_claim(destination, reference)?;
    let (stored,body,origin)=match (reference,content,&destination.authority) {
        (DeliveryRef::Reply(reply),ReplyContent::Text(text),ReplyAuthority::Personal {origin,..})=>{
            let message=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"_id":&reply.reply_id,"conversation_id":&reply.turn.conversation_id,"turn_id":&reply.turn.turn_id,"user_id":&origin.authority.user_id,"role":"assistant"}).await?.ok_or_else(refused)?;
            if message.platform_origin.as_ref()!=Some(&personal::model_origin(origin)) || text!=render_saved(&message,&state.config.frontend_url){return Err(refused());}
            (DeliveryReference::Reply {conversation_id:reply.turn.conversation_id.clone(),turn_id:reply.turn.turn_id.clone(),reply_id:reply.reply_id.clone()},Zeroizing::new(text),message.platform_origin)
        },
        (DeliveryRef::Fixed {..},ReplyContent::Fixed(reason),ReplyAuthority::Fixed {..})=>(DeliveryReference::Fixed {reason:format!("{reason:?}")},Zeroizing::new(fixed_text(reason).into()),None),
        (DeliveryRef::Fixed {..},ReplyContent::PrivateChatHandoff,ReplyAuthority::Fixed {..})=>(DeliveryReference::Handoff,Zeroizing::new("Open this manager's private chat to use your own NyxBot. Group chats cannot link or show personal replies.".into()),None),
        _=>return Err(refused()),
    };
    let row = enqueue(state, destination, stored, &claim, origin).await?;
    send(state, identity, row, body, api).await
}

pub async fn deliver_command(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    command: super::platform_manager_command_service::CommittedCommand,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    let destination = ReplyDestination {
        manager: command.manager,
        sender_id: command.sender_id,
        chat_id: command.chat_id,
        authority: ReplyAuthority::Fixed {
            ingress: command.ingress.clone(),
        },
        expires_at: Utc::now() + Duration::minutes(1),
    };
    let body = command.reply.render(&state.config.frontend_url)?;
    let row = enqueue(
        state,
        &destination,
        DeliveryReference::Command,
        &command.ingress,
        None,
    )
    .await?;
    send(state, identity, row, body, api).await
}

pub async fn sweep(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    limit: u32,
) -> AppResult<RecoveryBatch> {
    let now = Utc::now();
    // A process lost after claim may have sent. Never reset that claim to pending.
    state.db.collection::<Delivery>(COLLECTION_NAME).update_many(doc! {"state":"dispatching","updated_at":{"$lte":bson::DateTime::from_chrono(now-Duration::minutes(2))}},doc! {"$set":{"state":"uncertain","updated_at":bson::DateTime::now()}}).await?;
    state.db.collection::<Delivery>(COLLECTION_NAME).update_many(doc! {"state":"pending","$or":[{"expires_at":{"$lte":bson::DateTime::now()}},{"reference.kind":"command"}]},doc! {"$set":{"state":"refused","updated_at":bson::DateTime::now()}}).await?;
    let mut rows=state.db.collection::<Delivery>(COLLECTION_NAME).find(doc! {"state":"pending","reference.kind":"reply","retry_at":{"$lte":bson::DateTime::now()},"expires_at":{"$gt":bson::DateTime::now()}}).limit(i64::from(limit.min(100))).await?;
    let mut processed = 0;
    while let Some(row) = rows.try_next().await? {
        if let DeliveryReference::Reply { reply_id, .. } = &row.reference
            && let Some(message) = state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":reply_id})
                .await?
        {
            let _ = send(
                state,
                identity.clone(),
                row,
                Zeroizing::new(render_saved(&message, &state.config.frontend_url)),
                &super::telegram_new_api::TelegramApi::new(&state.http_client),
            )
            .await?;
        }
        processed += 1;
    }
    Ok(RecoveryBatch {
        processed,
        has_more: processed == limit.min(100),
    })
}
