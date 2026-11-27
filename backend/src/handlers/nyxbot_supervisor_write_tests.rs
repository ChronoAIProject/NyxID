//! Real owner ingress -> delegation card -> catalog MCP/proxy write wait ->
//! original owner DM decision -> provider write -> independent catalog read.
use super::*;
use crate::services::assistant_team_service as team;
use bson::Document;
use crate::models::approval_request::ApprovalRequest;
use crate::services::billing::route_inventory::BillingRoutePolicy;
use std::sync::OnceLock;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path_regex}};

#[derive(Default)]
struct Ledger {
    writes: Vec<(String,Value)>,
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
    let response=Box::pin(crate::handlers::mcp_transport::mcp_post(State(state.clone()),
        axum::Extension(BillingRoutePolicy::Metered(crate::services::billing::BillingIngress::Mcp)),request)).await;
    let text=body_text(response).await;
    serde_json::from_str(&text).unwrap_or_else(|_|panic!("MCP returned invalid JSON: {text}"))
}

async fn model(state: &AppState, raw: &str, proxy: bool, observed: &Arc<Mutex<Vec<Value>>>) -> String {
    let (_,key,_)=key_service::validate_api_key(&state.db,raw).await.unwrap();
    let row=state.db.collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"credential_api_key_id":&key.id}).await.unwrap().unwrap();
    if row.is_subagent() {
        assert!(row.active_turn.as_ref().unwrap().supervisor_source.is_some(),"delegation must carry original source");
        let args=json!({"app_token":"bascnLedger","table_id":"tblLedger","body":{}});
        let before=mcp(state,raw,"ledger__bitable_records_search",args.clone()).await;
        observed.lock().await.push(json!({"stage":"before","result":before}));
        let written=if proxy {
            use tower::ServiceExt;
            let (_,private)=crate::routes::build_router_with_state(state.clone());
            let request=axum::http::Request::builder().method("PUT")
                .uri("/api/v1/proxy/s/ledger/bitable/v1/apps/bascnLedger/tables/tblLedger/records/recTarget")
                .header("x-api-key",raw).header("content-type","application/json")
                .body(Body::from(json!({"fields":{"Status":"Approved"}}).to_string())).unwrap();
            let response=private.with_state(state.clone()).oneshot(request).await.unwrap();
            json!({"status":response.status().as_u16(),"body":body_text(response).await})
        } else {
            mcp(state,raw,"ledger__bitable_record_update",json!({"app_token":"bascnLedger","table_id":"tblLedger",
                "record_id":"recTarget","body":{"fields":{"Status":"Approved"}}})).await
        };
        observed.lock().await.push(json!({"stage":"write","result":written}));
        let after=mcp(state,raw,"ledger__bitable_records_search",args).await;
        observed.lock().await.push(json!({"stage":"independent_readback","result":after.clone()}));
        return format!("Independent bitable_records_search result: {after}");
    }
    // Event reports consume results; they never reassign completed work.
    if row.active_turn.as_ref().unwrap().origin==crate::models::assistant_conversation::TurnOrigin::Event {
        return serde_json::to_string(&row.active_turn.as_ref().unwrap().events.iter().map(|e|&e.text).collect::<Vec<_>>()).unwrap();
    }
    let mut args=json!({"subagent":"accountant","text":"Update recTarget Status from Pending to Approved; read back independently."});
    if let Some(card)=state.db.collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>("assistant_acknowledgements")
        .find_one(doc! {"conversation_id":&row.id,"status":"allowed","tool_name":"nyxid__message_subagent"})
        .sort(doc! {"created_at":-1}).await.unwrap() {
        args["acknowledgement_id"]=json!(card.id);
    }
    let result=mcp(state,raw,"nyxid__message_subagent",args).await;
    observed.lock().await.push(json!({"stage":"delegation","result":result.clone()}));
    if state.db.collection::<Document>("assistant_acknowledgements").count_documents(doc! {
        "conversation_id":&row.id,"status":"pending"
    }).await.unwrap()>0 { return format!("Review the exact accountant assignment: {result}"); }
    let finished=mcp(state,raw,"nyxid__wait_for_subagents",json!({"subagents":["accountant"],"timeout_secs":20})).await;
    format!("Accountant's independently observed result: {finished}")
}

struct Journey {
    state: AppState,
    channel: NyxbotChannel,
    key: String,
    chat: MockServer,
    _ledger_server: MockServer,
    ledger: Arc<std::sync::Mutex<Ledger>>,
    observed: Arc<Mutex<Vec<Value>>>,
    _engine: tokio_util::task::AbortOnDropHandle<()>,
}

async fn setup_journey(slug: &str, proxy: bool) -> Journey {
    let runtime=Arc::new(OnceLock::<AppState>::new());
    let observed=Arc::new(Mutex::new(Vec::new()));
    let active=runtime.clone(); let evidence=observed.clone();
    let router=Router::new().route("/v1/responses",post(move |headers:HeaderMap| {
        let state=active.get().unwrap().clone(); let evidence=evidence.clone();
        async move {
            let raw=headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let answer=Box::pin(model(&state,raw,proxy,&evidence)).await;
            let session=format!("conv_{}",Uuid::new_v4().simple());
            let response=format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple());
            let stream=async_stream::stream! {
                yield Ok::<_,Infallible>(Event::default().data(json!({"type":"response.completed","sequence_number":1,
                    "response":{"id":response,"conversation":{"id":session},"status":"completed","output":message_output(&answer)}}).to_string()));
            };
            axum::response::Sse::new(stream).into_response()
        }
    }));
    let db=connect_transaction_test_database("ws7_supervisor_write").await;
    let (state,_,engine)=Box::pin(setup_with_engine(db,Value::Null,Some(router))).await;
    assert!(runtime.set(state.clone()).is_ok());
    crate::services::assistant_settings_service::update(&state.db,OWNER,crate::services::assistant_settings_service::Update {
        access_mode:Some(crate::models::assistant_settings::AccessMode::AskEveryTime),..Default::default()
    }).await.unwrap();
    let ledger=Arc::new(std::sync::Mutex::new(Ledger {target:"Pending".into(),control:"Untouched".into(),..Default::default()}));
    let ledger_server=MockServer::start().await;
    let read=ledger.clone();
    Mock::given(method("POST")).and(path_regex(".*/bitable/v1/apps/bascnLedger/tables/tblLedger/records/search$"))
        .respond_with(move |_:&wiremock::Request| {
            let mut r=read.lock().unwrap(); let value=r.target.clone();r.reads.push(value);
            ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[
                {"record_id":"recTarget","fields":{"Status":r.target}},
                {"record_id":"recControl","fields":{"Status":r.control}}
            ],"has_more":false}}))
        }).mount(&ledger_server).await;
    let written=ledger.clone();
    Mock::given(method("PUT")).and(path_regex(".*/bitable/v1/apps/bascnLedger/tables/tblLedger/records/[^/]+$"))
        .respond_with(move |request:&wiremock::Request| {
            let record=request.url.path().rsplit('/').next().unwrap().to_owned();
            let body:Value=serde_json::from_slice(&request.body).unwrap();
            let mut r=written.lock().unwrap();r.writes.push((record.clone(),body.clone()));
            if record=="recTarget" {r.target=body["fields"]["Status"].as_str().unwrap().into();}
            else {r.control=body["fields"]["Status"].as_str().unwrap().into();}
            // The write response deliberately omits values. Only a separate
            // actual provider search can supply final-value evidence.
            ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"record":{"record_id":record}}}))
        }).mount(&ledger_server).await;
    let catalog=Uuid::new_v4().to_string();let endpoint=Uuid::new_v4().to_string();let connection=Uuid::new_v4().to_string();
    let base=if slug.ends_with("-bot") {ledger_server.uri()} else {format!("{}/open-apis",ledger_server.uri())};
    let mut service=crate::models::downstream_service::test_helpers::dummy_service();
    service.id=catalog.clone();service.slug=slug.into();service.name="Merchant test ledger".into();service.base_url=base.clone();service.created_by="system".into();
    state.db.collection::<DownstreamService>(SERVICES).insert_one(service).await.unwrap();
    crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&state.db).await.unwrap();
    state.db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(crate::test_utils::test_user_endpoint(&endpoint,OWNER,"Merchant ledger",&base,None,Some(&catalog))).await.unwrap();
    state.db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(crate::test_utils::test_user_service(&connection,OWNER,"ledger",&endpoint,Some(&catalog),None)).await.unwrap();
    let targets=team::resolve_targets(&state.db,&state.node_ws_manager,OWNER,OWNER,&["ledger".into()]).await.unwrap();
    let (agent,_)=team::create_specialist(&state.db,&state.encryption_keys,OWNER,team::CreateRequest {
        name:"accountant".into(),description:"Test ledger accountant".into(),display_name:None,persona:None,voice_instructions:None,
        targets,account_read:false,specialty:None,created_by:"user",machines:None,logins:None,
    }).await.unwrap().unwrap();
    team::set_agent_policy(&state.db,&crate::services::audit_service::AuditActor {user_id:OWNER.into(),
        ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},&agent.id,team::AgentPolicy {
        model:None,access_mode_override:Some(Some(crate::models::assistant_settings::AccessMode::Nyxmode)),
    }).await.unwrap();
    let (channel,key)=channel(&state,"gateway").await;
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&channel.id},doc! {"$set":{"owner_sender_ids":["7"]}}).await.unwrap();
    assert_eq!(put_binding(State(state.clone()),Path("bnd_ws7".into()),bearer(&key),Json(binding_body(&key,OWNER))).await.status(),StatusCode::OK);
    assert_eq!(put_conversation(State(state.clone()),Path(("bnd_ws7".into(),PARTITION.into())),bearer(&key)).await.status(),StatusCode::OK);
    let chat=MockServer::start().await;
    let encrypted=state.encryption_keys.encrypt(b"ws7-token").await.unwrap();
    state.db.collection::<Document>("channel_bots").update_one(doc! {"_id":&channel.channel_bot_id},doc! {"$set":{"bot_token_encrypted":bson::Binary {
        subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted
    }}}).await.unwrap();
    Mock::given(path_regex(".*/sendMessage$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":91}}))).mount(&chat).await;
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS.lock().unwrap().insert(channel.channel_bot_id.clone(),Arc::new(
        crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(&chat.uri())
    ));
    // Match normal gateway primary delivery as well as the separate local,
    // source-anchored Telegram supplemental review.
    let mut gateway=crate::models::downstream_service::test_helpers::dummy_service();gateway.id=Uuid::new_v4().to_string();gateway.slug=GATEWAY_SLUG.into();gateway.base_url=chat.uri();
    state.db.collection::<DownstreamService>(SERVICES).insert_one(gateway).await.unwrap();
    Mock::given(path_regex("/events/.*/reply-target$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.ws7","expires_at":(Utc::now()+ChronoDuration::hours(1)).timestamp()}))).mount(&chat).await;
    Mock::given(path_regex("/reply-targets/.*/messages$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}}))).mount(&chat).await;
    Journey {state,channel,key,chat,_ledger_server:ledger_server,ledger,observed,_engine:tokio_util::task::AbortOnDropHandle::new(engine)}
}

async fn owner(j:&Journey,text:&str,id:&str) -> Response {
    // The authenticated relay persists full inbound identity before invoking
    // NyxBot. This is the original-message anchor used by real supplemental sends.
    j.state.db.collection::<Document>("channel_messages").update_one(doc! {"_id":id},doc! {"$setOnInsert":{
        "conversation_id":&j.channel.route_id,"user_id":OWNER,"channel_bot_id":&j.channel.channel_bot_id,
        "platform":"telegram","platform_conversation_id":"123","direction":"inbound","platform_message_id":id,
        "sender_platform_id":"7","original_message":true,"content_type":"text",
        "agent_api_key_id":&j.channel.route_api_key_id,"created_at":bson::DateTime::now()
    }}).upsert(true).await.unwrap();
    let mut input=event(text,"7",id);input["event_context"]["activity"]["conversation"]["id"]=json!("123");
    respond(&j.state,&j.key,&input,id).await
}

async fn wait_review(j:&Journey) -> (ApprovalRequest,String) {
    tokio::time::timeout(std::time::Duration::from_secs(15),async {
        loop {
            if let Some(row)=j.state.db.collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"user_id":OWNER,"supervisor_write.review_confirmed":true,"status":"pending"}).await.unwrap() {
                for request in j.chat.received_requests().await.unwrap() {
                    if let Ok(body)=serde_json::from_slice::<Value>(&request.body)
                        && let Some(text)=body["text"].as_str()
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

async fn start(j:&Journey) -> tokio::task::JoinHandle<String> {
    let initial=body_text(owner(j,"Ask accountant to update the identified test row", "ws7-ask").await).await;
    assert!(!initial.is_empty());
    let code=regex::Regex::new(r"yes ([0-9]{4})").unwrap().captures(&initial).expect("sent assignment confirmation code")[1].to_owned();
    let response=owner(j,&format!("yes {code}"),"ws7-delegation-yes").await;
    tokio::spawn(body_text(response))
}

#[tokio::test]
async fn supervisor_write_two_approvals_real_catalog_and_independent_readback() {
    for (slug,proxy) in [("api-feishu",false),("api-feishu-bot",false),("api-feishu",true)] {
        let j=Box::pin(setup_journey(slug,proxy)).await;
        assert_eq!(crate::services::approval_service::user_approval_mode(&j.state.db,OWNER).await.unwrap(),crate::services::approval_policy::GlobalApprovalMode::WritesOnly);
        assert_eq!(j.state.db.collection::<Document>("service_approval_configs").count_documents(doc! {"user_id":OWNER}).await.unwrap(),0,"implicit mode must work");
        let result=start(&j).await;
        let (review,code)=wait_review(&j).await;
        assert_eq!(review.approval_mode,crate::models::service_approval_config::ApprovalMode::PerRequest);
        assert!(review.execution_proposal.is_none());
        assert_eq!(j.ledger.lock().unwrap().writes.len(),0);
        let decision=body_text(owner(&j,&format!("yes write-{code}"),"ws7-write-yes").await).await;
        assert!(decision.contains("Write approval saved"),"{decision}");
        let final_text=tokio::time::timeout(std::time::Duration::from_secs(25),result).await.unwrap().unwrap();
        let ledger=j.ledger.lock().unwrap();
        assert_eq!(ledger.writes.len(),1);assert_eq!(ledger.writes[0].0,"recTarget");
        assert_eq!(ledger.target,"Approved");assert_eq!(ledger.control,"Untouched");
        assert_eq!(ledger.reads,["Pending","Approved"],"provider search must read after the write");
        drop(ledger);
        assert!(j.observed.lock().await.iter().any(|r|r["stage"]=="independent_readback" && r["result"].to_string().contains("Approved")));
        assert!(final_text.contains("Approved"),"{final_text}");
        let replay=body_text(owner(&j,&format!("yes write-{code}"),"ws7-write-yes").await).await;
        assert!(!replay.is_empty());assert_eq!(j.ledger.lock().unwrap().writes.len(),1);
        let stored=j.state.db.collection::<ApprovalRequest>("approval_requests").find_one(doc! {"_id":&review.id}).await.unwrap().unwrap();
        assert!(stored.supervisor_write.unwrap().dm_decision_receipt.is_some());
        assert_eq!(j.state.db.collection::<Document>("approval_grants").count_documents(doc! {"user_id":OWNER}).await.unwrap(),0);
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS.lock().unwrap().remove(&j.channel.channel_bot_id);
    }
}

#[tokio::test]
async fn supervisor_write_rejection_and_legacy_approval_never_send() {
    for legacy in [false,true] {
        let j=Box::pin(setup_journey("api-feishu-bot",false)).await;
        let result=start(&j).await;let (review,code)=wait_review(&j).await;
        if legacy {
            j.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&review.id},doc! {"$set":{"status":"approved","decision_channel":"web"}}).await.unwrap();
        } else {
            let denied=body_text(owner(&j,&format!("no write-{code}"),"ws7-write-no").await).await;
            assert!(denied.contains("Write rejected"),"{denied}");
        }
        let _=tokio::time::timeout(std::time::Duration::from_secs(25),result).await.unwrap().unwrap();
        let ledger=j.ledger.lock().unwrap();assert!(ledger.writes.is_empty());assert_eq!(ledger.target,"Pending");assert_eq!(ledger.control,"Untouched");
        drop(ledger);
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS.lock().unwrap().remove(&j.channel.channel_bot_id);
    }
}
