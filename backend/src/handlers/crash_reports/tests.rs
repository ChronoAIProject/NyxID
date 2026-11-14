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
    let (_, report_id) = service::insert(&db, &state.crash_reports, "test", &id, &rev, report())
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

#[derive(Clone)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn sentinel_ingress_storage_and_tracing_never_retain_untrusted_text() {
    let (db, id, rev) = account("crash_log_privacy").await;
    readiness::ready_test_replica(&db).await;
    flags::set_platform_override(&db, service::FLAG, &FlagTarget::User(id.clone()), true, &id)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let capture = Capture(Default::default());
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    // Keep the dispatcher installed for this current-thread runtime's entire
    // capture, including nested database futures and their production outcomes.
    let capture_guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
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
    .await;
    drop(capture_guard);
    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("Crash ingestion outcome"));
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
