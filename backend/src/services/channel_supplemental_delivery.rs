//! Source selection for supplemental text. Never changes primary answer ownership.
use bson::{Document, doc};
use chrono::{DateTime, Utc};
use mongodb::Database;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, ChannelOrigin, TurnOrigin},
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        nyxbot_channel::{
            EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotEvent, NyxbotThread,
        },
    },
};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    CheckingNotice,
    CapacityNotice,
    MachineDesktopNotice,
    TeamResult,
    TeamAdditionalResult,
    PermissionResult,
}
impl Purpose {
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
    AuthorityRefused,
    SourceUnavailable,
    ReferenceUnavailable,
    ReferenceExpired,
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
    reference_live(thread)?;
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

pub async fn source(
    db: &Database,
    row: &AssistantConversation,
    origin: &ChannelOrigin,
) -> AppResult<Source> {
    let Some(turn) = &row.active_turn else {
        return Ok(Source::Unbound);
    };
    let mut ids = Vec::new();
    let mut expected =
        turn.origin == TurnOrigin::Channel && turn.asked_from.as_ref() == Some(origin);
    if let Some(id) = &turn.channel_event_id {
        ids.push(id.clone());
    }
    for event in &turn.events {
        if event.reply_to.iter().any(|candidate| candidate == origin)
            && !event.source_event_ids.is_empty()
        {
            expected = true;
            ids.extend(event.source_event_ids.iter().cloned());
        }
    }
    // IDs come from admission, never from the thread's current event reference.
    for id in ids {
        if let Some(event) = db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {
                "_id":id,"channel_id":&origin.nyxbot_channel_id,
                "conversation_id":&row.id,"turn_id":&turn.turn_id,
                "delivery.origin.partition":&origin.partition,
            })
            .await?
        {
            return Ok(Source::Event(Box::new(event)));
        }
    }
    Ok(if expected {
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
            Ok(Some(_)) => return Ok(Ok((id.clone(), Anchor::ChatPointer))),
            Err(reason) => return Ok(Err(reason)),
            Ok(None) => {}
        }
    }
    if purpose.progress() {
        return Ok(Err(Reason::MissingAnchor));
    }
    let id = match reference_anchor(thread) {
        Ok(id) => id,
        Err(reason) => return Ok(Err(reason)),
    };
    Ok(
        match validate_anchor(db, channel, thread, id, None).await? {
            Ok(Some(_)) => Ok((id.to_owned(), Anchor::ChatReferenceEvent)),
            Ok(None) => Err(Reason::MissingAnchor),
            Err(reason) => Err(reason),
        },
    )
}
