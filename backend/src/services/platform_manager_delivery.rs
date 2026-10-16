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
        UnavailableReason::PublicQuotaLimited => {
            "Public chat has reached a daily limit. Try again after midnight UTC, or send /link to use your own NyxBot."
        }
        UnavailableReason::PublicTurnFailed => {
            "The public guide could not finish this turn. Please try again to start fresh, or send /link to use your own NyxBot."
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
    // One Telegram operation per durable claim. Oversized output stays in the app.
    if text.encode_utf16().count() > 3900 {
        text =
            "Your NyxBot reply is ready in NyxID; it is too long for one Telegram message.".into();
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
            notices.push_str(&format!(
                "\n\nAction: {summary}\nReply yes {code} to allow or no {code} to deny."
            ));
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
    let row = Delivery {
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
        || delivery.sender_id <= 0
        || delivery.sender_id != delivery.chat_id
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
            "manager_bot_id": &delivery.manager_bot_id, "telegram_bot_id": delivery.telegram_bot_id,
            "manager_authority_epoch": delivery.manager_authority_epoch,
            "sender_id": delivery.sender_id, "chat_id": delivery.chat_id,
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
        DeliveryReference::Fixed { reason } if reason == "PublicTurnFailed" => {
            if body != fixed_text(UnavailableReason::PublicTurnFailed) {
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

enum CapturedSend {
    Duplicate,
    Deferred,
    Ready(Box<PlatformCredential>),
}

async fn send(
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
    let public_lane = matches!(row.reference, DeliveryReference::PublicReply { .. });
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
    let claim_id = uuid::Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    let captured = state.clone();
    let delivery = row.clone();
    let worker_claim = claim_id.clone();
    let result=session.start_transaction().and_run((captured,identity,public,delivery,worker_claim,body.clone()),|session,(state,identity,public,row,claim,body)|Box::pin(async move {
        let result:AppResult<_>=async {
            let Some(observation)=validate_in_session(state,identity.as_ref(),public.as_deref(),session,row,body.as_str()).await? else {return Ok(CapturedSend::Deferred);};
            if state.db.collection::<Delivery>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"state":"pending","expires_at":{"$gt":bson::DateTime::now()}},doc! {"$set":{"state":"dispatching","claim_id":claim,"updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1{return Ok(CapturedSend::Duplicate);}
            let credential=state.db.collection::<PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME).find_one(doc! {"provider":"telegram-new","fields.manager_bot_id":&row.manager_bot_id,"fields.observation_id":observation,"fields.webhook_ready":"true"}).session(&mut *session).await?.ok_or_else(refused)?;
            Ok(CapturedSend::Ready(Box::new(credential)))
        }.await;
        tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error);
    let credential = match result {
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
            Zeroizing::new(fixed_text(reason).into()),
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
    let body = command.reply.render(&state.config.frontend_url)?;
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
        Ok(CapturedSend::Duplicate) => return Ok(PublicSignupDeliveryOutcome::Unavailable),
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
        DeliveryOutcome::Sent { .. } => PublicSignupDeliveryOutcome::Sent,
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
                DeliveryReference::Fixed { reason } => {
                    fixed_reason(reason).map(|reason| fixed_text(reason).to_owned())
                }
                DeliveryReference::Handoff => Some(handoff_text().to_owned()),
                DeliveryReference::Command | DeliveryReference::PublicSignup { .. } => None,
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
        ModeDisabled,
    ]
    .into_iter()
    .find(|candidate| format!("{candidate:?}") == reason)
}
