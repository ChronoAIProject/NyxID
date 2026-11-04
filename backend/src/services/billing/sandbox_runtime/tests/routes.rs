use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(
    router: &Router,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/api/v1/billing/sandboxes{path}"));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let request = request
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| json!({"body":String::from_utf8_lossy(&body)})),
    )
}

#[tokio::test]
async fn mounted_producer_contract_authenticates_and_recovers_receipts_and_cursor() {
    let mut f = fixture("sandbox_routes").await;
    let mut state =
        crate::test_utils::test_app_state_with_config(f.db.clone(), (*f.billing.config).clone());
    state.billing = Arc::new(BillingService::new_with_lago(
        f.db.clone(),
        f.billing.config.clone(),
        f.provider.clone(),
    ));
    let (token, jti) = crate::crypto::jwt::generate_service_account_token(
        &state.jwt_keys,
        &state.config,
        &f.producer.id,
        WRITE_SCOPE,
        3600,
        0,
    )
    .unwrap();
    f.db.collection::<Document>("service_account_tokens")
        .update_one(
            doc! {"jti": &f.producer.token_jti},
            doc! {"$set":{"jti":&jti}},
        )
        .await
        .unwrap();
    f.producer.token_jti = jti;
    let human = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&f.owner).unwrap(),
        "openid",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, mounted) = crate::routes::build_router();
    let router = mounted.with_state(state);
    let sandbox = format!("sbx_{}", Uuid::new_v4().simple());
    let quote = json!({"owner_id":f.owner, "sandbox_id":sandbox, "allocation":allocation()});
    for (bearer, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(human.as_str()), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            call(&router, bearer, "POST", "/quotes", quote.clone())
                .await
                .0,
            expected
        );
        assert_eq!(
            call(
                &router,
                bearer,
                "GET",
                &format!("/cursors/{sandbox}"),
                Value::Null
            )
            .await
            .0,
            expected
        );
    }
    let (status, absent) = call(
        &router,
        Some(&token),
        "GET",
        &format!("/cursors/{sandbox}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{absent}");
    let (status, quoted) = call(&router, Some(&token), "POST", "/quotes", quote).await;
    assert_eq!(status, StatusCode::OK, "{quoted}");
    assert_eq!(quoted["prices"].as_array().unwrap().len(), 3);
    let input = json!({"request_id":Uuid::new_v4().to_string(),"quote_id":quoted["quote_id"],
        "owner_id":f.owner,"sandbox_id":sandbox,"runtime_generation":1,"operation_id":Uuid::new_v4().to_string(),
        "previous_window_id":null,"seconds":2,"allocation":allocation()});
    let (status, admitted) = call(&router, Some(&token), "POST", "/windows", input.clone()).await;
    assert_eq!(status, StatusCode::OK, "{admitted}");
    assert_eq!(admitted["evidence_accepted"], false);
    assert_eq!(admitted["components"].as_array().unwrap().len(), 3);
    let receipt_path = format!("/receipts/{}", input["request_id"].as_str().unwrap());
    assert_eq!(
        call(&router, Some(&token), "GET", &receipt_path, Value::Null)
            .await
            .1,
        admitted
    );
    assert_eq!(
        call(&router, Some(&token), "POST", "/windows", input.clone())
            .await
            .1,
        admitted
    );
    let mut changed = input;
    changed["seconds"] = json!(3);
    assert_eq!(
        call(&router, Some(&token), "POST", "/windows", changed)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let stored = f
        .billing
        .sandbox_window_receipt(&f.producer, admitted["request_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    let window = elapsed(&f, stored).await;
    let mut proof = serde_json::to_value(evidence(&window)).unwrap();
    proof.as_object_mut().unwrap().remove("window_id");
    let (status, accepted) = call(
        &router,
        Some(&token),
        "POST",
        &format!("/windows/{}/evidence", window.id),
        proof.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["evidence_accepted"], true);
    assert_eq!(accepted["last_evidence_revision"], 2);
    let (status, cursor) = call(
        &router,
        Some(&token),
        "GET",
        &format!("/cursors/{sandbox}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cursor}");
    assert_eq!(cursor["last_evidence_revision"], 2);
    assert_eq!(cursor["evidence_covered_until"], window.covered_until);
    assert_eq!(
        call(
            &router,
            Some(&token),
            "POST",
            &format!("/windows/{}/evidence", window.id),
            proof
        )
        .await
        .1,
        accepted
    );
    f.db.collection::<Document>("service_account_tokens")
        .update_one(
            doc! {"jti":&f.producer.token_jti},
            doc! {"$set":{"revoked":true}},
        )
        .await
        .unwrap();
    assert_eq!(
        call(&router, Some(&token), "GET", &receipt_path, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
