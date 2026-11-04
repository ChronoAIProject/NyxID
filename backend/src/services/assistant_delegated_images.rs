//! Assigned tool images retain their source lifetime and a server-authored delivery edge.
use super::{assistant_nyxagent as engine, assistant_team_service as team};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{
            ActiveTurn, AgentEvent, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
            ChannelOrigin, TurnOrigin,
        },
        assistant_delegated_image::{Assignment, DeliveryReference},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};

pub const MAX_REFERENCES: usize = 16;

fn eligible(row: &AssistantConversation) -> bool {
    !row.guest_turn
        && !row.is_subagent()
        && row.group_id.is_none()
        && row.collaboration.is_none()
        && row.platform_thread.is_none()
}
fn same_destination(a: &ChannelOrigin, b: &ChannelOrigin) -> bool {
    a.nyxbot_channel_id == b.nyxbot_channel_id
        && a.partition == b.partition
        && a.platform == b.platform
        && a.thread == b.thread
}

/// Called inside source admission, before it is executable. The recipient write
/// conflicts with concurrent Stop, reset, settlement or deletion.
pub async fn assign_in_session(
    db: &Database,
    tx: &mut ClientSession,
    source: &mut AssistantConversation,
) -> AppResult<()> {
    let Some(turn) = source.active_turn.as_ref() else {
        return Ok(());
    };
    if turn.origin != TurnOrigin::Orchestrator || source.guest_turn || !source.is_subagent() {
        return Ok(());
    }
    let Some(target) = turn.report_to.as_ref() else {
        return Ok(());
    };
    let Some(recipient) = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":target,"user_id":&source.user_id})
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    let Some(receiving) = engine::live_turn(&recipient, Utc::now()).filter(|t| !t.stop_requested)
    else {
        return Ok(());
    };
    if !eligible(&recipient) {
        return Ok(());
    }
    let routing = engine::answer_routing(&recipient, receiving);
    let mut destinations = routing.also;
    let mut trigger_chat = None;
    match routing.primary {
        engine::PrimaryDestination::ChannelEvent { origin } => destinations.push(origin),
        engine::PrimaryDestination::ChatUpdate { origins } => destinations.extend(origins),
        engine::PrimaryDestination::Trigger { run_id } => {
            if let Some(run) = db.collection::<Document>(crate::models::trigger_run::COLLECTION_NAME)
                .find_one(doc! {"_id":run_id,"user_id":&source.user_id,"thread_id":&recipient.id,"turn_id":&receiving.turn_id})
                .session(&mut *tx).await?
                && let Ok(delivery) = run.get_document("deliver_to")
                && delivery.get_str("type").ok() == Some("chat") {
                trigger_chat = delivery.get_str("chat_id").ok().map(str::to_owned);
            }
        }
        _ => {}
    }
    destinations.truncate(engine::MAX_ALSO_DELIVER + 1);
    let assignment = Assignment {
        id: turn.turn_id.clone(),
        actor: source.user_id.clone(),
        source_agent: source
            .agent_id
            .clone()
            .ok_or(AppError::AssistantTurnRequired)?,
        source_conversation: source.id.clone(),
        source_turn: turn.turn_id.clone(),
        recipient_conversation: recipient.id.clone(),
        assigning_turn: receiving.turn_id.clone(),
        recipient_reset_at: recipient.context_reset_at,
        destinations,
        trigger_chat,
    };
    db.collection::<Document>(CONVERSATIONS).update_one(
        doc! {"_id":&recipient.id,"user_id":&recipient.user_id,"active_turn.turn_id":&receiving.turn_id,"active_turn.stop_requested":false},
        doc! {"$inc":{"delegation_revision":1_i64}}).session(&mut *tx).await?;
    source
        .active_turn
        .as_mut()
        .unwrap()
        .delegated_images
        .assignment = Some(assignment);
    Ok(())
}

/// Accepted results and their queue entries commit together. Business refusals
/// must not roll back source settlement; the caller persists the refusal so the
/// existing after-settlement hook can attempt a text-only notice instead.
pub async fn publish_in_session(
    db: &Database,
    tx: &mut ClientSession,
    message: &AssistantMessage,
    turn: &ActiveTurn,
) -> AppResult<Option<&'static str>> {
    let Some(edge) = &message.delegated_images.assignment else {
        return Ok(None);
    };
    // A cancelled invocation must still settle its source turn, but cannot
    // publish a continuation. Match the existing invocation event producer.
    if !super::assistant_invocation_service::fence_in_session(
        db,
        tx,
        &edge.actor,
        turn.invocation_id.as_deref(),
    )
    .await?
    {
        return Ok(None);
    }
    let agent = db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&edge.source_agent})
        .session(&mut *tx)
        .await?;
    let Some(agent) = agent else { return Ok(None) };
    let status = match message.error_code.as_deref() {
        None => "replied".to_owned(),
        Some("cancelled") => "was stopped".to_owned(),
        Some(code) => format!("failed ({code})"),
    };
    let mut event = team::event("subagent_settled", format!(
        "Specialist {} {status}. Reply excerpt: \"{}\" Read more with nyxid__read_subagent (agent ID {}).",
        engine::identifier(&agent.name), engine::excerpt(&message.text, 1200).replace('"', "'"), agent.id),
        Some(&edge.source_agent)).for_invocation(turn.invocation_id.as_deref());
    event.id = edge.id.clone();
    event.delegated_result = Some(edge.clone());
    // Preserve exact followed-thread destinations through the event routing reducer.
    event.reply_to = edge
        .destinations
        .iter()
        .filter(|o| o.thread.is_some())
        .cloned()
        .collect();
    let refusal = publication_outcome(
        engine::append_events_in_session(
            db,
            &edge.actor,
            &edge.recipient_conversation,
            &[event],
            tx,
        )
        .await,
    )?;
    if let Some(code) = refusal {
        tracing::warn!(target: "nyxid::assistant_delegated_images",
            actor_id = %edge.actor, source_agent_id = %edge.source_agent,
            source_conversation_id = %edge.source_conversation, source_turn_id = %edge.source_turn,
            recipient_conversation_id = %edge.recipient_conversation, recipient_turn_id = %edge.assigning_turn,
            assignment_id = %edge.id, code,
            "Delegated result publication refused; text-only settlement notice remains best-effort");
    }
    Ok(refusal)
}

fn publication_outcome(result: AppResult<bool>) -> AppResult<Option<&'static str>> {
    match result {
        Ok(true) => Ok(None),
        Ok(false) => Ok(Some("event_recipient_unavailable")),
        Err(AppError::Conflict(message)) if message == "Assistant event queue is full" => {
            Ok(Some("event_queue_full"))
        }
        Err(AppError::Conflict(_)) => Ok(Some("event_identity_conflict")),
        Err(AppError::AssistantTurnRequired) => Ok(Some("event_invocation_unavailable")),
        // These are all business refusals append currently returns. Database,
        // transport and encoding/internal errors retain transaction semantics.
        Err(error) => Err(error),
    }
}

async fn source_result(
    db: &Database,
    tx: &mut ClientSession,
    edge: &Assignment,
) -> AppResult<Option<AssistantMessage>> {
    let Some(source) = db.collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&edge.source_conversation,"user_id":&edge.actor,"agent_id":&edge.source_agent})
        .session(&mut *tx).await? else { return Ok(None) };
    // Assigned home work only. Hidden group history is never exported by an ID.
    if !source.is_subagent() || source.group_id.is_some() || source.collaboration.is_some() {
        return Ok(None);
    }
    let agent = team::agent_for_conversation(db, &source).await?;
    if agent.destroyed_at.is_some() {
        return Ok(None);
    }
    super::org_agent_service::require_use(db, &edge.actor, &agent).await?;
    let message = db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "user_id":&edge.actor,"conversation_id":&edge.source_conversation,"turn_id":&edge.source_turn,"role":"assistant",
    }).session(&mut *tx).await?;
    Ok(message.filter(|m| {
        m.delegated_images.assignment.as_ref() == Some(edge)
            && m.delegated_images.publication_refusal.is_none()
    }))
}

async fn receiver_valid(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    edge: &Assignment,
) -> AppResult<bool> {
    if !eligible(row)
        || row.id != edge.recipient_conversation
        || row.user_id != edge.actor
        || row.context_reset_at != edge.recipient_reset_at
    {
        return Ok(false);
    }
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":&edge.actor,"is_active":true})
        .session(&mut *tx)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let agent = team::agent_for_conversation(db, row).await?;
    if agent.destroyed_at.is_some() || !agent.is_nyxbot() || agent.user_id != edge.actor {
        return Ok(false);
    }
    if let Some(turn) = row
        .active_turn
        .as_ref()
        .filter(|t| t.turn_id == edge.assigning_turn)
    {
        return Ok(!turn.stop_requested && engine::live_turn(row, Utc::now()).is_some());
    }
    Ok(db.collection::<Document>(MESSAGES).find_one(doc! {"user_id":&edge.actor,"conversation_id":&row.id,"turn_id":&edge.assigning_turn,"role":"assistant","status":"completed"})
        .session(&mut *tx).await?.is_some())
}

async fn destination_valid(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    edge: &Assignment,
) -> AppResult<bool> {
    for origin in &edge.destinations {
        // Clearing the home DM in the web app never forwards an old result to
        // the previous chat, or to a new private chat the owner later opens.
        if row
            .channel
            .as_ref()
            .or(row.reply_channel.as_ref())
            .is_none_or(|live| !same_destination(live, origin))
        {
            continue;
        }
        if super::channel_turn_delivery::target(db, row, origin)
            .await?
            .is_some()
        {
            if origin.thread.is_some() {
                Box::pin(super::channel_thread_follow_service::validate_in_session(
                    db,
                    &edge.actor,
                    origin,
                    &row.id,
                    true,
                    tx,
                ))
                .await?;
            }
            return Ok(true);
        }
    }
    if let Some(chat_id) = &edge.trigger_chat {
        use crate::models::nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, NyxbotChannel, NyxbotThread,
            THREADS_COLLECTION_NAME as THREADS,
        };
        let Some(thread) = db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id":chat_id,"user_id":&edge.actor,"allow_posts":true})
            .await?
        else {
            return Ok(false);
        };
        let Some(channel) = db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&thread.channel_id,"user_id":&edge.actor,"status":"active"})
            .await?
        else {
            return Ok(false);
        };
        if let Some(agent) = thread.agent_id.as_ref().or(channel.agent_id.as_ref())
            && Some(agent) != row.agent_id.as_ref()
        {
            return Ok(false);
        }
        if let Some(org) = &channel.bot_owner_id
            && !matches!(
                super::org_service::resolve_owner_access(db, &edge.actor, org).await?,
                super::org_service::OwnerAccess::AsOrgAdmin { .. }
            )
        {
            return Ok(false);
        }
        return Ok(db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(
                doc! {"_id":&channel.channel_bot_id,"is_active":true,"status":{"$ne":"suspended"}},
            )
            .await?
            .is_some());
    }
    // App-only results may be adopted but never acquire a chat destination.
    Ok(edge.destinations.is_empty())
}

fn unavailable(error: &AppError) -> bool {
    error.is_forbidden()
        || matches!(
            error,
            AppError::NotFound(_)
                | AppError::AssistantAttachmentExpired
                | AppError::AssistantTurnRequired
        )
}

async fn references(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    edge: &Assignment,
) -> AppResult<Vec<DeliveryReference>> {
    if !receiver_valid(db, tx, row, edge).await? || !destination_valid(db, tx, row, edge).await? {
        return Ok(Vec::new());
    }
    let Some(turn) = engine::live_turn(row, Utc::now()).filter(|t| !t.stop_requested) else {
        return Ok(Vec::new());
    };
    let Some(message) = source_result(db, tx, edge).await? else {
        return Ok(Vec::new());
    };
    if message.status != "completed" {
        return Ok(Vec::new());
    }
    let mut refs = Vec::new();
    for attachment in message
        .attachments
        .into_iter()
        .filter(engine::chat_tool_image)
        .take(engine::MAX_TURN_ATTACHMENTS)
    {
        let filter = doc! {"_id":&attachment.id,"user_id":&edge.actor,"conversation_id":&edge.source_conversation,"turn_id":&edge.source_turn,"origin":"tool"};
        match super::assistant_upload_retention::require_available(db, filter).await {
            Ok(()) => refs.push(DeliveryReference {
                assignment: edge.clone(),
                recipient_turn: turn.turn_id.clone(),
                attachment,
            }),
            Err(error) if unavailable(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(refs)
}

/// Do not render a revoked source's excerpt into the next model input. Legacy
/// notices keep their existing behavior; only exact assignment results use this gate.
pub async fn filter_events_in_session(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    events: Vec<AgentEvent>,
) -> AppResult<Vec<AgentEvent>> {
    let mut allowed = Vec::new();
    for event in events {
        if let Some(edge) = &event.delegated_result {
            let valid: AppResult<bool> = async {
                Ok(event.id == edge.id
                    && event.agent_id.as_deref() == Some(&edge.source_agent)
                    && receiver_valid(db, tx, row, edge).await?
                    && source_result(db, tx, edge).await?.is_some())
            }
            .await;
            match valid {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) if unavailable(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        allowed.push(event);
    }
    Ok(allowed)
}

/// Admission has removed the events only in memory. Adoption and that removal
/// are saved by the same conversation transaction. Guests never drain them.
pub async fn adopt_in_session(
    db: &Database,
    tx: &mut ClientSession,
    row: &mut AssistantConversation,
    events: &[AgentEvent],
) -> AppResult<()> {
    if !eligible(row) {
        return Ok(());
    }
    for event in events {
        let Some(edge) = &event.delegated_result else {
            continue;
        };
        if event.id != edge.id || event.agent_id.as_deref() != Some(&edge.source_agent) {
            continue;
        }
        let refs = match references(db, tx, row, edge).await {
            Ok(refs) => refs,
            Err(error) if unavailable(&error) => Vec::new(),
            Err(error) => return Err(error),
        };
        let saved = &mut row
            .active_turn
            .as_mut()
            .ok_or(AppError::AssistantTurnRequired)?
            .delegated_images
            .references;
        for reference in refs {
            if saved.len() < MAX_REFERENCES
                && !saved
                    .iter()
                    .any(|r| r.attachment.id == reference.attachment.id)
            {
                saved.push(reference)
            }
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct WaitResult {
    pub references: Vec<DeliveryReference>,
    pub results: Vec<(String, Option<team::ReplySummary>)>,
}

/// Wait and event drain serialize on the receiving conversation. Pull only the
/// exact observed events after adoption; a newer result from the same agent stays.
pub async fn consume_wait(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    agents: &[String],
) -> AppResult<WaitResult> {
    let context = (
        db.clone(),
        owner.to_owned(),
        conversation.to_owned(),
        turn.to_owned(),
        agents.to_vec(),
    );
    super::transaction_service::run_db_only(db, &context, |tx, (db, owner, conversation, turn, agents)| Box::pin(async move {
        let rows = db.collection::<AssistantConversation>(CONVERSATIONS);
        let Some(mut row) = rows.find_one(doc! {"_id":conversation,"user_id":owner,"active_turn.turn_id":turn,"active_turn.stop_requested":false}).session(&mut *tx).await? else { return Err(AppError::AssistantTurnRequired) };
        if row.guest_turn || row.is_subagent() || engine::live_turn(&row, Utc::now()).is_none() { return Err(AppError::AssistantTurnRequired) }
        // Only consume results for this live coordinator's server-derived run.
        // Ordinary owner waits must preserve unrelated queued automation replies.
        let run_id = row.active_turn.as_ref().and_then(|t| t.trigger_run_id.as_deref());
        let events: Vec<_> = row.pending_events.iter().filter(|e| e.kind == "subagent_settled" && e.trigger_run_id.as_deref() == run_id && e.agent_id.as_ref().is_some_and(|id| agents.contains(id))).cloned().collect();
        adopt_in_session(db, tx, &mut row, &events).await?;
        let mut results = Vec::new();
        for event in &events {
            if let Some(edge) = &event.delegated_result {
                let result: AppResult<Option<AssistantMessage>> = async {
                    if !receiver_valid(db, tx, &row, edge).await? { return Ok(None) }
                    source_result(db, tx, edge).await
                }.await;
                let message = match result { Ok(message) => message, Err(error) if unavailable(&error) => None, Err(error) => return Err(error) };
                results.push((edge.source_agent.clone(), message.map(|m| team::ReplySummary {seq:m.seq,status:m.status,text:engine::excerpt(&m.text,team::REPLY_EXCERPT_CHARS),created_at:m.created_at})));
            }
        }
        let ids: Vec<_> = events.iter().map(|e| e.id.clone()).collect();
        let refs = row.active_turn.as_ref().unwrap().delegated_images.references.clone();
        rows.update_one(doc! {"_id":conversation,"user_id":owner,"active_turn.turn_id":turn,"active_turn.stop_requested":false},doc! {
            "$set":{"active_turn.delegated_images.references":bson::to_bson(&refs).map_err(|_|AppError::Internal("Invalid delegated image metadata".into()))?},
            "$pull":{"pending_events":{"id":{"$in":ids}}},
        }).session(&mut *tx).await?;
        Ok(WaitResult {references:refs,results})
    })).await
}

/// Check the persisted recipient projection, exact source result, current ACL,
/// reset/Stop fence, retention and destination immediately before a send.
#[allow(clippy::too_many_arguments)]
pub async fn authorize_send(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    reference: &DeliveryReference,
    channel: &str,
    partition: &str,
    chat_id: &str,
) -> AppResult<bool> {
    let edge = &reference.assignment;
    if edge.actor != owner
        || edge.recipient_conversation != conversation
        || reference.recipient_turn != turn
    {
        return Ok(false);
    }
    let Some(row) = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":conversation,"user_id":owner})
        .await?
    else {
        return Ok(false);
    };
    let mut tx = db.client().start_session().await?;
    if !receiver_valid(db, &mut tx, &row, edge).await?
        || !destination_valid(db, &mut tx, &row, edge).await?
    {
        return Ok(false);
    }
    let saved = if let Some(active) = row.active_turn.as_ref().filter(|t| t.turn_id == turn) {
        if active.stop_requested || engine::live_turn(&row, Utc::now()).is_none() {
            return Ok(false);
        }
        active.delegated_images.references.clone()
    } else {
        let Some(message)=db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn,"role":"assistant","status":"completed"}).await? else { return Ok(false) };
        message.delegated_images.references
    };
    if !saved.iter().any(|r| {
        r.assignment == *edge
            && r.recipient_turn == turn
            && r.attachment.id == reference.attachment.id
    }) {
        return Ok(false);
    }
    if !edge
        .destinations
        .iter()
        .any(|o| o.nyxbot_channel_id == channel && o.partition == partition)
        && edge.trigger_chat.as_deref() != Some(chat_id)
    {
        return Ok(false);
    }
    if let Some(origin) = edge
        .destinations
        .iter()
        .find(|o| o.nyxbot_channel_id == channel && o.partition == partition)
        && super::channel_turn_delivery::target(db, &row, origin)
            .await?
            .is_none()
    {
        return Ok(false);
    }
    let Some(message) = source_result(db, &mut tx, edge).await? else {
        return Ok(false);
    };
    if message.status != "completed"
        || !message.attachments.iter().any(|a| {
            engine::chat_tool_image(a)
                && a.id == reference.attachment.id
                && a.content_type == reference.attachment.content_type
                && a.size == reference.attachment.size
        })
    {
        return Ok(false);
    }
    super::assistant_upload_retention::require_available(db, doc! {"_id":&reference.attachment.id,"user_id":owner,"conversation_id":&edge.source_conversation,"turn_id":&edge.source_turn,"origin":"tool"}).await?;
    Ok(true)
}

#[cfg(test)]
pub async fn settled_references(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<Vec<DeliveryReference>> {
    #[derive(serde::Deserialize)]
    struct Projection {
        #[serde(default)]
        delegated_images: crate::models::assistant_delegated_image::DelegatedImages,
    }
    Ok(db.collection::<Projection>(MESSAGES).find_one(doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn,"role":"assistant","status":"completed"})
        .projection(doc! {"delegated_images":1}).await?.map(|m|m.delegated_images.references.into_iter().take(MAX_REFERENCES).collect()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_business_refusals_degrade_but_database_and_transport_errors_abort() {
        assert_eq!(publication_outcome(Ok(true)).unwrap(), None);
        for (result, code) in [
            (Ok(false), "event_recipient_unavailable"),
            (
                Err(AppError::Conflict("Assistant event queue is full".into())),
                "event_queue_full",
            ),
            (
                Err(AppError::Conflict(
                    "Assistant event identity changed".into(),
                )),
                "event_identity_conflict",
            ),
            (
                Err(AppError::AssistantTurnRequired),
                "event_invocation_unavailable",
            ),
        ] {
            assert_eq!(publication_outcome(result).unwrap(), Some(code));
        }
        assert!(matches!(
            publication_outcome(Err(AppError::DatabaseError(mongodb::error::Error::custom(
                "unavailable"
            )))),
            Err(AppError::DatabaseError(_))
        ));
        assert!(matches!(
            publication_outcome(Err(AppError::PoolAttemptTransport(
                crate::errors::PoolAttemptTransportKind::Connect
            ))),
            Err(AppError::PoolAttemptTransport(_))
        ));
        assert!(matches!(
            publication_outcome(Err(AppError::Internal("encoding failed".into()))),
            Err(AppError::Internal(_))
        ));
    }
}
