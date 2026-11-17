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
    let now = chrono::Utc::now();
    db.collection::<crate::models::oauth_client::OauthClient>(
        crate::models::oauth_client::COLLECTION_NAME,
    )
    .insert_one(crate::models::oauth_client::OauthClient {
        id: "private-client-id".into(),
        client_name: "Channel list authority fixture".into(),
        client_secret_hash: String::new(),
        redirect_uris: vec![],
        allowed_scopes: "openid profile".into(),
        scope_provenance: Default::default(),
        grant_types: "authorization_code".into(),
        client_type: "public".into(),
        is_active: true,
        delegation_scopes: String::new(),
        default_service_catalog_slugs: vec![],
        broker_capability_enabled: false,
        revocation_webhook_url: None,
        revocation_webhook_secret_encrypted: None,
        connection_webhook_url: None,
        connection_webhook_secret_encrypted: None,
        connection_webhook_key_id: None,
        connection_webhook_enabled: false,
        created_by: None,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap();
    let consent = crate::services::consent_service::grant_consent_with_services(
        &db,
        &actor.to_string(),
        "private-client-id",
        "openid profile",
        None,
    )
    .await
    .unwrap();
    let reference = crate::services::consent_service::ensure_issuance_reference(&db, &consent)
        .await
        .unwrap();
    let oauth = jwt::generate_oauth_access_token_with_consent(
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
        Some(&reference),
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
    // Assistant tool callers receive identical server-authored rendering for
    // guided and unguided denials; HTTP-only guidance metadata stays hidden.
    let old = crate::services::assistant_account_tools::error_result(AppError::Forbidden(
        "Missing scope".into(),
    ));
    let guided = crate::services::assistant_account_tools::error_result(
        AppError::insufficient_scope("Missing scope"),
    );
    assert_eq!(old.value, guided.value);
    assert_eq!(old.value["message"], "Missing scope");
    assert!(old.value.get("instructions").is_none());
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

#[tokio::test]
async fn notification_routes_are_first_party_human_only_and_require_both_flags() {
    use crate::services::feature_flag_service as flags;
    let db =
        crate::test_utils::connect_transaction_test_database("notifications_auth_matrix").await;
    crate::services::assistant_notification_service::ensure_indexes(&db)
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let actor = Uuid::new_v4();
    let owner = actor.to_string();
    db.collection::<User>(USERS)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
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
    let (_, private) = crate::routes::build_router();
    let app = private.with_state(state);
    let root = "/api/v1/assistant/nyxagent/notifications";
    for enabled in [false, true] {
        flags::set_platform_override(
            &db,
            flags::ASSISTANT_NOTIFICATIONS_FLAG_KEY,
            &flags::FlagTarget::User(owner.clone()),
            enabled,
            &owner,
        )
        .await
        .unwrap();
        for (method, suffix) in [("GET", ""), ("GET", "/summary"), ("POST", "/read")] {
            let uri = format!("{root}{suffix}");
            let body = if method == "POST" {
                json!({"ids":[Uuid::new_v4().to_string()]}).to_string()
            } else {
                String::new()
            };
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&uri)
                        .header("authorization", format!("Bearer {human}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            if enabled {
                assert!(
                    response.status().is_success(),
                    "{method} {uri}: {}",
                    response.status()
                );
                if method == "GET" {
                    let value = json_body(response).await;
                    if suffix.is_empty() {
                        assert_eq!(value["items"], json!([]));
                    } else {
                        assert_eq!(value["unread_count"], 0);
                    }
                }
            } else {
                assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
                let value = json_body(response).await;
                assert_eq!(value["error"], "not_found");
                assert_eq!(value["message"], "Not found: Assistant route not found.");
            }
            for (kind, claims) in [
                ("relay", json!({"relay":true})),
                ("service_account", json!({"sa":true})),
                (
                    "delegated",
                    json!({"delegated":true,"scope":"account:read"}),
                ),
                ("oauth_client", json!({"client_id":"third-party-client"})),
            ] {
                let token = format!(
                    "header.{}.signature",
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
                );
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(&uri)
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(body.clone()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::FORBIDDEN,
                    "{kind} {method} {uri}"
                );
                assert_eq!(
                    json_body(response).await["details"]["credential_type"],
                    kind
                );
            }
            for token in [
                "nyxid_ag_ordinary-key",
                "nyxid_sk_conversation-key",
                "nyx_conversation-key",
            ] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(&uri)
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(body.clone()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
                assert_eq!(
                    json_body(response).await["details"]["credential_type"],
                    "api_key"
                );
            }
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&uri)
                        .header("x-api-key", "nyxid_ag_header-key")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(
                delegated_read_denied_path(&uri),
                "delegated account:read must not admit {uri}"
            );
        }
    }
    flags::set_platform_override(
        &db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::User(owner.clone()),
        false,
        &owner,
    )
    .await
    .unwrap();
    for suffix in ["", "/summary"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{root}{suffix}"))
                    .header("authorization", format!("Bearer {human}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn notification_routes_preserve_settings_and_audit_only_explicit_mark_all() {
    use crate::services::feature_flag_service as flags;
    use mongodb::bson::{self, DateTime, Document};

    let db = crate::test_utils::connect_transaction_test_database("notifications_http_state").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let actor = Uuid::new_v4();
    let owner = actor.to_string();
    db.collection::<User>(USERS)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        flags::ASSISTANT_NOTIFICATIONS_FLAG_KEY,
        &flags::FlagTarget::User(owner.clone()),
        true,
        &owner,
    )
    .await
    .unwrap();
    let token = jwt::generate_access_token(
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
    let (_, private) = crate::routes::build_router();
    let app = private.with_state(state);
    let request = |method: &str, path: &str, body: Value| {
        Request::builder()
            .method(method)
            .uri(format!("/api/v1/assistant/nyxagent/{path}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(if method == "GET" {
                String::new()
            } else {
                body.to_string()
            }))
            .unwrap()
    };
    let off = json!({"push": {
        "automation_result": false, "automation_problem": false, "needs_decision": false,
        "agent_update": false, "chat_update": false, "group_mention": false,
    }});
    let response = app
        .clone()
        .oneshot(request("GET", "settings", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["notifications"], off);
    let chosen = json!({"push": {
        "automation_result": true, "automation_problem": false, "needs_decision": true,
        "agent_update": false, "chat_update": true, "group_mention": false,
    }});
    for body in [
        json!({"notifications": chosen}),
        json!({"timezone": "Asia/Singapore"}),
    ] {
        let response = app
            .clone()
            .oneshot(request("PUT", "settings", body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["notifications"], chosen);
    }
    let settings = db.collection::<Document>(crate::models::assistant_settings::COLLECTION_NAME);
    assert_eq!(
        settings
            .find_one(doc! {"_id": &owner})
            .await
            .unwrap()
            .unwrap()["notifications"],
        bson::to_bson(&chosen).unwrap()
    );
    // A pre-rollout document has no notifications field and still reads all switches off.
    settings
        .update_one(doc! {"_id": &owner}, doc! {"$unset": {"notifications": ""}})
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(request("GET", "settings", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["notifications"], off);

    let before = chrono::Utc::now() - chrono::Duration::hours(1);
    let old = DateTime::from_chrono(before - chrono::Duration::seconds(1));
    let newer = DateTime::from_chrono(before + chrono::Duration::seconds(1));
    let conversation = Uuid::new_v4().to_string();
    let ids: Vec<String> = (0..5).map(|_| Uuid::new_v4().to_string()).collect();
    let notifications = db.collection::<Document>("assistant_notifications");
    notifications
        .insert_many([
            doc! {"_id": &ids[0], "user_id": &owner, "unread": true, "updated_at": old},
            doc! {"_id": &ids[1], "user_id": &owner, "unread": true, "updated_at": old,
                  "source": {"type": "channel_chat", "id": &conversation, "conversation_id": &conversation}},
            doc! {"_id": &ids[2], "user_id": &owner, "unread": true, "updated_at": old},
            doc! {"_id": &ids[3], "user_id": &owner, "unread": true, "updated_at": newer},
            doc! {"_id": &ids[4], "user_id": Uuid::new_v4().to_string(), "unread": true, "updated_at": old},
        ])
        .await
        .unwrap();
    let audit = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    let read_event = doc! {"user_id": &owner, "event_type": "assistant_notifications_read"};
    for body in [
        json!({"ids": [&ids[0], &ids[4]]}),
        json!({"source": {"type": "conversation", "id": conversation}}),
    ] {
        let response = app
            .clone()
            .oneshot(request("POST", "notifications/read", body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["updated"], 1);
        assert_eq!(audit.count_documents(read_event.clone()).await.unwrap(), 0);
    }
    for body in [
        json!({"ids": [], "all": true, "before": before.to_rfc3339()}),
        json!({"all": true}),
        json!({"all": false, "before": before.to_rfc3339()}),
        json!({"ids": [&ids[2]], "before": before.to_rfc3339()}),
    ] {
        let response = app
            .clone()
            .oneshot(request("POST", "notifications/read", body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(audit.count_documents(read_event.clone()).await.unwrap(), 0);
    let response = app
        .oneshot(request(
            "POST",
            "notifications/read",
            json!({"all": true, "before": before.to_rfc3339()}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["updated"], 1);
    assert_eq!(audit.count_documents(read_event.clone()).await.unwrap(), 1);
    let event = audit.find_one(read_event).await.unwrap().unwrap();
    assert_eq!(
        event.get_document("event_data").unwrap(),
        &doc! {"count": 1_i64, "scope": "all"}
    );
    for id in &ids[..3] {
        assert!(
            !notifications
                .find_one(doc! {"_id": id})
                .await
                .unwrap()
                .unwrap()
                .get_bool("unread")
                .unwrap()
        );
    }
    for id in &ids[3..] {
        assert!(
            notifications
                .find_one(doc! {"_id": id})
                .await
                .unwrap()
                .unwrap()
                .get_bool("unread")
                .unwrap()
        );
    }
    db.drop().await.unwrap();
}
