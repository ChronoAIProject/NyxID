//! NyxBot Notifications behavioral tests against real MongoDB transitions.
use super::*;
use crate::{
    models::{
        assistant_conversation::ChannelOrigin,
        assistant_settings::{NotificationPreferences, NotificationPushPreferences},
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        trigger::{COLLECTION_NAME as TRIGGER_ROWS, Trigger, TriggerDelivery, TriggerVerification},
        trigger_schedule::{
            IntervalUnit, OverlapPolicy, ScheduleKind, ScheduleSpec, TriggerSource,
        },
        user::{COLLECTION_NAME as USER_ROWS, UserType},
    },
    services::{
        assistant_acknowledgement_service as acks,
        assistant_agent_credential_service as credentials, assistant_group_service as groups,
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
    reset_flag_cache();
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
    let credential = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &row.user_id,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let message_id = Uuid::new_v4().to_string();
    engine::finish_turn(
        &f.state.db,
        row,
        &credential.api_key_id,
        &message_id,
        &engine::TurnResult {
            text: text.into(),
            session_id: Some("conv_session".into()),
            response_id: Some("resp_1".into()),
            error,
        },
    )
    .await
    .unwrap();
    message_id
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
        push: Some(push_state(&PushPlan::Pending)),
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
    assert!(row.active_turn.as_ref().unwrap().notifications_enabled);
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
    settle(&f, &next, "ok", None).await;
    // Destroying the agent stops further updates for its threads.
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&thread.id), "More", TurnOrigin::Orchestrator),
    )
    .await;
    team::destroy(db, &f.owner, &researcher.id).await.unwrap();
    settle(&f, &row, "late", Some(TurnError::new("cancelled"))).await;
    assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
}

// ---------------------------------------------------------------------------
// Channel chats
// ---------------------------------------------------------------------------

async fn seed_channel(f: &Fixture, owner: &str, org: Option<&str>) -> (String, String) {
    let channel = Uuid::new_v4().to_string();
    let thread = Uuid::new_v4().to_string();
    let mut doc = doc! {"_id": &channel, "user_id": owner, "channel_bot_id": "bot",
    "platform": "telegram", "status": "active", "bot_label": "Support bot", "transport": "gateway"};
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
            "partition": "chat-1", "title": "Alice", "kind": "private"},
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
    assert_eq!(rows_of(&f, &f.owner).await.len(), 1);
    assert!(
        list(&f.state, &f.owner, &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(rows_of(&f, &f.owner).await.is_empty());
    // Chat removal before settlement: no live thread, no row.
    db.collection::<Document>(CHANNELS)
        .update_one(doc! {"_id": &channel}, doc! {"$set": {"status": "active"}})
        .await
        .unwrap();
    db.collection::<Document>(CHANNEL_THREADS)
        .delete_one(doc! {"_id": &thread})
        .await
        .unwrap();
    let row3 = begin(&f, &f.owner, &again).await;
    settle(&f, &row3, "after removal", None).await;
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
    let reply = settle(&f, &row, "private lane text", None).await;
    // The sanitized public transcript entry the settlement would publish.
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
    let run = schedules::new_run(trigger, at, at + Duration::hours(1));
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
        role: row.role.clone(),
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
    assert!(rows[0].resolved_at.is_some());
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

// ---------------------------------------------------------------------------
// Web groups
// ---------------------------------------------------------------------------

#[tokio::test]
async fn group_notices_coalesce_and_outside_posts_do_not_notify_and_group_reads_alias_by_id() {
    let f = fixture("notif_groups").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let researcher = specialist(&f, "researcher").await;
    let group = groups::create(db, &f.owner, "Ops", &[researcher.id.clone()], "user")
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
    // Deleting the group purges its rows at the next read of a dangling row.
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
    purge(db, doc! {fields::SOURCE_GROUP_ID: &group.id}, None)
        .await
        .unwrap();
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
    let group = crate::models::assistant_group::AssistantGroup {
        id: Uuid::new_v4().to_string(),
        user_id: f.owner.clone(),
        collaboration: None,
        participant_user_ids: Vec::new(),
        created_by_user_id: None,
        name: "Ops".into(),
        member_agent_ids: Vec::new(),
        lead_agent_id: String::new(),
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
        agent_id: None,
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

async fn sweep_capturing(
    f: &Fixture,
    sent: &Sent,
    outcome: impl Fn() -> Result<automation::Outcome, &'static str> + Clone,
) {
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
    .unwrap();
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
    let long = "word ".repeat(100);
    let preview = preview_text(&long);
    assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);
    assert!(preview.ends_with('…'));
    assert_eq!(preview_text("a\u{202e}b\tc"), "ab c");
    assert_eq!(parse_kinds("agent_update, chat_update").unwrap().len(), 2);
    assert!(parse_kinds("nope").is_err());
    assert_eq!(reason_copy("grace_expired").starts_with("Skipped"), true);
}
