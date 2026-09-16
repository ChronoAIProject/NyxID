use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use mongodb::bson::{self, Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    AppState,
    crypto::jwt,
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        service_account::{COLLECTION_NAME as ACCOUNTS, ServiceAccount},
        service_account_token::{COLLECTION_NAME as TOKENS, ServiceAccountToken},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{
        catalog_skill_service,
        curation_grant_service::{self as grants, IssueCurationGrant},
        service_account_service as accounts,
    },
    test_utils,
};

pub(super) struct Fixture {
    pub state: AppState,
    pub sa: ServiceAccount,
    pub secret: String,
    pub service: DownstreamService,
    pub owner: String,
    pub human_token: String,
}

pub(super) async fn fixture(label: &str, curated: bool) -> Fixture {
    let db = test_utils::connect_transaction_test_database(label).await;
    catalog_skill_service::ensure_indexes(&db).await.unwrap();
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let owner = Uuid::new_v4().to_string();
    let mut user = test_utils::test_user(&owner, UserType::Person);
    user.is_admin = true;
    user.role_ids.push(
        crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap()
            .admin,
    );
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = format!("ornn-{}", service.id);
    service.created_by = owner.clone();
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    let (mut sa, secret) = accounts::create_service_account(
        &db,
        "Curator",
        None,
        "catalog:skills:read catalog:skills:write proxy",
        &[],
        None,
        &owner,
    )
    .await
    .unwrap();
    if curated {
        sa = grants::issue(&db, &sa.id, &owner, grant_input(&service.id))
            .await
            .unwrap();
    }
    let state = test_utils::test_app_state(db);
    let human_token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    Fixture {
        state,
        sa,
        secret,
        service,
        owner,
        human_token,
    }
}

pub(super) fn grant_input(id: &str) -> IssueCurationGrant {
    IssueCurationGrant {
        service_ids: vec![id.into()],
        ornn_proxy_service_id: Some(id.into()),
        expires_at: Some(Utc::now() + Duration::hours(2)),
        max_writes: 10,
        window_seconds: 3600,
    }
}

pub(super) async fn token(f: &Fixture, scope: Option<&str>) -> String {
    accounts::authenticate_client_credentials(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &f.sa.client_id,
        &f.secret,
        scope,
    )
    .await
    .unwrap()
    .access_token
}

fn router(state: &AppState) -> Router {
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    public.merge(private).with_state(state.clone())
}

pub(super) async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(body.map_or(Body::empty(), |body| Body::from(body.to_string())))
        .unwrap();
    let response = router(state).oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes)})),
    )
}

#[tokio::test]
async fn curation_router_scoped_discovery_history_and_route_confinement() {
    let f = fixture("curation_routes", true).await;
    let bearer = token(&f, None).await;
    let path = format!("/api/v1/catalog-curation/services/{}/skills", f.service.id);
    let (status, listing) = request(
        &f.state,
        "GET",
        "/api/v1/catalog-curation/services",
        &bearer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(listing["services"].as_array().unwrap().len(), 1);
    assert_eq!(listing["services"][0]["id"], f.service.id);
    let write = json!({"base_revision": 0, "request_id": Uuid::new_v4().to_string(), "recommended_skills": ["other-publisher/manual"]});
    let (status, result) = request(&f.state, "PUT", &path, &bearer, Some(write.clone())).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["skills_revision"], 1);
    assert_eq!(
        request(&f.state, "PUT", &path, &bearer, Some(write))
            .await
            .0,
        StatusCode::OK
    );
    let (status, history) =
        request(&f.state, "GET", &format!("{path}/history"), &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["history"].as_array().unwrap().len(), 1);
    for forbidden in [
        "client_secret",
        "credential_encrypted",
        "client_secret_hash",
        "secret_prefix",
    ] {
        assert!(!history.to_string().contains(forbidden));
        assert!(!listing.to_string().contains(forbidden));
    }
    let other = Uuid::new_v4();
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/catalog-curation/services/{other}/skills"),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for path in [
        format!("/api/v1/proxy/{other}/packages"),
        format!("/api/v1/proxy/{}/packages?_nyxid_via={other}", f.service.id),
        format!("/api/v1/proxy/s/{}/packages", f.service.slug),
        "/api/v1/proxy/services".into(),
        "/api/v1/llm/openai/v1/models".into(),
        "/api/v1/llm/status".into(),
        "/api/v1/llm/gateway/v1/models".into(),
        "/api/v1/providers".into(),
        "/api/v1/connections".into(),
        "/api/v1/nodes".into(),
        "/api/v1/oracle/pools".into(),
        "/api/v1/triggers".into(),
        "/api/v1/services".into(),
        "/api/v1/catalog".into(),
    ] {
        let (status, body) = request(&f.state, "GET", &path, &bearer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
    let req = Request::builder()
        .uri(format!("/api/v1/proxy/{}/packages", f.service.id))
        .header("authorization", format!("Bearer {bearer}"))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router(&f.state).oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&f.state, "GET", &path, &f.human_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let readonly = token(&f, Some("catalog:skills:read")).await;
    assert_eq!(request(&f.state, "PUT", &path, &readonly, Some(json!({"base_revision":1,"request_id":Uuid::new_v4().to_string(),"recommended_skills":[]}))).await.0, StatusCode::FORBIDDEN);
    let claims = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &bearer).unwrap();
    for change in [
        doc! {"scope":"proxy"},
        doc! {"service_account_id":other.to_string()},
        doc! {"revoked":true},
        doc! {"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(1))},
    ] {
        let saved = f
            .state
            .db
            .collection::<Document>(TOKENS)
            .find_one(doc! {"jti":&claims.jti})
            .await
            .unwrap()
            .unwrap();
        f.state
            .db
            .collection::<Document>(TOKENS)
            .update_one(doc! {"jti":&claims.jti}, doc! {"$set":change})
            .await
            .unwrap();
        assert_eq!(
            request(&f.state, "GET", &path, &bearer, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        f.state
            .db
            .collection::<Document>(TOKENS)
            .replace_one(doc! {"jti":&claims.jti}, saved)
            .await
            .unwrap();
    }
    f.state
        .db
        .collection::<Document>(TOKENS)
        .delete_one(doc! {"jti":&claims.jti})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn curation_grant_platform_custody_and_revoked_account_management() {
    let f = fixture("curation_custody", false).await;
    let path = format!("/api/v1/admin/service-accounts/{}", f.sa.id);
    for field in [
        "curation_grant",
        "purpose",
        "platform_protected",
        "credential_generation",
    ] {
        let mut body = json!({"name":"Injected", "allowed_scopes":"proxy"});
        body[field] = json!("curation");
        for (method, path) in [
            ("PUT", path.as_str()),
            ("POST", "/api/v1/admin/service-accounts"),
        ] {
            assert_eq!(
                request(&f.state, method, path, &f.human_token, Some(body.clone()))
                    .await
                    .0,
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
    }
    let grant_path = format!("{path}/curation-grant");
    let body = json!({"service_ids":[f.service.id],"ornn_proxy_service_id":f.service.id,"max_writes":10,"window_seconds":3600});
    let (status, response) = request(
        &f.state,
        "POST",
        &grant_path,
        &f.human_token,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["platform_protected"], true);
    assert!(!response.to_string().contains("client_secret"));
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"is_admin":false,"role_ids":[]}},
        )
        .await
        .unwrap();
    for (method, route, body) in [
        ("POST", grant_path.clone(), Some(body)),
        ("PUT", path.clone(), Some(json!({"name":"Takeover"}))),
        ("POST", format!("{path}/rotate-secret"), None),
        ("DELETE", grant_path.clone(), None),
    ] {
        assert_eq!(
            request(&f.state, method, &route, &f.human_token, body)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"is_admin":true,"role_ids":[crate::services::role_service::get_platform_role_ids(&f.state.db).await.unwrap().admin]}})
        .await
        .unwrap();
    let bearer = token(&f, None).await;
    assert_eq!(
        request(&f.state, "DELETE", &grant_path, &f.human_token, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f.state,
            "GET",
            "/api/v1/catalog-curation/services",
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, response) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"name":"Disabled curator", "is_active":false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let stored = accounts::get_service_account(&f.state.db, &f.sa.id)
        .await
        .unwrap();
    assert!(stored.platform_protected);
    assert_eq!(
        stored.purpose,
        crate::models::service_account::ServiceAccountPurpose::Curation
    );
    assert!(!stored.is_active);
    assert!(stored.curation_grant.is_none());
    assert!(
        accounts::update_service_account(
            &f.state.db,
            &f.sa.id,
            Some("stale-owner"),
            None,
            None,
            None,
            None,
            None,
            false
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn curation_late_old_generation_issuance_fails_real_extractor_and_legacy_fence() {
    let mut f = fixture("curation_rotation", true).await;
    let before = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let issuance = accounts::ISSUANCE_BARRIER.scope(
        (before.clone(), resume.clone()),
        accounts::authenticate_client_credentials(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &f.sa.client_id,
            &f.secret,
            None,
        ),
    );
    let rotate = async {
        before.wait().await;
        let result = accounts::rotate_secret(&f.state.db, &f.sa.id, true)
            .await
            .unwrap();
        resume.wait().await;
        result
    };
    let (late, (_, secret)) = tokio::join!(issuance, rotate);
    let late = late.unwrap().access_token;
    let claims = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &late).unwrap();
    let row = f
        .state
        .db
        .collection::<ServiceAccountToken>(TOKENS)
        .find_one(doc! {"jti":&claims.jti})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !row.revoked,
        "late token inserted after rotation's revoke sweep"
    );
    assert_eq!(row.credential_generation, 0);
    assert_eq!(
        request(
            &f.state,
            "GET",
            "/api/v1/catalog-curation/services",
            &late,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        accounts::authenticate_client_credentials(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &f.sa.client_id,
            &f.secret,
            None
        )
        .await
        .is_err()
    );
    f.secret = secret;
    let fresh = token(&f, None).await;
    assert_eq!(
        request(
            &f.state,
            "GET",
            "/api/v1/catalog-curation/services",
            &fresh,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut claims = serde_json::to_value(
        jwt::verify_token(&f.state.jwt_keys, &f.state.config, &fresh).unwrap(),
    )
    .unwrap();
    claims.as_object_mut().unwrap().remove("sgen");
    let legacy = jsonwebtoken::encode(
        &jsonwebtoken::decode_header(&fresh).unwrap(),
        &claims,
        &f.state.jwt_keys.encoding,
    )
    .unwrap();
    f.state
        .db
        .collection::<Document>(TOKENS)
        .update_one(
            doc! {"jti":claims["jti"].as_str().unwrap()},
            doc! {"$unset":{"credential_generation":""}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            "/api/v1/catalog-curation/services",
            &legacy,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":&f.sa.id},
            doc! {"$unset":{"credential_generation":""}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            "/api/v1/catalog-curation/services",
            &legacy,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn curation_proxy_uses_catalog_endpoint_and_only_dedicated_sa_credentials() {
    use crate::models::{
        provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
        service_provider_requirement::{
            COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
        },
        user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
        user_provider_token::{COLLECTION_NAME as PROVIDER_TOKENS, UserProviderToken},
        user_service::{COLLECTION_NAME as USER_SERVICES, UserService},
        user_service_connection::{COLLECTION_NAME as CONNECTIONS, UserServiceConnection},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    let mut f = fixture("curation_proxy_binding", true).await;
    let catalog = MockServer::start().await;
    let owner_endpoint = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/packages"))
        .and(header("authorization", "Bearer dedicated-sa-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"credential":"dedicated-sa"})),
        )
        .expect(1)
        .mount(&catalog)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&owner_endpoint)
        .await;
    f.service.base_url = catalog.uri();
    f.service.auth_method = "bearer".into();
    f.service.requires_user_credential = true;
    f.service.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"master-key-must-not-be-used")
        .await
        .unwrap();
    f.state
        .db
        .collection::<DownstreamService>(SERVICES)
        .replace_one(doc! {"_id":&f.service.id}, &f.service)
        .await
        .unwrap();
    let endpoint = test_utils::test_user_endpoint(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "owner override",
        &owner_endpoint.uri(),
        None,
        Some(&f.service.id),
    );
    let instance = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        &f.service.slug,
        &endpoint.id,
        Some(&f.service.id),
        Some(&Uuid::new_v4().to_string()),
    );
    f.state
        .db
        .collection::<UserEndpoint>(ENDPOINTS)
        .insert_one(endpoint)
        .await
        .unwrap();
    f.state
        .db
        .collection::<UserService>(USER_SERVICES)
        .insert_one(instance)
        .await
        .unwrap();
    for (owner, key) in [
        (&f.owner, "owner-broad-key"),
        (&f.sa.id, "dedicated-sa-key"),
    ] {
        f.state
            .db
            .collection::<UserServiceConnection>(CONNECTIONS)
            .insert_one(UserServiceConnection {
                id: Uuid::new_v4().to_string(),
                user_id: owner.clone(),
                service_id: f.service.id.clone(),
                credential_encrypted: Some(
                    f.state
                        .encryption_keys
                        .encrypt(key.as_bytes())
                        .await
                        .unwrap(),
                ),
                credential_type: Some("bearer".into()),
                credential_label: None,
                metadata: None,
                is_active: true,
                state_version: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
    }
    let bearer = token(&f, None).await;
    let route = format!("/api/v1/proxy/{}/packages", f.service.id);
    let (status, body) = request(&f.state, "GET", &route, &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["credential"], "dedicated-sa");
    // A missing SA credential must not fall back to either owner or master.
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .update_one(
            doc! {"user_id":&f.sa.id},
            doc! {"$set":{"credential_encrypted":bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &route, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
    // A provider-delegated Ornn credential is resolved with the SA UUID too.
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&f.service.id},
            doc! {"$set":{"auth_method":"none","requires_user_credential":false}},
        )
        .await
        .unwrap();
    let provider: ProviderConfig = bson::from_document(doc! {"_id":Uuid::new_v4().to_string(),"slug":"ornn-provider","name":"Ornn","provider_type":"api_key","is_active":true,"created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
    f.state
        .db
        .collection::<ProviderConfig>(PROVIDERS)
        .insert_one(&provider)
        .await
        .unwrap();
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_one(ServiceProviderRequirement {
            id: Uuid::new_v4().to_string(),
            service_id: f.service.id.clone(),
            provider_config_id: provider.id.clone(),
            required: true,
            scopes: None,
            injection_method: "bearer".into(),
            injection_key: Some("Authorization".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    for (owner, key) in [
        (&f.owner, "owner-broad-provider-key"),
        (&f.sa.id, "dedicated-sa-provider-key"),
    ] {
        f.state
            .db
            .collection::<UserProviderToken>(PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: Uuid::new_v4().to_string(),
                user_id: owner.clone(),
                provider_config_id: provider.id.clone(),
                connection_id: None,
                credential_user_id: None,
                token_type: "api_key".into(),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                api_key_encrypted: Some(
                    f.state
                        .encryption_keys
                        .encrypt(key.as_bytes())
                        .await
                        .unwrap(),
                ),
                status: "active".into(),
                state_version: 0,
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: Some(owner_endpoint.uri()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
    }
    Mock::given(method("GET"))
        .and(path("/packages"))
        .and(header("authorization", "Bearer dedicated-sa-provider-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"credential":"dedicated-provider"})),
        )
        .expect(1)
        .mount(&catalog)
        .await;
    let (status, body) = request(&f.state, "GET", &route, &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["credential"], "dedicated-provider");
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .update_one(doc! {"user_id":&f.sa.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &route, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(catalog.received_requests().await.unwrap().len(), 2);
    assert_eq!(owner_endpoint.received_requests().await.unwrap().len(), 0);
}

#[tokio::test]
async fn curation_introspection_requires_current_persisted_token() {
    let f = fixture("curation_introspection", true).await;
    let (client, secret) = crate::services::oauth_client_service::create_client(
        &f.state.db,
        "Inspector",
        &["https://example.test/callback".into()],
        "confidential",
        &f.owner,
        "",
        "openid",
        Default::default(),
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    let bearer = token(&f, None).await;
    let claims = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &bearer).unwrap();
    async fn introspect(f: &Fixture, client: &str, secret: &str, bearer: &str) -> Value {
        let body = format!(
            "client_id={}&client_secret={}&token={}",
            urlencoding::encode(client),
            urlencoding::encode(secret),
            urlencoding::encode(bearer)
        );
        let req = Request::builder()
            .method("POST")
            .uri("/oauth/introspect")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let response = router(&f.state).oneshot(req).await.unwrap();
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap()
    }
    let secret = secret.unwrap();
    assert_eq!(
        introspect(&f, &client.id, &secret, &bearer).await["active"],
        true
    );
    f.state
        .db
        .collection::<Document>(TOKENS)
        .delete_one(doc! {"jti":&claims.jti})
        .await
        .unwrap();
    assert_eq!(
        introspect(&f, &client.id, &secret, &bearer).await["active"],
        false
    );
    let fresh = token(&f, None).await;
    accounts::rotate_secret(&f.state.db, &f.sa.id, true)
        .await
        .unwrap();
    assert_eq!(
        introspect(&f, &client.id, &secret, &fresh).await["active"],
        false
    );
}

#[tokio::test]
async fn curation_human_mixed_retry_nullable_noop_and_postcommit_oidc_repair() {
    use crate::models::{
        catalog_skill_revision::{COLLECTION_NAME as HISTORY, OPERATIONS},
        oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient},
        user_service::{COLLECTION_NAME as INSTANCES, UserService},
    };
    let f = fixture("curation_human_mixed", true).await;
    let route = format!("/api/v1/services/{}", f.service.id);
    let request_id = Uuid::new_v4().to_string();
    let body = json!({"name":"Mixed edit","recommended_skills":["one"],"skills_revision":0,"skills_request_id":request_id});
    let (status, response) =
        request(&f.state, "PUT", &route, &f.human_token, Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["skills_revision"], 1);
    let mut different = body.clone();
    different["default_request_headers"] = Value::Null;
    assert_eq!(
        request(&f.state, "PUT", &route, &f.human_token, Some(different))
            .await
            .0,
        StatusCode::CONFLICT,
        "omitted headers and explicit null have distinct effects"
    );
    let instance = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "instance",
        &Uuid::new_v4().to_string(),
        Some(&f.service.id),
        None,
    );
    f.state
        .db
        .collection::<UserService>(INSTANCES)
        .insert_one(&instance)
        .await
        .unwrap();
    let before = f
        .state
        .db
        .collection::<Document>(SERVICES)
        .find_one(doc! {"_id":&f.service.id})
        .await
        .unwrap()
        .unwrap();
    let instance_before = f
        .state
        .db
        .collection::<Document>(INSTANCES)
        .find_one(doc! {"_id":&instance.id})
        .await
        .unwrap()
        .unwrap();
    // Description null means omitted; URL empty strings normalize to null.
    // An empty description string would be a real metadata edit.
    let noop = json!({"name":"Mixed edit","description":null,"homepage_url":"","repository_url":"","identity_propagation_mode":"none","identity_include_user_id":false,"forward_access_token":false,"recommended_skills":["one"],"skills_revision":1,"skills_request_id":Uuid::new_v4().to_string()});
    let (status, response) = request(&f.state, "PUT", &route, &f.human_token, Some(noop)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        f.state
            .db
            .collection::<Document>(SERVICES)
            .find_one(doc! {"_id":&f.service.id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(INSTANCES)
            .find_one(doc! {"_id":&instance.id})
            .await
            .unwrap()
            .unwrap(),
        instance_before
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    // Unchanged skills plus changed metadata still has a durable request outcome.
    let mixed = json!({"name":"Metadata-only effect","recommended_skills":["one"],"skills_revision":1,"skills_request_id":Uuid::new_v4().to_string()});
    assert_eq!(
        request(&f.state, "PUT", &route, &f.human_token, Some(mixed.clone()))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &route,
            &f.human_token,
            Some(json!({"name":"Later metadata"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, response) =
        request(&f.state, "PUT", &route, &f.human_token, Some(mixed.clone())).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["name"], "Later metadata");
    let mut conflict = mixed;
    conflict["name"] = json!("Different retry");
    assert_eq!(
        request(&f.state, "PUT", &route, &f.human_token, Some(conflict))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (client, _) = crate::services::oauth_client_service::create_client(
        &f.state.db,
        "Downstream",
        &["https://old.test/callback".into()],
        "public",
        &f.owner,
        "",
        "openid",
        Default::default(),
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>(CLIENTS)
        .update_one(doc! {"_id":&client.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&f.service.id},
            doc! {"$set":{"oauth_client_id":&client.id}},
        )
        .await
        .unwrap();
    let body = json!({"base_url":"https://new.test","recommended_skills":["two"],"skills_revision":1,"skills_request_id":Uuid::new_v4().to_string()});
    let (status, response) =
        request(&f.state, "PUT", &route, &f.human_token, Some(body.clone())).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "injected postcommit inactive OIDC failure: {response}"
    );
    let committed = f
        .state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&f.service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed.skills_revision, 2);
    assert_eq!(committed.base_url, "https://new.test");
    f.state
        .db
        .collection::<Document>(CLIENTS)
        .update_one(doc! {"_id":&client.id}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "PUT", &route, &f.human_token, Some(body.clone()))
            .await
            .0,
        StatusCode::OK
    );
    let read_client = async {
        f.state
            .db
            .collection::<OauthClient>(CLIENTS)
            .find_one(doc! {"_id":&client.id})
            .await
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        read_client.await.redirect_uris,
        vec!["https://new.test/callback"]
    );
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &route,
            &f.human_token,
            Some(json!({"base_url":"https://latest.test"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&f.state, "PUT", &route, &f.human_token, Some(body))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.state
            .db
            .collection::<OauthClient>(CLIENTS)
            .find_one(doc! {"_id":&client.id})
            .await
            .unwrap()
            .unwrap()
            .redirect_uris,
        vec!["https://latest.test/callback"]
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(HISTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn curation_human_create_handler_converges_on_winner_and_late_slug_receipt() {
    use crate::services::api_key_mutation_service::TransactionCollisionHook;
    let f = fixture("curation_create_router", false).await;
    let input = json!({"name":"Concurrent created","slug":"concurrent-created","base_url":"https://example.test","auth_method":"none","recommended_skills":["one"],"skills_request_id":Uuid::new_v4().to_string()});
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let a = TransactionCollisionHook::new(barrier.clone());
    let b = TransactionCollisionHook::new(barrier);
    let (a, b) = tokio::join!(
        catalog_skill_service::COLLISION.scope(
            a,
            request(
                &f.state,
                "POST",
                "/api/v1/services",
                &f.human_token,
                Some(input.clone())
            )
        ),
        catalog_skill_service::COLLISION.scope(
            b,
            request(
                &f.state,
                "POST",
                "/api/v1/services",
                &f.human_token,
                Some(input.clone())
            )
        )
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["id"], b.1["id"]);
    assert_eq!(
        f.state
            .db
            .collection::<Document>(SERVICES)
            .count_documents(doc! {"slug":"concurrent-created"})
            .await
            .unwrap(),
        1
    );
    let mut input = input;
    input["slug"] = json!("late-slug");
    input["skills_request_id"] = json!(Uuid::new_v4().to_string());
    let read = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let late = super::services::CREATE_BEFORE_SLUG.scope(
        (read.clone(), resume.clone()),
        request(
            &f.state,
            "POST",
            "/api/v1/services",
            &f.human_token,
            Some(input.clone()),
        ),
    );
    let winner = async {
        read.wait().await;
        let result = request(
            &f.state,
            "POST",
            "/api/v1/services",
            &f.human_token,
            Some(input.clone()),
        )
        .await;
        resume.wait().await;
        result
    };
    let (late, winner) = tokio::join!(late, winner);
    assert_eq!(winner.0, StatusCode::OK, "{}", winner.1);
    assert_eq!(late.0, StatusCode::OK, "{}", late.1);
    assert_eq!(late.1["id"], winner.1["id"]);
}

#[tokio::test]
async fn curation_router_rejects_other_credential_classes_and_live_grant_changes() {
    use crate::crypto::token::hash_token;
    let f = fixture("curation_wrong_classes", true).await;
    let route = "/api/v1/catalog-curation/services";
    let raw_key = format!("nyxid_{}", Uuid::new_v4().simple());
    f.state.db.collection::<Document>("api_keys").insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"name":"test","key_prefix":"nyxid_test","key_hash":hash_token(&raw_key),"scopes":"proxy account:read catalog:skills:read", "is_active":true,"created_at":bson::DateTime::now()}).await.unwrap();
    let req = Request::builder()
        .uri(route)
        .header("x-api-key", &raw_key)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router(&f.state).oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let session_token = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<crate::models::session::Session>("sessions")
        .insert_one(crate::models::session::Session {
            id: Uuid::new_v4().to_string(),
            user_id: f.owner.clone(),
            token_hash: hash_token(&session_token),
            ip_address: None,
            user_agent: None,
            expires_at: Utc::now() + Duration::hours(1),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        })
        .await
        .unwrap();
    let req = Request::builder()
        .uri(route)
        .header("cookie", format!("nyx_session={session_token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router(&f.state).oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let original = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &f.human_token).unwrap();
    for marker in [
        json!({"delegated":true,"act":{"sub":Uuid::new_v4().to_string()}}),
        json!({"relay":true}),
    ] {
        let mut claims = serde_json::to_value(&original).unwrap();
        claims
            .as_object_mut()
            .unwrap()
            .extend(marker.as_object().unwrap().clone());
        claims["scope"] = json!("account:read catalog:skills:read");
        let token = jsonwebtoken::encode(
            &jsonwebtoken::decode_header(&f.human_token).unwrap(),
            &claims,
            &f.state.jwt_keys.encoding,
        )
        .unwrap();
        assert_eq!(
            request(&f.state, "GET", route, &token, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let bearer = token(&f, None).await;
    let account = f
        .state
        .db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id":&f.sa.id})
        .await
        .unwrap()
        .unwrap();
    for mutation in [
        doc! {"$unset":{"curation_grant":""}},
        doc! {"$set":{"curation_grant.expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(1))}},
        doc! {"$set":{"is_active":false}},
        doc! {"$set":{"allowed_scopes":"proxy"}},
    ] {
        f.state
            .db
            .collection::<Document>(ACCOUNTS)
            .update_one(doc! {"_id":&f.sa.id}, mutation)
            .await
            .unwrap();
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED),
            "{status} {body}"
        );
        f.state
            .db
            .collection::<Document>(ACCOUNTS)
            .replace_one(doc! {"_id":&f.sa.id}, &account)
            .await
            .unwrap();
    }
    let skill_route = format!("{route}/{}/skills", f.service.id);
    assert_eq!(request(&f.state,"PUT",&skill_route,&bearer,Some(json!({"base_revision":0,"request_id":Uuid::new_v4().to_string(),"recommended_skills":[],"base_url":"https://evil.test"}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn curation_org_owner_role_injection_and_self_grant_are_denied() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    let f = fixture("curation_org_custody", false).await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_utils::test_membership(
            &org,
            &f.owner,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(doc! {"_id":&f.sa.id}, doc! {"$set":{"owner_user_id":&org}})
        .await
        .unwrap();
    let path = format!("/api/v1/admin/service-accounts/{}", f.sa.id);
    let body = json!({"service_ids":[f.service.id],"ornn_proxy_service_id":f.service.id,"max_writes":10,"window_seconds":3600});
    assert_eq!(
        request(
            &f.state,
            "POST",
            &format!("{path}/curation-grant"),
            &f.human_token,
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "even platform admin cannot grant org-owned SA"
    );
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"is_admin":false,"role_ids":[]}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &path,
            &f.human_token,
            Some(json!({"role_ids":["platform-admin"]}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(request(&f.state,"POST","/api/v1/admin/service-accounts",&f.human_token,Some(json!({"name":"Role injection","allowed_scopes":"proxy","target_org_id":org,"role_ids":["platform-admin"]}))).await.0,StatusCode::FORBIDDEN);
    assert_eq!(
        request(
            &f.state,
            "POST",
            &format!("{path}/curation-grant"),
            &f.human_token,
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let bearer = token(&f, None).await;
    assert_eq!(
        request(
            &f.state,
            "POST",
            &format!("{path}/curation-grant"),
            &bearer,
            Some(body)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
