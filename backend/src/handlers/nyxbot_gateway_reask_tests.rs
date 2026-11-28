use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Copy)]
enum ResponseKind {
    Http(u16, &'static str),
    Timeout,
    Reset,
    Oversize,
    Truncated(u16),
    BodyTimeout(u16),
}
const CONFIRMED: ResponseKind = ResponseKind::Http(200, r#"{"delivery":{"outcome":"confirmed"}}"#);
const IN_PROGRESS: ResponseKind =
    ResponseKind::Http(409, r#"{"detail":"routed_reply_in_progress"}"#);

#[derive(Clone, PartialEq, Eq)]
struct RequestIdentity {
    path: String,
    key: String,
    body: Vec<u8>,
}
struct Gateway {
    requests: Arc<Mutex<Vec<RequestIdentity>>>,
    messages: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
    worker: tokio::task::JoinHandle<()>,
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl Gateway {
    fn requests(&self) -> Vec<RequestIdentity> {
        self.requests.lock().unwrap().clone()
    }
    fn assert_one_message(&self, count: usize) {
        let requests = self.requests();
        assert_eq!(requests.len(), count);
        assert!(requests.iter().all(|r| r == &requests[0]));
        assert_eq!(self.messages.lock().unwrap().len(), 1);
    }
}

// The mock takes custody before choosing a response. HTTP retries can read that
// same custody, but only the first identical key/body creates a platform message.
async fn gateway(f: &Fixture, responses: Vec<ResponseKind>) -> Gateway {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    f.state
        .db
        .collection::<bson::Document>(SERVICES)
        .update_one(doc! {"slug":GATEWAY_SLUG}, doc! {"$set":{"base_url":base}})
        .await
        .unwrap();
    let requests = Arc::new(Mutex::new(Vec::<RequestIdentity>::new()));
    let messages = Arc::new(Mutex::new(
        std::collections::HashMap::<String, Vec<u8>>::new(),
    ));
    let observed = requests.clone();
    let delivered = messages.clone();
    let worker = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let observed = observed.clone();
            let delivered = delivered.clone();
            let responses = responses.clone();
            connections.spawn(async move {
                let mut bytes = Vec::new();
                let (head, length) = loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 { return; }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                        let length: usize = head.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                        }).unwrap();
                        break (head, length);
                    }
                };
                let start = head.len() + 4;
                while bytes.len() < start + length {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buf[..n]);
                }
                let request = RequestIdentity {
                    path: head.lines().next().unwrap().split_whitespace().nth(1).unwrap().to_owned(),
                    key: head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("idempotency-key").then(|| value.trim().to_owned())
                    }).unwrap_or_default(),
                    body: bytes[start..start+length].to_vec(),
                };
                if request.path.ends_with("/reply-target") {
                    stream.write_all(b"HTTP/1.1 404 Test\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();
                    return;
                }
                let index = {
                    let mut rows = observed.lock().unwrap();
                    rows.push(request.clone());
                    rows.len() - 1
                };
                delivered.lock().unwrap().entry(request.key).or_insert(request.body);
                let response = responses[index.min(responses.len()-1)];
                let (status, body) = match response {
                    ResponseKind::Oversize => {
                        stream.write_all(b"HTTP/1.1 200 Test\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n{}").await.unwrap();
                        return;
                    }
                    ResponseKind::Reset => {
                        #[allow(deprecated)]
                        stream.set_linger(Some(Duration::ZERO)).unwrap();
                        return;
                    }
                    ResponseKind::Timeout => {
                        tokio::time::sleep(Duration::from_secs(21)).await;
                        (200, r#"{"delivery":{"outcome":"confirmed"}}"#)
                    }
                    ResponseKind::Http(status, body) => (status, body),
                    ResponseKind::Truncated(status) | ResponseKind::BodyTimeout(status) => {
                        stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{").as_bytes()).await.unwrap();
                        if matches!(response, ResponseKind::BodyTimeout(_)) {
                            tokio::time::sleep(Duration::from_secs(21)).await;
                        }
                        return;
                    }
                };
                let _ = stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            });
        }
    });
    Gateway {
        requests,
        messages,
        worker,
    }
}

async fn prepared(name: &str) -> Fixture {
    let f = fixture(name).await;
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    settled(&f, false).await;
    let target = event_record::seal(
        &f.state.encryption_keys,
        &f.event.id,
        &f.event.user_id,
        &[event_record::TARGET, &f.event.channel_id],
        b"cmaeg1.rt.original",
    )
    .await
    .unwrap();
    patch(&f, doc! {
        "delivery.state":"pending",
        "delivery.target_ciphertext":bson::Binary { subtype:bson::spec::BinarySubtype::Generic, bytes:target },
        "delivery.target_context_version": event_record::version(&f.state.encryption_keys).map(i32::from),
        "delivery.target_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(23)),
        "status":"completed","dispatch_identity.version":1,
        "dispatch_identity.keys":[dispatch_identity::dispatch_key(&f.channel.id,"original-post")],
    }).await;
    f
}
async fn wait_row(f: &Fixture) -> bson::Document {
    raw(f)
        .await
        .get_document("delivery")
        .unwrap()
        .get_document("preparation")
        .unwrap()
        .get_document("gateway_receipt")
        .unwrap()
        .clone()
}
async fn health(f: &Fixture) -> crate::services::channel_bot_health::Recommendation {
    let bot = f
        .state
        .db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&f.channel.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    crate::services::channel_bot_health::recommend(
        &crate::services::channel_bot_health::HealthFacts::for_bot(&bot, Utc::now(), true),
    )
}
async fn provider(f: &Fixture) -> Value {
    serde_json::from_str::<Value>(&body_text(lookup(f, "original-post", None).await).await).unwrap()
        ["provider_delivery"]
        .clone()
}
fn faults() -> Vec<(&'static str, ResponseKind, &'static str, Option<i32>)> {
    vec![
        ("timeout", ResponseKind::Timeout, "no_response", None),
        ("reset", ResponseKind::Reset, "no_response", None),
        (
            "500",
            ResponseKind::Http(500, "{}"),
            "server_error",
            Some(500),
        ),
        (
            "502",
            ResponseKind::Http(502, "{}"),
            "server_error",
            Some(502),
        ),
        (
            "504",
            ResponseKind::Http(504, "{}"),
            "server_error",
            Some(504),
        ),
        (
            "503",
            ResponseKind::Http(503, "{}"),
            "server_error",
            Some(503),
        ),
        (
            "503_other",
            ResponseKind::Http(503, r#"{"detail":"different"}"#),
            "server_error",
            Some(503),
        ),
        (
            "unparseable",
            ResponseKind::Http(200, "not json"),
            "unreadable_response",
            Some(200),
        ),
        (
            "oversize",
            ResponseKind::Oversize,
            "unreadable_response",
            Some(200),
        ),
        (
            "truncated",
            ResponseKind::Truncated(200),
            "unreadable_response",
            Some(200),
        ),
        (
            "body_timeout",
            ResponseKind::BodyTimeout(200),
            "unreadable_response",
            Some(200),
        ),
        (
            "204",
            ResponseKind::Http(204, ""),
            "unreadable_response",
            Some(204),
        ),
    ]
}

#[tokio::test]
async fn gateway_reask_transport_matrix_confirms_one_identical_platform_message() {
    Box::pin(async {
        for (name, fault, failure, http_status) in faults() {
            let f = prepared(&format!("reask_confirm_{name}")).await;
            let server = gateway(&f, vec![fault, CONFIRMED]).await;
            late_delivery::process(&f.state, &f.event.id).await;
            let wait = wait_row(&f).await;
            assert_eq!(wait.get_i32("version"), Ok(2));
            assert_eq!(wait.get_bool("pending"), Ok(true));
            let observation = wait.get_document("observation").unwrap();
            assert_eq!(observation.get_str("failure"), Ok(failure));
            assert_eq!(observation.get_i32("status").ok(), http_status);
            assert!(outcomes(&f).await.is_empty());
            assert_eq!(provider(&f).await, "started");
            let advice = health(&f).await;
            assert_eq!(advice.action, "gateway");
            assert!(advice.text.contains("still trying"));
            assert!(!advice.text.contains("accepted"));
            due(&f).await;
            let (a, b) = tokio::join!(
                late_delivery::sweep(&f.state),
                late_delivery::sweep(&f.state)
            );
            a.unwrap();
            b.unwrap();
            assert_eq!(status(&f).await, "sent");
            assert_eq!(provider(&f).await, "confirmed");
            server.assert_one_message(2);
            let audits = outcomes(&f).await;
            assert_eq!(audits.len(), 1);
            assert!(!audits[0].to_string().contains("Useful answer"));
            assert!(!audits[0].to_string().contains("cmaeg1.rt"));
        }
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_transport_matrix_expires_with_truthful_health_and_lookup() {
    Box::pin(async {
        for (name, fault, failure, _) in faults() {
            let f = prepared(&format!("reask_expire_{name}")).await;
            let server = gateway(&f, vec![fault]).await;
            late_delivery::process(&f.state, &f.event.id).await;
            due(&f).await;
            late_delivery::sweep(&f.state).await.unwrap();
            assert_eq!(provider(&f).await, "started");
            assert!(outcomes(&f).await.is_empty());
            patch(&f, doc! {"delivery.preparation.gateway_receipt.deadline":bson::DateTime::from_millis(0)}).await;
            due(&f).await;
            late_delivery::sweep(&f.state).await.unwrap();
            late_delivery::sweep(&f.state).await.unwrap();
            assert_eq!(status(&f).await, "unknown");
            assert_eq!(provider(&f).await, "unconfirmed");
            assert_eq!(wait_row(&f).await.get_bool("pending"), Ok(false));
            server.assert_one_message(2);
            let advice = health(&f).await;
            assert_eq!(advice.action, "gateway");
            assert!(!advice.text.contains("accepted"));
            assert!(!advice.text.contains("still trying"));
            assert!(advice.text.contains("before checking stopped"));
            let audits = outcomes(&f).await;
            assert_eq!(audits.len(), 1);
            let metadata = audits[0].to_string();
            assert!(metadata.contains(failure));
            assert_eq!(audits[0].get_document("event_data").unwrap().get_bool("gateway_admitted"), Ok(false));
        }
    }).await;
}

#[tokio::test]
async fn gateway_reask_admission_is_only_recorded_at_first_exact_409() {
    Box::pin(async {
        let f = prepared("reask_late_admission").await;
        let server = gateway(
            &f,
            vec![ResponseKind::Timeout, IN_PROGRESS, IN_PROGRESS, CONFIRMED],
        )
        .await;
        late_delivery::process(&f.state, &f.event.id).await;
        assert!(outcomes(&f).await.is_empty());
        assert!(
            wait_row(&f)
                .await
                .get("admitted_at")
                .unwrap()
                .as_null()
                .is_some()
        );
        for count in [1, 1, 2] {
            due(&f).await;
            late_delivery::sweep(&f.state).await.unwrap();
            assert_eq!(outcomes(&f).await.len(), count);
        }
        assert_eq!(status(&f).await, "sent");
        server.assert_one_message(4);
        assert!(outcomes(&f).await.iter().all(|row| {
            row.get_document("event_data")
                .unwrap()
                .get_bool("gateway_admitted")
                == Ok(true)
        }));
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_frozen_sending_recovers_one_stream_after_lease() {
    Box::pin(async {
        for hook in [
            "late_text_dispatch_committed",
            "late_text_accepted_before_receipt",
        ] {
            let f = prepared(&format!("reask_crash_{hook}")).await;
            let server = gateway(&f, vec![CONFIRMED]).await;
            crate::services::durable_wake_tests::drop_after(
                hook,
                late_delivery::process(&f.state, &f.event.id),
            )
            .await;
            let sent = usize::from(hook == "late_text_accepted_before_receipt");
            if sent == 1 {
                // The existing model also reads legacy integer-array ciphertext.
                let stored = raw(&f).await;
                let bytes = stored
                    .get_document("delivery")
                    .unwrap()
                    .get_binary_generic("target_ciphertext")
                    .unwrap();
                let legacy = bson::Bson::Array(
                    bytes
                        .iter()
                        .map(|byte| bson::Bson::Int32(i32::from(*byte)))
                        .collect(),
                );
                patch(&f, doc! {"delivery.target_ciphertext":legacy}).await;
            }
            assert_eq!(server.requests().len(), sent);
            assert_eq!(status(&f).await, "sending");
            late_delivery::sweep(&f.state).await.unwrap();
            assert_eq!(
                server.requests().len(),
                sent,
                "live lease cannot be reclaimed"
            );
            let before = raw(&f).await;
            let barrier = *before
                .get_document("delivery")
                .unwrap()
                .get_datetime("checked_at")
                .unwrap();
            expire_text_preparation(&f).await;
            let (a, b) = tokio::join!(
                late_delivery::sweep(&f.state),
                late_delivery::sweep(&f.state)
            );
            a.unwrap();
            b.unwrap();
            assert_eq!(status(&f).await, "sent");
            server.assert_one_message(sent + 1);
            let wait = wait_row(&f).await;
            assert_eq!(wait.get_datetime("started_at"), Ok(&barrier));
            assert!(
                wait.get_datetime("deadline").unwrap().timestamp_millis()
                    <= barrier.timestamp_millis() + 86_400_000
            );
            assert_eq!(outcomes(&f).await.len(), 1);
        }
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_recovery_requires_frozen_target_hash_and_original_clock() {
    Box::pin(async {
        for field in ["hash", "hash_version", "target", "target_expiry", "preparation_version", "clock", "clock_missing"] {
            let f = prepared(&format!("reask_legacy_{field}")).await;
            let server = gateway(&f, vec![CONFIRMED]).await;
            crate::services::durable_wake_tests::drop_after("late_text_dispatch_committed", late_delivery::process(&f.state, &f.event.id)).await;
            expire_text_preparation(&f).await;
            let change = match field {
                "hash" => doc! {"delivery.preparation.gateway_request_hash":bson::Bson::Null},
                "hash_version" => doc! {"delivery.preparation.gateway_request_version":2},
                "target" => doc! {"delivery.target_ciphertext":bson::Bson::Null},
                "target_expiry" => doc! {"delivery.target_expires_at":bson::Bson::Null},
                "preparation_version" => doc! {"delivery.preparation.version":0, "delivery.checked_at":bson::DateTime::from_millis(0)},
                "clock" => doc! {"delivery.checked_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1))},
                _ => doc! {"delivery.checked_at":bson::Bson::Null},
            };
            patch(&f, change).await;
            assert!(prep::gateway::recover(&f.state.db, &f.event.id).await.unwrap().is_none());
            if field != "clock_missing" {
                late_delivery::sweep(&f.state).await.unwrap();
                assert_eq!(status(&f).await, "unknown");
            }
            assert!(server.requests().is_empty());
        }
    }).await;
}

#[tokio::test]
async fn gateway_reask_interruption_deadline_uses_barrier_without_extension() {
    Box::pin(async {
        let f = prepared("reask_interrupted_expired").await;
        let server = gateway(&f, vec![CONFIRMED]).await;
        crate::services::durable_wake_tests::drop_after(
            "late_text_dispatch_committed",
            late_delivery::process(&f.state, &f.event.id),
        )
        .await;
        let barrier = bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(25));
        patch(&f, doc! {"delivery.checked_at":barrier}).await;
        expire_text_preparation(&f).await;
        late_delivery::sweep(&f.state).await.unwrap();
        assert!(server.requests().is_empty());
        let wait = wait_row(&f).await;
        assert_eq!(wait.get_datetime("started_at"), Ok(&barrier));
        assert_eq!(
            wait.get_datetime("deadline").unwrap().timestamp_millis(),
            barrier.timestamp_millis() + 86_400_000
        );
        assert_eq!(wait.get_bool("pending"), Ok(false));
        let advice = health(&f).await;
        assert!(advice.text.contains("stopped before recording"));
        assert!(!advice.text.contains("accepted"));
        assert_eq!(outcomes(&f).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_v1_upgrade_v2_old_reader_exclusion_and_deadline_contract() {
    Box::pin(async {
        let f = prepared("reask_upgrade").await;
        let _server = gateway(&f, vec![ResponseKind::Http(500, "{}")]).await;
        late_delivery::process(&f.state, &f.event.id).await;
        patch(&f, doc! {"delivery.preparation.gateway_receipt.version":1}).await;
        assert_eq!(provider(&f).await, "started");
        due(&f).await;
        let saved = prep::gateway::acquire(&f.state.db, &f.event.id, "new-worker")
            .await
            .unwrap()
            .unwrap();
        let wait = saved.gateway_receipt.unwrap();
        assert_eq!(wait.version, 2);
        assert_eq!(provider(&f).await, "started");
        let mut old_query = prep::gateway::due(Utc::now() + ChronoDuration::days(1));
        old_query.insert("delivery.preparation.gateway_receipt.version", 1);
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(EVENTS)
                .count_documents(old_query)
                .await
                .unwrap(),
            0
        );
        assert!(
            prep::gateway::record(
                &f.state.db,
                &f.event.id,
                "new-worker",
                &f.channel.channel_bot_id,
                &crate::models::gateway_reply::Reply {
                    outcome: crate::models::gateway_reply::Outcome::Pending,
                    observation: wait.observation.clone()
                },
                Some(&wait),
                wait.deadline + ChronoDuration::days(1),
                "gateway_reply_waiting"
            )
            .await
            .unwrap()
        );
        assert_eq!(
            wait_row(&f)
                .await
                .get_datetime("deadline")
                .unwrap()
                .to_chrono(),
            wait.deadline
        );
        // #2561's observation reader ignores the additive failure field and its
        // closed Code enum still reads every newly written observation.
        #[derive(serde::Deserialize)]
        struct OldObservation {
            status: Option<u16>,
            code: Option<crate::models::gateway_reply::Code>,
        }
        let old: OldObservation = bson::from_document(
            wait_row(&f)
                .await
                .get_document("observation")
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(old.status, Some(500));
        assert!(old.code.is_none());
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_explicit_receipts_valid_json_and_definite_4xx_are_terminal() {
    Box::pin(async {
        for (name, response, expected) in [
            ("missing", ResponseKind::Http(200,"{}"), "unknown"),
            ("null", ResponseKind::Http(200,"null"), "unknown"),
            ("unrecognized", ResponseKind::Http(200,r#"{"delivery":{"outcome":"new_outcome"}}"#), "unknown"),
            ("unknown", ResponseKind::Http(200,r#"{"delivery":{"outcome":"unknown","reason":"delivery_in_progress_or_uncertain"}}"#), "unknown"),
            ("rate_limited", ResponseKind::Http(200,r#"{"delivery":{"outcome":"rate_limited"}}"#), "unknown"),
            ("unsupported", ResponseKind::Http(200,r#"{"delivery":{"outcome":"unsupported"}}"#), "unknown"),
            ("rejected", ResponseKind::Http(200,r#"{"delivery":{"outcome":"rejected"}}"#), "refused"),
            ("broken403", ResponseKind::Truncated(403), "refused"),
            ("slow403", ResponseKind::BodyTimeout(403), "refused"),
            ("broken409", ResponseKind::Truncated(409), "refused"),
        ] {
            let f = prepared(&format!("reask_terminal_{name}")).await;
            let server = gateway(&f, vec![response, CONFIRMED]).await;
            late_delivery::process(&f.state, &f.event.id).await;
            due(&f).await;
            late_delivery::sweep(&f.state).await.unwrap();
            assert_eq!(status(&f).await, expected);
            server.assert_one_message(1);
            assert_eq!(outcomes(&f).await.len(), 1);
        }
    }).await;
}

#[tokio::test]
async fn gateway_reask_legacy_unkeyed_operation_never_retries_transport_failure() {
    Box::pin(async {
        for (name, response) in [
            ("reset", ResponseKind::Reset),
            ("500", ResponseKind::Http(500,"{}")),
            ("broken", ResponseKind::Truncated(200)),
        ] {
            let f = prepared(&format!("reask_legacy_transport_{name}")).await;
            patch(&f, doc! {"delivery.target_ciphertext":bson::Bson::Null,"delivery.target_expires_at":bson::Bson::Null}).await;
            let server = gateway(&f, vec![response, CONFIRMED]).await;
            late_delivery::process(&f.state, &f.event.id).await;
            due(&f).await;
            late_delivery::sweep(&f.state).await.unwrap();
            assert_eq!(status(&f).await, "unknown");
            let requests = server.requests();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].path.ends_with("/replies"));
            assert!(requests[0].key.is_empty());
            assert_eq!(outcomes(&f).await.len(), 1);
        }
    }).await;
}

#[tokio::test]
async fn gateway_reask_connect_failure_admits_only_on_first_arriving_request() {
    Box::pin(async {
        let f = prepared("reask_connect_failure").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        f.state
            .db
            .collection::<bson::Document>(SERVICES)
            .update_one(doc! {"slug":GATEWAY_SLUG}, doc! {"$set":{"base_url":base}})
            .await
            .unwrap();
        late_delivery::process(&f.state, &f.event.id).await;
        let wait = wait_row(&f).await;
        assert_eq!(
            wait.get_document("observation").unwrap().get_str("failure"),
            Ok("no_response")
        );
        assert!(outcomes(&f).await.is_empty());
        let server = gateway(&f, vec![CONFIRMED]).await;
        due(&f).await;
        late_delivery::sweep(&f.state).await.unwrap();
        assert_eq!(status(&f).await, "sent");
        server.assert_one_message(1);
        let request = &server.requests()[0];
        assert_eq!(request.key, f.event.id);
        assert_eq!(request.path, "/reply-targets/cmaeg1.rt.original/messages");
        assert_eq!(request.body, br#"{"text":"Useful answer"}"#);
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_local_request_and_decryption_errors_are_not_transport_failures() {
    Box::pin(async {
        for failure in ["target", "header", "channel_binding"] {
            let f = prepared(&format!("reask_local_error_{failure}")).await;
            let server = gateway(&f, vec![CONFIRMED]).await;
            if failure == "target" {
                patch(
                    &f,
                    doc! {"delivery.target_ciphertext":bson::Binary {
                        subtype:bson::spec::BinarySubtype::Generic, bytes:vec![0,1,2],
                    }},
                )
                .await;
            } else if failure == "channel_binding" {
                let ciphertext = channel_key::seal(&f.state.encryption_keys, "other-channel", &f.channel.user_id,
                    f.channel.agent_api_key_id.as_deref().unwrap(), "fixture-bearer").await.unwrap();
                f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id":&f.channel.id},
                    doc! {"$set":{channel_key::FIELD:bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},
                    channel_key::VERSION_PATH:3}}).await.unwrap();
            } else {
                let key = channel_key::seal(&f.state.encryption_keys, &f.channel.id, &f.channel.user_id, f.channel.agent_api_key_id.as_deref().unwrap(), "invalid\nheader").await.unwrap();
                f.state
                    .db
                    .collection::<bson::Document>(CHANNELS)
                    .update_one(
                        doc! {"_id":&f.channel.id},
                        doc! {"$set":{"agent_key_ciphertext":bson::Binary {
                            subtype:bson::spec::BinarySubtype::Generic, bytes:key,
                        }}},
                    )
                    .await
                    .unwrap();
            }
            late_delivery::process(&f.state, &f.event.id).await;
            due(&f).await;
            late_delivery::sweep(&f.state).await.unwrap();
            assert!(server.requests().is_empty());
            let row = raw(&f).await;
            let d = row.get_document("delivery").unwrap();
            assert!(
                d.get_document("preparation")
                    .unwrap()
                    .get_document("gateway_receipt")
                    .is_err()
            );
            assert!(
                d.get_document("gateway_observation")
                    .unwrap()
                    .get("failure")
                    .unwrap()
                    .as_null()
                    .is_some()
            );
        }
    })
    .await;
}

#[tokio::test]
async fn gateway_reask_effect_boundary_keeps_slow_preparation_inside_one_live_lease() {
    Box::pin(async {
        let f = pending("reask_effect_lease", 409, "routed_reply_in_progress").await;
        due(&f).await;
        let saved = prep::gateway::acquire(&f.state.db, &f.event.id, "slow-preparer")
            .await
            .unwrap()
            .unwrap();
        let deadline = saved.gateway_receipt.unwrap().deadline;
        // Model preparation consuming most of the acquisition lease.
        f.state
            .db
            .collection::<bson::Document>(EVENTS)
            .update_one(
                doc! {"_id":&f.event.id},
                vec![doc! {"$set":{
                    "delivery.preparation.lease_until":{"$add":["$$NOW",3_000]},
                    "delivery.receipt_due_at":{"$add":["$$NOW",3_000]},
                }}],
            )
            .await
            .unwrap();
        let before = raw(&f).await;
        let old_due = *before
            .get_document("delivery")
            .unwrap()
            .get_datetime("receipt_due_at")
            .unwrap();
        prep::gateway::before_effect(&f.state.db, &f.event.id, "slow-preparer")
            .await
            .unwrap();
        let after = raw(&f).await;
        let d = after.get_document("delivery").unwrap();
        let lease = d
            .get_document("preparation")
            .unwrap()
            .get_datetime("lease_until")
            .unwrap();
        assert!(lease.timestamp_millis() >= old_due.timestamp_millis() + 117_000);
        assert!(d.get_datetime("receipt_due_at").unwrap() >= lease);
        assert_eq!(
            wait_row(&f)
                .await
                .get_datetime("deadline")
                .unwrap()
                .to_chrono(),
            deadline
        );
        tokio::time::sleep(Duration::from_millis(3_100)).await;
        assert!(
            prep::gateway::acquire(&f.state.db, &f.event.id, "competitor")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            prep::gateway::before_effect(&f.state.db, &f.event.id, "competitor")
                .await
                .is_err()
        );
    })
    .await;
}
