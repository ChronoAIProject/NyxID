//! Telegram relay → gateway provider → member home → relay reply regressions.
//! The engine and Telegram HTTP endpoints are local doubles. A provider final
//! and a mock send receipt are separate assertions; neither is client readback.
use super::*;
use crate::models::{
    assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    channel_bot::COLLECTION_NAME as BOTS,
    channel_conversation::{COLLECTION_NAME as ROUTES, ChannelConversation},
    channel_message::{COLLECTION_NAME as RELAY_MESSAGES, ChannelMessage},
};
use crate::services::{channel_platform::PlatformAdapter, channel_relay_service as relay};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const FIRST: &str = "Use amber471 as my project tag and give me a fresh receipt.";
const FOLLOW_UP: &str =
    "What was the receipt for my project tag? Repeat the tag and receipt from your last answer.";
const AFTER_FAILURE: &str = "Try again with the project tag from my previous message.";
// A bounded upstream diagnostic, deliberately outside NyxID's public error codes.
const UPSTREAM_FAILURE: &str = "fixture_engine_failure";

struct ReplyAdapterGuard(String);
impl Drop for ReplyAdapterGuard {
    fn drop(&mut self) {
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

fn frames(wire: &str) -> Vec<Value> {
    wire.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).expect("provider data frame is valid JSON"))
        .collect()
}

#[tokio::test]
async fn telegram_org_member_dm_2641_follow_up_uses_first_answer_and_replay_is_inert() {
    Box::pin(continuity(false)).await;
}

#[tokio::test]
async fn telegram_org_member_dm_2641_failed_turn_releases_home_for_next_message() {
    Box::pin(continuity(true)).await;
}

type Context = Arc<Mutex<Option<(String, String)>>>;

struct MockEngine {
    db: mongodb::Database,
    calls: Calls,
    context: Context,
    first_fails: bool,
}

fn mock_engine(db: mongodb::Database, calls: Calls, context: Context, first_fails: bool) -> Router {
    Router::new()
        .route("/v1/responses", post(engine_answer))
        .with_state(Arc::new(MockEngine {
            db,
            calls,
            context,
            first_fails,
        }))
}

async fn engine_answer(
    State(engine): State<Arc<MockEngine>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let raw = headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap();
    let (_, key, _) = key_service::validate_api_key(&engine.db, raw)
        .await
        .unwrap();
    let conversation = engine
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"credential_api_key_id":&key.id})
        .await
        .unwrap()
        .unwrap();
    let turn = conversation
        .active_turn
        .as_ref()
        .expect("live turn at engine boundary");
    // Reset history belongs in session instructions. Inspect it transiently;
    // only this boolean is retained, never the instructions or recap text.
    let reset_recap_has_first = body["instructions"]
        .as_str()
        .is_some_and(|instructions| instructions.contains(FIRST));
    // Capture attribution, continuity and the synthetic response identity below.
    // Credentials, headers and stable instructions stay outside diagnostics.
    let observed = json!({"input":body["input"],"conversation":body["conversation"],
        "reset_recap_has_first":reset_recap_has_first,
        "actor":conversation.user_id,"home":conversation.id,"key_id":key.id,
        "turn_id":turn.turn_id,"event_id":turn.channel_event_id,"context":turn.channel_context});
    let mut calls = engine.calls.lock().await;
    let index = calls.len();
    calls.push(observed);
    drop(calls);
    let mut context = engine.context.lock().await;
    let session = format!("conv_{}", Uuid::new_v4().simple());
    let (session, answer) = if index == 0 {
        // This receipt exists only in the first engine answer. A successful
        // follow-up must consult the context identified by its session handle.
        let answer = format!("amber471 receipt {}", Uuid::new_v4().simple());
        *context = Some((session.clone(), answer.clone()));
        (session, Some(answer))
    } else if index == 1 && engine.first_fails {
        let input = body["input"].as_str().unwrap_or_default();
        let answer = (body["conversation"].is_null()
            && reset_recap_has_first
            && input.ends_with(AFTER_FAILURE)
            && !input.contains(FIRST))
        .then(|| "Recovered amber471".to_owned());
        (session, answer)
    } else if index == 1 {
        let (previous_session, previous_answer) = context.as_ref().unwrap();
        let (tag, receipt) = previous_answer.split_once(" receipt ").unwrap();
        let answer = (body["conversation"] == *previous_session)
            .then(|| format!("Receipt {receipt} belongs to {tag}."));
        (previous_session.clone(), answer)
    } else {
        (session, None)
    };
    let failed = (engine.first_fails && index == 0) || answer.is_none();
    // The real engine client validates conv_<32 hex> and
    // resp_<same session hex>_<32 hex> before persisting either identity.
    let response_id = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
    let created = json!({"type":"response.created","sequence_number":0,
        "response":{"id":response_id,"conversation":{"id":session},
            "status":"in_progress","output":[]}});
    let completed = json!({"type":if failed {"response.failed"} else {"response.completed"},
        "sequence_number":1,"response":{"id":response_id,
        "conversation":{"id":session},"status":if failed {"failed"} else {"completed"},
        "error":if failed {json!({"code":UPSTREAM_FAILURE,"message":"Synthetic engine failure or missing context"})} else {Value::Null},
        "output":if failed {json!([])} else {message_output(answer.as_deref().unwrap())}}});
    let frames = vec![created, completed];
    {
        let mut calls = engine.calls.lock().await;
        calls[index]["engine_session_id"] = json!(session);
        calls[index]["engine_response_id"] = json!(response_id);
    }
    axum::response::Sse::new(futures::stream::iter(
        frames
            .into_iter()
            .map(|frame| Ok::<_, Infallible>(Event::default().data(frame.to_string()))),
    ))
    .into_response()
}

async fn fixture(first_fails: bool) -> Fixture {
    // Each invocation owns its database, engine memory, HTTP servers and bot
    // adapter override. No shared database or timing-based fence cleanup.
    let db = connect_transaction_test_database(if first_fails {
        "org_dm_2641_failure"
    } else {
        "org_dm_2641_continuity"
    })
    .await;
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let memory = Arc::new(Mutex::new(None::<(String, String)>));
    let engine = mock_engine(db.clone(), calls.clone(), memory.clone(), first_fails);
    let (state, _, server) = Box::pin(setup_with_engine(db, Value::Null, Some(engine))).await;
    let server = tokio_util::task::AbortOnDropHandle::new(server);
    let (row, key, agent, org_id) = Box::pin(org_channel(&state, false)).await;
    assert_eq!(row.platform, "telegram");
    assert_eq!(row.transport, "gateway");
    assert!(
        row.gateway_source.is_none(),
        "Telegram uses NyxID relay-backed ingress"
    );

    let member = context_2187::member(&state, &row, &org_id).await;
    assert_ne!(member, row.user_id, "linking admin supplies transport only");
    let home = Box::pin(team::home_thread_for(
        &state.db,
        &state.encryption_keys,
        &member,
        &agent,
    ))
    .await
    .unwrap();
    assert!(home.channel.is_none() && !home.guest_turn);

    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_turns".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_org_partition(&state, &key).await;
    let platform = MockServer::start().await;
    let update = BotSecret::update(
        &state.encryption_keys,
        &row.channel_bot_id,
        &org_id,
        doc! {},
        &[(BotSecret::Token, b"123:org-dm-2641-test")],
    )
    .await;
    let updated = state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(doc! {"_id":&row.channel_bot_id,"user_id":&org_id}, update)
        .await
        .unwrap();
    assert_eq!(
        updated.matched_count, 1,
        "fixture updates its exact org bot"
    );
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            row.channel_bot_id.clone(),
            Arc::new(
                crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                    &platform.uri(),
                ),
            ),
        );
    let adapter = ReplyAdapterGuard(row.channel_bot_id.clone());
    Mock::given(method("POST"))
        .and(path("/bot123:org-dm-2641-test/sendMessage"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            // Supplemental admission notices and primary replies need distinct
            // provider receipts even when anchored to the same inbound message.
            let base = if body["text"] == Notice::Pending.text(None) {
                2000
            } else {
                1000
            };
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{
                "message_id":base + body["reply_to_message_id"].as_i64().unwrap()}}))
        })
        .mount(&platform)
        .await;

    Fixture {
        state,
        row,
        key,
        member,
        home,
        org_id,
        calls,
        memory,
        platform,
        _server: server,
        _adapter: adapter,
    }
}

struct Fixture {
    state: AppState,
    row: NyxbotChannel,
    key: String,
    member: String,
    home: AssistantConversation,
    org_id: String,
    calls: Calls,
    memory: Context,
    platform: MockServer,
    _server: tokio_util::task::AbortOnDropHandle<()>,
    _adapter: ReplyAdapterGuard,
}

// Reply capabilities deliberately have no Debug implementation.
struct Source {
    id: String,
    native_id: i64,
    text: String,
    request: Value,
    reply_token: String,
}

struct Original {
    source: Source,
    turn_id: String,
    response_id: String,
    thread_reply: bool,
}

async fn continuity(first_fails: bool) {
    let f = Box::pin(fixture(first_fails)).await;
    let mut originals: Vec<Original> = Vec::new();
    let mut expected_sends = Vec::new();
    let mut delivered_first: Option<(String, String)> = None;
    for (index, text) in [
        FIRST,
        if first_fails {
            AFTER_FAILURE
        } else {
            FOLLOW_UP
        },
    ]
    .into_iter()
    .enumerate()
    {
        let source = relay_source(&f, index, text).await;
        let user_input = source.request["input"][0]["content"][0]["text"]
            .as_str()
            .expect("gateway request carries user input text");
        assert_eq!(user_input, source.text);
        if index == 1 && !first_fails {
            let (_, receipt) = delivered_first
                .as_ref()
                .expect("base primary POST precedes C2");
            assert!(!source.text.contains(receipt));
            assert!(
                !user_input.contains(receipt),
                "C2 does not supply the receipt"
            );
        }
        let frames = provider_frames(&f.state, &f.key, &source.request, &source.id).await;
        let thread_reply = private_reply_route(&frames);
        let terminal = frames.last().unwrap();
        let response_id = frames[0]["response"]["id"].as_str().unwrap().to_owned();
        assert_eq!(terminal["response"]["id"], response_id);
        let failed = first_fails && index == 0;
        assert_eq!(
            terminal["type"],
            if failed {
                "response.failed"
            } else {
                "response.completed"
            },
            "{frames:?}"
        );
        assert_eq!(
            frames
                .iter()
                .filter(|v| v["type"] == "response.output_item.done")
                .count(),
            usize::from(!failed)
        );
        let (turn, answer) = settled_answer(&f, &source, index, failed).await;
        assert!(originals.iter().all(|previous| previous.turn_id != turn));
        if failed {
            // With no partial text, final_reply returns the public error and
            // late delivery emits response.failed. Simulated CMA sends no final;
            // verify_receipts permits only an optional pending notice here.
            assert_eq!(terminal["response"]["status"], "failed");
            assert_eq!(
                terminal["response"]["error"],
                json!({
                    "code":"assistant_unavailable","message":"NyxBot could not finish this turn."
                })
            );
            assert!(terminal["response"].get("output").is_none());
            assert!(frames.iter().all(|frame| {
                !frame["type"]
                    .as_str()
                    .unwrap()
                    .starts_with("response.output_")
            }));
        } else {
            let output = terminal["response"]["output"].as_array().unwrap();
            assert_eq!(output.len(), 1, "one provider final per successful source");
            let text = output[0]["content"][0]["text"].as_str().unwrap();
            assert_eq!(text, answer);
            let expected = if index == 0 {
                // Compare the base provider output and HTTP POST with the exact
                // first engine answer, before allowing the independent C2 in.
                f.memory.lock().await.as_ref().unwrap().1.clone()
            } else if first_fails {
                "Recovered amber471".into()
            } else {
                let (tag, receipt) = delivered_first.as_ref().unwrap();
                let memory = f.memory.lock().await;
                let (session, _) = memory.as_ref().unwrap();
                let calls = f.calls.lock().await;
                assert_eq!(calls[1]["conversation"], *session);
                assert!(!calls[1]["input"].as_str().unwrap().contains(receipt));
                format!("Receipt {receipt} belongs to {tag}.")
            };
            assert_eq!(text, expected);
            let receipt = relay_final(&f, &source, text, thread_reply)
                .await
                .unwrap()
                .0;
            assert_eq!(
                receipt.platform_message_id.as_deref(),
                Some((1000 + source.native_id).to_string().as_str())
            );
            expected_sends.push((source.native_id, expected));
        }
        originals.push(Original {
            source,
            turn_id: turn,
            response_id,
            thread_reply,
        });
        // Inspect the actual mock transport before the next source is admitted.
        // This rejects a missing/pending-only base as well as extra or wrong sends.
        let delivered = verify_receipts(&f, &originals, &expected_sends).await;
        if index == 0 && !first_fails {
            let (tag, receipt) = delivered.primary_posts[0]
                .1
                .split_once(" receipt ")
                .unwrap();
            delivered_first = Some((tag.into(), receipt.into()));
        }
    }
    let delivered = verify_receipts(&f, &originals, &expected_sends).await;
    // C2 follows the original base answer. All replays, including consumed
    // delivery tokens, run only after these two independent messages settle.
    for original in &originals {
        let source = &original.source;
        let replay = provider_frames(&f.state, &f.key, &source.request, &source.id).await;
        let replay_thread_reply = private_reply_route(&replay);
        assert_eq!(replay_thread_reply, original.thread_reply);
        assert_eq!(f.calls.lock().await.len(), 2);
        assert_eq!(
            f.state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .count_documents(doc! {"conversation_id":&f.home.id})
                .await
                .unwrap(),
            4
        );
        let replayed = f
            .state
            .db
            .collection::<bson::Document>(EVENTS)
            .find_one(doc! {"channel_id":&f.row.id,"event_id":&source.id})
            .await
            .unwrap()
            .unwrap();
        let mapping = crate::services::nyxbot_response_read::mapping(&replayed)
            .unwrap()
            .unwrap();
        assert_eq!(mapping.thread_reply, Some(original.thread_reply));
        assert_empty_replay(
            &replay,
            &original.response_id,
            &mapping,
            &source.request["metadata"],
        );
        let replayed: NyxbotEvent = bson::from_document(replayed).unwrap();
        assert_eq!(
            replayed.conversation_id.as_deref(),
            Some(f.home.id.as_str())
        );
        assert_eq!(replayed.turn_id.as_deref(), Some(original.turn_id.as_str()));
        if expected_sends
            .iter()
            .any(|(native, _)| *native == source.native_id)
        {
            let duplicate =
                relay_final(&f, source, "duplicate must not send", replay_thread_reply).await;
            assert!(
                matches!(duplicate, Err(AppError::Unauthorized(message)) if message == "Reply token already used")
            );
        }
        assert_eq!(
            verify_receipts(&f, &originals, &expected_sends).await,
            delivered,
            "replay must not send another primary reply or pending notice"
        );
    }
    let saved = engine::get(&f.state.db, &f.member, &f.home.id)
        .await
        .unwrap();
    assert!(saved.active_turn.is_none() && saved.pending_events.is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .count_documents(doc! {"user_id":&f.member,"agent_id":&f.home.agent_id,"channel":null})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id":&f.row.id})
            .await
            .unwrap(),
        2
    );
    let db = f.state.db.clone();
    drop(f);
    db.drop().await.unwrap();
}

/// Legacy/private created frames omit the marker; any explicit marker must be
/// boolean and consistent. A true marker on any response would route to a thread.
fn private_reply_route(frames: &[Value]) -> bool {
    assert_eq!(frames[0]["type"], "response.created");
    let thread_reply = frames[0]["response"]
        .get("thread_reply")
        .map(|value| value.as_bool().expect("thread_reply is a boolean"))
        .unwrap_or(false);
    assert!(!thread_reply, "private DM must not select a thread route");
    for frame in frames {
        if let Some(value) = frame["response"].get("thread_reply") {
            assert_eq!(
                value.as_bool(),
                Some(thread_reply),
                "response route marker cannot change"
            );
        }
    }
    thread_reply
}

fn assert_empty_replay(
    frames: &[Value],
    response_id: &str,
    mapping: &crate::models::nyxbot_response_read::ResponseRead,
    metadata: &Value,
) {
    // Mapped reads observe settlement: created, optional progress, completed.
    // They retain identity and empty output even when the original turn failed.
    assert!(frames.len() >= 2);
    assert_eq!(frames[0]["type"], "response.created");
    assert_eq!(frames.last().unwrap()["type"], "response.completed");
    assert!(
        frames[1..frames.len() - 1]
            .iter()
            .all(|frame| frame["type"] == "response.in_progress")
    );
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(frame["sequence_number"], index);
        let response = &frame["response"];
        assert_eq!(response["id"], response_id);
        assert_eq!(
            response["conversation"],
            json!({"id":mapping.conversation_id})
        );
        assert_eq!(response["metadata"], *metadata);
        assert_eq!(
            response["output"],
            json!([]),
            "no replay frame carries deliverable output"
        );
        let status = if index == 0 {
            "queued"
        } else if index == frames.len() - 1 {
            "completed"
        } else {
            "in_progress"
        };
        assert_eq!(response["status"], status);
    }
}

/// Simulated CMA consumes the provider final and uses the callback's one-use
/// reply capability. NyxID's real relay handler/Telegram adapter then send it.
async fn relay_final(
    f: &Fixture,
    source: &Source,
    text: &str,
    thread_reply: bool,
) -> AppResult<Json<crate::handlers::channel_relay::AsyncReplyResponse>> {
    crate::handlers::channel_relay::async_reply(
        State(f.state.clone()),
        bearer(&source.reply_token),
        crate::mw::auth::OptionalAuthUser(None),
        Json(crate::handlers::channel_relay::AsyncReplyRequest {
            thread_reply,
            message_id: source.id.clone(),
            reply: crate::handlers::channel_relay::AsyncReplyBody {
                text: Some(text.into()),
                metadata: None,
                attachments: vec![],
            },
        }),
    )
    .await
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DeliveryCounts {
    pending: usize,
    primary: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct DeliveryEvidence {
    counts: Vec<DeliveryCounts>,
    primary_posts: Vec<(i64, String)>,
}

async fn verify_receipts(
    f: &Fixture,
    originals: &[Original],
    expected: &[(i64, String)],
) -> DeliveryEvidence {
    let db = &f.state.db;
    let requests = f.platform.received_requests().await.unwrap();
    let mut counts = vec![DeliveryCounts::default(); originals.len()];
    let mut primary_posts = Vec::new();
    for request in &requests {
        assert_eq!(request.method.as_str(), "POST");
        assert!(request.url.path() == "/bot123:org-dm-2641-test/sendMessage");
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["chat_id"], "7",
            "every send stays in the original private DM"
        );
        assert!(
            body.get("message_thread_id").is_none(),
            "private Telegram payload has no thread target"
        );
        let native_id = body["reply_to_message_id"].as_i64().unwrap();
        let index = originals
            .iter()
            .position(|original| original.source.native_id == native_id)
            .expect("send must reply to an original source");
        if body["text"] == Notice::Pending.text(None) {
            assert_eq!(
                counts[index].primary, 0,
                "pending notice precedes its primary reply"
            );
            counts[index].pending += 1;
            assert!(
                counts[index].pending <= 1,
                "at most one pending notice per source"
            );
        } else {
            let (_, text) = expected
                .iter()
                .find(|(id, _)| *id == native_id)
                .expect("failed source has no primary reply");
            assert_eq!(body["text"], *text, "unexpected primary content");
            counts[index].primary += 1;
            primary_posts.push((native_id, body["text"].as_str().unwrap().to_owned()));
        }
    }
    assert_eq!(
        primary_posts
            .iter()
            .map(|(native, _)| *native)
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|(native, _)| *native)
            .collect::<Vec<_>>(),
        "primary replies retain the original source order"
    );
    for (original, count) in originals.iter().zip(&counts) {
        let source = &original.source;
        let primary = expected
            .iter()
            .any(|(native, _)| *native == source.native_id);
        assert_eq!(
            count.primary,
            usize::from(primary),
            "exactly the expected primary count per source"
        );
        let outbound: Vec<ChannelMessage> = db
            .collection(RELAY_MESSAGES)
            .find(doc! {"direction":"outbound","reply_to_message_id":&source.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(outbound.len(), count.pending + count.primary);
        let mut receipt_ids: Vec<_> = outbound
            .iter()
            .map(|message| message.platform_message_id.clone().unwrap())
            .collect();
        let mut expected_ids = Vec::new();
        if primary {
            expected_ids.push((1000 + source.native_id).to_string());
        }
        if count.pending == 1 {
            expected_ids.push((2000 + source.native_id).to_string());
        }
        receipt_ids.sort();
        expected_ids.sort();
        assert_eq!(
            receipt_ids, expected_ids,
            "primary and notice receipts remain distinct"
        );
        for outbound in &outbound {
            assert_eq!(outbound.platform_conversation_id.as_deref(), Some("7"));
            assert_eq!(
                outbound.channel_bot_id.as_deref(),
                Some(f.row.channel_bot_id.as_str())
            );
            assert_eq!(outbound.user_id, f.org_id);
        }
    }
    DeliveryEvidence {
        counts,
        primary_posts,
    }
}

async fn relay_source(f: &Fixture, index: usize, text: &str) -> Source {
    let db = &f.state.db;
    let route = db
        .collection::<ChannelConversation>(ROUTES)
        .find_one(doc! {"_id":&f.row.route_id})
        .await
        .unwrap()
        .unwrap();
    let source_id = Uuid::new_v4().to_string();
    let native_id = 101 + index as i64;
    let update = json!({"update_id":201 + index,"message":{"message_id":native_id,
        "date":Utc::now().timestamp(),"chat":{"id":7,"type":"private"},
        "from":{"id":7,"is_bot":false,"first_name":"Member"},"text":text}});
    let inbound = crate::services::channel_adapters::telegram::TelegramAdapter::new()
        .parse_inbound(&serde_json::to_vec(&update).unwrap())
        .await
        .unwrap()
        .remove(0);
    let source = relay::store_inbound_message_with_id(
        db,
        &f.row.channel_bot_id,
        &route.id,
        &f.org_id,
        "telegram",
        &inbound,
        &f.row.route_api_key_id,
        &source_id,
    )
    .await
    .unwrap();
    assert_eq!(source.platform_conversation_id.as_deref(), Some("7"));
    assert_eq!(
        source.platform_message_id.as_deref(),
        Some(native_id.to_string().as_str())
    );
    let reply_token = crate::crypto::jwt::generate_relay_reply_token(
        &f.state.jwt_keys,
        &f.state.config,
        &f.row.route_api_key_id,
        &route.id,
        &source_id,
        "telegram",
    )
    .unwrap();
    let callback = relay::build_callback_payload(
        &source,
        &route,
        &f.row.route_api_key_id,
        "gateway",
        &inbound,
        Some(reply_token),
        "https://nyxid.example",
    );
    assert!(callback.raw_platform_data.as_ref() == Some(&update));
    let request = gateway_handoff(&callback);
    Source {
        id: source_id,
        native_id,
        text: text.into(),
        request,
        reply_token: callback.reply_token.unwrap(),
    }
}

async fn settled_answer(
    f: &Fixture,
    source: &Source,
    index: usize,
    failed: bool,
) -> (String, String) {
    let db = &f.state.db;
    let admitted = db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"channel_id":&f.row.id,"event_id":&source.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        admitted.relay_message_id.as_deref(),
        Some(source.id.as_str())
    );
    assert_eq!(
        admitted.conversation_id.as_deref(),
        Some(f.home.id.as_str())
    );
    assert_eq!(admitted.resolved_conversation_id, admitted.conversation_id);
    assert_eq!(admitted.status, if failed { "failed" } else { "completed" });
    let delivery = admitted.delivery.as_ref().unwrap();
    assert_eq!(delivery.state, "streamed");
    assert_eq!(
        delivery.origin.org_execution.as_ref().unwrap().actor_id,
        f.member
    );
    assert!(!delivery.guest);
    let turn = admitted.turn_id.as_ref().unwrap();
    let saved = engine::get(db, &f.member, &f.home.id).await.unwrap();
    assert!(
        saved.active_turn.is_none(),
        "completion/failure releases the exact home fence"
    );
    assert!(
        saved.pending_events.is_empty(),
        "follow-up cannot be left queued"
    );
    assert_eq!(saved.credential_api_key_id, f.home.credential_api_key_id);
    assert_eq!(saved.agent_owner_id.as_deref(), Some(f.org_id.as_str()));
    assert!(saved.channel.is_none() && !saved.guest_turn);
    let reply_origin = saved.reply_channel.as_ref().unwrap();
    assert_eq!(reply_origin.nyxbot_channel_id, f.row.id);
    assert_eq!(reply_origin.partition, PARTITION);
    assert_eq!(
        reply_origin.org_execution.as_ref().unwrap().actor_id,
        f.member
    );
    let messages: Vec<AssistantMessage> = db
        .collection(MESSAGES)
        .find(doc! {"conversation_id":&f.home.id,"turn_id":turn})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        messages.len(),
        2,
        "one user input and one settled reply per source"
    );
    let user = messages.iter().find(|m| m.role == "user").unwrap();
    assert_eq!(user.text, source.text);
    assert_eq!(user.via.as_deref(), Some("telegram"));
    let answer = messages.iter().find(|m| m.role == "assistant").unwrap();
    assert_eq!(answer.status, if failed { "failed" } else { "completed" });
    {
        let observed = f.calls.lock().await;
        assert_eq!(observed.len(), index + 1);
        let call = &observed[index];
        assert_eq!(call["actor"], f.member);
        assert_eq!(call["home"], f.home.id);
        assert_eq!(call["key_id"], f.home.credential_api_key_id);
        assert_eq!(call["turn_id"], *turn);
        assert_eq!(call["event_id"], admitted.id);
        assert_eq!(call["context"]["source_event_id"], admitted.id);
        let contexts = turn_context_tests::contexts(call);
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0]["sender"]["classification"], "org_member");
        assert_eq!(contexts[0]["identity"]["platform"], "telegram");
        assert_eq!(
            contexts[0]["identity"]["channel_bot_id"],
            f.row.channel_bot_id
        );
        assert_eq!(contexts[0]["conversation"]["kind"], "private");
    }
    let call = f.calls.lock().await[index].clone();
    let engine_session_id = call["engine_session_id"].as_str().unwrap();
    let engine_response_id = call["engine_response_id"].as_str().unwrap();
    assert!(call["input"].as_str().unwrap().ends_with(&source.text));
    if index == 0 || failed {
        assert!(call["conversation"].is_null());
    }
    let reset_after_failure = index == 1 && source.text == AFTER_FAILURE;
    assert_eq!(call["reset_recap_has_first"], reset_after_failure);
    if reset_after_failure {
        assert!(
            call["conversation"].is_null(),
            "failed session is not reused"
        );
        assert!(!call["input"].as_str().unwrap().contains(FIRST));
    }
    if failed {
        assert!(saved.nyxagent_session_id.is_none());
        assert!(saved.nyxagent_last_response_id.is_none());
        assert_eq!(saved.context_reset_reason.as_deref(), Some("turn_failed"));
        assert!(
            answer.text.is_empty(),
            "failure has no partial text to hand off"
        );
        assert_eq!(answer.error_code.as_deref(), Some("assistant_unavailable"));
        assert_eq!(
            answer.upstream_error_code.as_deref(),
            Some(UPSTREAM_FAILURE)
        );
        let detail = answer.error_detail.as_ref().unwrap();
        assert_eq!(detail.stage, "stream");
        assert_eq!(detail.turn_id, *turn);
        assert_eq!(detail.response_id.as_deref(), Some(engine_response_id));
    } else {
        assert!(answer.error_code.is_none() && answer.upstream_error_code.is_none());
        assert_eq!(
            saved.nyxagent_session_id.as_deref(),
            Some(engine_session_id)
        );
        assert_eq!(
            saved.nyxagent_last_response_id.as_deref(),
            Some(engine_response_id)
        );
    }
    (turn.clone(), answer.text.clone())
}

/// CMA is external to this harness. Its handoff uses the real relay callback;
/// NyxID's provider handler and stored-source authentication execute normally.
fn gateway_handoff(callback: &relay::CallbackPayload) -> Value {
    let text = callback.content.text.as_deref().unwrap();
    let mut request = event(text, &callback.sender.platform_id, &callback.message_id);
    let activity = &mut request["event_context"]["activity"];
    activity.as_object_mut().unwrap().remove("telegram_input");
    activity["source"]["platform"] = json!(callback.platform);
    activity["source"]["route_id"] = json!(callback.conversation.id);
    activity["conversation"]["id"] = json!(callback.conversation.platform_id);
    activity["conversation"]["kind"] = json!(callback.conversation.conversation_type);
    activity["actor"]["display_name"] = json!(callback.sender.display_name);
    activity["occurred_at"] = json!(callback.timestamp);
    request
}

async fn provider_frames(state: &AppState, key: &str, body: &Value, source: &str) -> Vec<Value> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let response = Box::pin(respond(state, key, body, source)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let frames = frames(&body_text(response).await);
        assert!(
            !frames.is_empty(),
            "provider must return a terminal response"
        );
        frames
    })
    .await
    .expect("DM must settle without lease expiry or a sweep")
}
