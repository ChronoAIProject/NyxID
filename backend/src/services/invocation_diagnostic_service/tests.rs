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
            project(&result(status, reachable, reason), Observation::Caller).assessment,
            expected
        );
    }
    let unavailable = project(
        &result("unknown", false, "no_health_path"),
        Observation::Caller,
    );
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
        project(&response, Observation::Caller).assessment,
        Assessment::DependencyUnavailable
    );
    response.readiness = Some(CheckResult {
        configured: Some(false),
        ..Default::default()
    });
    assert_eq!(
        project(&response, Observation::Caller).assessment,
        Assessment::OperationSpecificLikely
    );
    response.readiness = Some(CheckResult {
        status: CheckStatus::NotConfigured,
        ..Default::default()
    });
    assert_eq!(
        project(&response, Observation::Caller).assessment,
        Assessment::OperationSpecificLikely
    );
    response.readiness = Some(CheckResult {
        status: CheckStatus::NotReady,
        reachable: Some(true),
        ..Default::default()
    });
    assert_eq!(
        project(&response, Observation::Caller).assessment,
        Assessment::Unknown
    );
    response.health.checked_at = Some(Utc::now() - chrono::Duration::seconds(61));
    assert_eq!(
        project(&response, Observation::Caller).observation,
        Observation::Unknown
    );
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
    let evidence = serde_json::to_string(&project(
        &result("unhealthy", true, "arbitrary_internal_secret"),
        Observation::Caller,
    ))
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
    models::api_key::ApiKeyPurpose,
    models::user::{User, UserType},
    mw::auth::AuthMethod,
    test_utils::{
        connect_test_database, test_app_config, test_app_state_with_config, test_auth_user,
        test_user, test_user_endpoint, test_user_service,
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

#[allow(clippy::too_many_arguments)]
async fn redeem_for_test(
    db: &mongodb::Database,
    config: &crate::config::AppConfig,
    actor_id: &str,
    execution_owner_id: &str,
    auth_method: AuthMethod,
    api_key_id: Option<&str>,
    client_id: Option<&str>,
    allow_all_services: bool,
    allowed_service_ids: &[String],
    handle: &str,
    caller_token: Option<&str>,
) -> AppResult<RedeemResult> {
    let mut auth = test_auth_user(actor_id);
    auth.auth_method = auth_method;
    auth.api_key_id = api_key_id.map(str::to_owned);
    auth.allow_all_services = allow_all_services;
    auth.allowed_service_ids = allowed_service_ids.to_vec();
    if auth.auth_method == AuthMethod::AccessToken {
        auth.oauth_client_id = client_id.map(str::to_owned);
    } else {
        auth.acting_client_id = client_id.map(str::to_owned);
    }
    if auth.auth_method == AuthMethod::ServiceAccount {
        auth.approval_owner_user_id = Some(execution_owner_id.to_owned());
    }
    let credential = match auth.auth_method {
        AuthMethod::AccessToken => caller_token.map(CallerCredential::Bearer),
        AuthMethod::ApiKey => caller_token.map(CallerCredential::ApiKey),
        _ => None,
    };
    redeem(db, config, &auth, handle, credential).await
}

async fn diagnose_for_test(
    db: &mongodb::Database,
    config: &crate::config::AppConfig,
    actor_id: &str,
    exact_id: &str,
    caller_token: Option<&str>,
) -> Option<Diagnostic> {
    let mut auth = test_auth_user(actor_id);
    auth.auth_method = AuthMethod::AccessToken;
    diagnose(
        db,
        config,
        &auth,
        exact_id,
        caller_token.map(CallerCredential::Bearer),
        Observation::Caller,
    )
    .await
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

pub(crate) async fn mount_healthy_agent(server: &MockServer, id: &str, key: &str) {
    for (verb, suffix) in [("GET", "status"), ("POST", "probes")] {
        Mock::given(method(verb))
            .and(path(format!("/api/v1/monitoring/services/{id}/{suffix}")))
            .and(header("x-api-key", key))
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
    let observed = diagnose_for_test(
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
    let limited = diagnose_for_test(
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
    let result = diagnose_for_test(&state.db, &state.config, &actor, &id, None)
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
        diagnose_for_test(
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
        let result = diagnose_for_test(
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
        let result = diagnose_for_test(
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
                .set_delay(Duration::from_secs(4))
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
        let result = diagnose_for_test(
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
        assert!(start.elapsed() < Duration::from_millis(3500));
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
        diagnose_for_test(
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
        diagnose_for_test(
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

#[tokio::test]
async fn handle_redemption_probes_once_and_completed_result_is_retry_readable() {
    let (state, actor, id, server) = fixture().await;
    mount_healthy(&server, &id).await;
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("diagnostic handle must be issued");
    assert!(!issued.handle.contains(&actor));
    assert!(!issued.handle.contains(&id));

    let first = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    )
    .await
    .unwrap();
    let RedeemResult::Completed {
        diagnostic, fresh, ..
    } = first
    else {
        panic!("first redemption must complete");
    };
    assert!(fresh);
    assert_eq!(diagnostic.observation, Observation::Caller);
    assert_eq!(diagnostic.assessment, Assessment::OperationSpecificLikely);

    let second = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    )
    .await
    .unwrap();
    assert!(matches!(
        second,
        RedeemResult::Completed { fresh: false, .. }
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn general_agent_key_preserves_child_binding_and_returns_agent_evidence() {
    let (state, actor, id, server) = fixture().await;
    let key_id = Uuid::new_v4().to_string();
    let child_id = Uuid::new_v4().to_string();
    mount_healthy_agent(&server, &id, "current-agent-key").await;
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::ApiKey,
        Some(&key_id),
        Some(&child_id),
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("Agent Key diagnostic handle must be issued");

    let mut sibling = test_auth_user(&actor);
    sibling.auth_method = AuthMethod::ApiKey;
    sibling.api_key_id = Some(key_id.clone());
    sibling.api_key_credential_id = Some(Uuid::new_v4().to_string());
    sibling.scope = "proxy".into();
    let error = redeem(
        &state.db,
        &state.config,
        &sibling,
        &issued.handle,
        Some(CallerCredential::ApiKey("sibling-agent-key")),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));

    let mut auth = sibling;
    auth.api_key_credential_id = Some(child_id);
    let result = redeem(
        &state.db,
        &state.config,
        &auth,
        &issued.handle,
        Some(CallerCredential::ApiKey("current-agent-key")),
    )
    .await
    .unwrap();
    let RedeemResult::Completed {
        diagnostic, fresh, ..
    } = result
    else {
        panic!("Agent Key redemption must complete");
    };
    assert!(fresh);
    assert_eq!(diagnostic.observation, Observation::Agent);
    assert_eq!(diagnostic.assessment, Assessment::OperationSpecificLikely);
    assert_eq!(diagnostic.outcome, Outcome::Completed);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn handle_binding_expiry_and_non_delegable_identities_do_not_probe() {
    let (state, actor, id, server) = fixture().await;
    let client_bound = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        Some("oauth-client-a"),
        &id,
    )
    .await
    .unwrap()
    .expect("client-bound diagnostic handle must be issued");
    let error = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        Some("oauth-client-b"),
        true,
        &[],
        &client_bound.handle,
        Some("original-caller"),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));

    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::ApiKey,
        Some("agent-key-id"),
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("agent-key diagnostic handle must be issued");

    let stranger = uuid::Uuid::new_v4().to_string();
    let error = redeem_for_test(
        &state.db,
        &state.config,
        &stranger,
        &actor,
        AuthMethod::ApiKey,
        Some("agent-key-id"),
        None,
        true,
        &[],
        &issued.handle,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));

    let result = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::ApiKey,
        Some("agent-key-id"),
        None,
        true,
        &[],
        &issued.handle,
        None,
    )
    .await
    .unwrap();
    let RedeemResult::Completed { diagnostic, .. } = result else {
        panic!("non-delegable result must complete without probing");
    };
    assert_eq!(diagnostic.outcome, Outcome::NonDelegable);
    assert_eq!(diagnostic.observation, Observation::Unknown);
    assert!(server.received_requests().await.unwrap().is_empty());

    let expired = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("expiry-test diagnostic handle must be issued");
    state
        .db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"handle_hash":crate::crypto::token::hash_token(&expired.handle)},
            doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let error = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &expired.handle,
        Some("original-caller"),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn service_account_redemption_revalidates_with_its_execution_owner() {
    let (state, owner, id, server) = fixture().await;
    let service_account_id = Uuid::new_v4().to_string();
    let issued = issue(
        &state.db,
        &state.config,
        &service_account_id,
        &AuthMethod::ServiceAccount,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("service-account diagnostic handle must be issued");

    let result = redeem_for_test(
        &state.db,
        &state.config,
        &service_account_id,
        &owner,
        AuthMethod::ServiceAccount,
        None,
        None,
        true,
        &[],
        &issued.handle,
        None,
    )
    .await
    .unwrap();
    let RedeemResult::Completed { diagnostic, .. } = result else {
        panic!("service-account diagnostic must complete without probing");
    };
    assert_eq!(diagnostic.outcome, Outcome::NonDelegable);
    assert_eq!(diagnostic.observation, Observation::Unknown);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn shared_agent_redemption_preserves_owner_shared_connection_authority() {
    use crate::{
        models::{assistant_agent::GuestAccess, assistant_sharing::ShareRole},
        services::{
            assistant_team_service as team, feature_flag_service as flags,
            org_agent_service as access, org_agent_tests::Fixture, shared_agent_service as sharing,
        },
    };
    use std::{collections::BTreeMap, sync::Arc};

    let fixture = Fixture::new("invocation_diagnostic_shared_authority").await;
    sharing::ensure_indexes(&fixture.state.db).await.unwrap();
    flags::set_platform_override(
        &fixture.state.db,
        sharing::FLAG,
        &flags::FlagTarget::Global,
        true,
        &fixture.admin,
    )
    .await
    .unwrap();
    let (agent, _) = team::create_specialist(
        &fixture.state.db,
        &fixture.state.encryption_keys,
        &fixture.admin,
        fixture.request("diagnostic-shared-agent"),
    )
    .await
    .unwrap()
    .unwrap();
    let service = fixture.service(&fixture.admin, "shared-diagnostic").await;
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"grants.service_ids": [&service]}},
        )
        .await
        .unwrap();
    sharing::update(
        &fixture.state.db,
        &fixture.admin,
        &agent.id,
        sharing::SharingUpdate {
            expected_version: 0,
            enabled: true,
            audience: vec![
                serde_json::from_value(serde_json::json!({
                    "subject_kind": "person",
                    "email": format!("{}@example.com", fixture.outsider),
                    "role": ShareRole::User,
                }))
                .unwrap(),
            ],
            shared_access: BTreeMap::from([(service.clone(), GuestAccess::Read)]),
        },
        true,
    )
    .await
    .unwrap();
    let snapshot = sharing::require(&fixture.state.db, &fixture.outsider, &agent)
        .await
        .unwrap();
    let mut auth = test_auth_user(&fixture.outsider);
    auth.auth_method = AuthMethod::ApiKey;
    auth.api_key_id = Some(Uuid::new_v4().to_string());
    auth.allow_all_services = false;
    auth.allowed_service_ids = vec![service.clone()];
    auth.assistant_agent_owner_id = Some(fixture.admin.clone());
    auth.org_agent_access = Some(Arc::new(access::RequestAccess::from_shared(snapshot)));

    let mut config = test_app_config();
    config.chrono_probe_url = Some("http://diagnostic.invalid".into());
    let issued = issue(
        &fixture.state.db,
        &config,
        &fixture.outsider,
        &AuthMethod::ApiKey,
        auth.api_key_id.as_deref(),
        None,
        None,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    let result = redeem(&fixture.state.db, &config, &auth, &issued.handle, None)
        .await
        .unwrap();
    let RedeemResult::Completed { diagnostic, .. } = result else {
        panic!("shared Agent Key redemption must complete without probing");
    };
    assert_eq!(diagnostic.outcome, Outcome::NonDelegable);
    assert_eq!(diagnostic.observation, Observation::Unknown);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_model_transport_redemption_preserves_the_proxy_scope_exception() {
    let (state, actor, service, server) = fixture().await;
    let catalog_id = Uuid::new_v4().to_string();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.id.clone_from(&catalog_id);
    catalog.inference = Some(
        serde_json::from_value(
            serde_json::json!({"wire_protocol":"openai_responses","model_list":true}),
        )
        .unwrap(),
    );
    state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(catalog)
        .await
        .unwrap();
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &service},
            doc! {"$set": {"catalog_service_id": &catalog_id}},
        )
        .await
        .unwrap();

    let key_id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::assistant_agent_credential::AssistantAgentCredential>(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::assistant_agent_credential::AssistantAgentCredential {
                collaboration: false,
                secret_context_versions: Default::default(),
                id: Uuid::new_v4().to_string(),
                user_id: actor.clone(),
                conversation_id: Uuid::new_v4().to_string(),
                api_key_id: key_id.clone(),
                key_ciphertext: vec![],
                created_at: Utc::now(),
                last_used_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let mut auth = test_auth_user(&actor);
    auth.auth_method = AuthMethod::ApiKey;
    auth.api_key_id = Some(key_id.clone());
    auth.api_key_purpose = ApiKeyPurpose::General;
    auth.allow_all_services = false;
    auth.allowed_service_ids.clear();
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::ApiKey,
        Some(&key_id),
        None,
        None,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    let result = redeem(&state.db, &state.config, &auth, &issued.handle, None)
        .await
        .unwrap();
    let RedeemResult::Completed { diagnostic, .. } = result else {
        panic!("assistant model transport redemption must complete without probing");
    };
    assert_eq!(diagnostic.outcome, Outcome::NonDelegable);
    assert_eq!(diagnostic.observation, Observation::Unknown);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn running_claims_are_never_replayed() {
    let (state, actor, id, server) = fixture().await;
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("running-claim diagnostic handle must be issued");
    state
        .db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"handle_hash":crate::crypto::token::hash_token(&issued.handle)},
            doc! {"$set":{
                "state":"running",
                "claim_id":"dead-worker",
                "claimed_at":mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(30)),
            }},
        )
        .await
        .unwrap();
    let result = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    )
    .await
    .unwrap();
    assert!(matches!(result, RedeemResult::NotReady));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_redemptions_start_only_one_probe() {
    let (state, actor, id, server) = fixture().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/monitoring/services/{id}/status")))
        .and(header("authorization", "Bearer original-caller"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(150))
                .set_body_json(status(&id, "healthy", true, "ok")),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/monitoring/services/{id}/probes")))
        .and(header("authorization", "Bearer original-caller"))
        .respond_with(ResponseTemplate::new(200).set_body_json(status(&id, "healthy", true, "ok")))
        .expect(1)
        .mount(&server)
        .await;
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("concurrent-redemption diagnostic handle must be issued");

    let first = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    );
    let second = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();
    assert!(
        matches!(
            (&first, &second),
            (
                RedeemResult::Completed { fresh: true, .. },
                RedeemResult::NotReady
            ) | (
                RedeemResult::NotReady,
                RedeemResult::Completed { fresh: true, .. }
            )
        ),
        "one redemption must own the claim while the other observes it"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn redemption_waits_only_while_concurrency_capacity_is_exhausted() {
    let (state, actor, id, server) = fixture().await;
    let catalog_id = Uuid::new_v4().to_string();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.id.clone_from(&catalog_id);
    catalog.concurrency_policy = Some(
        crate::models::service_concurrency::ServiceConcurrencyPolicy {
            service_id: catalog_id.clone(),
            default_limit: Some(1),
            users: vec![],
            orgs: vec![],
        },
    );
    state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(catalog)
        .await
        .unwrap();
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":&catalog_id}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::service_concurrency::COLLECTION_NAME)
        .insert_one(doc! {
            "service_id":&catalog_id,
            "person_id":&actor,
            "leases":[{"expires_at":mongodb::bson::DateTime::from_chrono(
                Utc::now() + chrono::Duration::seconds(5)
            )}],
        })
        .await
        .unwrap();
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("capacity-test diagnostic handle must be issued");

    let result = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    )
    .await
    .unwrap();
    assert!(matches!(result, RedeemResult::NotReady));
    let row = state
        .db
        .collection::<InvocationDiagnostic>(COLLECTION_NAME)
        .find_one(doc! {"handle_hash":crate::crypto::token::hash_token(&issued.handle)})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, InvocationDiagnosticState::Pending);
    assert!(server.received_requests().await.unwrap().is_empty());

    mount_healthy(&server, &id).await;
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&catalog_id},
            doc! {"$set":{"concurrency_policy.default_limit":2_i32}},
        )
        .await
        .unwrap();
    let result = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        RedeemResult::Completed { fresh: true, .. }
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn issuance_does_not_read_the_feature_flag_but_redemption_respects_it() {
    let (state, actor, id, server) = fixture().await;
    feature_flag_service::set_platform_override(
        &state.db,
        feature_flag_service::SERVICE_MONITORING_FLAG_KEY,
        &feature_flag_service::FlagTarget::Global,
        false,
        &actor,
    )
    .await
    .unwrap();
    let issued = issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("feature-flag diagnostic handle must be issued");

    let result = redeem_for_test(
        &state.db,
        &state.config,
        &actor,
        &actor,
        AuthMethod::AccessToken,
        None,
        None,
        true,
        &[],
        &issued.handle,
        Some("original-caller"),
    )
    .await
    .unwrap();
    let RedeemResult::Completed { diagnostic, .. } = result else {
        panic!("disabled monitoring must complete without caller evidence");
    };
    assert_eq!(diagnostic.observation, Observation::Unknown);
    assert_eq!(diagnostic.outcome, Outcome::Unavailable);
    assert!(server.received_requests().await.unwrap().is_empty());
}
