//! CMA 217d66fa36c0 providers/{frozen,recovery}.rs acceptance and readback for
//! the ordinary relay target. Every command keeps its source UUID and evt_ key.
use super::*;
use crate::services::nyxbot_response_recovery as durable;
use http_body_util::BodyExt;
use tower::ServiceExt;

pub(super) async fn enable(f: &Fixture) {
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    enable_relay_response_recovery(&f.state).await;
    durable::ensure_indexes(&f.state.db).await.unwrap();
    f.auth
        .relay_recovery
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let base = crate::services::nyxbot_gateway_service::gateway_base(&f.state)
        .await
        .unwrap();
    relay_responses::clear_capabilities(&base);
}

struct Reader {
    router: Router,
    authorization: axum::http::HeaderValue,
}

impl Reader {
    async fn new(f: &Fixture, row: &NyxbotChannel) -> Self {
        let key = decrypt_agent_key(&f.state, row).await.unwrap().unwrap();
        let mut authorization: axum::http::HeaderValue =
            format!("Bearer {}", key.as_str()).parse().unwrap();
        authorization.set_sensitive(true);
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        Self {
            router: private.with_state(f.state.clone()),
            authorization,
        }
    }

    async fn get(&self, path: &str) -> Response {
        self.router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .header("authorization", self.authorization.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn submit(&self, command: &Value, key: &str) -> Response {
        self.router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/v1/nyxbot/responses")
                    .header("authorization", self.authorization.clone())
                    .header("content-type", "application/json")
                    .header("idempotency-key", key)
                    .body(Body::from(serde_json::to_vec(command).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn snapshot(&self, id: &str) -> Value {
        let response = self.get(&format!("/api/v1/nyxbot/responses/{id}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_str(&body_text(response).await).unwrap()
    }

    async fn context(&self, row: &NyxbotChannel, command: &Value, event_id: &str) {
        let binding = row.binding_id.as_deref().unwrap();
        let response = self
            .get(&format!(
                "/api/v1/nyxbot/bindings/{binding}/conversations/{PARTITION}/events/{event_id}"
            ))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let actual: Value = serde_json::from_str(&body_text(response).await).unwrap();
        // Equality includes the event-specific opaque reference, without ever
        // printing it (including on failure). Reads/replay cannot retarget it.
        assert!(
            actual == command["event_context"],
            "Saved event context changed"
        );
    }
}

fn assert_identity(snapshot: &Value, accepted: &Value, command: &Value) {
    assert_eq!(snapshot["id"], accepted["id"]);
    assert_eq!(snapshot["object"], "response");
    assert_eq!(snapshot["conversation"], accepted["conversation"]);
    assert!(
        snapshot["metadata"] == command["metadata"],
        "Recovery must echo the exact command metadata"
    );
    assert!(snapshot["output"].is_array());
}

async fn first_record(body: &mut Body) -> String {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut prefix = String::new();
        loop {
            let frame = body.frame().await.unwrap().unwrap();
            if let Ok(data) = frame.into_data() {
                prefix.push_str(std::str::from_utf8(&data).unwrap());
                assert!(prefix.len() <= 1024 * 1024);
                if prefix.contains("\n\n") && !frames(&prefix).is_empty() {
                    return prefix;
                }
            }
        }
    })
    .await
    .expect("provider acceptance prefix")
}

pub(super) async fn assert_source_context(
    f: &Fixture,
    row: &NyxbotChannel,
    command: &Value,
    event_id: &str,
) {
    Reader::new(f, row)
        .await
        .context(row, command, event_id)
        .await;
}

pub(super) async fn answer(
    f: &Fixture,
    row: &NyxbotChannel,
    command: &Value,
    event_id: &str,
    expected: &str,
    probe: &ModelProbe,
) {
    // Channel partition policy is conversation_and_sender, not the opaque
    // provider conversation ID. Pin the real private chat/sender values.
    assert_eq!(
        command["metadata"],
        json!({
            "cmaeg.channel":&sha256_hex(row.gateway_channel_id.as_deref().unwrap())[..16],
            "cmaeg.event":event_id,"cmaeg.partition":&sha256_hex("77\u{1f}77")[..16],"cmaeg.actor":"77",
        })
    );
    assert_eq!(command["conversation"], PARTITION);
    assert_eq!(command["event_context"]["activity"]["event_id"], event_id);
    assert!(
        command["event_context"]["event_ref"]
            .as_str()
            .unwrap()
            .len()
            > 512
    );
    let key = format!("evt_{event_id}");
    assert_ne!(key, event_id);
    let reader = Reader::new(f, row).await;
    let response = reader.submit(command, &key).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get_all("x-cma-response-id")
            .iter()
            .count(),
        1
    );
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let mut body = response.into_body();
    let prefix = first_record(&mut body).await;
    let first = frames(&prefix);
    assert_eq!(first.len(), 1);
    assert!(prefix.lines().any(|line| line == "id: 0"));
    assert_eq!(first[0]["type"], "response.created");
    assert_eq!(first[0]["sequence_number"], 0);
    let accepted = &first[0]["response"];
    assert_eq!(accepted["id"], id);
    assert_eq!(accepted["status"], "queued");
    assert_eq!(accepted["output"], json!([]));
    assert!(
        accepted["conversation"]["id"]
            .as_str()
            .is_some_and(|s| s.starts_with("nyxbot_") && s.len() <= 512)
    );
    assert_identity(accepted, accepted, command);
    // Frozen submission acknowledges frame zero and drops the POST observer.
    // The fixture model remains blocked after its real image read, allowing
    // recovery to claim output before any delayed delivery can send an answer.
    drop(body);
    tokio::time::timeout(Duration::from_secs(30), probe.entered.notified())
        .await
        .expect("model reached the exact turn");
    let pending = reader.snapshot(&id).await;
    assert_identity(&pending, accepted, command);
    assert_eq!(pending["status"], "in_progress");
    assert_eq!(pending["output"], json!([]));
    reader.context(row, command, event_id).await;
    let replay = reader
        .get(&format!(
            "/api/v1/nyxbot/responses/{id}/events?starting_after=0"
        ))
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    probe.release.notify_one();
    let tail = tokio::time::timeout(Duration::from_secs(60), body_text(replay))
        .await
        .expect("original turn recovery completes");
    let wire = format!("{prefix}{tail}");
    assert_answer(&wire, expected);
    let lifecycle = frames(&wire);
    assert_eq!(lifecycle.len(), 4);
    assert_eq!(lifecycle[1]["type"], "response.in_progress");
    let ids: Vec<u64> = wire
        .lines()
        .filter_map(|line| line.strip_prefix("id: "))
        .map(|value| value.parse().unwrap())
        .collect();
    assert_eq!(ids, [0, 1, 2, 3]);
    for (sequence, frame) in lifecycle.iter().enumerate() {
        assert_eq!(frame["sequence_number"], sequence);
        if frame.get("response").is_some() {
            assert_identity(&frame["response"], accepted, command);
        }
    }
    let completed = reader.snapshot(&id).await;
    assert_identity(&completed, accepted, command);
    assert_eq!(completed["status"], "completed");
    assert!(
        completed == lifecycle[3]["response"],
        "Snapshot and terminal event must agree"
    );
    let stored = durable::load(&f.state.db, row, &id).await.unwrap().unwrap();
    assert!(stored.terminal);
    assert_eq!(stored.user_id, OWNER);
    assert_eq!(stored.channel_id, row.id);
    assert_eq!(Some(stored.binding_id.as_str()), row.binding_id.as_deref());
    assert_eq!(stored.partition, PARTITION);
    assert_eq!(
        stored.event_key,
        dispatch_identity::dispatch_key(&row.id, &key)
    );
    assert_eq!(stored.conversation_id.as_deref(), Some(f.home.id.as_str()));
    assert_eq!(stored.request_hash, sha256_hex(command.to_string()));
    let saved = durable::payload(&f.state.encryption_keys, &stored)
        .await
        .unwrap();
    assert!(
        saved.snapshot == completed && saved.frames == lifecycle,
        "Stored recovery snapshot and frames must match the protocol readback"
    );
    let event = f
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key,"channel_id":&row.id,"event_id":event_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.conversation_id.as_deref(), Some(f.home.id.as_str()));
    assert_eq!(event.delivery.as_ref().unwrap().state, "replay");
    reader.context(row, command, event_id).await;
    let repeated = reader.submit(command, &key).await;
    assert_eq!(repeated.status(), StatusCode::OK);
    assert!(
        frames(&body_text(repeated).await) == lifecycle,
        "Same-key replay must retain the same response"
    );
    reader.context(row, command, event_id).await;
    let drained = reader
        .get(&format!(
            "/api/v1/nyxbot/responses/{id}/events?starting_after=3"
        ))
        .await;
    assert_eq!(drained.status(), StatusCode::OK);
    assert!(body_text(drained).await.is_empty());
}
