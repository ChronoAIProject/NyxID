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
    collections::{HashMap, HashSet},
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
use tokio::sync::{broadcast, mpsc, watch};

use crate::models::{
    assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
    assistant_group::{COLLECTION_NAME as GROUPS, MESSAGES_COLLECTION_NAME as GROUP_MESSAGES},
    channel_bot::COLLECTION_NAME as CHANNEL_BOTS,
    connect_link::COLLECTION_NAME as CONNECT_LINKS,
};

const CHANNEL_THREADS: &str = crate::models::nyxbot_channel::THREADS_COLLECTION_NAME;
const MACHINES: &str = crate::models::node::COLLECTION_NAME;
const MACHINE_DESKTOPS: &str = crate::models::machine_desktop::COLLECTION_NAME;
const MACHINE_SETUPS: &str = crate::models::machine_setup::COLLECTION_NAME;
const CAPACITY: usize = 1024;
/// Per-owner buffer for browser streams.
const OWNER_CAPACITY: usize = 64;
const ORG_QUEUE_CAPACITY: usize = 256;
const ORG_COALESCE_WINDOW: Duration = Duration::from_millis(250);
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
        title_changed: bool,
    },
    /// A group or its transcript changed.
    Group {
        id: String,
        user_id: String,
    },
    /// Organization group identifier routed to a live eligible person.
    OrgGroup {
        id: String,
        user_id: String,
    },
    /// A connect link was written; `status` is its current status.
    ConnectLink {
        id: String,
        user_id: String,
        status: String,
    },
    ChannelThread {
        id: String,
        user_id: String,
        channel_id: String,
        parent_id: Option<String>,
        conversation_id: Option<String>,
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
    MachineUpdate {
        id: String,
        user_id: String,
    },
    MachineSetup {
        id: String,
        user_id: String,
    },
    TriggerCreated {
        user_id: String,
        watch_id: String,
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
            | Self::OrgGroup { user_id, .. }
            | Self::ConnectLink { user_id, .. }
            | Self::ChannelBot { user_id, .. }
            | Self::ChannelThread { user_id, .. }
            | Self::Machine { user_id, .. }
            | Self::MachineUpdate { user_id, .. }
            | Self::MachineSetup { user_id, .. }
            | Self::MachineDesktop { user_id, .. }
            | Self::TriggerCreated { user_id, .. } => Some(user_id),
            Self::Resync => None,
        }
    }
}

/// Fan-out of live events: every event to in-process workers, and each
/// owner's events (plus resyncs) to that owner's browser streams only.
#[derive(Clone, Debug)]
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

    pub(crate) async fn publish_resolved(&self, db: &Database, event: LiveEvent) {
        let LiveEvent::OrgGroup { id, .. } = &event else {
            self.publish(event);
            return;
        };
        let result: crate::errors::AppResult<()> = async {
            let Some(group) = db
                .collection::<crate::models::assistant_group::AssistantGroup>(GROUPS)
                .find_one(mongodb::bson::doc! {"_id":id})
                .await?
            else {
                return Ok(());
            };
            if !super::org_group_service::is_org(&group) {
                return Ok(());
            }
            for actor in &group.participant_user_ids {
                if super::org_group_service::authorize(db, actor, group.clone(), None)
                    .await
                    .is_ok()
                {
                    self.publish(LiveEvent::OrgGroup {
                        id: id.clone(),
                        user_id: actor.clone(),
                    });
                }
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            tracing::debug!("Organization group live delivery deferred");
        }
    }

    /// Never wait for organization reads in the replica-wide stream. A full
    /// queue drops an identifier; browser polling and the worker sweep repair
    /// missed notifications. Personal events always take the direct path.
    fn route(&self, org_groups: &mpsc::Sender<String>, event: LiveEvent) {
        if let LiveEvent::OrgGroup { id, .. } = event {
            let _ = org_groups.try_send(id);
        } else {
            self.publish(event);
        }
    }

    /// Follow the database's change stream until the process exits,
    /// reopening it (resuming where possible) after any error.
    pub async fn run(&self, db: Database) {
        let (org_groups, pending) = mpsc::channel(ORG_QUEUE_CAPACITY);
        // JoinSet aborts the sole worker when run is cancelled (including tests).
        let mut workers = tokio::task::JoinSet::new();
        let live = self.clone();
        let worker_db = db.clone();
        workers.spawn(async move {
            org_group_worker(pending, |id| {
                live.publish_resolved(
                    &worker_db,
                    LiveEvent::OrgGroup {
                        id,
                        user_id: String::new(),
                    },
                )
            })
            .await;
        });
        let mut resume: Option<ResumeToken> = None;
        let mut backoff = 1;
        loop {
            let mut opened = false;
            let outcome = self
                .follow(&db, &org_groups, &mut resume, &mut opened)
                .await;
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
        org_groups: &mpsc::Sender<String>,
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
                self.route(org_groups, event);
            }
        }
        Ok(())
    }
}

/// One bounded worker, with one eligibility resolution per distinct group in
/// each batch. Even a continuously busy group cannot extend the coalescing
/// window or grow the batch beyond the queue capacity plus the first item.
async fn org_group_worker<F, Fut>(mut pending: mpsc::Receiver<String>, mut resolve: F)
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    while let Some(first) = pending.recv().await {
        tokio::time::sleep(ORG_COALESCE_WINDOW).await;
        let mut groups = HashSet::from([first]);
        for _ in 0..ORG_QUEUE_CAPACITY {
            let Ok(id) = pending.try_recv() else { break };
            groups.insert(id);
        }
        for id in groups {
            resolve(id).await;
        }
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
                {"ns.coll": CHANNEL_THREADS,"$or":[
                    {"operationType":{"$in":["insert","replace"]}},
                    {"updateDescription.updatedFields.follow_state":{"$exists":true}},
                    {"updateDescription.updatedFields.follow_revision":{"$exists":true}},
                    {"updateDescription.updatedFields.follow_expires_at":{"$exists":true}},
                    {"updateDescription.updatedFields.context_status":{"$exists":true}},
                    {"updateDescription.updatedFields.follow_busy_count":{"$exists":true}},
                    {"updateDescription.updatedFields.follow_drop_count":{"$exists":true}},
                    {"updateDescription.updatedFields.threads":{"$exists":true}},
                    {"updateDescription.updatedFields.members":{"$exists":true}},
                    {"updateDescription.updatedFields.agent_id":{"$exists":true}},
                ]},
                {"ns.coll": crate::models::approval_request::COLLECTION_NAME, "fullDocument.assistant_group.group_id": {"$type":"string"}},
                {"ns.coll": crate::models::machine_update::COLLECTION_NAME, "fullDocument.attempt_id": {"$type":"string"}},
                // Replacing an already-online socket can leave status unchanged.
                // Update watches still need the authenticated reconnect event.
                {"ns.coll": MACHINES, "updateDescription.updatedFields.connected_at": {"$exists": true}},
                {"ns.coll": crate::models::trigger::COLLECTION_NAME, "operationType": "insert", "fullDocument.setup_watch_id": {"$type":"string"}},
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
            "fullDocument.created_by_user_id": 1, "fullDocument.org_group": 1,
            "fullDocument.group_request_id": 1,
            "fullDocument.assistant_group.group_id": 1,
            "fullDocument.status": 1, "fullDocument.is_active": 1,
            "fullDocument.setup_watch_id": 1,
            "fullDocument.channel_id":1,"fullDocument.parent_chat_id":1,"fullDocument.conversation_id":1,

            "fullDocument.attempt_id": 1, "fullDocument.requested_by": 1,
            "fullDocument.active_turn.turn_id": 1, "fullDocument.message_count": 1,
            // Signal metadata invalidation without projecting the title text.
            "fullDocument.title_changed": {"$ne": [{"$type": "$updateDescription.updatedFields.title"}, "missing"]},
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
    if collection == crate::models::approval_request::COLLECTION_NAME {
        return Some(LiveEvent::OrgGroup {
            id: full
                .get_document("assistant_group")
                .ok()?
                .get_str("group_id")
                .ok()?
                .into(),
            user_id,
        });
    }
    if (collection == GROUPS && full.get_str("created_by_user_id").is_ok())
        || (collection == GROUP_MESSAGES && full.get_bool("org_group").unwrap_or(false))
        || (collection == CONVERSATIONS && full.get_str("group_request_id").is_ok())
    {
        return Some(LiveEvent::OrgGroup {
            id: if collection == GROUPS {
                key
            } else {
                full.get_str("group_id").ok()?.into()
            },
            user_id,
        });
    }
    Some(match collection {
        CONVERSATIONS => LiveEvent::Conversation {
            title_changed: full.get_bool("title_changed").unwrap_or(false),
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
        crate::models::machine_update::COLLECTION_NAME => LiveEvent::MachineUpdate {
            id: full.get_str("attempt_id").ok()?.into(),
            user_id: full.get_str("requested_by").unwrap_or(&user_id).into(),
        },
        MACHINES => LiveEvent::Machine { id: key, user_id },
        MACHINE_DESKTOPS => LiveEvent::MachineDesktop { id: key, user_id },
        MACHINE_SETUPS => LiveEvent::MachineSetup { id: key, user_id },
        crate::models::trigger::COLLECTION_NAME => LiveEvent::TriggerCreated {
            user_id,
            watch_id: full.get_str("setup_watch_id").ok()?.into(),
        },
        CONNECT_LINKS => LiveEvent::ConnectLink {
            id: key,
            user_id,
            status: full.get_str("status").unwrap_or_default().to_owned(),
        },
        CHANNEL_THREADS => LiveEvent::ChannelThread {
            id: key,
            user_id,
            channel_id: full.get_str("channel_id").ok()?.into(),
            parent_id: full.get_str("parent_chat_id").ok().map(str::to_owned),
            conversation_id: full.get_str("conversation_id").ok().map(str::to_owned),
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
    async fn slow_org_resolution_does_not_delay_personal_conversation_events() {
        let live = AssistantLive::new();
        let mut owner = live.subscribe_owner("person").unwrap();
        let (queue, pending) = mpsc::channel(ORG_QUEUE_CAPACITY);
        let (started, mut resolutions) = mpsc::unbounded_channel();
        let release = Arc::new(tokio::sync::Notify::new());
        let blocker = release.clone();
        let worker = tokio::spawn(org_group_worker(pending, move |id| {
            let started = started.clone();
            let blocker = blocker.clone();
            async move {
                started.send(id).unwrap();
                blocker.notified().await;
            }
        }));
        // Saturating the queue neither waits nor schedules unbounded work.
        for _ in 0..ORG_QUEUE_CAPACITY * 2 {
            live.route(
                &queue,
                LiveEvent::OrgGroup {
                    id: "group".into(),
                    user_id: "org".into(),
                },
            );
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), resolutions.recv())
                .await
                .unwrap()
                .as_deref(),
            Some("group")
        );
        let personal = LiveEvent::Conversation {
            id: "thread".into(),
            user_id: "person".into(),
            group_id: None,
            turn_id: None,
            messages: 1,
            title_changed: false,
        };
        live.route(&queue, personal.clone());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), owner.events.recv())
                .await
                .unwrap()
                .unwrap(),
            personal
        );
        assert!(resolutions.try_recv().is_err());
        drop(queue);
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap();
        assert!(
            resolutions.recv().await.is_none(),
            "all duplicate group ids coalesced into one resolution"
        );
    }

    #[test]
    fn org_group_notice_routing_uses_the_server_marker_not_the_author() {
        for author in [None, Some("person")] {
            let mut full = doc! {"user_id":"org", "group_id":"group", "org_group":true};
            if let Some(author) = author {
                full.insert("author_user_id", author);
            }
            let change: ChangeStreamEvent<Document> = mongodb::bson::from_document(doc! {
                "_id":{"_data":"token"}, "operationType":"insert", "ns":{"db":"test","coll":GROUP_MESSAGES},
                "documentKey":{"_id":"notice"}, "fullDocument":full,
            }).unwrap();
            assert_eq!(
                decode(&change),
                Some(LiveEvent::OrgGroup {
                    id: "group".into(),
                    user_id: "org".into()
                })
            );
        }
        let projection = pipeline().pop().unwrap();
        assert_eq!(
            projection
                .get_document("$project")
                .unwrap()
                .get_i32("fullDocument.org_group"),
            Ok(1)
        );
        assert!(
            !projection
                .get_document("$project")
                .unwrap()
                .contains_key("fullDocument.author_user_id")
        );
    }

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
            title_changed: false,
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
        conversations
            .update_one(
                doc! {"_id": "nyxa-1"},
                doc! {"$set": {"title": "New private title"}},
            )
            .await
            .unwrap();
        let changed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(
                    event @ LiveEvent::Conversation {
                        title_changed: true,
                        ..
                    },
                ) = events.recv().await
                {
                    return event;
                }
            }
        })
        .await
        .unwrap();
        assert!(!format!("{changed:?}").contains("private title"));

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
