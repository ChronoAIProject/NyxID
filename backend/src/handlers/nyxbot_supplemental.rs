//! Source-bound supplemental text delivery and metadata-only auditing.
use super::*;
use crate::models::assistant_conversation::AssistantConversation;
use crate::services::channel_supplemental_delivery as sources;
pub(crate) use crate::services::channel_supplemental_delivery::{Anchor, Purpose, Reason};

#[derive(Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Egress {
    LocalRelay,
    GatewayEvent,
    GatewaySideMessage,
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
    pub(super) saved_result: bool,
    pub(super) supervisor_occurrence: Option<String>,
    pub(crate) source: Option<sources::ResultSource>,
    pub(super) machine_control: Option<(String, i64)>,
    pub(super) native_route: Option<native_responses::SupplementalRoute>,
    pub(super) require_side_message: bool,
    pub purpose: Purpose,
    pub anchor: Anchor,
    pub event: Option<String>,
    pub turn: Option<String>,
    egress: Egress,
    outcome: Outcome,
    reason: Option<Reason>,
}
impl Decision {
    pub fn new(purpose: Purpose) -> Self {
        Self {
            saved_result: false,
            supervisor_occurrence: None,
            source: None,
            machine_control: None,
            native_route: None,
            require_side_message: false,
            purpose,
            anchor: Anchor::None,
            event: None,
            turn: None,
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
    pub(super) fn needs_post_turn_claim(&self) -> bool {
        self.purpose.result() && !self.saved_result && self.turn.is_some()
    }
    pub(super) async fn claim_result(
        &mut self,
        state: &AppState,
        owner: &str,
        conversation: &str,
        origin: &ChannelOrigin,
    ) -> AppResult<bool> {
        match sources::claim_result(&state.db, owner, conversation, self.turn.as_deref(), origin)
            .await?
        {
            Ok(()) => Ok(true),
            Err(reason) => {
                self.skip(reason);
                Ok(false)
            }
        }
    }
    pub(crate) fn not_sent(&self) -> bool {
        matches!(self.egress, Egress::Skipped)
    }
    pub(crate) fn confirmed(&self) -> bool {
        matches!(self.outcome, Outcome::Confirmed)
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
    let Some(id) = event.relay_message_id.as_ref().or_else(|| {
        event
            .delivery
            .as_ref()
            .filter(|delivery| decision.saved_result && delivery.transport == "direct")
            .map(|_| &event.event_id)
    }) else {
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
        Ok(Some(message)) => match sources::result_window(decision.purpose, message.created_at) {
            Ok(()) => Ok(Some(id.clone())),
            Err(reason) => {
                decision.skip(reason);
                Ok(None)
            }
        },
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
    if !decision.purpose.result() && decision.supervisor_occurrence.is_none() {
        if let Err(reason) = sources::reference_live(thread) {
            decision.skip(reason);
            return Ok(());
        }
        // Progress retains the original gateway-reference window.
        crate::services::nyxbot_thread_reference::open(&state.encryption_keys, thread).await?;
    }
    if decrypt_agent_key(state, channel).await?.is_none() {
        decision.skip(Reason::AuthorityRefused);
        return Ok(());
    }
    let id = match sources::source_for(&state.db, &row.id, origin, decision.source.as_ref()).await?
    {
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
            let result_anchor = if decision.purpose.result() {
                let Some(id) = event_anchor(state, &event, channel, thread, decision).await? else {
                    return Ok(());
                };
                Some(id)
            } else {
                None
            };
            if late_delivery::eligible(state, &event, delivery, row)
                .await?
                .is_none()
            {
                decision.skip(Reason::AuthorityRefused);
                return Ok(());
            }
            match result_anchor {
                Some(id) => id,
                None => {
                    let Some(id) = event_anchor(state, &event, channel, thread, decision).await?
                    else {
                        return Ok(());
                    };
                    id
                }
            }
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
    if decision.purpose.result() {
        // Complete local authority and credential preparation before the claim;
        // the ordinary adapter still revalidates immediately before sending.
        prepare_local_result(state, channel, thread, &id, text).await?;
        if !decision.saved_result
            && !decision
                .claim_result(state, &row.user_id, &row.id, origin)
                .await?
        {
            return Ok(());
        }
    }
    decision.dispatch(Egress::LocalRelay);
    crate::services::channel_reply_notice::supplemental(Box::pin(send_local(
        state, channel, &id, origin, text, decision,
    )))
    .await?;
    decision.outcome("confirmed");
    Ok(())
}

pub(super) async fn send_local(
    state: &AppState,
    channel: &NyxbotChannel,
    id: &str,
    origin: &ChannelOrigin,
    text: &str,
    decision: &Decision,
) -> AppResult<()> {
    let send = direct_reply(state, channel, id, text, None);
    if let Some(occurrence) = &decision.supervisor_occurrence {
        sources::SAVED_RELAY_KEY
            .scope(occurrence.clone(), send)
            .await?;
        let plan = crate::services::channel_text_delivery::load(&state.db, occurrence)
            .await?
            .ok_or_else(|| AppError::Forbidden("Complete review delivery is unconfirmed".into()))?;
        if plan.parts.is_empty()
            || plan
                .parts
                .iter()
                .any(|part| part.state != crate::models::channel_text_delivery::TextPartState::Sent)
        {
            return Err(AppError::Forbidden(
                "Complete review delivery is unconfirmed".into(),
            ));
        }
        Ok(())
    } else if decision.saved_result {
        let turn = decision
            .turn
            .as_deref()
            .ok_or(AppError::AssistantTurnRequired)?;
        sources::SAVED_RELAY_KEY
            .scope(sources::saved_key(turn, origin), send)
            .await
    } else {
        send.await
    }
}

pub(super) async fn prepare_local_result(
    state: &AppState,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    id: &str,
    text: &str,
) -> AppResult<()> {
    use crate::handlers::channel_relay as relay;
    let auth = route_key_auth(state, channel).await?;
    let body = relay::AsyncReplyRequest {
        message_id: id.to_owned(),
        thread_reply: false,
        reply: relay::AsyncReplyBody {
            text: Some(text.to_owned()),
            metadata: crate::services::channel_reply_plan::assistant_metadata(&channel.platform),
            attachments: Vec::new(),
        },
    };
    let context =
        relay::resolve_reply_request_context(state, &HeaderMap::new(), Some(&auth), &body).await?;
    let adapter = relay::reply_adapter(state, &context)?;
    let bot = channel_bot_service::get_bot(&state.db, &channel.channel_bot_id).await?;
    crate::services::channel_credentials::resolve_bot_token(
        &state.db,
        &state.encryption_keys,
        adapter.as_ref(),
        &bot,
    )
    .await?;
    prepare_result_text(
        text,
        &channel.platform,
        thread.platform_chat_id.as_deref().unwrap_or_default(),
    )?;
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
    decision.anchor = Anchor::EventSource;
    let outcome = if let Some(occurrence) = decision.supervisor_occurrence.clone() {
        decision.require_side_message = true;
        Some(
            native_responses::send_occurrence(
                state,
                channel,
                origin,
                conversation,
                text,
                &occurrence,
                decision,
            )
            .await?,
        )
    } else {
        native_responses::send_supplemental(state, channel, origin, conversation, text, decision)
            .await?
    };
    if let Some(outcome) = outcome {
        decision.outcome(outcome);
    }
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
    if decision.purpose.result() {
        if let Err(reason) = sources::result_window(decision.purpose, event.created_at) {
            decision.skip(reason);
            return Ok(());
        }
    } else {
        if event.created_at + ChronoDuration::minutes(29) <= Utc::now() {
            decision.skip(Reason::ReferenceExpired);
            return Ok(());
        }
        let Some(ciphertext) = &event.event_context_ciphertext else {
            decision.skip(Reason::ReferenceUnavailable);
            return Ok(());
        };
        let context: Value = serde_json::from_slice(
            &crate::services::nyxbot_event_record::event_context(
                &state.encryption_keys,
                &event,
                ciphertext,
            )
            .await?,
        )
        .map_err(|_| AppError::Internal("Channel source unavailable".into()))?;
        if context["event_ref"].as_str().is_none() {
            decision.skip(Reason::ReferenceUnavailable);
            return Ok(());
        }
    }
    if decrypt_agent_key(state, channel).await?.is_none() {
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
    if decision.purpose.result() {
        // This shared resolver prepares live credentials and the exact admitted
        // root without sending or acquiring an image delivery claim.
        thread_follow::bound_image_audience(state, channel, owner, origin, conversation).await?;
        prepare_result_text(text, &channel.platform, "")?;
        if !decision
            .claim_result(state, owner, conversation, origin)
            .await?
        {
            return Ok(());
        }
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

pub(super) fn prepare_result_text(text: &str, platform: &str, chat: &str) -> AppResult<()> {
    crate::services::channel_reply_plan::ordinary(
        &crate::services::channel_platform::OutboundReply {
            text: Some(text.to_owned()),
            metadata: crate::services::channel_reply_plan::assistant_metadata(platform),
            attachments: Vec::new(),
            reply_to_platform_message_id: None,
        },
        platform,
        chat,
    )?;
    Ok(())
}
