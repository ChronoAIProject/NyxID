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
        connected(&f.state.db, &f.owner, "nyx-vendor", &server.uri()).await;
        connected(&f.state.db, &f.owner, "other-vendor", &server.uri()).await;
        let tool = "nyx-vendor__proxy";
        let args = json!({"method":"POST","path":"/effect","body":{"value":"original"}});
        let original = invocation(tool, &args, first_form);
        assert_eq!(
            result(invoke(&f, original.clone()).await, false).await["saved"],
            true
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        let activity = &row.active_turn.as_ref().unwrap().activities[0];
        assert_eq!(activity.status, "completed");
        assert_eq!(activity.label, tool);
        assert_eq!(
            activity.digest.as_deref(),
            Some(chat_activity_digest(&original).as_str())
        );
        assert_eq!(
            activity.retry_identity.as_deref(),
            Some(chat_retry_digest(&original).as_str())
        );
        if legacy {
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$unset":{"active_turn.activities.0.retry_identity":""}},
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
                    invoke(&f, invocation("other-vendor__proxy", &args, 0)).await,
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
    let service = connected(&f.state.db, &f.owner, "nyx-typed", &server.uri()).await;
    let now = bson::DateTime::now();
    f.state.db.collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"service_id":service,
            "name":"write","method":"POST","path":"/typed","is_active":true,
            "request_content_type":"application/json","request_body_required":true,
            "request_body_schema":{"type":"object","properties":{"arguments_json":{}},"required":["arguments_json"]},
            "created_at":now,"updated_at":now}).await.unwrap();
    let tool = "nyx-typed__write";
    let string = json!({"arguments_json":"{\"value\":1}"});
    let object = json!({"arguments_json":{"value":1}});
    let original = invocation(tool, &string, 0);
    assert_eq!(
        result(invoke(&f, original.clone()).await, false).await["saved"],
        true
    );
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
        activity.retry_identity.as_deref(),
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
    connected(&f.state.db, &f.owner, "nyx-precedence", &server.uri()).await;
    let tool = "nyx-precedence__proxy";
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
        json!({"tool_name":tool,"arguments_json":false,"method":"POST","path":"/effect","body":{"value":"chosen"},"result_query":null}),
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
    let projected = direct_call(
        &f,
        &auth,
        "nyx__call_tool",
        json!({"tool_name":tool,
        "arguments_json":args.to_string(),"result_query":{"section":"unsupported"}}),
    )
    .await;
    assert!(!tool_succeeded(&projected));
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    f.state.db.drop().await.unwrap();
}
