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
