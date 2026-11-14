//! Supplemental text uses the local relay without marking a gateway event replied.
use super::*;
use crate::models::assistant_conversation::AssistantConversation;
use crate::services::channel_supplemental_delivery as sources;
pub(crate) use crate::services::channel_supplemental_delivery::{Anchor, Purpose, Reason};

#[derive(Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Egress {
    LocalRelay,
    GatewayEvent,
    Skipped,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Confirmed,
    Rejected,
    Unknown,
    Skipped,
}

pub struct Decision {
    pub purpose: Purpose,
    pub anchor: Anchor,
    pub event: Option<String>,
    egress: Egress,
    outcome: Outcome,
    reason: Option<Reason>,
}
impl Decision {
    pub fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            anchor: Anchor::None,
            event: None,
            egress: Egress::Skipped,
            outcome: Outcome::Skipped,
            reason: Some(Reason::SourceUnavailable),
        }
    }
    pub fn skip(&mut self, reason: Reason) {
        self.egress = Egress::Skipped;
        self.outcome = Outcome::Skipped;
        self.reason = Some(reason);
    }
    pub fn dispatch(&mut self, egress: Egress) {
        self.egress = egress;
        self.reason = None;
        self.outcome = Outcome::Unknown;
    }
    pub fn outcome(&mut self, outcome: &'static str) {
        self.outcome = match outcome {
            "confirmed" => Outcome::Confirmed,
            "rejected" | "refused" => Outcome::Rejected,
            _ => Outcome::Unknown,
        };
    }
    pub fn result(&mut self, result: &AppResult<()>) {
        if let Err(error) = result {
            if self.egress == Egress::Skipped {
                self.reason = Some(match error {
                    AppError::Forbidden(_) | AppError::NotFound(_) => Reason::AuthorityRefused,
                    _ => Reason::SourceUnavailable,
                });
                return;
            }
            self.reason = None;
            self.outcome = match images::classify_outcome(error) {
                images::ImageOutcome::Failed(_) => Outcome::Rejected,
                _ => Outcome::Unknown,
            };
        }
    }
    pub async fn record(&self, state: &AppState, row: &AssistantConversation) {
        self.record_ids(
            state,
            &row.user_id,
            &row.id,
            row.active_turn.as_ref().map(|t| t.turn_id.as_str()),
        )
        .await;
    }
    pub async fn record_ids(
        &self,
        state: &AppState,
        owner: &str,
        conversation: &str,
        turn: Option<&str>,
    ) {
        audit(
            state,
            owner,
            "nyxbot_channel_supplemental_delivery",
            json!({
                "purpose":self.purpose,"egress":self.egress,"anchor":self.anchor,
                "outcome":self.outcome,"reason":self.reason,"turn_id":turn,
                "channel_event_id":self.event,"conversation_id":conversation,
            }),
        )
        .await;
    }
}

pub async fn event_anchor(
    state: &AppState,
    event: &NyxbotEvent,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    decision: &mut Decision,
) -> AppResult<Option<String>> {
    decision.event = Some(event.id.clone());
    decision.anchor = Anchor::EventSource;
    let Some(id) = &event.relay_message_id else {
        decision.skip(Reason::MissingAnchor);
        return Ok(None);
    };
    match sources::validate_anchor(
        &state.db,
        channel,
        thread,
        id,
        event.delivery.as_ref().map(|d| d.sender_id.as_str()),
    )
    .await?
    {
        Ok(Some(_)) => Ok(Some(id.clone())),
        Ok(None) => {
            decision.skip(Reason::MissingAnchor);
            Ok(None)
        }
        Err(reason) => {
            decision.skip(reason);
            Ok(None)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    state: &AppState,
    row: &AssistantConversation,
    origin: &ChannelOrigin,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    text: &str,
    decision: &mut Decision,
) -> AppResult<()> {
    if let Err(reason) = sources::reference_live(thread) {
        decision.skip(reason);
        return Ok(());
    }
    // Keep the existing gateway-send eligibility even though egress is local.
    let ciphertext = thread
        .event_ref_ciphertext
        .as_deref()
        .ok_or_else(|| AppError::Internal("Event reference unavailable".into()))?;
    let reference = Zeroizing::new(state.encryption_keys.decrypt(ciphertext).await?);
    std::str::from_utf8(&reference)
        .map_err(|_| AppError::Internal("Event reference unavailable".into()))?;
    if decrypt_agent_key(state, channel).await?.is_none() {
        decision.skip(Reason::AuthorityRefused);
        return Ok(());
    }
    let id = match sources::source(&state.db, row, origin).await? {
        sources::Source::Missing => {
            decision.skip(Reason::MissingAnchor);
            return Ok(());
        }
        sources::Source::Event(event) => {
            decision.event = Some(event.id.clone());
            let Some(delivery) = &event.delivery else {
                decision.skip(Reason::SourceUnavailable);
                return Ok(());
            };
            if late_delivery::eligible(state, &event, delivery, row)
                .await?
                .is_none()
            {
                decision.skip(Reason::AuthorityRefused);
                return Ok(());
            }
            let Some(id) = event_anchor(state, &event, channel, thread, decision).await? else {
                return Ok(());
            };
            id
        }
        sources::Source::Unbound => {
            match sources::chat_anchor(&state.db, channel, thread, decision.purpose).await? {
                Ok((id, anchor)) => {
                    decision.anchor = anchor;
                    id
                }
                Err(reason) => {
                    decision.skip(reason);
                    return Ok(());
                }
            }
        }
    };
    decision.dispatch(Egress::LocalRelay);
    crate::services::channel_reply_notice::supplemental(Box::pin(direct_reply(
        state, channel, &id, text, None,
    )))
    .await?;
    decision.outcome("confirmed");
    Ok(())
}

pub async fn native(
    state: &AppState,
    channel: &NyxbotChannel,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
    decision: &mut Decision,
) -> AppResult<()> {
    decision.event = origin.native_event_id.clone();
    if decision.purpose.progress() {
        decision.skip(Reason::NativeProgressUnsupported);
        return Ok(());
    }
    decision.anchor = Anchor::EventSource;
    decision.dispatch(Egress::GatewayEvent);
    let outcome =
        native_responses::send_supplemental(state, channel, origin, conversation, text).await?;
    decision.outcome(outcome);
    Ok(())
}

pub async fn followed(
    state: &AppState,
    channel: &NyxbotChannel,
    owner: &str,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
    decision: &mut Decision,
) -> AppResult<()> {
    let binding = origin
        .thread
        .as_ref()
        .ok_or_else(crate::services::channel_thread_follow_service::not_found)?;
    let event = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {
        "channel_id":&channel.id,"user_id":&channel.user_id,"event_id":&binding.source_message_id,
        "resolved_thread_id":&binding.child_id,
    }).await?;
    let Some(event) = event else {
        decision.skip(Reason::SourceUnavailable);
        return Ok(());
    };
    let Ok((actor, admitted_origin, admitted_conversation)) =
        crate::services::channel_thread_service::gateway::admitted_reply(
            &event,
            channel,
            &binding.source_message_id,
        )
    else {
        decision.skip(Reason::SourceUnavailable);
        return Ok(());
    };
    if actor != owner
        || admitted_conversation != conversation
        || !super::gateway_threads::same_reply_identity(admitted_origin, origin)
    {
        decision.skip(Reason::SourceUnavailable);
        return Ok(());
    }
    decision.event = Some(event.id.clone());
    decision.anchor = Anchor::ThreadBinding;
    if event.created_at + ChronoDuration::minutes(29) <= Utc::now() {
        decision.skip(Reason::ReferenceExpired);
        return Ok(());
    }
    let Some(ciphertext) = &event.event_context_ciphertext else {
        decision.skip(Reason::ReferenceUnavailable);
        return Ok(());
    };
    let context: Value = serde_json::from_slice(&Zeroizing::new(
        state.encryption_keys.decrypt(ciphertext).await?,
    ))
    .map_err(|_| AppError::Internal("Channel source unavailable".into()))?;
    if context["event_ref"].as_str().is_none() || decrypt_agent_key(state, channel).await?.is_none()
    {
        decision.skip(Reason::ReferenceUnavailable);
        return Ok(());
    }
    // The local bound adapter independently validates the exact root and live
    // source authority. Do not replace a missing source with a chat pointer.
    if event.relay_message_id.as_deref() != Some(binding.source_message_id.as_str()) {
        decision.skip(Reason::MissingAnchor);
        return Ok(());
    }
    if state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {"_id": &binding.source_message_id})
        .projection(doc! {"_id": 1})
        .await?
        .is_none()
    {
        decision.skip(Reason::MissingAnchor);
        return Ok(());
    }
    decision.dispatch(Egress::LocalRelay);
    let outcome = crate::services::channel_reply_notice::supplemental(Box::pin(
        thread_follow::send_bound_reply(
            state,
            channel,
            owner,
            origin,
            conversation,
            &crate::services::channel_platform::OutboundReply {
                text: Some(text.to_owned()),
                attachments: Vec::new(),
                reply_to_platform_message_id: None,
                metadata: crate::services::channel_reply_plan::assistant_metadata(
                    &channel.platform,
                ),
            },
        ),
    ))
    .await?;
    if let Some(error) = outcome.error {
        return Err(error);
    }
    decision.outcome("confirmed");
    Ok(())
}
