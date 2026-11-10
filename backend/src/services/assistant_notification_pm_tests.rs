//! Regressions requested by PM review round 1. These exercise the real MongoDB paths.
use super::*;

async fn produce(f: &Fixture, user: &str, conversation: &str) {
    let db = f.state.db.clone();
    let user = user.to_owned();
    let source = conversation_source(conversation);
    let mut session = db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = record_in_session(
                &db,
                session,
                NewRow {
                    user_id: &user,
                    kind: NotificationKind::AgentUpdate,
                    source: source.clone(),
                    id: None,
                    outcome: Some("completed".into()),
                    reason: None,
                    card_expires_at: None,
                    push: PushPlan::Pending,
                },
            )
            .await;
            transactions::transaction_result(result)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn pm_f1_preference_off_backlog_never_enters_the_queue_ahead_of_an_opted_in_person() {
    let f = fixture("pm_f1_queue").await;
    set_flag(&f, true).await;
    for _ in 0..65 {
        let conversation = seed_conversation(&f, &f.owner, "Push disabled").await;
        produce(&f, &f.owner, &conversation).await;
    }
    let opted_in = person(&f).await;
    push_prefs(&f, &opted_in, all_push()).await;
    let conversation = seed_conversation(&f, &opted_in, "Push enabled").await;
    produce(&f, &opted_in, &conversation).await;
    let sent: Sent = Default::default();
    sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)).await;
    let recipients: Vec<_> = sent
        .lock()
        .unwrap()
        .iter()
        .map(|row| row.0.clone())
        .collect();
    assert_eq!(
        recipients,
        [opted_in],
        "preference-off traffic must not occupy the global push batch"
    );
    for row in rows_of(&f, &f.owner).await {
        let push = row.push.unwrap();
        assert_eq!(push.state, "skipped");
        assert_eq!(push.code.as_deref(), Some("preference_off"));
        assert_eq!(push.attempts, 0);
        assert!(push.claimed_at.is_none());
    }
}

#[tokio::test]
async fn pm_f2_a_never_returning_provider_has_a_bounded_sweep_and_cannot_block_shared_recovery() {
    let f = fixture("pm_f2_deadline").await;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    for _ in 0..4 {
        let conversation = seed_conversation(&f, &f.owner, "Slow transport").await;
        insert_row(
            &f,
            &seeded(
                &f.owner,
                NotificationKind::AgentUpdate,
                conversation_source(&conversation),
            ),
        )
        .await;
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_send = calls.clone();
    let next_step = Arc::new(AtomicUsize::new(0));
    let next_step_after_send = next_step.clone();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(22), async {
        sweep_with(&f.state, move |_, _, _, _| {
            calls_for_send.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<Result<automation::Outcome, &'static str>>()
        })
        .await
        .unwrap();
        next_step_after_send.fetch_add(1, Ordering::SeqCst);
    })
    .await;
    assert!(
        completed.is_ok(),
        "push delivery exceeded the 20-second sweep budget and blocked the next recovery step"
    );
    assert_eq!(next_step.load(Ordering::SeqCst), 1);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "independent sources must send with bounded concurrency"
    );
    let source = include_str!("../handlers/assistant_team.rs");
    assert!(
        source.contains("assistant_notification_service::spawn_worker(state.clone())"),
        "push must have its own task"
    );
    assert!(
        !source.contains("assistant_notification_service::sweep(&state)"),
        "the shared assistant sweep must not await push delivery"
    );
    for row in rows_of(&f, &f.owner).await {
        assert_eq!(
            row.push.unwrap().state,
            "unknown",
            "started sends time out ambiguously and must never be replayed"
        );
    }
}

#[tokio::test]
async fn pm_f2_two_concurrent_workers_send_each_notification_once() {
    let f = fixture("pm_f2_workers").await;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    for _ in 0..8 {
        let conversation = seed_conversation(&f, &f.owner, "Concurrent workers").await;
        insert_row(
            &f,
            &seeded(
                &f.owner,
                NotificationKind::AgentUpdate,
                conversation_source(&conversation),
            ),
        )
        .await;
    }
    let sent: Sent = Default::default();
    let (a, b) = tokio::join!(
        try_sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
        try_sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
    );
    a.unwrap();
    b.unwrap();
    let sent = sent.lock().unwrap();
    let ids: HashSet<_> = sent
        .iter()
        .map(|row| row.3["notification_id"].clone())
        .collect();
    assert_eq!(sent.len(), 8);
    assert_eq!(ids.len(), 8);
}

#[tokio::test]
async fn pm_f3_different_chats_settle_without_a_shared_channel_write_or_notification_retries() {
    let channel_writes = Arc::new(AtomicUsize::new(0));
    let inserts = Arc::new(std::sync::Mutex::new(HashMap::<String, usize>::new()));
    let (writes, attempts) = (channel_writes.clone(), inserts.clone());
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            if event.command.get_str("findAndModify").ok() == Some(CHANNELS) {
                writes.fetch_add(1, Ordering::SeqCst);
            }
            // The very first read in finish_turn_inner's transaction body:
            // count body entries, including attempts aborted before insertion.
            if event.command.get_str("find").ok() == Some(MESSAGES)
                && let Ok(filter) = event.command.get_document("filter")
                && filter.len() == 4
                && filter.contains_key("_id")
                && filter.contains_key("turn_id")
                && let Ok(conversation) = filter.get_str("conversation_id")
            {
                *attempts
                    .lock()
                    .unwrap()
                    .entry(conversation.into())
                    .or_default() += 1;
            }
        }
    });
    let db = connect_transaction_test_database_with_command_handler("pm_f3_chats", handler).await;
    let f = fixture_with(db).await;
    set_flag(&f, true).await;
    let (channel, _) = seed_channel(&f, &f.owner, None).await;
    let mut rows = Vec::new();
    for index in 0..6 {
        let partition = format!("chat-{index}");
        if index != 1 {
            f.state
                .db
                .collection::<Document>(CHANNEL_THREADS)
                .insert_one(doc! {
                    "_id": Uuid::new_v4().to_string(), "channel_id": &channel, "user_id": &f.owner,
                    "partition": &partition, "title": &partition, "kind": "private",
                    "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
                })
                .await
                .unwrap();
        }
        let mut input = start(None, "Guest question", TurnOrigin::Channel);
        let mut origin = channel_origin(&channel);
        origin.partition = partition;
        input.channel = Some(origin);
        input.guest = true;
        rows.push(begin(&f, &f.owner, &input).await);
    }
    channel_writes.store(0, Ordering::SeqCst);
    inserts.lock().unwrap().clear();
    futures::future::join_all(rows.iter().map(|row| settle(&f, row, "Guest answer", None))).await;
    assert_eq!(rows_of(&f, &f.owner).await.len(), 6);
    let attempts = inserts.lock().unwrap().clone();
    assert_eq!(
        channel_writes.load(Ordering::SeqCst),
        0,
        "settlements in different chats must not write the bot's shared channel document"
    );
    for row in rows {
        assert_eq!(
            attempts.get(&row.id),
            Some(&1),
            "settlement transaction body retried for {}",
            row.id
        );
    }
    let fenced = f
        .state
        .db
        .collection::<Document>(CHANNEL_THREADS)
        .count_documents(doc! {"channel_id": &channel, "notification_fence": {"$gt": 0}})
        .await
        .unwrap();
    assert_eq!(fenced, 6, "each chat has its own write fence");
}

#[tokio::test]
async fn pm_f4_channel_and_event_activity_share_one_home_row_and_unlink_follows_latest_activity() {
    let f = fixture("pm_f4_home").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let bot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = team::home_thread(db, &f.state.encryption_keys, &bot)
        .await
        .unwrap();
    let (channel, _) = seed_channel(&f, &f.owner, None).await;
    let mut input = start(Some(&home.id), "Channel DM", TurnOrigin::Channel);
    input.reply_channel = Some(channel_origin(&channel));
    let row = begin(&f, &f.owner, &input).await;
    engine::set_reply_channel(db, &f.owner, &home.id, &channel_origin(&channel))
        .await
        .unwrap();
    settle(&f, &row, "Channel reply", None).await;
    engine::push_events(
        db,
        &f.owner,
        &home.id,
        vec![team::event(
            "message",
            "Background work finished".into(),
            None,
        )],
    )
    .await
    .unwrap();
    let row = begin(
        &f,
        &f.owner,
        &start(Some(&home.id), "Background event", TurnOrigin::Event),
    )
    .await;
    settle(&f, &row, "Event reply", None).await;
    let rows = rows_of(&f, &f.owner).await;
    assert_eq!(
        rows.len(),
        1,
        "one visible home thread must have one coalesced unread row"
    );
    assert_eq!(rows[0].count, 2);
    assert_eq!(rows[0].kind, NotificationKind::AgentUpdate);
    assert!(rows[0].source.channel_id.is_none());
    assert_eq!(summary(db, &f.owner).await.unwrap().unread_count, 1);
    assert_eq!(
        list(&f.state, &f.owner, &page(50))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    purge(db, doc! {fields::SOURCE_CHANNEL_ID: &channel}, None)
        .await
        .unwrap();
    assert_eq!(
        rows_of(&f, &f.owner).await.len(),
        1,
        "latest non-channel activity survives unlink"
    );
    let mut input = start(Some(&home.id), "Another DM", TurnOrigin::Channel);
    input.reply_channel = Some(channel_origin(&channel));
    let row = begin(&f, &f.owner, &input).await;
    engine::set_reply_channel(db, &f.owner, &home.id, &channel_origin(&channel))
        .await
        .unwrap();
    settle(&f, &row, "Latest channel reply", None).await;
    assert_eq!(rows_of(&f, &f.owner).await[0].count, 3);
    purge(db, doc! {fields::SOURCE_CHANNEL_ID: &channel}, None)
        .await
        .unwrap();
    assert!(
        rows_of(&f, &f.owner).await.is_empty(),
        "latest channel activity is removed with that channel"
    );
}

#[tokio::test]
async fn pm_f6_list_and_summary_share_the_same_live_visibility_matrix() {
    let cases = [
        "personal",
        "org",
        "revoked_membership",
        "inactive_org",
        "collaboration_removed",
        "retired_channel",
        "unlinked_platform",
        "deleted_trigger",
        "deleted_run",
        "decided_card",
        "expired_card",
        "pending_card",
        "org_guest_non_admin",
        "org_guest_admin",
    ];
    let mut differences = Vec::new();
    for case in cases {
        let f = fixture(&format!("pm_f6_{case}")).await;
        let db = &f.state.db;
        let conversation = seed_conversation(&f, &f.owner, case).await;
        let mut source = conversation_source(&conversation);
        let mut kind = NotificationKind::AgentUpdate;
        let expected = matches!(
            case,
            "personal" | "org" | "pending_card" | "org_guest_admin"
        );
        if matches!(
            case,
            "org"
                | "revoked_membership"
                | "inactive_org"
                | "org_guest_non_admin"
                | "org_guest_admin"
        ) {
            let org = Uuid::new_v4().to_string();
            let mut owner = test_user(&org, UserType::Org);
            if case == "inactive_org" {
                owner.is_active = false;
            }
            db.collection(USER_ROWS).insert_one(owner).await.unwrap();
            let role = if case == "org_guest_admin" {
                OrgRole::Admin
            } else {
                OrgRole::Member
            };
            let mut membership = test_membership(&org, &f.owner, role, None);
            membership.revoked_at = (case == "revoked_membership").then(Utc::now);
            db.collection(MEMBERSHIPS)
                .insert_one(membership)
                .await
                .unwrap();
            db.collection::<Document>(CONVERSATIONS)
                .update_one(
                    doc! {"_id": &conversation},
                    doc! {"$set": {"agent_owner_id": &org}},
                )
                .await
                .unwrap();
            if case.starts_with("org_guest_") {
                let (channel, thread) = seed_channel(&f, &f.owner, Some(&org)).await;
                kind = NotificationKind::ChatUpdate;
                source.source_type = SourceType::ChannelChat;
                source.id = thread.clone();
                source.conversation_id = None;
                source.channel_id = Some(channel);
                source.thread_id = Some(thread);
                source.lane = Some(LANE_ORG_GUEST.into());
            }
        }
        if case == "collaboration_removed" {
            let group = Uuid::new_v4().to_string();
            db.collection::<Document>(GROUPS).insert_one(doc! {
                "_id": &group, "user_id": &f.owner, "name": "Collaboration",
                "collaboration": {"version": i64::from(crate::models::assistant_group_collaboration::VERSION), "creator_user_id": &f.owner, "membership_revision": 1},
                "participant_user_ids": [], "member_agent_ids": [], "lead_agent_id": "", "last_recipient_ids": [],
                "created_by": "user", "message_count": 0, "pending_agent_ids": [], "hops_remaining": 0, "followers": [],
                "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
            }).await.unwrap();
            kind = NotificationKind::GroupMention;
            source = NotificationSource {
                source_type: SourceType::CollaborationGroup,
                id: group.clone(),
                group_id: Some(group),
                ..Default::default()
            };
        }
        if case == "retired_channel" {
            let (channel, thread) = seed_channel(&f, &f.owner, None).await;
            db.collection::<Document>(CHANNELS)
                .update_one(
                    doc! {"_id": &channel},
                    doc! {"$set": {"status": "disconnected"}},
                )
                .await
                .unwrap();
            kind = NotificationKind::ChatUpdate;
            source.source_type = SourceType::ChannelChat;
            source.channel_id = Some(channel);
            source.thread_id = Some(thread);
        }
        if case == "unlinked_platform" {
            let binding = Uuid::new_v4().to_string();
            db.collection::<Document>(PLATFORM_BINDINGS).insert_one(doc! {"_id": &binding, "user_id": &f.owner, "generation": 1_i64, "conversation_id": &conversation, "state": "revoked"}).await.unwrap();
            db.collection::<Document>(CONVERSATIONS).update_one(doc! {"_id": &conversation}, doc! {"$set": {"platform_thread": {"binding_id": &binding, "binding_generation": 1_i64, "user_id": &f.owner, "platform": "telegram"}}}).await.unwrap();
            kind = NotificationKind::ChatUpdate;
            source.source_type = SourceType::ChannelChat;
            source.platform_binding_id = Some(binding);
            source.platform_binding_generation = Some(1);
        }
        if matches!(case, "deleted_trigger" | "deleted_run") {
            let trigger = Uuid::new_v4().to_string();
            let run = Uuid::new_v4().to_string();
            if case != "deleted_trigger" {
                db.collection::<Document>(TRIGGERS)
                    .insert_one(doc! {"_id": &trigger, "user_id": &f.owner, "label": "Run"})
                    .await
                    .unwrap();
            }
            if case != "deleted_run" {
                db.collection::<Document>(RUNS)
                    .insert_one(doc! {"_id": &run, "user_id": &f.owner, "trigger_id": &trigger})
                    .await
                    .unwrap();
            }
            kind = NotificationKind::AutomationResult;
            source.source_type = SourceType::Trigger;
            source.id = trigger.clone();
            source.trigger_id = Some(trigger);
            source.run_id = Some(run);
        }
        let mut row = seeded(&f.owner, kind, source);
        if case.ends_with("_card") {
            let id = Uuid::new_v4().to_string();
            let expires =
                Utc::now() + Duration::minutes(if case == "expired_card" { -1 } else { 10 });
            db.collection::<Document>(ACKS).insert_one(doc! {"_id": &id, "user_id": &f.owner, "conversation_id": &conversation, "summary": "Allow?", "status": if case == "decided_card" {"allowed"} else {"pending"}, "expires_at": bson_date(expires)}).await.unwrap();
            row.kind = NotificationKind::NeedsDecision;
            row.coalesced = false;
            row.source.card_id = Some(id);
            row.card_expires_at = Some(expires);
        }
        insert_row(&f, &row).await;
        let before = summary(db, &f.owner).await.unwrap().unread_count;
        let listed = list(
            &f.state,
            &f.owner,
            &ListQuery {
                limit: 50,
                unread_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let after = summary(db, &f.owner).await.unwrap().unread_count;
        let want = i64::from(expected);
        if before != want || listed.items.len() as i64 != want || after != want {
            differences.push(format!("{case}: expected {want}, summary before={before}, unread list={}, summary after={after}", listed.items.len()));
        }
    }
    assert!(
        differences.is_empty(),
        "list/summary ACL matrix disagreed:\n{}",
        differences.join("\n")
    );
}

#[tokio::test]
async fn pm_f1_creation_preferences_refresh_on_put_and_expire_on_other_replicas() {
    let f = fixture("pm_f1_preference_cache").await;
    // A default-off cache entry is replaced by the settings PUT in this process.
    assert!(matches!(
        creation_push_plan(&f.state.db, &f.owner, NotificationKind::AgentUpdate)
            .await
            .unwrap(),
        PushPlan::Skipped("preference_off")
    ));
    push_prefs(&f, &f.owner, all_push()).await;
    assert!(matches!(
        creation_push_plan(&f.state.db, &f.owner, NotificationKind::AgentUpdate)
            .await
            .unwrap(),
        PushPlan::Pending
    ));
    // Simulate a settings PUT on a different process: the old local decision
    // remains valid for at most the documented 30-second cache lifetime.
    f.state
        .db
        .collection::<Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.owner},
            doc! {"$set": {"notifications.push.agent_update": false}},
        )
        .await
        .unwrap();
    assert!(matches!(
        creation_push_plan(&f.state.db, &f.owner, NotificationKind::AgentUpdate)
            .await
            .unwrap(),
        PushPlan::Pending
    ));
    {
        let key = flag_cache_key(&f.state.db, &f.owner);
        let mut cache = push_preference_cache().lock().unwrap();
        cache.get_mut(&key).unwrap().0 = Instant::now() - PUSH_PREFERENCE_CACHE_TTL;
    }
    assert!(matches!(
        creation_push_plan(&f.state.db, &f.owner, NotificationKind::AgentUpdate)
            .await
            .unwrap(),
        PushPlan::Skipped("preference_off")
    ));
    assert!(push_preference_cache().lock().unwrap().len() <= PUSH_PREFERENCE_CACHE_CAPACITY);
}

#[tokio::test]
async fn pm_f2_delivery_concurrency_is_bounded_even_for_one_person() {
    let f = fixture("pm_f2_concurrency").await;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    for _ in 0..8 {
        let conversation = seed_conversation(&f, &f.owner, "Concurrent delivery").await;
        insert_row(
            &f,
            &seeded(
                &f.owner,
                NotificationKind::AgentUpdate,
                conversation_source(&conversation),
            ),
        )
        .await;
    }
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (sender_active, sender_peak) = (active.clone(), peak.clone());
    tokio::time::timeout(
        std::time::Duration::from_secs(22),
        sweep_with(&f.state, move |_, _, _, _| {
            let (barrier, active, peak) =
                (barrier.clone(), sender_active.clone(), sender_peak.clone());
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                barrier.wait().await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(automation::Outcome::Accepted)
            }
        }),
    )
    .await
    .expect("four independent sources must make progress together")
    .unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 4);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    for row in rows_of(&f, &f.owner).await {
        assert_eq!(row.push.unwrap().state, "sent");
    }
}

#[tokio::test]
async fn pm_f3_unlink_invalidates_a_stale_settlement_snapshot() {
    use crate::models::{nyxbot_channel::NyxbotChannel, nyxbot_gateway_release::ReleaseReason};
    let f = fixture("pm_f3_unlink_snapshot").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let (channel, _) = seed_channel(&f, &f.owner, None).await;
    let mut input = start(None, "First external question", TurnOrigin::Channel);
    input.channel = Some(channel_origin(&channel));
    input.guest = true;
    let first = begin(&f, &f.owner, &input).await;
    let reply = settle(&f, &first, "First answer", None).await;
    let mut message = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id": &reply})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    input.conversation_id = Some(first.id.clone());
    let late = begin(&f, &f.owner, &input).await;
    let turn = late.active_turn.as_ref().unwrap();
    message.id = Uuid::new_v4().to_string();
    message.turn_id = turn.turn_id.clone();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    // Establish the old source snapshot before real unlink commits.
    db.collection::<Document>(CHANNELS)
        .find_one(doc! {"_id": &channel})
        .session(&mut tx)
        .await
        .unwrap()
        .unwrap();
    let connection = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &channel})
        .await
        .unwrap()
        .unwrap();
    crate::services::nyxbot_gateway_release_service::retire_connection(
        &f.state,
        &connection,
        ReleaseReason::Disconnect,
        "disconnected",
        None,
    )
    .await
    .unwrap();
    let error = record_turn_settled_in_session(db, &mut tx, &late, turn, &message)
        .await
        .expect_err("a stale snapshot must conflict on the per-chat fence");
    assert!(
        matches!(error, AppError::DatabaseError(error) if error.contains_label("TransientTransactionError"))
    );
    tx.abort_transaction().await.unwrap();
    // The actual settlement retries against the retired channel and still
    // saves its durable reply without recreating a feed row.
    settle(&f, &late, "Late answer is saved", None).await;
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

#[tokio::test]
async fn pm_f3_trigger_deletion_invalidates_a_stale_run_snapshot_without_a_trigger_write() {
    let f = fixture("pm_f3_trigger_snapshot").await;
    let db = &f.state.db;
    set_flag(&f, true).await;
    let bot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let trigger = automation(&f, &bot.id, DeliverTo::Thread).await;
    let mut run = run_for(&f, &trigger).await;
    run.outcome = RunOutcome::Completed;
    // Already terminal history cannot produce again and must not add writes
    // to deletion's fence set.
    let mut historical = run.clone();
    historical.id = Uuid::new_v4().to_string();
    historical.scheduled_at -= Duration::days(1);
    db.collection::<TriggerRun>(RUNS)
        .insert_one(&historical)
        .await
        .unwrap();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    db.collection::<Document>(TRIGGERS)
        .find_one(doc! {"_id": &trigger.id})
        .session(&mut tx)
        .await
        .unwrap()
        .unwrap();
    trigger_service::delete(db, &trigger).await.unwrap();
    let retained = db
        .collection::<Document>(RUNS)
        .find_one(doc! {"_id": &historical.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!retained.contains_key("notification_fence"));
    let error = record_run_terminal_in_session(db, &mut tx, &run)
        .await
        .expect_err("trigger deletion must invalidate a producer's old run snapshot");
    assert!(
        matches!(error, AppError::DatabaseError(error) if error.contains_label("TransientTransactionError"))
    );
    tx.abort_transaction().await.unwrap();
    schedules::finish(
        db,
        &run.id,
        RunOutcome::Completed,
        None,
        &f.state.config.frontend_url,
    )
    .await
    .unwrap();
    assert!(rows_of(&f, &f.owner).await.is_empty());
}

#[tokio::test]
async fn pm_f2_a_missing_worker_singleton_is_recreated_and_delivery_continues() {
    let f = fixture("pm_f2_missing_worker").await;
    set_flag(&f, true).await;
    push_prefs(&f, &f.owner, all_push()).await;
    let conversation = seed_conversation(&f, &f.owner, "Restored source").await;
    produce(&f, &f.owner, &conversation).await;
    f.state
        .db
        .collection::<Document>(PUSH_WORKER_STATE)
        .delete_many(doc! {})
        .await
        .unwrap();
    let sent: Sent = Default::default();
    let (a, b) = tokio::join!(
        try_sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
        try_sweep_capturing(&f, &sent, || Ok(automation::Outcome::Accepted)),
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(
        rows_of(&f, &f.owner).await[0].push.as_ref().unwrap().state,
        "sent"
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(PUSH_WORKER_STATE)
            .count_documents(doc! {"_id": "push"})
            .await
            .unwrap(),
        1
    );
}
