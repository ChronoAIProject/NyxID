use super::*;
use crate::{
    models::user::{User, UserType},
    services::{
        feature_flag_service::{self as flags, FlagTarget},
        security_readiness_service as readiness,
    },
    test_utils::*,
};
use chrono::Timelike;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn consent_get_warm_default_off_adds_zero_commands_and_enabled_rollout_is_bounded() {
    let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
    let seen = commands.clone();
    let db = connect_test_database_with_command_handler(
        "crash_hot_path",
        EventHandler::callback(move |e| {
            if let CommandEvent::Started(e) = e {
                seen.lock().unwrap().push(e.command);
            }
        }),
    )
    .await
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&id, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let get = || {
        crate::handlers::users::get_analytics_consent(
            axum::extract::State(state.clone()),
            test_auth_user(&id),
        )
    };
    let _ = get().await.unwrap(); // Warm absence only.
    commands.lock().unwrap().clear();
    let (_, axum::Json(off)) = get().await.unwrap();
    assert_eq!(off.diagnostics_status, DiagnosticsStatus::NotOffered);
    let reads = commands.lock().unwrap().clone();
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(
        reads[0].get_str("find").unwrap(),
        crate::models::user::COLLECTION_NAME
    );
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let (_, axum::Json(on)) = get().await.unwrap();
    assert_eq!(on.diagnostics_status, DiagnosticsStatus::ConsentRequired);
    let reads = commands.lock().unwrap().clone();
    assert!(reads.len() <= 8, "bounded enabled path: {reads:?}");
    assert_eq!(
        reads
            .iter()
            .filter(
                |d| d.get_str("find") == Ok(crate::models::feature_flag_override::COLLECTION_NAME)
            )
            .count(),
        2
    );
    assert!(
        !reads
            .iter()
            .any(|d| d.get_str("find")
                == Ok(crate::models::feature_flag_definition::COLLECTION_NAME))
    );
    println!("consent GET enabled read count: {}", reads.len());
}

#[tokio::test]
async fn unserving_readiness_is_unknown_and_ingestion_returns_503_without_destroying_pending() {
    let (db, id, rev) = tests::account("crash_unserving").await;
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let extra = readiness::overlapping_test_replica(&db).await;
    readiness::remove_test_replica_capability(&db, &extra, FLAG).await;
    assert_eq!(eligibility(&db, &id).await, DiagnosticsStatus::Unknown);
    let state = test_app_state(db.clone());
    let error = crate::handlers::crash_reports::ingest(
        axum::extract::State(state),
        test_auth_user(&id),
        Ok(axum::extract::ConnectInfo(
            "127.0.0.1:1234".parse().unwrap(),
        )),
        axum::http::HeaderMap::new(),
        Ok(axum::body::Bytes::new()),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, AppError::SecurityReadinessUnavailable));
    crate::test_utils::await_responsive_majority_io(&db).await;
    assert!(
        (consent::permits_diagnostics(&db, &id, &rev).await
            == consent::ConsentPermission::Permitted(()))
    );
    flags::set_platform_override(&db, FLAG, &FlagTarget::Global, false, &id)
        .await
        .unwrap();
    assert_eq!(eligibility(&db, &id).await, DiagnosticsStatus::NotOffered);
    assert_eq!(admin::rollout(&db).await.unwrap(), admin::Rollout::Killed);
}

#[tokio::test]
async fn policy_two_put_requires_an_offer_and_legacy_put_stays_policy_one() {
    use crate::handlers::users::{AnalyticsConsentRequest, set_analytics_consent};
    let (db, id, _) = tests::account("crash_policy_offer").await;
    let state = test_app_state(db.clone());
    let put = |version: Option<&str>| {
        set_analytics_consent(
            axum::extract::State(state.clone()),
            test_auth_user(&id),
            axum::Json(AnalyticsConsentRequest {
                account_id: id.clone(),
                enabled: true,
                policy_version: version.map(str::to_owned),
            }),
        )
    };
    assert!(matches!(
        put(Some("2")).await,
        Err(AppError::ValidationError(_))
    ));
    let (_, axum::Json(old)) = put(None).await.unwrap();
    assert_eq!(old.policy_version.as_deref(), Some("1"));
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let (_, axum::Json(new)) = put(Some("2")).await.unwrap();
    assert_eq!(new.diagnostics_status, DiagnosticsStatus::Available);
    let extra = readiness::overlapping_test_replica(&db).await;
    readiness::remove_test_replica_capability(&db, &extra, FLAG).await;
    let error = put(Some("2")).await.err().unwrap();
    assert!(matches!(error, AppError::SecurityReadinessUnavailable));
    use axum::response::IntoResponse;
    assert_eq!(
        error.into_response().status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn rollout_summary_distinguishes_off_cohort_global_and_killed() {
    let (db, id, _) = tests::account("crash_rollout_states").await;
    assert_eq!(admin::rollout(&db).await.unwrap(), admin::Rollout::Off);
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    assert_eq!(admin::rollout(&db).await.unwrap(), admin::Rollout::Cohort);
    flags::set_platform_override(&db, FLAG, &FlagTarget::Global, true, &id)
        .await
        .unwrap();
    assert_eq!(admin::rollout(&db).await.unwrap(), admin::Rollout::Global);
    flags::set_platform_override(&db, FLAG, &FlagTarget::Global, false, &id)
        .await
        .unwrap();
    assert_eq!(admin::rollout(&db).await.unwrap(), admin::Rollout::Killed);
}

#[tokio::test]
async fn admin_rollout_write_invalidates_the_negative_scope_cache() {
    use crate::services::{audit_service::AuditActor, feature_flag_admin_service as admin_flags};
    init_test_audit_chain_hmac_key();
    let (db, id, _) = tests::account("crash_admin_cache").await;
    assert_eq!(eligibility(&db, &id).await, DiagnosticsStatus::NotOffered);
    readiness::ready_test_replica(&db).await;
    admin_flags::mutate(
        &db,
        FLAG,
        admin_flags::Mutation::Override {
            kind: crate::models::feature_flag_override::FlagTargetKind::User,
            target_id: Some(id.clone()),
            enabled: Some(true),
        },
        &AuditActor {
            user_id: id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        eligibility(&db, &id).await,
        DiagnosticsStatus::ConsentRequired
    );
}

#[tokio::test]
async fn crash_ingestion_distinguishes_permitted_denied_and_unknown_consent_reads() {
    use consent::ConsentPermission;
    let (db, id, revision) = tests::account("crash_consent_tristate").await;
    crate::test_utils::await_responsive_majority_io(&db).await;
    let permission = consent::permits_diagnostics(&db, &id, &revision).await;
    assert_eq!(permission, ConsentPermission::Permitted(()));
    let result = insert_with_consent(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &revision,
        tests::report(),
        std::future::ready(permission),
    )
    .await;
    assert!(result.is_ok());
    let denial = consent::permits_diagnostics(&db, &id, "stale").await;
    assert_eq!(denial, ConsentPermission::Denied);
    let error = insert_with_consent(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &revision,
        tests::report(),
        std::future::ready(denial),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::CrashReportConsentRequired));
    let unknown = async {
        consent::bounded_consent_read(std::future::pending(), Duration::ZERO)
            .await
            .for_revision(&revision)
    };
    let error = insert_with_consent(
        &db,
        &CrashConfig::default(),
        "test",
        &id,
        &revision,
        tests::report(),
        unknown,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::SecurityReadinessUnavailable));
    use axum::response::IntoResponse;
    assert_eq!(
        error.into_response().status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        db.collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn crash_timestamp_is_bounded_and_normalized_in_storage_and_export() {
    let (db, id, revision) = tests::account("crash_timestamp_normalized").await;
    let now = Utc::now();
    let mut report = tests::report();
    report.captured_at = format!("{}Z", now.format("%Y-%m-%dT%H:%M:%S"))
        .replace('Z', ".000000000000000000000000000000000000Z");
    assert!(validation::validate(&report, now).is_err());
    let expected = now.with_nanosecond(120_000_000).unwrap().to_rfc3339();
    report.captured_at = format!("{}.120000000Z", now.format("%Y-%m-%dT%H:%M:%S"));
    let (_, row_id) = tests::insert(&db, &CrashConfig::default(), "test", &id, &revision, report)
        .await
        .unwrap();
    let stored = db
        .collection::<ClientErrorReport>(COLLECTION_NAME)
        .find_one(doc! {"_id": row_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.report.captured_at, expected);
    let safe = crate::services::crash_report_export::SanitizedReport {
        report: &stored.report,
        environment: &stored.environment,
        severity: &stored.severity,
    };
    let bytes = crate::services::crash_report_export::sentry::envelope(&safe);
    let payload: serde_json::Value =
        serde_json::from_slice(bytes.split(|b| *b == b'\n').nth(2).unwrap()).unwrap();
    assert_eq!(payload["timestamp"], expected);
}

#[tokio::test]
async fn crash_rate_limits_check_account_then_ip_then_global_without_spending_global_on_refusal() {
    use crate::models::coordination::RATE_WINDOW_COLLECTION_NAME as WINDOWS;
    let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
    let seen = commands.clone();
    let db = connect_test_database_with_command_handler(
        "crash_rate_order",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                seen.lock().unwrap().push(event.command);
            }
        }),
    )
    .await
    .unwrap();
    crate::test_utils::ensure_rate_window_headroom(
        Duration::from_secs(60),
        Duration::from_secs(10),
    )
    .await;
    rate_limits(&db, "one", Some("127.0.0.1")).await.unwrap();
    let updates: Vec<_> = commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| command.get_str("findAndModify") == Ok(WINDOWS))
        .map(|command| command.to_string())
        .collect();
    assert_eq!(updates.len(), 4);
    for (command, namespace) in updates.iter().zip([
        "crash:account-minute",
        "crash:account-day",
        "crash:ip",
        "crash:global",
    ]) {
        assert!(command.contains(namespace), "{command}");
    }
    db.collection::<Document>(WINDOWS)
        .update_one(
            doc! {"namespace": "crash:account-minute"},
            doc! {"$set": {"count": 5_i64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        rate_limits(&db, "one", Some("127.0.0.1")).await,
        Err(AppError::RateLimitedWithRetry { .. })
    ));
    let global = db
        .collection::<Document>(WINDOWS)
        .find_one(doc! {"namespace": "crash:global"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(global.get_i64("count").unwrap(), 1);
    rate_limits(&db, "two", Some("127.0.0.2")).await.unwrap();
    let global = db
        .collection::<Document>(WINDOWS)
        .find_one(doc! {"namespace": "crash:global"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(global.get_i64("count").unwrap(), 2);
}
