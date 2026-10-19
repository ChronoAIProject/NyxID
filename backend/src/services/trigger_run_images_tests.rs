//! #1884 §3.4: an automation run accumulates the tool images of every one of
//! its turns (initial, confirmation-resumed and async-result continuations)
//! from the exact persisted settlement snapshot, bounded and evicted against
//! the run's configured chat audience.
use super::*;
use crate::models::trigger_run::TriggerRun;
use crate::{
    models::{
        assistant_conversation::{AssistantConversation, TurnAttachment},
        nyxbot_image_delivery::ImageDeliveryState,
    },
    services::{channel_image_delivery as claims, channel_platform::OutboundReply},
};

const OWNER: &str = "owner";

fn image(id: &str) -> TurnAttachment {
    TurnAttachment {
        chat_delivery: Some(true),
        image_input: None,
        image_inspection: None,
        origin: "tool".into(),
        pages: None,
        id: id.into(),
        content_type: "image/png".into(),
        size: 12,
        label: "private_tool_label".into(),
    }
}

/// A conversation as `finish_turn_inner` holds it: the fenced persisted active
/// turn with its trusted attachment metadata, not a pre-turn snapshot.
fn conversation(
    id: &str,
    turn_id: &str,
    origin: &str,
    run_id: Option<&str>,
    attachments: Vec<TurnAttachment>,
) -> AssistantConversation {
    let now = bson::DateTime::now();
    let mut turn = doc! {
        "turn_id": turn_id, "origin": origin, "started_at": now,
        "attachments": bson::to_bson(&attachments).unwrap(),
    };
    if let Some(run_id) = run_id {
        turn.insert("trigger_run_id", run_id);
    }
    bson::from_document(doc! {
        "_id": id, "user_id": OWNER, "title": "Automation", "model": "nyxagent/chat",
        "credential_api_key_id": "key", "message_count": 1, "active_turn": turn,
        "created_at": now, "updated_at": now,
    })
    .unwrap()
}

async fn insert_run(db: &Database, id: &str, thread: &str, turn: &str, deliver_to: &DeliverTo) {
    let now = chrono::Utc::now();
    db.collection::<Document>(RUNS)
        .insert_one(doc! {
            "_id": id, "trigger_id": "trigger", "user_id": OWNER,
            "scheduled_at": bson::DateTime::from_chrono(now),
            "deadline": bson::DateTime::from_chrono(now + chrono::Duration::hours(1)),
            "outcome": "started", "reason": null, "thread_id": thread, "turn_id": turn,
            "duration_ms": null, "fence": "fence",
            "lease_until": bson::DateTime::from_chrono(now),
            "expires_at": bson::DateTime::from_chrono(now + chrono::Duration::days(7)),
            "deliver_to": bson::to_bson(deliver_to).unwrap(),
        })
        .await
        .unwrap();
}

/// What `trigger_schedule::admit_turn` does for every (re)admitted run turn,
/// continuation included: the run follows its current turn.
async fn readmit(db: &Database, run_id: &str, turn: &str) {
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id": run_id}, doc! {"$set": {"turn_id": turn}})
        .await
        .unwrap();
}

async fn accumulate(db: &Database, conversation: &AssistantConversation) {
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    accumulate_in_session(db, &mut session, conversation)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
}

async fn load(db: &Database, run_id: &str) -> TriggerRun {
    db.collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": run_id})
        .await
        .unwrap()
        .unwrap()
}

fn ids(run: &TriggerRun) -> Vec<&str> {
    run.image_refs
        .iter()
        .map(|item| item.attachment.id.as_str())
        .collect()
}

#[tokio::test]
async fn run_turns_accumulate_only_chat_tool_images_once_each() {
    let db = crate::test_utils::connect_transaction_test_database("run_images_filter").await;
    insert_run(&db, "run", "thread", "t1", &DeliverTo::Thread).await;
    let mut screenshot = image("screenshot");
    screenshot.chat_delivery = Some(false);
    // Attached by an old replica: no chat_delivery, identified by its label only.
    let mut legacy_screenshot = image("legacy-screenshot");
    legacy_screenshot.chat_delivery = None;
    legacy_screenshot.label = "Machine screenshot".into();
    let mut document = image("document");
    document.content_type = "application/pdf".into();
    let mut upload = image("upload");
    upload.origin = "upload".into();
    let row = conversation(
        "thread",
        "t1",
        "trigger",
        Some("run"),
        vec![
            image("a"),
            screenshot,
            image("b"),
            legacy_screenshot,
            document,
            upload,
            image("a"),
        ],
    );
    accumulate(&db, &row).await;
    let run = load(&db, "run").await;
    assert_eq!(ids(&run), ["a", "b"]);
    assert_eq!(run.image_refs_truncated, 0);
    for item in &run.image_refs {
        assert_eq!(item.conversation_id, "thread");
        assert_eq!(item.turn_id, "t1");
        assert_eq!(item.attachment.content_type, "image/png");
        assert_eq!(item.attachment.size, 12);
        assert_eq!(item.attachment.label, "private_tool_label");
        assert!(engine::chat_tool_image(&item.attachment));
    }
    // Settlement retries of the same snapshot never duplicate a ref.
    accumulate(&db, &row).await;
    assert_eq!(ids(&load(&db, "run").await), ["a", "b"]);
    // A turn without tool images leaves the run untouched.
    accumulate(
        &db,
        &conversation("thread", "t1", "trigger", Some("run"), Vec::new()),
    )
    .await;
    assert_eq!(ids(&load(&db, "run").await), ["a", "b"]);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn confirmation_and_async_continuations_append_their_own_turn_snapshots() {
    let db = crate::test_utils::connect_transaction_test_database("run_images_turns").await;
    insert_run(&db, "run", "thread", "t1", &DeliverTo::Thread).await;
    accumulate(
        &db,
        &conversation("thread", "t1", "trigger", Some("run"), vec![image("a")]),
    )
    .await;
    // The owner allowed a card: the scheduler re-admits the run on a new turn.
    readmit(&db, "run", "t2").await;
    accumulate(
        &db,
        &conversation(
            "thread",
            "t2",
            "trigger",
            Some("run"),
            vec![image("b"), image("a")],
        ),
    )
    .await;
    // An async operation result resumes the run as an event turn.
    readmit(&db, "run", "t3").await;
    accumulate(
        &db,
        &conversation("thread", "t3", "event", Some("run"), vec![image("c")]),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(ids(&run), ["a", "b", "c"]);
    let turns: Vec<&str> = run
        .image_refs
        .iter()
        .map(|item| item.turn_id.as_str())
        .collect();
    assert_eq!(turns, ["t1", "t2", "t3"]);
    assert!(
        run.image_refs
            .iter()
            .all(|item| item.conversation_id == "thread")
    );
    // Only the run's exact current turn is a run turn: a settlement that is
    // not the admitted turn (a stale or replaced one) adds nothing.
    accumulate(
        &db,
        &conversation("thread", "t2", "trigger", Some("run"), vec![image("late")]),
    )
    .await;
    accumulate(
        &db,
        &conversation("thread", "t9", "trigger", Some("run"), vec![image("stray")]),
    )
    .await;
    assert_eq!(ids(&load(&db, "run").await), ["a", "b", "c"]);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn unrelated_owner_work_other_runs_threads_and_owners_never_enter_a_run() {
    let db = crate::test_utils::connect_transaction_test_database("run_images_unrelated").await;
    insert_run(&db, "run-a", "thread", "t1", &DeliverTo::Thread).await;
    insert_run(&db, "run-b", "thread", "t5", &DeliverTo::Thread).await;
    accumulate(
        &db,
        &conversation("thread", "t1", "trigger", Some("run-a"), vec![image("a")]),
    )
    .await;
    // The owner's own turn in the same home thread, with its own images.
    accumulate(
        &db,
        &conversation("thread", "t2", "user", None, vec![image("owner")]),
    )
    .await;
    // Another run sharing the home thread accumulates only its own turn.
    accumulate(
        &db,
        &conversation("thread", "t5", "trigger", Some("run-b"), vec![image("b")]),
    )
    .await;
    // A turn naming run-a from a different thread is not run-a's work.
    accumulate(
        &db,
        &conversation(
            "other-thread",
            "t1",
            "trigger",
            Some("run-a"),
            vec![image("elsewhere")],
        ),
    )
    .await;
    // Another owner's conversation cannot reach this owner's run.
    let mut foreign = conversation(
        "thread",
        "t1",
        "trigger",
        Some("run-a"),
        vec![image("foreign")],
    );
    foreign.user_id = "someone-else".into();
    accumulate(&db, &foreign).await;
    assert_eq!(ids(&load(&db, "run-a").await), ["a"]);
    assert_eq!(ids(&load(&db, "run-b").await), ["b"]);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn accumulated_refs_survive_a_restart_between_turns_and_delivery() {
    let db = crate::test_utils::connect_transaction_test_database("run_images_restart").await;
    insert_run(&db, "run", "thread", "t1", &DeliverTo::Thread).await;
    accumulate(
        &db,
        &conversation(
            "thread",
            "t1",
            "trigger",
            Some("run"),
            vec![image("a"), image("b")],
        ),
    )
    .await;
    // Nothing is held in memory: a fresh process reads the refs back from the
    // run and keeps appending to them.
    let persisted = load(&db, "run").await;
    assert_eq!(ids(&persisted), ["a", "b"]);
    readmit(&db, "run", "t2").await;
    accumulate(
        &db,
        &conversation("thread", "t2", "trigger", Some("run"), vec![image("c")]),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(ids(&run), ["a", "b", "c"]);
    let snapshot: Vec<(String, String)> = run
        .image_refs
        .iter()
        .map(|item| (item.turn_id.clone(), item.attachment.id.clone()))
        .collect();
    assert_eq!(
        snapshot,
        [("t1", "a"), ("t1", "b"), ("t2", "c")].map(|(turn, id)| (turn.to_owned(), id.to_owned()))
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn refs_are_bounded_to_sixteen_and_count_truncation_without_a_chat_audience() {
    let db = crate::test_utils::connect_transaction_test_database("run_images_bound").await;
    insert_run(&db, "run", "thread", "t1", &DeliverTo::Thread).await;
    let first: Vec<_> = (0..10).map(|index| image(&format!("i{index}"))).collect();
    accumulate(
        &db,
        &conversation("thread", "t1", "trigger", Some("run"), first),
    )
    .await;
    readmit(&db, "run", "t2").await;
    let second: Vec<_> = (10..17).map(|index| image(&format!("i{index}"))).collect();
    accumulate(
        &db,
        &conversation("thread", "t2", "trigger", Some("run"), second),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(run.image_refs.len(), MAX_RUN_IMAGES);
    assert_eq!(MAX_RUN_IMAGES, 16);
    // Oldest first; the newest image is the one that did not fit.
    let expected: Vec<String> = (0..16).map(|index| format!("i{index}")).collect();
    assert_eq!(ids(&run), expected);
    assert_eq!(run.image_refs_truncated, 1);
    // Without a chat audience nothing is ever terminal, so nothing is evicted.
    readmit(&db, "run", "t3").await;
    accumulate(
        &db,
        &conversation(
            "thread",
            "t3",
            "trigger",
            Some("run"),
            vec![image("x"), image("y")],
        ),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(ids(&run), expected);
    assert_eq!(run.image_refs_truncated, 3);
    // A settlement retry of the same turn counts nothing twice.
    accumulate(
        &db,
        &conversation(
            "thread",
            "t3",
            "trigger",
            Some("run"),
            vec![image("x"), image("y")],
        ),
    )
    .await;
    assert_eq!(load(&db, "run").await.image_refs_truncated, 5);
    db.drop().await.unwrap();
}

/// The run delivers to a Telegram forum topic; its audience key is the one
/// `post_automation_images` resolves for that thread.
async fn chat_destination(db: &Database) -> (DeliverTo, String) {
    let now = bson::DateTime::now();
    db.collection::<Document>(BOTS)
        .insert_one(doc! {
            "_id": "bot", "user_id": OWNER, "platform": "telegram", "label": "Home bot",
            "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1, 2, 3] },
            "platform_bot_id": "100", "platform_bot_username": "home_bot",
            "webhook_registered": true, "webhook_secret_hash": "hash", "status": "active",
            "is_active": true, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    db.collection::<Document>(CHANNELS)
        .insert_one(doc! {
            "_id": "channel", "user_id": OWNER, "channel_bot_id": "bot", "platform": "telegram",
            "bot_label": "Home bot", "transport": "direct", "status": "active",
            "route_api_key_id": "route-key", "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    db.collection::<Document>(THREADS)
        .insert_one(doc! {
            "_id": "chat", "channel_id": "channel", "user_id": OWNER, "partition": "chat",
            "kind": "group", "platform_chat_id": "-100", "platform_thread_id": "7",
            "allow_posts": true, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let reply = OutboundReply {
        text: None,
        metadata: Some(serde_json::json!({"message_thread_id": "7"})),
        attachments: Vec::new(),
        reply_to_platform_message_id: None,
    };
    let audience = channel_image_audience::resolve("bot", "telegram", "-100", &reply);
    assert_eq!(audience, "bot\u{1f}-100\u{1f}7");
    (
        DeliverTo::Chat {
            chat_id: "chat".into(),
        },
        audience,
    )
}

async fn settle(db: &Database, attachment: &str, audience: &str, state: ImageDeliveryState) {
    let claims::ImageClaim::Dispatch { id, attempt } =
        claims::claim(db, OWNER, "thread", attachment, audience)
            .await
            .unwrap()
    else {
        panic!("fresh claim")
    };
    match state {
        ImageDeliveryState::Pending => {
            db.collection::<Document>(CLAIMS)
                .update_one(
                    doc! {"_id": id},
                    doc! {"$set": {"state": "pending", "attempt": null}},
                )
                .await
                .unwrap();
        }
        ImageDeliveryState::Sending => {}
        state => {
            assert!(matches!(
                claims::finish(db, &id, &attempt, state, None)
                    .await
                    .unwrap(),
                claims::ImageCompletion::Recorded
            ));
        }
    }
}

#[tokio::test]
async fn full_runs_evict_the_oldest_ref_terminal_for_the_configured_audience_only() {
    let db = crate::test_utils::connect_transaction_test_database("run_images_evict").await;
    let (deliver_to, audience) = chat_destination(&db).await;
    insert_run(&db, "run", "thread", "t1", &deliver_to).await;
    let first: Vec<_> = (0..16).map(|index| image(&format!("i{index}"))).collect();
    accumulate(
        &db,
        &conversation("thread", "t1", "trigger", Some("run"), first),
    )
    .await;
    assert_eq!(load(&db, "run").await.image_refs.len(), 16);
    // i0 is waiting to be sent, i1 reached a *different* topic, i2 and i5 are
    // terminal for the run's own topic, i7 is mid-send.
    settle(&db, "i0", &audience, ImageDeliveryState::Pending).await;
    settle(&db, "i1", "bot\u{1f}-100\u{1f}8", ImageDeliveryState::Sent).await;
    settle(&db, "i2", &audience, ImageDeliveryState::Sent).await;
    settle(&db, "i5", &audience, ImageDeliveryState::Failed).await;
    settle(&db, "i7", &audience, ImageDeliveryState::Sending).await;
    readmit(&db, "run", "t2").await;
    accumulate(
        &db,
        &conversation("thread", "t2", "trigger", Some("run"), vec![image("j")]),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(run.image_refs.len(), 16);
    assert!(
        !ids(&run).contains(&"i2"),
        "the oldest terminal ref goes first"
    );
    assert!(ids(&run).contains(&"i0"), "a pending ref is never evicted");
    assert!(
        ids(&run).contains(&"i1"),
        "a send to another topic is not terminal here"
    );
    assert!(
        ids(&run).contains(&"i7"),
        "an in-flight send is not terminal"
    );
    assert_eq!(ids(&run).last(), Some(&"j"));
    assert_eq!(run.image_refs_truncated, 0);
    readmit(&db, "run", "t3").await;
    accumulate(
        &db,
        &conversation(
            "thread",
            "t3",
            "trigger",
            Some("run"),
            vec![image("k"), image("m")],
        ),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(run.image_refs.len(), 16);
    assert!(
        !ids(&run).contains(&"i5"),
        "the next terminal ref makes room for k"
    );
    assert!(ids(&run).contains(&"k"));
    assert!(
        !ids(&run).contains(&"m"),
        "with no terminal ref left the newest image is dropped"
    );
    assert_eq!(run.image_refs_truncated, 1);
    // Delivery of j and k settles them; the next turn evicts them in order.
    settle(&db, "j", &audience, ImageDeliveryState::Skipped).await;
    settle(&db, "k", &audience, ImageDeliveryState::Unknown).await;
    readmit(&db, "run", "t4").await;
    accumulate(
        &db,
        &conversation(
            "thread",
            "t4",
            "trigger",
            Some("run"),
            vec![image("n"), image("o")],
        ),
    )
    .await;
    let run = load(&db, "run").await;
    assert_eq!(run.image_refs.len(), 16);
    assert!(!ids(&run).contains(&"j") && !ids(&run).contains(&"k"));
    assert!(ids(&run).contains(&"n") && ids(&run).contains(&"o"));
    assert_eq!(run.image_refs_truncated, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn unrelated_invalid_run_fields_do_not_block_image_accumulation() {
    let db = crate::test_utils::connect_transaction_test_database("run_image_projection").await;
    insert_run(&db, "run", "conversation", "turn", &DeliverTo::default()).await;
    db.collection::<Document>(RUNS).update_one(doc! {"_id":"run"},doc! {"$set":{"confirmation_policy":"invalid unrelated value","missed":"invalid unrelated value"}}).await.unwrap();
    let row = conversation(
        "conversation",
        "turn",
        "trigger",
        Some("run"),
        vec![image("image")],
    );
    accumulate(&db, &row).await;
    let saved = db
        .collection::<Document>(RUNS)
        .find_one(doc! {"_id":"run"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.get_array("image_refs").unwrap().len(), 1);
    db.drop().await.unwrap();
}
