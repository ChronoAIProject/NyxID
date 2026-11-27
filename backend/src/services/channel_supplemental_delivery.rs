//! Source selection for supplemental text. Never changes primary answer ownership.
use bson::{Document, doc};
use chrono::{DateTime, Utc};
use mongodb::Database;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{ChannelOrigin, TurnOrigin},
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        nyxbot_channel::{
            EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotEvent, NyxbotThread,
        },
    },
};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    SupervisorReview,
    CheckingNotice,
    CapacityNotice,
    MachineDesktopNotice,
    TeamResult,
    TeamAdditionalResult,
    PermissionResult,
}
impl Purpose {
    pub fn result(self) -> bool {
        matches!(self, Self::TeamResult | Self::TeamAdditionalResult)
    }

    pub fn progress(self) -> bool {
        matches!(
            self,
            Self::CheckingNotice | Self::CapacityNotice | Self::MachineDesktopNotice
        )
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    EventSource,
    ChatPointer,
    ChatReferenceEvent,
    ThreadBinding,
    None,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    MissingAnchor,
    InvalidAnchor,
    StaleReferenceAnchor,
    NativeProgressUnsupported,
    MissingResultIdentity,
    AuthorityRefused,
    SourceUnavailable,
    ReferenceUnavailable,
    ReferenceExpired,
    AnchorExpired,
    AlreadyClaimed,
    EmptyOutput,
    NoDestination,
}

/// Bind both components old writers can replace. This digest is never logged.
pub fn reference_binding(ciphertext: &[u8], expires: DateTime<Utc>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nyxbot-reference-relay-v1\0");
    hash.update(expires.timestamp_millis().to_be_bytes());
    hash.update(ciphertext);
    format!("{:x}", hash.finalize())
}

/// Merge into the same atomic update that installs the reference and its deadline.
pub fn reference_fields(
    ciphertext: Vec<u8>,
    expires: DateTime<Utc>,
    relay: Option<&str>,
) -> Document {
    let binding = relay.map(|_| reference_binding(&ciphertext, expires));
    doc! {
        "event_ref_ciphertext": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: ciphertext },
        "event_ref_expires_at": bson::DateTime::from_chrono(expires),
        "event_ref_relay_message_id": relay,
        "event_ref_relay_binding": binding,
    }
}

pub fn reference_live(thread: &NyxbotThread) -> Result<(), Reason> {
    match (&thread.event_ref_ciphertext, thread.event_ref_expires_at) {
        (Some(_), Some(expires)) if expires > Utc::now() => Ok(()),
        (Some(_), Some(_)) => Err(Reason::ReferenceExpired),
        _ => Err(Reason::ReferenceUnavailable),
    }
}

pub fn reference_anchor(thread: &NyxbotThread) -> Result<&str, Reason> {
    let id = thread
        .event_ref_relay_message_id
        .as_deref()
        .ok_or(Reason::MissingAnchor)?;
    let binding = reference_binding(
        thread
            .event_ref_ciphertext
            .as_deref()
            .ok_or(Reason::ReferenceUnavailable)?,
        thread
            .event_ref_expires_at
            .ok_or(Reason::ReferenceUnavailable)?,
    );
    if thread.event_ref_relay_binding.as_deref() != Some(binding.as_str()) {
        return Err(Reason::StaleReferenceAnchor);
    }
    Ok(id)
}

pub enum Source {
    Event(Box<NyxbotEvent>),
    Unbound,
    Missing,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ResultSource {
    pub turn_id: String,
    pub event_id: Option<String>,
    ids: Vec<String>,
    expected: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SavedDestination {
    pub origin: ChannelOrigin,
    pub source: ResultSource,
    pub additional: bool,
}

impl SavedDestination {
    pub(crate) fn purpose(&self) -> Purpose {
        if self.additional {
            Purpose::TeamAdditionalResult
        } else {
            Purpose::TeamResult
        }
    }
}

#[derive(Deserialize)]
struct SavedHandoffTargets {
    handoff_delivery: SavedHandoffRouting,
}

#[derive(Deserialize)]
struct SavedHandoffRouting {
    targets: Vec<SavedDestination>,
}

/// A pending or leased handoff can still deliver unsent parts. Only a handoff
/// whose delivery worker cleared both fields has completed its delivery work.
pub(crate) async fn finished_handoff_targets(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    reply_id: &str,
) -> AppResult<Vec<SavedDestination>> {
    let mut filter = settled_reply_filter(owner, conversation, turn);
    filter.extend(doc! {
        "_id": reply_id, "handoff_delivery.targets": {"$exists": true},
        "handoff_delivery_due_at": {"$exists": false},
        "handoff_delivery_lease": {"$exists": false},
    });
    Ok(db
        .collection::<SavedHandoffTargets>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(filter)
        .projection(doc! {"handoff_delivery.targets": 1})
        .await?
        .map_or_else(Vec::new, |reply| reply.handoff_delivery.targets))
}

pub(crate) fn managed_source(
    turn: &crate::models::assistant_conversation::ActiveTurn,
) -> Option<&crate::models::assistant_conversation::ChatRestartSource> {
    turn.restart_admission
        .as_deref()
        .and_then(|admission| admission.chat_source.as_deref())
        .or_else(|| {
            turn.execution_attempt
                .as_ref()?
                .foreground_delivery
                .as_deref()
        })
        .filter(|source| {
            !matches!(
                source,
                crate::models::assistant_conversation::ChatRestartSource::PersonalManager { .. }
            )
        })
}

pub(crate) fn saved_destinations(
    row: &crate::models::assistant_conversation::AssistantConversation,
    turn: &crate::models::assistant_conversation::ActiveTurn,
    source: &crate::models::assistant_conversation::ChatRestartSource,
    failed: bool,
) -> Vec<SavedDestination> {
    use crate::models::assistant_conversation::ChatRestartSource;
    if turn
        .execution_task
        .as_ref()
        .and_then(|task| task.channel_source.as_ref())
        .is_some_and(|source| source.original_turn_id != turn.turn_id)
    {
        return Vec::new();
    }
    let primary = match source {
        ChatRestartSource::Wake { reply_channel, .. }
            if !failed
                && turn
                    .events
                    .iter()
                    .all(|event| event.source_event_ids.is_empty()) =>
        {
            reply_channel.as_ref()
        }
        _ => None,
    };
    primary
        .into_iter()
        .map(|origin| (origin, false))
        .chain(
            turn.also_deliver
                .iter()
                .filter(|origin| Some(*origin) != primary)
                .map(|origin| (origin, true)),
        )
        .filter(|(origin, _)| {
            origin.org_execution.is_none() && origin.thread.is_none() && !row.guest_turn
        })
        .map(|(origin, additional)| SavedDestination {
            origin: origin.clone(),
            source: ResultSource::capture(turn, origin),
            additional,
        })
        .collect()
}

impl ResultSource {
    pub(crate) fn supervisor(
        source: &crate::models::execution_permission_channel::ChannelSource,
    ) -> Self {
        Self {
            turn_id: source.original_turn_id.clone(),
            event_id: Some(source.event_id.clone()),
            ids: vec![source.event_id.clone()],
            expected: true,
        }
    }
    pub(crate) fn capture(
        turn: &crate::models::assistant_conversation::ActiveTurn,
        origin: &ChannelOrigin,
    ) -> Self {
        let asked_here = turn.asked_from.as_ref() == Some(origin);
        let mut ids: Vec<_> = turn.channel_event_id.iter().cloned().collect();
        let mut expected = turn.origin == TurnOrigin::Channel && asked_here;
        for event in &turn.events {
            if event.reply_to.contains(origin) && !event.source_event_ids.is_empty() {
                expected = true;
                ids.extend(event.source_event_ids.iter().cloned());
            }
        }
        Self {
            turn_id: turn.turn_id.clone(),
            event_id: origin
                .native_event_id
                .clone()
                .or_else(|| asked_here.then(|| turn.channel_event_id.clone()).flatten()),
            ids,
            expected,
        }
    }
}

pub(crate) async fn source_for(
    db: &Database,
    conversation: &str,
    origin: &ChannelOrigin,
    binding: Option<&ResultSource>,
) -> AppResult<Source> {
    let Some(binding) = binding else {
        return Ok(Source::Unbound);
    };
    // IDs come from admission, never from the thread's current event reference.
    for id in &binding.ids {
        if let Some(event) = db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {
                "_id":id,"channel_id":&origin.nyxbot_channel_id,
                "conversation_id":conversation,"turn_id":&binding.turn_id,
                "delivery.origin.partition":&origin.partition,
            })
            .await?
        {
            return Ok(Source::Event(Box::new(event)));
        }
    }
    Ok(if binding.expected {
        Source::Missing
    } else {
        Source::Unbound
    })
}

/// Read by ID first: a mismatching retained row must never trigger fallback.
pub async fn validate_anchor(
    db: &Database,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    id: &str,
    sender: Option<&str>,
) -> AppResult<Result<Option<ChannelMessage>, Reason>> {
    let Some(message) = db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id":id})
        .await?
    else {
        return Ok(Ok(None));
    };
    let owner = channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id);
    // Followed children are keyed by their root, not platform_thread_id.
    // The bound adapter revalidates this binding and its live generation.
    let same_thread = if let Some(root) = &thread.follow.thread_root_id {
        message.thread_context.as_ref().is_some_and(|facts| {
            facts.root_id.as_ref() == Some(root)
                && Some(facts.kind) == thread.follow.thread_kind
                && Some(&facts.chat_id) == thread.platform_chat_id.as_ref()
        })
    } else {
        super::channel_image_audience::message_thread(&message)
            == super::channel_image_audience::chat_thread(thread)
    };
    let correct = message.direction == "inbound"
        && message.user_id == owner
        && message.channel_bot_id.as_deref() == Some(channel.channel_bot_id.as_str())
        && super::channel_platform::canonical_platform(&message.platform)
            == super::channel_platform::canonical_platform(&channel.platform)
        && message.agent_api_key_id.as_deref() == Some(channel.route_api_key_id.as_str())
        && channel.route_id.as_deref() == Some(message.conversation_id.as_str())
        && message
            .platform_conversation_id
            .as_deref()
            .is_some_and(|chat| {
                !chat.is_empty() && chat != "*" && thread.platform_chat_id.as_deref() == Some(chat)
            })
        && same_thread
        && sender.is_none_or(|sender| message.sender_platform_id.as_deref() == Some(sender));
    if !correct {
        return Ok(Err(Reason::InvalidAnchor));
    }
    Ok(Ok(Some(message)))
}

pub async fn chat_anchor(
    db: &Database,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    purpose: Purpose,
) -> AppResult<Result<(String, Anchor), Reason>> {
    if let Some(id) = &thread.last_relay_message_id {
        match validate_anchor(db, channel, thread, id, None).await? {
            Ok(Some(message)) => {
                return Ok(result_window(purpose, message.created_at)
                    .map(|()| (id.clone(), Anchor::ChatPointer)));
            }
            Err(reason) => return Ok(Err(reason)),
            Ok(None) => {}
        }
    }
    if purpose.progress() {
        return Ok(Err(Reason::MissingAnchor));
    }
    if !purpose.result()
        && let Err(reason) = reference_live(thread)
    {
        return Ok(Err(reason));
    }
    let id = match reference_anchor(thread) {
        Ok(id) => id,
        Err(reason) => return Ok(Err(reason)),
    };
    Ok(
        match validate_anchor(db, channel, thread, id, None).await? {
            Ok(Some(message)) => result_window(purpose, message.created_at)
                .map(|()| (id.to_owned(), Anchor::ChatReferenceEvent)),
            Ok(None) => Err(Reason::MissingAnchor),
            Err(reason) => Err(reason),
        },
    )
}

pub fn result_window(purpose: Purpose, created_at: DateTime<Utc>) -> Result<(), Reason> {
    if purpose.result() && created_at + chrono::Duration::hours(24) <= Utc::now() {
        Err(Reason::AnchorExpired)
    } else {
        Ok(())
    }
}

/// The destination is stable across pointer refreshes and primary/additional routing.
/// Followed children identify the original platform root without using an inbound ID.
fn result_destination(origin: &ChannelOrigin) -> String {
    let binding = origin
        .thread
        .as_ref()
        .map(|b| (&b.child_id, b.generation, b.channel_generation, &b.agent_id));
    destination_hash(
        &origin.nyxbot_channel_id,
        &origin.partition,
        binding.map(|(child, generation, channel_generation, agent)| {
            (
                child.as_str(),
                generation,
                channel_generation,
                agent.as_str(),
            )
        }),
    )
}

fn destination_hash(
    channel: &str,
    partition: &str,
    binding: Option<(&str, i64, i64, &str)>,
) -> String {
    let encoded = serde_json::to_vec(&(channel, partition, binding))
        .expect("destination identifiers serialize");
    let mut hash = Sha256::new();
    hash.update(b"nyxbot-supplemental-result-v1\0");
    hash.update(encoded);
    format!("{:x}", hash.finalize())
}

pub(crate) fn saved_destination_key(
    turn: &str,
    channel: &str,
    partition: &str,
    binding: Option<(&str, i64, i64, &str)>,
) -> String {
    format!(
        "saved-result:{turn}:{}",
        destination_hash(channel, partition, binding)
    )
}

pub(crate) fn saved_key(turn: &str, origin: &ChannelOrigin) -> String {
    format!("saved-result:{turn}:{}", result_destination(origin))
}

/// Hash the local dispatch identity and part, keeping gateway headers printable
/// and independent of the sealed event reference and text.
pub(crate) fn side_message_key<T: Serialize>(identity: &T, part: usize) -> String {
    let encoded = serde_json::to_vec(&(identity, part)).expect("dispatch identifiers serialize");
    let digest = Sha256::digest(encoded);
    format!("nyxbot-side-v1:{digest:x}")
}

tokio::task_local! {pub(crate) static SAVED_RELAY_KEY: String;}

pub(crate) fn settled_reply_filter(owner: &str, conversation: &str, turn: &str) -> Document {
    doc! {
        "user_id": owner, "conversation_id": conversation, "turn_id": turn, "role": "assistant",
        "status": {"$in": ["completed", "failed", "cancelled"]},
    }
}

/// A possibly committed claim never authorizes dispatch. This marker has no retry path.
pub async fn claim_result(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: Option<&str>,
    origin: &ChannelOrigin,
) -> AppResult<Result<(), Reason>> {
    let Some(turn) = turn else {
        return Ok(Err(Reason::SourceUnavailable));
    };
    let messages = db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME);
    let identity = settled_reply_filter(owner, conversation, turn);
    let Some(saved) = messages
        .find_one(identity.clone())
        .projection(doc! {"_id":1})
        .await?
    else {
        return Ok(Err(Reason::SourceUnavailable));
    };
    let destination = result_destination(origin);
    let mut filter = identity;
    filter.insert(
        "_id",
        saved
            .get("_id")
            .cloned()
            .ok_or_else(|| crate::errors::AppError::Internal("Settled reply unavailable".into()))?,
    );
    filter.insert("supplemental_result_claims", doc! {"$ne":&destination});
    let claimed = messages
        .update_one(
            filter,
            doc! {"$addToSet":{"supplemental_result_claims":destination}},
        )
        .await
        // Database validation details can contain the marker; never log those details.
        .map_err(|_| {
            crate::errors::AppError::Internal("Supplemental result claim unavailable".into())
        })?;
    Ok(if claimed.modified_count == 1 {
        Ok(())
    } else {
        Err(Reason::AlreadyClaimed)
    })
}
