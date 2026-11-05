use super::*;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
use crate::services::feature_flag_service as flags;
use serde_json::{Value, json};

async fn organization(db: &mongodb::Database, admin: &str) -> String {
    let org = insert_owner(db).await;
    db.collection::<User>(USERS)
        .update_one(doc! {"_id": &org}, doc! {"$set": {"user_type": "org"}})
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            admin,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    org
}

fn app(state: &crate::AppState) -> Router {
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    private.with_state(state.clone())
}

async fn request(
    app: &Router,
    token: &str,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(route_request(
            method,
            path,
            token,
            if body.is_null() {
                Body::empty()
            } else {
                Body::from(body.to_string())
            },
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn org_wallet_read_is_non_provisioning_and_admin_setup_checkout_history_are_scoped() {
    let db = crate::test_utils::connect_transaction_test_database("org_wallet_web_admin").await;
    let admin = insert_owner(&db).await;
    let org = organization(&db, &admin).await;
    let lago = Arc::new(FakeLago::default());
    let state = billing_route_state(db.clone(), lago.clone(), 0);
    let app = app(&state);
    let token = route_access_token(&state, &admin);
    let wallet_path = format!("/api/v1/billing/wallet?owner_id={org}");
    assert_eq!(
        request(&app, &token, Method::GET, &wallet_path, Value::Null).await,
        (StatusCode::OK, Value::Null)
    );
    // Org-owned keys use the org as their principal, so the existing ACL
    // classifies access as Direct. Explicit GET must still never provision.
    assert!(
        crate::handlers::billing::get_wallet(
            axum::extract::State(state.clone()),
            crate::test_utils::test_auth_user(&org),
            axum::extract::Query(crate::handlers::billing::ProvisionWalletRequest {
                owner_id: Some(org.clone()),
            }),
        )
        .await
        .unwrap()
        .0
        .is_none()
    );
    assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 0);
    assert_eq!(
        db.collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );

    let (status, wallet) = request(
        &app,
        &token,
        Method::POST,
        "/api/v1/billing/wallet",
        json!({"owner_id":org}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{wallet}");
    assert_eq!(wallet["owner_id"], org);
    assert!(wallet["lago_subscription_id"].is_string());
    let (_, read) = request(&app, &token, Method::GET, &wallet_path, Value::Null).await;
    assert_eq!(read["owner_id"], org);
    assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
    let (status, checkout) = request(
        &app,
        &token,
        Method::POST,
        "/api/v1/billing/topup",
        json!({"owner_id":org,"amount_credits":10,"idempotency_key":"org-web-checkout"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{checkout}");
    assert_eq!(checkout["owner_id"], org);
    assert_eq!(checkout["checkout_url"], "https://pay.example/checkout");
    assert_eq!(
        *lago.topup_wallets.lock().unwrap(),
        vec![wallet["lago_wallet_id"].as_str().unwrap().to_owned()]
    );
    let (status, history) = request(
        &app,
        &token,
        Method::GET,
        &format!("/api/v1/billing/topups?owner_id={org}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["total"], 1);
    assert_eq!(history["topups"][0]["amount_credits"], 10);
    let (_, personal) = request(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/topups",
        Value::Null,
    )
    .await;
    assert_eq!(personal["total"], 0);
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([super::usage::meter(&org, 3), super::usage::meter(&admin, 7)])
        .await
        .unwrap();
    let (status, usage) = request(
        &app,
        &token,
        Method::GET,
        &format!("/api/v1/billing/usage?owner_id={org}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["owner_id"], org);
    assert_eq!(usage["totals"]["quantity"], 3);
    let (_, personal_usage) = request(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/usage",
        Value::Null,
    )
    .await;
    assert_eq!(personal_usage["totals"]["quantity"], 7);

    // Omitted owner keeps the historical personal GET provisioning behavior.
    let (status, personal) = request(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/wallet",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{personal}");
    assert_eq!(personal["owner_id"], admin);
    assert_eq!(personal["created"], true);
    assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 2);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_wallet_member_viewer_stranger_and_revoked_admin_cannot_manage_or_view() {
    let db = crate::test_utils::connect_transaction_test_database("org_wallet_web_acl").await;
    let admin = insert_owner(&db).await;
    let org = organization(&db, &admin).await;
    let lago = Arc::new(FakeLago::default());
    let state = billing_route_state(db.clone(), lago.clone(), 0);
    let app = app(&state);
    for role in [Some(OrgRole::Member), Some(OrgRole::Viewer), None] {
        let actor = insert_owner(&db).await;
        if let Some(role) = role {
            db.collection::<OrgMembership>(MEMBERSHIPS)
                .insert_one(crate::test_utils::test_membership(&org, &actor, role, None))
                .await
                .unwrap();
        }
        assert_refused(&app, &route_access_token(&state, &actor), &org).await;
    }
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .update_many(
            doc! {"org_user_id":&org},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_refused(&app, &route_access_token(&state, &admin), &org).await;
    assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 0);
    assert!(lago.topup_wallets.lock().unwrap().is_empty());
    db.drop().await.unwrap();
}

async fn assert_refused(app: &Router, token: &str, org: &str) {
    for path in ["wallet", "topups", "usage"] {
        let (status, body) = request(
            app,
            token,
            Method::GET,
            &format!("/api/v1/billing/{path}?owner_id={org}"),
            Value::Null,
        )
        .await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{path}: {status} {body}"
        );
    }
    for path in ["wallet", "topup"] {
        let (status, body) = request(
            app,
            token,
            Method::POST,
            &format!("/api/v1/billing/{path}"),
            json!({"owner_id":org,"amount_credits":10,"idempotency_key":"refused-topup"}),
        )
        .await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{path}: {status} {body}"
        );
    }
}

#[tokio::test]
async fn org_wallet_rollout_still_applies_and_only_explicit_setup_repairs_subscription() {
    let db =
        crate::test_utils::connect_transaction_test_database("org_wallet_web_subscription").await;
    let admin = insert_owner(&db).await;
    let org = organization(&db, &admin).await;
    let lago = Arc::new(FakeLago::default());
    let state = billing_route_state(db.clone(), lago.clone(), 0);
    let app = app(&state);
    let token = route_access_token(&state, &admin);
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        false,
        &admin,
    )
    .await
    .unwrap();
    assert_refused(&app, &token, &org).await;
    assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 0);
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &admin,
    )
    .await
    .unwrap();
    let original = state.billing.ensure_wallet(&org).await.unwrap().wallet;
    db.collection::<BillingWallet>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&org},
            doc! {"$unset":{"lago_subscription_id":""}},
        )
        .await
        .unwrap();
    let (_, read) = request(
        &app,
        &token,
        Method::GET,
        &format!("/api/v1/billing/wallet?owner_id={org}"),
        Value::Null,
    )
    .await;
    assert!(read["lago_subscription_id"].is_null());
    assert!(
        state
            .billing
            .get_wallet(&org)
            .await
            .unwrap()
            .unwrap()
            .lago_subscription_id
            .is_none()
    );
    let (status, repaired) = request(
        &app,
        &token,
        Method::POST,
        "/api/v1/billing/wallet",
        json!({"owner_id":org}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repaired}");
    let current = state.billing.get_wallet(&org).await.unwrap().unwrap();
    assert_eq!(current.lago_subscription_id, original.lago_subscription_id);
    assert_eq!(current.balance_credits, original.balance_credits);
    assert_eq!(current.reserved_credits, original.reserved_credits);
    assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
    db.drop().await.unwrap();
}
