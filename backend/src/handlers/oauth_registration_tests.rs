use super::*;
use crate::models::oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient, ScopeProvenance};
use crate::test_utils::{connect_test_database, test_app_state};
use axum::body::{Body, to_bytes};
use axum::http::Request;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn registration(
    state: &AppState,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, Value) {
    post_oauth(state, "/oauth/register", body, content_type).await
}

async fn post_oauth(
    state: &AppState,
    path: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, Value) {
    let (public, _) = crate::routes::build_router_with_state(state.clone());
    let mut request = Request::builder().method("POST").uri(path);
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let response = public
        .with_state(state.clone())
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("non-JSON registration response: status {status}"));
    (status, body)
}

#[tokio::test]
async fn register_client_http_normalizes_advertised_auth_methods() {
    let db = connect_test_database("dcr_http_auth")
        .await
        .expect("regression requires MongoDB");
    let state = test_app_state(db.clone());
    let mut bodies = ["client_secret_post", "client_secret_basic", "none"]
        .map(|method| {
            json!({
                "token_endpoint_auth_method": method,
                "redirect_uris": ["https://app.example/callback"],
            })
            .to_string()
        })
        .to_vec();
    // Verbatim registration probe bodies A and B from issue #1759.
    bodies.extend([
        r#"{"client_name":"Claude","redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"client_secret_post"}"#.to_owned(),
        r#"{"client_name":"Claude","redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none","scope":"claudeai"}"#.to_owned(),
    ]);
    for body in bodies {
        let (status, response) = registration(&state, &body, Some("application/json")).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(response["token_endpoint_auth_method"], "none");
        assert!(response.get("client_secret").is_none());
        assert_eq!(
            response["scope"],
            oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES
        );
        assert!(
            response["scope"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .any(|s| s == "proxy")
        );
        let client = db
            .collection::<OauthClient>(CLIENTS)
            .find_one(doc! {"_id": response["client_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(client.client_type, "public");
        assert_eq!(client.client_secret_hash, "NONE");
        assert_eq!(
            client.allowed_scopes,
            oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES
        );
        assert_eq!(client.scope_provenance, ScopeProvenance::Defaulted);
    }
}

#[tokio::test]
async fn register_client_http_filters_unknown_scopes_without_widening_known_scopes() {
    let db = connect_test_database("dcr_http_scope")
        .await
        .expect("regression requires MongoDB");
    let state = test_app_state(db.clone());
    for (requested, effective, provenance) in [
        (
            "claudeai",
            oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES,
            ScopeProvenance::Defaulted,
        ),
        (
            "openid email claudeai",
            "openid email",
            ScopeProvenance::Explicit,
        ),
    ] {
        let (status, response) = registration(
            &state,
            &json!({"scope": requested}).to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(response["scope"], effective);
        let client = db
            .collection::<OauthClient>(CLIENTS)
            .find_one(doc! {"_id": response["client_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(client.scope_provenance, provenance);
        assert_eq!(client.allowed_scopes, effective);
    }
}

#[tokio::test]
async fn register_client_http_malformed_json_has_rfc7591_error() {
    let db = connect_test_database("dcr_http_json")
        .await
        .expect("regression requires MongoDB");
    let state = test_app_state(db);
    let (status, response) = registration(&state, "{", Some("application/json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["error"], "invalid_client_metadata");
    assert!(response["error_description"].is_string());
}

#[tokio::test]
async fn register_client_http_rejects_invalid_metadata_without_creating_rows() {
    let db = connect_test_database("dcr_http_errors")
        .await
        .expect("MongoDB required");
    let state = test_app_state(db.clone());
    let cases = [
        (
            json!({"client_name": "n".repeat(257)}),
            "invalid_client_metadata",
        ),
        (
            json!({"token_endpoint_auth_method": "private_key_jwt"}),
            "invalid_client_metadata",
        ),
        (
            json!({"redirect_uris": "https://example.com"}),
            "invalid_client_metadata",
        ),
        (
            json!({"redirect_uris": ["javascript:alert(1)"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": ["https://example.com/cb#secret"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": ["not a uri"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": vec!["https://example.com/cb"; 17]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": [format!("https://example.com/{}", "x".repeat(2048))]}),
            "invalid_redirect_uri",
        ),
        (json!({"scope": "x ".repeat(65)}), "invalid_client_metadata"),
        (json!({"scope": "x".repeat(257)}), "invalid_client_metadata"),
    ];
    for (body, code) in cases {
        let (status, response) =
            registration(&state, &body.to_string(), Some("application/json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"], code);
        assert!(response["error_description"].is_string());
        assert_eq!(response.as_object().unwrap().len(), 2);
        assert!(!response.to_string().contains("secret"));
    }
    let (status, response) = registration(&state, "{}", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["error"], "invalid_client_metadata");
    let (status, response) = registration(
        &state,
        &json!({"client_name": "x".repeat(2 * 1024 * 1024)}).to_string(),
        Some("application/json"),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response["error"], "invalid_client_metadata");
    assert_eq!(
        db.collection::<OauthClient>(CLIENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

use crate::crypto::{jwt, token::hash_token};
use crate::models::authorization_code::{AuthorizationCode, COLLECTION_NAME as CODES};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH, RefreshToken};
use crate::models::user::UserType;
use crate::test_utils::{test_auth_user, test_user, test_user_service};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn redirect_parameter(response: &Response, key: &str) -> String {
    assert_eq!(response.status(), StatusCode::FOUND);
    url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap())
        .unwrap()
        .query_pairs()
        .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
        .unwrap()
}

async fn token_http(
    state: &AppState,
    params: &[(&str, &str)],
    resources: &[String],
) -> (StatusCode, Value) {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.extend_pairs(params.iter().copied());
    for resource in resources {
        form.append_pair("resource", resource);
    }
    post_oauth(
        state,
        "/oauth/token",
        &form.finish(),
        Some("application/x-www-form-urlencoded"),
    )
    .await
}

async fn assert_token_grant(
    state: &AppState,
    response: &Value,
    ids: &[String],
    resources: &[String],
    original_ids: &[String],
    original_resources: &[String],
) {
    let claims = jwt::verify_token(
        &state.jwt_keys,
        &state.config,
        response["access_token"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(claims.allowed_service_ids.as_deref(), Some(ids));
    assert_eq!(claims.allow_all_services, Some(false));
    assert_eq!(claims.resources.as_deref().unwrap_or_default(), resources);
    assert!(claims.scope.split_whitespace().any(|s| s == "proxy"));
    assert!(!claims.scope.contains("broker_binding"));
    crate::handlers::mcp_transport::assert_oauth_service_scope(
        state,
        response["access_token"].as_str().unwrap(),
        ids,
    )
    .await;
    let refresh = jwt::verify_token(
        &state.jwt_keys,
        &state.config,
        response["refresh_token"].as_str().unwrap(),
    )
    .unwrap();
    let stored = state
        .db
        .collection::<RefreshToken>(REFRESH)
        .find_one(doc! {"jti": refresh.jti})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.allowed_service_ids, original_ids);
    assert_eq!(stored.resource_uris, original_resources);
    assert!(!stored.allow_all_services);
    assert!(!stored.revoked);
}

async fn restricted_round_trip(mixed: bool, request_mcp: bool, zero: bool) {
    let db = connect_test_database("dcr_restricted_round_trip")
        .await
        .expect("MongoDB required");
    let state = test_app_state(db.clone());
    let user = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let ids = vec![Uuid::new_v4().to_string(), Uuid::new_v4().to_string()];
    for (id, slug) in ids.iter().zip(["a", "b"]) {
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(id, &user, slug, "endpoint", None, None))
            .await
            .unwrap();
    }
    let mcp = oauth_resource_service::mcp_resource_uri(&state.config);
    let ra = oauth_resource_service::user_service_resource_uri(&state.config, "a");
    let rb = oauth_resource_service::user_service_resource_uri(&state.config, "b");
    let resources = if mixed {
        vec![ra.clone(), mcp.clone()]
    } else {
        vec![mcp.clone()]
    };
    let consented = if zero { vec![] } else { ids.clone() };
    let granted = if mixed {
        vec![ids[0].clone()]
    } else {
        consented.clone()
    };
    let (status, registered) = registration(
        &state,
        &json!({
            "client_name": "Round trip", "redirect_uris": ["https://app.example/callback"],
            "token_endpoint_auth_method": "client_secret_post", "scope": "claudeai",
        })
        .to_string(),
        Some("application/json"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let client_id = registered["client_id"].as_str().unwrap();
    let verifier = "nyxid-1759-1760-pkce-regression-verifier-0123456789";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let mut authorization = json!({
        "response_type": "code", "client_id": client_id, "redirect_uri": "https://app.example/callback",
        "scope": "claudeai", "code_challenge": challenge, "code_challenge_method": "S256",
        "state": "round-trip-state", "prompt": "consent", "resource": resources,
    });
    let query: AuthorizeQuery = serde_json::from_value(authorization.clone()).unwrap();
    let response = authorize_inner(
        &state,
        OptionalAuthUser(Some(test_auth_user(&user))),
        &query,
        true,
        None,
    )
    .await
    .unwrap();
    authorization["consent_request"] = json!(redirect_parameter(&response, "consent_request"));
    authorization["decision"] = json!("allow");
    authorization["allow_all_services"] = json!(false);
    authorization["allowed_service_ids"] = json!(consented);
    let response = authorize_decision(
        State(state.clone()),
        OptionalAuthUser(Some(test_auth_user(&user))),
        TelemetryContext::default(),
        Form(serde_json::from_value(authorization).unwrap()),
    )
    .await
    .unwrap();
    let code = redirect_parameter(&response, "code");
    let stored = db
        .collection::<AuthorizationCode>(CODES)
        .find_one(doc! {"code_hash": hash_token(&code)})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.allowed_service_ids, granted);
    assert_eq!(stored.resource_uris, resources);
    assert!(!stored.allow_all_services);
    let requested = if request_mcp {
        vec![mcp.clone()]
    } else {
        vec![]
    };
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", &code),
        ("redirect_uri", "https://app.example/callback"),
        ("code_verifier", verifier),
    ];
    // Public normalization must work with the returned metadata; an extra secret
    // supplied by a client does not turn it into a confidential client.
    if mixed {
        params.push(("client_secret", "irrelevant-public-client-secret"));
    }
    let (status, mut tokens) = token_http(&state, &params, &requested).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        tokens["scope"],
        oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES
    );
    let access_resources = if request_mcp { &requested } else { &resources };
    assert_token_grant(
        &state,
        &tokens,
        &granted,
        access_resources,
        &granted,
        &resources,
    )
    .await;
    // Both refresh forms preserve the original grant; narrowing never compounds.
    for requested in [vec![], vec![format!("{mcp}/")], vec![]] {
        let refresh = tokens["refresh_token"].as_str().unwrap();
        let (status, next) = token_http(
            &state,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh),
                ("scope", "openid proxy urn:nyxid:scope:broker_binding"),
            ],
            &requested,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let effective = if requested.is_empty() {
            resources.clone()
        } else {
            vec![mcp.clone()]
        };
        assert_token_grant(&state, &next, &granted, &effective, &granted, &resources).await;
        tokens = next;
    }
    let refresh = tokens["refresh_token"].as_str().unwrap();
    for expansion in [vec![rb], vec![format!("{mcp}?ungranted=1")]] {
        let (status, error) = token_http(
            &state,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh),
            ],
            &expansion,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["error"], "invalid_target");
        assert_token_grant(&state, &tokens, &granted, &resources, &granted, &resources).await;
    }
    if mixed {
        let services = db.collection::<UserService>(USER_SERVICES);
        let original_service = services
            .find_one(doc! {"_id": &ids[0]})
            .await
            .unwrap()
            .unwrap();
        let refresh_tokens = db.collection::<RefreshToken>(REFRESH);
        for inactive in [false, true] {
            let refresh = tokens["refresh_token"].as_str().unwrap();
            let claims = jwt::verify_token(&state.jwt_keys, &state.config, refresh).unwrap();
            let before = refresh_tokens
                .find_one(doc! {"jti": &claims.jti})
                .await
                .unwrap()
                .unwrap();
            let count_before = refresh_tokens.count_documents(doc! {}).await.unwrap();
            assert!(!before.revoked);
            if inactive {
                services
                    .update_one(doc! {"_id": &ids[0]}, doc! {"$set": {"is_active": false}})
                    .await
                    .unwrap();
            } else {
                services.delete_one(doc! {"_id": &ids[0]}).await.unwrap();
            }
            let (status, error) = token_http(
                &state,
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", client_id),
                    ("refresh_token", refresh),
                ],
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            // No catalog fallback exists for this fixture's missing service.
            assert_eq!(error["error"], "invalid_target");
            let after = refresh_tokens
                .find_one(doc! {"jti": &claims.jti})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(after, before);
            assert_eq!(
                refresh_tokens.count_documents(doc! {}).await.unwrap(),
                count_before
            );

            services
                .replace_one(doc! {"_id": &ids[0]}, &original_service)
                .upsert(true)
                .await
                .unwrap();
            let (status, next) = token_http(
                &state,
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", client_id),
                    ("refresh_token", refresh),
                ],
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_token_grant(&state, &next, &granted, &resources, &granted, &resources).await;
            tokens = next;
        }
        let refresh = tokens["refresh_token"].as_str().unwrap();
        // Reusing a slug cannot give its new ID the authority of the consented ID.
        db.collection::<UserService>(USER_SERVICES)
            .delete_one(doc! {"_id": &ids[0]})
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &Uuid::new_v4().to_string(),
                &user,
                "a",
                "endpoint",
                None,
                None,
            ))
            .await
            .unwrap();
        let (status, next) = token_http(
            &state,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh),
            ],
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_token_grant(&state, &next, &[], &[mcp], &granted, &resources).await;
    }
}

#[tokio::test]
async fn dcr_pkce_mcp_code_and_refresh_preserve_restricted_services() {
    restricted_round_trip(false, true, false).await;
}
#[tokio::test]
async fn dcr_pkce_mcp_code_without_requested_resource_preserves_services() {
    restricted_round_trip(false, false, false).await;
}
#[tokio::test]
async fn dcr_pkce_mixed_resource_refresh_retains_original_grant_and_rejects_slug_reuse() {
    restricted_round_trip(true, true, false).await;
}
#[tokio::test]
async fn dcr_pkce_mcp_code_and_refresh_preserve_explicit_zero_services() {
    restricted_round_trip(false, true, true).await;
}

#[tokio::test]
async fn dcr_scope_validation_is_consistent_for_authorize_par_and_consent() {
    let db = connect_test_database("dcr_scope_consistency")
        .await
        .expect("MongoDB required");
    let state = test_app_state(db.clone());
    let (status, registered) = registration(
        &state,
        &json!({
            "scope": "openid email claudeai", "redirect_uris": ["https://app.example/callback"],
        })
        .to_string(),
        Some("application/json"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let client_id = registered["client_id"].as_str().unwrap();
    for (scope, expected) in [
        ("claudeai", "openid email"),
        ("openid claudeai", "openid"),
        ("openid offline_access claudeai", "invalid_scope"),
    ] {
        let params = json!({"response_type": "code", "client_id": client_id,
            "redirect_uri": "https://app.example/callback", "scope": scope,
            "code_challenge": "challenge", "code_challenge_method": "S256"});
        let query: AuthorizeQuery = serde_json::from_value(params.clone()).unwrap();
        let result = validate_authorize_request(&state, &query).await;
        let mut par_form = params.clone();
        par_form["client_secret"] = json!("public-placeholder");
        let par = pushed_authorization_request(
            State(state.clone()),
            HeaderMap::new(),
            Form(serde_json::from_value(par_form).unwrap()),
        )
        .await;
        if expected == "invalid_scope" {
            assert!(matches!(result, Err(AppError::InvalidScope(_))));
            assert!(matches!(par, Err(AppError::InvalidScope(_))));
        } else {
            assert_eq!(result.unwrap().1, expected);
            let (_, Json(par)) = par.unwrap();
            let pushed = resolve_pushed_authorize_params(
                &state,
                serde_json::from_value(
                    json!({"client_id": client_id, "request_uri": par.request_uri}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(
                validate_authorize_request(&state, &pushed).await.unwrap().1,
                expected
            );
        }
    }
    let user = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let query: AuthorizeQuery =
        serde_json::from_value(json!({"response_type": "code", "client_id": client_id,
        "redirect_uri": "https://app.example/callback", "scope": "email claudeai",
        "code_challenge": "challenge", "code_challenge_method": "S256", "prompt": "consent"}))
        .unwrap();
    let response = authorize_inner(
        &state,
        OptionalAuthUser(Some(test_auth_user(&user))),
        &query,
        true,
        None,
    )
    .await
    .unwrap();
    let signed = redirect_parameter(&response, "consent_request");
    oauth_client_service::admin_update_client(
        &db,
        client_id,
        oauth_client_service::AdminUpdateClient {
            allowed_scopes: Some("openid"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let form = serde_json::from_value(json!({"response_type": "code", "client_id": client_id,
        "redirect_uri": "https://app.example/callback", "decision": "allow", "consent_request": signed})).unwrap();
    let error = authorize_decision(
        State(state.clone()),
        OptionalAuthUser(Some(test_auth_user(&user))),
        TelemetryContext::default(),
        Form(form),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::InvalidScope(_)),
        "consent revalidates current registered scopes"
    );
    // Developer/admin clients remain strict; no shared validator has been relaxed.
    db.collection::<OauthClient>(CLIENTS)
        .update_one(
            doc! {"_id": client_id},
            doc! {"$set": {"created_by": "developer"}},
        )
        .await
        .unwrap();
    let mut strict = query;
    strict.scope = Some("openid claudeai".into());
    assert!(matches!(
        validate_authorize_request(&state, &strict).await,
        Err(AppError::InvalidScope(_))
    ));
    assert!(oauth_client_service::validate_allowed_scopes("openid claudeai").is_err());
    assert!(oauth_client_service::validate_redirect_uris(&[]).is_err());
}

#[tokio::test]
async fn register_client_http_validates_redirects_and_preserves_broker_gate() {
    let db = connect_test_database("dcr_redirect_broker")
        .await
        .expect("MongoDB required");
    let mut config = crate::test_utils::test_app_config();
    config.broker_require_admin_capability = true;
    let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    for scope in [
        "urn:nyxid:scope:broker_binding",
        "claudeai urn:nyxid:scope:broker_binding",
    ] {
        let (status, error) = registration(
            &state,
            &json!({"scope": scope}).to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["error"], "invalid_client_metadata");
    }
    assert_eq!(
        db.collection::<OauthClient>(CLIENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let (status, response) = registration(&state, &json!({"redirect_uris": [
        "  https://app.example/callback  ", "https://app.example/callback", "myapp://callback", "http://127.0.0.1:3210/callback",
    ]}).to_string(), Some("application/json")).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        response["redirect_uris"],
        json!([
            "https://app.example/callback",
            "myapp://callback",
            "http://127.0.0.1:3210/callback"
        ])
    );
    let client = db
        .collection::<OauthClient>(CLIENTS)
        .find_one(doc! {"_id": response["client_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(json!(client.redirect_uris), response["redirect_uris"]);
    assert!(!client.broker_capability_enabled);
    assert!(client.delegation_scopes.is_empty());
}

#[tokio::test]
async fn register_client_logs_are_bounded_sanitized_outcomes() {
    use tracing::instrument::WithSubscriber;
    const TEST_NAME: &str =
        "handlers::oauth::registration_tests::register_client_logs_are_bounded_sanitized_outcomes";
    const CHILD_ENV: &str = "NYXID_TEST_DCR_TRACE_CHILD";
    if std::env::var(CHILD_ENV).as_deref() != Ok(TEST_NAME) {
        // As in auth_agent_key's privacy tests, isolate tracing-core's global
        // callsite cache from first use on other, unsubscribed libtest threads.
        // Database selection and process-specific coverage settings are inherited.
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(180),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture", "--color", "never"])
                .env(CHILD_ENV, TEST_NAME)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("isolated DCR tracing test timed out")
        .expect("start tracing test");
        // A failed privacy test may have captured fixture secrets. Report only
        // status, and require exactly one test so a bad filter cannot pass.
        assert!(
            output.status.success(),
            "isolated DCR tracing assertions failed: {}",
            output.status
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("test result: ok. 1 passed; 0 failed;"),
            "exactly one tracing test must run"
        );
        return;
    }
    let db = connect_test_database("dcr_logs")
        .await
        .expect("MongoDB required");
    let state = test_app_state(db);
    let capture = tempfile::NamedTempFile::new().unwrap();
    let writer = capture.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.try_clone().unwrap())
        .finish();
    async {
        let (status, _) = registration(&state, &json!({
            "client_name": format!("name\u{001b}\r\n{}", "n".repeat(160)),
            "redirect_uris": ["https://userinfo:password@evil.example:8443/private-path?code=query-secret", "myapp://private/path?secret=custom-secret"],
            "scope": (0..20).map(|i| format!("unknown{i}/{}", "x".repeat(80))).collect::<Vec<_>>().join(" "),
        }).to_string(), Some("application/json")).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = registration(&state, &json!({
            "client_name": "Rejected", "token_endpoint_auth_method": "auth-secret",
            "redirect_uris": vec!["https://userinfo:password@evil.example/private-path?code=query-secret#fragment-secret"; 20],
            "grant_types": vec!["grant-secret"; 20], "response_types": vec!["response-secret"; 20],
        }).to_string(), Some("application/json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = registration(&state, "{\"raw-secret\":", Some("application/json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }.with_subscriber(subscriber).await;
    let logs = std::fs::read_to_string(capture.path()).unwrap();
    for secret in [
        "userinfo",
        "password",
        "private-path",
        "query-secret",
        "custom-secret",
        "fragment-secret",
        "auth-secret",
        "grant-secret",
        "response-secret",
        "raw-secret",
        "\\u001b",
    ] {
        assert!(
            !logs.contains(secret),
            "registration logs leaked sensitive data"
        );
    }
    let events: Vec<Value> = logs
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|event| event["target"] == "nyxid::oauth::dcr")
        .collect();
    assert_eq!(events.len(), 3, "one outcome event per request");
    let accepted = &events[0]["fields"];
    assert_eq!(accepted["outcome"], "registered");
    assert_eq!(accepted["scope_provenance"], "defaulted");
    assert_eq!(accepted["dropped_unknown_scope_count"], 20);
    let name = accepted["client_name"].as_str().unwrap();
    assert_eq!(name.len(), 128);
    assert!(name.starts_with("name???"));
    let sample: Vec<String> =
        serde_json::from_str(accepted["dropped_unknown_scope_sample"].as_str().unwrap()).unwrap();
    assert_eq!(sample.len(), 8);
    assert!(sample.iter().all(|s| s.len() <= 32 && !s.contains('/')));
    let hosts: Vec<String> =
        serde_json::from_str(accepted["redirect_hosts"].as_str().unwrap()).unwrap();
    assert_eq!(hosts, ["https://evil.example:8443", "myapp:"]);
    let rejected = &events[1]["fields"];
    assert_eq!(rejected["outcome"], "rejected");
    for field in ["redirect_hosts", "grant_types", "response_types"] {
        let values: Vec<String> = serde_json::from_str(rejected[field].as_str().unwrap()).unwrap();
        assert_eq!(values.len(), 8);
        assert!(values.iter().all(|v| v.len() <= 128));
    }
    assert_eq!(events[2]["fields"]["reason"], "invalid_json");
    assert!(events.iter().all(|event| event.to_string().len() < 2500));
}

#[tokio::test]
async fn registration_internal_errors_do_not_expose_details() {
    let response =
        DcrError::from(AppError::Internal("database credential secret".into())).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"error": "server_error", "error_description": "An internal error occurred"})
    );
}

#[tokio::test]
async fn pkce_code_exchange_rejects_resource_expansion_and_slug_rebinding_authority() {
    let db = connect_test_database("dcr_code_resource_fences")
        .await
        .expect("MongoDB required");
    let state = test_app_state(db.clone());
    let user = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let original_id = Uuid::new_v4().to_string();
    let service = test_user_service(&original_id, &user, "reused", "endpoint", None, None);
    db.collection::<UserService>(USER_SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    let (_, registered) = registration(
        &state,
        &json!({"redirect_uris": ["https://app.example/callback"]}).to_string(),
        Some("application/json"),
    )
    .await;
    let client_id = registered["client_id"].as_str().unwrap();
    let verifier = "negative-pkce-code-exchange-verifier-0123456789012345";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let mcp = oauth_resource_service::mcp_resource_uri(&state.config);
    let resource = oauth_resource_service::user_service_resource_uri(&state.config, "reused");
    let original_resources = vec![resource, mcp.clone()];
    for rebind in [false, true] {
        let authorization = json!({"response_type": "code", "client_id": client_id,
            "redirect_uri": "https://app.example/callback", "scope": "openid proxy offline_access",
            "code_challenge": challenge, "code_challenge_method": "S256", "resource": original_resources, "prompt": "consent"});
        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(test_auth_user(&user))),
            &serde_json::from_value(authorization.clone()).unwrap(),
            true,
            None,
        )
        .await
        .unwrap();
        let mut form = authorization;
        form["consent_request"] = json!(redirect_parameter(&response, "consent_request"));
        form["allowed_service_ids"] = json!([original_id]);
        form["allow_all_services"] = json!(false);
        form["decision"] = json!("allow");
        let response = authorize_decision(
            State(state.clone()),
            OptionalAuthUser(Some(test_auth_user(&user))),
            TelemetryContext::default(),
            Form(serde_json::from_value(form).unwrap()),
        )
        .await
        .unwrap();
        let code = redirect_parameter(&response, "code");
        let requested = if rebind {
            db.collection::<UserService>(USER_SERVICES)
                .delete_one(doc! {"_id": &original_id})
                .await
                .unwrap();
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(test_user_service(
                    &Uuid::new_v4().to_string(),
                    &user,
                    "reused",
                    "endpoint",
                    None,
                    None,
                ))
                .await
                .unwrap();
            vec![]
        } else {
            vec![format!("{mcp}?expanded=1")]
        };
        let (status, response) = token_http(
            &state,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("code", &code),
                ("code_verifier", verifier),
                ("redirect_uri", "https://app.example/callback"),
            ],
            &requested,
        )
        .await;
        if rebind {
            assert_eq!(status, StatusCode::OK);
            assert_token_grant(
                &state,
                &response,
                &[],
                std::slice::from_ref(&mcp),
                std::slice::from_ref(&original_id),
                &original_resources,
            )
            .await;
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(response["error"], "invalid_target");
            let stored = db
                .collection::<AuthorizationCode>(CODES)
                .find_one(doc! {"code_hash": hash_token(&code)})
                .await
                .unwrap()
                .unwrap();
            assert!(
                stored.used,
                "existing code-claim semantics consume a rejected exchange"
            );
            assert_eq!(
                db.collection::<RefreshToken>(REFRESH)
                    .count_documents(doc! {"client_id": client_id})
                    .await
                    .unwrap(),
                0
            );
        }
    }
}
