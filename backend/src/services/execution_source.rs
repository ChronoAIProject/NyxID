//! Additional authority retained by delegated work, independent of the Bot key
//! executing it and independent of where its result will be delivered.
//! Legacy Voice fields remain the storage projection; only this boundary
//! interprets them as source authority.
use crate::{
    errors::{AppError, AppResult, access_denial::permanent_authority_failure},
    models::{
        assistant_conversation::{ActiveTurn, AgentEvent, AssistantConversation},
        async_service_operation::DeliveryBinding,
    },
};
use mongodb::{ClientSession, Database, bson::doc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    VoiceRequest(&'a str),
}

impl<'a> Source<'a> {
    pub async fn require(self, db: &Database, user: &str) -> AppResult<()> {
        match self {
            Self::VoiceRequest(id) => Box::pin(super::voice::source::request(db, user, id)).await,
        }
    }
    pub async fn fence(
        self,
        db: &Database,
        user: &str,
        session: &mut ClientSession,
    ) -> AppResult<()> {
        match self {
            Self::VoiceRequest(id) => {
                Box::pin(super::voice::source::fence_request(db, user, id, session)).await
            }
        }
    }
    /// Compatibility projection for existing persisted request fields.
    pub fn legacy_request_id(self) -> &'a str {
        match self {
            Self::VoiceRequest(id) => id,
        }
    }
}

pub fn for_turn(turn: &ActiveTurn) -> Option<Source<'_>> {
    turn.voice_request_id
        .as_deref()
        .or(turn.voice_source_request_id.as_deref())
        .map(Source::VoiceRequest)
}
pub fn for_delivery(delivery: &DeliveryBinding) -> Option<Source<'_>> {
    delivery
        .voice_request_id
        .as_deref()
        .map(Source::VoiceRequest)
}
pub fn for_event(event: &AgentEvent) -> Option<Source<'_>> {
    event
        .voice_source_request_id
        .as_deref()
        .map(Source::VoiceRequest)
}

/// Source authority is restored on both root and descendant continuations;
/// only a root continuation may settle the originating request.
pub fn restore_delivery(delivery: &DeliveryBinding, turn: &mut ActiveTurn) {
    turn.voice_request_id = delivery
        .voice_request_id
        .clone()
        .filter(|_| !delivery.voice_source_only);
    turn.voice_source_request_id = delivery
        .voice_request_id
        .clone()
        .filter(|_| delivery.voice_source_only);
}

pub fn legacy_turn_request_id(turn: &ActiveTurn) -> Option<&str> {
    for_turn(turn).map(Source::legacy_request_id)
}

pub async fn source_for_chat(
    db: &Database,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<Option<String>> {
    let row = db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &chat.conversation_id, "user_id": &chat.user_id, "credential_api_key_id": &chat.api_key_id})
        .await?.ok_or(AppError::AssistantTurnRequired)?;
    let source = row.active_turn.as_ref().and_then(for_turn);
    if let Some(source) = source {
        if row.active_turn.as_ref().is_none_or(|turn| {
            Some(turn.turn_id.as_str()) != chat.turn_id.as_deref() || turn.stop_requested
        }) {
            return Err(AppError::AssistantTurnRequired);
        }
        source.require(db, &chat.user_id).await?;
    }
    Ok(source.map(|s| s.legacy_request_id().to_owned()))
}

/// Uncertainty aborts admission without consuming any queued event. Confirmed
/// lost authority discards only the event belonging to that source.
pub async fn filter_events_in_session(
    db: &Database,
    user: &str,
    events: Vec<AgentEvent>,
    session: &mut ClientSession,
) -> AppResult<Vec<AgentEvent>> {
    let mut eligible = Vec::new();
    for event in events {
        if let Some(source) = for_event(&event) {
            match source.fence(db, user, session).await {
                Ok(()) => {}
                Err(error) if permanent_authority_failure(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        eligible.push(event);
    }
    Ok(eligible)
}
