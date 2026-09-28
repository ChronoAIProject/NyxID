use axum::http::StatusCode;
use mongodb::bson::{self, Document, doc};
use serde_json::json;
use uuid::Uuid;

use super::curation_tests::{Fixture, fixture, request, token};
use crate::models::{
    downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
    service_account::COLLECTION_NAME as ACCOUNTS,
    service_account_token::COLLECTION_NAME as TOKENS,
    user_endpoint::COLLECTION_NAME as ENDPOINTS,
    user_service::COLLECTION_NAME as USER_SERVICES,
};
use crate::test_utils;

const READ_PERMISSION: &str = "nyxid:catalog:skills:read";
const WRITE_PERMISSION: &str = "nyxid:catalog:skills:write";
const SCOPES: &str = "catalog:skills:read catalog:skills:write user-services:read proxy";
const ORNN_EDITOR_SCOPES: &str = "catalog:skills:read catalog:skills:write proxy";
const ORNN_PERMISSIONS: [&str; 3] = ["ornn:skill:read", "ornn:skill:create", "ornn:skill:update"];

async fn ornn_account(label: &str) -> (Fixture, wiremock::MockServer, String) {
    let mut f = fixture(label, false).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, role) = request(
        &f.state,
        "POST",
        "/api/v1/admin/roles",
        &f.human_token,
        Some(json!({
            "name": "Ornn publisher",
            "slug": format!("ornn-publisher-{}", Uuid::new_v4()),
            "permissions": ORNN_PERMISSIONS,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let role_id = role["id"].as_str().unwrap().to_owned();
    let (status, saved) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/service-accounts/{}", f.sa.id),
        &f.human_token,
        Some(json!({"allowed_scopes": "proxy", "role_ids": [&role_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["purpose"], "general");

    f.service.slug = "ornn-api".into();
    f.service.base_url = upstream.uri();
    f.service.service_category = "internal".into();
    f.service.auth_method = "none".into();
    f.service.requires_user_credential = false;
    f.service.identity_propagation_mode = "both".into();
    f.service.identity_include_user_id = true;
    f.service.identity_jwt_audience = Some("ornn-scope-save-test".into());
    f.service.proxy_operation_policy = None;
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .replace_one(doc! {"_id": &f.service.id}, &f.service)
        .await
        .unwrap();
    (f, upstream, role_id)
}

async fn save_editor_scopes(f: &Fixture, scopes: &str) {
    let (status, saved) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/service-accounts/{}", f.sa.id),
        &f.human_token,
        Some(json!({"allowed_scopes": scopes})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["purpose"], "catalog_editor");
    assert_eq!(saved["catalog_scope_authorized"], true);
    assert_eq!(saved["platform_protected"], true);
}

fn assert_ornn_identity(f: &Fixture, request: &wiremock::Request, permissions: &[&str]) {
    let assertion = request.headers["x-nyxid-identity-token"].to_str().unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&["ornn-scope-save-test"]);
    validation.set_issuer(&[&f.state.config.jwt_issuer]);
    let claims = jsonwebtoken::decode::<serde_json::Value>(
        assertion,
        &f.state.jwt_keys.decoding,
        &validation,
    )
    .unwrap()
    .claims;
    assert_eq!(claims["sub"], f.sa.id);
    assert_eq!(claims["nyx_service_id"], f.service.id);
    assert_eq!(
        request.headers["x-nyxid-user-id"].to_str().unwrap(),
        f.sa.id
    );
    let mut actual: Vec<String> = serde_json::from_value(claims["permissions"].clone()).unwrap();
    actual.sort();
    let mut expected = permissions.to_vec();
    expected.sort();
    assert_eq!(actual, expected);
}

async fn add_ornn_connection(f: &Fixture, owner: &str, credential: &[u8]) -> String {
    use crate::models::user_service_connection::{
        COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
    };
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .insert_one(UserServiceConnection {
            id: id.clone(),
            user_id: owner.into(),
            service_id: f.service.id.clone(),
            credential_encrypted: Some(f.state.encryption_keys.encrypt(credential).await.unwrap()),
            credential_type: Some("bearer".into()),
            credential_label: None,
            metadata: None,
            is_active: true,
            state_version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    id
}

async fn editor(label: &str) -> (Fixture, String, String) {
    let f = fixture(label, false).await;
    let (status, role) = request(
        &f.state,
        "POST",
        "/api/v1/admin/roles",
        &f.human_token,
        Some(json!({
            "name": "Catalog skill editor",
            "slug": format!("catalog-skill-editor-{}", Uuid::new_v4()),
            "permissions": [READ_PERMISSION, WRITE_PERMISSION],
            "is_default": false,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let role_id = role["id"].as_str().unwrap().to_owned();
    let (status, account) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/service-accounts/{}", f.sa.id),
        &f.human_token,
        Some(json!({"role_ids": [&role_id], "allowed_scopes": SCOPES})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account["purpose"], "catalog_editor");
    assert_eq!(account["platform_protected"], true);
    assert!(account["curation_grant"].is_null());
    // Preserve coverage of existing role-authorized editors after migration.
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"catalog_scope_authorized": false}},
        )
        .await
        .unwrap();
    let bearer = token(&f, None).await;
    (f, role_id, bearer)
}

async fn add_catalog_service(f: &Fixture, slug: &str) -> DownstreamService {
    let mut service = f.service.clone();
    service.id = Uuid::new_v4().to_string();
    service.slug = slug.into();
    service.name = format!("Catalog {slug}");
    service.base_url = "https://hidden-catalog-target.example".into();
    service.credential_encrypted = b"hidden-master-credential".to_vec();
    service.recommended_skills = Some(vec!["ornn/setup".into()]);
    service.recommended_skill_refs = None;
    service.skills_revision = 0;
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .insert_one(&service)
        .await
        .unwrap();
    service
}

async fn add_personal_connection(f: &Fixture) -> String {
    let endpoint_id = Uuid::new_v4().to_string();
    let endpoint = test_utils::test_user_endpoint(
        &endpoint_id,
        &f.owner,
        "Private endpoint",
        "https://private-connection.example",
        None,
        None,
    );
    f.state
        .db
        .collection::<Document>(ENDPOINTS)
        .insert_one(bson::to_document(&endpoint).unwrap())
        .await
        .unwrap();
    let service = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "private-connection",
        &endpoint_id,
        None,
        None,
    );
    let id = service.id.clone();
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .insert_one(bson::to_document(&service).unwrap())
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn editor_reads_current_and_future_catalog_without_connection_grants() {
    let (f, _, bearer) = editor("catalog_editor_reads_all").await;
    let personal_id = add_personal_connection(&f).await;
    let future = add_catalog_service(&f, "future-service").await;

    let (status, listing) = request(&f.state, "GET", "/api/v1/keys", &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let keys = listing["keys"].as_array().unwrap();
    assert!(keys.iter().any(|key| key["id"] == f.service.id));
    assert!(keys.iter().any(|key| key["id"] == future.id));
    assert!(
        keys.iter()
            .all(|key| key["resource_type"] == "catalog_service")
    );
    assert!(!keys.iter().any(|key| key["id"] == personal_id));

    for service in [&f.service, &future] {
        let (status, detail) = request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{}", service.id),
            &bearer,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["id"], service.id);
        assert_eq!(detail["resource_type"], "catalog_service");
        assert!(
            detail["skills_manifest_digest"]
                .as_str()
                .unwrap()
                .starts_with("v1:")
        );
        let expected = [
            "id",
            "slug",
            "name",
            "label",
            "service_type",
            "is_active",
            "catalog_service_id",
            "catalog_service_slug",
            "catalog_service_name",
            "recommended_skills",
            "recommended_skill_refs",
            "skills_revision",
            "skills_manifest_digest",
            "resource_type",
        ];
        assert_eq!(
            detail.as_object().unwrap().len(),
            expected.len(),
            "{detail}"
        );
        assert!(expected.iter().all(|field| detail.get(field).is_some()));
        for forbidden in [
            "hidden-catalog-target",
            "hidden-master-credential",
            "private-connection",
            "credential_encrypted",
            "base_url",
            "default_request_headers",
        ] {
            assert!(
                !detail.to_string().contains(forbidden),
                "{forbidden}: {detail}"
            );
            assert!(
                !listing.to_string().contains(forbidden),
                "{forbidden}: {listing}"
            );
        }
    }
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{personal_id}"),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
    );
    let (status, discovery) = request(
        &f.state,
        "GET",
        "/api/v1/catalog-curation/services",
        &bearer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{discovery}");
    assert!(
        discovery["services"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == future.id)
    );
}

#[tokio::test]
async fn editor_updates_future_catalog_with_revision_and_replay_fences() {
    let (f, _, bearer) = editor("catalog_editor_writes_all").await;
    let future = add_catalog_service(&f, "future-writable-service").await;
    let path = format!("/api/v1/catalog-curation/services/{}/skills", future.id);
    let write = json!({
        "base_revision": 0,
        "request_id": Uuid::new_v4().to_string(),
        "recommended_skills": ["ornn/new-setup"],
    });
    let (status, result) = request(&f.state, "PUT", &path, &bearer, Some(write.clone())).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["skills_revision"], 1);
    assert_eq!(result["recommended_skills"], json!(["ornn/new-setup"]));
    let (status, replay) = request(&f.state, "PUT", &path, &bearer, Some(write)).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["skills_revision"], 1);
    let (status, stale) = request(
        &f.state,
        "PUT",
        &path,
        &bearer,
        Some(json!({
            "base_revision": 0,
            "request_id": Uuid::new_v4().to_string(),
            "recommended_skills": ["ornn/stale"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    let (status, detail) = request(
        &f.state,
        "GET",
        &format!("/api/v1/keys/{}", future.id),
        &bearer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["recommended_skills"], json!(["ornn/new-setup"]));
    assert_eq!(detail["skills_revision"], 1);
}

#[tokio::test]
async fn editor_authority_tracks_live_role_scope_and_token_state() {
    let (f, role_id, bearer) = editor("catalog_editor_live_authority").await;
    let catalog_path = format!("/api/v1/keys/{}", f.service.id);
    let skills_path = format!("/api/v1/catalog-curation/services/{}/skills", f.service.id);
    let write = || {
        json!({
            "base_revision": 0,
            "request_id": Uuid::new_v4().to_string(),
            "recommended_skills": ["ornn/denied"],
        })
    };
    let readonly = token(&f, Some("catalog:skills:read user-services:read")).await;
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &readonly, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&f.state, "PUT", &skills_path, &readonly, Some(write()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let no_key_scope = token(&f, Some("catalog:skills:read")).await;
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &no_key_scope, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &no_key_scope, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    let (status, role) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/roles/{role_id}"),
        &f.human_token,
        Some(json!({"permissions": [READ_PERMISSION]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&f.state, "PUT", &skills_path, &bearer, Some(write()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    let (status, role) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/roles/{role_id}"),
        &f.human_token,
        Some(json!({"permissions": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&f.state, "GET", &skills_path, &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    let (status, role) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/roles/{role_id}"),
        &f.human_token,
        Some(json!({"permissions": [READ_PERMISSION, WRITE_PERMISSION]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let account_path = format!("/api/v1/admin/service-accounts/{}", f.sa.id);
    let (status, account) = request(
        &f.state,
        "PUT",
        &account_path,
        &f.human_token,
        Some(json!({"role_ids": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account["purpose"], "catalog_editor");
    assert_eq!(account["platform_protected"], true);
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, account) = request(
        &f.state,
        "PUT",
        &account_path,
        &f.human_token,
        Some(json!({"role_ids": [role_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account["purpose"], "catalog_editor");
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::OK
    );
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"allowed_scopes": "proxy"}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&f.state, "GET", &skills_path, &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"allowed_scopes": SCOPES}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::OK
    );

    let claims =
        crate::crypto::jwt::verify_token(&f.state.jwt_keys, &f.state.config, &bearer).unwrap();
    f.state
        .db
        .collection::<Document>(TOKENS)
        .update_one(doc! {"jti": &claims.jti}, doc! {"$set": {"revoked": true}})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &catalog_path, &bearer, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn editor_cannot_mutate_keys_or_read_unrelated_account_routes() {
    let (f, _, bearer) = editor("catalog_editor_key_boundaries").await;
    let detail = format!("/api/v1/keys/{}", f.service.id);
    for (method, path) in [
        ("POST", "/api/v1/keys"),
        ("PUT", detail.as_str()),
        ("DELETE", detail.as_str()),
    ] {
        let (status, body) = request(&f.state, method, path, &bearer, Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
    }
    for path in [
        "/api/v1/user-services",
        "/api/v1/endpoints",
        "/api/v1/api-keys/external",
        "/api/v1/catalog",
    ] {
        let (status, body) = request(&f.state, "GET", path, &bearer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys/not-a-uuid", &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for path in [
        format!("/api/v1/proxy/{}/skills", f.service.id),
        "/api/v1/proxy/s/ornn-api/skills".into(),
        "/api/v1/llm/openai/v1/models".into(),
    ] {
        let (status, body) = request(&f.state, "GET", &path, &bearer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
    for path in ["/api/v1/keys", detail.as_str()] {
        assert_eq!(
            request(&f.state, "HEAD", path, &bearer, None).await.0,
            StatusCode::FORBIDDEN
        );
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let (public, private) = crate::routes::build_router_with_state(f.state.clone());
        let response = public
            .merge(private)
            .with_state(f.state.clone())
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {bearer}"))
                    .header("connection", "upgrade")
                    .header("upgrade", "websocket")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn general_account_scope_without_editor_role_stays_unprivileged() {
    let f = fixture("catalog_editor_general_scope", false).await;
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"allowed_scopes": SCOPES}},
        )
        .await
        .unwrap();
    let bearer = token(&f, None).await;
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{}", f.service.id),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
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
}

#[tokio::test]
async fn editor_created_with_role_is_protected_immediately() {
    let f = fixture("catalog_editor_create_custody", false).await;
    let (status, role) = request(
        &f.state,
        "POST",
        "/api/v1/admin/roles",
        &f.human_token,
        Some(json!({
            "name": "Catalog editor at creation",
            "slug": format!("catalog-editor-create-{}", Uuid::new_v4()),
            "permissions": [READ_PERMISSION, WRITE_PERMISSION],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let (status, created) = request(
        &f.state,
        "POST",
        "/api/v1/admin/service-accounts",
        &f.human_token,
        Some(json!({
            "name": "New catalog editor",
            "allowed_scopes": SCOPES,
            "role_ids": [role["id"]],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap();
    let account = f
        .state
        .db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.get_str("purpose").unwrap(), "catalog_editor");
    assert!(account.get_bool("platform_protected").unwrap());
    assert!(matches!(
        account.get("curation_grant"),
        None | Some(bson::Bson::Null)
    ));
}

#[tokio::test]
async fn editor_cannot_be_downgraded_by_legacy_grant_management() {
    let (f, _, bearer) = editor("catalog_editor_legacy_grant_guard").await;
    let grant_path = format!("/api/v1/admin/service-accounts/{}/curation-grant", f.sa.id);
    for (method, expected) in [
        ("POST", StatusCode::CONFLICT),
        ("DELETE", StatusCode::FORBIDDEN),
    ] {
        let payload = (method == "POST").then(|| {
            json!({
                "service_ids": [&f.service.id],
                "max_writes": 100,
                "window_seconds": 3600,
            })
        });
        let (status, body) = request(&f.state, method, &grant_path, &f.human_token, payload).await;
        assert_eq!(status, expected, "{method}: {body}");
    }
    let account = f
        .state
        .db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id": &f.sa.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.get_str("purpose").unwrap(), "catalog_editor");
    assert!(account.get_bool("platform_protected").unwrap());
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &bearer, None)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn catalog_scope_save_preserves_ornn_slug_workflow_without_operation_policy() {
    use crate::models::user_service_connection::{
        COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    let (f, upstream, role_id) = ornn_account("editor_ornn_scope_save").await;
    let proxy_token = token(&f, None).await;
    let validate_path = "/api/v1/proxy/s/ornn-api/api/v1/skill-format/validate";
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"valid": true})))
        .expect(3)
        .mount(&upstream)
        .await;
    let (status, body) = request(
        &f.state,
        "POST",
        validate_path,
        &proxy_token,
        Some(json!({"fixture": "before-scope-save"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "before scope save: {body}");
    assert_eq!(body["valid"], true);
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &proxy_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    let owner_upstream = MockServer::start().await;
    let endpoint = test_utils::test_user_endpoint(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "Creator private Ornn endpoint",
        &owner_upstream.uri(),
        None,
        Some(&f.service.id),
    );
    let private_connection = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "ornn-api",
        &endpoint.id,
        Some(&f.service.id),
        None,
    );
    f.state
        .db
        .collection::<Document>(ENDPOINTS)
        .insert_one(bson::to_document(&endpoint).unwrap())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .insert_one(bson::to_document(&private_connection).unwrap())
        .await
        .unwrap();
    add_ornn_connection(&f, &f.owner, b"creator-private-key").await;

    save_editor_scopes(&f, ORNN_EDITOR_SCOPES).await;
    let saved = f
        .state
        .db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id": &f.sa.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.get_array("role_ids").unwrap(),
        &vec![bson::Bson::String(role_id)]
    );
    let bearer = token(&f, None).await;
    for token in [&proxy_token, &bearer] {
        let (status, body) = request(
            &f.state,
            "POST",
            validate_path,
            token,
            Some(json!({"fixture": "after-scope-save"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "after scope save: {body}");
        assert_eq!(body["valid"], true);
    }
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &proxy_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN,
        "saving live catalog scopes must not add them to an existing proxy-only token"
    );

    let (status, listing) = request(&f.state, "GET", "/api/v1/keys", &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let keys = listing["keys"].as_array().unwrap();
    assert!(keys.iter().any(|key| key["id"] == f.service.id));
    assert!(
        keys.iter()
            .all(|key| key["resource_type"] == "catalog_service")
    );
    assert!(!keys.iter().any(|key| key["id"] == private_connection.id));
    let catalog_path = format!("/api/v1/keys/{}", f.service.id);
    let (status, detail) = request(&f.state, "GET", &catalog_path, &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["catalog_service_id"], f.service.id);
    let (status, assigned) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/catalog-curation/services/{}/skills", f.service.id),
        &bearer,
        Some(json!({
            "base_revision": detail["skills_revision"],
            "request_id": Uuid::new_v4().to_string(),
            "recommended_skills": ["publisher/scope-save-regression"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let (status, detail) = request(&f.state, "GET", &catalog_path, &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["recommended_skills"],
        json!(["publisher/scope-save-regression"])
    );
    assert_eq!(detail["skills_revision"], 1);

    for (verb, skill_path) in [
        ("POST", "/api/v1/skills"),
        ("GET", "/api/v1/skills/owned"),
        ("PUT", "/api/v1/skills/owned"),
    ] {
        Mock::given(method(verb))
            .and(path(skill_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "owned"})))
            .expect(if verb == "GET" { 2 } else { 1 })
            .mount(&upstream)
            .await;
        let (status, body) = request(
            &f.state,
            verb,
            &format!("/api/v1/proxy/s/ornn-api{skill_path}"),
            &bearer,
            (verb != "GET").then(|| json!({"isPrivate": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{verb} {skill_path}: {body}");
    }
    let (status, body) = request(
        &f.state,
        "GET",
        &format!("/api/v1/proxy/{}/api/v1/skills/owned", f.service.id),
        &bearer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "UUID alias: {body}");
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 7);
    for request in &requests {
        assert_ornn_identity(&f, request, &ORNN_PERMISSIONS);
        assert!(
            !request
                .headers
                .values()
                .any(|value| value.as_bytes() == b"Bearer creator-private-key")
        );
    }
    assert!(owner_upstream.received_requests().await.unwrap().is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<UserServiceConnection>(CONNECTIONS)
            .count_documents(doc! {"user_id": &f.sa.id})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn editor_ornn_optional_policy_preserves_explicit_denies_and_live_authority() {
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path},
    };

    let (f, upstream, role_id) = ornn_account("editor_ornn_live_boundaries").await;
    save_editor_scopes(&f, ORNN_EDITOR_SCOPES).await;
    let bearer = token(&f, None).await;
    let routes = [
        "/api/v1/proxy/s/ornn-api/api/v1/skills/owned".to_owned(),
        format!("/api/v1/proxy/{}/api/v1/skills/owned", f.service.id),
    ];
    Mock::given(method("GET"))
        .and(path("/api/v1/skills/owned"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "owned"})))
        .mount(&upstream)
        .await;
    let services = f.state.db.collection::<Document>(CATALOG);
    for policy in [
        json!({"rules": []}),
        json!({"rules": [{"method": "POST", "path_template": "/api/v1/skills"}]}),
    ] {
        services
            .update_one(
                doc! {"_id": &f.service.id},
                doc! {"$set": {"proxy_operation_policy": bson::to_bson(&policy).unwrap()}},
            )
            .await
            .unwrap();
        for route in &routes {
            let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "explicit policy: {body}");
        }
    }
    services
        .update_one(
            doc! {"_id": &f.service.id},
            doc! {"$unset": {"proxy_operation_policy": ""}},
        )
        .await
        .unwrap();
    let no_proxy = token(&f, Some("catalog:skills:read catalog:skills:write")).await;
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &no_proxy, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "token scope: {body}");
    }
    let other = add_catalog_service(&f, "ornn-api-other").await;
    for route in [
        format!("/api/v1/proxy/{}/api/v1/skills/owned", other.id),
        "/api/v1/proxy/s/ornn-api-other/api/v1/skills/owned".into(),
        format!("{}?_nyxid_via={}", routes[0], Uuid::new_v4()),
        format!("{}?_nyxid_via={}", routes[1], Uuid::new_v4()),
        "/api/v1/proxy/services".into(),
    ] {
        let (status, body) = request(&f.state, "GET", &route, &bearer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "target boundary: {body}");
    }
    {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        for route in &routes {
            let (public, private) = crate::routes::build_router_with_state(f.state.clone());
            let response = public
                .merge(private)
                .with_state(f.state.clone())
                .oneshot(
                    Request::builder()
                        .uri(route)
                        .header("authorization", format!("Bearer {bearer}"))
                        .header("connection", "upgrade")
                        .header("upgrade", "websocket")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "WebSocket upgrades must remain forbidden"
            );
        }
    }
    save_editor_scopes(&f, "catalog:skills:read catalog:skills:write").await;
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "live scope: {body}");
    }
    assert!(upstream.received_requests().await.unwrap().is_empty());

    save_editor_scopes(&f, "proxy").await;
    let proxy_only = token(&f, None).await;
    let (status, body) = request(&f.state, "GET", &routes[0], &proxy_only, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "proxy without catalog scopes: {body}"
    );
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, role) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/roles/{role_id}"),
        &f.human_token,
        Some(json!({"permissions": ["ornn:skill:read"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let (status, body) = request(&f.state, "GET", &routes[1], &proxy_only, None).await;
    assert_eq!(status, StatusCode::OK, "live Ornn role: {body}");
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_ornn_identity(&f, &requests[0], &ORNN_PERMISSIONS);
    assert_ornn_identity(&f, &requests[1], &["ornn:skill:read"]);

    let claims =
        crate::crypto::jwt::verify_token(&f.state.jwt_keys, &f.state.config, &proxy_only).unwrap();
    f.state
        .db
        .collection::<Document>(TOKENS)
        .update_one(doc! {"jti": &claims.jti}, doc! {"$set": {"revoked": true}})
        .await
        .unwrap();
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &proxy_only, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "revoked token: {body}");
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn editor_ornn_internal_transport_credential_requires_signed_sa_identity() {
    use crate::models::user_service_connection::COLLECTION_NAME as CONNECTIONS;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{header, method, path},
    };

    let (mut f, upstream, role_id) = ornn_account("editor_ornn_transport").await;
    f.service.auth_method = "bearer".into();
    f.service.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"server-transport-key")
        .await
        .unwrap();
    let services = f.state.db.collection::<Document>(CATALOG);
    let original = bson::to_document(&f.service).unwrap();
    services
        .replace_one(doc! {"_id": &f.service.id}, &original)
        .await
        .unwrap();
    add_ornn_connection(&f, &f.owner, b"creator-private-key").await;
    save_editor_scopes(&f, ORNN_EDITOR_SCOPES).await;
    let bearer = token(&f, None).await;
    let routes = [
        "/api/v1/proxy/s/ornn-api/api/v1/skills/owned".to_owned(),
        format!("/api/v1/proxy/{}/api/v1/skills/owned", f.service.id),
    ];
    for key in ["server-transport-key", "dedicated-sa-key"] {
        Mock::given(method("GET"))
            .and(path("/api/v1/skills/owned"))
            .and(header("authorization", format!("Bearer {key}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "owned"})))
            .expect(2)
            .mount(&upstream)
            .await;
    }
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert_eq!(status, StatusCode::OK, "internal transport: {body}");
    }
    let mut broken_signer = f.state.clone();
    broken_signer.jwt_keys.encoding = jsonwebtoken::EncodingKey::from_secret(b"not-an-rsa-key");
    for route in &routes {
        let (status, body) = request(&broken_signer, "GET", route, &bearer, None).await;
        assert!(status.is_server_error(), "identity signing failure: {body}");
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
    for (label, changed) in [
        ("no identity", doc! {"identity_propagation_mode": "none"}),
        (
            "unsigned identity",
            doc! {"identity_propagation_mode": "headers"},
        ),
        (
            "user credential required",
            doc! {"requires_user_credential": true},
        ),
        (
            "ordinary connection",
            doc! {"service_category": "connection"},
        ),
        (
            "provider credential",
            doc! {"provider_config_id": Uuid::new_v4().to_string()},
        ),
        (
            "platform key binding",
            doc! {"platform_key": {"enabled": true, "audience": "public"}},
        ),
        (
            "destination selection",
            doc! {"destination_targets": {"other": "https://other.example"}},
        ),
        (
            "empty transport credential",
            doc! {"credential_encrypted": bson::to_bson(&Vec::<u8>::new()).unwrap()},
        ),
        ("inactive target", doc! {"is_active": false}),
        ("non HTTP target", doc! {"service_type": "ssh"}),
    ] {
        let mut service = original.clone();
        service.extend(changed);
        let service = bson::from_document::<DownstreamService>(service)
            .unwrap_or_else(|error| panic!("invalid {label} test fixture: {error}"));
        f.state
            .db
            .collection::<DownstreamService>(CATALOG)
            .replace_one(doc! {"_id": &f.service.id}, service)
            .await
            .unwrap();
        for route in &routes {
            let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{label}: {body}");
        }
        assert_eq!(
            upstream.received_requests().await.unwrap().len(),
            2,
            "{label}"
        );
    }
    for developer_app_ids in [None, Some(vec![Uuid::new_v4().to_string()])] {
        let mut private = f.service.clone();
        private.visibility = "private".into();
        private.developer_app_ids = developer_app_ids;
        f.state
            .db
            .collection::<DownstreamService>(CATALOG)
            .replace_one(doc! {"_id": &f.service.id}, private)
            .await
            .unwrap();
        for route in &routes {
            let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "private transport credential without SA consent: {body}"
            );
        }
        assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
    }
    services
        .replace_one(doc! {"_id": &f.service.id}, original)
        .await
        .unwrap();
    let connection_id = add_ornn_connection(&f, &f.sa.id, b"dedicated-sa-key").await;
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "dedicated credential precedence: {body}"
        );
    }
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .update_one(
            doc! {"_id": &connection_id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "disabled connection: {body}");
    }
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    for request in &requests {
        assert_ornn_identity(&f, request, &ORNN_PERMISSIONS);
    }

    let (status, body) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/roles/{role_id}"),
        &f.human_token,
        Some(json!({"permissions": [READ_PERMISSION, WRITE_PERMISSION]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"catalog_scope_authorized": false}},
        )
        .await
        .unwrap();
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "legacy editor without policy: {body}"
        );
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("explicit proxy operation policy"),
            "{body}"
        );
    }
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .delete_one(doc! {"_id": &connection_id})
        .await
        .unwrap();
    services
        .update_one(
            doc! {"_id": &f.service.id},
            doc! {"$set": {"proxy_operation_policy": {"rules": [
                {"method": "GET", "path_template": "/api/v1/skills/{id}"}
            ]}}},
        )
        .await
        .unwrap();
    for route in &routes {
        let (status, body) = request(&f.state, "GET", route, &bearer, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "legacy editor without own credential: {body}"
        );
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("dedicated service-account"),
            "{body}"
        );
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn editor_proxy_permits_ornn_skill_cru_with_only_sa_credential() {
    use crate::models::user_service_connection::{
        COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    let (f, role_id, _) = editor("catalog_editor_ornn_cru").await;
    let bearer = token(&f, Some("proxy")).await;
    let role_path = format!("/api/v1/admin/roles/{role_id}");
    let (status, role) = request(
        &f.state,
        "PUT",
        &role_path,
        &f.human_token,
        Some(json!({"permissions": [
            READ_PERMISSION, WRITE_PERMISSION,
            "ornn:skill:read", "ornn:skill:create", "ornn:skill:update",
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");

    let account = f
        .state
        .db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id": &f.sa.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!account.get_bool("catalog_scope_authorized").unwrap());
    let upstream = MockServer::start().await;
    let mut service = f.service.clone();
    service.slug = "ornn-api".into();
    service.base_url = upstream.uri();
    service.auth_method = "bearer".into();
    service.requires_user_credential = true;
    service.identity_propagation_mode = "both".into();
    service.identity_include_user_id = true;
    service.identity_jwt_audience = Some("ornn-editor-test".into());
    service.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"master-key-must-not-be-used")
        .await
        .unwrap();
    service.proxy_operation_policy = Some(
        serde_json::from_value(json!({"rules": [
            {"method": "GET", "path_template": "/api/v1/skills/{id}"},
            {"method": "POST", "path_template": "/api/v1/skills"},
            {"method": "PUT", "path_template": "/api/v1/skills/{id}"},
        ]}))
        .unwrap(),
    );
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .replace_one(doc! {"_id": &service.id}, &service)
        .await
        .unwrap();
    f.state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .insert_one(UserServiceConnection {
            id: Uuid::new_v4().to_string(),
            user_id: f.sa.id.clone(),
            service_id: service.id.clone(),
            credential_encrypted: Some(
                f.state
                    .encryption_keys
                    .encrypt(b"dedicated-sa-key")
                    .await
                    .unwrap(),
            ),
            credential_type: Some("bearer".into()),
            credential_label: None,
            metadata: None,
            is_active: true,
            state_version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    for (verb, skill_path) in [
        ("GET", "/api/v1/skills/owned"),
        ("POST", "/api/v1/skills"),
        ("PUT", "/api/v1/skills/owned"),
    ] {
        Mock::given(method(verb))
            .and(path(skill_path))
            .and(header("authorization", "Bearer dedicated-sa-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .expect(2)
            .mount(&upstream)
            .await;
        for target in [service.id.as_str(), "s/ornn-api"] {
            let route = format!("/api/v1/proxy/{target}{skill_path}");
            let payload = (verb != "GET").then(|| json!({"test":true}));
            let (status, body) = request(&f.state, verb, &route, &bearer, payload).await;
            assert_eq!(status, StatusCode::OK, "{verb} {route}: {body}");
            assert_eq!(body["ok"], true);
        }
    }
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 6);
    let assertion = requests[0]
        .headers
        .get("x-nyxid-identity-token")
        .unwrap()
        .to_str()
        .unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&["ornn-editor-test"]);
    validation.set_issuer(&[&f.state.config.jwt_issuer]);
    let claims = jsonwebtoken::decode::<serde_json::Value>(
        assertion,
        &f.state.jwt_keys.decoding,
        &validation,
    )
    .unwrap()
    .claims;
    assert_eq!(claims["sub"], f.sa.id);
    let permissions = claims["permissions"].as_array().unwrap();
    for permission in ["ornn:skill:read", "ornn:skill:create", "ornn:skill:update"] {
        assert!(permissions.iter().any(|value| value == permission));
    }
    for permission in ["ornn:skill:delete", "ornn:admin:skill"] {
        assert!(!permissions.iter().any(|value| value == permission));
    }

    for (verb, path) in [
        (
            "DELETE",
            format!("/api/v1/proxy/{}/api/v1/skills/owned", service.id),
        ),
        (
            "GET",
            format!("/api/v1/proxy/{}/api/v1/skills/owned", Uuid::new_v4()),
        ),
        (
            "GET",
            "/api/v1/proxy/s/other-service/api/v1/skills/owned".into(),
        ),
        (
            "GET",
            format!(
                "/api/v1/proxy/{}/api/v1/skills/owned?_nyxid_via={}",
                service.id,
                Uuid::new_v4()
            ),
        ),
    ] {
        let (status, body) = request(&f.state, verb, &path, &bearer, None).await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{verb} {path}: {body}"
        );
        assert_eq!(
            upstream.received_requests().await.unwrap().len(),
            6,
            "{path}"
        );
    }
    let (status, _) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/service-accounts/{}", f.sa.id),
        &f.human_token,
        Some(json!({"allowed_scopes":"catalog:skills:read"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/proxy/{}/api/v1/skills/owned", service.id),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), 6);
}

#[tokio::test]
async fn activation_precondition_rejects_stale_access_and_accepts_unchanged_assignment() {
    let (f, role_id, _) = editor("editor_activation_precondition").await;
    let (_, ornn_role) = request(&f.state, "POST", "/api/v1/admin/roles", &f.human_token,
        Some(json!({"name":"Ornn retained", "slug":format!("ornn-retained-{}", Uuid::new_v4()), "permissions":["ornn:skill:read"], "is_default":false}))).await;
    let ornn_role_id = ornn_role["id"].as_str().unwrap();
    let role_ids = vec![role_id.as_str(), ornn_role_id];
    let accounts = f.state.db.collection::<Document>(ACCOUNTS);
    accounts.update_one(doc! {"_id": &f.sa.id}, doc! {"$set": {"role_ids": &role_ids}, "$unset": {"purpose":"", "platform_protected":""}}).await.unwrap();
    let before = accounts
        .find_one(doc! {"_id": &f.sa.id})
        .await
        .unwrap()
        .unwrap();
    let bearer = token(&f, None).await;
    for path in [
        "/api/v1/keys".to_owned(),
        format!("/api/v1/keys/{}", f.service.id),
    ] {
        assert_eq!(
            request(&f.state, "GET", &path, &bearer, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let expected = json!({"role_ids": role_ids, "allowed_scopes": SCOPES, "purpose": "general", "platform_protected": false, "is_active": true});
    let path = format!("/api/v1/admin/service-accounts/{}", f.sa.id);
    let body = json!({"role_ids": role_ids, "allowed_scopes": SCOPES, "expected_access": expected});
    for mutation in [
        doc! {"role_ids": []},
        doc! {"allowed_scopes": "proxy"},
        doc! {"is_active":false},
        doc! {"purpose":"curation"},
        doc! {"platform_protected":true},
    ] {
        accounts
            .update_one(doc! {"_id": &f.sa.id}, doc! {"$set": &mutation})
            .await
            .unwrap();
        assert_eq!(
            request(&f.state, "PUT", &path, &f.human_token, Some(body.clone()))
                .await
                .0,
            StatusCode::CONFLICT
        );
        let stored = accounts
            .find_one(doc! {"_id": &f.sa.id})
            .await
            .unwrap()
            .unwrap();
        for (key, value) in &mutation {
            assert_eq!(stored.get(key), Some(value));
        }
        accounts
            .replace_one(doc! {"_id": &f.sa.id}, &before)
            .await
            .unwrap();
    }
    assert_eq!(
        request(&f.state, "PUT", &path, &f.human_token, Some(body))
            .await
            .0,
        StatusCode::OK
    );
    let after = accounts
        .find_one(doc! {"_id": &f.sa.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.get_array("role_ids").unwrap(),
        before.get_array("role_ids").unwrap()
    );
    for field in [
        "client_id",
        "client_secret_hash",
        "secret_prefix",
        "credential_generation",
        "is_active",
    ] {
        assert_eq!(after.get(field), before.get(field));
    }
    let bearer = token(&f, None).await;
    let (status, listing) = request(&f.state, "GET", "/api/v1/keys", &bearer, None).await;
    assert_eq!(status, StatusCode::OK);
    let id = listing["keys"][0]["id"].as_str().unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{id}"),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn admin_scope_save_grants_catalog_reads_without_roles_and_revokes_live() {
    let f = fixture("scope_editor_save", false).await;
    let accounts = f.state.db.collection::<Document>(ACCOUNTS);
    accounts
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"allowed_scopes":"catalog:skills:read", "role_ids": []}},
        )
        .await
        .unwrap();
    let path = format!("/api/v1/admin/service-accounts/{}", f.sa.id);
    let old_token = token(&f, None).await;
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &old_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"allowed_scopes":"catalog:skills:read"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["catalog_scope_authorized"], true);
    assert_eq!(saved["purpose"], "catalog_editor");
    assert!(saved["role_ids"].as_array().unwrap().is_empty());
    let (status, list) = request(&f.state, "GET", "/api/v1/keys", &old_token, None).await;
    assert_eq!(status, StatusCode::OK);
    let id = list["keys"][0]["id"].as_str().unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{id}"),
            &old_token,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(request(&f.state,"PUT",&format!("/api/v1/catalog-curation/services/{id}/skills"),&old_token,Some(json!({"recommended_skills":["test"],"base_revision":0,"request_id":Uuid::new_v4().to_string()}))).await.0,StatusCode::FORBIDDEN);
    let private_id = add_personal_connection(&f).await;
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{private_id}"),
            &old_token,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"allowed_scopes":"catalog:skills:write"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["catalog_scope_authorized"], true);
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &old_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let writer = token(&f, None).await;
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &writer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status,_) = request(&f.state,"PUT",&format!("/api/v1/catalog-curation/services/{id}/skills"),&writer,Some(json!({"recommended_skills":["test"],"base_revision":0,"request_id":Uuid::new_v4().to_string()}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &path,
            &f.human_token,
            Some(json!({"allowed_scopes":"proxy"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(request(&f.state,"PUT",&format!("/api/v1/catalog-curation/services/{id}/skills"),&writer,Some(json!({"recommended_skills":[],"base_revision":1,"request_id":Uuid::new_v4().to_string()}))).await.0,StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_scope_create_without_roles_and_legacy_role_save_do_not_cross_modes() {
    let f = fixture("scope_editor_create", false).await;
    let (status, created) = request(
        &f.state,
        "POST",
        "/api/v1/admin/service-accounts",
        &f.human_token,
        Some(json!({"name":"Scope editor","allowed_scopes":"catalog:skills:read","role_ids":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap();
    let (status, stored) = request(
        &f.state,
        "GET",
        &format!("/api/v1/admin/service-accounts/{id}"),
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["catalog_scope_authorized"], true);
    let created_account =
        crate::services::service_account_service::get_service_account(&f.state.db, id)
            .await
            .unwrap();
    let created_fixture = Fixture {
        state: f.state.clone(),
        sa: created_account,
        secret: created["client_secret"].as_str().unwrap().to_owned(),
        service: f.service.clone(),
        owner: f.owner.clone(),
        human_token: f.human_token.clone(),
    };
    let bearer = token(&created_fixture, None).await;
    let (status, list) = request(&f.state, "GET", "/api/v1/keys", &bearer, None).await;
    assert_eq!(status, StatusCode::OK);
    let listed_id = list["keys"][0]["id"].as_str().unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{listed_id}"),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (legacy, role, _) = editor("scope_editor_legacy").await;
    let path = format!("/api/v1/admin/service-accounts/{}", legacy.sa.id);
    let (status, saved) = request(
        &legacy.state,
        "PUT",
        &path,
        &legacy.human_token,
        Some(json!({"role_ids":[role]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["catalog_scope_authorized"], false);
    let (status, saved) = request(
        &legacy.state,
        "PUT",
        &path,
        &legacy.human_token,
        Some(json!({"allowed_scopes":SCOPES})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["catalog_scope_authorized"], true);
}

#[tokio::test]
async fn org_admin_and_machine_cannot_issue_scope_authority() {
    use crate::models::{
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        user::{COLLECTION_NAME as USERS, UserType},
    };
    let f = fixture("scope_editor_unprivileged", false).await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(USERS)
        .insert_one(bson::to_document(&test_utils::test_user(&org, UserType::Org)).unwrap())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(MEMBERSHIPS)
        .insert_one(
            bson::to_document(&test_utils::test_membership(
                &org,
                &f.owner,
                OrgRole::Admin,
                None,
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(doc! {"_id":&f.sa.id}, doc! {"$set":{"owner_user_id":&org}})
        .await
        .unwrap();
    let machine = token(&f, None).await;
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"is_admin":false,"role_ids":[]}},
        )
        .await
        .unwrap();
    let path = format!("/api/v1/admin/service-accounts/{}", f.sa.id);
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"allowed_scopes":"catalog:skills:read"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["catalog_scope_authorized"], false);
    assert_eq!(saved["purpose"], "general");
    let bearer = token(&f, None).await;
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &path,
            &machine,
            Some(json!({"allowed_scopes":"catalog:skills:read"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status,saved)=request(&f.state,"POST","/api/v1/admin/service-accounts",&f.human_token,Some(json!({"name":"Org scopes", "target_org_id":&org,"allowed_scopes":"catalog:skills:read"}))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, stored) = request(
        &f.state,
        "GET",
        &format!(
            "/api/v1/admin/service-accounts/{}",
            saved["id"].as_str().unwrap()
        ),
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["catalog_scope_authorized"], false);
    let operator = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap()
        .operator;
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"role_ids":[operator]}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":&f.sa.id},
            doc! {"$set":{"owner_user_id":&f.owner}},
        )
        .await
        .unwrap();
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"allowed_scopes":"catalog:skills:read"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["catalog_scope_authorized"], false);
    assert_eq!(saved["purpose"], "general");
    let bearer = token(&f, None).await;
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &bearer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
