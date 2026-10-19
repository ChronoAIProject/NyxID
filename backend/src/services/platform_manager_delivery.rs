//! Single-send delivery of retained personal replies and ephemeral command output.
use super::{
    api_key_mutation_service as tx, platform_bot_runtime_control_service as controls,
    platform_manager_contract::*, platform_personal_turn_service as personal,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as CONVERSATIONS, PlatformReplyRouting,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        platform_bot_config::{COLLECTION_NAME as PROFILES, PlatformBotConfig},
        platform_bot_runtime_controls::PlatformBotRuntimeControls,
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
static PUBLIC_SENDS: Semaphore = Semaphore::const_new(4);

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
    db.collection::<AssistantMessage>(MESSAGES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform_delivery_recovery_queued":1,"created_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name("platform_saved_delivery_recovery".to_owned())
                        .partial_filter_expression(
                            doc! {"role":"assistant","platform_reply.kind":"captured"},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

fn refused() -> AppError {
    AppError::Conflict("Channel reply authority is unavailable".into())
}

pub(crate) fn fixed_text(reason: UnavailableReason) -> &'static str {
    match reason {
        UnavailableReason::PersonalDisabled => {
            "Personal NyxBot is paused for this manager. Your saved thread remains in NyxID."
        }
        UnavailableReason::OwnerUnavailable | UnavailableReason::OriginRevoked => {
            "This personal link or confirmation is unavailable, expired, or belongs to another private chat. Send /link here to review a fresh request, or manage Linked channels in NyxID."
        }
        UnavailableReason::InsufficientFunds => {
            "Your NyxBot could not start with the available credits. Review Billing in NyxID."
        }
        UnavailableReason::CapacityLimited => {
            "Your NyxBot is busy. Wait for the current work or open NyxID."
        }
        UnavailableReason::PublicQuotaLimited => {
            "Public chat has reached a daily limit. Try again after midnight UTC, or send /link to use your own NyxBot."
        }
        UnavailableReason::SponsorUnavailable => {
            "Public chat funding is unavailable. Send /link to use your own NyxBot when personal linking is enabled."
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
    // One text operation per durable claim. Oversized output stays in the app.
    if text.encode_utf16().count() > 3900 {
        text =
            "Your NyxBot reply is ready in NyxID; it is too long for one channel message.".into();
    }
    if let Ok(mut url) = reqwest::Url::parse(frontend) {
        url.set_path("/assistant");
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("c", &message.conversation_id);
        url.set_fragment(None);
        let link = format!("\n\nReview actions and continue this conversation in NyxID: {url}");
        if text.encode_utf16().count() + link.encode_utf16().count() <= 4096 {
            text.push_str(&link);
        }
    }
    text
}

fn blocked_reply(message: &AssistantMessage) -> bool {
    matches!(
        message.error_code.as_deref(),
        Some("cancelled" | "stopped" | "origin_revoked")
    ) || matches!(message.status.as_str(), "cancelled" | "stopped")
}

pub(crate) fn captured_destination(
    message: &AssistantMessage,
) -> AppResult<Option<ReplyDestination>> {
    let Some(PlatformReplyRouting::Captured {
        origin, expires_at, ..
    }) = &message.platform_reply
    else {
        return Ok(None);
    };
    if blocked_reply(message) || *expires_at <= Utc::now() || message.role != "assistant" {
        return Err(refused());
    }
    let origin = personal::contract_origin(origin)?;
    if origin.authority.user_id != message.user_id {
        return Err(refused());
    }
    Ok(Some(ReplyDestination {
        manager: origin.authority.manager.clone(),
        sender_id: origin.authority.sender_id,
        chat_id: origin.authority.chat_id,
        authority: ReplyAuthority::Personal {
            origin,
            turn: TurnRef {
                conversation_id: message.conversation_id.clone(),
                turn_id: message.turn_id.clone(),
            },
        },
        expires_at: *expires_at,
    }))
}

async fn load_personal_saved(
    state: &AppState,
    destination: &ReplyDestination,
    reply: &ReplyRef,
) -> AppResult<AssistantMessage> {
    let ReplyAuthority::Personal { origin, turn } = &destination.authority else {
        return Err(refused());
    };
    if *turn != reply.turn {
        return Err(refused());
    }
    let saved=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "_id":&reply.reply_id,"user_id":&origin.authority.user_id,"conversation_id":&turn.conversation_id,"turn_id":&turn.turn_id,"role":"assistant"
    }).await?.ok_or_else(refused)?;
    let exact = captured_destination(&saved)?.ok_or_else(refused)?;
    let ReplyAuthority::Personal {
        origin: captured, ..
    } = exact.authority
    else {
        return Err(refused());
    };
    if captured != *origin
        || exact.manager != destination.manager
        || exact.sender_id != destination.sender_id
        || exact.chat_id != destination.chat_id
    {
        return Err(refused());
    }
    Ok(saved)
}

fn action_notice(summary: &str, code: &str, discord: bool) -> String {
    let instruction = if discord {
        format!("Use /chat message with yes {code} to allow or no {code} to deny.")
    } else {
        format!("Reply yes {code} to allow or no {code} to deny.")
    };
    format!("\n\nAction: {summary}\n{instruction}")
}

fn append_card_notices(
    message: &AssistantMessage,
    frontend: &str,
    cards: &[crate::models::assistant_acknowledgement::AssistantAcknowledgement],
    key_id: &str,
    group_id: Option<&str>,
) -> String {
    use crate::models::assistant_acknowledgement::CardRouting;
    let mut rendered = message.clone();
    let mut notices = String::new();
    // The selector considers all pending action cards in this conversation.
    // A bounded overflow cannot prove code uniqueness and directs to the app.
    if cards.len() <= 64 {
        for card in cards
            .iter()
            .filter(|card| card.requested_turn_id.as_deref() == Some(message.turn_id.as_str()))
            .take(3)
        {
            let code = super::assistant_acknowledgement_service::confirm_code(&card.id);
            let unique = cards
                .iter()
                .filter(|other| {
                    super::assistant_acknowledgement_service::confirm_code(&other.id) == code
                })
                .count()
                == 1;
            let routing = match card.routing.as_ref() {
                Some(CardRouting::NotInvolved) => message.report_to.is_none() && group_id.is_none(),
                Some(CardRouting::Assigned { report_to }) => {
                    message.report_to.as_ref() == Some(report_to)
                }
                // Group involvement requires its retained group membership proof;
                // platform personal threads do not supply that proof.
                Some(CardRouting::GroupWithNyxbot { .. }) | None => false,
            };
            if card.api_key_id != key_id
                || !unique
                || !routing
                || card.voice_request_id.is_some()
                || card.trigger_run_id.is_some()
                || card.authored_skill.is_some()
            {
                continue;
            }
            let summary = super::assistant_nyxagent::excerpt(&card.summary, 260);
            let discord = matches!(&message.platform_reply, Some(PlatformReplyRouting::Captured { origin, .. }) if origin.thread.platform == "discord");
            notices.push_str(&action_notice(&summary, &code, discord));
        }
    }
    if !cards.is_empty() && notices.is_empty() {
        notices.push_str("\n\nPending actions require review in this exact NyxID conversation.");
    }
    if rendered.text.encode_utf16().count() > 1800 && !notices.is_empty() {
        rendered.text = "Your full NyxBot reply is ready in NyxID.".into();
    }
    rendered.text.push_str(&notices);
    render_saved(&rendered, frontend)
}

pub(crate) async fn render_personal(
    state: &AppState,
    message: &AssistantMessage,
    session: Option<&mut mongodb::ClientSession>,
) -> AppResult<String> {
    use crate::models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKS,
    };
    let filter = doc! {"user_id":&message.user_id,"conversation_id":&message.conversation_id,
    "kind":"action","status":"pending","decider":"user","authored_skill":bson::Bson::Null,
    "expires_at":{"$gt":bson::DateTime::now()}};
    let collection = state.db.collection::<AssistantAcknowledgement>(ACKS);
    let conversations = state.db.collection::<AssistantConversation>(CONVERSATIONS);
    let (cards, thread): (Vec<AssistantAcknowledgement>, Option<AssistantConversation>) =
        match session {
            Some(session) => {
                let mut cursor = collection
                    .find(filter)
                    .sort(doc! {"created_at":1,"_id":1})
                    .limit(65)
                    .session(&mut *session)
                    .await?;
                let cards = cursor.stream(&mut *session).try_collect().await?;
                let thread = conversations
                    .find_one(doc! {"_id":&message.conversation_id,"user_id":&message.user_id})
                    .session(session)
                    .await?;
                (cards, thread)
            }
            None => (
                collection
                    .find(filter)
                    .sort(doc! {"created_at":1,"_id":1})
                    .limit(65)
                    .await?
                    .try_collect()
                    .await?,
                conversations
                    .find_one(doc! {"_id":&message.conversation_id,"user_id":&message.user_id})
                    .await?,
            ),
        };
    let thread = thread.ok_or_else(refused)?;
    Ok(append_card_notices(
        message,
        &state.config.frontend_url,
        &cards,
        &thread.credential_api_key_id,
        thread.group_id.as_deref(),
    ))
}

/// Reloads only the actual durable Saved reply. Destination is never caller chosen.
pub(crate) async fn saved_personal_reply(
    state: &AppState,
    row: &AssistantConversation,
    reply: &ReplyRef,
) -> AppResult<Option<DeliveryOutcome>> {
    saved_personal_reply_with_api(
        state,
        row,
        reply,
        &super::telegram_new_api::TelegramApi::new(&state.http_client),
    )
    .await
}

pub(crate) async fn saved_personal_reply_with_api(
    state: &AppState,
    row: &AssistantConversation,
    reply: &ReplyRef,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<Option<DeliveryOutcome>> {
    if row.id != reply.turn.conversation_id {
        return Err(refused());
    }
    let saved=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "_id":&reply.reply_id,"user_id":&row.user_id,"conversation_id":&row.id,"turn_id":&reply.turn.turn_id,"role":"assistant"
    }).await?.ok_or_else(refused)?;
    let Some(destination) = captured_destination(&saved)? else {
        return Ok(None);
    };
    let body = render_personal(state, &saved, None).await?;
    deliver(
        state,
        super::platform_personal_binding_service::producer(state.clone()),
        &destination,
        &DeliveryRef::Reply(reply.clone()),
        ReplyContent::Text(body),
        api,
    )
    .await
    .map(Some)
}

fn receipt_claim(
    destination: &ReplyDestination,
    delivery: &DeliveryRef,
) -> AppResult<Option<IngressClaim>> {
    match (&destination.authority, delivery) {
        (ReplyAuthority::Personal { origin, turn }, DeliveryRef::Reply(reply))
            if *turn == reply.turn
                && destination.manager == origin.authority.manager
                && destination.sender_id == origin.authority.sender_id
                && destination.chat_id == origin.authority.chat_id =>
        {
            match &origin.source {
                PlatformOriginSource::Ingress(claim) => Ok(Some(claim.clone())),
                PlatformOriginSource::ThreadEvent { .. } => Ok(None),
            }
        }
        (ReplyAuthority::Fixed { ingress }, DeliveryRef::Fixed { ingress: other })
            if ingress == other =>
        {
            Ok(Some(ingress.clone()))
        }
        _ => Err(refused()),
    }
}

fn new_delivery(
    destination: &ReplyDestination,
    reference: DeliveryReference,
    ingress: Option<&IngressClaim>,
    origin: Option<crate::models::assistant_conversation::PlatformTurnOrigin>,
    origin_turn_id: Option<&str>,
) -> AppResult<Delivery> {
    let identity = match &reference {
        DeliveryReference::Reply { reply_id, .. }
        | DeliveryReference::PublicReply { reply_id, .. } => format!("reply:{reply_id}"),
        DeliveryReference::Command => "command".into(),
        DeliveryReference::PublicSignup { intent_id, .. } => format!("public-signup:{intent_id}"),
        _ => "fixed".into(),
    };
    let admission = match (ingress, origin.as_ref().map(|origin| &origin.source)) {
        (Some(ingress), _) => ingress.receipt_id.clone(),
        (
            None,
            Some(crate::models::assistant_conversation::PlatformOriginSource::ThreadEvent {
                admission_id,
            }),
        ) => format!("event:{admission_id}"),
        _ => return Err(refused()),
    };
    let dedupe_key = hex::encode(Sha256::digest(
        format!(
            "manager-send:v1:{}:{}:{}:{identity}",
            destination.manager.profile_id, destination.manager.telegram_bot_id, admission
        )
        .as_bytes(),
    ));
    let now = Utc::now();
    Ok(Delivery {
        id: uuid::Uuid::new_v4().to_string(),
        dedupe_key: dedupe_key.clone(),
        profile_id: destination.manager.profile_id.clone(),
        manager_bot_id: destination.manager.manager_bot_id.clone(),
        telegram_bot_id: destination.manager.telegram_bot_id,
        manager_authority_epoch: destination.manager.manager_authority_epoch,
        sender_id: destination.sender_id,
        chat_id: destination.chat_id,
        ingress_id: ingress.map(|claim| claim.receipt_id.clone()),
        ingress_claim_id: ingress.map(|claim| claim.claim_id.clone()),
        origin,
        origin_turn_id: origin_turn_id.map(str::to_owned),
        reference,
        state: DeliveryState::Pending,
        claim_id: None,
        telegram_message_ids: vec![],
        channel_message_ids: vec![],
        expires_at: destination.expires_at,
        retry_at: now,
        updated_at: now,
    })
}

async fn enqueue(
    state: &AppState,
    destination: &ReplyDestination,
    reference: DeliveryReference,
    ingress: Option<&IngressClaim>,
    origin: Option<crate::models::assistant_conversation::PlatformTurnOrigin>,
    origin_turn_id: Option<&str>,
) -> AppResult<Delivery> {
    let row = new_delivery(destination, reference, ingress, origin, origin_turn_id)?;
    let dedupe_key = row.dedupe_key.clone();
    let inserted = state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .update_one(
            doc! {"dedupe_key":&dedupe_key},
            doc! {"$setOnInsert":bson::to_document(&row).map_err(|_|refused())?},
        )
        .upsert(true)
        .await;
    if let Err(error) = inserted
        && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(failure) if matches!(failure, mongodb::error::WriteFailure::WriteError(write) if write.code == 11000))
    {
        return Err(error.into());
    }
    let stored = state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .find_one(doc! {"dedupe_key":dedupe_key})
        .await?
        .ok_or_else(refused)?;
    if let DeliveryReference::Reply { reply_id, .. } = &stored.reference {
        state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .update_one(
                doc! {"_id":reply_id,"role":"assistant"},
                doc! {"$set":{"platform_delivery_recovery_queued":true}},
            )
            .await?;
    }
    if let Some(ingress) = ingress
        && matches!(
            stored.reference,
            DeliveryReference::Reply { .. } | DeliveryReference::PublicReply { .. }
        )
    {
        state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .update_one(
                doc! {"_id":&ingress.receipt_id,"claim_id":&ingress.claim_id,"phase":"settled"},
                doc! {"$set":{"delivery_recovery_queued":true}},
            )
            .await?;
    }
    Ok(stored)
}

async fn fence_transport_in_session(
    state: &AppState,
    session: &mut mongodb::ClientSession,
    row: &Delivery,
) -> AppResult<
    Option<(
        Option<PlatformManagerIngress>,
        PlatformBotRuntimeControls,
        String,
    )>,
> {
    if row.expires_at <= Utc::now() {
        return Err(refused());
    }
    let receipt = match (&row.ingress_id, &row.ingress_claim_id) {
        (Some(id), Some(claim)) => Some(state.db.collection::<PlatformManagerIngress>(INGRESS).find_one(doc! {
        "_id":id,"claim_id":claim,"profile_id":&row.profile_id,
        "manager_bot_id":&row.manager_bot_id,"telegram_bot_id":row.telegram_bot_id,
        "manager_authority_epoch":row.manager_authority_epoch,"sender_id":row.sender_id,"chat_id":row.chat_id,
    }).session(&mut *session).await?.ok_or_else(refused)?),
        (None,None) if matches!(row.reference,DeliveryReference::Reply { .. }) => None,
        _ => return Err(refused()),
    };
    let profile=state.db.collection::<PlatformBotConfig>(PROFILES).find_one(doc! {"_id":&row.profile_id,"manager_bot_id":&row.manager_bot_id,"deleted_at":bson::Bson::Null}).session(&mut *session).await?.ok_or_else(refused)?;
    let policy = controls::get_in_session(&state.db, session, &row.profile_id)
        .await?
        .ok_or_else(refused)?;
    if policy.retired
        || policy.manager_authority_epoch != row.manager_authority_epoch
        || policy.telegram_bot_id != row.telegram_bot_id
        || policy.manager_bot_id != row.manager_bot_id
    {
        return Err(refused());
    }
    if profile.channel_transport.is_some() {
        let controls = controls::fence_manager_in_session(
            &state.db,
            session,
            &profile.id,
            &row.manager_bot_id,
            row.telegram_bot_id,
            Some(&profile.observation_id),
        )
        .await?
        .ok_or_else(refused)?;
        return Ok(Some((receipt, controls, profile.observation_id)));
    }
    let current = state
        .db
        .collection::<PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME)
        .find_one(doc! {"provider":"telegram-new"})
        .session(&mut *session)
        .await?;
    if let Some(credential) = &current
        && credential
            .fields
            .get("manager_bot_id")
            .is_some_and(|id| id != &row.manager_bot_id)
    {
        return Err(refused());
    }
    if !current.as_ref().is_some_and(|credential| {
        credential.fields.get("webhook_ready").map(String::as_str) == Some("true")
            && credential.fields.get("observation_id") == Some(&profile.observation_id)
    }) {
        return Ok(None);
    }
    let controls = controls::fence_manager_in_session(
        &state.db,
        session,
        &row.profile_id,
        &row.manager_bot_id,
        row.telegram_bot_id,
        Some(&profile.observation_id),
    )
    .await?
    .ok_or_else(refused)?;
    if controls.manager_authority_epoch != row.manager_authority_epoch || controls.retired {
        return Err(refused());
    }
    Ok(Some((receipt, controls, profile.observation_id)))
}

async fn validate_in_session(
    state: &AppState,
    identity: &dyn ManagerIdentityProducer,
    public: Option<&dyn PublicTurnProducer>,
    session: &mut mongodb::ClientSession,
    row: &Delivery,
    body: &str,
) -> AppResult<Option<String>> {
    let Some((receipt, controls, observation)) =
        fence_transport_in_session(state, session, row).await?
    else {
        return Ok(None);
    };
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
            let source_turn = row.origin_turn_id.as_deref().ok_or_else(refused)?;
            let route = PlatformReplyRouting::Captured {
                origin: Box::new(row.origin.clone().ok_or_else(refused)?),
                turn_id: source_turn.to_owned(),
                expires_at: row.expires_at,
            };
            let user_id = &origin.authority.user_id;
            let message=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
                "_id":reply_id,"conversation_id":conversation_id,"turn_id":turn_id,"user_id":user_id,"role":"assistant"
            }).session(&mut *session).await?.ok_or_else(refused)?;
            if message.platform_reply.as_ref() != Some(&route) || blocked_reply(&message) {
                return Err(refused());
            }
            // The delivery ceiling comes from the original admitted input, not
            // the fresh authority that executed a delayed result/event.
            let source=state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
                "conversation_id":conversation_id,"turn_id":source_turn,"user_id":user_id,"role":{"$in":["user","event"]},
                "execution_pending":{"$ne":true},"steering":bson::Bson::Null,
            }).session(&mut *session).await?.ok_or_else(refused)?;
            if source.platform_origin.as_ref() != row.origin.as_ref()
                || source.platform_reply.as_ref() != Some(&route)
            {
                return Err(refused());
            }
            match (&origin.source, receipt.as_ref()) {
                (PlatformOriginSource::Ingress(claim), Some(receipt)) if source.role == "user" => {
                    let IngressAudience::Personal {
                        user_id,
                        binding_id,
                        binding_generation,
                        personal_control_revision,
                    } = &receipt.audience
                    else {
                        return Err(refused());
                    };
                    let execution = receipt.execution.as_ref().ok_or_else(refused)?;
                    if claim.receipt_id != receipt.id
                        || claim.claim_id != receipt.claim_id
                        || *user_id != origin.authority.user_id
                        || *binding_id != origin.authority.binding_id
                        || *binding_generation != origin.authority.binding_generation
                        || *personal_control_revision != origin.authority.personal_control_revision
                        || receipt.phase != IngressPhase::Settled
                        || execution.conversation_id != *conversation_id
                        || execution.turn_id != source_turn
                        || (turn_id == source_turn && receipt.reply_id.as_ref() != Some(reply_id))
                    {
                        return Err(refused());
                    }
                }
                (PlatformOriginSource::ThreadEvent { admission_id }, None)
                    if admission_id == source_turn && source.role == "event" => {}
                _ => return Err(refused()),
            }
            if !matches!(
                identity
                    .fence_personal_origin(&state.db, session, &origin.authority)
                    .await?,
                FenceOutcome::Authorized
            ) {
                return Err(refused());
            }
            let thread = state
                .db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"_id":conversation_id,"user_id":user_id})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            if thread.platform_thread.as_ref() != Some(&personal::model_origin(&origin).thread)
                || render_personal(state, &message, Some(session)).await? != body
            {
                return Err(refused());
            }
        }
        DeliveryReference::Command => {
            let receipt = receipt.as_ref().ok_or_else(refused)?;
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
            let canonical = match &row.reference {
                DeliveryReference::Fixed { reason } => {
                    fixed_text(fixed_reason(reason).ok_or_else(refused)?)
                }
                _ => handoff_text(),
            };
            if super::platform_channel_service::format_controls(
                &row.manager_bot_id,
                canonical.into(),
            ) != body
            {
                return Err(refused());
            }
            if receipt.as_ref().ok_or_else(refused)?.audience == IngressAudience::Control {
                return Err(refused());
            }
        }
        // Public proof output has its own executing-turn/intent transaction.
        DeliveryReference::Faq {
            article_ids,
            public_policy_revision,
            linked,
            linking_enabled,
        } => {
            let receipt = receipt.as_ref().ok_or_else(refused)?;
            if controls.emergency_stopped
                || controls.retired
                || (!linked && !controls.public_enabled)
                || (!linked && controls.public_policy.is_some())
                || controls.personal_enabled != *linking_enabled
                || controls.public_policy_revision != *public_policy_revision
                || if *linked {
                    !matches!(receipt.audience, IngressAudience::Personal { .. })
                } else {
                    receipt.audience
                        != (IngressAudience::Public {
                            public_policy_revision: *public_policy_revision,
                        })
                }
                || super::platform_channel_service::format_controls(
                    &row.manager_bot_id,
                    super::platform_faq_service::render_mode(
                        article_ids,
                        *linked,
                        *linking_enabled,
                    )?,
                ) != body
            {
                return Err(refused());
            }
            if receipt.phase != IngressPhase::Settled {
                return Err(refused());
            }
            if *linked {
                let resolution = identity
                    .resolve_sender(
                        &state.db,
                        session,
                        &VerifiedManagerUpdate {
                            profile_id: receipt.profile_id.clone(),
                            manager_bot_id: receipt.manager_bot_id.clone(),
                            telegram_bot_id: receipt.telegram_bot_id,
                            observation_id: receipt.observation_id.clone(),
                            sender_id: receipt.sender_id,
                            chat_id: receipt.chat_id,
                            chat_kind: ManagerChatKind::Private,
                            update_id: receipt.update_id,
                            message_id: receipt.message_id,
                            command_eligibility: ManagerCommandEligibility::Ineligible,
                        },
                    )
                    .await?;
                let authorized = match (&receipt.audience, resolution) {
                    (IngressAudience::Public { .. }, SenderResolution::Unlinked) => !linked,
                    (
                        IngressAudience::Personal {
                            user_id,
                            binding_id,
                            binding_generation,
                            personal_control_revision,
                        },
                        SenderResolution::Personal { authority, .. },
                    ) => {
                        *linked
                            && authority.user_id == *user_id
                            && authority.binding_id == *binding_id
                            && authority.binding_generation == *binding_generation
                            && authority.personal_control_revision == *personal_control_revision
                    }
                    _ => false,
                };
                if !authorized {
                    return Err(refused());
                }
            }
        }
        DeliveryReference::PublicReply { reply_id, .. } => {
            let destination = public_destination(row)?;
            let receipt = receipt.as_ref().ok_or_else(refused)?;
            if receipt.reply_id.as_ref() != Some(reply_id) || receipt.phase != IngressPhase::Settled
            {
                return Err(refused());
            }
            let producer = public.ok_or_else(refused)?;
            if !matches!(
                producer
                    .fence_public_reply(&state.db, session, &destination)
                    .await?,
                FenceOutcome::Authorized
            ) {
                return Err(refused());
            }
            let reply = public_reply_ref(row)?;
            let canonical = super::platform_public_credential_service::load_public_reply(
                state,
                &destination,
                &reply,
            )
            .await?;
            if canonical.as_str() != body {
                return Err(refused());
            }
        }
        DeliveryReference::PublicSignup { .. } => return Err(refused()),
    }
    Ok(Some(observation))
}

enum CapturedSend {
    Duplicate,
    Deferred,
    Ready(Box<PlatformCredential>),
    Channel(Box<super::platform_channel_service::CredentialSnapshot>),
}

async fn send(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    row: Delivery,
    body: Zeroizing<String>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    send_with_adapter(state, identity, public, row, body, api, None).await
}

#[allow(clippy::too_many_arguments)]
async fn send_with_adapter(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    row: Delivery,
    body: Zeroizing<String>,
    api: &super::telegram_new_api::TelegramApi<'_>,
    adapter: Option<Box<dyn super::channel_platform::PlatformAdapter>>,
) -> AppResult<DeliveryOutcome> {
    if row.state != DeliveryState::Pending {
        return Ok(DeliveryOutcome::Duplicate);
    }
    if row.expires_at <= Utc::now() || body.encode_utf16().count() > 4096 {
        refuse_pending(state, &row).await?;
        return Ok(DeliveryOutcome::Refused(
            UnavailableReason::TransportUnavailable,
        ));
    }
    if row.retry_at > Utc::now() {
        return Ok(DeliveryOutcome::Deferred {
            retry_at: row.retry_at,
        });
    }
    let personal = matches!(
        row.reference,
        DeliveryReference::Reply { .. } | DeliveryReference::Command
    );
    let public_lane = matches!(
        row.reference,
        DeliveryReference::PublicReply { .. } | DeliveryReference::Faq { .. }
    );
    let permits = if public_lane {
        &PUBLIC_SENDS
    } else if personal {
        &PERSONAL_SENDS
    } else {
        &STATUS_SENDS
    };
    let pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        if public_lane {
            "manager_public_send"
        } else if personal {
            "manager_personal_send"
        } else {
            "manager_status_send"
        },
        &format!("{}:{}", row.manager_bot_id, row.sender_id),
        1,
        std::time::Duration::from_secs(1),
    )
    .await?;
    // Application lane budgets are shared across replicas. Status/public load
    // cannot consume the personal reserve; provider 429 remains authoritative.
    let manager_pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        if public_lane {
            "manager_public_lane"
        } else if personal {
            "manager_personal_lane"
        } else {
            "manager_status_lane"
        },
        &row.manager_bot_id,
        if personal { 24 } else { 4 },
        std::time::Duration::from_secs(1),
    )
    .await?;
    let permit = permits.try_acquire();
    if !pacing.allowed || !manager_pacing.allowed || permit.is_err() {
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
    // Refresh before the final transaction, then capture the resulting encrypted
    // token there. Never resolve a replacement credential after claiming a send.
    if let Some(profile) = state
        .db
        .collection::<PlatformBotConfig>(PROFILES)
        .find_one(doc! {"_id":&row.profile_id})
        .await?
        && let Some(transport) = &profile.channel_transport
        && let Some(bot) = state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find_one(super::platform_channel_service::bot_filter(
                &profile, transport,
            ))
            .await?
        && bot.credential_source == "connection"
    {
        let adapter =
            super::channel_adapters::resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
        let _token = super::channel_credentials::resolve_bot_token(
            &state.db,
            &state.encryption_keys,
            adapter.as_ref(),
            &bot,
        )
        .await?;
    }
    let claim_id = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    let captured = state.clone();
    let delivery = row.clone();
    let worker_claim = claim_id.clone();
    let result=session.start_transaction().and_run((captured,identity,public,delivery,worker_claim,body.clone()),|session,(state,identity,public,row,claim,body)|Box::pin(async move {
        let result:AppResult<_>=async {
            let Some(observation)=validate_in_session(state,identity.as_ref(),public.as_deref(),session,row,body.as_str()).await? else {return Ok(CapturedSend::Deferred);};
            if state.db.collection::<Delivery>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"state":"pending","expires_at":{"$gt":bson::DateTime::now()}},doc! {"$set":{"state":"dispatching","claim_id":claim,"updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1{return Ok(CapturedSend::Duplicate);}
            let profile = state.db.collection::<PlatformBotConfig>(PROFILES).find_one(doc! {"_id":&row.profile_id,"observation_id":&observation,"deleted_at":bson::Bson::Null}).session(&mut *session).await?.ok_or_else(refused)?;
            if let Some(transport)=&profile.channel_transport {
                let bot=state.db.collection::<crate::models::channel_bot::ChannelBot>(crate::models::channel_bot::COLLECTION_NAME).find_one(super::platform_channel_service::bot_filter(&profile,transport)).session(&mut *session).await?.ok_or_else(refused)?;
                let snapshot=super::platform_channel_service::CredentialSnapshot::load(&state.db,session,&bot,true).await?;
                if snapshot.stamp()?!=transport.credential_stamp {return Err(refused());}
                return Ok(CapturedSend::Channel(Box::new(snapshot)));
            }
            let credential=state.db.collection::<PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME).find_one(doc! {"provider":"telegram-new","fields.manager_bot_id":&row.manager_bot_id,"fields.observation_id":observation,"fields.webhook_ready":"true"}).session(&mut *session).await?.ok_or_else(refused)?;
            Ok(CapturedSend::Ready(Box::new(credential)))
        }.await;
        tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error);
    let credential = match result {
        Ok(CapturedSend::Channel(snapshot)) => {
            return send_channel_claimed(state, &row, &claim_id, body, &snapshot, adapter).await;
        }
        Ok(CapturedSend::Ready(value)) => value,
        Ok(CapturedSend::Duplicate) => return Ok(DeliveryOutcome::Duplicate),
        Ok(CapturedSend::Deferred) => {
            if matches!(row.reference, DeliveryReference::Command) {
                refuse_pending(state, &row).await?;
                return Ok(DeliveryOutcome::Refused(
                    UnavailableReason::TransportUnavailable,
                ));
            }
            let retry_at = Utc::now() + Duration::seconds(5);
            state.db.collection::<Delivery>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"state":"pending"},doc! {"$set":{"retry_at":bson::DateTime::from_chrono(retry_at),"updated_at":bson::DateTime::now()}}).await?;
            return Ok(DeliveryOutcome::Deferred { retry_at });
        }
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
    send_claimed(state, &row, &claim_id, body, credential, api).await
}

struct PreparedChannelSend {
    adapter: Box<dyn super::channel_platform::PlatformAdapter>,
    token: Zeroizing<String>,
    secrets: Option<super::channel_platform::PlatformVerifySecrets>,
    conversation_id: String,
    reply: super::channel_platform::OutboundReply,
}
async fn prepare_channel_send(
    state: &AppState,
    row: &Delivery,
    body: Zeroizing<String>,
    snapshot: &super::platform_channel_service::CredentialSnapshot,
) -> AppResult<PreparedChannelSend> {
    let bot = &snapshot.bot;
    let peer =
        super::platform_channel_service::peer(&state.db, &row.profile_id, row.sender_id).await?;
    if row.sender_id != row.chat_id {
        return Err(refused());
    }
    let adapter =
        super::channel_adapters::resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let token = snapshot.token(&state.encryption_keys).await?;
    let secrets = Some(
        snapshot
            .secrets(&state.encryption_keys, adapter.as_ref())
            .await?,
    );
    let limit = if bot.platform == "discord" {
        2000
    } else {
        4096
    };
    let rendered = if body.encode_utf16().count() > limit {
        if let DeliveryReference::Reply {
            conversation_id, ..
        } = &row.reference
        {
            format!(
                "Your NyxBot reply is ready in NyxID: {}/assistant?c={}",
                state.config.frontend_url.trim_end_matches('/'),
                conversation_id
            )
        } else {
            return Err(refused());
        }
    } else {
        body.to_string()
    };
    let mut metadata = Some(serde_json::json!({"shared_plain_text":true}));
    if bot.platform == "discord" {
        if let Some(ingress_id) = &row.ingress_id {
            let receipt = state
                .db
                .collection::<PlatformManagerIngress>(INGRESS)
                .find_one(doc! {"_id":ingress_id})
                .await?
                .ok_or_else(refused)?;
            let context = state.db.collection::<crate::models::platform_channel_identity::PlatformChannelIdentity>(crate::models::platform_channel_identity::COLLECTION_NAME)
                .find_one(doc! {"profile_id":&row.profile_id,"kind":"message","handle":receipt.message_id,"sender_id":&peer.sender_id,"conversation_id":&peer.conversation_id,"expires_at":{"$gt":bson::DateTime::now()}}).await?;
            // Expired interaction tokens cannot complete the deferred interaction.
            // Deliver long-running results using the bot token to the same bound DM.
            if let Some(context) = context {
                let bytes = Zeroizing::new(
                    state
                        .encryption_keys
                        .decrypt_with_aad(
                            &context.reply_context_encrypted,
                            format!("platform-reply:{}:{}", row.profile_id, receipt.message_id)
                                .as_bytes(),
                        )
                        .await?,
                );
                let thread = std::str::from_utf8(&bytes).map_err(|_| refused())?;
                metadata = Some(
                    serde_json::json!({"shared_plain_text":true,"interaction_thread_id":thread}),
                );
            }
        }
    }
    let reply = super::channel_platform::OutboundReply {
        attachments: vec![],
        text: Some(rendered),
        reply_to_platform_message_id: None,
        metadata,
    };
    Ok(PreparedChannelSend {
        adapter,
        token,
        secrets,
        conversation_id: peer.conversation_id,
        reply,
    })
}

async fn send_channel_claimed(
    state: &AppState,
    row: &Delivery,
    claim_id: &str,
    body: Zeroizing<String>,
    snapshot: &super::platform_channel_service::CredentialSnapshot,
    adapter: Option<Box<dyn super::channel_platform::PlatformAdapter>>,
) -> AppResult<DeliveryOutcome> {
    let mut prepared = match prepare_channel_send(state, row, body, snapshot).await {
        Ok(prepared) => prepared,
        Err(error) => {
            let terminal = matches!(
                error,
                AppError::Conflict(_)
                    | AppError::ValidationError(_)
                    | AppError::NotFound(_)
                    | AppError::Forbidden(_)
            );
            let retry_at = Utc::now() + Duration::seconds(5);
            state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
                doc! {"_id":&row.id,"claim_id":claim_id,"state":"dispatching"},
                doc! {"$set":{"state":if terminal {"refused"} else {"pending"},"claim_id":bson::Bson::Null,"retry_at":bson::DateTime::from_chrono(retry_at),"updated_at":bson::DateTime::now()}},
            ).await?;
            return Ok(if terminal {
                DeliveryOutcome::Refused(UnavailableReason::TransportUnavailable)
            } else {
                DeliveryOutcome::Deferred { retry_at }
            });
        }
    };
    if let Some(adapter) = adapter {
        prepared.adapter = adapter;
    }
    send_prepared_channel(state, row, claim_id, snapshot, prepared).await
}

async fn send_prepared_channel(
    state: &AppState,
    row: &Delivery,
    claim_id: &str,
    snapshot: &super::platform_channel_service::CredentialSnapshot,
    prepared: PreparedChannelSend,
) -> AppResult<DeliveryOutcome> {
    let bot = &snapshot.bot;
    let billing = super::channel_billing_service::ChannelBilling::for_bot(
        &state.db,
        &state.billing,
        bot,
        None,
    );
    let credentials = super::channel_platform::BotCredentials {
        token: &prepared.token,
        platform_bot_id: Some(&bot.platform_bot_id),
        platform_secrets: prepared.secrets.as_ref(),
        billing: billing.as_ref(),
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(25),
        prepared.adapter.send_reply_outcome(
            &state.http_client,
            &credentials,
            &prepared.conversation_id,
            &prepared.reply,
        ),
    )
    .await;
    let mut ids = vec![];
    let mut observation = None;
    let mut accepted = false;
    if let Ok(Ok(outcome)) = result {
        accepted = outcome.error.is_none()
            && outcome
                .observation
                .as_ref()
                .is_none_or(|record| record.complete && !record.uncertain);
        ids = outcome
            .observation
            .as_ref()
            .map(|record| record.component_ids.clone())
            .unwrap_or_default();
        if let Some(id) = outcome.final_id
            && !ids.contains(&id)
        {
            ids.push(id);
        }
        observation = outcome.observation;
    }
    // An uncertain or partial provider effect is retained and is never replayed.
    state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
        doc! {"_id":&row.id,"claim_id":claim_id,"state":"dispatching"},
        doc! {"$set":{"state":if accepted {"sent"} else {"uncertain"},"channel_message_ids":&ids,"channel_send":bson::to_bson(&observation).map_err(|_|refused())?,"updated_at":bson::DateTime::now()}},
    ).await?;

    let error = AppError::ChannelPlatformError("Shared bot delivery is uncertain".into());
    super::channel_bot_service::record_outbound_outcome(
        &state.db,
        &bot.id,
        if accepted { None } else { Some(&error) },
    )
    .await;
    Ok(if accepted {
        DeliveryOutcome::ChannelSent { message_ids: ids }
    } else {
        DeliveryOutcome::Uncertain
    })
}

async fn send_claimed(
    state: &AppState,
    row: &Delivery,
    claim_id: &str,
    body: Zeroizing<String>,
    credential: Box<PlatformCredential>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    let values = super::platform_credential_service::decrypt_snapshot(
        &state.encryption_keys,
        Some(credential.as_ref()),
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
        finish(state, row, claim_id, "refused", vec![], None).await?;
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
            finish(state, row, claim_id, "sent", vec![id], None).await?;
            return Ok(DeliveryOutcome::Sent {
                telegram_message_ids: vec![id],
            });
        }
        if envelope["ok"] == false
            && envelope["error_code"] == 429
            && !matches!(
                row.reference,
                DeliveryReference::Command | DeliveryReference::PublicSignup { .. }
            )
            && let Some(delay) = envelope["parameters"]["retry_after"]
                .as_i64()
                .filter(|value| *value > 0 && *value <= 86400)
        {
            let retry_at = Utc::now() + Duration::seconds(delay);
            finish(state, row, claim_id, "pending", vec![], Some(retry_at)).await?;
            return Ok(DeliveryOutcome::Deferred { retry_at });
        }
        if envelope["ok"] == false {
            finish(state, row, claim_id, "refused", vec![], None).await?;
            return Ok(DeliveryOutcome::Refused(
                UnavailableReason::TransportUnavailable,
            ));
        }
    }
    finish(state, row, claim_id, "uncertain", vec![], None).await?;
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
    deliver_with_adapter(state, identity, destination, reference, content, api, None).await
}

async fn deliver_with_adapter(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    destination: &ReplyDestination,
    reference: &DeliveryRef,
    content: ReplyContent,
    api: &super::telegram_new_api::TelegramApi<'_>,
    adapter: Option<Box<dyn super::channel_platform::PlatformAdapter>>,
) -> AppResult<DeliveryOutcome> {
    let claim = receipt_claim(destination, reference)?;
    let (stored, body, origin, source_turn) = match (reference, content, &destination.authority) {
        (DeliveryRef::Reply(reply), ReplyContent::Text(text), ReplyAuthority::Personal { .. }) => {
            let message = load_personal_saved(state, destination, reply).await?;
            let Some(PlatformReplyRouting::Captured {
                origin, turn_id, ..
            }) = &message.platform_reply
            else {
                return Err(refused());
            };
            if text != render_personal(state, &message, None).await? {
                return Err(refused());
            }
            (
                DeliveryReference::Reply {
                    conversation_id: reply.turn.conversation_id.clone(),
                    turn_id: reply.turn.turn_id.clone(),
                    reply_id: reply.reply_id.clone(),
                },
                Zeroizing::new(text),
                Some(origin.as_ref().clone()),
                Some(turn_id.clone()),
            )
        }
        (DeliveryRef::Fixed { .. }, ReplyContent::Fixed(reason), ReplyAuthority::Fixed { .. }) => (
            DeliveryReference::Fixed {
                reason: format!("{reason:?}"),
            },
            Zeroizing::new(super::platform_channel_service::format_controls(
                &destination.manager.manager_bot_id,
                fixed_text(reason).into(),
            )),
            None,
            None,
        ),
        (
            DeliveryRef::Fixed { .. },
            ReplyContent::PrivateChatHandoff,
            ReplyAuthority::Fixed { .. },
        ) => (
            DeliveryReference::Handoff,
            Zeroizing::new(handoff_text().into()),
            None,
            None,
        ),
        _ => return Err(refused()),
    };
    let canonical = if let DeliveryReference::Reply {
        reply_id,
        conversation_id,
        turn_id,
    } = &stored
    {
        let reply = ReplyRef {
            turn: TurnRef {
                conversation_id: conversation_id.clone(),
                turn_id: turn_id.clone(),
            },
            reply_id: reply_id.clone(),
        };
        captured_destination(&load_personal_saved(state, destination, &reply).await?)?
    } else {
        None
    };
    let row = enqueue(
        state,
        canonical.as_ref().unwrap_or(destination),
        stored,
        claim.as_ref(),
        origin,
        source_turn.as_deref(),
    )
    .await?;
    let outcome = send_with_adapter(state, identity, None, row, body, api, adapter).await?;
    report_delivery(&outcome, "reply");
    Ok(outcome)
}

pub(crate) async fn deliver_faq(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    admitted: &PublicIngress,
    text: &str,
    linked: bool,
    linking_enabled: bool,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    let article_ids = super::platform_faq_service::select(text)?;
    let mut session = state.db.client().start_session().await?;
    let row = session
        .start_transaction()
        .and_run(
            (&state.db, admitted, article_ids, linked, linking_enabled),
            |session, (db, admitted, ids, linked, enabled)| {
                Box::pin(async move {
                    tx::transaction_result(
                        queue_faq_in_session(db, session, admitted, ids, *linked, *enabled).await,
                    )
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    let DeliveryReference::Faq {
        article_ids,
        linked,
        linking_enabled,
        ..
    } = &row.reference
    else {
        return Err(refused());
    };
    // Admission owns the selected content and mode guidance, including when the
    // caller still holds an earlier runtime snapshot. Final validation checks drift.
    let body = Zeroizing::new(super::platform_channel_service::format_controls(
        &row.manager_bot_id,
        super::platform_faq_service::render_mode(article_ids, *linked, *linking_enabled)?,
    ));
    send(state, identity, None, row, body, api).await
}

/// Joins admission's transaction so a crash before the first send still leaves
/// an approved-content reference for recovery, never the visitor's question.
pub(crate) async fn queue_faq_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    admitted: &PublicIngress,
    article_ids: &[String],
    linked: bool,
    linking_enabled: bool,
) -> AppResult<Delivery> {
    let destination = ReplyDestination {
        manager: admitted.manager.clone(),
        sender_id: admitted.sender_id,
        chat_id: admitted.chat_id,
        authority: ReplyAuthority::Fixed {
            ingress: admitted.ingress.clone(),
        },
        expires_at: Utc::now() + Duration::minutes(2),
    };
    let proposed = new_delivery(
        &destination,
        DeliveryReference::Faq {
            article_ids: article_ids.to_vec(),
            linked,
            linking_enabled,
            public_policy_revision: admitted.public_policy_revision,
        },
        Some(&admitted.ingress),
        None,
        None,
    )?;
    let claim = &admitted.ingress;
    let receipt=db.collection::<PlatformManagerIngress>(INGRESS).find_one(doc! {"_id":&claim.receipt_id,"claim_id":&claim.claim_id,"phase":{"$in":["admitted","settled"]}}).session(&mut *session).await?.ok_or_else(refused)?;
    if receipt.profile_id != proposed.profile_id
        || receipt.sender_id != proposed.sender_id
        || receipt.chat_id != proposed.chat_id
    {
        return Err(refused());
    }
    db.collection::<Delivery>(COLLECTION_NAME)
        .update_one(
            doc! {"dedupe_key":&proposed.dedupe_key},
            doc! {"$setOnInsert":bson::to_document(&proposed).map_err(|_|refused())?},
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    db.collection::<PlatformManagerIngress>(INGRESS).update_one(doc! {"_id":&claim.receipt_id,"claim_id":&claim.claim_id},doc! {"$set":{"phase":"settled","settled_at":bson::DateTime::now(),"delivery_recovery_queued":true}}).session(&mut *session).await?;
    db.collection::<Delivery>(COLLECTION_NAME)
        .find_one(doc! {"dedupe_key":&proposed.dedupe_key})
        .session(session)
        .await?
        .ok_or_else(refused)
}

fn public_reply_ref(row: &Delivery) -> AppResult<ReplyRef> {
    let DeliveryReference::PublicReply {
        conversation_id,
        turn_id,
        reply_id,
        ..
    } = &row.reference
    else {
        return Err(refused());
    };
    Ok(ReplyRef {
        turn: TurnRef {
            conversation_id: conversation_id.clone(),
            turn_id: turn_id.clone(),
        },
        reply_id: reply_id.clone(),
    })
}
fn public_destination(row: &Delivery) -> AppResult<ReplyDestination> {
    let DeliveryReference::PublicReply {
        public_session_id,
        public_policy_revision,
        ..
    } = &row.reference
    else {
        return Err(refused());
    };
    let manager = ManagerAuthority {
        profile_id: row.profile_id.clone(),
        manager_bot_id: row.manager_bot_id.clone(),
        telegram_bot_id: row.telegram_bot_id,
        manager_authority_epoch: row.manager_authority_epoch,
    };
    let ingress = PublicIngress {
        manager: manager.clone(),
        sender_id: row.sender_id,
        chat_id: row.chat_id,
        ingress: IngressClaim {
            receipt_id: row.ingress_id.clone().ok_or_else(refused)?,
            claim_id: row.ingress_claim_id.clone().ok_or_else(refused)?,
        },
        public_policy_revision: *public_policy_revision,
    };
    Ok(ReplyDestination {
        manager,
        sender_id: row.sender_id,
        chat_id: row.chat_id,
        expires_at: row.expires_at,
        authority: ReplyAuthority::Public {
            ingress,
            public_session_id: public_session_id.clone(),
            turn: public_reply_ref(row)?.turn,
        },
    })
}

pub(crate) async fn deliver_public_reply(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Arc<dyn PublicTurnProducer>,
    destination: &ReplyDestination,
    reply: &ReplyRef,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    let body =
        super::platform_public_credential_service::load_public_reply(state, destination, reply)
            .await?;
    let ReplyAuthority::Public {
        ingress,
        public_session_id,
        turn,
    } = &destination.authority
    else {
        return Err(refused());
    };
    if *turn != reply.turn {
        return Err(refused());
    }
    let row = enqueue(
        state,
        destination,
        DeliveryReference::PublicReply {
            public_session_id: public_session_id.clone(),
            conversation_id: turn.conversation_id.clone(),
            turn_id: turn.turn_id.clone(),
            reply_id: reply.reply_id.clone(),
            public_policy_revision: ingress.public_policy_revision,
        },
        Some(&ingress.ingress),
        None,
        None,
    )
    .await?;
    let outcome = send(state, identity, Some(public), row, body, api).await?;
    report_delivery(&outcome, "public_reply");
    Ok(outcome)
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
    let body = Zeroizing::new(super::platform_channel_service::format_controls(
        &destination.manager.manager_bot_id,
        command
            .reply
            .render(&state.config.frontend_url)?
            .to_string(),
    ));
    let row = enqueue(
        state,
        &destination,
        DeliveryReference::Command,
        Some(&command.ingress),
        None,
        None,
    )
    .await?;
    let outcome = send(state, identity, None, row, body, api).await?;
    report_delivery(&outcome, "command");
    Ok(outcome)
}

/// A public tool sees only this status, never the intent bearer or its URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublicSignupDeliveryOutcome {
    Sent,
    Unavailable,
    Uncertain,
}

/// The intent mint must already be committed. There is no retryable body queue.
pub(crate) async fn deliver_public_signup(
    state: &AppState,
    context: &PublicExecutionContext,
    token: Zeroizing<String>,
) -> AppResult<PublicSignupDeliveryOutcome> {
    deliver_public_signup_with_api(
        state,
        context,
        token,
        &super::telegram_new_api::TelegramApi::new(&state.http_client),
    )
    .await
}

pub(crate) async fn deliver_public_signup_with_api(
    state: &AppState,
    context: &PublicExecutionContext,
    token: Zeroizing<String>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<PublicSignupDeliveryOutcome> {
    use super::platform_link_intent_service as intents;
    let ingress = &context.ingress;
    if ingress.sender_id <= 0
        || ingress.sender_id != ingress.chat_id
        || context.expires_at <= Utc::now()
    {
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    // This read obtains metadata for dedupe; the final transaction revalidates
    // the exact token, intent and execution authority with a write fence.
    let intent =
        match intents::preview(&state.db, state.auth_device_hmac_key.as_slice(), &token).await {
            Ok(intent) => intent,
            Err(AppError::Conflict(_) | AppError::NotFound(_) | AppError::ValidationError(_)) => {
                return Ok(PublicSignupDeliveryOutcome::Unavailable);
            }
            Err(error) => return Err(error),
        };
    let destination = ReplyDestination {
        manager: ingress.manager.clone(),
        sender_id: ingress.sender_id,
        chat_id: ingress.chat_id,
        authority: ReplyAuthority::Public {
            ingress: ingress.clone(),
            public_session_id: context.public_session_id.clone(),
            turn: context.turn.clone(),
        },
        expires_at: context.expires_at.min(intent.expires_at),
    };
    let row = enqueue(
        state,
        &destination,
        DeliveryReference::PublicSignup {
            public_session_id: context.public_session_id.clone(),
            conversation_id: context.turn.conversation_id.clone(),
            turn_id: context.turn.turn_id.clone(),
            intent_id: intent.id.clone(),
        },
        Some(&ingress.ingress),
        None,
        None,
    )
    .await?;
    if row.state != DeliveryState::Pending {
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    let pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        "manager_public_send",
        &format!("{}:{}", row.manager_bot_id, row.sender_id),
        1,
        std::time::Duration::from_secs(1),
    )
    .await?;
    let lane = super::coordination_service::RateWindowStore::admit(
        &state.db,
        "manager_public_lane",
        &row.manager_bot_id,
        4,
        std::time::Duration::from_secs(1),
    )
    .await?;
    let permit = PUBLIC_SENDS.try_acquire();
    if !pacing.allowed || !lane.allowed || permit.is_err() {
        refuse_pending(state, &row).await?;
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    let _permit = permit.map_err(|_| refused())?;
    let claim = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    let result = session.start_transaction().and_run(
        (state, context, &row, token.as_str(), &claim),
        |session, (state, context, row, token, claim)| Box::pin(async move {
            let result: AppResult<_> = async {
                let Some((_, _, observation)) = fence_transport_in_session(state, session, row).await? else {
                    return Ok(CapturedSend::Deferred);
                };
                let intent_id = intents::fence_public_intent_in_session(
                    &state.db, session, state.auth_device_hmac_key.as_slice(), context, token,
                ).await?;
                if !matches!(&row.reference, DeliveryReference::PublicSignup {
                    public_session_id, conversation_id, turn_id, intent_id: expected,
                } if public_session_id == &context.public_session_id
                    && conversation_id == &context.turn.conversation_id
                    && turn_id == &context.turn.turn_id && expected == &intent_id)
                    || row.profile_id != context.ingress.manager.profile_id
                    || row.manager_bot_id != context.ingress.manager.manager_bot_id
                    || row.telegram_bot_id != context.ingress.manager.telegram_bot_id
                    || row.manager_authority_epoch != context.ingress.manager.manager_authority_epoch
                    || row.sender_id != context.ingress.sender_id || row.chat_id != context.ingress.chat_id
                    || row.ingress_id.as_ref() != Some(&context.ingress.ingress.receipt_id)
                    || row.ingress_claim_id.as_ref() != Some(&context.ingress.ingress.claim_id)
                {
                    return Err(refused());
                }
                if state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
                    doc! {"_id":&row.id,"state":"pending","reference.kind":"public_signup","expires_at":{"$gt":bson::DateTime::now()}},
                    doc! {"$set":{"state":"dispatching","claim_id":claim.as_str(),"updated_at":bson::DateTime::now()}},
                ).session(&mut *session).await?.matched_count != 1 {
                    return Ok(CapturedSend::Duplicate);
                }
                let credential=state.db.collection::<PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME)
                    .find_one(doc! {"provider":"telegram-new","fields.manager_bot_id":&row.manager_bot_id,"fields.observation_id":observation,"fields.webhook_ready":"true"})
                    .session(&mut *session).await?.ok_or_else(refused)?;
                Ok(CapturedSend::Ready(Box::new(credential)))
            }.await;
            tx::transaction_result(result)
        }),
    ).await.map_err(tx::map_transaction_error);
    let credential = match result {
        Ok(CapturedSend::Ready(credential)) => credential,
        Ok(CapturedSend::Duplicate | CapturedSend::Channel(_)) => {
            return Ok(PublicSignupDeliveryOutcome::Unavailable);
        }
        Ok(CapturedSend::Deferred)
        | Err(AppError::Conflict(_) | AppError::NotFound(_) | AppError::ValidationError(_)) => {
            refuse_pending(state, &row).await?;
            return Ok(PublicSignupDeliveryOutcome::Unavailable);
        }
        Err(error) => return Err(error),
    };
    let body = super::platform_manager_command_service::PersonalCommandReply::Link { token }
        .render(&state.config.frontend_url)?;
    if body.encode_utf16().count() > 4096 {
        finish(state, &row, &claim, "refused", vec![], None).await?;
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    let outcome = send_claimed(state, &row, &claim, body, credential, api).await?;
    report_delivery(&outcome, "public_signup");
    Ok(match outcome {
        DeliveryOutcome::Sent { .. } | DeliveryOutcome::ChannelSent { .. } => {
            PublicSignupDeliveryOutcome::Sent
        }
        DeliveryOutcome::Uncertain => PublicSignupDeliveryOutcome::Uncertain,
        DeliveryOutcome::Duplicate
        | DeliveryOutcome::Deferred { .. }
        | DeliveryOutcome::Refused(_) => PublicSignupDeliveryOutcome::Unavailable,
    })
}

fn report_delivery(outcome: &DeliveryOutcome, source: &'static str) {
    match outcome {
        DeliveryOutcome::Sent {
            telegram_message_ids,
        } => tracing::debug!(
            source,
            message_count = telegram_message_ids.len(),
            "Manager delivery confirmed by Telegram"
        ),
        DeliveryOutcome::ChannelSent { message_ids } => tracing::debug!(
            source,
            message_count = message_ids.len(),
            "Shared delivery accepted by channel provider"
        ),
        DeliveryOutcome::Duplicate => tracing::debug!(source, "Manager delivery already claimed"),
        DeliveryOutcome::Deferred { retry_at } => {
            tracing::debug!(source,%retry_at,"Manager delivery deferred to durable retry")
        }
        DeliveryOutcome::Refused(reason) => {
            tracing::info!(source, ?reason, "Manager delivery refused")
        }
        DeliveryOutcome::Uncertain => tracing::warn!(
            source,
            "Manager delivery outcome uncertain; automatic replay disabled"
        ),
    }
}

pub async fn sweep(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Arc<dyn PublicTurnProducer>,
    limit: u32,
) -> AppResult<RecoveryBatch> {
    sweep_with_api(
        state,
        identity,
        public,
        limit,
        &super::telegram_new_api::TelegramApi::new(&state.http_client),
    )
    .await
}
pub(crate) async fn sweep_with_api(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Arc<dyn PublicTurnProducer>,
    limit: u32,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<RecoveryBatch> {
    let batch = limit.min(100);
    if batch == 0 {
        return Ok(RecoveryBatch {
            processed: 0,
            has_more: false,
        });
    }
    let now = Utc::now();
    let stale_before = bson::DateTime::from_chrono(now - Duration::minutes(2));
    let due = bson::DateTime::from_chrono(now);
    let mut rows = state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .find(doc! {"$or":[
            {"state":"dispatching","updated_at":{"$lte":stale_before}},
            {"state":"pending","retry_at":{"$lte":due}}
        ]})
        .sort(doc! {"updated_at":1,"_id":1})
        .limit(i64::from(batch))
        .await?;
    let mut processed = 0;
    while let Some(row) = rows.try_next().await? {
        // A process lost after claim may have sent. Never replay the attempt.
        if row.state == DeliveryState::Dispatching {
            state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
                doc! {"_id":&row.id,"state":"dispatching","claim_id":&row.claim_id,"updated_at":{"$lte":stale_before}},
                doc! {"$set":{"state":"uncertain","updated_at":due}},
            ).await?;
        } else if row.expires_at <= now
            || matches!(
                row.reference,
                DeliveryReference::Command | DeliveryReference::PublicSignup { .. }
            )
        {
            refuse_pending(state, &row).await?;
        } else {
            let body = match &row.reference {
                DeliveryReference::Reply { reply_id, .. } => match state
                    .db
                    .collection::<AssistantMessage>(MESSAGES)
                    .find_one(doc! {"_id":reply_id})
                    .await?
                {
                    Some(message) => match render_personal(state, &message, None).await {
                        Ok(body) => Some(body),
                        Err(
                            AppError::Conflict(_)
                            | AppError::NotFound(_)
                            | AppError::Forbidden(_)
                            | AppError::ValidationError(_),
                        ) => None,
                        Err(error) => return Err(error),
                    },
                    None => None,
                },
                DeliveryReference::PublicReply { .. } => {
                    match super::platform_public_credential_service::load_public_reply(
                        state,
                        &public_destination(&row)?,
                        &public_reply_ref(&row)?,
                    )
                    .await
                    {
                        Ok(body) => Some(body.to_string()),
                        Err(
                            AppError::Conflict(_)
                            | AppError::NotFound(_)
                            | AppError::Forbidden(_)
                            | AppError::ValidationError(_),
                        ) => None,
                        Err(error) => return Err(error),
                    }
                }
                DeliveryReference::Fixed { reason } => fixed_reason(reason).map(|reason| {
                    super::platform_channel_service::format_controls(
                        &row.manager_bot_id,
                        fixed_text(reason).to_owned(),
                    )
                }),
                DeliveryReference::Handoff => Some(handoff_text().to_owned()),
                DeliveryReference::Faq {
                    article_ids,
                    linked,
                    linking_enabled,
                    ..
                } => Some(super::platform_channel_service::format_controls(
                    &row.manager_bot_id,
                    super::platform_faq_service::render_mode(
                        article_ids,
                        *linked,
                        *linking_enabled,
                    )?,
                )),
                DeliveryReference::Command | DeliveryReference::PublicSignup { .. } => None,
            };
            if let Some(body) = body {
                match send(
                    state,
                    identity.clone(),
                    Some(public.clone()),
                    row,
                    Zeroizing::new(body),
                    api,
                )
                .await
                {
                    Ok(outcome) => report_delivery(&outcome, "recovery"),
                    Err(error) => tracing::warn!(
                        error_code = error.error_code(),
                        "Manager delivery recovery remains pending"
                    ),
                }
            } else {
                refuse_pending(state, &row).await?;
            }
        }
        processed += 1;
    }
    Ok(RecoveryBatch {
        processed,
        has_more: processed == batch,
    })
}

async fn refuse_pending(state: &AppState, row: &Delivery) -> AppResult<()> {
    state
        .db
        .collection::<Delivery>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id,"state":"pending"},
            doc! {"$set":{"state":"refused","updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

fn handoff_text() -> &'static str {
    "Open this manager's private chat to use your own NyxBot. Group chats cannot link or show personal replies."
}

fn fixed_reason(reason: &str) -> Option<UnavailableReason> {
    // Only fixed, bounded strings can be reconstructed; never an arbitrary body.
    use UnavailableReason::*;
    [
        PersonalDisabled,
        OwnerUnavailable,
        OriginRevoked,
        InsufficientFunds,
        CapacityLimited,
        EmergencyStopped,
        ManagerUnavailable,
        TransportUnavailable,
        PublicDisabled,
        PublicNotReady,
        PublicQuotaLimited,
        SponsorUnavailable,
        RuntimeUninitialized,
        EngineUnavailable,
        ModeDisabled,
    ]
    .into_iter()
    .find(|candidate| format!("{candidate:?}") == reason)
}

#[cfg(test)]
mod channel_notice_tests {
    #[test]
    fn discord_action_notice_preserves_summary_and_uses_deliverable_command() {
        let summary = "Reply yes MODEL_TEXT; https://example.test/a_b";
        let text = super::action_notice(summary, "A1B2", true);
        assert!(text.contains(summary));
        assert!(text.contains("/chat message with yes A1B2"));
        assert!(super::action_notice(summary, "A1B2", false).contains("Reply yes A1B2"));
    }
}

#[cfg(test)]
mod channel_pre_send_tests {
    use super::*;
    use mongodb::bson::Document;
    #[tokio::test]
    async fn expired_discord_context_falls_back_to_same_private_dm_and_settles_sent() {
        let (db, _, _, profile, bot) = super::super::platform_channel_service::tests::fixture(
            "discord_expired_reply_context",
            "discord",
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        ensure_indexes(&db).await.unwrap();
        let input = super::super::channel_platform::InboundMessage {
            platform_message_id: "interaction-one".into(),
            conversation_id: "D1".into(),
            conversation_type: "private".into(),
            sender_platform_id: "U1".into(),
            sender_display_name: None,
            content_type: "text".into(),
            text: Some("/faq".into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            thread_id: None,
            raw_data: serde_json::json!({}),
        };
        let update = super::super::platform_channel_service::verified_input(&db, &profile, &input)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&profile.id},
                doc! {"$set":{"public_enabled":true}},
            )
            .await
            .unwrap();
        let identity = super::super::platform_personal_binding_service::producer(state.clone());
        super::super::platform_manager_dispatch_service::admit_text_update(
            &db,
            identity.as_ref(),
            &update,
            "/faq",
            true,
        )
        .await
        .unwrap();
        let row = db
            .collection::<Delivery>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        db.collection::<Delivery>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"state":"dispatching","claim_id":"expiry-claim"}},
            )
            .await
            .unwrap();
        // Invalid expired ciphertext must never be decrypted or sent to Discord.
        db.collection::<Document>(crate::models::platform_channel_identity::COLLECTION_NAME).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"profile_id":&profile.id,"kind":"message","external_key":"interaction-one","handle":update.message_id,"sender_id":"U1","conversation_id":"D1",
            "reply_context_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0]},"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(1)),
        }).await.unwrap();
        let mut session = db.client().start_session().await.unwrap();
        let snapshot = super::super::platform_channel_service::CredentialSnapshot::load(
            &db,
            &mut session,
            &bot,
            false,
        )
        .await
        .unwrap();
        let mut prepared = prepare_channel_send(
            &state,
            &row,
            Zeroizing::new("Long-running result".into()),
            &snapshot,
        )
        .await
        .unwrap();
        assert!(
            prepared
                .reply
                .metadata
                .as_ref()
                .unwrap()
                .get("interaction_thread_id")
                .is_none()
        );
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/channels/D1/messages"))
            .and(wiremock::matchers::body_json(serde_json::json!({"content":"Long-running result","allowed_mentions":{"parse":[],"replied_user":false},"flags":4})))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"discord-result"}))).expect(1).mount(&server).await;
        prepared.adapter = Box::new(
            super::super::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                &server.uri(),
            ),
        );
        assert!(matches!(
            send_prepared_channel(&state, &row, "expiry-claim", &snapshot, prepared)
                .await
                .unwrap(),
            DeliveryOutcome::ChannelSent { .. }
        ));
        let finished = db
            .collection::<Delivery>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(finished.state, DeliveryState::Sent));
        assert_eq!(finished.channel_message_ids, vec!["discord-result"]);
        server.verify().await;
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn missing_peer_before_provider_effect_is_refused_without_uncertain_claim() {
        let (db, update, _, profile, bot) = super::super::platform_channel_service::tests::fixture(
            "generic_no_effect_settlement",
            "slack",
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        ensure_indexes(&db).await.unwrap();
        let identity = super::super::platform_personal_binding_service::producer(state.clone());
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&profile.id},
                doc! {"$set":{"public_enabled":true}},
            )
            .await
            .unwrap();
        super::super::platform_manager_dispatch_service::admit_text_update(
            &db,
            identity.as_ref(),
            &update,
            "/faq",
            true,
        )
        .await
        .unwrap();
        let row = db
            .collection::<Delivery>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        db.collection::<Delivery>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"state":"dispatching","claim_id":"test-claim"}},
            )
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        let snapshot = super::super::platform_channel_service::CredentialSnapshot::load(
            &db,
            &mut session,
            &bot,
            false,
        )
        .await
        .unwrap();
        assert!(matches!(
            send_channel_claimed(
                &state,
                &row,
                "test-claim",
                Zeroizing::new("Safe FAQ".into()),
                &snapshot,
                None,
            )
            .await
            .unwrap(),
            DeliveryOutcome::Refused(_)
        ));
        let finished = db
            .collection::<Delivery>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(finished.state, DeliveryState::Refused));
        assert!(finished.channel_message_ids.is_empty());
        db.drop().await.unwrap();
    }
}

/// Inject only the provider HTTP adapter; the service integration uses the same
/// canonical enqueue, pacing, claim transaction and effect settlement as production.
#[cfg(test)]
pub(crate) async fn deliver_channel_with_test_adapter(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    destination: &ReplyDestination,
    reply: &ReplyRef,
    adapter: Box<dyn super::channel_platform::PlatformAdapter>,
) -> AppResult<DeliveryOutcome> {
    let message = load_personal_saved(state, destination, reply).await?;
    let body = render_personal(state, &message, None).await?;
    deliver_with_adapter(
        state,
        identity,
        destination,
        &DeliveryRef::Reply(reply.clone()),
        ReplyContent::Text(body),
        &super::telegram_new_api::TelegramApi::new(&state.http_client),
        Some(adapter),
    )
    .await
}
