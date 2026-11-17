use super::*;
use crate::services::nyxbot_response_read as read;

async fn map(f: &Fixture) -> String {
    let id = dispatch_identity::relay_response(&f.channel.id, "read-command");
    let command = read::command(
        &id,
        &json!({"metadata":{"cmaeg.channel":"0123456789abcdef","cmaeg.event":f.event.event_id,"cmaeg.partition":"0123456789abcdef","cmaeg.actor":"7"}}),
    );
    let mut mapping = read::identity(&f.channel, &f.event, command);
    mapping.thread_reply = Some(false);
    patch(f,doc! {"relay_message_id":&f.event.event_id,"response_read":bson::to_bson(&mapping).unwrap(),"response_read_admitted":true}).await;
    id
}
async fn observe(f: &Fixture, id: &str) -> Value {
    let response =
        native_responses::snapshot(State(f.state.clone()), Path(id.into()), bearer(&f.key)).await;
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_str(&body_text(response).await).unwrap()
}

#[tokio::test]
async fn response_read_2588_success_and_failure_preserve_local_delivery_in_both_orders() {
    for failed in [false, true] {
        let mut first_body = None;
        for read_first in [false, true] {
            let f = Box::pin(fixture("response_read_delivery")).await;
            let id = map(&f).await;
            if read_first {
                assert_eq!(observe(&f, &id).await["status"], "in_progress");
            }
            settled(&f, failed).await;
            patch(&f, doc! {"delivery.state":"pending"}).await;
            let before: bson::Document = f
                .state
                .db
                .collection(EVENTS)
                .find_one(doc! {"_id":&f.event.id})
                .await
                .unwrap()
                .unwrap();
            if read_first {
                let snapshot = observe(&f, &id).await;
                assert_eq!(snapshot["status"], "completed");
                assert_eq!(snapshot["output"], json!([]));
                assert!(snapshot.get("error").is_none());
                let after: bson::Document = f
                    .state
                    .db
                    .collection(EVENTS)
                    .find_one(doc! {"_id":&f.event.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    before, after,
                    "read has no writes, including delivery metadata"
                );
            }
            assert!(
                !relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id)
                    .await
                    .unwrap()
            );
            late_delivery::process(&f.state, &f.event.id).await;
            assert_eq!(status(&f).await, "sent");
            let request = sends(&f).await.remove(0);
            let body = request.body_json::<Value>().unwrap();
            assert_eq!(
                body["text"],
                if failed {
                    "Useful answer\n\n[Incomplete — I could not finish this turn.]"
                } else {
                    "Useful answer"
                }
            );
            if let Some(first) = &first_body {
                assert_eq!(&request.body, first);
            } else {
                first_body = Some(request.body.clone());
            }
            assert_eq!(observe(&f, &id).await["output"], json!([]));
            late_delivery::process(&f.state, &f.event.id).await;
            assert_eq!(sends(&f).await.len(), 1);
            let events = native_responses::events(
                State(f.state.clone()),
                Path(id),
                axum::extract::Query(native_responses::Cursor {
                    starting_after: Some("1".into()),
                }),
                bearer(&f.key),
            )
            .await;
            let text = body_text(events).await;
            assert!(text.contains("response.completed"));
            assert!(
                !text.contains("Useful answer")
                    && !text.contains("Incomplete")
                    && !text.contains("response.failed")
            );
        }
    }
}

#[tokio::test]
async fn response_read_2588_settlement_projection_does_not_load_text_and_deleted_history_is_absent()
{
    let f = Box::pin(fixture("response_read_projection")).await;
    let id = map(&f).await;
    settled(&f, true).await;
    let created =
        crate::services::nyxbot_response_recovery::Payload::new(&id, "unused", Default::default())
            .frames
            .remove(0);
    assert!(
        response_read::reply(
            f.state.clone(),
            &f.channel,
            f.event.id.clone(),
            id.clone(),
            created,
            "must not become a turn answer".into(),
            false
        )
        .await
        .is_err()
    );
    // Even malformed reply storage must never be opened for a bound turn.
    patch(
        &f,
        doc! {"response_reply":{"version":1,"ciphertext":"invalid"}},
    )
    .await;
    // Deliberately not deserializable as AssistantMessage; projection must read
    // only settlement, never load a body or invoke late delivery during GET.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"turn_id":&f.event.turn_id},
            doc! {"$set":{"text":{"invalid":"body"},"attachments":"invalid"}},
        )
        .await
        .unwrap();
    assert_eq!(observe(&f, &id).await["status"], "completed");
    assert!(sends(&f).await.is_empty());
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .delete_one(doc! {"_id":&f.event.conversation_id})
        .await
        .unwrap();
    assert_eq!(
        native_responses::snapshot(State(f.state.clone()), Path(id), bearer(&f.key))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn response_read_2588_unmapped_parent_cannot_release_fifo() {
    let f = Box::pin(fixture("response_read_no_mapping_control")).await;
    settled(&f, false).await;
    patch(&f, doc! {"delivery.state":"pending"}).await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(json!({"detail":"routed_reply_in_progress"})),
        )
        .with_priority(1)
        .mount(&f.gateway)
        .await;
    late_delivery::process(&f.state, &f.event.id).await;
    let id = dispatch_identity::relay_response(&f.channel.id, "old-command");
    assert_eq!(
        native_responses::snapshot(State(f.state.clone()), Path(id), bearer(&f.key))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    patch(
        &f,
        doc! {"delivery.receipt_due_at":bson::DateTime::from_millis(0)},
    )
    .await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "unknown");
    let requests = sends(&f).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(
        requests[0].headers.get("idempotency-key"),
        requests[1].headers.get("idempotency-key")
    );
}

#[tokio::test]
async fn response_read_2588_alias_during_admission_survives_mapping_promotion() {
    let f = Box::pin(fixture("response_read_admission_alias")).await;
    let mut event = f.event.clone();
    event.id = Uuid::new_v4().to_string();
    event.event_id = Uuid::new_v4().to_string();
    event.turn_id = None;
    event.conversation_id = None;
    event.delivery = None;
    event.status = "running".into();
    event.relay_message_id = Some(event.event_id.clone());
    event.dispatch_identity = Some(dispatch_identity::admission(&f.channel.id, "initial"));
    let original = read::command(
        "resp_initial",
        &json!({"metadata":{"cmaeg.event":event.event_id}}),
    );
    let provisional = read::identity(&f.channel, &event, original.clone());
    crate::services::channel_media_events::insert_response_event(
        &f.state.db,
        &event,
        Some(&provisional),
    )
    .await
    .unwrap();
    let alias = read::command(
        "resp_alias",
        &json!({"metadata":{"cmaeg.event":event.event_id,"cmaeg.actor":"7"}}),
    );
    assert_eq!(
        dispatch_identity::alias_response(
            &f.state.db,
            &event.id,
            &f.channel.id,
            &dispatch_identity::dispatch_key(&f.channel.id, "alias"),
            Some(&alias)
        )
        .await
        .unwrap(),
        dispatch_identity::AliasOutcome::Added
    );
    assert!(
        read::load(&f.state.db, &f.channel, "resp_alias")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !relay_responses::claim_event_replay(&f.state.db, &f.channel, &event.id)
            .await
            .unwrap()
    );
    read::admit(&f.state.db, &f.channel, &event, original)
        .await
        .unwrap()
        .unwrap();
    for id in ["resp_initial", "resp_alias"] {
        let view = read::load(&f.state.db, &f.channel, id)
            .await
            .unwrap()
            .unwrap();
        assert!(!view.terminal);
        assert_eq!(view.payload.snapshot["id"], id);
    }
    let raw: bson::Document = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!raw.contains_key("response_read_pending"));
    assert_eq!(read::mapping(&raw).unwrap().unwrap().commands.len(), 2);
    // An old admission lacks the provisional marker and cannot be enrolled.
    event.id = Uuid::new_v4().to_string();
    event.event_id = Uuid::new_v4().to_string();
    event.relay_message_id = Some(event.event_id.clone());
    event.dispatch_identity = None;
    crate::services::channel_media_events::insert_event(&f.state.db, &event)
        .await
        .unwrap();
    let old = read::command("resp_old", &json!({}));
    assert!(
        read::admit(&f.state.db, &f.channel, &event, old)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn response_read_2588_unaccepted_busy_retry_keeps_admission_and_acceptance() {
    let f = Box::pin(fixture("response_read_busy_retry")).await;
    relay_responses::clear_capabilities(&f.gateway.uri());
    let mut live = f.channel.clone();
    live.binding_id = Some("retry-binding".into());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&live.id}, &live)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/channel-capabilities"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}}),
            ),
        )
        .expect(1)
        .mount(&f.gateway)
        .await;
    let mut request = event("retry after busy", "7", "retry-source");
    request["metadata"] = json!({"cmaeg.channel":&sha256_hex("")[..16],"cmaeg.event":"retry-source","cmaeg.partition":"0123456789abcdef","cmaeg.actor":"7"});
    let key = "retry-command";
    let mut source = admission(&live.id, "retry-source");
    source.id = dispatch_identity::dispatch_key(&live.id, key);
    source.status = "retryable".into();
    source.native_bot_budget_claimed = true;
    source.retry_request_sha256 = Some(sha256_hex(request.to_string()));
    source.dispatch_identity = Some(dispatch_identity::admission(&live.id, key));
    source.relay_message_id = Some("retry-source".into());
    source.event_context_ciphertext = Some(
        f.state
            .encryption_keys
            .encrypt(
                serde_json::to_string(&request["event_context"])
                    .unwrap()
                    .as_bytes(),
            )
            .await
            .unwrap(),
    );
    let id = dispatch_identity::relay_response(&live.id, key);
    let mapping = read::identity(&live, &source, read::command(&id, &request));
    let mut raw = bson::to_document(&source).unwrap();
    raw.insert("response_read", bson::to_bson(&mapping).unwrap());
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .insert_one(raw)
        .await
        .unwrap();
    let view = read::load(&f.state.db, &live, &id).await.unwrap().unwrap();
    assert_eq!(
        native_responses::snapshot(State(f.state.clone()), Path(id.clone()), bearer(&f.key),)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(view.retryable && !view.admitted && !view.terminal);
    assert!(
        !relay_responses::claim_event_replay(&f.state.db, &live, &source.id)
            .await
            .unwrap()
    );
    // The original pre-acceptance retry, rather than a recovery GET, must
    // continue to admit the verified human after capacity returns.
    let response = Box::pin(respond(&f.state, &f.key, &request, key)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-cma-response-id"], id);
    let text = body_text(response).await;
    let prefix: Value = serde_json::from_str(
        text.lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(prefix["sequence_number"], 0);
    assert_eq!(prefix["response"]["status"], "queued");
    assert_eq!(prefix["response"]["metadata"], request["metadata"]);
    assert!(text.contains("Here is your answer"), "{text}");
    let saved: NyxbotEvent = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":&source.id})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.turn_id.is_some());
    assert_eq!(observe(&f, &id).await["status"], "completed");
    let raw: bson::Document = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":&source.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(raw.get_bool("response_read_admitted"), Ok(true));
}

#[tokio::test]
async fn response_read_2588_old_replica_can_store_only_a_previously_unaccepted_retry() {
    let f = Box::pin(fixture("response_read_old_retry")).await;
    let id = map(&f).await;
    patch(&f, doc! {"gateway_response_id":"resp_old_stored"}).await;
    assert!(
        !relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id)
            .await
            .unwrap()
    );
    patch(&f,doc! {"status":"retryable","turn_id":bson::Bson::Null,"conversation_id":bson::Bson::Null,"delivery":bson::Bson::Null}).await;
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&f.event.id},
            doc! {"$unset":{"response_read_admitted":"","gateway_response_id":""}},
        )
        .await
        .unwrap();
    assert!(
        !relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id)
            .await
            .unwrap()
    );
    // This is the existing old prepare transaction's event-side ownership
    // marker. Production calls this claim only after loading its stored row.
    patch(
        &f,
        doc! {"status":"resuming","gateway_response_id":"resp_old_stored"},
    )
    .await;
    assert!(
        read::load(&f.state.db, &f.channel, &id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        relay_responses::claim_event_replay(&f.state.db, &f.channel, &f.event.id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn response_read_2588_abandoned_parent_403_rejects_same_key_reply_without_retry() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = Box::pin(fixture("response_read_abandoned_403")).await;
    let id = map(&f).await;
    settled(&f, false).await;
    patch(&f, doc! {"delivery.state":"pending"}).await;
    let abandoned = Arc::new(AtomicBool::new(false));
    let gate = abandoned.clone();
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(move |_: &wiremock::Request| {
            if gate.load(Ordering::SeqCst) {
                ResponseTemplate::new(403).set_body_json(json!({"detail":"routed_reply_revoked"}))
            } else {
                ResponseTemplate::new(409)
                    .set_body_json(json!({"detail":"routed_reply_in_progress"}))
            }
        })
        .with_priority(1)
        .expect(2)
        .mount(&f.gateway)
        .await;
    late_delivery::process(&f.state, &f.event.id).await;
    assert_eq!(status(&f).await, "unknown");
    assert_eq!(observe(&f, &id).await["output"], json!([]));
    abandoned.store(true, Ordering::SeqCst);
    patch(
        &f,
        doc! {"delivery.receipt_due_at":bson::DateTime::from_millis(0)},
    )
    .await;
    let (result, trace) =
        crate::turn_trace::capture(Box::pin(late_delivery::sweep(&f.state))).await;
    result.unwrap();
    assert_eq!(status(&f).await, "refused");
    assert!(
        trace
            .iter()
            .any(|row| row["stage"] == "channel" && row["outcome"] == "rejected")
    );
    let raw: bson::Document = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":&f.event.id})
        .await
        .unwrap()
        .unwrap();
    let delivery = raw.get_document("delivery").unwrap();
    assert!(!delivery.contains_key("receipt_due_at"));
    assert_eq!(
        delivery
            .get_document("preparation")
            .unwrap()
            .get_document("gateway_receipt")
            .unwrap()
            .get_bool("pending"),
        Ok(false)
    );
    assert_eq!(observe(&f, &id).await["output"], json!([]));
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::process(&f.state, &f.event.id).await;
    let requests = sends(&f).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(
        requests[0].headers.get("idempotency-key"),
        requests[1].headers.get("idempotency-key")
    );
}
