//! Real owner ingress -> delegation card -> catalog MCP/proxy write wait ->
//! original owner DM decision -> provider write -> independent catalog read.
use super::*;
use crate::models::approval_request::ApprovalRequest;
use crate::services::assistant_team_service as team;
use crate::services::billing::route_inventory::BillingRoutePolicy;
use crate::services::nyxbot_response_recovery as recovery;
use bson::Document;
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path_regex},
};

#[derive(Default)]
struct Ledger {
    writes: Vec<(String, Value)>,
    target: String,
    control: String,
    reads: Vec<String>,
}

async fn mcp(state: &AppState, raw: &str, name: &str, args: Value) -> Value {
    let request=axum::http::Request::builder().method("POST").uri("/mcp")
        .header("x-api-key",raw).header("content-type","application/json")
        .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}
        }}).to_string())).unwrap();
    let response = Box::pin(crate::handlers::mcp_transport::mcp_post(
        State(state.clone()),
        axum::Extension(BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Mcp,
        )),
        request,
    ))
    .await;
    let text = body_text(response).await;
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("MCP returned invalid JSON: {text}"))
}

fn find_value(value: &Value, key: &str) -> Option<Value> {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|v| find_value(&v, key)),
        Value::Object(fields) => fields
            .get(key)
            .cloned()
            .or_else(|| fields.values().find_map(|v| find_value(v, key))),
        Value::Array(values) => values.iter().find_map(|v| find_value(v, key)),
        _ => None,
    }
}
fn read_sentence(result: &Value) -> String {
    let Some(marker) =
        find_value(result, "read_marker").and_then(|v| v.as_str().map(str::to_owned))
    else {
        return "Independent observation is unavailable; no final value is confirmed.".into();
    };
    let Some(status) = find_value(result, "Status").and_then(|v| v.as_str().map(str::to_owned))
    else {
        return "Independent observation has no target value; no final value is confirmed.".into();
    };
    format!(
        "Base bascnLedger, table tblLedger, record recTarget: Status = {status}. Independent observation {marker}."
    )
}
fn owner_text(stream: &str) -> String {
    stream
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|frame| {
            (frame["type"] == "response.completed").then(|| {
                frame["response"]["output"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|item| item["content"].as_array().into_iter().flatten())
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .unwrap_or_else(|| panic!("no completed owner message: {stream}"))
}
async fn model(
    state: &AppState,
    raw: &str,
    proxy: bool,
    stage: usize,
    observed: &Arc<Mutex<Vec<Value>>>,
) -> String {
    let (_, key, _) = key_service::validate_api_key(&state.db, raw).await.unwrap();
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"credential_api_key_id":&key.id})
        .await
        .unwrap()
        .unwrap();
    if row.is_subagent() {
        assert!(
            row.active_turn
                .as_ref()
                .unwrap()
                .supervisor_source
                .is_some(),
            "delegation must carry original source"
        );
        let args = json!({"app_token":"bascnLedger","table_id":"tblLedger","body":{}});
        let before = mcp(state, raw, "ledger__bitable_records_search", args.clone()).await;
        observed
            .lock()
            .await
            .push(json!({"stage":"before","result":before.clone()}));
        if stage == 0 {
            return format!("Pending ledger entries. {}", read_sentence(&before));
        }
        let written = if proxy {
            use tower::ServiceExt;
            let (_, private) = crate::routes::build_router_with_state(state.clone());
            let request=axum::http::Request::builder().method("PUT")
                .uri("/api/v1/proxy/s/ledger/bitable/v1/apps/bascnLedger/tables/tblLedger/records/recTarget")
                .header("x-api-key",raw).header("content-type","application/json")
                .body(Body::from(json!({"fields":{"Status":"Approved"}}).to_string())).unwrap();
            let response = private
                .with_state(state.clone())
                .oneshot(request)
                .await
                .unwrap();
            json!({"status":response.status().as_u16(),"body":body_text(response).await})
        } else if stage == 2 {
            let (first, second) = tokio::join!(
                mcp(
                    state,
                    raw,
                    "ledger__bitable_record_update",
                    json!({"app_token":"bascnLedger","table_id":"tblLedger",
                    "record_id":"recTarget","body":{"fields":{"Status":"Approved"}}})
                ),
                mcp(
                    state,
                    raw,
                    "ledger__bitable_record_update",
                    json!({"app_token":"bascnLedger","table_id":"tblLedger",
                    "record_id":"recControl","body":{"fields":{"Status":"Approved"}}})
                )
            );
            json!({"first":first,"second":second})
        } else {
            mcp(
                state,
                raw,
                "ledger__bitable_record_update",
                json!({"app_token":"bascnLedger","table_id":"tblLedger",
                "record_id":"recTarget","body":{"fields":{"Status":"Approved"}}}),
            )
            .await
        };
        observed
            .lock()
            .await
            .push(json!({"stage":"write","result":written}));
        let after = mcp(state, raw, "ledger__bitable_records_search", args).await;
        observed
            .lock()
            .await
            .push(json!({"stage":"independent_readback","result":after.clone()}));
        return read_sentence(&after);
    }
    // Event reports consume results; they never reassign completed work.
    if row.active_turn.as_ref().unwrap().origin
        == crate::models::assistant_conversation::TurnOrigin::Event
    {
        return serde_json::to_string(
            &row.active_turn
                .as_ref()
                .unwrap()
                .events
                .iter()
                .map(|e| &e.text)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    }
    let task = if stage == 0 {
        "List pending ledger rows and return the real observed values."
    } else {
        "Update recTarget Status from Pending to Approved; read back independently."
    };
    let mut args = json!({"subagent":"accountant","text":task});
    if let Some(card)=state.db.collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>("assistant_acknowledgements")
        .find_one(doc! {"conversation_id":&row.id,"status":"allowed","tool_name":"nyxid__message_subagent"})
        .sort(doc! {"created_at":-1}).await.unwrap() {
        args["acknowledgement_id"]=json!(card.id);
    }
    let result = mcp(state, raw, "nyxid__message_subagent", args).await;
    observed
        .lock()
        .await
        .push(json!({"stage":"delegation","result":result.clone()}));
    if state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .count_documents(doc! {
            "conversation_id":&row.id,"status":"pending"
        })
        .await
        .unwrap()
        > 0
    {
        return format!("Review the exact accountant assignment: {result}");
    }
    let _finished = mcp(
        state,
        raw,
        "nyxid__wait_for_subagents",
        json!({"subagents":["accountant"],"timeout_secs":20}),
    )
    .await;
    let evidence = observed.lock().await;
    let latest = evidence
        .iter()
        .rev()
        .find(|r| r["stage"] == "independent_readback" || r["stage"] == "before")
        .expect("specialist must observe ledger");
    format!("Accountant report. {}", read_sentence(&latest["result"]))
}

struct Journey {
    state: AppState,
    channel: NyxbotChannel,
    key: String,
    chat: MockServer,
    _ledger_server: MockServer,
    ledger: Arc<std::sync::Mutex<Ledger>>,
    observed: Arc<Mutex<Vec<Value>>>,
    stage: Arc<AtomicUsize>,
    _engine: tokio_util::task::AbortOnDropHandle<()>,
}

async fn setup_journey(slug: &str, proxy: bool) -> Journey {
    setup_transport_journey(slug, proxy, "relay").await
}
async fn setup_transport_journey(slug: &str, proxy: bool, transport: &str) -> Journey {
    let direct = transport == "direct";
    let native = transport == "native";
    let runtime = Arc::new(OnceLock::<AppState>::new());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let stage = Arc::new(AtomicUsize::new(0));
    let active = runtime.clone();
    let evidence = observed.clone();
    let active_stage = stage.clone();
    let router=Router::new().route("/v1/responses",post(move |headers:HeaderMap| {
        let state=active.get().unwrap().clone(); let evidence=evidence.clone(); let stage=active_stage.load(Ordering::SeqCst);
        async move {
            let raw=headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let answer=Box::pin(model(&state,raw,proxy,stage,&evidence)).await;
            let session=format!("conv_{}",Uuid::new_v4().simple());
            let response=format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple());
            let stream=async_stream::stream! {
                yield Ok::<_,Infallible>(Event::default().data(json!({"type":"response.completed","sequence_number":1,
                    "response":{"id":response,"conversation":{"id":session},"status":"completed","output":message_output(&answer)}}).to_string()));
            };
            axum::response::Sse::new(stream).into_response()
        }
    }));
    let db = connect_transaction_test_database("ws7_supervisor_write").await;
    let (state, _, engine) = Box::pin(setup_with_engine(db, Value::Null, Some(router))).await;
    assert!(runtime.set(state.clone()).is_ok());
    crate::services::assistant_settings_service::update(
        &state.db,
        OWNER,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ledger = Arc::new(std::sync::Mutex::new(Ledger {
        target: "Pending".into(),
        control: "Untouched".into(),
        ..Default::default()
    }));
    let ledger_server = MockServer::start().await;
    let read = ledger.clone();
    Mock::given(method("POST")).and(path_regex(".*/bitable/v1/apps/bascnLedger/tables/tblLedger/records/search$"))
        .respond_with(move |_:&wiremock::Request| {
            let mut r=read.lock().unwrap(); let value=r.target.clone();r.reads.push(value);
            ResponseTemplate::new(200).set_body_json(json!({"code":0,"read_marker":format!("provider-search-{}",r.reads.len()),"data":{"items":[
                {"record_id":"recTarget","fields":{"Status":r.target}},
                {"record_id":"recControl","fields":{"Status":r.control}}
            ],"has_more":false}}))
        }).mount(&ledger_server).await;
    let written = ledger.clone();
    Mock::given(method("PUT"))
        .and(path_regex(
            ".*/bitable/v1/apps/bascnLedger/tables/tblLedger/records/[^/]+$",
        ))
        .respond_with(move |request: &wiremock::Request| {
            let record = request.url.path().rsplit('/').next().unwrap().to_owned();
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let mut r = written.lock().unwrap();
            r.writes.push((record.clone(), body.clone()));
            if record == "recTarget" {
                r.target = body["fields"]["Status"].as_str().unwrap().into();
            } else {
                r.control = body["fields"]["Status"].as_str().unwrap().into();
            }
            // The write response deliberately omits values. Only a separate
            // actual provider search can supply final-value evidence.
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"record":{"record_id":record}}}))
        })
        .mount(&ledger_server)
        .await;
    let catalog = Uuid::new_v4().to_string();
    let endpoint = Uuid::new_v4().to_string();
    let connection = Uuid::new_v4().to_string();
    let base = if slug.ends_with("-bot") {
        ledger_server.uri()
    } else {
        format!("{}/open-apis", ledger_server.uri())
    };
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = catalog.clone();
    service.slug = slug.into();
    service.name = "Merchant test ledger".into();
    service.base_url = base.clone();
    service.created_by = "system".into();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&state.db)
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint,
            OWNER,
            "Merchant ledger",
            &base,
            None,
            Some(&catalog),
        ))
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(crate::test_utils::test_user_service(
            &connection,
            OWNER,
            "ledger",
            &endpoint,
            Some(&catalog),
            None,
        ))
        .await
        .unwrap();
    let targets = team::resolve_targets(
        &state.db,
        &state.node_ws_manager,
        OWNER,
        OWNER,
        &["ledger".into()],
    )
    .await
    .unwrap();
    let (agent, _) = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        team::CreateRequest {
            name: "accountant".into(),
            description: "Test ledger accountant".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets,
            account_read: false,
            specialty: None,
            created_by: "user",
            machines: None,
            logins: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    team::set_agent_policy(
        &state.db,
        &crate::services::audit_service::AuditActor {
            user_id: OWNER.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        &agent.id,
        team::AgentPolicy {
            model: None,
            access_mode_override: Some(Some(
                crate::models::assistant_settings::AccessMode::Nyxmode,
            )),
        },
    )
    .await
    .unwrap();
    let (channel, key) = if native {
        native_tests::native_channel(&state).await
    } else {
        channel(&state, if direct { "direct" } else { "gateway" }).await
    };
    let sender = if direct { "ou_owner" } else { "7" };
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&channel.id},
            doc! {"$set":{"owner_sender_ids":[sender]}},
        )
        .await
        .unwrap();
    if !direct && !native {
        assert_eq!(
            put_binding(
                State(state.clone()),
                Path("bnd_ws7".into()),
                bearer(&key),
                Json(binding_body(&key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            put_conversation(
                State(state.clone()),
                Path(("bnd_ws7".into(), PARTITION.into())),
                bearer(&key)
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    let chat = MockServer::start().await;
    let encrypted = state
        .encryption_keys
        .encrypt(if direct { b"app:secret" } else { b"ws7-token" })
        .await
        .unwrap();
    state
        .db
        .collection::<Document>("channel_bots")
        .update_one(
            doc! {"_id":&channel.channel_bot_id},
            doc! {"$set":{"bot_token_encrypted":bson::Binary {
                subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted
            }}},
        )
        .await
        .unwrap();
    if direct {
        Mock::given(path_regex(".*/tenant_access_token/internal$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"code":0,"tenant_access_token":"ws7-tenant","expire":7200}),
                ),
            )
            .mount(&chat)
            .await;
        Mock::given(path_regex(".*/im/v1/messages$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"ws7-receipt"}})),
            )
            .mount(&chat)
            .await;
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .insert(
                channel.channel_bot_id.clone(),
                Arc::new(
                    crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                        &chat.uri(),
                        "lark",
                    ),
                ),
            );
    } else {
        Mock::given(path_regex(".*/sendMessage$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":91}})),
            )
            .mount(&chat)
            .await;
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            channel.channel_bot_id.clone(),
            Arc::new(
                crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                    &chat.uri(),
                ),
            ),
        );
    }
    // Match normal gateway primary delivery as well as the separate local,
    // source-anchored Telegram supplemental review.
    let mut gateway = crate::models::downstream_service::test_helpers::dummy_service();
    gateway.id = Uuid::new_v4().to_string();
    gateway.slug = GATEWAY_SLUG.into();
    gateway.base_url = chat.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(gateway)
        .await
        .unwrap();
    Mock::given(path_regex("/events/.*/reply-target$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.ws7","expires_at":(Utc::now()+ChronoDuration::hours(1)).timestamp()}))).mount(&chat).await;
    Mock::given(path_regex("/reply-targets/.*/messages$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .mount(&chat)
        .await;
    if native {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(
                "/events/opaque-native-ref/side-messages",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"available":true})))
            .mount(&chat)
            .await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::path(
                "/events/opaque-native-ref/side-messages",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
            )
            .mount(&chat)
            .await;
    }
    Journey {
        state,
        channel,
        key,
        chat,
        _ledger_server: ledger_server,
        ledger,
        observed,
        stage,
        _engine: tokio_util::task::AbortOnDropHandle::new(engine),
    }
}

async fn owner(j: &Journey, text: &str, id: &str) -> Response {
    // The authenticated relay persists full inbound identity before invoking
    // NyxBot. This is the original-message anchor used by real supplemental sends.
    j.state.db.collection::<Document>("channel_messages").update_one(doc! {"_id":id},doc! {"$setOnInsert":{
        "conversation_id":&j.channel.route_id,"user_id":OWNER,"channel_bot_id":&j.channel.channel_bot_id,
        "platform":"telegram","platform_conversation_id":"123","direction":"inbound","platform_message_id":id,
        "sender_platform_id":"7","original_message":true,"content_type":"text",
        "agent_api_key_id":&j.channel.route_api_key_id,"created_at":bson::DateTime::now()
    }}).upsert(true).await.unwrap();
    let mut input = event(text, "7", id);
    input["event_context"]["activity"]["conversation"]["id"] = json!("123");
    respond(&j.state, &j.key, &input, id).await
}

fn sent_text(body: &Value) -> Option<String> {
    body["text"].as_str().map(str::to_owned).or_else(|| {
        body["content"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v["text"].as_str().map(str::to_owned))
    })
}

async fn wait_review(j: &Journey) -> (ApprovalRequest, String) {
    tokio::time::timeout(std::time::Duration::from_secs(15),async {
        loop {
            if let Some(row)=j.state.db.collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"user_id":OWNER,"supervisor_write.review_confirmed":true,"status":"pending"}).await.unwrap() {
                for request in j.chat.received_requests().await.unwrap() {
                    if let Ok(body)=serde_json::from_slice::<Value>(&request.body)
                        && let Some(text)=sent_text(&body)
                        && let Some((_,tail))=text.split_once("yes write-") {
                        let code=tail.split_whitespace().next().unwrap().to_owned();
                        assert!(text.contains("recTarget") && text.contains("Approved") && text.contains("Merchant ledger"),"{text}");
                        return (row,code);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("complete review must reach the originating Telegram chat")
}

async fn assign_from_sent_card(
    j: &Journey,
    text: &str,
    id: &str,
) -> tokio::task::JoinHandle<String> {
    let initial = owner_text(&body_text(owner(j, text, &format!("{id}-ask")).await).await);
    let code = regex::Regex::new(r"yes ([0-9]{4})")
        .unwrap()
        .captures(&initial)
        .expect("code in the sent assignment card")[1]
        .to_owned();
    let response = owner(j, &format!("yes {code}"), &format!("{id}-delegation-yes")).await;
    tokio::spawn(body_text(response))
}
async fn start(j: &Journey, write_stage: usize) -> tokio::task::JoinHandle<String> {
    let listing = assign_from_sent_card(
        j,
        "Ask accountant for today's pending ledger rows",
        "ws7-list",
    )
    .await;
    let listed = owner_text(
        &tokio::time::timeout(std::time::Duration::from_secs(25), listing)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(
        listed.contains("recTarget")
            && listed.contains("Pending")
            && listed.contains("provider-search-1"),
        "actual owner list message: {listed}"
    );
    assert_eq!(j.ledger.lock().unwrap().writes.len(), 0);
    j.stage.store(write_stage, Ordering::SeqCst);
    assign_from_sent_card(
        j,
        "Choose Base bascnLedger table tblLedger record recTarget: Status Pending -> Approved",
        "ws7-write",
    )
    .await
}

#[tokio::test]
async fn supervisor_write_two_approvals_real_catalog_and_independent_readback() {
    for (slug, proxy) in [
        ("api-feishu", false),
        ("api-feishu-bot", false),
        ("api-feishu", true),
    ] {
        let j = Box::pin(setup_journey(slug, proxy)).await;
        assert_eq!(
            crate::services::approval_service::user_approval_mode(&j.state.db, OWNER)
                .await
                .unwrap(),
            crate::services::approval_policy::GlobalApprovalMode::WritesOnly
        );
        assert_eq!(
            j.state
                .db
                .collection::<Document>("service_approval_configs")
                .count_documents(doc! {"user_id":OWNER})
                .await
                .unwrap(),
            0,
            "implicit mode must work"
        );
        let result = start(&j, 1).await;
        let (review, code) = wait_review(&j).await;
        assert_eq!(
            review.approval_mode,
            crate::models::service_approval_config::ApprovalMode::PerRequest
        );
        assert!(review.execution_proposal.is_none());
        assert_eq!(j.ledger.lock().unwrap().writes.len(), 0);
        let decision =
            body_text(owner(&j, &format!("yes write-{code}"), "ws7-write-yes").await).await;
        assert!(decision.contains("Write approval saved"), "{decision}");
        let final_text = tokio::time::timeout(std::time::Duration::from_secs(25), result)
            .await
            .unwrap()
            .unwrap();
        {
            let ledger = j.ledger.lock().unwrap();
            assert_eq!(ledger.writes.len(), 1);
            assert_eq!(ledger.writes[0].0, "recTarget");
            assert_eq!(ledger.target, "Approved");
            assert_eq!(ledger.control, "Untouched");
            assert_eq!(
                ledger.reads,
                ["Pending", "Pending", "Approved"],
                "provider search must read after the write"
            );
        }
        assert!(
            j.observed
                .lock()
                .await
                .iter()
                .any(|r| r["stage"] == "independent_readback"
                    && r["result"].to_string().contains("Approved"))
        );
        let final_message = owner_text(&final_text);
        assert!(
            final_message.contains("Approved") && final_message.contains("provider-search-3"),
            "actual owner final message: {final_message}"
        );
        let replay =
            body_text(owner(&j, &format!("yes write-{code}"), "ws7-write-yes").await).await;
        assert!(!replay.is_empty());
        assert_eq!(j.ledger.lock().unwrap().writes.len(), 1);
        let stored = j
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"_id":&review.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            stored
                .supervisor_write
                .unwrap()
                .dm_decision_receipt
                .is_some()
        );
        assert_eq!(
            j.state
                .db
                .collection::<Document>("approval_grants")
                .count_documents(doc! {"user_id":OWNER})
                .await
                .unwrap(),
            0
        );
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&j.channel.channel_bot_id);
    }
}

#[tokio::test]
async fn supervisor_write_rejection_and_legacy_approval_never_send() {
    for legacy in [false, true] {
        let j = Box::pin(setup_journey("api-feishu-bot", false)).await;
        let result = start(&j, 1).await;
        let (review, code) = wait_review(&j).await;
        if legacy {
            j.state
                .db
                .collection::<Document>("approval_requests")
                .update_one(
                    doc! {"_id":&review.id},
                    doc! {"$set":{"status":"approved","decision_channel":"web"}},
                )
                .await
                .unwrap();
        } else {
            let denied =
                body_text(owner(&j, &format!("no write-{code}"), "ws7-write-no").await).await;
            assert!(denied.contains("Write rejected"), "{denied}");
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(25), result)
            .await
            .unwrap()
            .unwrap();
        {
            let ledger = j.ledger.lock().unwrap();
            assert!(ledger.writes.is_empty());
            assert_eq!(ledger.target, "Pending");
            assert_eq!(ledger.control, "Untouched");
        }
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&j.channel.channel_bot_id);
    }
}

#[tokio::test]
async fn supervisor_write_wrong_chat_sender_expiry_and_revocation_never_send() {
    for control in ["wrong_chat", "wrong_sender", "expired", "revoked_key"] {
        let j = Box::pin(setup_journey("api-feishu", false)).await;
        let result = start(&j, 1).await;
        let (review, code) = wait_review(&j).await;
        if control == "wrong_chat" {
            let other = "conv_11111111111111111111111111111111";
            assert_eq!(
                put_conversation(
                    State(j.state.clone()),
                    Path(("bnd_ws7".into(), other.into())),
                    bearer(&j.key)
                )
                .await
                .status(),
                StatusCode::OK
            );
            let mut input = event(&format!("yes write-{code}"), "7", "ws7-wrong-chat");
            input["conversation"] = json!(other);
            input["event_context"]["conversation_id"] = json!(other);
            input["event_context"]["activity"]["conversation"]["id"] = json!("456");
            let _ = body_text(respond(&j.state, &j.key, &input, "ws7-wrong-chat").await).await;
        } else if control == "wrong_sender" {
            let mut input = event(&format!("yes write-{code}"), "8", "ws7-wrong-sender");
            input["event_context"]["activity"]["conversation"]["id"] = json!("123");
            let _ = body_text(respond(&j.state, &j.key, &input, "ws7-wrong-sender").await).await;
        } else if control == "expired" {
            j.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&review.id},doc! {
                "$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}
            }).await.unwrap();
            let _ =
                body_text(owner(&j, &format!("yes write-{code}"), "ws7-expired-decision").await)
                    .await;
        } else {
            let binding = review.supervisor_write.as_ref().unwrap();
            j.state
                .db
                .collection::<Document>("api_keys")
                .update_one(
                    doc! {"_id":&binding.specialist_key},
                    doc! {
                        "$set":{"allowed_service_ids":[],"allow_all_services":false}
                    },
                )
                .await
                .unwrap();
            let _ =
                body_text(owner(&j, &format!("yes write-{code}"), "ws7-revoked-decision").await)
                    .await;
        }
        let current = j
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"_id":&review.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            current
                .supervisor_write
                .as_ref()
                .unwrap()
                .dm_decision_receipt
                .is_none(),
            "{control} must not create a DM receipt"
        );
        assert_ne!(current.status, "approved");
        assert!(j.ledger.lock().unwrap().writes.is_empty(), "{control}");
        // Advance only this fixture's request expiry to settle the connected
        // waiter promptly. There is no shared clock, DB or production effect.
        j.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&review.id},doc! {
            "$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}
        }).await.unwrap();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(25), result)
            .await
            .unwrap()
            .unwrap();
        {
            let ledger = j.ledger.lock().unwrap();
            assert!(ledger.writes.is_empty(), "{control}");
            assert_eq!(ledger.target, "Pending");
            assert_eq!(ledger.control, "Untouched");
        }
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&j.channel.channel_bot_id);
    }
}

async fn all_reviews(j: &Journey, count: usize) -> Vec<(ApprovalRequest, String)> {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let rows = j
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find(doc! {
                    "user_id":OWNER,"supervisor_write.review_confirmed":true,"status":"pending"
                })
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            let mut found = Vec::new();
            for request in j.chat.received_requests().await.unwrap() {
                if let Ok(body) = serde_json::from_slice::<Value>(&request.body)
                    && let Some(text) = body["text"].as_str()
                    && let Some((_, tail)) = text.split_once("yes write-")
                {
                    assert_eq!(
                        body["chat_id"].as_str(),
                        Some("123"),
                        "review must use the original owner chat"
                    );
                    let code = tail.split_whitespace().next().unwrap().to_owned();
                    let digest = crate::services::audit_service::keyed_fingerprint(
                        crate::services::audit_service::audit_chain_hmac_key(),
                        b"supervisor-code-v1",
                        code.as_bytes(),
                    )
                    .unwrap();
                    if let Some(row) = rows
                        .iter()
                        .find(|r| r.supervisor_write.as_ref().unwrap().code_digest == digest)
                    {
                        found.push((row.clone(), code));
                    }
                }
            }
            if found.len() == count {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("all distinct exact-write reviews must reach the original chat")
}

#[tokio::test]
async fn supervisor_write_parallel_occurrences_cannot_borrow_a_decision() {
    let j = Box::pin(setup_journey("api-feishu-bot", false)).await;
    let result = start(&j, 2).await;
    let mut reviews = all_reviews(&j, 2).await;
    reviews.sort_by_key(|(r, _)| r.resource.clone());
    let (control, control_code) = reviews
        .iter()
        .find(|(r, _)| r.resource.as_ref().unwrap().contains("recControl"))
        .unwrap();
    let (target, target_code) = reviews
        .iter()
        .find(|(r, _)| r.resource.as_ref().unwrap().contains("recTarget"))
        .unwrap();
    assert_ne!(control.id, target.id);
    assert_ne!(control_code, target_code);
    let accepted =
        body_text(owner(&j, &format!("yes write-{target_code}"), "ws7-parallel-yes").await).await;
    assert!(accepted.contains("Write approval saved"));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while j.ledger.lock().unwrap().writes.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let _ =
        body_text(owner(&j, &format!("yes write-{target_code}"), "ws7-parallel-yes").await).await;
    assert_eq!(j.ledger.lock().unwrap().writes.len(), 1);
    let pending = j
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"_id":&control.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, "pending");
    assert!(
        pending
            .supervisor_write
            .unwrap()
            .dm_decision_receipt
            .is_none()
    );
    let denied =
        body_text(owner(&j, &format!("no write-{control_code}"), "ws7-parallel-no").await).await;
    assert!(denied.contains("Write rejected"));
    let final_text = tokio::time::timeout(std::time::Duration::from_secs(25), result)
        .await
        .unwrap()
        .unwrap();
    assert!(owner_text(&final_text).contains("provider-search-3"));
    {
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 1);
        assert_eq!(ledger.target, "Approved");
        assert_eq!(ledger.control, "Untouched");
    }
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&j.channel.channel_bot_id);
}

fn direct_message(j: &Journey, text: &str, id: &str) -> tokio::task::JoinHandle<()> {
    let state = j.state.clone();
    let channel = j.channel.clone();
    let text = text.to_owned();
    let id = id.to_owned();
    tokio::spawn(async move {
        let (_observer, completed) = watch_relay_callback(&channel.id, &id);
        assert!(
            direct_private(&state, &channel, &id, "ou_owner", &text)
                .await
                .is_success()
        );
        tokio::time::timeout(std::time::Duration::from_secs(30), completed)
            .await
            .unwrap()
            .unwrap();
    })
}

async fn direct_texts(j: &Journey) -> Vec<String> {
    j.chat
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| {
            let body = serde_json::from_slice::<Value>(&r.body).ok()?;
            let text = sent_text(&body)?;
            assert_eq!(
                body["receive_id"], "oc_ou_owner",
                "every owner-facing message must use the original Feishu/Lark DM"
            );
            Some(text)
        })
        .collect()
}

fn delegation_code(text: &str) -> String {
    regex::Regex::new(r"yes ([0-9]{4})")
        .unwrap()
        .captures(text)
        .expect("owner-facing delegation code")[1]
        .to_owned()
}

#[tokio::test]
async fn supervisor_write_signed_feishu_relay_returns_list_review_and_verified_result_to_same_dm() {
    let j = Box::pin(setup_transport_journey("api-feishu-bot", false, "direct")).await;
    direct_message(
        &j,
        "Ask accountant for today's pending ledger rows",
        "ws7-direct-list",
    )
    .await
    .unwrap();
    let code = delegation_code(direct_texts(&j).await.last().unwrap());
    direct_message(&j, &format!("yes {code}"), "ws7-direct-list-yes")
        .await
        .unwrap();
    let messages = direct_texts(&j).await;
    assert!(
        messages.iter().any(|text| text.contains("recTarget")
            && text.contains("Pending")
            && text.contains("provider-search-1")),
        "actual DM list: {messages:?}"
    );
    assert!(j.ledger.lock().unwrap().writes.is_empty());
    j.stage.store(1, Ordering::SeqCst);
    direct_message(
        &j,
        "Choose Base bascnLedger table tblLedger record recTarget: Status Pending -> Approved",
        "ws7-direct-write",
    )
    .await
    .unwrap();
    let code = delegation_code(direct_texts(&j).await.last().unwrap());
    let completed = direct_message(
        &j,
        &format!("yes {code}"),
        "ws7-direct-write-assignment-yes",
    );
    let (review, write_code) = wait_review(&j).await;
    let binding = review.supervisor_write.as_ref().unwrap();
    let private_metadata = serde_json::to_string(binding).unwrap();
    assert!(!private_metadata.contains(&write_code));
    assert!(!private_metadata.contains("Approved"));
    assert!(!private_metadata.contains("http://"));
    let source = j
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&binding.source.channel.event_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.delivery.as_ref().unwrap().sender_id, "ou_owner");
    assert_eq!(source.delivery.as_ref().unwrap().origin.platform, "lark");
    direct_message(
        &j,
        &format!("yes write-{write_code}"),
        "ws7-direct-write-yes",
    )
    .await
    .unwrap();
    completed.await.unwrap();
    let messages = direct_texts(&j).await;
    assert!(
        messages.iter().any(|text| text.contains("recTarget")
            && text.contains("Approved")
            && text.contains("provider-search-3")),
        "actual DM result: {messages:?}"
    );
    direct_message(
        &j,
        &format!("yes write-{write_code}"),
        "ws7-direct-write-yes",
    )
    .await
    .unwrap();
    {
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 1);
        assert_eq!(ledger.target, "Approved");
        assert_eq!(ledger.control, "Untouched");
        assert_eq!(ledger.reads, ["Pending", "Pending", "Approved"]);
    }
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&j.channel.channel_bot_id);
}

#[tokio::test]
async fn supervisor_write_review_respects_a_configured_timeout_below_eighty_seconds() {
    let j = Box::pin(setup_journey("api-feishu", false)).await;
    crate::services::notification_service::get_or_create_channel(&j.state.db, OWNER)
        .await
        .unwrap();
    j.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"user_id":OWNER},
            doc! {
                "$set":{"approval_timeout_secs":30}
            },
        )
        .await
        .unwrap();
    let result = start(&j, 1).await;
    let (review, code) = wait_review(&j).await;
    assert!(review.expires_at <= review.created_at + ChronoDuration::seconds(30));
    assert!(
        review.expires_at
            <= review
                .supervisor_write
                .as_ref()
                .unwrap()
                .source
                .channel
                .expires_at
    );
    let denied =
        body_text(owner(&j, &format!("no write-{code}"), "ws7-short-timeout-no").await).await;
    assert!(denied.contains("Write rejected"));
    let _ = tokio::time::timeout(std::time::Duration::from_secs(25), result)
        .await
        .unwrap()
        .unwrap();
    assert!(j.ledger.lock().unwrap().writes.is_empty());
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&j.channel.channel_bot_id);
}

async fn native_owner(j: &Journey, text: &str, id: &str) -> Response {
    respond(
        &j.state,
        &j.key,
        &native_tests::native_event(&j.channel, id, text),
        &format!("ws7-native-{id}"),
    )
    .await
}

async fn retained_event_context(state: &AppState, event: &NyxbotEvent) -> Value {
    let context = crate::services::nyxbot_event_record::event_context(
        &state.encryption_keys,
        event,
        event.event_context_ciphertext.as_deref().unwrap(),
    )
    .await
    .unwrap();
    serde_json::from_slice(&context).unwrap()
}

#[tokio::test]
async fn supervisor_write_native_new_reply_event_decides_original_write_and_preserves_result_source()
 {
    let j = Box::pin(setup_transport_journey("api-feishu-bot", false, "native")).await;
    let initial = owner_text(
        &body_text(native_owner(&j, "Ask accountant for today's pending ledger rows", "801").await)
            .await,
    );
    let first_code = delegation_code(&initial);
    let list =
        owner_text(&body_text(native_owner(&j, &format!("yes {first_code}"), "802").await).await);
    assert!(
        list.contains("recTarget")
            && list.contains("Pending")
            && list.contains("provider-search-1"),
        "native owner list: {list}"
    );
    j.stage.store(1, Ordering::SeqCst);
    let assignment=owner_text(&body_text(native_owner(&j,"Choose Base bascnLedger table tblLedger record recTarget: Status Pending -> Approved","803").await).await);
    let code = delegation_code(&assignment);
    let response = native_owner(&j, &format!("yes {code}"), "804").await;
    let result = tokio::spawn(body_text(response));
    let (review, write_code) = wait_review(&j).await;
    let original = &review.supervisor_write.as_ref().unwrap().source.channel;
    let decision = native_owner(
        &j,
        &format!("Yes write-{}", write_code.to_ascii_uppercase()),
        "805",
    )
    .await;
    let response_id = decision.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let decision_text = owner_text(&body_text(decision).await);
    assert!(
        decision_text.contains("Write approval saved"),
        "{decision_text}"
    );
    let reply = recovery::load(&j.state.db, &j.channel, &response_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        reply.event_key, original.event_id,
        "the owner reply has a new provider event identity"
    );
    let decided_event = j
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&reply.event_key})
        .await
        .unwrap()
        .unwrap();
    let source_event = j
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&original.event_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source_event.channel_id, decided_event.channel_id);
    assert_eq!(source_event.user_id, decided_event.user_id);
    assert_eq!(source_event.partition, decided_event.partition);
    assert_ne!(source_event.event_id, decided_event.event_id);
    // An inline decision finishes the native response without admitting a new
    // assistant turn or creating turn-delivery state. Its authenticated source
    // identity is retained in the encrypted ingress context instead.
    assert!(reply.terminal);
    assert!(decided_event.turn_id.is_none());
    assert!(decided_event.delivery.is_none());
    let source = retained_event_context(&j.state, &source_event).await;
    let current = retained_event_context(&j.state, &decided_event).await;
    assert_eq!(current["activity"]["event_id"], "discord:100:805");
    assert_eq!(current["activity"]["actor"]["kind"], "human");
    assert_eq!(current["activity"]["actor"]["id"], "7");
    assert_eq!(current["activity"]["conversation"]["id"], "200");
    for field in ["source", "actor", "conversation"] {
        assert_eq!(source["activity"][field], current["activity"][field]);
    }
    let final_text = owner_text(
        &tokio::time::timeout(std::time::Duration::from_secs(25), result)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(
        final_text.contains("Approved") && final_text.contains("provider-search-3"),
        "native owner result: {final_text}"
    );
    {
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 1);
        assert_eq!(ledger.target, "Approved");
        assert_eq!(ledger.control, "Untouched");
        assert_eq!(ledger.reads, ["Pending", "Pending", "Approved"]);
    }
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&j.channel.channel_bot_id);
}
