use super::*;
use crate::models::channel_thread::{ChannelThreadFacts, ThreadAddress};
use crate::services::channel_thread_service::operation_tests;

async fn setup() -> (Database, ThreadReplyTarget, String) {
    let (db, _, source) = operation_tests::fixture("telegram").await;
    operation_tests::enable(&db).await;
    super::super::assistant_nyxagent::ensure_indexes(&db)
        .await
        .unwrap();
    let agent = super::super::assistant_team_service::ensure_nyxbot(&db, "owner")
        .await
        .unwrap();
    db.collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":"link"},
            doc! {"$set":{"owner_sender_ids":["human"],"agent_id":&agent.id}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(THREADS)
        .insert_one(doc! {"_id":"parent","user_id":"owner","channel_id":"link",
        "partition":"parent","kind":"group","platform_chat_id":"-100","owner_seen":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()})
        .await
        .unwrap();
    (
        db,
        ThreadReplyTarget::fixture("telegram", source.thread_context.unwrap()),
        agent.id,
    )
}
async fn choose(db: &Database, t: &ThreadReplyTarget, agent: &str, addressed: bool) -> Selection {
    select(
        db, "owner", "link", "parent", "parent", t, "source", "human", addressed, false, agent,
        true,
    )
    .await
    .unwrap()
}
fn child(selection: Selection) -> (NyxbotThread, ThreadTurnBinding) {
    match selection {
        Selection::Child(c, b) => (*c, b),
        _ => panic!("expected child"),
    }
}
fn origin(child: &NyxbotThread, b: ThreadTurnBinding) -> ChannelOrigin {
    ChannelOrigin {
        nyxbot_channel_id: "link".into(),
        partition: child.partition.clone(),
        platform: "telegram".into(),
        thread: Some(Box::new(b)),
    }
}
async fn activate_claim(db: &Database, c: &NyxbotThread, b: ThreadTurnBinding) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    admit_turn(
        db,
        "owner",
        &origin(c, b),
        c.conversation_id.as_deref().unwrap(),
        &mut session,
    )
    .await?;
    session.commit_transaction().await?;
    Ok(())
}

#[tokio::test]
async fn concurrent_activation_reuses_child_and_stop_fences_unclaimed_turn() {
    let (db, t, a) = setup().await;
    let (one, two) = tokio::join!(choose(&db, &t, &a, true), choose(&db, &t, &a, true));
    let (c, b) = child(one);
    let (other, _) = child(two);
    assert_eq!(c.id, other.id);
    assert_eq!(c.conversation_id, other.conversation_id);
    assert_eq!(c.follow.follow_state.as_deref(), Some("opening"));
    let stopped = stop(&db, "owner", "link", "parent", &c.id).await.unwrap();
    assert_eq!(stopped.conversation_id, c.conversation_id);
    assert!(activate_claim(&db, &c, b.clone()).await.is_err());
    assert!(matches!(choose(&db, &t, &a, false).await, Selection::Quiet));
    let (again, next) = child(choose(&db, &t, &a, true).await);
    assert_eq!(again.id, c.id);
    assert_eq!(again.conversation_id, c.conversation_id);
    assert!(next.revision > b.revision);
    activate_claim(&db, &again, next).await.unwrap();
    assert_eq!(
        counts(&db, "owner", &["parent".into()]).await.unwrap()["parent"],
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn unmentioned_follow_checks_sender_each_time_and_guests_cannot_stop() {
    let (db, t, a) = setup().await;
    assert!(matches!(choose(&db, &t, &a, false).await, Selection::Quiet));
    let (c, b) = child(choose(&db, &t, &a, true).await);
    activate_claim(&db, &c, b).await.unwrap();
    let (_, owner) = child(choose(&db, &t, &a, false).await);
    assert!(!owner.guest);
    let guest = select(
        &db, "owner", "link", "parent", "parent", &t, "source", "guest", false, false, &a, true,
    )
    .await
    .unwrap();
    let (_, guest) = child(guest);
    assert!(guest.guest);
    assert!(matches!(
        select(
            &db, "owner", "link", "parent", "parent", &t, "source", "guest", true, true, &a, true
        )
        .await
        .unwrap(),
        Selection::Quiet
    ));
    update_settings(
        &db,
        "owner",
        "link",
        "parent",
        doc! {"$set":{"members":"owner"}},
        false,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        select(
            &db, "owner", "link", "parent", "parent", &t, "source", "guest", false, false, &a, true
        )
        .await
        .unwrap(),
        Selection::Quiet
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn idle_expiry_reacquires_capacity_and_all_keeps_same_child_stopped() {
    let (db, t, a) = setup().await;
    let (c, b) = child(choose(&db, &t, &a, true).await);
    activate_claim(&db, &c, b).await.unwrap();
    db.collection::<NyxbotThread>(THREADS).update_one(doc! {"_id":&c.id},doc! {"$set":{"follow_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(matches!(choose(&db, &t, &a, false).await, Selection::Quiet));
    sweep(&db).await.unwrap();
    let (same, b) = child(choose(&db, &t, &a, true).await);
    assert_eq!(same.id, c.id);
    activate_claim(&db, &same, b).await.unwrap();
    stop(&db, "owner", "link", "parent", &c.id).await.unwrap();
    update_settings(
        &db,
        "owner",
        "link",
        "parent",
        doc! {"$set":{"reply_mode":"all"}},
        false,
        false,
    )
    .await
    .unwrap();
    let (same, b) = child(choose(&db, &t, &a, false).await);
    assert_eq!(same.id, c.id);
    activate_claim(&db, &same, b).await.unwrap();
    let same = db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&c.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(same.follow.follow_state.as_deref(), Some("stopped"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn admitted_binding_survives_stop_but_not_relink_or_sender_revocation() {
    let (db, t, a) = setup().await;
    let (c, b) = child(choose(&db, &t, &a, true).await);
    activate_claim(&db, &c, b.clone()).await.unwrap();
    stop(&db, "owner", "link", "parent", &c.id).await.unwrap();
    let o = origin(&c, b);
    let conversation = c.conversation_id.as_deref().unwrap();
    validate_delivery(&db, "owner", &o, conversation)
        .await
        .unwrap();
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(doc! {"_id":"link"}, doc! {"$set":{"owner_sender_ids":[]}})
        .await
        .unwrap();
    assert!(
        validate_delivery(&db, "owner", &o, conversation)
            .await
            .is_err()
    );
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":"link"},
            doc! {"$set":{"owner_sender_ids":["human"]}},
        )
        .await
        .unwrap();
    relink(&db, "owner", "link", "replacement").await.unwrap();
    assert!(
        validate_delivery(&db, "owner", &o, conversation)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn capacity_and_rate_limits_are_atomic_without_eviction() {
    let (db, t, a) = setup().await;
    let now = bson::DateTime::now();
    db.collection::<bson::Document>(THREADS).insert_many((0..CHAT_CAP-1).map(|i|doc! {"_id":format!("busy-{i}"),"channel_id":"link","partition":format!("busy-{i}"),
        "record_scope":SCOPE,"parent_chat_id":"parent","follow_state":"active","follow_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1)),"created_at":now})).await.unwrap();
    let t2 = ThreadReplyTarget::fixture(
        "telegram",
        ChannelThreadFacts {
            root_id: Some("43".into()),
            ..t.facts().clone()
        },
    );
    let (r1, r2) = tokio::join!(
        select(
            &db, "owner", "link", "parent", "parent", &t, "source", "human", true, false, &a, true
        ),
        select(
            &db, "owner", "link", "parent", "parent", &t2, "source", "human", true, false, &a, true
        )
    );
    assert_eq!(usize::from(r1.is_ok()) + usize::from(r2.is_ok()), 1);
    assert_eq!(
        db.collection::<bson::Document>(THREADS)
            .count_documents(doc! {"record_scope":SCOPE})
            .await
            .unwrap(),
        CHAT_CAP
    );
    db.collection::<bson::Document>(THREADS)
        .delete_many(doc! {"record_scope":SCOPE})
        .await
        .unwrap();
    // Nine more activations fit; the failed capacity transaction consumed none.
    for i in 0..9 {
        let next = ThreadReplyTarget::fixture(
            "telegram",
            ChannelThreadFacts {
                root_id: Some(format!("root-{i}")),
                ..t.facts().clone()
            },
        );
        choose(&db, &next, &a, true).await;
    }
    assert!(matches!(
        select(
            &db, "owner", "link", "parent", "parent", &t2, "source", "human", true, false, &a, true
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn rollback_rejects_new_follows_and_preserves_known_addressed_routing() {
    let (db, t, a) = setup().await;
    let (c, b) = child(choose(&db, &t, &a, true).await);
    activate_claim(&db, &c, b.clone()).await.unwrap();
    db.collection::<bson::Document>("feature_flag_overrides")
        .update_one(doc! {"_id":"flag"}, doc! {"$set":{"enabled":false}})
        .await
        .unwrap();
    assert!(matches!(choose(&db, &t, &a, false).await, Selection::Quiet));
    let (same, b) = child(choose(&db, &t, &a, true).await);
    assert_eq!(same.id, c.id);
    activate_claim(&db, &same, b).await.unwrap();
    let other = ThreadReplyTarget::fixture(
        "telegram",
        ChannelThreadFacts {
            root_id: Some("other".into()),
            address: ThreadAddress::Mention,
            ..t.facts().clone()
        },
    );
    assert!(matches!(
        choose(&db, &other, &a, true).await,
        Selection::Legacy
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn owner_queue_is_bounded_keeps_event_anchors_and_rechecks_authority_after_stop() {
    let (db, t, a) = setup().await;
    let (c, b) = child(choose(&db, &t, &a, true).await);
    let conversation = c.conversation_id.as_deref().unwrap();
    db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id":conversation,"user_id":"owner","title":"Thread","model":"test","credential_api_key_id":"credential",
        "message_count":0_i64,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    let source = db
        .collection::<crate::models::channel_message::ChannelMessage>("channel_messages")
        .find_one(doc! {"_id":"source"})
        .await
        .unwrap()
        .unwrap();
    for i in 0..21 {
        let mut message = source.clone();
        message.id = format!("source-{i}");
        message.platform_message_id = Some(format!("message-{i}"));
        message.thread_context.as_mut().unwrap().message_id = format!("message-{i}");
        db.collection::<crate::models::channel_message::ChannelMessage>("channel_messages")
            .insert_one(&message)
            .await
            .unwrap();
        let mut binding = b.clone();
        binding.source_message_id = message.id;
        let mut event = super::super::assistant_team_service::event(
            "message",
            format!("ordinary admitted request {i}"),
            None,
        );
        event.question_key = Some(format!("question-{i}"));
        event.reply_to = vec![origin(&c, binding)];
        assert_eq!(
            enqueue(&db, "owner", conversation, event).await.unwrap(),
            i < 20
        );
    }
    db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":conversation}, doc! {"$set":{"active_turn":{
            "turn_id":"abandoned","started_at":bson::DateTime::now(),
            "lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1)),
            "question_key":"abandoned","asked_from":bson::to_bson(&origin(&c,b.clone())).unwrap()
        }}}).await.unwrap();
    assert_eq!(
        coalesce(
            &db,
            "owner",
            conversation,
            "abandoned",
            &origin(&c, b.clone())
        )
        .await
        .unwrap(),
        None
    );
    for _ in 0..25 {
        assert_eq!(
            coalesce(
                &db,
                "owner",
                conversation,
                "question-0",
                &origin(&c, b.clone())
            )
            .await
            .unwrap(),
            Some(true)
        );
    }
    stop(&db, "owner", "link", "parent", &c.id).await.unwrap();
    assert!(
        coalesce(
            &db,
            "owner",
            conversation,
            "question-0",
            &origin(&c, b.clone())
        )
        .await
        .is_err()
    );
    assert_eq!(prune_queue(&db, "owner", conversation).await.unwrap(), 20);
    let rows = db.collection::<AssistantConversation>(
        crate::models::assistant_conversation::COLLECTION_NAME,
    );
    let queued = rows
        .find_one(doc! {"_id":conversation})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.pending_events.len(), 20);
    for (i, event) in queued.pending_events.iter().enumerate() {
        assert_eq!(
            event.reply_to.len(),
            1,
            "duplicates never grow reply destinations"
        );
        let binding = event.reply_to[0].thread.as_ref().unwrap();
        assert!(binding.queued);
        assert_eq!(binding.source_message_id, format!("source-{i}"));
    }
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(doc! {"_id":"link"}, doc! {"$set":{"owner_sender_ids":[]}})
        .await
        .unwrap();
    assert_eq!(prune_queue(&db, "owner", conversation).await.unwrap(), 0);
    assert!(
        rows.find_one(doc! {"_id":conversation})
            .await
            .unwrap()
            .unwrap()
            .pending_events
            .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn settings_off_and_foreign_management_fail_closed_without_deleting_history() {
    let (db, t, a) = setup().await;
    let (c, b) = child(choose(&db, &t, &a, true).await);
    assert!(stop(&db, "other", "link", "parent", &c.id).await.is_err());
    assert!(
        stop(&db, "owner", "link", "wrong-parent", &c.id)
            .await
            .is_err()
    );
    assert!(
        list_children(&db, "owner", "link", Some("other"), "all", None, 25)
            .await
            .is_err()
    );
    assert!(
        list_children(
            &db,
            "owner",
            "link",
            Some("parent"),
            "all",
            Some("malformed"),
            25
        )
        .await
        .is_err()
    );
    update_settings(
        &db,
        "owner",
        "link",
        "parent",
        doc! {"$set":{"threads":"off"}},
        true,
        false,
    )
    .await
    .unwrap();
    assert!(activate_claim(&db, &c, b).await.is_err());
    assert!(matches!(choose(&db, &t, &a, true).await, Selection::Legacy));
    let (history, _) = list_children(&db, "owner", "link", Some("parent"), "all", None, 25)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].conversation_id, c.conversation_id);
    db.drop().await.unwrap();
}

#[test]
fn settings_patch_distinguishes_omission_from_null() {
    type Settings = crate::handlers::nyxbot::chats::ChatSettings;
    assert!(
        serde_json::from_value::<Settings>(serde_json::json!({}))
            .unwrap()
            .threads
            .is_none()
    );
    assert_eq!(
        serde_json::from_value::<Settings>(serde_json::json!({"threads":"follow"}))
            .unwrap()
            .threads
            .as_deref(),
        Some("follow")
    );
    assert!(serde_json::from_value::<Settings>(serde_json::json!({"threads":null})).is_err());
}

#[tokio::test]
async fn unknown_child_contract_never_reactivates_or_delivers() {
    let (db, t, a) = setup().await;
    let (c, b) = child(choose(&db, &t, &a, true).await);
    activate_claim(&db, &c, b.clone()).await.unwrap();
    for fields in [
        doc! {"follow_state":"future_state"},
        doc! {"follow_state":"active","thread_identity_version":2},
        // Email is a supported child kind in PR E; use an actually unknown
        // forward value to exercise the fail-closed contract.
        doc! {"thread_identity_version":1,"thread_kind":"future_kind"},
    ] {
        db.collection::<NyxbotThread>(THREADS)
            .update_one(doc! {"_id":&c.id}, doc! {"$set":fields})
            .await
            .unwrap();
        assert!(matches!(choose(&db, &t, &a, true).await, Selection::Quiet));
        assert!(
            validate_delivery(
                &db,
                "owner",
                &origin(&c, b.clone()),
                c.conversation_id.as_deref().unwrap()
            )
            .await
            .is_err()
        );
    }
    db.drop().await.unwrap();
}
