use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn denial_guidance_preserves_all_early_layer_messages_and_admission() {
    let app = Router::new()
        .fallback(|| async { StatusCode::NO_CONTENT })
        .layer(middleware::from_fn(reject_oauth_client_tokens))
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_api_key_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));
    // Deliberately unverified: these layers remain deny-only peeks. Signature
    // validation is still the AuthUser extractor's job on permitted surfaces.
    for (claims, kind, message) in [
        (
            json!({"relay":true}),
            "relay",
            "Relay tokens cannot access this endpoint",
        ),
        (
            json!({"sa":true}),
            "service_account",
            "Service accounts cannot access this endpoint",
        ),
        (
            json!({"delegated":true}),
            "delegated",
            DELEGATED_ENDPOINT_FORBIDDEN,
        ),
        (
            json!({"client_id":"private-client-id"}),
            "oauth_client",
            "A first-party human account session is required",
        ),
    ] {
        let token = format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/assistant/nyxagent/private-id")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response).await;
        assert_eq!(body["error"], "forbidden");
        assert_eq!(body["error_code"], 1002);
        assert_eq!(body["message"], format!("Forbidden: {message}"));
        assert_eq!(body["details"]["credential_type"], kind);
        assert_eq!(body["details"]["reason"], "credential_type_unsupported");
        assert_eq!(body["details"]["accepted"], json!(["user_session"]));
        for secret in [&token, "private-client-id", "private-id"] {
            assert!(!body.to_string().contains(secret));
        }
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/channel-bots")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn denial_guidance_channel_routes_keep_human_success_and_agent_denial() {
    Box::pin(channel_routes_keep_human_success_and_agent_denial()).await;
}

#[tokio::test]
async fn key_update_service_account_denial_matches_middleware_guidance() {
    use axum::extract::{Path, State};
    use mongodb::bson::{DateTime, Document};

    let db = crate::test_utils::connect_transaction_test_database("key_update_denial").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let id = Uuid::new_v4().to_string();
    db.collection::<Document>(SERVICE_ACCOUNTS)
        .insert_one(doc! {
            "_id": &id, "name": "Fixture", "client_id": "sa_private_fixture",
            "client_secret_hash": "private_fixture_hash", "secret_prefix": "private_prefix",
            "allowed_scopes": "proxy", "is_active": true, "created_by": Uuid::new_v4().to_string(),
            "created_at": DateTime::now(), "updated_at": DateTime::now(),
        })
        .await
        .unwrap();
    let mut auth = crate::test_utils::test_auth_user(&id);
    auth.auth_method = AuthMethod::ServiceAccount;
    let error = Box::pin(crate::handlers::key_updates::update_key(
        State(state),
        auth,
        Path("private_resource_id".into()),
        Request::builder()
            .method("PUT")
            .body(Body::empty())
            .unwrap(),
    ))
    .await
    .unwrap_err();
    assert!(error.is_forbidden());
    let body = serde_json::to_value(error.response_body()).unwrap();
    assert_eq!(body["error"], "forbidden");
    assert_eq!(body["error_code"], 1002);
    assert_eq!(
        body["message"],
        "Forbidden: Service accounts cannot access this endpoint"
    );
    assert_eq!(body["details"]["reason"], "credential_type_unsupported");
    assert_eq!(body["details"]["credential_type"], "service_account");
    assert_eq!(body["details"]["accepted"], json!(["user_session"]));
    assert!(!body.to_string().contains("private_"));
    assert!(!body.to_string().contains(&id));
    db.drop().await.unwrap();
}

async fn channel_routes_keep_human_success_and_agent_denial() {
    let db = crate::test_utils::connect_transaction_test_database("credential_denial_routes").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let actor = Uuid::new_v4();
    db.collection::<User>(USERS)
        .insert_one(crate::test_utils::test_user(
            &actor.to_string(),
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let raw_key = "nyxid_ag_denial_valid_fixture";
    let key = super::tests::delegated_fixture_api_key(
        &Uuid::new_v4().to_string(),
        &actor.to_string(),
        &hash_token(raw_key),
    );
    db.collection::<ApiKey>(API_KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    let human = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &actor,
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let expired = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &actor,
        "openid profile",
        None,
        Some(-3600),
        None,
        None,
        None,
    )
    .unwrap();
    let oauth = jwt::generate_oauth_access_token(
        &state.jwt_keys,
        &state.config,
        &actor,
        "openid profile",
        None,
        None,
        None,
        None,
        None,
        "private-client-id",
    )
    .unwrap();
    let (_, private) = crate::routes::build_router();
    let app = private.with_state(state.clone());
    for path in ["/api/v1/channel-bots", "/api/v1/channel-conversations"] {
        let request = |token: &str| {
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap()
        };
        // These channel lists do not have the separate first-party-only gate.
        for token in [&human, &oauth] {
            let response = app.clone().oneshot(request(token)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        for header in ["authorization", "x-api-key"] {
            let value = if header == "authorization" {
                format!("Bearer {raw_key}")
            } else {
                raw_key.into()
            };
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header(header, value)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let body = json_body(response).await;
            assert_eq!(
                body["message"],
                "Forbidden: API keys cannot access this endpoint"
            );
            assert_eq!(body["error"], "forbidden");
            assert_eq!(body["error_code"], 1002);
            assert_eq!(body["details"]["reason"], "credential_type_unsupported");
            assert_eq!(body["details"]["credential_type"], "api_key");
            assert_eq!(body["details"]["accepted"], json!(["user_session"]));
            for secret in [raw_key, &actor.to_string(), &key.id] {
                assert!(!body.to_string().contains(secret));
            }
        }
        for (token, reason) in [
            (expired.as_str(), "credential_expired"),
            ("invalid-jwt", "authentication_failed"),
        ] {
            let response = app.clone().oneshot(request(token)).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let body = json_body(response).await;
            assert_eq!(body["details"]["reason"], reason);
            assert!(body["details"].get("accepted").is_none());
            assert!(!body.to_string().contains(token));
        }
    }
    // The same valid Agent Key is accepted by assigned-conversation discovery.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/channel-relay/conversations")
                .header("authorization", format!("Bearer {raw_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["total"], 0);
    // Invalid and expired API keys reach authentication on the agent surface.
    db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &key.id},
            doc! {"$set": {"expires_at": mongodb::bson::DateTime::from_chrono(
                chrono::Utc::now() - chrono::Duration::hours(1)
            )}},
        )
        .await
        .unwrap();
    for token in [raw_key, "nyxid_ag_nonexistent_fixture"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/channel-relay/conversations")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = json_body(response).await;
        assert_ne!(body["details"]["reason"], "credential_type_unsupported");
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn denial_guidance_scope_failures_do_not_claim_wrong_credential_type() {
    let token = format!(
        "header.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"delegated":true,"scope":"proxy:*"}).to_string())
    );
    let response = Router::new()
        .route("/api/v1/keys", get(|| async { StatusCode::OK }))
        .layer(middleware::from_fn(reject_delegated_tokens))
        .oneshot(
            Request::builder()
                .uri("/api/v1/keys")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = json_body(response).await;
    assert_eq!(
        body["message"],
        format!("Forbidden: {DELEGATED_ENDPOINT_FORBIDDEN}")
    );
    assert_eq!(body["details"]["reason"], "insufficient_scope");
    assert!(body["details"].get("accepted").is_none());
    let mut user = crate::test_utils::test_auth_user(&Uuid::new_v4().to_string());
    user.auth_method = AuthMethod::ApiKey;
    user.scope = "read".into();
    for error in [
        user.ensure_write_scope().unwrap_err(),
        user.ensure_rest_proxy_access().unwrap_err(),
        user.ensure_llm_proxy_access().unwrap_err(),
    ] {
        assert_eq!(
            error.response_body().details.unwrap()["reason"],
            "insufficient_scope"
        );
        assert_eq!(error.error_code(), 1002);
    }
    let generic = AppError::Forbidden("Resource permission denied".into()).response_body();
    assert!(generic.details.is_none());
    // Existing assistant tool callers retain their fixed denial copy too.
    let old = crate::services::assistant_account_tools::error_result(AppError::Forbidden(
        "Missing scope".into(),
    ));
    let guided = crate::services::assistant_account_tools::error_result(
        AppError::insufficient_scope("Missing scope"),
    );
    assert_eq!(old.value, guided.value);
}

#[test]
fn denial_guidance_verified_first_party_guard_matches_early_guard() {
    let mut user = crate::test_utils::test_auth_user(&Uuid::new_v4().to_string());
    user.auth_method = AuthMethod::Session;
    crate::handlers::login_client_context::require_first_party_human(&user).unwrap();
    for (method, kind) in [
        (AuthMethod::ApiKey, "api_key"),
        (AuthMethod::Delegated, "delegated"),
        (AuthMethod::Relay, "relay"),
        (AuthMethod::ServiceAccount, "service_account"),
        (AuthMethod::AccessToken, "oauth_client"),
    ] {
        user.auth_method = method;
        user.oauth_client_id = Some("private-client-id".into());
        let body = crate::handlers::login_client_context::require_first_party_human(&user)
            .unwrap_err()
            .response_body();
        assert_eq!(body.details.unwrap()["credential_type"], kind);
    }
}
