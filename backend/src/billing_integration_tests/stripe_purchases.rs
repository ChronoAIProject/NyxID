use super::*;
use crate::models::credits::Credits;
use crate::services::billing::{
    lago_client::external_purchases::{ExternalPayment, ExternalPurchaseReceipt, ExternalReversal},
    stripe::{Stripe, StripeMode, StripeSettings},
};
use serde_json::{Value, json};
use std::sync::{Mutex, atomic::AtomicBool};
use wiremock::{
    Mock, MockServer, Request as StripeRequest, ResponseTemplate,
    matchers::{method, path},
};

#[derive(Default)]
pub(super) struct LagoPurchaseFixture {
    payment: Mutex<Option<ExternalPayment>>,
    lose_first_response: AtomicBool,
    reversals: Mutex<std::collections::HashMap<String, i64>>,
    reversal_failure: AtomicUsize,
    cancellation_calls: AtomicUsize,
}

impl LagoPurchaseFixture {
    pub(super) fn apply(
        &self,
        wallet: &BillingWallet,
        payment: &ExternalPayment,
    ) -> AppResult<ExternalPurchaseReceipt> {
        let mut saved = self.payment.lock().unwrap();
        if let Some(previous) = saved.as_ref() {
            assert_eq!(previous, payment);
        } else {
            *saved = Some(payment.clone());
        }
        if self.lose_first_response.swap(false, Ordering::SeqCst) {
            return Err(AppError::BillingProviderUnavailable(
                "fixture lost the response after committing the purchase".into(),
            ));
        }
        Ok(ExternalPurchaseReceipt {
            payment: payment.clone(),
            wallet_id: wallet.lago_wallet_id.clone().unwrap(),
            wallet_transaction_id: "fixture-transaction".into(),
            status: "settled".into(),
            transaction_type: "inbound".into(),
            transaction_status: "purchased".into(),
            settled_at: Utc::now(),
        })
    }

    pub(super) fn apply_reversal(&self, reversal: &ExternalReversal<'_>) -> AppResult<String> {
        let mut reversals = self.reversals.lock().unwrap();
        if reversal.cancel_only {
            self.cancellation_calls.fetch_add(1, Ordering::SeqCst);
            if !reversals.contains_key(reversal.decision_id) {
                reversals.insert(reversal.decision_id.into(), 0);
            }
        }
        if reversals.get(reversal.decision_id) == Some(&0) {
            return Err(AppError::Conflict(
                "fixture reversal did not debit credits".into(),
            ));
        }
        if reversal.credits > 40 {
            return Err(AppError::Conflict(
                "fixture has only 40 verified unused credits in the lot".into(),
            ));
        }
        let failure = self.reversal_failure.swap(0, Ordering::SeqCst);
        if failure != 1 {
            let credits = reversals
                .entry(reversal.decision_id.into())
                .or_insert(reversal.credits);
            assert_eq!(*credits, reversal.credits);
        }
        if failure != 0 {
            return Err(AppError::BillingProviderUnavailable(
                "fixture lost the reversal response".into(),
            ));
        }
        Ok(reversal.decision_id.to_owned())
    }

    pub(super) fn balance(&self) -> i64 {
        self.payment
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.credits)
            .unwrap_or(0)
            - self.reversals.lock().unwrap().values().sum::<i64>()
    }
}

const FIXTURE_SECRET: &str = "fixture-webhook-secret";

#[tokio::test]
async fn stripe_purchase_through_billing_routes_recovers_paid_credits_and_late_document() {
    exercise_stripe_purchase_route(None, false).await;
}

#[tokio::test]
#[ignore = "requires a local pinned Lago instance with the external purchase extension"]
async fn stripe_purchase_through_billing_routes_with_local_lago() {
    exercise_stripe_purchase_route(Some(local_lago_fixture()), false).await;
}

#[tokio::test]
#[ignore = "requires a local pinned Lago instance with the external purchase extension"]
async fn stripe_purchase_through_billing_routes_with_local_lago_refund() {
    exercise_stripe_purchase_route(Some(local_lago_fixture()), true).await;
}

fn local_lago_fixture() -> (String, String, String) {
    let base = std::env::var("NYXID_TEST_LAGO_EXTENSION_URL").unwrap();
    let key = std::env::var("NYXID_TEST_LAGO_EXTENSION_KEY").unwrap();
    let secret = std::env::var("NYXID_TEST_LAGO_EXTENSION_SECRET").unwrap();
    (base, key, secret)
}

async fn exercise_stripe_purchase_route(
    local_lago: Option<(String, String, String)>,
    refund_after_use: bool,
) {
    let db = crate::test_utils::connect_transaction_test_database("stripe_purchase_route").await;
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let owner = insert_owner(&db).await;
    let foreign_owner = insert_owner(&db).await;
    let lago = Arc::new(FakeLago {
        wallet_balance: Some(Credits::ZERO),
        external_purchases: Some(LagoPurchaseFixture {
            lose_first_response: AtomicBool::new(true),
            ..Default::default()
        }),
        ..Default::default()
    });
    let mut state = billing_route_state(db.clone(), lago.clone(), 0);
    let mut real_connector = None;
    let catalog_id = Uuid::new_v4().to_string();
    let fixture_metric = format!("nx_stripe_fixture_{}", Uuid::new_v4().simple());
    let lago_secret = match local_lago.as_ref() {
        Some((base, key, secret)) => {
            let url = url::Url::parse(base).unwrap();
            assert!(
                matches!(url.host_str(), Some("127.0.0.1" | "localhost")),
                "local Lago fixture only"
            );
            let plan = format!("nx_stripe_fixture_{}", Uuid::new_v4().simple());
            let http = reqwest::Client::new();
            let metric: Value = http.post(format!("{base}/api/v1/billable_metrics")).bearer_auth(key)
                .json(&json!({"billable_metric": {"name":"NyxID Stripe fixture requests", "code": fixture_metric,
                    "aggregation_type":"sum_agg", "field_name":"quantity", "recurring":false}}))
                .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
            http.post(format!("{base}/api/v1/plans")).bearer_auth(key)
                .json(&json!({"plan": {"name": "NyxID mount test", "code": plan, "interval": "monthly",
                    "amount_cents": 0, "amount_currency": "USD", "pay_in_advance": false,
                    "charges": [{"billable_metric_id": metric["billable_metric"]["lago_id"],
                        "charge_model":"standard","properties":{"amount":"10"}}]}}))
                .send().await.unwrap().error_for_status().unwrap();
            http.post(format!("{base}/api/v1/features"))
                .bearer_auth(key)
                .json(&json!({"feature":{"code":catalog_id, "name":"Metered proxy fixture"}}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            http.post(format!("{base}/api/v1/plans/{plan}/entitlements"))
                .bearer_auth(key)
                .json(&json!({"entitlements":{(catalog_id.as_str()):{}}}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            let real = Arc::new(
                crate::services::billing::lago_client::LagoClient::new(base.clone(), key.clone())
                    .unwrap(),
            );
            let mut config = state.config.clone();
            config.lago_plan_code = plan;
            state.billing = Arc::new(BillingService::new_with_lago(
                db.clone(),
                Arc::new(config),
                real.clone(),
            ));
            real_connector = Some(real);
            secret.clone()
        }
        None => "lago-fixture-key-long-enough-for-fixture".into(),
    };
    let provider = MockServer::start().await;
    let stripe = Stripe::with_endpoint(
        StripeSettings {
            mode: StripeMode::Test,
            account_id: "acct_fixture".into(),
            portal_configuration_id: "bpc_fixture".into(),
            product_tax_code: "txcd_fixture".into(),
            offer_version: "fixture-v1".into(),
            unit_amount_minor: 100,
            maximum_subtotal_minor: 1_000_000,
        },
        "http://localhost:3000",
        "sk_test_fixture".into(),
        FIXTURE_SECRET.into(),
        lago_secret,
        &format!("{}/", provider.uri()),
    )
    .unwrap();
    let service = (*state.billing).clone().with_stripe(stripe);
    let wallet = service.ensure_wallet(&owner).await.unwrap().wallet;
    state.billing = Arc::new(service);
    let token = route_access_token(&state, &owner);
    let foreign_token = route_access_token(&state, &foreign_owner);
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let app = public.merge(private).with_state(state.clone());

    Mock::given(method("GET"))
        .and(path("/account"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "acct_fixture"})))
        .mount(&provider)
        .await;
    Mock::given(method("GET"))
        .and(path("/customers/cus_fixture"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cus_fixture", "livemode": false, "metadata": {"nyxid_owner_id": &owner},
        })))
        .mount(&provider)
        .await;

    let admin = insert_owner(&db).await;
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let admin_role = db
        .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
        .find_one(doc! {"slug": crate::models::role::PLATFORM_ADMIN_ROLE_SLUG})
        .await
        .unwrap()
        .unwrap();
    db.collection::<User>(USERS)
        .update_one(
            doc! {"_id": &admin},
            doc! {"$addToSet": {"role_ids": admin_role.id}},
        )
        .await
        .unwrap();
    let admin_token = route_access_token(&state, &admin);
    let enrollment = json!({"owner_id": owner, "stripe_customer_id": "cus_fixture"});
    let (status, body) = call(
        &app,
        &foreign_token,
        Method::POST,
        "/api/v1/admin/billing/stripe/enroll",
        enrollment.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = call(
        &app,
        &admin_token,
        Method::POST,
        "/api/v1/admin/billing/stripe/enroll",
        json!({"owner_id":foreign_owner, "stripe_customer_id":"cus_fixture"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = call(
        &app,
        &admin_token,
        Method::POST,
        "/api/v1/admin/billing/stripe/enroll",
        enrollment,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    if real_connector.is_none() {
        let capability_server = MockServer::start().await;
        let wallet = super::wallet(&db, &owner).await;
        let wallet_id = wallet.lago_wallet_id.as_deref().unwrap();
        let endpoint = format!(
            "/api/v1/wallets/{}/nyxid_external_credit_purchases/capability",
            urlencoding::encode(wallet_id)
        );
        let client = crate::services::billing::lago_client::LagoClient::new(
            capability_server.uri(),
            "fixture-key".into(),
        )
        .unwrap();
        for version in [1, 2] {
            let _mock = Mock::given(method("GET"))
                .and(path(endpoint.clone()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "version":version, "wallet_id":wallet_id, "invoice_created":false,
                    "stripe_account_id":"acct_fixture", "livemode":false,
                })))
                .expect(1)
                .mount_as_scoped(&capability_server)
                .await;
            let result = client
                .verify_external_credit_wallet(&wallet, "acct_fixture", "fixture-capability-secret")
                .await;
            if version == 1 {
                assert!(matches!(result, Err(AppError::Conflict(_))), "{result:?}");
            } else {
                result.unwrap();
            }
        }
    }
    let provider_calls = provider.received_requests().await.unwrap().len();
    let invalid = json!({"amount_credits": 9, "idempotency_key": "test-below-minimum"});
    let (status, _) = call(&app, &token, Method::POST, "/api/v1/billing/topup", invalid).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        provider.received_requests().await.unwrap().len(),
        provider_calls
    );

    let order = crate::services::billing::provisioning::deterministic_topup_session_id(
        &owner,
        "stripe-purchase-fixture",
    );
    let order_for_response = order.clone();
    let form_checked = Arc::new(AtomicBool::new(false));
    let checked = form_checked.clone();
    let lose_checkout = real_connector.is_none();
    Mock::given(method("POST"))
        .and(path("/checkout/sessions"))
        .and(checkout_matches(order.clone()))
        .respond_with(move |request: &StripeRequest| {
            let form = url::form_urlencoded::parse(&request.body)
                .into_owned()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(form["automatic_tax[enabled]"], "true");
            assert_eq!(form["invoice_creation[enabled]"], "true");
            assert_eq!(form["tax_id_collection[enabled]"], "true");
            assert_eq!(form["line_items[0][quantity]"], "100");
            assert_eq!(form["line_items[0][price_data][unit_amount]"], "100");
            assert_eq!(
                request.headers["idempotency-key"].to_str().unwrap(),
                order_for_response.as_str()
            );
            if !checked.swap(true, Ordering::SeqCst) && lose_checkout {
                return ResponseTemplate::new(503);
            }
            ResponseTemplate::new(200).set_body_json(checkout(
                &order_for_response,
                "open",
                "unpaid",
                false,
            ))
        })
        .expect(if lose_checkout { 2 } else { 1 })
        .mount(&provider)
        .await;
    let request = json!({"amount_credits": 100, "idempotency_key": "stripe-purchase-fixture"});
    let (status, body) = call(
        &app,
        &token,
        Method::POST,
        "/api/v1/billing/topup",
        request.clone(),
    )
    .await;
    let (status, body) = if lose_checkout {
        assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
        state.billing.reconciler().run_once().await.unwrap();
        call(
            &app,
            &token,
            Method::POST,
            "/api/v1/billing/topup",
            request.clone(),
        )
        .await
    } else {
        (status, body)
    };
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["checkout_url"],
        "https://checkout.stripe.com/c/pay/fixture"
    );
    assert!(form_checked.load(Ordering::SeqCst));
    let (status, body) = call(&app, &token, Method::POST, "/api/v1/billing/topup", request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reused"], true);

    let mode = Arc::new(AtomicUsize::new(0));
    let mode_for_response = mode.clone();
    let order_for_response = order.clone();
    Mock::given(method("GET"))
        .and(path(format!("/checkout/sessions/cs_test_{order}")))
        .respond_with(move |_: &StripeRequest| {
            let state = mode_for_response.load(Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(checkout(
                &order_for_response,
                "complete",
                if state == 0 { "unpaid" } else { "paid" },
                state >= 2,
            ))
        })
        .mount(&provider)
        .await;
    let refund_observed = Arc::new(AtomicUsize::new(0));
    let refund_flag = refund_observed.clone();
    let order_for_response = order.clone();
    Mock::given(method("GET")).and(path("/refunds"))
        .respond_with(move |_: &StripeRequest| {
            let stage = refund_flag.load(Ordering::SeqCst);
            let data = if stage == 0 { vec![] } else {
                let mut data = vec![json!({"id": format!("re_{order_for_response}"), "charge": format!("ch_{order_for_response}"),
                    "payment_intent": format!("pi_{order_for_response}"), "currency":"usd",
                    "status":if stage == 2 || stage >= 4 {"succeeded"} else {"pending"},"amount":2180})];
                if stage >= 4 {
                    data.push(json!({"id": format!("re_other_{order_for_response}"), "charge": format!("ch_{order_for_response}"),
                        "payment_intent": format!("pi_{order_for_response}"), "currency":"usd","status":"succeeded","amount":120}));
                }
                for (index, starts_at) in [(1, 5), (2, 7)] {
                    if stage >= starts_at {
                        data.push(json!({"id":format!("re_uncertain_{index}_{order_for_response}"),
                            "charge":format!("ch_{order_for_response}"),"currency":"usd","amount":100,
                            "status":if stage == starts_at {"succeeded"} else {"failed"}}));
                    }
                }
                if stage >= 9 {
                    data.push(json!({"id":format!("re_new_pending_{order_for_response}"),
                        "charge":format!("ch_{order_for_response}"),"currency":"usd","amount":100,
                        "status":if stage == 9 {"pending"} else {"failed"}}));
                }
                data
            };
            ResponseTemplate::new(200).set_body_json(json!({"data": data, "has_more": false}))
        }).mount(&provider).await;
    let refund_flag = refund_observed.clone();
    let disputed = Arc::new(AtomicUsize::new(0));
    let disputed_flag = disputed.clone();
    let order_for_response = order.clone();
    Mock::given(method("GET")).and(path(format!("/payment_intents/pi_{order}")))
        .respond_with(move |_: &StripeRequest| ResponseTemplate::new(200).set_body_json(json!({
            "id":format!("pi_{order_for_response}"), "livemode":false, "customer":"cus_fixture", "status":"succeeded",
            "currency":"usd", "amount_received":10900, "latest_charge":{"id":format!("ch_{order_for_response}"),
                "amount_refunded": match refund_flag.load(Ordering::SeqCst) {
                    0 => 0, 3 => 2190, 5 | 7 => 2400, 4.. => 2300, _ => 2180
                },
                "disputed": disputed_flag.load(Ordering::SeqCst) != 0}
        }))).mount(&provider).await;
    let disputed_flag = disputed.clone();
    let order_for_response = order.clone();
    Mock::given(method("GET")).and(path("/disputes"))
        .respond_with(move |_: &StripeRequest| {
            let stage = disputed_flag.load(Ordering::SeqCst);
            if stage == 0 {
                return ResponseTemplate::new(200).set_body_json(json!({"data": [], "has_more": false}));
            }
            let status = if stage == 1 {"needs_response"} else {"lost"};
            ResponseTemplate::new(200).set_body_json(json!({"data": [{
                "id":format!("dp_{order_for_response}"),"charge":format!("ch_{order_for_response}"),
                "payment_intent":format!("pi_{order_for_response}"),"currency":"usd","livemode":false,"status":status,"amount":500,
            }], "has_more":false}))
        }).mount(&provider).await;

    assert_eq!(
        event(&app, &order, "checkout.session.completed", false).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        event(&app, &order, "checkout.session.completed", true).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(lago.external_purchases.as_ref().unwrap().balance(), 0);
    mode.store(1, Ordering::SeqCst);
    if real_connector.is_none() {
        // A lost dispute can first be observed while the paid order has no
        // Lago credit lot. A zero-credit terminal decision must release it.
        disputed.store(1, Ordering::SeqCst);
        assert_eq!(
            event(&app, &order, "checkout.session.completed", true).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(lago.external_purchases.as_ref().unwrap().balance(), 0);
        let path = format!("/api/v1/admin/billing/stripe/purchases/{order}/reversals/dp_{order}");
        let resolution = json!({"decision_id":Uuid::new_v4(), "action":"release", "credits":0,"reason":"fixture approved zero credits"});
        assert_eq!(
            call(&app, &admin_token, Method::POST, &path, resolution.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
        disputed.store(2, Ordering::SeqCst);
        // The decision commits, then the purchase provider commits and loses
        // its first response. Stripe delivery below recovers the same lot.
        assert_eq!(
            call(&app, &admin_token, Method::POST, &path, resolution)
                .await
                .0,
            StatusCode::PAYMENT_REQUIRED
        );
        let (_, history) = call(
            &app,
            &token,
            Method::GET,
            "/api/v1/billing/topups",
            Value::Null,
        )
        .await;
        assert_eq!(history["topups"][0]["stripe"]["total_minor"], 10900);
        assert_eq!(wallet_balance(&db, &owner).await, Credits::ZERO);
        assert_eq!(lago.external_purchases.as_ref().unwrap().balance(), 100);
    }

    let status = event(
        &app,
        &order,
        "checkout.session.async_payment_succeeded",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        event(
            &app,
            &order,
            "checkout.session.async_payment_succeeded",
            true
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        event(&app, &order, "checkout.session.completed", true).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(wallet_balance(&db, &owner).await, Credits::from_whole(100));
    if let Some(connector) = &real_connector {
        let lots = connector
            .wallet_transactions(wallet.lago_wallet_id.as_deref().unwrap())
            .await
            .unwrap();
        assert_eq!(lots.len(), 1);
        assert_eq!(lots[0].credit_amount, Credits::from_whole(100));
        assert_eq!(lots[0].transaction_status, "purchased");
        assert!(connector.credit_invoices(&owner).await.unwrap().is_empty());
    }
    mode.store(2, Ordering::SeqCst);
    state.billing.reconciler().run_once().await.unwrap();
    let (_, history) = call(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/topups",
        Value::Null,
    )
    .await;
    assert_eq!(
        history["topups"][0]["status"],
        if real_connector.is_none() {
            "payment_reversed"
        } else {
            "credits_available"
        }
    );
    assert_eq!(history["topups"][0]["amount_credits"], 100);
    assert_eq!(history["topups"][0]["stripe"]["document_available"], true);

    Mock::given(method("GET"))
        .and(path(format!("/invoices/in_{order}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":format!("in_{order}"), "livemode":false, "customer":"cus_fixture", "status":"paid",
            "currency":"usd", "subtotal":10000, "amount_paid":10900,"amount_remaining":0,
            "hosted_invoice_url":"https://invoice.stripe.com/i/fixture",
        })))
        .expect(1)
        .mount(&provider)
        .await;
    let url = format!("/api/v1/billing/topups/{order}/document");
    assert_eq!(
        call(&app, &foreign_token, Method::GET, &url, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = call(&app, &token, Method::GET, &url, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["file_url"], "https://invoice.stripe.com/i/fixture");

    Mock::given(method("POST")).and(path("/billing_portal/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "livemode":false, "customer":"cus_fixture", "url":"https://billing.stripe.com/p/session/fixture",
        }))).expect(1).mount(&provider).await;
    let (status, body) = call(
        &app,
        &token,
        Method::POST,
        "/api/v1/billing/stripe-portal",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    if let (Some(connector), Some((base, key, _))) = (&real_connector, &local_lago) {
        let subscription = wallet.lago_subscription_id.as_deref().unwrap();
        spend_through_proxy(&state, &app, &owner, &token, &fixture_metric, &catalog_id).await;
        let usage = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let usage = connector.current_usage(&owner, subscription).await.unwrap();
                if usage.raw["customer_usage"]["amount_cents"] == 1000 {
                    break usage;
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        })
        .await
        .expect("Lago rated one request");
        assert_eq!(usage.raw["customer_usage"]["taxes_amount_cents"], 0);
        state.billing.reconciler().run_once().await.unwrap();
        assert_eq!(wallet_balance(&db, &owner).await, Credits::from_whole(90));
        if refund_after_use {
            refund_observed.store(2, Ordering::SeqCst);
            assert_eq!(
                event(&app, &order, "checkout.session.completed", true).await,
                StatusCode::NO_CONTENT
            );
            let route =
                format!("/api/v1/admin/billing/stripe/purchases/{order}/reversals/re_{order}");
            let decision = json!({"decision_id": Uuid::new_v4(), "action": "reverse_credits",
                "credits": 20, "reason": "Refund of twenty unused credits after one metered request"});
            for _ in 0..2 {
                let (status, body) =
                    call(&app, &admin_token, Method::POST, &route, decision.clone()).await;
                assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
            }
            assert_eq!(wallet_balance(&db, &owner).await, Credits::from_whole(70));
            let (_, history) = call(
                &app,
                &token,
                Method::GET,
                "/api/v1/billing/topups",
                Value::Null,
            )
            .await;
            assert_eq!(history["topups"][0]["stripe"]["refunded_minor"], 2180);
            assert_eq!(
                history["topups"][0]["stripe"]["reversals"][0]["returned_credits"],
                20
            );
        }
        let http = reqwest::Client::new();
        http.delete(format!(
            "{base}/api/v1/subscriptions/{}",
            urlencoding::encode(subscription)
        ))
        .query(&[("on_termination_invoice", "generate")])
        .bearer_auth(key)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
        let invoice = tokio::time::timeout(std::time::Duration::from_secs(45), async {
            loop {
                let invoices = connector
                    .subscription_invoices(&owner, 1, 20)
                    .await
                    .unwrap();
                if let Some(invoice) =
                    invoices["invoices"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|invoice| {
                            invoice["fees_amount_cents"] == 1000 && invoice["status"] == "finalized"
                        })
                {
                    break invoice.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        })
        .await
        .expect("Lago closed the fixture period");
        assert_eq!(invoice["prepaid_credit_amount_cents"], 1000);
        assert_eq!(invoice["total_due_amount_cents"], 0);
        assert_eq!(invoice["taxes_amount_cents"], 0);
        assert_eq!(
            connector.wallet_balance_exact(&owner).await.unwrap(),
            Credits::from_whole(if refund_after_use { 70 } else { 90 })
        );
        return;
    }
    // Missed webhook detection and a refused decision correction through
    // the mounted admin route.
    refund_observed.store(3, Ordering::SeqCst);
    assert_eq!(
        event(&app, &order, "checkout.session.completed", true).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (_, held_history) = call(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/topups",
        Value::Null,
    )
    .await;
    assert_eq!(held_history["topups"][0]["status"], "reversal_review");
    assert_eq!(held_history["topups"][0]["stripe"]["refunded_minor"], 0);
    refund_observed.store(1, Ordering::SeqCst);
    assert_eq!(
        event(&app, &order, "checkout.session.completed", true).await,
        StatusCode::NO_CONTENT
    );
    let review_order = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "stripe-review",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Requests,
        },
        "held-stripe-usage",
        &owner,
    );
    assert!(matches!(
        state.billing.open(&review_order).await,
        Err(AppError::WalletPaymentReview)
    ));
    refund_observed.store(2, Ordering::SeqCst);
    db.collection::<bson::Document>(crate::models::billing_topup_session::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &order},
            doc! {"$set": {
                "updated_at": bson::DateTime::from_chrono(Utc::now() - Duration::hours(2)),
            }},
        )
        .await
        .unwrap();
    state.billing.reconciler().run_once().await.unwrap();
    let (_, history) = call(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/topups",
        Value::Null,
    )
    .await;
    assert_eq!(history["topups"][0]["status"], "reversal_review");
    let (blocked, _) = call(
        &app,
        &token,
        Method::POST,
        "/api/v1/billing/topup",
        json!({"amount_credits":20,"idempotency_key":"held-wallet-new-purchase"}),
    )
    .await;
    assert_eq!(blocked, StatusCode::CONFLICT);
    let route = format!("/api/v1/admin/billing/stripe/purchases/{order}/reversals/re_{order}");
    let refused = json!({"decision_id":Uuid::new_v4(), "action":"reverse_credits", "credits":41,"reason":"fixture exceeds lot remainder"});
    assert_eq!(
        call(&app, &admin_token, Method::POST, &route, refused)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (_, list) = call(
        &app,
        &admin_token,
        Method::GET,
        &format!("/api/v1/admin/billing/stripe/purchases/{order}/reversals"),
        Value::Null,
    )
    .await;
    let refund = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["provider_id"].as_str().unwrap().starts_with("re_"))
        .unwrap();
    assert_eq!(refund["decision"]["state"], "refused");
    let corrected = json!({"decision_id":Uuid::new_v4(), "action":"reverse_credits", "credits":20,"reason":"fixture verified unused amount"});
    assert_eq!(
        call(&app, &admin_token, Method::POST, &route, corrected.clone())
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, &admin_token, Method::POST, &route, corrected)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(wallet_balance(&db, &owner).await, Credits::from_whole(80));
    let (_, history) = call(
        &app,
        &token,
        Method::GET,
        "/api/v1/billing/topups",
        Value::Null,
    )
    .await;
    assert_eq!(history["topups"][0]["status"], "payment_reversed");
    assert_eq!(history["topups"][0]["stripe"]["refunded_minor"], 2180);
    let reversal = history["topups"][0]["stripe"]["reversals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "refund")
        .unwrap();
    assert_eq!(reversal["returned_credits"], 20);
    refund_observed.store(4, Ordering::SeqCst);
    assert_eq!(
        event(&app, &order, "checkout.session.completed", true).await,
        StatusCode::NO_CONTENT
    );
    let second = format!("{}/re_other_{order}", route.rsplit_once('/').unwrap().0);
    let zero = json!({"decision_id": Uuid::new_v4(), "action":"write_off", "credits":0,
        "reason":"fixture approved a monetary-only correction"});
    assert_eq!(
        call(&app, &admin_token, Method::POST, &second, zero)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(wallet_balance(&db, &owner).await, Credits::from_whole(80));

    let fixture = lago.external_purchases.as_ref().unwrap();
    for (index, stage, expected_state, balance) in [(1, 5, "refused", 80), (2, 7, "applied", 79)] {
        let refund = format!("re_uncertain_{index}_{order}");
        let route = format!("/api/v1/admin/billing/stripe/purchases/{order}/reversals/{refund}");
        refund_observed.store(stage, Ordering::SeqCst);
        fixture.reversal_failure.store(index, Ordering::SeqCst);
        let (status, body) = call(
            &app,
            &admin_token,
            Method::POST,
            &route,
            json!({
                "decision_id": Uuid::new_v4(), "action":"reverse_credits", "credits":1,
                "reason":"fixture approved a credit reversal"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");

        refund_observed.store(stage + 1, Ordering::SeqCst);
        assert_eq!(
            event(&app, &order, "checkout.session.completed", true).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(fixture.cancellation_calls.load(Ordering::SeqCst), index);
        assert_eq!(
            wallet_balance(&db, &owner).await,
            Credits::from_whole(balance)
        );
        let saved = db
            .collection::<bson::Document>(crate::models::billing_topup_session::COLLECTION_NAME)
            .find_one(doc! {"_id": &order})
            .await
            .unwrap()
            .unwrap();
        let purchase = saved.get_document("stripe_purchase").unwrap();
        assert!(!purchase.get_bool("reversal_review_required").unwrap());
        let reversal = purchase
            .get_array("reversals")
            .unwrap()
            .iter()
            .filter_map(bson::Bson::as_document)
            .find(|row| row.get_str("provider_id") == Ok(refund.as_str()))
            .unwrap();
        let decision = reversal.get_document("decision").unwrap();
        assert_eq!(decision.get_str("state"), Ok(expected_state));
        if index == 1 {
            continue;
        }
        let diagnostic_code = "stripe_purchase_finance_correction";
        let diagnostics = crate::services::billing::startup_diagnostics(&db)
            .await
            .unwrap();
        assert!(diagnostics.iter().any(|d| d.code == diagnostic_code
            && d.technical_details.as_deref() == Some(format!("{order}/{refund}").as_str())));
        refund_observed.store(9, Ordering::SeqCst);
        assert_eq!(
            event(&app, &order, "checkout.session.completed", true).await,
            StatusCode::NO_CONTENT
        );
        let correction_path = format!("{route}/finance-correction");
        let void_id = decision.get_str("lago_void_transaction_id").unwrap();
        let correction =
            json!({"lago_void_transaction_id": void_id, "reference": "test-ledger-adjustment-42"});
        let wrong_void = json!({"lago_void_transaction_id": Uuid::new_v4(), "reference": "test-ledger-adjustment-42"});
        assert_eq!(
            call(
                &app,
                &admin_token,
                Method::PUT,
                &correction_path,
                wrong_void
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(
                &app,
                &token,
                Method::PUT,
                &correction_path,
                correction.clone()
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        for _ in 0..2 {
            let (status, body) = call(
                &app,
                &admin_token,
                Method::PUT,
                &correction_path,
                correction.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        }
        let changed_reference =
            json!({"lago_void_transaction_id": void_id, "reference": "a different reference"});
        assert_eq!(
            call(
                &app,
                &admin_token,
                Method::PUT,
                &correction_path,
                changed_reference
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        let (_, admin_reversals) = call(
            &app,
            &admin_token,
            Method::GET,
            &format!("/api/v1/admin/billing/stripe/purchases/{order}/reversals"),
            Value::Null,
        )
        .await;
        let corrected = admin_reversals
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["provider_id"] == refund)
            .unwrap();
        assert_eq!(corrected["finance_correction_required"], false);
        assert_eq!(
            corrected["finance_correction"]["reference"],
            "test-ledger-adjustment-42"
        );
        assert!(matches!(
            state.billing.open(&review_order).await,
            Err(AppError::WalletPaymentReview)
        ));
        refund_observed.store(10, Ordering::SeqCst);
        assert_eq!(
            event(&app, &order, "checkout.session.completed", true).await,
            StatusCode::NO_CONTENT
        );
        let diagnostics = crate::services::billing::startup_diagnostics(&db)
            .await
            .unwrap();
        assert!(!diagnostics.iter().any(|d| d.code == diagnostic_code));
        assert_eq!(
            wallet_balance(&db, &owner).await,
            Credits::from_whole(balance)
        );
        let audit = db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(
            audit
                .count_documents(
                    doc! {"event_type": "billing_stripe_finance_correction_acknowledged",
                    "event_data.stripe_reversal_id": &refund, "seq": {"$type": "long"}}
                )
                .await
                .unwrap(),
            1
        );
    }

    let case = CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "stripe_credits",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Requests,
    };
    let tax_reads = lago.tax_fetches.load(Ordering::SeqCst);
    let usage = route_context(&case, "stripe-credit-only", &owner);
    let meter = state.billing.open(&usage).await.unwrap();
    state.billing.mark_forwarded(&meter).await.unwrap();
    state
        .billing
        .settle(&meter, PlatformUsage::single_request(0), None, None)
        .await
        .unwrap();
    let settlement = usage_row(&db, "stripe-credit-only").await.funding.unwrap();
    assert!(!settlement.tax_accounting);
    assert_eq!(settlement.wallet_tax_credits, Some(Credits::ZERO));
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), tax_reads);
    let balance_before_unpaid_checkouts = fixture.balance();
    assert_eq!(balance_before_unpaid_checkouts, 79);
    // Terminal delayed payment events retain the order and remove the resume
    // link. They cannot deliver credits by user redirect or event shape alone.
    for (name, provider_status, event_kind, displayed) in [
        (
            "delayed-failure",
            "complete",
            "checkout.session.async_payment_failed",
            "failed",
        ),
        (
            "checkout-expired",
            "expired",
            "checkout.session.expired",
            "expired",
        ),
    ] {
        let terminal_order =
            crate::services::billing::provisioning::deterministic_topup_session_id(&owner, name);
        Mock::given(method("POST"))
            .and(path("/checkout/sessions"))
            .and(checkout_matches(terminal_order.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(checkout(
                &terminal_order,
                "open",
                "unpaid",
                false,
            )))
            .expect(1)
            .mount(&provider)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/checkout/sessions/cs_test_{terminal_order}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(checkout(
                &terminal_order,
                provider_status,
                "unpaid",
                false,
            )))
            .expect(1)
            .mount(&provider)
            .await;
        assert_eq!(
            call(
                &app,
                &token,
                Method::POST,
                "/api/v1/billing/topup",
                json!({
                    "amount_credits":100,"idempotency_key":name,
                })
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            event(&app, &terminal_order, event_kind, true).await,
            StatusCode::NO_CONTENT
        );
        let (_, history) = call(
            &app,
            &token,
            Method::GET,
            "/api/v1/billing/topups",
            Value::Null,
        )
        .await;
        let row = history["topups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == terminal_order)
            .unwrap();
        assert_eq!(row["status"], displayed);
        assert!(row["checkout_url"].is_null());
        assert_eq!(fixture.balance(), balance_before_unpaid_checkouts);
    }
}

async fn spend_through_proxy(
    state: &crate::AppState,
    app: &Router,
    owner: &str,
    token: &str,
    metric: &str,
    catalog_id: &str,
) {
    use crate::models::service_billing::{PricingSyncStatus, ServicePlatformPricing};

    let db = &state.db;
    set_billing_fixture_approval_required(db, owner, false).await;
    let (downstream_url, downstream) = start_billing_downstream().await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = catalog_id.into();
    catalog.slug = "stripe-purchase-proxy".into();
    catalog.base_url = downstream_url.clone();
    catalog.billing = Some(ServiceBilling {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: false,
        platform_metric: Some(BillingMetric::Requests),
        platform_pricing: Some(ServicePlatformPricing {
            credits_per_unit: "10".into(),
            lago_metric_code: metric.into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
        }),
        ..Default::default()
    });
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let service = insert_route_service(
        db,
        owner,
        "stripe-purchase-proxy",
        &downstream_url,
        Some(&catalog.id),
        None,
    )
    .await;
    call_mounted_route(
        app,
        route_request(
            Method::GET,
            "/api/v1/proxy/s/stripe-purchase-proxy/ping",
            token,
            Body::empty(),
        ),
    )
    .await;
    let rows = wait_for_charged_usage(db, doc! {"service_slug": &service.slug}, 1).await;
    let funding = rows[0].funding.as_ref().unwrap();
    assert_eq!(funding.wallet_charge_credits, Some(Credits::from_whole(10)));
    assert_eq!(funding.wallet_tax_credits, Some(Credits::ZERO));
    assert!(!funding.tax_accounting);
    // The sender gives newly finalized usage a grace period. Use the normal
    // sender and wait for its durable acknowledgement before closing the period.
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            state
                .billing
                .reconciler()
                .push_usage_for_test()
                .await
                .unwrap();
            let row = db
                .collection::<UsageMeterRow>(USAGE_METER)
                .find_one(doc! {"_id": &rows[0].id})
                .await
                .unwrap()
                .unwrap();
            if row.lago_acked {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    })
    .await
    .expect("Lago acknowledged the metered proxy request");
    downstream.abort();
}

fn checkout_matches(order: String) -> impl wiremock::Match {
    move |request: &StripeRequest| {
        url::form_urlencoded::parse(&request.body)
            .any(|(key, value)| key == "client_reference_id" && value == order.as_str())
    }
}

fn checkout(order: &str, status: &str, payment_status: &str, invoice: bool) -> Value {
    let paid = payment_status == "paid";
    json!({
        "id":format!("cs_test_{order}"),"url":"https://checkout.stripe.com/c/pay/fixture", "livemode":false,
        "customer":"cus_fixture", "client_reference_id":order, "mode":"payment","status":status,
        "payment_status":payment_status,"currency":"usd","amount_subtotal":10000,
        "amount_total":if paid {10900} else {10000},
        "automatic_tax":{"enabled":true,"status":paid.then_some("complete")},
        "total_details":{"amount_tax":if paid {900} else {0},"amount_discount":0,"amount_shipping":0},
        "payment_intent":paid.then(|| format!("pi_{order}")),"invoice":invoice.then(|| format!("in_{order}")),
    })
}

async fn wallet_balance(db: &mongodb::Database, owner: &str) -> Credits {
    db.collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! {"owner_id": owner})
        .await
        .unwrap()
        .unwrap()
        .balance_credits
}

async fn call(
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
            Body::from(body.to_string()),
        ))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

async fn event(app: &Router, order: &str, kind: &str, valid: bool) -> StatusCode {
    let body = json!({"type":kind,"livemode":false,"data":{"object":{"id":format!("cs_test_{order}"),"client_reference_id":order}}}).to_string();
    let timestamp = Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(FIXTURE_SECRET.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{body}").as_bytes());
    let signature = if valid {
        hex::encode(mac.finalize().into_bytes())
    } else {
        "bad".into()
    };
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/webhooks/stripe-billing")
        .header("stripe-signature", format!("t={timestamp},v1={signature}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    if status.is_server_error() || status == StatusCode::PAYMENT_REQUIRED {
        let body = to_bytes(response.into_body(), 100_000).await.unwrap();
        eprintln!(
            "{kind} returned HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    status
}
