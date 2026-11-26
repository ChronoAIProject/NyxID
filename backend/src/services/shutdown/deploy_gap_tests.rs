use super::*;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use chrono::Utc;
use mongodb::bson::{self, Document, doc};
use std::time::Duration;
use tower::ServiceExt;

async fn events(db: &mongodb::Database, event: &str) -> Vec<Document> {
    use futures::TryStreamExt;
    db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":event})
        .sort(doc! {"seq":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
async fn shutdown_deploy_gap_start_records_identify_restarts_and_previous_heartbeat() {
    let db = crate::test_utils::connect_transaction_test_database("deploy_gap_start").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let previous = uuid::Uuid::new_v4().to_string();
    let heartbeat = Utc::now() - chrono::Duration::seconds(12);
    let observations = db.collection::<Document>(crate::models::security_replica::COLLECTION_NAME);
    observations.insert_many([
        doc! {"_id":&previous,"generation_id":&previous,"instance_name":&state.replica_identity.instance_name,"observed_at":bson::DateTime::from_chrono(heartbeat)},
        doc! {"_id":"older","generation_id":"older","instance_name":&state.replica_identity.instance_name,"observed_at":bson::DateTime::from_chrono(heartbeat-chrono::Duration::seconds(50))},
        doc! {"_id":"unrelated","generation_id":"unrelated","instance_name":"other-instance","observed_at":bson::DateTime::now()},
        doc! {"_id":&state.replica_identity.generation_id,"generation_id":&state.replica_identity.generation_id,"instance_name":&state.replica_identity.instance_name,"observed_at":bson::DateTime::now()},
    ]).await.unwrap();
    let bound_at = Utc::now();
    startup::listeners_bound(&state, bound_at).await.unwrap();
    let rows = events(&db, "backend_process_started").await;
    assert_eq!(rows.len(), 1, "missing process-start record");
    assert!(rows[0].contains_key("entry_hash"));
    let data = rows[0].get_document("event_data").unwrap();
    assert_eq!(
        data.get_str("generation").unwrap(),
        state.replica_identity.generation_id
    );
    assert!(!data.get_str("build_commit").unwrap().is_empty());
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(data.get_str("listeners_bound_at").unwrap()).unwrap(),
        bound_at
    );
    let prior = data.get_document("previous_generation").unwrap();
    assert_eq!(prior.get_str("generation").unwrap(), previous);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(prior.get_str("last_heartbeat_at").unwrap())
            .unwrap()
            .timestamp_millis(),
        heartbeat.timestamp_millis()
    );
    let mut restarted = state.clone();
    restarted.replica_identity =
        Arc::new(crate::services::node_owner_service::ReplicaIdentity::new(
            state.replica_identity.instance_name.clone(),
            state.replica_identity.internal_base_url.clone(),
        ));
    startup::listeners_bound(&restarted, Utc::now())
        .await
        .unwrap();
    let rows = events(&db, "backend_process_started").await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1]
            .get_document("event_data")
            .unwrap()
            .get_document("previous_generation")
            .unwrap()
            .get_str("generation")
            .unwrap(),
        state.replica_identity.generation_id
    );
    assert_eq!(
        rows[1].get_str("prev_hash").unwrap(),
        rows[0].get_str("entry_hash").unwrap()
    );
    assert!(events(&db, "backend_shutdown_drain").await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_deploy_gap_start_is_bounded_and_does_not_block_health() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut options = mongodb::options::ClientOptions::parse(format!(
        "mongodb://{}/?directConnection=true",
        listener.local_addr().unwrap()
    ))
    .await
    .unwrap();
    options.server_selection_timeout = Some(Duration::from_secs(60));
    let client = mongodb::Client::with_options(options).unwrap();
    let mut state = crate::test_utils::test_app_state_no_db().await;
    state.db = client.database("deploy_gap_unavailable");
    let start = tokio::time::Instant::now();
    let record = startup::listeners_bound(&state, Utc::now());
    let app = Router::new()
        .route("/health", get(crate::handlers::health::health_check))
        .with_state(state);
    let response = tokio::time::timeout(
        Duration::from_millis(500),
        app.oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(4), record)
        .await
        .unwrap()
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(4));
}

fn total(snapshot: &serde_json::Value, field: &str) -> u64 {
    snapshot["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket[field].as_u64().unwrap())
        .sum()
}

#[tokio::test(start_paused = true)]
async fn shutdown_deploy_gap_traffic_counts_probes_delayed_responses_and_expires() {
    let runtime = Runtime::default();
    let (send, receive) = tokio::sync::oneshot::channel();
    let entered = Arc::new(Mutex::new(Some(send)));
    let release = Arc::new(tokio::sync::Notify::new());
    let notify = release.clone();
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/ready", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
        .route(
            "/slow",
            get(move || {
                let entered = entered.clone();
                let notify = notify.clone();
                async move {
                    entered.lock().unwrap().take().unwrap().send(()).unwrap();
                    notify.notified().await;
                    StatusCode::OK
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            runtime.clone(),
            crate::mw::shutdown::track,
        ));
    for uri in ["/health", "/ready"] {
        app.clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
    }
    let slow = tokio::spawn(
        app.clone()
            .oneshot(Request::builder().uri("/slow").body(Body::empty()).unwrap()),
    );
    receive.await.unwrap();
    let snapshot = serde_json::to_value(runtime.recent_traffic()).unwrap();
    assert_eq!(total(&snapshot, "received"), 3);
    assert_eq!(total(&snapshot, "responded"), 2);
    tokio::time::advance(Duration::from_secs(5)).await;
    release.notify_one();
    slow.await.unwrap().unwrap();
    let snapshot = serde_json::to_value(runtime.recent_traffic()).unwrap();
    let buckets = snapshot["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0]["received"], 3);
    assert_eq!(buckets[0]["responded"], 2);
    assert_eq!(buckets[1]["responded"], 1);
    assert!(buckets[0]["start_at"].is_string());
    tokio::time::advance(Duration::from_secs(65)).await;
    let snapshot = serde_json::to_value(runtime.recent_traffic()).unwrap();
    assert_eq!(snapshot["buckets"].as_array().unwrap().len(), 12);
    assert_eq!(total(&snapshot, "received"), 0);
    assert_eq!(total(&snapshot, "responded"), 0);
    app.oneshot(
        Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    let snapshot = serde_json::to_value(runtime.recent_traffic()).unwrap();
    assert_eq!(total(&snapshot, "received"), 1);
    assert_eq!(total(&snapshot, "responded"), 1);
}

#[tokio::test]
async fn shutdown_deploy_gap_drain_records_include_recent_traffic() {
    let db = crate::test_utils::connect_transaction_test_database("deploy_gap_drain").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .layer(middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ));
    app.oneshot(
        Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    let drain = drain::Drain::begin(&state, Duration::from_secs(25), "SIGTERM").unwrap();
    state.shutdown.tasks_stopped().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::time::resume();
    let recorded = async {
        loop {
            if !events(&db, "backend_shutdown_drain_progress")
                .await
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    // Drop the progress writer before finish competes for the audit append lock.
    tokio::select! {
        never = drain.progress(&state) => match never {},
        result = tokio::time::timeout(Duration::from_secs(3), recorded) => result.unwrap(),
    };
    drain
        .audit(
            &state,
            drain::End::Idle,
            Default::default(),
            tokio::time::Instant::now() + Duration::from_secs(3),
        )
        .await;
    for event in [
        "backend_shutdown_drain_started",
        "backend_shutdown_drain_progress",
        "backend_shutdown_drain",
    ] {
        let rows = events(&db, event).await;
        assert_eq!(rows.len(), 1, "{event}");
        let data = rows[0].get_document("event_data").unwrap();
        let traffic = serde_json::to_value(
            data.get_document("recent_traffic")
                .expect("missing recent traffic"),
        )
        .unwrap();
        assert_eq!(total(&traffic, "received"), 1);
        assert_eq!(total(&traffic, "responded"), 1);
    }
    db.drop().await.unwrap();
}
