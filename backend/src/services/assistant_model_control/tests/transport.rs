//! Real local proxy/provider/MCP paths. Authored source; runtime validation pending.
use super::*;
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoTaxConfig, LagoUsage,
    OwnerProvisionInput,
};
use crate::{
    models::{
        billing_wallet::BillingWallet,
        credits::Credits,
        usage_meter::{CredentialClass, UsageMeterRow, UsageStatus},
    },
    services::billing::{BillingIngress, route_inventory::BillingRoutePolicy},
};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Extension, Path, State},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use futures::TryStreamExt;
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

mod compact;
mod initial_compact;

struct Provider {
    calls: AtomicUsize,
    headers: tokio::sync::Mutex<Vec<HeaderMap>>,
    terminal: tokio::sync::Notify,
    eof: tokio::sync::Semaphore,
    mode: &'static str,
    call_override: tokio::sync::Mutex<Option<Value>>,
    repeat_call: tokio::sync::Mutex<Option<Value>>,
    tool_calls: AtomicUsize,
    bodies: tokio::sync::Mutex<Vec<(String, Value)>>,
}
fn completion(call: usize) -> Value {
    json!({"id":format!("resp-{call}"),"object":"response","status":"completed","error":null,"incomplete_details":null,
        "output":if call==0 {json!([{"id":"fc-1","type":"function_call","status":"completed","call_id":"call-1",
            "name":"nyx__search_tools","arguments":"{\"query\":\"missing-test-tool\"}"}])}else {json!([{"id":"msg-2","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"done"}]}])}})
}
async fn provider(
    State(peer): State<Arc<Provider>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    let call = peer.calls.fetch_add(1, Ordering::SeqCst);
    peer.headers.lock().await.push(headers);
    peer.bodies.lock().await.push(("responses".into(), body));
    if peer.mode == "redirect" || (peer.mode == "redirect_back" && call == 0) {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [("location", "/second-send")],
        )
            .into_response();
    }
    let mut completed = completion(call);
    if call == 0
        && let Some(calls) = peer.call_override.lock().await.as_ref()
    {
        completed["output"] = calls.clone();
    }
    if call == 1
        && let Some(calls) = peer.repeat_call.lock().await.as_ref()
    {
        completed["output"] = calls.clone();
    }
    if peer.mode == "malformed_id" {
        completed["output"][0]["id"] = json!({"not":"a native item id"});
    } else if peer.mode == "reasoning_status" {
        completed["output"].as_array_mut().unwrap().push(json!({"type":"reasoning","id":"rs-invalid-status","status":"in_progress","summary":[],"encrypted_content":null}));
    }
    let frame = format!(
        "data: {}\n\n",
        json!({"type":"response.completed","response":completed})
    );
    let stream = async_stream::stream! {
        let middle=frame.len()/2;
        yield Ok::<Bytes,std::io::Error>(Bytes::copy_from_slice(&frame.as_bytes()[..middle]));
        if peer.mode=="truncated" {return;}
        yield Ok(Bytes::copy_from_slice(&frame.as_bytes()[middle..]));
        peer.terminal.notify_one();
        if call==0 || peer.mode=="passive_pending" {peer.eof.acquire().await.unwrap().forget();}
    };
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}
async fn fixture(mode: &'static str) -> (Controlled, Arc<Provider>, tokio::task::JoinHandle<()>) {
    fixture_with_issuance(mode, true).await
}
async fn fixture_with_issuance(
    mode: &'static str,
    issuance: bool,
) -> (Controlled, Arc<Provider>, tokio::task::JoinHandle<()>) {
    let mut c = Controlled::configured("p2_real_transport", issuance).await;
    let peer = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        headers: tokio::sync::Mutex::new(vec![]),
        terminal: tokio::sync::Notify::new(),
        eof: tokio::sync::Semaphore::new(0),
        mode,
        call_override: tokio::sync::Mutex::new(None),
        repeat_call: tokio::sync::Mutex::new(None),
        tool_calls: AtomicUsize::new(0),
        bodies: tokio::sync::Mutex::new(vec![]),
    });
    let second = peer.clone();
    let app = Router::new()
        .route("/responses", post(provider))
        .route("/chat/completions", post(provider))
        .route("/messages", post(provider))
        .route("/embeddings", post(provider))
        .route("/responses/compact", post(compact::provider))
        .route(
            "/tool-output",
            get(|State(peer): State<Arc<Provider>>| async move {
                peer.tool_calls.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"blob":"界🙂".repeat(60_000)}))
            }),
        )
        .route(
            "/second-send",
            post(move || {
                second.calls.fetch_add(1, Ordering::SeqCst);
                let back = second.mode == "redirect_back";
                async move {
                    if back {
                        (StatusCode::TEMPORARY_REDIRECT, [("location", "/responses")])
                            .into_response()
                    } else {
                        "unexpected redirect".into_response()
                    }
                }
            }),
        )
        .with_state(peer.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let service = crate::services::user_service_service::find_user_service_by_id(
        &c.f.state.db,
        &c.connection,
    )
    .await
    .unwrap()
    .unwrap();
    c.f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(doc! {"_id":service.endpoint_id}, doc! {"$set":{"url":url}})
        .await
        .unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let encrypted =
        c.f.state
            .encryption_keys
            .encrypt(b"provider-fixture-key")
            .await
            .unwrap();
    c.f.state.db.collection::<Document>("user_api_keys").insert_one(doc!{
        "_id":&key,"user_id":&c.f.owner,"label":"test provider","credential_type":"api_key","status":"active",
        "credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    c.f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&c.connection},
            doc! {"$set":{"api_key_id":key,"auth_method":"bearer"}},
        )
        .await
        .unwrap();
    c.f.auth.allowed_service_ids.push(c.connection.clone());
    c.f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&c.f.chat.api_key_id},
            doc! {"$addToSet":{"allowed_service_ids":&c.connection}},
        )
        .await
        .unwrap();
    let billing = crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: false,
        platform_metric: Some(crate::models::service_billing::BillingMetric::Requests),
        ..Default::default()
    };
    c.f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&c.target.service.id},
            doc! {"$set":{"billing":bson::to_bson(&billing).unwrap()}},
        )
        .await
        .unwrap();
    crate::billing_integration_tests::create_usage_index(&c.f.state.db).await;
    // Match the production wallet uniqueness fences before the callbacks race
    // to provision their first wallet; usage transaction uniqueness is separate.
    c.f.state
        .db
        .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .create_indexes([
            mongodb::IndexModel::builder()
                .keys(doc! { "owner_id": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
            mongodb::IndexModel::builder()
                .keys(doc! { "lago_customer_id": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        ])
        .await
        .unwrap();
    c.f.state
        .db
        .collection("billing_rate_cache")
        .insert_one(crate::billing_integration_tests::rate(
            "platform_requests",
            Utc::now(),
        ))
        .await
        .unwrap();
    c.f.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &c.f.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        10,
    ));
    c.target = crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
        &c.f.state.db,
        &c.f.state.encryption_keys,
        &c.f.owner,
        &c.connection,
        None,
        None,
        crate::services::proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .target;
    (c, peer, server)
}

/// Acknowledges only the actual one-credit winning transactions. Its accrued
/// usage grows from those ACKs, so the real reconciler refreshes the same wallet
/// from settled provider balance minus precisely the emitted usage.
struct CountedLago {
    customer: String,
    subscription: String,
    expected: BTreeSet<String>,
    acknowledged: tokio::sync::Mutex<BTreeSet<String>>,
    reads: tokio::sync::Mutex<Vec<LagoRead>>,
}

#[derive(Debug, PartialEq, Eq)]
enum LagoRead {
    Usage(String, String),
    Balance(String),
}

#[async_trait::async_trait]
impl LagoApi for CountedLago {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        assert_eq!(owner.external_customer_id, self.customer);
        Ok(self.customer.clone())
    }
    async fn ensure_subscription(&self, customer: &str, _: &str) -> AppResult<String> {
        assert_eq!(customer, self.customer);
        Ok(self.subscription.clone())
    }
    async fn tax_config(&self, customer: &str, subscription: &str) -> AppResult<LagoTaxConfig> {
        assert_eq!(customer, self.customer);
        assert_eq!(subscription, self.subscription);
        Ok(crate::services::billing::taxes::zero_config())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        assert!(self.expected.contains(&event.transaction_id));
        assert_eq!(
            event.external_subscription_id.as_deref(),
            Some(self.subscription.as_str())
        );
        assert!(event.external_customer_id.is_none());
        assert_eq!(event.code, "platform_requests");
        assert_eq!(event.properties.quantity, json!(1));
        assert_eq!(event.properties.service_code.as_deref(), Some("p2-model"));
        assert_eq!(event.properties.layer.as_deref(), Some("platform"));
        assert!(
            self.acknowledged
                .lock()
                .await
                .insert(event.transaction_id.clone())
        );
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        let mut replies = Vec::new();
        for event in events {
            replies.push(self.record_event(event).await?);
        }
        Ok(replies)
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        assert_eq!(customer, self.customer);
        assert!(subscription.is_empty() || subscription == self.subscription);
        let count = {
            let acknowledged = self.acknowledged.lock().await;
            assert_eq!(*acknowledged, self.expected);
            acknowledged.len()
        };
        self.reads
            .lock()
            .await
            .push(LagoRead::Usage(customer.into(), subscription.into()));
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: json!({"customer_usage":{"total_amount_cents":(count * 100).to_string(),
                "charges_usage":[{"billable_metric_code":"platform_requests","units":count.to_string()}]}}),
        })
    }
    async fn wallet_balance(&self, customer: &str) -> AppResult<i64> {
        assert_eq!(customer, self.customer);
        assert_eq!(*self.acknowledged.lock().await, self.expected);
        self.reads
            .lock()
            .await
            .push(LagoRead::Balance(customer.into()));
        Ok(10_000)
    }
    async fn entitlements(&self, subscription: &str) -> AppResult<Vec<Entitlement>> {
        assert_eq!(subscription, self.subscription);
        Ok(Vec::new())
    }
}

async fn assert_only_wallet(c: &Controlled, wallet: &BillingWallet) {
    let wallets: Vec<BillingWallet> =
        c.f.state
            .db
            .collection(crate::models::billing_wallet::COLLECTION_NAME)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(wallets.as_slice(), std::slice::from_ref(wallet));
}

async fn reconcile_charged_wallet(
    c: &Controlled,
    charged: &[UsageMeterRow],
    wallet: BillingWallet,
) -> BillingWallet {
    assert!(!charged.is_empty());
    let charge = Credits::from_whole(i64::try_from(charged.len()).unwrap());
    // Released+ledgered rows precede the final settlement-lock cleanup. Wait
    // for that real writer, then inspect a fresh snapshot of this same wallet.
    let wallet = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let Some(ready) =
                c.f.state
                    .db
                    .collection::<BillingWallet>("billing_wallet")
                    .find_one(doc! {"_id":&wallet.id,"active_settlement":bson::Bson::Null})
                    .await
                    .unwrap()
            {
                break ready;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_only_wallet(c, &wallet).await;
    assert_eq!(wallet.owner_id, c.f.owner);
    assert_eq!(wallet.lago_customer_id, c.f.owner);
    assert_eq!(wallet.lago_wallet_id, Some(format!("{}:wallet", c.f.owner)));
    assert_eq!(
        wallet.lago_subscription_id,
        Some(format!("{}:{}", c.f.owner, c.f.state.config.lago_plan_code))
    );
    assert_eq!(wallet.balance_credits, Credits::from_whole(10_000));
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(
        wallet.pending_lago_debits, charge,
        "only completed sends accrue debits"
    );
    let mut ids = Vec::new();
    let mut transactions = BTreeSet::new();
    for row in charged {
        assert_eq!(row.wallet_id.as_deref(), Some(wallet.id.as_str()));
        assert_eq!(row.billing_owner_id, wallet.owner_id);
        assert_eq!(row.status, UsageStatus::Finalized);
        assert!(row.forwarded && row.released && !row.lago_acked);
        assert_eq!(row.quantity, Some(1));
        assert_eq!(
            row.funding.as_ref().unwrap().wallet_charge_credits,
            Some(Credits::from_whole(1))
        );
        ids.push(row.id.clone());
        assert!(transactions.insert(row.transaction_id.clone()));
    }
    // Advance only these already-settled rows past the reconciler's 30-second
    // push grace. ACK/funding/financial fields are written by production paths.
    let due = c.f.state.db.collection::<UsageMeterRow>("usage_meter").update_many(
        doc! {"_id":{"$in":&ids},"wallet_id":&wallet.id,"status":"finalized","released":true,"funding.settled":true,"lago_acked":false},
        doc! {"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(31))}},
    ).await.unwrap();
    assert_eq!(due.matched_count, u64::try_from(charged.len()).unwrap());
    let lago = Arc::new(CountedLago {
        customer: wallet.lago_customer_id.clone(),
        subscription: wallet.lago_subscription_id.clone().unwrap(),
        expected: transactions.clone(),
        acknowledged: tokio::sync::Mutex::new(BTreeSet::new()),
        reads: tokio::sync::Mutex::new(Vec::new()),
    });
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = true;
    let stats = crate::services::billing::reconcile::BillingReconciler::new(
        c.f.state.db.clone(),
        Some(lago.clone()),
        Arc::new(config),
    )
    .run_once()
    .await
    .unwrap();
    assert_eq!(stats.pushed, u64::try_from(charged.len()).unwrap());
    assert_eq!(stats.retried, 0);
    assert_eq!(stats.dead_lettered, 0);
    assert_eq!(stats.wallet_balance_refreshes, 1);
    assert_eq!(
        *lago.reads.lock().await,
        vec![
            LagoRead::Usage(wallet.lago_customer_id.clone(), String::new()),
            LagoRead::Balance(wallet.lago_customer_id.clone()),
            LagoRead::Usage(
                wallet.lago_customer_id.clone(),
                wallet.lago_subscription_id.clone().unwrap(),
            ),
        ]
    );
    assert_eq!(*lago.acknowledged.lock().await, transactions);
    assert_eq!(
        c.f.state
            .db
            .collection::<UsageMeterRow>("usage_meter")
            .count_documents(
                doc! {"_id":{"$in":ids},"lago_acked":true,"status":"finalized","released":true},
            )
            .await
            .unwrap(),
        u64::try_from(charged.len()).unwrap()
    );
    let refreshed =
        c.f.state
            .db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {"_id":&wallet.id})
            .await
            .unwrap()
            .unwrap();
    assert_only_wallet(c, &refreshed).await;
    assert_eq!(refreshed.id, wallet.id);
    assert_eq!(
        refreshed.balance_credits,
        wallet.balance_credits.checked_sub(charge).unwrap()
    );
    assert_eq!(refreshed.reserved_credits, Credits::ZERO);
    assert_eq!(refreshed.pending_lago_debits, Credits::ZERO);
    refreshed
}

async fn callback(
    c: &Controlled,
    body: &Value,
    step: Option<&wire::PreparedStep>,
) -> AppResult<axum::response::Response> {
    callback_control(c, body, step, true).await
}
async fn callback_control(
    c: &Controlled,
    body: &Value,
    step: Option<&wire::PreparedStep>,
    include_control: bool,
) -> AppResult<axum::response::Response> {
    callback_path(c, body, step, include_control, "responses").await
}
async fn callback_path(
    c: &Controlled,
    body: &Value,
    step: Option<&wire::PreparedStep>,
    include_control: bool,
    path: &str,
) -> AppResult<axum::response::Response> {
    callback_with_headers(c, body, step, include_control, path, c.headers()).await
}

async fn callback_with_headers(
    c: &Controlled,
    body: &Value,
    step: Option<&wire::PreparedStep>,
    include_control: bool,
    path: &str,
    headers: HeaderMap,
) -> AppResult<axum::response::Response> {
    // Use production key authority construction, including its live turn
    // fence. A synthetic chat alone skips ModelRequest capture and therefore
    // cannot prove ordinary request-priced model usage at response EOF.
    let key =
        c.f.state
            .db
            .collection::<crate::models::api_key::ApiKey>("api_keys")
            .find_one(doc! {"_id":&c.f.chat.api_key_id})
            .await?
            .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&c.f.state.db, &key, None, None, None).await?;
    assert!(auth.assistant_turn_fence.is_some());
    assert!(auth.assistant_chat.is_some());
    let mut req = Request::builder().method("POST").uri(format!(
        "/api/v1/proxy/s/p2-model/{path}?_nyxid_via={}",
        c.connection
    ));
    *req.headers_mut().unwrap() = headers;
    if !include_control {
        req.headers_mut().unwrap().remove(wire::CONTROL_HEADER);
    }
    if let Some(step) = step {
        req = req.header(wire::STEP_HEADER, step.step_bearer.expose());
    }
    let mut req = req
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    req.extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    Box::pin(crate::handlers::proxy::proxy_request_by_slug(
        State(c.f.state.clone()),
        auth,
        Default::default(),
        Path(("p2-model".into(), path.into())),
        req,
    ))
    .await
}
#[tokio::test]
async fn real_generated_correlation_is_forwarded_without_rebinding_but_caller_id_stays_exact() {
    for caller_supplied in [false, true] {
        let (mut c, peer, server) = fixture("completed").await;
        let mut headers = c.headers();
        if caller_supplied {
            headers.insert("x-request-id", "caller-fixed".parse().unwrap());
        }
        let initial = request(vec![user("initial")]);
        let first =
            callback_with_headers(&c, &initial.body, None, true, "responses", headers.clone())
                .await
                .unwrap();
        assert_eq!(first.status(), StatusCode::CONFLICT);
        let first_id = first.headers()["x-request-id"].clone();
        let refusal: Value =
            serde_json::from_slice(&to_bytes(first.into_body(), 16384).await.unwrap()).unwrap();
        let repeated =
            callback_with_headers(&c, &initial.body, None, true, "responses", headers.clone())
                .await
                .unwrap();
        if caller_supplied {
            assert_eq!(first_id, "caller-fixed");
            assert_eq!(repeated.headers()["x-request-id"], first_id);
        } else {
            assert_ne!(repeated.headers()["x-request-id"], first_id);
        }
        let same: Value =
            serde_json::from_slice(&to_bytes(repeated.into_body(), 16384).await.unwrap()).unwrap();
        assert_eq!(same, refusal);
        assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
        c.resume_card(refusal["acknowledgement_id"].as_str().unwrap().into())
            .await;
        // Refresh the control on this genuine Allow continuation, preserving
        // the exact original caller header when one was actually supplied.
        headers.insert(wire::CONTROL_HEADER, c.token.expose().parse().unwrap());
        let mut candidate = request(vec![user("Saved Allow narration")]);
        if caller_supplied {
            candidate
                .semantic_headers
                .push(("x-request-id".into(), "caller-fixed".into()));
        }
        let body = candidate.body.clone();
        let wire::PrepareReply::Prepared { step } =
            prepare(&c.f.state, &c.f.chat, &c.token, candidate)
                .await
                .unwrap()
        else {
            panic!("exact caller headers must prepare");
        };
        let mut changed = headers.clone();
        changed.insert("x-request-id", "different-caller-id".parse().unwrap());
        assert!(
            callback_with_headers(&c, &body, Some(&step), true, "responses", changed)
                .await
                .is_err()
        );
        assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
        peer.eof.add_permits(1);
        let response = callback_with_headers(&c, &body, Some(&step), true, "responses", headers)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let sent_id = response.headers()["x-request-id"].clone();
        to_bytes(response.into_body(), wire::MAX_STREAM_BYTES)
            .await
            .unwrap();
        assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(peer.headers.lock().await[0]["x-request-id"], sent_id);
        if caller_supplied {
            assert_eq!(sent_id, "caller-fixed");
        } else {
            assert_ne!(sent_id, first_id);
        }
        assert!(matches!(
            current(&c, &step).await,
            wire::PrepareReply::Completed { .. }
        ));
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

async fn admitted(c: &mut Controlled, peer: &Provider) -> (String, wire::PreparedStep) {
    admitted_request(
        c,
        peer,
        request(vec![user("initial")]),
        request(vec![user("Saved Allow narration")]),
    )
    .await
}
async fn admitted_request(
    c: &mut Controlled,
    peer: &Provider,
    initial: wire::PrepareRequest,
    resumed: wire::PrepareRequest,
) -> (String, wire::PreparedStep) {
    let response = callback(c, &initial.body, None).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let first_correlation = response.headers()["x-request-id"].clone();
    let refusal: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    let card = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    let saved =
        c.f.state
            .db
            .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                "assistant_acknowledgements",
            )
            .find_one(doc! {"_id":&card})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.service_id.as_deref(), Some(c.connection.as_str()));
    assert_eq!(refusal["summary"], saved.summary);
    assert_eq!(refusal["service_slug"], "p2-model");
    for expected in [
        c.connection.as_str(),
        "p2-model",
        "model-v1",
        "fixed Responses configuration",
        "saved Allow narration",
    ] {
        assert!(
            saved.summary.contains(expected),
            "missing bounded consent disclosure: {expected}"
        );
    }
    assert!(
        !saved
            .summary
            .contains("Approval covers this exact request only")
    );
    assert!(!saved.summary.contains("thread_id"));
    let repeated = callback(c, &initial.body, None).await.unwrap();
    assert_ne!(repeated.headers()["x-request-id"], first_correlation);
    let repeated: Value =
        serde_json::from_slice(&to_bytes(repeated.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(
        refusal, repeated,
        "the real saved card and immediate refusal must agree on reuse"
    );
    let diagnosis =
        c.f.state
            .db
            .collection::<Document>("assistant_service_execution")
            .find_one(doc! {"kind":"model_fallback_diagnostic","conversation_id":&c.f.row.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        diagnosis.get_str("scope").unwrap(),
        "validated_model_continuation_v1"
    );
    let cause = crate::services::assistant_service_execution::inference_fallback_reason(
        "responses",
        &initial.body,
    )
    .unwrap();
    assert_eq!(diagnosis.get_str("reason").unwrap(), cause.code());
    assert!(saved.summary.contains(cause.reusable_message()));
    assert!(
        !diagnosis
            .get_str("explanation")
            .unwrap()
            .contains("exact request only")
    );
    assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
    c.resume_card(card.clone()).await;
    let allowed =
        c.f.state
            .db
            .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                "assistant_acknowledgements",
            )
            .find_one(doc! {"_id":&card})
            .await
            .unwrap()
            .unwrap();
    let root =
        c.f.state
            .db
            .collection::<ModelControlRoot>(COLLECTION_NAME)
            .find_one(doc! {"acknowledgement_id":&card})
            .await
            .unwrap()
            .unwrap();
    assert!(
        root.expires_at <= allowed.expires_at,
        "reusable use never extends the actual saved Allow expiry"
    );
    let wire::PrepareReply::Prepared { step } = prepare(&c.f.state, &c.f.chat, &c.token, resumed)
        .await
        .unwrap()
    else {
        panic!("saved Allow must prepare");
    };
    (card, step)
}
async fn current(c: &Controlled, step: &wire::PreparedStep) -> wire::PrepareReply {
    status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::StatusRequest {
            protocol: wire::PROTOCOL.into(),
            root_id: step.root_id.clone(),
            step_id: Some(step.step_id.clone()),
            request_binding: Some(step.request_binding.clone()),
        },
    )
    .await
    .unwrap()
}
async fn actual_tool(c: &Controlled, tool: &wire::PreparedTool) -> Value {
    actual_named_tool(
        c,
        tool,
        "nyx__search_tools",
        json!({"query":"missing-test-tool"}),
    )
    .await
}
async fn actual_named_tool(
    c: &Controlled,
    tool: &wire::PreparedTool,
    name: &str,
    arguments: Value,
) -> Value {
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &c.f.state.db,
        &c.f.state.encryption_keys,
        &c.f.owner,
        &c.f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let req=Request::builder().method("POST").uri("/mcp").header("x-api-key",credential.raw_key.as_str())
        .header("content-type","application/json").header("accept","application/json")
        .header(wire::CONTROL_HEADER,c.token.expose()).header(wire::TOOL_HEADER,tool.tool_bearer.expose())
        .body(Body::from(json!({"jsonrpc":"2.0","id":"rpc-host-call","method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string())).unwrap();
    let response = Box::pin(crate::handlers::mcp_transport::mcp_post(
        State(c.f.state.clone()),
        Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
        req,
    ))
    .await;
    let value: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    value
}

#[tokio::test]
async fn real_billed_duplicate_callbacks_claim_once_then_observed_tool_completion_allows_one_followup()
 {
    counted_followup(None).await;
}

#[tokio::test]
async fn real_unknown_and_oversized_metadata_one_allow_followup_keeps_opaque_options_and_claims_once()
 {
    for metadata in [
        json!({"unknown-provider-option":"opaque-value"}),
        json!({"x-codex-turn-metadata":"x".repeat(8193)}),
    ] {
        counted_followup(Some(metadata)).await;
    }
}

async fn counted_followup(metadata: Option<Value>) {
    let make = |input| {
        let mut candidate = request(input);
        if let Some(metadata) = &metadata {
            candidate.body["client_metadata"] = metadata.clone();
        }
        candidate
    };
    let (mut c, peer, server) = fixture("completed").await;
    let (card, step) = admitted_request(
        &mut c,
        &peer,
        make(vec![user("initial")]),
        make(vec![user("Saved Allow narration")]),
    )
    .await;
    // The prepare response was lost: authenticated root status retrieves the
    // identical capability, expiry and logical step, then that recovered token
    // participates in the actual two-reservation/final-send race below.
    let wire::PrepareReply::Prepared { step: recovered } = status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::StatusRequest {
            protocol: wire::PROTOCOL.into(),
            root_id: step.root_id.clone(),
            step_id: None,
            request_binding: None,
        },
    )
    .await
    .unwrap() else {
        panic!("lost prepare reply must recover Prepared");
    };
    assert_eq!(
        serde_json::to_value(&step).unwrap(),
        serde_json::to_value(&recovered).unwrap()
    );
    let body = make(vec![user("Saved Allow narration")]).body;
    if metadata.is_some() {
        for changed in [
            json!({"changed-key":"opaque-value"}),
            json!({"unknown-provider-option":"changed-value"}),
            json!({"x-codex-turn-metadata":"x".repeat(8194)}),
        ] {
            let mut tampered = body.clone();
            tampered["client_metadata"] = changed;
            assert!(callback(&c, &tampered, Some(&step)).await.is_err());
        }
        let mut tampered = body.clone();
        tampered["model"] = json!("different-model");
        assert!(callback(&c, &tampered, Some(&step)).await.is_err());
        let mut tampered = body.clone();
        tampered["tools"][0]["parameters"] =
            json!({"type":"object","properties":{"new-authority":{"type":"string"}}});
        assert!(callback(&c, &tampered, Some(&step)).await.is_err());
        let selected = c.connection.clone();
        c.connection = connected(
            &c.f.state.db,
            &c.f.owner,
            "p2-model",
            "https://example.invalid",
        )
        .await;
        assert!(callback(&c, &body, Some(&step)).await.is_err());
        c.connection = selected;
        assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
    }
    let (entered, mut arrivals) = tokio::sync::mpsc::channel(2);
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let first = BEFORE_FINAL_CLAIM.scope(
        (entered.clone(), release.clone()),
        callback(&c, &body, Some(&step)),
    );
    let second = BEFORE_FINAL_CLAIM.scope(
        (entered, release.clone()),
        callback(&c, &body, Some(&recovered)),
    );
    let both = async { tokio::join!(first, second) };
    tokio::pin!(both);
    for _ in 0..2 {
        tokio::select! {result=&mut both=>panic!("callbacks ended before claim barrier: {}",usize::from(result.0.is_ok())+usize::from(result.1.is_ok())),
        value=arrivals.recv()=>assert!(value.is_some()), _=tokio::time::sleep(std::time::Duration::from_secs(15))=>panic!("claim barrier timed out")}
    }
    let held: Vec<UsageMeterRow> =
        c.f.state
            .db
            .collection("usage_meter")
            .find(doc! {"user_service_id":&c.connection})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(held.len(), 2);
    assert!(held.iter().all(|r| r.reserved_credits > Credits::ZERO
        && !r.released
        && r.forwarded
        && r.status == UsageStatus::Forwarded));
    assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
    release.add_permits(2);
    let (a, b) = both.await;
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let winner = match (a, b) {
        (Ok(response), Err(_)) | (Err(_), Ok(response)) => response,
        _ => panic!("exactly one callback wins"),
    };
    assert_eq!(winner.status(), StatusCode::OK);
    let mut reading = Box::pin(to_bytes(winner.into_body(), wire::MAX_STREAM_BYTES));
    tokio::select! {result=&mut reading=>panic!("EOF must remain pending: {result:?}"),_=peer.terminal.notified()=>{}}
    assert!(matches!(
        current(&c, &step).await,
        wire::PrepareReply::InProgress { .. }
    ));
    assert!(
        tool_prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolPrepareRequest {
                protocol: wire::PROTOCOL.into(),
                name: "nyx__search_tools".into(),
                arguments: json!({"query":"missing-test-tool"}),
                call_id: Some("call-1".into()),
                result_char_limit: wire::default_tool_result_chars(),
                refusal_source: None,
            }
        )
        .await
        .is_err()
    );
    peer.eof.add_permits(1);
    reading.await.unwrap();
    let wire::PrepareReply::Completed { receipt } = current(&c, &step).await else {
        panic!("actual upstream EOF must durably complete");
    };
    let tool = tool_prepare(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::ToolPrepareRequest {
            protocol: wire::PROTOCOL.into(),
            name: "nyx__search_tools".into(),
            arguments: json!({"query":"missing-test-tool"}),
            call_id: Some("call-1".into()),
            result_char_limit: wire::default_tool_result_chars(),
            refusal_source: None,
        },
    )
    .await
    .unwrap();
    let rpc = actual_tool(&c, &tool).await;
    assert!(rpc["error"].is_null());
    let result = rpc["result"].clone();
    let proof = tool_status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::ToolStatusRequest {
            protocol: wire::PROTOCOL.into(),
            tool: tool.clone(),
        },
    )
    .await
    .unwrap();
    let live = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap();
    assert!(
        live.active_turn
            .as_ref()
            .unwrap()
            .activities
            .iter()
            .any(|a| a.id == proof.server_activity_id)
    );
    assert!(!actual_tool(&c, &tool).await["error"].is_null());
    // Duplicate actual RPC has no reusable receipt or second tool execution.
    let after = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap();
    assert_eq!(
        live.active_turn.as_ref().unwrap().activity_total,
        after.active_turn.as_ref().unwrap().activity_total
    );
    let mut next = make(vec![user("Saved Allow narration")]);
    next.body["input"]
        .as_array_mut()
        .unwrap()
        .extend(completion(0)["output"].as_array().unwrap().iter().cloned());
    if metadata.is_some() {
        let native_call = next.body["input"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap();
        native_call.as_object_mut().unwrap().remove("status");
        native_call["id"] = Value::Null;
    }
    next.body["input"].as_array_mut().unwrap().push(
        json!({"id":null,"type":"function_call_output","call_id":"call-1","output":result.to_string()}),
    );
    next.preceding_receipt = Some(receipt.receipt_bearer);
    next.tool_results = vec![proof];
    let next_body = next.body.clone();
    let wire::PrepareReply::Prepared { step: next_step } =
        prepare(&c.f.state, &c.f.chat, &c.token, next)
            .await
            .unwrap()
    else {
        panic!("real host receipt must admit next step");
    };
    assert_eq!(next_step.ordinal, 1);
    assert_eq!(next_step.root_id, step.root_id);
    let followup = callback(&c, &next_body, Some(&next_step)).await.unwrap();
    to_bytes(followup.into_body(), wire::MAX_STREAM_BYTES)
        .await
        .unwrap();
    assert!(callback(&c, &body, Some(&step)).await.is_err());
    assert!(callback(&c, &next_body, Some(&next_step)).await.is_err());
    assert_eq!(
        peer.calls.load(Ordering::SeqCst),
        2,
        "one send per genuine logical step"
    );
    // A retained completed root is a tombstone, not renewed consent. Advance
    // the persisted use deadline instead of waiting ten minutes in a test.
    c.f.state.db.collection::<Document>(COLLECTION_NAME).update_one(doc!{"_id":&step.root_id},
        doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(
        prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            make(vec![user("new after expiry")])
        )
        .await
        .is_err()
    );
    assert!(callback(&c, &next_body, Some(&next_step)).await.is_err());
    assert_eq!(peer.calls.load(Ordering::SeqCst), 2);
    for headers in peer.headers.lock().await.iter() {
        assert!(!headers.contains_key(wire::CONTROL_HEADER));
        assert!(!headers.contains_key(wire::STEP_HEADER));
        assert!(!headers.contains_key(wire::TOOL_HEADER));
        assert_eq!(headers["authorization"], "Bearer provider-fixture-key");
    }
    let charged = crate::billing_integration_tests::wait_for_charged_usage(
        &c.f.state.db,
        doc! {"user_service_id":&c.connection,"status":"finalized"},
        2,
    )
    .await;
    assert!(charged.iter().all(|r| r.billing_owner_id == c.f.owner
        && r.api_key_id.as_deref() == Some(c.f.chat.api_key_id.as_str())
        && r.credential_class == CredentialClass::UserOwned));
    let loser = c
        .f
        .state
        .db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"user_service_id":&c.connection,"last_error":"assistant_authority_changed"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loser.status, UsageStatus::Failed);
    assert!(loser.released);
    assert!(!loser.forwarded);
    let wallet =
        c.f.state
            .db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {"_id":loser.wallet_id.unwrap()})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    let wallet = reconcile_charged_wallet(&c, &charged, wallet).await;
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"_id":card,"status":"used"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    server.abort();
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn real_truncated_dropped_and_redirected_provider_steps_never_replay() {
    for mode in ["truncated", "dropped", "redirect"] {
        let (mut c, peer, server) = fixture(mode).await;
        let (_, step) = admitted(&mut c, &peer).await;
        let body = request(vec![user("Saved Allow narration")]).body;
        let response = callback(&c, &body, Some(&step)).await.unwrap();
        if mode == "dropped" {
            drop(response);
        } else {
            if mode == "redirect" {
                assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
            }
            let _ = to_bytes(response.into_body(), wire::MAX_STREAM_BYTES).await;
        }
        assert!(matches!(
            current(&c, &step).await,
            wire::PrepareReply::OutcomeUnknown | wire::PrepareReply::InProgress { .. }
        ));
        assert!(callback(&c, &body, Some(&step)).await.is_err());
        assert!(matches!(
            prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                request(vec![user("Saved Allow narration")])
            )
            .await
            .unwrap(),
            wire::PrepareReply::OutcomeUnknown | wire::PrepareReply::InProgress { .. }
        ));
        assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
        peer.eof.add_permits(1);
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn real_malformed_provider_item_identity_or_status_never_mints_completion_or_replays() {
    for mode in ["malformed_id", "reasoning_status"] {
        let (mut c, peer, server) = fixture(mode).await;
        let (_, step) = admitted(&mut c, &peer).await;
        let body = request(vec![user("Saved Allow narration")]).body;
        let response = callback(&c, &body, Some(&step)).await.unwrap();
        peer.eof.add_permits(1);
        to_bytes(response.into_body(), wire::MAX_STREAM_BYTES)
            .await
            .unwrap();
        assert!(matches!(
            current(&c, &step).await,
            wire::PrepareReply::OutcomeUnknown
        ));
        assert!(callback(&c, &body, Some(&step)).await.is_err());
        assert!(matches!(
            prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                request(vec![user("Saved Allow narration")])
            )
            .await
            .unwrap(),
            wire::PrepareReply::OutcomeUnknown
        ));
        assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

fn continuation(c: &Controlled, card: Option<&str>, retry: Option<&str>) -> engine::TurnRequest {
    engine::TurnRequest {
        acknowledgement_id: card.map(str::to_owned),
        retry_of: retry.map(str::to_owned),
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: Some(c.f.row.id.clone()),
        text: "Saved Allow narration".into(),
        model: None,
        access_mode: None,
    }
}
async fn refresh_control(c: &mut Controlled) {
    c.f.chat = acks::for_key(
        &c.f.state.db,
        &c.f.owner,
        Some(&c.f.row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    c.token = issue(
        &c.f.state,
        IssueRequest {
            chat: &c.f.chat,
            runtime_catalog_id: Some(&c.target.service.id),
            outer_body: &json!({"input":"Saved Allow narration","store":true}),
            negotiated_v1: true,
        },
    )
    .await
    .unwrap()
    .unwrap()
    .token;
}
async fn settle_failed(c: &Controlled, code: &str) {
    engine::finish_turn(
        &c.f.state.db,
        &c.f.row,
        &c.f.row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::internal(code, "stream")),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn real_allow_continuation_redelivery_and_failed_turn_retry_preserve_uncertain_step() {
    uncertain_redelivery(None).await;
}

#[tokio::test]
async fn real_unknown_and_oversized_metadata_uncertain_continuation_retry_never_resends() {
    for metadata in [
        json!({"unknown-provider-option":"opaque-value"}),
        json!({"x-codex-turn-metadata":"x".repeat(8193)}),
    ] {
        uncertain_redelivery(Some(metadata)).await;
    }
}

async fn uncertain_redelivery(metadata: Option<Value>) {
    let make = |input| {
        let mut candidate = super::request(input);
        if let Some(metadata) = &metadata {
            candidate.body["client_metadata"] = metadata.clone();
        }
        candidate
    };
    for delivery in ["continuation", "retry", "explicit_unknown_retry"] {
        let (mut c, peer, server) = fixture("truncated").await;
        let (card, step) = admitted_request(
            &mut c,
            &peer,
            make(vec![user("initial")]),
            make(vec![user("Saved Allow narration")]),
        )
        .await;
        let body = make(vec![user("Saved Allow narration")]).body;
        let response = callback(&c, &body, Some(&step)).await.unwrap();
        let _ = to_bytes(response.into_body(), wire::MAX_STREAM_BYTES).await;
        assert!(matches!(
            current(&c, &step).await,
            wire::PrepareReply::OutcomeUnknown
        ));
        assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
        let failed_turn = c.f.row.active_turn.as_ref().unwrap().turn_id.clone();
        settle_failed(
            &c,
            if delivery == "explicit_unknown_retry" {
                "outcome_unknown"
            } else {
                "invalid_stream"
            },
        )
        .await;
        let request = continuation(
            &c,
            (delivery == "continuation").then_some(card.as_str()),
            (delivery != "continuation").then_some(failed_turn.as_str()),
        );
        let (a, b) = tokio::join!(
            Box::pin(engine::begin_turn(
                &c.f.state.db,
                &c.f.owner,
                &request,
                &c.f.state.encryption_keys
            )),
            Box::pin(engine::begin_turn(
                &c.f.state.db,
                &c.f.owner,
                &request,
                &c.f.state.encryption_keys
            ))
        );
        if delivery == "explicit_unknown_retry" {
            assert!(
                a.is_err() && b.is_err(),
                "the existing Retry gate rejects an explicit unknown outcome"
            );
        } else {
            assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
            c.f.row = a.or(b).unwrap();
            assert_ne!(c.f.row.active_turn.as_ref().unwrap().turn_id, failed_turn);
            refresh_control(&mut c).await;
            assert!(matches!(
                current(&c, &step).await,
                wire::PrepareReply::OutcomeUnknown
            ));
            assert!(matches!(
                prepare(
                    &c.f.state,
                    &c.f.chat,
                    &c.token,
                    make(vec![user("Saved Allow narration")])
                )
                .await
                .unwrap(),
                wire::PrepareReply::OutcomeUnknown
            ));
            assert!(callback(&c, &body, Some(&step)).await.is_err());
            let saved_input = c
                .f
                .state
                .db
                .collection::<crate::models::assistant_message::AssistantMessage>(
                    "assistant_messages",
                )
                .find_one(
                    doc! {"conversation_id":&c.f.row.id,"turn_id":&c.f.chat.turn_id,"role":"user"},
                )
                .await
                .unwrap()
                .unwrap();
            if delivery == "retry" {
                assert_eq!(saved_input.retry_of.as_deref(), Some(failed_turn.as_str()));
            } else {
                assert_eq!(saved_input.continuation_of.as_deref(), Some(card.as_str()));
            }
        }
        assert_eq!(
            peer.calls.load(Ordering::SeqCst),
            1,
            "new native turn must not rearm the original dispatch"
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        peer.eof.add_permits(1);
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn real_saved_allow_retry_before_first_prepare_keeps_anchor_and_dispatches_once() {
    let (mut c, peer, server) = fixture("completed").await;
    let response = callback(&c, &request(vec![user("initial")]).body, None)
        .await
        .unwrap();
    let refusal: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    let card = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    c.resume_card(card.clone()).await;
    let failed = c.f.row.active_turn.as_ref().unwrap().turn_id.clone();
    settle_failed(&c, "invalid_stream").await;
    let retry = continuation(&c, None, Some(&failed));
    c.f.row = engine::begin_turn(
        &c.f.state.db,
        &c.f.owner,
        &retry,
        &c.f.state.encryption_keys,
    )
    .await
    .unwrap();
    refresh_control(&mut c).await;
    let candidate = request(vec![user("Saved Allow narration")]);
    let body = candidate.body.clone();
    let wire::PrepareReply::Prepared { step } = prepare(&c.f.state, &c.f.chat, &c.token, candidate)
        .await
        .unwrap()
    else {
        panic!("Retry carries the original saved Allow anchor before step zero");
    };
    assert_eq!(step.ordinal, 0);
    peer.eof.add_permits(1);
    let response = callback(&c, &body, Some(&step)).await.unwrap();
    to_bytes(response.into_body(), wire::MAX_STREAM_BYTES)
        .await
        .unwrap();
    assert!(matches!(
        current(&c, &step).await,
        wire::PrepareReply::Completed { .. }
    ));
    assert!(callback(&c, &body, Some(&step)).await.is_err());
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"_id":card,"status":"used"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    crate::billing_integration_tests::wait_for_charged_usage(
        &c.f.state.db,
        doc! {"user_service_id":&c.connection,"status":"finalized"},
        1,
    )
    .await;
    server.abort();
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn real_negotiated_initial_strict_read_stays_card_free_and_has_no_reusable_step() {
    let (c, peer, server) = fixture("completed").await;
    let mut candidate = request(vec![user("plain question")]);
    candidate
        .body
        .as_object_mut()
        .unwrap()
        .remove("client_metadata");
    assert!(matches!(
        prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::PrepareRequest {
                body: candidate.body.clone(),
                ..request(vec![])
            }
        )
        .await
        .unwrap(),
        wire::PrepareReply::AutomaticRead
    ));
    peer.eof.add_permits(1);
    let response = callback(&c, &candidate.body, None).await.unwrap();
    to_bytes(response.into_body(), wire::MAX_STREAM_BYTES)
        .await
        .unwrap();
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let root =
        c.f.state
            .db
            .collection::<ModelControlRoot>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(root.phase, "unactivated");
    assert!(
        persistence::open(&c.f.state, &root)
            .await
            .unwrap()
            .steps
            .is_empty()
    );
    server.abort();
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn real_final_claim_stop_key_revocation_or_catalog_drift_releases_billing_without_provider_send()
 {
    for fence in ["stop", "revoke", "catalog"] {
        let (mut c, peer, server) = fixture("completed").await;
        let (_, step) = admitted(&mut c, &peer).await;
        let body = request(vec![user("Saved Allow narration")]).body;
        let (entered, mut arrivals) = tokio::sync::mpsc::channel(1);
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let dispatch =
            BEFORE_FINAL_CLAIM.scope((entered, release.clone()), callback(&c, &body, Some(&step)));
        tokio::pin!(dispatch);
        tokio::select! {result=&mut dispatch=>panic!("callback ended before claim: {}",result.is_ok()),
        arrival=arrivals.recv()=>assert!(arrival.is_some()),
        _=tokio::time::sleep(std::time::Duration::from_secs(15))=>panic!("claim barrier timed out")}
        let held =
            c.f.state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {"user_service_id":&c.connection})
                .await
                .unwrap()
                .unwrap();
        // Forwarded is existing pre-send bookkeeping; the actual provider has
        // not been called at this final-authority barrier.
        assert!(held.reserved_credits > Credits::ZERO && !held.released && held.forwarded);
        assert_eq!(held.status, UsageStatus::Forwarded);
        assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
        if fence == "stop" {
            engine::request_stop(&c.f.state.db, &c.f.owner, &c.f.row.id)
                .await
                .unwrap();
        } else if fence == "revoke" {
            c.f.state
                .db
                .collection::<Document>("api_keys")
                .update_one(
                    doc! {"_id":&c.f.chat.api_key_id},
                    doc! {"$set":{"is_active":false}},
                )
                .await
                .unwrap();
        } else {
            c.f.state
                .db
                .collection::<Document>("downstream_services")
                .update_one(
                    doc! {"_id":&c.target.service.id},
                    doc! {"$unset":{"inference":""}},
                )
                .await
                .unwrap();
        }
        release.add_permits(1);
        assert!(dispatch.await.is_err());
        assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
        let loser =
            c.f.state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {"_id":held.id})
                .await
                .unwrap()
                .unwrap();
        assert!(loser.released && !loser.forwarded);
        assert_eq!(loser.status, UsageStatus::Failed);
        assert_eq!(
            loser.last_error.as_deref(),
            Some("assistant_authority_changed")
        );
        let wallet =
            c.f.state
                .db
                .collection::<BillingWallet>("billing_wallet")
                .find_one(doc! {"_id":loser.wallet_id.unwrap()})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
        assert_eq!(
            c.f.state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .count_documents(doc! {"user_service_id":&c.connection,"status":"finalized"})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {"status":"allowed"})
                .await
                .unwrap(),
            1
        );
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn real_catalog_inference_removal_preserves_unactivated_exact_only_but_never_downgrades_active_root()
 {
    for active in [false, true] {
        let (mut c, peer, server) = fixture("completed").await;
        let step = if active {
            Some(admitted(&mut c, &peer).await.1)
        } else {
            None
        };
        c.f.state
            .db
            .collection::<Document>("downstream_services")
            .update_one(
                doc! {"_id":&c.target.service.id},
                doc! {"$unset":{"inference":""}},
            )
            .await
            .unwrap();
        c.target.service.inference = None;
        let body = request(vec![user(if active {
            "Saved Allow narration"
        } else {
            "initial"
        })])
        .body;
        if active {
            assert!(
                callback_control(&c, &body, step.as_ref(), true)
                    .await
                    .is_err()
            );
            assert!(callback_control(&c, &body, None, false).await.is_err());
            let mut unrelated = c.headers();
            unrelated.remove(wire::CONTROL_HEADER);
            assert!(
                c.callback(
                    "unrelated-effect",
                    &json!({"effect":"requires its own consent"}),
                    &unrelated,
                    true
                )
                .await
                .unwrap()
                .is_none(),
                "model lineage must not replace an unrelated effect gate"
            );
            assert_eq!(
                c.f.state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1
            );
        } else {
            let plain = callback_control(&c, &body, None, false).await.unwrap();
            let negotiated = callback_control(&c, &body, None, true).await.unwrap();
            assert_eq!(plain.status(), negotiated.status());
            let plain: Value =
                serde_json::from_slice(&to_bytes(plain.into_body(), 16384).await.unwrap()).unwrap();
            let negotiated: Value =
                serde_json::from_slice(&to_bytes(negotiated.into_body(), 16384).await.unwrap())
                    .unwrap();
            assert_eq!(
                plain, negotiated,
                "negotiation must retain legacy exact card identity"
            );
            assert_eq!(
                c.f.state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .count_documents(doc! {"tool_name":"nyxid.model_control_v1"})
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("usage_meter")
                .count_documents(doc! {"user_service_id":&c.connection})
                .await
                .unwrap(),
            0,
            "catalog drift is rejected before billing admission"
        );
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

/// Real provider declarations and actual MCP dispatch, including duplicate
/// equal name/arguments selected by native callId. No forged tool receipt.
#[tokio::test]
async fn real_native_call_ids_select_equal_declarations_and_bind_first_projection_limit() {
    let (mut c, peer, server) = fixture("completed").await;
    let first = completion(0)["output"][0].clone();
    let mut second = first.clone();
    second["id"] = json!("fc-2");
    second["call_id"] = json!("call-2");
    let calls = json!([first, second]);
    *peer.call_override.lock().await = Some(calls.clone());
    c.allow_resume().await;
    let candidate = request(vec![user("Saved Allow narration")]);
    let body = candidate.body.clone();
    let wire::PrepareReply::Prepared { step } = prepare(&c.f.state, &c.f.chat, &c.token, candidate)
        .await
        .unwrap()
    else {
        panic!("prepared");
    };
    peer.eof.add_permits(1);
    to_bytes(
        callback(&c, &body, Some(&step)).await.unwrap().into_body(),
        wire::MAX_STREAM_BYTES,
    )
    .await
    .unwrap();
    let wire::PrepareReply::Completed { receipt } = current(&c, &step).await else {
        panic!("completed");
    };
    let prepare_call = |id: Option<&str>, limit| wire::ToolPrepareRequest {
        protocol: wire::PROTOCOL.into(),
        name: "nyx__search_tools".into(),
        arguments: json!({"query":"missing-test-tool"}),
        call_id: id.map(str::to_owned),
        result_char_limit: limit,
        refusal_source: None,
    };
    assert!(
        tool_prepare(&c.f.state, &c.f.chat, &c.token, prepare_call(None, 1024))
            .await
            .is_err()
    );
    for id in ["", "forged", "call-3"] {
        assert!(
            tool_prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                prepare_call(Some(id), 1024)
            )
            .await
            .is_err()
        );
    }
    let mut proofs = vec![];
    let mut outputs = vec![];
    for id in ["call-1", "call-2"] {
        let tool = tool_prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            prepare_call(Some(id), 1024),
        )
        .await
        .unwrap();
        let recovered = tool_prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            prepare_call(Some(id), 1024),
        )
        .await
        .unwrap();
        assert_eq!(tool.tool_bearer.expose(), recovered.tool_bearer.expose());
        assert!(
            tool_prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                prepare_call(Some(id), 2048)
            )
            .await
            .is_err()
        );
        let rpc = actual_tool(&c, &tool).await;
        assert!(rpc["error"].is_null());
        let projected = wire::tool_result_projection(&rpc["result"], 1024);
        let proof = tool_status(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolStatusRequest {
                protocol: wire::PROTOCOL.into(),
                tool: tool.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(proof.call_id, id);
        assert_eq!(proof.result_binding, digest(json!(projected.text)));
        outputs.push(json!({"type":"function_call_output","call_id":id,"output":projected.text}));
        proofs.push(proof);
        assert!(!actual_tool(&c, &tool).await["error"].is_null());
    }
    let mut next = request(vec![user("Saved Allow narration")]);
    next.body["input"]
        .as_array_mut()
        .unwrap()
        .extend(calls.as_array().unwrap().iter().cloned());
    next.body["input"].as_array_mut().unwrap().extend(outputs);
    next.preceding_receipt = Some(receipt.receipt_bearer);
    next.tool_results = proofs;
    assert!(matches!(
        prepare(&c.f.state, &c.f.chat, &c.token, next)
            .await
            .unwrap(),
        wire::PrepareReply::Prepared { .. }
    ));
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    c.f.state.db.drop().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn real_policy_deny_and_large_successful_tool_results_bind_exact_existing_projection() {
    for kind in ["denied", "truncated"] {
        let (mut c, peer, server) = fixture("completed").await;
        let inner = if kind == "denied" {
            json!({"service_id":c.connection,"enabled":false})
        } else {
            json!({})
        };
        let inner_name = if kind == "denied" {
            "nyxid__set_service_enabled"
        } else {
            "p2-model__read_blob"
        };
        let arguments = json!({"tool_name":inner_name,"arguments_json":inner.to_string()});
        let call = json!({"id":"fc-projection","type":"function_call","status":"completed","call_id":"call-projection","name":"nyx__call_tool","arguments":arguments.to_string()});
        *peer.call_override.lock().await = Some(json!([call.clone()]));
        if kind == "truncated" {
            let mut endpoint = crate::test_utils::test_service_endpoint(
                &c.target.service.id,
                "read_blob",
                "GET",
                "/tool-output",
            );
            endpoint.risk = Some(crate::models::service_endpoint::EndpointRisk::Read);
            c.f.state
                .db
                .collection("service_endpoints")
                .insert_one(endpoint)
                .await
                .unwrap();
        }
        let mut initial = request(vec![user("initial")]);
        initial.body["tools"][0]["name"] = json!("nyx__call_tool");
        let card = c.observe_request(initial).await;
        c.resume_card(card).await;
        let effect_card = if kind == "denied" {
            let facts = acks::ActionFacts::of(acks::classify_owner_action(inner_name, &inner));
            let need = acks::confirmation_needed(&c.f.state.db, &c.f.chat, facts, false)
                .await
                .unwrap()
                .ask()
                .unwrap();
            let refusal = acks::confirm_action(
                &c.f.state.db,
                &c.f.chat,
                inner_name,
                &inner,
                &need,
                "Disable this exact saved connection",
            )
            .await
            .unwrap()
            .unwrap();
            let id = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
            acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &id, false)
                .await
                .unwrap();
            Some(id)
        } else {
            None
        };
        let mut candidate = request(vec![user("Saved Allow narration")]);
        candidate.body["tools"][0]["name"] = json!("nyx__call_tool");
        let body = candidate.body.clone();
        let wire::PrepareReply::Prepared { step } =
            prepare(&c.f.state, &c.f.chat, &c.token, candidate)
                .await
                .unwrap()
        else {
            panic!("prepared");
        };
        peer.eof.add_permits(1);
        to_bytes(
            callback(&c, &body, Some(&step)).await.unwrap().into_body(),
            wire::MAX_STREAM_BYTES,
        )
        .await
        .unwrap();
        let wire::PrepareReply::Completed { receipt } = current(&c, &step).await else {
            panic!("completed");
        };
        let tool = tool_prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolPrepareRequest {
                protocol: wire::PROTOCOL.into(),
                name: "nyx__call_tool".into(),
                arguments: arguments.clone(),
                call_id: Some("call-projection".into()),
                result_char_limit: 1024,
                refusal_source: None,
            },
        )
        .await
        .unwrap();
        let rpc = actual_named_tool(&c, &tool, "nyx__call_tool", arguments.clone()).await;
        assert!(rpc["error"].is_null());
        let result = &rpc["result"];
        let projected = wire::tool_result_projection(result, 1024);
        assert!(projected.success);
        if let Some(effect_card) = effect_card {
            assert!(matches!(
                projected.refusal.as_ref().map(|r| r.kind),
                Some(wire::RefusalKind::Denied)
            ));
            assert!(
                projected
                    .text
                    .starts_with("Acknowledgement denied on the card. END the turn.")
            );
            let saved =
                c.f.state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .find_one(doc! {"_id":effect_card})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(saved.get_str("status").unwrap(), "denied");
            let connection = crate::services::user_service_service::find_user_service_by_id(
                &c.f.state.db,
                &c.connection,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(connection.is_active);
            assert_eq!(peer.tool_calls.load(Ordering::SeqCst), 0);
        } else {
            assert!(result.to_string().len() > 256 * 1024);
            assert!(result.to_string().len() < wire::MAX_TOOL_RESULT_BYTES);
            let small: Value = serde_json::from_str(&projected.text).unwrap();
            assert_eq!(small["truncated"], true);
            assert_eq!(peer.tool_calls.load(Ordering::SeqCst), 1);
        }
        let proof = tool_status(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolStatusRequest {
                protocol: wire::PROTOCOL.into(),
                tool: tool.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(proof.result_binding, digest(json!(projected.text)));
        let live = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
            .await
            .unwrap();
        assert!(
            live.active_turn
                .as_ref()
                .unwrap()
                .activities
                .iter()
                .any(|a| a.id == proof.server_activity_id)
        );
        assert!(
            !actual_named_tool(&c, &tool, "nyx__call_tool", arguments).await["error"].is_null()
        );
        let mut next = request(vec![user("Saved Allow narration")]);
        next.body["tools"][0]["name"] = json!("nyx__call_tool");
        next.body["input"].as_array_mut().unwrap().extend([call,json!({"type":"function_call_output","call_id":"call-projection","output":projected.text})]);
        next.preceding_receipt = Some(receipt.receipt_bearer);
        next.tool_results = vec![proof];
        let original_proof = next.tool_results[0].clone();
        let repeated = if kind == "denied" {
            let mut again = next.body["input"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["type"] == "function_call")
                .unwrap()
                .clone();
            again["id"] = json!("fc-repeated");
            again["call_id"] = json!("call-repeated");
            *peer.repeat_call.lock().await = Some(json!([again.clone()]));
            Some(again)
        } else {
            None
        };
        let next_body = next.body.clone();
        let wire::PrepareReply::Prepared { step: next_step } =
            prepare(&c.f.state, &c.f.chat, &c.token, next)
                .await
                .unwrap()
        else {
            panic!("projected receipt allows explanation");
        };
        to_bytes(
            callback(&c, &next_body, Some(&next_step))
                .await
                .unwrap()
                .into_body(),
            wire::MAX_STREAM_BYTES,
        )
        .await
        .unwrap();
        if let Some(repeated) = repeated {
            let wire::PrepareReply::Completed {
                receipt: second_receipt,
            } = current(&c, &next_step).await
            else {
                panic!("repeat declaration observed");
            };
            let args: Value =
                serde_json::from_str(repeated["arguments"].as_str().unwrap()).unwrap();
            let reuse = |source: wire::ToolResultReceipt, limit| wire::ToolPrepareRequest {
                protocol: wire::PROTOCOL.into(),
                name: "nyx__call_tool".into(),
                arguments: args.clone(),
                call_id: Some("call-repeated".into()),
                result_char_limit: limit,
                refusal_source: Some(source),
            };
            let mut forged = original_proof.clone();
            forged.evidence = wire::ToolResultEvidence::ReusedRefusal {
                origin_step_id: "forged".into(),
                origin_call_id: "forged".into(),
            };
            assert!(
                tool_prepare(&c.f.state, &c.f.chat, &c.token, reuse(forged, 1024))
                    .await
                    .is_err()
            );
            assert!(
                tool_prepare(
                    &c.f.state,
                    &c.f.chat,
                    &c.token,
                    reuse(original_proof.clone(), 2048)
                )
                .await
                .is_err()
            );
            let activity = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .activity_total;
            let prepared = tool_prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                reuse(original_proof.clone(), 1024),
            )
            .await
            .unwrap();
            let same = tool_prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                reuse(original_proof.clone(), 1024),
            )
            .await
            .unwrap();
            assert_eq!(prepared.tool_bearer.expose(), same.tool_bearer.expose());
            let proof = tool_status(
                &c.f.state,
                &c.f.chat,
                &c.token,
                wire::ToolStatusRequest {
                    protocol: wire::PROTOCOL.into(),
                    tool: prepared.clone(),
                },
            )
            .await
            .unwrap();
            assert!(
                matches!(&proof.evidence,wire::ToolResultEvidence::ReusedRefusal{origin_step_id,origin_call_id} if origin_step_id==&step.step_id && origin_call_id=="call-projection")
            );
            assert_eq!(proof.server_activity_id, original_proof.server_activity_id);
            assert_eq!(proof.result_binding, original_proof.result_binding);
            assert!(
                !actual_named_tool(&c, &prepared, "nyx__call_tool", args).await["error"].is_null()
            );
            assert_eq!(
                engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
                    .await
                    .unwrap()
                    .active_turn
                    .unwrap()
                    .activity_total,
                activity
            );
            let mut final_request = request_with_body(&next_body);
            final_request.body["input"].as_array_mut().unwrap().extend([repeated,json!({"type":"function_call_output","call_id":"call-repeated","output":projected.text})]);
            final_request.preceding_receipt = Some(second_receipt.receipt_bearer);
            final_request.tool_results = vec![proof.clone()];
            let mut dropped = request_with_body(&final_request.body);
            dropped.body["input"].as_array_mut().unwrap().pop();
            dropped.preceding_receipt = final_request.preceding_receipt.clone();
            dropped.tool_results = vec![proof.clone()];
            assert!(
                prepare(&c.f.state, &c.f.chat, &c.token, dropped)
                    .await
                    .is_err()
            );
            let mut twice = request_with_body(&final_request.body);
            let extra = twice.body["input"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .clone();
            twice.body["input"].as_array_mut().unwrap().push(extra);
            twice.preceding_receipt = final_request.preceding_receipt.clone();
            twice.tool_results = vec![proof.clone(), proof];
            assert!(
                prepare(&c.f.state, &c.f.chat, &c.token, twice)
                    .await
                    .is_err()
            );
            let final_body = final_request.body.clone();
            let wire::PrepareReply::Prepared { step: final_step } =
                prepare(&c.f.state, &c.f.chat, &c.token, final_request)
                    .await
                    .unwrap()
            else {
                panic!("cached refusal proof");
            };
            to_bytes(
                callback(&c, &final_body, Some(&final_step))
                    .await
                    .unwrap()
                    .into_body(),
                wire::MAX_STREAM_BYTES,
            )
            .await
            .unwrap();
            assert!(callback(&c, &final_body, Some(&final_step)).await.is_err());
        }
        assert_eq!(
            peer.calls.load(Ordering::SeqCst),
            if kind == "denied" { 3 } else { 2 }
        );
        assert_eq!(
            peer.tool_calls.load(Ordering::SeqCst),
            usize::from(kind == "truncated")
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            if kind == "denied" { 2 } else { 1 }
        );
        c.f.state.db.drop().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn real_invalid_a_b_a_reuses_b_activity_only_after_two_actual_policy_attempts() {
    let (mut c, peer, server) = fixture("completed").await;
    let connection = c.connection.clone();
    let args = |ack: &str| json!({"tool_name":"nyxid__set_service_enabled","arguments_json":json!({"service_id":connection,"enabled":false,"acknowledgement_id":ack}).to_string()});
    let declarations:Vec<Value>=[("call-a","A"),("call-b","B"),("call-c","A")].into_iter().map(|(id,ack)|
        json!({"type":"function_call","id":format!("fc-{id}"),"status":"completed","call_id":id,"name":"nyx__call_tool","arguments":args(ack).to_string()})).collect();
    *peer.call_override.lock().await = Some(json!(declarations));
    let mut initial = request(vec![user("initial")]);
    initial.body["tools"][0]["name"] = json!("nyx__call_tool");
    let card = c.observe_request(initial).await;
    c.resume_card(card).await;
    let mut candidate = request(vec![user("Saved Allow narration")]);
    candidate.body["tools"][0]["name"] = json!("nyx__call_tool");
    let body = candidate.body.clone();
    let wire::PrepareReply::Prepared { step } = prepare(&c.f.state, &c.f.chat, &c.token, candidate)
        .await
        .unwrap()
    else {
        panic!("prepared");
    };
    peer.eof.add_permits(1);
    to_bytes(
        callback(&c, &body, Some(&step)).await.unwrap().into_body(),
        wire::MAX_STREAM_BYTES,
    )
    .await
    .unwrap();
    let wire::PrepareReply::Completed { receipt } = current(&c, &step).await else {
        panic!("completed");
    };
    let attempt = |id: &str, ack: &str, source| wire::ToolPrepareRequest {
        protocol: wire::PROTOCOL.into(),
        name: "nyx__call_tool".into(),
        arguments: args(ack),
        call_id: Some(id.into()),
        result_char_limit: 1024,
        refusal_source: source,
    };
    let before = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap()
        .activity_total;
    let mut proofs = Vec::new();
    let mut outputs = Vec::new();
    for (id, ack) in [("call-a", "A"), ("call-b", "B")] {
        let tool = tool_prepare(&c.f.state, &c.f.chat, &c.token, attempt(id, ack, None))
            .await
            .unwrap();
        let rpc = actual_named_tool(&c, &tool, "nyx__call_tool", args(ack)).await;
        assert!(rpc["error"].is_null());
        let projection = wire::tool_result_projection(&rpc["result"], 1024);
        assert!(projection.success);
        assert!(matches!(
            projection.refusal.as_ref().map(|r| r.kind),
            Some(wire::RefusalKind::Invalid)
        ));
        let proof = tool_status(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolStatusRequest {
                protocol: wire::PROTOCOL.into(),
                tool,
            },
        )
        .await
        .unwrap();
        assert_eq!(proof.step_id, step.step_id);
        assert_eq!(proof.result_binding, digest(json!(projection.text)));
        proofs.push(proof);
        outputs.push(projection.text);
    }
    // Production confirm_action intentionally uses fixed wording for both
    // invalid IDs. Companion HTTP tests separately discriminate differing text.
    assert_eq!(outputs[0], outputs[1]);
    assert_ne!(proofs[0].server_activity_id, proofs[1].server_activity_id);
    let live = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap();
    assert_eq!(live.activity_total, before + 2);
    for proof in &proofs {
        assert!(
            live.activities
                .iter()
                .any(|a| a.id == proof.server_activity_id)
        );
    }
    let reused = tool_prepare(
        &c.f.state,
        &c.f.chat,
        &c.token,
        attempt("call-c", "A", Some(proofs[1].clone())),
    )
    .await
    .unwrap();
    let repeated = tool_prepare(
        &c.f.state,
        &c.f.chat,
        &c.token,
        attempt("call-c", "A", Some(proofs[1].clone())),
    )
    .await
    .unwrap();
    assert_eq!(reused.tool_bearer.expose(), repeated.tool_bearer.expose());
    let proof = tool_status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::ToolStatusRequest {
            protocol: wire::PROTOCOL.into(),
            tool: reused.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(proof.server_activity_id, proofs[1].server_activity_id);
    assert_eq!(proof.result_binding, proofs[1].result_binding);
    assert!(
        matches!(&proof.evidence,wire::ToolResultEvidence::ReusedRefusal{origin_step_id,origin_call_id} if origin_step_id==&step.step_id && origin_call_id=="call-b")
    );
    assert!(!actual_named_tool(&c, &reused, "nyx__call_tool", args("A")).await["error"].is_null());
    assert_eq!(
        engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .activity_total,
        before + 2
    );
    outputs.push(outputs[1].clone());
    proofs.push(proof);
    let mut next = request_with_body(&body);
    next.body["input"]
        .as_array_mut()
        .unwrap()
        .extend(declarations.iter().cloned());
    for (call, text) in declarations.into_iter().zip(outputs) {
        next.body["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"function_call_output","call_id":call["call_id"],"output":text}));
    }
    next.preceding_receipt = Some(receipt.receipt_bearer);
    next.tool_results = proofs;
    let next_body = next.body.clone();
    let wire::PrepareReply::Prepared { step } = prepare(&c.f.state, &c.f.chat, &c.token, next)
        .await
        .unwrap()
    else {
        panic!("receipt progression");
    };
    to_bytes(
        callback(&c, &next_body, Some(&step))
            .await
            .unwrap()
            .into_body(),
        wire::MAX_STREAM_BYTES,
    )
    .await
    .unwrap();
    assert_eq!(peer.calls.load(Ordering::SeqCst), 2);
    assert_eq!(peer.tool_calls.load(Ordering::SeqCst), 0);
    let connection = crate::services::user_service_service::find_user_service_by_id(
        &c.f.state.db,
        &c.connection,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(connection.is_active);
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    c.f.state.db.drop().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn real_tool_prepared_control_drift_and_stale_turn_refuse_claim_and_status_without_activity()
{
    let (mut c, peer, server) = fixture("completed").await;
    let call = |id: &str| json!({"type":"function_call","id":format!("fc-{id}"),"status":"completed","call_id":id,"name":"nyx__search_tools","arguments":"{\"query\":\"missing-test-tool\"}"});
    *peer.call_override.lock().await = Some(json!([call("call-a"), call("call-b")]));
    let (_, step) = admitted(&mut c, &peer).await;
    peer.eof.add_permits(1);
    let body = request(vec![user("Saved Allow narration")]).body;
    to_bytes(
        callback(&c, &body, Some(&step)).await.unwrap().into_body(),
        wire::MAX_STREAM_BYTES,
    )
    .await
    .unwrap();
    let mut tools = Vec::new();
    for id in ["call-a", "call-b"] {
        tools.push(
            tool_prepare(
                &c.f.state,
                &c.f.chat,
                &c.token,
                wire::ToolPrepareRequest {
                    protocol: wire::PROTOCOL.into(),
                    name: "nyx__search_tools".into(),
                    arguments: json!({"query":"missing-test-tool"}),
                    call_id: Some(id.into()),
                    result_char_limit: 1024,
                    refusal_source: None,
                },
            )
            .await
            .unwrap(),
        );
    }
    let original = c.token.clone();
    let different=issue(&c.f.state,IssueRequest{chat:&c.f.chat,runtime_catalog_id:Some(&c.target.service.id),outer_body:&json!({"input":"Saved Allow narration","store":true,"instructions":"different outer snapshot"}),negotiated_v1:true}).await.unwrap().unwrap().token;
    assert_ne!(different.expose(), original.expose());
    let before = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap()
        .activity_total;
    c.token = different;
    assert!(!actual_tool(&c, &tools[0]).await["error"].is_null());
    assert!(
        tool_status(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolStatusRequest {
                protocol: wire::PROTOCOL.into(),
                tool: tools[0].clone()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .activity_total,
        before
    );
    let different = c.token.clone();
    c.token = original.clone();
    assert!(actual_tool(&c, &tools[0]).await["error"].is_null());
    let proof = tool_status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::ToolStatusRequest {
            protocol: wire::PROTOCOL.into(),
            tool: tools[0].clone(),
        },
    )
    .await
    .unwrap();
    c.token = different;
    assert!(
        tool_status(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::ToolStatusRequest {
                protocol: wire::PROTOCOL.into(),
                tool: tools[0].clone()
            }
        )
        .await
        .is_err()
    );
    c.token = original.clone();
    let recovered = tool_status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::ToolStatusRequest {
            protocol: wire::PROTOCOL.into(),
            tool: tools[0].clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(recovered.bearer.expose(), proof.bearer.expose());
    let old_turn = c.f.chat.turn_id.clone().unwrap();
    settle_failed(&c, "invalid_stream").await;
    c.f.row = engine::begin_turn(
        &c.f.state.db,
        &c.f.owner,
        &continuation(&c, None, Some(&old_turn)),
        &c.f.state.encryption_keys,
    )
    .await
    .unwrap();
    refresh_control(&mut c).await;
    let retry_activity = engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap()
        .activity_total;
    for token in [c.token.clone(), original] {
        c.token = token;
        assert!(!actual_tool(&c, &tools[1]).await["error"].is_null());
        assert!(
            tool_status(
                &c.f.state,
                &c.f.chat,
                &c.token,
                wire::ToolStatusRequest {
                    protocol: wire::PROTOCOL.into(),
                    tool: tools[0].clone()
                }
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        engine::get(&c.f.state.db, &c.f.owner, &c.f.row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .activity_total,
        retry_activity
    );
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(peer.tool_calls.load(Ordering::SeqCst), 0);
    c.f.state.db.drop().await.unwrap();
    server.abort();
}
