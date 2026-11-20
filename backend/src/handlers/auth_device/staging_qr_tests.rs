use super::*;
use crate::{
    models::feature_flag_override::COLLECTION_NAME as OVERRIDES, services::staging_qr::Peer,
};
use serde_json::json;

#[tokio::test]
async fn staging_qr_two_servers_approve_only_the_reviewed_staging_account() {
    let mut prod = setup_state("qr_prod").await.expect("production database");
    let mut stage = setup_state("qr_stage").await.expect("staging database");
    let (prod_keys, stage_keys) = crate::crypto::jwt::test_rotation_keys();
    prod.jwt_keys = prod_keys;
    stage.jwt_keys = stage_keys;
    stage.jwt_keys.retained.clear();
    stage.config.jwt_issuer = "https://staging-api.nyxid.ai".into();
    stage.staging_qr = Peer::Receiver(prod.jwt_keys.decoding.clone());
    let staging_server = spawn_test_server(stage.clone()).await;
    prod.staging_qr = Peer::Sender {
        client: Client::new(),
        endpoint: format!(
            "{}/api/v1/auth/device/staging/receive",
            staging_server.base_url
        ),
    };
    let prod_server = spawn_test_server(prod.clone()).await;
    let prod_id = Uuid::new_v4().to_string();
    let stage_id = Uuid::new_v4().to_string();
    insert_user(&prod, &prod_id).await;
    let token = assured_access_token(&prod, &prod_id).await;
    let (status, request) = post_json(
        &staging_server,
        "/api/v1/auth/device/request",
        None,
        json!({"client_label":"Staging browser"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let path = "/api/v1/auth/device/staging";
    let preview = json!({"action":"preview","user_code": request["user_code"]});
    assert_eq!(
        post_json(&prod_server, path, Some(&token), preview.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let overrides = prod.db.collection::<bson::Document>(OVERRIDES);
    let now = bson::DateTime::now();
    let override_id = Uuid::new_v4().to_string();
    overrides.insert_one(doc! {"_id": &override_id, "flag_key": "auth:staging-qr-login",
        "target_kind":"user", "target_key":&prod_id, "enabled":true, "created_at":now, "updated_at":now, "updated_by":"test"}).await.unwrap();
    // The matching account must already exist and be verified on staging.
    assert_eq!(
        post_json(&prod_server, path, Some(&token), preview.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut stage_user = test_user(&stage_id, UserType::Person);
    stage_user.email = format!("{prod_id}@example.com");
    stage_user.email_verified = false;
    let users = stage.db.collection::<crate::models::user::User>(USERS);
    users.insert_one(stage_user).await.unwrap();
    assert_eq!(
        post_json(&prod_server, path, Some(&token), preview.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    users
        .update_one(
            doc! {"_id": &stage_id},
            doc! {"$set":{"email_verified":true}},
        )
        .await
        .unwrap();
    let (status, reviewed) = post_json(&prod_server, path, Some(&token), preview).await;
    assert_eq!(status, StatusCode::OK, "{reviewed}");
    assert_eq!(reviewed["preview"]["client_label"], "Staging browser");
    assert_eq!(reviewed["binding"]["staging_user"], stage_id);
    assert_eq!(reviewed["binding"]["production_user"], prod_id);
    let decision = json!({"action":"approve", "binding":reviewed["binding"], "user_code":request["user_code"]});
    let mut stale = decision.clone();
    stale["binding"]["request_id"] = json!(Uuid::new_v4());
    assert_eq!(
        post_json(&prod_server, path, Some(&token), stale).await.0,
        StatusCode::FORBIDDEN
    );
    overrides
        .update_one(doc! {"_id": &override_id}, doc! {"$set":{"enabled":false}})
        .await
        .unwrap();
    assert_eq!(
        post_json(&prod_server, path, Some(&token), decision.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    overrides
        .update_one(doc! {"_id": &override_id}, doc! {"$set":{"enabled":true}})
        .await
        .unwrap();
    let (status, result) = post_json(&prod_server, path, Some(&token), decision.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result, json!({"ok":true}));
    assert_eq!(
        post_json(&prod_server, path, Some(&token), decision)
            .await
            .0,
        StatusCode::GONE
    );
    let (status, delivery) = post_json(
        &staging_server,
        "/api/v1/auth/device/poll",
        None,
        json!({"device_code":request["device_code"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claims = crate::crypto::jwt::verify_token(
        &stage.jwt_keys,
        &stage.config,
        delivery["access_token"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(claims.sub, stage_id);
    assert_eq!(claims.iss, stage.config.jwt_issuer);
    assert!(
        crate::crypto::jwt::verify_token(
            &prod.jwt_keys,
            &prod.config,
            delivery["access_token"].as_str().unwrap()
        )
        .is_err()
    );
    // The same phone can still use the ordinary production QR flow.
    let prod_request = request_and_approve_with_token(&prod_server, &token).await;
    let (status, local) = post_json(
        &prod_server,
        "/api/v1/auth/device/poll",
        None,
        json!({"device_code":prod_request["device_code"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        crate::crypto::jwt::verify_token(
            &prod.jwt_keys,
            &prod.config,
            local["access_token"].as_str().unwrap()
        )
        .unwrap()
        .sub,
        prod_id
    );

    // Denial also stays on staging, and the initiating browser cannot collect a session.
    let (_, other) = post_json(
        &staging_server,
        "/api/v1/auth/device/request",
        None,
        json!({}),
    )
    .await;
    let (_, second_preview) = post_json(
        &prod_server,
        path,
        Some(&token),
        json!({"action":"preview", "user_code":other["user_code"]}),
    )
    .await;
    let (status, _) = post_json(&prod_server, path, Some(&token),
        json!({"action":"deny", "user_code":other["user_code"], "binding":second_preview["binding"]})).await;
    assert_eq!(status, StatusCode::OK);
    let (_, denied) = post_json(
        &staging_server,
        "/api/v1/auth/device/poll",
        None,
        json!({"device_code":other["device_code"]}),
    )
    .await;
    assert_eq!(denied["error_code"], 11204);
    // A production bearer is never a staging QR assertion, even though it uses the same signing key.
    assert_eq!(
        post_json(
            &staging_server,
            "/api/v1/auth/device/staging/receive",
            None,
            json!({"proof":token})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
