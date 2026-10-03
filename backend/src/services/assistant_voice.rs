//! Provider-independent voice admission. Provider adapters remain disabled in Phase 2.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as CONVERSATIONS, TurnOrigin,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        assistant_voice::{MAX_QUEUED, REQUESTS, RequestState, VoicePreferences, VoiceRequest},
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use uuid::Uuid;

pub async fn require_enabled(db: &Database, user: &str) -> AppResult<()> {
    super::assistant_nyxagent::require_enabled(db, user).await?;
    if !super::feature_flag_service::personal_flag_enabled(
        db,
        user,
        super::feature_flag_service::ASSISTANT_VOICE_FLAG_KEY,
    )
    .await?
    {
        return Err(AppError::NotFound("Voice route not found".into()));
    }
    Ok(())
}

pub fn validate_preferences(p: &VoicePreferences) -> AppResult<()> {
    let valid_id = |id: &str| Uuid::parse_str(id).is_ok();
    if !valid_id(&p.service_id)
        || p.connection_id.as_deref().is_some_and(|s| !valid_id(s))
        || (p.key_source == crate::models::assistant_voice::VoiceKeySource::Own)
            != p.connection_id.is_some()
        || !matches!(p.model.as_str(), "gpt-live-1" | "grok-voice-think-fast-2.0")
        || p.voice
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 64)
        || p.language
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 35)
    {
        return Err(AppError::ValidationError(
            "Invalid voice preferences".into(),
        ));
    }
    Ok(())
}

pub async fn thread(db: &Database, user: &str, id: &str) -> AppResult<AssistantConversation> {
    let row = super::assistant_nyxagent::get(db, user, id).await?;
    private_thread(&row)?;
    Ok(row)
}

fn private_thread(row: &AssistantConversation) -> AppResult<()> {
    if row.channel.is_some() || row.group_id.is_some() || row.automation_thread || row.guest_turn {
        return Err(AppError::Forbidden(
            "Voice requires a private owner thread".into(),
        ));
    }
    Ok(())
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<VoiceRequest>(crate::models::assistant_voice::COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"user_id":1,"session_id":1,"source_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"conversation_id":1,"state":1,"message_seq":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"state":1,"created_at":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"root_request_id":1})
                .build(),
        ])
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_voice::WINDOWS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"billing_request_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"settled":1,"deadline":1})
                .build(),
        ])
        .await?;
    Ok(())
}

/// Only trusted adapters call this after correlating a final transcript and delegation.
/// There is deliberately no browser endpoint for arbitrary provider function payloads.
#[cfg_attr(not(test), allow(dead_code))] // Called by provider adapters in Phase 3.
pub async fn enqueue(
    db: &Database,
    user: &str,
    conversation: &str,
    session_id: &str,
    source_id: &str,
    text: &str,
) -> AppResult<VoiceRequest> {
    require_enabled(db, user).await?;
    thread(db, user, conversation).await?;
    if Uuid::parse_str(session_id).is_err()
        || source_id.is_empty()
        || source_id.len() > 256
        || text.trim().is_empty()
        || text.len() > 32_768
    {
        return Err(AppError::ValidationError("Invalid voice request".into()));
    }
    let request = VoiceRequest {
        id: Uuid::new_v4().to_string(),
        user_id: user.into(),
        conversation_id: conversation.into(),
        session_id: session_id.into(),
        source_id: source_id.into(),
        message_id: Uuid::new_v4().to_string(),
        message_seq: 0,
        turn_id: Uuid::new_v4().to_string(),
        state: RequestState::Queued,
        pending_acknowledgement_ids: Vec::new(),
        acknowledgement_id: None,
        credential_api_key_id: None,
        parent_turn_id: None,
        root_request_id: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(30),
    };
    let text = text.to_owned();
    let db = db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(enqueue_in_session(&db, request.clone(), &text, session)).await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn enqueue_in_session(
    db: &Database,
    mut request: VoiceRequest,
    text: &str,
    session: &mut ClientSession,
) -> AppResult<VoiceRequest> {
    let requests = db.collection::<VoiceRequest>(REQUESTS);
    if let Some(existing) = requests
        .find_one(doc! {"user_id":&request.user_id,
        "session_id":&request.session_id,"source_id":&request.source_id})
        .session(&mut *session)
        .await?
    {
        if existing.conversation_id != request.conversation_id {
            return Err(AppError::Conflict(
                "Voice source belongs to another thread".into(),
            ));
        }
        return Ok(existing);
    }
    let conversations = db.collection::<AssistantConversation>(CONVERSATIONS);
    let mut row = conversations
        .find_one(doc! {"_id":&request.conversation_id,"user_id":&request.user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
    private_thread(&row)?;
    if requests
        .count_documents(doc! {"conversation_id":&row.id,"state":"queued",
        "expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())}})
        .session(&mut *session)
        .await?
        >= MAX_QUEUED
    {
        return Err(AppError::VoiceQueueFull);
    }
    row.message_count += 1;
    row.updated_at = Utc::now();
    request.message_seq = row.message_count;
    // The conversation write serializes concurrent admissions and queue claims.
    conversations
        .replace_one(doc! {"_id":&row.id,"user_id":&request.user_id}, &row)
        .session(&mut *session)
        .await?;
    db.collection::<AssistantMessage>(MESSAGES)
        .insert_one(AssistantMessage {
            execution_pending: true,
            id: request.message_id.clone(),
            conversation_id: row.id,
            user_id: request.user_id.clone(),
            seq: request.message_seq,
            turn_id: request.turn_id.clone(),
            role: "user".into(),
            text: text.into(),
            status: "completed".into(),
            error_code: None,
            created_at: request.created_at,
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: Some(TurnOrigin::User),
            via: Some("voice".into()),
        })
        .session(&mut *session)
        .await?;
    requests.insert_one(&request).session(&mut *session).await?;
    Ok(request)
}

pub async fn get(
    db: &Database,
    user: &str,
    conversation: &str,
    id: &str,
) -> AppResult<VoiceRequest> {
    thread(db, user, conversation).await?;
    db.collection::<VoiceRequest>(REQUESTS)
        .find_one(doc! {"_id":id,"user_id":user,"conversation_id":conversation})
        .await?
        .ok_or_else(|| AppError::NotFound("Voice request not found".into()))
}

/// Called inside ordinary begin_turn's transaction, before its message/fence write.
pub(crate) async fn claim(
    db: &Database,
    row: &AssistantConversation,
    id: &str,
    text: &str,
    session: &mut ClientSession,
) -> AppResult<VoiceRequest> {
    private_thread(row)?;
    let requests = db.collection::<VoiceRequest>(REQUESTS);
    let request = requests
        .find_one(
            doc! {"_id":id,"conversation_id":&row.id,"user_id":&row.user_id,
            "state":"queued","expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Conflict("Voice request is no longer queued".into()))?;
    if requests
        .find_one(doc! {"conversation_id":&row.id,"state":"queued",
        "message_seq":{"$lt":request.message_seq},
        "expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())}})
        .session(&mut *session)
        .await?
        .is_some()
    {
        // Creation clocks can disagree with committed sequence order. This is
        // retryable admission pressure, never a reason to cancel accepted work.
        return Err(AppError::AssistantTurnActive);
    }
    let message = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "_id":&request.message_id,"user_id":&row.user_id,"conversation_id":&row.id,
            "turn_id":&request.turn_id,"seq":request.message_seq,"role":"user",
            "execution_pending":true,"via":"voice",
        })
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Conflict("Voice input unavailable".into()))?;
    if message.text != text {
        return Err(AppError::Conflict("Voice input changed".into()));
    }
    if request
        .credential_api_key_id
        .as_ref()
        .is_some_and(|k| *k != row.credential_api_key_id)
    {
        return Err(AppError::Conflict(
            "Voice continuation credential changed".into(),
        ));
    }
    if let Some(parent) = &request.parent_turn_id {
        if let Some(root) = &request.root_request_id {
            let root = requests
                .find_one(doc! {"_id":root,"user_id":&row.user_id,
                "conversation_id":&row.id})
                .session(&mut *session)
                .await?;
            if root.is_none_or(|r| r.state == RequestState::Cancelled) {
                return Err(AppError::Conflict("Voice task was stopped".into()));
            }
        }
        let parent_request = requests
            .find_one(doc! {
                "conversation_id":&row.id,"user_id":&row.user_id,"turn_id":parent,
            })
            .session(&mut *session)
            .await?;
        if parent_request.is_none_or(|p| p.state == RequestState::Cancelled) {
            return Err(AppError::Conflict("Voice continuation was stopped".into()));
        }
        if row
            .active_turn
            .as_ref()
            .is_some_and(|t| t.turn_id == *parent && t.stop_requested)
        {
            return Err(AppError::Conflict("Voice continuation was stopped".into()));
        }
        let cancelled = db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
            "conversation_id":&row.id,"turn_id":parent,"role":"assistant","error_code":"cancelled"
        }).session(&mut *session).await?.is_some();
        if cancelled {
            return Err(AppError::Conflict("Voice continuation was stopped".into()));
        }
    }
    requests
        .update_one(
            doc! {"_id":id,"state":"queued"},
            doc! {"$set":{"state":"claimed"}},
        )
        .session(&mut *session)
        .await?;
    Ok(request)
}

pub async fn queued(db: &Database) -> AppResult<Vec<VoiceRequest>> {
    db.collection::<VoiceRequest>(REQUESTS)
        .find(doc! {"state":"queued"})
        .sort(doc! {"created_at":1})
        .limit(100)
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

/// Never replay a claimed request after a lost worker. A pending card can survive
/// the call; expire only its bridge state, through the same transaction fence as
/// decisions. Existing turn admission handles the stale conversation lease.
pub async fn recover(db: &Database) -> AppResult<()> {
    let now = Utc::now();
    let cutoff = now - Duration::seconds(super::assistant_nyxagent::ACTIVE_TURN_TTL_SECS);
    let candidates: Vec<VoiceRequest> = db
        .collection::<VoiceRequest>(REQUESTS)
        .find(doc! {
            "state":{"$in":["claimed","awaiting_confirmation"]},
            "created_at":{"$lte":bson::DateTime::from_chrono(cutoff)}
        })
        .sort(doc! {"created_at":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for candidate in candidates {
        let db = db.clone();
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run2(async move |session| {
                let result: AppResult<()> =
                    Box::pin(async {
                        let requests = db.collection::<VoiceRequest>(REQUESTS);
                        let Some(row) = requests.find_one(doc! {
                    "_id":&candidate.id,"state":{"$in":["claimed","awaiting_confirmation"]}
                }).session(&mut *session).await? else { return Ok(()) };
                        let thread = db
                            .collection::<AssistantConversation>(CONVERSATIONS)
                            .find_one(doc! {"_id":&row.conversation_id,"user_id":&row.user_id})
                            .session(&mut *session)
                            .await?;
                        if thread
                            .as_ref()
                            .and_then(|t| super::assistant_nyxagent::live_turn(t, now))
                            .is_some_and(|t| t.turn_id == row.turn_id)
                        {
                            return Ok(());
                        }
                        let lost = thread.as_ref().is_none_or(|t| {
                            t.active_turn
                                .as_ref()
                                .is_some_and(|a| a.turn_id == row.turn_id)
                        });
                        if !lost && row.state == RequestState::AwaitingConfirmation {
                            // All cards have their own short authority expiry; no grant is
                            // extended and no continuation is invented for a timed-out card.
                            let pending = db.collection::<bson::Document>(
                        crate::models::assistant_acknowledgement::COLLECTION_NAME
                    ).count_documents(doc! {"_id":{"$in":&row.pending_acknowledgement_ids},
                        "status":"pending","expires_at":{"$gt":bson::DateTime::from_chrono(now)}})
                        .session(&mut *session).await?;
                            if pending > 0 {
                                return Ok(());
                            }
                        }
                        requests
                            .update_one(
                                doc! {"_id":&row.id},
                                doc! {"$set":{
                                    "state":"cancelled","pending_acknowledgement_ids":[]
                                }},
                            )
                            .session(&mut *session)
                            .await?;
                        Ok(())
                    })
                    .await;
                super::api_key_mutation_service::transaction_result(result)
            })
            .await
            .map_err(super::api_key_mutation_service::map_transaction_error)?;
    }
    Ok(())
}

pub(crate) async fn await_confirmation(
    db: &Database,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    session: &mut ClientSession,
) -> AppResult<()> {
    if let Some(request_id) = &card.voice_request_id
        && card.decider == "user"
    {
        db.collection::<VoiceRequest>(REQUESTS)
            .update_one(
                doc! {
                    "_id":request_id,"state":{"$in":["claimed","awaiting_confirmation"]}
                },
                doc! {"$set":{"state":"awaiting_confirmation"},
                "$addToSet":{"pending_acknowledgement_ids":&card.id}},
            )
            .session(&mut *session)
            .await?;
    }
    Ok(())
}

pub(crate) async fn continuation(
    db: &Database,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    session: &mut ClientSession,
) -> AppResult<Option<String>> {
    let Some(parent_id) = &card.voice_request_id else {
        return Ok(None);
    };
    if card.decider != "user" || card.trigger_run_id.is_some() {
        return Ok(None);
    }
    let parent = db
        .collection::<VoiceRequest>(REQUESTS)
        .find_one(doc! {
            "_id":parent_id,"user_id":&card.user_id,"conversation_id":&card.conversation_id
        })
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Conflict("Voice request unavailable".into()))?;
    if parent.state == RequestState::Cancelled {
        return Err(AppError::Conflict("Voice task was stopped".into()));
    }
    let stopped = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {
            "_id":&card.conversation_id,"active_turn.turn_id":&parent.turn_id,
            "active_turn.stop_requested":true,
        })
        .session(&mut *session)
        .await?
        .is_some();
    if stopped {
        return Err(AppError::Conflict("Voice task was stopped".into()));
    }
    let text = format!(
        "The owner decided the confirmation: {} (acknowledgement_id {}). Continue within the existing authority; do not retry a denied action.",
        card.status, card.id
    );
    let now = Utc::now();
    let request = enqueue_in_session(
        db,
        VoiceRequest {
            id: Uuid::new_v4().to_string(),
            user_id: card.user_id.clone(),
            conversation_id: card.conversation_id.clone(),
            session_id: parent.session_id,
            source_id: format!("ack:{}", card.id),
            message_id: Uuid::new_v4().to_string(),
            message_seq: 0,
            turn_id: Uuid::new_v4().to_string(),
            state: RequestState::Queued,
            pending_acknowledgement_ids: Vec::new(),
            acknowledgement_id: Some(card.id.clone()),
            credential_api_key_id: Some(card.api_key_id.clone()),
            parent_turn_id: Some(parent.turn_id),
            root_request_id: Some(parent.root_request_id.unwrap_or(parent.id)),
            created_at: now,
            expires_at: card.expires_at.min(now + Duration::minutes(30)),
        },
        &text,
        session,
    )
    .await?;
    db.collection::<VoiceRequest>(REQUESTS)
        .update_one(
            doc! {"_id":parent_id},
            doc! {"$pull":{"pending_acknowledgement_ids":&card.id}},
        )
        .session(&mut *session)
        .await?;
    let still_running = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {
            "_id": &card.conversation_id, "active_turn.turn_id": &request.parent_turn_id
        })
        .session(&mut *session)
        .await?
        .is_some();
    if !still_running {
        db.collection::<VoiceRequest>(REQUESTS).update_one(doc! {"_id":parent_id,"state":"awaiting_confirmation","pending_acknowledgement_ids":{"$size":0}},
            doc! {"$set":{"state":"completed"}}).session(&mut *session).await?;
    }
    Ok(Some(request.id))
}

pub async fn cancel(db: &Database, user: &str, conversation: &str, id: &str) -> AppResult<()> {
    thread(db, user, conversation).await?;
    let db = db.clone();
    let user = user.to_owned();
    let conversation = conversation.to_owned();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = Box::pin(async {
                let requests = db.collection::<VoiceRequest>(REQUESTS);
                let request = requests
                    .find_one(doc! {"_id":&id,"user_id":&user,"conversation_id":&conversation})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Voice request not found".into()))?;
                let root = request.root_request_id.as_deref().unwrap_or(&request.id);
                let mut family = doc! {"user_id":&user,"conversation_id":&conversation,
                "$or":[{"_id":root},{"root_request_id":root}]};
                if request.state == RequestState::Cancelled {
                    return Ok(());
                }
                family.insert(
                    "state",
                    doc! {"$in":["queued","claimed","awaiting_confirmation"]},
                );
                if requests
                    .find_one(family.clone())
                    .session(&mut *session)
                    .await?
                    .is_none()
                {
                    return Ok(());
                }
                // The original task is the cancellation fence even after its first
                // turn settled. Every later continuation checks this same root.
                requests
                    .update_one(doc! {"_id":root}, doc! {"$set":{"state":"cancelled"}})
                    .session(&mut *session)
                    .await?;
                requests
                    .update_many(family, doc! {"$set":{"state":"cancelled"}})
                    .session(&mut *session)
                    .await?;
                let conversations = db.collection::<AssistantConversation>(CONVERSATIONS);
                let current = conversations
                    .find_one(doc! {"_id":&conversation,"user_id":&user})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
                if let Some(active) = current.active_turn
                    && let Some(active_id) = active.voice_request_id
                {
                    let belongs = requests
                        .find_one(doc! {"_id":&active_id,
                        "$or":[{"_id":root},{"root_request_id":root}]})
                        .session(&mut *session)
                        .await?
                        .is_some();
                    if belongs {
                        conversations
                            .update_one(
                                doc! {"_id":&conversation,"active_turn.turn_id":&active.turn_id},
                                doc! {"$set":{"active_turn.stop_requested":true}},
                            )
                            .session(&mut *session)
                            .await?;
                    }
                }
                Ok(())
            })
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

#[derive(serde::Serialize)]
pub struct VoiceOption {
    service_id: String,
    connection_id: Option<String>,
    key_source: &'static str,
    model: &'static str,
    available: bool,
    unavailable_reason: &'static str,
    pricing: Option<super::inference_service::LanePricingView>,
    billing_owner: &'static str,
}

pub async fn options(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    thread: &AssistantConversation,
) -> AppResult<Vec<VoiceOption>> {
    // Reuse catalog visibility and its request-scoped, batched platform grants.
    // Discovery neither provisions a connection nor decrypts a user credential.
    let catalog = super::catalog_service::list_catalog_all(db, keys, &thread.user_id).await?;
    let connections =
        super::user_service_service::list_user_services_with_sources(db, &thread.user_id).await?;
    let services: Vec<crate::models::downstream_service::DownstreamService> = db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {"slug":{"$in":["llm-openai","llm-xai"]},"is_active":true})
        .await?
        .try_collect()
        .await?;
    let mut options = Vec::new();
    for entry in catalog {
        let model = match entry.slug.as_str() {
            "llm-openai" => "gpt-live-1",
            "llm-xai" => "grok-voice-think-fast-2.0",
            _ => continue,
        };
        if !entry.inference.as_ref().is_some_and(|i| i.realtime) || entry.requires_gateway_url {
            continue;
        }
        let Some(service) = services.iter().find(|s| s.slug == entry.slug) else {
            continue;
        };
        if entry.platform_key.available {
            options.push(VoiceOption {
                service_id: service.id.clone(),
                connection_id: None,
                key_source: "platform",
                model,
                available: false,
                unavailable_reason: "provider_adapter_not_enabled",
                pricing: entry.platform_key.pricing,
                billing_owner: "acting_person",
            });
        }
        let resource_owner = thread.agent_owner_id.as_deref().unwrap_or(&thread.user_id);
        for connection in connections.iter().filter(|c| {
            c.service.user_id == resource_owner
                && !matches!(
                    c.source,
                    super::user_service_service::CredentialSource::Org { allowed: false, .. }
                )
                && c.service.catalog_service_id.as_deref() == Some(&service.id)
                && c.service.node_id.is_none()
                && c.service.api_key_id.is_some()
                && c.service.credential_binding.as_deref() != Some("platform")
        }) {
            options.push(VoiceOption {
                service_id: service.id.clone(),
                connection_id: Some(connection.service.id.clone()),
                key_source: "own",
                model,
                available: false,
                unavailable_reason: "provider_adapter_not_enabled",
                pricing: entry.byok_pricing.clone(),
                billing_owner: if resource_owner == thread.user_id {
                    "acting_person"
                } else {
                    "organization"
                },
            });
        }
    }
    Ok(options)
}
