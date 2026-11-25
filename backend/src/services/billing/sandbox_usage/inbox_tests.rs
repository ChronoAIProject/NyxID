use super::{
    inbox::{self, InboxError},
    inbox_authority::{self, Authority, READ_SCOPE, Registration, WRITE_SCOPE},
};
use crate::models::sandbox_usage_inbox::Validation;
use bson::{DateTime, Document, doc};
use mongodb::Database;
use uuid::Uuid;

const NAMESPACE: &str = "cma-example";
const ISSUER: &str = "nyxid-example";
const OPEN: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/stream-open.canonical.json");
const USAGE: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/allocated.canonical.json");
const CORRECTION: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/correction.canonical.json");
const SEAL: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/stream-seal.canonical.json");

struct Fixture {
    db: Database,
    account: String,
    jti: String,
    scopes: String,
}

impl Fixture {
    async fn open(name: &str, namespace: &str) -> Self {
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        Self::on(db, namespace).await
    }

    async fn on(db: Database, namespace: &str) -> Self {
        inbox::prepare(&db).await.unwrap();
        let account = Uuid::new_v4().to_string();
        let scopes = format!("{READ_SCOPE} {WRITE_SCOPE}");
        let now = DateTime::now();
        db.collection::<Document>("service_accounts")
            .insert_one(doc! {
                "_id":&account, "name":"CMA custody test",
                "client_id":format!("sa_{}", Uuid::new_v4().simple()),
                "client_secret_hash":"test-only", "secret_prefix":"test",
                "allowed_scopes":&scopes, "is_active":true, "credential_generation":0_i64,
                "purpose":"general", "created_by":"fixture", "created_at":now, "updated_at":now,
            })
            .await
            .unwrap();
        let jti = issue_token(&db, &account, &scopes, 0).await;
        let revision = inbox_authority::register(
            &db,
            Registration {
                account_id: &account,
                namespace,
                owner_issuer: ISSUER,
                expected_revision: 0,
                enabled: true,
                accepting: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(revision, 1);
        Self {
            db,
            account,
            jti,
            scopes,
        }
    }

    async fn authority(&self, scope: &'static str) -> Authority {
        Authority::load(&self.db, &self.account, &self.jti, &self.scopes, scope)
            .await
            .unwrap()
    }
}

async fn issue_token(db: &Database, account: &str, scope: &str, generation: i64) -> String {
    let jti = Uuid::new_v4().to_string();
    persist_token(db, account, scope, generation, &jti).await;
    jti
}

async fn persist_token(db: &Database, account: &str, scope: &str, generation: i64, jti: &str) {
    let now = DateTime::now();
    db.collection::<Document>("service_account_tokens")
        .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(), "jti":jti,"service_account_id":account,
            "scope":scope, "revoked":false, "credential_generation":generation,
            "created_at":now, "expires_at":DateTime::from_millis(now.timestamp_millis() + 60_000),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn mounted_custody_routes_require_the_correct_scope_and_recover_the_original_receipt() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    let fx = Fixture::open("cma_custody_http", NAMESPACE).await;
    let state = crate::test_utils::test_app_state(fx.db.clone());
    let mut tokens = Vec::new();
    for scope in [WRITE_SCOPE, READ_SCOPE] {
        let (jwt, jti) = crate::crypto::jwt::generate_service_account_token(
            &state.jwt_keys,
            &state.config,
            &fx.account,
            scope,
            60,
            0,
        )
        .unwrap();
        persist_token(&fx.db, &fx.account, scope, 0, &jti).await;
        tokens.push(jwt);
    }
    let (_, mounted) = crate::routes::build_router();
    let router = mounted.with_state(state);
    let call = |method: &str, path: &str, bearer: Option<&str>, body: &'static [u8]| {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/api/v1/billing/sandboxes{path}"))
            .header("content-type", "application/json");
        if let Some(bearer) = bearer {
            request = request.header("authorization", format!("Bearer {bearer}"));
        }
        let request = request.body(Body::from(body)).unwrap();
        let router = router.clone();
        async move {
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 512 * 1024).await.unwrap();
            let body = serde_json::from_slice::<Value>(&bytes)
                .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
            (status, body)
        }
    };
    let events = "/usage-events";
    let writer = Some(tokens[0].as_str());
    let reader = Some(tokens[1].as_str());
    assert_eq!(
        call("POST", events, None, OPEN).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call("POST", events, reader, OPEN).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, body) = call("POST", events, writer, USAGE).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "stream_not_open");

    let (status, body) = call("POST", events, writer, OPEN).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        body["processing"],
        json!({
            "validation_status": "validated",
            "validation_reason": null,
            "owner_status": "unresolved",
            "financial_status": "shadow",
        }),
    );
    let original = body["receipt"].clone();
    let receipt = format!("/usage-receipts/{}", original["event_id"].as_str().unwrap());
    let cursor = format!("/usage-cursors/{}", original["stream_id"].as_str().unwrap());
    for path in [&receipt, &cursor] {
        let (status, body) = call("GET", path, writer, b"").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }
    let (status, observed) = call("GET", &receipt, reader, b"").await;
    assert_eq!(status, StatusCode::OK, "{observed}");
    assert_eq!(observed["receipt"], original);
    let (status, observed) = call("POST", events, writer, OPEN).await;
    assert_eq!(status, StatusCode::OK, "{observed}");
    assert_eq!(observed["receipt"], original);
    let (status, cursor_page) = call("GET", &cursor, reader, b"").await;
    assert_eq!(status, StatusCode::OK, "{cursor_page}");
    assert_eq!(
        cursor_page,
        json!({
            "stream_id": original["stream_id"],
            "contiguous_custody_sequence": "1",
            "custody_chain_head": original["payload_digest"],
            "contiguous_validated_sequence": "1",
            "validated_chain_head": original["payload_digest"],
            "highest_received_sequence": "1",
            "missing_ranges": [],
            "next_page": null,
        }),
    );
    let (status, body) = call("GET", &format!("{cursor}?page=outdated"), reader, b"").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "conflict");
}

#[tokio::test]
async fn replay_retains_one_receipt_after_concurrent_submission_and_sealing() {
    let fx = Fixture::open("cma_custody_replay", NAMESPACE).await;
    let writer = fx.authority(WRITE_SCOPE).await;
    let reader = fx.authority(READ_SCOPE).await;
    assert!(matches!(
        inbox::accept(&fx.db, &writer, USAGE).await,
        Err(InboxError::Prerequisite("stream_not_open"))
    ));
    inbox::accept(&fx.db, &writer, OPEN).await.unwrap();
    assert!(matches!(
        inbox::accept(&fx.db, &writer, CORRECTION).await,
        Err(InboxError::Prerequisite("predecessor_missing"))
    ));
    let (left, right) = tokio::join!(
        inbox::accept(&fx.db, &writer, USAGE),
        inbox::accept(&fx.db, &writer, USAGE)
    );
    let (first, receipt) = left.unwrap();
    let (second, concurrent) = right.unwrap();
    let receipt = receipt.receipt;
    assert_ne!(first, second);
    assert_eq!(receipt, concurrent.receipt);
    inbox::accept(&fx.db, &writer, CORRECTION).await.unwrap();
    inbox::accept(&fx.db, &writer, SEAL).await.unwrap();
    let (created, replay) = inbox::accept(&fx.db, &writer, USAGE).await.unwrap();
    assert!(!created);
    assert_eq!(receipt, replay.receipt);
    assert_eq!(
        Some(receipt.clone()),
        inbox::receipt(&fx.db, &reader, receipt.event_id.as_str())
            .await
            .unwrap()
            .map(|value| value.receipt)
    );
    let cursor = inbox::cursor(&fx.db, &reader, receipt.stream_id.as_str(), None)
        .await
        .unwrap()
        .unwrap();
    let cursor = serde_json::to_value(cursor).unwrap();
    assert_eq!(cursor["contiguous_validated_sequence"], "4");
    assert_eq!(cursor["contiguous_custody_sequence"], "4");
    assert_eq!(
        fx.db
            .collection::<Document>("sandbox_usage_events_v1")
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        fx.db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn replaced_token_can_read_retained_custody_after_new_intake_closes() {
    let mut fx = Fixture::open("cma_custody_rotation", NAMESPACE).await;
    let stale = fx.authority(WRITE_SCOPE).await;
    let stale_read = fx.authority(READ_SCOPE).await;
    let (_, receipt) = inbox::accept(&fx.db, &stale, OPEN).await.unwrap();
    let receipt = receipt.receipt;
    inbox_authority::register(
        &fx.db,
        Registration {
            account_id: &fx.account,
            namespace: NAMESPACE,
            owner_issuer: ISSUER,
            expected_revision: 1,
            enabled: true,
            accepting: false,
        },
    )
    .await
    .unwrap();
    fx.db
        .collection::<Document>("service_accounts")
        .update_one(
            doc! {"_id":&fx.account},
            doc! {"$inc":{"credential_generation":1_i64}},
        )
        .await
        .unwrap();
    fx.jti = issue_token(&fx.db, &fx.account, &fx.scopes, 1).await;
    assert!(matches!(
        inbox::accept(&fx.db, &stale, USAGE).await,
        Err(InboxError::Forbidden)
    ));
    assert!(matches!(
        inbox::receipt(&fx.db, &stale_read, receipt.event_id.as_str()).await,
        Err(InboxError::Forbidden)
    ));
    let read = fx.authority(READ_SCOPE).await;
    assert_eq!(
        inbox::receipt(&fx.db, &read, receipt.event_id.as_str())
            .await
            .unwrap()
            .map(|value| value.receipt),
        Some(receipt.clone())
    );
    let write = fx.authority(WRITE_SCOPE).await;
    let (created, replay) = inbox::accept(&fx.db, &write, OPEN).await.unwrap();
    assert!(!created);
    assert_eq!(receipt, replay.receipt);
    assert!(matches!(
        inbox::accept(&fx.db, &write, USAGE).await,
        Err(InboxError::Unavailable)
    ));
    let foreign = Fixture::on(fx.db.clone(), "foreign-fixture").await;
    let other = foreign.authority(READ_SCOPE).await;
    assert!(
        inbox::receipt(&fx.db, &other, receipt.event_id.as_str())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        inbox::cursor(&fx.db, &other, receipt.stream_id.as_str(), None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn missing_sequence_preserves_sparse_custody_until_validation_can_continue() {
    let fx = Fixture::open("cma_custody_sparse", NAMESPACE).await;
    let writer = fx.authority(WRITE_SCOPE).await;
    let reader = fx.authority(READ_SCOPE).await;
    inbox::accept(&fx.db, &writer, OPEN).await.unwrap();
    let mut later: serde_json::Value = serde_json::from_slice(USAGE).unwrap();
    later["sequence"] = serde_json::json!("3");
    later["event_id"] = serde_json::json!("00000000-0000-4000-8000-00000000004c");
    let current = super::contract::decode_event(USAGE, inbox::clock()).unwrap();
    later["previous_event_digest"] = serde_json::json!(current.digest().as_str());
    later["facts"] = serde_json::json!([]);
    let bytes = serde_json::to_vec(&later).unwrap();
    let (created, deferred) = inbox::accept(&fx.db, &writer, &bytes).await.unwrap();
    assert!(created);
    assert!(matches!(deferred.validation, Validation::Pending));
    let stream = deferred.receipt.stream_id.as_str();
    let page = inbox::cursor(&fx.db, &reader, stream, None)
        .await
        .unwrap()
        .unwrap();
    let page = serde_json::to_value(page).unwrap();
    assert_eq!(page["highest_received_sequence"], "3");
    assert_eq!(page["contiguous_custody_sequence"], "1");
    assert_eq!(
        page["missing_ranges"],
        serde_json::json!([{"first":"2","last":"2"}])
    );
    inbox::accept(&fx.db, &writer, USAGE).await.unwrap();
    let repaired = inbox::receipt(&fx.db, &reader, deferred.receipt.event_id.as_str())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deferred.receipt, repaired.receipt);
    assert!(matches!(repaired.validation, Validation::Validated));
    let page = inbox::cursor(&fx.db, &reader, stream, None)
        .await
        .unwrap()
        .unwrap();
    let page = serde_json::to_value(page).unwrap();
    assert_eq!(page["contiguous_validated_sequence"], "3");
    assert_eq!(page["missing_ranges"], serde_json::json!([]));
}

#[tokio::test]
async fn correction_keeps_its_historical_owner_and_usage_time_tariff_after_erasure() {
    use super::{inbox_shadow, pricing_schema, pricing_store_tests, pricing_tests};
    use crate::models::{
        sandbox_usage_rates::{COLLECTION_NAME as RATES, UsageTariff},
        user::{COLLECTION_NAME as USERS, User, UserType},
    };
    let fx = Fixture::open("cma_custody_shadow", NAMESPACE).await;
    let writer = fx.authority(WRITE_SCOPE).await;
    pricing_store_tests::enable(&fx.db, true).await;
    pricing_schema::prepare(&fx.db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    fx.db
        .collection::<User>(USERS)
        .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let initial_tariff = pricing_tests::tariff();
    fx.db
        .collection::<UsageTariff>(RATES)
        .insert_one(&initial_tariff)
        .await
        .unwrap();
    let (open, open_hash) = with_owner(OPEN, &owner, None, false);
    let (usage, usage_hash) = with_owner(USAGE, &owner, Some(&open_hash), false);
    inbox::accept(&fx.db, &writer, &open).await.unwrap();
    let (_, accepted) = inbox::accept(&fx.db, &writer, &usage).await.unwrap();
    let stream = accepted.receipt.stream_id.as_str();
    let fact = "00000000-0000-4000-8000-000000000064";
    let before = inbox_shadow::inspect(&fx.db, ISSUER, NAMESPACE, stream, fact)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before["owner_status"], "resolved");
    assert_eq!(before["financial_status"], "shadow");
    assert_eq!(before["plans"][0]["tariff_id"], initial_tariff.id);

    let mut later_tariff = pricing_tests::tariff();
    later_tariff.effective_from += chrono::Duration::minutes(1);
    later_tariff.version = 2;
    fx.db
        .collection::<UsageTariff>(RATES)
        .insert_one(&later_tariff)
        .await
        .unwrap();
    fx.db
        .collection::<User>(USERS)
        .delete_one(doc! {"_id":&owner})
        .await
        .unwrap();
    let (correction, _) = with_owner(CORRECTION, &owner, Some(&usage_hash), true);
    inbox::accept(&fx.db, &writer, &correction).await.unwrap();
    let after = inbox_shadow::inspect(&fx.db, ISSUER, NAMESPACE, stream, fact)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after["owner_status"], "resolved");
    assert_eq!(after["financial_status"], "shadow");
    assert_ne!(after["source_event_id"], before["source_event_id"]);
    assert_eq!(
        after["plans"][0]["tariff_id"],
        before["plans"][0]["tariff_id"]
    );
    assert_eq!(after["plans"][0]["pin_id"], before["plans"][0]["pin_id"]);
    assert_eq!(
        fx.db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

fn with_owner(
    bytes: &[u8],
    owner: &str,
    previous: Option<&str>,
    correction: bool,
) -> (Vec<u8>, String) {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    value["owner_ref"]["subject"] = serde_json::json!(owner);
    if let Some(previous) = previous {
        value["previous_event_digest"] = serde_json::json!(previous);
        if correction {
            value["correction"]["expected_predecessor_digest"] = serde_json::json!(previous);
        }
    }
    let bytes = serde_json::to_vec(&value).unwrap();
    let event = super::contract::decode_event(&bytes, inbox::clock()).unwrap();
    (bytes, event.digest().as_str().to_string())
}
