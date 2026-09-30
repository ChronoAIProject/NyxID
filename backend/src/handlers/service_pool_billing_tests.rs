use super::*;
use crate::errors::AppResult;
use crate::models::billing_wallet::BillingWallet;
use crate::models::credits::Credits;
use crate::models::downstream_service::DownstreamService;
use crate::models::service_billing::{
    BillingMetric, LanePricing, PricingSyncStatus, ServiceBilling,
};
use crate::models::usage_meter::{CredentialClass, UsageMeterRow, UsageStatus};
use crate::services::billing::BillingService;
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
};
use crate::services::billing::ledger;
use futures::TryStreamExt;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

struct EntitledLago;

#[async_trait::async_trait]
impl LagoApi for EntitledLago {
    async fn ensure_customer(&self, input: &OwnerProvisionInput) -> AppResult<String> {
        Ok(input.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, customer: &str, _: &str) -> AppResult<String> {
        Ok(customer.into())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        Ok(events
            .iter()
            .map(|event| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: json!({}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        Ok(1)
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "*".into(),
            raw: json!({}),
        }])
    }
}

struct BilledFixture {
    proxy: Fixture,
    first: MockServer,
    second: MockServer,
}

async fn billed_fixture(label: &str, first_response: ResponseTemplate) -> BilledFixture {
    let mut proxy = fixture(label, StatusCode::TOO_MANY_REQUESTS, "priority", false).await;
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(first_response)
        .mount(&first)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "review-billed", "object": "chat.completion", "choices": [],
            "usage": { "prompt_tokens": 1, "completion_tokens": 0, "total_tokens": 1 }
        })))
        .mount(&second)
        .await;

    let owner = proxy.auth.user_id.to_string();
    let db = &proxy.state.db;
    let mut catalog = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! { "slug": "pool-review-api" })
        .await
        .unwrap()
        .unwrap();
    catalog.base_url = first.uri();
    catalog.auth_method = "bearer".into();
    catalog.auth_key_name = "Authorization".into();
    catalog.platform_key =
        Some(serde_json::from_value(json!({ "enabled": true, "audience": "public" })).unwrap());
    catalog.credential_encrypted = proxy
        .state
        .encryption_keys
        .encrypt(b"review-platform-secret")
        .await
        .unwrap();
    let lane = |code: &str| LanePricing {
        metric: BillingMetric::Tokens,
        credits_per_unit: "1".into(),
        lago_metric_code: code.into(),
        sync_status: PricingSyncStatus::Synced,
        sync_error: None,
        components: vec![],
    };
    catalog.billing = Some(ServiceBilling {
        byok_pricing: Some(lane("platform_svc_pool-review-api_byok")),
        platform_key_pricing: Some(lane("platform_svc_pool-review-api_pk")),
        ..Default::default()
    });
    db.collection::<DownstreamService>("downstream_services")
        .replace_one(doc! { "_id": &catalog.id }, &catalog)
        .await
        .unwrap();
    for (slug, url) in [
        ("review-first", first.uri()),
        ("review-second", second.uri()),
    ] {
        let service = db
            .collection::<Document>("user_services")
            .find_one(doc! { "slug": slug })
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>("user_endpoints")
            .update_one(
                doc! { "_id": service.get_str("endpoint_id").unwrap() },
                doc! { "$set": { "url": url } },
            )
            .await
            .unwrap();
    }
    db.collection::<Document>("user_services").update_one(doc! { "slug": "review-first" }, doc! { "$set": {
        "credential_binding": "platform", "auth_method": "bearer", "auth_key_name": "Authorization"
    } }).await.unwrap();
    let key_id = Uuid::new_v4().to_string();
    let encrypted = proxy
        .state
        .encryption_keys
        .encrypt(b"review-byok-secret")
        .await
        .unwrap();
    let now = mongodb::bson::DateTime::now();
    db.collection::<Document>("user_api_keys").insert_one(doc! {
        "_id": &key_id, "user_id": &owner, "label": "Review BYOK", "credential_type": "bearer",
        "status": "active", "credential_epoch": 1_i64,
        "credential_encrypted": mongodb::bson::Binary { subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: encrypted },
        "created_at": now, "updated_at": now,
    }).await.unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! { "slug": "review-second" },
            doc! { "$set": {
                "api_key_id": key_id, "auth_method": "bearer", "auth_key_name": "Authorization"
            } },
        )
        .await
        .unwrap();
    db.collection::<Document>("billing_wallet").insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "owner_id": &owner, "lago_customer_id": &owner,
        "lago_wallet_id": "review-wallet", "lago_subscription_id": "review-plan", "plan_kind": "prepaid",
        "balance_credits": Credits::from_whole(1), "reserved_credits": Credits::ZERO,
        "pending_lago_debits": Credits::ZERO, "overdraft_cap_credits": Credits::ZERO,
        "has_payment_instrument": false, "suspended": false, "collection_state": "good",
        "balance_synced_at": now, "created_at": now, "updated_at": now,
    }).await.unwrap();
    for code in [
        "platform_svc_pool-review-api_byok",
        "platform_svc_pool-review-api_pk",
    ] {
        db.collection::<Document>("billing_rate_cache")
            .insert_one(doc! {
                "_id": format!("{code}:*"), "lago_metric_code": code,
                "credits_per_unit_micros": 1_000_000_i64, "synced_at": now,
            })
            .await
            .unwrap();
    }
    proxy.state.config.billing_enabled = true;
    proxy.state.billing = Arc::new(BillingService::new_with_lago(
        db.clone(),
        Arc::new(proxy.state.config.clone()),
        Arc::new(EntitledLago),
    ));
    BilledFixture {
        proxy,
        first,
        second,
    }
}

async fn settled_rows(fixture: &BilledFixture, expected: usize) -> Vec<UsageMeterRow> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let rows: Vec<UsageMeterRow> = fixture
                .proxy
                .state
                .db
                .collection("usage_meter")
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if rows.len() == expected && rows.iter().all(|row| row.released) {
                return rows;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("all attempt reservations are durably finalized or released")
}

#[tokio::test]
async fn pool_proxy_revoked_platform_grant_excludes_member_before_byok_dispatch() {
    let fixture = billed_fixture("pool_revoked_platform", ResponseTemplate::new(200)).await;
    fixture
        .proxy
        .state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! { "slug": "pool-review-api" },
            doc! { "$set": {
                "platform_key.audience": "restricted", "platform_key.allowed_owner_ids": [],
            } },
        )
        .await
        .unwrap();
    fixture
        .proxy
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.proxy.pool_id },
            doc! { "$set": {
                "failover.max_attempts": 1,
            } },
        )
        .await
        .unwrap();
    let mut expected_decrypts = fixture.proxy.state.encryption_keys.decrypt_stats();
    expected_decrypts.v2_current += 1;
    let response = call(&fixture.proxy, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(fixture.first.received_requests().await.unwrap().is_empty());
    assert_eq!(fixture.second.received_requests().await.unwrap().len(), 1);
    let rows = settled_rows(&fixture, 1).await;
    assert_eq!(rows[0].credential_class, CredentialClass::UserOwned);
    assert_eq!(
        fixture.proxy.state.encryption_keys.decrypt_stats(),
        expected_decrypts
    );
    fixture.proxy.state.db.drop().await.unwrap();
}

async fn assert_wallet(fixture: &BilledFixture, pending: i64) {
    let wallet = fixture
        .proxy
        .state
        .db
        .collection::<BillingWallet>("billing_wallet")
        .find_one(doc! { "owner_id": fixture.proxy.auth.user_id.to_string() })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        wallet.reserved_credits,
        Credits::ZERO,
        "no attempt may strand a wallet hold"
    );
    assert_eq!(wallet.pending_lago_debits, Credits::from_whole(pending));
}

async fn assert_successful_backup(fixture: &BilledFixture) {
    let response = call(&fixture.proxy, "{}").await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "one credit must fund the backup after releasing the failed primary"
    );
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    to_bytes(response.into_body(), 8192).await.unwrap();
    let rows = settled_rows(fixture, 2).await;
    let primary = rows
        .iter()
        .find(|row| row.credential_class == CredentialClass::NyxidManagedMaster)
        .unwrap();
    let backup = rows
        .iter()
        .find(|row| row.credential_class == CredentialClass::UserOwned)
        .unwrap();
    assert_ne!(primary.billing_request_id, backup.billing_request_id);
    assert_eq!(
        primary.billing_owner_id,
        fixture.proxy.auth.user_id.to_string()
    );
    assert_eq!(
        backup.billing_owner_id,
        fixture.proxy.auth.user_id.to_string()
    );
    assert!(
        primary.quantity.is_none_or(|quantity| quantity == 0),
        "a rejection cannot use byte-based token estimates"
    );
    assert_eq!(backup.status, UsageStatus::Finalized);
    assert_eq!(backup.quantity, Some(1));
    assert_wallet(fixture, 1).await;
    let report = ledger::verify_chain(
        &fixture.proxy.state.db,
        &ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(report.status, ledger::BillingLedgerStatus::Ok);
    assert_eq!(
        report.checked_count, 1,
        "only known consumed backup usage is charged"
    );
    let requests = fixture.second.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers["authorization"],
        "Bearer review-byok-secret"
    );
}

#[tokio::test]
async fn pool_proxy_billing_preserves_reported_consumption_from_both_attempts() {
    let fixture = billed_fixture(
        "pool_billing_consumed_rejection",
        ResponseTemplate::new(429).set_body_json(json!({
            "error": { "message": "quota reached after partial work" },
            "usage": { "prompt_tokens": 1, "completion_tokens": 0, "total_tokens": 1 }
        })),
    )
    .await;
    fixture
        .proxy
        .state
        .db
        .collection::<Document>("billing_wallet")
        .update_one(
            doc! { "owner_id": fixture.proxy.auth.user_id.to_string() },
            doc! { "$set": { "balance_credits": Credits::from_whole(2) } },
        )
        .await
        .unwrap();
    let response = call(&fixture.proxy, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    to_bytes(response.into_body(), 8192).await.unwrap();
    let rows = settled_rows(&fixture, 2).await;
    assert_ne!(rows[0].billing_request_id, rows[1].billing_request_id);
    for row in rows {
        assert_eq!(
            row.pool_attempt.as_ref().and_then(|a| a.outcome),
            Some(crate::models::usage_meter::PoolAttemptOutcome::Reported)
        );
        assert_eq!(
            row.quantity,
            Some(1),
            "reported provider consumption survives failover"
        );
        assert_eq!(row.status, UsageStatus::Finalized);
        assert_eq!(row.billing_owner_id, fixture.proxy.auth.user_id.to_string());
    }
    assert_wallet(&fixture, 2).await;
    let report = ledger::verify_chain(
        &fixture.proxy.state.db,
        &ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(report.status, ledger::BillingLedgerStatus::Ok);
    assert_eq!(
        report.checked_count, 2,
        "each consumed attempt is posted exactly once"
    );
    assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
    assert_eq!(fixture.second.received_requests().await.unwrap().len(), 1);
    let usage_events: Vec<Document> =
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let events: Vec<Document> = fixture
                    .proxy
                    .state
                    .db
                    .collection::<Document>("audit_log")
                    .find(doc! { "event_type": "llm_usage_reported" })
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                if events.len() >= 2 {
                    break events;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("both consumed attempts retain existing LLM usage auditing");
    assert_eq!(usage_events.len(), 2);
    for event in usage_events {
        assert_eq!(
            event.get_str("user_id").unwrap(),
            fixture.proxy.auth.user_id.to_string()
        );
        let data = event.get_document("event_data").unwrap();
        assert_eq!(data.get_i64("total_tokens").unwrap(), 1);
        assert!(!format!("{event:?}").contains("review-platform-secret"));
        assert!(!format!("{event:?}").contains("review-byok-secret"));
    }
    fixture.proxy.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_billing_releases_rejected_platform_hold_before_byok_admission() {
    let fixture = billed_fixture(
        "pool_billing_rejection",
        ResponseTemplate::new(429).set_body_json(json!({
            "error": { "message": "quota exhausted", "type": "rate_limit_error" }
        })),
    )
    .await;
    assert_successful_backup(&fixture).await;
    let requests = fixture.first.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers["authorization"],
        "Bearer review-platform-secret"
    );
    fixture.proxy.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_billing_releases_proven_unsent_hold_before_byok_admission() {
    let fixture = billed_fixture("pool_billing_unsent", ResponseTemplate::new(429)).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    fixture
        .proxy
        .state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! { "slug": "pool-review-api" },
            doc! { "$set": { "base_url": unavailable_url } },
        )
        .await
        .unwrap();
    fixture
        .proxy
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.proxy.pool_id },
            doc! { "$set": { "failover.retry_on": ["connect_error"] } },
        )
        .await
        .unwrap();
    assert_successful_backup(&fixture).await;
    assert!(fixture.first.received_requests().await.unwrap().is_empty());
    fixture.proxy.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_billing_unknown_timeout_releases_hold_without_inventing_usage() {
    let fixture = billed_fixture(
        "pool_billing_unknown",
        ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_secs(3))
            .set_body_json(json!({"usage": {"total_tokens": 1}})),
    )
    .await;
    fixture
        .proxy
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.proxy.pool_id },
            doc! { "$set": {
                "failover.retry_on": ["timeout"], "failover.per_attempt_timeout_ms": 1_000,
                "failover.overall_deadline_ms": 2_000,
            } },
        )
        .await
        .unwrap();
    let response = match try_call(&fixture.proxy, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert!(response.status().is_server_error());
    assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
    assert!(fixture.second.received_requests().await.unwrap().is_empty());
    let rows = settled_rows(&fixture, 1).await;
    assert!(
        rows[0].quantity.is_none(),
        "unknown dispatched usage must remain unknown"
    );
    assert_ne!(rows[0].status, UsageStatus::Finalized);
    assert!(rows[0].forwarded);
    assert_wallet(&fixture, 0).await;
    assert_eq!(
        fixture
            .proxy
            .state
            .db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    fixture.proxy.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_paused_consumer_renews_lease_and_lease_loss_closes_provider() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut fixture = billed_fixture(
        "pool_paused_lease",
        ResponseTemplate::new(200).set_body_string("unused"),
    )
    .await;
    fixture.proxy.state.config.proxy_stream_idle_timeout_secs = 60;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (filled_tx, filled_rx) = tokio::sync::oneshot::channel();
    let provider = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        let mut byte = [0; 1];
        while !headers.ends_with(b"\r\n\r\n") {
            assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
            headers.push(byte[0]);
        }
        let headers = String::from_utf8(headers).unwrap();
        let body_len = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
            .expect("buffered request has a known content length");
        socket.read_exact(&mut vec![0; body_len]).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let data = b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        let chunk = format!(
            "{:x}\r\n{}\r\n",
            data.len(),
            std::str::from_utf8(data).unwrap()
        );
        // Separate writes fill the bounded forwarding queue while the client
        // retains its response without polling any body frames.
        for _ in 0..32 {
            socket.write_all(chunk.as_bytes()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        filled_tx.send(()).unwrap();
        socket.read(&mut byte).await
    });
    let db = &fixture.proxy.state.db;
    db.collection::<Document>("downstream_services")
        .update_one(
            doc! { "slug": "pool-review-api" },
            doc! { "$set": { "base_url": &url } },
        )
        .await
        .unwrap();
    let response = Box::pin(try_call(&fixture.proxy, "{}"))
        .await
        .expect("stream headers and first frame");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    tokio::time::timeout(std::time::Duration::from_secs(5), filled_rx)
        .await
        .unwrap()
        .unwrap();
    let rows = db.collection::<UsageMeterRow>("usage_meter");
    let initial = rows.find_one(doc! {}).await.unwrap().unwrap();
    assert!(!initial.released);
    let initial_lease = initial.pool_attempt.as_ref().unwrap().lease_until;
    tokio::time::timeout(std::time::Duration::from_secs(14), async {
        loop {
            let current = rows
                .find_one(doc! { "_id": &initial.id })
                .await
                .unwrap()
                .unwrap();
            assert!(!current.released);
            if current.pool_attempt.as_ref().unwrap().lease_until > initial_lease {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("lease renewal must continue while the response body is unpolled");

    rows.update_one(
        doc! { "_id": &initial.id },
        doc! { "$set": { "pool_attempt.lease_until": mongodb::bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::seconds(1)) } },
    )
    .await
    .unwrap();
    assert_eq!(
        crate::services::billing::pool_attempt::recover_expired(db, chrono::Utc::now())
            .await
            .unwrap(),
        1
    );
    let closed = tokio::time::timeout(std::time::Duration::from_secs(14), provider)
        .await
        .expect("lost billing lease must cancel the provider while the consumer is paused")
        .unwrap();
    assert!(
        matches!(closed, Ok(0)) || closed.is_err(),
        "provider connection must terminate"
    );
    assert!(fixture.second.received_requests().await.unwrap().is_empty());
    assert!(fixture.first.received_requests().await.unwrap().is_empty());
    let released = rows
        .find_one(doc! { "_id": &initial.id })
        .await
        .unwrap()
        .unwrap();
    assert!(released.released && released.forwarded);
    assert!(
        released.quantity.is_none(),
        "partial text is not reported token usage"
    );
    assert_wallet(&fixture, 0).await;
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let row = rows
                .find_one(doc! {"_id":&initial.id})
                .await
                .unwrap()
                .unwrap();
            if row.pool_attempt.as_ref().unwrap().completion_cause
                == Some(crate::models::usage_meter::PoolCompletionCause::LeaseLost)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("lease loss is persisted independently of accounting outcome");
    assert_eq!(
        db.collection::<Document>("service_pool_member_health")
            .count_documents(doc! {"cooldown_until":{"$gt":mongodb::bson::DateTime::now()}})
            .await
            .unwrap(),
        0
    );
    drop(response);
    db.drop().await.unwrap();
}
