use super::*;
use crate::{
    services::{
        analytics_consent_service as consent,
        crash_report_service::tests::{account, report},
        feature_flag_service::{self as flags, FlagTarget},
        security_readiness_service as readiness,
    },
    test_utils::*,
};
use axum::{
    Router, body::Body, extract::rejection::ExtensionRejection, http::Request, routing::post,
};
use tower::ServiceExt;
fn no_peer() -> Result<ConnectInfo<std::net::SocketAddr>, ExtensionRejection> {
    Ok(ConnectInfo("127.0.0.1:1234".parse().unwrap()))
}
fn device(actor: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("x-nyxid-analytics", "enabled".parse().unwrap());
    h.insert("x-nyxid-analytics-account", actor.parse().unwrap());
    h
}
fn body(actor: &str, rev: &str) -> Bytes {
    Bytes::from(
        serde_json::to_vec(
            &serde_json::json!({"account_id":actor,"revision":rev,"report":report()}),
        )
        .unwrap(),
    )
}
#[tokio::test]
async fn ingestion_auth_consent_device_readiness_kill_and_encoding_matrix() {
    let (db, id, rev) = account("crash_handler").await;
    let state = test_app_state(db.clone());
    let auth = test_auth_user(&id);
    let result = ingest(
        State(state.clone()),
        auth.clone(),
        no_peer(),
        device(&id),
        Ok(body(&id, &rev)),
    )
    .await;
    assert!(matches!(result, Err(AppError::CrashReportingUnavailable)));
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    for mode in ["dnt", "sec-gpc", "missing-device", "wrong-account"] {
        let mut h = device(&id);
        match mode {
            "missing-device" => {
                h.remove("x-nyxid-analytics");
            }
            "wrong-account" => {
                h.insert(
                    "x-nyxid-analytics-account",
                    uuid::Uuid::new_v4().to_string().parse().unwrap(),
                );
            }
            _ => {
                h.insert(mode, "1".parse().unwrap());
            }
        }
        assert!(matches!(
            ingest(
                State(state.clone()),
                auth.clone(),
                no_peer(),
                h,
                Ok(body(&id, &rev))
            )
            .await,
            Err(AppError::CrashReportConsentRequired)
        ));
    }
    let mut h = device(&id);
    h.insert("content-encoding", "gzip".parse().unwrap());
    assert_eq!(
        ingest(
            State(state.clone()),
            auth.clone(),
            no_peer(),
            h,
            Ok(body(&id, &rev))
        )
        .await
        .unwrap()
        .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    // A separate account avoids obscuring the consent checks with its spent minute budget.
    let (db, id, rev) = account("crash_handler_accept").await;
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let auth = test_auth_user(&id);
    let bytes = body(&id, &rev);
    for _ in 0..2 {
        crate::test_utils::await_responsive_majority_io(&db).await;
        assert_eq!(
            ingest(
                State(state.clone()),
                auth.clone(),
                no_peer(),
                device(&id),
                Ok(bytes.clone())
            )
            .await
            .unwrap()
            .status(),
            StatusCode::ACCEPTED
        );
    }
    consent::set_acknowledging(&db, &id, false, consent::PolicyVersion::Diagnostics)
        .await
        .unwrap();
    crate::test_utils::await_responsive_majority_io(&db).await;
    assert!(matches!(
        ingest(
            State(state.clone()),
            auth.clone(),
            no_peer(),
            device(&id),
            Ok(body(&id, &rev))
        )
        .await,
        Err(AppError::CrashReportConsentRequired)
    ));
    let denied = db
        .collection::<bson::Document>(COUNTERS)
        .find_one(bson::doc! {"platform": "web", "outcome": "consent_denied"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(denied.get_i64("count").unwrap(), 1);
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::Global, false, &id)
        .await
        .unwrap();
    assert!(matches!(
        ingest(
            State(state.clone()),
            auth.clone(),
            no_peer(),
            device(&id),
            Ok(body(&id, &rev))
        )
        .await,
        Err(AppError::CrashReportingUnavailable)
    ));
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::ServiceAccount,
    ] {
        let mut forbidden = auth.clone();
        forbidden.auth_method = method;
        assert!(
            ingest(
                State(state.clone()),
                forbidden,
                no_peer(),
                device(&id),
                Ok(body(&id, &rev))
            )
            .await
            .is_err()
        );
    }
    assert!(require_admin(&state, &auth).await.is_err());
}
#[tokio::test]
async fn raw_ingress_bounds_body_and_closed_dto_rejects_unknown_fields() {
    let app = Router::new().route(
        "/",
        post(|bytes: Result<Bytes, BytesRejection>| async move {
            match bytes {
                Ok(bytes) => {
                    if serde_json::from_slice::<IngestRequest>(&bytes).is_ok() {
                        StatusCode::ACCEPTED
                    } else {
                        StatusCode::BAD_REQUEST
                    }
                }
                Err(_) => StatusCode::PAYLOAD_TOO_LARGE,
            }
        })
        .layer(axum::extract::DefaultBodyLimit::max(65536)),
    );
    for (bytes, status) in [
        (vec![b' '; 65537], StatusCode::PAYLOAD_TOO_LARGE),
        (
            br#"{"account_id":"x","revision":"x","report":{},"secret":"SECRET_SENTINEL"}"#.to_vec(),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(Request::post("/").body(Body::from(bytes)).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
}
#[tokio::test]
async fn legacy_put_records_policy_one_and_response_additions_are_safe() {
    let (db, id, _) = account("crash_legacy_policy").await;
    let state = test_app_state(db.clone());
    let old: super::super::users::AnalyticsConsentRequest =
        serde_json::from_str(&format!(r#"{{"account_id":"{id}","enabled":true}}"#)).unwrap();
    let (_, Json(response)) =
        super::super::users::set_analytics_consent(State(state), test_auth_user(&id), Json(old))
            .await
            .unwrap();
    assert_eq!(response.policy_version.as_deref(), Some("1"));
    assert_eq!(response.current_policy_version, "1");
    assert!(!response.diagnostics_available);
    assert!(consent::PolicyVersion::parse(Some("3")).is_err());
}

#[tokio::test]
async fn admin_operator_projection_and_nonhuman_denial() {
    use crate::services::role_service;
    use mongodb::bson::doc;
    let (db, id, rev) = account("crash_admin_roles").await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let auth = test_auth_user(&id);
    assert!(require_admin(&state, &auth).await.is_err());
    let report = report();
    let artifact = report.artifact_id.clone().unwrap();
    service::tests::register_report_release(&db, &report).await;
    crate::test_utils::await_responsive_majority_io(&db).await;
    let (_, report_id) = service::insert(&db, &state.crash_reports, "test", &id, &rev, report)
        .await
        .unwrap();
    for role in [roles.admin, roles.operator] {
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&id}, doc! {"$set":{"role_ids":[role]}})
            .await
            .unwrap();
        let response = detail(State(state.clone()), auth.clone(), Path(report_id.clone()))
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !text.contains("user_id")
                && !text.contains("consent_revision")
                && !text.contains(&id)
                && !text.contains(&rev)
        );
        let response = summary(
            State(state.clone()),
            auth.clone(),
            Query(SummaryQuery {
                window: None,
                platform: None,
            }),
        )
        .await
        .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            response["versions"][0]["artifact_ids"],
            serde_json::json!([artifact])
        );
        assert_eq!(response["versions"][0]["artifact_id"], artifact);
        for method in [
            crate::mw::auth::AuthMethod::ApiKey,
            crate::mw::auth::AuthMethod::Delegated,
            crate::mw::auth::AuthMethod::ServiceAccount,
        ] {
            let mut denied = auth.clone();
            denied.auth_method = method;
            assert!(
                detail(
                    State(state.clone()),
                    denied.clone(),
                    Path(report_id.clone())
                )
                .await
                .is_err()
            );
            assert!(
                ingest(
                    State(state.clone()),
                    denied,
                    no_peer(),
                    device(&id),
                    Ok(body(&id, &rev))
                )
                .await
                .is_err()
            );
        }
    }
}

#[tokio::test]
async fn sentinel_ingress_storage_and_tracing_never_retain_untrusted_text() {
    use tracing::instrument::WithSubscriber;
    let (db, id, rev) = account("crash_log_privacy").await;
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let capture = service::logging::tests::Capture::default();
    async {
        let mut raw = serde_json::json!({"account_id":id,"revision":rev,"report":report()});
        raw["report"]["message"] = serde_json::json!(
            "SECRET_SENTINEL Bearer secret person@example.test /Users/private eyJhbGci.token"
        );
        let result = ingest(
            State(state.clone()),
            test_auth_user(&id),
            no_peer(),
            device(&id),
            Ok(Bytes::from(serde_json::to_vec(&raw).unwrap())),
        )
        .await;
        assert!(matches!(result, Err(AppError::ValidationError(_))));
        crate::test_utils::await_responsive_majority_io(&db).await;
        ingest(
            State(state.clone()),
            test_auth_user(&id),
            no_peer(),
            device(&id),
            Ok(body(&id, &rev)),
        )
        .await
        .unwrap();
    }
    .with_subscriber(capture.subscriber())
    .await;
    let logs = capture.text();
    assert!(logs.contains("Crash ingestion outcome"));
    let fields: Vec<_> = capture
        .events()
        .into_iter()
        .map(|e| e["fields"].clone())
        .collect();
    assert!(fields.contains(&serde_json::json!({"message":"Crash ingestion outcome", "outcome":"invalid", "platform":"unknown"})));
    assert!(fields.contains(&serde_json::json!({"message":"Crash ingestion outcome", "outcome":"accepted", "platform":"web"})));
    for text in [
        "SECRET_SENTINEL",
        "Bearer secret",
        "person@example.test",
        "/Users/private",
        "eyJhbGci.token",
    ] {
        assert!(!logs.contains(text));
        let stored = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(mongodb::bson::doc! {})
            .await
            .unwrap()
            .unwrap();
        assert!(!format!("{stored:?}").contains(text));
    }
}

#[tokio::test]
async fn insert_failures_distinguish_unavailable_invalid_and_consent_denied_counters() {
    use consent::ConsentPermission;

    let (db, id, revision) = account("crash_insert_error_counters").await;
    // Exercise the real insert gate with a bounded-deadline failure, independent
    // of MongoDB latency and without delaying the admission or rate-limit paths.
    let unknown = consent::bounded_consent_read(std::future::pending(), std::time::Duration::ZERO)
        .await
        .for_revision(&revision);
    assert_eq!(unknown, ConsentPermission::Unknown);
    for (permission, invalid, status, outcome) in [
        (
            unknown,
            false,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
        ),
        (
            ConsentPermission::Denied,
            false,
            StatusCode::FORBIDDEN,
            "consent_denied",
        ),
        (
            ConsentPermission::Permitted(()),
            true,
            StatusCode::BAD_REQUEST,
            "invalid",
        ),
    ] {
        let mut report = report();
        if invalid {
            report.app_version = "invalid/version".into();
        }
        let error = service::insert_with_consent(
            &db,
            &crate::services::crash_report_export::CrashConfig::default(),
            "test",
            &id,
            &revision,
            report,
            std::future::ready(permission),
        )
        .await
        .unwrap_err();
        let response = insert_error(&db, Platform::Web, error)
            .await
            .unwrap_err()
            .into_response();
        assert_eq!(response.status(), status);
        let counter = db
            .collection::<bson::Document>(COUNTERS)
            .find_one(bson::doc! {"platform": "web", "outcome": outcome})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(counter.get_i64("count").unwrap(), 1);
    }
    assert_eq!(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .count_documents(bson::doc! {})
            .await
            .unwrap(),
        0
    );

    for error in [
        AppError::Internal("test internal failure".into()),
        AppError::DatabaseError(mongodb::error::Error::custom("test database failure")),
    ] {
        let response = insert_error(&db, Platform::Web, error)
            .await
            .unwrap_err()
            .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
    let counter = db
        .collection::<bson::Document>(COUNTERS)
        .find_one(bson::doc! {"platform": "web", "outcome": "unavailable"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counter.get_i64("count").unwrap(), 3);
}

#[tokio::test]
async fn server_validation_rejection_increments_invalid_counter_after_rate_limits() {
    let (db, id, rev) = account("crash_validation_counter").await;
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let mut raw = serde_json::to_value(report()).unwrap();
    raw["app_version"] = serde_json::json!("invalid/version");
    let value = serde_json::json!({
        "account_id": id,
        "revision": rev,
        "report": raw,
    });
    let state = test_app_state(db.clone());
    assert!(matches!(
        ingest(
            State(state),
            test_auth_user(&id),
            no_peer(),
            device(&id),
            Ok(Bytes::from(serde_json::to_vec(&value).unwrap())),
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    let counter = db
        .collection::<bson::Document>(COUNTERS)
        .find_one(bson::doc! {"platform":"web", "outcome":"invalid"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counter.get_i64("count").unwrap(), 1);
}

#[tokio::test]
async fn serving_readiness_with_flag_off_does_not_offer_diagnostics_or_fail_consent() {
    let (db, id, _) = account("crash_not_offered").await;
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    let state = test_app_state(db.clone());
    let (_, Json(response)) =
        super::super::users::get_analytics_consent(State(state.clone()), test_auth_user(&id))
            .await
            .unwrap();
    assert_eq!(response.current_policy_version, "1");
    assert!(!response.diagnostics_offered);
    assert!(!response.diagnostics_available);
    crate::services::feature_flag_service::set_platform_override(
        &db,
        service::FLAG,
        &crate::services::feature_flag_service::FlagTarget::User(id.clone()),
        true,
        &id,
    )
    .await
    .unwrap();
    let (_, Json(response)) =
        super::super::users::get_analytics_consent(State(state.clone()), test_auth_user(&id))
            .await
            .unwrap();
    assert_eq!(response.current_policy_version, "2");
    assert!(response.diagnostics_offered);
    crate::services::feature_flag_service::set_platform_override(
        &db,
        service::FLAG,
        &crate::services::feature_flag_service::FlagTarget::Global,
        false,
        &id,
    )
    .await
    .unwrap();
    let (_, Json(response)) =
        super::super::users::get_analytics_consent(State(state), test_auth_user(&id))
            .await
            .unwrap();
    assert_eq!(response.current_policy_version, "1");
    assert!(!response.diagnostics_offered);
}

#[tokio::test]
async fn crash_status_lookup_failures_do_not_fail_consent_get_or_put() {
    let (db, id, _) = account("crash_status_failure").await;
    readiness::ready_test_replica(&db).await;
    db.collection::<bson::Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .insert_one(bson::doc! {"_id":uuid::Uuid::new_v4().to_string(),"org_user_id":null,"enabled":"invalid"}).await.unwrap();
    let state = test_app_state(db.clone());
    let (_, Json(response)) =
        super::super::users::get_analytics_consent(State(state.clone()), test_auth_user(&id))
            .await
            .unwrap();
    assert_eq!(response.current_policy_version, "1");
    assert!(!response.diagnostics_offered);
    let (_, Json(response)) = super::super::users::set_analytics_consent(
        State(state),
        test_auth_user(&id),
        Json(super::super::users::AnalyticsConsentRequest {
            account_id: id.clone(),
            enabled: false,
            policy_version: None,
        }),
    )
    .await
    .unwrap();
    assert!(!response.enabled);
    assert!(!response.diagnostics_offered);
    assert_eq!(response.policy_version.as_deref(), Some("1"));
}

#[tokio::test]
async fn release_registration_is_admin_only_first_party_idempotent_and_metadata_audited() {
    use crate::services::role_service;
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    let (db, id, _) = account("crash_register_admin").await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let mut auth = test_auth_user(&id);
    auth.ip_address = Some("SECRET_SENTINEL".into());
    auth.user_agent = Some("person@example.test".into());
    let request = || service::releases::RegisterRelease {
        platform: Platform::Ios,
        app_version: "1.0.4".into(),
        app_build: "45".into(),
        artifact_id: None,
    };
    for role in [None, Some(roles.operator)] {
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"role_ids":role.into_iter().collect::<Vec<_>>()}},
            )
            .await
            .unwrap();
        let error = register_release(State(state.clone()), auth.clone(), Json(request()))
            .await
            .err()
            .unwrap();
        assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    }
    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id":&id}, doc! {"$set":{"role_ids":[roles.admin]}})
        .await
        .unwrap();
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::Delegated,
        crate::mw::auth::AuthMethod::ServiceAccount,
        crate::mw::auth::AuthMethod::AccessToken,
    ] {
        let mut other = auth.clone();
        other.auth_method = method.clone();
        if method == crate::mw::auth::AuthMethod::AccessToken {
            other.oauth_client_id = Some(uuid::Uuid::new_v4().to_string());
        }
        assert!(
            register_release(State(state.clone()), other, Json(request()))
                .await
                .is_err()
        );
    }
    let releases =
        db.collection::<bson::Document>(crate::models::client_release_registry::COLLECTION_NAME);
    let audits = db.collection::<crate::models::audit_log::AuditLog>(
        crate::models::audit_log::COLLECTION_NAME,
    );
    assert_eq!(releases.count_documents(doc! {}).await.unwrap(), 0);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"crash_reports.release_registered"})
            .await
            .unwrap(),
        0
    );
    for status in ["registered", "already_registered"] {
        let response = register_release(State(state.clone()), auth.clone(), Json(request()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"status":status})
        );
    }
    assert_eq!(releases.count_documents(doc! {}).await.unwrap(), 1);
    let stored = releases.find_one(doc! {}).await.unwrap().unwrap();
    assert_eq!(stored.get_str("registered_by").unwrap(), id);
    let rows: Vec<_> = audits
        .find(doc! {"event_type":"crash_reports.release_registered"})
        .sort(doc! {"seq":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for (row, outcome) in rows.iter().zip(["registered", "already_registered"]) {
        assert_eq!(row.user_id.as_deref(), Some(id.as_str()));
        assert!(row.ip_address.is_none() && row.user_agent.is_none());
        let bytes = serde_json::to_string(row).unwrap();
        assert!(!bytes.contains("SECRET_SENTINEL") && !bytes.contains("person@example.test"));
        assert_eq!(
            row.event_data,
            Some(
                serde_json::json!({"platform":"ios","app_version":"1.0.4","app_build":"45","artifact_id":null,"outcome":outcome})
            )
        );
    }
    let mut bad = request();
    bad.app_version = "SECRET_SENTINEL/person@example.test".into();
    assert!(matches!(
        register_release(State(state.clone()), auth.clone(), Json(bad)).await,
        Err(AppError::ValidationError(_))
    ));
    assert!(serde_json::from_value::<service::releases::RegisterRelease>(serde_json::json!({"platform":"ios","app_version":"1","app_build":"1","message":"SECRET_SENTINEL"})).is_err());
    // A rejected registry write must never claim a successful registration in audit.
    db.run_command(doc! {"collMod":crate::models::client_release_registry::COLLECTION_NAME,"validator":{"_id":{"$type":"int"}}}).await.unwrap();
    let mut rejected = request();
    rejected.app_build = "46".into();
    assert!(
        register_release(State(state), auth, Json(rejected))
            .await
            .is_err()
    );
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"crash_reports.release_registered"})
            .await
            .unwrap(),
        2
    );
    assert_eq!(releases.count_documents(doc! {}).await.unwrap(), 1);
}

#[tokio::test]
async fn default_off_ingestion_never_performs_an_unthrottled_counter_write() {
    let (db, id, rev) = account("crash_off_no_writes").await;
    let state = test_app_state(db.clone());
    for _ in 0..10 {
        assert!(matches!(
            ingest(
                State(state.clone()),
                test_auth_user(&id),
                no_peer(),
                device(&id),
                Ok(body(&id, &rev))
            )
            .await,
            Err(AppError::CrashReportingUnavailable)
        ));
    }
    assert_eq!(
        db.collection::<bson::Document>(COUNTERS)
            .count_documents(bson::doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn web_manifest_runs_only_after_successful_insert_and_never_delays_ingestion() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    let (db, id, revision) = account("crash_manifest_after_insert").await;
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new().route(
        "/crash-build.json",
        axum::routing::get({
            let started = started.clone();
            let release = release.clone();
            let calls = calls.clone();
            move || {
                let started = started.clone();
                let release = release.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    let _permit = release.acquire().await.unwrap();
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut state = test_app_state(db.clone());
    state.config.environment = "development".into();
    state.config.frontend_url = origin;
    state.config.domain_aliases.frontend_urls.clear();
    let send = |rev: &str| {
        ingest(
            State(state.clone()),
            test_auth_user(&id),
            no_peer(),
            device(&id),
            Ok(body(&id, rev)),
        )
    };
    assert!(matches!(
        send(&uuid::Uuid::new_v4().to_string()).await,
        Err(AppError::CrashReportConsentRequired)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    db.run_command(bson::doc! {"collMod":COLLECTION_NAME,"validator":{"_id":{"$type":"int"}}})
        .await
        .unwrap();
    assert!(send(&revision).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    db.run_command(bson::doc! {"collMod":COLLECTION_NAME,"validator":{}})
        .await
        .unwrap();
    // Admission's linearizable consent gate keeps its production 200/250 ms
    // bounds. Establish responsive majority I/O after the validator writes.
    crate::test_utils::await_responsive_majority_io(&db).await;
    let response = tokio::time::timeout(Duration::from_secs(2), send(&revision))
        .await
        .expect("manifest I/O must not be inline")
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .count_documents(bson::doc! {})
            .await
            .unwrap(),
        1
    );
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .expect("background fetch must start");
    assert_eq!(state.shutdown.counts().detached, 1);
    assert_eq!(state.shutdown.begin_drain().unwrap().detached, 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), state.shutdown.idle())
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<bson::Document>(crate::models::client_release_registry::COLLECTION_NAME)
            .count_documents(bson::doc! {})
            .await
            .unwrap(),
        0
    );
    release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), state.shutdown.idle())
        .await
        .expect("drain completes only after registration returns");
    assert_eq!(state.shutdown.counts().detached, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
