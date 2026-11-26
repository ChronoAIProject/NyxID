use super::*;
use crate::services::assistant_tool_result_service as retained;

async fn page(response: Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 20_000)
        .await
        .unwrap();
    let envelope: Value = serde_json::from_slice(&body).unwrap();
    assert!(envelope["result"].to_string().len() <= retained::BUDGET);
    assert_eq!(envelope["result"]["isError"], false, "{envelope}");
    serde_json::from_str(envelope["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn google_docs_tabs_and_export_and_large_text_are_read_once_with_real_mcp_dispatch() {
    let f = boxed_chat_fixture("large_result_google").await;
    let text = "正文和😀\n".repeat(2200);
    let docs=json!({"documentId":"synthetic","revisionId":"r1","tabs":[{"documentTab":{"body":{"content":[{"paragraph":{"elements":[{"textRun":{"content":text}}]}}]}}}]}).to_string();
    let bodies = Arc::new([docs, text, "x".repeat(30_000)]);
    let hits = Arc::new(AtomicUsize::new(0));
    let upstream = {
        let bodies = bodies.clone();
        let hits = hits.clone();
        Router::new().fallback(any(move |uri: axum::http::Uri| {
            let i = match uri.path() {
                "/documents/synthetic" => {
                    assert!(
                        uri.query()
                            .unwrap_or_default()
                            .contains("includeTabsContent=true")
                    );
                    0
                }
                "/files/synthetic/export" => {
                    assert!(
                        uri.query()
                            .unwrap_or_default()
                            .contains("mimeType=text%2Fplain")
                    );
                    1
                }
                _ => 2,
            };
            hits.fetch_add(1, Ordering::SeqCst);
            let body = bodies[i].clone();
            async move {
                (
                    [(
                        "content-type",
                        if i == 0 {
                            "application/json"
                        } else {
                            "text/plain"
                        },
                    )],
                    body,
                )
            }
        }))
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "api-google-drive".into();
    catalog.name = "Synthetic document service".into();
    catalog.base_url = format!("http://{}", listener.local_addr().unwrap());
    catalog.service_category = "internal".into();
    catalog.auth_method = "none".into();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let now = bson::DateTime::now();
    for (name, path, parameters) in [
        (
            "docs_get_document",
            "/documents/{documentId}",
            json!([{"name":"documentId","in":"path","required":true,"schema":{"type":"string"}},{"name":"includeTabsContent","in":"query","schema":{"type":"boolean"}}]),
        ),
        (
            "drive_export_file",
            "/files/{fileId}/export",
            json!([{"name":"fileId","in":"path","required":true,"schema":{"type":"string"}},{"name":"mimeType","in":"query","schema":{"type":"string"}}]),
        ),
        ("large_text", "/text", json!([])),
    ] {
        f.state.db.collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME).insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"service_id":&catalog.id,"name":name,"method":if name=="large_text" {"POST"} else {"GET"},"path":path,"parameters":bson::to_bson(&parameters).unwrap(),"is_active":true,"created_at":now,"updated_at":now}).await.unwrap();
    }
    let refusal = service_gate(
        &f.state.db,
        &f.chat,
        &catalog.id,
        &catalog.slug,
        &catalog.name,
        true,
    )
    .await
    .unwrap()
    .unwrap();
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.row.id,
        refusal["acknowledgement_id"].as_str().unwrap(),
        true,
    )
    .await
    .unwrap();
    let mut last_reference = Value::Null;
    for (i, (name, args)) in [
        (
            "api-google-drive__docs_get_document",
            json!({"documentId":"synthetic","includeTabsContent":true}),
        ),
        (
            "api-google-drive__drive_export_file",
            json!({"fileId":"synthetic","mimeType":"text/plain"}),
        ),
        ("api-google-drive__large_text", json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let auth = authenticate(&f).await;
        let reply = if i % 2 == 0 {
            direct_call(&f, &auth, name, args).await
        } else {
            call(&f, &auth, name, args).await
        };
        let expected = result_digest(&retained::envelope(&bodies[i])).0;
        assert_eq!(
            reply.extensions().get::<ToolResultDigest>().unwrap().0,
            expected
        );
        let first = page(reply).await;
        last_reference = first["ref"].clone();
        let mut assembled = first["content"].as_str().unwrap().to_owned();
        let count = first["parts"].as_u64().unwrap();
        for part in 1..count {
            // Each real HTTP callback authenticates again and updates last_used_at.
            let auth = authenticate(&f).await;
            let value = page(
                call(
                    &f,
                    &auth,
                    retained::TOOL_NAME,
                    json!({"ref":first["ref"],"part":part}),
                )
                .await,
            )
            .await;
            assembled.push_str(value["content"].as_str().unwrap());
        }
        assert_eq!(assembled, bodies[i]);
        assert_eq!(hits.load(Ordering::SeqCst), i + 1);
    }
    // A completed write stays fenced on Retry; the old snapshot is not adopted.
    use crate::services::assistant_nyxagent as engine;
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let failed_turn = row.active_turn.as_ref().unwrap().turn_id.clone();
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
            retry_of: Some(failed_turn),
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
    .unwrap();
    let auth = authenticate(&f).await;
    let retry = result(
        direct_call(&f, &auth, "api-google-drive__large_text", json!({})).await,
        true,
    )
    .await;
    assert_eq!(retry["error"], "retry_duplicate_action");
    let refused = direct_call(
        &f,
        &auth,
        retained::TOOL_NAME,
        json!({"ref":last_reference,"part":0}),
    )
    .await;
    let raw = axum::body::to_bytes(refused.into_body(), 20_000)
        .await
        .unwrap();
    let refused: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn part_reads_are_guest_safe_read_only_in_every_access_mode_and_hidden_from_non_chat() {
    let f = boxed_chat_fixture("large_result_modes").await;
    assert_eq!(
        acks::known_action_class(retained::TOOL_NAME, &json!({})),
        Some(acks::OwnerActionClass::Read)
    );
    for guest in [false, true] {
        mark_guest(&f, guest).await;
        let auth = authenticate(&f).await;
        let chat = auth.chat.as_ref().unwrap();
        let first = retained::retain(
            &f.state.db,
            &f.state.encryption_keys,
            chat,
            &auth.user_id,
            &"private".repeat(4000),
        )
        .await
        .unwrap();
        let before = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        for mode in ["autopilot", "nyxmode", "ask_every_time"] {
            f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_settings::COLLECTION_NAME)
                .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"access_mode":mode}})
                .upsert(true)
                .await
                .unwrap();
            let reply = page(
                direct_call(
                    &f,
                    &auth,
                    retained::TOOL_NAME,
                    json!({"ref":first["ref"],"part":1}),
                )
                .await,
            )
            .await;
            assert_eq!(reply["part"], 1);
        }
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_acknowledgement::COLLECTION_NAME
                )
                .count_documents(doc! {})
                .await
                .unwrap(),
            before
        );
        let tools = handle_tools_list(
            &f.state,
            &auth,
            None,
            &JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "tools/list".into(),
                params: None,
            },
        )
        .await;
        let raw = axum::body::to_bytes(tools.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        let tool = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == retained::TOOL_NAME)
            .unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["idempotentHint"], true);
        assert!(tool["title"].as_str().is_some());
    }
    let mut auth = authenticate(&f).await;
    auth.chat = None;
    let original = vec![json!({"type":"text","text":"x".repeat(30000)})];
    let (content, digest) =
        retained_results::project(&f.state, &auth, original.clone(), false).await;
    assert_eq!(content, original);
    assert!(digest.is_none());
    assert!(
        !mcp_service::generate_tool_definitions(&[], None)
            .iter()
            .any(|v| v.name == retained::TOOL_NAME)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn projection_preserves_small_results_and_large_business_digests() {
    let f = boxed_chat_fixture("large_result_digest").await;
    let auth = authenticate(&f).await;
    let small = vec![json!({"type":"text","text":"ordinary"})];
    let (actual, digest) = retained_results::project(&f.state, &auth, small.clone(), false).await;
    assert_eq!(actual, small);
    assert!(digest.is_none());
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let large = vec![json!({"type":"text","text":"same output".repeat(3000)})];
    let (a, da) = retained_results::project(&f.state, &auth, large.clone(), false).await;
    let (b, db) = retained_results::project(&f.state, &auth, large.clone(), false).await;
    assert_ne!(a, b);
    assert_eq!(da.unwrap().0, db.unwrap().0);
    let error = retained_results::project(&f.state, &auth, large.clone(), true).await;
    assert_eq!(error.0, large);
    assert!(error.1.is_none());
    let first: Value = serde_json::from_str(a[0]["text"].as_str().unwrap()).unwrap();
    for method in [
        AuthMethod::Session,
        AuthMethod::AccessToken,
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
    ] {
        let mut narrowed = auth.clone();
        narrowed.auth_method = method;
        let (content, digest) =
            retained_results::project(&f.state, &narrowed, large.clone(), false).await;
        assert_eq!(content, large);
        assert!(digest.is_none());
        let arguments = json!({"ref":first["ref"],"part":1});
        let refused =
            box_tool_future(|| retained_results::read(&f.state, &narrowed, &arguments, None)).await;
        let body = axum::body::to_bytes(refused.into_body(), 20_000)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["result"]["isError"], true);
    }
    let mut wrong_key = auth.clone();
    wrong_key.api_key_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(retained_results::chat(&wrong_key).is_none());
    let left = json!({"name":retained::TOOL_NAME,"arguments":{"ref":"one","part":2}});
    let right = json!({"name":"nyx__call_tool","arguments":{"tool_name":retained::TOOL_NAME,"arguments_json":"{\"ref\":\"two\",\"part\":2}"}});
    assert_eq!(
        retained_results::progress_params(&left),
        retained_results::progress_params(&right)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_unknown_account_tool_refuses_continuation_before_any_card() {
    let f = boxed_chat_fixture("large_result_old_replica").await;
    let before = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    // The old-replica fallback is deliberately still absent from account tools.
    // This exercises its unchanged validation/dispatch, bypassing the new native seam.
    let result = f
        .tools()
        .execute(
            &f.auth,
            retained::TOOL_NAME,
            &json!({"ref":uuid::Uuid::new_v4().to_string(),"part":1}),
        )
        .await;
    assert!(result.is_error);
    // The existing public error adapter deliberately hides internal NotFound
    // detail. Assert its stable public refusal, not the private error string.
    assert_eq!(result.value["error"], "not_found");
    assert_eq!(result.value["message"], "Resource not found.");
    assert!(result.permission_request.is_none());
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn oversized_unretainable_result_reports_completed_effect_without_replay_advice() {
    let f = boxed_chat_fixture("large_result_limit_notice").await;
    let auth = authenticate(&f).await;
    let original = vec![json!({"type":"text","text":"x".repeat(retained::MAX_BYTES+1)})];
    let expected = result_digest(&json!({"content":original,"isError":false})).0;
    let (content, digest) = retained_results::project(&f.state, &auth, original, false).await;
    let value: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(value["operation_completed"], true);
    assert_eq!(value["result_unavailable"], true);
    assert!(value["message"].as_str().unwrap().contains("Do not repeat"));
    assert!(value.get("ref").is_none());
    assert_eq!(digest.unwrap().0, expected);
    assert!(json!({"content":content,"isError":false}).to_string().len() < retained::BUDGET);
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn list_schedules_reads_all_sixty_through_direct_and_wrapped_mcp() {
    use crate::models::trigger::COLLECTION_NAME;

    let f = Box::pin(orchestrator_fixture("list_sixty_schedules")).await;
    let auth = authenticate(&f).await;
    let now = bson::DateTime::now();
    let instruction = "Review 值 \"quoted\"\n".repeat(210);
    let mut expected = Vec::new();
    let mut rows = Vec::new();
    for i in 0..62 {
        let at = bson::DateTime::from_millis(now.timestamp_millis() - i % 2);
        let id = uuid::Uuid::new_v4().to_string();
        let mut row = doc! {
            "_id": &id, "user_id": &f.owner, "label": format!("task {i}"),
            "source": "schedule", "schedule": {"kind": "at", "at": now},
            "status": if i == 0 { "active" } else { "disabled" },
            "secret_hash": format!("schedule:{id}"), "verification": {"mode":"schedule"},
            "delivery": {"type":"assistant", "agent_id": &f.chat.agent_id, "instruction": &instruction},
            "created_at": at, "updated_at": now,
        };
        if i == 1 || i == 60 {
            row.remove("source"); // Legacy webhook source.
            row.remove("schedule");
            row.insert("verification", doc! {"mode":"token","location":"bearer"});
        }
        match i {
            60 => {
                row.insert("delivery", doc! {"type": "notification"});
            }
            61 => {
                row.insert("user_id", uuid::Uuid::new_v4().to_string());
            }
            _ => expected.push((std::cmp::Reverse(at), id)),
        }
        rows.push(row);
    }
    f.state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .insert_many(rows)
        .await
        .unwrap();
    expected.sort();
    let expected: Vec<_> = expected.into_iter().map(|(_, id)| id).collect();

    let mut args = json!({});
    let mut actual = Vec::new();
    let mut pages = 0;
    loop {
        let name = "nyxid__list_schedules";
        let reply = if pages == 0 {
            direct_call(&f, &auth, name, args.clone()).await
        } else {
            call(&f, &auth, name, args.clone()).await
        };
        let mut value = page(reply).await;
        if let Some(reference) = value["ref"].as_str().map(str::to_owned) {
            let mut text = value["content"].as_str().unwrap().to_string();
            while let Some(part) = value["next_part"].as_u64() {
                value = page(
                    call(
                        &f,
                        &auth,
                        retained::TOOL_NAME,
                        json!({"ref":reference,"part":part}),
                    )
                    .await,
                )
                .await;
                text.push_str(value["content"].as_str().unwrap());
            }
            assert_eq!(value["complete"], true);
            value = serde_json::from_str(&text).unwrap();
        }
        let items = value["schedules"].as_array().unwrap();
        assert!(!items.is_empty());
        for item in items {
            actual.push(item["id"].as_str().unwrap().to_owned());
            assert_eq!(item["delivery"]["instruction"], instruction);
        }
        pages += 1;
        assert!(actual.len() <= expected.len());
        if value["next_cursor"].is_null() {
            break;
        }
        args["cursor"] = value["next_cursor"].clone();
    }
    assert!(pages > 1);
    assert_eq!(actual, expected);

    let active = page(
        direct_call(
            &f,
            &auth,
            "nyxid__list_schedules",
            json!({"enabled":true,"limit":1}),
        )
        .await,
    )
    .await;
    let active = active["schedules"].as_array().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["label"], "task 0");
    let empty = page(
        call(
            &f,
            &auth,
            "nyxid__list_schedules",
            json!({"agent_id":uuid::Uuid::new_v4()}),
        )
        .await,
    )
    .await;
    assert_eq!(empty["schedules"], json!([]));
    assert!(empty["next_cursor"].is_null());
    f.state.db.drop().await.unwrap();
}
