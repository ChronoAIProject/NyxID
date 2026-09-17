use super::build_router_with_state;
use crate::AppState;
use crate::crypto::jwt;
use crate::models::api_key::{ApiKey, COLLECTION_NAME as KEYS};
use crate::models::downstream_service::{
    COLLECTION_NAME as CATALOG, DownstreamService, PlatformKeyAudience, PlatformKeyConfig,
};
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_api_key::{COLLECTION_NAME as CREDENTIALS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as SERVICES, UserService};
use crate::services::key_service::{self, CreatedApiKey};
use crate::services::user_api_key_service::{self, CreateApiKeyParams};
use crate::test_utils::{
    connect_test_database, test_app_state, test_auto_connected_catalog_service, test_membership,
    test_user, test_user_endpoint, test_user_service,
};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use mongodb::bson::doc;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    state: AppState,
    app: Router,
    person: String,
    org: String,
    personal: UserService,
    shared: UserService,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let db = connect_test_database(name)
            .await
            .expect("service inventory router tests require MongoDB; tests must not skip");
        db.run_command(doc! { "ping": 1 }).await.unwrap();
        eprintln!("MongoDB connected; executing service inventory router test: {name}");
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&person, UserType::Person),
                test_user(&org, UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(&org, &person, OrgRole::Member, None))
            .await
            .unwrap();
        let state = test_app_state(db);
        let personal = Self::insert_service(&state, &person, "personal").await;
        let shared = Self::insert_service(&state, &org, "shared").await;
        let (_, private) = build_router_with_state(state.clone());
        let app = private.with_state(state.clone());
        Self {
            state,
            app,
            person,
            org,
            personal,
            shared,
        }
    }

    async fn insert_service(state: &AppState, owner: &str, slug: &str) -> UserService {
        let endpoint_id = Uuid::new_v4().to_string();
        state
            .db
            .collection::<UserEndpoint>(ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                owner,
                slug,
                "https://example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        let credential = user_api_key_service::create_api_key(
            &state.db,
            &state.encryption_keys,
            owner,
            CreateApiKeyParams {
                label: slug,
                credential_type: "api_key",
                credential: "inventory-test-credential",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        let mut service = test_user_service(
            &Uuid::new_v4().to_string(),
            owner,
            slug,
            &endpoint_id,
            None,
            None,
        );
        service.api_key_id = Some(credential.id);
        service.auth_method = "bearer".into();
        state
            .db
            .collection::<UserService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        service
    }

    async fn key(&self, owner: &str, allowed: Option<&[String]>, auto: bool) -> CreatedApiKey {
        key_service::create_api_key_with_scope_authorization(
            &self.state.db,
            owner,
            Some(owner),
            "inventory reader",
            "proxy",
            None,
            None,
            allowed,
            Some(&[]),
            Some(allowed.is_none()),
            Some(auto),
            Some(false),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .unwrap()
    }

    fn read_paths(&self) -> Vec<String> {
        vec![
            "/api/v1/keys".into(),
            format!("/api/v1/keys/{}", self.personal.id),
            format!("/api/v1/keys/{}/authorization", self.personal.id),
            "/api/v1/user-services".into(),
            "/api/v1/endpoints".into(),
            format!(
                "/api/v1/endpoints/{}/authorization",
                self.personal.endpoint_id
            ),
            format!(
                "/api/v1/endpoints/{}/openapi-endpoints",
                self.personal.endpoint_id
            ),
            "/api/v1/api-keys/external".into(),
            format!(
                "/api/v1/api-keys/external/{}/authorization",
                self.personal.api_key_id.as_deref().unwrap()
            ),
        ]
    }

    fn human_routes(&self) -> Vec<(&'static str, String)> {
        let service = &self.personal.id;
        let endpoint = &self.personal.endpoint_id;
        let external = self.personal.api_key_id.as_deref().unwrap();
        vec![
            ("POST", "/api/v1/keys".into()),
            ("PUT", format!("/api/v1/keys/{service}")),
            ("DELETE", format!("/api/v1/keys/{service}")),
            ("PUT", format!("/api/v1/user-services/{service}")),
            ("DELETE", format!("/api/v1/user-services/{service}")),
            (
                "PATCH",
                format!("/api/v1/user-services/{service}/ssh-auth-mode"),
            ),
            ("PUT", format!("/api/v1/endpoints/{endpoint}")),
            ("DELETE", format!("/api/v1/endpoints/{endpoint}")),
            (
                "POST",
                "/api/v1/api-keys/external/gcp-service-account".into(),
            ),
            ("PUT", format!("/api/v1/api-keys/external/{external}")),
            ("DELETE", format!("/api/v1/api-keys/external/{external}")),
            ("GET", "/api/v1/api-keys".into()),
            ("GET", format!("/api/v1/api-keys/{}", Uuid::new_v4())),
            ("POST", "/api/v1/api-keys".into()),
        ]
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        credential: Option<(&str, &str)>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some((header, value)) = credential {
            request = request.header(header, value);
        }
        let body = if let Some(body) = body {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        } else {
            Body::empty()
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, headers, body)
    }

    async fn platform_catalog(&self) -> DownstreamService {
        let mut catalog = test_auto_connected_catalog_service();
        catalog.auth_method = "bearer".into();
        catalog.credential_encrypted = self
            .state
            .encryption_keys
            .encrypt(b"inventory-platform-test")
            .await
            .unwrap();
        catalog.platform_key = Some(PlatformKeyConfig {
            enabled: true,
            audience: PlatformKeyAudience::Public,
            allowed_owner_ids: vec![],
        });
        self.state
            .db
            .collection::<DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        catalog
    }
}

#[tokio::test]
async fn non_api_key_authentication_classes_preserve_inventory_behavior() {
    let f = Fixture::new("inventory_auth_classes").await;
    let catalog = f.platform_catalog().await;
    for (scope, expected) in [
        ("account:read", StatusCode::OK),
        ("proxy:*", StatusCode::FORBIDDEN),
    ] {
        let token = jwt::generate_delegated_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &Uuid::parse_str(&f.person).unwrap(),
            scope,
            "inventory-reader",
            60,
            None,
        )
        .unwrap();
        let bearer = format!("Bearer {token}");
        for path in f.read_paths() {
            let (status, _, body) = f
                .request("GET", &path, Some(("authorization", &bearer)), None)
                .await;
            assert_eq!(status, expected, "delegated {scope}, {path}: {body}");
        }
        for (method, path) in f
            .human_routes()
            .into_iter()
            .filter(|(method, _)| *method != "GET")
        {
            let (status, _, body) = f
                .request(method, &path, Some(("authorization", &bearer)), None)
                .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "delegated {method} {path}: {body}"
            );
        }
    }
    assert_eq!(
        f.state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {"user_id": &f.person, "catalog_service_id": &catalog.id})
            .await
            .unwrap(),
        1,
        "delegated listing must retain provisioning"
    );
    let (sa, _) = jwt::generate_service_account_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::new_v4().to_string(),
        "account:read",
        60,
        0,
    )
    .unwrap();
    let key = f.key(&f.person, None, false).await;
    let relay = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.person).unwrap(),
        "account:read",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id,
            api_key_name: key.name,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    for token in [sa, relay] {
        let bearer = format!("Bearer {token}");
        for (method, path) in f
            .read_paths()
            .into_iter()
            .map(|p| ("GET", p))
            .chain(f.human_routes())
        {
            let (status, _, body) = f
                .request(method, &path, Some(("authorization", &bearer)), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
        }
    }
    f.state.db.drop().await.unwrap();
}

fn ids(body: &Value, field: &str) -> std::collections::BTreeSet<String> {
    body[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn unrestricted_person_keys_read_inventory_with_membership_scope_and_provenance() {
    use crate::models::org_membership::MemberScopeSource;
    let f = Fixture::new("inventory_person").await;
    let disabled = Fixture::insert_service(&f.state, &f.person, "disabled").await;
    f.state
        .db
        .collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &disabled.id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    let outside = Fixture::insert_service(&f.state, &f.org, "outside-role-scope").await;
    f.state.db.collection::<OrgMembership>(MEMBERSHIPS).update_one(doc! {"member_user_id": &f.person}, doc! {"$set": {"scope_source": mongodb::bson::to_bson(&MemberScopeSource::Inherit).unwrap()}}).await.unwrap();
    crate::services::org_role_scope_service::set_scope(
        &f.state.db,
        &f.org,
        OrgRole::Member,
        Some(vec![f.shared.id.clone()]),
        &f.person,
    )
    .await
    .unwrap();
    let viewer_org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&viewer_org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(
            &viewer_org,
            &f.person,
            OrgRole::Viewer,
            None,
        ))
        .await
        .unwrap();
    let viewer = Fixture::insert_service(&f.state, &viewer_org, "viewer-service").await;
    let key = f.key(&f.person, None, false).await;
    let bearer = format!("Bearer {}", key.full_key);
    for credential in [
        ("x-api-key", key.full_key.as_str()),
        ("authorization", bearer.as_str()),
    ] {
        for path in f.read_paths().into_iter().chain([
            format!("/api/v1/keys/{}", f.personal.slug),
            format!("/api/v1/keys/{}/authorization", f.shared.slug),
        ]) {
            let (status, _, body) = f.request("GET", &path, Some(credential), None).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            if path == "/api/v1/keys" || path == "/api/v1/user-services" {
                let field = if path.ends_with("/keys") {
                    "keys"
                } else {
                    "services"
                };
                let mut expected = vec![f.personal.id.clone(), f.shared.id.clone()];
                if field == "keys" {
                    expected.push(disabled.id.clone());
                }
                assert_eq!(ids(&body, field), expected.into_iter().collect());
                for item in body[field].as_array().unwrap() {
                    let source = &item["credential_source"];
                    if item["id"] == f.shared.id {
                        assert_eq!(source["type"], "org");
                        assert_eq!(source["org_id"], f.org);
                        assert_eq!(source["allowed"], true);
                    } else {
                        assert_eq!(source["type"], "personal");
                    }
                    if item["id"] == disabled.id {
                        assert_eq!(item["is_active"], false);
                    }
                    assert_ne!(item["id"], viewer.id);
                    assert_ne!(item["id"], outside.id);
                }
            }
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn restricted_keys_filter_lists_and_gate_resolved_details() {
    let f = Fixture::new("inventory_restricted").await;
    let other = Fixture::insert_service(&f.state, &f.person, "other-personal").await;
    let key = f
        .key(&f.person, Some(std::slice::from_ref(&f.personal.id)), false)
        .await;
    let bearer = format!("Bearer {}", key.full_key);
    for credential in [
        ("x-api-key", key.full_key.as_str()),
        ("authorization", bearer.as_str()),
    ] {
        for path in f.read_paths() {
            let (status, _, body) = f.request("GET", &path, Some(credential), None).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            let list = match path.as_str() {
                "/api/v1/keys" => Some(("keys", &f.personal.id)),
                "/api/v1/user-services" => Some(("services", &f.personal.id)),
                "/api/v1/endpoints" => Some(("endpoints", &f.personal.endpoint_id)),
                "/api/v1/api-keys/external" => {
                    Some(("api_keys", f.personal.api_key_id.as_ref().unwrap()))
                }
                _ => None,
            };
            if let Some((field, expected)) = list {
                assert_eq!(ids(&body, field), [expected.clone()].into_iter().collect());
            }
        }
        for (service_label, service) in [
            ("other personal service", &other),
            ("shared org service", &f.shared),
        ] {
            for (route_label, path) in [
                ("key by id", format!("/api/v1/keys/{}", service.id)),
                ("key by slug", format!("/api/v1/keys/{}", service.slug)),
                (
                    "key authorization",
                    format!("/api/v1/keys/{}/authorization", service.id),
                ),
                (
                    "endpoint authorization",
                    format!("/api/v1/endpoints/{}/authorization", service.endpoint_id),
                ),
                (
                    "endpoint OpenAPI operations",
                    format!(
                        "/api/v1/endpoints/{}/openapi-endpoints",
                        service.endpoint_id
                    ),
                ),
                (
                    "external credential authorization",
                    format!(
                        "/api/v1/api-keys/external/{}/authorization",
                        service.api_key_id.as_deref().unwrap()
                    ),
                ),
            ] {
                let (status, _, body) = f.request("GET", &path, Some(credential), None).await;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{service_label} {route_label}: {body}"
                );
                assert_eq!(
                    body["error_code"], 9000,
                    "{service_label} {route_label}: {body}"
                );
            }
        }
        let missing = Uuid::new_v4();
        for path in [
            format!("/api/v1/keys/{missing}"),
            format!("/api/v1/keys/{missing}/authorization"),
            format!("/api/v1/endpoints/{missing}/authorization"),
            format!("/api/v1/endpoints/{missing}/openapi-endpoints"),
            format!("/api/v1/api-keys/external/{missing}/authorization"),
        ] {
            let (status, _, body) = f.request("GET", &path, Some(credential), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        }
    }
    // Empty scope remains restrictive, including orphan resource metadata.
    let empty = f.key(&f.person, Some(&[]), false).await;
    for (path, field) in [
        ("/api/v1/keys", "keys"),
        ("/api/v1/user-services", "services"),
        ("/api/v1/endpoints", "endpoints"),
        ("/api/v1/api-keys/external", "api_keys"),
    ] {
        let (status, _, body) = f
            .request("GET", path, Some(("x-api-key", &empty.full_key)), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert!(body[field].as_array().unwrap().is_empty());
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn restricted_keys_use_effective_auto_connected_scope() {
    let f = Fixture::new("inventory_auto_scope").await;
    f.state
        .db
        .collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &f.personal.id},
            doc! {"$set": {"source": "auto_provision"}},
        )
        .await
        .unwrap();
    let key = f.key(&f.person, Some(&[]), true).await;
    assert!(key.allowed_service_ids.is_empty());
    for path in f.read_paths() {
        let (status, _, body) = f
            .request("GET", &path, Some(("x-api-key", &key.full_key)), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        if path == "/api/v1/keys" || path == "/api/v1/user-services" {
            let field = if path.ends_with("/keys") {
                "keys"
            } else {
                "services"
            };
            assert_eq!(
                ids(&body, field),
                [f.personal.id.clone()].into_iter().collect()
            );
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_keys_and_endpoint_owner_selection_preserve_direct_and_admin_access() {
    let f = Fixture::new("inventory_org_owner").await;
    let org_key = f.key(&f.org, None, false).await;
    let person_key = f.key(&f.person, None, false).await;
    for (path, field, expected) in [
        ("/api/v1/keys", "keys", &f.shared.id),
        ("/api/v1/user-services", "services", &f.shared.id),
        ("/api/v1/endpoints", "endpoints", &f.shared.endpoint_id),
        (
            "/api/v1/api-keys/external",
            "api_keys",
            f.shared.api_key_id.as_ref().unwrap(),
        ),
    ] {
        let (status, _, body) = f
            .request("GET", path, Some(("x-api-key", &org_key.full_key)), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(ids(&body, field), [expected.clone()].into_iter().collect());
        if field == "keys" || field == "services" {
            assert_eq!(body[field][0]["credential_source"]["type"], "personal");
        }
    }
    for (route_label, path) in [
        ("key by id", format!("/api/v1/keys/{}", f.shared.id)),
        (
            "key authorization by slug",
            format!("/api/v1/keys/{}/authorization", f.shared.slug),
        ),
        (
            "endpoint authorization",
            format!("/api/v1/endpoints/{}/authorization", f.shared.endpoint_id),
        ),
        (
            "endpoint OpenAPI operations",
            format!(
                "/api/v1/endpoints/{}/openapi-endpoints",
                f.shared.endpoint_id
            ),
        ),
        (
            "external credential authorization",
            format!(
                "/api/v1/api-keys/external/{}/authorization",
                f.shared.api_key_id.as_deref().unwrap()
            ),
        ),
    ] {
        let (status, _, body) = f
            .request("GET", &path, Some(("x-api-key", &org_key.full_key)), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{route_label}: {body}");
    }
    let other_org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&other_org, UserType::Org))
        .await
        .unwrap();
    for (key, owner, expected) in [
        (&org_key, &f.org, StatusCode::OK),
        (&org_key, &other_org, StatusCode::FORBIDDEN),
        (&person_key, &f.org, StatusCode::FORBIDDEN),
    ] {
        let (status, _, body) = f
            .request(
                "GET",
                &format!("/api/v1/endpoints?org_id={owner}"),
                Some(("x-api-key", &key.full_key)),
                None,
            )
            .await;
        assert_eq!(status, expected, "{body}");
        if status == StatusCode::FORBIDDEN {
            assert_eq!(body["error_code"], 8103);
        }
    }
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            doc! {"member_user_id": &f.person, "org_user_id": &f.org},
            doc! {"$set": {"role": "admin"}},
        )
        .await
        .unwrap();
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/api/v1/endpoints?org_id={}", f.org),
            Some(("x-api-key", &person_key.full_key)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        ids(&body, "endpoints"),
        [f.shared.endpoint_id.clone()].into_iter().collect()
    );
    let scoped = f
        .key(&f.person, Some(std::slice::from_ref(&f.shared.id)), false)
        .await;
    let other = Fixture::insert_service(&f.state, &f.org, "other-org-service").await;
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/api/v1/endpoints?org_id={}", f.org),
            Some(("x-api-key", &scoped.full_key)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        ids(&body, "endpoints"),
        [f.shared.endpoint_id.clone()].into_iter().collect()
    );
    assert!(!ids(&body, "endpoints").contains(&other.endpoint_id));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn api_key_listing_neither_provisions_nor_reconciles_inventory() {
    let f = Fixture::new("inventory_read_only_list").await;
    let catalog = f.platform_catalog().await;
    // This stale automatic row would be deleted by ordinary reconciliation.
    f.state.db.collection::<UserService>(SERVICES).update_one(doc! {"_id": &f.personal.id}, doc! {"$set": {"source": "auto_provision", "catalog_service_id": Uuid::new_v4().to_string()}}).await.unwrap();
    let key = f.key(&f.person, None, false).await;
    let before = f
        .state
        .db
        .collection::<UserService>(SERVICES)
        .count_documents(doc! {})
        .await
        .unwrap();
    for header in ["x-api-key", "authorization"] {
        let value = if header == "authorization" {
            format!("Bearer {}", key.full_key)
        } else {
            key.full_key.clone()
        };
        let (status, _, body) = f
            .request("GET", "/api/v1/keys", Some((header, &value)), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            f.state
                .db
                .collection::<UserService>(SERVICES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            before
        );
        assert_eq!(
            f.state
                .db
                .collection::<UserService>(SERVICES)
                .count_documents(doc! {"catalog_service_id": &catalog.id})
                .await
                .unwrap(),
            0
        );
        assert!(
            f.state
                .db
                .collection::<UserService>(SERVICES)
                .find_one(doc! {"_id": &f.personal.id})
                .await
                .unwrap()
                .is_some()
        );
    }
    let session =
        crate::services::token_service::create_session(&f.state.db, &f.person, None, None)
            .await
            .unwrap();
    let cookie = format!("nyx_session={}", session.session_token);
    let (status, _, body) = f
        .request("GET", "/api/v1/keys", Some(("cookie", &cookie)), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        f.state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {"user_id": &f.person, "catalog_service_id": &catalog.id})
            .await
            .unwrap(),
        1
    );
    assert!(
        f.state
            .db
            .collection::<UserService>(SERVICES)
            .find_one(doc! {"_id": &f.personal.id})
            .await
            .unwrap()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn api_key_detail_reads_leave_pending_oauth_placeholders_untouched() {
    let f = Fixture::new("inventory_read_only_detail").await;
    let id = f.personal.api_key_id.as_deref().unwrap();
    f.state.db.collection::<UserApiKey>(CREDENTIALS).update_one(doc! {"_id": id}, doc! {"$set": {"credential_type": "oauth2", "status": "pending_auth", "provider_config_id": Uuid::new_v4().to_string()}}).await.unwrap();
    let before = f
        .state
        .db
        .collection::<mongodb::bson::Document>(CREDENTIALS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap();
    let key = f.key(&f.person, None, false).await;
    let bearer = format!("Bearer {}", key.full_key);
    for credential in [
        ("x-api-key", key.full_key.as_str()),
        ("authorization", bearer.as_str()),
    ] {
        for suffix in ["", "/authorization"] {
            let path = format!("/api/v1/keys/{}{suffix}", f.personal.id);
            let (status, _, body) = f.request("GET", &path, Some(credential), None).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            let after = f
                .state
                .db
                .collection::<mongodb::bson::Document>(CREDENTIALS)
                .find_one(doc! {"_id": id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                before, after,
                "API-key reads must not reconcile pending placeholders"
            );
        }
    }
    let session =
        crate::services::token_service::create_session(&f.state.db, &f.person, None, None)
            .await
            .unwrap();
    let cookie = format!("nyx_session={}", session.session_token);
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/api/v1/keys/{}", f.personal.id),
            Some(("cookie", &cookie)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = f
        .state
        .db
        .collection::<UserApiKey>(CREDENTIALS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.status, "failed",
        "session reads still reconcile abandoned OAuth"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn inventory_writes_and_nyxid_key_management_remain_human_only() {
    let f = Fixture::new("inventory_human_only").await;
    for owner in [&f.person, &f.org] {
        let key = f.key(owner, None, false).await;
        let bearer = format!("Bearer {}", key.full_key);
        for credential in [
            ("x-api-key", key.full_key.as_str()),
            ("authorization", bearer.as_str()),
        ] {
            for (method, path) in f
                .human_routes()
                .into_iter()
                .chain([("GET", format!("/api/v1/api-keys/{}", key.id))])
            {
                let (status, _, body) = f.request(method, &path, Some(credential), None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
                assert_eq!(
                    body["message"],
                    "Forbidden: API keys cannot access this endpoint"
                );
            }
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn scheduled_invocation_keys_cannot_read_inventory() {
    let f = Fixture::new("inventory_scheduled").await;
    let key = f.key(&f.person, None, false).await;
    f.state
        .db
        .collection::<ApiKey>(KEYS)
        .update_one(
            doc! {"_id": &key.id},
            doc! {"$set": {"purpose": "scheduled_invocation", "scheduled_write_enabled": true}},
        )
        .await
        .unwrap();
    let bearer = format!("Bearer {}", key.full_key);
    for credential in [
        ("x-api-key", key.full_key.as_str()),
        ("authorization", bearer.as_str()),
    ] {
        for path in f.read_paths() {
            let (status, _, body) = f.request("GET", &path, Some(credential), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
            assert_eq!(body["error_code"], 9009);
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn merged_inventory_methods_keep_allow_headers_and_human_updates() {
    let f = Fixture::new("inventory_method_merge").await;
    for (method, path, mut expected) in [
        (
            "PUT",
            "/api/v1/keys".to_string(),
            vec!["GET", "HEAD", "POST"],
        ),
        ("POST", "/api/v1/user-services".into(), vec!["GET", "HEAD"]),
        (
            "PATCH",
            format!("/api/v1/keys/{}", f.personal.id),
            vec!["DELETE", "GET", "HEAD", "PUT"],
        ),
        (
            "POST",
            "/api/v1/api-keys/external".into(),
            vec!["GET", "HEAD"],
        ),
    ] {
        let (status, headers, _) = f.request(method, &path, None, None).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        let mut allowed = headers["allow"]
            .to_str()
            .unwrap()
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>();
        allowed.sort_unstable();
        expected.sort_unstable();
        assert_eq!(allowed, expected);
    }
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.person).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let bearer = format!("Bearer {token}");
    let (status, _, body) = f
        .request(
            "PUT",
            &format!("/api/v1/keys/{}", f.personal.id),
            Some(("authorization", &bearer)),
            Some(json!({"label": "Human updated"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["label"], "Human updated");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn resource_reads_require_intersection_of_key_and_membership_scopes() {
    let f = Fixture::new("inventory_scope_intersection").await;
    let other = Fixture::insert_service(&f.state, &f.org, "other-scope").await;
    let key = f
        .key(&f.person, Some(std::slice::from_ref(&f.shared.id)), false)
        .await;
    // Both services share metadata, but the caller's membership and API key
    // cover different services after a live membership-scope change.
    f.state.db.collection::<UserService>(SERVICES).update_one(doc! {"_id": &other.id}, doc! {"$set": {"endpoint_id": &f.shared.endpoint_id, "api_key_id": &f.shared.api_key_id}}).await.unwrap();
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            doc! {"member_user_id": &f.person, "org_user_id": &f.org},
            doc! {"$set": {"role": "admin", "allowed_service_ids": [&other.id]}},
        )
        .await
        .unwrap();
    for (route_label, path) in [
        (
            "endpoint authorization",
            format!("/api/v1/endpoints/{}/authorization", f.shared.endpoint_id),
        ),
        (
            "endpoint OpenAPI operations",
            format!(
                "/api/v1/endpoints/{}/openapi-endpoints",
                f.shared.endpoint_id
            ),
        ),
        (
            "external credential authorization",
            format!(
                "/api/v1/api-keys/external/{}/authorization",
                f.shared.api_key_id.as_deref().unwrap()
            ),
        ),
    ] {
        let (status, _, body) = f
            .request("GET", &path, Some(("x-api-key", &key.full_key)), None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{route_label}: {body}");
        assert_eq!(body["error_code"], 9000);
    }
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/api/v1/endpoints?org_id={}", f.org),
            Some(("x-api-key", &key.full_key)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["endpoints"], json!([]));
    f.state.db.drop().await.unwrap();
}
