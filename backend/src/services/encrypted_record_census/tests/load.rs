//! Run on the isolated performance replica set. Each round replays the same
//! offered schedule. Latency includes time queued behind an earlier request.
use super::*;
use crate::{
    handlers::saved_logins,
    models::{
        saved_login::{COLLECTION_NAME, SavedLogin},
        user::UserType,
    },
    services::saved_login_service::{self, Input},
    test_utils::{test_app_state, test_auth_user, test_user},
};
use axum::{
    Json,
    extract::{Path, State},
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

struct Verified<'a> {
    keys: &'a EncryptionKeys,
    ids: BTreeSet<String>,
}

#[async_trait::async_trait]
impl Observations for Verified<'_> {
    async fn previous_dependents(&self, _: &Cut, _: &Row) -> AppResult<Dependents> {
        Ok(Dependents::Rows(vec![]))
    }
    async fn current_dependents(&self, _: &Snapshot<'_>, _: &Row) -> AppResult<Dependents> {
        Ok(Dependents::Rows(vec![]))
    }
    async fn replace(
        &mut self,
        _: &Snapshot<'_>,
        row: &Row,
        source: Option<Document>,
    ) -> AppResult<()> {
        if let Some(source) = source {
            let record: SavedLogin = bson::from_document(source).map_err(|_| malformed())?;
            saved_login_service::verify_current_secrets(self.keys, &record).await?;
            self.ids.insert(record.id);
        } else {
            self.ids.remove(row.id.as_str().ok_or_else(malformed)?);
        }
        Ok(())
    }
    async fn rescan(&mut self, at: &Snapshot<'_>) -> AppResult<()> {
        self.ids.clear();
        let mut cursor = at.find(COLLECTION_NAME, doc! {}, doc! {}).await?;
        while let Some(d) = cursor.try_next().await? {
            let record: SavedLogin = bson::from_document(d).map_err(|_| malformed())?;
            saved_login_service::verify_current_secrets(self.keys, &record).await?;
            self.ids.insert(record.id);
        }
        Ok(())
    }
}

fn input() -> Input {
    Input {
        label: "census timing fixture".into(),
        allowed_origins: vec!["https://example.com".into()],
        username: Zeroizing::new("fixture".into()),
        password: Some(Zeroizing::new("p".repeat(512))),
        totp_secret: None,
        confirm_each_sign_in: true,
    }
}

#[ignore = "opt-in isolated database load rehearsal"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saved_login_writes_during_capture_and_authenticated_scan() {
    // An external runner can interleave this workload with the identical
    // foreground fixture compiled against the unpatched server.
    let selection = std::env::var("NYXID_CENSUS_LOAD_MODE")
        .ok()
        .map(|value| value.parse::<usize>().expect("mode 0..=3"));
    assert!(selection.is_none_or(|mode| mode < 4));
    let rounds = if selection.is_some() { 1 } else { 8 };
    let source_commands = Arc::new(AtomicUsize::new(0));
    let journal_commands = Arc::new(AtomicUsize::new(0));
    let source_counter = source_commands.clone();
    let journal_counter = journal_commands.clone();
    let monitor = mongodb::event::EventHandler::callback(move |event| {
        if let mongodb::event::command::CommandEvent::Started(command) = event {
            match command.command.get_str(&command.command_name) {
                Ok(COLLECTION_NAME) => {
                    source_counter.fetch_add(1, Ordering::Relaxed);
                }
                Ok(LOG) => {
                    journal_counter.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "census_load",
        monitor,
    )
    .await;
    let state = test_app_state(db.clone());
    let keys = state.encryption_keys.as_ref();
    let mut owners = vec![];
    for _ in 0..4 {
        let owner = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let record = saved_login_service::put(&db, keys, &owner, &owner, None, input())
            .await
            .unwrap();
        owners.push((owner, record.id));
    }
    // Separate connection pool for background inspection.
    let url = std::env::var("NYXID_TEST_DATABASE_URL").expect("isolated test DB");
    let reader = mongodb::Client::with_uri_str(url)
        .await
        .unwrap()
        .database(db.name());
    let scope = Scope {
        namespaces: [COLLECTION_NAME.into()].into(),
        ..scope()
    };
    for round in 0..rounds {
        let order = if round % 2 == 0 {
            [0, 1, 2, 3]
        } else {
            [3, 2, 1, 0]
        };
        for mode in order
            .into_iter()
            .filter(|mode| selection.is_none_or(|s| s == *mode))
        {
            let started = if mode >= 2 {
                Some(Census::start(&reader, keys, scope.clone()).await.unwrap())
            } else {
                None
            };
            source_commands.store(0, Ordering::Relaxed);
            journal_commands.store(0, Ordering::Relaxed);
            let stop = AtomicBool::new(false);
            let collector = async {
                let Some((census, mut last)) = started else {
                    return 0;
                };
                let mut observations = Verified {
                    keys,
                    ids: BTreeSet::new(),
                };
                observations
                    .rescan(&census.snapshot(&last).await.unwrap())
                    .await
                    .unwrap();
                let mut cuts = 0;
                while !stop.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let end = witness(&census, &census.request_cut().await.unwrap()).await;
                    census
                        .reconcile(&last, &end, &mut observations)
                        .await
                        .unwrap();
                    if mode == 3 {
                        observations
                            .rescan(&census.snapshot(&end).await.unwrap())
                            .await
                            .unwrap();
                    }
                    assert_eq!(observations.ids.len(), 4);
                    last = end;
                    cuts += 1;
                }
                cuts
            };
            let traffic = async {
                let start = Instant::now();
                let jobs = owners.iter().enumerate().map(|(lane, (owner, id))| {
                    let state = &state;
                    async move {
                        let mut observed = vec![];
                        for nth in 0..100 {
                            let due = start + Duration::from_millis((nth * 50 + lane * 12) as u64);
                            tokio::time::sleep_until(due.into()).await;
                            let reply = saved_logins::replace(
                                State(state.clone()),
                                test_auth_user(owner),
                                Path(id.clone()),
                                Json(input()),
                            )
                            .await
                            .unwrap();
                            assert_eq!(reply.0.id, *id);
                            observed.push(due.elapsed().as_micros());
                        }
                        observed
                    }
                });
                let mut times: Vec<_> = futures::future::join_all(jobs)
                    .await
                    .into_iter()
                    .flatten()
                    .collect();
                let micros = start.elapsed().as_micros();
                stop.store(true, Ordering::Release);
                times.sort_unstable();
                (micros, times)
            };
            let ((micros, times), cuts) = tokio::join!(traffic, collector);
            assert!(mode < 2 || cuts > 0);
            let source = source_commands.load(Ordering::Relaxed);
            let journal = journal_commands.load(Ordering::Relaxed);
            assert!(source >= times.len());
            assert_eq!(journal, 0);
            println!(
                "CENSUS_LOAD round={round} mode={mode} completed={} elapsed_us={micros} p50={} p95={} p99={} cuts={cuts} source_commands={source} foreground_journal={journal} errors=0",
                times.len(),
                times[199],
                times[379],
                times[395]
            );
        }
    }
    db.drop().await.unwrap();
}
