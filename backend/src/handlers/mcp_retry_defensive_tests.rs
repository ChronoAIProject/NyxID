use super::*;
use crate::services::assistant_nyxagent as engine;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn invocation(tool: &str, args: &Value, form: usize) -> Value {
    let arguments = match form {
        0 => return json!({"name":tool,"arguments":args}),
        1 => json!({"tool_name":tool,"arguments_json":args.to_string()}),
        2 => json!({"tool_name":tool,"arguments":args}),
        3 => {
            let mut flat = args.clone();
            flat["tool_name"] = json!(tool);
            flat
        }
        _ => {
            json!({"tool_name":tool,"arguments_json":args.to_string(),"arguments":{"ignored":true},"decoration":"historical"})
        }
    };
    json!({"name":"nyx__call_tool","arguments":arguments})
}

async fn invoke(f: &Fixture, params: Value) -> Response {
    let auth = authenticate(f).await;
    direct_call(
        f,
        &auth,
        params["name"].as_str().unwrap(),
        params["arguments"].clone(),
    )
    .await
}

async fn retry_current(
    f: &Fixture,
) -> crate::models::assistant_conversation::AssistantConversation {
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let turn = row.active_turn.as_ref().unwrap().turn_id.clone();
    engine::finish_turn(
        &f.state.db,
        &row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::new("idle_timeout")),
        },
    )
    .await
    .unwrap();
    engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: Some(turn),
            attachment_ids: vec![],
            agent_id: None,
            conversation_id: Some(row.id),
            text: "Please manage my account".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap()
}

/// Resolve the same caller-specific catalog and MCP discovery used by dispatch.
/// Catalog connections publish service_id = user-service ID, not catalog ID.
async fn discovered_tool(f: &Fixture, service_id: &str, name: &str) -> Value {
    let auth = authenticate(f).await;
    let catalog = mcp_service::load_operation_catalog(
        &f.state.db,
        f.state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(&auth),
        mcp_service_scope(&auth),
    )
    .await
    .unwrap();
    let service = catalog
        .services
        .iter()
        .find(|s| s.service_id == service_id)
        .unwrap();
    assert!(service.executable);
    assert!(
        matches!(&service.source, mcp_service::McpToolSource::UserManaged {
        user_service_id, ..
    } if user_service_id == service_id)
    );
    let definition = mcp_service::generate_tool_definitions(std::slice::from_ref(service), None)
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
    let found = result(
        direct_call(f, &auth, "nyx__search_tools", json!({"query":name})).await,
        false,
    )
    .await;
    let published = found["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap();
    assert_eq!(published["service_id"], service_id, "{found}");
    assert_eq!(published["executable"], true);
    for (key, schema) in definition.input_schema["properties"].as_object().unwrap() {
        assert_eq!(&published["inputSchema"]["properties"][key], schema);
    }
    definition.input_schema
}

async fn assert_provider_success(f: &Fixture, params: Value) {
    // result() checks the real JSON-RPC result/content/isError envelope before
    // decoding its text. Ordinary JSON provider bodies stay byte-preserving.
    assert_eq!(
        result(invoke(f, params).await, false).await,
        json!({"saved":true})
    );
}

#[tokio::test]
async fn retry_provider_wrappers_do_not_repeat_completed_effects() {
    for (first_form, legacy) in [
        (0, false),
        (1, false),
        (2, false),
        (3, false),
        (4, false),
        (0, true),
        (1, true),
        (4, true),
    ] {
        let f = orchestrator_fixture("retry_provider_forms").await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/effect"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"saved":true})))
            .mount(&server)
            .await;
        let service = connected(&f.state.db, &f.owner, "nyx-vendor", &server.uri()).await;
        let other = connected(&f.state.db, &f.owner, "other-vendor", &server.uri()).await;
        let tool = "nyx-vendor__request";
        let schema = discovered_tool(&f, &service, tool).await;
        assert_eq!(schema["properties"]["method"]["type"], "string");
        assert!(schema["properties"].get("body").is_some());
        discovered_tool(&f, &other, "other-vendor__request").await;
        let args = json!({"method":"POST","path":"/effect","body":{"value":"original"}});
        let original = invocation(tool, &args, first_form);
        assert_provider_success(&f, original.clone()).await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].body_json::<Value>().unwrap(), args["body"]);
        let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        let activity = row.active_turn.as_ref().unwrap().activities.last().unwrap();
        assert_eq!(activity.status, "completed");
        assert_eq!(activity.label, tool);
        assert_eq!(
            activity.digest.as_deref(),
            Some(chat_activity_digest(&original).as_str())
        );
        assert_eq!(
            activity.retry_identity.as_deref(),
            Some(chat_activity_digest(&original).as_str())
        );
        assert_eq!(
            activity.business_identity.as_deref(),
            Some(chat_retry_digest(&original).as_str())
        );
        if legacy {
            let index = row.active_turn.as_ref().unwrap().activities.len() - 1;
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$unset":{format!("active_turn.activities.{index}.retry_identity"):"",format!("active_turn.activities.{index}.business_identity"):""}},
                )
                .await
                .unwrap();
        }
        let retry = retry_current(&f).await;
        let turn = retry.active_turn.as_ref().unwrap();
        // Older readers still receive their unprefixed same-form evidence.
        assert!(
            turn.retry_digests
                .contains(&chat_activity_digest(&original))
        );
        for form in 0..4 {
            let response = result(invoke(&f, invocation(tool, &args, form)).await, true).await;
            let expected = if legacy && first_form == 4 {
                "retry_evidence_requires_review"
            } else {
                "retry_duplicate_action"
            };
            assert_eq!(
                response["error"], expected,
                "legacy={legacy}, first={first_form}, next={form}: {response}"
            );
            if expected == "retry_evidence_requires_review" {
                assert_eq!(
                    response["message"],
                    "Historical completed effects cannot be distinguished from this request. Review completed actions before starting new work; no call was dispatched."
                );
            }
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
        if !legacy {
            let mut changed = args.clone();
            changed["body"]["value"] = json!("new intent");
            assert_eq!(
                result(invoke(&f, invocation(tool, &changed, 1)).await, false).await["saved"],
                true
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
            let chained = retry_current(&f).await;
            assert!(
                chained
                    .active_turn
                    .unwrap()
                    .retry_digests
                    .contains(&chat_retry_digest(&original))
            );
            let repeated = result(invoke(&f, invocation(tool, &args, 0)).await, true).await;
            assert_eq!(repeated["error"], "retry_duplicate_action");
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
        } else {
            // An unrelated canonical tool is provably different, even when the
            // old provider starts with nyx and the old wrapper was decorated.
            assert_eq!(
                result(
                    invoke(&f, invocation("other-vendor__request", &args, 0)).await,
                    false
                )
                .await["saved"],
                true
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn retry_provider_literal_argument_types_survive_activity_finalization() {
    let f = orchestrator_fixture("retry_provider_literal_types").await;
    assert!(
        !engine::retry_duplicate_any(
            &f.state.db,
            &f.owner,
            &f.row.id,
            f.chat.turn_id.as_deref().unwrap(),
            "v2:unused",
            Some("nyx-typed__write"),
            || panic!("ordinary turn must not expand legacy hashes")
        )
        .await
        .unwrap()
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/typed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"saved":true})))
        .mount(&server)
        .await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "nyx-typed".into();
    catalog.name = "Synthetic typed provider".into();
    catalog.base_url = server.uri();
    // A connection catalog is explicitly attached to a personal user service.
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let service = connected(&f.state.db, &f.owner, "nyx-typed", &server.uri()).await;
    let connection = f
        .state
        .db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&service})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&connection.endpoint_id},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    let now = bson::DateTime::now();
    f.state.db.collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"service_id":&catalog.id,
            "name":"write","method":"POST","path":"/typed","is_active":true,
            "request_content_type":"application/json","request_body_required":true,
            "request_body_schema":{"type":"object","properties":{"arguments_json":{}},"required":["arguments_json"]},
            "created_at":now,"updated_at":now}).await.unwrap();
    let tool = "nyx-typed__write";
    let schema = discovered_tool(&f, &service, tool).await;
    assert_eq!(schema["required"], json!(["arguments_json"]));
    assert_eq!(schema["properties"]["arguments_json"], json!({}));
    assert!(schema["properties"].get("body").is_none());
    let auth = authenticate(&f).await;
    let loaded = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let selected = loaded.iter().find(|s| s.service_id == service).unwrap();
    assert!(
        matches!(&selected.source, mcp_service::McpToolSource::UserManaged {
        user_service_id, catalog_service_id: Some(id), ..
    } if user_service_id == &service && id == &catalog.id)
    );
    let string = json!({"arguments_json":"{\"value\":1}"});
    let object = json!({"arguments_json":{"value":1}});
    let original = invocation(tool, &string, 0);
    assert_provider_success(&f, original.clone()).await;
    let retry = retry_current(&f).await;
    assert!(
        retry
            .active_turn
            .as_ref()
            .unwrap()
            .retry_digests
            .contains(&chat_activity_digest(&original))
    );
    // Legacy parsing produces the same exact hash for these different provider
    // inputs. Modern matching must use the literal v2 identity exclusively.
    assert_eq!(
        chat_activity_digest(&original),
        chat_activity_digest(&invocation(tool, &object, 0))
    );
    assert_eq!(
        result(invoke(&f, invocation(tool, &object, 1)).await, false).await["saved"],
        true
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body_json::<Value>().unwrap(), string);
    assert_eq!(requests[1].body_json::<Value>().unwrap(), object);
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let activity = row.active_turn.unwrap().activities.pop().unwrap();
    assert_eq!(activity.status, "completed");
    assert_eq!(
        activity.business_identity.as_deref(),
        Some(chat_retry_digest(&invocation(tool, &object, 1)).as_str())
    );
    retry_current(&f).await;
    for args in [&string, &object] {
        for form in [0, 1, 2] {
            assert_eq!(
                result(invoke(&f, invocation(tool, args, form)).await, true).await["error"],
                "retry_duplicate_action"
            );
        }
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn retry_provider_argument_precedence_and_errors_do_not_dispatch_fallback() {
    let f = orchestrator_fixture("retry_argument_precedence").await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/effect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"saved":true})))
        .mount(&server)
        .await;
    let service = connected(&f.state.db, &f.owner, "nyx-precedence", &server.uri()).await;
    let tool = "nyx-precedence__request";
    discovered_tool(&f, &service, tool).await;
    let args = json!({"method":"POST","path":"/effect","body":{"value":"chosen"}});
    let auth = authenticate(&f).await;
    let invalid = direct_call(
        &f,
        &auth,
        "nyx__call_tool",
        json!({"tool_name":tool,
        "arguments_json":"{malformed","arguments":args,"method":"POST","path":"/effect"}),
    )
    .await;
    let raw = axum::body::to_bytes(invalid.into_body(), 20_000)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("arguments_json must be a valid JSON string.")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    for wrapper in [
        json!({"tool_name":tool,"arguments_json":args.to_string(),"arguments":{"method":"DELETE"},"method":"DELETE"}),
        json!({"tool_name":tool,"arguments_json":false,"arguments":args,"method":"DELETE"}),
        json!({"tool_name":tool,"arguments_json":false,"method":"POST","path":"/effect","body":{"value":"chosen"}}),
    ] {
        assert_eq!(
            result(
                direct_call(&f, &auth, "nyx__call_tool", wrapper).await,
                false
            )
            .await["saved"],
            true
        );
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    for request in requests {
        assert_eq!(request.body_json::<Value>().unwrap(), args["body"]);
    }
    // result_query remains an independently validated projection. Supplying it
    // for a write refuses before another provider effect is dispatched.
    for (query, expected) in [
        (Value::Null, "result_query must contain 1–256 characters"),
        (
            json!({"section":"unsupported"}),
            "result_query must contain 1–256 characters",
        ),
        (
            json!(" \tOverview 中文\n "),
            "result_query requires a published Notion block-children GET",
        ),
    ] {
        let projected = direct_call(
            &f,
            &auth,
            "nyx__call_tool",
            json!({"tool_name":tool,
            "arguments_json":args.to_string(),"result_query":query}),
        )
        .await;
        let raw = axum::body::to_bytes(projected.into_body(), 20_000)
            .await
            .unwrap();
        let rpc: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(rpc["result"]["isError"], true, "{rpc}");
        assert_eq!(rpc["result"]["content"][0]["text"], expected);
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
    f.state.db.drop().await.unwrap();
}

// Frozen fca745e13 algorithms: intentionally independent of the production
// wrapper decoder, legacy_writer_digest and v2 matcher. These fixtures emulate
// base projection/admission/lookup, not execution of a second server binary.
fn base_exact(params: &Value) -> String {
    let mut args = params.get("arguments").cloned().unwrap_or(Value::Null);
    if let Some(inner) = args
        .get("arguments_json")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        args["arguments_json"] = inner;
    }
    crate::services::assistant_continuation::canonical_digest(
        &json!({"name":params.get("name").unwrap_or(&Value::Null),"arguments":args}),
    )
}

fn base_native(params: &Value) -> String {
    let (name, mut args) = if params["name"] == "nyx__call_tool" {
        (
            params["arguments"]["tool_name"].as_str(),
            params["arguments"]["arguments_json"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok()),
        )
    } else {
        (params["name"].as_str(), params.get("arguments").cloned())
    };
    if let Some(args) = args.as_mut().and_then(Value::as_object_mut) {
        let field = match name {
            Some("nyxid__create_schedule") => Some("acknowledgement_id"),
            Some("nyxid__remember") => Some("recovery"),
            _ => None,
        };
        if let Some(field) = field {
            args.remove(field);
            return base_exact(&json!({"name":name,"arguments":args}));
        }
    }
    base_exact(params)
}

async fn base_admission_digests(f: &Fixture, turn: &str) -> Vec<String> {
    let reply = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"conversation_id":&f.row.id,"turn_id":turn,"role":"assistant"})
        .projection(doc! {"activities.digest":1,"activities.retry_identity":1,
        "activities.status":1,"activity_count":1})
        .await
        .unwrap()
        .unwrap();
    let activities = reply.get_array("activities").unwrap();
    assert_eq!(
        reply.get_i64("activity_count").unwrap(),
        activities.len() as i64
    );
    let mut hashes = vec![];
    for activity in activities {
        let activity = activity.as_document().unwrap();
        assert_eq!(activity.get_str("status").unwrap(), "completed");
        for key in ["digest", "retry_identity"] {
            if let Ok(digest) = activity.get_str(key) {
                assert!(!digest.starts_with("v2:"));
                if !hashes.iter().any(|old| old == digest) {
                    hashes.push(digest.to_string());
                }
            }
        }
        assert!(!activity.contains_key("business_identity"));
    }
    hashes
}

fn native_input(tool: &str) -> Value {
    if tool == "nyxid__create_schedule" {
        json!({"label":"Synthetic Retry fixture","instruction":"Say hello",
            "owner_timezone":"UTC","schedule":{"kind":"at","at":"2099-12-31T15:00:00Z"}})
    } else {
        json!({"text":"Synthetic private memory"})
    }
}

async fn native_effect(f: &Fixture, tool: &str, form: usize) -> (Value, Value) {
    native_effect_with_args(f, tool, form, native_input(tool)).await
}

async fn native_effect_with_args(
    f: &Fixture,
    tool: &str,
    form: usize,
    mut args: Value,
) -> (Value, Value) {
    let mut approved = Vec::<String>::new();
    // Exercise each real owner card, not a blanket fence bypass or an assumed
    // successful second call. Both card and effect keep normal native dispatch.
    for _ in 0..4 {
        let params = invocation(tool, &args, form);
        let response = invoke(f, params.clone()).await;
        let raw = axum::body::to_bytes(response.into_body(), 100_000)
            .await
            .unwrap();
        let rpc: Value = serde_json::from_slice(&raw).unwrap();
        assert!(rpc.get("error").is_none(), "{rpc}");
        let value: Value =
            serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        if rpc["result"]["isError"] == false {
            if tool == "nyxid__create_schedule" {
                assert!(value["schedule"]["id"].is_string(), "{value}");
            } else {
                assert!(value["remembered"].is_string(), "{value}");
            }
            let cards = acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap();
            for id in &approved {
                let card = cards.iter().find(|c| &c.id == id).unwrap();
                assert_eq!(card.status, "used");
            }
            return (params, value);
        }
        assert_eq!(value["error"], "acknowledgement_required", "{value}");
        let id = value["acknowledgement_id"].as_str().unwrap();
        let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        engine::finish_turn(
            &f.state.db,
            &row,
            &row.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "Waiting for the owner decision".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        )
        .await
        .unwrap();
        let decision = acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        assert_eq!(decision.status, "allowed");
        assert_eq!(decision.tool_name.as_deref(), Some(tool));
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            &engine::TurnRequest {
                acknowledgement_id: Some(id.to_string()),
                retry_of: None,
                attachment_ids: vec![],
                agent_id: None,
                conversation_id: Some(f.row.id.clone()),
                text: "Please manage my account".into(),
                model: None,
                access_mode: None,
            },
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        if let Some(retry) = value.get("retry_arguments") {
            assert!(retry.is_object());
            args = retry.clone();
        }
        args["acknowledgement_id"] = json!(id);
        approved.push(id.to_string());
    }
    panic!("native effect did not complete after required owner decisions");
}

fn native_handle(tool: &str, replacement: bool) -> Value {
    let id = if replacement {
        "22222222-2222-4222-8222-222222222222"
    } else {
        "11111111-1111-4111-8111-111111111111"
    };
    if tool == "nyxid__remember" {
        json!({"failed_activity_id":id,"part":1,"total":1})
    } else {
        json!(id)
    }
}

async fn assert_native_effect_count(f: &Fixture, tool: &str) {
    if tool == "nyxid__create_schedule" {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::trigger::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&f.owner})
                .await
                .unwrap(),
            1
        );
    } else {
        let agent = f
            .state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(
                crate::models::assistant_agent::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&f.chat.agent_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(agent.memory.len(), 1);
        assert_eq!(agent.memory[0].text, "Synthetic private memory");
    }
}

#[tokio::test]
async fn retry_new_native_writer_preserves_base_reader_fences() {
    for tool in ["nyxid__create_schedule", "nyxid__remember"] {
        for first in 0..4 {
            let f = orchestrator_fixture("retry_new_native_base_reader").await;
            let (original, _) = native_effect(&f, tool, first).await;
            let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
            let turn = row.active_turn.as_ref().unwrap();
            let activity = turn.activities.last().unwrap();
            let expected = base_native(&invocation(tool, &native_input(tool), 0));
            assert_eq!(
                activity.digest.as_deref(),
                Some(base_exact(&original).as_str())
            );
            assert_eq!(activity.retry_identity.as_deref(), Some(expected.as_str()));
            assert_eq!(
                activity.business_identity.as_deref(),
                Some(chat_retry_digest(&original).as_str())
            );
            let old_turn = turn.turn_id.clone();
            retry_current(&f).await;
            let old = base_admission_digests(&f, &old_turn).await;
            assert!(old.contains(&expected));
            // A base replica admits only these existing fields. Modern
            // transport then consumes that base-admitted Retry without an
            // additive retry_match, using the same frozen base candidates.
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.retry_digests":&old},
                    "$unset":{"active_turn.retry_match":""}},
                )
                .await
                .unwrap();
            for form in [0, 1] {
                // the base normalizer supports direct/JSON
                for changed_handle in [false, true] {
                    let mut args = native_input(tool);
                    if changed_handle {
                        args[if tool == "nyxid__create_schedule" {
                            "acknowledgement_id"
                        } else {
                            "recovery"
                        }] = native_handle(tool, true);
                    }
                    let next = invocation(tool, &args, form);
                    assert!(old.contains(&base_exact(&next)) || old.contains(&base_native(&next)));
                    let refused = result(invoke(&f, next).await, true).await;
                    assert_eq!(refused["error"], "retry_duplicate_action", "{refused}");
                    assert_native_effect_count(&f, tool).await;
                }
            }
            // Refused duplicates retain all three evidences through another
            // finalization, including fields the base reader still consumes.
            let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
            let turn = row.active_turn.as_ref().unwrap();
            let refused = turn.activities.last().unwrap();
            assert_eq!(refused.retry_identity.as_deref(), Some(expected.as_str()));
            assert_eq!(
                refused.failure.as_ref().unwrap().kind,
                "retry_duplicate_action"
            );
            let prior = turn.turn_id.clone();
            retry_current(&f).await;
            assert!(base_admission_digests(&f, &prior).await.contains(&expected));
            for form in [2, 3] {
                // fully upgraded readers also normalize these
                assert_eq!(
                    result(
                        invoke(&f, invocation(tool, &native_input(tool), form)).await,
                        true
                    )
                    .await["error"],
                    "retry_duplicate_action"
                );
            }
            assert_native_effect_count(&f, tool).await;
            f.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn retry_old_native_writer_requires_review_for_retired_handles() {
    for tool in ["nyxid__create_schedule", "nyxid__remember"] {
        for (index, label) in [tool, "tool", "service_tool", "nyx__call_tool"]
            .into_iter()
            .enumerate()
        {
            let f = orchestrator_fixture("retry_old_native_retired_handle").await;
            native_effect(&f, tool, 0).await;
            let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
            let mut successful =
                bson::to_document(row.active_turn.as_ref().unwrap().activities.last().unwrap())
                    .unwrap();
            let mut args = native_input(tool);
            let handle = if tool == "nyxid__create_schedule" {
                "acknowledgement_id"
            } else {
                "recovery"
            };
            args[handle] = native_handle(tool, false);
            let historical = invocation(tool, &args, 2 + index % 2);
            // Synthetic old-writer snapshot of the real saved effect: nested
            // and flat base hashes retain a one-use/presentation handle. No
            // new credential or authority is implied by these fixture bytes.
            let digest = base_exact(&historical);
            assert_eq!(base_native(&historical), digest);
            successful.insert("label", label);
            successful.insert("digest", &digest);
            successful.insert("retry_identity", &digest);
            successful.remove("business_identity");
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.activities":[successful],
                    "active_turn.activity_count":1_i64,"active_turn.activity_total":1_i64}},
                )
                .await
                .unwrap();
            retry_current(&f).await;
            for form in 0..4 {
                for omit_handle in [false, true] {
                    let mut next = native_input(tool);
                    if !omit_handle {
                        next[handle] = native_handle(tool, true);
                    }
                    let refused =
                        result(invoke(&f, invocation(tool, &next, form)).await, true).await;
                    assert_eq!(
                        refused["error"], "retry_evidence_requires_review",
                        "{refused}"
                    );
                    assert_eq!(
                        refused["message"],
                        "Historical completed effects cannot be distinguished from this request. Review completed actions before starting new work; no call was dispatched."
                    );
                }
            }
            let retry = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
            assert!(
                retry.active_turn.as_ref().unwrap().activities.is_empty(),
                "ambiguous requests are neither duplicate-completed nor unknown effects"
            );
            assert_native_effect_count(&f, tool).await;
            // Base-admitted Retry has no additive matching metadata. Its
            // unmatched native write also requires review; reads still work.
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$unset":{"active_turn.retry_match":""}},
                )
                .await
                .unwrap();
            assert_eq!(
                result(
                    invoke(&f, invocation(tool, &native_input(tool), 0)).await,
                    true
                )
                .await["error"],
                "retry_evidence_requires_review"
            );
            // Native reads remain available; the exact setup reader exception
            // may run repeatedly, even on a Retry admitted by a base replica.
            for name in [
                "nyxid__list_schedules",
                "nyxid__get_agent_skills",
                "nyxid__get_agent_skills",
            ] {
                let args = if name == "nyxid__get_agent_skills" {
                    json!({"agent":f.chat.agent_id})
                } else {
                    json!({})
                };
                result(invoke(&f, invocation(name, &args, 2)).await, false).await;
            }
            // An exact historical request is proven completed, distinct from
            // the review-needed refusal and recorded as a duplicate only.
            assert_eq!(
                result(invoke(&f, historical).await, true).await["error"],
                "retry_duplicate_action"
            );
            assert_native_effect_count(&f, tool).await;
            retry_current(&f).await;
            // Chained Retry retains the unresolved historical ancestor. A
            // changed intent is not silently authorized as a fresh effect.
            let mut changed = native_input(tool);
            changed[if tool == "nyxid__create_schedule" {
                "label"
            } else {
                "text"
            }] = json!("changed intent");
            assert_eq!(
                result(invoke(&f, invocation(tool, &changed, 0)).await, true).await["error"],
                "retry_evidence_requires_review"
            );
            assert_native_effect_count(&f, tool).await;
            f.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn retry_old_normalized_native_writer_keeps_known_duplicate_fences() {
    for tool in ["nyxid__create_schedule", "nyxid__remember"] {
        for form in [0, 1] {
            let f = orchestrator_fixture("retry_old_normalized_native").await;
            native_effect(&f, tool, form).await;
            let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
            let mut activity =
                bson::to_document(row.active_turn.as_ref().unwrap().activities.last().unwrap())
                    .unwrap();
            let mut args = native_input(tool);
            let handle = if tool == "nyxid__create_schedule" {
                "acknowledgement_id"
            } else {
                "recovery"
            };
            args[handle] = native_handle(tool, false);
            let original = invocation(tool, &args, form);
            let exact = base_exact(&original);
            let normalized = base_native(&original);
            assert_ne!(exact, normalized);
            activity.insert("digest", exact);
            activity.insert("retry_identity", normalized);
            activity.remove("business_identity");
            // Faithful base writer layout, independent of current normalization.
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.activities":[activity],
                    "active_turn.activity_count":1_i64,"active_turn.activity_total":1_i64}},
                )
                .await
                .unwrap();
            retry_current(&f).await;
            for next in 0..4 {
                for changed_handle in [false, true] {
                    let mut args = native_input(tool);
                    if changed_handle {
                        args[handle] = native_handle(tool, true);
                    }
                    let refused =
                        result(invoke(&f, invocation(tool, &args, next)).await, true).await;
                    assert_eq!(refused["error"], "retry_duplicate_action", "{refused}");
                }
            }
            assert_native_effect_count(&f, tool).await;
            f.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn retry_decorated_legacy_run_schedule_now_does_not_enqueue_again() {
    let f = orchestrator_fixture("retry_legacy_run_schedule_now").await;
    let (_, schedule) = native_effect(&f, "nyxid__create_schedule", 0).await;
    let id = schedule["schedule"]["id"].as_str().unwrap();
    let runs = f
        .state
        .db
        .collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME);
    assert_eq!(
        runs.count_documents(doc! {"trigger_id":id}).await.unwrap(),
        0
    );
    let args = json!({"id":id});
    let decorated = invocation("nyxid__run_schedule_now", &args, 4);
    let queued = result(invoke(&f, decorated.clone()).await, false).await;
    let run = queued["run"]["id"].as_str().unwrap();
    assert_eq!(
        runs.count_documents(doc! {"trigger_id":id}).await.unwrap(),
        1
    );
    assert_eq!(
        runs.find_one(doc! {"_id":run})
            .await
            .unwrap()
            .unwrap()
            .get_str("outcome")
            .unwrap(),
        "pending"
    );
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let activities = &row.active_turn.as_ref().unwrap().activities;
    let last = activities.last().unwrap();
    assert_eq!(last.label, "nyxid__run_schedule_now");
    assert_eq!(last.status, "completed");
    assert_eq!(
        last.digest.as_deref(),
        Some(base_exact(&decorated).as_str())
    );
    let index = activities.len() - 1;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$unset":{
            format!("active_turn.activities.{index}.retry_identity"):"",
            format!("active_turn.activities.{index}.business_identity"):""}},
        )
        .await
        .unwrap();
    let retry = retry_current(&f).await;
    assert!(
        retry
            .active_turn
            .as_ref()
            .unwrap()
            .retry_match
            .as_ref()
            .unwrap()
            .historical_native_tools
            .contains(&"nyxid__run_schedule_now".to_string())
    );
    for form in 0..4 {
        let refused = result(
            invoke(&f, invocation("nyxid__run_schedule_now", &args, form)).await,
            true,
        )
        .await;
        assert_eq!(
            refused["error"], "retry_evidence_requires_review",
            "{refused}"
        );
        assert_eq!(
            refused["message"],
            "Historical completed effects cannot be distinguished from this request. Review completed actions before starting new work; no call was dispatched."
        );
    }
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    assert!(row.active_turn.as_ref().unwrap().activities.is_empty());
    // Actual closed reads and a provably different native write remain usable.
    result(
        invoke(&f, invocation("nyxid__list_schedules", &json!({}), 2)).await,
        false,
    )
    .await;
    native_effect(&f, "nyxid__remember", 0).await;
    assert_eq!(
        result(invoke(&f, decorated).await, true).await["error"],
        "retry_duplicate_action"
    );
    retry_current(&f).await;
    assert_eq!(
        result(
            invoke(&f, invocation("nyxid__run_schedule_now", &args, 0)).await,
            true
        )
        .await["error"],
        "retry_evidence_requires_review"
    );
    assert_eq!(
        runs.count_documents(doc! {"trigger_id":id}).await.unwrap(),
        1
    );
    assert_eq!(
        runs.find_one(doc! {"_id":run})
            .await
            .unwrap()
            .unwrap()
            .get_str("outcome")
            .unwrap(),
        "pending"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn retry_new_memory_recovery_writer_preserves_base_reader_fences() {
    let f = orchestrator_fixture("retry_memory_recovery_base_reader").await;
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let rejected = result(
        invoke(
            &f,
            invocation("nyxid__remember", &json!({"text":"x".repeat(501)}), 0),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(rejected["details"]["reason"], "note_too_long");
    let failed = rejected["details"]["failed_activity_id"].as_str().unwrap();
    let mut args = native_input("nyxid__remember");
    args["recovery"] = json!({"failed_activity_id":failed,"part":1,"total":1});
    // The required owner card starts a fresh, normally admitted continuation.
    // That exact approval permits the supported independent recovery save.
    let (original, saved) = native_effect_with_args(&f, "nyxid__remember", 2, args).await;
    assert_eq!(saved["recovery_status"], "unlinked_after_approval");
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let turn = row.active_turn.as_ref().unwrap();
    assert_eq!(turn.activities.len(), 1);
    let original_args = retry_identity::decode_arguments(&original["arguments"]).unwrap();
    let mut baseline = original_args.clone();
    baseline.as_object_mut().unwrap().remove("recovery");
    let expected = base_native(&invocation("nyxid__remember", &baseline, 0));
    assert_eq!(
        turn.activities[0].retry_identity.as_deref(),
        Some(expected.as_str())
    );
    let prior = turn.turn_id.clone();
    retry_current(&f).await;
    let old = base_admission_digests(&f, &prior).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.retry_digests":&old},
            "$unset":{"active_turn.retry_match":""}},
        )
        .await
        .unwrap();
    for form in [0, 1] {
        for replacement in [false, true] {
            let mut args = baseline.clone();
            if replacement {
                args["recovery"] = native_handle("nyxid__remember", true);
            }
            let next = invocation("nyxid__remember", &args, form);
            assert!(old.contains(&base_native(&next)));
            assert_eq!(
                result(invoke(&f, next).await, true).await["error"],
                "retry_duplicate_action"
            );
        }
    }
    assert_native_effect_count(&f, "nyxid__remember").await;
    f.state.db.drop().await.unwrap();
}
