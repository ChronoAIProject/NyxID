//! Canonical API client fixtures, not a native Codex/deployment claim.
use super::*;

pub(super) async fn provider(
    State(peer): State<Arc<Provider>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    peer.calls.fetch_add(1, Ordering::SeqCst);
    peer.headers.lock().await.push(headers);
    peer.bodies
        .lock()
        .await
        .push(("responses/compact".into(), body.clone()));
    if peer.mode == "compact_redirect" {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [("location", "/second-send")],
        )
            .into_response();
    }
    let mut output = body["input"].as_array().unwrap().clone();
    // Return a genuinely different canonical window, retaining tool history
    // and privileged text but replacing the ordinary user item with compaction.
    output.retain(|item| item["role"] != "user");
    output.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"canonical summary"}]}));
    output.push(json!({"type":"compaction","id":"cmp-item-1","encrypted_content":"opaque canonical provider state"}));
    let response = json!({"id":"cmp-response-1","object":"response.compaction","created_at":1793160000,
        "output":output,"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}});
    let sse = peer.mode == "compact_sse";
    let bytes = if sse {
        format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":response})
        )
        .into_bytes()
    } else {
        serde_json::to_vec(&response).unwrap()
    };
    let stream = async_stream::stream! {
        let middle = bytes.len()/2;
        yield Ok::<Bytes,std::io::Error>(Bytes::copy_from_slice(&bytes[..middle]));
        if peer.mode=="compact_truncated" {return;}
        yield Ok(Bytes::copy_from_slice(&bytes[middle..]));
        peer.terminal.notify_one();
        peer.eof.acquire().await.unwrap().forget();
    };
    (
        [(
            "content-type",
            if sse {
                "text/event-stream"
            } else {
                "application/json"
            },
        )],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn completed_generation(
    c: &mut Controlled,
    peer: &Provider,
) -> (String, wire::PreparedStep, wire::PrepareRequest) {
    let (card, step) = admitted(c, peer).await;
    let response = callback(
        c,
        &request(vec![user("Saved Allow narration")]).body,
        Some(&step),
    )
    .await
    .unwrap();
    peer.eof.add_permits(1);
    to_bytes(response.into_body(), wire::MAX_STREAM_BYTES)
        .await
        .unwrap();
    peer.terminal.notified().await;
    let wire::PrepareReply::Completed { receipt } = current(c, &step).await else {
        panic!("actual generation EOF must commit completion");
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
            result_char_limit: 10000,
            refusal_source: None,
        },
    )
    .await
    .unwrap();
    let result = actual_tool(c, &tool).await;
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
    let mut input = vec![user("Saved Allow narration")];
    input.extend(completion(0)["output"].as_array().unwrap().iter().cloned());
    let projected = wire::tool_result_projection(&result["result"], 10000);
    input.push(json!({"type":"function_call_output","call_id":"call-1","output":projected.text}));
    let mut compact = request(vec![]);
    compact.relative_path = "responses/compact".into();
    compact.body = json!({"model":"model-v1","input":input});
    compact.preceding_receipt = Some(receipt.receipt_bearer);
    compact.tool_results = vec![proof];
    (card, step, compact)
}

fn copy_request(candidate: &wire::PrepareRequest) -> wire::PrepareRequest {
    serde_json::from_value(serde_json::to_value(candidate).unwrap()).unwrap()
}

#[tokio::test]
async fn real_one_allow_generation_tool_compact_json_eof_and_full_replacement_keep_one_family() {
    let (mut c, peer, server) = fixture("compact_completed").await;
    let (card, generation, candidate) = completed_generation(&mut c, &peer).await;
    let saved =
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {"_id":&card})
            .await
            .unwrap()
            .unwrap();
    assert!(
        saved
            .get_str("summary")
            .unwrap()
            .contains("Responses generation and standalone compact family")
    );
    for change in [
        "missing_receipt",
        "changed_result",
        "changed_model",
        "instructions",
        "tools",
        "metadata",
        "stream",
    ] {
        let mut changed = copy_request(&candidate);
        match change {
            "missing_receipt" => changed.tool_results.clear(),
            "changed_result" => changed.body["input"][2]["output"] = json!("forged result"),
            "changed_model" => changed.body["model"] = json!("another-model"),
            "instructions" => changed.body["instructions"] = json!("new instructions"),
            "tools" => changed.body["tools"] = json!([]),
            "metadata" => changed.body["client_metadata"] = json!({"unknown":"changed"}),
            _ => changed.body["stream"] = json!(false),
        }
        assert!(
            prepare(&c.f.state, &c.f.chat, &c.token, changed)
                .await
                .is_err(),
            "{change}"
        );
    }
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    let wire::PrepareReply::Prepared { step } =
        prepare(&c.f.state, &c.f.chat, &c.token, copy_request(&candidate))
            .await
            .unwrap()
    else {
        panic!("actual completed tool receipt must admit compact");
    };
    assert_eq!(step.ordinal, 1);
    assert_eq!(step.root_id, generation.root_id);
    // Recovery returns the exact unsent step, never a second ordinal/capability.
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
        panic!("same prepare")
    };
    assert_eq!(recovered.step_id, step.step_id);
    assert_eq!(recovered.step_bearer.expose(), step.step_bearer.expose());
    for change in ["body", "options", "path"] {
        let mut changed = copy_request(&candidate);
        match change {
            "body" => changed.body["input"][0]["content"] = json!("another unsent context"),
            "options" => changed.body["instructions"] = json!("new privileged instructions"),
            _ => {
                changed.relative_path = "responses".into();
                changed.body = request(vec![user("Saved Allow narration")]).body;
            }
        }
        assert!(
            prepare(&c.f.state, &c.f.chat, &c.token, changed)
                .await
                .is_err(),
            "{change}"
        );
        assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    }
    let (entered, mut arrivals) = tokio::sync::mpsc::channel(2);
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let first = BEFORE_FINAL_CLAIM.scope(
        (entered.clone(), release.clone()),
        callback_path(&c, &candidate.body, Some(&step), true, "responses/compact"),
    );
    let second = BEFORE_FINAL_CLAIM.scope(
        (entered, release.clone()),
        callback_path(
            &c,
            &candidate.body,
            Some(&recovered),
            true,
            "responses/compact",
        ),
    );
    let both = async { tokio::join!(first, second) };
    tokio::pin!(both);
    for _ in 0..2 {
        tokio::select! {result=&mut both=>panic!("callbacks ended before claim barrier: {}",usize::from(result.0.is_ok())+usize::from(result.1.is_ok())),
        value=arrivals.recv()=>assert!(value.is_some()),
        _=tokio::time::sleep(std::time::Duration::from_secs(15))=>panic!("claim barrier timed out")}
    }
    let holds: Vec<UsageMeterRow> =
        c.f.state
            .db
            .collection("usage_meter")
            .find(doc! {"user_service_id":&c.connection,"status":"reserved"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(holds.len(), 2);
    assert!(
        holds
            .iter()
            .all(|row| row.reserved_credits > Credits::ZERO && !row.released && !row.forwarded)
    );
    release.add_permits(2);
    let (a, b) = both.await;
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let response = a.or(b).unwrap();
    assert_eq!(response.headers()["content-type"], "application/json");
    let mut source = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = source.try_next().await.unwrap() {
        bytes.extend_from_slice(&chunk);
        if serde_json::from_slice::<Value>(&bytes).is_ok() {
            break;
        }
    }
    peer.terminal.notified().await;
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
                result_char_limit: 10000,
                refusal_source: None,
            }
        )
        .await
        .is_err()
    );
    peer.eof.add_permits(1);
    while let Some(chunk) = source.try_next().await.unwrap() {
        bytes.extend_from_slice(&chunk);
    }
    let canonical: Value = serde_json::from_slice(&bytes).unwrap();
    let wire::PrepareReply::Completed { receipt } = current(&c, &step).await else {
        panic!("JSON EOF commits compact")
    };
    let (_, private) = persistence::load(&c.f.state, &step.root_id, None)
        .await
        .unwrap();
    let StepPhase::Completed { completion, .. } = &private.steps.last().unwrap().phase else {
        panic!("completed")
    };
    assert!(
        completion.calls.is_empty(),
        "retained history never declares new tools"
    );
    assert!(private.steps.last().unwrap().tools.is_empty());
    let observed = completion.compaction.as_ref().unwrap();
    assert_eq!(observed.response, canonical);
    assert_eq!(observed.body_sha256, hex::encode(Sha256::digest(&bytes)));
    assert_eq!(
        receipt.output_binding,
        digest(json!({"response":canonical,"body_sha256":observed.body_sha256}))
    );
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
                result_char_limit: 10000,
                refusal_source: None,
            }
        )
        .await
        .is_err()
    );
    let mut next = request(canonical["output"].as_array().unwrap().clone());
    next.preceding_receipt = Some(receipt.receipt_bearer.clone());
    for change in [
        "pruned",
        "ciphertext",
        "reordered",
        "arguments",
        "output",
        "added_user",
        "config",
        "old_tool_receipt",
    ] {
        let mut changed = copy_request(&next);
        match change {
            "pruned" => {
                changed.body["input"].as_array_mut().unwrap().remove(0);
            }
            "ciphertext" => {
                changed.body["input"][3]["encrypted_content"] = json!("changed opaque state")
            }
            "reordered" => changed.body["input"].as_array_mut().unwrap().swap(0, 1),
            "arguments" => changed.body["input"][0]["arguments"] = json!("{}"),
            "output" => changed.body["input"][1]["output"] = json!("forged host result"),
            "added_user" => changed.body["input"]
                .as_array_mut()
                .unwrap()
                .push(user("new unadmitted input")),
            "config" => changed.body["parallel_tool_calls"] = json!(false),
            _ => changed.tool_results = candidate.tool_results.clone(),
        }
        assert!(
            prepare(&c.f.state, &c.f.chat, &c.token, changed)
                .await
                .is_err(),
            "{change}"
        );
    }
    let next_body = next.body.clone();
    let wire::PrepareReply::Prepared { step: next_step } =
        prepare(&c.f.state, &c.f.chat, &c.token, next)
            .await
            .unwrap()
    else {
        panic!("whole canonical output must advance")
    };
    assert_eq!(next_step.ordinal, 2);
    assert_eq!(next_step.root_id, step.root_id);
    let response = callback(&c, &next_body, Some(&next_step)).await.unwrap();
    to_bytes(response.into_body(), wire::MAX_STREAM_BYTES)
        .await
        .unwrap();
    assert!(
        callback_path(&c, &candidate.body, Some(&step), true, "responses/compact")
            .await
            .is_err()
    );
    assert!(callback(&c, &next_body, Some(&next_step)).await.is_err());
    assert_eq!(peer.calls.load(Ordering::SeqCst), 3);
    let bodies = peer.bodies.lock().await;
    assert_eq!(
        bodies[1],
        ("responses/compact".into(), candidate.body.clone())
    );
    assert_eq!(bodies[2].1["input"], canonical["output"]);
    assert_eq!(bodies[0].1["tools"], bodies[2].1["tools"]);
    assert_eq!(
        bodies[0].1["client_metadata"],
        bodies[2].1["client_metadata"]
    );
    drop(bodies);
    for headers in peer.headers.lock().await.iter() {
        assert!(!headers.contains_key(wire::CONTROL_HEADER));
        assert!(!headers.contains_key(wire::STEP_HEADER));
        assert!(!headers.contains_key(wire::TOOL_HEADER));
        assert_eq!(headers["authorization"], "Bearer provider-fixture-key");
    }
    let charged = crate::billing_integration_tests::wait_for_charged_usage(
        &c.f.state.db,
        doc! {"user_service_id":&c.connection,"status":"finalized"},
        3,
    )
    .await;
    assert!(charged.iter().all(|r| r.billing_owner_id == c.f.owner
        && r.user_service_id.as_deref() == Some(c.connection.as_str())
        && r.service_id.as_deref() == Some(c.target.service.id.as_str())
        && r.api_key_id.as_deref() == Some(c.f.chat.api_key_id.as_str())
        && r.credential_class == CredentialClass::UserOwned));
    let compact_usage: Vec<_> = charged
        .iter()
        .filter(|row| {
            row.token_breakdown
                .as_ref()
                .is_some_and(|usage| usage.prompt_tokens == 100 && usage.completion_tokens == 20)
        })
        .collect();
    assert_eq!(
        compact_usage.len(),
        1,
        "actual compact JSON usage survives the unchanged EOF billing capture"
    );
    assert_eq!(compact_usage[0].quantity, Some(1));
    let loser = c
        .f
        .state
        .db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"user_service_id":&c.connection,"last_error":"assistant_authority_changed"})
        .await
        .unwrap()
        .unwrap();
    assert!(loser.released && !loser.forwarded);
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
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    server.abort();
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn real_compact_truncation_drop_sse_redirect_lost_output_and_retry_never_rearm_the_same_step()
{
    for mode in [
        "compact_truncated",
        "compact_drop",
        "compact_sse",
        "compact_redirect",
        "compact_lost_output",
    ] {
        let (mut c, peer, server) = fixture(mode).await;
        let (_, _, candidate) = completed_generation(&mut c, &peer).await;
        let wire::PrepareReply::Prepared { step } =
            prepare(&c.f.state, &c.f.chat, &c.token, copy_request(&candidate))
                .await
                .unwrap()
        else {
            panic!("compact prepared")
        };
        let response = callback_path(&c, &candidate.body, Some(&step), true, "responses/compact")
            .await
            .unwrap();
        if mode == "compact_drop" {
            let mut body = response.into_body().into_data_stream();
            assert!(body.try_next().await.unwrap().is_some());
            drop(body);
        } else {
            peer.eof.add_permits(1);
            let _ = to_bytes(response.into_body(), wire::MAX_STREAM_BYTES).await;
        }
        let status = current(&c, &step).await;
        if mode == "compact_lost_output" {
            assert!(matches!(status, wire::PrepareReply::Completed { .. }));
        } else {
            assert!(matches!(
                status,
                wire::PrepareReply::InProgress { .. } | wire::PrepareReply::OutcomeUnknown
            ));
        }
        assert!(
            callback_path(&c, &candidate.body, Some(&step), true, "responses/compact")
                .await
                .is_err()
        );
        let failed = c.f.chat.turn_id.clone().unwrap();
        settle_failed(&c, "invalid_stream").await;
        c.f.row = engine::begin_turn(
            &c.f.state.db,
            &c.f.owner,
            &continuation(&c, None, Some(&failed)),
            &c.f.state.encryption_keys,
        )
        .await
        .unwrap();
        refresh_control(&mut c).await;
        let recovered = prepare(&c.f.state, &c.f.chat, &c.token, copy_request(&candidate))
            .await
            .unwrap();
        if mode == "compact_lost_output" {
            assert!(matches!(recovered, wire::PrepareReply::Completed { .. }));
        } else {
            assert!(matches!(
                recovered,
                wire::PrepareReply::InProgress { .. } | wire::PrepareReply::OutcomeUnknown
            ));
        }
        assert!(
            callback_path(&c, &candidate.body, Some(&step), true, "responses/compact")
                .await
                .is_err()
        );
        assert_eq!(peer.calls.load(Ordering::SeqCst), 2, "{mode}");
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
async fn real_compact_final_claim_stop_revoke_expiry_or_connection_drift_releases_hold_without_send()
 {
    for fence in ["stop", "revoke", "expiry", "connection"] {
        let (mut c, peer, server) = fixture("compact_completed").await;
        let (_, _, candidate) = completed_generation(&mut c, &peer).await;
        let wire::PrepareReply::Prepared { step } =
            prepare(&c.f.state, &c.f.chat, &c.token, copy_request(&candidate))
                .await
                .unwrap()
        else {
            panic!("compact prepared")
        };
        let (entered, mut arrivals) = tokio::sync::mpsc::channel(1);
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let dispatch = BEFORE_FINAL_CLAIM.scope(
            (entered, release.clone()),
            callback_path(&c, &candidate.body, Some(&step), true, "responses/compact"),
        );
        tokio::pin!(dispatch);
        tokio::select! {result=&mut dispatch=>panic!("callback ended before compact claim: {}",result.is_ok()),
        arrival=arrivals.recv()=>assert!(arrival.is_some()),
        _=tokio::time::sleep(std::time::Duration::from_secs(15))=>panic!("claim barrier timed out")}
        let held =
            c.f.state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {"user_service_id":&c.connection,"status":"reserved"})
                .await
                .unwrap()
                .unwrap();
        assert!(held.reserved_credits > Credits::ZERO && !held.released && !held.forwarded);
        match fence {
            "stop" => {
                engine::request_stop(&c.f.state.db, &c.f.owner, &c.f.row.id)
                    .await
                    .unwrap();
            }
            "revoke" => {
                c.f.state
                    .db
                    .collection::<Document>("api_keys")
                    .update_one(
                        doc! {"_id":&c.f.chat.api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "expiry" => {
                c.f.state.db.collection::<Document>(COLLECTION_NAME).update_one(
                doc!{"_id":&step.root_id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
            }
            _ => {
                c.f.state
                    .db
                    .collection::<Document>("user_services")
                    .update_one(
                        doc! {"_id":&c.connection},
                        doc! {"$set":{"auth_method":"none"}},
                    )
                    .await
                    .unwrap();
            }
        }
        release.add_permits(1);
        assert!(dispatch.await.is_err(), "{fence}");
        assert_eq!(
            peer.calls.load(Ordering::SeqCst),
            1,
            "only preceding generation sent: {fence}"
        );
        let loser =
            c.f.state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {"_id":held.id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(loser.status, UsageStatus::Failed);
        assert!(loser.released && !loser.forwarded);
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
        let charged = crate::billing_integration_tests::wait_for_charged_usage(
            &c.f.state.db,
            doc! {"user_service_id":&c.connection,"status":"finalized"},
            1,
        )
        .await;
        assert_eq!(charged.len(), 1);
        assert_eq!(charged[0].billing_owner_id, c.f.owner);
        assert_eq!(
            charged[0].api_key_id.as_deref(),
            Some(c.f.chat.api_key_id.as_str())
        );
        assert!(
            callback_path(&c, &candidate.body, Some(&step), true, "responses/compact")
                .await
                .is_err()
        );
        assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
        server.abort();
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn real_compact_first_exact_card_deny_settles_exclusive_notice_without_model_or_provider() {
    let (c, peer, server) = fixture("compact_completed").await;
    let body = json!({"model":"model-v1","input":[user("initial")],"store":false});
    let response = callback_path(&c, &body, None, true, "responses/compact")
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let refusal: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    let card = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    let saved =
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {"_id":&card})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        saved.get_str("tool_name").unwrap(),
        "nyxid.service_execution"
    );
    let binding =
        c.f.state
            .db
            .collection::<Document>(crate::services::assistant_service_execution::COLLECTION)
            .find_one(doc! {"_id":&card,"kind":"card_binding"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        binding
            .get_document("operation")
            .unwrap()
            .get_str("kind")
            .unwrap(),
        "legacy_inference"
    );
    let root =
        c.f.state
            .db
            .collection::<ModelControlRoot>(COLLECTION_NAME)
            .find_one(doc! {"user_id":&c.f.owner,"conversation_id":&c.f.row.id})
            .await
            .unwrap()
            .unwrap();
    let private = persistence::open(&c.f.state, &root).await.unwrap();
    assert_eq!(root.phase, "unactivated");
    assert!(private.envelope.is_none() && private.steps.is_empty());
    assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
    super::super::denial::assert_model_deny_settlement(&c, card).await;
    assert_eq!(peer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {"user_service_id":&c.connection,"forwarded":true})
            .await
            .unwrap(),
        0
    );
    server.abort();
    c.f.state.db.drop().await.unwrap();
}
