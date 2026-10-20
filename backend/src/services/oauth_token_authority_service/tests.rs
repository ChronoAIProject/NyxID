use super::*;
use crate::{
    crypto::jwt::{self, TokenRestrictionClaims},
    models::{consent::ProtectedConsentRef, user::UserType},
    test_utils,
};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    state: crate::AppState,
    user: Uuid,
    client: String,
}
impl Fixture {
    async fn new() -> Self {
        let db = test_utils::connect_test_database("oauth_live_authority")
            .await
            .unwrap();
        let user = Uuid::new_v4();
        db.collection::<User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_utils::test_user(&user.to_string(), UserType::Person))
            .await
            .unwrap();
        super::super::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let client = Uuid::new_v4().to_string();
        let now = Utc::now();
        db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
            .insert_one(OauthClient {
                id: client.clone(),
                client_name: "Authority regression".into(),
                client_secret_hash: crate::crypto::token::hash_token("fixture-client-secret"),
                redirect_uris: vec!["https://example.test/callback".into()],
                allowed_scopes: "openid profile proxy".into(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".into(),
                client_type: "confidential".into(),
                is_active: true,
                delegation_scopes: "proxy:* mcp:catalog:read".into(),
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
        Self {
            state: test_utils::test_app_state(db),
            user,
            client,
        }
    }
    async fn grant(&self) -> ProtectedConsentRef {
        let consent = consent_service::grant_consent(
            &self.state.db,
            &self.user.to_string(),
            &self.client,
            "openid profile proxy",
        )
        .await
        .unwrap();
        consent_service::ensure_issuance_reference(&self.state.db, &consent)
            .await
            .unwrap()
    }
    fn claims(&self, token: &str) -> Claims {
        jwt::verify_token(&self.state.jwt_keys, &self.state.config, token).unwrap()
    }
    fn oauth_token(&self, client: &str, reference: &ProtectedConsentRef) -> String {
        jwt::generate_oauth_access_token_with_consent(
            &self.state.jwt_keys,
            &self.state.config,
            &self.user,
            "openid profile proxy",
            None,
            None,
            None,
            None,
            None,
            client,
            Some(reference),
        )
        .unwrap()
    }

    async fn userinfo(&self, token: &str) -> StatusCode {
        Router::new()
            .route("/oauth/userinfo", get(crate::handlers::oauth::userinfo))
            .with_state(self.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/oauth/userinfo")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    async fn introspection(&self, token: &str, caller: &str) -> bool {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", caller)
            .append_pair("client_secret", "fixture-client-secret")
            .append_pair("token", token)
            .finish();
        let response = Router::new()
            .route(
                "/oauth/introspect",
                post(crate::handlers::oauth::introspect),
            )
            .with_state(self.state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/introspect")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["active"]
            .as_bool()
            .unwrap()
    }

    async fn mcp_initialize(&self, token: &str) -> StatusCode {
        use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
        let response = Router::new().route("/mcp", post(crate::handlers::mcp_transport::mcp_post))
            .layer(axum::Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp)))
            .with_state(self.state.clone())
            .oneshot(Request::builder().method("POST").uri("/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"authority-regression","version":"1"}}}).to_string())).unwrap())
            .await.unwrap();
        let status = response.status();
        if status == StatusCode::OK {
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                body.get("error").is_none(),
                "MCP initialization must succeed, not merely return HTTP 200"
            );
            assert!(body["result"]["protocolVersion"].is_string());
        }
        status
    }
    async fn profile(&self, token: &str) -> StatusCode {
        Router::new()
            .route("/api/v1/users/me", get(crate::handlers::users::get_me))
            .with_state(self.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/users/me")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    async fn callback(&self, token: &str) -> StatusCode {
        Router::new()
            .route(
                "/api/v1/proxy/s/authority-fixture",
                get(|_: crate::mw::auth::AuthUser| async { StatusCode::OK }),
            )
            .with_state(self.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/proxy/s/authority-fixture")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    fn callbacks(&self, source: &Claims) -> Vec<String> {
        let restrictions = TokenRestrictionClaims::from_claims(source);
        let proxy = super::super::identity_service::generate_proxy_delegation_token(
            &self.state.jwt_keys,
            &self.state.config,
            &self.user,
            "proxy:*",
            "alias",
            Some("aevatar"),
            Some(&restrictions),
        )
        .unwrap();
        let mut auth = test_utils::test_auth_user(&self.user.to_string());
        auth.oauth_lineage = jwt::authority_lineage(source);
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.slug = "aevatar".into();
        service.delegation_token_scope = "proxy:*".into();
        let header =
            crate::handlers::assistant::build_forward_authorization(&self.state, &auth, &service)
                .unwrap();
        vec![
            proxy,
            header
                .to_str()
                .unwrap()
                .strip_prefix("Bearer ")
                .unwrap()
                .to_owned(),
        ]
    }
}

#[tokio::test]
async fn real_userinfo_introspection_and_manual_mcp_keep_client_authorities_separate() {
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let token = fixture.oauth_token(&fixture.client, &reference);
    let other_id = Uuid::new_v4().to_string();
    let mut other = fixture
        .state
        .db
        .collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .find_one(doc! {"_id":&fixture.client})
        .await
        .unwrap()
        .unwrap();
    other.id = other_id.clone();
    fixture
        .state
        .db
        .collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .insert_one(other)
        .await
        .unwrap();
    let other_consent = consent_service::grant_consent(
        &fixture.state.db,
        &fixture.user.to_string(),
        &other_id,
        "openid profile proxy",
    )
    .await
    .unwrap();
    let other_reference =
        consent_service::ensure_issuance_reference(&fixture.state.db, &other_consent)
            .await
            .unwrap();
    let unaffected = fixture.oauth_token(&other_id, &other_reference);
    for (candidate, caller) in [(&token, &fixture.client), (&unaffected, &other_id)] {
        assert_eq!(fixture.userinfo(candidate).await, StatusCode::OK);
        assert!(fixture.introspection(candidate, caller).await);
        assert_eq!(fixture.mcp_initialize(candidate).await, StatusCode::OK);
    }
    let independent =
        mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap()
            .database(fixture.state.db.name());
    consent_service::revoke_consent(&independent, &fixture.user.to_string(), &fixture.client)
        .await
        .unwrap();
    let successor_reference = fixture.grant().await;
    let successor = fixture.oauth_token(&fixture.client, &successor_reference);
    assert_eq!(fixture.userinfo(&token).await, StatusCode::UNAUTHORIZED);
    assert!(!fixture.introspection(&token, &fixture.client).await);
    assert_eq!(
        fixture.mcp_initialize(&token).await,
        StatusCode::UNAUTHORIZED
    );
    for (candidate, caller) in [(&successor, &fixture.client), (&unaffected, &other_id)] {
        assert_eq!(fixture.userinfo(candidate).await, StatusCode::OK);
        assert!(fixture.introspection(candidate, caller).await);
        assert_eq!(fixture.mcp_initialize(candidate).await, StatusCode::OK);
    }
    // Even if the calling introspection client remains valid, removal of the
    // token's own client must deny all three independent authority readers.
    independent
        .collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .delete_one(doc! {"_id":&fixture.client})
        .write_concern(mongodb::options::WriteConcern::majority())
        .await
        .unwrap();
    assert_eq!(fixture.userinfo(&successor).await, StatusCode::UNAUTHORIZED);
    assert!(!fixture.introspection(&successor, &other_id).await);
    assert_eq!(
        fixture.mcp_initialize(&successor).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(fixture.userinfo(&unaffected).await, StatusCode::OK);
    assert!(fixture.introspection(&unaffected, &other_id).await);
    assert_eq!(fixture.mcp_initialize(&unaffected).await, StatusCode::OK);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ordinary_grant_and_production_callbacks_stay_revoked_after_regrant() {
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let token = jwt::generate_oauth_access_token_with_consent(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "openid profile proxy",
        None,
        None,
        None,
        None,
        None,
        &fixture.client,
        Some(&reference),
    )
    .unwrap();
    let source = fixture.claims(&token);
    let callbacks = fixture.callbacks(&source);
    assert_eq!(fixture.profile(&token).await, StatusCode::OK);
    for callback in &callbacks {
        assert_eq!(fixture.callback(callback).await, StatusCode::OK);
    }
    // Independent driver handle: no positive permission cache may survive this write.
    let client = mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let independent = client.database(fixture.state.db.name());
    consent_service::revoke_consent(&independent, &fixture.user.to_string(), &fixture.client)
        .await
        .unwrap();
    fixture.grant().await;
    assert_eq!(fixture.profile(&token).await, StatusCode::UNAUTHORIZED);
    for callback in callbacks {
        assert_eq!(fixture.callback(&callback).await, StatusCode::UNAUTHORIZED);
    }
    let system = super::super::nyxbot_gateway_service::creator_bearer(
        &fixture.state,
        &fixture.user.to_string(),
    )
    .unwrap();
    assert_eq!(fixture.profile(&system).await, StatusCode::OK);
}

#[tokio::test]
async fn catalog_and_sidless_ordinary_delegation_callbacks_keep_own_authority_row() {
    for catalog in [false, true] {
        let fixture = Fixture::new().await;
        let reference = fixture.grant().await;
        // Both supported exchanges may lack pre-existing lineage and source sid.
        let (token, _) = jwt::generate_delegated_access_token_for_client(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &fixture.user,
            "proxy:*",
            &fixture.client,
            catalog.then_some(fixture.client.as_str()),
            300,
            None,
        )
        .unwrap();
        let source = fixture.claims(&token);
        assert!(source.sid.is_none() && source.oauth_lineage.is_none());
        persist(
            &fixture.state.db,
            &source,
            vec![ConsentBinding {
                client_id: fixture.client.clone(),
                scopes: "openid".into(),
                reference,
            }],
            None,
        )
        .await
        .unwrap();
        let lineage = jwt::authority_lineage(&source).expect("OAuth exchange lineage");
        assert_eq!(lineage.delegation_jti.as_deref(), Some(source.jti.as_str()));
        let callbacks = fixture.callbacks(&source);
        for callback in &callbacks {
            assert_eq!(fixture.callback(callback).await, StatusCode::OK);
        }
        consent_service::revoke_consent(
            &fixture.state.db,
            &fixture.user.to_string(),
            &fixture.client,
        )
        .await
        .unwrap();
        fixture.grant().await;
        assert!(validate_claims(&fixture.state.db, &source).await.is_err());
        for callback in callbacks {
            assert_eq!(fixture.callback(&callback).await, StatusCode::UNAUTHORIZED);
        }
    }
}

#[tokio::test]
async fn missing_online_oauth_row_never_falls_back_to_service_origin() {
    let fixture = Fixture::new().await;
    fixture.grant().await;
    let (token, _) = jwt::generate_delegated_access_token_for_client(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "proxy:*",
        &fixture.client,
        None,
        300,
        None,
    )
    .unwrap();
    assert_eq!(fixture.profile(&token).await, StatusCode::UNAUTHORIZED);
    let source = fixture.claims(&token);
    for callback in fixture.callbacks(&source) {
        assert_eq!(fixture.callback(&callback).await, StatusCode::UNAUTHORIZED);
    }
    let first_party = jwt::generate_access_token(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "account:read",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    for callback in fixture.callbacks(&fixture.claims(&first_party)) {
        assert_eq!(fixture.callback(&callback).await, StatusCode::OK);
    }
}

#[tokio::test]
async fn actual_exchange_and_refresh_cannot_acquire_regranted_consent() {
    use super::super::token_exchange_service as exchange;
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let source = jwt::generate_oauth_access_token_with_consent(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "openid profile proxy",
        None,
        None,
        None,
        None,
        None,
        &fixture.client,
        Some(&reference),
    )
    .unwrap();
    let issued = exchange::exchange_token(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.client,
        "fixture-client-secret",
        &source,
        "urn:ietf:params:oauth:token-type:access_token",
        Some("proxy:*"),
    )
    .await
    .unwrap();
    let claims = fixture.claims(&issued.access_token);
    validate_claims(&fixture.state.db, &claims).await.unwrap();
    let callbacks = fixture.callbacks(&claims);
    for callback in &callbacks {
        assert_eq!(fixture.callback(callback).await, StatusCode::OK);
    }
    let refreshed = exchange::refresh_delegation_token(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.user.to_string(),
        &fixture.client,
        None,
        "proxy:*",
        &TokenRestrictionClaims::from_claims(&claims),
        &claims.jti,
    )
    .await
    .unwrap();
    let refreshed_claims = fixture.claims(&refreshed.access_token);
    validate_claims(&fixture.state.db, &refreshed_claims)
        .await
        .unwrap();
    let independent =
        mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap()
            .database(fixture.state.db.name());
    consent_service::revoke_consent(&independent, &fixture.user.to_string(), &fixture.client)
        .await
        .unwrap();
    fixture.grant().await;
    for token in [&source, &issued.access_token, &refreshed.access_token] {
        assert!(
            validate_claims(&fixture.state.db, &fixture.claims(token))
                .await
                .is_err()
        );
    }
    for callback in callbacks {
        assert_eq!(fixture.callback(&callback).await, StatusCode::UNAUTHORIZED);
    }
    assert!(
        exchange::refresh_delegation_token(
            &fixture.state.db,
            &fixture.state.config,
            &fixture.state.jwt_keys,
            &fixture.user.to_string(),
            &fixture.client,
            None,
            "proxy:*",
            &TokenRestrictionClaims::from_claims(&claims),
            &claims.jti
        )
        .await
        .is_err()
    );
    assert!(
        exchange::exchange_token(
            &fixture.state.db,
            &fixture.state.config,
            &fixture.state.jwt_keys,
            &fixture.client,
            "fixture-client-secret",
            &source,
            "urn:ietf:params:oauth:token-type:access_token",
            Some("proxy:*")
        )
        .await
        .is_err()
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn revocation_and_client_soft_deletion_fail_tokens_on_another_app_state() {
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let old = fixture.oauth_token(&fixture.client, &reference);
    let other_db = mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
        .database(fixture.state.db.name());
    let replica = Fixture {
        state: test_utils::test_app_state(other_db),
        user: fixture.user,
        client: fixture.client.clone(),
    };
    assert_eq!(replica.userinfo(&old).await, StatusCode::OK);
    assert!(replica.introspection(&old, &fixture.client).await);
    consent_service::revoke_consent(
        &fixture.state.db,
        &fixture.user.to_string(),
        &fixture.client,
    )
    .await
    .unwrap();
    assert_eq!(replica.userinfo(&old).await, StatusCode::UNAUTHORIZED);
    let reference = fixture.grant().await;
    let current = fixture.oauth_token(&fixture.client, &reference);
    assert_eq!(replica.userinfo(&old).await, StatusCode::UNAUTHORIZED);
    assert_eq!(replica.userinfo(&current).await, StatusCode::OK);
    let refresh = super::super::oauth_service::issue_oauth_refresh_token(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.client,
        &fixture.user.to_string(),
        "openid profile proxy",
        &[],
        &[],
        false,
        Some(&reference),
    )
    .await
    .unwrap();
    // Exercise the public lifecycle service: deletion is normally a soft delete.
    super::super::oauth_client_service::delete_client(&fixture.state.db, &fixture.client)
        .await
        .unwrap();
    assert_eq!(replica.userinfo(&current).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        replica.mcp_initialize(&current).await,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        super::super::token_service::refresh_tokens(
            &replica.state.db,
            &replica.state.config,
            &replica.state.jwt_keys,
            &refresh.refresh_token,
            None,
            None
        )
        .await
        .is_err()
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ordinary_oauth_revocation_does_not_replace_relay_parent_key_authority() {
    let fixture = Fixture::new().await;
    fixture.grant().await;
    let key = super::super::key_service::create_api_key(
        &fixture.state.db,
        &fixture.user.to_string(),
        "Relay parent",
        "proxy",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let relay = jwt::generate_relay_access_token(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "proxy",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id.clone(),
            api_key_name: key.name,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
        },
    )
    .unwrap();
    let claims = fixture.claims(&relay);
    validate_claims(&fixture.state.db, &claims).await.unwrap();
    crate::mw::auth::ensure_relay_agent_key_active(&fixture.state.db, &claims)
        .await
        .unwrap();
    consent_service::revoke_consent(
        &fixture.state.db,
        &fixture.user.to_string(),
        &fixture.client,
    )
    .await
    .unwrap();
    super::super::oauth_client_service::delete_client(&fixture.state.db, &fixture.client)
        .await
        .unwrap();
    validate_claims(&fixture.state.db, &claims).await.unwrap();
    crate::mw::auth::ensure_relay_agent_key_active(&fixture.state.db, &claims)
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(doc! {"_id":&key.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        crate::mw::auth::ensure_relay_agent_key_active(&fixture.state.db, &claims)
            .await
            .is_err()
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authorization_code_issuance_held_across_revocation_never_returns_authority() {
    use super::super::oauth_service as oauth;
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let code = oauth::create_authorization_code_with_consent(
        &fixture.state.db,
        &fixture.client,
        &fixture.user.to_string(),
        "https://example.test/callback",
        "openid profile proxy",
        None,
        None,
        None,
        None,
        None,
        &[],
        &[],
        false,
        None,
        Some(&reference),
    )
    .await
    .unwrap();
    let pause = std::sync::Arc::new(oauth::IssuancePause::default());
    let issuance = oauth::ISSUANCE_PAUSE.scope(
        pause.clone(),
        oauth::exchange_authorization_code(
            &fixture.state.db,
            &fixture.state.config,
            &fixture.state.jwt_keys,
            false,
            &code,
            &fixture.client,
            "https://example.test/callback",
            None,
            Some("fixture-client-secret"),
            None,
            None,
            None,
            None,
        ),
    );
    let revocation = async {
        tokio::time::timeout(std::time::Duration::from_secs(10), pause.entered.notified())
            .await
            .unwrap();
        consent_service::revoke_consent(
            &fixture.state.db,
            &fixture.user.to_string(),
            &fixture.client,
        )
        .await
        .unwrap();
        // A regrant while issuance is held cannot lend the old request its revision.
        fixture.grant().await;
        pause.release.notify_one();
    };
    let (result, ()) = tokio::join!(issuance, revocation);
    assert!(matches!(result, Err(AppError::Conflict(_))));
    let collection = fixture
        .state
        .db
        .collection::<crate::models::refresh_token::RefreshToken>(
            crate::models::refresh_token::COLLECTION_NAME,
        );
    assert_eq!(
        collection
            .count_documents(doc! {"user_id":fixture.user.to_string()})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        collection
            .count_documents(doc! {"user_id":fixture.user.to_string(),"revoked":false})
            .await
            .unwrap(),
        0
    );
    fixture.state.db.drop().await.unwrap();
}

async fn seed_existing_session(fixture: &Fixture) {
    let now = Utc::now();
    fixture
        .state
        .db
        .collection::<Session>(crate::models::session::COLLECTION_NAME)
        .insert_one(Session {
            id: Uuid::new_v4().to_string(),
            user_id: fixture.user.to_string(),
            purpose: Some(crate::models::session::SessionPurpose::Authenticated),
            token_hash: crate::crypto::token::hash_token("unrelated-existing-session-fixture"),
            ip_address: None,
            user_agent: None,
            expires_at: now + chrono::Duration::hours(1),
            revoked: false,
            created_at: now,
            last_active_at: now,
        })
        .await
        .unwrap();
}

async fn token_and_session_rows(db: &Database) -> Vec<Vec<bson::Document>> {
    use futures::TryStreamExt;
    let mut rows = Vec::new();
    for collection in [
        crate::models::refresh_token::COLLECTION_NAME,
        crate::models::session::COLLECTION_NAME,
        crate::models::consent::COLLECTION_NAME,
    ] {
        rows.push(
            db.collection::<bson::Document>(collection)
                .find(doc! {})
                .sort(doc! {"_id": 1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap(),
        );
    }
    rows
}

async fn assert_retryable_authority_response(response: axum::response::Response, oauth: bool) {
    use axum::body::to_bytes;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("retry-after").unwrap(), "1");
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    if oauth {
        assert_eq!(body["error"], "temporarily_unavailable");
    } else {
        assert_eq!(body["error"], "oauth_authority_unavailable");
        assert_eq!(body["error_code"], 12900);
    }
    let description = body.to_string();
    assert!(!description.contains("authorize again"));
    assert!(!description.contains("invalid_grant"));
    assert!(!description.contains("invalidAuthorityTestOperator"));
}

#[tokio::test]
async fn unavailable_authority_reads_return_retryable_503_without_consuming_state() {
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let token = fixture.oauth_token(&fixture.client, &reference);
    super::super::oauth_service::issue_oauth_refresh_token(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.client,
        &fixture.user.to_string(),
        "openid profile proxy",
        &[],
        &[],
        false,
        Some(&reference),
    )
    .await
    .unwrap();
    seed_existing_session(&fixture).await;
    let before = token_and_session_rows(&fixture.state.db).await;
    assert_eq!(fixture.userinfo(&token).await, StatusCode::OK);
    for collection in [
        crate::models::user::COLLECTION_NAME,
        crate::models::oauth_client::COLLECTION_NAME,
        crate::models::consent::COLLECTION_NAME,
    ] {
        for kind in [ReadFaultKind::Timeout, ReadFaultKind::Database] {
            let response = READ_FAULT
                .scope(
                    ReadFault { collection, kind },
                    Router::new()
                        .route("/oauth/userinfo", get(crate::handlers::oauth::userinfo))
                        .with_state(fixture.state.clone())
                        .oneshot(
                            Request::builder()
                                .uri("/oauth/userinfo")
                                .header("authorization", format!("Bearer {token}"))
                                .body(Body::empty())
                                .unwrap(),
                        ),
                )
                .await
                .unwrap();
            assert_retryable_authority_response(response, false).await;
            assert_eq!(before, token_and_session_rows(&fixture.state.db).await);
            assert_eq!(
                READ_FAULT
                    .scope(
                        ReadFault { collection, kind },
                        fixture.mcp_initialize(&token)
                    )
                    .await,
                StatusCode::SERVICE_UNAVAILABLE,
            );
            assert_eq!(before, token_and_session_rows(&fixture.state.db).await);
        }
    }
    assert_eq!(fixture.userinfo(&token).await, StatusCode::OK);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn unavailable_consent_reads_preserve_refresh_and_introspection_retryability() {
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let access = fixture.oauth_token(&fixture.client, &reference);
    let refresh = super::super::oauth_service::issue_oauth_refresh_token(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.client,
        &fixture.user.to_string(),
        "openid profile proxy",
        &[],
        &[],
        false,
        Some(&reference),
    )
    .await
    .unwrap();
    seed_existing_session(&fixture).await;
    let before = token_and_session_rows(&fixture.state.db).await;
    for kind in [ReadFaultKind::Timeout, ReadFaultKind::Database] {
        let fault = ReadFault {
            collection: crate::models::consent::COLLECTION_NAME,
            kind,
        };
        let direct = READ_FAULT
            .scope(
                fault,
                consent_service::validate_protected_reference(
                    &fixture.state.db,
                    &fixture.user.to_string(),
                    &fixture.client,
                    "openid",
                    Some(&reference),
                ),
            )
            .await;
        assert!(matches!(direct, Err(AppError::OAuthAuthorityUnavailable)));
        let app = Router::new()
            .route("/oauth/token", post(crate::handlers::oauth::token))
            .route(
                "/oauth/introspect",
                post(crate::handlers::oauth::introspect),
            )
            .with_state(fixture.state.clone());
        for (path, params, oauth) in [
            (
                "/oauth/token",
                vec![
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.refresh_token.as_str()),
                    ("client_id", fixture.client.as_str()),
                    ("client_secret", "fixture-client-secret"),
                ],
                true,
            ),
            (
                "/oauth/introspect",
                vec![
                    ("token", access.as_str()),
                    ("client_id", fixture.client.as_str()),
                    ("client_secret", "fixture-client-secret"),
                ],
                false,
            ),
        ] {
            let response = READ_FAULT
                .scope(
                    fault,
                    app.clone().oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(path)
                            .header("content-type", "application/x-www-form-urlencoded")
                            .body(Body::from(
                                url::form_urlencoded::Serializer::new(String::new())
                                    .extend_pairs(params)
                                    .finish(),
                            ))
                            .unwrap(),
                    ),
                )
                .await
                .unwrap();
            assert_retryable_authority_response(response, oauth).await;
            assert_eq!(before, token_and_session_rows(&fixture.state.db).await);
        }
    }
    assert_eq!(fixture.userinfo(&access).await, StatusCode::OK);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn definitive_missing_inactive_expired_and_mismatched_authority_still_returns_401() {
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let token = fixture.oauth_token(&fixture.client, &reference);
    for (collection, id, cases) in [
        (
            crate::models::user::COLLECTION_NAME,
            fixture.user.to_string(),
            vec!["missing", "inactive"],
        ),
        (
            crate::models::oauth_client::COLLECTION_NAME,
            fixture.client.clone(),
            vec!["missing", "inactive"],
        ),
        (
            crate::models::consent::COLLECTION_NAME,
            reference.consent_id.clone(),
            vec!["missing", "revision", "expired", "scope"],
        ),
    ] {
        let collection = fixture.state.db.collection::<bson::Document>(collection);
        let original = collection
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .unwrap();
        for case in cases {
            assert_eq!(fixture.userinfo(&token).await, StatusCode::OK);
            if case == "missing" {
                collection
                    .delete_one(doc! {"_id": &id})
                    .write_concern(WriteConcern::majority())
                    .await
                    .unwrap();
            } else {
                let fields = match case {
                    "inactive" => doc! {"is_active": false},
                    "revision" => doc! {"revision": Uuid::new_v4().to_string()},
                    "expired" => {
                        doc! {"expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}
                    }
                    "scope" => doc! {"scopes": "openid"},
                    _ => unreachable!(),
                };
                collection
                    .update_one(doc! {"_id": &id}, doc! {"$set": fields})
                    .write_concern(WriteConcern::majority())
                    .await
                    .unwrap();
            }
            assert_eq!(
                fixture.userinfo(&token).await,
                StatusCode::UNAUTHORIZED,
                "{case}"
            );
            if case == "missing" {
                collection.insert_one(&original).await.unwrap();
            } else {
                collection
                    .replace_one(doc! {"_id": &id}, &original)
                    .await
                    .unwrap();
            }
        }
    }
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn delegation_fingerprint_mismatch_is_definitive_and_positive_reads_are_operation_local() {
    use axum::response::IntoResponse;
    let fixture = Fixture::new().await;
    let reference = fixture.grant().await;
    let (token, _) = jwt::generate_delegated_access_token_for_client(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "proxy:*",
        &fixture.client,
        Some(&fixture.client),
        300,
        None,
    )
    .unwrap();
    let claims = fixture.claims(&token);
    persist(
        &fixture.state.db,
        &claims,
        vec![ConsentBinding {
            client_id: fixture.client.clone(),
            scopes: "openid".into(),
            reference,
        }],
        None,
    )
    .await
    .unwrap();
    let mut reads = AuthorityReads::new(&fixture.state.db);
    validate_claims_with_reads(&mut reads, &claims)
        .await
        .unwrap();
    // The same receiver is also a consent client. A repeated check in this
    // operation uses its already validated snapshot even when another read
    // would fail. A separate validation must perform new reads and fail 503.
    let fault = ReadFault {
        collection: crate::models::oauth_client::COLLECTION_NAME,
        kind: ReadFaultKind::Database,
    };
    READ_FAULT
        .scope(fault, active_client(&mut reads, &fixture.client))
        .await
        .unwrap();
    assert!(matches!(
        READ_FAULT
            .scope(fault, validate_claims(&fixture.state.db, &claims))
            .await,
        Err(AppError::OAuthAuthorityUnavailable)
    ));
    fixture
        .state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &claims.jti},
            doc! {"$set": {"claims_fingerprint": "mismatched-fingerprint"}},
        )
        .write_concern(WriteConcern::majority())
        .await
        .unwrap();
    let denied = validate_claims(&fixture.state.db, &claims)
        .await
        .unwrap_err();
    assert_eq!(denied.into_response().status(), StatusCode::UNAUTHORIZED);
    fixture.state.db.drop().await.unwrap();
}
