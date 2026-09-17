use crate::{
    models::{org_membership::OrgRole, user::UserType, user_service::UserService},
    services::{key_service, service_history},
    test_utils::*,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use bson::{Document, doc};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    router: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(
                    body.map(|v| Body::from(v.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn service_history_mounted_acl_summary_node_redaction_and_api_key_attribution() {
    let db = connect_transaction_test_database("history_mounted_acl").await;
    let actor = Uuid::new_v4();
    let org = Uuid::new_v4().to_string();
    let outsider = Uuid::new_v4().to_string();
    db.collection("users")
        .insert_many([
            test_user(&actor.to_string(), UserType::Person),
            test_user(&org, UserType::Org),
            test_user(&outsider, UserType::Person),
        ])
        .await
        .unwrap();
    db.collection("org_memberships")
        .insert_one(test_membership(
            &org,
            &actor.to_string(),
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let endpoint = test_user_endpoint(
        &Uuid::new_v4().to_string(),
        &org,
        "Org service",
        "https://example.com",
        None,
        None,
    );
    db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(&endpoint)
        .await
        .unwrap();
    let mut service = test_user_service(
        &Uuid::new_v4().to_string(),
        &org,
        "history",
        &endpoint.id,
        None,
        None,
    );
    service.auth_method = "none".into();
    service_history::collection::<UserService>(&db, "user_services")
        .insert_one(&service)
        .await
        .unwrap();
    let mut sibling = service.clone();
    sibling.id = Uuid::new_v4().to_string();
    sibling.slug = "sibling".into();
    service_history::collection::<UserService>(&db, "user_services")
        .insert_one(&sibling)
        .await
        .unwrap();
    let visible_node = Uuid::new_v4().to_string();
    let hidden_node = Uuid::new_v4().to_string();
    for (id, owner) in [(&visible_node, &org), (&hidden_node, &outsider)] {
        db.collection::<Document>("nodes").insert_one(doc! { "_id": id, "user_id": owner, "name": "node", "status": "offline", "auth_token_hash": "test", "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now() }).await.unwrap();
    }
    for id in [&hidden_node, &visible_node] {
        service_history::collection::<Document>(&db, "user_services")
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": { "node_id": id } },
            )
            .await
            .unwrap();
    }
    let state = test_app_state(db.clone());
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &actor,
        "openid",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = crate::routes::build_router();
    let router = private.with_state(state.clone());
    let history_path = format!("/api/v1/keys/{}/history", service.id);
    let detail_path = format!("/api/v1/keys/{}", service.id);
    let (status, history) = request(&router, &token, "GET", &history_path, None).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert!(history.to_string().contains(&visible_node));
    assert!(!history.to_string().contains(&hidden_node));
    let (status, detail) = request(&router, &token, "GET", &detail_path, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(detail["authorship"].is_object());
    let key = key_service::create_api_key_with_scope_authorization(
        &db,
        &actor.to_string(),
        Some(&actor.to_string()),
        "Alice",
        "read write",
        None,
        None,
        Some(std::slice::from_ref(&service.id)),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    // Production management routes retain their existing human-only gate.
    assert_eq!(
        request(&router, &key.full_key, "GET", &history_path, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // Exercise defense in depth with real credential extraction below that gate:
    // no test-created AuthUser can invent the key's owner, service or node scope.
    let key_router = Router::new()
        .route(
            "/api/v1/keys/history/archived",
            axum::routing::get(super::get_archived),
        )
        .route(
            "/api/v1/keys/{service_id}/history",
            axum::routing::get(super::get_history),
        )
        .route(
            "/api/v1/endpoints/{endpoint_id}",
            axum::routing::put(crate::handlers::user_endpoints::update_endpoint),
        )
        .layer(axum::middleware::from_fn(
            service_history::context::middleware,
        ))
        .with_state(state);
    let (status, history) = request(&key_router, &key.full_key, "GET", &history_path, None).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert!(!history.to_string().contains(&visible_node));
    assert_eq!(
        request(
            &key_router,
            &key.full_key,
            "GET",
            &format!("/api/v1/keys/{}/history", sibling.id),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, result) = request(
        &key_router,
        &key.full_key,
        "PUT",
        &format!("/api/v1/endpoints/{}", endpoint.id),
        Some(serde_json::json!({ "label": "New label" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let event = db
        .collection::<crate::models::service_change_event::ServiceChangeEvent>(
            crate::models::service_change_event::COLLECTION_NAME,
        )
        .find_one(doc! { "service_id": &service.id, "entity_type": "user_endpoints" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.actor.api_key_id.as_deref(), Some(key.id.as_str()));
    assert_eq!(
        event.actor.person_id.as_deref(),
        Some(actor.to_string().as_str())
    );
    assert_eq!(event.owner_id, org);
    let org_key = key_service::create_api_key(
        &db,
        &org,
        "Org automation",
        "read write",
        None,
        None,
        Some(std::slice::from_ref(&service.id)),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let (status, result) = request(
        &key_router,
        &org_key.full_key,
        "PUT",
        &format!("/api/v1/endpoints/{}", endpoint.id),
        Some(serde_json::json!({ "label": "Org key edit" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let event = db
        .collection::<crate::models::service_change_event::ServiceChangeEvent>(
            crate::models::service_change_event::COLLECTION_NAME,
        )
        .find_one(doc! { "service_id": &service.id, "actor.api_key_id": &org_key.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event.actor.kind,
        crate::models::service_change_event::HistoryActorKind::ApiKey
    );
    assert!(event.actor.person_id.is_none());
    assert_eq!(event.owner_id, org);

    // Current role, resource scope and owner activity all gate the same metadata.
    for role in ["member", "viewer"] {
        db.collection::<Document>("org_memberships")
            .update_one(
                doc! { "org_user_id": &org },
                doc! { "$set": { "role": role } },
            )
            .await
            .unwrap();
        assert_eq!(
            request(&router, &token, "GET", &history_path, None).await.0,
            StatusCode::NOT_FOUND
        );
        let (status, detail) = request(&router, &token, "GET", &detail_path, None).await;
        if status == StatusCode::OK {
            assert!(detail.get("authorship").is_none_or(Value::is_null));
        }
    }
    db.collection::<Document>("org_memberships")
        .update_one(
            doc! { "org_user_id": &org },
            doc! { "$set": { "role": "admin", "allowed_service_ids": [&sibling.id] } },
        )
        .await
        .unwrap();
    assert_eq!(
        request(&router, &token, "GET", &history_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    db.collection::<Document>("org_memberships")
        .update_one(
            doc! { "org_user_id": &org },
            doc! { "$set": { "allowed_service_ids": [&service.id] } },
        )
        .await
        .unwrap();
    service_history::collection::<UserService>(&db, "user_services")
        .delete_one(doc! { "_id": &service.id })
        .await
        .unwrap();
    let (status, archive) = request(
        &key_router,
        &key.full_key,
        "GET",
        "/api/v1/keys/history/archived",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{archive}");
    assert_eq!(archive["services"][0]["service_id"], service.id);
    db.collection::<Document>("users")
        .update_one(
            doc! { "_id": &org },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert_eq!(
        request(&router, &token, "GET", &history_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &router,
            &token,
            "GET",
            "/api/v1/keys/history/archived",
            None
        )
        .await
        .1["services"],
        serde_json::json!([])
    );
}
