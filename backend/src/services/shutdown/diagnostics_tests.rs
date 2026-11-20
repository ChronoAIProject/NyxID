use super::*;
use crate::test_utils::{connect_transaction_test_database, test_app_state};
use axum::{Router, body::Body, http::Request, middleware, routing::get};
use bson::{Document, doc};
use futures::TryStreamExt;
use std::{convert::Infallible, time::Duration};
use tokio::time::Instant;
use tower::ServiceExt;

async fn audit_data(db: &mongodb::Database, event: &str) -> Vec<Document> {
    db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type": event})
        .sort(doc! {"seq": 1})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            assert!(row.contains_key("entry_hash"));
            row.get_document("event_data").unwrap().clone()
        })
        .collect()
}

#[tokio::test]
async fn shutdown_service_labels_content_type_families_and_unresolved_request() {
    use axum::{extract::Path, http::header::CONTENT_TYPE, response::IntoResponse};
    let runtime = Runtime::default();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let entered = Arc::new(Mutex::new(Some(entered)));
    let app = Router::new()
        .route(
            "/unresolved/{id}",
            get(move || {
                let entered = entered.clone();
                async move {
                    entered.lock().unwrap().take().unwrap().send(()).unwrap();
                    std::future::pending::<()>().await
                }
            }),
        )
        .route(
            "/stream/{kind}",
            get(|Path(kind): Path<String>| async move {
                let mut response = Body::from_stream(futures::stream::pending::<
                    Result<axum::body::Bytes, Infallible>,
                >())
                .into_response();
                let content_type = match kind.as_str() {
                    "sse" => Some("Text/Event-Stream; private=secret"),
                    "json" => Some("application/json; charset=utf-8"),
                    "suffix" => Some("application/problem+json"),
                    "other" => Some("application/private-user-type"),
                    _ => None,
                };
                if let Some(value) = content_type {
                    response
                        .headers_mut()
                        .insert(CONTENT_TYPE, value.parse().unwrap());
                }
                response
            }),
        )
        .layer(middleware::from_fn_with_state(
            runtime.clone(),
            crate::mw::shutdown::track,
        ));
    let request = tokio::spawn(
        app.clone().oneshot(
            Request::builder()
                .uri("/unresolved/private-id")
                .body(Body::empty())
                .unwrap(),
        ),
    );
    waiting.await.unwrap();
    let mut responses = Vec::new();
    for kind in ["sse", "json", "suffix", "other", "missing"] {
        responses.push(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/stream/{kind}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        );
    }
    let summary = serde_json::to_value(runtime.open_http()).unwrap();
    let entries = summary["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        4,
        "response families must be separate groups"
    );
    for (family, count) in [("event-stream", 1), ("json", 2), ("other", 2)] {
        let entry = entries
            .iter()
            .find(|e| e["content_type"] == family)
            .unwrap();
        assert_eq!(entry["count"], count);
    }
    let unresolved = entries.iter().find(|e| e["kind"] == "request").unwrap();
    assert!(unresolved.get("service").is_none());
    assert!(unresolved.get("content_type").is_none());
    assert!(!summary.to_string().contains("private"));
    request.abort();
    let _ = request.await;
    drop(responses);
    assert!(runtime.open_http().entries.is_empty());
}

#[tokio::test]
async fn shutdown_service_labels_update_open_request_once_after_pool_selection() {
    use axum::{extract::State, response::IntoResponse};
    let runtime = Runtime::default();
    let app = Router::new()
        .route(
            "/api/v1/proxy/s/{slug}/{*path}",
            get(|State(runtime): State<Runtime>| async move {
                assert!(runtime.open_http().entries[0].service.is_none());
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(7)).await;
                let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
                catalog.slug = "failed-candidate".into();
                let (_, failed) = diagnostics::candidate(async {
                    diagnostics::resolved_catalog(&catalog);
                    assert!(runtime.open_http().entries[0].service.is_none());
                })
                .await;
                drop(failed);
                catalog.slug = "selected-catalog".into();
                let (_, selected) = diagnostics::candidate(async {
                    diagnostics::resolved_catalog(&catalog);
                })
                .await;
                selected.publish();
                catalog.slug = "must-not-overwrite".into();
                diagnostics::resolved_catalog(&catalog);
                let request = runtime.open_http();
                assert_eq!(
                    request.entries[0].service.as_deref(),
                    Some("selected-catalog")
                );
                assert_eq!(request.entries[0].kind, "request");
                assert_eq!(request.entries[0].oldest_age_secs, 7);
                assert!(request.entries[0].content_type.is_none());
                tokio::time::resume();
                (
                    [("content-type", "application/json")],
                    Body::from_stream(futures::stream::pending::<
                        Result<axum::body::Bytes, Infallible>,
                    >()),
                )
                    .into_response()
            }),
        )
        .with_state(runtime.clone())
        .layer(middleware::from_fn_with_state(
            runtime.clone(),
            crate::mw::shutdown::track,
        ));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/proxy/s/private-alias/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let summary = runtime.open_http();
    assert_eq!(
        summary.entries[0].service.as_deref(),
        Some("selected-catalog")
    );
    assert_eq!(summary.entries[0].kind, "stream");
    assert_eq!(summary.entries[0].content_type, Some("json"));
    drop(response);
    assert!(runtime.open_http().entries.is_empty());
}

#[tokio::test]
async fn shutdown_service_labels_uuid_and_unrelated_routes_cannot_expose_service_names() {
    use axum::response::IntoResponse;
    let runtime = Runtime::default();
    for (route, uri, slug, expected) in [
        (
            "/api/v1/proxy/{service_id}/{*path}",
            "/api/v1/proxy/private-id/events",
            "550e8400-e29b-41d4-a716-446655440000",
            Some("custom"),
        ),
        (
            "/ordinary/{id}",
            "/ordinary/private-id",
            "catalog-name",
            None,
        ),
    ] {
        let app = Router::new()
            .route(
                route,
                get(move || async move {
                    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
                    catalog.slug = slug.into();
                    diagnostics::resolved_catalog(&catalog);
                    Body::from_stream(futures::stream::pending::<
                        Result<axum::body::Bytes, Infallible>,
                    >())
                    .into_response()
                }),
            )
            .layer(middleware::from_fn_with_state(
                runtime.clone(),
                crate::mw::shutdown::track,
            ));
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let summary = runtime.open_http();
        assert_eq!(summary.entries[0].service.as_deref(), expected);
        assert!(!serde_json::to_string(&summary).unwrap().contains(slug));
        drop(response);
        assert!(runtime.open_http().entries.is_empty());
    }
}

#[tokio::test]
async fn shutdown_diagnostics_start_finish_are_bounded_sorted_and_metadata_only() {
    let db = connect_transaction_test_database("shutdown_diagnostics_routes").await;
    let state = test_app_state(db.clone());
    let mut app = Router::new();
    for index in 0..23 {
        app = app.route(
            &format!("/route{index}/{{id}}"),
            get(|| async {
                Body::from_stream(futures::stream::pending::<
                    Result<axum::body::Bytes, Infallible>,
                >())
            }),
        );
    }
    let app = app.layer(middleware::from_fn_with_state(
        state.shutdown.clone(),
        crate::mw::shutdown::track,
    ));
    let mut responses = Vec::new();
    for index in (0..23).chain([0, 0, 0]) {
        responses.push(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/route{index}/private-id?token=private-query"))
                        .header("authorization", "private-header")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        );
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3)).await;
    tokio::time::resume();
    let drain = drain::Drain::begin(&state, Duration::from_secs(600), "SIGTERM").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while audit_data(&db, "backend_shutdown_drain_started")
            .await
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drain
        .audit(
            &state,
            drain::End::Deadline,
            Default::default(),
            Instant::now() + Duration::from_secs(3),
        )
        .await;
    for (event, field) in [
        ("backend_shutdown_drain_started", "open_http_at_start"),
        ("backend_shutdown_drain", "open_http_at_finish"),
    ] {
        let rows = audit_data(&db, event).await;
        assert_eq!(rows.len(), 1);
        let summary = rows[0]
            .get_document(field)
            .expect("missing route breakdown");
        let entries = summary.get_array("entries").unwrap();
        assert_eq!(entries.len(), 20);
        assert_eq!(summary.get_i64("other_count").unwrap(), 3);
        let top = entries[0].as_document().unwrap();
        assert_eq!(top.get_str("route").unwrap(), "/route0/{id}");
        assert_eq!(top.get_str("method").unwrap(), "GET");
        assert_eq!(top.get_str("kind").unwrap(), "stream");
        assert_eq!(top.get_i64("count").unwrap(), 4);
        assert!(top.get_i64("oldest_age_secs").unwrap() >= 3);
        assert!(!summary.to_string().contains("private"));
    }
    drop(responses);
    assert_eq!(state.shutdown.tracked_units(), 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_diagnostics_progress_fixed_points_budget_and_finish() {
    for budget in [10, 25, 60, 300, 301, 600] {
        let db = connect_transaction_test_database("shutdown_diagnostics_progress").await;
        let state = test_app_state(db.clone());
        let work = state.shutdown.work();
        let drain = drain::Drain::begin(&state, Duration::from_secs(budget), "SIGTERM").unwrap();
        state.shutdown.tasks_stopped().await;
        let progress = drain.progress(&state);
        tokio::pin!(progress);
        for point in [10, 25, 60, 300] {
            tokio::time::pause();
            tokio::time::advance(
                (drain.started + Duration::from_secs(point))
                    .saturating_duration_since(Instant::now()),
            )
            .await;
            tokio::time::resume();
            let expected = [10, 25, 60, 300]
                .into_iter()
                .filter(|p| *p <= point && *p < budget)
                .count();
            let recorded = async {
                loop {
                    let rows = audit_data(&db, "backend_shutdown_drain_progress").await;
                    if rows.len() >= expected {
                        return rows;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            };
            let rows = tokio::select! {
                biased;
                never = &mut progress => match never {},
                result = tokio::time::timeout(Duration::from_secs(5), recorded) =>
                    result.unwrap_or_else(|_| panic!("missing checkpoint: budget={budget}, point={point}")),
            };
            assert_eq!(rows.len(), expected, "budget={budget}, point={point}");
            for (row, expected_point) in rows.iter().zip([10, 25, 60, 300]) {
                assert_eq!(row.get_i64("elapsed_secs").unwrap(), expected_point);
                assert_eq!(
                    row.get_str("generation").unwrap(),
                    state.replica_identity.generation_id
                );
                assert_eq!(
                    row.get_document("in_flight")
                        .unwrap()
                        .get_i64("detached")
                        .unwrap(),
                    1
                );
                assert!(row.contains_key("node_operations"));
                assert!(row.contains_key("open_http"));
            }
        }
        tokio::select! {
            never = &mut progress => match never {},
            () = drain.audit(&state, drain::End::Idle, Default::default(), Instant::now() + Duration::from_secs(3)) => {},
        }
        drop(work);
        let _ = tokio::time::timeout(Duration::from_millis(20), &mut progress).await;
        assert_eq!(
            audit_data(&db, "backend_shutdown_drain_progress")
                .await
                .len(),
            [10, 25, 60, 300]
                .into_iter()
                .filter(|p| *p < budget)
                .count()
        );
        assert_eq!(state.shutdown.tracked_units(), 0);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn shutdown_diagnostics_tracks_requests_upgrades_and_safe_fallbacks_but_not_subscriptions() {
    use axum::{extract::State, http::Method, response::IntoResponse};
    let runtime = Runtime::default();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let entered = Arc::new(Mutex::new(Some(entered)));
    let app = Router::new()
        .route(
            "/request/{id}",
            get(move || {
                let entered = entered.clone();
                async move {
                    entered.lock().unwrap().take().unwrap().send(()).unwrap();
                    std::future::pending::<()>().await;
                }
            }),
        )
        .route(
            "/upgrade/{id}",
            get(|State(runtime): State<Runtime>| async move {
                let upgrade = runtime
                    .clone()
                    .upgrade_callback(|()| std::future::pending());
                // The actual upgrade owns the guard beyond the request's scope.
                let task = runtime.spawn_worker(upgrade(()));
                axum::Extension(Arc::new(task)).into_response()
            }),
        )
        .route(
            "/subscription",
            get(|| async {
                let mut response = Body::from_stream(futures::stream::pending::<
                    Result<axum::body::Bytes, Infallible>,
                >())
                .into_response();
                response.extensions_mut().insert(RecoverableSubscription);
                response
            }),
        )
        .fallback(|| async {
            Body::from_stream(futures::stream::pending::<
                Result<axum::body::Bytes, Infallible>,
            >())
        })
        .with_state(runtime.clone())
        .layer(middleware::from_fn_with_state(
            runtime.clone(),
            crate::mw::shutdown::track,
        ));
    let request = tokio::spawn(
        app.clone().oneshot(
            Request::builder()
                .uri("/request/private")
                .body(Body::empty())
                .unwrap(),
        ),
    );
    waiting.await.unwrap();
    let upgrade = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/upgrade/private")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let subscription = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/subscription")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let fallback = app
        .oneshot(
            Request::builder()
                .method(Method::from_bytes(b"PRIVATE-METHOD").unwrap())
                .uri("/private-path?private-query")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(7)).await;
    let summary = runtime.open_http();
    assert_eq!(summary.entries.len(), 3);
    for (route, method, kind) in [
        ("/request/{id}", "GET", "request"),
        ("/upgrade/{id}", "GET", "stream"),
        ("<unmatched>", "OTHER", "stream"),
    ] {
        let entry = summary
            .entries
            .iter()
            .find(|entry| entry.route == route)
            .unwrap();
        assert_eq!(
            (entry.method, entry.kind, entry.count, entry.oldest_age_secs),
            (method, kind, 1, 7)
        );
    }
    assert_eq!(runtime.counts().subscriptions, 1);
    assert!(!serde_json::to_string(&summary).unwrap().contains("private"));
    request.abort();
    let _ = request.await;
    runtime.stop();
    drop((upgrade, subscription, fallback));
    runtime.tasks_stopped().await;
    assert!(runtime.open_http().entries.is_empty());
    assert_eq!(runtime.tracked_units(), 0);
}

#[tokio::test]
async fn shutdown_diagnostics_early_finish_cancels_future_checkpoints() {
    let db = connect_transaction_test_database("shutdown_diagnostics_early").await;
    let state = test_app_state(db.clone());
    let drain = drain::Drain::begin(&state, Duration::from_secs(600), "SIGTERM").unwrap();
    state.shutdown.tasks_stopped().await;
    let progress = drain.progress(&state);
    tokio::pin!(progress);
    let _ = tokio::time::timeout(Duration::from_millis(1), &mut progress).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3)).await;
    tokio::time::resume();
    assert_eq!(
        drain
            .wait(&state, drain.work_deadline(), std::future::pending())
            .await,
        drain::End::Idle
    );
    drain
        .audit(
            &state,
            drain::End::Idle,
            Default::default(),
            Instant::now() + Duration::from_secs(3),
        )
        .await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(400)).await;
    tokio::time::resume();
    let _ = tokio::time::timeout(Duration::from_millis(20), &mut progress).await;
    assert!(
        audit_data(&db, "backend_shutdown_drain_progress")
            .await
            .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn shutdown_diagnostics_stalled_progress_does_not_hold_idle_or_finish() {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = mongodb::Client::with_uri_str(format!(
        "mongodb://{}/?directConnection=true&serverSelectionTimeoutMS=60000",
        listener.local_addr().unwrap()
    ))
    .await
    .unwrap();
    let state = test_app_state(client.database("shutdown_diagnostics_stalled"));
    let mut drain = drain::Drain::begin(&state, Duration::from_secs(600), "SIGTERM").unwrap();
    state.shutdown.tasks_stopped().await;
    drain.started -= Duration::from_secs(10);
    let progress = drain.progress(&state);
    tokio::pin!(progress);
    let _ = tokio::time::timeout(Duration::from_millis(20), &mut progress).await;
    assert_eq!(state.shutdown.tracked_units(), 0);
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(20),
            drain.wait(&state, drain.work_deadline(), std::future::pending())
        )
        .await
        .unwrap(),
        drain::End::Idle
    );
    let finish = drain.audit(
        &state,
        drain::End::SecondSignal,
        Default::default(),
        Instant::now() + Duration::from_millis(20),
    );
    tokio::time::timeout(Duration::from_millis(100), async {
        tokio::select! {
            () = finish => {},
            never = &mut progress => match never {},
        }
    })
    .await
    .expect("pending progress must not add its 2.5s audit allowance to finish");
    state.shutdown.stop();
    state.shutdown.tasks_stopped().await;
}

#[tokio::test(start_paused = true)]
async fn shutdown_diagnostics_oldest_age_follows_remaining_work_and_reused_slots() {
    let runtime = Runtime::default();
    let oldest = runtime.request();
    tokio::time::advance(Duration::from_secs(8)).await;
    let newer = runtime.request();
    assert_eq!(runtime.open_http().entries[0].oldest_age_secs, 8);
    drop(oldest);
    assert_eq!(runtime.open_http().entries[0].oldest_age_secs, 0);
    let stream = runtime.stream();
    drop(newer);
    let summary = runtime.open_http();
    assert_eq!(summary.entries.len(), 1);
    assert_eq!(summary.entries[0].kind, "stream");
    assert_eq!(summary.entries[0].count, 1);
    drop(stream);
    assert!(runtime.open_http().entries.is_empty());
}
