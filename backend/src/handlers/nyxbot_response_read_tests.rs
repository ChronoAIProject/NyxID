use super::*;
use crate::services::nyxbot_response_read as read;
use http_body_util::BodyExt;

async fn advertised(f: &Fixture) {
    relay_responses::clear_capabilities(&f.gateway.uri());
    Mock::given(method("GET"))
        .and(path("/channel-capabilities"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}}),
            ),
        )
        .mount(&f.gateway)
        .await;
}
fn metadata(body: &mut Value, event: &str) {
    body["metadata"] = json!({"cmaeg.channel":&sha256_hex("cmaeg1.ch.test")[..16],
        "cmaeg.event":event,"cmaeg.partition":"0123456789abcdef","cmaeg.actor":"human"});
}
fn frames(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect()
}
async fn first(body: &mut axum::body::Body) -> String {
    String::from_utf8(
        body.frame()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

// Ported from CMA develop@0af4dcb3947097b5d36337963d51e2b94d7d0804:
// providers/frozen.rs::submit_frozen and recovery.rs::RecoveryIdentity::snapshot.
fn submit_frozen_fixture(
    status: StatusCode,
    headers: &HeaderMap,
    prefix: &str,
    metadata: &Value,
) -> Value {
    assert_eq!(status, StatusCode::OK);
    assert!(prefix.len() <= 1024 * 1024);
    assert_eq!(headers.get_all("x-cma-response-id").iter().count(), 1);
    assert_eq!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim(),
        "text/event-stream"
    );
    assert!(prefix.lines().any(|line| line == "id: 0"));
    assert!(prefix.lines().any(|line| line == "event: response.created"));
    let frame = frames(prefix).remove(0);
    assert_eq!(frame["type"], "response.created");
    assert_eq!(frame["sequence_number"].as_u64(), Some(0));
    let response = &frame["response"];
    assert_eq!(response["status"], "queued");
    assert_eq!(
        response["id"],
        headers["x-cma-response-id"].to_str().unwrap()
    );
    assert_eq!(response["object"], "response");
    assert!(response["output"].is_array());
    let id = response["conversation"]["id"].as_str().unwrap();
    assert!(!id.is_empty() && id.len() <= 512 && id != "." && id != "..");
    assert!(!id.bytes().any(|b| b.is_ascii_control()));
    assert_eq!(&response["metadata"], metadata);
    response.clone()
}
async fn snapshot(f: &Fixture, id: &str) -> Value {
    let response =
        native_responses::snapshot(State(f.state.clone()), Path(id.into()), bearer(&f.key)).await;
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_str(&body_text(response).await).unwrap()
}
async fn terminal(f: &Fixture, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let value = snapshot(f, id).await;
            if value["status"] == "completed" {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn response_read_2588_live_consumer_preserves_text_marker_and_completion() {
    let f = Box::pin(Fixture::with_output(
        "response_read_live",
        false,
        pretool_output(GATEWAY_LINK_REPLY),
    ))
    .await;
    advertised(&f).await;
    let (mut request, event) = f
        .input("live answer", ThreadAddress::Mention, false, true)
        .await;
    metadata(&mut request, &event);
    let response = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = response.into_body();
    let prefix = first(&mut body).await;
    let accepted = submit_frozen_fixture(status, &headers, &prefix, &request["metadata"]);
    assert_eq!(accepted["thread_reply"], true);
    let rest = String::from_utf8(body.collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert_gateway_link_frames(&frames(&rest));
    let id = accepted["id"].as_str().unwrap();
    let completed = terminal(&f, id).await;
    assert_eq!(completed["output"], json!([]));
    assert_eq!(completed["conversation"], accepted["conversation"]);
    assert_eq!(completed["metadata"], accepted["metadata"]);
    let raw: bson::Document = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"channel_id":&f.row.id,"event_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert!(!raw.contains_key("gateway_response_id"));
    assert!(!raw.contains_key("gateway_replay_claimed"));
    assert_eq!(
        raw.get_document("delivery")
            .unwrap()
            .get_str("state")
            .unwrap(),
        "streamed"
    );
    assert!(!raw.to_string().contains("booking.example"));
}

#[tokio::test]
async fn response_read_2588_replica_acceptance_lifecycle_and_fifo_receipt() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = Box::pin(Fixture::with_output(
        "response_read_replica",
        false,
        pretool_output(GATEWAY_LINK_REPLY),
    ))
    .await;
    advertised(&f).await;
    let parent_completed = Arc::new(AtomicBool::new(false));
    Mock::given(method("POST")).and(path("/events/cmaeg1.ev.sealed-reference/reply-target"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.read", "expires_at":(Utc::now()+ChronoDuration::hours(23)).timestamp()})))
        .mount(&f.gateway).await;
    let gate = parent_completed.clone();
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.read/messages"))
        .respond_with(move |_: &wiremock::Request| {
            if gate.load(Ordering::SeqCst) {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"delivery":{"outcome":"confirmed"}}))
            } else {
                ResponseTemplate::new(409)
                    .set_body_json(json!({"detail":"routed_reply_in_progress"}))
            }
        })
        .mount(&f.gateway)
        .await;
    let (mut request, event) = f
        .input("replica answer", ThreadAddress::Mention, false, true)
        .await;
    metadata(&mut request, &event);
    let blocked = f.calls.lock().await;
    let response = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = response.into_body();
    let accepted = submit_frozen_fixture(
        status,
        &headers,
        &first(&mut body).await,
        &request["metadata"],
    );
    let id = accepted["id"].as_str().unwrap();
    drop(body);
    assert_eq!(snapshot(&f, id).await["status"], "in_progress");
    let running = native_responses::events(
        State(f.state.clone()),
        Path(id.into()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: None,
        }),
        bearer(&f.key),
    )
    .await;
    assert_eq!(running.status(), StatusCode::OK);
    let mut body = running.into_body();
    let created = first(&mut body).await;
    let progress = first(&mut body).await;
    assert_eq!(frames(&created)[0]["sequence_number"], 0);
    assert_eq!(
        frames(&created)[0]["response"]["thread_reply"],
        accepted["thread_reply"]
    );
    assert_eq!(frames(&progress)[0]["sequence_number"], 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(80), body.frame())
            .await
            .is_err()
    );
    drop(body);
    let repeated = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    assert_eq!(repeated.status(), StatusCode::OK);
    let mut repeated = repeated.into_body();
    assert_eq!(
        frames(&first(&mut repeated).await)[0]["response"]["metadata"],
        request["metadata"]
    );
    drop(repeated);
    drop(blocked);
    let completed = terminal(&f, id).await;
    assert_eq!(completed["output"], json!([]));
    assert!(!completed.to_string().contains("booking.example"));
    assert_eq!(completed["conversation"], accepted["conversation"]);
    let saved: NyxbotEvent = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"channel_id":&f.row.id,"event_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !relay_responses::claim_event_replay(&f.state.db, &f.row, &saved.id)
            .await
            .unwrap()
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            late_delivery::process(&f.state, &saved.id).await;
            let row: NyxbotEvent = f
                .state
                .db
                .collection(EVENTS)
                .find_one(doc! {"_id":&saved.id})
                .await
                .unwrap()
                .unwrap();
            if row.delivery.as_ref().unwrap().state == "unknown" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let sends = |requests: Vec<wiremock::Request>| {
        requests
            .into_iter()
            .filter(|r| r.url.path().ends_with("/messages"))
            .collect::<Vec<_>>()
    };
    let first_send = sends(f.gateway.received_requests().await.unwrap()).remove(0);
    let message = first_send.body_json::<Value>().unwrap();
    assert_eq!(message["thread_reply"], true);
    assert_eq!(message["text"], GATEWAY_LINK_REPLY);
    // The mock gateway closes only a parent accepted by submit_frozen whose
    // provider read is terminal. Its automatic replay has no chat effect.
    parent_completed.store(
        snapshot(&f, id).await["status"] == "completed",
        Ordering::SeqCst,
    );
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&saved.id},
            doc! {"$set":{"delivery.receipt_due_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    let sent: NyxbotEvent = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent.delivery.unwrap().state, "sent");
    let requests = sends(f.gateway.received_requests().await.unwrap());
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].body, first_send.body);
    assert_eq!(
        requests[1].headers.get("idempotency-key"),
        first_send.headers.get("idempotency-key")
    );
    for after in [None, Some("0"), Some("1"), Some("2")] {
        let response = native_responses::events(
            State(f.state.clone()),
            Path(id.into()),
            axum::extract::Query(native_responses::Cursor {
                starting_after: after.map(str::to_owned),
            }),
            bearer(&f.key),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        let actual = frames(&body);
        let start = after.map_or(0, |n| n.parse::<usize>().unwrap() + 1);
        assert_eq!(actual.len(), 3 - start);
        if start < 3 {
            assert_eq!(actual.last().unwrap()["response"], completed);
        }
        assert!(!body.contains("booking.example"));
    }
    let bad = native_responses::events(
        State(f.state.clone()),
        Path(id.into()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: Some("3".into()),
        }),
        bearer(&f.key),
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let repeat = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    assert_eq!(
        frames(&body_text(repeat).await).last().unwrap()["response"],
        completed
    );
    let live = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    let lookup = dispatch_identity::lookup(
        &f.state.db,
        &f.state.encryption_keys,
        &live,
        &event,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        lookup,
        dispatch_identity::Lookup::Found(dispatch_identity::DispatchState::Finished {
            stored_response: false,
            ..
        })
    ));
}

#[tokio::test]
async fn response_read_2588_no_turn_link_reply_is_recovered_only_by_gateway_parent() {
    for live in [true, false] {
        let f = Box::pin(Fixture::new("response_read_notice")).await;
        advertised(&f).await;
        f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id":&f.row.id},doc! {
            "$set":{"link_code_hash":sha256_hex("test-link-code"),"link_code_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::minutes(5))}
        }).await.unwrap();
        let adapter = thread_follow::TEST_ADAPTERS
            .lock()
            .unwrap()
            .get(&f.row.channel_bot_id)
            .unwrap()
            .clone();
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .insert(f.row.channel_bot_id.clone(), adapter);
        Mock::given(method("POST"))
            .and(path("/open-apis/im/v1/messages/om_root/reply"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"om_notice"}})),
            )
            .expect(0)
            .mount(&f.platform)
            .await;
        let (mut request, event) = f
            .input("test-link-code", ThreadAddress::Mention, false, true)
            .await;
        metadata(&mut request, &event);
        let response = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = response.into_body();
        let accepted = submit_frozen_fixture(
            status,
            &headers,
            &first(&mut body).await,
            &request["metadata"],
        );
        assert_eq!(accepted["thread_reply"], true);
        if live {
            let text =
                String::from_utf8(body.collect().await.unwrap().to_bytes().to_vec()).unwrap();
            assert!(text.contains("Linked."));
            assert_eq!(frames(&text).last().unwrap()["type"], "response.completed");
        } else {
            drop(body);
        }
        let id = accepted["id"].as_str().unwrap();
        let completed = snapshot(&f, id).await;
        assert!(
            completed["output"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Linked.")
        );
        assert_eq!(completed["metadata"], request["metadata"]);
        // Full live consumption/read-ahead never consumes the recovery copy.
        for after in [None, Some("0"), Some("1"), Some("2"), Some("3")] {
            let response = native_responses::events(
                State(f.state.clone()),
                Path(id.into()),
                axum::extract::Query(native_responses::Cursor {
                    starting_after: after.map(str::to_owned),
                }),
                bearer(&f.key),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let frames = frames(&body_text(response).await);
            let start = after.map_or(0, |s| s.parse::<usize>().unwrap() + 1);
            assert_eq!(frames.len(), 4 - start);
            if start == 0 {
                assert_eq!(frames[0]["response"]["thread_reply"], true);
            }
            if start < 4 {
                assert_eq!(frames.last().unwrap()["response"], completed);
            }
        }
        let repeated = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
        assert_eq!(
            frames(&body_text(repeated).await).last().unwrap()["response"],
            completed
        );
        let alias = Box::pin(respond(&f.state, &f.key, &request, "notice-alias")).await;
        let alias = frames(&body_text(alias).await);
        assert_eq!(alias[0]["response"]["thread_reply"], true);
        assert_eq!(alias.last().unwrap()["response"]["output"], json!([]));
        let raw: bson::Document = f
            .state
            .db
            .collection(EVENTS)
            .find_one(doc! {"channel_id":&f.row.id,"event_id":&event})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.to_string().contains("Linked."));
        assert!(raw.contains_key("response_reply"));
        assert!(
            !raw.contains_key("gateway_response_id") && !raw.contains_key("gateway_replay_claimed")
        );
        late_delivery::process(&f.state, raw.get_str("_id").unwrap()).await;
        late_delivery::sweep(&f.state).await.unwrap();
        assert!(f.calls.lock().await.is_empty());
        f.platform.verify().await;
        f.state
            .db
            .collection::<bson::Document>(EVENTS)
            .update_one(
                doc! {"_id":raw.get_str("_id").unwrap()},
                doc! {"$set":{"response_read.expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert_eq!(
            native_responses::snapshot(State(f.state.clone()), Path(id.into()), bearer(&f.key))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        read::expire(&f.state.db).await.unwrap();
        let raw: bson::Document = f
            .state
            .db
            .collection(EVENTS)
            .find_one(doc! {"_id":raw.get_str("_id").unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.contains_key("response_reply") && !raw.contains_key("response_read"));
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&f.row.channel_bot_id);
    }
}

async fn prepared_thread_notice(f: &Fixture) -> (Value, String) {
    let (start, source) = f.input("start", ThreadAddress::Mention, false, true).await;
    assert!(
        f.send(&start, &source)
            .await
            .contains("Here is your answer")
    );
    let child = f.children().await.remove(0);
    settle(&f.state, child.conversation_id.as_deref().unwrap()).await;
    // This owner message reaches service selection after delivery preparation.
    // Its rejected selection rolls back turn admission in the followed child.
    f.input(
        "/service missing-connection\nRead my messages",
        ThreadAddress::NotAddressed,
        false,
        true,
    )
    .await
}

async fn deliver_prepared_notice(f: &Fixture, event: &str, lifecycle: &[Value]) {
    assert_eq!(lifecycle[0]["type"], "response.created");
    assert_eq!(lifecycle[0]["response"]["thread_reply"], true);
    let completed = lifecycle.last().unwrap();
    assert_eq!(completed["type"], "response.completed");
    let text = completed["response"]["output"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.starts_with("connection_unavailable:"), "{text}");
    let item = lifecycle
        .iter()
        .find(|f| f["type"] == "response.output_item.done")
        .unwrap();
    assert_eq!(item["item"], completed["response"]["output"][0]);

    let row: NyxbotEvent = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"channel_id":&f.row.id,"event_id":event})
        .await
        .unwrap()
        .unwrap();
    let binding = row
        .delivery
        .as_ref()
        .unwrap()
        .origin
        .thread
        .as_ref()
        .unwrap();
    assert_eq!(binding.source_message_id, event);
    assert_eq!(row.resolved_thread_id.as_ref(), Some(&binding.child_id));
    assert!(row.resolved_conversation_id.is_some());
    assert!(row.conversation_id.is_none() && row.turn_id.is_none());
    assert_eq!(row.status, "refused");
    assert!(row.event_context_ciphertext.is_none());

    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages/om_root/reply"))
        .and(body_partial_json(
            json!({"reply_in_thread":true, "content":json!({"text":text}).to_string()}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"message_id":"om_notice"}})),
        )
        .expect(1)
        .mount(&f.platform)
        .await;
    let (headers, _) = thread_reply_headers(&f.state, &f.row, event);
    let sent = Box::pin(post_thread_body(
        &f.state,
        &f.row,
        event,
        &headers,
        json!({"text":text}),
        false,
    ))
    .await;
    assert_eq!(sent.status(), StatusCode::OK);
    let receipt: Value = serde_json::from_str(&body_text(sent).await).unwrap();
    assert_eq!(receipt["platform_message_id"], "om_notice");
    f.platform.verify().await;
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&f.row.channel_bot_id);
}

#[tokio::test]
async fn response_read_2588_live_prepared_no_turn_notice_reaches_original_thread() {
    for negotiated in [false, true] {
        let f = Box::pin(Fixture::new("prepared_notice_live")).await;
        let (mut request, event) = Box::pin(prepared_thread_notice(&f)).await;
        if negotiated {
            advertised(&f).await;
            metadata(&mut request, &event);
        }
        let original = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
        assert_eq!(original.status(), StatusCode::OK);
        assert_eq!(
            original.headers().contains_key("x-cma-response-id"),
            negotiated
        );
        Box::pin(deliver_prepared_notice(
            &f,
            &event,
            &frames(&body_text(original).await),
        ))
        .await;
    }
}

#[tokio::test]
async fn response_read_2588_replica_prepared_no_turn_notice_reaches_original_thread() {
    let f = Box::pin(Fixture::new("prepared_notice_replica")).await;
    advertised(&f).await;
    let (mut request, event) = Box::pin(prepared_thread_notice(&f)).await;
    metadata(&mut request, &event);
    let original = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    let (parts, mut body) = original.into_parts();
    let accepted = submit_frozen_fixture(
        parts.status,
        &parts.headers,
        &first(&mut body).await,
        &request["metadata"],
    );
    drop(body);
    let id = accepted["id"].as_str().unwrap();
    let completed = snapshot(&f, id).await;
    assert_eq!(completed["status"], "completed");
    let replay = native_responses::events(
        State(f.state.clone()),
        Path(id.into()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: None,
        }),
        bearer(&f.key),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let lifecycle = frames(&body_text(replay).await);
    assert_eq!(lifecycle.last().unwrap()["response"], completed);
    Box::pin(deliver_prepared_notice(&f, &event, &lifecycle)).await;
}

#[tokio::test]
async fn response_read_2588_aliases_expiry_legacy_metadata_and_absence() {
    let f = Box::pin(Fixture::new("response_read_boundaries")).await;
    advertised(&f).await;
    let (mut request, event) = f.input("answer", ThreadAddress::Mention, false, true).await;
    request["metadata"] = json!({"cmaeg.actor":"human","cmaeg.event":event,"ignored":"never echo"});
    let response = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-cma-response-id").is_none());
    let body = body_text(response).await;
    assert!(!body.contains("id: 0\n"));
    assert_eq!(
        frames(&body)[0],
        json!({"type":"response.created","response":{"id":dispatch_identity::relay_response(&f.row.id,&event),"status":"in_progress","thread_reply":true}})
    );
    assert_eq!(
        body.split_once("\n\n").unwrap().0,
        format!(
            "event: response.created\ndata: {}",
            json!({"type":"response.created","response":{"id":dispatch_identity::relay_response(&f.row.id,&event),"status":"in_progress","thread_reply":true}})
        )
    );
    assert_eq!(
        f.gateway
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/channel-capabilities")
            .count(),
        0
    );
    let response_id = dispatch_identity::relay_response(&f.row.id, &event);
    let completed = terminal(&f, &response_id).await;
    assert_eq!(
        completed["metadata"],
        json!({"cmaeg.actor":"human","cmaeg.event":event})
    );
    let live = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    let raw: bson::Document = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"channel_id":&f.row.id,"event_id":&event})
        .await
        .unwrap()
        .unwrap();
    let saved = read::mapping(&raw).unwrap().unwrap();
    let alias = "another-provider-command";
    let repeat = Box::pin(respond(&f.state, &f.key, &request, alias)).await;
    assert_eq!(repeat.status(), StatusCode::OK);
    let replay = frames(&body_text(repeat).await);
    assert_eq!(
        replay.last().unwrap()["response"]["id"],
        dispatch_identity::relay_response(&f.row.id, alias)
    );
    assert_eq!(
        replay.last().unwrap()["response"]["conversation"],
        completed["conversation"]
    );
    let mut changed = request.clone();
    changed["metadata"]["cmaeg.actor"] = json!("someone-else");
    let response = Box::pin(respond(&f.state, &f.key, &changed, &event)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        body_text(response)
            .await
            .contains("response_command_changed")
    );
    let raw: bson::Document = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":raw.get_str("_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    let mapped = read::mapping(&raw).unwrap().unwrap();
    assert_eq!(mapped.expires_at, saved.expires_at);
    assert_eq!(mapped.commands.len(), 2);
    let mut wrong = live.clone();
    wrong.binding_id = Some("another-binding".into());
    assert!(
        read::load(&f.state.db, &wrong, &response_id)
            .await
            .unwrap()
            .is_none()
    );
    // A seven-day media event still has only a 24-hour response identity.
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":raw.get_str("_id").unwrap()},
            doc! {"$set":{
                "response_read.expires_at":bson::DateTime::from_millis(0),
                "expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7))
            }},
        )
        .await
        .unwrap();
    assert_eq!(
        native_responses::snapshot(
            State(f.state.clone()),
            Path(response_id.clone()),
            bearer(&f.key)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    read::expire(&f.state.db).await.unwrap();
    assert!(
        !relay_responses::claim_event_replay(&f.state.db, &live, raw.get_str("_id").unwrap())
            .await
            .unwrap()
    );
    // A pre-upgrade admission cannot be retrofitted by a duplicate or a GET.
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":raw.get_str("_id").unwrap()},
            doc! {"$unset":{"response_read_admitted":""}},
        )
        .await
        .unwrap();
    let duplicate = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    assert_eq!(duplicate.status(), StatusCode::OK);
    let _ = body_text(duplicate).await;
    assert!(
        read::load(&f.state.db, &live, &response_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        native_responses::snapshot(
            State(f.state.clone()),
            Path("resp_never_admitted".into()),
            bearer(&f.key)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn response_read_2588_no_storage_scope_and_advertisement_cost() {
    for mode in [
        "flag_off",
        "readiness",
        "negative",
        "unavailable",
        "no_binding",
    ] {
        let f = Box::pin(Fixture::new("response_read_scope")).await;
        relay_responses::clear_capabilities(&f.gateway.uri());
        f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id":&f.row.id},doc! {
            "$set":{"gateway_threads.enabled":false,"link_code_hash":sha256_hex("scope-link"),
                "link_code_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::minutes(5))}
        }).await.unwrap();
        if mode == "no_binding" {
            f.state
                .db
                .collection::<bson::Document>(CHANNELS)
                .update_one(doc! {"_id":&f.row.id}, doc! {"$unset":{"binding_id":""}})
                .await
                .unwrap();
        }
        if mode == "readiness" {
            enable_relay_response_recovery(&f.state).await;
        }
        let enabled = matches!(mode, "flag_off" | "readiness");
        Mock::given(method("GET"))
            .and(path("/channel-capabilities"))
            .respond_with(
                ResponseTemplate::new(if mode == "unavailable" { 503 } else { 200 }).set_body_json(
                    if enabled {
                        json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}})
                    } else {
                        json!({})
                    },
                ),
            )
            .expect(if mode == "no_binding" { 0 } else { 1 })
            .mount(&f.gateway)
            .await;
        let (mut request, event) = f
            .input("scope-link", ThreadAddress::Mention, false, true)
            .await;
        metadata(&mut request, &event);
        let response = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
        assert_eq!(response.status(), StatusCode::OK, "{mode}");
        assert_eq!(
            response.headers().contains_key("x-cma-response-id"),
            enabled,
            "{mode}"
        );
        assert!(body_text(response).await.contains("Linked."));
        let id = dispatch_identity::relay_response(&f.row.id, &event);
        let snapshot = snapshot(&f, &id).await;
        assert_eq!(snapshot["status"], "completed");
        if enabled {
            assert!(
                snapshot["output"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("Linked.")
            );
        } else {
            assert_eq!(snapshot["output"], json!([]));
        }
        assert_eq!(snapshot["metadata"], request["metadata"]);
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::nyxbot_gateway_response::COLLECTION_NAME
                )
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(f.calls.lock().await.is_empty());
        f.gateway.verify().await;
    }
}

#[tokio::test]
async fn response_read_2588_failed_followed_turn_stays_empty_and_sends_failure_in_thread() {
    let f = Box::pin(Fixture::new("response_read_failed_follow")).await;
    advertised(&f).await;
    Mock::given(method("POST")).and(path("/events/cmaeg1.ev.sealed-reference/reply-target"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.failure", "expires_at":(Utc::now()+ChronoDuration::hours(23)).timestamp()})))
        .mount(&f.gateway).await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.failure/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .expect(1)
        .mount(&f.gateway)
        .await;
    let (mut request, event) = f
        .input("failed answer", ThreadAddress::Mention, false, true)
        .await;
    metadata(&mut request, &event);
    let response = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = response.into_body();
    let accepted = submit_frozen_fixture(
        status,
        &headers,
        &first(&mut body).await,
        &request["metadata"],
    );
    let id = accepted["id"].as_str().unwrap();
    let _ = terminal(&f, id).await;
    let saved: NyxbotEvent = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"channel_id":&f.row.id,"event_id":&event})
        .await
        .unwrap()
        .unwrap();
    // Exercise the persisted failure branch while the original stream still
    // owns no output; the same turn/source and real follow binding remain.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"turn_id":&saved.turn_id,"role":"assistant"},
            doc! {"$set":{"status":"failed","text":""}},
        )
        .await
        .unwrap();
    let completed = snapshot(&f, id).await;
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["output"], json!([]));
    assert!(completed.get("error").is_none());
    drop(body);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            late_delivery::process(&f.state, &saved.id).await;
            let raw: NyxbotEvent = f
                .state
                .db
                .collection(EVENTS)
                .find_one(doc! {"_id":&saved.id})
                .await
                .unwrap()
                .unwrap();
            if raw.delivery.unwrap().state == "sent" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let requests = f.gateway.received_requests().await.unwrap();
    let sent = requests
        .iter()
        .find(|r| r.url.path().ends_with("/messages"))
        .unwrap()
        .body_json::<Value>()
        .unwrap();
    assert_eq!(
        sent,
        json!({"text":"The previous operation may have taken effect. Check its result before trying again.","thread_reply":true})
    );
    let events = native_responses::events(
        State(f.state.clone()),
        Path(id.into()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: Some("1".into()),
        }),
        bearer(&f.key),
    )
    .await;
    assert_eq!(
        frames(&body_text(events).await).last().unwrap()["response"],
        completed
    );
    let repeat = Box::pin(respond(&f.state, &f.key, &request, &event)).await;
    assert_eq!(
        frames(&body_text(repeat).await).last().unwrap()["response"],
        completed
    );
}

#[path = "nyxbot_org_thread_notice_tests.rs"]
mod org_thread_notice_tests;
