//! NyxBot Notifications behavioral tests against real MongoDB transitions.
use super::*;
use crate::{
    models::{
        assistant_conversation::{
            ChannelOrigin, PlatformOriginSource, PlatformThread, PlatformTurnOrigin,
        },
        assistant_settings::{NotificationPreferences, NotificationPushPreferences},
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        platform_entry_projection::{StoredEntryIdentity, StoredSenderIdentity},
        trigger::{COLLECTION_NAME as TRIGGER_ROWS, Trigger, TriggerDelivery, TriggerVerification},
        trigger_schedule::{
            IntervalUnit, OverlapPolicy, ScheduleKind, ScheduleSpec, TriggerSource,
        },
        user::{COLLECTION_NAME as USER_ROWS, UserType},
    },
    services::{
        assistant_acknowledgement_service as acks, assistant_group_service as groups,
        assistant_nyxagent::{self as engine, TurnRequest, TurnStart},
        assistant_settings_service as settings,
        assistant_team_service::{self as team, CreateRequest},
        feature_flag_service::{self as flags, FlagTarget},
        trigger_schedule as schedules, trigger_service,
    },
    test_utils::{
        connect_transaction_test_database, connect_transaction_test_database_with_command_handler,
        test_app_state, test_membership, test_user,
    },
};
use mongodb::event::{EventHandler, command::CommandEvent};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    state: AppState,
    owner: String,
}

async fn fixture_with(db: Database) -> Fixture {
    engine::ensure_indexes(&db).await.unwrap();
    ensure_indexes(&db).await.unwrap();
    schedules::indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(USER_ROWS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    Fixture {
        state: test_app_state(db),
        owner,
    }
}

async fn fixture(name: &str) -> Fixture {
    fixture_with(connect_transaction_test_database(name).await).await
}

async fn person(f: &Fixture) -> String {
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(USER_ROWS)
        .insert_one(test_user(&id, UserType::Person))
        .await
        .unwrap();
    id
}

async fn set_flag(f: &Fixture, on: bool) {
    flags::set_platform_override(&f.state.db, FLAG_KEY, &FlagTarget::Global, on, &f.owner)
        .await
        .unwrap();
    reset_flag_cache(&f.state.db);
}

fn request(conversation: Option<&str>, text: &str) -> TurnRequest {
    TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        conversation_id: conversation.map(str::to_owned),
        agent_id: None,
        text: text.into(),
        model: None,
        access_mode: None,
    }
}

fn start(conversation: Option<&str>, text: &str, origin: TurnOrigin) -> TurnStart {
    let mut start = TurnStart::from(&request(conversation, text));
    start.origin = origin;
    start
}

async fn begin(f: &Fixture, owner: &str, start: &TurnStart) -> AssistantConversation {
    engine::begin_turn(&f.state.db, owner, start, &f.state.encryption_keys)
        .await
        .unwrap()
}

/// Settle through the real `finish_turn` path; returns the reply message id.
async fn settle(
    f: &Fixture,
    row: &AssistantConversation,
    text: &str,
    error: Option<TurnError>,
) -> String {
    let (message_id, result) = try_settle(f, row, text, error).await;
    result.unwrap();
    message_id
}

/// Settle with the thread's own key id and return the engine's verdict (a
/// destroyed agent's late settlement may be refused).
async fn try_settle(
    f: &Fixture,
    row: &AssistantConversation,
    text: &str,
    error: Option<TurnError>,
) -> (String, AppResult<Option<TurnError>>) {
    let message_id = Uuid::new_v4().to_string();
    let result = engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &message_id,
        &engine::TurnResult {
            text: text.into(),
            session_id: Some("conv_session".into()),
            response_id: Some("resp_1".into()),
            error,
        },
    )
    .await;
    (message_id, result)
}

async fn rows_of(f: &Fixture, user: &str) -> Vec<AssistantNotification> {
    f.state
        .db
        .collection::<AssistantNotification>(COLLECTION_NAME)
        .find(doc! {"user_id": user})
        .sort(doc! {"created_at": 1, "_id": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn specialist(f: &Fixture, name: &str) -> crate::models::assistant_agent::AssistantAgent {
    team::create_specialist_for(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.owner,
        CreateRequest {
            machines: None,
            logins: None,
            name: name.into(),
            description: "Summarizes notes".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap()
    .0
}

fn page(limit: usize) -> ListQuery {
    ListQuery {
        limit,
        ..Default::default()
    }
}

/// A content-free feed row for read/summary/push tests whose parents are
/// seeded as plain documents.
fn seeded(user: &str, kind: NotificationKind, source: NotificationSource) -> AssistantNotification {
    let now = Utc::now();
    AssistantNotification {
        id: Uuid::new_v4().to_string(),
        user_id: user.into(),
        kind,
        source_key: source_key(&source),
        source,
        coalesced: kind.coalesced(),
        unread: true,
        read_at: None,
        resolved_at: None,
        resolution: None,
        card_expires_at: None,
        count: 1,
        outcome: Some("completed".into()),
        reason: None,
        push: Some(push_state(&PushPlan::Pending, kind)),
        created_at: now,
        updated_at: now,
        expires_at: now + Duration::days(RETENTION_DAYS),
    }
}

async fn insert_row(f: &Fixture, row: &AssistantNotification) {
    f.state
        .db
        .collection::<AssistantNotification>(COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
}

async fn seed_conversation(f: &Fixture, owner: &str, title: &str) -> String {
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .insert_one(doc! {"_id": &id, "user_id": owner, "title": title})
        .await
        .unwrap();
    id
}

fn conversation_source(id: &str) -> NotificationSource {
    NotificationSource {
        source_type: SourceType::Conversation,
        id: id.into(),
        conversation_id: Some(id.into()),
        ..Default::default()
    }
}

async fn push_prefs(f: &Fixture, user: &str, push: NotificationPushPreferences) {
    settings::update(
        &f.state.db,
        user,
        settings::Update {
            notifications: Some(NotificationPreferences { push }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

fn all_push() -> NotificationPushPreferences {
    NotificationPushPreferences {
        automation_result: true,
        automation_problem: true,
        needs_decision: true,
        agent_update: true,
        chat_update: true,
        group_mention: true,
    }
}

// ---------------------------------------------------------------------------
// Turn settlement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn background_turns_coalesce_until_read_while_user_turns_and_flag_off_produce_nothing() {
    let f = fixture("notif_turns").await;
    let db = &f.state.db;
    // Flag off: the admission snapshot is false, settlement writes nothing.
    let row = begin(
        &f,
        &f.owner,
        &start(None, "First question", TurnOrigin::User),
    )
    .await;
    settle(&f, &row, "Answer", None).await;
    set_flag(&f, true).await;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    // The owner is in the app: a user-origin turn never notifies.
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "Hello", TurnOrigin::User),
    )
    .await;
    settle(&f, &row, "Hi", None).await;
    assert!(rows_of(&f, &f.owner).await.is_empty());
    // NyxBot assigned work to a specialist: its reply is an agent update.
    let researcher = specialist(&f, "researcher").await;
    let thread = team::home_thread(db, &f.state.encryption_keys, &researcher)
        .await
        .unwrap();
    let row = begin(
        &f,
        &f.owner,
        &start(
            Some(&thread.id),
            "Summarise the notes",
            TurnOrigin::Orchestrator,
        ),
    )
    .await;
    assert_eq!(
        row.active_turn.as_ref().unwrap().notifications_enabled,
        Some(true)
    );
    let first_reply = settle(&f, &row, "# Summary\n\n- one\n- **two**", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    let first = &rows[0];
    assert_eq!(first.kind, NotificationKind::AgentUpdate);
    assert_eq!(first.source.source_type, SourceType::Conversation);
    assert_eq!(first.source.id, thread.id);
    assert_eq!(
        first.source.conversation_id.as_deref(),
        Some(thread.id.as_str())
    );
    assert_eq!(
        first.source.agent_id.as_deref(),
        Some(researcher.id.as_str())
    );
    assert_eq!(
        first.source.message_id.as_deref(),
        Some(first_reply.as_str())
    );
    assert_eq!(first.count, 1);
    assert!(first.unread && first.coalesced);
    assert_eq!(first.push.as_ref().unwrap().state, "pending");
    // More activity on the same unread source increments the same row.
    let row = begin(
        &f,
        &f.owner,
        &start(
            Some(&thread.id),
            "And the follow-up",
            TurnOrigin::Orchestrator,
        ),
    )
    .await;
    let second_reply = settle(&f, &row, "Done 2", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, first.id);
    assert_eq!(rows[0].count, 2);
    assert_eq!(
        rows[0].source.message_id.as_deref(),
        Some(second_reply.as_str())
    );
    // Listing renders a single-line markdown-free preview and a thread link.
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].preview, "Done 2");
    assert_eq!(listed.items[0].title, "researcher");
    assert_eq!(listed.items[0].count, 2);
    assert!(listed.items[0].url.contains(&format!("c={}", thread.id)));
    assert!(!listed.items[0].read && !listed.items[0].resolved);
    // Read: the next activity opens a new row.
    assert_eq!(
        mark_read(db, &f.owner, ReadScope::Ids(vec![first.id.clone()]))
            .await
            .unwrap(),
        1
    );
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&thread.id), "Again", TurnOrigin::Orchestrator),
    )
    .await;
    settle(&f, &row, "", Some(TurnError::new("assistant_unavailable"))).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 2);
    assert!(!rows[0].unread && rows[0].read_at.is_some());
    let failed = &rows[1];
    assert!(failed.unread);
    assert_eq!(failed.outcome.as_deref(), Some("failed"));
    assert_eq!(failed.reason.as_deref(), Some("assistant_unavailable"));
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 2);
    assert_eq!(
        listed.items[0].preview,
        preview_text(&TurnError::new("assistant_unavailable").message)
    );
    let summary = summary(db, &f.owner).await.unwrap();
    assert_eq!(summary.unread_count, 1);
    assert_eq!(summary.needs_decision_count, 0);
    assert_eq!(summary.by_source.len(), 1);
    assert_eq!(summary.by_source[0].id, thread.id);
    assert_eq!(
        summary.by_source[0].conversation_id.as_deref(),
        Some(thread.id.as_str())
    );
    // A wake-up event turn on NyxBot's home is an update too.
    engine::push_events(
        db,
        &f.owner,
        &home.id,
        vec![team::event(
            "message",
            "The researcher finished".into(),
            None,
        )],
    )
    .await
    .unwrap();
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "wake", TurnOrigin::Event),
    )
    .await;
    settle(&f, &row, "Noted the result", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].source.id, home.id);
    assert_eq!(rows[2].source.agent_id.as_deref(), Some(nyxbot.id.as_str()));
}

#[tokio::test]
async fn turn_lost_reclaim_records_a_failed_update_and_destroyed_agents_stop_updates() {
    let f = fixture("notif_turn_lost").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let researcher = specialist(&f, "researcher").await;
    let thread = team::home_thread(db, &f.state.encryption_keys, &researcher)
        .await
        .unwrap();
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&thread.id), "Long task", TurnOrigin::Orchestrator),
    )
    .await;
    // Expire the lease: the next send reclaims the turn as `turn_lost`.
    db.collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &thread.id},
            doc! {"$set": {
                "active_turn.started_at": bson_date(Utc::now() - Duration::seconds(engine::ACTIVE_TURN_TTL_SECS + 60)),
                "active_turn.lease_expires_at": bson::Bson::Null,
                "active_turn.heartbeat_at": bson::Bson::Null,
            }},
        )
        .await
        .unwrap();
    let next = begin(
        &f,
        &f.owner,
        &start(Some(&thread.id), "Next", TurnOrigin::User),
    )
    .await;
    assert_ne!(
        next.active_turn.as_ref().unwrap().turn_id,
        row.active_turn.as_ref().unwrap().turn_id
    );
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::AgentUpdate);
    assert_eq!(rows[0].outcome.as_deref(), Some("failed"));
    assert_eq!(rows[0].reason.as_deref(), Some("turn_lost"));
    let prior_reply = settle(&f, &next, "ok", None).await;
    // Destroying the agent stops further updates for its threads.
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&thread.id), "More", TurnOrigin::Orchestrator),
    )
    .await;
    team::destroy(db, &f.owner, &researcher.id).await.unwrap();
    assert!(
        rows_of(&f, &f.owner).await.is_empty(),
        "destroy purges the agent's rows"
    );
    // Destroy revoked the thread's key and asked the turn to stop: whether the
    // late settlement is refused or recorded as stopped, no update appears.
    let (late_reply, verdict) =
        try_settle(&f, &row, "late", Some(TurnError::new("cancelled"))).await;
    match verdict {
        Ok(Some(error)) => assert_eq!(error.code, "cancelled"),
        Err(AppError::NotFound(_) | AppError::AssistantTurnRequired) => {}
        other => panic!("unexpected late settlement verdict: {other:?}"),
    }
    assert!(rows_of(&f, &f.owner).await.is_empty());
    // Exercise the producer guard even when settlement refuses first.
    let mut message = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id": &prior_reply})
        .await
        .unwrap()
        .unwrap();
    message.id = late_reply;
    message.turn_id = row.active_turn.as_ref().unwrap().turn_id.clone();
    message.status = "failed".into();
    message.error_code = Some("cancelled".into());
    let mut session = db.client().start_session().await.unwrap();
    let db = db.clone();
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                record_turn_settled_in_session(
                    &db,
                    session,
                    &row,
                    row.active_turn.as_ref().unwrap(),
                    &message,
                )
                .await,
            )
        })
        .await
        .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

// ---------------------------------------------------------------------------
// Channel chats
// ---------------------------------------------------------------------------

async fn seed_channel(f: &Fixture, owner: &str, org: Option<&str>) -> (String, String) {
    let channel = Uuid::new_v4().to_string();
    let thread = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let mut doc = doc! {"_id": &channel, "user_id": owner, "channel_bot_id": "bot",
    "platform": "telegram", "status": "active", "bot_label": "Support bot", "transport": "gateway",
    "route_api_key_id": "route-key", "created_at": now, "updated_at": now};
    if let Some(org) = org {
        doc.insert("org_specialist", doc! {"org_id": org, "version": 1});
    }
    f.state
        .db
        .collection::<Document>(CHANNELS)
        .insert_one(doc)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(CHANNEL_THREADS)
        .insert_one(
            doc! {"_id": &thread, "channel_id": &channel, "user_id": owner,
            "partition": "chat-1", "title": "Alice", "kind": "private",
            "created_at": now, "updated_at": now},
        )
        .await
        .unwrap();
    (channel, thread)
}

fn channel_origin(channel: &str) -> ChannelOrigin {
    ChannelOrigin {
        org_execution: None,
        native_event_id: None,
        thread: None,
        nyxbot_channel_id: channel.into(),
        partition: "chat-1".into(),
        platform: "telegram".into(),
    }
}

#[tokio::test]
async fn channel_guest_turns_notify_the_owner_and_unlink_or_chat_removal_fence_late_settlement() {
    let f = fixture("notif_channel").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let (channel, thread) = seed_channel(&f, &f.owner, None).await;
    let mut first = start(None, "hi from telegram", TurnOrigin::Channel);
    first.channel = Some(channel_origin(&channel));
    first.guest = true;
    let row = begin(&f, &f.owner, &first).await;
    let reply = settle(&f, &row, "Hi Alice", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    let update = &rows[0];
    assert_eq!(update.kind, NotificationKind::ChatUpdate);
    assert_eq!(update.source.source_type, SourceType::ChannelChat);
    assert_eq!(update.source.id, row.id);
    assert_eq!(
        update.source.conversation_id.as_deref(),
        Some(row.id.as_str())
    );
    assert_eq!(update.source.channel_id.as_deref(), Some(channel.as_str()));
    assert_eq!(update.source.thread_id.as_deref(), Some(thread.as_str()));
    assert_eq!(update.source.message_id.as_deref(), Some(reply.as_str()));
    assert!(update.source.lane.is_none());
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items[0].title, "Support bot · Alice");
    assert_eq!(listed.items[0].preview, "Hi Alice");
    assert_eq!(listed.items[0].source.source_type, "channel_chat");
    assert!(listed.items[0].url.contains(&format!("c={}", row.id)));
    // Opening the thread reads the chat row through its conversation id.
    assert_eq!(
        mark_read(
            db,
            &f.owner,
            ReadScope::Source {
                source_type: SourceType::Conversation,
                id: row.id.clone(),
            }
        )
        .await
        .unwrap(),
        1
    );
    // Unlink before settlement: the channel write fence refuses the row and
    // the list purges what the channel left behind.
    let mut again = start(Some(&row.id), "more", TurnOrigin::Channel);
    again.channel = Some(channel_origin(&channel));
    again.guest = true;
    let row2 = begin(&f, &f.owner, &again).await;
    db.collection::<Document>(CHANNELS)
        .update_one(
            doc! {"_id": &channel},
            doc! {"$set": {"status": "disconnected"}},
        )
        .await
        .unwrap();
    settle(&f, &row2, "late reply", None).await;
    let retained = rows_of(&f, &f.owner).await;
    assert_eq!(retained.len(), 1);
    // Reopen the retained row so the summary has something to omit.
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &retained[0].id},
            doc! {"$set": {"unread": true, "read_at": bson::Bson::Null}},
        )
        .await
        .unwrap();
    // A present channel in a non-live status is repairable: list and summary
    // hide its rows instead of deleting them (the transactional retirement
    // hook owns real deletion).
    for status in ["disconnected", "failed"] {
        db.collection::<Document>(CHANNELS)
            .update_one(doc! {"_id": &channel}, doc! {"$set": {"status": status}})
            .await
            .unwrap();
        assert!(
            list(&f.state, &f.owner, &page(10))
                .await
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(summary(db, &f.owner).await.unwrap().unread_count, 0);
        assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
    }
    // Repaired: the retained row is back.
    db.collection::<Document>(CHANNELS)
        .update_one(doc! {"_id": &channel}, doc! {"$set": {"status": "active"}})
        .await
        .unwrap();
    let repaired = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(repaired.items.len(), 1);
    assert_eq!(repaired.items[0].title, "Support bot · Alice");
    assert_eq!(summary(db, &f.owner).await.unwrap().unread_count, 1);
    // Chat removal is final: a settlement without a live thread records
    // nothing and the retained row is purged at the next read.
    db.collection::<Document>(CHANNEL_THREADS)
        .delete_one(doc! {"_id": &thread})
        .await
        .unwrap();
    let row3 = begin(&f, &f.owner, &again).await;
    settle(&f, &row3, "after removal", None).await;
    assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
    assert!(
        list(&f.state, &f.owner, &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

#[tokio::test]
async fn org_guest_lane_notifies_the_route_admin_from_the_public_transcript_only() {
    let f = fixture("notif_org_guest").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let org = Uuid::new_v4().to_string();
    db.collection(USER_ROWS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let admin = person(&f).await;
    db.collection(MEMBERSHIPS)
        .insert_one(test_membership(&org, &admin, OrgRole::Admin, None))
        .await
        .unwrap();
    let researcher = specialist(&f, "sales").await;
    // The channel belongs to the route admin; the lane conversation does not.
    let (channel, thread) = seed_channel(&f, &admin, Some(&org)).await;
    let mut first = start(None, "visitor question", TurnOrigin::Channel);
    first.channel = Some(channel_origin(&channel));
    first.guest = true;
    let row = begin(&f, &f.owner, &first).await;
    let execution = crate::models::org_channel::Execution {
        version: 1,
        org_id: org.clone(),
        revision: "rev-1".into(),
        agent_id: researcher.id.clone(),
        actor_id: org.clone(),
        guest: true,
        guest_revision: None,
        sender_hash: "sender".into(),
        link_id: None,
        link_revision: None,
    };
    db.collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"channel.org_execution": bson::to_bson(&execution).unwrap()}},
        )
        .await
        .unwrap();
    // With the admin's feed off, a guest settlement reads the channel owner
    // and stops: no channel write (no fence), no row.
    set_flag(&f, false).await;
    settle(&f, &row, "while the feed is off", None).await;
    let untouched = db
        .collection::<Document>(CHANNELS)
        .find_one(doc! {"_id": &channel})
        .await
        .unwrap()
        .unwrap();
    assert!(!untouched.contains_key("notification_fence"));
    assert!(rows_of(&f, &admin).await.is_empty());
    set_flag(&f, true).await;
    // A fresh guest lane turn on the same channel and chat (an admitted lane
    // row cannot be re-entered here without the real organization binding).
    let mut more = start(None, "visitor question again", TurnOrigin::Channel);
    more.channel = Some(channel_origin(&channel));
    more.guest = true;
    let row = begin(&f, &f.owner, &more).await;
    db.collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"channel.org_execution": bson::to_bson(&execution).unwrap()}},
        )
        .await
        .unwrap();
    let reply = settle(&f, &row, "private lane text", None).await;
    assert!(
        db.collection::<Document>(CHANNELS)
            .find_one(doc! {"_id": &channel})
            .await
            .unwrap()
            .unwrap()
            .contains_key("notification_fence")
    );
    // Isolate the public-preview join from transcript publication: this
    // private thread's public entry is seeded; publisher behavior has separate
    // org_channel_service transcript tests.
    db.collection::<Document>(ORG_CHANNEL_MESSAGES)
        .insert_one(
            doc! {"_id": &reply, "text": "public reply", "role": "agent",
            "channel_id": &channel, "partition": "chat-1"},
        )
        .await
        .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
    let rows = rows_of(&f, &admin).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::ChatUpdate);
    assert_eq!(rows[0].source.lane.as_deref(), Some(LANE_ORG_GUEST));
    assert_eq!(rows[0].source.id, thread);
    assert!(rows[0].source.conversation_id.is_none());
    assert_eq!(
        rows[0].source.agent_id.as_deref(),
        Some(researcher.id.as_str())
    );
    let listed = list(&f.state, &admin, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].preview, "public reply");
    assert!(!listed.items[0].preview.contains("private"));
    assert!(listed.items[0].url.contains("details=channels"));
    assert!(listed.items[0].url.contains(&researcher.id));
    assert_eq!(summary(db, &admin).await.unwrap().unread_count, 1);
    // The admin lost the organization: hidden everywhere, nothing purged.
    db.collection::<Document>(MEMBERSHIPS)
        .update_many(
            doc! {"org_user_id": &org, "member_user_id": &admin},
            doc! {"$set": {"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(summary(db, &admin).await.unwrap().unread_count, 0);
    assert!(
        list(&f.state, &admin, &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(rows_of(&f, &admin).await.len(), 1);
}

/// Run the settlement producer for a turn whose origin is stamped in memory
/// (the engine passes the settled turn and reply the same way).
async fn settle_platform(
    f: &Fixture,
    row: &AssistantConversation,
    turn: &ActiveTurn,
    message: &AssistantMessage,
) {
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    record_turn_settled_in_session(&f.state.db, &mut session, row, turn, message)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
}

#[tokio::test]
async fn linked_platform_chat_replies_notify_the_owner_while_the_binding_is_live() {
    let f = fixture("notif_platform").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let binding = Uuid::new_v4().to_string();
    let thread = PlatformThread {
        platform: "telegram".into(),
        profile_id: "profile".into(),
        manager_bot_id: "7".into(),
        entry: StoredEntryIdentity::TelegramV1 { telegram_bot_id: 7 },
        binding_id: binding.clone(),
        binding_generation: 3,
        user_id: f.owner.clone(),
        nyxbot_agent_id: nyxbot.id.clone(),
    };
    // The saved thread keeps its platform identity; the binding is live.
    db.collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &home.id},
            doc! {"$set": {"platform_thread": bson::to_bson(&thread).unwrap()}},
        )
        .await
        .unwrap();
    db.collection::<Document>(PLATFORM_BINDINGS)
        .insert_one(
            doc! {"_id": &binding, "user_id": &f.owner, "generation": 3_i64,
            "conversation_id": &home.id, "state": "active"},
        )
        .await
        .unwrap();
    // A web continuation of the thread is a user turn in the app: nothing.
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "continue on the web", TurnOrigin::User),
    )
    .await;
    assert!(row.platform_thread.is_some());
    let turn = row.active_turn.clone().expect("live turn");
    let web_reply = settle(&f, &row, "Here you go", None).await;
    assert!(rows_of(&f, &f.owner).await.is_empty());
    // The same user turn arriving from the platform carries the
    // server-authored origin on the thread's own binding.
    let mut platform_turn = turn.clone();
    platform_turn.platform_origin = Some(PlatformTurnOrigin {
        thread: thread.clone(),
        manager_authority_epoch: 1,
        personal_control_revision: 1,
        sender: StoredSenderIdentity::TelegramV1 {
            telegram_sender_id: 5,
            telegram_chat_id: 5,
        },
        source: PlatformOriginSource::ThreadEvent {
            admission_id: turn.turn_id.clone(),
        },
    });
    let reply: AssistantMessage = db
        .collection(MESSAGES)
        .find_one(doc! {"_id": &web_reply})
        .await
        .unwrap()
        .unwrap();
    settle_platform(&f, &row, &platform_turn, &reply).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    let update = &rows[0];
    assert_eq!(update.kind, NotificationKind::ChatUpdate);
    assert_eq!(update.source.source_type, SourceType::ChannelChat);
    assert_eq!(update.source.id, home.id);
    assert_eq!(
        update.source.conversation_id.as_deref(),
        Some(home.id.as_str())
    );
    assert_eq!(
        update.source.platform_binding_id.as_deref(),
        Some(binding.as_str())
    );
    assert_eq!(update.source.platform_binding_generation, Some(3));
    assert!(update.source.channel_id.is_none() && update.source.thread_id.is_none());
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].title, "Telegram · NyxBot");
    assert_eq!(listed.items[0].preview, "Here you go");
    assert_eq!(listed.items[0].source.source_type, "channel_chat");
    assert!(listed.items[0].url.contains(&format!("c={}", home.id)));
    let counts = summary(db, &f.owner).await.unwrap();
    assert_eq!(counts.unread_count, 1);
    assert_eq!(counts.by_source[0].source_type, "channel_chat");
    assert_eq!(counts.by_source[0].id, home.id);
    // Relinked under a new generation: the old row hides, nothing is deleted.
    db.collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &home.id},
            doc! {"$set": {"platform_thread.binding_generation": 4_i64}},
        )
        .await
        .unwrap();
    assert!(
        list(&f.state, &f.owner, &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(summary(db, &f.owner).await.unwrap().unread_count, 0);
    assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
    db.collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &home.id},
            doc! {"$set": {"platform_thread.binding_generation": 3_i64}},
        )
        .await
        .unwrap();
    // Opening the saved thread reads the chat row through its conversation id.
    assert_eq!(
        mark_read(
            db,
            &f.owner,
            ReadScope::Source {
                source_type: SourceType::Conversation,
                id: home.id.clone(),
            }
        )
        .await
        .unwrap(),
        1
    );
    // A decision raised on the thread carries the binding reference and the
    // chat as its visible source.
    let row2 = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "Send the file", TurnOrigin::User),
    )
    .await;
    let card = action_card(&f, &row2, &nyxbot.id, "Send the file?").await;
    let decision = rows_of(&f, &f.owner)
        .await
        .into_iter()
        .find(|row| row.source.card_id.as_deref() == Some(card.id.as_str()))
        .unwrap();
    assert_eq!(decision.source.source_type, SourceType::ChannelChat);
    assert_eq!(decision.source.id, home.id);
    assert_eq!(
        decision.source.platform_binding_id.as_deref(),
        Some(binding.as_str())
    );
    let pending = list(
        &f.state,
        &f.owner,
        &ListQuery {
            pending_only: true,
            ..page(10)
        },
    )
    .await
    .unwrap();
    assert_eq!(pending.items.len(), 2);
    let decision_item = pending
        .items
        .iter()
        .find(|item| item.kind == "needs_decision")
        .expect("decision listed");
    assert!(decision_item.url.contains(&format!("card={}", card.id)));
    // Unlink revokes the binding and purges its rows (D7); a settlement that
    // lands afterwards records nothing.
    db.collection::<Document>(PLATFORM_BINDINGS)
        .update_one(doc! {"_id": &binding}, doc! {"$set": {"state": "revoked"}})
        .await
        .unwrap();
    purge(
        db,
        doc! {fields::SOURCE_PLATFORM_BINDING_ID: &binding},
        None,
    )
    .await
    .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
    let mut late = reply.clone();
    late.id = Uuid::new_v4().to_string();
    let mut late_turn = row2.active_turn.clone().expect("live turn");
    late_turn.platform_origin = platform_turn.platform_origin.clone();
    settle_platform(&f, &row2, &late_turn, &late).await;
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

// ---------------------------------------------------------------------------
// Trigger runs
// ---------------------------------------------------------------------------

async fn automation(f: &Fixture, agent_id: &str, deliver_to: DeliverTo) -> Trigger {
    trigger_service::create(
        &f.state.db,
        &f.state.encryption_keys,
        trigger_service::CreateInput {
            source: TriggerSource::Schedule,
            setup_watch_id: None,
            user_id: f.owner.clone(),
            label: "Morning report".into(),
            user_service_id: None,
            overlap: OverlapPolicy::Skip,
            schedule: Some(ScheduleSpec {
                timing: ScheduleKind::Every {
                    amount: 5,
                    unit: IntervalUnit::Minutes,
                    anchor: Utc::now(),
                },
                start: None,
                end: None,
                max_runs: None,
                grace_seconds: Some(60),
            }),
            verification: TriggerVerification::Schedule,
            delivery: TriggerDelivery::Assistant {
                confirmation_policy: Default::default(),
                agent_id: agent_id.into(),
                thread_policy: None,
                instruction: "Summarise my notifications".into(),
                deliver_to,
            },
        },
    )
    .await
    .unwrap()
    .trigger
}

async fn run_for(f: &Fixture, trigger: &Trigger) -> TriggerRun {
    let at = Utc::now();
    let mut run = schedules::new_run(trigger, at, at + Duration::hours(1));
    // Admission copies the trigger's delivery onto the run.
    if let TriggerDelivery::Assistant {
        agent_id,
        deliver_to,
        ..
    } = &trigger.delivery
    {
        run.agent_id = Some(agent_id.clone());
        run.deliver_to = deliver_to.clone();
    }
    f.state
        .db
        .collection::<TriggerRun>(RUNS)
        .insert_one(&run)
        .await
        .unwrap();
    run
}

#[tokio::test]
async fn trigger_terminals_record_results_and_actionable_problems_once_and_fence_deleted_triggers()
{
    let f = fixture("notif_triggers").await;
    let db = &f.state.db;
    let frontend = f.state.config.frontend_url.clone();
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let trigger = automation(&f, &nyxbot.id, DeliverTo::Thread).await;
    // Flag off: nothing.
    let run = run_for(&f, &trigger).await;
    schedules::finish(db, &run.id, RunOutcome::Completed, None, &frontend)
        .await
        .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
    set_flag(&f, true).await;
    let run = run_for(&f, &trigger).await;
    schedules::finish(db, &run.id, RunOutcome::Completed, None, &frontend)
        .await
        .unwrap();
    // A second terminal call is fenced by the run itself; the row is one per run.
    schedules::finish(
        db,
        &run.id,
        RunOutcome::Failed,
        Some("turn_lost"),
        &frontend,
    )
    .await
    .unwrap();
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, deterministic_id("run", &run.id));
    assert_eq!(rows[0].kind, NotificationKind::AutomationResult);
    assert_eq!(rows[0].source.source_type, SourceType::Trigger);
    assert_eq!(rows[0].source.id, trigger.id);
    assert_eq!(rows[0].source.run_id.as_deref(), Some(run.id.as_str()));
    assert!(!rows[0].coalesced);
    assert_eq!(rows[0].push.as_ref().unwrap().state, "pending");
    for (outcome, reason, expect_problem) in [
        (RunOutcome::Failed, Some("turn_lost"), true),
        (RunOutcome::Failed, Some("cancelled"), false),
        (RunOutcome::Skipped, Some("overlap"), false),
        (RunOutcome::Skipped, Some("trigger_paused"), false),
        (RunOutcome::Skipped, Some("grace_expired"), true),
        (RunOutcome::Skipped, Some("queue_full"), true),
    ] {
        let before = rows_of(&f, &f.owner).await.len();
        let run = run_for(&f, &trigger).await;
        schedules::finish(db, &run.id, outcome, reason, &frontend)
            .await
            .unwrap();
        let rows = rows_of(&f, &f.owner).await;
        assert_eq!(
            rows.len(),
            before + usize::from(expect_problem),
            "{outcome:?} {reason:?}"
        );
        if expect_problem {
            let row = rows.last().unwrap();
            assert_eq!(row.kind, NotificationKind::AutomationProblem);
            assert_eq!(row.reason.as_deref(), reason);
            assert_eq!(row.outcome.as_deref(), Some(outcome.as_str()));
        }
    }
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert!(
        listed
            .items
            .iter()
            .all(|item| item.title == "Morning report")
    );
    let problem = listed
        .items
        .iter()
        .find(|item| item.kind == "automation_problem")
        .unwrap();
    assert!(problem.url.contains("/assistant/automations"));
    assert!(!problem.preview.is_empty());
    // A run whose own delivery is push never pushes twice.
    let pushing = automation(&f, &nyxbot.id, DeliverTo::Notification).await;
    let run = run_for(&f, &pushing).await;
    schedules::finish(db, &run.id, RunOutcome::Completed, None, &frontend)
        .await
        .unwrap();
    let row = rows_of(&f, &f.owner)
        .await
        .into_iter()
        .find(|row| row.source.run_id.as_deref() == Some(run.id.as_str()))
        .unwrap();
    assert_eq!(row.push.as_ref().unwrap().state, "skipped");
    assert_eq!(
        row.push.as_ref().unwrap().code.as_deref(),
        Some("run_delivers_push")
    );
    // Trigger deleted before its run settles: the parent fence refuses the row.
    let doomed = automation(&f, &nyxbot.id, DeliverTo::Thread).await;
    let run = run_for(&f, &doomed).await;
    let before = rows_of(&f, &f.owner).await.len();
    trigger_service::delete(db, &doomed).await.unwrap();
    schedules::finish(db, &run.id, RunOutcome::Completed, None, &frontend)
        .await
        .unwrap();
    assert_eq!(rows_of(&f, &f.owner).await.len(), before);
}

#[tokio::test]
async fn automatic_pause_records_one_problem_with_the_notice_identity_and_no_second_push() {
    let f = fixture("notif_pause").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let created = automation(&f, &nyxbot.id, DeliverTo::Thread).await;
    // The stored generation has millisecond precision.
    let trigger: Trigger = db
        .collection(TRIGGER_ROWS)
        .find_one(doc! {"_id": &created.id})
        .await
        .unwrap()
        .unwrap();
    let at = Utc::now();
    let mut run = schedules::new_run(&trigger, at, at + Duration::hours(1));
    run.outcome = RunOutcome::Started;
    run.turn_id = Some(Uuid::new_v4().to_string());
    run.thread_id = Some(home.id.clone());
    run.trigger_generation = Some(trigger.updated_at);
    db.collection::<TriggerRun>(RUNS)
        .insert_one(&run)
        .await
        .unwrap();
    schedules::terminal_result(
        db,
        &run,
        Some("insufficient_credits"),
        &f.state.config.frontend_url,
    )
    .await
    .unwrap();
    let paused = db
        .collection::<Document>(TRIGGER_ROWS)
        .find_one(doc! {"_id": &trigger.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paused.get_str("status").unwrap(), "disabled");
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::AutomationProblem);
    assert_eq!(rows[0].outcome.as_deref(), Some("paused"));
    assert_eq!(rows[0].reason.as_deref(), Some("insufficient_credits"));
    assert_eq!(rows[0].push.as_ref().unwrap().state, "skipped");
    assert_eq!(
        rows[0].push.as_ref().unwrap().code.as_deref(),
        Some("pause_notice_push")
    );
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert!(listed.items[0].preview.starts_with("Automation paused:"));
    assert!(listed.items[0].url.contains(&format!("c={}", home.id)));
    // Recovery replays the same terminal transaction without a second row.
    schedules::terminal_result(
        db,
        &run,
        Some("insufficient_credits"),
        &f.state.config.frontend_url,
    )
    .await
    .unwrap();
    assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
}

// ---------------------------------------------------------------------------
// Decision cards
// ---------------------------------------------------------------------------

fn chat_authority(row: &AssistantConversation, agent_id: &str) -> acks::ChatAuthority {
    let turn = row.active_turn.as_ref().expect("live turn");
    acks::ChatAuthority {
        platform_model_context: None,
        automation_work: None,
        execution_group_id: None,
        execution_unsupported: false,
        execution_voice_task: false,
        execution_task: None,
        execution_permission: None,
        business_system_success: false,
        collaboration_thread: false,
        channel_adapter_origin: None,
        invocation_controlled: false,
        invocation_id: None,
        platform_thread: false,
        platform_origin: None,
        org_agent_access: None,
        turn_id: Some(turn.turn_id.clone()),
        turn_stopped: false,
        turn_live: true,
        report_to: None,
        with_nyxbot: false,
        machine_node_ids: Vec::new(),
        saved_login_ids: Vec::new(),
        confirmation_policy: None,
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        api_key_id: row.credential_api_key_id.clone(),
        role: row.role,
        agent_id: agent_id.into(),
        agent_name: "NyxBot".into(),
        guest: false,
    }
}

async fn action_card(
    f: &Fixture,
    row: &AssistantConversation,
    agent_id: &str,
    summary: &str,
) -> crate::models::assistant_acknowledgement::AssistantAcknowledgement {
    let args = json!({"id": summary});
    acks::request(
        &f.state.db,
        &chat_authority(row, agent_id),
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_service"),
            arguments: Some(&args),
            summary,
            platform: false,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn trigger_turn_cards_resolve_the_deferred_flag_when_the_card_is_raised() {
    let f = fixture("notif_trigger_cards").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "Scheduled work", TurnOrigin::Trigger),
    )
    .await;
    assert_eq!(
        row.active_turn.as_ref().unwrap().notifications_enabled,
        None
    );
    set_flag(&f, false).await;
    action_card(&f, &row, &nyxbot.id, "Disabled feed decision?").await;
    assert!(rows_of(&f, &f.owner).await.is_empty());
    set_flag(&f, true).await;
    let card = action_card(&f, &row, &nyxbot.id, "Automation needs a decision?").await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::NeedsDecision);
    assert_eq!(rows[0].source.card_id.as_deref(), Some(card.id.as_str()));
    assert_eq!(summary(db, &f.owner).await.unwrap().needs_decision_count, 1);
}

#[tokio::test]
async fn pending_cards_create_decision_rows_that_resolve_on_decide_expire_sweep_and_supersede() {
    let f = fixture("notif_cards").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "Delete service x", TurnOrigin::User),
    )
    .await;
    let card = action_card(&f, &row, &nyxbot.id, "Delete service x?").await;
    assert_eq!(card.status, "pending");
    // Requesting the same card again reuses it: still one row.
    action_card(&f, &row, &nyxbot.id, "Delete service x?").await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    let pending = &rows[0];
    assert_eq!(pending.id, deterministic_id("card", &card.id));
    assert_eq!(pending.kind, NotificationKind::NeedsDecision);
    assert_eq!(pending.source.card_id.as_deref(), Some(card.id.as_str()));
    assert_eq!(
        pending.source.api_key_id.as_deref(),
        Some(row.credential_api_key_id.as_str())
    );
    assert_eq!(pending.card_expires_at, Some(card.expires_at));
    assert!(pending.expires_at >= card.expires_at);
    let listed = list(
        &f.state,
        &f.owner,
        &ListQuery {
            kinds: Some(vec![NotificationKind::NeedsDecision]),
            pending_only: true,
            ..page(10)
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].preview, "Delete service x?");
    assert!(listed.items[0].url.contains(&format!("card={}", card.id)));
    let before = summary(db, &f.owner).await.unwrap();
    assert_eq!((before.unread_count, before.needs_decision_count), (1, 1));
    // Seeing the card reads it; it stays pending until decided.
    mark_read(db, &f.owner, ReadScope::Ids(vec![pending.id.clone()]))
        .await
        .unwrap();
    let seen = summary(db, &f.owner).await.unwrap();
    assert_eq!((seen.unread_count, seen.needs_decision_count), (0, 0));
    let still_pending = list(
        &f.state,
        &f.owner,
        &ListQuery {
            pending_only: true,
            ..page(10)
        },
    )
    .await
    .unwrap();
    assert_eq!(still_pending.items.len(), 1);
    assert!(still_pending.items[0].read && !still_pending.items[0].resolved);
    // Deciding resolves it in the decision transaction.
    acks::decide(db, &f.owner, &home.id, &card.id, true)
        .await
        .unwrap();
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows[0].resolution.as_deref(), Some("decided"));
    let resolved_at = rows[0].resolved_at.expect("resolved");
    // Resolving is the last activity: retention restarts from it (D7).
    assert_eq!(rows[0].updated_at, resolved_at);
    assert_eq!(
        rows[0].expires_at,
        resolved_at + Duration::days(RETENTION_DAYS)
    );
    assert!(
        list(
            &f.state,
            &f.owner,
            &ListQuery {
                pending_only: true,
                ..page(10)
            }
        )
        .await
        .unwrap()
        .items
        .is_empty()
    );
    // Lazy timeout expiry mirrors into the row.
    let second = action_card(&f, &row, &nyxbot.id, "Delete service y?").await;
    db.collection::<Document>(ACKS)
        .update_one(
            doc! {"_id": &second.id},
            doc! {"$set": {"expires_at": bson_date(Utc::now() - Duration::minutes(1))}},
        )
        .await
        .unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": deterministic_id("card", &second.id)},
            doc! {"$set": {"card_expires_at": bson_date(Utc::now() - Duration::minutes(1))}},
        )
        .await
        .unwrap();
    assert_eq!(summary(db, &f.owner).await.unwrap().needs_decision_count, 0);
    acks::expire(db, &f.owner, &home.id).await.unwrap();
    let expired = rows_of(&f, &f.owner)
        .await
        .into_iter()
        .find(|row| row.source.card_id.as_deref() == Some(second.id.as_str()))
        .unwrap();
    assert_eq!(expired.resolution.as_deref(), Some("expired"));
    assert_eq!(
        expired.expires_at,
        expired.resolved_at.expect("resolved") + Duration::days(RETENTION_DAYS)
    );
    // The sweep resolves a timed-out card no one touched.
    let third = action_card(&f, &row, &nyxbot.id, "Delete service z?").await;
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": deterministic_id("card", &third.id)},
            doc! {"$set": {"card_expires_at": bson_date(Utc::now() - Duration::minutes(1))}},
        )
        .await
        .unwrap();
    sweep_with(&f.state, |_, _, _, _| async {
        Ok(automation::Outcome::Accepted)
    })
    .await
    .unwrap();
    let swept = rows_of(&f, &f.owner)
        .await
        .into_iter()
        .find(|row| row.source.card_id.as_deref() == Some(third.id.as_str()))
        .unwrap();
    assert_eq!(swept.resolution.as_deref(), Some("expired"));
    // A card decided on another replica before its hook ran reconciles as
    // decided, not expired.
    let elsewhere = action_card(&f, &row, &nyxbot.id, "Delete service v?").await;
    db.collection::<Document>(ACKS)
        .update_one(
            doc! {"_id": &elsewhere.id},
            doc! {"$set": {"status": "allowed"}},
        )
        .await
        .unwrap();
    list(&f.state, &f.owner, &page(10)).await.unwrap();
    let reconciled = rows_of(&f, &f.owner)
        .await
        .into_iter()
        .find(|row| row.source.card_id.as_deref() == Some(elsewhere.id.as_str()))
        .unwrap();
    assert_eq!(reconciled.resolution.as_deref(), Some("decided"));
    // Full access retires ordinary asks: superseded.
    let fourth = action_card(&f, &row, &nyxbot.id, "Delete service w?").await;
    settings::update(
        db,
        &f.owner,
        settings::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let superseded = rows_of(&f, &f.owner)
        .await
        .into_iter()
        .find(|row| row.source.card_id.as_deref() == Some(fourth.id.as_str()))
        .unwrap();
    assert_eq!(superseded.resolution.as_deref(), Some("superseded"));
    settle(&f, &row, "done", None).await;
}

#[tokio::test]
async fn collaboration_cards_render_for_the_requester_and_ownership_mismatches_hide_not_purge() {
    let f = fixture("notif_card_owner").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let requester = person(&f).await;
    // The hidden slot belongs to the agent owner; the acknowledgement stores
    // that owner, while the row is the requester's.
    let conversation = Uuid::new_v4().to_string();
    db.collection::<Document>(CONVERSATIONS)
        .insert_one(
            doc! {"_id": &conversation, "user_id": &f.owner, "title": "Shared research",
            "collaboration": {"original_requester_id": &requester, "group_id": "grp"}},
        )
        .await
        .unwrap();
    let expires = Utc::now() + Duration::minutes(10);
    let card = Uuid::new_v4().to_string();
    db.collection::<Document>(ACKS)
        .insert_one(
            doc! {"_id": &card, "user_id": &f.owner, "conversation_id": &conversation,
            "status": "pending", "expires_at": bson_date(expires), "summary": "Export the sheet?"},
        )
        .await
        .unwrap();
    let mut row = seeded(
        &requester,
        NotificationKind::NeedsDecision,
        NotificationSource {
            source_type: SourceType::CollaborationGroup,
            id: "grp".into(),
            conversation_id: Some(conversation.clone()),
            card_id: Some(card.clone()),
            ..Default::default()
        },
    );
    row.card_expires_at = Some(expires);
    row.outcome = None;
    insert_row(&f, &row).await;
    let listed = list(&f.state, &requester, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].preview, "Export the sheet?");
    assert!(!listed.items[0].resolved);
    let counts = summary(db, &requester).await.unwrap();
    assert_eq!((counts.unread_count, counts.needs_decision_count), (1, 1));
    assert_eq!(rows_of(&f, &requester).await.len(), 1);
    // A card that belongs to another thread is hidden, never deleted.
    let foreign = Uuid::new_v4().to_string();
    db.collection::<Document>(ACKS)
        .insert_one(
            doc! {"_id": &foreign, "user_id": &f.owner, "conversation_id": "elsewhere",
            "status": "pending", "expires_at": bson_date(expires), "summary": "Other"},
        )
        .await
        .unwrap();
    let mut mismatched = seeded(
        &requester,
        NotificationKind::NeedsDecision,
        NotificationSource {
            source_type: SourceType::Conversation,
            id: conversation.clone(),
            conversation_id: Some(conversation.clone()),
            card_id: Some(foreign.clone()),
            ..Default::default()
        },
    );
    mismatched.card_expires_at = Some(expires);
    mismatched.outcome = None;
    insert_row(&f, &mismatched).await;
    assert_eq!(
        list(&f.state, &requester, &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(rows_of(&f, &requester).await.len(), 2);
    // An automation whose trigger and run belong to someone else is hidden too.
    let trigger = Uuid::new_v4().to_string();
    db.collection::<Document>(TRIGGER_ROWS)
        .insert_one(
            doc! {"_id": &trigger, "user_id": &f.owner, "label": "Nightly", "status": "active"},
        )
        .await
        .unwrap();
    let run = Uuid::new_v4().to_string();
    db.collection::<Document>(RUNS)
        .insert_one(
            doc! {"_id": &run, "user_id": &f.owner, "trigger_id": &trigger,
            "label": "Nightly", "outcome": "completed"},
        )
        .await
        .unwrap();
    insert_row(
        &f,
        &seeded(
            &requester,
            NotificationKind::AutomationResult,
            NotificationSource {
                source_type: SourceType::Trigger,
                id: trigger.clone(),
                trigger_id: Some(trigger.clone()),
                run_id: Some(run.clone()),
                ..Default::default()
            },
        ),
    )
    .await;
    assert_eq!(
        list(&f.state, &requester, &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(summary(db, &requester).await.unwrap().unread_count, 1);
    assert_eq!(rows_of(&f, &requester).await.len(), 3);
    // Only a genuinely missing card deletes its row.
    db.collection::<Document>(ACKS)
        .delete_one(doc! {"_id": &card})
        .await
        .unwrap();
    assert!(
        list(&f.state, &requester, &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(rows_of(&f, &requester).await.len(), 2);
}

#[tokio::test]
async fn group_cards_mark_the_visible_group_and_opening_it_reads_them_without_resolving() {
    let f = fixture("notif_group_cards").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let researcher = specialist(&f, "researcher").await;
    let group = groups::create(
        db,
        &f.owner,
        "Ops",
        std::slice::from_ref(&researcher.id),
        "user",
    )
    .await
    .unwrap();
    // The member speaks through a hidden thread of the group.
    let mut member = start(None, "Delete service g", TurnOrigin::Group);
    member.agent_id = Some(researcher.id.clone());
    member.group_id = Some(group.id.clone());
    let row = begin(&f, &f.owner, &member).await;
    assert_eq!(row.group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(
        row.active_turn.as_ref().unwrap().notifications_enabled,
        None
    );
    let card = action_card(&f, &row, &researcher.id, "Delete service g?").await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    let pending = &rows[0];
    assert_eq!(pending.kind, NotificationKind::NeedsDecision);
    // D6: the visible source is the group; the hidden thread stays on the
    // row for purge and card joins.
    assert_eq!(pending.source.source_type, SourceType::Group);
    assert_eq!(pending.source.id, group.id);
    assert_eq!(pending.source.group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(
        pending.source.conversation_id.as_deref(),
        Some(row.id.as_str())
    );
    assert_eq!(pending.source.card_id.as_deref(), Some(card.id.as_str()));
    let before = summary(db, &f.owner).await.unwrap();
    assert_eq!((before.unread_count, before.needs_decision_count), (1, 1));
    assert_eq!(before.by_source.len(), 1);
    assert_eq!(before.by_source[0].source_type, "group");
    assert_eq!(before.by_source[0].id, group.id);
    assert_eq!(
        before.by_source[0].conversation_id.as_deref(),
        Some(row.id.as_str())
    );
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].source.source_type, "group");
    assert_eq!(listed.items[0].preview, "Delete service g?");
    assert!(listed.items[0].url.contains(&format!("c={}", group.id)));
    // Opening the group (the client's group read) sees the card; it stays
    // pending until decided.
    assert_eq!(
        mark_read(
            db,
            &f.owner,
            ReadScope::Source {
                source_type: SourceType::Group,
                id: group.id.clone(),
            }
        )
        .await
        .unwrap(),
        1
    );
    let seen = summary(db, &f.owner).await.unwrap();
    assert_eq!((seen.unread_count, seen.needs_decision_count), (0, 0));
    assert!(seen.by_source.is_empty());
    let still_pending = list(
        &f.state,
        &f.owner,
        &ListQuery {
            pending_only: true,
            ..page(10)
        },
    )
    .await
    .unwrap();
    assert_eq!(still_pending.items.len(), 1);
    assert!(still_pending.items[0].read && !still_pending.items[0].resolved);
    let stored = rows_of(&f, &f.owner).await;
    assert!(stored[0].read_at.is_some() && stored[0].resolved_at.is_none());
    // Deciding on the group page resolves the row in the decision transaction.
    acks::decide(db, &f.owner, &row.id, &card.id, true)
        .await
        .unwrap();
    let decided = rows_of(&f, &f.owner).await;
    assert_eq!(decided[0].resolution.as_deref(), Some("decided"));
    assert!(
        list(
            &f.state,
            &f.owner,
            &ListQuery {
                pending_only: true,
                ..page(10)
            }
        )
        .await
        .unwrap()
        .items
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Web groups
// ---------------------------------------------------------------------------

#[tokio::test]
async fn group_notices_coalesce_and_outside_posts_do_not_notify_and_group_reads_alias_by_id() {
    let f = fixture("notif_groups").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let researcher = specialist(&f, "researcher").await;
    let group = groups::create(
        db,
        &f.owner,
        "Ops",
        std::slice::from_ref(&researcher.id),
        "user",
    )
    .await
    .unwrap();
    // NyxBot or the owner posting into the group from outside is a request,
    // not an update.
    groups::append(
        db,
        &f.owner,
        &group.id,
        "user",
        None,
        "Please summarise",
        Vec::new(),
    )
    .await
    .unwrap();
    groups::append(
        db,
        &f.owner,
        &group.id,
        "agent",
        Some(&researcher),
        "Posted from outside",
        Vec::new(),
    )
    .await
    .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
    groups::append(
        db,
        &f.owner,
        &group.id,
        "notice",
        Some(&researcher),
        "researcher could not reply (timeout).",
        Vec::new(),
    )
    .await
    .unwrap();
    groups::append(
        db,
        &f.owner,
        &group.id,
        "notice",
        Some(&researcher),
        "researcher stopped.",
        Vec::new(),
    )
    .await
    .unwrap();
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::AgentUpdate);
    assert_eq!(rows[0].source.source_type, SourceType::Group);
    assert_eq!(rows[0].source.group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(rows[0].count, 2);
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items[0].title, "researcher in Ops");
    assert_eq!(listed.items[0].preview, "researcher stopped.");
    assert!(listed.items[0].url.contains(&format!("c={}", group.id)));
    assert_eq!(
        mark_read(
            db,
            &f.owner,
            ReadScope::Source {
                source_type: SourceType::Group,
                id: group.id.clone(),
            }
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(summary(db, &f.owner).await.unwrap().unread_count, 0);
    // Deleting the group purges its rows in the delete transaction.
    groups::append(
        db,
        &f.owner,
        &group.id,
        "notice",
        Some(&researcher),
        "again",
        Vec::new(),
    )
    .await
    .unwrap();
    groups::delete(db, &f.owner, &group.id).await.unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

// ---------------------------------------------------------------------------
// Coalescing under concurrency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_producers_on_one_source_yield_one_unread_row_with_the_full_count() {
    let f = fixture("notif_concurrent").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let agent = specialist(&f, "coalescing").await;
    let group = crate::models::assistant_group::AssistantGroup {
        id: Uuid::new_v4().to_string(),
        user_id: f.owner.clone(),
        collaboration: None,
        participant_user_ids: Vec::new(),
        created_by_user_id: None,
        name: "Ops".into(),
        member_agent_ids: vec![agent.id.clone()],
        lead_agent_id: agent.id.clone(),
        last_recipient_ids: Vec::new(),
        last_recipient_ids_by_actor: Default::default(),
        created_by: "user".into(),
        message_count: 0,
        pending_agent_ids: Vec::new(),
        hops_remaining: 0,
        followers: Vec::new(),
        pending_checked_at: None,
        last_message_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let message = |seq: i64| GroupMessage {
        collaboration: None,
        activities: Vec::new(),
        org_group: false,
        author_user_id: None,
        author_display_name: None,
        request_id: None,
        attachments: Vec::new(),
        id: Uuid::new_v4().to_string(),
        group_id: group.id.clone(),
        user_id: f.owner.clone(),
        seq,
        role: "agent".into(),
        agent_id: Some(agent.id.clone()),
        agent_name: None,
        text: "reply".into(),
        recipient_agent_ids: Vec::new(),
        mentioned_participant_ids: Vec::new(),
        created_at: Utc::now(),
    };
    let produce = |seq: i64| {
        let db = db.clone();
        let owner = f.owner.clone();
        let group = group.clone();
        let message = message(seq);
        async move {
            let mut session = db.client().start_session().await.unwrap();
            session
                .start_transaction()
                .and_run2(async move |session| {
                    transactions::transaction_result(
                        record_group_reply_in_session(&db, session, &owner, &group, &message).await,
                    )
                })
                .await
                .map_err(transactions::map_transaction_error)
                .unwrap();
        }
    };
    let (_, _, _, _) = tokio::join!(produce(1), produce(2), produce(3), produce(4));
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].count, 4);
    assert!(rows[0].unread);
    // One visible source coalesces across chat kinds: a collaboration reply
    // and a mention in the same group share the unread row.
    let member = person(&f).await;
    let mut collaboration = group.clone();
    collaboration.id = Uuid::new_v4().to_string();
    collaboration.collaboration = Some(
        crate::models::assistant_group_collaboration::Collaboration {
            version: crate::models::assistant_group_collaboration::VERSION,
            creator_user_id: f.owner.clone(),
            membership_revision: 1,
        },
    );
    collaboration.participant_user_ids = vec![f.owner.clone(), member.clone()];
    let mixed = |mention: bool, seq: i64| {
        let db = db.clone();
        let member = member.clone();
        let group = collaboration.clone();
        let mut message = message(seq);
        message.group_id = group.id.clone();
        async move {
            let mut session = db.client().start_session().await.unwrap();
            session
                .start_transaction()
                .and_run2(async move |session| {
                    let result = if mention {
                        record_mention_in_session(&db, session, &member, &group, &message).await
                    } else {
                        record_collaboration_reply_in_session(
                            &db, session, &member, &group, &message,
                        )
                        .await
                    };
                    transactions::transaction_result(result)
                })
                .await
                .map_err(transactions::map_transaction_error)
                .unwrap();
        }
    };
    tokio::join!(
        mixed(true, 1),
        mixed(false, 2),
        mixed(true, 3),
        mixed(false, 4)
    );
    let rows = rows_of(&f, &member).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].count, 4);
    assert_eq!(rows[0].source.source_type, SourceType::CollaborationGroup);
    assert!(matches!(
        rows[0].kind,
        NotificationKind::GroupMention | NotificationKind::AgentUpdate
    ));
}

// ---------------------------------------------------------------------------
// Reads, paging, summary and organization access
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_scopes_paging_pending_filter_and_summary_are_idempotent_and_fenced() {
    let f = fixture("notif_reads").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let a = seed_conversation(&f, &f.owner, "Thread A").await;
    let b = seed_conversation(&f, &f.owner, "Thread B").await;
    let row_a = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&a),
    );
    let row_b = seeded(
        &f.owner,
        NotificationKind::ChatUpdate,
        NotificationSource {
            source_type: SourceType::ChannelChat,
            id: b.clone(),
            conversation_id: Some(b.clone()),
            ..Default::default()
        },
    );
    let (channel, thread) = seed_channel(&f, &f.owner, None).await;
    let mut row_b = row_b;
    row_b.source.channel_id = Some(channel.clone());
    row_b.source.thread_id = Some(thread.clone());
    row_b.source_key = source_key(&row_b.source);
    insert_row(&f, &row_a).await;
    insert_row(&f, &row_b).await;
    let listed = list(&f.state, &f.owner, &page(1)).await.unwrap();
    assert_eq!(listed.items.len(), 1);
    let cursor = listed.next_cursor.clone().expect("second page");
    let second = list(
        &f.state,
        &f.owner,
        &ListQuery {
            cursor: Some(cursor),
            ..page(1)
        },
    )
    .await
    .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_ne!(listed.items[0].id, second.items[0].id);
    assert!(second.next_cursor.is_none());
    assert!(matches!(
        list(
            &f.state,
            &f.owner,
            &ListQuery {
                cursor: Some("garbage".into()),
                ..page(1)
            }
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let kinds = list(
        &f.state,
        &f.owner,
        &ListQuery {
            kinds: Some(vec![NotificationKind::ChatUpdate]),
            ..page(10)
        },
    )
    .await
    .unwrap();
    assert_eq!(kinds.items.len(), 1);
    assert_eq!(kinds.items[0].kind, "chat_update");
    // Mark-all with the fence: a row that arrived after the look stays unread.
    let as_of = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let late = seeded(
        &f.owner,
        NotificationKind::GroupMention,
        NotificationSource {
            source_type: SourceType::CollaborationGroup,
            id: "g1".into(),
            group_id: Some("g1".into()),
            ..Default::default()
        },
    );
    insert_row(&f, &late).await;
    assert_eq!(
        mark_read(db, &f.owner, ReadScope::All { before: as_of })
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        mark_read(db, &f.owner, ReadScope::All { before: as_of })
            .await
            .unwrap(),
        0
    );
    let rows = rows_of(&f, &f.owner).await;
    assert!(rows.iter().filter(|row| row.unread).count() == 1);
    assert!(rows.iter().find(|row| row.id == late.id).unwrap().unread);
    // Another person's row is never touched.
    let other = person(&f).await;
    insert_row(
        &f,
        &seeded(
            &other,
            NotificationKind::AgentUpdate,
            conversation_source(&a),
        ),
    )
    .await;
    assert_eq!(
        mark_read(
            db,
            &f.owner,
            ReadScope::Ids(vec![rows_of(&f, &other).await[0].id.clone()])
        )
        .await
        .unwrap(),
        0
    );
    assert!(matches!(
        mark_read(db, &f.owner, ReadScope::Ids(Vec::new())).await,
        Err(AppError::BadRequest(_))
    ));
}

#[tokio::test]
async fn organization_access_is_batched_and_revocation_hides_rows_before_anything_is_purged() {
    let finds = Arc::new(AtomicUsize::new(0));
    let aggregates = Arc::new(AtomicUsize::new(0));
    let (find_count, aggregate_count) = (finds.clone(), aggregates.clone());
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            match event.command_name.as_str() {
                "find" => {
                    find_count.fetch_add(1, Ordering::SeqCst);
                }
                "aggregate" => {
                    aggregate_count.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    });
    let db = connect_transaction_test_database_with_command_handler("notif_org_acl", handler).await;
    let f = fixture_with(db).await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let orgs: Vec<String> = (0..3).map(|_| Uuid::new_v4().to_string()).collect();
    for org in &orgs {
        db.collection(USER_ROWS)
            .insert_one(test_user(org, UserType::Org))
            .await
            .unwrap();
        db.collection(MEMBERSHIPS)
            .insert_one(test_membership(org, &f.owner, OrgRole::Member, None))
            .await
            .unwrap();
    }
    // Three organization-owned agent threads, one organization group, one
    // personal thread, one card and one automation.
    for org in &orgs {
        let id = Uuid::new_v4().to_string();
        db.collection::<Document>(CONVERSATIONS)
            .insert_one(doc! {"_id": &id, "user_id": &f.owner, "title": "Org thread", "agent_owner_id": org})
            .await
            .unwrap();
        insert_row(
            &f,
            &seeded(
                &f.owner,
                NotificationKind::AgentUpdate,
                conversation_source(&id),
            ),
        )
        .await;
    }
    let group_id = Uuid::new_v4().to_string();
    db.collection::<Document>(GROUPS)
        .insert_one(
            doc! {"_id": &group_id, "user_id": &orgs[0], "name": "Org group",
            "created_by_user_id": &f.owner, "participant_user_ids": [&f.owner],
            "member_agent_ids": [], "lead_agent_id": "", "last_recipient_ids": [],
            "created_by": "user", "message_count": 0, "pending_agent_ids": [],
            "hops_remaining": 0, "followers": [], "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now()},
        )
        .await
        .unwrap();
    insert_row(
        &f,
        &seeded(
            &f.owner,
            NotificationKind::AgentUpdate,
            NotificationSource {
                source_type: SourceType::Group,
                id: group_id.clone(),
                group_id: Some(group_id.clone()),
                ..Default::default()
            },
        ),
    )
    .await;
    // A collaboration marker of an unknown version is unsupported: hidden.
    let unknown_group = Uuid::new_v4().to_string();
    db.collection::<Document>(GROUPS)
        .insert_one(
            doc! {"_id": &unknown_group, "user_id": &f.owner, "name": "Future group",
            "collaboration": {"version": 99, "creator_user_id": &f.owner, "membership_revision": 1},
            "participant_user_ids": [&f.owner], "member_agent_ids": [], "lead_agent_id": "",
            "last_recipient_ids": [], "created_by": "user", "message_count": 0,
            "pending_agent_ids": [], "hops_remaining": 0, "followers": [],
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()},
        )
        .await
        .unwrap();
    insert_row(
        &f,
        &seeded(
            &f.owner,
            NotificationKind::GroupMention,
            NotificationSource {
                source_type: SourceType::CollaborationGroup,
                id: unknown_group.clone(),
                group_id: Some(unknown_group.clone()),
                ..Default::default()
            },
        ),
    )
    .await;
    let personal = seed_conversation(&f, &f.owner, "Mine").await;
    insert_row(
        &f,
        &seeded(
            &f.owner,
            NotificationKind::AgentUpdate,
            conversation_source(&personal),
        ),
    )
    .await;
    let card_id = Uuid::new_v4().to_string();
    db.collection::<Document>(ACKS)
        .insert_one(
            doc! {"_id": &card_id, "user_id": &f.owner, "conversation_id": &personal,
            "summary": "Allow?", "status": "pending",
            "expires_at": bson_date(Utc::now() + Duration::minutes(10))},
        )
        .await
        .unwrap();
    let mut card_row = seeded(
        &f.owner,
        NotificationKind::NeedsDecision,
        conversation_source(&personal),
    );
    card_row.source.card_id = Some(card_id.clone());
    card_row.card_expires_at = Some(Utc::now() + Duration::minutes(10));
    card_row.coalesced = false;
    insert_row(&f, &card_row).await;
    let trigger_id = Uuid::new_v4().to_string();
    let run_id = Uuid::new_v4().to_string();
    db.collection::<Document>(TRIGGERS)
        .insert_one(
            doc! {"_id": &trigger_id, "user_id": &f.owner, "label": "Report", "status": "active"},
        )
        .await
        .unwrap();
    db.collection::<Document>(RUNS)
        .insert_one(
            doc! {"_id": &run_id, "user_id": &f.owner, "trigger_id": &trigger_id,
            "label": "Report", "outcome": "completed"},
        )
        .await
        .unwrap();
    let mut run_row = seeded(
        &f.owner,
        NotificationKind::AutomationResult,
        NotificationSource {
            source_type: SourceType::Trigger,
            id: trigger_id.clone(),
            trigger_id: Some(trigger_id.clone()),
            run_id: Some(run_id.clone()),
            ..Default::default()
        },
    );
    run_row.coalesced = false;
    insert_row(&f, &run_row).await;
    finds.store(0, Ordering::SeqCst);
    aggregates.store(0, Ordering::SeqCst);
    let listed = list(&f.state, &f.owner, &page(50)).await.unwrap();
    assert_eq!(listed.items.len(), 7);
    assert!(
        listed
            .items
            .iter()
            .all(|item| item.source.id != unknown_group)
    );
    // One row query plus at most one batched read per referenced collection.
    assert!(
        finds.load(Ordering::SeqCst) <= 12,
        "{}",
        finds.load(Ordering::SeqCst)
    );
    finds.store(0, Ordering::SeqCst);
    let before = summary(db, &f.owner).await.unwrap();
    assert_eq!(before.unread_count, 7);
    assert_eq!(before.needs_decision_count, 1);
    assert_eq!(aggregates.load(Ordering::SeqCst), 1);
    assert_eq!(finds.load(Ordering::SeqCst), 0);
    // Revoke one organization: its thread and group rows disappear from the
    // summary and the list, while the rows themselves remain.
    db.collection::<Document>(MEMBERSHIPS)
        .update_many(
            doc! {"org_user_id": &orgs[0], "member_user_id": &f.owner},
            doc! {"$set": {"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let after = summary(db, &f.owner).await.unwrap();
    assert_eq!(after.unread_count, 5);
    assert_eq!(rows_of(&f, &f.owner).await.len(), 8);
    let listed = list(&f.state, &f.owner, &page(50)).await.unwrap();
    assert_eq!(listed.items.len(), 5);
    assert_eq!(rows_of(&f, &f.owner).await.len(), 8);
    // A deactivated organization hides its rows as well.
    db.collection::<Document>(USER_ROWS)
        .update_one(doc! {"_id": &orgs[1]}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert_eq!(summary(db, &f.owner).await.unwrap().unread_count, 4);
    // A deleted source is purged on read.
    db.collection::<Document>(CONVERSATIONS)
        .delete_one(doc! {"_id": &personal})
        .await
        .unwrap();
    let listed = list(&f.state, &f.owner, &page(50)).await.unwrap();
    assert_eq!(listed.items.len(), 2);
    assert_eq!(rows_of(&f, &f.owner).await.len(), 6);
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

type Sent = Arc<std::sync::Mutex<Vec<(String, String, String, HashMap<String, String>)>>>;

async fn try_sweep_capturing(
    f: &Fixture,
    sent: &Sent,
    outcome: impl Fn() -> Result<automation::Outcome, &'static str> + Clone,
) -> AppResult<()> {
    let sent = sent.clone();
    sweep_with(&f.state, move |user, title, body, data| {
        let sent = sent.clone();
        let outcome = outcome.clone();
        async move {
            sent.lock().unwrap().push((user, title, body, data));
            outcome()
        }
    })
    .await
}

async fn sweep_capturing(
    f: &Fixture,
    sent: &Sent,
    outcome: impl Fn() -> Result<automation::Outcome, &'static str> + Clone,
) {
    try_sweep_capturing(f, sent, outcome).await.unwrap();
}

async fn push_of(f: &Fixture, id: &str) -> PushState {
    f.state
        .db
        .collection::<AssistantNotification>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
        .push
        .unwrap()
}

#[tokio::test]
async fn push_barrier_sends_once_honors_preferences_and_never_resends_ambiguous_claims() {
    let f = fixture("notif_push").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let thread = seed_conversation(&f, &f.owner, "Thread").await;
    let sent: Sent = Default::default();
    // No preference: skipped, never sent.
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert!(sent.lock().unwrap().is_empty());
    assert_eq!(
        push_of(&f, &row.id).await.code.as_deref(),
        Some("preference_off")
    );
    push_prefs(&f, &f.owner, all_push()).await;
    // Accepted: sent exactly once with the closed kind as `type`.
    let other = seed_conversation(&f, &f.owner, "Other").await;
    let row = seeded(
        &f.owner,
        NotificationKind::NeedsDecision,
        conversation_source(&other),
    );
    let mut row = row;
    row.coalesced = false;
    let card_id = Uuid::new_v4().to_string();
    db.collection::<Document>(ACKS)
        .insert_one(
            doc! {"_id": &card_id, "user_id": &f.owner, "conversation_id": &other,
            "summary": "Allow the thing?", "status": "pending",
            "expires_at": bson_date(Utc::now() + Duration::minutes(10))},
        )
        .await
        .unwrap();
    row.source.card_id = Some(card_id.clone());
    row.card_expires_at = Some(Utc::now() + Duration::minutes(10));
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let (user, title, body, data) = &sent[0];
        assert_eq!(user, &f.owner);
        assert_eq!(title, "Other");
        assert_eq!(body, "Allow the thing?");
        assert_eq!(data.get("type").map(String::as_str), Some("needs_decision"));
        assert_eq!(
            data.get("notification_id").map(String::as_str),
            Some(row.id.as_str())
        );
        assert!(
            data.get("deeplink")
                .unwrap()
                .contains(&format!("card={card_id}"))
        );
    }
    let push = push_of(&f, &row.id).await;
    assert_eq!(push.state, "sent");
    assert!(push.sent_at.is_some());
    assert_eq!(push.attempts, 1);
    // Ambiguous outcome: unknown, never resent.
    let third = seed_conversation(&f, &f.owner, "Third").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&third),
    );
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Unknown)).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 2);
    assert_eq!(push_of(&f, &row.id).await.state, "unknown");
    // A stale `sending` claim (crashed worker) becomes unknown, not resent.
    let fourth = seed_conversation(&f, &f.owner, "Fourth").await;
    let mut row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&fourth),
    );
    row.push = Some(PushState {
        state: "sending".into(),
        kind: Some(NotificationKind::AgentUpdate),
        attempt: Some("old".into()),
        code: None,
        attempts: 1,
        claimed_at: Some(Utc::now() - Duration::minutes(30)),
        first_attempt_at: Some(Utc::now() - Duration::minutes(30)),
        retry_at: None,
        sent_at: None,
    });
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 2);
    assert_eq!(push_of(&f, &row.id).await.state, "unknown");
    // A row read or resolved before the worker got to it is skipped.
    let fifth = seed_conversation(&f, &f.owner, "Fifth").await;
    let mut row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&fifth),
    );
    row.unread = false;
    row.read_at = Some(Utc::now());
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(
        push_of(&f, &row.id).await.code.as_deref(),
        Some("already_seen")
    );
    // Flag off: skipped.
    set_flag(&f, false).await;
    let sixth = seed_conversation(&f, &f.owner, "Sixth").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&sixth),
    );
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(push_of(&f, &row.id).await.code.as_deref(), Some("flag_off"));
    assert_eq!(sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn push_retries_only_conclusive_retryable_refusals_within_bounds_and_releases_budget() {
    let f = fixture("notif_push_retry").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let sent: Sent = Default::default();
    let thread = seed_conversation(&f, &f.owner, "Thread").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    insert_row(&f, &row).await;
    let refused = || {
        Ok(automation::Outcome::Refused(automation::Failure {
            code: "push_temporarily_refused",
            http_status: Some(429),
            retry_after: Some(5),
        }))
    };
    sweep_capturing(&f, &sent, refused).await;
    let push = push_of(&f, &row.id).await;
    assert_eq!(push.state, "retrying");
    assert_eq!(push.attempts, 1);
    assert!(push.retry_at.unwrap() >= Utc::now() + Duration::seconds(PUSH_RETRY_MIN_SECONDS - 2));
    // Not due yet: nothing happens; its own reservations were released.
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(
        db.collection::<Document>(PUSH_SOURCES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"push.retry_at": bson_date(Utc::now() - Duration::seconds(1))}},
        )
        .await
        .unwrap();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 2);
    let push = push_of(&f, &row.id).await;
    assert_eq!((push.state.as_str(), push.attempts), ("sent", 2));
    // Exhaustion: the third retryable refusal is terminal.
    let other = seed_conversation(&f, &f.owner, "Other").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&other),
    );
    insert_row(&f, &row).await;
    for attempt in 1..=PUSH_MAX_ATTEMPTS {
        sweep_capturing(&f, &sent, refused).await;
        let push = push_of(&f, &row.id).await;
        assert_eq!(push.attempts, attempt);
        if attempt < PUSH_MAX_ATTEMPTS {
            assert_eq!(push.state, "retrying");
            db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.id},
                    doc! {"$set": {"push.retry_at": bson_date(Utc::now() - Duration::seconds(1))}},
                )
                .await
                .unwrap();
        } else {
            assert_eq!(push.state, "failed");
            assert_eq!(push.code.as_deref(), Some("push_retries_exhausted"));
        }
    }
    // A definite refusal without Retry-After fails at once.
    let third = seed_conversation(&f, &f.owner, "Third").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&third),
    );
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || {
        Ok(automation::Outcome::Refused(automation::Failure::new(
            "push_provider_rejected",
        )))
    })
    .await;
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (push.state.as_str(), push.code.as_deref()),
        ("failed", Some("push_provider_rejected"))
    );
    // Availability failure before any send is a definite failure.
    let fourth = seed_conversation(&f, &f.owner, "Fourth").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&fourth),
    );
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Err("push_no_devices")).await;
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (push.state.as_str(), push.code.as_deref()),
        ("failed", Some("push_no_devices"))
    );
}

#[tokio::test]
async fn push_caps_hold_per_visible_source_across_kinds_and_per_rolling_hour_across_sweeps() {
    let f = fixture("notif_push_caps").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let sent: Sent = Default::default();
    // Same visible source, two kinds: one push in ten minutes.
    let thread = seed_conversation(&f, &f.owner, "Thread").await;
    let update = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    let card_id = Uuid::new_v4().to_string();
    db.collection::<Document>(ACKS)
        .insert_one(
            doc! {"_id": &card_id, "user_id": &f.owner, "conversation_id": &thread,
            "summary": "Allow?", "status": "pending",
            "expires_at": bson_date(Utc::now() + Duration::minutes(10))},
        )
        .await
        .unwrap();
    let mut decision = seeded(
        &f.owner,
        NotificationKind::NeedsDecision,
        conversation_source(&thread),
    );
    decision.coalesced = false;
    decision.source.card_id = Some(card_id);
    decision.card_expires_at = Some(Utc::now() + Duration::minutes(10));
    decision.created_at = update.created_at + Duration::milliseconds(1);
    insert_row(&f, &update).await;
    insert_row(&f, &decision).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(push_of(&f, &update.id).await.state, "sent");
    assert_eq!(
        push_of(&f, &decision.id).await.code.as_deref(),
        Some("source_rate")
    );
    // Thirty-one distinct sources, three concurrent sweeps: thirty sends.
    let mut ids = Vec::new();
    for index in 0..31 {
        let conversation = seed_conversation(&f, &f.owner, &format!("Thread {index}")).await;
        let row = seeded(
            &f.owner,
            NotificationKind::AgentUpdate,
            conversation_source(&conversation),
        );
        insert_row(&f, &row).await;
        ids.push(row.id);
    }
    let (a, b, c) = tokio::join!(
        sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
        sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
        sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
    );
    let _ = (a, b, c);
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    let rows = rows_of(&f, &f.owner).await;
    let sent_rows = rows
        .iter()
        .filter(|row| ids.contains(&row.id) && row.push.as_ref().unwrap().state == "sent")
        .count();
    let capped = rows
        .iter()
        .filter(|row| {
            ids.contains(&row.id)
                && row.push.as_ref().unwrap().code.as_deref() == Some("hourly_limit")
        })
        .count();
    assert_eq!((sent_rows, capped), (PUSH_HOURLY_LIMIT - 1, 2));
    assert_eq!(sent.lock().unwrap().len(), PUSH_HOURLY_LIMIT);
    let budget = db
        .collection::<Document>(PUSH_BUDGETS)
        .find_one(doc! {"_id": &f.owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        budget.get_array("reservations").unwrap().len(),
        PUSH_HOURLY_LIMIT
    );
}

#[test]
fn previews_are_single_line_bounded_and_markdown_free() {
    assert_eq!(
        preview_text("# Title\n\n- **bold** item\n```\ncode\n```\n[link](https://x.y) ![img](u)"),
        "Title bold item link img"
    );
    let long = "日本語テキスト ".repeat(60);
    let preview = preview_text(&long);
    assert_eq!(preview.chars().count(), PREVIEW_CHARS);
    assert!(preview.ends_with('…'));
    assert!(long.starts_with(preview.trim_end_matches('…')));
    let exact = "字".repeat(PREVIEW_CHARS);
    assert_eq!(preview_text(&exact), exact);
    let over = "字".repeat(PREVIEW_CHARS + 1);
    let preview = preview_text(&over);
    assert_eq!(preview.chars().count(), PREVIEW_CHARS);
    assert!(preview.ends_with('…'));
    assert_eq!(preview_text("a\u{202e}b\tc"), "ab c");
    assert_eq!(parse_kinds("agent_update, chat_update").unwrap().len(), 2);
    assert!(parse_kinds("nope").is_err());
    assert!(reason_copy("grace_expired").starts_with("Skipped"));
}

#[tokio::test]
async fn a_cached_feature_snapshot_makes_admission_read_free_and_authoritative() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && matches!(event.command_name.as_str(), "find" | "aggregate")
        {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    let db =
        connect_transaction_test_database_with_command_handler("notif_flag_cache", handler).await;
    let f = fixture_with(db).await;
    let db = &f.state.db;
    reset_flag_cache(db);
    // The cache remembers the flag as off.
    assert!(!enabled(db, &f.owner).await.unwrap());
    // The engine gate then resolves a newer snapshot with the flag on and
    // seeds the cache from it: admission reads nothing and follows the gate,
    // not the older cached decision.
    flags::set_platform_override(db, FLAG_KEY, &FlagTarget::Global, true, &f.owner)
        .await
        .unwrap();
    let features = flags::resolve_personal_features(db, &f.owner)
        .await
        .unwrap();
    assert!(enabled_in(&features));
    cache_features(db, &f.owner, &features);
    reads.store(0, Ordering::SeqCst);
    assert!(enabled(db, &f.owner).await.unwrap());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    // And the other way round: a gate snapshot without the flag wins over a
    // cached `true`.
    cache_features(db, &f.owner, &[]);
    reads.store(0, Ordering::SeqCst);
    assert!(!enabled(db, &f.owner).await.unwrap());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    // Snapshots are per database and person.
    let other = person(&f).await;
    reads.store(0, Ordering::SeqCst);
    assert!(enabled(db, &other).await.unwrap());
    assert!(reads.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn a_mixed_page_reads_each_referenced_collection_once() {
    let finds = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = finds.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && let Ok(collection) = event.command.get_str("find")
        {
            seen.lock().unwrap().push(collection.to_owned());
        }
    });
    let db = connect_transaction_test_database_with_command_handler("notif_message_batch", handler)
        .await;
    let f = fixture_with(db).await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    // An agent reply in a private thread, previewed by message id.
    let thread = seed_conversation(&f, &f.owner, "Notes").await;
    let reply = Uuid::new_v4().to_string();
    db.collection::<Document>(MESSAGES)
        .insert_one(
            doc! {"_id": &reply, "conversation_id": &thread, "user_id": &f.owner,
            "role": "assistant", "text": "Summarised.", "status": "completed", "turn_id": "t-1"},
        )
        .await
        .unwrap();
    let mut update = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    update.source.message_id = Some(reply.clone());
    insert_row(&f, &update).await;
    // An automation result, previewed by its run's conversation and turn.
    let trigger = Uuid::new_v4().to_string();
    db.collection::<Document>(TRIGGER_ROWS)
        .insert_one(
            doc! {"_id": &trigger, "user_id": &f.owner, "label": "Nightly", "status": "active"},
        )
        .await
        .unwrap();
    let run_thread = seed_conversation(&f, &f.owner, "Nightly").await;
    let run = Uuid::new_v4().to_string();
    db.collection::<Document>(RUNS)
        .insert_one(
            doc! {"_id": &run, "user_id": &f.owner, "trigger_id": &trigger,
            "label": "Nightly", "outcome": "completed", "thread_id": &run_thread, "turn_id": "t-9"},
        )
        .await
        .unwrap();
    db.collection::<Document>(MESSAGES)
        .insert_one(
            doc! {"_id": Uuid::new_v4().to_string(), "conversation_id": &run_thread,
            "user_id": &f.owner, "role": "assistant", "text": "Report sent.",
            "status": "completed", "turn_id": "t-9"},
        )
        .await
        .unwrap();
    insert_row(
        &f,
        &seeded(
            &f.owner,
            NotificationKind::AutomationResult,
            NotificationSource {
                source_type: SourceType::Trigger,
                id: trigger.clone(),
                trigger_id: Some(trigger.clone()),
                run_id: Some(run.clone()),
                conversation_id: Some(run_thread.clone()),
                turn_id: Some("t-9".into()),
                ..Default::default()
            },
        ),
    )
    .await;
    finds.lock().unwrap().clear();
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items.len(), 2);
    let previews: Vec<&str> = listed
        .items
        .iter()
        .map(|item| item.preview.as_str())
        .collect();
    assert!(previews.contains(&"Summarised."));
    assert!(previews.contains(&"Report sent."));
    let finds = finds.lock().unwrap();
    assert_eq!(
        finds
            .iter()
            .filter(|collection| collection.as_str() == MESSAGES)
            .count(),
        1
    );
    // The page itself plus one lookup per referenced collection, never two.
    let mut distinct = finds.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), finds.len());
}

// ---------------------------------------------------------------------------
// Push claim authority: due time, limits, and stale snapshots
// ---------------------------------------------------------------------------

fn refusal(retry_after: Option<u64>) -> automation::Failure {
    automation::Failure {
        code: if retry_after.is_some() {
            "push_temporarily_refused"
        } else {
            "push_provider_rejected"
        },
        http_status: Some(429),
        retry_after,
    }
}

async fn claim(f: &Fixture, row: &AssistantNotification) -> Option<ClaimedPush> {
    match claim_push(&f.state.db, row).await.unwrap() {
        Claim::Claimed(claimed) => Some(*claimed),
        Claim::Skipped | Claim::NotClaimed => None,
    }
}

async fn reservations(f: &Fixture, user: &str) -> (u64, usize) {
    let db = &f.state.db;
    let sources = db
        .collection::<Document>(PUSH_SOURCES)
        .count_documents(doc! {"user_id": user})
        .await
        .unwrap();
    let budget = db
        .collection::<Document>(PUSH_BUDGETS)
        .find_one(doc! {"_id": user})
        .await
        .unwrap()
        .map_or(0, |budget| {
            budget.get_array("reservations").map_or(0, Vec::len)
        });
    (sources, budget)
}

#[tokio::test]
async fn stale_sweep_snapshot_cannot_send_a_retry_early() {
    let f = fixture("notif_push_stale").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let thread = seed_conversation(&f, &f.owner, "Thread").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    insert_row(&f, &row).await;
    // Sweep A claims from its snapshot and the provider refuses retryably.
    let first = claim(&f, &row).await.expect("first claim");
    assert_eq!(first.row.push.as_ref().unwrap().attempts, 1);
    settle_push(db, &first, PushOutcome::Refused(refusal(Some(5))))
        .await
        .unwrap();
    let push = push_of(&f, &row.id).await;
    assert_eq!(push.state, "retrying");
    assert!(push.retry_at.unwrap() > Utc::now());
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
    // Sweep B still holds the old `pending` snapshot: the claim re-reads and
    // finds a retry that is not due, so nothing is sent early.
    assert!(claim(&f, &row).await.is_none());
    assert_eq!(push_of(&f, &row.id).await.state, "retrying");
    // Once due, the claim succeeds with the persisted attempt count.
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"push.retry_at": bson_date(Utc::now() - Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let second = claim(&f, &row).await.expect("due retry");
    assert_eq!(second.row.push.as_ref().unwrap().attempts, 2);
    assert_ne!(second.attempt, first.attempt);
    assert_eq!(reservations(&f, &f.owner).await, (1, 1));
    settle_push(db, &second, PushOutcome::Accepted)
        .await
        .unwrap();
    let push = push_of(&f, &row.id).await;
    assert_eq!((push.state.as_str(), push.attempts), ("sent", 2));
    // A settled attempt token is spent: a late settlement for it is a no-op.
    settle_push(db, &first, PushOutcome::Accepted)
        .await
        .unwrap();
    assert_eq!(push_of(&f, &row.id).await.attempts, 2);
}

#[tokio::test]
async fn claim_enforces_attempt_limit_and_retry_window_from_the_persisted_row() {
    let f = fixture("notif_push_limits").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let thread = seed_conversation(&f, &f.owner, "Thread").await;
    // Exhausted attempts: the due retry settles as failed without a send.
    let mut row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    row.push = Some(PushState {
        state: "retrying".into(),
        kind: None,
        attempt: Some("spent".into()),
        code: Some("push_temporarily_refused".into()),
        attempts: PUSH_MAX_ATTEMPTS,
        claimed_at: Some(Utc::now() - Duration::minutes(5)),
        first_attempt_at: Some(Utc::now() - Duration::minutes(5)),
        retry_at: Some(Utc::now() - Duration::seconds(1)),
        sent_at: None,
    });
    insert_row(&f, &row).await;
    assert!(claim(&f, &row).await.is_none());
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (push.state.as_str(), push.code.as_deref()),
        ("failed", Some("push_retries_exhausted"))
    );
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
    // Window elapsed: one earlier attempt 31 minutes ago, retry due now.
    let other = seed_conversation(&f, &f.owner, "Other").await;
    let mut row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&other),
    );
    row.push = Some(PushState {
        state: "retrying".into(),
        kind: None,
        attempt: Some("spent".into()),
        code: None,
        attempts: 1,
        claimed_at: Some(Utc::now() - Duration::minutes(31)),
        first_attempt_at: Some(Utc::now() - Duration::minutes(31)),
        retry_at: Some(Utc::now() - Duration::seconds(1)),
        sent_at: None,
    });
    insert_row(&f, &row).await;
    assert!(claim(&f, &row).await.is_none());
    assert_eq!(push_of(&f, &row.id).await.state, "failed");
    // A retry that would land past the window fails at settlement instead.
    let third = seed_conversation(&f, &f.owner, "Third").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&third),
    );
    insert_row(&f, &row).await;
    let claimed = claim(&f, &row).await.expect("claim");
    settle_push(db, &claimed, PushOutcome::Refused(refusal(Some(3600))))
        .await
        .unwrap();
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (push.state.as_str(), push.code.as_deref()),
        ("failed", Some("push_retries_exhausted"))
    );
    assert!(push.retry_at.is_none());
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
    // The retry delay honors Retry-After above the minute floor.
    let fourth = seed_conversation(&f, &f.owner, "Fourth").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&fourth),
    );
    insert_row(&f, &row).await;
    let claimed = claim(&f, &row).await.expect("claim");
    settle_push(db, &claimed, PushOutcome::Refused(refusal(Some(120))))
        .await
        .unwrap();
    let push = push_of(&f, &row.id).await;
    assert_eq!(push.state, "retrying");
    assert!(push.retry_at.unwrap() >= Utc::now() + Duration::seconds(118));
}

#[tokio::test]
async fn the_creating_kind_drives_push_preference_and_payload_after_coalescing() {
    let f = fixture("notif_push_coalesced").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let researcher = specialist(&f, "researcher").await;
    let research = groups::create(
        db,
        &f.owner,
        "Research",
        std::slice::from_ref(&researcher.id),
        "user",
    )
    .await
    .unwrap();
    // Only mentions push.
    push_prefs(
        &f,
        &f.owner,
        NotificationPushPreferences {
            group_mention: true,
            ..Default::default()
        },
    )
    .await;
    let mention = Uuid::new_v4().to_string();
    db.collection::<Document>(GROUP_MESSAGES)
        .insert_one(
            doc! {"_id": &mention, "group_id": &research.id, "user_id": &f.owner,
            "seq": 7_i64, "role": "user", "text": "@researcher please look"},
        )
        .await
        .unwrap();
    let row = seeded(
        &f.owner,
        NotificationKind::GroupMention,
        NotificationSource {
            source_type: SourceType::Group,
            id: research.id.clone(),
            group_id: Some(research.id.clone()),
            message_id: Some(mention.clone()),
            ..Default::default()
        },
    );
    insert_row(&f, &row).await;
    // Before the sweep, the agent's reply coalesces onto the unread row: the
    // row is relabelled, the latest message moves on.
    let reply = Uuid::new_v4().to_string();
    db.collection::<Document>(GROUP_MESSAGES)
        .insert_one(
            doc! {"_id": &reply, "group_id": &research.id, "user_id": &f.owner,
            "seq": 8_i64, "role": "agent", "agent_id": &researcher.id, "text": "Looked, all good."},
        )
        .await
        .unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"kind": "agent_update", "source.message_id": &reply,
            "source.agent_id": &researcher.id, "updated_at": bson_date(Utc::now())},
            "$inc": {"count": 1_i64}},
        )
        .await
        .unwrap();
    // The opted-in mention push goes out, labelled and titled as a mention,
    // with the latest content.
    let sent: Sent = Default::default();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let (_, title, body, data) = &sent[0];
        assert_eq!(data.get("type").map(String::as_str), Some("group_mention"));
        assert_eq!(data.get("kind").map(String::as_str), Some("group_mention"));
        assert_eq!(title, "Mention in Research");
        assert_eq!(body, "Looked, all good.");
    }
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (push.state.as_str(), push.kind),
        ("sent", Some(NotificationKind::GroupMention))
    );
    assert_eq!(reservations(&f, &f.owner).await, (1, 1));
    // The reverse never sends an un-opted push: a row created as a mention
    // and relabelled to an agent update is not pushed to someone who opted
    // into agent updates only.
    push_prefs(
        &f,
        &f.owner,
        NotificationPushPreferences {
            agent_update: true,
            ..Default::default()
        },
    )
    .await;
    let design = groups::create(
        db,
        &f.owner,
        "Design",
        std::slice::from_ref(&researcher.id),
        "user",
    )
    .await
    .unwrap();
    let second = seeded(
        &f.owner,
        NotificationKind::GroupMention,
        NotificationSource {
            source_type: SourceType::Group,
            id: design.id.clone(),
            group_id: Some(design.id.clone()),
            message_id: Some(mention.clone()),
            ..Default::default()
        },
    );
    insert_row(&f, &second).await;
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &second.id},
            doc! {"$set": {"kind": "agent_update"}, "$inc": {"count": 1_i64}},
        )
        .await
        .unwrap();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    let push = push_of(&f, &second.id).await;
    assert_eq!(
        (push.state.as_str(), push.code.as_deref()),
        ("skipped", Some("preference_off"))
    );
    assert_eq!(reservations(&f, &f.owner).await, (1, 1));
}

// ---------------------------------------------------------------------------
// Push sweep shape: claims right before each send, nothing stranded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claims_are_taken_immediately_before_each_send_and_stale_claims_are_never_sent() {
    let f = fixture("notif_push_claim_timing").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let mut due = Vec::new();
    for title in ["One", "Two", "Three"] {
        let thread = seed_conversation(&f, &f.owner, title).await;
        let row = seeded(
            &f.owner,
            NotificationKind::AgentUpdate,
            conversation_source(&thread),
        );
        insert_row(&f, &row).await;
        due.push(row.id);
    }
    // A claim left behind by a crashed worker, older than the window.
    let stale_thread = seed_conversation(&f, &f.owner, "Stale").await;
    let mut stale = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&stale_thread),
    );
    stale.push = Some(PushState {
        state: "sending".into(),
        kind: Some(NotificationKind::AgentUpdate),
        attempt: Some("stale".into()),
        code: None,
        attempts: 1,
        claimed_at: Some(Utc::now() - Duration::minutes(PUSH_SENDING_STALE_MINUTES + 1)),
        first_attempt_at: Some(Utc::now() - Duration::minutes(PUSH_SENDING_STALE_MINUTES + 1)),
        retry_at: None,
        sent_at: None,
    });
    insert_row(&f, &stale).await;
    // Every provider call observes its own row claimed moments ago: the
    // barrier is taken right before the call, never for the whole batch.
    let observed: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let (sender_db, seen) = (db.clone(), observed.clone());
    sweep_with(&f.state, move |_, _, _, data| {
        let db = sender_db.clone();
        let seen = seen.clone();
        async move {
            let id = data["notification_id"].clone();
            let row: AssistantNotification = db
                .collection(COLLECTION_NAME)
                .find_one(doc! {"_id": &id})
                .await
                .unwrap()
                .unwrap();
            let push = row.push.expect("claimed");
            assert_eq!(push.state, "sending");
            assert!(push.attempt.is_some());
            assert!(Utc::now() - push.claimed_at.expect("claimed_at") < Duration::seconds(5));
            // Rows still waiting their turn are not claimed yet.
            let waiting = db
                .collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {"user_id": &row.user_id, "push.state": "sending",
                "push.claimed_at": {"$gt": bson_date(Utc::now() - Duration::minutes(1))}})
                .await
                .unwrap();
            assert_eq!(waiting, 1);
            seen.lock().unwrap().push(id);
            Ok(automation::Outcome::Accepted)
        }
    })
    .await
    .unwrap();
    let mut observed = observed.lock().unwrap().clone();
    observed.sort();
    due.sort();
    assert_eq!(observed, due);
    for id in &due {
        let push = push_of(&f, id).await;
        assert_eq!((push.state.as_str(), push.attempts), ("sent", 1));
    }
    let aged = push_of(&f, &stale.id).await;
    assert_eq!(
        (aged.state.as_str(), aged.code.as_deref(), aged.sent_at),
        ("unknown", Some("push_delivery_unknown"), None)
    );
    // A second sweep finds nothing due and sends nothing more.
    let sent: Sent = Default::default();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert!(sent.lock().unwrap().is_empty());
    assert_eq!(reservations(&f, &f.owner).await, (3, 3));
}

#[tokio::test]
async fn pre_claim_failures_strand_nothing_and_the_next_sweep_delivers() {
    let f = fixture("notif_push_recover").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let other = person(&f).await;
    push_prefs(&f, &f.owner, all_push()).await;
    push_prefs(&f, &other, all_push()).await;
    let mine = seed_conversation(&f, &f.owner, "Mine").await;
    let theirs = seed_conversation(&f, &other, "Theirs").await;
    let my_row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&mine),
    );
    insert_row(&f, &my_row).await;
    insert_row(
        &f,
        &seeded(
            &other,
            NotificationKind::AgentUpdate,
            conversation_source(&theirs),
        ),
    )
    .await;
    // The owner's preferences cannot be read (a real lookup that fails to
    // decode): nothing of theirs is claimed, everyone else is delivered.
    let settings = db.collection::<Document>(crate::models::assistant_settings::COLLECTION_NAME);
    settings
        .update_one(
            doc! {"_id": &f.owner},
            doc! {"$set": {"access_mode": "nonsense"}},
        )
        .await
        .unwrap();
    let sent: Sent = Default::default();
    assert!(
        try_sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted))
            .await
            .is_err()
    );
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, other);
    }
    let push = push_of(&f, &my_row.id).await;
    assert_eq!(
        (
            push.state.as_str(),
            push.attempts,
            push.attempt,
            push.claimed_at
        ),
        ("pending", 0, None, None)
    );
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
    settings
        .update_one(
            doc! {"_id": &f.owner},
            doc! {"$set": {"access_mode": "nyxmode"}},
        )
        .await
        .unwrap();
    // Now a source lookup fails during the owner's batched preparation: both
    // of their due rows stay queued, unclaimed.
    let researcher = specialist(&f, "researcher").await;
    let group = groups::create(
        db,
        &f.owner,
        "Ops",
        std::slice::from_ref(&researcher.id),
        "user",
    )
    .await
    .unwrap();
    let mention = Uuid::new_v4().to_string();
    db.collection::<Document>(GROUP_MESSAGES)
        .insert_one(
            doc! {"_id": &mention, "group_id": &group.id, "user_id": &f.owner,
            "seq": 3_i64, "role": "user", "text": "@researcher look"},
        )
        .await
        .unwrap();
    insert_row(
        &f,
        &seeded(
            &f.owner,
            NotificationKind::GroupMention,
            NotificationSource {
                source_type: SourceType::Group,
                id: group.id.clone(),
                group_id: Some(group.id.clone()),
                message_id: Some(mention),
                ..Default::default()
            },
        ),
    )
    .await;
    let groups_collection = db.collection::<Document>(GROUPS);
    groups_collection
        .update_one(doc! {"_id": &group.id}, doc! {"$set": {"name": 5_i64}})
        .await
        .unwrap();
    assert!(
        try_sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted))
            .await
            .is_err()
    );
    assert_eq!(sent.lock().unwrap().len(), 1);
    let mine_rows = rows_of(&f, &f.owner).await;
    assert_eq!(mine_rows.len(), 2);
    for row in &mine_rows {
        let push = row.push.as_ref().expect("push");
        assert_eq!((push.state.as_str(), push.attempts), ("pending", 0));
    }
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
    // Repaired: the next sweep delivers both.
    groups_collection
        .update_one(doc! {"_id": &group.id}, doc! {"$set": {"name": "Ops"}})
        .await
        .unwrap();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 3);
    for row in rows_of(&f, &f.owner).await {
        let push = row.push.expect("push");
        assert_eq!((push.state.as_str(), push.attempts), ("sent", 1));
    }
    assert_eq!(reservations(&f, &f.owner).await, (2, 2));
}

#[tokio::test]
async fn default_off_preferences_and_a_disabled_feed_skip_without_a_claim() {
    let f = fixture("notif_push_precheck").await;
    set_flag(&f, true).await;
    let thread = seed_conversation(&f, &f.owner, "Thread").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread),
    );
    insert_row(&f, &row).await;
    let sent: Sent = Default::default();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (
            push.state.as_str(),
            push.code.as_deref(),
            push.attempts,
            push.attempt
        ),
        ("skipped", Some("preference_off"), 0, None)
    );
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
    set_flag(&f, false).await;
    let other = seed_conversation(&f, &f.owner, "Other").await;
    let row = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&other),
    );
    insert_row(&f, &row).await;
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    let push = push_of(&f, &row.id).await;
    assert_eq!(
        (push.state.as_str(), push.code.as_deref(), push.attempts),
        ("skipped", Some("flag_off"), 0)
    );
    assert!(sent.lock().unwrap().is_empty());
    assert_eq!(reservations(&f, &f.owner).await, (0, 0));
}

#[tokio::test]
async fn a_row_that_coalesces_after_preparation_gives_its_claim_back_and_sends_the_newer_preview_next()
 {
    let f = fixture("notif_push_requeue").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let thread_a = seed_conversation(&f, &f.owner, "First").await;
    let thread_b = seed_conversation(&f, &f.owner, "Second").await;
    let older = Uuid::new_v4().to_string();
    let newer = Uuid::new_v4().to_string();
    for (seq, (id, text)) in [(&older, "Older text"), (&newer, "Newer text")]
        .into_iter()
        .enumerate()
    {
        db.collection::<Document>(MESSAGES)
            .insert_one(
                doc! {"_id": id, "conversation_id": &thread_b, "user_id": &f.owner,
                "seq": seq as i64 + 1, "role": "assistant", "text": text,
                "status": "completed", "turn_id": "t"},
            )
            .await
            .unwrap();
    }
    let row_a = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread_a),
    );
    let mut row_b = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&thread_b),
    );
    row_b.source.message_id = Some(older.clone());
    // The sweep works oldest first: A is sent before B is claimed.
    row_b.created_at = row_a.created_at + Duration::seconds(1);
    row_b.updated_at = row_b.created_at;
    insert_row(&f, &row_a).await;
    insert_row(&f, &row_b).await;
    // While A is being sent, B coalesces a newer reply (its push barrier is
    // untouched): B's prepared preview is now stale.
    let sent: Sent = Default::default();
    let mutated = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (sender_db, sent_in, mutated_in, b_id, newer_id) = (
        db.clone(),
        sent.clone(),
        mutated.clone(),
        row_b.id.clone(),
        newer.clone(),
    );
    sweep_with(&f.state, move |user, title, body, data| {
        let db = sender_db.clone();
        let sent = sent_in.clone();
        let mutated = mutated_in.clone();
        let (b_id, newer_id) = (b_id.clone(), newer_id.clone());
        async move {
            if !mutated.swap(true, Ordering::SeqCst) {
                db.collection::<Document>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &b_id},
                        doc! {"$set": {"source.message_id": &newer_id,
                        "updated_at": bson_date(Utc::now())}, "$inc": {"count": 1_i64}},
                    )
                    .await
                    .unwrap();
            }
            sent.lock().unwrap().push((user, title, body, data));
            Ok(automation::Outcome::Accepted)
        }
    })
    .await
    .unwrap();
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].3.get("notification_id").map(String::as_str),
            Some(row_a.id.as_str())
        );
    }
    // B's claim was given back exactly: nothing consumed, nothing reserved.
    let push = push_of(&f, &row_b.id).await;
    assert_eq!(
        (
            push.state.as_str(),
            push.attempts,
            push.attempt,
            push.claimed_at
        ),
        ("pending", 0, None, None)
    );
    assert_eq!(reservations(&f, &f.owner).await, (1, 1));
    // The next sweep prepares the newer references and sends B once.
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(
            sent[1].3.get("notification_id").map(String::as_str),
            Some(row_b.id.as_str())
        );
        assert_eq!(sent[1].2, "Newer text");
    }
    let push = push_of(&f, &row_b.id).await;
    assert_eq!((push.state.as_str(), push.attempts), ("sent", 1));
    assert_eq!(reservations(&f, &f.owner).await, (2, 2));
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_continually_coalescing_row_is_pushed_with_its_prepared_preview_once_past_the_grace() {
    let finds = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = finds.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && let Ok(collection) = event.command.get_str("find")
        {
            seen.lock().unwrap().push(collection.to_owned());
        }
    });
    let db =
        connect_transaction_test_database_with_command_handler("notif_push_starvation", handler)
            .await;
    let f = fixture_with(db).await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let busy_thread = seed_conversation(&f, &f.owner, "Busy").await;
    let trigger_thread = seed_conversation(&f, &f.owner, "Trigger").await;
    let texts = [
        ("older", "Older text"),
        ("newer", "Newer text"),
        ("newest", "Newest text"),
    ];
    let mut message_ids = HashMap::new();
    for (seq, (key, text)) in texts.into_iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        db.collection::<Document>(MESSAGES)
            .insert_one(
                doc! {"_id": &id, "conversation_id": &busy_thread, "user_id": &f.owner,
                "seq": seq as i64 + 1, "role": "assistant", "text": text,
                "status": "completed", "turn_id": key},
            )
            .await
            .unwrap();
        message_ids.insert(key, id);
    }
    // The busy row keeps coalescing; the trigger row's send is where the
    // coalesce lands (after preparation, before the busy row's claim).
    let mut busy = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&busy_thread),
    );
    busy.source.message_id = Some(message_ids["older"].clone());
    let mut trigger = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&trigger_thread),
    );
    trigger.created_at = busy.created_at - Duration::seconds(1);
    trigger.updated_at = trigger.created_at;
    insert_row(&f, &busy).await;
    insert_row(&f, &trigger).await;
    let coalesce = |db: Database, busy_id: String, message_id: String| async move {
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &busy_id},
                doc! {"$set": {"source.message_id": &message_id,
                "updated_at": bson_date(Utc::now())}, "$inc": {"count": 1_i64}},
            )
            .await
            .unwrap();
    };
    let sent: Sent = Default::default();
    let sweep_with_coalesce = |message_key: &'static str| {
        let (sender_db, sent_in, busy_id, message_id, fired) = (
            db.clone(),
            sent.clone(),
            busy.id.clone(),
            message_ids[message_key].clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        sweep_with(&f.state, move |user, title, body, data| {
            let db = sender_db.clone();
            let sent = sent_in.clone();
            let (busy_id, message_id, fired) = (busy_id.clone(), message_id.clone(), fired.clone());
            async move {
                if !fired.swap(true, Ordering::SeqCst) {
                    coalesce(db, busy_id, message_id).await;
                }
                sent.lock().unwrap().push((user, title, body, data));
                Ok(automation::Outcome::Accepted)
            }
        })
    };
    // Young row: the stale claim is given back.
    sweep_with_coalesce("newer").await.unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    let push = push_of(&f, &busy.id).await;
    assert_eq!((push.state.as_str(), push.attempts), ("pending", 0));
    assert_eq!(reservations(&f, &f.owner).await, (1, 1));
    // The row keeps coalescing sweep after sweep. Once it is older than the
    // grace, the prepared preview goes out in the same sweep: no requeue, no
    // extra lookup on any referenced collection.
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &busy.id},
            doc! {"$set": {"created_at": bson_date(
                Utc::now() - Duration::minutes(PUSH_REQUEUE_GRACE_MINUTES + 1)
            )}},
        )
        .await
        .unwrap();
    let mut trigger2 = seeded(
        &f.owner,
        NotificationKind::AgentUpdate,
        conversation_source(&seed_conversation(&f, &f.owner, "Trigger two").await),
    );
    trigger2.created_at = Utc::now() - Duration::minutes(PUSH_REQUEUE_GRACE_MINUTES + 2);
    trigger2.updated_at = trigger2.created_at;
    insert_row(&f, &trigger2).await;
    finds.lock().unwrap().clear();
    sweep_with_coalesce("newest").await.unwrap();
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 3);
        assert_eq!(
            sent[2].3.get("notification_id").map(String::as_str),
            Some(busy.id.as_str())
        );
        // The prepared snapshot, not the body that landed during the sweep.
        assert_eq!(sent[2].2, "Newer text");
    }
    let push = push_of(&f, &busy.id).await;
    assert_eq!((push.state.as_str(), push.attempts), ("sent", 1));
    assert_eq!(reservations(&f, &f.owner).await, (3, 3));
    {
        let finds = finds.lock().unwrap();
        for collection in [MESSAGES, CONVERSATIONS, AGENTS] {
            assert!(
                finds
                    .iter()
                    .filter(|name| name.as_str() == collection)
                    .count()
                    <= 1,
                "{collection} read more than once"
            );
        }
    }
    // Nothing is due any more; the newest body arrives with the next activity.
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    assert_eq!(sent.lock().unwrap().len(), 3);
}

// ---------------------------------------------------------------------------
// Owner home thread heard through several bots
// ---------------------------------------------------------------------------

#[tokio::test]
async fn owner_home_thread_rows_follow_the_bot_the_owner_wrote_from_last() {
    let f = fixture("notif_home_bots").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let (bot_a, thread_a) = seed_channel(&f, &f.owner, None).await;
    let (bot_b, thread_b) = seed_channel(&f, &f.owner, None).await;
    db.collection::<Document>(CHANNELS)
        .update_one(
            doc! {"_id": &bot_b},
            doc! {"$set": {"bot_label": "Sales bot"}},
        )
        .await
        .unwrap();
    db.collection::<Document>(CHANNEL_THREADS)
        .update_one(doc! {"_id": &thread_b}, doc! {"$set": {"title": "Bob"}})
        .await
        .unwrap();
    // The owner writes to NyxBot from bot A: the reply is read there.
    let mut via_a = start(Some(&home.id), "hi via A", TurnOrigin::Channel);
    via_a.reply_channel = Some(channel_origin(&bot_a));
    let row = begin(&f, &f.owner, &via_a).await;
    engine::set_reply_channel(db, &f.owner, &home.id, &channel_origin(&bot_a))
        .await
        .unwrap();
    settle(&f, &row, "Reply via A", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::ChatUpdate);
    assert_eq!(rows[0].source.source_type, SourceType::ChannelChat);
    assert_eq!(rows[0].source.id, home.id);
    assert_eq!(rows[0].source.channel_id.as_deref(), Some(bot_a.as_str()));
    assert_eq!(rows[0].source.thread_id.as_deref(), Some(thread_a.as_str()));
    assert_eq!(
        list(&f.state, &f.owner, &page(10)).await.unwrap().items[0].title,
        "Support bot · Alice"
    );
    // Then from bot B: the same unread row now names bot B and its chat.
    let mut via_b = start(Some(&home.id), "hi via B", TurnOrigin::Channel);
    via_b.reply_channel = Some(channel_origin(&bot_b));
    let row = begin(&f, &f.owner, &via_b).await;
    engine::set_reply_channel(db, &f.owner, &home.id, &channel_origin(&bot_b))
        .await
        .unwrap();
    let reply_b = settle(&f, &row, "Reply via B", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].count, 2);
    assert_eq!(rows[0].source.channel_id.as_deref(), Some(bot_b.as_str()));
    assert_eq!(rows[0].source.thread_id.as_deref(), Some(thread_b.as_str()));
    assert_eq!(rows[0].source.message_id.as_deref(), Some(reply_b.as_str()));
    let listed = list(&f.state, &f.owner, &page(10)).await.unwrap();
    assert_eq!(listed.items[0].title, "Sales bot · Bob");
    assert_eq!(listed.items[0].preview, "Reply via B");
    // Retiring bot A purges nothing of bot B's; retiring bot B does.
    purge(db, doc! {fields::SOURCE_CHANNEL_ID: &bot_a}, None)
        .await
        .unwrap();
    assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
    purge(db, doc! {fields::SOURCE_CHANNEL_ID: &bot_b}, None)
        .await
        .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

#[path = "assistant_notification_pm_tests.rs"]
mod pm_review;
