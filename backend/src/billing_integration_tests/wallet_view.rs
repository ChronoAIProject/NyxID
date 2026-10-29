use super::*;
use crate::models::credits::Credits;
use crate::services::feature_flag_service as flags;
use futures::TryStreamExt;
use mongodb::bson::Document;
use serde_json::{Value, json};

mod credentials;
mod no_provider;
use no_provider::NoLago;

const VIEW: &str = "/api/v1/billing/wallet/view";
const LEGACY: &str = "/api/v1/billing/wallet";

fn mounted_app(state: &crate::AppState) -> Router {
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    private.with_state(state.clone())
}

fn view_state(db: mongodb::Database, lago: Arc<NoLago>) -> crate::AppState {
    let mut state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    state.billing = Arc::new(BillingService::new_with_lago(
        db,
        Arc::new(state.config.clone()),
        lago,
    ));
    state
}

async fn read_headers(app: &Router, path: &str, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn read(app: &Router, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let auth = token.map(|token| format!("Bearer {token}"));
    let headers: Vec<_> = auth
        .as_deref()
        .map(|auth| ("authorization", auth))
        .into_iter()
        .collect();
    read_headers(app, path, &headers).await
}

// Raw documents retain unknown fields and exact BSON types. Auth/security
// bookkeeping is outside this financial-state snapshot.
async fn financial_state(db: &mongodb::Database) -> Vec<(&'static str, Vec<Document>)> {
    let mut state = Vec::new();
    for collection in [
        BILLING_WALLET,
        "billing_tax_profiles",
        "billing_wallet_refresh_requests",
        "billing_ledger",
        "billing_accounts",
    ] {
        let documents = db
            .collection::<Document>(collection)
            .find(doc! {})
            .sort(doc! {"_id": 1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        state.push((collection, documents));
    }
    state
}

async fn seed_sentinels(db: &mongodb::Database) {
    for collection in ["billing_tax_profiles", "billing_wallet_refresh_requests"] {
        db.collection::<Document>(collection)
            .insert_one(doc! {
                "_id": "foreign-sentinel", "owner_id": "foreign-owner",
                "unmodeled": {"exact": "0.000000000001", "version": 2_i64}
            })
            .await
            .unwrap();
    }
}

async fn insert_wallet(db: &mongodb::Database, owner: &str) -> BillingWallet {
    let now = Utc::now();
    let wallet = BillingWallet {
        id: Uuid::new_v4().to_string(),
        owner_id: owner.into(),
        lago_customer_id: owner.into(),
        lago_wallet_id: Some("existing-wallet".into()),
        lago_subscription_id: None,
        plan_kind: PlanKind::Prepaid,
        balance_credits: "10.123456789012".parse().unwrap(),
        reserved_credits: "1.000000000001".parse().unwrap(),
        pending_lago_debits: "2.000000000002".parse().unwrap(),
        pending_topup_expiry_credits: "0.000000000003".parse().unwrap(),
        has_payment_instrument: false,
        overdraft_cap_credits: Credits::from_whole(5),
        suspended: false,
        collection_state: CollectionState::Good,
        topup_expiry_checked_at: None,
        active_topup_expiry: None,
        balance_synced_at: now,
        created_at: now,
        updated_at: now,
    };
    db.collection::<BillingWallet>(BILLING_WALLET)
        .insert_one(&wallet)
        .await
        .unwrap();
    db.collection::<Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id": &wallet.id},
            doc! {"$set": {
                "exact_accounting_version": 2_i64, "unmodeled_sentinel": {"keep": true}
            }},
        )
        .await
        .unwrap();
    wallet
}

fn assert_balances(body: &Value) {
    assert_eq!(
        body,
        &json!({
            "balance": "10.123456789012", "available": "7.123456789006", "overdraft_cap": "5",
            "reserved": "1.000000000001", "pending_debits": "2.000000000002", "pending_expiry": "0.000000000003",
            "unit": "credits"
        }),
        "dedicated balance DTO must contain no provider or operational fields"
    );
}

#[tokio::test]
async fn billing_wallet_view_existing_returns_only_exact_balances_without_mutation() {
    let db = connect_test_database("wallet_view_existing").await.unwrap();
    let owner = insert_owner(&db).await;
    let lago = Arc::new(NoLago::default());
    let state = view_state(db.clone(), lago.clone());
    insert_wallet(&db, &owner).await;
    seed_sentinels(&db).await;
    let token = route_access_token(&state, &owner);
    let app = mounted_app(&state);
    let before = financial_state(&db).await;
    let (status, body) = read(&app, VIEW, Some(&token)).await;
    let after = financial_state(&db).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_balances(&body);
    assert_eq!(after, before);
    lago.assert_unused();
    // Compare legacy amounts only after isolating this GET's raw side effects.
    let (status, legacy) = read(&app, LEGACY, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    for field in [
        "balance",
        "available",
        "overdraft_cap",
        "reserved",
        "pending_debits",
        "pending_expiry",
    ] {
        assert_eq!(legacy[field], body[field]);
    }
}

#[tokio::test]
async fn billing_wallet_view_absent_returns_null_preserves_foreign_wallet_and_never_provisions() {
    let db = connect_test_database("wallet_view_absent").await.unwrap();
    let owner = insert_owner(&db).await;
    let stranger = insert_owner(&db).await;
    insert_wallet(&db, &stranger).await;
    seed_sentinels(&db).await;
    let lago = Arc::new(NoLago::default());
    let state = view_state(db.clone(), lago.clone());
    let token = route_access_token(&state, &owner);
    let app = mounted_app(&state);
    let before = financial_state(&db).await;
    assert_eq!(before[0].1.len(), 1);
    for _ in 0..3 {
        assert_eq!(
            read(&app, VIEW, Some(&token)).await,
            (StatusCode::OK, Value::Null)
        );
        assert_eq!(financial_state(&db).await, before);
        lago.assert_unused();
    }
    assert!(
        db.collection::<Document>(BILLING_WALLET)
            .find_one(doc! {"owner_id": &owner})
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn billing_wallet_view_stale_subscribed_wallet_never_refreshes_or_contacts_provider() {
    let db = connect_test_database("wallet_view_stale").await.unwrap();
    let owner = insert_owner(&db).await;
    let wallet = insert_wallet(&db, &owner).await;
    let stale = bson::DateTime::from_chrono(Utc::now() - Duration::days(365));
    db.collection::<Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id": &wallet.id},
            doc! {"$set": {
                "lago_subscription_id": "live-subscription", "balance_synced_at": stale
            }},
        )
        .await
        .unwrap();
    seed_sentinels(&db).await;
    db.collection::<Document>("billing_tax_profiles")
        .insert_one(doc! {
            "_id": &owner, "owner_id": &owner, "synced_at": stale, "unmodeled": "preserve"
        })
        .await
        .unwrap();
    let lago = Arc::new(NoLago::default());
    let state = view_state(db.clone(), lago.clone());
    let app = mounted_app(&state);
    let token = route_access_token(&state, &owner);
    let before = financial_state(&db).await;
    let (status, body) = read(&app, VIEW, Some(&token)).await;
    assert_eq!(financial_state(&db).await, before);
    lago.assert_unused();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_balances(&body);
}

#[tokio::test]
async fn billing_wallet_view_requires_auth_and_rejects_api_key_shapes() {
    let db = connect_test_database("wallet_view_auth").await.unwrap();
    let state = view_state(db, Arc::new(NoLago::default()));
    let app = mounted_app(&state);
    assert_eq!(read(&app, VIEW, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        read(&app, VIEW, Some("invalid")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        read(&app, VIEW, Some("nyxid_ag_test")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        read_headers(&app, VIEW, &[("x-api-key", "nyxid_ag_test")])
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn billing_wallet_view_preserves_disabled_rollout_behavior() {
    let db = connect_test_database("wallet_view_rollout").await.unwrap();
    let owner = insert_owner(&db).await;
    let lago = Arc::new(NoLago::default());
    let state = view_state(db.clone(), lago.clone());
    let token = route_access_token(&state, &owner);
    let app = mounted_app(&state);
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        false,
        &owner,
    )
    .await
    .unwrap();
    let legacy = read(&app, LEGACY, Some(&token)).await;
    assert_eq!(legacy.0, StatusCode::FORBIDDEN);
    assert_eq!(read(&app, VIEW, Some(&token)).await, legacy);
    assert_eq!(
        db.collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    insert_wallet(&db, &owner).await;
    assert_eq!(read(&app, VIEW, Some(&token)).await, legacy);
    lago.assert_unused();
}

#[tokio::test]
async fn billing_wallet_view_without_lago_or_charging_reads_existing_or_null() {
    let db = connect_test_database("wallet_view_disabled").await.unwrap();
    let owner = insert_owner(&db).await;
    let absent = insert_owner(&db).await;
    insert_wallet(&db, &owner).await;
    let state = test_app_state_with_config(db.clone(), test_app_config());
    assert!(!state.billing.billing_enabled());
    assert!(!state.billing.lago_configured());
    let token = route_access_token(&state, &owner);
    let app = mounted_app(&state);
    let before = financial_state(&db).await;
    let (status, body) = read(&app, VIEW, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_balances(&body);
    let absent_token = route_access_token(&state, &absent);
    assert_eq!(
        read(&app, VIEW, Some(&absent_token)).await,
        (StatusCode::OK, Value::Null)
    );
    assert_eq!(financial_state(&db).await, before);
}
