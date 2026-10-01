//! Live change notifications for the assistant, from one MongoDB change
//! stream per process (NyxID requires a replica set or mongos, so change
//! streams are always available). Every replica sees every change, so a
//! finished connect link or a new channel bot resumes the waiting chat at
//! once whichever replica handled it, and browsers learn that a thread
//! changed without polling.
//!
//! Events carry identifiers, owner and status only: the stream projects
//! everything else away inside MongoDB, so no transcript, event text or
//! secret ever reaches this process through it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::StreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
    change_stream::event::{ChangeStreamEvent, OperationType, ResumeToken},
    options::FullDocumentType,
};
use tokio::sync::{broadcast, watch};

use crate::models::{
    assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
    assistant_group::{COLLECTION_NAME as GROUPS, MESSAGES_COLLECTION_NAME as GROUP_MESSAGES},
    channel_bot::COLLECTION_NAME as CHANNEL_BOTS,
    connect_link::COLLECTION_NAME as CONNECT_LINKS,
};

const MACHINES: &str = crate::models::node::COLLECTION_NAME;
const MACHINE_DESKTOPS: &str = crate::models::machine_desktop::COLLECTION_NAME;
const MACHINE_SETUPS: &str = crate::models::machine_setup::COLLECTION_NAME;
const CAPACITY: usize = 1024;
/// Per-owner buffer for browser streams.
const OWNER_CAPACITY: usize = 64;
/// Live browser streams one owner may hold open at once (tabs, devices).
pub const MAX_STREAMS_PER_OWNER: usize = 8;
const MAX_BACKOFF_SECS: u64 = 30;

/// One change the assistant cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// A conversation (thread) changed: a turn started or settled, events
    /// were queued, it was renamed. `group_id` is set for hidden member
    /// threads of a group.
    Conversation {
        id: String,
        user_id: String,
        group_id: Option<String>,
        /// The running turn, if any, and the message count: views refresh
        /// thread lists only when these change, not on every activity write.
        turn_id: Option<String>,
        messages: i64,
    },
    /// A group or its transcript changed.
    Group {
        id: String,
        user_id: String,
    },
    /// A connect link was written; `status` is its current status.
    ConnectLink {
        id: String,
        user_id: String,
        status: String,
    },
    /// A channel bot was created or changed.
    ChannelBot {
        id: String,
        user_id: String,
        active: bool,
    },
    /// Metadata-only machine capability/setup change.
    Machine {
        id: String,
        user_id: String,
    },
    MachineDesktop {
        id: String,
        user_id: String,
    },
    MachineSetup {
        id: String,
        user_id: String,
    },
    /// Changes may have been missed (the stream restarted or a receiver
    /// fell behind): re-read state instead of trusting the event history.
    Resync,
}

impl LiveEvent {
    /// The owner the event concerns; `None` for `Resync`.
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::Conversation { user_id, .. }
            | Self::Group { user_id, .. }
            | Self::ConnectLink { user_id, .. }
            | Self::ChannelBot { user_id, .. }
            | Self::Machine { user_id, .. }
            | Self::MachineSetup { user_id, .. }
            | Self::MachineDesktop { user_id, .. } => Some(user_id),
            Self::Resync => None,
        }
    }
}

/// Fan-out of live events: every event to in-process workers, and each
/// owner's events (plus resyncs) to that owner's browser streams only.
#[derive(Debug)]
pub struct AssistantLive {
    sender: broadcast::Sender<LiveEvent>,
    owners: Arc<Mutex<HashMap<String, Owner>>>,
    /// Whether this process's change stream is open right now. Browsers are
    /// only promised live updates (and slow their polls) while it is.
    open: watch::Sender<bool>,
}

#[derive(Debug)]
struct Owner {
    sender: broadcast::Sender<LiveEvent>,
    streams: usize,
}

/// One browser stream's subscription; releases its slot when dropped.
#[derive(Debug)]
pub struct OwnerStream {
    pub events: broadcast::Receiver<LiveEvent>,
    owners: Arc<Mutex<HashMap<String, Owner>>>,
    user_id: String,
}

impl Drop for OwnerStream {
    fn drop(&mut self) {
        let mut owners = self
            .owners
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(owner) = owners.get_mut(&self.user_id) {
            owner.streams = owner.streams.saturating_sub(1);
            if owner.streams == 0 {
                owners.remove(&self.user_id);
            }
        }
    }
}

impl Default for AssistantLive {
    fn default() -> Self {
        Self::new()
    }
}

impl AssistantLive {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CAPACITY);
        let (open, _) = watch::channel(false);
        Self {
            sender,
            owners: Arc::default(),
            open,
        }
    }

    /// True while the change stream is delivering.
    pub fn is_open(&self) -> bool {
        *self.open.borrow()
    }

    /// Follows the open state (a browser stream ends when it closes).
    pub fn watch_open(&self) -> watch::Receiver<bool> {
        self.open.subscribe()
    }

    /// Test hook: mark the stream open without MongoDB.
    #[cfg(test)]
    pub fn set_open_for_tests(&self, open: bool) {
        self.open.send_replace(open);
    }

    /// Every event, for in-process workers.
    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.sender.subscribe()
    }

    /// One owner's events for a browser stream, or `None` when the owner
    /// already holds `MAX_STREAMS_PER_OWNER` streams.
    pub fn subscribe_owner(&self, user_id: &str) -> Option<OwnerStream> {
        let mut owners = self
            .owners
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let owner = owners.entry(user_id.to_owned()).or_insert_with(|| Owner {
            sender: broadcast::channel(OWNER_CAPACITY).0,
            streams: 0,
        });
        if owner.streams >= MAX_STREAMS_PER_OWNER {
            return None;
        }
        owner.streams += 1;
        Some(OwnerStream {
            events: owner.sender.subscribe(),
            owners: self.owners.clone(),
            user_id: user_id.to_owned(),
        })
    }

    /// Deliver an event to subscribers (the change stream does this; tests
    /// may too). Owners' browser streams get only their own events, and
    /// every stream gets resyncs.
    pub fn publish(&self, event: LiveEvent) {
        {
            let owners = self
                .owners
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            match event.user_id() {
                Some(user_id) => {
                    if let Some(owner) = owners.get(user_id) {
                        let _ = owner.sender.send(event.clone());
                    }
                }
                None => {
                    for owner in owners.values() {
                        let _ = owner.sender.send(event.clone());
                    }
                }
            }
        }
        let _ = self.sender.send(event);
    }

    /// Follow the database's change stream until the process exits,
    /// reopening it (resuming where possible) after any error.
    pub async fn run(&self, db: Database) {
        let mut resume: Option<ResumeToken> = None;
        let mut backoff = 1;
        loop {
            let mut opened = false;
            let outcome = self.follow(&db, &mut resume, &mut opened).await;
            self.open.send_replace(false);
            if opened {
                // It was delivering: reopen quickly, resuming where it was.
                backoff = 1;
            }
            if let Err(error) = outcome {
                tracing::warn!(%error, "assistant change stream interrupted; reopening");
                if !opened {
                    // Could not even open with this token (e.g. the oplog no
                    // longer holds it): start fresh; the resync on reopen
                    // makes consumers re-read what they missed.
                    resume = None;
                }
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
        }
    }

    async fn follow(
        &self,
        db: &Database,
        resume: &mut Option<ResumeToken>,
        opened: &mut bool,
    ) -> mongodb::error::Result<()> {
        let mut stream = db
            .watch()
            .pipeline(pipeline())
            .full_document(FullDocumentType::UpdateLookup)
            .resume_after(resume.clone())
            .await?;
        *opened = true;
        self.open.send_replace(true);
        tracing::info!("assistant change stream open");
        // Anything written while the stream was closed is re-read now that
        // it delivers again.
        self.publish(LiveEvent::Resync);
        while let Some(change) = stream.next().await {
            let change = change?;
            *resume = Some(change.id.clone());
            if let Some(event) = decode(&change) {
                self.publish(event);
            }
        }
        Ok(())
    }
}

/// Only the collections and fields the assistant needs; everything else is
/// dropped inside MongoDB.
fn pipeline() -> Vec<Document> {
    vec![
        doc! {"$match": {
            "operationType": {"$in": ["insert", "update", "replace"]},
            "$or": [
                {"ns.coll": {"$in": [CONVERSATIONS, GROUPS, GROUP_MESSAGES]}},
                // Links and bots matter only when created or when their
                // status or activation changes, not on every bookkeeping write.
                {"ns.coll": {"$in": [CONNECT_LINKS, CHANNEL_BOTS, MACHINE_SETUPS, MACHINES, MACHINE_DESKTOPS]}, "$or": [
                    {"operationType": {"$in": ["insert", "replace"]}},
                    {"updateDescription.updatedFields.status": {"$exists": true}},
                    {"updateDescription.updatedFields.is_active": {"$exists": true}},
                    {"updateDescription.updatedFields.machine": {"$exists": true}},
                ]},
            ],
        }},
        doc! {"$project": {
            "operationType": 1, "ns": 1, "documentKey": 1,
            "fullDocument.user_id": 1, "fullDocument.group_id": 1,
            "fullDocument.status": 1, "fullDocument.is_active": 1,
            "fullDocument.active_turn.turn_id": 1, "fullDocument.message_count": 1,
        }},
    ]
}

fn decode(change: &ChangeStreamEvent<Document>) -> Option<LiveEvent> {
    if !matches!(
        change.operation_type,
        OperationType::Insert | OperationType::Update | OperationType::Replace
    ) {
        return None;
    }
    let collection = change.ns.as_ref()?.coll.as_deref()?;
    let full = change.full_document.as_ref()?;
    let user_id = full.get_str("user_id").ok()?.to_owned();
    let key = change
        .document_key
        .as_ref()?
        .get_str("_id")
        .ok()?
        .to_owned();
    Some(match collection {
        CONVERSATIONS => LiveEvent::Conversation {
            id: key,
            user_id,
            group_id: full.get_str("group_id").ok().map(str::to_owned),
            turn_id: full
                .get_document("active_turn")
                .ok()
                .and_then(|turn| turn.get_str("turn_id").ok())
                .map(str::to_owned),
            messages: full
                .get_i64("message_count")
                .or_else(|_| full.get_i32("message_count").map(i64::from))
                .unwrap_or(0),
        },
        GROUPS => LiveEvent::Group { id: key, user_id },
        GROUP_MESSAGES => LiveEvent::Group {
            id: full.get_str("group_id").ok()?.to_owned(),
            user_id,
        },
        MACHINES => LiveEvent::Machine { id: key, user_id },
        MACHINE_DESKTOPS => LiveEvent::MachineDesktop { id: key, user_id },
        MACHINE_SETUPS => LiveEvent::MachineSetup { id: key, user_id },
        CONNECT_LINKS => LiveEvent::ConnectLink {
            id: key,
            user_id,
            status: full.get_str("status").unwrap_or_default().to_owned(),
        },
        CHANNEL_BOTS => LiveEvent::ChannelBot {
            id: key,
            user_id,
            active: full.get_bool("is_active").unwrap_or(false),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn changes_are_published_with_identifiers_only() {
        let db =
            crate::test_utils::connect_transaction_test_database("assistant_live_stream").await;
        let live = std::sync::Arc::new(AssistantLive::new());
        let mut events = live.subscribe();
        let runner = {
            let live = live.clone();
            let db = db.clone();
            tokio::spawn(async move { live.run(db).await })
        };
        // The stream is open once writes start showing up; retry until then.
        let conversations = db.collection::<Document>(CONVERSATIONS);
        let wanted = LiveEvent::Conversation {
            id: "nyxa-1".into(),
            user_id: "owner".into(),
            group_id: None,
            turn_id: Some("turn-1".into()),
            messages: 4,
        };
        let seen = tokio::time::timeout(Duration::from_secs(20), async {
            let mut attempt = 0;
            loop {
                attempt += 1;
                conversations
                    .update_one(
                        doc! {"_id": "nyxa-1"},
                        doc! {"$set": {"user_id": "owner", "title": "Secret plans", "n": attempt,
                        "message_count": 4_i64,
                        "active_turn": {"turn_id": "turn-1", "activities": ["secret"]}}},
                    )
                    .upsert(true)
                    .await
                    .unwrap();
                let deadline = tokio::time::sleep(Duration::from_millis(300));
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        event = events.recv() => match event {
                            Ok(event) if event == wanted => return event,
                            _ => continue,
                        },
                        _ = &mut deadline => break,
                    }
                }
            }
        })
        .await
        .expect("a conversation change is published");
        assert_eq!(seen.user_id(), Some("owner"));
        // Group transcripts report their group; bots report activity.
        db.collection::<Document>(GROUP_MESSAGES)
            .insert_one(doc! {"_id": "m1", "group_id": "nyxg-1", "user_id": "owner", "text": "hi"})
            .await
            .unwrap();
        db.collection::<Document>(CHANNEL_BOTS)
            .insert_one(doc! {"_id": "b1", "user_id": "owner", "is_active": true,
            "bot_token_encrypted": "secret"})
            .await
            .unwrap();
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while rest.len() < 2 {
                if let Ok(event) = events.recv().await
                    && !matches!(event, LiveEvent::Conversation { .. } | LiveEvent::Resync)
                {
                    rest.push(event);
                }
            }
        })
        .await
        .expect("group and bot changes are published");
        assert_eq!(
            rest,
            vec![
                LiveEvent::Group {
                    id: "nyxg-1".into(),
                    user_id: "owner".into()
                },
                LiveEvent::ChannelBot {
                    id: "b1".into(),
                    user_id: "owner".into(),
                    active: true
                },
            ]
        );
        runner.abort();
    }
}
