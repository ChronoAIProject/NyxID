use super::*;
use crate::{handlers::health::worker_health_router, test_utils::test_app_config};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test(start_paused = true)]
async fn stopped_worker_goes_stale_even_while_scraped_and_another_replica_runs() {
    let mut config = test_app_config();
    config.chain_verify_interval_secs = 3600;
    let jobs = Jobs::new(&config);
    let other_replica = Jobs::new(&config);
    let worker = jobs.chain_verify.clone();
    let task = tokio::spawn(async move {
        let runtime = Runtime::default();
        let mut hourly = tokio::time::interval(Duration::from_secs(3600));
        while let Some(_work) = worker.tick(&runtime, &mut hourly).await {
            worker.finish(true);
        }
    });
    tokio::task::yield_now().await;
    assert_eq!(jobs.chain_verify.status().status, "live");
    let last_success = jobs.chain_verify.status().last_success_at.unwrap();
    // Long scheduled waits prove worker liveness without running another scan.
    for _ in 0..12 {
        tokio::time::advance(HEARTBEAT).await;
        tokio::task::yield_now().await;
        let status = jobs.chain_verify.status();
        assert_eq!(status.status, "live");
        assert_eq!(status.last_success_at, Some(last_success));
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    for seconds in (10..=60).step_by(10) {
        tokio::time::advance(Duration::from_secs(10)).await;
        other_replica.chain_verify.finish(true);
        let status = jobs.snapshot();
        assert_eq!(status["chain_verify"].last_success_at, Some(last_success));
        assert_eq!(
            status["chain_verify"].status,
            if seconds == 60 { "stale" } else { "live" }
        );
    }
}

#[tokio::test(start_paused = true)]
async fn only_work_progress_can_keep_an_active_pass_live() {
    let job = Job::new(true);
    let worker = job.clone();
    let task = tokio::spawn(async move {
        worker
            .observe(async {
                for _ in 0..8 {
                    checkpoint(true);
                    tokio::time::sleep(HEARTBEAT).await;
                }
                std::future::pending::<Result<(), ()>>().await
            })
            .await
    });
    tokio::task::yield_now().await;
    for _ in 0..8 {
        tokio::time::advance(HEARTBEAT).await;
        tokio::task::yield_now().await;
        assert_eq!(job.status().status, "live");
        assert!(job.status().last_success_at.is_none());
    }
    tokio::time::advance(STALE).await;
    checkpoint(true); // unscoped manual work
    assert_eq!(job.status().status, "stale");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn failed_substep_survives_a_successful_return_and_recovery_preserves_evidence() {
    let job = Job::new(true);
    let failed_pass = async {
        checkpoint(false);
        checkpoint(true);
        Ok::<_, ()>(())
    };
    job.observe(failed_pass).await.unwrap();
    assert_eq!(job.status().status, "failed");
    assert!(job.status().last_success_at.is_none());
    let failure = job.status().last_failure_at.unwrap();
    job.observe(async {
        skipped();
        Ok::<_, ()>(())
    })
    .await
    .unwrap();
    assert_eq!(job.status().status, "live");
    assert!(job.status().last_success_at.is_none());
    job.observe(async { Ok::<_, ()>(()) }).await.unwrap();
    assert_eq!(job.status().status, "live");
    assert!(job.status().last_success_at.is_some());
    assert_eq!(job.status().last_failure_at, Some(failure));
}

#[tokio::test(start_paused = true)]
async fn internal_response_has_only_fixed_metadata_and_reports_unstarted_disabled_and_drain() {
    let mut config = test_app_config();
    config.channel_poll_interval_secs = 0;
    config.chain_verify_interval_secs = 3600;
    let jobs = Arc::new(Jobs::new(&config));
    let runtime = Runtime::default();
    let app = worker_health_router(jobs.clone(), runtime.clone());
    assert_eq!(jobs.turn_sweep.status().status, "starting");
    let _ = jobs
        .schedules
        .observe(async { Err::<(), _>("private driver detail") })
        .await;
    tokio::time::advance(STALE).await;
    jobs.schedules.progress();
    runtime.begin_drain();
    let response = app
        .oneshot(
            Request::get("/internal/v1/health/jobs")
                .header("Authorization", "Bearer private-test-value")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["content-type"], "application/json");
    let bytes = to_bytes(response.into_body(), 2048).await.unwrap();
    let actual: Value = serde_json::from_slice(&bytes).unwrap();
    let empty = |status| json!({"status":status, "heartbeat_at":null, "last_success_at":null, "last_failure_at":null});
    assert_eq!(
        actual,
        json!({
            "draining":true,
            "jobs":{
                "billing_reconcile": empty("stale"),
                "chain_verify": empty("stale"),
                "channel_polls": empty("disabled"),
                "schedules":{
                    "status":"failed",
                    "heartbeat_at":jobs.schedules.status().heartbeat_at.unwrap(),
                    "last_failure_at":jobs.schedules.status().last_failure_at.unwrap(),
                    "last_success_at":null
                },
                "turn_sweep":empty("stale"),
            }
        })
    );
}

#[tokio::test]
async fn worker_health_is_absent_from_the_public_listener() {
    let state = crate::test_utils::test_app_state_no_db().await;
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let response = public
        .merge(private)
        .with_state(state)
        .oneshot(
            Request::get("/internal/v1/health/jobs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn aborting_the_real_scheduler_expires_its_last_successful_empty_scan() {
    let db = crate::test_utils::connect_test_database("scheduler_heartbeat")
        .await
        .unwrap();
    crate::services::trigger_schedule::indexes(&db)
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let worker = crate::handlers::trigger_scheduler::spawn(state.clone());
    let job = &state.jobs.schedules;
    tokio::time::timeout(Duration::from_secs(5), async {
        while job.status().last_success_at.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("scheduler completed the empty scan");
    assert_eq!(job.status().status, "live");
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    db.drop().await.unwrap();
    tokio::time::pause();
    tokio::time::advance(STALE).await;
    assert_eq!(job.status().status, "stale");
}

#[tokio::test(start_paused = true)]
async fn intentionally_bounded_delivery_wait_remains_live_and_keeps_its_deadline() {
    let job = Job::new(true);
    let worker = job.clone();
    let task = tokio::spawn(async move {
        worker
            .observe(bounded_wait(
                Duration::from_secs(180),
                std::future::pending::<()>(),
            ))
            .await
    });
    tokio::task::yield_now().await;
    for _ in 0..11 {
        tokio::time::advance(HEARTBEAT).await;
        tokio::task::yield_now().await;
        assert_eq!(job.status().status, "live");
        assert!(job.status().last_success_at.is_none());
    }
    tokio::time::advance(HEARTBEAT).await;
    assert!(task.await.unwrap().is_err());
    assert_eq!(job.status().status, "failed");
}
