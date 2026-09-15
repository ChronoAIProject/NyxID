use super::super::app_requirements::tests::{
    Fixture, catalog, evidence, fixture, requirement, service,
};
use super::*;
use crate::models::app_connect_link::{AppConnectStatus, COLLECTION_NAME as LINKS};
use crate::models::app_requirement_manifest::{Enforcement, ValidatorSelection};
use crate::models::authorization_code::{AuthorizationCode, COLLECTION_NAME as CODES};
use crate::models::service_validation_record::ValidationOutcome;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::app_requirement_manifest_service as manifests;
use crate::services::token_service;
use axum::extract::Path;
use base64::Engine;
use mongodb::bson::{self, Document};
use sha2::{Digest, Sha256};

const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";
fn human(f: &Fixture) -> AuthUser {
    let mut auth = f.auth.clone();
    auth.auth_method = AuthMethod::Session;
    auth.oauth_client_id = None;
    auth
}
fn params(f: &Fixture) -> AuthorizeQuery {
    serde_json::from_value(serde_json::json!({
        "response_type": "code", "client_id": f.app.id, "redirect_uri": "https://app.example/callback",
        "scope": "openid proxy", "state": "original-state", "nonce": "original-nonce",
        "code_challenge": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
        "code_challenge_method": "S256",
    })).unwrap()
}
async fn gated(f: &Fixture, profile: bool, connect: bool) -> Option<String> {
    let slug = "api-github";
    let catalog_id = catalog(f, slug, "bearer").await;
    let mut r = requirement(slug);
    if profile {
        r.validator = ValidatorSelection::Profile {
            id: "github_user_v1".into(),
        };
    }
    manifests::publish(
        &f.state,
        &f.app,
        &f.auth.user_id.to_string(),
        manifests::PublishManifest {
            enforcement: Enforcement::Gate,
            requirements: vec![r],
        },
    )
    .await
    .unwrap();
    if !connect {
        return None;
    }
    let (s, key) = service(
        f,
        &f.auth.user_id.to_string(),
        &catalog_id,
        "personal-github",
    )
    .await;
    if profile {
        evidence(f, &s, &key, ValidationOutcome::Authenticated).await;
    }
    Some(s.id)
}
async fn authorize_response(f: &Fixture, p: &AuthorizeQuery) -> Response {
    authorize_inner(&f.state, OptionalAuthUser(Some(human(f))), p, true, None)
        .await
        .unwrap()
}
fn location(response: &Response) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .into()
}
async fn session(f: &Fixture, p: &AuthorizeQuery) -> AppConnectLink {
    let response = authorize_response(f, p).await;
    let url = url::Url::parse(&location(&response)).unwrap();
    assert!(
        url.path().starts_with("/connect/app/"),
        "expected checklist redirect"
    );
    let id = url.path_segments().unwrap().next_back().unwrap();
    let capability = url.fragment().unwrap().strip_prefix("t=").unwrap();
    app_links::redeem(&f.state, id, &f.auth.user_id.to_string(), capability)
        .await
        .unwrap()
}
async fn ready(f: &Fixture, p: &AuthorizeQuery) -> (AppConnectLink, ConsentDecisionForm) {
    let started = session(f, p).await;
    let link = app_links::ready(&f.state, &started.id, &started.user_id)
        .await
        .unwrap();
    assert_eq!(link.status, AppConnectStatus::ReadyForConsent);
    let url = app_connect_consent_url(&f.state, &link).await.unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    let token = parsed
        .query_pairs()
        .find(|(k, _)| k == "consent_request")
        .unwrap()
        .1
        .into_owned();
    let form = serde_json::from_value(serde_json::json!({
        "response_type": "code", "client_id": "ignored-form-client", "redirect_uri": "https://attacker.test",
        "scope": "openid proxy", "decision": "allow", "consent_request": token,
        "allowed_service_ids": link.selected_service_ids, "resource": p.resource,
    })).unwrap();
    (link, form)
}
async fn decide(f: &Fixture, form: ConsentDecisionForm) -> AppResult<Response> {
    authorize_decision(
        State(f.state.clone()),
        OptionalAuthUser(Some(human(f))),
        TelemetryContext::default(),
        Form(form),
    )
    .await
}
async fn count(f: &Fixture, collection: &str) -> u64 {
    f.state
        .db
        .collection::<Document>(collection)
        .count_documents(doc! {})
        .await
        .unwrap()
}

#[tokio::test]
async fn app_connect_authorize_db_prompt_none_unmet_creates_no_session_or_code() {
    let Some(f) = fixture("gate_none").await else {
        return;
    };
    gated(&f, true, false).await;
    let mut p = params(&f);
    p.prompt = Some("none".into());
    let response = authorize_response(&f, &p).await;
    assert_eq!(
        location(&response),
        build_callback_error_url(
            &p,
            "interaction_required",
            "App connection requirements need interaction"
        )
    );
    assert_eq!(count(&f, LINKS).await, 0);
    assert_eq!(count(&f, CODES).await, 0);
}

#[tokio::test]
async fn app_connect_authorize_db_gate_publishes_and_forged_omission_cannot_issue_code() {
    let Some(f) = fixture("gate_omission").await else {
        return;
    };
    gated(&f, true, true).await;
    let (link, mut form) = ready(&f, &params(&f)).await;
    form.allowed_service_ids.clear();
    assert!(matches!(
        decide(&f, form).await,
        Err(AppError::AppConnectResultMismatch)
    ));
    assert_eq!(count(&f, CODES).await, 0);
    assert_eq!(count(&f, "consents").await, 0);
    assert_eq!(
        app_links::load(&f.state, &link.id, &link.user_id)
            .await
            .unwrap()
            .status,
        AppConnectStatus::ReadyForConsent
    );
}

#[tokio::test]
async fn app_connect_authorize_db_unbound_token_cannot_claim_session() {
    let Some(f) = fixture("gate_unbound").await else {
        return;
    };
    gated(&f, false, true).await;
    let p = params(&f);
    let (_, mut form) = ready(&f, &p).await;
    form.consent_request = Some(
        sign_consent_request(&f.state, &f.auth.user_id.to_string(), &p, "openid proxy").unwrap(),
    );
    assert!(matches!(
        decide(&f, form).await,
        Err(AppError::AppConnectResultMismatch)
    ));
    assert_eq!(count(&f, CODES).await, 0);
}

#[tokio::test]
async fn app_connect_authorize_db_ready_rejects_61_second_evidence() {
    let Some(f) = fixture("gate_stale").await else {
        return;
    };
    gated(&f, true, true).await;
    let link = session(&f, &params(&f)).await;
    f.state.db.collection::<Document>("service_validation_records").update_many(doc! {},doc! { "$set": { "checked_at": bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(61)) } }).await.unwrap();
    assert!(matches!(
        app_links::ready(&f.state, &link.id, &link.user_id).await,
        Err(AppError::RequirementNotMet)
    ));
    assert_eq!(count(&f, CODES).await, 0);
}

#[tokio::test]
async fn app_connect_authorize_db_force_opens_even_with_fresh_covered_services() {
    let Some(f) = fixture("gate_force").await else {
        return;
    };
    let id = gated(&f, true, true).await.unwrap();
    consent_service::grant_consent_with_services(
        &f.state.db,
        &f.auth.user_id.to_string(),
        &f.app.id,
        "openid proxy",
        Some(vec![id]),
    )
    .await
    .unwrap();
    let mut p = params(&f);
    let silent = authorize_response(&f, &p).await;
    assert!(location(&silent).starts_with("https://app.example/callback?code="));
    assert_eq!(count(&f, LINKS).await, 0);
    p.nyx_connect = Some("force".into());
    let forced = session(&f, &p).await;
    assert!(matches!(forced.origin, AppConnectOrigin::Authorize { .. }));
    assert_eq!(count(&f, LINKS).await, 1);
}

#[tokio::test]
async fn app_connect_authorize_db_par_consumption_preserves_validated_parameters() {
    let Some(f) = fixture("gate_par").await else {
        return;
    };
    gated(&f, false, true).await;
    let p = params(&f);
    let (request_uri, _) = par_service::create_request(
        &f.state.db,
        &f.app.id,
        "code",
        &p.redirect_uri,
        p.scope.as_deref(),
        p.state.as_deref(),
        p.code_challenge.as_deref(),
        p.code_challenge_method.as_deref(),
        p.nonce.as_deref(),
        None,
        Some("force"),
        &[],
        None,
        None,
    )
    .await
    .unwrap();
    let raw: AuthorizeQuery = serde_json::from_value(serde_json::json!({"client_id":f.app.id,"request_uri":request_uri,"redirect_uri":"https://attacker.test","state":"forged"})).unwrap();
    let response = authorize(
        State(f.state.clone()),
        OptionalAuthUser(Some(human(&f))),
        HeaderMap::new(),
        Ok(Query(raw)),
    )
    .await
    .unwrap();
    let url = url::Url::parse(&location(&response)).unwrap();
    let id = url.path_segments().unwrap().next_back().unwrap();
    let link = app_links::load(&f.state, id, &f.auth.user_id.to_string())
        .await
        .unwrap();
    let AppConnectOrigin::Authorize {
        authorize_params: stored,
        ..
    } = link.origin
    else {
        panic!("authorize origin");
    };
    assert_eq!(stored.redirect_uri, p.redirect_uri);
    assert_eq!(stored.state, p.state);
    assert_eq!(stored.nonce, p.nonce);
    assert_eq!(stored.nyx_connect.as_deref(), Some("force"));
    assert_eq!(count(&f, "pushed_authorization_requests").await, 0);
    assert!(
        par_service::consume_request(&f.state.db, &request_uri, &f.app.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_connect_authorize_db_client_disabled_or_rollout_revoked_refuses_decision() {
    let Some(f) = fixture("gate_disabled").await else {
        return;
    };
    gated(&f, false, true).await;
    let (_, form) = ready(&f, &params(&f)).await;
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(
            doc! { "_id": &f.app.id },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert!(decide(&f, form).await.is_err());
    assert_eq!(count(&f, CODES).await, 0);
}

#[tokio::test]
async fn app_connect_authorize_db_consent_completion_and_code_are_single_use() {
    let Some(f) = fixture("gate_commit").await else {
        return;
    };
    let id = gated(&f, true, true).await.unwrap();
    let (link, form) = ready(&f, &params(&f)).await;
    let cloned: ConsentDecisionForm=serde_json::from_value(serde_json::json!({"response_type":"code","client_id":f.app.id,"redirect_uri":"https://app.example/callback","decision":"allow","consent_request":form.consent_request,"allowed_service_ids":[id]})).unwrap();
    let response = decide(&f, form).await.unwrap();
    assert!(location(&response).contains("&state=original-state"));
    assert_eq!(
        app_links::load(&f.state, &link.id, &link.user_id)
            .await
            .unwrap()
            .status,
        AppConnectStatus::Completed
    );
    assert!(decide(&f, cloned).await.is_err());
    assert_eq!(count(&f, CODES).await, 1);
    let code = f
        .state
        .db
        .collection::<AuthorizationCode>(CODES)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(code.allowed_service_ids, vec![id]);
    assert!(!code.allow_all_services);
    assert_eq!(code.nonce.as_deref(), Some("original-nonce"));
}

#[tokio::test]
async fn app_connect_authorize_db_cancel_wins_over_consent_without_grant_or_code() {
    let Some(f) = fixture("gate_cancel").await else {
        return;
    };
    gated(&f, false, true).await;
    let (link, form) = ready(&f, &params(&f)).await;
    let cancelled = app_links::cancel(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    let callback = app_links::terminal_callback_url(&cancelled)
        .unwrap()
        .unwrap();
    let url = url::Url::parse(&callback).unwrap();
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "error" && v == "access_denied")
    );
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "state" && v == "original-state")
    );
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "nyx_connect_status" && v == "cancelled")
    );
    assert!(decide(&f, form).await.is_err());
    assert_eq!(count(&f, CODES).await, 0);
    assert_eq!(count(&f, "consents").await, 0);
}

#[tokio::test]
async fn app_connect_authorize_db_token_narrowing_keeps_gated_and_rfc_resource_boundary() {
    let Some(f) = fixture("gate_narrow").await else {
        return;
    };
    let required_id = gated(&f, true, true).await.unwrap();
    let cat = catalog(&f, "extra-api", "bearer").await;
    let (extra, _) = service(&f, &f.auth.user_id.to_string(), &cat, "extra-service").await;
    let mut p = params(&f);
    let extra_uri = oauth_resource_service::user_service_resource_uri(&f.state.config, &extra.slug);
    p.resource = vec![extra_uri.clone()];
    let (_, mut form) = ready(&f, &p).await;
    form.allowed_service_ids.push(extra.id.clone());
    let response = decide(&f, form).await.unwrap();
    let url = url::Url::parse(&location(&response)).unwrap();
    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let stored = f
        .state
        .db
        .collection::<AuthorizationCode>(CODES)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.allowed_service_ids.contains(&required_id));
    assert!(stored.allowed_service_ids.contains(&extra.id));
    let exchanged = oauth_service::exchange_authorization_code(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        false,
        &code,
        &f.app.id,
        &p.redirect_uri,
        Some(VERIFIER),
        None,
        None,
        Some(std::slice::from_ref(&extra_uri)),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(exchanged.resource_uris, vec![extra_uri]);
    assert_eq!(exchanged.allowed_service_ids, vec![extra.id]);
    assert!(!exchanged.allow_all_services);
}

#[tokio::test]
async fn app_connect_authorize_db_app_b_cannot_read_authorize_session() {
    let Some(f) = fixture("gate_app_b").await else {
        return;
    };
    gated(&f, false, true).await;
    let link = session(&f, &params(&f)).await;
    let mut caller = f.auth.clone();
    caller.oauth_client_id = Some(uuid::Uuid::new_v4().to_string());
    let result = super::super::app_connect_links::get(
        State(f.state.clone()),
        caller,
        Path(link.id),
        axum::extract::ConnectInfo("127.0.0.1:42000".parse().unwrap()),
        HeaderMap::new(),
    )
    .await;
    assert!(matches!(result, Err(AppError::AppConnectLinkNotFound)));
}

#[tokio::test]
async fn app_connect_authorize_db_denial_and_expiry_keep_original_callback_and_state() {
    let Some(f) = fixture("gate_deny").await else {
        return;
    };
    gated(&f, false, true).await;
    let (link, mut form) = ready(&f, &params(&f)).await;
    form.decision = "deny".into();
    let denied = decide(&f, form).await.unwrap();
    let location = location(&denied);
    assert!(location.starts_with("https://app.example/callback?"));
    assert!(location.contains("error=access_denied"));
    assert!(location.contains("state=original-state"));
    assert!(location.contains(&format!("app_connect_link_id={}", link.id)));
    assert_eq!(count(&f, CODES).await, 0);
    let expired = session(&f, &params(&f)).await;
    f.state.db.collection::<Document>(LINKS).update_one(doc! { "_id": &expired.id },doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1)) } }).await.unwrap();
    app_links::expire_sessions(&f.state.db).await.unwrap();
    let expired = app_links::load(&f.state, &expired.id, &expired.user_id)
        .await
        .unwrap();
    assert_eq!(expired.status, AppConnectStatus::Expired);
    let callback = app_links::terminal_callback_url(&expired).unwrap().unwrap();
    assert!(callback.contains("error=invalid_request"));
    assert!(callback.contains("nyx_connect_status=expired"));
    assert!(
        app_links::ready(&f.state, &expired.id, &expired.user_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_connect_authorize_db_unsatisfiable_and_try_later_are_distinct_terminal_results() {
    let Some(f) = fixture("gate_terminal").await else {
        return;
    };
    gated(&f, true, true).await;
    let first = session(&f, &params(&f)).await;
    assert!(matches!(
        app_links::try_later(&f.state, &first.id, &first.user_id).await,
        Err(AppError::RequirementNotMet)
    ));
    // The four-second probe budget ended without a provider observation.
    f.state.db.collection::<Document>(LINKS).update_one(doc! { "_id": &first.id },doc! { "$set": { "items.0.state": "unknown", "items.0.reason_code": "validation_unavailable" } }).await.unwrap();
    let unavailable = app_links::try_later(&f.state, &first.id, &first.user_id)
        .await
        .unwrap();
    let callback = app_links::terminal_callback_url(&unavailable)
        .unwrap()
        .unwrap();
    assert!(callback.contains("error=temporarily_unavailable"));
    assert!(callback.contains("nyx_connect_status=unavailable"));
    let second = session(&f, &params(&f)).await;
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_many(doc! {}, doc! { "$set": { "is_active": false } })
        .await
        .unwrap();
    let failed = app_links::ready(&f.state, &second.id, &second.user_id)
        .await
        .unwrap();
    assert_eq!(failed.status, AppConnectStatus::Failed);
    let callback = app_links::terminal_callback_url(&failed).unwrap().unwrap();
    assert!(callback.contains("error=access_denied"));
    assert!(callback.contains("nyx_connect_status=failed"));
    assert!(callback.contains("nyx_connect_reason=requirement_unsatisfiable"));
}

#[tokio::test]
async fn app_connect_authorize_db_result_binding_rejects_other_user_client_version_and_result() {
    let Some(f) = fixture("gate_bindings").await else {
        return;
    };
    gated(&f, false, true).await;
    let (link, _) = ready(&f, &params(&f)).await;
    let p = params_from_session(&link).unwrap();
    let binding = p.app_connect.unwrap();
    let client = app_links::enabled_client(&f.state, &f.app.id)
        .await
        .unwrap();
    assert!(
        app_gate::bound_session(
            &f.state,
            &uuid::Uuid::new_v4().to_string(),
            &client,
            &binding
        )
        .await
        .is_err()
    );
    let mut wrong_client = client.clone();
    wrong_client.id = uuid::Uuid::new_v4().to_string();
    assert!(
        app_gate::bound_session(&f.state, &link.user_id, &wrong_client, &binding)
            .await
            .is_err()
    );
    let mut wrong_result = binding.clone();
    wrong_result.result_id = uuid::Uuid::new_v4().to_string();
    assert!(
        app_gate::bound_session(&f.state, &link.user_id, &client, &wrong_result)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Document>("app_requirement_results")
        .update_one(
            doc! { "_id": &binding.result_id },
            doc! { "$inc": { "manifest_version": 1 } },
        )
        .await
        .unwrap();
    assert!(
        app_gate::bound_session(&f.state, &link.user_id, &client, &binding)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_connect_authorize_db_authority_drift_and_rollout_disable_refuse_bound_consent() {
    let Some(f) = fixture("gate_authority").await else {
        return;
    };
    let id = gated(&f, true, true).await.unwrap();
    let (_, form) = ready(&f, &params(&f)).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "custom_user_agent": "changed-authority" } },
        )
        .await
        .unwrap();
    assert!(decide(&f, form).await.is_err());
    assert_eq!(count(&f, CODES).await, 0);
    let mut policy = f.state.app_connect_policy();
    policy.revision += 1;
    policy.rollout = crate::models::platform_settings::AppConnectRollout::Disabled;
    f.state.set_app_connect_policy_if_fresh(policy);
    let link = f
        .state
        .db
        .collection::<AppConnectLink>(LINKS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let binding = params_from_session(&link).unwrap().app_connect.unwrap();
    assert!(
        app_gate::bound_session(&f.state, &link.user_id, &f.app, &binding)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_connect_authorize_db_non_gate_and_rollout_off_keep_existing_silent_response() {
    let Some(f) = fixture("gate_regression").await else {
        return;
    };
    let p = params(&f);
    consent_service::grant_consent_with_services(
        &f.state.db,
        &f.auth.user_id.to_string(),
        &f.app.id,
        "openid proxy",
        Some(vec![]),
    )
    .await
    .unwrap();
    // Assert the existing byte-level response construction, allowing only the random code to vary.
    async fn unchanged(f: &Fixture, p: &AuthorizeQuery) {
        let response = authorize_response(f, p).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        let url = url::Url::parse(&location(&response)).unwrap();
        let code = url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(location(&response), build_callback_url(p, &code));
        assert!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(count(f, LINKS).await, 0);
    }
    unchanged(&f, &p).await;
    let cat = catalog(&f, "api-github", "bearer").await;
    let _ = cat;
    super::super::app_requirements::tests::publish(&f, requirement("api-github")).await;
    unchanged(&f, &p).await;
    f.state
        .db
        .collection::<Document>("app_requirement_manifests")
        .update_many(doc! {}, doc! { "$set": { "enforcement": "gate" } })
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(
            doc! { "_id": &f.app.id },
            doc! { "$set": { "app_connect_capability_enabled": false } },
        )
        .await
        .unwrap();
    unchanged(&f, &p).await;
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(
            doc! { "_id": &f.app.id },
            doc! { "$set": { "app_connect_capability_enabled": true } },
        )
        .await
        .unwrap();
    let mut policy = f.state.app_connect_policy();
    policy.revision += 1;
    policy.rollout = crate::models::platform_settings::AppConnectRollout::Disabled;
    f.state.set_app_connect_policy_if_fresh(policy);
    unchanged(&f, &p).await;
}

#[tokio::test]
async fn app_connect_authorize_db_token_scope_must_not_rebind_a_granted_org_slug() {
    let Some(f) = fixture("gate_slug_boundary").await else {
        return;
    };
    let catalog_id = catalog(&f, "api-github", "bearer").await;
    let (selected_org, _) = service(&f, &f.owner, &catalog_id, "shared-github").await;
    let (personal, _) = service(
        &f,
        &f.auth.user_id.to_string(),
        &catalog_id,
        "shared-github",
    )
    .await;
    let resource =
        oauth_resource_service::user_service_resource_uri(&f.state.config, &selected_org.slug);
    // Both services are grantable, but only the org connection was selected and consented.
    assert!(
        oauth_resource_service::validate_grantable_service_ids(
            &f.state.db,
            &f.auth.user_id.to_string(),
            std::slice::from_ref(&selected_org.id)
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        oauth_resource_service::resolve_token_resource_scope(
            &f.state.db,
            &f.state.config,
            &f.auth.user_id.to_string(),
            Some(std::slice::from_ref(&resource)),
            &[],
            std::slice::from_ref(&selected_org.id),
            false,
        )
        .await,
        Err(AppError::InvalidTarget(_))
    ));
    let scope = oauth_resource_service::resolve_token_resource_scope(
        &f.state.db,
        &f.state.config,
        &f.auth.user_id.to_string(),
        None,
        &[resource],
        std::slice::from_ref(&selected_org.id),
        false,
    )
    .await;
    match scope {
        Ok(scope) => {
            assert!(
                !scope.allowed_service_ids.contains(&personal.id),
                "token exchange granted the unselected personal connection instead of the consented org connection"
            );
            assert_eq!(scope.allowed_service_ids, vec![selected_org.id]);
        }
        Err(AppError::InvalidTarget(_)) => {} // Refusal also preserves the grant boundary.
        Err(error) => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn app_connect_authorize_db_stored_ids_derive_resources_and_requested_resource_only_narrows()
{
    let Some(f) = fixture("gate_id_resources").await else {
        return;
    };
    let cat = catalog(&f, "api-github", "bearer").await;
    let (a, _) = service(&f, &f.auth.user_id.to_string(), &cat, "first-github").await;
    let (b, _) = service(&f, &f.auth.user_id.to_string(), &cat, "second-github").await;
    let (foreign, _) = service(&f, &f.auth.user_id.to_string(), &cat, "ungranted-github").await;
    let a_uri = oauth_resource_service::user_service_resource_uri(&f.state.config, &a.slug);
    let b_uri = oauth_resource_service::user_service_resource_uri(&f.state.config, &b.slug);
    let foreign_uri =
        oauth_resource_service::user_service_resource_uri(&f.state.config, &foreign.slug);
    let ids = vec![a.id.clone(), b.id.clone()];
    let scope = oauth_resource_service::resolve_token_resource_scope(
        &f.state.db,
        &f.state.config,
        &f.auth.user_id.to_string(),
        None,
        std::slice::from_ref(&foreign_uri),
        &ids,
        false,
    )
    .await
    .unwrap();
    assert_eq!(scope.allowed_service_ids, ids);
    assert_eq!(scope.resource_uris, vec![a_uri.clone(), b_uri.clone()]);
    let narrowed = oauth_resource_service::resolve_token_resource_scope(
        &f.state.db,
        &f.state.config,
        &f.auth.user_id.to_string(),
        Some(std::slice::from_ref(&b_uri)),
        &[],
        &ids,
        false,
    )
    .await
    .unwrap();
    assert_eq!(narrowed.allowed_service_ids, vec![b.id]);
    assert_eq!(narrowed.resource_uris, vec![b_uri]);
    assert!(matches!(
        oauth_resource_service::resolve_token_resource_scope(
            &f.state.db,
            &f.state.config,
            &f.auth.user_id.to_string(),
            Some(&[foreign_uri]),
            &[a_uri],
            &ids,
            false
        )
        .await,
        Err(AppError::InvalidTarget(_))
    ));
}

#[tokio::test]
async fn app_connect_authorize_db_refresh_never_substitutes_or_expands_stored_ids() {
    let Some(f) = fixture("gate_refresh_boundary").await else {
        return;
    };
    let id = gated(&f, false, true).await.unwrap();
    let (_, form) = ready(&f, &params(&f)).await;
    let response = decide(&f, form).await.unwrap();
    let uri = url::Url::parse(&location(&response)).unwrap();
    let code = uri
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let tokens = oauth_service::exchange_authorization_code(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        false,
        &code,
        &f.app.id,
        "https://app.example/callback",
        Some(VERIFIER),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let refreshed = token_service::refresh_tokens(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &tokens.refresh_token,
        None,
        None,
    )
    .await
    .unwrap();
    let claims = crate::crypto::jwt::verify_token(
        &f.state.jwt_keys,
        &f.state.config,
        &refreshed.access_token,
    )
    .unwrap();
    assert_eq!(claims.allowed_service_ids, Some(vec![id.clone()]));
    let service_record = f
        .state
        .db
        .collection::<UserService>("user_services")
        .find_one(doc! { "_id": &id })
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! { "_id": &id }, doc! { "$set": { "is_active": false } })
        .await
        .unwrap();
    service(
        &f,
        &f.auth.user_id.to_string(),
        service_record.catalog_service_id.as_ref().unwrap(),
        &service_record.slug,
    )
    .await;
    let kept = token_service::refresh_tokens(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &refreshed.refresh_token,
        None,
        None,
    )
    .await
    .unwrap();
    let claims =
        crate::crypto::jwt::verify_token(&f.state.jwt_keys, &f.state.config, &kept.access_token)
            .unwrap();
    assert_eq!(claims.allowed_service_ids, Some(vec![id]));
}

#[tokio::test]
async fn app_connect_authorize_db_shadowed_selection_is_unmet_on_status_and_both_ready_origins() {
    use crate::models::app_connect_link::ItemState;
    use crate::models::app_requirement_manifest::OwnerPolicy;
    let Some(f) = fixture("gate_shadowed_ui").await else {
        return;
    };
    let cat = catalog(&f, "api-github", "bearer").await;
    let (org, _) = service(&f, &f.owner, &cat, "same-name").await;
    service(&f, &f.auth.user_id.to_string(), &cat, "same-name").await;
    let mut r = requirement("api-github");
    r.owner_policy = OwnerPolicy::PersonalOrOrgAllowed;
    manifests::publish(
        &f.state,
        &f.app,
        &f.auth.user_id.to_string(),
        manifests::PublishManifest {
            enforcement: Enforcement::Gate,
            requirements: vec![r],
        },
    )
    .await
    .unwrap();
    for authorize_origin in [false, true] {
        let link = if authorize_origin {
            session(&f, &params(&f)).await
        } else {
            let created = app_links::start_from_app(
                &f.state,
                &f.app.id,
                &f.auth.user_id.to_string(),
                "https://app.example/callback",
                "correlation",
            )
            .await
            .unwrap();
            let url = url::Url::parse(&created.connect_url).unwrap();
            app_links::redeem(
                &f.state,
                &created.link.id,
                &created.link.user_id,
                url.fragment().unwrap().strip_prefix("t=").unwrap(),
            )
            .await
            .unwrap()
        };
        app_links::select_item(&f.state, &link.id, &link.user_id, "required", Some(&org.id))
            .await
            .unwrap();
        let (live, report) = app_links::refresh(&f.state, &link.id, &link.user_id)
            .await
            .unwrap();
        assert_eq!(
            report.requirements[0].state,
            app_requirements_service::RequirementState::Unsatisfiable
        );
        assert_eq!(report.requirements[0].reason_code, Some("slug_shadowed"));
        assert_eq!(live.items[0].state, ItemState::Unmet);
        assert_eq!(live.items[0].reason_code.as_deref(), Some("slug_shadowed"));
        assert!(matches!(
            app_links::ready(&f.state, &link.id, &link.user_id).await,
            Err(AppError::RequirementNotMet)
        ));
        let status = super::super::app_requirements::status(State(f.state.clone()), f.auth.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(status.requirements[0].state, "unsatisfiable");
        assert_eq!(status.requirements[0].reason_code, Some("slug_shadowed"));
        assert_eq!(
            status.requirements[0].user_service_id.as_deref(),
            Some(org.id.as_str())
        );
    }
}

#[tokio::test]
async fn app_connect_authorize_db_delegation_resources_only_narrow_stored_ids() {
    use crate::crypto::{jwt, token::hash_token};
    use crate::services::{catalog_delegation_service, token_exchange_service};
    let Some(f) = fixture("gate_delegation_ids").await else {
        return;
    };
    let catalog_id = catalog(&f, "api-github", "bearer").await;
    let actor = f.auth.user_id.to_string();
    let (a, _) = service(&f, &actor, &catalog_id, "first").await;
    let (b, _) = service(&f, &actor, &catalog_id, "second").await;
    let (foreign, _) = service(&f, &actor, &catalog_id, "foreign").await;
    let ids = vec![a.id.clone(), b.id.clone()];
    let uri = oauth_resource_service::user_service_resource_uri(&f.state.config, &b.slug);
    let foreign_uri =
        oauth_resource_service::user_service_resource_uri(&f.state.config, &foreign.slug);
    let catalog_scope = catalog_delegation_service::MCP_CATALOG_READ_SCOPE;
    f.state.db.collection::<Document>("oauth_clients").update_one(
        doc! { "_id": &f.app.id },
        doc! { "$set": { "client_type": "confidential", "client_secret_hash": hash_token("fixture-secret"),
            "delegation_scopes": format!("llm:proxy {catalog_scope}") } },
    ).await.unwrap();
    consent_service::grant_consent_with_services(
        &f.state.db,
        &actor,
        &f.app.id,
        "openid",
        Some(ids.clone()),
    )
    .await
    .unwrap();
    let source = jwt::generate_oauth_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &f.auth.user_id,
        "openid",
        None,
        None,
        None,
        None,
        Some(jwt::AccessTokenRestrictions {
            resources: std::slice::from_ref(&foreign_uri),
            allowed_service_ids: &ids,
            allowed_node_ids: &[],
            allow_all_nodes: true,
        }),
        &f.app.id,
    )
    .unwrap();
    for scope in ["llm:proxy", catalog_scope] {
        let exchanged = token_exchange_service::exchange_token_with_authority(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &f.app.id,
            "fixture-secret",
            &source,
            "urn:ietf:params:oauth:token-type:access_token",
            Some(scope),
            std::slice::from_ref(&uri),
            Some(false),
            &ids,
            Some(true),
            &[],
        )
        .await
        .unwrap();
        let claims =
            jwt::verify_token(&f.state.jwt_keys, &f.state.config, &exchanged.access_token).unwrap();
        assert_eq!(claims.allowed_service_ids, Some(vec![b.id.clone()]));
        assert_eq!(claims.resources, Some(vec![uri.clone()]));
        let refreshed = token_exchange_service::refresh_delegation_token(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &actor,
            &f.app.id,
            Some(&f.app.id),
            scope,
            &jwt::TokenRestrictionClaims::from_claims(&claims),
        )
        .await
        .unwrap();
        let refreshed =
            jwt::verify_token(&f.state.jwt_keys, &f.state.config, &refreshed.access_token).unwrap();
        assert_eq!(refreshed.allowed_service_ids, Some(vec![b.id.clone()]));
        assert_eq!(refreshed.resources, Some(vec![uri.clone()]));
        assert!(matches!(
            token_exchange_service::exchange_token_with_authority(
                &f.state.db,
                &f.state.config,
                &f.state.jwt_keys,
                &f.app.id,
                "fixture-secret",
                &source,
                "urn:ietf:params:oauth:token-type:access_token",
                Some(scope),
                std::slice::from_ref(&foreign_uri),
                Some(false),
                &ids,
                Some(true),
                &[],
            )
            .await,
            Err(AppError::InvalidTarget(_))
        ));
    }
}

#[tokio::test]
async fn app_connect_authorize_db_unmet_resource_enters_gate_before_consent_resolution() {
    let Some(f) = fixture("gate_unconnected_resource").await else {
        return;
    };
    gated(&f, false, false).await;
    let mut p = params(&f);
    p.resource = vec![oauth_resource_service::user_service_resource_uri(
        &f.state.config,
        "api-github",
    )];
    p.prompt = Some("none".into());
    let response = authorize_response(&f, &p).await;
    assert!(location(&response).contains("error=interaction_required"));
    assert_eq!(count(&f, LINKS).await, 0);
    p.prompt = None;
    let link = session(&f, &p).await;
    assert_eq!(link.status, AppConnectStatus::InProgress);
    assert_eq!(count(&f, CODES).await, 0);
}

async fn refresh_grant(f: &Fixture, ids: &[String]) -> String {
    oauth_service::issue_oauth_refresh_token(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &f.app.id,
        &f.auth.user_id.to_string(),
        "openid proxy",
        &[],
        ids,
        false,
    )
    .await
    .unwrap()
    .refresh_token
}

#[tokio::test]
async fn app_connect_authorize_db_disabled_grant_refreshes_and_reenables_without_consent() {
    let Some(f) = fixture("gate_disabled_refresh").await else {
        return;
    };
    let cat = catalog(&f, "api-github", "bearer").await;
    let (selected, _) = service(&f, &f.auth.user_id.to_string(), &cat, "paused").await;
    let token = refresh_grant(&f, std::slice::from_ref(&selected.id)).await;
    let before = count(&f, "consents").await;
    let mut token = token;
    for active in [false, true] {
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(
                doc! { "_id": &selected.id },
                doc! { "$set": { "is_active": active } },
            )
            .await
            .unwrap();
        let refreshed = token_service::refresh_tokens(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &token,
            None,
            None,
        )
        .await
        .unwrap();
        let claims = crate::crypto::jwt::verify_token(
            &f.state.jwt_keys,
            &f.state.config,
            &refreshed.access_token,
        )
        .unwrap();
        assert_eq!(claims.allowed_service_ids, Some(vec![selected.id.clone()]));
        assert_eq!(
            claims.resources,
            Some(vec![oauth_resource_service::user_service_resource_uri(
                &f.state.config,
                &selected.slug
            )])
        );
        assert_eq!(count(&f, "consents").await, before);
        token = refreshed.refresh_token;
    }
}

#[tokio::test]
async fn app_connect_authorize_db_revoked_org_and_missing_ids_only_narrow_refresh() {
    let Some(f) = fixture("gate_revoked_org_refresh").await else {
        return;
    };
    let cat = catalog(&f, "api-github", "bearer").await;
    let (personal, _) = service(&f, &f.auth.user_id.to_string(), &cat, "personal").await;
    let (org, _) = service(&f, &f.owner, &cat, "org").await;
    let token = refresh_grant(
        &f,
        &[
            personal.id.clone(),
            org.id.clone(),
            uuid::Uuid::new_v4().to_string(),
        ],
    )
    .await;
    f.state
        .db
        .collection::<Document>("org_memberships")
        .delete_many(doc! {})
        .await
        .unwrap();
    let refreshed = token_service::refresh_tokens(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &token,
        None,
        None,
    )
    .await
    .unwrap();
    let claims = crate::crypto::jwt::verify_token(
        &f.state.jwt_keys,
        &f.state.config,
        &refreshed.access_token,
    )
    .unwrap();
    assert_eq!(claims.allowed_service_ids, Some(vec![personal.id]));
    assert_eq!(
        claims.resources,
        Some(vec![oauth_resource_service::user_service_resource_uri(
            &f.state.config,
            &personal.slug
        )])
    );
}

#[tokio::test]
async fn app_connect_authorize_db_tombstone_slug_reuse_preserves_only_granted_id() {
    let Some(f) = fixture("gate_tombstone_refresh").await else {
        return;
    };
    let cat = catalog(&f, "api-github", "bearer").await;
    let (old, key) = service(&f, &f.auth.user_id.to_string(), &cat, "reused").await;
    let token = refresh_grant(&f, std::slice::from_ref(&old.id)).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": &old.id },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .delete_one(doc! { "_id": key.id })
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .delete_one(doc! { "_id": &old.endpoint_id })
        .await
        .unwrap();
    let (replacement, _) = service(&f, &f.auth.user_id.to_string(), &cat, "reused").await;
    let uri = oauth_resource_service::user_service_resource_uri(&f.state.config, &old.slug);
    for requested in [None, Some(std::slice::from_ref(&uri))] {
        let refreshed = token_service::refresh_tokens(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &token,
            None,
            requested,
        )
        .await
        .unwrap();
        let claims = crate::crypto::jwt::verify_token(
            &f.state.jwt_keys,
            &f.state.config,
            &refreshed.access_token,
        )
        .unwrap();
        assert_eq!(claims.allowed_service_ids, Some(vec![old.id.clone()]));
        assert!(
            !claims
                .allowed_service_ids
                .unwrap()
                .contains(&replacement.id)
        );
    }
}

#[tokio::test]
async fn app_connect_authorize_db_api_mode_returns_checklist_consent_handoff() {
    let Some(f) = fixture("gate_api_handoff").await else {
        return;
    };
    gated(&f, false, false).await;
    for method in [AuthMethod::Session, AuthMethod::AccessToken] {
        let mut auth = human(&f);
        auth.auth_method = method;
        let result = authorize_inner(
            &f.state,
            OptionalAuthUser(Some(auth)),
            &params(&f),
            false,
            None,
        )
        .await;
        let Err(AppError::ConsentRequired { consent_url }) = result else {
            panic!("expected consent_required checklist handoff");
        };
        let url = url::Url::parse(&consent_url).unwrap();
        assert!(url.path().starts_with("/connect/app/"));
        let link_id = url.path_segments().unwrap().next_back().unwrap();
        let link = app_links::load(&f.state, link_id, &f.auth.user_id.to_string())
            .await
            .unwrap();
        assert!(link.redeemed_at.is_none());
        assert_eq!(link.status, AppConnectStatus::InProgress);
        let mut app_token = f.auth.clone();
        app_token.auth_method = AuthMethod::AccessToken;
        assert!(
            super::super::app_connect_links::require_human(&f.state, &app_token)
                .await
                .is_err()
        );
        app_links::redeem(
            &f.state,
            link_id,
            &link.user_id,
            url.fragment().unwrap().strip_prefix("t=").unwrap(),
        )
        .await
        .unwrap();
    }
    assert_eq!(count(&f, CODES).await, 0);
    let before = count(&f, LINKS).await;
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
    ] {
        let mut auth = human(&f);
        auth.auth_method = method;
        assert!(matches!(
            authorize_inner(
                &f.state,
                OptionalAuthUser(Some(auth)),
                &params(&f),
                false,
                None
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    assert_eq!(count(&f, LINKS).await, before);
}
