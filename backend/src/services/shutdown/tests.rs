use super::*;
use crate::{
    services::{
        coordination_service::{ClusterLeaseRuntime, LeaseStore},
        node_owner_service::ReplicaIdentity,
    },
    test_utils::{connect_transaction_test_database, test_app_state},
};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use bson::{Document, doc};
use std::time::Duration;
use tower::ServiceExt;

#[test]
fn shutdown_interruption_accounting_excludes_ordinary_settlements_on_either_side_of_cutoff() {
    let runtime = Runtime::default();
    let before = runtime.register_work(Kind::Turn {
        conversation_id: "conversation".into(),
        id: "before".into(),
        origin: "user",
    });
    let after = runtime.register_work(Kind::Turn {
        conversation_id: "conversation".into(),
        id: "after".into(),
        origin: "channel",
    });
    let unfinished = runtime.register_work(Kind::Turn {
        conversation_id: "conversation".into(),
        id: "unfinished".into(),
        origin: "trigger",
    });
    runtime.begin_drain();
    before.settled(false);
    runtime.interrupt();
    after.settled(false);
    runtime.interrupt();
    let late = runtime.register_work(Kind::Turn {
        conversation_id: "conversation".into(),
        id: "late".into(),
        origin: "user",
    });
    drop((before, after, unfinished, late));
    assert_eq!(
        runtime.outcomes(),
        (2, vec!["unfinished".into(), "late".into()])
    );
    assert_eq!(runtime.tracked_units(), 0);
}

#[tokio::test]
async fn shutdown_tracker_waits_for_detached_child_and_its_cleanup_then_is_empty() {
    let db = connect_transaction_test_database("shutdown_detached_cleanup").await;
    let runtime = Runtime::default();
    let (finish, pending) = tokio::sync::oneshot::channel();
    let child_runtime = runtime.clone();
    let child_db = db.clone();
    runtime.spawn(async move {
        pending.await.unwrap();
        child_runtime.spawn_cleanup(async move {
            crate::services::audit_service::log_system_event(
                child_db,
                "shutdown_child_finished",
                None,
            )
            .await
            .unwrap();
        });
    });
    runtime.begin_drain();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), runtime.idle())
            .await
            .is_err()
    );
    finish.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), runtime.idle())
        .await
        .unwrap();
    runtime.stop();
    runtime.tasks_stopped().await;
    assert_eq!(runtime.tracked_units(), 0);
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"shutdown_child_finished"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn shutdown_stop_joins_cancelled_child_cleanup_and_upgrade_accounting() {
    struct CompletionOnDrop(Runtime, mongodb::Database);
    impl Drop for CompletionOnDrop {
        fn drop(&mut self) {
            let db = self.1.clone();
            self.0.spawn_cleanup(async move {
                crate::services::audit_service::log_system_event(db, "child_cancelled", None)
                    .await
                    .unwrap();
            });
        }
    }

    let db = connect_transaction_test_database("shutdown_completion_tails").await;
    let runtime = Runtime::default();
    let mut children = tokio::task::JoinSet::new();
    let completion = CompletionOnDrop(runtime.clone(), db.clone());
    let (started, entered) = tokio::sync::oneshot::channel();
    runtime.spawn_joined(&mut children, async move {
        let _completion = completion;
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    entered.await.unwrap();

    let (finish, accounting) = tokio::sync::oneshot::channel();
    let callback_runtime = runtime.clone();
    let callback_db = db.clone();
    let callback = runtime
        .clone()
        .upgrade_completion_callback(move |()| async move {
            callback_runtime.stopped().await;
            accounting.await.unwrap();
            crate::services::audit_service::log_system_event(
                callback_db,
                "upgrade_accounted",
                None,
            )
            .await
            .unwrap();
        });
    let upgrade = tokio::spawn(callback(()));
    runtime.begin_drain();
    runtime.stop();
    tokio::time::timeout(Duration::from_secs(2), runtime.execution_stopped())
        .await
        .unwrap();
    assert!(
        children
            .join_next()
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), runtime.tasks_stopped())
            .await
            .is_err()
    );
    finish.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), runtime.tasks_stopped())
        .await
        .unwrap();
    upgrade.await.unwrap();
    assert_eq!(runtime.tracked_units(), 0);
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type": {"$in": ["child_cancelled", "upgrade_accounted"]}})
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn shutdown_interruption_retains_uncertain_provider_claim_and_releases_safe_fence() {
    let db = connect_transaction_test_database("shutdown_uncertain_claim").await;
    let runtime = Runtime::default();
    for (id, safe) in [("uncertain", false), ("safe", true)] {
        db.collection::<Document>("shutdown_claims")
            .insert_one(doc! {"_id":id,"claim":"original","held":true})
            .await
            .unwrap();
        let _ticket = runtime.register_release(Release {
            safe_after_stop: safe,
            db: db.clone(),
            collection: "shutdown_claims",
            filter: doc! {"_id":id,"claim":"original"},
            update: doc! {"$set":{"held":false}},
        });
    }
    runtime.begin_drain();
    runtime.stop();
    runtime.tasks_stopped().await;
    assert_eq!(release::registered(&runtime).await.unwrap(), 1);
    assert!(
        db.collection::<Document>("shutdown_claims")
            .find_one(doc! {"_id":"uncertain"})
            .await
            .unwrap()
            .unwrap()
            .get_bool("held")
            .unwrap()
    );
}

fn identity(instance: &str, generation: &str, url: &str) -> ReplicaIdentity {
    ReplicaIdentity {
        instance_name: instance.into(),
        generation_id: generation.into(),
        internal_base_url: url.into(),
    }
}

#[tokio::test]
async fn shutdown_second_signal_skips_wait_for_registered_work() {
    let db = connect_transaction_test_database("shutdown_second_signal").await;
    let state = test_app_state(db);
    let _work = state.shutdown.work();
    let drain = drain::Drain::begin(&state, Duration::from_secs(600), "SIGTERM").unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_millis(200),
        drain.wait(&state, drain.deadline, async { "SIGINT" }),
    )
    .await
    .unwrap();
    assert_eq!(outcome, drain::End::SecondSignal);
    assert!(!state.shutdown.counts().is_empty());
}

#[tokio::test]
async fn shutdown_zero_keeps_readiness_and_background_claims_unchanged() {
    let db = connect_transaction_test_database("shutdown_zero").await;
    let state = test_app_state(db);
    let _work = state.shutdown.work();
    assert!(drain::Drain::begin(&state, Duration::ZERO, "SIGTERM").is_none());
    assert!(!state.shutdown.is_draining());
    assert!(state.shutdown.background().is_some());
    assert_eq!(
        crate::handlers::health::ready(axum::extract::State(state.clone())).await,
        StatusCode::OK
    );
    assert_eq!(state.shutdown.tracked_units(), 1);
    assert_eq!(
        state.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type": {"$in": ["backend_shutdown_drain_started", "backend_shutdown_drain"]}})
            .await.unwrap(),
        0
    );
}

#[tokio::test]
async fn shutdown_serves_new_requests_health_200_ready_503_and_counts_response_streams() {
    let db = connect_transaction_test_database("shutdown_http").await;
    let state = test_app_state(db);
    let app = Router::new()
        .route("/health", get(crate::handlers::health::health_check))
        .route("/ready", get(crate::handlers::health::ready))
        .route("/work", get(|| async { "still serving" }))
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ));
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    state.shutdown.begin_drain().unwrap();
    for (path, expected) in [
        ("/health", StatusCode::OK),
        ("/ready", StatusCode::SERVICE_UNAVAILABLE),
        ("/work", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if path == "/work" {
            assert_eq!(state.shutdown.counts().streams, 1);
        }
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        if path == "/work" {
            assert_eq!(&bytes[..], b"still serving");
        }
    }
    assert!(state.shutdown.counts().is_empty());
}

#[tokio::test]
async fn shutdown_background_cluster_lease_hands_over_immediately_and_new_generation_is_fenced() {
    let db = connect_transaction_test_database("shutdown_cluster").await;
    let shutdown = Runtime::default();
    let first = identity("same-pod", "first", "http://127.0.0.1:1");
    let next = identity("same-pod", "next", "http://127.0.0.1:2");
    let runtime = ClusterLeaseRuntime::new(
        first.coordination_holder(),
        Duration::from_secs(600),
        Duration::from_secs(10),
    );
    runtime
        .acquire_background(&db, "telegram-poller", &shutdown)
        .await
        .unwrap()
        .unwrap();
    shutdown.begin_drain().unwrap();
    assert!(
        runtime
            .acquire_background(&db, "other", &shutdown)
            .await
            .unwrap()
            .is_none()
    );
    let mut uncertain = Vec::new();
    for name in [
        "oauth-refresh:request",
        "channel-webhook:platform",
        "telegram-new-manager-configuration",
        "telegram-manager-identity:manager",
        "telegram-child:bot",
        "discord-identify:bot",
    ] {
        uncertain.push(runtime.acquire(&db, name).await.unwrap().unwrap());
    }
    assert_eq!(release::cluster(&db, &first).await.unwrap(), 1);
    for lease in uncertain {
        assert!(
            LeaseStore::renew(&db, &lease, Duration::from_secs(600))
                .await
                .unwrap()
        );
    }
    let successor = LeaseStore::acquire(
        &db,
        "telegram-poller",
        &next.coordination_holder(),
        Duration::from_secs(600),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(release::cluster(&db, &first).await.unwrap(), 0);
    assert!(
        LeaseStore::renew(&db, &successor, Duration::from_secs(600))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn shutdown_trigger_claim_hands_over_and_old_writer_preserved_generation_cannot_be_released()
{
    use crate::services::trigger_schedule as triggers;
    let db = connect_transaction_test_database("shutdown_trigger").await;
    let state = test_app_state(db.clone());
    let runtime = state.shutdown.clone();
    let work = db.collection::<Document>(triggers::WORK);
    for id in ["legacy", "automation"] {
        work.insert_one(doc! {"_id": id, "kind":"run", "at": bson::DateTime::from_millis(0), "fence":"", "automation_v2": id == "automation"}).await.unwrap();
        let job = triggers::claim_for_replica(&db, id, chrono::Utc::now(), "first")
            .await
            .unwrap()
            .unwrap();
        let _release = triggers::retain_shutdown_release(&db, &job, "first", &runtime);
    }
    runtime.begin_drain();
    assert_eq!(release::registered(&runtime).await.unwrap(), 2);
    // Both generations of due work remain untouched by the draining scheduler.
    assert_eq!(
        crate::handlers::trigger_scheduler::tick(&state)
            .await
            .unwrap(),
        0
    );
    assert_eq!(work.count_documents(doc! {"fence": ""}).await.unwrap(), 2);
    for id in ["legacy", "automation"] {
        assert!(
            triggers::claim_for_replica(&db, id, chrono::Utc::now(), "next")
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(release::registered(&runtime).await.unwrap(), 0);
    let runtime = Runtime::default();
    work.update_one(
        doc! {"_id":"legacy"},
        doc! {"$set":{"at":bson::DateTime::from_millis(0)}},
    )
    .await
    .unwrap();
    let job = triggers::claim_for_replica(&db, "legacy", chrono::Utc::now(), "first")
        .await
        .unwrap()
        .unwrap();
    let _release = triggers::retain_shutdown_release(&db, &job, "first", &runtime);
    // This is precisely the old claim's write shape: generation is untouched.
    work.update_one(
        doc! {"_id":"legacy"},
        doc! {"$set":{"fence":"old-writer-new-fence"}},
    )
    .await
    .unwrap();
    assert_eq!(release::registered(&runtime).await.unwrap(), 0);
    assert_eq!(
        work.find_one(doc! {"_id":"legacy"})
            .await
            .unwrap()
            .unwrap()
            .get_str("fence")
            .unwrap(),
        "old-writer-new-fence"
    );
}

#[tokio::test]
async fn shutdown_node_handover_waits_for_late_socket_publication_and_teardown_before_peer_claim() {
    use crate::models::node::{COLLECTION_NAME, Node, NodeMetrics, NodeStatus};
    use crate::services::{node_owner_service as owners, node_ws_manager::NodeOutboundMessage};
    let db = connect_transaction_test_database("shutdown_node").await;
    let state = test_app_state(db.clone());
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    db.collection(COLLECTION_NAME)
        .insert_one(Node {
            id: id.clone(),
            user_id: uuid::Uuid::new_v4().to_string(),
            name: "drain fixture".into(),
            status: NodeStatus::Offline,
            auth_token_hash: "test".into(),
            signing_secret_encrypted: None,
            signing_secret_context_version: None,
            signing_secret_hash: "test".into(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            connection_owner: None,
            is_active: true,
            created_at: now,
            updated_at: now,
            machine: None,
            machine_confirm: Default::default(),
            allow_single_user_saved_logins: false,
        })
        .await
        .unwrap();
    owners::claim(
        &db,
        &id,
        &state.replica_identity,
        "socket",
        Duration::from_secs(600),
    )
    .await
    .unwrap()
    .unwrap();
    let (sender, _messages) = tokio::sync::mpsc::channel(8);
    let socket_work = state.shutdown.node_socket();
    let release_state = state.clone();
    let mut releasing = tokio::spawn(async move { release::nodes(&release_state).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut releasing)
            .await
            .is_err()
    );
    // Publish only after handover has observed the claimed, unregistered socket.
    let (_, mut close) =
        state
            .node_ws_manager
            .register_connection_with_id(&id, "socket".into(), sender);
    close.changed().await.unwrap();
    assert!(matches!(
        &*close.borrow(),
        Some(NodeOutboundMessage::Close { code: 1001, .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut releasing)
            .await
            .is_err()
    );
    assert!(
        db.collection::<Node>(COLLECTION_NAME)
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .unwrap()
            .connection_owner
            .is_some()
    );
    drop(socket_work);
    assert_eq!(releasing.await.unwrap().unwrap(), 1);
    let peer = identity("peer", "new-generation", "http://127.0.0.1:2");
    let owner = owners::claim(&db, &id, &peer, "replacement", Duration::from_secs(600))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(release::nodes(&state).await.unwrap(), 0);
    let stored = db
        .collection::<Node>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.connection_owner.unwrap().generation_id,
        owner.generation_id
    );
}

#[tokio::test]
async fn shutdown_idle_audit_is_one_chained_row_and_audit_failure_cannot_block_exit() {
    let db = connect_transaction_test_database("shutdown_audit").await;
    let state = test_app_state(db.clone());
    let drain = drain::Drain::begin(&state, Duration::from_secs(10), "SIGTERM").unwrap();
    for _ in 0..2 {
        drain
            .audit(
                &state,
                drain::End::Idle,
                Default::default(),
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await;
    }
    let audits = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"backend_shutdown_drain"})
            .await
            .unwrap(),
        1
    );
    let row = audits
        .find_one(doc! {"event_type":"backend_shutdown_drain"})
        .await
        .unwrap()
        .unwrap();
    assert!(row.contains_key("seq"));
    let data = row.get_document("event_data").unwrap();
    assert_eq!(
        data.get_str("generation").unwrap(),
        state.replica_identity.generation_id
    );
    assert_eq!(data.get_i64("turns_completed_during_drain").unwrap(), 0);
    assert!(!data.get_bool("deadline_reached").unwrap());
    db.run_command(doc! {"collMod": crate::models::audit_log::COLLECTION_NAME,
        "validator": {"event_type": {"$ne":"backend_shutdown_drain"}}, "validationLevel":"strict", "validationAction":"error"}).await.unwrap();
    let other = test_app_state(db);
    let failed = drain::Drain::begin(&other, Duration::from_secs(10), "SIGINT").unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        failed.audit(
            &other,
            drain::End::Idle,
            Default::default(),
            tokio::time::Instant::now() + Duration::from_millis(100),
        ),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn shutdown_start_audit_timeout_is_nonblocking_survives_stop_and_keeps_idle_outcome() {
    crate::test_utils::init_test_audit_chain_hmac_key();
    // An open loopback socket with no MongoDB handshake keeps selection pending.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = mongodb::Client::with_uri_str(format!(
        "mongodb://{}/?directConnection=true&serverSelectionTimeoutMS=60000",
        listener.local_addr().unwrap()
    ))
    .await
    .unwrap();
    for budget_secs in [30, 1] {
        let state = test_app_state(client.database("shutdown_start_audit_timeout"));
        let before = std::time::Instant::now();
        let drain =
            drain::Drain::begin(&state, Duration::from_secs(budget_secs), "SIGTERM").unwrap();
        assert!(before.elapsed() < Duration::from_millis(250));
        assert!(drain.counts.is_empty());
        assert_eq!(state.shutdown.counts().detached, 1);
        // Match finish_shutdown's join deadline for an immediate second signal.
        let join_deadline = tokio::time::Instant::now() + drain.cleanup_reserve() * 2 / 3;
        state.shutdown.stop();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), state.shutdown.tasks_stopped())
                .await
                .is_err(),
            "the pending audit must survive stop"
        );
        tokio::time::timeout_at(join_deadline, state.shutdown.tasks_stopped())
            .await
            .expect("the audit must finish within bounded cleanup, even with a short budget");
        assert_eq!(
            drain
                .wait(&state, drain.work_deadline(), std::future::pending())
                .await,
            drain::End::Idle
        );
        assert_eq!(state.shutdown.tracked_units(), 0);
        assert_eq!(state.shutdown.outcomes(), (0, vec![]));
    }
}

#[tokio::test]
async fn shutdown_fallback_query_uses_existing_heartbeat_index_without_collection_scan() {
    let db = connect_transaction_test_database("shutdown_query_plan").await;
    crate::services::assistant_nyxagent::ensure_indexes(&db)
        .await
        .unwrap();
    let rows = db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
    {
        use futures::TryStreamExt;
        let indexes: Vec<_> = rows
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            !indexes
                .iter()
                .any(|index| index.keys == doc! {"active_turn.worker_instance.id":1})
        );
    }
    rows.insert_many((0..1000).map(|i| doc! {"_id":format!("idle-{i}"),"user_id":format!("owner-{i}"),"active_turn":bson::Bson::Null})).await.unwrap();
    rows.insert_one(doc! {"_id":"active", "active_turn":{"heartbeat_at":bson::DateTime::now()}})
        .await
        .unwrap();
    let plan = db.run_command(doc! {"explain":{"find":crate::models::assistant_conversation::COLLECTION_NAME,"filter":drain::shutdown_candidates_filter(),"hint":drain::shutdown_candidates_index()},"verbosity":"executionStats"}).await.unwrap();
    let stats = plan.get_document("executionStats").unwrap();
    println!("shutdown candidates executionStats: {stats:?}");
    assert_eq!(stats.get_i32("nReturned").unwrap(), 1);
    assert_eq!(stats.get_i32("totalDocsExamined").unwrap(), 1);
    assert!(!format!("{:?}", plan.get_document("queryPlanner").unwrap()).contains("COLLSCAN"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_cluster_release_covers_owner_constants_and_excludes_billing_and_provision_tokens()
{
    let db = connect_transaction_test_database("shutdown_owner_constants").await;
    let owner = identity("owner", "gen", "http://127.0.0.1:1");
    let runtime = ClusterLeaseRuntime::new(
        crate::models::coordination::CoordinationHolder {
            instance_id: owner.instance_name.clone(),
            generation_id: owner.generation_id.clone(),
        },
        Duration::from_secs(30),
        Duration::from_secs(10),
    );
    let mut names: Vec<String> = release::CLUSTER_LEASES
        .iter()
        .map(|s| (*s).into())
        .collect();
    names.push(format!(
        "{}bot",
        crate::services::discord_gateway::SOCKET_LEASE_PREFIX
    ));
    for name in &names {
        assert!(runtime.acquire(&db, name).await.unwrap().is_some());
    }
    let billing = runtime
        .acquire(&db, crate::services::billing::reconcile::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    let provisioning = runtime
        .acquire(&db, "billing:provision:owner")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        release::cluster(&db, &owner).await.unwrap(),
        names.len() as u64
    );
    let peer = ClusterLeaseRuntime::new(
        crate::models::coordination::CoordinationHolder {
            instance_id: "peer".into(),
            generation_id: "new".into(),
        },
        Duration::from_secs(30),
        Duration::from_secs(10),
    );
    for name in names {
        assert!(peer.acquire(&db, &name).await.unwrap().is_some());
    }
    assert!(peer.acquire(&db, &billing.name).await.unwrap().is_none());
    assert!(
        peer.acquire(&db, &provisioning.name)
            .await
            .unwrap()
            .is_none()
    );
    assert!(LeaseStore::release(&db, &provisioning).await.unwrap());
    assert!(LeaseStore::release(&db, &billing).await.unwrap());
    db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "benchmark: explicitly run on the local MongoDB fixture"]
async fn shutdown_tracking_overhead_concurrent_requests_with_and_without_idle_waiter() {
    const TASKS: usize = 16;
    const PAIRS: usize = 20_000;
    let db = connect_transaction_test_database("shutdown_tracking_benchmark").await;
    for waiter in [false, true] {
        let runtime = Runtime::default();
        let held = runtime.work();
        let idle = waiter.then(|| {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.idle().await })
        });
        tokio::task::yield_now().await;
        let start = std::time::Instant::now();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..TASKS {
            let runtime = runtime.clone();
            tasks.spawn(async move {
                let started = std::time::Instant::now();
                let mut samples = Vec::new();
                for index in 0..PAIRS {
                    let sample = (index % 100 == 0).then(std::time::Instant::now);
                    // One response handoff, including both registrations and Drops.
                    let request = runtime.request();
                    let stream = runtime.stream();
                    drop(request);
                    drop(stream);
                    if let Some(sample) = sample {
                        samples.push(sample.elapsed().as_nanos());
                    }
                    if index % 1000 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                (started.elapsed().as_nanos(), samples)
            });
        }
        let mut task_ns = 0;
        let mut samples = Vec::new();
        while let Some(done) = tasks.join_next().await {
            let (elapsed, sample) = done.unwrap();
            task_ns += elapsed;
            samples.extend(sample);
        }
        samples.sort_unstable();
        println!(
            "tracking latency waiter={waiter}: mean task wall/op {:.1} ns; sampled p50={} ns p95={} ns",
            task_ns as f64 / (TASKS * PAIRS) as f64,
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100]
        );
        let elapsed = start.elapsed();
        println!(
            "tracking waiter={waiter}: {} requests, {:.1} ns/request (two register/drop pairs), elapsed={elapsed:?}",
            TASKS * PAIRS,
            elapsed.as_nanos() as f64 / (TASKS * PAIRS) as f64
        );
        drop(held);
        if let Some(idle) = idle {
            idle.await.unwrap();
        }
        assert_eq!(runtime.tracked_units(), 0);
    }
    let start = std::time::Instant::now();
    for _ in 0..200 {
        db.run_command(doc! {"ping":1}).await.unwrap();
    }
    println!(
        "MongoDB local ping: {:.1} us/round trip",
        start.elapsed().as_micros() as f64 / 200.0
    );
    db.drop().await.unwrap();
}
