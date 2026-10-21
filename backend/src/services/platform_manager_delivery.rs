//! Ordered delivery of retained personal replies and ephemeral command output.
mod media;
use super::{
    api_key_mutation_service as tx, platform_bot_runtime_control_service as controls,
    platform_chat_presentation::ChatPresentation, platform_entry_scope as scope,
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
static CONTROL_SENDS: Semaphore = Semaphore::const_new(4);
static STATUS_SENDS: Semaphore = Semaphore::const_new(4);
static PUBLIC_SENDS: Semaphore = Semaphore::const_new(4);

#[cfg(test)]
tokio::task_local! {
    static TEST_CONTROL_CAPACITY: Arc<Semaphore>;
}

#[cfg(test)]
pub(super) async fn with_control_capacity_for_test<T>(
    capacity: Arc<Semaphore>,
    work: impl std::future::Future<Output = T>,
) -> T {
    TEST_CONTROL_CAPACITY.scope(capacity, work).await
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    super::channel_text_delivery::ensure_indexes(db).await?;
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
    AppError::Conflict("Chat reply authority is unavailable".into())
}

pub(crate) fn fixed_text(reason: UnavailableReason, chat: ChatPresentation) -> String {
    let link = if chat == ChatPresentation::Telegram {
        "/link"
    } else {
        chat.login()
    };
    let help = chat.help();
    match reason {
        UnavailableReason::ConfirmationRejected => {
            format!(
                "This confirmation could not be accepted. The code or account review may be expired, invalid, or for another chat, or linking may be unavailable. Send {link} for a fresh account review when personal linking is enabled, then use its one-time confirmation in this same private chat."
            )
        }
        UnavailableReason::ControlInvalid => {
            format!(
                "That command could not be completed. Use {help} for supported controls, or send {link} for a fresh account review in this private chat."
            )
        }
        UnavailableReason::ControlRateLimited => {
            format!(
                "Too many chat controls were requested. Wait a moment, then retry here. Use {help} for supported controls."
            )
        }
        UnavailableReason::UnsupportedInput => {
            format!(
                "This chat supports text questions. Use {help} for the available controls, or open NyxID for attachments and actions."
            )
        }
        UnavailableReason::PersonalDisabled => {
            "Personal NyxBot is paused for this manager. Your saved thread remains in NyxID.".into()
        }
        UnavailableReason::OwnerUnavailable | UnavailableReason::OriginRevoked => {
            "This personal link is unavailable. Open NyxID to review your account and links.".into()
        }
        UnavailableReason::InsufficientFunds => {
            "Your NyxBot could not start with the available credits. Review Billing in NyxID."
                .into()
        }
        UnavailableReason::CapacityLimited => {
            "Your NyxBot is busy. Wait for the current work or open NyxID.".into()
        }
        UnavailableReason::PublicQuotaLimited => {
            format!(
                "Public chat has reached a daily limit. Try again after midnight UTC, or send {link} to use your own NyxBot."
            )
        }
        UnavailableReason::PublicTurnFailed => {
            format!(
                "The public guide could not finish this turn. Please try again to start fresh, or send {link} to use your own NyxBot."
            )
        }
        UnavailableReason::SponsorUnavailable => {
            format!(
                "Public chat funding is unavailable. Send {link} to use your own NyxBot when personal linking is enabled."
            )
        }
        UnavailableReason::EmergencyStopped
        | UnavailableReason::ManagerUnavailable
        | UnavailableReason::TransportUnavailable => {
            "This manager is currently unavailable. Open NyxID to continue.".into()
        }
        UnavailableReason::PublicDisabled
        | UnavailableReason::PublicNotReady
        | UnavailableReason::RuntimeUninitialized => {
            format!(
                "Public chat is unavailable. Send {link} in this private chat to use your own NyxBot when personal linking is enabled."
            )
        }
        _ => {
            format!(
                "This request is unavailable. Open NyxID to continue, or send an original {link} command in the manager's private chat."
            )
        }
    }
}

fn fixed_text_for(reason: UnavailableReason, entry: &EntryIdentity) -> AppResult<String> {
    Ok(fixed_text(reason, ChatPresentation::for_entry(entry)?))
}

fn manager_lane(row: &Delivery) -> String {
    match &row.entry {
        EntryIdentity::TelegramV1 { .. } => row.manager_bot_id.clone(),
        EntryIdentity::NativeV2 { .. } => {
            super::mcp_service::canonical_sha256(serde_json::json!([
                "native-manager-send-lane:v2",
                row.profile_id,
                row.entry,
            ]))
        }
    }
}

#[cfg(test)]
pub(crate) fn render_saved(message: &AssistantMessage, _frontend: &str) -> String {
    render_saved_text(message)
}

fn render_saved_text(message: &AssistantMessage) -> String {
    let text = message.text.clone();
    if text.trim().is_empty() {
        return if message.attachments.is_empty() {
            "Your NyxBot turn finished.".into()
        } else {
            "Your NyxBot attachments are ready.".into()
        };
    }
    text
}

fn conversation_url(frontend: &str, conversation: &str) -> String {
    let Ok(mut frontend) = reqwest::Url::parse(frontend) else {
        return String::new();
    };
    frontend.set_path("/");
    frontend.set_query(None);
    frontend.set_fragment(None);
    super::assistant_links::AssistantPage::Conversation { conversation }.url(frontend.as_str())
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
        sender: origin.authority.sender.clone(),
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
        || exact.sender != destination.sender
    {
        return Err(refused());
    }
    Ok(saved)
}

fn append_card_notices(
    message: &AssistantMessage,
    frontend: &str,
    cards: &[crate::models::assistant_acknowledgement::AssistantAcknowledgement],
    key_id: &str,
    group_id: Option<&str>,
    chat: ChatPresentation,
) -> AppResult<String> {
    use crate::models::assistant_acknowledgement::CardRouting;
    let mut rendered = message.clone();
    let mut notices = String::new();
    let mut chat_actions = 0;
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
            let instruction = chat.action_instruction(&code)?;
            notices.push_str(&format!("\n\nAction: {summary}\n{instruction}"));
            chat_actions += 1;
        }
    }
    if chat_actions < cards.len() {
        notices.push_str(&format!(
            "\n\nPending actions require review in NyxID: <{}>",
            conversation_url(frontend, &message.conversation_id)
        ));
    }
    if chat != ChatPresentation::Telegram
        && message.attachments.iter().any(|attachment| {
            attachment.origin == "tool"
                && attachment.chat_delivery != Some(false)
                && !(attachment.chat_delivery.is_none() && attachment.label == "Machine screenshot")
        })
    {
        notices.push_str(&format!(
            "\n\nThis chat cannot receive these attachments. Open them in NyxID: <{}>",
            conversation_url(frontend, &message.conversation_id)
        ));
    }
    rendered.text.push_str(&notices);
    Ok(render_saved_text(&rendered))
}

pub(crate) async fn render_personal(
    state: &AppState,
    message: &AssistantMessage,
    mut session: Option<&mut mongodb::ClientSession>,
) -> AppResult<String> {
    use crate::models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKS,
    };
    // Pin only card identities, never private summaries or rendered bodies. A
    // decided/expired card must not change the plan after an accepted prefix.
    let deliveries = state.db.collection::<bson::Document>(COLLECTION_NAME);
    let saved_filter = doc! {"reference.reply_id": &message.id, "reference.kind": "reply"};
    let snapshot = match session.as_deref_mut() {
        Some(tx) => {
            deliveries
                .find_one(saved_filter.clone())
                .session(tx)
                .await?
        }
        None => deliveries.find_one(saved_filter.clone()).await?,
    };
    let ids = snapshot
        .as_ref()
        .and_then(|row| row.get_array("notice_card_ids").ok());
    let mut filter = doc! {"user_id":&message.user_id,"conversation_id":&message.conversation_id,
    "kind":"action","status":"pending","decider":"user","authored_skill":bson::Bson::Null,
    "expires_at":{"$gt":bson::DateTime::now()}};
    if let Some(ids) = ids {
        filter = doc! {"_id": {"$in": ids}, "user_id": &message.user_id,
        "conversation_id": &message.conversation_id};
    }
    let collection = state.db.collection::<AssistantAcknowledgement>(ACKS);
    let conversations = state.db.collection::<AssistantConversation>(CONVERSATIONS);
    let (cards, thread): (Vec<AssistantAcknowledgement>, Option<AssistantConversation>) =
        match session.as_deref_mut() {
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
    if ids.is_none()
        && let Some(saved) = snapshot
    {
        let ids: Vec<_> = cards.iter().map(|card| card.id.as_str()).collect();
        // Concurrent renderers may pin the same plan. Accept that exact
        // snapshot again, but never overwrite a different renderer's plan.
        let update = deliveries.update_one(
            doc! {"_id":saved.get_str("_id").map_err(|_| refused())?,
            "$or":[{"notice_card_ids":{"$exists":false}}, {"notice_card_ids":&ids}]},
            doc! {"$set":{"notice_card_ids":&ids}},
        );
        let changed = match session {
            Some(tx) => update.session(tx).await?,
            None => update.await?,
        };
        if changed.matched_count != 1 {
            return Err(AppError::Conflict("Reply notice snapshot changed".into()));
        }
    }
    let thread = thread.ok_or_else(refused)?;
    let entry = &thread.platform_thread.as_ref().ok_or_else(refused)?.entry;
    append_card_notices(
        message,
        &state.config.frontend_url,
        &cards,
        &thread.credential_api_key_id,
        thread.group_id.as_deref(),
        ChatPresentation::for_entry(entry)?,
    )
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

pub(crate) fn saved_personal_reply_with_api<'a>(
    state: &'a AppState,
    row: &'a AssistantConversation,
    reply: &'a ReplyRef,
    api: &'a super::telegram_new_api::TelegramApi<'_>,
) -> futures::future::BoxFuture<'a, AppResult<Option<DeliveryOutcome>>> {
    // Owner turn settlement also calls this for replies without a platform route.
    // Keep the transport-specific delivery future outside those caller frames.
    Box::pin(saved_personal_reply_with_api_impl(state, row, reply, api))
}

async fn saved_personal_reply_with_api_impl(
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
                && destination.sender == origin.authority.sender =>
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

async fn enqueue(
    state: &AppState,
    destination: &ReplyDestination,
    reference: DeliveryReference,
    ingress: Option<&IngressClaim>,
    origin: Option<crate::models::assistant_conversation::PlatformTurnOrigin>,
    origin_turn_id: Option<&str>,
) -> AppResult<Delivery> {
    let identity = match &reference {
        DeliveryReference::Reply { reply_id, .. }
        | DeliveryReference::PublicReply { reply_id, .. } => format!("reply:{reply_id}"),
        DeliveryReference::Command { .. } => "command".into(),
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
    let dedupe_key = match &destination.manager.entry {
        EntryIdentity::TelegramV1 { telegram_bot_id } => hex::encode(Sha256::digest(
            format!(
                "manager-send:v1:{}:{}:{}:{identity}",
                destination.manager.profile_id, telegram_bot_id, admission
            )
            .as_bytes(),
        )),
        EntryIdentity::NativeV2 { .. } => {
            super::mcp_service::canonical_sha256(serde_json::json!([
                "manager-send:v2",
                destination.manager.profile_id,
                destination.manager.entry,
                admission,
                identity,
            ]))
        }
    };
    let now = Utc::now();
    let row = Delivery {
        id: uuid::Uuid::new_v4().to_string(),
        dedupe_key: dedupe_key.clone(),
        profile_id: destination.manager.profile_id.clone(),
        manager_bot_id: destination.manager.manager_bot_id.clone(),
        entry: destination.manager.entry.clone(),
        manager_authority_epoch: destination.manager.manager_authority_epoch,
        sender: destination.sender.clone(),
        ingress_id: ingress.map(|claim| claim.receipt_id.clone()),
        ingress_claim_id: ingress.map(|claim| claim.claim_id.clone()),
        origin,
        origin_turn_id: origin_turn_id.map(str::to_owned),
        reference,
        state: DeliveryState::Pending,
        claim_id: None,
        telegram_message_ids: vec![],
        platform_message_ids: vec![],
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
        && (matches!(
            stored.reference,
            DeliveryReference::Reply { .. } | DeliveryReference::PublicReply { .. }
        ) || matches!(&stored.reference, DeliveryReference::Fixed { reason } if reason == "PublicTurnFailed"))
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

/// Admission and its recoverable metadata commit together; raw questions never enter storage.
pub(crate) async fn queue_faq_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    update: &VerifiedManagerUpdate,
    claim: &IngressClaim,
    controls: &PlatformBotRuntimeControls,
    text: &str,
) -> AppResult<()> {
    let help = super::platform_help_service::published(db, &update.profile_id).await?;
    let article_ids = super::platform_faq_service::select_from(
        text,
        super::platform_faq_service::all_articles(&help.content),
    );
    let help = crate::models::platform_help::HelpReference {
        library_id: help.library_id,
        revision: help.revision,
    };
    let now = Utc::now();
    let row = Delivery {
        id: uuid::Uuid::new_v4().to_string(),
        dedupe_key: format!("faq:{}", claim.receipt_id),
        profile_id: update.profile_id.clone(),
        manager_bot_id: update.manager_bot_id.clone(),
        entry: update.entry.clone(),
        manager_authority_epoch: controls.manager_authority_epoch,
        sender: update.sender.clone(),
        ingress_id: Some(claim.receipt_id.clone()),
        ingress_claim_id: Some(claim.claim_id.clone()),
        origin: None,
        origin_turn_id: None,
        reference: DeliveryReference::Faq {
            help: Some(help),
            article_ids,
            linking_enabled: controls.personal_enabled,
            public_policy_revision: controls.public_policy_revision,
        },
        state: DeliveryState::Pending,
        claim_id: None,
        telegram_message_ids: vec![],
        platform_message_ids: vec![],
        expires_at: now + chrono::Duration::minutes(2),
        retry_at: now,
        updated_at: now,
    };
    db.collection::<Delivery>(COLLECTION_NAME)
        .insert_one(row)
        .session(&mut *session)
        .await?;
    if db.collection::<PlatformManagerIngress>(INGRESS).update_one(
        doc! {"_id":&claim.receipt_id,"claim_id":&claim.claim_id,"phase":"admitted"},
        doc! {"$set":{"phase":"settled","settled_at":bson::DateTime::from_chrono(now),"delivery_recovery_queued":true}}
    ).session(session).await?.matched_count!=1 {return Err(refused());}
    Ok(())
}

pub(crate) async fn deliver_faq(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    claim: &IngressClaim,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<bool> {
    let row=state.db.collection::<Delivery>(COLLECTION_NAME).find_one(doc! {"dedupe_key":format!("faq:{}",claim.receipt_id),"ingress_claim_id":&claim.claim_id}).await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let body =
        super::platform_faq_service::render_saved(&state.db, &row.reference, &row.entry).await?;
    send(state, identity, None, row, Zeroizing::new(body), api).await?;
    Ok(true)
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
        (Some(id), Some(claim)) => {
            let receipt = state.db.collection::<PlatformManagerIngress>(INGRESS)
                .find_one(doc! {"_id":id,"claim_id":claim,"profile_id":&row.profile_id,
                    "manager_bot_id":&row.manager_bot_id,"manager_authority_epoch":row.manager_authority_epoch})
                .session(&mut *session).await?.ok_or_else(refused)?;
            if receipt.entry != row.entry || receipt.sender != row.sender {
                return Err(refused());
            }
            Some(receipt)
        }
        (None, None) if matches!(row.reference, DeliveryReference::Reply { .. }) => None,
        _ => return Err(refused()),
    };
    let profile=state.db.collection::<PlatformBotConfig>(PROFILES).find_one(doc! {"_id":&row.profile_id,"manager_bot_id":&row.manager_bot_id,"deleted_at":bson::Bson::Null}).session(&mut *session).await?.ok_or_else(refused)?;
    let policy = controls::get_in_session(&state.db, session, &row.profile_id)
        .await?
        .ok_or_else(refused)?;
    if policy.retired
        || policy.manager_authority_epoch != row.manager_authority_epoch
        || policy.entry != row.entry
        || policy.manager_bot_id != row.manager_bot_id
    {
        return Err(refused());
    }
    let observation = match &row.entry {
        EntryIdentity::TelegramV1 { .. } => {
            let current = state
                .db
                .collection::<PlatformCredential>(
                    crate::models::platform_credential::COLLECTION_NAME,
                )
                .find_one(super::telegram_manager_registry::filter(
                    &row.manager_bot_id,
                ))
                .session(&mut *session)
                .await?;
            if current
                .as_ref()
                .and_then(|credential| credential.fields.get("manager_bot_id"))
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
            profile.observation_id.clone()
        }
        EntryIdentity::NativeV2 { .. } => {
            receipt.as_ref().ok_or_else(refused)?.observation_id.clone()
        }
    };
    let controls = controls::fence_manager_in_session(
        &state.db,
        session,
        &row.profile_id,
        &row.manager_bot_id,
        &row.entry,
        Some(&observation),
    )
    .await?
    .ok_or_else(refused)?;
    if controls.manager_authority_epoch != row.manager_authority_epoch || controls.retired {
        return Err(refused());
    }
    Ok(Some((receipt, controls, observation)))
}

/// Fixed system notice after a saved public failure. No execution proof,
/// transcript body or credential is admitted by this path.
async fn fence_public_failure_notice(
    state: &AppState,
    session: &mut mongodb::ClientSession,
    delivery: &Delivery,
    receipt: &PlatformManagerIngress,
    controls: &PlatformBotRuntimeControls,
) -> AppResult<()> {
    use crate::models::{
        platform_public_contract::{ACTIVATIONS, PublicDeploymentActivation},
        platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
        platform_public_turn::{COLLECTION_NAME as TURNS, PlatformPublicTurn},
    };
    let IngressAudience::Public {
        public_policy_revision,
    } = &receipt.audience
    else {
        return Err(refused());
    };
    if receipt.phase != IngressPhase::Settled
        || !controls.public_enabled
        || controls.emergency_stopped
        || controls.retired
        || *public_policy_revision != controls.public_policy_revision
        || scope::validate(
            &delivery.entry,
            &delivery.manager_bot_id,
            Some(&delivery.sender),
        )
        .is_err()
    {
        return Err(refused());
    }
    let notice_expires_at = receipt.settled_at.ok_or_else(refused)? + Duration::minutes(2);
    if delivery.expires_at > notice_expires_at {
        return Err(refused());
    }
    let execution = receipt.execution.as_ref().ok_or_else(refused)?;
    let public_session_id = execution.public_session_id.as_ref().ok_or_else(refused)?;
    let policy_digest = execution
        .public_policy_digest
        .as_ref()
        .ok_or_else(refused)?;
    let reply_id = receipt.reply_id.as_ref().ok_or_else(refused)?;
    let settled_receipt = state.db.collection::<PlatformManagerIngress>(INGRESS)
        .update_one(doc! {
            "_id": &receipt.id, "claim_id": &receipt.claim_id, "phase": "settled", "reply_id": reply_id,
            "execution.public_session_id": public_session_id, "execution.turn_id": &execution.turn_id,
            "execution.conversation_id": &execution.conversation_id,
            "execution.payer_user_id": &execution.payer_user_id, "execution.public_policy_digest": policy_digest,
            "$expr": {"$gt": [{"$dateAdd": {"startDate": "$settled_at", "unit": "minute", "amount": 2}}, "$$NOW"]},
        }, doc! {"$inc": {"effect_revision": 1_i64}})
        .session(&mut *session).await?;
    if settled_receipt.matched_count != 1 {
        return Err(refused());
    }
    let turn = state
        .db
        .collection::<PlatformPublicTurn>(TURNS)
        .update_one(
            doc! {
                "_id": &execution.turn_id, "conversation_id": &execution.conversation_id,
                "public_session_id": public_session_id, "receipt_id": &receipt.id,
                "claim_id": &receipt.claim_id, "reply_id": reply_id, "outcome": "failed",
                "policy_digest": policy_digest, "sponsor_owner_user_id": &execution.payer_user_id,
                "$expr": {"$gt": ["$expires_at", "$$NOW"]},
            },
            doc! {"$inc": {"failure_notice_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if turn.matched_count != 1 {
        return Err(refused());
    }
    let public = state.db.collection::<PlatformPublicSession>(SESSIONS)
        .find_one_and_update(doc! {
            "_id": public_session_id, "profile_id": &delivery.profile_id,
            "manager_bot_id": &delivery.manager_bot_id, scope::entry_key(&delivery.entry):scope::entry_value(&delivery.entry),
            "manager_authority_epoch": delivery.manager_authority_epoch,
            scope::actor_key(&delivery.sender,true):scope::actor_value(&delivery.sender), scope::destination_key(&delivery.sender,true):scope::destination_value(&delivery.sender),
            "public_policy_revision": public_policy_revision,
            "sponsor_owner_user_id": &execution.payer_user_id, "policy_digest": policy_digest,
            "$expr": {"$gt": ["$expires_at", "$$NOW"]},
        }, doc! {"$inc": {"effect_revision": 1_i64}})
        .session(&mut *session).await?.ok_or_else(refused)?;
    let selected = controls.public_policy.as_ref().ok_or_else(refused)?;
    if selected.sponsor_owner_user_id != public.sponsor_owner_user_id
        || selected.nyxagent_profile != public.nyxagent_profile
        || !super::platform_public_agent_service::fence_failure_notice_scopes_in_session(
            &state.db, session, &public,
        )
        .await?
    {
        return Err(refused());
    }
    let active = state.db.collection::<PublicDeploymentActivation>(ACTIVATIONS)
        .update_one(doc! {
            "_id": &delivery.profile_id, "policy_digest": policy_digest,
            "binding": bson::to_bson(&super::platform_public_policy::binding(&controls::policy(controls))?)
                .map_err(|_| refused())?,
        }, doc! {"$inc": {"effect_revision": 1_i64}})
        .session(&mut *session).await?;
    let sponsor = state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &public.sponsor_owner_user_id, "is_active": true, "user_type": "org"},
            doc! {"$inc": {"public_failure_notice_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if active.matched_count != 1 || sponsor.matched_count != 1 {
        return Err(refused());
    }
    Ok(())
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
        DeliveryReference::Faq {
            public_policy_revision,
            linking_enabled,
            ..
        } => {
            let receipt = receipt.as_ref().ok_or_else(refused)?;
            if controls.emergency_stopped
                || !controls.public_enabled
                || controls.public_policy_revision != *public_policy_revision
                || controls.personal_enabled != *linking_enabled
                || receipt.phase != IngressPhase::Settled
                || receipt.audience
                    != (IngressAudience::Public {
                        public_policy_revision: *public_policy_revision,
                    })
                || body
                    != super::platform_faq_service::render_saved(
                        &state.db,
                        &row.reference,
                        &row.entry,
                    )
                    .await?
            {
                return Err(refused());
            }
        }
        DeliveryReference::Reply {
            conversation_id,
            turn_id,
            reply_id,
        } => {
            let origin = row.origin.as_ref().ok_or_else(refused)?;
            let origin = personal::contract_origin(origin)?;
            if origin.authority.manager != controls::policy(&controls).manager
                || origin.authority.sender != row.sender
                || scope::validate(&row.entry, &row.manager_bot_id, Some(&row.sender)).is_err()
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
        DeliveryReference::Command { effect } => {
            let receipt = receipt.as_ref().ok_or_else(refused)?;
            let effect = effect.as_ref().ok_or_else(refused)?;
            if receipt.audience != IngressAudience::Control
                || !receipt.original_private_command
                || receipt.phase != IngressPhase::Settled
                || scope::validate(&row.entry, &row.manager_bot_id, Some(&row.sender)).is_err()
                || receipt.control_effect.as_ref() != Some(effect)
            {
                return Err(refused());
            }
            let update = VerifiedManagerUpdate {
                profile_id: row.profile_id.clone(),
                manager_bot_id: row.manager_bot_id.clone(),
                entry: row.entry.clone(),
                observation_id: observation.clone(),
                sender: row.sender.clone(),
                chat_kind: ManagerChatKind::Private,
                event: receipt.event.clone(),
                command_eligibility: ManagerCommandEligibility::OriginalPrivateMessage,
            };
            if !super::platform_manager_command_service::fence_effect_in_session(
                &state.db,
                session,
                &update,
                &receipt.id,
                effect,
            )
            .await?
            {
                return Err(refused());
            }
            if let Some(reply) = effect.static_reply()
                && reply
                    .render(&state.config.frontend_url, &row.entry)?
                    .as_str()
                    != body
            {
                return Err(refused());
            }
            if let crate::models::platform_manager_ingress::ManagerCommandEffect::Management {
                selection,
                ..
            } = effect
            {
                let token = if matches!(
                    selection,
                    crate::models::platform_manager_management::ManagerManagementView::Onboarding
                ) {
                    let url = body
                        .strip_prefix(
                            "Create a Telegram bot for the NyxID account linked to this chat: ",
                        )
                        .and_then(|value| value.split_once('\n').map(|(url, _)| url))
                        .and_then(|value| reqwest::Url::parse(value).ok())
                        .ok_or_else(refused)?;
                    Some(Zeroizing::new(
                        url.path_segments()
                            .and_then(|mut parts| parts.next_back())
                            .ok_or_else(refused)?
                            .to_owned(),
                    ))
                } else {
                    None
                };
                if super::platform_manager_management::render_in_session(
                    &state.db,
                    session,
                    &update,
                    effect,
                    &state.config.frontend_url,
                    token.as_deref().map(String::as_str),
                )
                .await?
                .as_str()
                    != body
                {
                    return Err(refused());
                }
            }
            if let crate::models::platform_manager_ingress::ManagerCommandEffect::Link {
                intent_id,
            } = effect
            {
                let url = body
                    .strip_prefix("Review your NyxID account at ")
                    .and_then(|value| value.split_once('\n').map(|(url, _)| url))
                    .and_then(|url| reqwest::Url::parse(url).ok())
                    .ok_or_else(refused)?;
                let token = Zeroizing::new(
                    url.fragment()
                        .and_then(|f| f.strip_prefix("intent_token="))
                        .ok_or_else(refused)?
                        .to_owned(),
                );
                if !super::platform_link_intent_service::command_token_matches_in_session(
                    &state.db,
                    session,
                    state.auth_device_hmac_key.as_slice(),
                    intent_id,
                    &token,
                )
                .await?
                    || (super::platform_manager_command_service::PersonalCommandReply::Link {
                        token,
                    })
                    .render(&state.config.frontend_url, &row.entry)?
                    .as_str()
                        != body
                {
                    return Err(refused());
                }
            }
        }
        DeliveryReference::Fixed { reason } if reason == "PublicTurnFailed" => {
            if body != fixed_text_for(UnavailableReason::PublicTurnFailed, &row.entry)? {
                return Err(refused());
            }
            fence_public_failure_notice(
                state,
                session,
                row,
                receipt.as_ref().ok_or_else(refused)?,
                &controls,
            )
            .await?;
        }
        DeliveryReference::Fixed { .. } | DeliveryReference::Handoff => {
            // Fixed responses cannot expose account-specific or model text.
            if receipt.as_ref().ok_or_else(refused)?.audience == IngressAudience::Control {
                return Err(refused());
            }
        }
        // Public proof output has its own executing-turn/intent transaction.
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
            super::platform_public_credential_service::fence_public_reply_body_in_session(
                state,
                session,
                &destination,
                &reply,
                body,
            )
            .await?;
        }
        DeliveryReference::PublicSignup { .. } => return Err(refused()),
    }
    Ok(Some(observation))
}

struct NativeSendPreparation {
    adapter: Arc<dyn super::channel_platform::PlatformAdapter>,
    deadline: chrono::DateTime<Utc>,
}

async fn prepare_native_send(
    state: &AppState,
    row: &Delivery,
    deadline: Option<chrono::DateTime<Utc>>,
) -> AppResult<Option<NativeSendPreparation>> {
    let EntryIdentity::NativeV2 { platform_entry } = &row.entry else {
        return Ok(None);
    };
    prepare_selected_native_send(state, row, platform_entry, deadline).await
}

fn prepare_selected_native_send<'a>(
    state: &'a AppState,
    row: &'a Delivery,
    platform_entry: &'a crate::models::platform_entry_identity::NativeEntryIdentity,
    deadline: Option<chrono::DateTime<Utc>>,
) -> futures::future::BoxFuture<'a, AppResult<Option<NativeSendPreparation>>> {
    Box::pin(prepare_selected_native_send_impl(
        state,
        row,
        platform_entry,
        deadline,
    ))
}

async fn prepare_selected_native_send_impl(
    state: &AppState,
    row: &Delivery,
    platform_entry: &crate::models::platform_entry_identity::NativeEntryIdentity,
    deadline: Option<chrono::DateTime<Utc>>,
) -> AppResult<Option<NativeSendPreparation>> {
    let deadline = deadline.ok_or_else(refused)?;
    let remaining = (deadline - Utc::now()).to_std().map_err(|_| refused())?;
    tokio::time::timeout(remaining, async {
        let ingress = state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .find_one(doc! {
                "_id":row.ingress_id.as_ref().ok_or_else(refused)?,
                "claim_id":row.ingress_claim_id.as_ref().ok_or_else(refused)?,
                "profile_id":&row.profile_id,"manager_authority_epoch":row.manager_authority_epoch,
            })
            .await?
            .ok_or_else(refused)?;
        if ingress.entry != row.entry || ingress.sender != row.sender {
            return Err(refused());
        }
        let receipt = state
            .db
            .collection::<crate::models::platform_entry_identity::NativeEntryReceipt>(
                crate::models::platform_entry_identity::NATIVE_RECEIPTS,
            )
            .find_one(doc! {"_id":&ingress.observation_id,"profile_id":&row.profile_id})
            .await?
            .ok_or_else(refused)?;
        let SenderIdentity::NativeV2 { platform_sender } = &row.sender else {
            return Err(refused());
        };
        if receipt.configuration.identity != *platform_entry
            || receipt.sender != *platform_sender
            || receipt.manager_authority_epoch != row.manager_authority_epoch
        {
            return Err(refused());
        }
        let adapter =
            super::platform_native_reply_service::send_adapter(state, platform_entry.platform)?;
        super::platform_native_reply_service::refresh_before_claim(
            state,
            &receipt,
            deadline,
            adapter.as_ref(),
        )
        .await?;
        Ok(Some(NativeSendPreparation {
            adapter,
            deadline: deadline.min(receipt.expires_at),
        }))
    })
    .await
    .map_err(|_| refused())?
}

fn native_send_deadline(row: &Delivery) -> AppResult<Option<chrono::DateTime<Utc>>> {
    if !matches!(row.entry, EntryIdentity::NativeV2 { .. }) {
        return Ok(None);
    }
    let budget = if matches!(
        row.reference,
        DeliveryReference::Reply { .. }
            | DeliveryReference::PublicReply { .. }
            | DeliveryReference::Faq { .. }
    ) {
        std::time::Duration::from_secs(10 * 60)
    } else {
        std::time::Duration::from_secs(20)
    };
    #[cfg(test)]
    let budget = TEST_NATIVE_SEND_BUDGET
        .try_with(|value| *value)
        .unwrap_or(budget)
        .min(budget);
    Ok(Some(row.expires_at.min(
        Utc::now() + Duration::from_std(budget).map_err(|_| refused())?,
    )))
}

#[cfg(test)]
pub(crate) struct NativeDeliveryStall {
    pub entered: std::sync::atomic::AtomicBool,
    pub duration: std::time::Duration,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_NATIVE_SEND_BUDGET: std::time::Duration;
    pub(crate) static TEST_NATIVE_CLAIM_STALL: Arc<NativeDeliveryStall>;
    pub(crate) static TEST_NATIVE_FINISH_STALL: Arc<NativeDeliveryStall>;
    pub(crate) static TEST_NATIVE_CLEANUP_STALL: Arc<NativeDeliveryStall>;
}

#[cfg(test)]
async fn stall_native_claim_for_test(native: bool) {
    if native && let Ok(stall) = TEST_NATIVE_CLAIM_STALL.try_with(Arc::clone) {
        stall
            .entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(stall.duration).await;
    }
}

async fn before_native_deadline<T>(
    deadline: Option<chrono::DateTime<Utc>>,
    future: impl std::future::Future<Output = AppResult<T>>,
) -> Option<AppResult<T>> {
    let Some(deadline) = deadline else {
        return Some(future.await);
    };
    let remaining = (deadline - Utc::now()).to_std().ok()?;
    tokio::time::timeout(remaining, future).await.ok()
}

async fn cleanup_before_deadline(
    deadline: Option<chrono::DateTime<Utc>>,
    future: impl std::future::Future<Output = AppResult<()>>,
) -> AppResult<bool> {
    #[cfg(test)]
    let future = async {
        if deadline.is_some()
            && let Ok(stall) = TEST_NATIVE_CLEANUP_STALL.try_with(Arc::clone)
        {
            stall
                .entered
                .store(true, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(stall.duration).await;
        }
        future.await
    };
    let result = before_native_deadline(deadline, future).await;
    match result {
        Some(result) => result.map(|()| true),
        None => Ok(false),
    }
}

enum CapturedMaterial {
    Telegram(Box<PlatformCredential>),
    Native(Box<super::platform_native_reply_service::NativeSendAuthority>),
}
enum CapturedSend {
    Duplicate,
    Deferred,
    Ready(CapturedMaterial),
}

async fn capture_material_in_session(
    state: &AppState,
    session: &mut mongodb::ClientSession,
    row: &Delivery,
    claim: &str,
    observation: &str,
    native: Option<&NativeSendPreparation>,
) -> AppResult<CapturedSend> {
    if let EntryIdentity::NativeV2 { .. } = &row.entry {
        let native = native.ok_or_else(refused)?;
        if native.deadline <= Utc::now() {
            return Err(refused());
        }
        let ingress = state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .find_one(doc! {
                "_id":row.ingress_id.as_ref().ok_or_else(refused)?,
                "claim_id":row.ingress_claim_id.as_ref().ok_or_else(refused)?,
                "observation_id":observation,
            })
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        let material = super::platform_native_reply_service::capture_in_session(
            state,
            session,
            row,
            claim,
            &ingress,
            native.adapter.as_ref(),
            native.deadline,
        )
        .await?;
        return Ok(CapturedSend::Ready(CapturedMaterial::Native(Box::new(
            material,
        ))));
    }
    let credential=state.db.collection::<PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME)
        .find_one(doc! {"provider": {"$in":["telegram-new".to_owned(), format!("telegram-new/{}", &row.manager_bot_id)]}, "fields.manager_bot_id": &row.manager_bot_id,"fields.observation_id":observation,"fields.webhook_ready":"true"})
        .session(&mut *session).await?.ok_or_else(refused)?;
    Ok(CapturedSend::Ready(CapturedMaterial::Telegram(Box::new(
        credential,
    ))))
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_BEFORE_COMMAND_CLAIM: Arc<tokio::sync::Barrier>;
}

fn send<'a>(
    state: &'a AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    row: Delivery,
    body: Zeroizing<String>,
    api: &'a super::telegram_new_api::TelegramApi<'_>,
) -> futures::future::BoxFuture<'a, AppResult<DeliveryOutcome>> {
    Box::pin(send_impl(state, identity, public, row, body, api))
}

async fn send_impl(
    state: &AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
    row: Delivery,
    body: Zeroizing<String>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    if row.state != DeliveryState::Pending {
        return Ok(DeliveryOutcome::Duplicate);
    }
    let native_deadline = native_send_deadline(&row)?;
    let multipart = matches!(
        row.reference,
        DeliveryReference::Reply { .. }
            | DeliveryReference::PublicReply { .. }
            | DeliveryReference::Faq { .. }
    );
    if row.expires_at <= Utc::now()
        || (!multipart
            && body.encode_utf16().count() > ChatPresentation::for_entry(&row.entry)?.text_limit())
    {
        cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
        return Ok(DeliveryOutcome::Refused(
            UnavailableReason::TransportUnavailable,
        ));
    }
    if row.retry_at > Utc::now() {
        return Ok(DeliveryOutcome::Deferred {
            retry_at: row.retry_at,
        });
    }
    let personal = matches!(row.reference, DeliveryReference::Reply { .. });
    let control = matches!(row.reference, DeliveryReference::Command { .. });
    let public_lane = matches!(
        row.reference,
        DeliveryReference::PublicReply { .. } | DeliveryReference::Faq { .. }
    );
    #[cfg(test)]
    let test_control_capacity = TEST_CONTROL_CAPACITY.try_with(Arc::clone).ok();
    let permits = if control {
        #[cfg(test)]
        if let Some(capacity) = &test_control_capacity {
            capacity.as_ref()
        } else {
            &CONTROL_SENDS
        }
        #[cfg(not(test))]
        &CONTROL_SENDS
    } else if public_lane {
        &PUBLIC_SENDS
    } else if personal {
        &PERSONAL_SENDS
    } else {
        &STATUS_SENDS
    };
    let pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        if control {
            "manager_control_send"
        } else if public_lane {
            "manager_public_send"
        } else if personal {
            "manager_personal_send"
        } else {
            "manager_status_send"
        },
        &format!(
            "{}:{}",
            manager_lane(&row),
            scope::actor_string(&row.sender)
        ),
        1,
        std::time::Duration::from_secs(1),
    )
    .await?;
    // Application lane budgets are shared across replicas. Status/public load
    // cannot consume the personal reserve; provider 429 remains authoritative.
    let manager_pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        if control {
            "manager_control_lane"
        } else if public_lane {
            "manager_public_lane"
        } else if personal {
            "manager_personal_lane"
        } else {
            "manager_status_lane"
        },
        &manager_lane(&row),
        if personal { 24 } else { 4 },
        std::time::Duration::from_secs(1),
    )
    .await?;
    let permit = permits.try_acquire();
    if !pacing.allowed || !manager_pacing.allowed || permit.is_err() {
        if matches!(row.reference, DeliveryReference::Command { .. }) {
            cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
            return Ok(DeliveryOutcome::Refused(UnavailableReason::CapacityLimited));
        }
        let retry_at = Utc::now() + Duration::seconds(1);
        let persisted = cleanup_before_deadline(native_deadline, async {
            state
                .db
                .collection::<Delivery>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.id,"state":"pending"},
                    doc! {"$set":{"retry_at":bson::DateTime::from_chrono(retry_at)}},
                )
                .await?;
            Ok(())
        })
        .await?;
        return Ok(if persisted {
            DeliveryOutcome::Deferred { retry_at }
        } else {
            DeliveryOutcome::Refused(UnavailableReason::TransportUnavailable)
        });
    }
    let _permit = permit.map_err(|_| refused())?;
    #[cfg(test)]
    if control && let Ok(barrier) = TEST_BEFORE_COMMAND_CLAIM.try_with(Arc::clone) {
        tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
            .await
            .expect("management send barrier");
        tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
            .await
            .expect("management send barrier");
    }
    let native = match prepare_native_send(state, &row, native_deadline).await {
        Ok(native) => native,
        Err(
            AppError::Conflict(_)
            | AppError::NotFound(_)
            | AppError::ValidationError(_)
            | AppError::Forbidden(_),
        ) => {
            cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
            return Ok(DeliveryOutcome::Refused(
                UnavailableReason::TransportUnavailable,
            ));
        }
        Err(error) => return Err(error),
    };
    let native_deadline = native
        .as_ref()
        .map(|prepared| prepared.deadline)
        .or(native_deadline);
    let claim_id = uuid::Uuid::new_v4().to_string();
    let captured = state.clone();
    let delivery = row.clone();
    let worker_claim = claim_id.clone();
    let result = before_native_deadline(native_deadline, async {
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().and_run((captured,identity.clone(),public.clone(),delivery,worker_claim,body.clone(),native.as_ref()),|session,(state,identity,public,row,claim,body,native)|Box::pin(async move {
        #[cfg(test)]
        stall_native_claim_for_test(native.is_some()).await;
        let result:AppResult<_>=async {
            let Some(observation)=validate_in_session(state,identity.as_ref(),public.as_deref(),session,row,body.as_str()).await? else {return Ok(CapturedSend::Deferred);};
            if state.db.collection::<Delivery>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"state":"pending","expires_at":{"$gt":bson::DateTime::now()}},doc! {"$set":{"state":"dispatching","claim_id":&*claim,"multipart_version":1,"updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1{return Ok(CapturedSend::Duplicate);}
            capture_material_in_session(state,session,row,claim,&observation,*native).await
        }.await;
        tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error)
    }).await;
    let Some(result) = result else {
        // No provider call occurs before the claimed transaction returns. A
        // dropped/unknown commit is left for durable dispatching recovery.
        return Ok(DeliveryOutcome::Refused(
            UnavailableReason::TransportUnavailable,
        ));
    };

    let credential = match result {
        Ok(CapturedSend::Ready(value)) => value,
        Ok(CapturedSend::Duplicate) => return Ok(DeliveryOutcome::Duplicate),
        Ok(CapturedSend::Deferred) => {
            if matches!(row.reference, DeliveryReference::Command { .. }) {
                cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
                return Ok(DeliveryOutcome::Refused(
                    UnavailableReason::TransportUnavailable,
                ));
            }
            let retry_at = Utc::now() + Duration::seconds(5);
            let persisted = cleanup_before_deadline(native_deadline, async {
                state.db.collection::<Delivery>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"state":"pending"},doc! {"$set":{"retry_at":bson::DateTime::from_chrono(retry_at),"updated_at":bson::DateTime::now()}}).await?;
                Ok(())
            }).await?;
            return Ok(if persisted {
                DeliveryOutcome::Deferred { retry_at }
            } else {
                DeliveryOutcome::Refused(UnavailableReason::TransportUnavailable)
            });
        }
        Err(error) => {
            if matches!(error, AppError::Conflict(_) | AppError::NotFound(_))
                || (native_deadline.is_some()
                    && matches!(error, AppError::Forbidden(_) | AppError::ValidationError(_)))
            {
                cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
                return Ok(DeliveryOutcome::Refused(UnavailableReason::OriginRevoked));
            }
            return Err(error);
        }
    };
    send_claimed(
        state,
        &row,
        &claim_id,
        body,
        credential,
        native.as_ref(),
        api,
        identity,
        public,
    )
    .await
}

fn multipart_key(row: &Delivery) -> String {
    format!("manager:{}", row.id)
}

#[allow(clippy::too_many_arguments)]
async fn send_claimed(
    state: &AppState,
    row: &Delivery,
    claim_id: &str,
    body: Zeroizing<String>,
    material: CapturedMaterial,
    native: Option<&NativeSendPreparation>,
    api: &super::telegram_new_api::TelegramApi<'_>,
    identity: Arc<dyn ManagerIdentityProducer>,
    public: Option<Arc<dyn PublicTurnProducer>>,
) -> AppResult<DeliveryOutcome> {
    use super::channel_text_delivery as journal;
    use crate::models::channel_text_delivery::TextPartState;
    if !matches!(
        row.reference,
        DeliveryReference::Reply { .. }
            | DeliveryReference::PublicReply { .. }
            | DeliveryReference::Faq { .. }
    ) {
        return send_single_claimed(state, row, claim_id, body, material, native, api).await;
    }
    let chat = ChatPresentation::for_entry(&row.entry)?;
    let (source, owner, url) = match &row.reference {
        DeliveryReference::Reply {
            reply_id,
            conversation_id,
            ..
        } => (
            reply_id.as_str(),
            row.origin
                .as_ref()
                .ok_or_else(refused)?
                .thread
                .user_id
                .as_str(),
            conversation_url(&state.config.frontend_url, conversation_id),
        ),
        DeliveryReference::PublicReply {
            reply_id,
            conversation_id,
            ..
        } => (
            reply_id.as_str(),
            row.profile_id.as_str(),
            conversation_url(&state.config.frontend_url, conversation_id),
        ),
        _ => (row.id.as_str(), row.profile_id.as_str(), String::new()),
    };
    let credential = if let CapturedMaterial::Telegram(credential) = &material {
        let values = super::platform_credential_service::decrypt_platform_snapshot(
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
            .map(|id| id.to_string())
            .as_deref()
            != Some(row.manager_bot_id.as_str())
        {
            finish(state, row, claim_id, "refused", vec![], None).await?;
            return Ok(DeliveryOutcome::Refused(
                UnavailableReason::TransportUnavailable,
            ));
        }
        Some(Zeroizing::new(token.to_owned()))
    } else {
        None
    };
    let mut rendered = body.to_string();
    if let Some(token) = &credential {
        let media = media::send_saved(
            state,
            row,
            claim_id,
            body.as_str(),
            identity.as_ref(),
            public.as_deref(),
            token,
            api,
        )
        .await;
        match media {
            Ok(note) => rendered.push_str(&note),
            Err(error) if error.retry_after_seconds().is_some() => {
                let retry_at =
                    Utc::now() + Duration::seconds(error.retry_after_seconds().unwrap() as i64);
                finish(state, row, claim_id, "pending", vec![], Some(retry_at)).await?;
                return Ok(DeliveryOutcome::Deferred { retry_at });
            }
            Err(error) => return Err(error),
        }
    }
    let parts = super::channel_reply_plan::assistant_parts(&rendered, chat.platform(), &url);
    let plan = journal::begin(
        &state.db,
        &multipart_key(row),
        owner,
        source,
        &super::channel_reply_plan::fingerprint(&parts),
        parts.len(),
    )
    .await?;
    let mut telegram_ids = Vec::new();
    let mut platform_ids = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        if plan.parts[index].state != TextPartState::Sent {
            pace_part(state, row, claim_id).await?;
        }
        // Recheck the complete captured route and current owner generation before
        // every effect, including after a prefix was accepted or a retry waited.
        let authorized = revalidate_part(
            state,
            row,
            claim_id,
            body.as_str(),
            identity.as_ref(),
            public.as_deref(),
        )
        .await?;
        if !authorized {
            finish(state, row, claim_id, "refused", telegram_ids, None).await?;
            return Ok(DeliveryOutcome::Refused(UnavailableReason::OriginRevoked));
        }
        let attempt = match journal::claim(&state.db, &plan, index).await {
            Ok(journal::PartClaim::Sent(previous)) => {
                if let Some(id) = previous.platform_message_id {
                    if credential.is_some() {
                        telegram_ids.push(id.parse::<i64>().map_err(|_| refused())?);
                    } else {
                        platform_ids.push(id);
                    }
                }
                continue;
            }
            Ok(journal::PartClaim::Send(attempt)) => attempt,
            Err(_) => {
                finish(state, row, claim_id, "uncertain", telegram_ids, None).await?;
                return Ok(DeliveryOutcome::Uncertain);
            }
        };
        let result = if let Some(token) = &credential {
            telegram_part(api, token, scope::actor_string(&row.sender).as_str(), &part).await
        } else if let CapturedMaterial::Native(authority) = &material {
            use super::platform_native_reply_service::NativeSendOutcome;
            match super::platform_native_reply_service::send_part(
                state,
                authority,
                Zeroizing::new(part.text.clone().unwrap_or_default()),
                native.ok_or_else(refused)?.adapter.as_ref(),
                part.metadata.as_ref(),
            )
            .await?
            {
                NativeSendOutcome::Sent {
                    platform_message_id,
                } => PartResult::Sent(platform_message_id),
                NativeSendOutcome::Refused => PartResult::Refused,
                NativeSendOutcome::Uncertain => PartResult::Uncertain,
            }
        } else {
            return Err(refused());
        };
        match result {
            PartResult::Sent(id) => {
                journal::finish(
                    &state.db,
                    &plan,
                    index,
                    &attempt,
                    TextPartState::Sent,
                    Some(&id),
                    None,
                )
                .await?;
                if credential.is_some() {
                    telegram_ids.push(id.parse::<i64>().map_err(|_| refused())?);
                } else {
                    platform_ids.push(id);
                }
                state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
                    doc! {"_id":&row.id,"state":"dispatching","claim_id":claim_id},
                    doc! {"$set":{"telegram_message_ids":&telegram_ids,"platform_message_ids":&platform_ids,"updated_at":bson::DateTime::now()}}
                ).await?;
            }
            PartResult::Retry(delay) => {
                journal::finish(
                    &state.db,
                    &plan,
                    index,
                    &attempt,
                    TextPartState::Pending,
                    None,
                    None,
                )
                .await?;
                let retry_at = Utc::now() + Duration::seconds(delay);
                finish(
                    state,
                    row,
                    claim_id,
                    "pending",
                    telegram_ids,
                    Some(retry_at),
                )
                .await?;
                return Ok(DeliveryOutcome::Deferred { retry_at });
            }
            PartResult::Refused => {
                journal::finish(
                    &state.db,
                    &plan,
                    index,
                    &attempt,
                    TextPartState::Refused,
                    None,
                    None,
                )
                .await?;
                finish(state, row, claim_id, "refused", telegram_ids, None).await?;
                return Ok(DeliveryOutcome::Refused(
                    UnavailableReason::TransportUnavailable,
                ));
            }
            PartResult::Uncertain => {
                finish(state, row, claim_id, "uncertain", telegram_ids, None).await?;
                return Ok(DeliveryOutcome::Uncertain);
            }
        }
    }
    finish(state, row, claim_id, "sent", telegram_ids.clone(), None).await?;
    Ok(if credential.is_some() {
        DeliveryOutcome::Sent {
            telegram_message_ids: telegram_ids,
        }
    } else {
        DeliveryOutcome::NativeSent {
            platform_message_ids: platform_ids,
        }
    })
}

async fn pace_part(state: &AppState, row: &Delivery, claim: &str) -> AppResult<()> {
    loop {
        if state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
            doc! {"_id":&row.id,"state":"dispatching","claim_id":claim,"expires_at":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"updated_at":bson::DateTime::now()}},
        ).await?.matched_count != 1 {
            return Err(refused());
        }
        let chat = super::coordination_service::RateWindowStore::admit(
            &state.db,
            "manager_part_chat",
            &format!("{}:{}", manager_lane(row), scope::actor_string(&row.sender)),
            1,
            std::time::Duration::from_secs(1),
        )
        .await?;
        if chat.allowed {
            let personal = matches!(row.reference, DeliveryReference::Reply { .. });
            let lane = super::coordination_service::RateWindowStore::admit(
                &state.db,
                if personal {
                    "manager_part_personal"
                } else {
                    "manager_part_public"
                },
                &manager_lane(row),
                if personal { 24 } else { 4 },
                std::time::Duration::from_secs(1),
            )
            .await?;
            if lane.allowed {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
}

async fn revalidate_part(
    state: &AppState,
    row: &Delivery,
    claim: &str,
    body: &str,
    identity: &dyn ManagerIdentityProducer,
    public: Option<&dyn PublicTurnProducer>,
) -> AppResult<bool> {
    let mut session = state.db.client().start_session().await?;
    let result = session
        .start_transaction()
        .and_run(
            (state, row, claim, body, identity, public),
            |session, (state, row, claim, body, identity, public)| {
                Box::pin(async move {
                    let result: AppResult<bool> = async {
                        if row.expires_at <= Utc::now() {
                            return Ok(false);
                        }
                        if validate_in_session(state, *identity, *public, session, row, body)
                            .await?
                            .is_none()
                        {
                            return Ok(false);
                        }
                        Ok(state
                            .db
                            .collection::<Delivery>(COLLECTION_NAME)
                            .update_one(
                                doc! {"_id":&row.id,"state":"dispatching","claim_id":*claim},
                                doc! {"$set":{"updated_at":bson::DateTime::now()}},
                            )
                            .session(session)
                            .await?
                            .matched_count
                            == 1)
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error);
    match result {
        Err(AppError::Conflict(_) | AppError::NotFound(_) | AppError::Forbidden(_)) => Ok(false),
        other => other,
    }
}

enum PartResult {
    Sent(String),
    Retry(i64),
    Refused,
    Uncertain,
}

async fn telegram_part(
    api: &super::telegram_new_api::TelegramApi<'_>,
    token: &str,
    chat: &str,
    part: &super::channel_platform::OutboundReply,
) -> PartResult {
    let Ok(chat) = chat.parse::<i64>() else {
        return PartResult::Refused;
    };
    let mut payload = serde_json::json!({"chat_id":chat,"text":part.text,"protect_content":true,"link_preview_options":{"is_disabled":true}});
    if let Some(entities) = part.metadata.as_ref().and_then(|m| m.get("entities")) {
        payload["entities"] = entities.clone();
    }
    let response = api
        .http
        .post(format!("{}/bot{token}/sendMessage", api.base_url))
        .timeout(std::time::Duration::from_secs(20))
        .json(&payload)
        .send()
        .await;
    let envelope = match response {
        Ok(mut response) => {
            if !response.status().is_success() && !response.status().is_client_error() {
                return PartResult::Uncertain;
            }
            let mut bytes = Zeroizing::new(Vec::new());
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) if bytes.len() + chunk.len() <= 65536 => {
                        bytes.extend_from_slice(&chunk)
                    }
                    Ok(None) => break,
                    _ => return PartResult::Uncertain,
                }
            }
            serde_json::from_slice::<serde_json::Value>(&bytes).ok()
        }
        Err(_) => None,
    };
    if let Some(value) = envelope {
        if value["ok"] == true
            && let Some(id) = value["result"]["message_id"].as_i64().filter(|id| *id > 0)
        {
            return PartResult::Sent(id.to_string());
        }
        if value["ok"] == false {
            if value["error_code"] == 429
                && let Some(delay) = value["parameters"]["retry_after"]
                    .as_i64()
                    .filter(|n| *n > 0 && *n <= 86400)
            {
                return PartResult::Retry(delay);
            }
            if value["error_code"]
                .as_i64()
                .is_some_and(|code| (400..500).contains(&code))
            {
                return PartResult::Refused;
            }
        }
    }
    PartResult::Uncertain
}

async fn send_single_claimed(
    state: &AppState,
    row: &Delivery,
    claim_id: &str,
    body: Zeroizing<String>,
    material: CapturedMaterial,
    native: Option<&NativeSendPreparation>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    match material {
        CapturedMaterial::Telegram(credential) => {
            send_telegram_claimed(state, row, claim_id, body, credential, api).await
        }
        CapturedMaterial::Native(authority) => {
            let native = native.ok_or_else(refused)?;
            let deadline = native.deadline.min(authority.deadline());
            let result = super::platform_native_reply_service::send_claimed(
                state,
                *authority,
                body,
                native.adapter.as_ref(),
            )
            .await?;
            use super::platform_native_reply_service::NativeSendOutcome;
            let (status, ids, outcome) = match result {
                NativeSendOutcome::Sent {
                    platform_message_id,
                } => {
                    let ids = vec![platform_message_id];
                    (
                        "sent",
                        ids.clone(),
                        DeliveryOutcome::NativeSent {
                            platform_message_ids: ids,
                        },
                    )
                }
                NativeSendOutcome::Refused => (
                    "refused",
                    vec![],
                    DeliveryOutcome::Refused(UnavailableReason::TransportUnavailable),
                ),
                NativeSendOutcome::Uncertain => ("uncertain", vec![], DeliveryOutcome::Uncertain),
            };
            let remaining = (deadline - Utc::now()).to_std().unwrap_or_default();
            let persisted = tokio::time::timeout(remaining, async {
                #[cfg(test)]
                if let Ok(stall) = TEST_NATIVE_FINISH_STALL.try_with(Arc::clone) {
                    stall.entered.store(true, std::sync::atomic::Ordering::SeqCst);
                    tokio::time::sleep(stall.duration).await;
                }
                state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
                doc! {"_id":&row.id,"state":"dispatching","claim_id":claim_id},
                doc! {"$set":{"state":status,"platform_message_ids":ids,"updated_at":bson::DateTime::now()}},
                ).await
            }).await;
            if !matches!(persisted, Ok(Ok(result)) if result.matched_count == 1) {
                return Ok(if matches!(outcome, DeliveryOutcome::Refused(_)) {
                    outcome
                } else {
                    DeliveryOutcome::Uncertain
                });
            }
            Ok(outcome)
        }
    }
}

async fn send_telegram_claimed(
    state: &AppState,
    row: &Delivery,
    claim_id: &str,
    body: Zeroizing<String>,
    credential: Box<PlatformCredential>,
    api: &super::telegram_new_api::TelegramApi<'_>,
) -> AppResult<DeliveryOutcome> {
    let EntryIdentity::TelegramV1 { telegram_bot_id } = &row.entry else {
        return Err(refused());
    };
    let SenderIdentity::TelegramV1 {
        telegram_chat_id: chat_id,
        ..
    } = &row.sender
    else {
        return Err(refused());
    };
    let values = super::platform_credential_service::decrypt_platform_snapshot(
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
        != Some(*telegram_bot_id)
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
        .json(&serde_json::json!({"chat_id":chat_id,"text":body.as_str(),"protect_content":true,"link_preview_options":{"is_disabled":true}}))
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
                DeliveryReference::Command { .. } | DeliveryReference::PublicSignup { .. }
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
    finish_parent(&state.db, &row.id, claim, status, ids, retry).await
}

async fn finish_parent(
    db: &Database,
    id: &str,
    claim: &str,
    status: &str,
    ids: Vec<i64>,
    retry: Option<chrono::DateTime<Utc>>,
) -> AppResult<()> {
    // A retry can lose authority while traversing an already accepted prefix.
    // Keep its earlier receipts even if this attempt observed fewer of them.
    let previous = doc! {"$ifNull":["$telegram_message_ids",[]]};
    let mut set = doc! {"state":status,"telegram_message_ids":{
        "$cond":[{"$gte":[{"$size":previous.clone()},ids.len() as i64]},previous,{"$literal":ids}]
    },"updated_at":bson::DateTime::now()};
    if let Some(at) = retry {
        set.insert("retry_at", bson::DateTime::from_chrono(at));
    }
    let changed = db
        .collection::<Delivery>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":id,"state":"dispatching","claim_id":claim},
            vec![doc! {"$set":set}],
        )
        .await?;
    if changed.matched_count != 1 {
        return Err(super::channel_text_delivery::unconfirmed());
    }
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
            Zeroizing::new(fixed_text_for(reason, &destination.manager.entry)?),
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
    let outcome = send(state, identity, None, row, body, api).await?;
    report_delivery(&outcome, "reply");
    Ok(outcome)
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
        entry: row.entry.clone(),
        manager_authority_epoch: row.manager_authority_epoch,
    };
    let ingress = PublicIngress {
        manager: manager.clone(),
        sender: row.sender.clone(),
        ingress: IngressClaim {
            receipt_id: row.ingress_id.clone().ok_or_else(refused)?,
            claim_id: row.ingress_claim_id.clone().ok_or_else(refused)?,
        },
        public_policy_revision: *public_policy_revision,
    };
    Ok(ReplyDestination {
        manager,
        sender: row.sender.clone(),
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
        sender: command.sender.clone(),
        authority: ReplyAuthority::Fixed {
            ingress: command.ingress.clone(),
        },
        expires_at: Utc::now() + Duration::minutes(1),
    };
    let body = if let super::platform_manager_command_service::PersonalCommandReply::Management {
        connect_token,
    } = &command.reply
    {
        let mut session = state.db.client().start_session().await?;
        let receipt = state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .find_one(doc! {"_id":&command.ingress.receipt_id,"claim_id":&command.ingress.claim_id})
            .session(&mut session)
            .await?
            .ok_or_else(refused)?;
        let update = VerifiedManagerUpdate {
            profile_id: receipt.profile_id,
            manager_bot_id: receipt.manager_bot_id,
            entry: receipt.entry.clone(),
            observation_id: receipt.observation_id,
            sender: receipt.sender.clone(),
            chat_kind: ManagerChatKind::Private,
            event: receipt.event.clone(),
            command_eligibility: ManagerCommandEligibility::OriginalPrivateMessage,
        };
        super::platform_manager_management::render_in_session(
            &state.db,
            &mut session,
            &update,
            &command.effect,
            &state.config.frontend_url,
            connect_token.as_deref().map(String::as_str),
        )
        .await?
    } else {
        command
            .effect
            .static_reply()
            .as_ref()
            .unwrap_or(&command.reply)
            .render(&state.config.frontend_url, &destination.manager.entry)?
    };
    let row = enqueue(
        state,
        &destination,
        DeliveryReference::Command {
            effect: Some(command.effect),
        },
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
    if scope::validate(
        &ingress.manager.entry,
        &ingress.manager.manager_bot_id,
        Some(&ingress.sender),
    )
    .is_err()
        || context.expires_at <= Utc::now()
    {
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    // This read obtains metadata for dedupe; the final transaction revalidates
    // the exact token, intent and execution authority with a write fence.
    let intent =
        match intents::preview(&state.db, state.auth_device_hmac_key.as_slice(), &token).await {
            Ok(intent) => intent,
            Err(
                AppError::Conflict(_)
                | AppError::NotFound(_)
                | AppError::ValidationError(_)
                | AppError::Forbidden(_),
            ) => {
                return Ok(PublicSignupDeliveryOutcome::Unavailable);
            }
            Err(error) => return Err(error),
        };
    let destination = ReplyDestination {
        manager: ingress.manager.clone(),
        sender: ingress.sender.clone(),
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
    let native_deadline = native_send_deadline(&row)?;
    let pacing = super::coordination_service::RateWindowStore::admit(
        &state.db,
        "manager_public_send",
        &format!(
            "{}:{}",
            manager_lane(&row),
            scope::actor_string(&row.sender)
        ),
        1,
        std::time::Duration::from_secs(1),
    )
    .await?;
    let lane = super::coordination_service::RateWindowStore::admit(
        &state.db,
        "manager_public_lane",
        &manager_lane(&row),
        4,
        std::time::Duration::from_secs(1),
    )
    .await?;
    let permit = PUBLIC_SENDS.try_acquire();
    if !pacing.allowed || !lane.allowed || permit.is_err() {
        cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    let _permit = permit.map_err(|_| refused())?;
    let native = match prepare_native_send(state, &row, native_deadline).await {
        Ok(native) => native,
        Err(
            AppError::Conflict(_)
            | AppError::NotFound(_)
            | AppError::ValidationError(_)
            | AppError::Forbidden(_),
        ) => {
            cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
            return Ok(PublicSignupDeliveryOutcome::Unavailable);
        }
        Err(error) => return Err(error),
    };
    let native_deadline = native
        .as_ref()
        .map(|prepared| prepared.deadline)
        .or(native_deadline);
    let claim = uuid::Uuid::new_v4().to_string();
    let result = before_native_deadline(native_deadline, async {
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().and_run(
        (state, context, &row, token.as_str(), &claim, native.as_ref()),
        |session, (state, context, row, token, claim, native)| Box::pin(async move {
            #[cfg(test)]
            stall_native_claim_for_test(native.is_some()).await;
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
                    || row.entry != context.ingress.manager.entry
                    || row.manager_authority_epoch != context.ingress.manager.manager_authority_epoch
                    || row.sender != context.ingress.sender
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
                capture_material_in_session(state,session,row,claim,&observation,*native).await
            }.await;
            tx::transaction_result(result)
        }),
    ).await.map_err(tx::map_transaction_error)
    }).await;
    let Some(result) = result else {
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    };

    let credential = match result {
        Ok(CapturedSend::Ready(credential)) => credential,
        Ok(CapturedSend::Duplicate) => return Ok(PublicSignupDeliveryOutcome::Unavailable),
        Ok(CapturedSend::Deferred)
        | Err(
            AppError::Conflict(_)
            | AppError::NotFound(_)
            | AppError::ValidationError(_)
            | AppError::Forbidden(_),
        ) => {
            cleanup_before_deadline(native_deadline, refuse_pending(state, &row)).await?;
            return Ok(PublicSignupDeliveryOutcome::Unavailable);
        }
        Err(error) => return Err(error),
    };
    let body = super::platform_manager_command_service::PersonalCommandReply::Link { token }
        .render(&state.config.frontend_url, &row.entry)?;
    if body.encode_utf16().count() > ChatPresentation::for_entry(&row.entry)?.text_limit() {
        cleanup_before_deadline(
            native_deadline,
            finish(state, &row, &claim, "refused", vec![], None),
        )
        .await?;
        return Ok(PublicSignupDeliveryOutcome::Unavailable);
    }
    let outcome =
        send_single_claimed(state, &row, &claim, body, credential, native.as_ref(), api).await?;
    report_delivery(&outcome, "public_signup");
    Ok(match outcome {
        DeliveryOutcome::Sent { .. } | DeliveryOutcome::NativeSent { .. } => {
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
        DeliveryOutcome::NativeSent {
            platform_message_ids,
        } => tracing::debug!(
            source,
            message_count = platform_message_ids.len(),
            "Manager native reply sent"
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
            let versioned = state.db.collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id,"multipart_version":1,"reference.kind":{"$in":["reply","public_reply","faq"]}}).await?.is_some();
            let safe = versioned
                && super::channel_text_delivery::load(&state.db, &multipart_key(&row))
                    .await?
                    .as_ref()
                    .is_none_or(super::channel_text_delivery::recoverable);
            state.db.collection::<Delivery>(COLLECTION_NAME).update_one(
                doc! {"_id":&row.id,"state":"dispatching","claim_id":&row.claim_id,"updated_at":{"$lte":stale_before}},
                doc! {"$set":{"state":if safe {"pending"} else {"uncertain"},"updated_at":due,"retry_at":due}},
            ).await?;
        } else if row.expires_at <= now
            || matches!(
                row.reference,
                DeliveryReference::Command { .. } | DeliveryReference::PublicSignup { .. }
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
                DeliveryReference::Fixed { reason } => fixed_reason(reason)
                    .map(|reason| fixed_text_for(reason, &row.entry))
                    .transpose()?,
                DeliveryReference::Handoff => Some(handoff_text().to_owned()),
                DeliveryReference::Faq { .. } => {
                    super::platform_faq_service::render_saved(&state.db, &row.reference, &row.entry)
                        .await
                        .ok()
                }
                DeliveryReference::Command { .. } | DeliveryReference::PublicSignup { .. } => None,
            };
            if let Some(body) = body {
                match send(
                    state,
                    identity.clone(),
                    Some(public.clone()),
                    row,
                    Zeroizing::new(body),
                    &super::telegram_new_api::TelegramApi::new(&state.http_client),
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
        PublicContractUnsupported,
        PublicAuthorityRejected,
        ProfileUnavailable,
        ProofExpired,
        PublicQuotaLimited,
        PublicTurnFailed,
        SponsorUnavailable,
        RuntimeUninitialized,
        EngineUnavailable,
        UnsupportedInput,
        ConfirmationRejected,
        ControlInvalid,
        ControlRateLimited,
        ModeDisabled,
    ]
    .into_iter()
    .find(|candidate| format!("{candidate:?}") == reason)
}

#[cfg(test)]
mod native_render_tests {
    use super::*;
    use crate::models::{
        assistant_acknowledgement::AssistantAcknowledgement, platform_entry_identity::EntryPlatform,
    };

    fn saved(text: &str) -> AssistantMessage {
        bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),
            "conversation_id":"nyxa-0123456789abcdef0123456789abcdef",
            "user_id":uuid::Uuid::new_v4().to_string(),
            "seq":2_i64,"turn_id":"turn","role":"assistant",
            "text":text,"status":"completed","created_at":bson::DateTime::now(),
        })
        .unwrap()
    }

    fn card(message: &AssistantMessage) -> AssistantAcknowledgement {
        bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),
            "conversation_id":&message.conversation_id,"user_id":&message.user_id,
            "api_key_id":"exact-thread-key","kind":"action",
            "requested_turn_id":&message.turn_id,"routing":{"kind":"not_involved"},
            "summary":"🔐変更".repeat(260),"status":"pending",
            "created_at":bson::DateTime::now(),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10)),
        })
        .unwrap()
    }

    #[test]
    fn native_attachment_fallback_keeps_the_exact_thread_without_linking_plain_replies() {
        for platform in [EntryPlatform::Discord, EntryPlatform::Slack] {
            for text in ["", "Here is the report."] {
                let mut message = saved(text);
                let render = |message: &AssistantMessage| {
                    append_card_notices(
                        message,
                        "https://app.nyxid.test/old?secret=discard#discard",
                        &[],
                        "exact-thread-key",
                        None,
                        ChatPresentation::Native(platform),
                    )
                    .unwrap()
                };
                assert!(!render(&message).contains("/assistant?c="));
                message.attachments.push(
                    bson::from_document(doc! {
                        "id":"report", "origin":"tool", "content_type":"application/pdf",
                        "size":100_i64, "label":"Report",
                    })
                    .unwrap(),
                );
                let output = render(&message);
                assert!(output.starts_with(text));
                assert!(output.contains("Open them in NyxID: <https://app.nyxid.test/assistant?c=nyxa-0123456789abcdef0123456789abcdef>"));
                assert!(!output.contains("discard"));
                message.attachments[0].chat_delivery = Some(false);
                assert!(!render(&message).contains("/assistant?c="));
            }
        }
    }

    #[test]
    fn control_failures_reconstruct_exact_safe_provider_guidance() {
        for platform in [EntryPlatform::Slack, EntryPlatform::Discord] {
            let entry = EntryIdentity::NativeV2 {
                platform_entry: crate::models::platform_entry_identity::NativeEntryIdentity {
                    version: 2,
                    platform,
                    channel_bot_id: uuid::Uuid::new_v4().to_string(),
                    platform_bot_id: "100".into(),
                },
            };
            for reason in [
                UnavailableReason::ConfirmationRejected,
                UnavailableReason::ControlInvalid,
                UnavailableReason::ControlRateLimited,
            ] {
                assert_eq!(fixed_reason(&format!("{reason:?}")), Some(reason));
                let body = fixed_text_for(reason, &entry).unwrap();
                assert!(!body.contains("/link") && !body.contains("P-AAAA-BBBB"));
                let guidance = match (platform, reason) {
                    (EntryPlatform::Discord, UnavailableReason::ConfirmationRejected) => {
                        "/nyx login"
                    }
                    (EntryPlatform::Slack, UnavailableReason::ConfirmationRejected) => "Send login",
                    (EntryPlatform::Discord, _) => "/nyx help",
                    _ => "help",
                };
                assert!(body.contains(guidance));
                if reason == UnavailableReason::ControlRateLimited {
                    assert!(body.contains("Wait a moment, then retry here"));
                }
            }
        }
    }

    #[test]
    fn long_unicode_native_reply_and_action_cards_keep_full_content() {
        let chat = ChatPresentation::Native(EntryPlatform::Discord);
        let message = saved(&"🧑🏽‍💻相談".repeat(600));
        let frontend = "https://app.nyxid.test/old?discard=secret#discard";
        let rendered = render_saved_text(&message);
        assert_eq!(rendered, message.text);
        assert!(!rendered.contains("discard"));

        let action = card(&message);
        let code = super::super::assistant_acknowledgement_service::confirm_code(&action.id);
        let rendered = append_card_notices(
            &message,
            frontend,
            std::slice::from_ref(&action),
            "exact-thread-key",
            None,
            chat,
        )
        .unwrap();
        assert!(rendered.starts_with(&message.text));
        assert!(rendered.contains(&format!("/nyx ask question:\"yes {code}\"")));
        assert!(rendered.contains(&format!("/nyx ask question:\"no {code}\"")));
        assert!(!rendered.contains("Reply yes"));
        assert!(!rendered.contains("/assistant?c="));

        let mut cards = vec![action];
        while cards.len() < 3 {
            let candidate = card(&message);
            let candidate_code =
                super::super::assistant_acknowledgement_service::confirm_code(&candidate.id);
            if cards.iter().all(|other| {
                super::super::assistant_acknowledgement_service::confirm_code(&other.id)
                    != candidate_code
            }) {
                cards.push(candidate);
            }
        }
        let rendered =
            append_card_notices(&message, frontend, &cards, "exact-thread-key", None, chat)
                .unwrap();
        assert!(rendered.starts_with(&message.text));
        assert!(!rendered.contains("/assistant?c="));
        assert!(rendered.contains("Action:"));
        assert!(!rendered.contains("Reply yes"));
    }
}

#[cfg(test)]
mod future_size_tests {
    use super::*;

    type SendArgs = (
        &'static AppState,
        Arc<dyn ManagerIdentityProducer>,
        Option<Arc<dyn PublicTurnProducer>>,
        Delivery,
        Zeroizing<String>,
        &'static super::super::telegram_new_api::TelegramApi<'static>,
    );

    fn future_size<Args, F: std::future::Future>(_: impl FnOnce(Args) -> F) -> usize {
        std::mem::size_of::<F>()
    }

    #[test]
    fn native_delivery_keeps_shared_settlement_and_send_futures_small() {
        let saved = future_size(
            |(state, row, reply): (
                &'static AppState,
                &'static AssistantConversation,
                &'static ReplyRef,
            )| { saved_personal_reply(state, row, reply) },
        );
        let prepare = future_size(
            |(state, row, deadline): (
                &'static AppState,
                &'static Delivery,
                Option<chrono::DateTime<Utc>>,
            )| { prepare_native_send(state, row, deadline) },
        );
        let send = future_size(|(state, identity, public, row, body, api): SendArgs| {
            send(state, identity, public, row, body, api)
        });
        let native = future_size(
            |(state, authority, body, adapter): (
                &'static AppState,
                super::super::platform_native_reply_service::NativeSendAuthority,
                Zeroizing<String>,
                &'static dyn super::super::channel_platform::PlatformAdapter,
            )| {
                super::super::platform_native_reply_service::send_claimed(
                    state, authority, body, adapter,
                )
            },
        );
        assert!(
            saved < 16 * 1024,
            "saved reply caller future grew to {saved} bytes"
        );
        assert!(
            prepare < 16 * 1024,
            "native selection future grew to {prepare} bytes"
        );
        assert!(
            send < 1024,
            "shared send caller future grew to {send} bytes"
        );
        assert!(
            native < 1024,
            "native send caller future grew to {native} bytes"
        );
    }

    #[tokio::test]
    async fn manager_outbound_parent_keeps_prior_receipts_when_a_retry_loses_authority() {
        let db = crate::test_utils::connect_transaction_test_database("manager_prefix").await;
        let rows = db.collection::<bson::Document>(COLLECTION_NAME);
        let id = uuid::Uuid::new_v4().to_string();
        rows.insert_one(doc! {"_id":&id,"state":"dispatching","claim_id":"attempt","telegram_message_ids":[101_i64,102_i64,103_i64]}).await.unwrap();
        for observed in [vec![], vec![101]] {
            rows.update_one(doc! {"_id":&id}, doc! {"$set":{"state":"dispatching"}})
                .await
                .unwrap();
            finish_parent(&db, &id, "attempt", "refused", observed, None)
                .await
                .unwrap();
            let row = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
            assert_eq!(
                row.get_array("telegram_message_ids").unwrap(),
                &vec![
                    bson::Bson::Int64(101),
                    bson::Bson::Int64(102),
                    bson::Bson::Int64(103)
                ]
            );
            assert_eq!(row.get_str("state").unwrap(), "refused");
        }
        rows.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"state":"dispatching","claim_id":"successor"}},
        )
        .await
        .unwrap();
        assert!(
            finish_parent(&db, &id, "attempt", "sent", vec![999], None)
                .await
                .is_err()
        );
        finish_parent(
            &db,
            &id,
            "successor",
            "sent",
            vec![101, 102, 103, 104],
            None,
        )
        .await
        .unwrap();
        let row = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        assert_eq!(row.get_array("telegram_message_ids").unwrap().len(), 4);
        assert_eq!(row.get_str("state").unwrap(), "sent");
        db.drop().await.unwrap();
    }
}
