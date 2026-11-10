//! NyxBot Notifications: the durable per-person feed behind the bell.
//!
//! Producers write inside the source transition's own MongoDB transaction
//! (turn settlement, trigger terminal, pause notice, card creation and
//! decision, web-group replies, collaboration posts), so a row exists if and
//! only if its source transition committed. Every producer first proves its
//! parent is live by writing it (or by relying on a write the transaction
//! already makes), which serializes it against a concurrent delete, unlink,
//! chat removal or destroy: snapshot reads alone would allow write skew. The
//! erasure fence of each source transaction covers these writes.
//!
//! Rows carry identifiers, kind, counts, timestamps, read/resolved state and
//! push state only. Previews are computed at list time from the referenced
//! source after live ownership checks; rows whose source is gone are purged.
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        assistant_agent::COLLECTION_NAME as AGENTS,
        assistant_conversation::{
            ActiveTurn, AssistantConversation, COLLECTION_NAME as CONVERSATIONS, PlatformThread,
            TurnOrigin,
        },
        assistant_group::{
            AssistantGroup, COLLECTION_NAME as GROUPS, GroupMessage,
            MESSAGES_COLLECTION_NAME as GROUP_MESSAGES,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        assistant_notification::{
            AssistantNotification, COLLECTION_NAME, NotificationKind, NotificationSource,
            PushState, SourceType,
        },
        assistant_settings::NotificationPushPreferences,
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, THREADS_COLLECTION_NAME as CHANNEL_THREADS},
        org_channel::MESSAGES as ORG_CHANNEL_MESSAGES,
        org_membership::COLLECTION_NAME as ORG_MEMBERSHIPS,
        platform_personal_binding::COLLECTION_NAME as PLATFORM_BINDINGS,
        trigger::COLLECTION_NAME as TRIGGERS,
        trigger_run::{COLLECTION_NAME as RUNS, RunOutcome, TriggerRun},
        trigger_schedule::DeliverTo,
        user::COLLECTION_NAME as USERS,
    },
    services::{
        api_key_mutation_service as transactions,
        assistant_links::{AGENT_DETAILS_CHANNELS, AssistantPage},
        assistant_nyxagent::TurnError,
        feature_flag_service,
        notification_service::automation,
    },
};

pub const FLAG_KEY: &str = "assistant:notifications";
/// Durable per-person rolling push budget (`_id` = user id).
pub const PUSH_BUDGETS: &str = "assistant_notification_push_budgets";
/// Durable per-source push slots (`_id` = person + visible source,
/// `user_id` for erasure), TTL one hour.
pub const PUSH_SOURCES: &str = "assistant_notification_push_sources";
/// Channel statuses whose feed is visible. Failed and disconnected channels
/// hide their rows until repaired; actual retirement purges the rows.
pub const LIVE_CHANNEL_STATUSES: [&str; 2] = ["pending", "active"];
pub const LANE_ORG_GUEST: &str = "org_guest";
pub const LANE_ORG_MEMBER: &str = "org_member";
/// Rows expire this long after their last activity.
pub const RETENTION_DAYS: i64 = 30;
pub const MAX_LIST_LIMIT: usize = 50;
pub const DEFAULT_LIST_LIMIT: usize = 20;
pub const MAX_READ_IDS: usize = 100;
/// Single-line preview length in Unicode scalars.
pub const PREVIEW_CHARS: usize = 200;
/// Bounded per-source aggregate on the summary.
pub const SUMMARY_SOURCES: usize = 200;
/// Per-process flag cache: admin flips apply within this window.
const FLAG_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
const FLAG_CACHE_CAPACITY: usize = 4096;
/// Push: at most one push per visible source in this window and this many
/// per rolling hour.
pub const PUSH_SOURCE_INTERVAL_MINUTES: i64 = 10;
pub const PUSH_HOURLY_LIMIT: usize = 30;
/// A `sending` claim older than this is ambiguous: it becomes `unknown`
/// and is never resent.
const PUSH_SENDING_STALE_MINUTES: i64 = 10;
/// Conclusive retryable refusals (429/503) retry at least this far apart, at
/// most this many attempts, within this window after the first attempt.
pub const PUSH_MAX_ATTEMPTS: u32 = 3;
pub const PUSH_RETRY_MIN_SECONDS: i64 = 60;
pub const PUSH_RETRY_WINDOW_MINUTES: i64 = 30;
const PUSH_BATCH: i64 = 64;
const PUSH_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Skipped run reasons the owner can act on (D3), taken from the scheduler's
/// real terminal reasons: `grace_expired` is what a run deferred by budget,
/// capacity or a busy thread settles as when its window closes. Other skips
/// (`overlap*`, `trigger_paused`, `trigger_deleted`, `missed_window`) are
/// expected or owner-caused and produce nothing.
const ACTIONABLE_SKIP_REASONS: &[&str] = &[
    "grace_expired",
    "queue_full",
    "owner_unavailable",
    "target_unavailable",
    "assistant_disabled",
];

/// Stored field names for purge and resolution filters.
pub mod fields {
    pub const USER_ID: &str = "user_id";
    pub const KIND: &str = "kind";
    pub const SOURCE_TYPE: &str = "source.type";
    pub const SOURCE_ID: &str = "source.id";
    pub const SOURCE_CONVERSATION_ID: &str = "source.conversation_id";
    pub const SOURCE_GROUP_ID: &str = "source.group_id";
    pub const SOURCE_CHANNEL_ID: &str = "source.channel_id";
    pub const SOURCE_THREAD_ID: &str = "source.thread_id";
    pub const SOURCE_TRIGGER_ID: &str = "source.trigger_id";
    pub const SOURCE_AGENT_ID: &str = "source.agent_id";
    pub const SOURCE_RUN_ID: &str = "source.run_id";
    pub const SOURCE_CARD_ID: &str = "source.card_id";
    pub const SOURCE_API_KEY_ID: &str = "source.api_key_id";
    /// Linked platform chats: unlink purges every row of the binding.
    pub const SOURCE_PLATFORM_BINDING_ID: &str = "source.platform_binding_id";
}

// ---------------------------------------------------------------------------
// Feature flag
// ---------------------------------------------------------------------------

fn flag_cache() -> &'static Mutex<HashMap<String, (Instant, bool)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, bool)>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn flag_cache_key(db: &Database, user_id: &str) -> String {
    // Isolated test databases and tenants model independent deployments in
    // one process: never share a person's flag across databases.
    format!("{}\u{1f}{user_id}", db.name())
}

/// Whether the feed is enabled for `user_id`, with a bounded per-process
/// cache so producers without an admission snapshot add no database reads
/// on hot paths. Admin flag changes apply within [`FLAG_CACHE_TTL`].
pub async fn enabled(db: &Database, user_id: &str) -> AppResult<bool> {
    let key = flag_cache_key(db, user_id);
    if let Some(value) = flag_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
        .filter(|(at, _)| at.elapsed() < FLAG_CACHE_TTL)
        .map(|(_, value)| *value)
    {
        return Ok(value);
    }
    let value = feature_flag_service::personal_flag_enabled(db, user_id, FLAG_KEY).await?;
    let mut cache = flag_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= FLAG_CACHE_CAPACITY {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), value));
    Ok(value)
}

/// The same decision from an already-resolved feature snapshot (HTTP
/// handlers reuse the engine's `require_enabled_features` result).
pub fn enabled_in(features: &[String]) -> bool {
    features.iter().any(|key| key == FLAG_KEY)
}

/// Seed the cache from a feature snapshot the caller already resolved for
/// `user_id` (the engine gate), so the admission that follows reads nothing
/// and agrees with the gate even when an older cached decision differs.
pub fn cache_features(db: &Database, user_id: &str, features: &[String]) {
    let key = flag_cache_key(db, user_id);
    let mut cache = flag_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= FLAG_CACHE_CAPACITY && !cache.contains_key(&key) {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), enabled_in(features)));
}

/// Forget this database's cached decisions (tests flip the flag mid-run;
/// each test owns a database, so parallel tests never evict each other).
#[cfg(test)]
pub fn reset_flag_cache(db: &Database) {
    let prefix = format!("{}\u{1f}", db.name());
    flag_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|key, _| !key.starts_with(&prefix));
}

// ---------------------------------------------------------------------------
// Indexes
// ---------------------------------------------------------------------------

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let collection = db.collection::<Document>(COLLECTION_NAME);
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "updated_at": -1, "_id": -1})
                .build(),
        )
        .await?;
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "unread": 1, "resolved_at": 1, "kind": 1})
                .build(),
        )
        .await?;
    // Coalescing: at most one unread row per person and source.
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "source_key": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"coalesced": true, "unread": true})
                        .build(),
                )
                .build(),
        )
        .await?;
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"kind": 1, "resolved_at": 1, "card_expires_at": 1})
                .build(),
        )
        .await?;
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"push.state": 1, "created_at": 1})
                .options(
                    IndexOptions::builder()
                        .partial_filter_expression(
                            doc! {"push.state": {"$in": ["pending", "sending", "retrying"]}},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    for field in [
        fields::SOURCE_CONVERSATION_ID,
        fields::SOURCE_GROUP_ID,
        fields::SOURCE_CHANNEL_ID,
        fields::SOURCE_THREAD_ID,
        fields::SOURCE_TRIGGER_ID,
        fields::SOURCE_AGENT_ID,
        fields::SOURCE_RUN_ID,
        fields::SOURCE_CARD_ID,
        fields::SOURCE_API_KEY_ID,
        fields::SOURCE_PLATFORM_BINDING_ID,
    ] {
        collection
            .create_index(
                IndexModel::builder()
                    .keys(doc! {field: 1})
                    .options(
                        IndexOptions::builder()
                            .partial_filter_expression(doc! {field: {"$type": "string"}})
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(PUSH_SOURCES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"sent_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(3600))
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(PUSH_SOURCES)
        .create_index(IndexModel::builder().keys(doc! {"user_id": 1}).build())
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Producers
// ---------------------------------------------------------------------------

fn bson_date(at: DateTime<Utc>) -> bson::DateTime {
    bson::DateTime::from_chrono(at)
}

fn deterministic_id(scope: &str, id: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("nyxid:notification:{scope}:{id}").as_bytes(),
    )
    .to_string()
}

/// One visible source coalesces across chat kinds (D5): a reply and a
/// mention in the same group share the unread row; the latest event's kind
/// is kept on the row.
pub fn source_key(source: &NotificationSource) -> String {
    format!("{}:{}", source.source_type.as_str(), source.id)
}

enum PushPlan {
    Pending,
    Skipped(&'static str),
}

struct NewRow<'a> {
    user_id: &'a str,
    kind: NotificationKind,
    source: NotificationSource,
    /// Deterministic identity for one-per-run/card rows.
    id: Option<String>,
    outcome: Option<String>,
    reason: Option<String>,
    card_expires_at: Option<DateTime<Utc>>,
    push: PushPlan,
}

/// The push barrier at creation. `kind` freezes the creating kind: a later
/// coalesce may relabel the row, but the person is pushed for what they
/// opted into, and the payload is built for that kind.
fn push_state(plan: &PushPlan, kind: NotificationKind) -> PushState {
    let (state, code) = match plan {
        PushPlan::Pending => ("pending", None),
        PushPlan::Skipped(code) => ("skipped", Some((*code).to_owned())),
    };
    PushState {
        state: state.into(),
        kind: Some(kind),
        attempt: None,
        code,
        attempts: 0,
        claimed_at: None,
        first_attempt_at: None,
        retry_at: None,
        sent_at: None,
    }
}

/// Record one row with exactly one row write inside the caller's
/// transaction. Coalesced kinds upsert on the unique partial index
/// (person, visible source, unread): a concurrent producer conflicts
/// transiently and retries, never duplicates. Deterministic rows are an
/// insert-if-absent upsert, so retries and recovery never duplicate.
async fn record_in_session(
    db: &Database,
    session: &mut ClientSession,
    new: NewRow<'_>,
) -> AppResult<()> {
    let now = Utc::now();
    let mut expires_at = now + Duration::days(RETENTION_DAYS);
    if let Some(card_expires_at) = new.card_expires_at {
        // A pending decision never expires before its card does.
        expires_at = expires_at.max(card_expires_at);
    }
    let key = source_key(&new.source);
    let coalesced = new.kind.coalesced();
    let row = AssistantNotification {
        id: new.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
        user_id: new.user_id.to_owned(),
        kind: new.kind,
        source: new.source,
        source_key: key.clone(),
        coalesced,
        unread: true,
        read_at: None,
        resolved_at: None,
        resolution: None,
        card_expires_at: new.card_expires_at,
        count: 1,
        outcome: new.outcome,
        reason: new.reason,
        push: Some(push_state(&new.push, new.kind)),
        created_at: now,
        updated_at: now,
        expires_at,
    };
    let mut document = bson::to_document(&row)
        .map_err(|_| AppError::Internal("Notification serialization failed".into()))?;
    let collection = db.collection::<Document>(COLLECTION_NAME);
    if !coalesced {
        collection
            .update_one(doc! {"_id": &row.id}, doc! {"$setOnInsert": document})
            .upsert(true)
            .session(&mut *session)
            .await?;
        return Ok(());
    }
    // Latest-event fields move onto an existing unread row; everything else
    // is fixed at insert. The filter's equality fields seed the new row.
    let mut set = doc! {};
    let mut on_insert = doc! {};
    let source = document
        .remove("source")
        .and_then(|value| value.as_document().cloned())
        .unwrap_or_default();
    for (field, value) in source {
        let path = format!("source.{field}");
        if matches!(
            field.as_str(),
            "message_id" | "turn_id" | "seq" | "agent_id"
        ) {
            set.insert(path, value);
        } else {
            on_insert.insert(path, value);
        }
    }
    for (field, value) in document {
        match field.as_str() {
            "user_id" | "source_key" | "coalesced" | "unread" | "count" => {}
            "kind" | "outcome" | "reason" | "updated_at" | "expires_at" => {
                set.insert(field, value);
            }
            _ => {
                on_insert.insert(field, value);
            }
        }
    }
    collection
        .update_one(
            doc! {"user_id": new.user_id, "source_key": &key, "coalesced": true, "unread": true},
            doc! {"$setOnInsert": on_insert, "$set": set, "$inc": {"count": 1_i64}},
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    Ok(())
}

/// Parent write fence: the trigger row is written so a concurrent trigger
/// delete serializes against this producer. Returns false when it is gone.
async fn fence_trigger(
    db: &Database,
    session: &mut ClientSession,
    user_id: &str,
    trigger_id: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(TRIGGERS)
        .update_one(
            doc! {"_id": trigger_id, "user_id": user_id},
            doc! {"$inc": {"notification_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        == 1)
}

/// Whether the thread's personal platform binding is still active for this
/// exact owner, generation and conversation (projection read, no write).
async fn platform_binding_live(
    db: &Database,
    session: &mut ClientSession,
    thread: &PlatformThread,
    conversation_id: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(PLATFORM_BINDINGS)
        .find_one(doc! {
            "_id": &thread.binding_id, "user_id": &thread.user_id,
            "generation": thread.binding_generation, "conversation_id": conversation_id,
            "state": "active",
        })
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some())
}

/// The live channel's owner (the route admin for organization bots), read
/// with a projection and no write.
async fn live_channel_owner(
    db: &Database,
    session: &mut ClientSession,
    channel_id: &str,
) -> AppResult<Option<String>> {
    Ok(db
        .collection::<Document>(CHANNELS)
        .find_one(doc! {"_id": channel_id, "status": {"$in": LIVE_CHANNEL_STATUSES.to_vec()}})
        .projection(doc! {"user_id": 1})
        .session(&mut *session)
        .await?
        .and_then(|channel| channel.get_str("user_id").ok().map(str::to_owned)))
}

/// Parent write fence for channel chats: the channel row is written so the
/// transactional unlink (`retire_connection`, which sets `status`) and the
/// transactional chat removal (`remove_thread`, which increments the channel's
/// `notification_generation`) serialize against this producer. Returns the
/// live channel's `user_id`, or `None` once it is retired (`failed` or
/// `disconnected` also purge the feed) or gone.
async fn fence_channel(
    db: &Database,
    session: &mut ClientSession,
    channel_id: &str,
) -> AppResult<Option<String>> {
    Ok(db
        .collection::<Document>(CHANNELS)
        .find_one_and_update(
            doc! {"_id": channel_id, "status": {"$in": LIVE_CHANNEL_STATUSES.to_vec()}},
            doc! {"$inc": {"notification_fence": 1_i64}},
        )
        .projection(doc! {"_id": 1, "user_id": 1})
        .session(&mut *session)
        .await?
        .and_then(|channel| channel.get_str("user_id").ok().map(str::to_owned)))
}

/// The chat's `nyxbot_threads` row. Skew-safe after [`fence_channel`]: chat
/// removal writes the same channel row before deleting the thread.
async fn live_thread_id(
    db: &Database,
    session: &mut ClientSession,
    channel_id: &str,
    partition: &str,
) -> AppResult<Option<String>> {
    Ok(db
        .collection::<Document>(CHANNEL_THREADS)
        .find_one(doc! {"channel_id": channel_id, "partition": partition})
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .and_then(|thread| thread.get_str("_id").ok().map(str::to_owned)))
}

/// A destroyed agent produces no further updates. This read is skew-safe:
/// destroy writes every thread of the agent, including the conversation the
/// calling settlement transaction replaces, so one of the two transactions
/// retries and observes the other.
async fn agent_live(db: &Database, session: &mut ClientSession, agent_id: &str) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(AGENTS)
        .find_one(doc! {"_id": agent_id, "destroyed_at": bson::Bson::Null})
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some())
}

/// Turn settlement (`finish_turn_inner`) and the `turn_lost` reclaim: a reply
/// settled in an owner-visible thread by a turn the owner did not start from
/// the app. Uses the flag snapshot taken at admission (no reads when off),
/// except for an organization guest lane whose audience is the route admin.
pub async fn record_turn_settled_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    turn: &ActiveTurn,
    message: &AssistantMessage,
) -> AppResult<()> {
    // A channel chat: the thread's own channel, or, for a Channel-origin turn
    // on the agent's home thread, the chat the owner wrote from (owner DMs).
    // Event and orchestrator turns keep a stale delivery preference out of it.
    let channel = row.channel.as_ref().or(match turn.origin {
        TurnOrigin::Channel => row.reply_channel.as_ref(),
        _ => None,
    });
    let org_guest = channel
        .and_then(|channel| channel.org_execution.as_ref())
        .is_some_and(|execution| execution.guest);
    if (!turn.notifications_enabled && !org_guest)
        || row.guest_model_setup
        || row.collaboration.is_some()
        || row.group_id.is_some()
        || message.role != "assistant"
    {
        return Ok(());
    }
    // A linked platform chat (owner DMs through a manager bot) proves itself
    // with the turn's server-authored origin on the thread's own binding.
    let platform = row.platform_thread.as_ref().filter(|thread| {
        turn.platform_origin
            .as_ref()
            .is_some_and(|origin| origin.thread == **thread)
    });
    match turn.origin {
        // The owner is in the app for a web turn; a run's terminal transition
        // covers trigger turns; hidden member threads are covered by the group
        // transcript.
        TurnOrigin::User if platform.is_none() => return Ok(()),
        TurnOrigin::Trigger | TurnOrigin::Group => return Ok(()),
        TurnOrigin::User | TurnOrigin::Orchestrator | TurnOrigin::Event | TurnOrigin::Channel => {}
    }
    let outcome = Some(
        match message.error_code.as_deref() {
            Some("cancelled") => "cancelled",
            Some(_) => "failed",
            None => "completed",
        }
        .to_owned(),
    );
    let (audience, kind, source) = if let Some(channel) = channel {
        if org_guest {
            // The linking admin's flag decides before the channel is fenced:
            // a disabled feed costs one projection read and no write (D8).
            let Some(admin) = live_channel_owner(db, session, &channel.nyxbot_channel_id).await?
            else {
                return Ok(());
            };
            if !enabled(db, &admin).await? {
                return Ok(());
            }
        }
        let Some(channel_owner) = fence_channel(db, session, &channel.nyxbot_channel_id).await?
        else {
            return Ok(());
        };
        let Some(thread_id) =
            live_thread_id(db, session, &channel.nyxbot_channel_id, &channel.partition).await?
        else {
            return Ok(());
        };
        if let Some(execution) = channel.org_execution.as_ref() {
            // Organization lanes: a guest's hidden lane is not the acting
            // person's; the linking admin who owns the route is told, and the
            // row references only the channel's public transcript (the
            // sanitized copy published in this same transaction). Members are
            // told about their own lane.
            let (audience, lane, conversation_id) = if execution.guest {
                (channel_owner, LANE_ORG_GUEST, None)
            } else {
                (row.user_id.clone(), LANE_ORG_MEMBER, Some(row.id.clone()))
            };
            (
                audience,
                NotificationKind::ChatUpdate,
                NotificationSource {
                    source_type: SourceType::ChannelChat,
                    id: thread_id.clone(),
                    conversation_id,
                    channel_id: Some(channel.nyxbot_channel_id.clone()),
                    thread_id: Some(thread_id),
                    lane: Some(lane.into()),
                    agent_id: Some(execution.agent_id.clone()),
                    message_id: Some(message.id.clone()),
                    turn_id: Some(turn.turn_id.clone()),
                    ..Default::default()
                },
            )
        } else {
            (
                row.user_id.clone(),
                NotificationKind::ChatUpdate,
                NotificationSource {
                    source_type: SourceType::ChannelChat,
                    id: row.id.clone(),
                    conversation_id: Some(row.id.clone()),
                    channel_id: Some(channel.nyxbot_channel_id.clone()),
                    thread_id: Some(thread_id),
                    agent_id: row.agent_id.clone(),
                    message_id: Some(message.id.clone()),
                    turn_id: Some(turn.turn_id.clone()),
                    ..Default::default()
                },
            )
        }
    } else if let Some(thread) = platform {
        // The reply was read in the platform chat. The binding must still be
        // live and bound to this exact thread (the saved-reply settlement in
        // this transaction wrote it); unlink purges by binding id, so a
        // settlement that lands after unlink records nothing (D7).
        if thread.user_id != row.user_id
            || !platform_binding_live(db, session, thread, &row.id).await?
        {
            return Ok(());
        }
        (
            row.user_id.clone(),
            NotificationKind::ChatUpdate,
            NotificationSource {
                source_type: SourceType::ChannelChat,
                id: row.id.clone(),
                conversation_id: Some(row.id.clone()),
                agent_id: row.agent_id.clone(),
                message_id: Some(message.id.clone()),
                turn_id: Some(turn.turn_id.clone()),
                platform_binding_id: Some(thread.binding_id.clone()),
                platform_binding_generation: Some(thread.binding_generation),
                ..Default::default()
            },
        )
    } else {
        if let Some(agent_id) = row.agent_id.as_deref()
            && !agent_live(db, session, agent_id).await?
        {
            return Ok(());
        }
        (
            row.user_id.clone(),
            NotificationKind::AgentUpdate,
            NotificationSource {
                source_type: SourceType::Conversation,
                id: row.id.clone(),
                conversation_id: Some(row.id.clone()),
                agent_id: row.agent_id.clone(),
                message_id: Some(message.id.clone()),
                turn_id: Some(turn.turn_id.clone()),
                ..Default::default()
            },
        )
    };
    record_in_session(
        db,
        session,
        NewRow {
            user_id: &audience,
            kind,
            source,
            id: None,
            outcome,
            reason: message.error_code.clone(),
            card_expires_at: None,
            push: PushPlan::Pending,
        },
    )
    .await
}

fn run_kind(run: &TriggerRun) -> Option<NotificationKind> {
    match run.outcome {
        RunOutcome::Completed => Some(NotificationKind::AutomationResult),
        // The owner cancelled it themselves.
        RunOutcome::Failed if run.reason.as_deref() == Some("cancelled") => None,
        RunOutcome::Failed => Some(NotificationKind::AutomationProblem),
        RunOutcome::Skipped
            if run
                .reason
                .as_deref()
                .is_some_and(|reason| ACTIONABLE_SKIP_REASONS.contains(&reason)) =>
        {
            Some(NotificationKind::AutomationProblem)
        }
        RunOutcome::Skipped | RunOutcome::Pending | RunOutcome::Started | RunOutcome::Waiting => {
            None
        }
    }
}

fn run_source(run: &TriggerRun) -> NotificationSource {
    NotificationSource {
        source_type: SourceType::Trigger,
        id: run.trigger_id.clone(),
        conversation_id: run.thread_id.clone(),
        trigger_id: Some(run.trigger_id.clone()),
        agent_id: run.agent_id.clone(),
        run_id: Some(run.id.clone()),
        turn_id: run.turn_id.clone(),
        ..Default::default()
    }
}

/// Trigger run terminal transition (`trigger_schedule::finish_update`): one
/// row per run. A run whose own delivery is push gets no second push.
pub async fn record_run_terminal_in_session(
    db: &Database,
    session: &mut ClientSession,
    run: &TriggerRun,
) -> AppResult<()> {
    let Some(kind) = run_kind(run) else {
        return Ok(());
    };
    if !enabled(db, &run.user_id).await? {
        return Ok(());
    }
    if !fence_trigger(db, session, &run.user_id, &run.trigger_id).await? {
        return Ok(());
    }
    let push = if matches!(run.deliver_to, DeliverTo::Notification) {
        PushPlan::Skipped("run_delivers_push")
    } else {
        PushPlan::Pending
    };
    record_in_session(
        db,
        session,
        NewRow {
            user_id: &run.user_id,
            kind,
            source: run_source(run),
            id: Some(deterministic_id("run", &run.id)),
            outcome: Some(run.outcome.as_str().to_owned()),
            reason: run.reason.clone(),
            card_expires_at: None,
            push,
        },
    )
    .await
}

/// Automatic trigger pause (`record_terminal_in_session`): the trigger row
/// is already written in this transaction, and the existing pause notice
/// already pushes, so this row never pushes again.
pub async fn record_run_paused_in_session(
    db: &Database,
    session: &mut ClientSession,
    run: &TriggerRun,
    notice_id: &str,
    reason: &str,
) -> AppResult<()> {
    if !enabled(db, &run.user_id).await? {
        return Ok(());
    }
    record_in_session(
        db,
        session,
        NewRow {
            user_id: &run.user_id,
            kind: NotificationKind::AutomationProblem,
            source: run_source(run),
            id: Some(deterministic_id("pause", notice_id)),
            outcome: Some("paused".into()),
            reason: Some(reason.to_owned()),
            card_expires_at: None,
            push: PushPlan::Skipped("pause_notice_push"),
        },
    )
    .await
}

/// A pending owner action card was inserted (`request_candidate_in_session`,
/// which already writes the conversation's authority fence). The admission
/// flag snapshot is reused when the thread has a live turn.
pub async fn record_card_pending_in_session(
    db: &Database,
    session: &mut ClientSession,
    card: &AssistantAcknowledgement,
    conversation: &AssistantConversation,
) -> AppResult<()> {
    if card.status != "pending" {
        return Ok(());
    }
    let audience = conversation
        .collaboration
        .as_ref()
        .map_or(conversation.user_id.as_str(), |binding| {
            binding.original_requester_id.as_str()
        });
    let enabled = match conversation.active_turn.as_ref() {
        Some(turn) if audience == conversation.user_id => turn.notifications_enabled,
        _ => self::enabled(db, audience).await?,
    };
    if !enabled {
        return Ok(());
    }
    let (source_type, id) = card_visible_source(conversation);
    record_in_session(
        db,
        session,
        NewRow {
            user_id: audience,
            kind: NotificationKind::NeedsDecision,
            source: NotificationSource {
                source_type,
                id,
                conversation_id: Some(conversation.id.clone()),
                // Channel unlink and group deletion purge by these references.
                channel_id: conversation
                    .channel
                    .as_ref()
                    .or(conversation.reply_channel.as_ref())
                    .map(|channel| channel.nyxbot_channel_id.clone()),
                group_id: conversation.group_id.clone().or_else(|| {
                    conversation
                        .collaboration
                        .as_ref()
                        .map(|binding| binding.group_id.clone())
                }),
                // Unlinking the platform chat purges its decisions too.
                platform_binding_id: conversation
                    .platform_thread
                    .as_ref()
                    .map(|thread| thread.binding_id.clone()),
                platform_binding_generation: conversation
                    .platform_thread
                    .as_ref()
                    .map(|thread| thread.binding_generation),
                agent_id: conversation.agent_id.clone(),
                card_id: Some(card.id.clone()),
                api_key_id: Some(card.api_key_id.clone()),
                run_id: card
                    .automation_work
                    .as_ref()
                    .map(|work| work.run_id.clone())
                    .or_else(|| card.trigger_run_id.clone()),
                turn_id: card.creation_turn_id.clone(),
                ..Default::default()
            },
            id: Some(deterministic_id("card", &card.id)),
            outcome: None,
            reason: Some(card.kind.clone()),
            card_expires_at: Some(card.expires_at),
            push: PushPlan::Pending,
        },
    )
    .await
}

/// The visible source a decision row belongs to (D6): a card raised through
/// a hidden group member thread or collaboration slot is the group's (the
/// sidebar marks the group and opening it reads the card), a card in a
/// channel thread is that chat's, and a private thread is itself.
/// `source.conversation_id` keeps the exact thread for purge and card joins.
fn card_visible_source(conversation: &AssistantConversation) -> (SourceType, String) {
    if let Some(group) = conversation.group_id.as_ref() {
        (SourceType::Group, group.clone())
    } else if let Some(binding) = conversation.collaboration.as_ref() {
        (SourceType::CollaborationGroup, binding.group_id.clone())
    } else if conversation.channel.is_some() || conversation.platform_thread.is_some() {
        (SourceType::ChannelChat, conversation.id.clone())
    } else {
        (SourceType::Conversation, conversation.id.clone())
    }
}

/// Which decision rows a resolution applies to.
pub enum CardScope<'a> {
    Ids(&'a [String]),
    Conversation(&'a str),
    Conversations(&'a [&'a str]),
    ApiKey(&'a str),
    Run(&'a str),
}

fn card_filter(scope: &CardScope<'_>) -> Document {
    let mut filter = doc! {
        fields::KIND: NotificationKind::NeedsDecision.as_str(),
        "resolved_at": bson::Bson::Null,
    };
    match scope {
        CardScope::Ids(ids) => {
            filter.insert(fields::SOURCE_CARD_ID, doc! {"$in": ids.to_vec()});
        }
        CardScope::Conversation(id) => {
            filter.insert(fields::SOURCE_CONVERSATION_ID, *id);
        }
        CardScope::Conversations(ids) => {
            filter.insert(fields::SOURCE_CONVERSATION_ID, doc! {"$in": ids.to_vec()});
        }
        CardScope::ApiKey(id) => {
            filter.insert(fields::SOURCE_API_KEY_ID, *id);
        }
        CardScope::Run(id) => {
            filter.insert(fields::SOURCE_RUN_ID, *id);
        }
    }
    filter
}

/// Resolving is the row's last activity: it is seen, and retention restarts
/// from now (the card no longer needs to outlive its timeout).
fn resolution_update(now: DateTime<Utc>, resolution: &str) -> Document {
    doc! {"$set": {
        "resolved_at": bson_date(now), "resolution": resolution,
        "unread": false, "updated_at": bson_date(now),
        "expires_at": bson_date(now + Duration::days(RETENTION_DAYS)),
    }}
}

/// Close decision rows whose card was decided, expired or superseded.
pub async fn resolve_cards(
    db: &Database,
    scope: CardScope<'_>,
    resolution: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let collection = db.collection::<Document>(COLLECTION_NAME);
    let action = collection.update_many(
        card_filter(&scope),
        resolution_update(Utc::now(), resolution),
    );
    match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    };
    Ok(())
}

pub async fn resolve_cards_in_session(
    db: &Database,
    session: &mut ClientSession,
    scope: CardScope<'_>,
    resolution: &str,
) -> AppResult<()> {
    resolve_cards(db, scope, resolution, Some(session)).await
}

/// Mirror of the lazy card timeout (`acknowledgements::expire`): rows whose
/// card deadline passed are resolved without a join.
pub async fn resolve_expired_cards(
    db: &Database,
    user_id: &str,
    conversation: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now();
    let mut filter = doc! {
        fields::USER_ID: user_id,
        fields::KIND: NotificationKind::NeedsDecision.as_str(),
        "resolved_at": bson::Bson::Null,
        "card_expires_at": {"$lte": bson_date(now)},
    };
    if let Some(conversation) = conversation {
        filter.insert(fields::SOURCE_CONVERSATION_ID, conversation);
    }
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(filter, resolution_update(now, "expired"))
        .await?;
    Ok(())
}

/// A member's reply or notice entered a personal or organization web group
/// transcript (`append_following_bound`, which already writes the group).
pub async fn record_group_reply_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    group: &AssistantGroup,
    message: &GroupMessage,
) -> AppResult<()> {
    if group.collaboration.is_some() || !matches!(message.role.as_str(), "agent" | "notice") {
        return Ok(());
    }
    if !enabled(db, owner).await? {
        return Ok(());
    }
    record_in_session(
        db,
        session,
        NewRow {
            user_id: owner,
            kind: NotificationKind::AgentUpdate,
            source: NotificationSource {
                source_type: SourceType::Group,
                id: group.id.clone(),
                group_id: Some(group.id.clone()),
                agent_id: message.agent_id.clone(),
                message_id: Some(message.id.clone()),
                seq: Some(message.seq),
                ..Default::default()
            },
            id: None,
            outcome: Some(
                if message.role == "notice" {
                    "failed"
                } else {
                    "completed"
                }
                .into(),
            ),
            reason: None,
            card_expires_at: None,
            push: PushPlan::Pending,
        },
    )
    .await
}

/// An agent's public reply in a collaboration group reached the person who
/// asked (`runtime::publish_reply`, which already writes the group).
pub async fn record_collaboration_reply_in_session(
    db: &Database,
    session: &mut ClientSession,
    requester: &str,
    group: &AssistantGroup,
    message: &GroupMessage,
) -> AppResult<()> {
    if !matches!(message.role.as_str(), "agent" | "notice") || !enabled(db, requester).await? {
        return Ok(());
    }
    record_in_session(
        db,
        session,
        NewRow {
            user_id: requester,
            kind: NotificationKind::AgentUpdate,
            source: NotificationSource {
                source_type: SourceType::CollaborationGroup,
                id: group.id.clone(),
                group_id: Some(group.id.clone()),
                agent_id: message.agent_id.clone(),
                message_id: Some(message.id.clone()),
                seq: Some(message.seq),
                ..Default::default()
            },
            id: None,
            outcome: Some(
                if message.role == "notice" {
                    "failed"
                } else {
                    "completed"
                }
                .into(),
            ),
            reason: None,
            card_expires_at: None,
            push: PushPlan::Pending,
        },
    )
    .await
}

/// A collaboration post mentioned `user_id` (`notifications::enqueue`, which
/// runs in the post's transaction after the group was written).
pub async fn record_mention_in_session(
    db: &Database,
    session: &mut ClientSession,
    user_id: &str,
    group: &AssistantGroup,
    message: &GroupMessage,
) -> AppResult<()> {
    if !enabled(db, user_id).await? {
        return Ok(());
    }
    record_in_session(
        db,
        session,
        NewRow {
            user_id,
            kind: NotificationKind::GroupMention,
            source: NotificationSource {
                source_type: SourceType::CollaborationGroup,
                id: group.id.clone(),
                group_id: Some(group.id.clone()),
                agent_id: message.agent_id.clone(),
                message_id: Some(message.id.clone()),
                seq: Some(message.seq),
                ..Default::default()
            },
            id: None,
            outcome: None,
            reason: None,
            card_expires_at: None,
            push: PushPlan::Pending,
        },
    )
    .await
}

/// Delete rows by any stored field (purge paths own the filter).
pub async fn purge(
    db: &Database,
    filter: Document,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let collection = db.collection::<Document>(COLLECTION_NAME);
    let action = collection.delete_many(filter);
    match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    };
    Ok(())
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn strip_links(text: &str) -> String {
    // `[label](url)` and `![alt](url)` keep only their label.
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let (before, after) = rest.split_at(start);
        let before = before.strip_suffix('!').unwrap_or(before);
        out.push_str(before);
        let Some(close) = after.find("](") else {
            out.push_str(after);
            return out;
        };
        let label = &after[1..close];
        let Some(end) = after[close..].find(')') else {
            out.push_str(after);
            return out;
        };
        out.push_str(label);
        rest = &after[close + end + 1..];
    }
    out.push_str(rest);
    out
}

/// One safe line of at most [`PREVIEW_CHARS`] scalars: markdown markers,
/// control and bidi characters removed, whitespace collapsed.
pub fn preview_text(raw: &str) -> String {
    let mut joined = String::new();
    let mut in_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || trimmed.is_empty() {
            continue;
        }
        let trimmed = trimmed.trim_start_matches(['#', '>']).trim_start();
        let trimmed = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .unwrap_or(trimmed);
        if !joined.is_empty() {
            joined.push(' ');
        }
        joined.push_str(trimmed);
    }
    let cleaned: String = strip_links(&joined)
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '`' | '*' | '~' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= PREVIEW_CHARS {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(PREVIEW_CHARS - 1).collect();
    out.push('…');
    out
}

fn label(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

/// Owner-safe copy for scheduler, delivery and model failure codes.
pub fn reason_copy(code: &str) -> String {
    match code {
        "grace_expired" => "Skipped: the run could not start within its grace window (budget, capacity or a busy thread).".into(),
        "queue_full" => "Skipped: a previous run was still queued.".into(),
        "owner_unavailable" => "Skipped: the account was unavailable.".into(),
        "target_unavailable" => "Skipped: the agent is no longer available.".into(),
        "assistant_disabled" => "Skipped: the assistant engine is disabled for this account.".into(),
        "turn_lost" => "The run's turn was lost before it finished.".into(),
        "thread_deleted" => "The run's thread was deleted before it finished.".into(),
        "delivery_interrupted" | "result_delivery_interrupted" => {
            "The result was saved, but its delivery was interrupted.".into()
        }
        "result_delivery_failed" => "The result was saved, but its chat delivery failed.".into(),
        "confirmation_expired" => "A confirmation card expired before the run could continue.".into(),
        "confirmation_denied" => "A confirmation was denied, so the run stopped.".into(),
        "target_changed" => "The automation was changed while the run waited.".into(),
        "turn_start_failed" => "The run could not start a turn.".into(),
        "async_operation_failed" => "An asynchronous service operation of the run failed.".into(),
        "coordinator_timeout" => "The run reached its one-hour deadline.".into(),
        "continuation_limit_exceeded" => "The run reached its continuation limit.".into(),
        "coordinator_protocol_unsupported" => "The run used an unsupported completion protocol.".into(),
        code if code.starts_with("push_") => automation::message(code).into(),
        code => TurnError::new(code).message,
    }
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: usize,
    pub unread_only: bool,
    /// Only unresolved rows; for decision rows also only cards that have not
    /// timed out. Seen (read) pending cards remain included.
    pub pending_only: bool,
    pub kinds: Option<Vec<NotificationKind>>,
    pub since: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceDto {
    #[serde(rename = "type")]
    pub source_type: &'static str,
    pub id: String,
    pub label: String,
    pub conversation_id: Option<String>,
    pub group_id: Option<String>,
    pub channel_id: Option<String>,
    pub trigger_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NotificationItem {
    pub id: String,
    pub kind: &'static str,
    pub count: i64,
    pub source: SourceDto,
    pub title: String,
    pub preview: String,
    pub url: String,
    pub read: bool,
    pub resolved: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ListPage {
    pub items: Vec<NotificationItem>,
    pub next_cursor: Option<String>,
    pub as_of: String,
}

fn encode_cursor(row: &AssistantNotification) -> String {
    format!("{}|{}", row.updated_at.timestamp_millis(), row.id)
}

fn parse_cursor(cursor: &str) -> AppResult<(bson::DateTime, String)> {
    let invalid = || AppError::BadRequest("Invalid cursor".into());
    let (millis, id) = cursor.split_once('|').ok_or_else(invalid)?;
    let millis: i64 = millis.parse().map_err(|_| invalid())?;
    if id.is_empty() || id.len() > 64 {
        return Err(invalid());
    }
    Ok((bson::DateTime::from_millis(millis), id.to_owned()))
}

pub fn parse_kinds(value: &str) -> AppResult<Vec<NotificationKind>> {
    let mut kinds = Vec::new();
    for part in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let kind = NotificationKind::parse(part)
            .ok_or_else(|| AppError::BadRequest(format!("Unknown notification kind: {part}")))?;
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    Ok(kinds)
}

/// One page, newest activity first: one row query plus at most one batched
/// lookup per referenced collection, live ownership checks, and purge of rows
/// whose source no longer exists.
pub async fn list(state: &AppState, user_id: &str, query: &ListQuery) -> AppResult<ListPage> {
    let as_of = Utc::now();
    let limit = query.limit.clamp(1, MAX_LIST_LIMIT);
    let mut filter = doc! {fields::USER_ID: user_id};
    if query.unread_only {
        filter.insert("unread", true);
    }
    let mut clauses: Vec<Document> = Vec::new();
    if query.pending_only {
        filter.insert("resolved_at", bson::Bson::Null);
        clauses.push(doc! {"$or": [
            {fields::KIND: {"$ne": NotificationKind::NeedsDecision.as_str()}},
            {"card_expires_at": {"$gt": bson_date(as_of)}},
        ]});
    }
    if let Some(kinds) = query.kinds.as_ref().filter(|kinds| !kinds.is_empty()) {
        filter.insert(
            fields::KIND,
            doc! {"$in": kinds.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()},
        );
    }
    if let Some(since) = query.since {
        filter.insert("updated_at", doc! {"$gte": bson_date(since)});
    }
    if let Some(cursor) = query.cursor.as_deref() {
        let (at, id) = parse_cursor(cursor)?;
        clauses.push(doc! {"$or": [
            {"updated_at": {"$lt": at}},
            {"updated_at": at, "_id": {"$lt": id}},
        ]});
    }
    if !clauses.is_empty() {
        filter.insert("$and", clauses);
    }
    let mut rows: Vec<AssistantNotification> = state
        .db
        .collection(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"updated_at": -1, "_id": -1})
        .limit(limit as i64 + 1)
        .await?
        .try_collect()
        .await?;
    let next_cursor = if rows.len() > limit {
        rows.truncate(limit);
        rows.last().map(encode_cursor)
    } else {
        None
    };
    let mut items = resolve_items(state, user_id, rows).await?;
    if query.pending_only {
        // A card decided elsewhere a moment ago is no longer pending.
        items.retain(|item| !item.resolved);
    }
    Ok(ListPage {
        items,
        next_cursor,
        as_of: rfc3339(as_of),
    })
}

struct Lookups {
    conversations: HashMap<String, Document>,
    groups: HashMap<String, AssistantGroup>,
    runs: HashMap<String, Document>,
    triggers: HashMap<String, Document>,
    cards: HashMap<String, Document>,
    messages: HashMap<String, Document>,
    /// Assistant replies of automation runs, by `{conversation_id}|{turn_id}`.
    replies: HashMap<String, Document>,
    group_messages: HashMap<String, Document>,
    /// Sanitized organization channel transcript entries, by message id.
    org_messages: HashMap<String, Document>,
    agents: HashMap<String, Document>,
    /// Channel chats by `nyxbot_threads._id`.
    threads: HashMap<String, Document>,
    channels: HashMap<String, Document>,
    /// The person's active membership role per organization owner id.
    memberships: HashMap<String, String>,
    /// Organization owners that are active organization users.
    active_orgs: HashSet<String>,
}

async fn load_documents(
    db: &Database,
    collection: &str,
    filter: Document,
    projection: Document,
) -> AppResult<Vec<Document>> {
    Ok(db
        .collection::<Document>(collection)
        .find(filter)
        .projection(projection)
        .await?
        .try_collect()
        .await?)
}

fn by_id(rows: Vec<Document>) -> HashMap<String, Document> {
    rows.into_iter()
        .filter_map(|row| {
            row.get_str("_id")
                .ok()
                .map(|id| (id.to_owned(), row.clone()))
        })
        .collect()
}

fn ids(set: &HashSet<String>) -> Vec<String> {
    set.iter().cloned().collect()
}

fn org_of_conversation(conversation: &Document, user_id: &str) -> Option<String> {
    conversation
        .get_str("agent_owner_id")
        .ok()
        .filter(|org| *org != user_id)
        .map(str::to_owned)
}

fn org_of_group(group: &AssistantGroup) -> Option<String> {
    (crate::services::assistant_group_kind::classify(group)
        == crate::services::assistant_group_kind::GroupKind::Organization)
        .then(|| group.user_id.clone())
}

/// Roles the typed `OrgMembership` deserializes; anything else is not a member.
const MEMBER_ROLES: [&str; 3] = ["admin", "member", "viewer"];

fn org_of_channel(channel: &Document) -> Option<String> {
    channel
        .get_document("org_specialist")
        .ok()
        .and_then(|binding| binding.get_str("org_id").ok())
        .map(str::to_owned)
}

async fn load_lookups(
    db: &Database,
    user_id: &str,
    rows: &[AssistantNotification],
) -> AppResult<Lookups> {
    let mut conversation_ids = HashSet::new();
    let mut group_ids = HashSet::new();
    let mut run_ids = HashSet::new();
    let mut trigger_ids = HashSet::new();
    let mut card_ids = HashSet::new();
    let mut message_ids = HashSet::new();
    let mut org_message_ids = HashSet::new();
    let mut reply_pairs: Vec<(String, String)> = Vec::new();
    let mut group_message_ids = HashSet::new();
    let mut agent_ids = HashSet::new();
    let mut channel_ids = HashSet::new();
    let mut thread_ids = HashSet::new();
    for row in rows {
        let source = &row.source;
        if let Some(id) = &source.conversation_id {
            conversation_ids.insert(id.clone());
        }
        if let Some(id) = &source.group_id {
            group_ids.insert(id.clone());
        }
        if let Some(id) = &source.trigger_id {
            trigger_ids.insert(id.clone());
        }
        if let Some(id) = &source.agent_id {
            agent_ids.insert(id.clone());
        }
        if let Some(id) = &source.channel_id {
            channel_ids.insert(id.clone());
        }
        if let Some(id) = &source.thread_id {
            thread_ids.insert(id.clone());
        }
        match row.kind {
            NotificationKind::AutomationResult | NotificationKind::AutomationProblem => {
                if let Some(id) = &source.run_id {
                    run_ids.insert(id.clone());
                }
                if let (Some(thread), Some(turn)) = (&source.conversation_id, &source.turn_id)
                    && row.kind == NotificationKind::AutomationResult
                {
                    reply_pairs.push((thread.clone(), turn.clone()));
                }
            }
            NotificationKind::NeedsDecision => {
                if let Some(id) = &source.card_id {
                    card_ids.insert(id.clone());
                }
            }
            NotificationKind::AgentUpdate | NotificationKind::ChatUpdate
                if source.group_id.is_none() =>
            {
                if let Some(id) = &source.message_id {
                    if source.lane.is_some() {
                        org_message_ids.insert(id.clone());
                    } else {
                        message_ids.insert(id.clone());
                    }
                }
            }
            NotificationKind::AgentUpdate
            | NotificationKind::ChatUpdate
            | NotificationKind::GroupMention => {
                if let Some(id) = &source.message_id {
                    group_message_ids.insert(id.clone());
                }
            }
        }
    }
    let conversations = if conversation_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                CONVERSATIONS,
                doc! {"_id": {"$in": ids(&conversation_ids)}},
                doc! {"_id": 1, "user_id": 1, "title": 1, "agent_id": 1, "agent_owner_id": 1,
                "collaboration.original_requester_id": 1, "channel.nyxbot_channel_id": 1,
                "channel.platform": 1, "group_id": 1, "platform_thread.binding_id": 1,
                "platform_thread.binding_generation": 1, "platform_thread.user_id": 1,
                "platform_thread.platform": 1},
            )
            .await?,
        )
    };
    let groups: HashMap<String, AssistantGroup> = if group_ids.is_empty() {
        HashMap::new()
    } else {
        db.collection::<AssistantGroup>(GROUPS)
            .find(doc! {"_id": {"$in": ids(&group_ids)}})
            .await?
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|group| (group.id.clone(), group))
            .collect()
    };
    let runs = if run_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                RUNS,
                doc! {"_id": {"$in": ids(&run_ids)}},
                doc! {"_id": 1, "user_id": 1, "label": 1, "outcome": 1, "reason": 1,
                "thread_id": 1, "turn_id": 1, "agent_id": 1, "trigger_id": 1,
                "notification_delivery.reason": 1},
            )
            .await?,
        )
    };
    let triggers = if trigger_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                TRIGGERS,
                doc! {"_id": {"$in": ids(&trigger_ids)}},
                doc! {"_id": 1, "user_id": 1, "label": 1, "status": 1},
            )
            .await?,
        )
    };
    let cards = if card_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                ACKS,
                doc! {"_id": {"$in": ids(&card_ids)}},
                doc! {"_id": 1, "user_id": 1, "summary": 1, "status": 1, "expires_at": 1,
                "conversation_id": 1},
            )
            .await?,
        )
    };
    // One read of `assistant_messages` serves both previews by message id
    // and automation replies by conversation and turn (<= 1 query per
    // referenced collection).
    let (messages, replies) = if message_ids.is_empty() && reply_pairs.is_empty() {
        (HashMap::new(), HashMap::new())
    } else {
        let mut clauses: Vec<Document> = Vec::new();
        if !message_ids.is_empty() {
            clauses.push(doc! {"_id": {"$in": ids(&message_ids)}});
        }
        if !reply_pairs.is_empty() {
            let pairs: Vec<Document> = reply_pairs
                .iter()
                .map(|(thread, turn)| doc! {"conversation_id": thread, "turn_id": turn})
                .collect();
            clauses.push(doc! {"user_id": user_id, "role": "assistant", "$or": pairs});
        }
        let wanted_pairs: HashSet<String> = reply_pairs
            .iter()
            .map(|(thread, turn)| format!("{thread}|{turn}"))
            .collect();
        let mut messages = HashMap::new();
        let mut replies = HashMap::new();
        for row in load_documents(
            db,
            MESSAGES,
            doc! {"$or": clauses},
            doc! {"_id": 1, "text": 1, "status": 1, "error_code": 1, "conversation_id": 1,
            "turn_id": 1, "role": 1, "user_id": 1},
        )
        .await?
        {
            if let Ok(id) = row.get_str("_id")
                && message_ids.contains(id)
            {
                messages.insert(id.to_owned(), row.clone());
            }
            if row.get_str("role").ok() == Some("assistant")
                && row.get_str("user_id").ok() == Some(user_id)
                && let (Ok(thread), Ok(turn)) =
                    (row.get_str("conversation_id"), row.get_str("turn_id"))
            {
                let key = format!("{thread}|{turn}");
                if wanted_pairs.contains(&key) {
                    replies.insert(key, row);
                }
            }
        }
        (messages, replies)
    };
    let group_messages = if group_message_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                GROUP_MESSAGES,
                doc! {"_id": {"$in": ids(&group_message_ids)}},
                doc! {"_id": 1, "text": 1, "role": 1, "group_id": 1, "agent_id": 1},
            )
            .await?,
        )
    };
    let org_messages = if org_message_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                ORG_CHANNEL_MESSAGES,
                doc! {"_id": {"$in": ids(&org_message_ids)}},
                doc! {"_id": 1, "text": 1, "role": 1, "channel_id": 1},
            )
            .await?,
        )
    };
    let agents = if agent_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                AGENTS,
                doc! {"_id": {"$in": ids(&agent_ids)}},
                doc! {"_id": 1, "name": 1, "display_name": 1, "kind": 1, "destroyed_at": 1},
            )
            .await?,
        )
    };
    let threads = if thread_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                CHANNEL_THREADS,
                doc! {"_id": {"$in": ids(&thread_ids)}},
                doc! {"_id": 1, "conversation_id": 1, "title": 1, "kind": 1, "channel_id": 1},
            )
            .await?,
        )
    };
    let channels = if channel_ids.is_empty() {
        HashMap::new()
    } else {
        by_id(
            load_documents(
                db,
                CHANNELS,
                doc! {"_id": {"$in": ids(&channel_ids)}},
                doc! {"_id": 1, "user_id": 1, "platform": 1, "bot_label": 1, "status": 1,
                "org_specialist.org_id": 1},
            )
            .await?,
        )
    };
    // Organization access is evaluated in memory from one membership read and
    // one active-organization read, never per source.
    let mut org_ids: HashSet<String> = HashSet::new();
    org_ids.extend(
        conversations
            .values()
            .filter_map(|conversation| org_of_conversation(conversation, user_id)),
    );
    org_ids.extend(groups.values().filter_map(org_of_group));
    org_ids.extend(channels.values().filter_map(org_of_channel));
    let (memberships, active_orgs): (HashMap<String, String>, HashSet<String>) =
        if org_ids.is_empty() {
            (HashMap::new(), HashSet::new())
        } else {
            let memberships = load_documents(
                db,
                ORG_MEMBERSHIPS,
                doc! {"org_user_id": {"$in": ids(&org_ids)}, "member_user_id": user_id,
                "revoked_at": bson::Bson::Null},
                doc! {"org_user_id": 1, "role": 1},
            )
            .await?
            .into_iter()
            .filter_map(|row| {
                let org = row.get_str("org_user_id").ok()?.to_owned();
                let role = row.get_str("role").ok()?;
                MEMBER_ROLES.contains(&role).then(|| (org, role.to_owned()))
            })
            .collect();
            let active_orgs = load_documents(
                db,
                USERS,
                doc! {"_id": {"$in": ids(&org_ids)}, "is_active": true, "user_type": "org"},
                doc! {"_id": 1},
            )
            .await?
            .into_iter()
            .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
            .collect();
            (memberships, active_orgs)
        };
    Ok(Lookups {
        conversations,
        groups,
        runs,
        triggers,
        cards,
        messages,
        replies,
        group_messages,
        org_messages,
        agents,
        threads,
        channels,
        memberships,
        active_orgs,
    })
}

impl Lookups {
    /// `resolve_owner_access(..).can_read()` in memory: an active organization
    /// with an active membership of the person.
    fn org_member(&self, org: &str) -> bool {
        self.active_orgs.contains(org) && self.memberships.contains_key(org)
    }

    fn org_admin(&self, org: &str) -> bool {
        self.active_orgs.contains(org)
            && self
                .memberships
                .get(org)
                .is_some_and(|role| role == "admin")
    }

    /// Live access to a conversation row: its owner or collaboration
    /// requester, and for organization-owned agents a current member.
    fn conversation_visible(&self, user_id: &str, conversation: &Document) -> bool {
        let owner = conversation.get_str("user_id").unwrap_or_default();
        let requester = conversation
            .get_document("collaboration")
            .ok()
            .and_then(|binding| binding.get_str("original_requester_id").ok());
        if owner != user_id && requester != Some(user_id) {
            return false;
        }
        match org_of_conversation(conversation, user_id) {
            Some(org) => self.org_member(&org),
            None => true,
        }
    }

    /// Group read authorization through the structural classifier: personal
    /// owner, collaboration participant (exact marker version only), or
    /// organization participant with a current membership. Unknown markers
    /// are never readable.
    fn group_visible(&self, user_id: &str, group: &AssistantGroup) -> bool {
        use crate::services::assistant_group_kind::{GroupKind, classify};
        let participant = group.participant_user_ids.iter().any(|id| id == user_id);
        match classify(group) {
            GroupKind::Collaboration => participant,
            GroupKind::Organization => participant && self.org_member(&group.user_id),
            GroupKind::Personal => group.user_id == user_id,
            GroupKind::Unsupported => false,
        }
    }
}

fn agent_title(agents: &HashMap<String, Document>, agent_id: Option<&str>) -> Option<String> {
    let agent = agents.get(agent_id?)?;
    let name = agent
        .get_str("display_name")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| agent.get_str("name").ok())?;
    Some(label(name, 80))
}

fn channel_label(channel: &Document) -> String {
    channel
        .get_str("bot_label")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| channel.get_str("platform").ok())
        .map(|value| label(value, 60))
        .unwrap_or_else(|| "Chat".into())
}

/// A row of a linked platform chat must still describe the thread's own
/// binding (id, generation, owner); a relinked or foreign thread hides it.
fn platform_thread_matches(
    conversation: &Document,
    source: &NotificationSource,
    user_id: &str,
) -> bool {
    let Some(binding) = source.platform_binding_id.as_deref() else {
        return true;
    };
    conversation
        .get_document("platform_thread")
        .ok()
        .is_some_and(|thread| {
            thread.get_str("binding_id").ok() == Some(binding)
                && thread.get_i64("binding_generation").ok() == source.platform_binding_generation
                && thread.get_str("user_id").ok() == Some(user_id)
        })
}

fn platform_label(platform: &str) -> String {
    match platform {
        "telegram" => "Telegram".into(),
        "slack" => "Slack".into(),
        "discord" => "Discord".into(),
        "whatsapp" => "WhatsApp".into(),
        "lark" => "Lark".into(),
        "feishu" => "Feishu".into(),
        "x" => "X".into(),
        other => label(other, 24),
    }
}

fn message_preview(message: &Document) -> String {
    let text = message.get_str("text").unwrap_or_default();
    if (message.get_str("status").ok() == Some("failed") || text.trim().is_empty())
        && let Ok(code) = message.get_str("error_code")
    {
        return preview_text(&TurnError::new(code).message);
    }
    preview_text(text)
}

enum Built {
    Item {
        item: Box<NotificationItem>,
        /// A decision row whose card settled without its hook: the label to
        /// record lazily.
        resolve: Option<&'static str>,
    },
    /// The source no longer exists: delete the row.
    Purge,
    /// The person can no longer see the source: hide the row.
    Hidden,
}

fn item(
    row: &AssistantNotification,
    label_text: String,
    title: String,
    preview: String,
    url: String,
    resolved: bool,
) -> NotificationItem {
    NotificationItem {
        id: row.id.clone(),
        kind: row.kind.as_str(),
        count: row.count,
        source: SourceDto {
            source_type: row.source.source_type.as_str(),
            id: row.source.id.clone(),
            label: label_text,
            conversation_id: row.source.conversation_id.clone(),
            group_id: row.source.group_id.clone(),
            channel_id: row.source.channel_id.clone(),
            trigger_id: row.source.trigger_id.clone(),
            agent_id: row.source.agent_id.clone(),
        },
        title,
        preview,
        url,
        read: row.read_at.is_some(),
        resolved,
        created_at: rfc3339(row.created_at),
        updated_at: rfc3339(row.updated_at),
    }
}

enum ChannelState<'a> {
    /// The channel document is gone: real retirement, the row goes.
    Absent,
    /// Present but not live (`failed` is repairable): the row hides.
    Retired,
    Live(&'a Document),
}

fn channel_state<'a>(lookups: &'a Lookups, source: &NotificationSource) -> ChannelState<'a> {
    match source
        .channel_id
        .as_deref()
        .and_then(|id| lookups.channels.get(id))
    {
        None => ChannelState::Absent,
        Some(channel)
            if channel
                .get_str("status")
                .is_ok_and(|status| LIVE_CHANNEL_STATUSES.contains(&status)) =>
        {
            ChannelState::Live(channel)
        }
        Some(_) => ChannelState::Retired,
    }
}

/// How a decision row settles when its card is no longer pending and no
/// hook recorded it (decided on another replica, timed out, or withdrawn).
fn card_resolution(status: &str) -> &'static str {
    match status {
        "pending" | "expired" => "expired",
        "allowed" | "allowed_pending_attachment" | "denied" | "used" | "completed" => "decided",
        _ => "superseded",
    }
}

fn ready(item: NotificationItem) -> Built {
    Built::Item {
        item: Box::new(item),
        resolve: None,
    }
}

fn build(
    user_id: &str,
    frontend: &str,
    row: &AssistantNotification,
    lookups: &Lookups,
    now: DateTime<Utc>,
) -> Built {
    let source = &row.source;
    let resolved = row.resolved_at.is_some();
    match row.kind {
        NotificationKind::ChatUpdate if source.lane.is_some() => {
            // Organization channel lane: the row references the chat's public
            // transcript only; a guest's hidden lane is never read.
            let Some(thread) = source
                .thread_id
                .as_deref()
                .and_then(|id| lookups.threads.get(id))
            else {
                return Built::Purge;
            };
            let channel = match channel_state(lookups, source) {
                ChannelState::Absent => return Built::Purge,
                ChannelState::Retired => return Built::Hidden,
                ChannelState::Live(channel) => channel,
            };
            let visible = match source.lane.as_deref() {
                // The route admin must still administer the channel's organization.
                Some(LANE_ORG_GUEST) => {
                    channel.get_str("user_id").ok() == Some(user_id)
                        && org_of_channel(channel).is_some_and(|org| lookups.org_admin(&org))
                }
                Some(LANE_ORG_MEMBER) => match source
                    .conversation_id
                    .as_deref()
                    .and_then(|id| lookups.conversations.get(id))
                {
                    Some(conversation) => lookups.conversation_visible(user_id, conversation),
                    None => return Built::Purge,
                },
                _ => false,
            };
            if !visible {
                return Built::Hidden;
            }
            let preview = source
                .message_id
                .as_deref()
                .and_then(|id| lookups.org_messages.get(id))
                .filter(|message| {
                    message.get_str("channel_id").ok() == source.channel_id.as_deref()
                })
                .map(|message| preview_text(message.get_str("text").unwrap_or_default()))
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    row.reason
                        .as_deref()
                        .map(|code| preview_text(&TurnError::new(code).message))
                })
                .unwrap_or_default();
            let bot = channel_label(channel);
            let chat = thread
                .get_str("title")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| label(value, 60))
                .unwrap_or_else(|| "chat".into());
            let url = match source.agent_id.as_deref() {
                Some(agent) => AssistantPage::AgentDetails {
                    agent,
                    tab: AGENT_DETAILS_CHANNELS,
                }
                .url(frontend),
                None => AssistantPage::Agent { agent: None }.url(frontend),
            };
            ready(item(
                row,
                chat.clone(),
                format!("{bot} · {chat}"),
                preview,
                url,
                resolved,
            ))
        }
        NotificationKind::AgentUpdate | NotificationKind::ChatUpdate
            if source.group_id.is_none() =>
        {
            let Some(conversation) = source
                .conversation_id
                .as_deref()
                .and_then(|id| lookups.conversations.get(id))
            else {
                return Built::Purge;
            };
            if !lookups.conversation_visible(user_id, conversation)
                || !platform_thread_matches(conversation, source, user_id)
            {
                return Built::Hidden;
            }
            let conversation_id = source.conversation_id.clone().unwrap_or_default();
            let preview = source
                .message_id
                .as_deref()
                .and_then(|id| lookups.messages.get(id))
                .filter(|message| {
                    message.get_str("conversation_id").ok() == source.conversation_id.as_deref()
                })
                .map(message_preview)
                .or_else(|| {
                    row.reason
                        .as_deref()
                        .map(|code| preview_text(&TurnError::new(code).message))
                })
                .unwrap_or_default();
            let agent = agent_title(&lookups.agents, source.agent_id.as_deref());
            let (label_text, title) = if row.kind == NotificationKind::ChatUpdate
                && source.channel_id.is_none()
                && source.platform_binding_id.is_some()
            {
                // A linked platform chat: labelled by platform and agent.
                let platform = conversation
                    .get_document("platform_thread")
                    .ok()
                    .and_then(|thread| thread.get_str("platform").ok())
                    .map(platform_label)
                    .unwrap_or_else(|| "Chat".into());
                let who = agent
                    .clone()
                    .or_else(|| {
                        conversation
                            .get_str("title")
                            .ok()
                            .map(|value| label(value, 60))
                    })
                    .unwrap_or_else(|| "Agent".into());
                (who.clone(), format!("{platform} · {who}"))
            } else if row.kind == NotificationKind::ChatUpdate {
                let channel = match channel_state(lookups, source) {
                    ChannelState::Absent => return Built::Purge,
                    ChannelState::Retired => return Built::Hidden,
                    ChannelState::Live(channel) => channel,
                };
                let thread = match source.thread_id.as_deref() {
                    Some(id) => match lookups.threads.get(id) {
                        Some(thread) => Some(thread),
                        // The chat was removed; its history stays, the row goes.
                        None => return Built::Purge,
                    },
                    None => None,
                };
                let bot = channel_label(channel);
                let chat = thread
                    .and_then(|thread| thread.get_str("title").ok())
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| label(value, 60))
                    .or_else(|| {
                        conversation
                            .get_str("title")
                            .ok()
                            .map(|value| label(value, 60))
                    })
                    .unwrap_or_else(|| "chat".into());
                (chat.clone(), format!("{bot} · {chat}"))
            } else {
                let title = agent
                    .clone()
                    .or_else(|| {
                        conversation
                            .get_str("title")
                            .ok()
                            .map(|value| label(value, 80))
                    })
                    .unwrap_or_else(|| "Agent".into());
                (title.clone(), title)
            };
            let url = AssistantPage::Conversation {
                conversation: &conversation_id,
            }
            .url(frontend);
            ready(item(row, label_text, title, preview, url, resolved))
        }
        NotificationKind::AgentUpdate
        | NotificationKind::ChatUpdate
        | NotificationKind::GroupMention => {
            let Some(group) = source
                .group_id
                .as_deref()
                .and_then(|id| lookups.groups.get(id))
            else {
                return Built::Purge;
            };
            if !lookups.group_visible(user_id, group) {
                return Built::Hidden;
            }
            let preview = source
                .message_id
                .as_deref()
                .and_then(|id| lookups.group_messages.get(id))
                .filter(|message| message.get_str("group_id").ok() == source.group_id.as_deref())
                .map(|message| preview_text(message.get_str("text").unwrap_or_default()))
                .unwrap_or_default();
            let name = label(&group.name, 80);
            let title = if row.kind == NotificationKind::GroupMention {
                format!("Mention in {name}")
            } else {
                agent_title(&lookups.agents, source.agent_id.as_deref())
                    .map(|agent| format!("{agent} in {name}"))
                    .unwrap_or_else(|| name.clone())
            };
            let url = AssistantPage::Group { group: &group.id }.url(frontend);
            ready(item(row, name, title, preview, url, resolved))
        }
        NotificationKind::NeedsDecision => {
            let Some(conversation) = source
                .conversation_id
                .as_deref()
                .and_then(|id| lookups.conversations.get(id))
            else {
                return Built::Purge;
            };
            let Some(card) = source
                .card_id
                .as_deref()
                .and_then(|id| lookups.cards.get(id))
            else {
                return Built::Purge;
            };
            if !lookups.conversation_visible(user_id, conversation)
                || !platform_thread_matches(conversation, source, user_id)
            {
                return Built::Hidden;
            }
            // The card must be this thread's; anything else is hidden, never
            // deleted (only a missing card document is).
            if card.get_str("conversation_id").ok() != source.conversation_id.as_deref() {
                return Built::Hidden;
            }
            // A card raised in a group or channel chat also requires that
            // room's live access; a retained hidden lane never leaks it.
            let group = match source.group_id.as_deref() {
                Some(id) => match lookups.groups.get(id) {
                    Some(group) if lookups.group_visible(user_id, group) => Some(group),
                    Some(_) => return Built::Hidden,
                    None => return Built::Purge,
                },
                None => None,
            };
            if source.channel_id.is_some() {
                match channel_state(lookups, source) {
                    ChannelState::Absent => return Built::Purge,
                    ChannelState::Retired => return Built::Hidden,
                    ChannelState::Live(_) => {}
                }
            }
            let status = card.get_str("status").unwrap_or_default();
            let pending = status == "pending"
                && card
                    .get_datetime("expires_at")
                    .is_ok_and(|at| at.to_chrono() > now);
            let resolve = (!resolved && !pending).then(|| card_resolution(status));
            let resolved = resolved || !pending;
            let preview = preview_text(card.get_str("summary").unwrap_or_default());
            let title = agent_title(&lookups.agents, source.agent_id.as_deref())
                .or_else(|| {
                    conversation
                        .get_str("title")
                        .ok()
                        .map(|value| label(value, 80))
                })
                .unwrap_or_else(|| "Decision needed".into());
            let conversation_id = source.conversation_id.clone().unwrap_or_default();
            let card_id = source.card_id.clone().unwrap_or_default();
            let url = match group {
                // Group cards are decided on the visible group page, never
                // through the hidden member thread.
                Some(group) => AssistantPage::Group { group: &group.id }.url(frontend),
                None => AssistantPage::ReviewCard {
                    conversation: &conversation_id,
                    card: &card_id,
                }
                .url(frontend),
            };
            Built::Item {
                item: Box::new(item(row, title.clone(), title, preview, url, resolved)),
                resolve,
            }
        }
        NotificationKind::AutomationResult | NotificationKind::AutomationProblem => {
            let Some(trigger) = source
                .trigger_id
                .as_deref()
                .and_then(|id| lookups.triggers.get(id))
            else {
                return Built::Purge;
            };
            // Ownership is evaluated here, never by a lookup filter: a
            // mismatch hides the row, only a missing document deletes it.
            if trigger.get_str("user_id").ok() != Some(user_id) {
                return Built::Hidden;
            }
            let run = source.run_id.as_deref().and_then(|id| lookups.runs.get(id));
            if run.is_none() && row.outcome.as_deref() != Some("paused") {
                return Built::Purge;
            }
            if let Some(run) = run
                && (run.get_str("user_id").ok() != Some(user_id)
                    || run.get_str("trigger_id").ok() != source.trigger_id.as_deref())
            {
                return Built::Hidden;
            }
            let title = run
                .and_then(|run| run.get_str("label").ok())
                .or_else(|| trigger.get_str("label").ok())
                .map(|value| label(value, 80))
                .unwrap_or_else(|| "Automation".into());
            let reason = row
                .reason
                .as_deref()
                .or_else(|| run.and_then(|run| run.get_str("reason").ok()));
            let preview = match row.outcome.as_deref() {
                Some("completed") => source
                    .conversation_id
                    .as_deref()
                    .zip(source.turn_id.as_deref())
                    .and_then(|(thread, turn)| lookups.replies.get(&format!("{thread}|{turn}")))
                    .map(message_preview)
                    .unwrap_or_else(|| "Completed.".into()),
                Some("paused") => format!(
                    "Automation paused: {}",
                    reason.map(reason_copy).unwrap_or_default()
                ),
                _ => reason
                    .map(reason_copy)
                    .unwrap_or_else(|| "The run did not complete.".into()),
            };
            let url = match source.conversation_id.as_deref() {
                Some(conversation) if lookups.conversations.contains_key(conversation) => {
                    AssistantPage::Conversation { conversation }.url(frontend)
                }
                _ => AssistantPage::Automations {
                    setup: None,
                    agent: source.agent_id.as_deref(),
                }
                .url(frontend),
            };
            ready(item(
                row,
                title.clone(),
                title,
                preview_text(&preview),
                url,
                resolved,
            ))
        }
    }
}

/// Build DTOs for `rows` (already selected for `user_id`), purging rows whose
/// source is gone and hiding rows the person can no longer see.
pub async fn resolve_items(
    state: &AppState,
    user_id: &str,
    rows: Vec<AssistantNotification>,
) -> AppResult<Vec<NotificationItem>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let db = &state.db;
    let lookups = load_lookups(db, user_id, &rows).await?;
    let now = Utc::now();
    let mut items = Vec::with_capacity(rows.len());
    let mut purge_ids = Vec::new();
    let mut resolve: HashMap<&'static str, Vec<String>> = HashMap::new();
    for row in &rows {
        match build(user_id, &state.config.frontend_url, row, &lookups, now) {
            Built::Item {
                item: built,
                resolve: label,
            } => {
                if let Some(label) = label {
                    resolve.entry(label).or_default().push(row.id.clone());
                }
                items.push(*built);
            }
            Built::Purge => purge_ids.push(row.id.clone()),
            Built::Hidden => {}
        }
    }
    if !purge_ids.is_empty() {
        db.collection::<Document>(COLLECTION_NAME)
            .delete_many(doc! {"_id": {"$in": purge_ids}, fields::USER_ID: user_id})
            .await?;
    }
    for (label, ids) in resolve {
        // The card is no longer pending (decided elsewhere, timed out or
        // withdrawn) and no hook recorded it.
        db.collection::<Document>(COLLECTION_NAME)
            .update_many(
                doc! {"_id": {"$in": ids}, fields::USER_ID: user_id,
                "resolved_at": bson::Bson::Null},
                resolution_update(now, label),
            )
            .await?;
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// Summary and read state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct SourceUnread {
    pub source_type: String,
    pub id: String,
    pub conversation_id: Option<String>,
    pub unread: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub unread_count: i64,
    pub needs_decision_count: i64,
    pub by_source: Vec<SourceUnread>,
    pub by_source_truncated: bool,
}

fn count_of(doc: &Document, key: &str) -> i64 {
    doc.get_i64(key)
        .or_else(|_| doc.get_i32(key).map(i64::from))
        .unwrap_or(0)
}

fn lookup_by_id(from: &str, local: &str, as_: &str, projection: Document) -> Document {
    doc! {"$lookup": {
        "from": from, "localField": local, "foreignField": "_id",
        "pipeline": [{"$project": projection}], "as": as_,
    }}
}

/// The summary's live access check, mirroring [`Lookups`] inside the single
/// aggregate: every referenced parent is joined by id and the person's
/// current membership is joined for organization-owned sources.
fn summary_pipeline(user_id: &str, now: bson::DateTime) -> Vec<Document> {
    let needs_decision = NotificationKind::NeedsDecision.as_str();
    let missing_or_null = |field: &str| doc! {"$in": [{"$type": field}, ["missing", "null"]]};
    let present = |array: &str| doc! {"$gt": [{"$size": array}, 0]};
    let org_active = present("$org_user");
    let member_ok = present("$membership");
    let admin_ok = doc! {"$gt": [{"$size": {"$filter": {
        "input": "$membership", "as": "m", "cond": {"$eq": ["$$m.role", "admin"]},
    }}}, 0]};
    let conv_ok = doc! {"$and": [
        present("$conv"),
        {"$or": [
            {"$eq": ["$conv0.user_id", user_id]},
            {"$eq": ["$conv0.collaboration.original_requester_id", user_id]},
        ]},
        {"$or": [
            missing_or_null("$conv0.agent_owner_id"),
            {"$eq": ["$conv0.agent_owner_id", user_id]},
            {"$and": [org_active.clone(), member_ok.clone()]},
        ]},
    ]};
    let participant = doc! {"$in": [user_id, {"$ifNull": ["$grp0.participant_user_ids", []]}]};
    // Mirrors `assistant_group_kind::classify`: an unknown marker version is
    // unsupported and never readable.
    let grp_ok = doc! {"$and": [
        present("$grp"),
        {"$switch": {"branches": [
            {"case": {"$not": [missing_or_null("$grp0.collaboration")]},
             "then": {"$and": [
                 {"$eq": ["$grp0.collaboration.version",
                          i64::from(crate::models::assistant_group_collaboration::VERSION)]},
                 participant.clone(),
             ]}},
            {"case": {"$not": [missing_or_null("$grp0.created_by_user_id")]},
             "then": {"$and": [participant, org_active.clone(), member_ok.clone()]}},
        ], "default": {"$eq": ["$grp0.user_id", user_id]}}},
    ]};
    // Linked platform chats: the thread still carries this exact binding.
    let platform_ok = doc! {"$and": [
        conv_ok.clone(),
        {"$not": [missing_or_null("$source.platform_binding_id")]},
        {"$eq": ["$conv0.platform_thread.binding_id", "$source.platform_binding_id"]},
        {"$eq": ["$conv0.platform_thread.binding_generation", "$source.platform_binding_generation"]},
        {"$eq": ["$conv0.platform_thread.user_id", user_id]},
    ]};
    let platform_bound = doc! {"$or": [
        missing_or_null("$source.platform_binding_id"),
        platform_ok.clone(),
    ]};
    let thread_ok = doc! {"$or": [
        missing_or_null("$source.thread_id"),
        present("$thread"),
    ]};
    let chan_ok = doc! {"$and": [
        present("$chan"),
        {"$in": ["$chan0.status", LIVE_CHANNEL_STATUSES.to_vec()]},
        thread_ok,
        {"$switch": {"branches": [
            {"case": {"$eq": ["$source.lane", LANE_ORG_GUEST]},
             "then": {"$and": [{"$eq": ["$chan0.user_id", user_id]}, org_active.clone(), admin_ok]}},
        ], "default": conv_ok.clone()}},
    ]};
    let run_ok = doc! {"$and": [
        present("$run"),
        {"$eq": ["$run0.user_id", user_id]},
        {"$eq": ["$run0.trigger_id", "$source.trigger_id"]},
    ]};
    let trig_ok = doc! {"$and": [
        present("$trig"),
        {"$eq": ["$trig0.user_id", user_id]},
        {"$or": [run_ok, {"$eq": ["$outcome", "paused"]}]},
    ]};
    let chan_live = doc! {"$and": [
        present("$chan"),
        {"$in": ["$chan0.status", LIVE_CHANNEL_STATUSES.to_vec()]},
    ]};
    let card_ok = doc! {"$and": [
        conv_ok.clone(),
        platform_bound,
        {"$or": [missing_or_null("$source.group_id"), grp_ok.clone()]},
        {"$or": [missing_or_null("$source.channel_id"), chan_live]},
        present("$card"),
        {"$eq": ["$card0.conversation_id", "$source.conversation_id"]},
        {"$eq": ["$card0.status", "pending"]},
        {"$gt": ["$card0.expires_at", now]},
    ]};
    let visible = doc! {"$switch": {"branches": [
        {"case": {"$eq": ["$kind", needs_decision]}, "then": card_ok},
        {"case": {"$in": ["$kind", [
            NotificationKind::AutomationResult.as_str(),
            NotificationKind::AutomationProblem.as_str(),
        ]]}, "then": trig_ok},
        {"case": {"$eq": ["$kind", NotificationKind::ChatUpdate.as_str()]},
         "then": {"$cond": [missing_or_null("$source.channel_id"), platform_ok, chan_ok]}},
        {"case": {"$eq": ["$kind", NotificationKind::GroupMention.as_str()]}, "then": grp_ok.clone()},
    ], "default": {"$cond": [missing_or_null("$source.group_id"), conv_ok, grp_ok]}}};
    let org_of = doc! {"$ifNull": [
        "$conv0.agent_owner_id",
        {"$ifNull": ["$grp0.user_id", "$chan0.org_specialist.org_id"]},
    ]};
    vec![
        doc! {"$match": {
            fields::USER_ID: user_id, "unread": true, "resolved_at": bson::Bson::Null,
            "$or": [
                {fields::KIND: {"$ne": needs_decision}},
                {"card_expires_at": {"$gt": now}},
            ],
        }},
        lookup_by_id(
            CONVERSATIONS,
            fields::SOURCE_CONVERSATION_ID,
            "conv",
            doc! {"user_id": 1, "agent_owner_id": 1, "collaboration.original_requester_id": 1,
            "platform_thread.binding_id": 1, "platform_thread.binding_generation": 1,
            "platform_thread.user_id": 1},
        ),
        lookup_by_id(
            GROUPS,
            fields::SOURCE_GROUP_ID,
            "grp",
            doc! {"user_id": 1, "participant_user_ids": 1, "collaboration": 1,
            "created_by_user_id": 1},
        ),
        lookup_by_id(
            CHANNELS,
            fields::SOURCE_CHANNEL_ID,
            "chan",
            doc! {"user_id": 1, "status": 1, "org_specialist.org_id": 1},
        ),
        lookup_by_id(
            CHANNEL_THREADS,
            fields::SOURCE_THREAD_ID,
            "thread",
            doc! {"_id": 1},
        ),
        lookup_by_id(
            TRIGGERS,
            fields::SOURCE_TRIGGER_ID,
            "trig",
            doc! {"user_id": 1},
        ),
        lookup_by_id(
            RUNS,
            fields::SOURCE_RUN_ID,
            "run",
            doc! {"user_id": 1, "trigger_id": 1},
        ),
        lookup_by_id(
            ACKS,
            fields::SOURCE_CARD_ID,
            "card",
            doc! {"status": 1, "expires_at": 1, "conversation_id": 1},
        ),
        doc! {"$addFields": {
            "conv0": {"$arrayElemAt": ["$conv", 0]},
            "grp0": {"$arrayElemAt": ["$grp", 0]},
            "chan0": {"$arrayElemAt": ["$chan", 0]},
            "trig0": {"$arrayElemAt": ["$trig", 0]},
            "run0": {"$arrayElemAt": ["$run", 0]},
            "card0": {"$arrayElemAt": ["$card", 0]},
        }},
        doc! {"$addFields": {"org_of": org_of}},
        doc! {"$lookup": {
            "from": ORG_MEMBERSHIPS, "let": {"org": "$org_of"},
            "pipeline": [
                {"$match": {"$expr": {"$and": [
                    {"$eq": ["$org_user_id", "$$org"]},
                    {"$eq": ["$member_user_id", user_id]},
                    {"$in": [{"$type": "$revoked_at"}, ["missing", "null"]]},
                    {"$in": ["$role", MEMBER_ROLES.to_vec()]},
                ]}}},
                {"$project": {"role": 1}},
            ],
            "as": "membership",
        }},
        doc! {"$lookup": {
            "from": USERS, "let": {"org": "$org_of"},
            "pipeline": [
                {"$match": {"$expr": {"$and": [
                    {"$eq": ["$_id", "$$org"]},
                    {"$eq": ["$is_active", true]},
                    {"$eq": ["$user_type", "org"]},
                ]}}},
                {"$project": {"_id": 1}},
            ],
            "as": "org_user",
        }},
        doc! {"$addFields": {"visible": visible}},
        doc! {"$match": {"visible": true}},
        doc! {"$facet": {
            "totals": [{"$group": {
                "_id": bson::Bson::Null,
                "unread": {"$sum": 1},
                "needs_decision": {"$sum": {"$cond": [{"$eq": ["$kind", needs_decision]}, 1, 0]}},
            }}],
            "by_source": [
                {"$group": {
                    "_id": {"type": "$source.type", "id": "$source.id"},
                    "conversation_id": {"$first": "$source.conversation_id"},
                    "unread": {"$sum": 1},
                }},
                {"$sort": {"unread": -1, "_id.id": 1}},
                {"$limit": SUMMARY_SOURCES as i64 + 1},
            ],
        }},
    ]
}

/// One aggregate: unseen active rows (timed-out cards excluded) whose source
/// the person can still see, unseen pending decisions, and a bounded
/// per-source breakdown for sidebar dots.
pub async fn summary(db: &Database, user_id: &str) -> AppResult<Summary> {
    let now = bson_date(Utc::now());
    let mut cursor = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate(summary_pipeline(user_id, now))
        .await?;
    let Some(result) = cursor.try_next().await? else {
        return Ok(Summary {
            unread_count: 0,
            needs_decision_count: 0,
            by_source: Vec::new(),
            by_source_truncated: false,
        });
    };
    let totals = result
        .get_array("totals")
        .ok()
        .and_then(|rows| rows.first())
        .and_then(bson::Bson::as_document);
    let mut by_source: Vec<SourceUnread> = result
        .get_array("by_source")
        .ok()
        .map(|rows| {
            rows.iter()
                .filter_map(bson::Bson::as_document)
                .filter_map(|row| {
                    let key = row.get_document("_id").ok()?;
                    Some(SourceUnread {
                        source_type: key.get_str("type").ok()?.to_owned(),
                        id: key.get_str("id").ok()?.to_owned(),
                        conversation_id: row.get_str("conversation_id").ok().map(str::to_owned),
                        unread: count_of(row, "unread"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let by_source_truncated = by_source.len() > SUMMARY_SOURCES;
    by_source.truncate(SUMMARY_SOURCES);
    Ok(Summary {
        unread_count: totals.map_or(0, |doc| count_of(doc, "unread")),
        needs_decision_count: totals.map_or(0, |doc| count_of(doc, "needs_decision")),
        by_source,
        by_source_truncated,
    })
}

/// Which rows a read marks.
#[derive(Clone, Debug)]
pub enum ReadScope {
    Ids(Vec<String>),
    Source {
        source_type: SourceType,
        id: String,
    },
    /// Everything that was already there when the person looked.
    All {
        before: DateTime<Utc>,
    },
}

/// Mark rows seen. Idempotent; decision rows stay listed until resolved.
pub async fn mark_read(db: &Database, user_id: &str, scope: ReadScope) -> AppResult<u64> {
    let now = Utc::now();
    let mut filter = doc! {fields::USER_ID: user_id, "unread": true};
    match scope {
        ReadScope::Ids(ids) => {
            if ids.is_empty() || ids.len() > MAX_READ_IDS {
                return Err(AppError::BadRequest(format!(
                    "ids must contain between 1 and {MAX_READ_IDS} entries"
                )));
            }
            filter.insert("_id", doc! {"$in": ids});
        }
        ReadScope::Source { source_type, id } => {
            let mut matches = vec![doc! {
                fields::SOURCE_TYPE: source_type.as_str(), fields::SOURCE_ID: &id,
            }];
            match source_type {
                // Channel and automation rows of that thread are read with it.
                SourceType::Conversation => {
                    matches.push(doc! {fields::SOURCE_CONVERSATION_ID: &id});
                }
                // One group id covers personal, organization and collaboration rows.
                SourceType::Group | SourceType::CollaborationGroup => {
                    matches.push(doc! {fields::SOURCE_GROUP_ID: &id});
                }
                SourceType::ChannelChat | SourceType::Trigger => {}
            }
            filter.insert("$or", matches);
        }
        ReadScope::All { before } => {
            filter.insert("updated_at", doc! {"$lte": bson_date(before)});
        }
    }
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .update_many(
            filter,
            doc! {"$set": {"unread": false, "read_at": bson_date(now)}},
        )
        .await?
        .modified_count)
}

// ---------------------------------------------------------------------------
// Sweep: expired decisions and the durable push worker
// ---------------------------------------------------------------------------

pub fn push_preferred(preferences: &NotificationPushPreferences, kind: NotificationKind) -> bool {
    match kind {
        NotificationKind::AutomationResult => preferences.automation_result,
        NotificationKind::AutomationProblem => preferences.automation_problem,
        NotificationKind::NeedsDecision => preferences.needs_decision,
        NotificationKind::AgentUpdate => preferences.agent_update,
        NotificationKind::ChatUpdate => preferences.chat_update,
        NotificationKind::GroupMention => preferences.group_mention,
    }
}

fn push_source_slot(user_id: &str, source_type: SourceType, source_id: &str) -> String {
    format!("{user_id}\u{1f}{}\u{1f}{source_id}", source_type.as_str())
}

/// Compare-and-set the push barrier on a queued or claimed row.
async fn push_transition(
    db: &Database,
    id: &str,
    from: &[&str],
    attempt: Option<&str>,
    set: Document,
) -> AppResult<bool> {
    let mut filter = doc! {"_id": id, "push.state": {"$in": from}};
    if let Some(attempt) = attempt {
        filter.insert("push.attempt", attempt);
    }
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(filter, doc! {"$set": set})
        .await?
        .matched_count
        == 1)
}

async fn push_skip(db: &Database, id: &str, code: &str) -> AppResult<()> {
    push_transition(
        db,
        id,
        &["pending", "retrying"],
        None,
        doc! {"push.state": "skipped", "push.code": code},
    )
    .await
    .map(|_| ())
}

/// A row claimed for one send: the authoritative post-claim snapshot and
/// the attempt token every later barrier write must present.
#[derive(Clone, Debug)]
pub(crate) struct ClaimedPush {
    pub(crate) row: AssistantNotification,
    pub(crate) attempt: String,
}

pub(crate) enum Claim {
    Claimed(Box<ClaimedPush>),
    /// The row was settled inside the claim (seen, capped or exhausted).
    Skipped,
    /// Not due, already claimed by another replica, or gone.
    NotClaimed,
}

/// What a send attempt produced; `Unavailable` is a definite failure before
/// any provider call.
pub(crate) enum PushOutcome {
    Accepted,
    Refused(automation::Failure),
    Unavailable(&'static str),
    Unknown,
}

fn push_retry_window(first_attempt_at: DateTime<Utc>) -> DateTime<Utc> {
    first_attempt_at + Duration::minutes(PUSH_RETRY_WINDOW_MINUTES)
}

/// Claim one row for sending. The caller's snapshot only names the row: the
/// transaction re-reads it and decides from the current state, so a stale
/// snapshot can never send early. Due time, the attempt limit and the retry
/// window are enforced here; the erasure fence, the per-source slot, the
/// rolling hourly budget and the row's `sending` barrier commit together, so
/// neither a concurrent replica nor a concurrent erasure can double-send or
/// recreate budget records after a purge.
pub(crate) async fn claim_push(db: &Database, row: &AssistantNotification) -> AppResult<Claim> {
    let mut session = db.client().start_session().await?;
    let work_db = db.clone();
    let row_id = row.id.clone();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &work_db;
            let operation: AppResult<Claim> = async {
                let now = Utc::now();
                let attempt = Uuid::new_v4().to_string();
                // Due now: queued, or a retry whose time has come.
                let Some(current) = db
                    .collection::<AssistantNotification>(COLLECTION_NAME)
                    .find_one(doc! {"_id": &row_id, "$or": [
                        {"push.state": "pending"},
                        {"push.state": "retrying", "push.retry_at": {"$lte": bson_date(now)}},
                    ]})
                    .session(&mut *session)
                    .await?
                else {
                    return Ok(Claim::NotClaimed);
                };
                crate::services::erasure_admission::fence_in_session(db, &current.user_id, session)
                    .await?;
                let rows = db.collection::<Document>(COLLECTION_NAME);
                let settle =
                    |set: Document| rows.update_one(doc! {"_id": &current.id}, doc! {"$set": set});
                if current.resolved_at.is_some() || !current.unread {
                    settle(doc! {"push.state": "skipped", "push.code": "already_seen"})
                        .session(&mut *session)
                        .await?;
                    return Ok(Claim::Skipped);
                }
                let push = current
                    .push
                    .clone()
                    .unwrap_or_else(|| push_state(&PushPlan::Pending, current.kind));
                let first_attempt_at = push.first_attempt_at.unwrap_or(now);
                if push.attempts >= PUSH_MAX_ATTEMPTS || now >= push_retry_window(first_attempt_at)
                {
                    settle(
                        doc! {"push.state": "failed", "push.code": "push_retries_exhausted",
                        "push.retry_at": bson::Bson::Null},
                    )
                    .session(&mut *session)
                    .await?;
                    return Ok(Claim::Skipped);
                }
                let previous_attempt = push.attempt.as_deref();
                // Per visible source (person, source type, source id), whatever the kinds.
                let slot = push_source_slot(
                    &current.user_id,
                    current.source.source_type,
                    &current.source.id,
                );
                let cutoff = now - Duration::minutes(PUSH_SOURCE_INTERVAL_MINUTES);
                let taken = db
                    .collection::<Document>(PUSH_SOURCES)
                    .find_one(doc! {"_id": &slot})
                    .session(&mut *session)
                    .await?
                    .is_some_and(|existing| {
                        existing
                            .get_datetime("sent_at")
                            .is_ok_and(|at| at.to_chrono() > cutoff)
                            && existing.get_str("attempt").ok() != previous_attempt
                    });
                if taken {
                    settle(doc! {"push.state": "skipped", "push.code": "source_rate"})
                        .session(&mut *session)
                        .await?;
                    return Ok(Claim::Skipped);
                }
                // Rolling hour: reservations are timestamps kept on one document.
                let hour_ago = bson_date(now - Duration::hours(1));
                let budget = db
                    .collection::<Document>(PUSH_BUDGETS)
                    .find_one(doc! {"_id": &current.user_id})
                    .session(&mut *session)
                    .await?;
                let mut reservations: Vec<Document> = budget
                    .as_ref()
                    .and_then(|budget| budget.get_array("reservations").ok())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(bson::Bson::as_document)
                            .filter(|item| {
                                item.get_datetime("at").is_ok_and(|at| *at > hour_ago)
                                    && item.get_str("attempt").ok() != previous_attempt
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if reservations.len() >= PUSH_HOURLY_LIMIT {
                    settle(doc! {"push.state": "skipped", "push.code": "hourly_limit"})
                        .session(&mut *session)
                        .await?;
                    return Ok(Claim::Skipped);
                }
                reservations.push(doc! {"at": bson_date(now), "attempt": &attempt});
                db.collection::<Document>(PUSH_BUDGETS)
                    .update_one(
                        doc! {"_id": &current.user_id},
                        doc! {"$set": {"reservations": reservations}},
                    )
                    .upsert(true)
                    .session(&mut *session)
                    .await?;
                db.collection::<Document>(PUSH_SOURCES)
                    .update_one(
                        doc! {"_id": &slot},
                        doc! {"$set": {"sent_at": bson_date(now), "user_id": &current.user_id,
                        "attempt": &attempt}},
                    )
                    .upsert(true)
                    .session(&mut *session)
                    .await?;
                let attempts = push.attempts + 1;
                settle(doc! {
                    "push.state": "sending", "push.attempt": &attempt,
                    "push.claimed_at": bson_date(now), "push.attempts": i64::from(attempts),
                    "push.first_attempt_at": bson_date(first_attempt_at),
                    "push.retry_at": bson::Bson::Null,
                })
                .session(&mut *session)
                .await?;
                let mut claimed = current;
                claimed.push = Some(PushState {
                    state: "sending".into(),
                    kind: push.kind,
                    attempt: Some(attempt.clone()),
                    code: push.code,
                    attempts,
                    claimed_at: Some(now),
                    first_attempt_at: Some(first_attempt_at),
                    retry_at: None,
                    sent_at: push.sent_at,
                });
                Ok(Claim::Claimed(Box::new(ClaimedPush {
                    row: claimed,
                    attempt,
                })))
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Give back the reservations of a definite non-send (never after an
/// accepted or ambiguous send), so the same row's retry is not throttled by
/// its own earlier attempt.
async fn release_reservations(db: &Database, claimed: &ClaimedPush) -> AppResult<()> {
    let row = &claimed.row;
    db.collection::<Document>(PUSH_SOURCES)
        .delete_one(doc! {
            "_id": push_source_slot(&row.user_id, row.source.source_type, &row.source.id),
            "attempt": &claimed.attempt,
        })
        .await?;
    db.collection::<Document>(PUSH_BUDGETS)
        .update_one(
            doc! {"_id": &row.user_id},
            doc! {"$pull": {"reservations": {"attempt": &claimed.attempt}}},
        )
        .await?;
    Ok(())
}

/// Settle a claimed row without sending (definite skip): the barrier moves
/// `sending -> skipped` under the attempt token and the reservations return.
pub(crate) async fn skip_claimed(
    db: &Database,
    claimed: &ClaimedPush,
    code: &'static str,
) -> AppResult<()> {
    let settled = push_transition(
        db,
        &claimed.row.id,
        &["sending"],
        Some(&claimed.attempt),
        doc! {"push.state": "skipped", "push.code": code},
    )
    .await?;
    if settled {
        release_reservations(db, claimed).await?;
    }
    Ok(())
}

/// Settle the claimed row after the provider answered. Attempts and the retry
/// window come from the claimed snapshot, never from an older read. Only a
/// conclusive retryable refusal retries, at least a minute apart, at most
/// [`PUSH_MAX_ATTEMPTS`] attempts, and never past the window after the first
/// attempt; ambiguous outcomes keep their reservations and are never resent.
pub(crate) async fn settle_push(
    db: &Database,
    claimed: &ClaimedPush,
    outcome: PushOutcome,
) -> AppResult<()> {
    let now = Utc::now();
    let push = claimed.row.push.as_ref();
    let attempts = push.map_or(1, |push| push.attempts);
    let first_attempt_at = push.and_then(|push| push.first_attempt_at).unwrap_or(now);
    let (set, release) = match outcome {
        PushOutcome::Accepted => (
            doc! {"push.state": "sent", "push.sent_at": bson_date(now)},
            false,
        ),
        PushOutcome::Refused(failure) => {
            let retry_at = failure
                .retry_after
                .map(|delay| now + Duration::seconds((delay as i64).max(PUSH_RETRY_MIN_SECONDS)));
            match retry_at.filter(|retry_at| {
                attempts < PUSH_MAX_ATTEMPTS && *retry_at <= push_retry_window(first_attempt_at)
            }) {
                Some(retry_at) => (
                    doc! {"push.state": "retrying", "push.code": failure.code,
                    "push.retry_at": bson_date(retry_at)},
                    true,
                ),
                None => (
                    doc! {"push.state": "failed", "push.code": if failure.retry_after.is_some() {
                        "push_retries_exhausted"
                    } else {
                        failure.code
                    }},
                    true,
                ),
            }
        }
        PushOutcome::Unavailable(code) => (doc! {"push.state": "failed", "push.code": code}, true),
        PushOutcome::Unknown => (
            doc! {"push.state": "unknown", "push.code": "push_delivery_unknown"},
            false,
        ),
    };
    let settled = push_transition(
        db,
        &claimed.row.id,
        &["sending"],
        Some(&claimed.attempt),
        set,
    )
    .await?;
    if settled && release {
        release_reservations(db, claimed).await?;
    }
    Ok(())
}

/// Resolve timed-out decision rows and send queued pushes with the real transport.
pub async fn sweep(state: &AppState) -> AppResult<()> {
    sweep_with(state, |user, title, body, data| async move {
        send_push(state, &user, &title, &body, data).await
    })
    .await
}

async fn send_push(
    state: &AppState,
    user_id: &str,
    title: &str,
    body: &str,
    data: HashMap<String, String>,
) -> Result<automation::Outcome, &'static str> {
    let channel = match automation::prepare(
        &state.db,
        &state.config,
        state.fcm_auth.is_some(),
        state.apns_auth.is_some(),
        user_id,
    )
    .await
    {
        Ok(Ok(channel)) => channel,
        Ok(Err(failure)) => return Err(failure.code),
        Err(_) => return Err("push_not_set_up"),
    };
    Ok(automation::send_data(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &channel,
        title,
        body,
        data,
    )
    .await)
}

/// The kind a row is pushed as: the creating kind frozen on the barrier,
/// whatever a later coalesce relabelled the row to.
fn push_kind(row: &AssistantNotification) -> NotificationKind {
    row.push
        .as_ref()
        .and_then(|push| push.kind)
        .unwrap_or(row.kind)
}

/// The sweep with an injectable sender: `Err(code)` is a definite
/// availability failure before any send. Rows are claimed one at a time;
/// everything after the claim (flag, preference, access, preview, payload)
/// uses the claimed snapshot, resolved once per person.
pub async fn sweep_with<F, Fut>(state: &AppState, send: F) -> AppResult<()>
where
    F: Fn(String, String, String, HashMap<String, String>) -> Fut,
    Fut: std::future::Future<Output = Result<automation::Outcome, &'static str>>,
{
    let db = &state.db;
    let now = Utc::now();
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(
            doc! {fields::KIND: NotificationKind::NeedsDecision.as_str(),
            "resolved_at": bson::Bson::Null, "card_expires_at": {"$lte": bson_date(now)}},
            resolution_update(now, "expired"),
        )
        .await?;
    // An ambiguous claim is never resent.
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(
            doc! {"push.state": "sending",
            "push.claimed_at": {"$lte": bson_date(now - Duration::minutes(PUSH_SENDING_STALE_MINUTES))}},
            doc! {"$set": {"push.state": "unknown", "push.code": "push_delivery_unknown"}},
        )
        .await?;
    let pending: Vec<AssistantNotification> = db
        .collection(COLLECTION_NAME)
        .find(doc! {"$or": [
            {"push.state": "pending"},
            {"push.state": "retrying", "push.retry_at": {"$lte": bson_date(now)}},
        ]})
        .sort(doc! {"created_at": 1})
        .limit(PUSH_BATCH)
        .await?
        .try_collect()
        .await?;
    let mut claims: Vec<(String, Vec<ClaimedPush>)> = Vec::new();
    for row in pending {
        // Cheap pre-check on the snapshot; the claim re-reads and decides.
        if row.resolved_at.is_some() || !row.unread {
            push_skip(db, &row.id, "already_seen").await?;
            continue;
        }
        let claimed = match claim_push(db, &row).await? {
            Claim::Claimed(claimed) => *claimed,
            Claim::Skipped | Claim::NotClaimed => continue,
        };
        match claims
            .iter()
            .position(|(user, _)| *user == claimed.row.user_id)
        {
            Some(index) => claims[index].1.push(claimed),
            None => claims.push((claimed.row.user_id.clone(), vec![claimed])),
        }
    }
    for (user, claims) in claims {
        if !self::enabled(db, &user).await? {
            for claimed in &claims {
                skip_claimed(db, claimed, "flag_off").await?;
            }
            continue;
        }
        let preference = crate::services::assistant_settings_service::get(db, &user)
            .await?
            .notifications
            .push;
        let mut wanted = Vec::with_capacity(claims.len());
        let mut views = Vec::with_capacity(claims.len());
        for claimed in claims {
            let kind = push_kind(&claimed.row);
            if !push_preferred(&preference, kind) {
                skip_claimed(db, &claimed, "preference_off").await?;
                continue;
            }
            // Title and preview follow the pushed kind with the latest refs.
            let mut view = claimed.row.clone();
            view.kind = kind;
            views.push(view);
            wanted.push(claimed);
        }
        if wanted.is_empty() {
            continue;
        }
        // One batched lookup for everything this person is owed.
        let built: HashMap<String, NotificationItem> = resolve_items(state, &user, views)
            .await?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        for claimed in wanted {
            let row = &claimed.row;
            let Some(built) = built.get(&row.id) else {
                // Purged or hidden while queued.
                skip_claimed(db, &claimed, "source_unavailable").await?;
                continue;
            };
            let kind = push_kind(row);
            // `type` is the closed kind: mobile refreshes the matching view on receipt.
            let mut data = HashMap::from([
                ("type".to_owned(), kind.as_str().to_owned()),
                ("kind".to_owned(), kind.as_str().to_owned()),
                ("notification_id".to_owned(), row.id.clone()),
                ("deeplink".to_owned(), built.url.clone()),
            ]);
            if let Some(conversation) = row.source.conversation_id.as_ref() {
                data.insert("conversation_id".to_owned(), conversation.clone());
            }
            let body = if built.preview.is_empty() {
                built.title.clone()
            } else {
                built.preview.clone()
            };
            let outcome = match tokio::time::timeout(
                PUSH_SEND_TIMEOUT,
                send(user.clone(), built.title.clone(), body, data),
            )
            .await
            {
                Ok(Ok(automation::Outcome::Accepted)) => PushOutcome::Accepted,
                Ok(Ok(automation::Outcome::Refused(failure))) => PushOutcome::Refused(failure),
                Ok(Err(code)) => PushOutcome::Unavailable(code),
                Ok(Ok(automation::Outcome::Unknown)) | Err(_) => PushOutcome::Unknown,
            };
            settle_push(db, &claimed, outcome).await?;
            tracing::debug!(
                target: "nyxid::assistant_notifications",
                notification_id = %row.id, kind = kind.as_str(), "Notification push settled"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "assistant_notification_service_tests.rs"]
mod tests;
