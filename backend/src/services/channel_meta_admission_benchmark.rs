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
