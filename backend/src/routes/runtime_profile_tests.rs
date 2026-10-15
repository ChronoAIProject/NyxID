//! Mounted admin API coverage on a disposable transaction-capable database.
use super::build_router_with_state;
use crate::{
    crypto::jwt,
    models::{
        machine_runtime_billing::RATE_COLLECTION,
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::role_service,
    test_utils,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn runtime_profile_admin_routes_expose_safe_specs_and_reject_invalid_sale_bindings() {
    let db = test_utils::connect_transaction_test_database("profile_admin").await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let admin_id = Uuid::new_v4();
    let mut admin = test_utils::test_user(&admin_id.to_string(), UserType::Person);
    admin.role_ids = vec![roles.admin];
    db.collection::<User>(USERS)
        .insert_one(admin)
        .await
        .unwrap();
    super::remote_machine_flag_tests::set_enabled(&db, &admin_id.to_string(), true).await;
    let state = test_utils::test_app_state(db.clone());
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &admin_id,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);
    let rates = db.collection::<Document>(RATE_COLLECTION);
    for body in [
        json!({"profile_version":"unknown","credits_per_second":"0.000040"}),
        json!({"profile_version":"remote-shell-files-v1","credits_per_second":"0.000040"}),
        json!({"profile_version":null,"credits_per_second":"0.000040"}),
        json!({"profile_version":"remote-machine","credits_per_second":"0.000041"}),
        json!({"profile_version":"remote-machine","credits_per_second":"0"}),
        json!({"profile_version":"remote-machine","credits_per_second":"-0.000040"}),
        json!({"profile_version":"remote-machine","credits_per_second":"NaN"}),
        json!({"profile_version":"remote-machine","credits_per_second":"0.000040","workspace_gib":50}),
    ] {
        let invalid_body = body.clone();
        let (status, value) = request(
            &app,
            "POST",
            "/api/v1/admin/billing/runtime/rates",
            &token,
            body,
        )
        .await;
        let expected = if invalid_body["profile_version"].is_null()
            || invalid_body.get("workspace_gib").is_some()
        {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::BAD_REQUEST
        };
        assert_eq!(status, expected, "{value}; invalid request: {invalid_body}");
        assert_eq!(
            rates.count_documents(doc! {}).await.unwrap(),
            0,
            "invalid request staged a rate"
        );
    }
    let (status, staged) = request(
        &app,
        "POST",
        "/api/v1/admin/billing/runtime/rates",
        &token,
        json!({"profile_version":"remote-machine","credits_per_second":"0.000040"}),
    )
    .await;
    assert!(status.is_success(), "{status}: {staged}");
    let id = staged["rate_version"].as_str().unwrap();
    assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 4);
    assert_eq!(staged["profile_version"], "remote-machine");
    assert_eq!(staged["profile"]["cpu_millis"], 2000);
    assert_eq!(staged["profile"]["memory_mib"], 2048);
    assert_eq!(staged["profile"]["workspace_gib"], 20);
    assert!(staged["profile"]["ephemeral_gib"].is_null());
    assert_eq!(staged["credits_per_second"], "0.00004");
    assert_eq!(staged["sync_status"], "pending");
    assert_eq!(staged["retired"], false);
    assert!(staged["created_at"].is_string());
    assert!(staged["verified_at"].is_null());
    let stored = rates.find_one(doc! {"_id":id}).await.unwrap().unwrap();
    assert_eq!(stored.get_str("profile_version").unwrap(), "remote-machine");
    assert_eq!(
        stored.get_str("lago_metric_code").unwrap(),
        format!("remote_machine_running_seconds_{}", id.replace('-', ""))
    );
    let (status, view) = request(
        &app,
        "GET",
        "/api/v1/admin/billing/runtime",
        &token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    // MongoDB persists datetime values at millisecond precision.
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(view["rates"][0]["created_at"].as_str().unwrap())
            .unwrap()
            .timestamp_millis(),
        chrono::DateTime::parse_from_rfc3339(staged["created_at"].as_str().unwrap())
            .unwrap()
            .timestamp_millis()
    );
    let mut persisted_view = staged.clone();
    persisted_view["created_at"] = view["rates"][0]["created_at"].clone();
    assert_eq!(view["rates"][0], persisted_view);
    assert!(view["rates"][0].get("lago_metric_code").is_none());
    assert!(view["rates"][0].get("plan_code").is_none());
    let (status, retired) = request(
        &app,
        "POST",
        &format!("/api/v1/admin/billing/runtime/rates/{id}/retire"),
        &token,
        json!({}),
    )
    .await;
    assert!(status.is_success(), "{status}: {retired}");
    let (status, view) = request(
        &app,
        "GET",
        "/api/v1/admin/billing/runtime",
        &token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["rates"][0]["rate_version"], id);
    assert_eq!(view["rates"][0]["profile"], staged["profile"]);
    assert_eq!(
        view["rates"][0]["credits_per_second"],
        staged["credits_per_second"]
    );
    assert_eq!(view["rates"][0]["retired"], true);
    db.drop().await.unwrap();
}
