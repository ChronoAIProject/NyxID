use super::*;
use serde_json::json;

fn result(status: &str, reachable: bool, reason: &str) -> ProbeResult {
    serde_json::from_value(json!({
        "service":{"id":"11111111-1111-4111-8111-111111111111"},
        "health":{"status":status,"reachable":reachable,"reason":reason,"checked_at":Utc::now()}
    }))
    .unwrap()
}

#[test]
fn attribution_keeps_unknown_and_transport_failures_distinct() {
    for (status, reachable, reason, expected) in [
        ("healthy", true, "ok", Assessment::OperationSpecificLikely),
        (
            "unhealthy",
            true,
            "status_mismatch",
            Assessment::HealthAssertionFailed,
        ),
        ("unhealthy", false, "broker_error", Assessment::BrokerError),
        ("unhealthy", false, "node_offline", Assessment::NodeOffline),
        ("unhealthy", false, "timeout", Assessment::Timeout),
        ("unknown", false, "", Assessment::Unknown),
        ("healthy", false, "", Assessment::Unknown),
    ] {
        assert_eq!(
            project(&result(status, reachable, reason)).assessment,
            expected
        );
    }
    let unavailable = project(&result("unknown", false, "no_health_path"));
    assert_eq!(unavailable.observation, Observation::Unknown);
    assert_eq!(unavailable.assessment, Assessment::Unknown);
}

#[test]
fn readiness_is_independent_and_stale_evidence_cannot_attribute() {
    let mut response = result("healthy", true, "ok");
    response.readiness = Some(CheckResult {
        status: CheckStatus::NotReady,
        reachable: Some(true),
        checked_at: Some(Utc::now()),
        ..Default::default()
    });
    assert_eq!(
        project(&response).assessment,
        Assessment::DependencyUnavailable
    );
    response.readiness.as_mut().unwrap().checked_at = None;
    assert_eq!(project(&response).assessment, Assessment::Unknown);
    response.health.checked_at = Some(Utc::now() - chrono::Duration::seconds(61));
    assert_eq!(project(&response).observation, Observation::Unknown);
}

#[test]
fn only_known_health_recipes_allow_automatic_probes() {
    let mut check = CheckResult::default();
    assert!(!known_health_path(&check));
    check.path = "healthz?secret=never_expose".into();
    check.source = Source::Owner;
    assert!(known_health_path(&check));
    check.reason = "no_health_path".into();
    assert!(!known_health_path(&check));
    let evidence = serde_json::to_string(&project(&result(
        "unhealthy",
        true,
        "arbitrary_internal_secret",
    )))
    .unwrap();
    assert!(!evidence.contains("arbitrary_internal_secret"));
    assert!(!evidence.contains("healthz"));
    assert!(evidence.len() < 160);
}

#[test]
fn local_denials_and_cancellation_do_not_probe() {
    for error in [
        AppError::Unauthorized("denied".into()),
        AppError::Forbidden("denied".into()),
        AppError::BadRequest("invalid operation".into()),
        AppError::ClientDisconnected,
        AppError::ServiceConcurrencyLimited,
    ] {
        assert!(!eligible_error(&error));
    }
    assert!(eligible_error(&AppError::NodeProxyTimeout));
    assert!(same_origin(
        "http://probe:8080/private?secret",
        "http://probe:8080"
    ));
    assert!(!same_origin("http://other:8080", "http://probe:8080"));
}

use crate::{
    AppState,
    models::user::{User, UserType},
    test_utils::{
        connect_test_database, test_app_config, test_app_state_with_config, test_user,
        test_user_endpoint, test_user_service,
    },
};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

pub(crate) async fn fixture() -> (AppState, String, String, MockServer) {
    let db = connect_test_database("invocation_diagnostic")
        .await
        .unwrap();
    let server = MockServer::start().await;
    let mut config = test_app_config();
    config.chrono_probe_url = Some(server.uri());
    let state = test_app_state_with_config(db, config);
    let actor = Uuid::new_v4().to_string();
    let id = Uuid::new_v4().to_string();
    let endpoint_id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    state
        .db
        .collection::<UserEndpoint>(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(test_user_endpoint(
            &endpoint_id,
            &actor,
            "private",
            "http://private.internal",
            None,
            None,
        ))
        .await
        .unwrap();
    state
        .db
        .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .insert_one(test_user_service(
            &id,
            &actor,
            "same-slug",
            &endpoint_id,
            None,
            None,
        ))
        .await
        .unwrap();
    feature_flag_service::set_platform_override(
        &state.db,
        feature_flag_service::SERVICE_MONITORING_FLAG_KEY,
        &feature_flag_service::FlagTarget::Global,
        true,
        &actor,
    )
    .await
    .unwrap();
    (state, actor, id, server)
}

pub(crate) fn status(id: &str, health: &str, reachable: bool, reason: &str) -> serde_json::Value {
    json!({
        "service":{"id":id,"secret":"must_not_leak"},
        "health":{"status":health,"reachable":reachable,"source":"owner","path":"healthz?secret=must_not_leak","reason":reason,"checked_at":Utc::now()},
        "readiness":null
    })
}

pub(crate) async fn mount_healthy(server: &MockServer, id: &str) {
    for (verb, suffix) in [("GET", "status"), ("POST", "probes")] {
        Mock::given(method(verb))
            .and(path(format!("/api/v1/monitoring/services/{id}/{suffix}")))
            .and(header("authorization", "Bearer original-caller"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(status(id, "healthy", true, "ok")),
            )
            .expect(1)
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn personal_exact_service_forwards_only_the_original_bearer_and_limits_repeated_failures() {
    let (state, actor, id, server) = fixture().await;
    // A colliding slug must not influence the exact selected instance.
    let other = Uuid::new_v4().to_string();
    state
        .db
        .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .insert_one(test_user_service(
            &other,
            &actor,
            "same-slug",
            "other-endpoint",
            None,
            None,
        ))
        .await
        .unwrap();
    mount_healthy(&server, &id).await;
    let observed = diagnose(
        &state.db,
        &state.config,
        &actor,
        &id,
        Some("original-caller"),
    )
    .await
    .unwrap();
    assert_eq!(observed.observation, Observation::Caller);
    assert_eq!(observed.assessment, Assessment::OperationSpecificLikely);
    let limited = diagnose(
        &state.db,
        &state.config,
        &actor,
        &id,
        Some("original-caller"),
    )
    .await
    .unwrap();
    assert_eq!(limited.outcome, Outcome::RateLimited);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn non_delegable_and_unauthorized_calls_never_use_a_system_key() {
    let (state, actor, id, server) = fixture().await;
    let result = diagnose(&state.db, &state.config, &actor, &id, None)
        .await
        .unwrap();
    assert_eq!(result.outcome, Outcome::NonDelegable);
    assert_eq!(result.observation, Observation::Unknown);
    let stranger = Uuid::new_v4().to_string();
    state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&stranger, UserType::Person))
        .await
        .unwrap();
    assert!(
        diagnose(
            &state.db,
            &state.config,
            &stranger,
            &id,
            Some("unprivileged")
        )
        .await
        .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn unconfigured_paths_do_not_start_discovery_and_identity_mismatches_fail_open() {
    for mismatch in [false, true] {
        let (state, actor, id, server) = fixture().await;
        let response = if mismatch {
            status(&Uuid::new_v4().to_string(), "healthy", true, "ok")
        } else {
            json!({"service":{"id":id},"health":{"status":"unknown","source":"none","reason":"no_health_path"}})
        };
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(1)
            .mount(&server)
            .await;
        let result = diagnose(
            &state.db,
            &state.config,
            &actor,
            &id,
            Some("original-caller"),
        )
        .await
        .unwrap();
        assert_eq!(result.observation, Observation::Unknown);
        assert_eq!(
            result.outcome,
            if mismatch {
                Outcome::Unavailable
            } else {
                Outcome::NoHealthPath
            }
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn changed_recipe_and_probe_identity_cannot_be_used_as_caller_evidence() {
    for identity_changed in [false, true] {
        let (state, actor, id, server) = fixture().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(status(&id, "healthy", true, "ok")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut response = status(&id, "healthy", true, "ok");
        if identity_changed {
            response["service"]["id"] = json!(Uuid::new_v4().to_string());
        } else {
            response["health"]["path"] = json!("new-path");
        }
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(1)
            .mount(&server)
            .await;
        let result = diagnose(
            &state.db,
            &state.config,
            &actor,
            &id,
            Some("original-caller"),
        )
        .await
        .unwrap();
        assert_eq!(result.observation, Observation::Unknown);
        assert_eq!(result.assessment, Assessment::Unknown);
        assert_eq!(result.outcome, Outcome::Unavailable);
    }
}

#[tokio::test]
async fn monitoring_outage_and_timeout_are_unknown_and_bounded() {
    for slow in [false, true] {
        let (state, actor, id, server) = fixture().await;
        let template = if slow {
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(2))
                .set_body_json(status(&id, "healthy", true, "ok"))
        } else {
            ResponseTemplate::new(503).set_body_string("secret internal failure")
        };
        Mock::given(method("GET"))
            .respond_with(template)
            .expect(1)
            .mount(&server)
            .await;
        let start = std::time::Instant::now();
        let result = diagnose(
            &state.db,
            &state.config,
            &actor,
            &id,
            Some("original-caller"),
        )
        .await
        .unwrap();
        assert_eq!(result.observation, Observation::Unknown);
        assert_eq!(
            result.outcome,
            if slow {
                Outcome::Timeout
            } else {
                Outcome::Unavailable
            }
        );
        assert!(start.elapsed() < Duration::from_millis(1100));
    }
}

#[tokio::test]
async fn flag_off_and_monitoring_service_aliases_do_not_recurse() {
    let (state, actor, id, server) = fixture().await;
    state
        .db
        .collection::<UserEndpoint>(crate::models::user_endpoint::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$set":{"url":format!("{}/private",server.uri())}},
        )
        .await
        .unwrap();
    assert!(
        diagnose(
            &state.db,
            &state.config,
            &actor,
            &id,
            Some("original-caller")
        )
        .await
        .is_none()
    );
    feature_flag_service::set_platform_override(
        &state.db,
        feature_flag_service::SERVICE_MONITORING_FLAG_KEY,
        &feature_flag_service::FlagTarget::Global,
        false,
        &actor,
    )
    .await
    .unwrap();
    assert!(
        diagnose(
            &state.db,
            &state.config,
            &actor,
            &id,
            Some("original-caller")
        )
        .await
        .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
