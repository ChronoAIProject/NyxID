//! Local replica-set diagnostics for #2277. No remote provider traffic.
use mongodb::{
    Client,
    bson::{Document, doc},
    options::ClientOptions,
};
use std::time::Duration;

async fn client(pool: u32) -> Client {
    let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("explicit local test URI");
    let mut options = ClientOptions::parse(uri).await.unwrap();
    options.max_pool_size = Some(pool);
    options.min_pool_size = Some(0);
    Client::with_options(options).unwrap()
}

#[tokio::test]
async fn replica_set_transaction_does_not_pin_a_connection() {
    let client = client(1).await;
    let db = client.database(&format!("pool_2277_{}", uuid::Uuid::new_v4().simple()));
    let rows = db.collection::<Document>("rows");
    rows.insert_one(doc! {"_id":"placeholder", "value":0})
        .await
        .unwrap();
    let work = async {
        let mut session = client.start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        rows.update_one(doc! {"_id":"placeholder"}, doc! {"$set":{"value":1}})
            .session(&mut session)
            .await
            .unwrap();
        // Outside the transaction this reads the old committed version, even
        // though the only pooled socket has already run the transaction write.
        let old = rows
            .find_one(doc! {"_id":"placeholder"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old.get_i32("value").unwrap(), 0);
        session.commit_transaction().await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .expect("pool-one watchdog");
    assert_eq!(
        rows.find_one(doc! {"_id":"placeholder"})
            .await
            .unwrap()
            .unwrap()
            .get_i32("value")
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
    eprintln!(
        "2277: pool=1 transaction write -> sessionless read -> commit: PASS (no connection pinning)"
    );
}

/// Opt-in diagnostic harness. The ordinary suite keeps its existing pool and
/// command handlers. Reports contain command names and source frames only.
/// Stack symbolication can perturb short deadline fixtures; full validation
/// enables pool one without NYXID_TEST_POOL_TRACE.
pub(crate) fn configure(options: &mut ClientOptions) {
    if std::env::var("NYXID_TEST_POOL_ONE").as_deref() != Ok("1") {
        return;
    }
    options.max_pool_size = Some(1);
    options.min_pool_size = options.min_pool_size.map(|minimum| minimum.min(1));
    if !matches!(
        std::env::var("NYXID_TEST_POOL_TRACE").as_deref(),
        Ok("1" | "transactions")
    ) {
        return;
    }
    let previous = options.command_event_handler.take();
    let active = std::sync::Mutex::new(std::collections::HashSet::<Vec<u8>>::new());
    options.command_event_handler = Some(mongodb::event::EventHandler::callback(move |event| {
        use mongodb::event::{EventHandler, command::CommandEvent};
        if let CommandEvent::Started(e) = &event
            && let Ok(lsid) = e.command.get_document("lsid")
        {
            let id = bson::to_vec(lsid).unwrap();
            let mut sessions = active.lock().unwrap();
            if e.command.get_bool("startTransaction") == Ok(true) {
                sessions.insert(id.clone());
                report("transaction", &e.command_name);
            } else if matches!(
                e.command_name.as_str(),
                "commitTransaction" | "abortTransaction"
            ) {
                sessions.remove(&id);
            } else if !sessions.is_empty() && e.command.get_bool("autocommit") != Ok(false) {
                report("sessionless_while_transaction_open", &e.command_name);
            }
        }
        if let Some(previous) = &previous {
            match previous {
                EventHandler::Callback(callback) => callback(event),
                EventHandler::AsyncCallback(callback) => {
                    tokio::spawn(callback(event));
                }
                EventHandler::TokioMpsc(sender) => {
                    let sender = sender.clone();
                    tokio::spawn(async move {
                        let _ = sender.send(event).await;
                    });
                }
                _ => {}
            }
        }
    }));
}

fn report(kind: &str, command: &str) {
    let mode = std::env::var("NYXID_TEST_POOL_TRACE").unwrap_or_default();
    if mode != "1" && !(mode == "transactions" && kind == "transaction") {
        return;
    }
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    let frames: Vec<_> = trace
        .lines()
        .filter(|line| {
            (line.contains("nyxid_server::") || line.contains("backend/src/"))
                && !line.contains("pool_starvation_tests")
        })
        .map(|line| {
            line.find("backend/src/")
                .map_or(line.trim(), |index| &line[index..])
        })
        .take(80)
        .collect();
    let line = format!("2277_TRACE {kind} {command} {}", frames.join(" | "));
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    if SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(line.clone())
    {
        eprintln!("{line}");
    }
}

fn audit_entry() -> crate::models::audit_log::AuditLog {
    crate::models::audit_log::AuditLog {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: None,
        event_type: "pool_probe".into(),
        event_data: None,
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
        seq: None,
        prev_hash: None,
        entry_hash: None,
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn group_turn_missing_nyxbot_does_not_start_a_nested_owner_transaction() {
    use crate::services::{
        assistant_authority_tests, assistant_group_service as groups, assistant_nyxagent as engine,
    };
    let f = Box::pin(assistant_authority_tests::direct_fixture("pool_2277_group")).await;
    let single = client(1).await;
    let db = single.database(f.state.db.name());
    let agent = f.row.agent_id.as_ref().unwrap();
    let group = groups::create(
        &db,
        &f.owner,
        "Placeholder group",
        std::slice::from_ref(agent),
        "user",
    )
    .await
    .unwrap();
    db.collection::<Document>("assistant_agents")
        .delete_one(doc! {"user_id":&f.owner,"kind":"nyxbot"})
        .await
        .unwrap();
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        agent_id: Some(agent.clone()),
        conversation_id: None,
        text: "Placeholder group request".into(),
        model: None,
        access_mode: None,
    });
    start.origin = crate::models::assistant_conversation::TurnOrigin::Group;
    start.group_id = Some(group.id);
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        Box::pin(engine::begin_turn(
            &db,
            &f.owner,
            &start,
            &f.state.encryption_keys,
        )),
    )
    .await;
    assert!(
        result.is_ok(),
        "group admission waited for its own owner-fence transaction"
    );
    result.unwrap().unwrap();
}

#[tokio::test]
async fn missing_nyxbot_creation_rolls_back_with_its_owner_transaction() {
    let client = client(1).await;
    let db = client.database(&format!("pool_2277_{}", uuid::Uuid::new_v4().simple()));
    let rows = db.collection::<Document>("assistant_agents");
    db.create_collection("assistant_agents").await.unwrap();
    db.collection::<Document>(crate::services::erasure_admission::COLLECTION)
        .insert_one(doc! {"_id":"placeholder-owner","erasure_accepted":false,"revision":0_i64})
        .await
        .unwrap();
    let work = async {
        let mut session = client.start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        crate::services::erasure_admission::fence_in_session(
            &db,
            "placeholder-owner",
            &mut session,
        )
        .await
        .unwrap();
        let agent = crate::services::assistant_team_service::ensure_nyxbot_in_session(
            &db,
            "placeholder-owner",
            &mut session,
        )
        .await
        .unwrap();
        assert!(
            rows.find_one(doc! {"_id":&agent.id})
                .session(&mut session)
                .await
                .unwrap()
                .is_some()
        );
        // No independently committed profile may escape an aborted admission.
        assert!(
            rows.find_one(doc! {"_id":&agent.id})
                .await
                .unwrap()
                .is_none()
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 0);
    };
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .expect("atomic creation watchdog");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn audit_retry_preserves_uuid_content_hash_and_sequence() {
    use crate::services::audit_chain_service as audit;
    let client = client(1).await;
    let db = client.database(&format!("pool_2277_{}", uuid::Uuid::new_v4().simple()));
    let rows = db.collection::<Document>("audit_log");
    rows.create_index(
        mongodb::IndexModel::builder()
            .keys(doc! {"seq":1})
            .options(
                mongodb::options::IndexOptions::builder()
                    .name("audit_log_seq_unique".to_owned())
                    .unique(true)
                    .build(),
            )
            .build(),
    )
    .await
    .unwrap();
    let entry = audit_entry();
    let work = async {
        let first = audit::append_chained_entry(&db, entry.clone(), &[7; 32])
            .await
            .unwrap();
        let replay = audit::append_chained_entry(&db, entry.clone(), &[7; 32])
            .await
            .unwrap();
        assert_eq!(
            bson::to_document(&first).unwrap(),
            bson::to_document(&replay).unwrap()
        );
        let mut different = entry.clone();
        different.event_type = "different-placeholder-event".into();
        assert!(
            audit::append_chained_entry(&db, different, &[7; 32])
                .await
                .unwrap_err()
                .get_custom::<&str>()
                .is_some_and(|message| message.contains("different immutable content"))
        );
        let report = audit::verify_chain(&db, &[7; 32], None, None, None)
            .await
            .unwrap();
        assert_eq!(report.status, audit::AuditChainStatus::Ok);
        assert_eq!(report.checked_count, 1);
    };
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .expect("audit replay watchdog");
    db.drop().await.unwrap();
}

/// A gate keeps the holder open after an ordinary append starts. Three seconds
/// is workload input, not a pass-time threshold; only persistence is asserted.
#[tokio::test]
#[ignore = "exclusive disposable MongoDB transaction lifetime experiment"]
async fn ordinary_audit_survives_a_holder_beyond_the_old_retry_budget() {
    use crate::services::audit_chain_service as audit;
    let (issued, mut started) = tokio::sync::watch::channel(false);
    let (rejected, mut attempts) = tokio::sync::watch::channel(0_usize);
    let mut options = ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    options.max_pool_size = Some(10);
    options.command_event_handler = Some(mongodb::event::EventHandler::callback(move |event| {
        if let mongodb::event::command::CommandEvent::Started(e) = &event
            && e.command_name == "insert"
            && let Ok(rows) = e.command.get_array("documents")
            && rows
                .first()
                .and_then(|row| row.as_document())
                .and_then(|row| row.get_str("event_type").ok())
                == Some("pool_probe_worker")
        {
            issued.send_replace(true);
        }
        let failed_insert = match &event {
            mongodb::event::command::CommandEvent::Failed(e) => e.command_name == "insert",
            mongodb::event::command::CommandEvent::Succeeded(e) => {
                e.command_name == "insert"
                    && e.reply
                        .get_array("writeErrors")
                        .is_ok_and(|errors| !errors.is_empty())
            }
            _ => false,
        };
        if failed_insert {
            rejected.send_modify(|n| *n += 1);
        }
    }));
    let client = Client::with_options(options).unwrap();
    let db = client.database(&format!("pool_2277_{}", uuid::Uuid::new_v4().simple()));
    db.collection::<Document>("audit_log")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"seq":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("audit_log_seq_unique".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let mut holder = client.start_session().await.unwrap();
    holder.start_transaction().await.unwrap();
    audit::append_chained_entry_in_session(&db, &mut holder, audit_entry(), &[7; 32])
        .await
        .unwrap();
    let worker_db = db.clone();
    let worker = tokio::spawn(async move {
        let mut entry = audit_entry();
        entry.event_type = "pool_probe_worker".into();
        audit::append_chained_entry(&worker_db, entry, &[7; 32]).await
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while !*started.borrow_and_update() {
            started.changed().await.unwrap();
        }
    })
    .await
    .expect("ordinary insert must reach the holder before opening its gate");
    let (release, gate) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        release.send(()).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while *attempts.borrow_and_update() < 8 {
            attempts.changed().await.unwrap();
        }
    })
    .await
    .expect("eight conflicts observed while the holder is still uncommitted");
    gate.await.unwrap();
    holder.commit_transaction().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), worker)
        .await
        .unwrap()
        .unwrap();
    let verification = audit::verify_chain(&db, &[7; 32], None, None, None)
        .await
        .unwrap();
    eprintln!(
        "2277_A1_HOLDER holder_gate_seconds=3 ordinary_ok={} chain_entries={}",
        result.is_ok(),
        verification.checked_count
    );
    db.drop().await.unwrap();
    assert!(
        result.is_ok(),
        "ordinary audit dropped while holder outlived eight-attempt budget: {result:?}"
    );
    assert_eq!(verification.checked_count, 2);
}
