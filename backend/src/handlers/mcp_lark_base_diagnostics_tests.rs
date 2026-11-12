//! Real catalog URL conventions, previous-message PDFs, and external diagnostics.
use super::*;
use crate::services::assistant_nyxagent as engine;
use futures::TryStreamExt;

fn pdf() -> Vec<u8> {
    use lopdf::{Document, Object, dictionary};
    let mut pdf = Document::with_version("1.5");
    let pages = pdf.new_object_id();
    let page = pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]});
    pdf.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}),
    );
    let catalog = pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}
async fn named_call(
    u: &BaseFixture,
    auth: &McpAuthContext,
    meta: bool,
    operation: &str,
    args: Value,
) -> Value {
    let slug = if u.catalog.slug.ends_with("-bot") {
        format!("{}-3", u.catalog.slug)
    } else {
        "receipts".into()
    };
    let name = format!("{slug}__{operation}");
    let params = if meta {
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}})
    } else {
        json!({"name":name,"arguments":args})
    };
    rpc(&u.f, auth, "tools/call", Some(params)).await
}
async fn audits(u: &BaseFixture, event: &str, count: usize) -> Vec<Value> {
    // log_async appends through the audit-chain writer. Wait only for our DB.
    for _ in 0..100 {
        let rows: Vec<crate::models::audit_log::AuditLog> =
            u.f.state
                .db
                .collection(crate::models::audit_log::COLLECTION_NAME)
                .find(doc! {"event_type":event})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        if rows.len() >= count {
            return rows
                .into_iter()
                .map(|row| row.event_data.unwrap())
                .collect();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("missing {event} result audit");
}
async fn last_failure(u: &BaseFixture) -> crate::models::assistant_conversation::ToolFailure {
    let row = engine::get(&u.f.state.db, &u.f.owner, &u.f.row.id)
        .await
        .unwrap();
    row.active_turn
        .unwrap()
        .activities
        .last()
        .unwrap()
        .failure
        .clone()
        .unwrap()
}
async fn assert_reason(u: &BaseFixture, code: &str, service_event: bool, outcome: &str) {
    assert_eq!(last_failure(u).await.reason.as_deref(), Some(code));
    let tool = audits(u, "mcp_tool_call", 1).await;
    assert_eq!(tool.len(), 1, "one composite result, never a child result");
    assert_eq!(tool[0]["reason_code"], code);
    assert_eq!(tool[0]["service_id"], u.instance);
    assert_eq!(tool[0]["outcome"], outcome);
    if service_event {
        let events = audits(u, "service_request", 1).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["reason_code"], code);
        assert_eq!(events[0]["outcome"], outcome);
        assert_eq!(events[0]["provider_code"], tool[0]["provider_code"]);
        assert_eq!(events[0]["response_status"], tool[0]["response_status"]);
    }
    for value in &tool {
        let text = value.to_string();
        // Fixture markers only. The index, not the marker, goes in the message.
        for (index, marker) in [
            "Receipt",
            "Card Number",
            "secret-do-not-audit",
            "provider-private-prose",
            "private-receipt.pdf",
            "tenant-test-token",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(!text.contains(marker), "diagnostic leaked marker #{index}");
        }
    }
}

#[tokio::test]
async fn lark_base_writes_four_catalogs_prior_message_pdf_uses_trusted_routes_and_real_tenant_exchange()
 {
    for slug in ["api-lark", "api-feishu", "api-lark-bot", "api-feishu-bot"] {
        for meta in [false, true] {
            let u = setup_catalog("base_prior_pdf", slug).await;
            let bytes = pdf();
            let id = bind_named_upload(&u.f, "private-receipt.pdf", &bytes).await;
            u.f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .update_one(doc! {"_id":&id}, doc! {"$set":{"source":{"event_key":"previous-message","declared":1_i64,"index":0_i64,"embedded":false,"turn_id":u.f.chat.turn_id.as_deref()}}}).await.unwrap();
            let old_turn = u.f.chat.turn_id.clone();
            engine::finish_turn(
                &u.f.state.db,
                &u.f.row,
                &u.f.chat.api_key_id,
                &Uuid::new_v4().to_string(),
                &engine::TurnResult {
                    text: "Read the PDF".into(),
                    session_id: None,
                    response_id: None,
                    error: None,
                },
            )
            .await
            .unwrap();
            engine::begin_turn(
                &u.f.state.db,
                &u.f.owner,
                &engine::TurnRequest {
                    acknowledgement_id: None,
                    retry_of: None,
                    attachment_ids: vec![],
                    agent_id: None,
                    conversation_id: Some(u.f.row.id.clone()),
                    text: "Save the previous receipt".into(),
                    model: None,
                    access_mode: None,
                },
                &u.f.state.encryption_keys,
            )
            .await
            .unwrap();
            let auth = authenticate(&u.f).await;
            assert_ne!(auth.chat.as_ref().unwrap().turn_id, old_turn);
            let response = named_call(&u, &auth, meta, attachment::COMPOSITE_OPERATION,
                json!({"app_token":"base","table_id":"table","fields":{},"attachments":{"Receipt":[id]}})).await;
            assert_eq!(
                payload(&response)["record_id"],
                "rec_receipt",
                "{slug}: {response}"
            );
            let sent = requests(&u).await;
            let api: Vec<_> = sent
                .iter()
                .filter(|r| !r.url.path().ends_with("/internal"))
                .collect();
            assert_eq!(api.len(), 3);
            for request in &api {
                assert!(request.url.path().starts_with("/open-apis/"));
            }
            let upload = &api[1].body;
            assert!(upload.windows(bytes.len()).any(|window| window == bytes));
            let body = String::from_utf8_lossy(upload);
            assert!(body.contains("bitable_file"));
            assert!(body.contains("application/pdf"));
            assert!(body.contains(&format!("\r\n\r\n{}\r\n", bytes.len())));
            assert_eq!(ledger(&u).await.files[0].content_sha256, {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(&bytes))
            });
            if slug.ends_with("-bot") {
                assert_eq!(sent.len(), 4);
                assert_eq!(
                    serde_json::from_slice::<Value>(&sent[0].body).unwrap(),
                    json!({"app_id":"test-app","app_secret":"secret-do-not-audit"})
                );
                for request in api {
                    assert_eq!(request.headers["authorization"], "Bearer tenant-test-token");
                }
            }
            let events = audits(&u, "mcp_tool_call", 1).await;
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["outcome"], "succeeded");
            assert_eq!(
                audits(&u, "service_request", 1).await[0]["outcome"],
                "succeeded"
            );
            cleanup(u).await;
        }
    }
}

#[tokio::test]
async fn lark_base_writes_field_failures_reach_activity_model_and_both_audits() {
    for (case, expected, outcome, response) in [
        ("transport", "field_read_transport", "not_sent", ResponseTemplate::new(200).set_body_bytes(vec![b'x';1024*1024+1])),
        ("provider", "field_read_provider_error", "provider_refused", ResponseTemplate::new(403).set_body_json(json!({"code":1254302,"msg":"provider-private-prose"}))),
        ("missing", "field_missing", "not_sent", ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[],"has_more":false}}))),
        ("wrong_type", "field_not_attachment", "not_sent", ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[{"field_name":"Receipt","type":1}],"has_more":false}}))),
        ("html", "field_read_invalid_response", "not_sent", ResponseTemplate::new(200).set_body_string("<html>provider-private-prose</html>")),
        ("no_code", "field_read_invalid_response", "not_sent", ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}}))),
        ("invalid", "field_read_invalid_response", "not_sent", ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{}}))),
    ] {
        for meta in [false,true] {
            let u = setup_catalog("base_field_diagnostic", "api-lark-bot").await;
            Mock::given(path("/open-apis/bitable/v1/apps/base/tables/table/fields"))
                .respond_with(response.clone()).with_priority(1).mount(&u.server).await;
            let auth = authenticate(&u.f).await;
            let result = named_call(&u,&auth,meta,attachment::COMPOSITE_OPERATION,
                json!({"app_token":"base","table_id":"table","fields":{},"attachments":{"Receipt":[u.attachment_id]}})).await;
            assert_error(&result);
            assert!(!text(&result).contains("provider-private-prose"));
            assert_reason(&u,expected,true,outcome).await;
            if case == "provider" { assert_eq!(audits(&u,"mcp_tool_call",1).await[0]["provider_code"],1254302); }
            assert_no_create(&u).await;
            cleanup(u).await;
        }
    }
}

#[tokio::test]
async fn lark_base_writes_tenant_exchange_refusal_is_not_sent_and_diagnosed() {
    let u = setup_catalog("base_exchange_diagnostic", "api-lark-bot").await;
    Mock::given(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code":99991663,"msg":"provider-private-prose"})),
        )
        .with_priority(1)
        .mount(&u.server)
        .await;
    let auth = authenticate(&u.f).await;
    let result = named_call(
        &u,
        &auth,
        false,
        attachment::COMPOSITE_OPERATION,
        args(&u.attachment_id),
    )
    .await;
    assert_error(&result);
    assert_reason(&u, "credential_preparation", true, "not_sent").await;
    assert_eq!(requests(&u).await.len(), 1);
    assert_no_create(&u).await;
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_writes_eligibility_and_source_refusals_are_audited_before_execution() {
    for (variant, reason) in [
        ("flag", "eligibility:flag_off"),
        ("missing", "attachment_unreadable"),
        ("incomplete", "attachment_incomplete_set"),
    ] {
        let u = setup("base_local_diagnostic").await;
        let auth = authenticate(&u.f).await;
        let mut input = args(&u.attachment_id);
        match variant {
            "flag" => writes(&u.f, false).await,
            "missing" => {
                input["attachments"]["Receipt"] = json!([Uuid::new_v4().to_string()]);
            }
            _ => {
                u.f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .update_one(doc! {"_id":&u.attachment_id},doc! {"$set":{"source":{"event_key":"incomplete","declared":2_i64,"index":0_i64,"embedded":false,"turn_id":u.f.chat.turn_id.as_deref()}}}).await.unwrap();
            }
        }
        assert_error(&call(&u, &auth, false, input).await);
        assert_reason(&u, reason, false, "not_sent").await;
        assert!(requests(&u).await.is_empty());
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_attachment_standalone_bot_upload_success_and_refusal_have_result_diagnostics() {
    for slug in ["api-lark", "api-feishu", "api-lark-bot", "api-feishu-bot"] {
        for fail in [false, true] {
            let u = setup_catalog("base_media_diagnostic", slug).await;
            if fail {
                Mock::given(path("/open-apis/drive/v1/medias/upload_all"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(json!({"code":1061002,"msg":"provider-private-prose"})),
                    )
                    .with_priority(1)
                    .mount(&u.server)
                    .await;
            }
            let auth = authenticate(&u.f).await;
            let result = named_call(&u,&auth,true,attachment::UPLOAD_OPERATION,
                json!({"attachment_id":u.attachment_id,"parent_type":"bitable_file","parent_node":"base"})).await;
            if fail {
                assert_error(&result);
                assert!(text(&result).contains("1061002"));
                assert_reason(&u, "upload_provider_error", true, "provider_refused").await;
            } else {
                assert_ne!(result["result"]["isError"], true, "{result}");
                assert_eq!(
                    audits(&u, "mcp_tool_call", 1).await[0]["outcome"],
                    "succeeded"
                );
                assert_eq!(
                    audits(&u, "service_request", 1).await[0]["outcome"],
                    "succeeded"
                );
            }
            cleanup(u).await;
        }
    }
}

#[tokio::test]
async fn lark_base_writes_uncertain_create_reports_both_results_and_never_resends() {
    let u = setup_catalog("base_uncertain_diagnostic", "api-feishu-bot").await;
    Mock::given(path("/open-apis/bitable/v1/apps/base/tables/table/records"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({"code":1255006,"msg":"provider-private-prose"})),
        )
        .with_priority(1)
        .mount(&u.server)
        .await;
    let auth = authenticate(&u.f).await;
    assert_error(
        &named_call(
            &u,
            &auth,
            false,
            attachment::COMPOSITE_OPERATION,
            args(&u.attachment_id),
        )
        .await,
    );
    assert_reason(&u, "create_uncertain", true, "uncertain").await;
    assert_eq!(
        ledger(&u).await.state,
        crate::models::lark_base_write::WriteState::Uncertain
    );
    assert_error(
        &named_call(
            &u,
            &auth,
            true,
            attachment::COMPOSITE_OPERATION,
            args(&u.attachment_id),
        )
        .await,
    );
    assert_eq!(
        last_failure(&u).await.reason.as_deref(),
        Some("ledger:uncertain")
    );
    assert_eq!(audits(&u, "mcp_tool_call", 2).await.len(), 2);
    assert_eq!(
        requests(&u)
            .await
            .iter()
            .filter(|r| r.url.path().ends_with("/records"))
            .count(),
        1
    );
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_writes_successful_contract_probes_do_not_leave_a_false_refusal_diagnostic() {
    let u = setup_catalog("base_probe_diagnostic", "api-lark-bot").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == attachment::COMPOSITE_OPERATION)
        .unwrap();
    let diagnostic = attachment::diagnostics::Handle::default();
    attachment::diagnostics::scope(
        diagnostic.clone(),
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            endpoint,
            &args(&u.attachment_id),
        ),
    )
    .await
    .unwrap();
    assert!(diagnostic.lock().unwrap().reason.is_none());
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_writes_typed_fields_rejects_success_status_without_api_success() {
    for slug in ["api-lark", "api-feishu", "api-lark-bot", "api-feishu-bot"] {
        for (reason, response) in [
            (
                "field_read_invalid_response",
                ResponseTemplate::new(200).set_body_string("<html>provider-private-prose</html>"),
            ),
            (
                "field_read_invalid_response",
                ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})),
            ),
            (
                "field_read_provider_error",
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":1254302,"msg":"provider-private-prose"})),
            ),
        ] {
            let u = setup_catalog("typed_lark_fields_response", slug).await;
            Mock::given(path("/open-apis/bitable/v1/apps/base/tables/table/fields"))
                .respond_with(response)
                .with_priority(1)
                .mount(&u.server)
                .await;
            let auth = authenticate(&u.f).await;
            let result = named_call(
                &u,
                &auth,
                false,
                attachment::FIELDS_OPERATION,
                json!({"app_token":"base","table_id":"table"}),
            )
            .await;
            assert_error(&result);
            assert!(!text(&result).contains("provider-private-prose"));
            assert_reason(&u, reason, true, "provider_refused").await;
            assert_eq!(
                audits(&u, "mcp_tool_call", 1).await[0]["response_status"],
                200
            );
            cleanup(u).await;
        }
    }
}

#[tokio::test]
async fn lark_base_writes_bot_existing_recipe_scope_survives_sync_without_owner_repair() {
    for slug in ["api-lark-bot", "api-feishu-bot"] {
        let u = setup_catalog("bot_recipe_contract_stability", slug).await;
        receipt_scope(&u).await;
        let before =
            u.f.state
                .db
                .collection::<bson::Document>("api_keys")
                .find_one(doc! {"_id":&u.f.chat.api_key_id})
                .await
                .unwrap()
                .unwrap();
        let scopes: crate::models::agent_operation_scope::OperationScopes =
            bson::from_bson(before.get("assistant_operation_scopes").unwrap().clone()).unwrap();
        let digest = crate::services::agent_operation_scope_service::contract_digest(
            scopes.get(&u.instance),
        );
        // Complete the fixture's four-operation subset before comparing the
        // whole catalog: adding unrelated absent operations is normal sync work.
        crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&u.f.state.db)
            .await
            .unwrap();
        let before_catalog = crate::services::mcp_service::load_operation_catalog(
            &u.f.state.db,
            u.f.state.node_ws_manager.as_ref(),
            &u.f.owner,
            crate::services::mcp_service::NodeScope::Unrestricted,
            crate::services::mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .unwrap();
        for _ in 0..2 {
            crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&u.f.state.db)
                .await
                .unwrap();
        }
        let after_catalog = crate::services::mcp_service::load_operation_catalog(
            &u.f.state.db,
            u.f.state.node_ws_manager.as_ref(),
            &u.f.owner,
            crate::services::mcp_service::NodeScope::Unrestricted,
            crate::services::mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .unwrap();
        assert_eq!(
            crate::services::mcp_service::operation_catalog_digest(&before_catalog.services),
            crate::services::mcp_service::operation_catalog_digest(&after_catalog.services)
        );
        assert_eq!(
            crate::services::mcp_service::exact_operation_view_digest(
                &crate::services::mcp_service::exact_operation_view(&before_catalog.services)
            ),
            crate::services::mcp_service::exact_operation_view_digest(
                &crate::services::mcp_service::exact_operation_view(&after_catalog.services)
            )
        );
        for old_service in &before_catalog.services {
            let new_service = after_catalog
                .services
                .iter()
                .find(|s| s.service_id == old_service.service_id)
                .unwrap();
            for old in &old_service.endpoints {
                let new = new_service
                    .endpoints
                    .iter()
                    .find(|ep| ep.endpoint_id == old.endpoint_id)
                    .unwrap();
                assert_eq!(
                    crate::services::mcp_service::endpoint_contract_digest(old),
                    crate::services::mcp_service::endpoint_contract_digest(new)
                );
                assert_eq!(
                    old_service.durable_endpoint_metadata[&old.endpoint_id].operation_generation,
                    new_service.durable_endpoint_metadata[&new.endpoint_id].operation_generation
                );
            }
        }
        let after =
            u.f.state
                .db
                .collection::<bson::Document>("api_keys")
                .find_one(doc! {"_id":&u.f.chat.api_key_id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            before.get("assistant_operation_scopes"),
            after.get("assistant_operation_scopes")
        );
        assert_eq!(
            digest,
            crate::services::agent_operation_scope_service::contract_digest(
                scopes.get(&u.instance)
            )
        );
        let auth = authenticate(&u.f).await;
        let result = named_call(
            &u,
            &auth,
            false,
            attachment::COMPOSITE_OPERATION,
            args(&u.attachment_id),
        )
        .await;
        assert_eq!(
            payload(&result)["record_id"],
            "rec_receipt",
            "{slug}: {result}"
        );
        assert_eq!(
            requests(&u)
                .await
                .iter()
                .filter(|r| r.url.path().ends_with("/records"))
                .count(),
            1
        );
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_writes_existing_field_scope_keeps_original_path_and_limits() {
    for slug in ["api-lark-bot", "api-feishu-bot"] {
        let u = setup_catalog("bot_existing_field_scope", slug).await;
        let fields =
            u.f.state
                .db
                .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
                .find_one(doc! {"service_id":&u.catalog.id,"name":attachment::FIELDS_OPERATION})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            fields.path,
            "/bitable/v1/apps/{app_token}/tables/{table_id}/fields"
        );
        let scope: crate::models::agent_operation_scope::AgentOperationScope = serde_json::from_value(json!({
            "revision":7,"catalog_service_id":u.catalog.id,"operations":[{
                "endpoint_id":fields.id,"rule":{"method":"GET","path_template":fields.path},
                "inputs":{"path":{"app_token":{"type":"exact","value":"base"},"table_id":{"type":"exact","value":"table"}}}
            }]
        })).unwrap();
        let scopes = crate::models::agent_operation_scope::OperationScopes::from([(
            u.instance.clone(),
            scope,
        )]);
        u.f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&u.f.chat.api_key_id},
                doc! {"$set":{"assistant_operation_scopes":bson::to_bson(&scopes).unwrap()}},
            )
            .await
            .unwrap();
        let auth = authenticate(&u.f).await;
        let denied = named_call(
            &u,
            &auth,
            false,
            attachment::FIELDS_OPERATION,
            json!({"app_token":"base","table_id":"other"}),
        )
        .await;
        assert_error(&denied);
        assert!(requests(&u).await.is_empty());
        let allowed = named_call(
            &u,
            &auth,
            false,
            attachment::FIELDS_OPERATION,
            json!({"app_token":"base","table_id":"table"}),
        )
        .await;
        assert_eq!(payload(&allowed)["code"], 0, "{allowed}");
        assert_eq!(
            requests(&u)
                .await
                .iter()
                .filter(|r| r.url.path() == "/open-apis/bitable/v1/apps/base/tables/table/fields")
                .count(),
            1
        );
        cleanup(u).await;
    }
}
