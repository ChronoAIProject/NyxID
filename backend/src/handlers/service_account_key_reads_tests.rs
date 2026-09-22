use axum::http::StatusCode;
use chrono::{Duration, Utc};
use mongodb::bson::{self, Document, doc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::curation_tests::{Fixture, fixture, request, token};
use crate::{
    models::{
        downstream_service::COLLECTION_NAME as CATALOG,
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        service_account::COLLECTION_NAME as ACCOUNTS,
        service_account_key_read_grant::COLLECTION_NAME as GRANTS,
        user::{COLLECTION_NAME as USERS, UserType},
        user_api_key::COLLECTION_NAME as CREDENTIALS,
        user_endpoint::COLLECTION_NAME as ENDPOINTS,
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    services::{curation_grant_service, service_account_key_read_service as reads},
    test_utils,
};

async fn setup(label: &str, curated: bool) -> (Fixture, UserService, String) {
    let f = fixture(label, curated).await;
    f.state.db.collection::<Document>(ACCOUNTS).update_one(
        doc! {"_id": &f.sa.id},
        doc! {"$set": {"allowed_scopes": "catalog:skills:read catalog:skills:write proxy user-services:read"}},
    ).await.unwrap();
    let endpoint_id = Uuid::new_v4().to_string();
    let ep = test_utils::test_user_endpoint(
        &endpoint_id,
        &f.owner,
        "Test connection",
        "https://example.com/secret-url",
        Some("https://example.com/spec?secret=hidden"),
        Some(&f.service.id),
    );
    f.state
        .db
        .collection::<Document>(ENDPOINTS)
        .insert_one(bson::to_document(&ep).unwrap())
        .await
        .unwrap();
    let mut svc = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "test-connection",
        &endpoint_id,
        Some(&f.service.id),
        None,
    );
    let credential_id = Uuid::new_v4().to_string();
    svc.api_key_id = Some(credential_id.clone());
    svc.auth_method = "bearer".into();
    let mut row = bson::to_document(&svc).unwrap();
    row.insert("default_request_headers", bson::bson!([{"name":"x-test", "value":"header-secret", "sensitive":false, "overridable":false}]));
    row.insert(
        "ws_frame_injections",
        bson::bson!([{"template":"frame-secret"}]),
    );
    f.state
        .db
        .collection::<Document>(SERVICES)
        .insert_one(row)
        .await
        .unwrap();
    // Intentionally unreadable credential material: a metadata read must not resolve it.
    f.state.db.collection::<Document>(CREDENTIALS).insert_one(doc! {
        "_id": &credential_id, "user_id": &f.owner, "credential_type": "oauth2", "status": "pending_auth",
        "credential_encrypted": "invalid-ciphertext-secret", "oauth_client_id": "client-secret-marker"
    }).await.unwrap();
    let pin = json!({"source":"ornn", "skill_id":"skill-1", "name":"ops/manual", "version":"1.0", "sha256":"a".repeat(64), "dependencies":[]});
    f.state.db.collection::<Document>(CATALOG).update_one(doc! {"_id": &f.service.id}, doc! {"$set": {
        "recommended_skills": ["ops/manual"], "recommended_skill_refs": bson::to_bson(&vec![pin]).unwrap(), "skills_revision": 7i64,
    }}).await.unwrap();
    let bearer = token(&f, None).await;
    (f, svc, bearer)
}

async fn grant(f: &Fixture, ids: &[String]) {
    let (status, body) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/admin/service-accounts/{}/key-read-grant", f.sa.id),
        &f.human_token,
        Some(json!({"user_service_ids": ids})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user_service_ids"], json!(ids));
}

#[tokio::test]
async fn sa_key_read_is_projected_and_has_no_connection_side_effects() {
    let (f, svc, bearer) = setup("sa_key_read_projection", false).await;
    let path = format!("/api/v1/keys/{}", svc.id);
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let mut snapshots = Vec::new();
    for (collection, id) in [
        (SERVICES, svc.id.as_str()),
        (ENDPOINTS, svc.endpoint_id.as_str()),
        (CREDENTIALS, svc.api_key_id.as_deref().unwrap()),
    ] {
        snapshots.push((
            collection,
            id.to_string(),
            f.state
                .db
                .collection::<Document>(collection)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap(),
        ));
    }
    let (status, body) = request(&f.state, "GET", &path, &bearer, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], svc.id);
    assert_eq!(body["recommended_skill_refs"][0]["name"], "ops/manual");
    assert_eq!(body["skills_revision"], 7);
    assert!(
        body["skills_manifest_digest"]
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
    ];
    assert_eq!(body.as_object().unwrap().len(), expected.len());
    assert!(expected.iter().all(|key| body.get(key).is_some()));
    assert!(!body.to_string().contains("secret"));
    for (collection, id, before) in snapshots {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn sa_key_read_rejects_other_ids_slugs_and_writes() {
    let (f, svc, bearer) = setup("sa_key_read_boundaries", false).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let sibling = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "sibling",
        &svc.endpoint_id,
        None,
        None,
    );
    f.state
        .db
        .collection::<UserService>(SERVICES)
        .insert_one(&sibling)
        .await
        .unwrap();
    for id in [
        &sibling.id,
        &svc.slug,
        &f.service.id,
        &svc.endpoint_id,
        &Uuid::new_v4().to_string(),
    ] {
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
            StatusCode::NOT_FOUND,
            "{id}"
        );
    }
    let path = format!("/api/v1/keys/{}", svc.id);
    for method in ["PUT", "DELETE", "HEAD"] {
        assert_eq!(
            request(
                &f.state,
                method,
                &path,
                &bearer,
                Some(json!({"label":"changed"}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "{method}"
        );
    }
    for path in [
        "/api/v1/keys".to_string(),
        format!("{path}/authorization"),
        "/api/v1/user-services".into(),
    ] {
        assert_eq!(
            request(&f.state, "GET", &path, &bearer, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    for method in ["GET", "PUT", "DELETE"] {
        assert_eq!(
            request(
                &f.state,
                method,
                &format!("/api/v1/admin/service-accounts/{}/key-read-grant", f.sa.id),
                &bearer,
                Some(json!({"user_service_ids":[svc.id]}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn sa_key_read_checks_live_scopes_grant_expiry_and_revocation() {
    let (f, svc, bearer) = setup("sa_key_read_live", false).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let path = format!("/api/v1/keys/{}", svc.id);
    let no_read_token = token(&f, Some("proxy")).await;
    assert_eq!(
        request(&f.state, "GET", &path, &no_read_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":&f.sa.id},
            doc! {"$set":{"allowed_scopes":"proxy"}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":&f.sa.id},
            doc! {"$set":{"allowed_scopes":reads::READ_SCOPE}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::OK
    );
    f.state.db.collection::<Document>(GRANTS).update_one(doc! {"_id":&f.sa.id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
    grant(&f, std::slice::from_ref(&svc.id)).await;
    assert_eq!(
        request(
            &f.state,
            "DELETE",
            &format!("/api/v1/admin/service-accounts/{}/key-read-grant", f.sa.id),
            &f.human_token,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn sa_key_read_preserves_overrides_disabled_reads_and_hides_deleted_rows() {
    let (f, svc, bearer) = setup("sa_key_read_inheritance", false).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let path = format!("/api/v1/keys/{}", svc.id);
    for names in [vec!["custom/manual"], vec![]] {
        f.state
            .db
            .collection::<Document>(ENDPOINTS)
            .update_one(
                doc! {"_id":&svc.endpoint_id},
                doc! {"$set":{"recommended_skills":&names}},
            )
            .await
            .unwrap();
        let (status, body) = request(&f.state, "GET", &path, &bearer, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["recommended_skills"], json!(names));
        assert!(body["recommended_skill_refs"].is_null());
        assert!(body["skills_revision"].is_null());
    }
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(doc! {"_id":&svc.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let (status, body) = request(&f.state, "GET", &path, &bearer, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], false);
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&svc.id},
            doc! {"$set":{"deleted_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn sa_key_read_curation_requires_both_grants() {
    let (f, svc, bearer) = setup("sa_key_read_curation", true).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let path = format!("/api/v1/keys/{}", svc.id);
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::OK
    );
    curation_grant_service::revoke(&f.state.db, &f.sa.id)
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn sa_key_read_revalidates_memberships_and_resource_owners() {
    let (f, svc, bearer) = setup("sa_key_read_owner", false).await;
    let org_id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(USERS)
        .insert_one(bson::to_document(&test_utils::test_user(&org_id, UserType::Org)).unwrap())
        .await
        .unwrap();
    let membership = test_utils::test_membership(
        &org_id,
        &f.owner,
        OrgRole::Member,
        Some(vec![svc.id.clone()]),
    );
    f.state
        .db
        .collection::<Document>(MEMBERSHIPS)
        .insert_one(bson::to_document(&membership).unwrap())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(doc! {"_id":&svc.id}, doc! {"$set":{"user_id":&org_id}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ENDPOINTS)
        .update_one(
            doc! {"_id":&svc.endpoint_id},
            doc! {"$set":{"user_id":&org_id}},
        )
        .await
        .unwrap();
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let path = format!("/api/v1/keys/{}", svc.id);
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::OK
    );
    f.state
        .db
        .collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! {"_id":&membership.id},
            doc! {"$set":{"allowed_service_ids":[]}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::NOT_FOUND
    );
    f.state
        .db
        .collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! {"_id":&membership.id},
            doc! {"$set":{"allowed_service_ids":[&svc.id],"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::NOT_FOUND
    );
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(doc! {"_id":&svc.id}, doc! {"$set":{"user_id":&f.owner}})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn sa_key_read_grants_validate_input_and_owner_access() {
    let (f, svc, bearer) = setup("sa_key_read_grant_validation", false).await;
    let path = format!("/api/v1/admin/service-accounts/{}/key-read-grant", f.sa.id);
    for body in [
        json!({"user_service_ids":[]}),
        json!({"user_service_ids":[svc.id,svc.id]}),
        json!({"user_service_ids":["slug"]}),
        json!({"user_service_ids":[svc.id],"expires_at":"2000-01-01T00:00:00Z"}),
        json!({"user_service_ids":[svc.id],"allowed_scopes":"proxy"}),
    ] {
        let status = request(&f.state, "PUT", &path, &f.human_token, Some(body))
            .await
            .0;
        assert!(status.is_client_error());
    }
    let other = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(USERS)
        .insert_one(bson::to_document(&test_utils::test_user(&other, UserType::Person)).unwrap())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(doc! {"_id":&svc.id}, doc! {"$set":{"user_id":other}})
        .await
        .unwrap();
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &path,
            &f.human_token,
            Some(json!({"user_service_ids":[svc.id]}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, body) = request(&f.state, "GET", &path, &f.human_token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, Value::Null);
}

#[tokio::test]
async fn sa_key_read_revalidates_endpoint_lifecycle_and_preserves_invalid_replacements() {
    let (f, svc, bearer) = setup("sa_key_read_endpoint", false).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let original = reads::get_grant(&f.state.db, &f.sa.id)
        .await
        .unwrap()
        .unwrap();
    let endpoints = f.state.db.collection::<Document>(ENDPOINTS);
    let before = endpoints
        .find_one(doc! {"_id": &svc.endpoint_id})
        .await
        .unwrap()
        .unwrap();
    for mutation in [
        doc! {"$set": {"deleted_at": bson::DateTime::now()}},
        doc! {"$set": {"user_id": Uuid::new_v4().to_string()}},
    ] {
        endpoints
            .update_one(doc! {"_id": &svc.endpoint_id}, mutation)
            .await
            .unwrap();
        assert_eq!(
            request(
                &f.state,
                "GET",
                &format!("/api/v1/keys/{}", svc.id),
                &bearer,
                None
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert!(
            reads::issue(
                &f.state.db,
                &f.sa.id,
                &f.owner,
                std::slice::from_ref(&svc.id),
                None
            )
            .await
            .is_err()
        );
        assert_eq!(
            bson::to_document(
                &reads::get_grant(&f.state.db, &f.sa.id)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            bson::to_document(&original).unwrap()
        );
        endpoints
            .replace_one(doc! {"_id": &svc.endpoint_id}, &before)
            .await
            .unwrap();
    }
    endpoints
        .delete_one(doc! {"_id": &svc.endpoint_id})
        .await
        .unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{}", svc.id),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        reads::issue(
            &f.state.db,
            &f.sa.id,
            &f.owner,
            std::slice::from_ref(&svc.id),
            None
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn sa_key_read_replacement_custom_response_and_upgrade_boundaries() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (f, svc, bearer) = setup("sa_key_read_replace", false).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let sibling = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "custom",
        &svc.endpoint_id,
        None,
        None,
    );
    f.state
        .db
        .collection::<UserService>(SERVICES)
        .insert_one(&sibling)
        .await
        .unwrap();
    let too_many = (0..101)
        .map(|_| Uuid::new_v4().to_string())
        .collect::<Vec<_>>();
    assert!(
        reads::issue(&f.state.db, &f.sa.id, &f.owner, &too_many, None)
            .await
            .is_err()
    );
    grant(&f, std::slice::from_ref(&sibling.id)).await;
    assert_eq!(
        request(
            &f.state,
            "GET",
            &format!("/api/v1/keys/{}", svc.id),
            &bearer,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let path = format!("/api/v1/keys/{}", sibling.id);
    let (status, body) = request(&f.state, "GET", &path, &bearer, None).await;
    assert_eq!(status, StatusCode::OK);
    for field in [
        "catalog_service_id",
        "recommended_skills",
        "recommended_skill_refs",
        "skills_revision",
    ] {
        assert!(body[field].is_null(), "{field}");
    }
    assert_eq!(body["name"], "Test connection");
    for upgraded in [false, true] {
        let (public, private) = crate::routes::build_router_with_state(f.state.clone());
        let mut req = Request::builder()
            .uri(&path)
            .header("authorization", format!("Bearer {bearer}"));
        if upgraded {
            req = req
                .header("upgrade", "websocket")
                .header("connection", "upgrade");
        }
        let response = public
            .merge(private)
            .with_state(f.state.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        if upgraded {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        } else {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "private, no-store");
        }
    }
}

#[tokio::test]
async fn sa_key_read_org_owned_account_and_inactive_owner() {
    let (f, svc, _) = setup("sa_key_read_org_sa", false).await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(USERS)
        .insert_one(bson::to_document(&test_utils::test_user(&org, UserType::Org)).unwrap())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"owner_user_id": &org}},
        )
        .await
        .unwrap();
    for (collection, id) in [(SERVICES, &svc.id), (ENDPOINTS, &svc.endpoint_id)] {
        f.state
            .db
            .collection::<Document>(collection)
            .update_one(doc! {"_id": id}, doc! {"$set": {"user_id": &org}})
            .await
            .unwrap();
    }
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let bearer = token(&f, None).await;
    let path = format!("/api/v1/keys/{}", svc.id);
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::OK
    );
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id": &org}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::NOT_FOUND
    );
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id": &org}, doc! {"$set": {"is_active": true}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &f.sa.id},
            doc! {"$set": {"owner_user_id": &f.owner}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &bearer, None).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn sa_key_read_grant_admin_routes_enforce_platform_roles() {
    let (f, svc, _) = setup("sa_key_read_admin_roles", false).await;
    grant(&f, std::slice::from_ref(&svc.id)).await;
    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    let path = format!("/api/v1/admin/service-accounts/{}/key-read-grant", f.sa.id);
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! {"_id": &f.owner},
            doc! {"$set": {"is_admin": false, "role_ids": [roles.operator]}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &f.human_token, None)
            .await
            .0,
        StatusCode::OK
    );
    for method in ["PUT", "DELETE"] {
        assert_eq!(
            request(
                &f.state,
                method,
                &path,
                &f.human_token,
                Some(json!({"user_service_ids": [svc.id]}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id": &f.owner}, doc! {"$set": {"role_ids": []}})
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", &path, &f.human_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
