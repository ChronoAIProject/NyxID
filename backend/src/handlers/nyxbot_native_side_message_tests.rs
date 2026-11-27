use super::*;
use crate::models::assistant_conversation::AssistantConversation;
use std::{collections::BTreeMap, time::Duration};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

const SIDE: &str = "/events/opaque-native-ref/side-messages";
const REPLIES: &str = "/events/opaque-native-ref/replies";

async fn register_gateway(state: &AppState, base: String) {
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = GATEWAY_SLUG.into();
    service.base_url = base;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
}

struct Fixture {
    state: AppState,
    channel: NyxbotChannel,
    agent_key: String,
    conversation: AssistantConversation,
    event: NyxbotEvent,
    response_id: String,
    gateway: MockServer,
}

impl Fixture {
    async fn new(whatsapp: bool) -> Self {
        let (state, _, _) = setup("native_side_message").await;
        let (channel, agent_key) = if whatsapp {
            native_whatsapp_channel(&state).await
        } else {
            native_channel(&state).await
        };
        let request = if whatsapp {
            whatsapp_request(&channel, "wamid.side", "7", "Hello")
        } else {
            native_event(&channel, "500", "Hello")
        };
        let response = respond(&state, &agent_key, &request, "side-message").await;
        let response_id = response.headers()["x-cma-response-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(body_text(response).await.contains("Here is your answer"));
        let stored = recovery::load(&state.db, &channel, &response_id)
            .await
            .unwrap()
            .unwrap();
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&stored.event_key})
            .await
            .unwrap()
            .unwrap();
        let conversation =
            engine::get(&state.db, OWNER, stored.conversation_id.as_deref().unwrap())
                .await
                .unwrap();
        let gateway = MockServer::start().await;
        register_gateway(&state, gateway.uri()).await;
        Self {
            state,
            channel,
            agent_key,
            conversation,
            event,
            response_id,
            gateway,
        }
    }

    fn origin(&self) -> &ChannelOrigin {
        &self.event.delivery.as_ref().unwrap().origin
    }

    async fn settled_result(&self) -> AssistantConversation {
        let mut row = self.conversation.clone();
        let turn = Uuid::new_v4().to_string();
        let messages = self
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME);
        let seq = messages
            .find_one(doc! {"conversation_id":&row.id})
            .sort(doc! {"seq":-1})
            .await
            .unwrap()
            .unwrap()
            .get_i64("seq")
            .unwrap()
            + 1;
        messages.insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"user_id":&row.user_id,"conversation_id":&row.id,
                "turn_id":&turn,"seq":seq,"role":"assistant","text":"private side text","status":"completed",
                "created_at":bson::DateTime::now(),
            }).await.unwrap();
        row.active_turn = Some(bson::from_document(doc! {
            "turn_id":turn,"origin":"event","started_at":bson::DateTime::now(),"stop_requested":false,
        }).unwrap());
        row
    }

    async fn mount_capability(&self, response: ResponseTemplate, count: u64) {
        Mock::given(method("GET"))
            .and(path(SIDE))
            .respond_with(response)
            .expect(count)
            .mount(&self.gateway)
            .await;
    }

    async fn audit(&self, purpose: &str, egress: &str, outcome: &str, count: u64) {
        let rows = self
            .state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
        let audits: Vec<bson::Document> = rows
            .find(doc! {
                "event_type":"nyxbot_channel_supplemental_delivery",
                "event_data.purpose":purpose,
                "event_data.egress":egress,
                "event_data.outcome":outcome,
            })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(audits.len() as u64, count);
        for audit in audits {
            let data = audit.get_document("event_data").unwrap();
            assert_eq!(data.get_str("channel_event_id"), Ok(self.event.id.as_str()));
            let serialized = audit.to_string();
            for secret in [
                &self.agent_key,
                "opaque-native-ref",
                "private side text",
                "nyxbot-side-v1:",
            ] {
                assert!(!serialized.contains(secret));
            }
        }
    }

    async fn snapshot(&self) -> String {
        body_text(
            native_responses::snapshot(
                State(self.state.clone()),
                Path(self.response_id.clone()),
                bearer(&self.agent_key),
            )
            .await,
        )
        .await
    }
}

fn capability(available: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"available":available}))
}

fn receipt(outcome: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":outcome}}))
}

fn assert_side_post(request: &Request) -> (&str, String) {
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(request.url.path(), SIDE);
    let body: Value = request.body_json().unwrap();
    let object = body.as_object().unwrap();
    assert_eq!(object.len(), 1);
    let key = request.headers["idempotency-key"].to_str().unwrap();
    assert!(!key.is_empty() && key.len() <= 128 && key.bytes().all(|b| b.is_ascii_graphic()));
    assert!(!key.contains("opaque-native-ref"));
    (key, object["text"].as_str().unwrap().to_owned())
}

const PRIMARY_ANSWER: &str = "Primary answer";
const FAILURE_ANSWER: &str = "Configured gateway failure answer";
const FIRST_SIDE: &str = "Desktop control requested";
const UNCERTAIN_SIDE: &str = "Desktop control requested again";

#[derive(Default)]
struct GatewayEffects {
    side_receipts: BTreeMap<String, (String, &'static str)>,
    delivered: Vec<String>,
    tool_replied: bool,
}

impl GatewayEffects {
    fn side_message(&mut self, request: &Request) -> ResponseTemplate {
        let (key, text) = assert_side_post(request);
        if let Some((prior_text, outcome)) = self.side_receipts.get(key) {
            return if prior_text == &text {
                receipt(outcome)
            } else {
                ResponseTemplate::new(409)
            };
        }
        // An unknown receipt may follow a platform write. Exact replays read
        // the independent receipt and leave the original write as the only effect.
        let outcome = if text == UNCERTAIN_SIDE {
            "unknown"
        } else {
            "confirmed"
        };
        self.delivered.push(text.clone());
        self.side_receipts.insert(key.to_owned(), (text, outcome));
        receipt(outcome)
    }

    fn apply_provider_frames(&mut self, frames: &[Value]) {
        // CMA's delivery/step.rs and routed/replay_projection.rs project a
        // committed message or terminal failure through the ordinary reply marker.
        if self.tool_replied {
            return;
        }
        for frame in frames {
            match frame["type"].as_str() {
                Some("response.output_item.done") => {
                    self.delivered
                        .push(frame["item"]["content"][0]["text"].as_str().unwrap().into());
                }
                Some("response.failed") => self.delivered.push(FAILURE_ANSWER.into()),
                _ => {}
            }
        }
    }
}

#[tokio::test]
async fn supplemental_native_side_messages_before_primary_answer_and_failure() {
    for failed in [false, true] {
        Box::pin(side_messages_before_primary(failed)).await;
    }
}

async fn side_messages_before_primary(failed: bool) {
    use crate::handlers::assistant_handoff_test_upstream::{RESPONSE, SESSION};
    use tokio::sync::Notify;

    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let upstream = Router::new().route("/v1/responses", post({
        let started = started.clone();
        let release = release.clone();
        move || {
            let started = started.clone();
            let release = release.clone();
            async move {
                let stream = async_stream::stream! {
                    let created = json!({
                        "type":"response.created","sequence_number":0,
                        "response":{"id":RESPONSE,"conversation":{"id":SESSION}},
                    });
                    yield Ok::<_, std::convert::Infallible>(Event::default().data(created.to_string()));
                    started.notify_one();
                    release.notified().await;
                    let terminal = if failed {
                        json!({
                            "type":"response.failed","sequence_number":1,
                            "response":{"id":RESPONSE,"conversation":{"id":SESSION},
                                "status":"failed","error":{"code":"upstream_error"},"output":[]},
                        })
                    } else {
                        json!({
                            "type":"response.completed","sequence_number":1,
                            "response":{"id":RESPONSE,"conversation":{"id":SESSION},
                                "status":"completed","output":message_output(PRIMARY_ANSWER)},
                        })
                    };
                    yield Ok(Event::default().data(terminal.to_string()));
                };
                axum::response::Sse::new(stream)
            }
        }
    }));
    let db = connect_transaction_test_database("native_side_before_primary").await;
    let (state, _, upstream_server) =
        Box::pin(setup_with_engine(db, Value::Null, Some(upstream))).await;
    let (channel, key) = native_channel(&state).await;
    let gateway = MockServer::start().await;
    register_gateway(&state, gateway.uri()).await;
    Mock::given(method("GET"))
        .and(path(SIDE))
        .respond_with(capability(true))
        .expect(5)
        .mount(&gateway)
        .await;
    let effects = Arc::new(std::sync::Mutex::new(GatewayEffects::default()));
    let side = effects.clone();
    Mock::given(method("POST"))
        .and(path(SIDE))
        .respond_with(move |request: &Request| side.lock().unwrap().side_message(request))
        .expect(5)
        .mount(&gateway)
        .await;
    let ordinary = effects.clone();
    Mock::given(method("POST"))
        .and(path(REPLIES))
        .respond_with(move |_: &Request| {
            ordinary.lock().unwrap().tool_replied = true;
            receipt("confirmed")
        })
        .expect(0)
        .mount(&gateway)
        .await;

    let request = native_event(&channel, "502", "Hello");
    let response = respond(&state, &key, &request, "side-before-primary").await;
    assert_eq!(response.status(), StatusCode::OK);
    let response_id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let provider = tokio::spawn(body_text(response));
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .unwrap();
    let stored = recovery::load(&state.db, &channel, &response_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.terminal);
    assert!(stored.answer_delivery.is_none());
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await
        .unwrap()
        .unwrap();
    let origin = &event.delivery.as_ref().unwrap().origin;
    let conversation = stored.conversation_id.as_deref().unwrap();
    state.db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id":conversation,"active_turn.turn_id":&event.turn_id},
        doc! {"$set":{
            "active_turn.external_check_started":true,
            "active_turn.started_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(10)),
        }},
    ).await.unwrap();
    let row = engine::get(&state.db, OWNER, conversation).await.unwrap();
    assert_eq!(
        row.active_turn.as_ref().map(|turn| turn.turn_id.as_str()),
        event.turn_id.as_deref()
    );
    assert_eq!(
        row.channel.as_ref().or(row.reply_channel.as_ref()),
        Some(origin)
    );
    for (revision, text) in [
        (1, FIRST_SIDE),
        (1, FIRST_SIDE),
        (2, UNCERTAIN_SIDE),
        (2, UNCERTAIN_SIDE),
    ] {
        deliver_machine_control_notice(&state, &row, text, "one-desktop-session", revision).await;
    }

    use crate::handlers::assistant_nyxagent::send_checking_notice;
    assert!(send_checking_notice(&state, &row, origin).await);
    assert!(!send_checking_notice(&state, &row, origin).await);
    let checking = crate::services::assistant_checking_notice::TEXT;
    gateway.verify().await;
    let requests = gateway.received_requests().await.unwrap();
    assert_eq!(requests.len(), 10);
    assert!(requests.iter().all(|request| request.url.path() == SIDE));
    {
        let state = effects.lock().unwrap();
        assert_eq!(state.side_receipts.len(), 3);
        assert_eq!(state.delivered, [FIRST_SIDE, UNCERTAIN_SIDE, checking]);
        assert!(!state.tool_replied);
    }
    let pending = recovery::load(&state.db, &channel, &response_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!pending.terminal);
    assert!(pending.answer_delivery.is_none());
    let settled = state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .count_documents(
            doc! {"conversation_id":conversation,"turn_id":&event.turn_id,"role":"assistant",
            "status":{"$in":["completed","failed","cancelled","interrupted"]}},
        )
        .await
        .unwrap();
    assert_eq!(settled, 0);
    let audits = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    for (purpose, outcome, count) in [
        ("machine_desktop_notice", "confirmed", 2),
        ("machine_desktop_notice", "unknown", 2),
        ("checking_notice", "confirmed", 1),
    ] {
        assert_eq!(
            audits
                .count_documents(doc! {
                    "event_type":"nyxbot_channel_supplemental_delivery",
                    "event_data.channel_event_id":&event.id,
                    "event_data.egress":"gateway_side_message",
                    "event_data.purpose":purpose,"event_data.outcome":outcome,
                })
                .await
                .unwrap(),
            count
        );
    }

    release.notify_one();
    let wire = tokio::time::timeout(Duration::from_secs(10), provider)
        .await
        .unwrap()
        .unwrap();
    let frames = frames(&wire);
    let (terminal, answer) = if failed {
        ("response.failed", FAILURE_ANSWER)
    } else {
        ("response.completed", PRIMARY_ANSWER)
    };
    assert_eq!(frames.last().unwrap()["type"], terminal);
    {
        let mut effects = effects.lock().unwrap();
        effects.apply_provider_frames(&frames);
        assert_eq!(
            effects.delivered,
            [FIRST_SIDE, UNCERTAIN_SIDE, checking, answer]
        );
    }
    upstream_server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn supplemental_native_side_messages_progress_preserves_primary_recovery() {
    Box::pin(async {
        let mut f = Fixture::new(false).await;
        let primary = f.snapshot().await;
        f.conversation.reply_channel = Some(f.origin().clone());
        f.mount_capability(capability(true), 5).await;
        Mock::given(method("POST"))
            .and(path(SIDE))
            .respond_with(receipt("confirmed"))
            .expect(5)
            .mount(&f.gateway)
            .await;
        for purpose in [
            supplemental::Purpose::CheckingNotice,
            supplemental::Purpose::CapacityNotice,
        ] {
            deliver_to_for(
                &f.state,
                &f.conversation,
                f.origin(),
                "unused",
                None,
                purpose,
            )
            .await;
        }
        // A replay of the same desktop request carries the same key. A new
        // revision has its own identity even with identical text.
        for revision in [1, 1, 2] {
            deliver_machine_control_notice(
                &f.state,
                &f.conversation,
                "private side text",
                "desktop-request",
                revision,
            )
            .await;
        }
        f.gateway.verify().await;
        let calls = f.gateway.received_requests().await.unwrap();
        assert_eq!(calls.len(), 10);
        for pair in calls.as_chunks::<2>().0 {
            let get = &pair[0];
            assert_eq!(get.method.as_str(), "GET");
            assert_eq!(get.url.path(), SIDE);
            assert!(get.body.is_empty() && !get.headers.contains_key("idempotency-key"));
            let bearer = format!("Bearer {}", f.agent_key);
            for request in pair {
                assert_eq!(request.headers["authorization"].to_str().unwrap(), bearer);
            }
        }
        let posts: Vec<_> = calls
            .iter()
            .skip(1)
            .step_by(2)
            .map(assert_side_post)
            .collect();
        assert_ne!(posts[0].0, posts[1].0);
        assert_eq!(posts[2], posts[3]);
        assert_ne!(posts[2].0, posts[4].0);
        assert_eq!(posts[2].1, "private side text");
        for (purpose, count) in [
            ("checking_notice", 1),
            ("capacity_notice", 1),
            ("machine_desktop_notice", 3),
        ] {
            f.audit(purpose, "gateway_side_message", "confirmed", count)
                .await;
        }
        assert_eq!(primary, f.snapshot().await);
        let current = f
            .state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&f.event.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.delivery.unwrap().state, "replay");
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_team_results_use_the_settling_turn() {
    Box::pin(async {
        let f = Fixture::new(false).await;
        f.mount_capability(capability(true), 3).await;
        Mock::given(method("POST"))
            .and(path(SIDE))
            .respond_with(receipt("confirmed"))
            .expect(3)
            .mount(&f.gateway)
            .await;
        let text = "private side text";
        let mut saved = supplemental::Decision::new(supplemental::Purpose::TeamResult);
        saved.saved_result = true;
        saved.turn.clone_from(&f.event.turn_id);
        assert_eq!(
            native_responses::send_supplemental(
                &f.state,
                &f.channel,
                f.origin(),
                &f.conversation.id,
                text,
                &mut saved,
            )
            .await
            .unwrap(),
            Some("confirmed")
        );
        let first = f.settled_result().await;
        deliver_to_for_turn(
            &f.state,
            &f.conversation,
            f.origin(),
            text,
            None,
            supplemental::Purpose::TeamResult,
            first.active_turn.as_ref(),
        )
        .await;
        let second = f.settled_result().await;
        for result in [&first, &second, &second] {
            // An explicitly supplied result turn works with a settled chat
            // snapshot that no longer carries an active turn.
            deliver_to_for_turn(
                &f.state,
                &f.conversation,
                f.origin(),
                text,
                None,
                supplemental::Purpose::TeamAdditionalResult,
                result.active_turn.as_ref(),
            )
            .await;
        }
        f.gateway.verify().await;
        let requests = f.gateway.received_requests().await.unwrap();
        let posts: Vec<_> = requests
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .map(assert_side_post)
            .collect();
        assert_eq!(posts.len(), 3);
        assert!(posts.iter().all(|(_, sent)| sent == text));
        assert_eq!(
            posts
                .iter()
                .map(|(key, _)| key)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        let audits = f
            .state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(audits.count_documents(doc! {
            "event_type":"nyxbot_channel_supplemental_delivery","event_data.reason":"already_claimed",
        }).await.unwrap(), 2);
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_unbound_result_keeps_legacy_fallback() {
    Box::pin(async {
        let f = Fixture::new(false).await;
        assert!(f.conversation.active_turn.is_none());
        let text = "private side text";
        for (probe, available) in [
            (capability(true), true),
            (capability(false), false),
            (ResponseTemplate::new(404), false),
        ] {
            f.gateway.reset().await;
            f.mount_capability(probe, 1).await;
            Mock::given(method("POST"))
                .and(path(REPLIES))
                .respond_with(receipt("confirmed"))
                .expect(u64::from(!available))
                .mount(&f.gateway)
                .await;

            deliver_to(&f.state, &f.conversation, f.origin(), text, None).await;
            f.gateway.verify().await;
            let requests = f.gateway.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1 + usize::from(!available));
            assert_eq!(requests[0].method.as_str(), "GET");
            assert_eq!(requests[0].url.path(), SIDE);
            if let Some(reply) = requests.get(1) {
                assert_eq!(reply.method.as_str(), "POST");
                assert_eq!(reply.url.path(), REPLIES);
                assert_eq!(reply.body_json::<Value>().unwrap(), json!({"text":text}));
                assert!(!reply.headers.contains_key("idempotency-key"));
            }
        }
        f.audit("team_result", "gateway_event", "confirmed", 2)
            .await;
        f.audit("team_result", "skipped", "skipped", 1).await;
        let audits = f
            .state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(
            audits
                .count_documents(doc! {
                    "event_type":"nyxbot_channel_supplemental_delivery",
                    "event_data.reason":"missing_result_identity",
                })
                .await
                .unwrap(),
            1
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_probe_unavailable_uses_legacy_results() {
    Box::pin(async {
        let f = Fixture::new(false).await;
        for response in [
            capability(false),
            ResponseTemplate::new(404),
            ResponseTemplate::new(503),
            ResponseTemplate::new(200).set_body_json(json!({"available":"true"})),
            capability(true).set_delay(Duration::from_secs(3)),
        ] {
            f.gateway.reset().await;
            f.mount_capability(response, 2).await;
            Mock::given(method("POST"))
                .and(path(REPLIES))
                .respond_with(receipt("confirmed"))
                .expect(1)
                .mount(&f.gateway)
                .await;
            deliver_to(
                &f.state,
                &f.settled_result().await,
                f.origin(),
                "private side text",
                None,
            )
            .await;
            deliver_to_for(
                &f.state,
                &f.conversation,
                f.origin(),
                "unused",
                None,
                supplemental::Purpose::CheckingNotice,
            )
            .await;
            f.gateway.verify().await;
            let calls = f.gateway.received_requests().await.unwrap();
            assert_eq!(calls.len(), 3);
            assert_eq!(calls[1].url.path(), REPLIES);
            assert!(!calls[1].headers.contains_key("idempotency-key"));
        }
        f.audit("team_result", "gateway_event", "confirmed", 5)
            .await;
        f.audit("checking_notice", "skipped", "skipped", 5).await;
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_refused_probe_has_no_fallback() {
    Box::pin(async {
        let f = Fixture::new(false).await;
        for status in [401, 403, 410] {
            f.gateway.reset().await;
            f.mount_capability(ResponseTemplate::new(status), 1).await;
            deliver_to(
                &f.state,
                &f.settled_result().await,
                f.origin(),
                "private side text",
                None,
            )
            .await;
            f.gateway.verify().await;
            let calls = f.gateway.received_requests().await.unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].method.as_str(), "GET");
        }
        f.audit("team_result", "skipped", "skipped", 3).await;
        let refusals = f
            .state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {
                "event_type":"nyxbot_channel_supplemental_delivery",
                "event_data.reason":"authority_refused",
            })
            .await
            .unwrap();
        assert_eq!(refusals, 3);
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_unknown_source_short_circuits_discovery() {
    Box::pin(async {
        let f = Fixture::new(false).await;
        f.state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id":&f.channel.id},
                doc! {"$set":{"gateway_source":{"type":"future_native_source"}}},
            )
            .await
            .unwrap();
        deliver_to(
            &f.state,
            &f.settled_result().await,
            f.origin(),
            "private side text",
            None,
        )
        .await;
        assert!(f.gateway.received_requests().await.unwrap().is_empty());
        f.audit("team_result", "skipped", "skipped", 1).await;
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_refusal_headers_win_over_slow_body() {
    let router = Router::new().route(
        SIDE,
        axum::routing::get(|| async {
            (
                StatusCode::FORBIDDEN,
                axum::body::Body::from_stream(async_stream::stream! {
                    yield Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b"{"));
                    std::future::pending::<()>().await;
                }),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let result =
        crate::services::nyxbot_gateway_service::side_message_available(&base, SIDE, "test-key")
            .await;
    assert!(matches!(result, Err(AppError::Forbidden(_))));
    server.abort();
}

#[tokio::test]
async fn supplemental_native_side_messages_refusal_leaves_saved_part_unclaimed() {
    Box::pin(async {
        use crate::{
            models::channel_text_delivery::TextPartState,
            services::channel_text_delivery as journal,
        };
        let f = Fixture::new(false).await;
        f.mount_capability(capability(true), 2).await;
        Mock::given(method("POST"))
            .and(path(SIDE))
            .respond_with(receipt("confirmed"))
            .expect(1)
            .mount(&f.gateway)
            .await;
        let turn = Uuid::new_v4().to_string();
        let mut decision = supplemental::Decision::new(supplemental::Purpose::TeamResult);
        decision.saved_result = true;
        decision.turn = Some(turn.clone());
        let channels = f.state.db.collection::<bson::Document>(CHANNELS);
        let site = "native_side_message_before_authority";
        crate::services::durable_wake_tests::resume_after(
            site,
            async {
                let result = native_responses::send_supplemental(
                    &f.state,
                    &f.channel,
                    f.origin(),
                    &f.conversation.id,
                    "A saved result",
                    &mut decision,
                )
                .await;
                assert!(matches!(result, Err(AppError::NotFound(_))));
            },
            async {
                channels
                    .update_one(
                        doc! {"_id":&f.channel.id},
                        doc! {"$set":{"gateway_native_phase":"preparing"}},
                    )
                    .await
                    .unwrap();
            },
        )
        .await;
        let key = crate::services::channel_supplemental_delivery::saved_key(&turn, f.origin());
        let plan = journal::load(&f.state.db, &key).await.unwrap().unwrap();
        assert_eq!(plan.parts[0].state, TextPartState::Pending);
        assert!(plan.parts[0].attempt_id.is_none());
        assert_eq!(f.gateway.received_requests().await.unwrap().len(), 1);
        channels
            .update_one(
                doc! {"_id":&f.channel.id},
                doc! {"$set":{"gateway_native_phase":"ready"}},
            )
            .await
            .unwrap();
        assert_eq!(
            native_responses::send_supplemental(
                &f.state,
                &f.channel,
                f.origin(),
                &f.conversation.id,
                "A saved result",
                &mut decision,
            )
            .await
            .unwrap(),
            Some("confirmed")
        );
        f.gateway.verify().await;
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_outcomes_never_fall_back_or_change_health() {
    Box::pin(async {
        let f = Fixture::new(false).await;
        let channels = f.state.db.collection::<NyxbotChannel>(CHANNELS);
        channels
            .update_one(
                doc! {"_id":&f.channel.id},
                doc! {"$set":{"gateway_native_delivery":"reply_rejected"}},
            )
            .await
            .unwrap();
        for response in [
            receipt("confirmed"),
            receipt("rejected"),
            receipt("rate_limited"),
            receipt("unsupported"),
            receipt("unknown"),
            receipt("future_outcome"),
            ResponseTemplate::new(409),
            ResponseTemplate::new(503),
            ResponseTemplate::new(404),
            receipt("confirmed").set_delay(Duration::from_secs(21)),
        ] {
            f.gateway.reset().await;
            f.mount_capability(capability(true), 1).await;
            Mock::given(method("POST"))
                .and(path(SIDE))
                .respond_with(response)
                .expect(1)
                .mount(&f.gateway)
                .await;
            deliver_to(
                &f.state,
                &f.settled_result().await,
                f.origin(),
                "private side text",
                None,
            )
            .await;
            f.gateway.verify().await;
            let calls = f.gateway.received_requests().await.unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(assert_side_post(&calls[1]).1, "private side text");
        }
        for (outcome, count) in [("confirmed", 1), ("rejected", 4), ("unknown", 5)] {
            f.audit("team_result", "gateway_side_message", outcome, count)
                .await;
        }
        let channel = channels
            .find_one(doc! {"_id":&f.channel.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            channel.gateway_native_delivery.as_deref(),
            Some("reply_rejected")
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_saved_multipart_resume_keeps_its_route() {
    Box::pin(async {
        let f = Fixture::new(true).await;
        let text = format!("{}\n\n{}", "a".repeat(3000), "b".repeat(3000));
        for side in [false, true] {
            f.gateway.reset().await;
            f.mount_capability(capability(side), 1).await;
            let route = if side { SIDE } else { REPLIES };
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(receipt("confirmed"))
                .expect(1)
                .mount(&f.gateway)
                .await;
            let mut decision = supplemental::Decision::new(supplemental::Purpose::TeamResult);
            decision.saved_result = true;
            let turn = Uuid::new_v4().to_string();
            decision.turn = Some(turn.clone());
            crate::services::durable_wake_tests::drop_after(
                "native_supplemental_part_confirmed",
                native_responses::send_supplemental(
                    &f.state,
                    &f.channel,
                    f.origin(),
                    &f.conversation.id,
                    &text,
                    &mut decision,
                ),
            )
            .await;
            f.gateway.verify().await;
            let prefix = f.gateway.received_requests().await.unwrap();
            assert_eq!(prefix.len(), 2);
            assert!(
                prefix[1].body_json::<Value>().unwrap()["text"]
                    .as_str()
                    .unwrap()
                    .contains('a')
            );

            f.gateway.reset().await;
            f.mount_capability(ResponseTemplate::new(503), u64::from(side))
                .await;
            Mock::given(method("POST"))
                .and(path(REPLIES))
                .respond_with(receipt("confirmed"))
                .expect(u64::from(!side))
                .mount(&f.gateway)
                .await;
            let outcome = native_responses::send_supplemental(
                &f.state,
                &f.channel,
                f.origin(),
                &f.conversation.id,
                &text,
                &mut decision,
            )
            .await;
            f.gateway.verify().await;
            let resumed = f.gateway.received_requests().await.unwrap();
            assert_eq!(resumed.len(), 1);
            if side {
                assert!(matches!(outcome, Err(AppError::NotFound(_))));
                assert_eq!(resumed[0].method.as_str(), "GET");
            } else {
                assert_eq!(outcome.unwrap(), Some("confirmed"));
                assert_eq!(resumed[0].url.path(), REPLIES);
                let suffix: Value = resumed[0].body_json().unwrap();
                assert!(
                    suffix["text"]
                        .as_str()
                        .unwrap()
                        .chars()
                        .all(|c| c == 'b' || c.is_whitespace())
                );
            }
            let saved =
                crate::services::channel_supplemental_delivery::saved_key(&turn, f.origin());
            let plan = crate::services::channel_text_delivery::load(&f.state.db, &saved)
                .await
                .unwrap()
                .unwrap();
            use crate::models::channel_text_delivery::TextPartState::{Pending, Sent};
            assert_eq!(
                plan.parts.iter().map(|part| part.state).collect::<Vec<_>>(),
                [Sent, if side { Pending } else { Sent }]
            );
        }
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_native_side_messages_whatsapp_parts_stop_and_saved_journal_blocks_replay() {
    Box::pin(async {
        let f = Fixture::new(true).await;
        let text = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(3000),
            "b".repeat(3000),
            "c".repeat(3000)
        );
        let expected = crate::services::channel_reply_plan::text_parts(
            &crate::services::channel_platform::OutboundReply {
                text: Some(text.clone()),
                attachments: vec![],
                reply_to_platform_message_id: None,
                metadata: None,
            },
            "whatsapp",
        )
        .unwrap();
        assert_eq!(expected.len(), 3);
        for (outcome, sent) in [("confirmed", 3), ("unknown", 2), ("rejected", 2)] {
            f.gateway.reset().await;
            f.mount_capability(capability(true), if sent == 3 { 1 } else { 2 })
                .await;
            let parts = expected.clone();
            Mock::given(method("POST"))
                .and(path(SIDE))
                .respond_with(move |request: &Request| {
                    let (_, text) = assert_side_post(request);
                    receipt(if parts[1].text.as_deref() == Some(text.as_str()) {
                        outcome
                    } else {
                        "confirmed"
                    })
                })
                .expect(sent as u64)
                .mount(&f.gateway)
                .await;
            let turn = Uuid::new_v4().to_string();
            let mut decision = supplemental::Decision::new(supplemental::Purpose::TeamResult);
            decision.saved_result = true;
            decision.turn = Some(turn.clone());
            assert_eq!(
                native_responses::send_supplemental(
                    &f.state,
                    &f.channel,
                    f.origin(),
                    &f.conversation.id,
                    &text,
                    &mut decision,
                )
                .await
                .unwrap(),
                Some(outcome)
            );
            let duplicate = native_responses::send_supplemental(
                &f.state,
                &f.channel,
                f.origin(),
                &f.conversation.id,
                &text,
                &mut decision,
            )
            .await;
            if sent == 3 {
                assert_eq!(duplicate.unwrap(), Some("confirmed"));
            } else {
                assert!(duplicate.is_err());
            }
            f.gateway.verify().await;
            let requests = f.gateway.received_requests().await.unwrap();
            assert!(requests.iter().all(|r| r.url.path() == SIDE));
            let posts: Vec<_> = requests
                .iter()
                .filter(|r| r.method.as_str() == "POST")
                .map(assert_side_post)
                .collect();
            assert_eq!(posts.len(), sent);
            assert!(posts.windows(2).all(|pair| pair[0].0 != pair[1].0));
            for ((_, text), part) in posts.iter().zip(&expected) {
                assert_eq!(Some(text), part.text.as_ref());
            }
        }
        f.state.db.drop().await.unwrap();
    })
    .await;
}
