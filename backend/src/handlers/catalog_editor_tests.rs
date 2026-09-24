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
    let (status, account) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/service-accounts/{}", f.sa.id),
        &f.human_token,
        Some(json!({"allowed_scopes": SCOPES})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account["purpose"], "general");
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
async fn editor_proxy_permits_ornn_skill_cru_with_only_sa_credential() {
    use crate::models::user_service_connection::{
        COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    let (f, role_id, bearer) = editor("catalog_editor_ornn_cru").await;
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
            .expect(1)
            .mount(&upstream)
            .await;
        let route = format!("/api/v1/proxy/{}{skill_path}", service.id);
        let payload = (verb != "GET").then(|| json!({"test":true}));
        let (status, body) = request(&f.state, verb, &route, &bearer, payload).await;
        assert_eq!(status, StatusCode::OK, "{verb} {route}: {body}");
        assert_eq!(body["ok"], true);
    }
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
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
        ("GET", "/api/v1/proxy/s/ornn-api/api/v1/skills/owned".into()),
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
            3,
            "{path}"
        );
    }
    let (status, role) = request(
        &f.state,
        "PUT",
        &role_path,
        &f.human_token,
        Some(json!({"permissions": ["ornn:skill:read", "ornn:skill:create", "ornn:skill:update"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
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
    assert_eq!(upstream.received_requests().await.unwrap().len(), 3);
}
