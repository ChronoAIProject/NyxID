//! Manual phase/command timing with production durability and boundary checks.
//! Captures command names, collection names and durations only, never bodies.
use super::*;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Commands {
    pending: HashMap<i32, (usize, String)>,
    totals: BTreeMap<(usize, String), (usize, Duration)>,
}

#[tokio::test]
#[ignore = "manual audit fixture/restart/append command-duration profile"]
async fn audit_lifecycle_command_profile() {
    let phase = Arc::new(AtomicUsize::new(0));
    let commands = Arc::new(Mutex::new(Commands::default()));
    let (observed_phase, observed) = (phase.clone(), commands.clone());
    let began = Instant::now();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "audit_lifecycle_profile",
        EventHandler::callback(move |event| {
            let mut commands = observed.lock().unwrap();
            match event {
                CommandEvent::Started(event) => {
                    let collection = event.command.get_str(&event.command_name).unwrap_or("");
                    let name = format!("{}:{}", event.command_name, collection);
                    commands.pending.insert(
                        event.request_id,
                        (observed_phase.load(Ordering::SeqCst), name),
                    );
                }
                CommandEvent::Succeeded(event) => {
                    if let Some(key) = commands.pending.remove(&event.request_id) {
                        let total = commands.totals.entry(key).or_default();
                        total.0 += 1;
                        total.1 += event.duration;
                    }
                }
                CommandEvent::Failed(event) => {
                    commands.pending.remove(&event.request_id);
                }
                _ => (),
            }
        }),
    )
    .await;
    eprintln!(
        "audit_profile phase=fixture elapsed_ms={:.3}",
        began.elapsed().as_secs_f64() * 1000.0
    );

    phase.store(1, Ordering::SeqCst);
    let began = Instant::now();
    initialize(&db, &KEY, &AuditConfig::default())
        .await
        .unwrap();
    eprintln!(
        "audit_profile phase=restart elapsed_ms={:.3}",
        began.elapsed().as_secs_f64() * 1000.0
    );

    phase.store(2, Ordering::SeqCst);
    let began = Instant::now();
    let (mut bodies, mut commits) = (0, 0);
    for _ in 0..256 {
        chain::append_chained_entry_observed(&db, entry(), &KEY, |b, c| {
            bodies += u32::from(c == 0);
            commits += u32::from(c > 0);
            assert!(b <= crate::services::audit_runtime::transaction::MAX_BODY_ATTEMPTS);
        })
        .await
        .unwrap();
    }
    eprintln!(
        "audit_profile phase=direct appends=256 elapsed_ms={:.3} appends_per_sec={:.2} bodies={bodies} commits={commits}",
        began.elapsed().as_secs_f64() * 1000.0,
        256.0 / began.elapsed().as_secs_f64()
    );

    phase.store(3, Ordering::SeqCst);
    let sample = entry();
    let mut prepared = sample.clone();
    chain::prepare_chained_entry(&mut prepared, 1, chain::GENESIS_PREV_HASH.to_owned(), &KEY)
        .unwrap();
    let began = Instant::now();
    for _ in 0..10_000 {
        std::hint::black_box(
            chain::compute_entry_hash(std::hint::black_box(&prepared), &KEY).unwrap(),
        );
    }
    eprintln!(
        "audit_profile phase=hmac iterations=10000 elapsed_ms={:.3}",
        began.elapsed().as_secs_f64() * 1000.0
    );

    let report = chain::verify_chain(&db, &KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.status, chain::AuditChainStatus::Ok);
    assert_eq!(report.checked_count, 256);
    for ((phase, name), (count, duration)) in &commands.lock().unwrap().totals {
        eprintln!(
            "audit_profile_command phase={phase} name={name} count={count} elapsed_ms={:.3}",
            duration.as_secs_f64() * 1000.0
        );
    }
}
