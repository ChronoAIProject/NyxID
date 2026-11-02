//! Exercise the mounted HTTP contract, including the human/CatalogEditor dispatcher.
use super::*;
use crate::{models::user::UserType, test_utils};
use axum::http::StatusCode;
use serde_json::{Value, json};

async fn single_field_updates(catalog_backed: bool) {
    let db = test_utils::connect_transaction_test_database("keys_2098").await;
    let state = test_utils::test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_utils::test_user(&owner.to_string(), UserType::Person))
        .await
        .unwrap();
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "openid profile email",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let mut create = json!({"slug":"custom-bearer", "label":"Original", "auth_method":"bearer",
        "credential":"test-only-secret", "endpoint_url":"https://example.com/original"});
    if catalog_backed {
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = "catalog-bearer".into();
        catalog.base_url = "https://example.com/original".into();
        catalog.auth_method = "bearer".into();
        catalog.auth_key_name = "Authorization".into();
        catalog.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .unwrap();
        create["service_slug"] = json!("catalog-bearer");
    }
    let request = crate::handlers::curation_tests::request;
    let (status, created) = request(&state, "POST", "/api/v1/keys", &token, Some(create)).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["catalog_service_id"].is_string(), catalog_backed);
    let path = format!("/api/v1/keys/{}", created["id"].as_str().unwrap());
    let (status, initial) = request(&state, "GET", &path, &token, None).await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    assert_eq!(initial["id"], created["id"]);
    assert_eq!(initial["endpoint_url"], created["endpoint_url"]);
    for (field, value) in [
        ("endpoint_url", json!("https://example.com/updated")),
        ("label", json!("Updated")),
        ("credential", json!("rotated-test-only-secret")),
        ("custom_user_agent", json!("test-agent/2")),
        (
            "openapi_spec_url",
            json!("https://example.com/openapi.json"),
        ),
        ("is_active", json!(false)),
        ("is_active", json!(true)),
    ] {
        let (status, updated) =
            request(&state, "PUT", &path, &token, Some(json!({field: value}))).await;
        // Read even after a failed PUT so a partial commit is visible in the assertion.
        let (read_status, saved) = request(&state, "GET", &path, &token, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{field}: PUT={updated}, stored={saved}"
        );
        assert_eq!(read_status, StatusCode::OK, "{saved}");
        assert_eq!(updated["id"], created["id"]);
        assert_eq!(updated["catalog_service_id"], created["catalog_service_id"]);
        if field != "credential" {
            assert_eq!(updated[field], value, "{field}");
            assert_eq!(saved[field], value, "{field}");
        } else {
            assert!(updated.get("credential").is_none());
        }
    }
    // Validation must reject a single-field change without changing persisted state.
    for body in [
        json!({"endpoint_url":"ftp://example.com/bad"}),
        json!({"label":""}),
    ] {
        let (_, before) = request(&state, "GET", &path, &token, None).await;
        let (status, error) = request(&state, "PUT", &path, &token, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        let (_, after) = request(&state, "GET", &path, &token, None).await;
        for field in ["endpoint_url", "label", "is_active", "updated_at"] {
            assert_eq!(after[field], before[field]);
        }
    }
    if !catalog_backed {
        assert_eq!(
            db.collection::<Value>(DOWNSTREAM_SERVICES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn keys_2098_custom_single_field_updates_return_committed_connection_without_catalog() {
    Box::pin(single_field_updates(false)).await;
}

#[tokio::test]
async fn keys_2098_catalog_single_field_updates_return_committed_connection() {
    Box::pin(single_field_updates(true)).await;
}
