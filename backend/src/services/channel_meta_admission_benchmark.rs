//! Exercise the production deduplication, grant and control transactions on a local replica.
use super::*;
use crate::models::user::{User, UserType};
use crate::services::{channel_adapters::meta::tests as fixture, channel_admission_service};
use crate::test_utils::{
    connect_transaction_test_database_with_command_handler, test_app_state, test_user,
};
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "admission contention benchmark: run alone on a local MongoDB replica"]
async fn concurrent_page_admission_with_shared_and_independent_admins() {
    const PAGES: usize = 4;
    const WAVES: usize = 32;
    let mut total_failures = 0;
    for shared in [false, true] {
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = starts.clone();
        let db = connect_transaction_test_database_with_command_handler(
            "meta_admission_benchmark",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(command) = event
                    && command.command.get_bool("startTransaction") == Ok(true)
                {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            }),
        )
        .await;
        let state = test_app_state(db.clone());
        ensure_indexes(&db).await.unwrap();
        let mut templates = Vec::new();
        for page in 0..PAGES {
            // Distinct NyxID owners keep the existing per-owner admission write out of the comparison.
            let owner = uuid::Uuid::new_v4().to_string();
            db.collection::<User>(crate::models::user::COLLECTION_NAME)
                .insert_one(test_user(&owner, UserType::Person))
                .await
                .unwrap();
            let mut bot = fixture::registered_bot(&state, &owner, "messenger").await;
            bot.platform_bot_id = (1000 + page).to_string();
            if !shared {
                bot.meta_binding.as_mut().unwrap().grant_subject_id = (900 + page).to_string();
            }
            db.collection::<ChannelBot>(BOTS)
                .replace_one(doc! {"_id":&bot.id}, &bot)
                .await
                .unwrap();
            let message = fixture::original(&state, &bot).await;
            assert!(
                channel_admission_service::store(&db, message.clone())
                    .await
                    .unwrap()
                    .is_some()
            );
            templates.push(message);
        }
        let before = starts.load(Ordering::Relaxed);
        let mut latencies = Vec::new();
        let mut failures = 0;
        for wave in 0..WAVES {
            let admissions = templates.iter().enumerate().map(|(page, template)| {
                let mut message = template.clone();
                message.id = uuid::Uuid::new_v4().to_string();
                message.platform_message_id = Some(format!("mid.benchmark.{page}.{wave}"));
                let db = &db;
                async move {
                    let start = Instant::now();
                    let result = channel_admission_service::store(db, message).await;
                    (start.elapsed(), result)
                }
            });
            for (elapsed, result) in futures::future::join_all(admissions).await {
                latencies.push(elapsed);
                match result {
                    Ok(Some(_)) => {}
                    Ok(None) => failures += 1,
                    Err(error) => {
                        if failures == 0 {
                            eprintln!("first failed admission: {error}");
                        }
                        failures += 1;
                    }
                }
            }
        }
        latencies.sort_unstable();
        let p95 = latencies[(latencies.len() * 95).div_ceil(100) - 1];
        let transactions = starts.load(Ordering::Relaxed) - before;
        eprintln!(
            "shared_admin={shared} pages={PAGES} requests={} transactions_started={transactions} p95_ms={:.2} failed_admissions={failures}",
            latencies.len(),
            p95.as_secs_f64() * 1000.0
        );
        db.drop().await.unwrap();
        total_failures += failures;
    }
    assert_eq!(
        total_failures, 0,
        "unique, authorized admissions must all commit"
    );
}

/// Ordinary owner turns can contain more messages than the Meta proof budget.
/// Measure the extra projected read on the same transaction that publishes a card.
#[tokio::test]
async fn long_web_turn_requests_cards_with_one_projected_provenance_read_each() {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_authority_tests, assistant_nyxagent,
    };
    use std::sync::Mutex;
    let timings = Arc::new(Mutex::new((None, Vec::new())));
    let captured = timings.clone();
    let db = connect_transaction_test_database_with_command_handler(
        "long_web_turn_cards",
        EventHandler::callback(move |event| {
            let mut times = captured.lock().unwrap();
            match event {
                CommandEvent::Started(c)
                    if c.command.get_str("find") == Ok("assistant_messages")
                        && c.command
                            .get_document("projection")
                            .is_ok_and(|p| p.contains_key("meta_grants")) =>
                {
                    times.0 = Some(c.request_id);
                }
                CommandEvent::Succeeded(c) if times.0 == Some(c.request_id) => {
                    times.1.push(c.duration);
                    times.0 = None;
                }
                _ => {}
            }
        }),
    )
    .await;
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    assistant_nyxagent::ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let row =
        assistant_authority_tests::new_orchestrator(&state, &owner, "Review an operation").await;
    let chat = assistant_authority_tests::chat_for_row(&state, &owner, &row).await;
    let turn = &row.active_turn.as_ref().unwrap().turn_id;
    let messages = db.collection::<bson::Document>("assistant_messages");
    let sample = messages
        .find_one(doc! {"conversation_id": &row.id, "turn_id":turn})
        .await
        .unwrap()
        .unwrap();
    let history: Vec<_> = (1..=101_i64)
        .map(|n| {
            let mut message = sample.clone();
            message.insert("_id", uuid::Uuid::new_v4().to_string());
            message.insert("role", "assistant");
            message.insert("seq", n + 100);
            message
        })
        .collect();
    messages.insert_many(history).await.unwrap();
    let mut elapsed = Vec::new();
    let samples = 32;
    for n in 0..samples {
        let arguments = serde_json::json!({"item": n});
        let start = Instant::now();
        let card = acks::request(
            &db,
            &chat,
            acks::Request {
                kind: "action",
                service: None,
                tool: Some("review_item"),
                arguments: Some(&arguments),
                summary: "Review this item",
                platform: false,
            },
        )
        .await
        .unwrap();
        elapsed.push(start.elapsed());
        assert_eq!(card.requested_turn_id.as_ref(), Some(turn));
        assert_eq!(card.creation_turn_id.as_ref(), Some(turn));
    }
    let mut reads = timings.lock().unwrap().1.clone();
    assert_eq!(reads.len(), samples);
    reads.sort_unstable();
    elapsed.sort_unstable();
    let p95 = (samples * 95).div_ceil(100) - 1;
    eprintln!(
        "ordinary_cards={samples} projected_reads={} projected_read_p95_us={} card_p95_us={}",
        reads.len(),
        reads[p95].as_micros(),
        elapsed[p95].as_micros()
    );
    let auto = acks::AutoAllowed {
        class: acks::OwnerActionClass::Read,
        mode: crate::models::assistant_settings::AccessMode::Autopilot,
        reason: "approval",
    };
    let mut receipt_times = Vec::new();
    for n in 0..samples {
        let start = Instant::now();
        acks::record_auto_allowed(
            &db,
            &chat,
            "read_item",
            &serde_json::json!({"item":n}),
            &auto,
            "Read this item",
        )
        .await;
        receipt_times.push(start.elapsed());
    }
    let receipts = db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .count_documents(doc! {"conversation_id":&row.id,"decided_by":"access_mode"})
        .await
        .unwrap();
    assert_eq!(receipts, samples as u64);
    let receipt_reads = timings.lock().unwrap().1.len() - reads.len();
    assert_eq!(receipt_reads, samples);
    receipt_times.sort_unstable();
    eprintln!(
        "ordinary_auto_receipts={receipts} projected_reads={receipt_reads} receipt_p95_us={}",
        receipt_times[p95].as_micros()
    );
    db.drop().await.unwrap();
}
