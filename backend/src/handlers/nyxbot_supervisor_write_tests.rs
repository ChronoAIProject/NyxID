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

#[path = "nyxbot_supervisor_marked_tests.rs"]
mod marked_tests;

#[derive(Default)]
struct Ledger {
    writes: Vec<(String, Value)>,
    rejected: Vec<String>,
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

async fn raw_update(state: &AppState, raw: &str, selected: &str, body: &str) -> Value {
    raw_update_with_headers(state, raw, selected, body, &[]).await
}

async fn raw_update_with_headers(
    state: &AppState,
    raw: &str,
    selected: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> Value {
    use tower::ServiceExt;
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let mut request = axum::http::Request::builder()
        .method("PUT")
        .uri(format!(
            "/api/v1/proxy/s/ledger/bitable/v1/apps/bascnLedger/tables/tblLedger/records/{selected}"
        ))
        .header("x-api-key", raw)
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Body::from(body.to_owned())).unwrap();
    let response = private
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    json!({"status":response.status().as_u16(),"body":body_text(response).await})
}

fn update_arguments(record: &str, fields: Value) -> Value {
    // The published update tool flattens its required JSON object body. The
    // optional search body below uses the separately published wrapper shape.
    json!({"app_token":"bascnLedger","table_id":"tblLedger",
        "record_id":record,"fields":fields})
}

fn tool_value(result: &Value) -> Option<Value> {
    let result = result.get("result").unwrap_or(result);
    if result["isError"] == true {
        return None;
    }
    result["content"]
        .as_array()?
        .iter()
        .filter(|part| part["type"] == "text")
        .find_map(|part| serde_json::from_str(part["text"].as_str()?).ok())
}

fn row_observation(result: &Value, selected: &str) -> Option<String> {
    row_observation_at(result, "bascnLedger", "tblLedger", selected)
}

fn row_observation_at(result: &Value, base: &str, table: &str, selected: &str) -> Option<String> {
    let response = tool_value(result)?;
    if response["code"] != 0 {
        return None;
    }
    let record = response["data"]["items"]
        .as_array()?
        .iter()
        .find(|record| record["record_id"] == selected)?;
    let id = record["record_id"].as_str()?;
    let status = record["fields"]["Status"].as_str()?;
    let marker = record["fields"]["Observation"].as_str()?;
    Some(format!(
        "Base {base}, table {table}, record {id}: Status = {status}; independent observation {marker}."
    ))
}

const NO_CONFIRMATION: &str =
    "The selected row has no matching specialist observation; no final value is confirmed.";

fn retained_readback_target(input: &str) -> Option<(String, String, String)> {
    let target = regex::Regex::new(r"NyxID has settled the retained write attempt for Base ([A-Za-z0-9]{1,64}), table ([A-Za-z0-9]{1,64}), record ([A-Za-z0-9]{1,64})\.").unwrap();
    let captures = target
        .captures_iter(input)
        .last()
        .filter(|_| input.contains("Independently read this exact record"))?;
    Some((
        captures[1].to_owned(),
        captures[2].to_owned(),
        captures[3].to_owned(),
    ))
}
fn checked_report(report: Option<&str>, selected: &str) -> String {
    report
        .filter(|report| {
            let observation = report
                .rsplit_once(" After: ")
                .map_or(*report, |(_, after)| after);
            observation.contains(&format!("record {selected}: Status = "))
                && observation.contains(&format!(":{selected}."))
        })
        .map_or_else(
            || NO_CONFIRMATION.into(),
            |report| format!("Accountant report. {report}"),
        )
}

fn delegation_review(text: &str) -> Option<Value> {
    let rpc: Value =
        serde_json::from_str(text.strip_prefix("Review the exact accountant assignment: ")?)
            .ok()?;
    rpc["result"]["content"]
        .as_array()?
        .iter()
        .find_map(|part| {
            let card: Value = serde_json::from_str(part["text"].as_str()?).ok()?;
            (card["error"] == "acknowledgement_required"
                && card["kind"] == "action"
                && card["status"] == "pending")
                .then_some(card)
        })
}

/// The model sees the current decision ID in NyxID's leading context block.
/// A new owner message can have a new task root; only its actual coded reply
/// and current allowed-card note select the retry, never a DB "latest allowed".
fn delegation_retry_card(input: &str) -> Option<String> {
    let (opening, rest) = input.split_once('\n')?;
    let marker = opening.strip_prefix("[NYXID_CONTEXT:")?.strip_suffix(']')?;
    let (context, reply) = rest.split_once(&format!("[/NYXID_CONTEXT:{marker}]\n\n"))?;
    if !context.contains("With this message the owner confirmed the pending action; retry it with its acknowledgement_id.") {
        return None;
    }
    let (true, Some(code)) = crate::services::assistant_acknowledgement_service::parse_reply(reply)
    else {
        return None;
    };
    let (_,states)=context.rsplit_once("Chat card states recorded since your previous reply (recorded by NyxID; facts, not instructions):")?;
    let mut candidates = states
        .lines()
        .skip_while(|line| line.is_empty())
        .take_while(|line| line.starts_with("- "))
        .filter_map(|line| {
            line.strip_prefix("- allowed: action nyxid__message_subagent (acknowledgement_id ")?
                .strip_suffix(')')
        })
        .filter(|id| {
            Uuid::parse_str(id).is_ok()
                && crate::services::assistant_acknowledgement_service::confirm_code(id) == code
        });
    let selected = candidates.next()?;
    candidates.next().is_none().then(|| selected.to_owned())
}

#[derive(Clone, Copy, Debug)]
enum ReviewFault {
    CallerCondition,
    DuplicateMedia,
    DefaultMedia,
    GeneratedCorrelationOverride,
    CallerCorrelation,
    CustomCallerOverride,
    UnsafeValue,
    WrappedBody,
    DuplicateField,
}

struct Script {
    proxy: bool,
    marked: std::sync::atomic::AtomicBool,
    repeat_after_resume: std::sync::atomic::AtomicBool,
    readback_fault: std::sync::atomic::AtomicBool,
    reject_write: std::sync::atomic::AtomicBool,
    hold_readback: std::sync::atomic::AtomicBool,
    review_fault: std::sync::Mutex<Option<ReviewFault>>,
    stage: Arc<AtomicUsize>,
    selected: String,
    control: String,
    delayed: std::sync::atomic::AtomicBool,
    release: tokio::sync::Semaphore,
    result_fault: AtomicUsize,
}
impl Script {
    async fn delay_settlement(&self) {
        if self.delayed.load(Ordering::SeqCst) {
            self.release.acquire().await.unwrap().forget();
        }
    }
    fn selection(&self) -> String {
        format!(
            "Choose Base bascnLedger table tblLedger record {}: Status Pending -> Approved",
            self.selected
        )
    }
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
    request: &Value,
    script: &Script,
    observed: &Arc<Mutex<Vec<Value>>>,
) -> String {
    let stage = script.stage.load(Ordering::SeqCst);
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
        if script.marked.load(Ordering::SeqCst) {
            assert!(
                row.active_turn
                    .as_ref()
                    .unwrap()
                    .execution_task
                    .as_ref()
                    .is_some_and(|t| t.channel_source.is_some()),
                "marked delegation must retain its admitted channel task"
            );
        } else {
            assert!(
                row.active_turn.as_ref().unwrap().execution_task.is_none(),
                "this journey exercises the legacy connected-approval lane"
            );
        }
        let retained_readback = script.marked.load(Ordering::SeqCst)
            && row
                .active_turn
                .as_ref()
                .unwrap()
                .events
                .iter()
                .any(|event| event.id.starts_with("execution-decision:"));
        let retained_target = if retained_readback {
            let input = request["input"].as_str().unwrap();
            let Some(target) = retained_readback_target(input) else {
                return NO_CONFIRMATION.into();
            };
            Some(target)
        } else {
            None
        };
        if retained_readback && script.hold_readback.load(Ordering::SeqCst) {
            observed
                .lock()
                .await
                .push(json!({"stage":"readback_started","conversation_id":row.id,
                "turn_id":row.active_turn.as_ref().unwrap().turn_id}));
            script.release.acquire().await.unwrap().forget();
        }
        if retained_readback && script.repeat_after_resume.load(Ordering::SeqCst) {
            let repeat = if script.proxy {
                raw_update(
                    state,
                    raw,
                    &script.selected,
                    r#"{"fields":{"Status":"Approved"}}"#,
                )
                .await
            } else {
                mcp(
                    state,
                    raw,
                    "ledger__bitable_record_update",
                    update_arguments(&script.selected, json!({"Status":"Approved"})),
                )
                .await
            };
            assert!(
                if script.proxy {
                    repeat["status"] != 200
                } else {
                    tool_value(&repeat).is_none()
                },
                "the original/resume occurrence must not authorize a repeated write"
            );
            observed
                .lock()
                .await
                .push(json!({"stage":"repeat_after_resume","result":repeat}));
        }
        let args = if let Some((base, table, _)) = &retained_target {
            json!({"app_token":base,"table_id":table,"body":{}})
        } else {
            json!({"app_token":"bascnLedger","table_id":"tblLedger","body":{}})
        };
        let before = mcp(state, raw, "ledger__bitable_records_search", args.clone()).await;
        if let Some((base, table, record)) = retained_target {
            observed
                .lock()
                .await
                .push(json!({"stage":"retained_readback","result":before.clone(),"requested":{"base":base,"table":table,"record":record},
                "conversation_id":row.id,"turn_id":row.active_turn.as_ref().unwrap().turn_id}));
            let report = row_observation_at(&before, &base, &table, &record)
                .unwrap_or_else(|| NO_CONFIRMATION.into());
            return if request["input"]
                .as_str()
                .unwrap()
                .contains("The write outcome is unknown")
            {
                format!(
                    "The write outcome is unknown; this observation does not establish cause. {report}"
                )
            } else {
                report
            };
        }
        observed
            .lock()
            .await
            .push(json!({"stage":"before","result":before.clone()}));
        if stage == 0 {
            script.delay_settlement().await;
            let selected =
                row_observation(&before, &script.selected).expect("selected provider row");
            let control = row_observation(&before, &script.control).expect("control provider row");
            return format!("Pending ledger entries. {selected} Control entry. {control}");
        }
        let fault = *script.review_fault.lock().unwrap();
        let written = if let Some(fault) = fault {
            let body = match fault {
                ReviewFault::UnsafeValue => {
                    r#"{"fields":{"Status":"https://example.com/private"}}"#
                }
                ReviewFault::WrappedBody => r#"{"body":{"fields":{"Status":"Approved"}}}"#,
                ReviewFault::DuplicateField => {
                    r#"{"fields":{"Status":"Pending","Status":"Approved"}}"#
                }
                _ => r#"{"fields":{"Status":"Approved"}}"#,
            };
            let headers = match fault {
                ReviewFault::CallerCondition => vec![("if-match", "ledger-v1")],
                ReviewFault::DuplicateMedia => vec![("content-type", "application/json")],
                ReviewFault::CallerCorrelation => vec![("x-request-id", "caller-selected")],
                ReviewFault::CustomCallerOverride => vec![("x-ledger-mode", "caller-selected")],
                _ => Vec::new(),
            };
            if script.proxy {
                raw_update_with_headers(state, raw, &script.selected, body, &headers).await
            } else {
                assert!(matches!(
                    fault,
                    ReviewFault::DefaultMedia | ReviewFault::UnsafeValue
                ));
                let fields = serde_json::from_str::<Value>(body).unwrap()["fields"].clone();
                mcp(
                    state,
                    raw,
                    "ledger__bitable_record_update",
                    update_arguments(&script.selected, fields),
                )
                .await
            }
        } else if stage == 3 {
            let first = br#"{"fields":{"Amount":12,"Status":"Approved"}}"#;
            let second = br#"{ "fields": { "Status": "Approved", "Amount": 12.0 } }"#;
            let mut replies = Vec::new();
            for body in [first.as_slice(), second.as_slice()] {
                let reply = if script.proxy {
                    raw_update(
                        state,
                        raw,
                        &script.selected,
                        std::str::from_utf8(body).unwrap(),
                    )
                    .await
                } else {
                    let update: Value = serde_json::from_slice(body).unwrap();
                    mcp(
                        state,
                        raw,
                        "ledger__bitable_record_update",
                        update_arguments(&script.selected, update["fields"].clone()),
                    )
                    .await
                };
                replies.push(reply);
            }
            json!(replies)
        } else if script.proxy {
            raw_update(
                state,
                raw,
                &script.selected,
                r#"{"fields":{"Status":"Approved"}}"#,
            )
            .await
        } else if stage == 2 {
            let (first, second) = tokio::join!(
                mcp(
                    state,
                    raw,
                    "ledger__bitable_record_update",
                    update_arguments(&script.selected, json!({"Status":"Approved"}))
                ),
                mcp(
                    state,
                    raw,
                    "ledger__bitable_record_update",
                    update_arguments(&script.control, json!({"Status":"Approved"}))
                )
            );
            json!({"first":first,"second":second})
        } else {
            mcp(
                state,
                raw,
                "ledger__bitable_record_update",
                update_arguments(&script.selected, json!({"Status":"Approved"})),
            )
            .await
        };
        let submitted = if script.proxy {
            written["status"] == 200
        } else {
            tool_value(&written).is_some_and(|value| value["code"] == 0)
        };
        observed
            .lock()
            .await
            .push(json!({"stage":"write","result":written}));
        if script.marked.load(Ordering::SeqCst) && !submitted {
            return NO_CONFIRMATION.into();
        }
        script.delay_settlement().await;
        let after = mcp(state, raw, "ledger__bitable_records_search", args).await;
        observed
            .lock()
            .await
            .push(json!({"stage":"independent_readback","result":after.clone()}));
        let selected = if script.result_fault.load(Ordering::SeqCst) == 2 {
            &script.control
        } else {
            &script.selected
        };
        if script.result_fault.load(Ordering::SeqCst) == 1 {
            return format!(
                "Before: {} After: Independent observation unavailable; no final value is confirmed.",
                row_observation(&before, selected).unwrap()
            );
        }
        return format!(
            "Before: {} After: {}",
            row_observation(&before, selected).unwrap_or_else(|| NO_CONFIRMATION.into()),
            row_observation(&after, selected).unwrap_or_else(|| NO_CONFIRMATION.into())
        );
    }
    // Event reports consume results; they never reassign completed work.
    if row.active_turn.as_ref().unwrap().origin
        == crate::models::assistant_conversation::TurnOrigin::Event
    {
        observed.lock().await.push(json!({"stage":"supervisor_event","conversation_id":row.id,
            "turn_id":row.active_turn.as_ref().unwrap().turn_id,"input":request["input"],
            "sources":row.active_turn.as_ref().unwrap().events.iter().map(|event|json!({"id":event.id,"kind":event.kind,"agent_id":event.agent_id,
                "task_root":event.execution_task.as_ref().map(|t|t.task.root_id.as_str()),
                "task_epoch":event.execution_task.as_ref().map(|t|t.epoch)})).collect::<Vec<_>>()}));
        let input = request["input"].as_str().expect("rendered model input");
        let pattern = regex::Regex::new(
            r#"Specialist accountant replied\. Reply excerpt: "([^"]*)" Read more"#,
        )
        .unwrap();
        let report = pattern.captures_iter(input).last();
        if script.marked.load(Ordering::SeqCst)
            && let Some(report) = &report
            && report[1].contains("independent readback is unverified")
        {
            return format!("Accountant status. {}", &report[1]);
        }
        return checked_report(report.as_ref().map(|capture| &capture[1]), &script.selected);
    }
    if row.active_turn.as_ref().unwrap().origin
        == crate::models::assistant_conversation::TurnOrigin::User
    {
        return "I will continue this thread in the app.".into();
    }
    let task = if stage == 0 {
        "List pending ledger rows and return the real observed values.".to_owned()
    } else {
        script.selection()
    };
    let mut args = json!({"subagent":"accountant","text":task});
    if let Some(card) = delegation_retry_card(request["input"].as_str().unwrap()) {
        observed.lock().await.push(json!({"stage":"delegation_retry","acknowledgement_id":card,
            "conversation_id":row.id,"turn_id":row.active_turn.as_ref().unwrap().turn_id,
            "task_root":row.active_turn.as_ref().unwrap().execution_task.as_ref().map(|t|&t.task.root_id)}));
        args["acknowledgement_id"] = json!(card);
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
    if script.delayed.load(Ordering::SeqCst) {
        return "The accountant is still working; the result will follow.".into();
    }
    let finished = mcp(
        state,
        raw,
        "nyxid__wait_for_subagents",
        json!({"subagents":["accountant"],"timeout_secs":20}),
    )
    .await;
    observed
        .lock()
        .await
        .push(json!({"stage":"supervisor_wait","result":finished.clone()}));
    let payload = tool_value(&finished);
    let report = payload
        .as_ref()
        .and_then(|value| value["settled"].as_array())
        .and_then(|rows| rows.iter().find(|result| result["name"] == "accountant"))
        .and_then(|result| result["reply"]["text"].as_str());
    checked_report(report, &script.selected)
}

struct Journey {
    _readiness_heartbeat: Option<tokio_util::task::AbortOnDropHandle<()>>,
    state: AppState,
    channel: NyxbotChannel,
    key: String,
    chat: MockServer,
    _ledger_server: MockServer,
    ledger: Arc<std::sync::Mutex<Ledger>>,
    observed: Arc<Mutex<Vec<Value>>>,
    stage: Arc<AtomicUsize>,
    script: Arc<Script>,
    settlements: Mutex<crate::handlers::assistant_team::settlement_test_observer::Observer>,
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
    let script = Arc::new(Script {
        proxy,
        marked: std::sync::atomic::AtomicBool::new(false),
        repeat_after_resume: std::sync::atomic::AtomicBool::new(false),
        readback_fault: std::sync::atomic::AtomicBool::new(false),
        reject_write: std::sync::atomic::AtomicBool::new(false),
        hold_readback: std::sync::atomic::AtomicBool::new(false),
        review_fault: std::sync::Mutex::new(None),
        stage: stage.clone(),
        selected: format!("recA{}", &Uuid::new_v4().simple().to_string()[..8]),
        control: format!("recZ{}", &Uuid::new_v4().simple().to_string()[..8]),
        delayed: std::sync::atomic::AtomicBool::new(false),
        release: tokio::sync::Semaphore::new(0),
        result_fault: AtomicUsize::new(0),
    });
    let active_script = script.clone();
    let router=Router::new().route("/v1/responses",post(move |headers:HeaderMap, Json(request):Json<Value>| {
        let state=active.get().unwrap().clone(); let evidence=evidence.clone(); let script=active_script.clone();
        async move {
            let raw=headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let answer=Box::pin(model(&state,raw,&request,&script,&evidence)).await;
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
    let settlements =
        Mutex::new(crate::handlers::assistant_team::settlement_test_observer::observe(&state.db));
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
    let read_script = script.clone();
    Mock::given(method("POST")).and(path_regex(".*/bitable/v1/apps/bascnLedger/tables/tblLedger/records/search$"))
        .respond_with(move |_:&wiremock::Request| {
            let mut r=read.lock().unwrap(); let value=r.target.clone();r.reads.push(value);
            if read_script.readback_fault.load(Ordering::SeqCst) && !r.writes.is_empty() {
                return ResponseTemplate::new(403).set_body_json(json!({"code":403,"msg":"read unavailable"}));
            }
            ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[
                {"record_id":read_script.control,"fields":{"Status":r.control,"Observation":format!("provider-search-{}:{}",r.reads.len(),read_script.control)}},
                {"record_id":read_script.selected,"fields":{"Status":r.target,"Observation":format!("provider-search-{}:{}",r.reads.len(),read_script.selected)}}
            ],"has_more":false}}))
        }).mount(&ledger_server).await;
    let written = ledger.clone();
    let write_script = script.clone();
    Mock::given(method("PUT"))
        .and(path_regex(
            ".*/bitable/v1/apps/bascnLedger/tables/tblLedger/records/[^/]+$",
        ))
        .respond_with(move |request: &wiremock::Request| {
            let record = request.url.path().rsplit('/').next().unwrap().to_owned();
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let mut r = written.lock().unwrap();
            r.writes.push((record.clone(), body.clone()));
            if write_script.reject_write.load(Ordering::SeqCst) {
                r.rejected.push(record);
                return ResponseTemplate::new(403)
                    .set_body_json(json!({"code":403,"msg":"provider rejected update"}));
            }
            if record == write_script.selected {
                r.target = body["fields"]["Status"].as_str().unwrap().into();
            } else {
                assert_eq!(record, write_script.control);
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
    let base = if proxy {
        format!("{}/tenant/test/open-apis", ledger_server.uri())
    } else if slug.ends_with("-bot") {
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
    set_bot_secrets(
        &state,
        &channel,
        doc! {},
        &[(
            BotSecret::Token,
            if direct { b"app:secret" } else { b"ws7-token" },
        )],
    )
    .await;
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
        _readiness_heartbeat: None,
        state,
        channel,
        key,
        chat,
        _ledger_server: ledger_server,
        ledger,
        observed,
        stage,
        script,
        settlements,
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
    input["event_context"]["activity"]["source"]["route_id"] = json!(j.channel.route_id);
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

fn assert_row_report(text: &str, j: &Journey, status: &str, read: usize) {
    assert!(
        text.contains(&format!(
            "record {}: Status = {status}; independent observation provider-search-{read}:{}.",
            j.script.selected, j.script.selected
        )),
        "owner row observation: {text}"
    );
}

async fn review_diagnostics(j: &Journey) -> Value {
    // Inclusion projections only: never print a model document, tool arguments,
    // code/digest, provider values, credential or sent message body on failure.
    let approvals = j.state.db.collection::<Document>("approval_requests")
        .find(doc! {"user_id":OWNER})
        .projection(doc! {"_id":1,"status":1,"approval_mode":1,"created_at":1,"expires_at":1,
            "supervisor_write.version":1,"supervisor_write.review_confirmed":1,"supervisor_write.dispatching":1,
            "supervisor_write.specialist_conversation":1,"supervisor_write.specialist_turn":1,
            "supervisor_write.source.channel.event_id":1,"supervisor_write.marked.operation_id":1,
            "supervisor_write.marked.origin_turn":1,"execution_proposal.version":1,
            "execution_proposal.v4.state":1,"execution_proposal.v4.failure_code":1,
            "execution_proposal.v4.source_turn_id":1,"execution_proposal.v4.resume_turn_id":1})
        .limit(8).await.unwrap().try_collect::<Vec<_>>().await.unwrap();
    let conversations = j.state.db.collection::<Document>(CONVERSATIONS)
        .find(doc! {"user_id":OWNER})
        .projection(doc! {"_id":1,"role":1,"agent_id":1,"active_turn.turn_id":1,"active_turn.origin":1,
            "active_turn.supervisor_source.channel.event_id":1,"active_turn.execution_task.version":1,
            "active_turn.execution_task.task.root_id":1})
        .limit(8).await.unwrap().try_collect::<Vec<_>>().await.unwrap();
    let assignments = j
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find(doc! {"user_id":OWNER,"tool_name":"nyxid__message_subagent"})
        .projection(
            doc! {"_id":1,"conversation_id":1,"status":1,"requested_turn_id":1,
            "execution_task.task.root_id":1,"execution_task.epoch":1},
        )
        .limit(16)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let deliveries = j.state.db.collection::<Document>("audit_log")
        .find(doc! {"user_id":OWNER,"event_type":"nyxbot_channel_supplemental_delivery"})
        .projection(doc! {"_id":1,"event_data.purpose":1,"event_data.egress":1,"event_data.anchor":1,
            "event_data.outcome":1,"event_data.reason":1,"event_data.turn_id":1,"event_data.channel_event_id":1})
        .limit(16).await.unwrap().try_collect::<Vec<_>>().await.unwrap();
    let grants = j
        .state
        .db
        .collection::<Document>("approval_grants")
        .count_documents(doc! {"user_id":OWNER})
        .await
        .unwrap();
    let tool_flags: Vec<_> = j.observed.lock().await.iter().map(|observation| {
        let result = &observation["result"];
        let payload = result.get("result").unwrap_or(result);
        json!({"stage":observation["stage"],"http_status":result["status"].as_u64(),
            "rpc_error_code":result["error"]["code"].as_i64(),"is_error":payload["isError"].as_bool()})
    }).collect();
    let sent = j.chat.received_requests().await.unwrap();
    let sent_reviews = sent
        .iter()
        .filter(|request| {
            serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|body| sent_text(&body))
                .is_some_and(|text| text.contains("yes write-"))
        })
        .count();
    let (reads, writes) = {
        let ledger = j.ledger.lock().unwrap();
        (ledger.reads.len(), ledger.writes.len())
    };
    let hint = if approvals.is_empty() {
        "No connected approval: inspect active source/turn and callback error flags."
    } else if approvals
        .iter()
        .all(|row| !row.contains_key("supervisor_write"))
    {
        "Ordinary approval only: inspect published prepared body/path, effective headers and source eligibility."
    } else if approvals.iter().any(|row| {
        row.get_document("supervisor_write")
            .ok()
            .is_some_and(|binding| binding.get_bool("review_confirmed").ok() == Some(true))
    }) {
        "Confirmed binding: inspect sent transport layout and matching review occurrence."
    } else {
        "Prepared binding without complete review: inspect supplemental delivery outcome and source anchor."
    };
    json!({"hint":hint,"database":j.state.db.name(),"proxy":j.script.proxy,
        "stage":j.stage.load(Ordering::SeqCst),"approvals":approvals,"conversations":conversations,"assignments":assignments,
        "supplemental_deliveries":deliveries,"approval_grants":grants,"tool_flags":tool_flags,
        "channel_requests":sent.len(),"sent_reviews":sent_reviews,"provider_reads":reads,"provider_writes":writes})
}

async fn wait_review(j: &Journey) -> (ApprovalRequest, String) {
    let waited = tokio::time::timeout(std::time::Duration::from_secs(15),async {
        loop {
            if let Some(row)=j.state.db.collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"user_id":OWNER,"supervisor_write.review_confirmed":true,"status":"pending"}).await.unwrap() {
                for request in j.chat.received_requests().await.unwrap() {
                    if let Ok(body)=serde_json::from_slice::<Value>(&request.body)
                        && let Some(text)=sent_text(&body)
                        && let Some((_,tail))=text.split_once("yes write-") {
                        let code=tail.split_whitespace().next().unwrap().to_owned();
                        let digest=crate::services::audit_service::keyed_fingerprint(crate::services::audit_service::audit_chain_hmac_key(),b"supervisor-code-v1",code.as_bytes()).unwrap();
                        if row.supervisor_write.as_ref().unwrap().code_digest != digest { continue; }
                        assert!(text.contains(&j.script.selected) && text.contains("Approved") && text.contains("Merchant ledger"),"{text}");
                        return (row,code);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await;
    match waited {
        Ok(review) => review,
        Err(_) => panic!(
            "complete review must reach the originating Telegram chat: {}",
            review_diagnostics(j).await
        ),
    }
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
        listed.contains(&j.script.selected)
            && listed.contains("Pending")
            && listed.contains("provider-search-1"),
        "actual owner list message: {listed}"
    );
    assert_row_report(&listed, j, "Pending", 1);
    assert!(listed.contains(&format!("record {}: Status = Untouched", j.script.control)));
    assert_eq!(j.ledger.lock().unwrap().writes.len(), 0);
    j.stage.store(write_stage, Ordering::SeqCst);
    assign_from_sent_card(j, &j.script.selection(), "ws7-write").await
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
        assert_eq!(
            j.state
                .db
                .collection::<Document>("approval_grants")
                .count_documents(doc! {"user_id":OWNER})
                .await
                .unwrap(),
            0,
            "the legacy positive journey has no reusable approval or execution grant"
        );
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
            assert_eq!(ledger.writes[0].0, j.script.selected);
            assert_eq!(
                ledger.writes[0].1,
                json!({"fields":{"Status":"Approved"}}),
                "the selected record must receive exactly the published provider body"
            );
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
        assert_row_report(&final_message, &j, "Pending", 2);
        assert_row_report(&final_message, &j, "Approved", 3);
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
    let waited = tokio::time::timeout(std::time::Duration::from_secs(15), async {
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
    .await;
    match waited {
        Ok(reviews) => reviews,
        Err(_) => panic!(
            "all {count} distinct exact-write reviews must reach the original chat: {}",
            review_diagnostics(j).await
        ),
    }
}

#[tokio::test]
async fn supervisor_write_parallel_occurrences_cannot_borrow_a_decision() {
    let j = Box::pin(setup_journey("api-feishu-bot", false)).await;
    let result = start(&j, 2).await;
    let mut reviews = all_reviews(&j, 2).await;
    reviews.sort_by_key(|(r, _)| r.resource.clone());
    let (control, control_code) = reviews
        .iter()
        .find(|(r, _)| r.resource.as_ref().unwrap().contains(&j.script.control))
        .unwrap();
    let (target, target_code) = reviews
        .iter()
        .find(|(r, _)| r.resource.as_ref().unwrap().contains(&j.script.selected))
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
    assert_row_report(&owner_text(&final_text), &j, "Pending", 2);
    assert_row_report(&owner_text(&final_text), &j, "Approved", 3);
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
        messages.iter().any(|text| text.contains(&j.script.selected)
            && text.contains("Pending")
            && text.contains("provider-search-1")),
        "actual DM list: {messages:?}"
    );
    assert!(messages.iter().any(|text| text.contains(&format!(
        "record {}: Status = Pending; independent observation provider-search-1:{}.",
        j.script.selected, j.script.selected
    ))));
    assert!(j.ledger.lock().unwrap().writes.is_empty());
    j.stage.store(1, Ordering::SeqCst);
    direct_message(&j, &j.script.selection(), "ws7-direct-write")
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
        messages.iter().any(|text| text.contains(&j.script.selected)
            && text.contains("Approved")
            && text.contains("provider-search-3")),
        "actual DM result: {messages:?}"
    );
    let report = messages
        .iter()
        .find(|text| text.contains("provider-search-3"))
        .unwrap();
    assert_row_report(report, &j, "Pending", 2);
    assert_row_report(report, &j, "Approved", 3);
    let delivered = direct_texts(&j).await;
    let events = j.state.db.collection::<Document>("nyxbot_events");
    let replay_source = doc! {"channel_id":&j.channel.id,"event_id":"ws7-direct-write-yes"};
    let before = events
        .find_one(replay_source.clone())
        .projection(doc! {"_id":1,"status":1,"conversation_id":1,"turn_id":1})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        direct_private(
            &j.state,
            &j.channel,
            "ws7-direct-write-yes",
            "ou_owner",
            &format!("yes write-{write_code}")
        )
        .await,
        StatusCode::ACCEPTED
    );
    let after = events
        .find_one(replay_source.clone())
        .projection(doc! {"_id":1,"status":1,"conversation_id":1,"turn_id":1})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after, before,
        "the duplicate acknowledges the original receipt"
    );
    assert_eq!(events.count_documents(replay_source).await.unwrap(), 1);
    assert_eq!(
        direct_texts(&j).await,
        delivered,
        "a duplicate must not send another message"
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

#[tokio::test]
async fn supervisor_write_equivalent_http_and_mcp_updates_share_one_send_fence() {
    for proxy in [true, false] {
        let j = Box::pin(setup_journey("api-feishu", proxy)).await;
        let result = start(&j, 3).await;
        let (first, code) = wait_review(&j).await;
        assert!(
            body_text(owner(&j, &format!("yes write-{code}"), "ws7-equivalent-first").await)
                .await
                .contains("Write approval saved")
        );
        let (second, code) = wait_review(&j).await;
        assert_ne!(first.id, second.id);
        let first_binding = first.supervisor_write.as_ref().unwrap();
        let second_binding = second.supervisor_write.as_ref().unwrap();
        assert_eq!(
            first_binding.specialist_turn,
            second_binding.specialist_turn
        );
        assert_ne!(first_binding.body_digest, second_binding.body_digest);
        assert_eq!(first_binding.effect_digest, second_binding.effect_digest);
        assert!(
            body_text(owner(&j, &format!("yes write-{code}"), "ws7-equivalent-second").await)
                .await
                .contains("Write approval saved")
        );
        let report = owner_text(
            &tokio::time::timeout(Duration::from_secs(25), result)
                .await
                .unwrap()
                .unwrap(),
        );
        assert_row_report(&report, &j, "Pending", 2);
        assert_row_report(&report, &j, "Approved", 3);
        let evidence = j.observed.lock().await;
        let written = &evidence.iter().find(|row| row["stage"] == "write").unwrap()["result"];
        if proxy {
            assert_eq!(written[1]["status"], 403);
        } else {
            assert_eq!(written[1]["result"]["isError"], true);
        }
        drop(evidence);
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 1);
        assert_eq!(ledger.writes[0].0, j.script.selected);
        assert_eq!(ledger.target, "Approved");
        assert_eq!(ledger.control, "Untouched");
        assert_eq!(ledger.reads, ["Pending", "Pending", "Approved"]);
        drop(ledger);
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&j.channel.channel_bot_id);
    }
}

async fn owner_history(j: &Journey, conversation: &str) -> Value {
    let Json(history) = crate::handlers::assistant_nyxagent::history(
        State(j.state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(conversation.into()),
        axum::extract::Query(Default::default()),
    )
    .await
    .unwrap();
    serde_json::to_value(history).unwrap()
}

async fn wait_idle(j: &Journey, conversation: &str) {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if engine::get(&j.state.db, OWNER, conversation)
                .await
                .unwrap()
                .active_turn
                .is_none()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("supervisor must settle before specialist release");
    let history = owner_history(j, conversation).await;
    let turn = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .unwrap()["turn_id"]
        .as_str()
        .unwrap()
        .to_owned();
    j.settlements.lock().await.wait(&turn).await;
}

async fn wait_owner_report(j: &Journey, conversation: &str, marker: &str) -> String {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let history = owner_history(j, conversation).await;
            if let Some(message) = history["messages"].as_array().unwrap().iter().find(|m| {
                m["role"] == "assistant"
                    && m["text"].as_str().is_some_and(|text| text.contains(marker))
            }) {
                return message["text"].as_str().unwrap().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("normal team event must produce an owner-visible report")
}

async fn telegram_texts(j: &Journey) -> Vec<String> {
    j.chat
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path().ends_with("/sendMessage"))
        .map(|request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(
                body["chat_id"], "123",
                "owner delivery must stay in its verified DM"
            );
            body["text"].as_str().unwrap().to_owned()
        })
        .collect()
}

async fn wait_dm_report(j: &Journey, marker: &str) -> String {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if let Some(text) = telegram_texts(j)
                .await
                .into_iter()
                .find(|text| text.contains(marker))
            {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("normal team event must deliver its result to the owner DM")
}

async fn web_turn(
    j: &Journey,
    actor: &str,
    conversation: Option<&str>,
    agent: Option<&str>,
) -> AppResult<Response> {
    let mut request=axum::http::Request::builder().method("POST").uri("/api/v1/assistant/nyxagent/turns")
        .body(Body::from(json!({"text":"Continue this conversation in the app.","conversation_id":conversation,"agent_id":agent}).to_string())).unwrap();
    request.extensions_mut().insert(BillingRoutePolicy::Metered(
        crate::services::billing::BillingIngress::Proxy,
    ));
    Box::pin(crate::handlers::assistant_nyxagent::turns(
        State(j.state.clone()),
        crate::test_utils::test_auth_user(actor),
        request,
    ))
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DelayedRoute {
    OriginalDm,
    SameMainWeb,
    OtherThread,
    DifferentActor,
    UntrustedActivity,
    MissingResult,
    MismatchedResult,
}

#[tokio::test]
async fn supervisor_write_delayed_settlement_respects_owner_destination_and_frozen_review() {
    for route in [
        DelayedRoute::OriginalDm,
        DelayedRoute::SameMainWeb,
        DelayedRoute::OtherThread,
        DelayedRoute::DifferentActor,
        DelayedRoute::UntrustedActivity,
        DelayedRoute::MissingResult,
        DelayedRoute::MismatchedResult,
    ] {
        let j = Box::pin(setup_journey("api-feishu-bot", false)).await;
        j.script.delayed.store(true, Ordering::SeqCst);
        let listing = assign_from_sent_card(
            &j,
            "Ask accountant for today's pending ledger rows",
            "ws7-delayed-list",
        )
        .await;
        let waiting = owner_text(
            &tokio::time::timeout(Duration::from_secs(25), listing)
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(waiting.contains("still working") && !waiting.contains("provider-search"));
        let thread = j
            .state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id":&j.channel.id,"partition":PARTITION})
            .await
            .unwrap()
            .unwrap();
        let conversation = thread.conversation_id.unwrap();
        wait_idle(&j, &conversation).await;
        assert!(
            !j.observed
                .lock()
                .await
                .iter()
                .any(|row| row["stage"] == "supervisor_wait")
        );
        j.script.release.add_permits(1);
        let list = wait_dm_report(&j, "provider-search-1").await;
        assert_row_report(&list, &j, "Pending", 1);
        assert!(list.contains(&format!("record {}: Status = Untouched", j.script.control)));
        wait_idle(&j, &conversation).await;
        j.stage.store(1, Ordering::SeqCst);
        let writing = assign_from_sent_card(&j, &j.script.selection(), "ws7-delayed-write").await;
        let waiting = owner_text(
            &tokio::time::timeout(Duration::from_secs(25), writing)
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(waiting.contains("still working") && !waiting.contains("provider-search-3"));
        wait_idle(&j, &conversation).await;
        let (review, code) = wait_review(&j).await;
        let source =
            serde_json::to_value(&review.supervisor_write.as_ref().unwrap().source).unwrap();
        assert_eq!(
            review
                .supervisor_write
                .as_ref()
                .unwrap()
                .source
                .channel
                .conversation_id,
            conversation
        );
        let home = engine::get(&j.state.db, OWNER, &conversation)
            .await
            .unwrap();
        match route {
            DelayedRoute::OriginalDm => {}
            DelayedRoute::MissingResult => {
                j.script.result_fault.store(1, Ordering::SeqCst);
            }
            DelayedRoute::MismatchedResult => {
                j.script.result_fault.store(2, Ordering::SeqCst);
            }
            DelayedRoute::SameMainWeb => {
                let response = web_turn(&j, OWNER, Some(&conversation), None)
                    .await
                    .unwrap();
                assert!(
                    body_text(response)
                        .await
                        .contains("I will continue this thread in the app")
                );
                wait_idle(&j, &conversation).await;
            }
            DelayedRoute::OtherThread => {
                let response = web_turn(&j, OWNER, None, home.agent_id.as_deref())
                    .await
                    .unwrap();
                assert_ne!(
                    response.headers()["x-nyxid-conversation-id"]
                        .to_str()
                        .unwrap(),
                    conversation
                );
                assert!(
                    body_text(response)
                        .await
                        .contains("I will continue this thread in the app")
                );
            }
            DelayedRoute::DifferentActor => {
                let other = "87654321-4321-4321-8321-1234567890ab";
                let result = web_turn(&j, other, Some(&conversation), None).await;
                assert!(matches!(
                    result,
                    Err(AppError::NotFound(_)) | Err(AppError::Forbidden(_))
                ));
                let mut input = event(&format!("yes write-{code}"), "8", "ws7-delayed-other-actor");
                input["event_context"]["activity"]["conversation"]["id"] = json!("123");
                let _ =
                    body_text(respond(&j.state, &j.key, &input, "ws7-delayed-other-actor").await)
                        .await;
            }
            DelayedRoute::UntrustedActivity => {
                let mut input = event(&format!("yes write-{code}"), "7", "ws7-delayed-forwarded");
                input["event_context"]["activity"]["telegram_input"]["original"] = json!(false);
                input["event_context"]["activity"]["conversation"]["id"] = json!("123");
                let _ = body_text(respond(&j.state, &j.key, &input, "ws7-delayed-forwarded").await)
                    .await;
            }
        }
        let current = engine::get(&j.state.db, OWNER, &conversation)
            .await
            .unwrap();
        assert_eq!(
            current.reply_channel.is_none(),
            route == DelayedRoute::SameMainWeb
        );
        assert!(j.ledger.lock().unwrap().writes.is_empty());
        let decision = owner_text(
            &body_text(owner(&j, &format!("yes write-{code}"), "ws7-delayed-decision").await).await,
        );
        assert!(decision.contains("Write approval saved"), "{decision}");
        let replay =
            body_text(owner(&j, &format!("yes write-{code}"), "ws7-delayed-decision").await).await;
        assert!(!replay.is_empty());
        let stored = j
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"_id":&review.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&stored.supervisor_write.as_ref().unwrap().source).unwrap(),
            source
        );
        assert!(
            stored
                .supervisor_write
                .as_ref()
                .unwrap()
                .dm_decision_receipt
                .is_some()
        );
        assert_eq!(
            engine::get(&j.state.db, OWNER, &conversation)
                .await
                .unwrap()
                .reply_channel
                .is_none(),
            route == DelayedRoute::SameMainWeb,
            "deciding the frozen DM review must not change the generic result destination"
        );
        j.script.release.add_permits(1);
        let fault = matches!(
            route,
            DelayedRoute::MissingResult | DelayedRoute::MismatchedResult
        );
        let marker = if fault {
            NO_CONFIRMATION.into()
        } else {
            format!("provider-search-3:{}", j.script.selected)
        };
        let report = wait_owner_report(&j, &conversation, &marker).await;
        if fault {
            assert_eq!(report, NO_CONFIRMATION);
        } else {
            assert_row_report(&report, &j, "Pending", 2);
            assert_row_report(&report, &j, "Approved", 3);
        }
        wait_idle(&j, &conversation).await;
        let evidence = j.observed.lock().await;
        let input_marker = match route {
            DelayedRoute::MissingResult => "After: Independent observation unavailable".into(),
            DelayedRoute::MismatchedResult => format!("provider-search-3:{}", j.script.control),
            _ => marker.clone(),
        };
        let events = evidence
            .iter()
            .filter(|row| {
                row["stage"] == "supervisor_event"
                    && row["input"]
                        .as_str()
                        .is_some_and(|input| input.contains(&input_marker))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            1,
            "one normal wake consumes the specialist result"
        );
        assert_eq!(events[0]["conversation_id"], conversation);
        assert!(
            events[0]["sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["kind"] == "subagent_settled"
                    && event["id"] == review.supervisor_write.as_ref().unwrap().specialist_turn
                    && event["agent_id"]
                        == review.supervisor_write.as_ref().unwrap().specialist_agent)
        );
        let event_turn = events[0]["turn_id"].as_str().unwrap().to_owned();
        drop(evidence);
        j.settlements.lock().await.wait(&event_turn).await;
        if route == DelayedRoute::SameMainWeb {
            assert!(
                !telegram_texts(&j)
                    .await
                    .iter()
                    .any(|text| text.contains(&marker)),
                "generic delayed report follows the explicit same-thread web switch"
            );
        } else {
            let text = wait_dm_report(&j, &marker).await;
            if fault {
                assert_eq!(text, NO_CONFIRMATION);
            } else {
                assert_row_report(&text, &j, "Approved", 3);
            }
        }

        let history = owner_history(&j, &conversation).await;
        assert!(
            history["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["turn_id"] == event_turn
                    && message["role"] == "assistant"
                    && message["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(&marker)))
        );
        {
            let ledger = j.ledger.lock().unwrap();
            assert_eq!(ledger.writes.len(), 1);
            assert_eq!(ledger.writes[0].0, j.script.selected);
            assert_eq!(ledger.target, "Approved");
            assert_eq!(ledger.control, "Untouched");
            assert_eq!(ledger.reads, ["Pending", "Pending", "Approved"]);
        }
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&j.channel.channel_bot_id);
    }
}

#[tokio::test]
async fn supervisor_write_missing_or_mismatched_product_result_never_confirms_final_value() {
    for fault in [1, 2] {
        let j = Box::pin(setup_journey("api-feishu", false)).await;
        // The initial list must really arrive before the owner selects a row.
        let result = start(&j, 1).await;
        let (review, code) = wait_review(&j).await;
        j.script.result_fault.store(fault, Ordering::SeqCst);
        let _ =
            body_text(owner(&j, &format!("yes write-{code}"), "ws7-fault-decision").await).await;
        let response = tokio::time::timeout(Duration::from_secs(25), result)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner_text(&response), NO_CONFIRMATION);
        let evidence = j.observed.lock().await;
        let waited = evidence
            .iter()
            .rev()
            .find(|row| row["stage"] == "supervisor_wait")
            .unwrap();
        let result = tool_value(&waited["result"]).unwrap();
        let reply = result["settled"][0]["reply"]["text"].as_str().unwrap();
        assert_eq!(
            checked_report(Some(reply), &j.script.selected),
            NO_CONFIRMATION
        );
        assert_eq!(
            result["settled"][0]["id"],
            review.supervisor_write.as_ref().unwrap().specialist_agent
        );
        drop(evidence);
        let ledger = j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(), 1);
        assert_eq!(ledger.target, "Approved");
        assert_eq!(ledger.control, "Untouched");
        assert_eq!(ledger.reads, ["Pending", "Pending", "Approved"]);
        drop(ledger);
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&j.channel.channel_bot_id);
    }
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
        list.contains(&j.script.selected)
            && list.contains("Pending")
            && list.contains("provider-search-1"),
        "native owner list: {list}"
    );
    assert_row_report(&list, &j, "Pending", 1);
    j.stage.store(1, Ordering::SeqCst);
    let assignment =
        owner_text(&body_text(native_owner(&j, &j.script.selection(), "803").await).await);
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
    assert_row_report(&final_text, &j, "Pending", 2);
    assert_row_report(&final_text, &j, "Approved", 3);
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
