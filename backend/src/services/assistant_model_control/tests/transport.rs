//! Real local proxy/provider/MCP paths. Authored source; runtime validation pending.
use super::*;
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
    routing::post,
};
use futures::TryStreamExt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Provider {
    calls: AtomicUsize,
    headers: tokio::sync::Mutex<Vec<HeaderMap>>,
    terminal: tokio::sync::Notify,
    eof: tokio::sync::Semaphore,
    mode: &'static str,
}
fn completion(call: usize) -> Value {
    json!({"id":format!("resp-{call}"),"object":"response","status":"completed","error":null,"incomplete_details":null,
        "output":if call==0 {json!([{"id":"fc-1","type":"function_call","status":"completed","call_id":"call-1",
            "name":"nyx__search_tools","arguments":"{\"query\":\"missing-test-tool\"}"}])}else {json!([{"id":"msg-2","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"done"}]}])}})
}
async fn provider(
    State(peer): State<Arc<Provider>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let call = peer.calls.fetch_add(1, Ordering::SeqCst);
    peer.headers.lock().await.push(headers);
    if peer.mode == "redirect" {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [("location", "/second-send")],
        )
            .into_response();
    }
    let mut completed = completion(call);
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
        if call==0 {peer.eof.acquire().await.unwrap().forget();}
    };
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}
async fn fixture(mode: &'static str) -> (Controlled, Arc<Provider>, tokio::task::JoinHandle<()>) {
    let mut c = Controlled::new("p2_real_transport").await;
    let peer = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        headers: tokio::sync::Mutex::new(vec![]),
        terminal: tokio::sync::Notify::new(),
        eof: tokio::sync::Semaphore::new(0),
        mode,
    });
    let second = peer.clone();
    let app = Router::new()
        .route("/responses", post(provider))
        .route(
            "/second-send",
            post(move || {
                second.calls.fetch_add(1, Ordering::SeqCst);
                async { "unexpected redirect" }
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
    let mut auth = c.f.auth.clone();
    auth.assistant_chat = Some(Arc::new(c.f.chat.clone()));
    let mut req = Request::builder().method("POST").uri(format!(
        "/api/v1/proxy/s/p2-model/responses?_nyxid_via={}",
        c.connection
    ));
    *req.headers_mut().unwrap() = c.headers();
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
        Path(("p2-model".into(), "responses".into())),
        req,
    ))
    .await
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
        .body(Body::from(json!({"jsonrpc":"2.0","id":"rpc-host-call","method":"tools/call","params":{"name":"nyx__search_tools","arguments":{"query":"missing-test-tool"}}}).to_string())).unwrap();
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
        json!({"x-codex-turn-metadata":"x".repeat(1025)}),
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
            json!({"x-codex-turn-metadata":"x".repeat(1026)}),
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
    assert!(
        held.iter()
            .all(|r| r.reserved_credits > Credits::ZERO && !r.released)
    );
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
                arguments: json!({"query":"missing-test-tool"})
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
            runtime_catalog_id: &c.target.service.id,
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
        json!({"x-codex-turn-metadata":"x".repeat(1025)}),
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
        assert!(held.reserved_credits > Credits::ZERO && !held.released && !held.forwarded);
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
