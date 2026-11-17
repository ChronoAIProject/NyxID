//! The real direct and universal chat dispatch paths, including binary corruption.
use super::*;
use crate::services::{assistant_nyxagent as engine, assistant_tool_files as files};

async fn setup_raw(name: &'static str) -> UploadFixture {
    let mut u = setup(name).await;
    u.catalog.slug = "manager-files".into();
    u.catalog.created_by = "manager".into();
    u.catalog.catalog_manager_created = true;
    u.catalog.default_request_headers = Some(vec![
        crate::models::default_request_header::DefaultRequestHeader {
            name: "content-type".into(),
            value: "application/json".to_string().into(),
            overridable: false,
            sensitive: false,
        }
        .into(),
    ]);
    u.f.state
        .db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&u.catalog.id}, &u.catalog)
        .await
        .unwrap();
    u.endpoint.name = "upload_file".into();
    u.endpoint.path = "/raw".into();
    u.endpoint.request_content_type = Some("application/octet-stream".into());
    u.endpoint.request_body_schema = Some(json!({"type":"string","format":"binary"}));
    u.endpoint.parameters =
        Some(json!([{"name":"fileName","in":"query","required":true,"schema":{"type":"string"}}]));
    u.f.state
        .db
        .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .replace_one(doc! {"_id":&u.endpoint.id}, &u.endpoint)
        .await
        .unwrap();
    let mut download = u.endpoint.clone();
    download.id = Uuid::new_v4().to_string();
    download.name = "download_file".into();
    download.method = "GET".into();
    download.path = "/download".into();
    download.parameters = None;
    download.request_content_type = None;
    download.request_body_schema = None;
    download.request_body_required = false;
    u.f.state
        .db
        .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .insert_one(download)
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/open-apis/raw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"uploaded":true})))
        .mount(&u.server)
        .await;
    u
}

async fn raw_call(
    u: &UploadFixture,
    auth: &McpAuthContext,
    meta: bool,
    operation: &str,
    args: Value,
) -> Value {
    let name = format!("receipts__{operation}");
    let params = if meta {
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}})
    } else {
        json!({"name":name,"arguments":args})
    };
    rpc(&u.f, auth, "tools/call", Some(params)).await
}
// Compare the legacy decoded text through the current transport projection.
// Text beyond main's retention cap has the same explicit unavailable result.
async fn assert_legacy_chat_projection(
    u: &UploadFixture,
    auth: &McpAuthContext,
    meta: bool,
    result: &Value,
    expected: &str,
) {
    use crate::services::assistant_tool_result_service as retained;
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert!(result["result"].to_string().len() <= retained::BUDGET);
    if retained::envelope(expected).to_string().len() <= retained::BUDGET {
        assert_eq!(text(result), expected);
        return;
    }
    let first: Value = serde_json::from_str(text(result)).unwrap();
    if expected.len() > retained::MAX_BYTES {
        assert_eq!(first, retained::unavailable());
        return;
    }
    assert_eq!(first["total_bytes"], expected.len());
    let mut assembled = first["content"].as_str().unwrap().to_owned();
    for part in 1..first["parts"].as_u64().unwrap() {
        let args = json!({"ref":first["ref"],"part":part});
        let params = if meta {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":retained::TOOL_NAME,"arguments_json":args.to_string()}})
        } else {
            json!({"name":retained::TOOL_NAME,"arguments":args})
        };
        let result = rpc(&u.f, auth, "tools/call", Some(params)).await;
        assert_eq!(result["result"]["isError"], false, "{result}");
        let page: Value = serde_json::from_str(text(&result)).unwrap();
        assembled.push_str(page["content"].as_str().unwrap());
    }
    assert_eq!(assembled, expected);
}

fn raw_args(id: &str) -> Value {
    json!({"attachment_id":id,"fileName":"report.pdf"})
}
async fn mode(u: &UploadFixture, mode: crate::models::assistant_settings::AccessMode) {
    crate::services::assistant_settings_service::update(
        &u.f.state.db,
        &u.f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(mode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}
fn pdf() -> Vec<u8> {
    use lopdf::{
        Document, Object, Stream,
        content::{Content, Operation},
        dictionary,
    };
    let mut doc = Document::with_version("1.5");
    let pages = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![20.into(), 20.into()]),
            Operation::new("Tj", vec![Object::string_literal("Round trip document")]),
            Operation::new("ET", vec![]),
        ],
    };
    let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),200.into(),200.into()],"Contents"=>stream,"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}});
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(std::str::from_utf8(&bytes).is_err());
    bytes
}

#[tokio::test]
async fn raw_attachment_pdf_download_read_upload_round_trips_exact_bytes_and_full_receipt() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_pdf_roundtrip").await;
    mode(&u, crate::models::assistant_settings::AccessMode::Autopilot).await;
    let bytes = pdf();
    Mock::given(method("GET"))
        .and(path("/open-apis/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .insert_header(
                    "content-disposition",
                    "attachment; filename*=UTF-8''%E6%96%87%E4%BB%B6.pdf",
                )
                .set_body_bytes(bytes.clone()),
        )
        .mount(&u.server)
        .await;
    let auth = authenticate(&u.f).await;
    for meta in [false, true] {
        writes(&u.f, false).await;
        let downloaded = raw_call(&u, &auth, meta, "download_file", json!({})).await;
        assert_eq!(downloaded["result"]["isError"], false, "{downloaded}");
        let metadata: Value =
            serde_json::from_str(text(&downloaded).lines().next().unwrap()).unwrap();
        assert_eq!(metadata["name"], "文件.pdf");
        assert_eq!(metadata["type"], "application/pdf");
        assert!(!text(&downloaded).contains("%PDF"));
        let id = metadata["attachment_id"].as_str().unwrap();
        let read = uploads::read(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref().unwrap(),
            &json!({"attachment_id":id}),
        )
        .await
        .unwrap();
        assert!(
            read["text"]
                .as_str()
                .unwrap()
                .contains("Round trip document"),
            "{read}"
        );
        let result = raw_call(&u, &auth, meta, "upload_file", raw_args(id)).await;
        assert_eq!(result["result"]["isError"], false, "{result}");
        let image = engine::get(&u.f.state.db, &u.f.owner, &u.f.row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .attachments
            .into_iter()
            .find(|a| a.id == id)
            .unwrap();
        assert_eq!(image.origin, "tool");
        assert_eq!(image.chat_delivery, Some(false));
        assert!(!engine::chat_tool_image(&image));
        assert!(
            uploads::chat_bytes(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                auth.chat.as_ref().unwrap(),
                id,
                true
            )
            .await
            .is_err()
        );
    }
    let sent = u.server.received_requests().await.unwrap();
    for request in sent.iter().filter(|r| r.method.as_str() == "POST") {
        assert_eq!(request.body, bytes);
        assert_eq!(request.headers["content-type"], "application/octet-stream");
        assert_eq!(
            request
                .url
                .query_pairs()
                .find(|(k, _)| k == "fileName")
                .unwrap()
                .1,
            "report.pdf"
        );
    }
    let receipts = acks::history(&u.f.state.db, &u.f.owner, &u.f.row.id)
        .await
        .unwrap();
    assert!(
        receipts
            .iter()
            .any(|r| r.decided_by.as_deref() == Some("access_mode")
                && r.summary.contains("文件.pdf"))
    );
    engine::delete(&u.f.state.db, &u.f.owner, &u.f.row.id)
        .await
        .unwrap();
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&u.f.owner})
            .await
            .unwrap(),
        0
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_auto_and_ask_cards_bind_bytes_attachment_and_parameters() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use crate::models::assistant_settings::AccessMode;
    let u = setup_raw("raw_cards").await;
    let auth = authenticate(&u.f).await;
    let other = bind_upload(&u.f, b"receipt bytes").await;
    for mode_value in [AccessMode::Nyxmode, AccessMode::AskEveryTime] {
        mode(&u, mode_value).await;
        for meta in [false, true] {
            let response =
                raw_call(&u, &auth, meta, "upload_file", raw_args(&u.attachment_id)).await;
            assert_error(&response);
            let card = payload(&response);
            assert_eq!(card["error"], "acknowledgement_required", "{card}");
            let id = card["acknowledgement_id"].as_str().unwrap();
            let history = acks::history(&u.f.state.db, &u.f.owner, &u.f.row.id)
                .await
                .unwrap();
            let summary = &history.iter().find(|c| c.id == id).unwrap().summary;
            for expected in ["receipt.txt", "13", "receipts", "upload_file", "report.pdf"] {
                assert!(summary.contains(expected), "{summary}");
            }
            acks::decide(&u.f.state.db, &u.f.owner, &u.f.row.id, id, true)
                .await
                .unwrap();
            let mut confirmed = raw_args(&u.attachment_id);
            confirmed["acknowledgement_id"] = json!(id);
            for changed in [
                json!({"attachment_id":other,"fileName":"report.pdf","acknowledgement_id":id}),
                json!({"attachment_id":u.attachment_id,"fileName":"other.pdf","acknowledgement_id":id}),
            ] {
                let result = raw_call(&u, &auth, meta, "upload_file", changed).await;
                assert_error(&result);
                assert_eq!(payload(&result)["error"], "acknowledgement_invalid");
            }
            replace_upload_bytes(&u, b"changed bytes").await;
            let result = raw_call(&u, &auth, meta, "upload_file", confirmed.clone()).await;
            assert_error(&result);
            assert_eq!(payload(&result)["error"], "acknowledgement_invalid");
            replace_upload_bytes(&u, b"receipt bytes").await;
            let sent = raw_call(&u, &auth, meta, "upload_file", confirmed).await;
            assert_eq!(sent["result"]["isError"], false, "{sent}");
        }
    }
    assert_eq!(u.server.received_requests().await.unwrap().len(), 4);
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_schema_is_always_on_preserves_config_contract_and_denies_non_chat() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_discovery").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "upload_file")
        .unwrap();
    let digest = mcp_service::endpoint_contract_digest(endpoint);
    for enabled in [false, true] {
        writes(&u.f, enabled).await;
        let list = rpc(&u.f, &auth, "tools/list", None).await;
        let tool = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "receipts__upload_file")
            .unwrap();
        assert!(
            tool["inputSchema"]["properties"]["attachment_id"].is_object(),
            "{tool}"
        );
        assert!(tool["inputSchema"]["properties"]["body"].is_object());
        assert_eq!(
            tool["inputSchema"]["oneOf"],
            json!([{"required":["body"]},{"required":["attachment_id"]}])
        );
        let config =
            crate::handlers::mcp::get_mcp_config(State(u.f.state.clone()), u.f.auth.clone())
                .await
                .unwrap()
                .0;
        let configured = config
            .services
            .iter()
            .find(|s| s.service_id == u.instance)
            .unwrap();
        assert!(
            configured
                .endpoints
                .iter()
                .any(|ep| ep.name == "upload_file")
        );
        assert_eq!(mcp_service::endpoint_contract_digest(endpoint), digest);
        assert!(mcp_service::build_input_schema(endpoint)["properties"]["body"].is_object());
        let prepared = mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            endpoint,
            &raw_args(&u.attachment_id),
        )
        .await
        .unwrap();
        assert!(prepared.attachment_policy_floor());
    }
    for chat in [None, {
        let mut guest = auth.chat.clone().unwrap();
        guest.guest = true;
        Some(guest)
    }] {
        assert!(
            mcp_service::prepare_chat_proxy_tool_call(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                chat.as_ref(),
                &service,
                endpoint,
                &raw_args(&u.attachment_id)
            )
            .await
            .is_err()
        );
        let schemas = crate::services::raw_attachment_tool::discovery_schemas(
            &u.f.state.db,
            chat.as_ref(),
            std::slice::from_ref(&service),
        )
        .await
        .unwrap();
        assert!(schemas.is_empty());
    }
    // Relay tokens can carry the same conversation lineage. They still do
    // not acquire the direct chat key's file capability.
    for method in [
        AuthMethod::Relay,
        AuthMethod::Delegated,
        AuthMethod::ServiceAccount,
        AuthMethod::AccessToken,
    ] {
        let mut caller = auth.clone();
        caller.is_api_key = false;
        caller.auth_method = method;
        let list = rpc(&u.f, &caller, "tools/list", None).await;
        let tool = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "receipts__upload_file")
            .unwrap();
        assert!(
            tool["inputSchema"]["properties"]
                .get("attachment_id")
                .is_none()
        );
        assert!(tool["inputSchema"]["properties"]["body"].is_object());
        for meta in [false, true] {
            assert_error(
                &raw_call(&u, &caller, meta, "upload_file", raw_args(&u.attachment_id)).await,
            );
        }
    }
    assert!(
        mcp_service::prepare_proxy_tool_call(&service, endpoint, &raw_args(&u.attachment_id))
            .is_err()
    );
    assert!(
        mcp_service::prepare_exact_proxy_tool_call(
            &service,
            endpoint,
            &raw_args(&u.attachment_id),
            Some("exact")
        )
        .is_err()
    );
    let mut generic = loaded(&u, &auth).await;
    generic.is_generic_proxy = true;
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &generic,
            endpoint,
            &raw_args(&u.attachment_id)
        )
        .await
        .is_err()
    );
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_chat_accepts_model_authored_body_and_handle_without_flag_and_refuses_both()
{
    let _budget_guard = uploads::byte_budget::test_guard().await;
    use base64::Engine;
    let u = setup_raw("raw_dual_input").await;
    mode(&u, crate::models::assistant_settings::AccessMode::Autopilot).await;
    writes(&u.f, false).await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "upload_file")
        .unwrap();
    let note = b"A note written by NyxBot.\n";
    let body = base64::engine::general_purpose::STANDARD.encode(note);
    let body_args = json!({"body":body,"fileName":"notes.txt"});
    let mut both = body_args.clone();
    both["attachment_id"] = json!(u.attachment_id);
    // Neither a body-only call nor the mutually-exclusive-input refusal needs
    // contract, attachment, or flag lookups during preparation.
    u.f.state.db.run_command(doc! {"profile":2}).await.unwrap();
    let legacy = mcp_service::prepare_proxy_tool_call(&service, endpoint, &body_args).unwrap();
    let prepared = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &body_args,
    )
    .await
    .unwrap();
    assert_eq!(
        prepared.operation_descriptor(),
        legacy.operation_descriptor()
    );
    assert!(!prepared.attachment_policy_floor());
    let refusal = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &both,
    )
    .await
    .err()
    .unwrap();
    assert!(refusal.to_string().contains("exactly one"));
    u.f.state.db.run_command(doc! {"profile":0}).await.unwrap();
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>("system.profile")
            .count_documents(doc! {"command.find":{"$exists":true}})
            .await
            .unwrap(),
        0
    );
    drop(prepared);
    for meta in [false, true] {
        let response = raw_call(&u, &auth, meta, "upload_file", both.clone()).await;
        assert_error(&response);
        assert!(response.to_string().contains("exactly one"), "{response}");
        let response = raw_call(&u, &auth, meta, "upload_file", body_args.clone()).await;
        assert_eq!(response["result"]["isError"], false, "{response}");
        let response = raw_call(&u, &auth, meta, "upload_file", raw_args(&u.attachment_id)).await;
        assert_eq!(response["result"]["isError"], false, "{response}");
    }
    let requests = u.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    for pair in requests.as_chunks::<2>().0 {
        assert_eq!(pair[0].body, note);
        assert_eq!(pair[1].body, b"receipt bytes");
    }
    // Main's separate live-turn RPC gate resolves all personal flags. Prove
    // generic preparation/egress ignores a malformed Lark flag at its own
    // boundary, without changing that existing gate or Lark's flag semantics.
    u.f.state.db.collection::<bson::Document>("feature_flag_overrides")
        .update_one(doc! {"flag_key":crate::services::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY},
            doc! {"$set":{"enabled":"invalid"}}).await.unwrap();
    let args = raw_args(&u.attachment_id);
    let validated = crate::services::raw_attachment_tool::uses_handle(
        &u.f.state.db,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args,
    )
    .await
    .unwrap()
    .unwrap();
    let materialized = crate::services::raw_attachment_tool::materialize(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        validated,
    )
    .await
    .unwrap();
    assert_eq!(materialized.body.as_ref(), b"receipt bytes");
    let guard = crate::services::assistant_service_execution::HttpEgressGuard::with_attachment(
        None,
        &u.f.state.db,
        &service,
        endpoint,
        Some(&materialized.binding),
    )
    .unwrap();
    guard.revalidate().await.unwrap();
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_specialist_without_service_grant_cannot_discover_file_handles_or_upload() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use crate::services::assistant_team_service as team;
    let u = setup_raw("raw_specialist_grants").await;
    let (_, home) = team::create_specialist(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        &u.f.owner,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "file-worker".into(),
            description: "Move files".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let row = engine::begin_turn(
        &u.f.state.db,
        &u.f.owner,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: Some(home.id),
            text: "Upload the report".into(),
            model: None,
            access_mode: None,
        },
        &u.f.state.encryption_keys,
    )
    .await
    .unwrap();
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        &u.f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", credential.raw_key.parse().unwrap());
    let auth = authenticate_mcp(&u.f.state, &headers, false).await.unwrap();
    let list = rpc(&u.f, &auth, "tools/list", None).await;
    let tool = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "receipts__upload_file")
        .unwrap();
    assert!(
        tool["inputSchema"]["properties"]
            .get("attachment_id")
            .is_none()
    );
    assert!(tool["inputSchema"]["properties"]["body"].is_object());
    for meta in [false, true] {
        assert_error(&raw_call(&u, &auth, meta, "upload_file", raw_args(&u.attachment_id)).await);
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_tool_file_limits_live_turn_fences_and_chunked_roundtrip() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_file_limits").await;
    let auth = authenticate(&u.f).await;
    let chat = auth.chat.as_ref().unwrap();
    let large = vec![0xff; crate::services::attachment_extraction::MAX_BYTES];
    let _reservation = uploads::byte_budget::reserve(large.len()).await.unwrap();
    let note = files::retain(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        chat,
        "large.bin",
        "application/octet-stream",
        &large,
    )
    .await
    .unwrap();
    let meta: Value = serde_json::from_str(note.lines().next().unwrap()).unwrap();
    let id = meta["attachment_id"].as_str().unwrap();
    let mut records =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .find(doc! {"$or":[{"_id":id},{"parent_attachment_id":id}]})
            .await
            .unwrap();
    let mut verified = 0;
    while let Some(row) = futures::TryStreamExt::try_next(&mut records).await.unwrap() {
        crate::services::assistant_record_service::verify_current_attachment(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            &row,
        )
        .await
        .unwrap();
        verified += 1;
    }
    let legacy_root =
        u.f.state
            .db
            .collection::<crate::models::assistant_attachment::AssistantAttachment>(
                crate::models::assistant_attachment::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
    let legacy_bytes = crate::services::assistant_record_service::decrypt_image(
        &u.f.state.encryption_keys,
        &legacy_root,
    )
    .await
    .unwrap();
    assert!(legacy_bytes.is_empty());
    assert_ne!(legacy_bytes.len() as i64, legacy_root.size); // Old retirement verifier's expected size.
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {"_id":id,"origin":"user_upload"})
            .await
            .unwrap(),
        0
    ); // Old child verifier's root query.
    assert_eq!(verified, 6); // Root and five 4 MiB encrypted chunks.
    let item = uploads::for_file_egress(&u.f.state.db, chat, id)
        .await
        .unwrap();
    assert_eq!(
        uploads::file_egress_bytes(&u.f.state.db, &u.f.state.encryption_keys, chat, &item)
            .await
            .unwrap(),
        large
    );
    assert_eq!(
        engine::read_attachment(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            &chat.user_id,
            &chat.conversation_id,
            id
        )
        .await
        .unwrap()
        .1,
        large
    );
    assert!(
        files::retain(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            chat,
            "too-big",
            "application/pdf",
            &vec![0; large.len() + 1]
        )
        .await
        .unwrap()
        .contains("file_exceeds_20_mib")
    );
    for _ in 1..engine::MAX_TURN_ATTACHMENTS {
        files::retain(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            chat,
            "file.bin",
            "application/octet-stream",
            b"bytes",
        )
        .await
        .unwrap();
    }
    let note = files::retain(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        chat,
        "omitted.bin",
        "application/octet-stream",
        b"bytes",
    )
    .await
    .unwrap();
    assert!(note.contains("turn_attachment_limit"));
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"origin":"tool"})
            .await
            .unwrap(),
        8
    );
    let mut other = chat.clone();
    other.turn_id = Some("other-turn".into());
    assert!(
        uploads::for_file_egress(&u.f.state.db, &other, id)
            .await
            .is_err()
    );
    let mut guest = chat.clone();
    guest.guest = true;
    assert!(
        uploads::read(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            &guest,
            &json!({"attachment_id":id})
        )
        .await
        .is_err()
    );
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.f.row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        uploads::for_file_egress(&u.f.state.db, chat, id)
            .await
            .is_err()
    );
    assert!(
        files::retain(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            chat,
            "stopped",
            "application/pdf",
            b"bytes"
        )
        .await
        .is_err()
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_final_egress_and_unsupported_paths_refuse_before_provider() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_egress").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "upload_file")
        .unwrap();
    let args = raw_args(&u.attachment_id);
    let validated = crate::services::raw_attachment_tool::uses_handle(
        &u.f.state.db,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args,
    )
    .await
    .unwrap()
    .unwrap();
    let materialized = crate::services::raw_attachment_tool::materialize(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        validated,
    )
    .await
    .unwrap();
    let guard = crate::services::assistant_service_execution::HttpEgressGuard::with_attachment(
        None,
        &u.f.state.db,
        &service,
        endpoint,
        Some(&materialized.binding),
    )
    .unwrap();
    guard.revalidate().await.unwrap();
    let before =
        crate::services::raw_attachment_tool::metadata_identity(&u.f.state.db, &u.attachment_id)
            .await
            .unwrap();
    // Re-encryption changes ciphertext, not authority or the approved buffer.
    replace_upload_bytes(&u, b"receipt bytes").await;
    guard.revalidate().await.unwrap();
    assert_eq!(
        before,
        crate::services::raw_attachment_tool::metadata_identity(&u.f.state.db, &u.attachment_id)
            .await
            .unwrap()
    );
    // Even replacement ciphertext cannot change the immutable materialized body.
    // A subsequent call materializes anew and its card-bound SHA-256 differs.
    replace_upload_bytes(&u, b"changed bytes").await;
    guard.revalidate().await.unwrap();
    assert_eq!(materialized.body.as_ref(), b"receipt bytes");
    let rows =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    rows.update_one(doc! {"_id":&u.attachment_id}, doc! {"$inc":{"size":1_i64}})
        .await
        .unwrap();
    assert!(guard.revalidate().await.is_err());
    rows.update_one(doc! {"_id":&u.attachment_id}, doc! {"$inc":{"size":-1_i64}})
        .await
        .unwrap();
    guard.revalidate().await.unwrap();
    let chunk = rows
        .find_one(doc! {"parent_attachment_id":&u.attachment_id})
        .await
        .unwrap()
        .unwrap();
    let old_id = chunk.get_str("_id").unwrap();
    let mut replacement = chunk.clone();
    replacement.insert("_id", Uuid::new_v4().to_string());
    rows.delete_one(doc! {"_id":old_id}).await.unwrap();
    rows.insert_one(replacement).await.unwrap();
    assert!(guard.revalidate().await.is_err());
    drop(guard);
    drop(materialized);
    for execution in ["job_start", "job_poll"] {
        u.f.state
            .db
            .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&endpoint.endpoint_id},
                doc! {"$set":{"execution":execution}},
            )
            .await
            .unwrap();
        assert!(
            mcp_service::prepare_chat_proxy_tool_call(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                auth.chat.as_ref(),
                &service,
                endpoint,
                &args
            )
            .await
            .is_err()
        );
    }
    u.f.state
        .db
        .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&endpoint.endpoint_id},
            doc! {"$set":{"execution":"http_operation"}},
        )
        .await
        .unwrap();
    let mut unsupported = endpoint.clone();
    unsupported.async_operation=Some(serde_json::from_value(json!({"status_operation":"status","result_operation":"result","cancel_operation":"cancel","id_field":"id","id_parameter":"id","status_field":"status","success_states":["done"],"failure_states":["error"],"error_field":null})).unwrap());
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            &unsupported,
            &args
        )
        .await
        .is_err()
    );
    for content_type in ["text/plain", "multipart/form-data"] {
        unsupported.async_operation = None;
        unsupported.request_body_schema = Some(json!({"type":"string"}));
        unsupported.request_content_type = Some(content_type.into());
        assert!(
            mcp_service::prepare_chat_proxy_tool_call(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                auth.chat.as_ref(),
                &service,
                &unsupported,
                &args
            )
            .await
            .is_err()
        );
    }
    let instance =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .find_one(doc! {"_id":&u.instance})
            .await
            .unwrap()
            .unwrap();
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":instance.get_str("endpoint_id").unwrap()},
            doc! {"$set":{"openapi_spec_url":"https://example.invalid/instance.json"}},
        )
        .await
        .unwrap();
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            endpoint,
            &args
        )
        .await
        .is_err()
    );
    assert!(
        crate::services::raw_attachment_tool::discovery_schemas(
            &u.f.state.db,
            auth.chat.as_ref(),
            std::slice::from_ref(&service)
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_binary_response_limits_preserve_the_original_reader() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_response_limits").await;
    let auth = authenticate(&u.f).await;
    let mut ordinary = auth.clone();
    ordinary.chat = None;
    for (bytes, budget_full) in [
        (
            vec![b'x'; crate::services::attachment_extraction::MAX_BYTES + 1],
            false,
        ),
        (
            [
                b"{\"text\":\"".to_vec(),
                vec![b'x'; crate::services::attachment_extraction::MAX_BYTES],
                b"\"}".to_vec(),
            ]
            .concat(),
            false,
        ),
        (vec![0xff; 100], true),
        (b"{\"text\":\"mislabeled JSON\"}".to_vec(), true),
    ] {
        u.server.reset().await;
        Mock::given(method("GET"))
            .and(path("/open-apis/download"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "content-type",
                        "application/octet-stream; charset=windows-1252",
                    )
                    .set_body_bytes(bytes),
            )
            .mount(&u.server)
            .await;
        let all = if budget_full {
            Some(
                uploads::byte_budget::reserve(uploads::byte_budget::CAPACITY / 2)
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        let expected = raw_call(&u, &ordinary, false, "download_file", json!({})).await;
        for meta in [false, true] {
            let result = raw_call(&u, &auth, meta, "download_file", json!({})).await;
            assert_legacy_chat_projection(&u, &auth, meta, &result, text(&expected)).await;
        }
        drop(all);
    }
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"origin":"tool"})
            .await
            .unwrap(),
        0
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_legacy_arguments_and_exact_descriptors_keep_original_parser() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_legacy_arguments").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "upload_file")
        .unwrap();
    let mut json_endpoint = endpoint.clone();
    json_endpoint.request_content_type = Some("application/json".into());
    json_endpoint.request_body_schema =
        Some(json!({"type":"object","properties":{"attachment_id":{"type":"string"}}}));
    let args = json!({"attachment_id":"provider-owned-id","fileName":"report.pdf"});
    for chat in [None, auth.chat.as_ref()] {
        let before = mcp_service::prepare_proxy_tool_call(&service, &json_endpoint, &args).unwrap();
        let after = mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            chat,
            &service,
            &json_endpoint,
            &args,
        )
        .await
        .unwrap();
        assert_eq!(before.operation_descriptor(), after.operation_descriptor());
        let exact =
            mcp_service::prepare_exact_proxy_tool_call(&service, &json_endpoint, &args, None)
                .unwrap();
        assert_eq!(before.operation_descriptor(), exact.operation_descriptor());
    }
    // Ineligible callers keep the original parser even with an undeclared
    // attachment_id alongside a real base64 body. Eligible chat calls refuse both.
    let args = json!({"attachment_id":"ordinary-provider-value","body":"JVBERi3/","fileName":"report.pdf"});
    let before = mcp_service::prepare_proxy_tool_call(&service, endpoint, &args);
    let mut guest = auth.chat.clone().unwrap();
    guest.guest = true;
    for chat in [None, Some(&guest)] {
        let after = mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            chat,
            &service,
            endpoint,
            &args,
        )
        .await;
        match (&before, after) {
            (Ok(before), Ok(after)) => {
                assert_eq!(before.operation_descriptor(), after.operation_descriptor());
            }
            (Err(before), Err(after)) => assert_eq!(before.to_string(), after.to_string()),
            _ => panic!("legacy parser result changed"),
        }
    }
    for enabled in [false, true] {
        writes(&u.f, enabled).await;
        assert!(
            crate::services::raw_attachment_tool::uses_handle(
                &u.f.state.db,
                None,
                &service,
                endpoint,
                &raw_args(&u.attachment_id)
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(
            crate::services::raw_attachment_tool::uses_handle(
                &u.f.state.db,
                auth.chat.as_ref(),
                &service,
                &json_endpoint,
                &args
            )
            .await
            .unwrap()
            .is_none()
        );
    }
    mode(&u, crate::models::assistant_settings::AccessMode::Autopilot).await;
    u.f.state.db.collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
        .update_one(doc! {"_id":&endpoint.endpoint_id},doc! {"$set":{"request_content_type":"application/json","request_body_schema":{"type":"object","properties":{"attachment_id":{"type":"string"}}}}}).await.unwrap();
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::AccessToken,
        AuthMethod::Delegated,
    ] {
        let mut caller = auth.clone();
        caller.auth_method = method;
        caller.chat = None;
        let response = raw_call(
            &u,
            &caller,
            false,
            "upload_file",
            json!({"attachment_id":"provider-owned-id","fileName":"report.pdf"}),
        )
        .await;
        assert_eq!(response["result"]["isError"], false, "{response}");
    }
    for request in u.server.received_requests().await.unwrap() {
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap(),
            json!({"attachment_id":"provider-owned-id"})
        );
    }
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_discovery_batches_reads_ignores_flags_and_lookup_failure_keeps_schemas() {
    let u = setup_raw("raw_discovery_compatibility").await;
    let auth = authenticate(&u.f).await;
    let mut row = u.endpoint.clone();
    row.id = Uuid::new_v4().to_string();
    row.name = "upload_second_file".into();
    u.f.state
        .db
        .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    let service = loaded(&u, &auth).await;
    u.f.state.db.run_command(doc! {"profile":2}).await.unwrap();
    for (chat, services) in [
        (None, std::slice::from_ref(&service)),
        (auth.chat.as_ref(), &[][..]),
    ] {
        assert!(
            crate::services::raw_attachment_tool::discovery_schemas(&u.f.state.db, chat, services)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>("system.profile")
            .count_documents(doc! {"command.find":{"$exists":true}})
            .await
            .unwrap(),
        0
    );
    let schemas = crate::services::raw_attachment_tool::discovery_schemas(
        &u.f.state.db,
        auth.chat.as_ref(),
        std::slice::from_ref(&service),
    )
    .await
    .unwrap();
    assert_eq!(schemas.len(), 2);
    u.f.state.db.run_command(doc! {"profile":0}).await.unwrap();
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>("system.profile")
            .count_documents(doc! {"command.find":"feature_flag_overrides"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>("system.profile")
            .count_documents(doc! {"command.find":service_endpoint::COLLECTION_NAME})
            .await
            .unwrap(),
        1
    );
    // Generic discovery never reads the Lark-only attachment-write flag.
    u.f.state.db.collection::<bson::Document>("feature_flag_overrides")
        .update_one(doc!{"flag_key":crate::services::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY},doc!{"$set":{"enabled":"invalid"}}).await.unwrap();
    for response in [
        rpc(&u.f, &auth, "tools/list", None).await,
        rpc(
            &u.f,
            &auth,
            "tools/call",
            Some(json!({"name":"nyx__search_tools","arguments":{"query":"upload"}})),
        )
        .await,
    ] {
        assert!(response.get("error").is_none(), "{response}");
        assert_ne!(response["result"]["isError"], true, "{response}");
        assert!(
            !response.to_string().contains("Failed to load tools"),
            "{response}"
        );
        assert!(
            response
                .to_string()
                .contains("File attachment ID from this owner conversation")
        );
    }
    // Fail the optional batch's endpoint deserialization after the ordinary
    // catalog has loaded. The shared discovery boundary used by list/search
    // must return no overrides, so both render the original base64 schema.
    u.f.state
        .db
        .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.endpoint.id},
            doc! {"$set":{"request_body_required":"invalid"}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::raw_attachment_tool::discovery_schemas(
            &u.f.state.db,
            auth.chat.as_ref(),
            std::slice::from_ref(&service),
        )
        .await
        .unwrap()
        .is_empty()
    );
    for endpoint in &service.endpoints {
        assert_eq!(
            mcp_service::build_input_schema(endpoint)["properties"].get("attachment_id"),
            None
        );
    }
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_missing_text_and_image_mimes_ignore_file_budget_and_nonchat_pdf_is_unchanged()
 {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_response_compatibility").await;
    let auth = authenticate(&u.f).await;
    let large = vec![b'x'; crate::services::attachment_extraction::MAX_BYTES + 1];
    let budget = uploads::byte_budget::reserve(uploads::byte_budget::CAPACITY / 2)
        .await
        .unwrap();
    for mime in [
        None,
        Some("text/plain"),
        Some("application/json"),
        Some("image/svg+xml"),
        Some("image/png"),
        Some("image/jpeg"),
        Some("image/gif"),
        Some("image/webp"),
    ] {
        u.server.reset().await;
        let mut response = ResponseTemplate::new(200).set_body_bytes(large.clone());
        if let Some(mime) = mime {
            response = response.insert_header("content-type", mime);
        }
        Mock::given(method("GET"))
            .and(path("/open-apis/download"))
            .respond_with(response)
            .mount(&u.server)
            .await;
        let result = raw_call(&u, &auth, false, "download_file", json!({})).await;
        assert_legacy_chat_projection(&u, &auth, false, &result, &String::from_utf8_lossy(&large))
            .await;
    }
    drop(budget);
    u.server.reset().await;
    let bytes = b"%PDF-\xff\x00".to_vec();
    Mock::given(method("GET"))
        .and(path("/open-apis/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .set_body_bytes(bytes.clone()),
        )
        .mount(&u.server)
        .await;
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::AccessToken,
        AuthMethod::Delegated,
        AuthMethod::ServiceAccount,
        AuthMethod::Relay,
    ] {
        let mut plain = auth.clone();
        plain.chat = None;
        plain.auth_method = method;
        let result = raw_call(&u, &plain, false, "download_file", json!({})).await;
        assert_eq!(text(&result), String::from_utf8_lossy(&bytes));
    }
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {"origin":"tool"})
            .await
            .unwrap(),
        0
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_inline_files_keep_legacy_root_payload_without_auth_document_writes() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_inline_compatibility").await;
    let auth = authenticate(&u.f).await;
    let keys =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    let credentials =
        u.f.state.db.collection::<bson::Document>(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
        );
    let key_before = keys
        .find_one(doc! {"_id":&u.f.chat.api_key_id})
        .await
        .unwrap()
        .unwrap();
    let credential_before = credentials
        .find_one(doc! {"api_key_id":&u.f.chat.api_key_id})
        .await
        .unwrap()
        .unwrap();
    let bytes = vec![0xff; crate::services::assistant_tool_files::INLINE_LIMIT];
    let _reservation = uploads::byte_budget::reserve(bytes.len()).await.unwrap();
    u.f.state.db.run_command(doc! {"profile":2}).await.unwrap();
    let note = files::retain(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref().unwrap(),
        "inline.pdf",
        "application/pdf",
        &bytes,
    )
    .await
    .unwrap();
    u.f.state.db.run_command(doc! {"profile":0}).await.unwrap();
    let auth_namespaces: Vec<_> = [
        crate::models::api_key::COLLECTION_NAME,
        crate::models::assistant_agent_credential::COLLECTION_NAME,
    ]
    .iter()
    .map(|name| format!("{}.{}", u.f.state.db.name(), name))
    .collect();
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>("system.profile")
            .count_documents(doc! {
                "$or":[{"command.update":{"$in":[crate::models::api_key::COLLECTION_NAME,
                    crate::models::assistant_agent_credential::COLLECTION_NAME]}},
                    {"op":"update","ns":{"$in":auth_namespaces}}]
            })
            .await
            .unwrap(),
        0
    );
    let meta: Value = serde_json::from_str(note.lines().next().unwrap()).unwrap();
    let id = meta["attachment_id"].as_str().unwrap();
    let row =
        u.f.state
            .db
            .collection::<crate::models::assistant_attachment::AssistantAttachment>(
                crate::models::assistant_attachment::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        crate::services::assistant_record_service::decrypt_image(&u.f.state.encryption_keys, &row)
            .await
            .unwrap()
            .as_slice(),
        bytes
    );
    let raw = bson::to_document(&row).unwrap();
    assert!(bson::to_vec(&raw).unwrap().len() < 16 * 1024 * 1024);
    assert_eq!(row.chunks, 0);
    let mut legacy = raw.clone();
    legacy.remove("chunks");
    assert_eq!(
        bson::from_document::<crate::models::assistant_attachment::AssistantAttachment>(legacy)
            .unwrap()
            .chunks,
        0
    );
    crate::services::assistant_record_service::verify_current_attachment(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        &raw,
    )
    .await
    .unwrap();
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {"parent_attachment_id":id})
            .await
            .unwrap(),
        0
    );
    let key_after = keys
        .find_one(doc! {"_id":&u.f.chat.api_key_id})
        .await
        .unwrap()
        .unwrap();
    let credential_after = credentials
        .find_one(doc! {"api_key_id":&u.f.chat.api_key_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key_before, key_after);
    assert_eq!(credential_before, credential_after);
    assert!(
        uploads::for_egress(&u.f.state.db, auth.chat.as_ref().unwrap(), id)
            .await
            .is_err()
    );
    assert!(
        uploads::chat_bytes(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref().unwrap(),
            id,
            true
        )
        .await
        .is_err()
    );
    assert!(
        uploads::for_file_egress(&u.f.state.db, auth.chat.as_ref().unwrap(), id)
            .await
            .is_ok()
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_candidate_text_is_identical_to_legacy_reqwest_decoding() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_candidate_text").await;
    let auth = authenticate(&u.f).await;
    let mut ordinary = auth.clone();
    ordinary.chat = None;
    for (mime, bytes) in [
        (
            "application/x-amz-json-1.0",
            b"{\"TableNames\":[\"one\"]}".as_slice(),
        ),
        (
            "application/x-amz-json-1.1",
            b"{\"TableNames\":[\"two\"]}".as_slice(),
        ),
        (
            "application/jwt",
            b"eyJhbGciOiJIUzI1NiJ9.e30.signature".as_slice(),
        ),
        (
            "application/x-javascript",
            b"console.log('hello');".as_slice(),
        ),
        (
            "application/octet-stream",
            b"{\"mislabelled\":true}".as_slice(),
        ),
        (
            "application/octet-stream",
            b"\xef\xbb\xbf{\"bom\":true}".as_slice(),
        ),
        (
            "application/octet-stream; charset=windows-1252",
            "caf\u{e9}".as_bytes(),
        ),
        (
            "application/octet-stream; charset=utf-16le",
            b"\xff\xfeh\0i\0".as_slice(),
        ),
        (
            "application/octet-stream; charset=windows-1252",
            b"caf\xe9".as_slice(),
        ),
    ] {
        u.server.reset().await;
        Mock::given(method("GET"))
            .and(path("/open-apis/download"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", mime)
                    .set_body_bytes(bytes),
            )
            .mount(&u.server)
            .await;
        let expected = raw_call(&u, &ordinary, false, "download_file", json!({})).await;
        // UTF-8 candidates take the buffered fallback. Invalid UTF-8 is a file
        // when capture is available, and takes the original reader under pressure.
        if std::str::from_utf8(bytes).is_ok() {
            for meta in [false, true] {
                let result = raw_call(&u, &auth, meta, "download_file", json!({})).await;
                assert_eq!(text(&result), text(&expected), "{mime}");
            }
        }
        let all = uploads::byte_budget::reserve(uploads::byte_budget::CAPACITY / 2)
            .await
            .unwrap();
        let result = raw_call(&u, &auth, false, "download_file", json!({})).await;
        assert_eq!(text(&result), text(&expected), "{mime}");
        drop(all);
    }
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {"origin":"tool"})
            .await
            .unwrap(),
        0
    );
    // Content-Disposition explicitly turns otherwise textual octet-stream data
    // into a file. The retained bytes must still equal the provider response.
    for (disposition, bytes) in [
        (Some("attachment"), b"{\"mislabelled\":true}".as_slice()),
        (
            Some("inline; filename=reply.json"),
            b"{\"mislabelled\":true}".as_slice(),
        ),
        (
            Some("inline; filename*=UTF-8''reply.json"),
            b"{\"mislabelled\":true}".as_slice(),
        ),
        (None, b"caf\xe9".as_slice()),
    ] {
        u.server.reset().await;
        let mut template = ResponseTemplate::new(200)
            .insert_header(
                "content-type",
                "application/octet-stream; charset=windows-1252",
            )
            .set_body_bytes(bytes);
        if let Some(disposition) = disposition {
            template = template.insert_header("content-disposition", disposition);
        }
        Mock::given(method("GET"))
            .and(path("/open-apis/download"))
            .respond_with(template)
            .mount(&u.server)
            .await;
        let result = raw_call(&u, &auth, false, "download_file", json!({})).await;
        let meta: Value = serde_json::from_str(text(&result).lines().next().unwrap()).unwrap();
        let id = meta["attachment_id"].as_str().unwrap();
        assert_eq!(
            engine::read_attachment(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                &u.f.owner,
                &u.f.row.id,
                id
            )
            .await
            .unwrap()
            .1,
            bytes
        );
    }
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_invalid_contract_is_clear_and_preparation_validates_once() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_validation_once").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "upload_file")
        .unwrap();
    let args = raw_args(&u.attachment_id);
    u.f.state.db.run_command(doc! {"profile":2}).await.unwrap();
    let validated = crate::services::raw_attachment_tool::uses_handle(
        &u.f.state.db,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args,
    )
    .await
    .unwrap()
    .unwrap();
    let prepared = crate::services::raw_attachment_tool::materialize(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        validated,
    )
    .await
    .unwrap();
    u.f.state.db.run_command(doc! {"profile":0}).await.unwrap();
    let profile = u.f.state.db.collection::<bson::Document>("system.profile");
    assert_eq!(
        profile
            .count_documents(doc! {"command.find":service_endpoint::COLLECTION_NAME})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        profile
            .count_documents(doc! {"command.find":"feature_flag_overrides"})
            .await
            .unwrap(),
        0
    );
    // All three identity snapshots (before/after materialization and final egress)
    // project only metadata; no payload or key-rotation metadata is fetched.
    let guard = crate::services::assistant_service_execution::HttpEgressGuard::with_attachment(
        None,
        &u.f.state.db,
        &service,
        endpoint,
        Some(&prepared.binding),
    )
    .unwrap();
    u.f.state.db.run_command(doc! {"profile":2}).await.unwrap();
    guard.revalidate().await.unwrap();
    u.f.state.db.run_command(doc! {"profile":0}).await.unwrap();
    let snapshots: Vec<bson::Document> = futures::TryStreamExt::try_collect(
        profile
            .find(doc! {
                "command.find":crate::models::assistant_attachment::COLLECTION_NAME,
                "command.projection.chunk_index":1,
            })
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(snapshots.len(), 3);
    for row in snapshots {
        let projection = row
            .get_document("command")
            .unwrap()
            .get_document("projection")
            .unwrap();
        for field in [
            "data_encrypted",
            "text_encrypted",
            "secret_context_versions",
        ] {
            assert!(!projection.contains_key(field));
        }
    }
    assert_eq!(
        profile
            .count_documents(doc! {"command.find":service_endpoint::COLLECTION_NAME})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        profile
            .count_documents(doc! {"command.find":"feature_flag_overrides"})
            .await
            .unwrap(),
        0
    );
    drop(guard);
    drop(prepared);
    let endpoints =
        u.f.state
            .db
            .collection::<bson::Document>(service_endpoint::COLLECTION_NAME);
    endpoints
        .update_one(
            doc! {"_id":&endpoint.endpoint_id},
            doc! {"$set":{"publication":"draft"}},
        )
        .await
        .unwrap();
    let refusal = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args,
    )
    .await
    .err()
    .unwrap();
    assert!(
        refusal
            .to_string()
            .contains("File uploads by attachment_id are unavailable")
    );
    assert!(!refusal.to_string().contains("base64"));
    // Ineligible callers still get precisely the old parser; flag state does
    // not change the clear contract refusal for eligible chat callers.
    let expected = mcp_service::prepare_proxy_tool_call(&service, endpoint, &args)
        .err()
        .unwrap()
        .to_string();
    for enabled in [false, true] {
        writes(&u.f, enabled).await;
        if enabled {
            u.f.state.db.collection::<bson::Document>("feature_flag_overrides")
                .update_one(doc!{"flag_key":crate::services::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY},
                    doc!{"$set":{"enabled":"invalid"}}).await.unwrap();
        }
        for chat in [None, auth.chat.as_ref()] {
            let actual = mcp_service::prepare_chat_proxy_tool_call(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                chat,
                &service,
                endpoint,
                &args,
            )
            .await
            .err()
            .unwrap();
            if chat.is_none() {
                assert_eq!(actual.to_string(), expected);
            } else {
                assert!(
                    actual
                        .to_string()
                        .contains("File uploads by attachment_id are unavailable")
                );
            }
        }
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_non_raster_images_are_opaque_and_image_wildcards_refuse() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup_raw("raw_opaque_image").await;
    let auth = authenticate(&u.f).await;
    Mock::given(method("GET"))
        .and(path("/open-apis/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/tiff")
                .set_body_bytes(b"II\x2a\x00\xff\x00".as_slice()),
        )
        .mount(&u.server)
        .await;
    let result = raw_call(&u, &auth, false, "download_file", json!({})).await;
    let metadata: Value = serde_json::from_str(text(&result).lines().next().unwrap()).unwrap();
    assert_eq!(metadata["type"], "application/octet-stream");
    let id = metadata["attachment_id"].as_str().unwrap();
    assert!(
        uploads::chat_bytes(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref().unwrap(),
            id,
            true
        )
        .await
        .is_err()
    );
    u.f.state
        .db
        .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.endpoint.id},
            doc! {"$set":{"request_content_type":"image/*"}},
        )
        .await
        .unwrap();
    let result = raw_call(&u, &auth, false, "upload_file", raw_args(id)).await;
    assert_error(&result);
    assert_eq!(
        u.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        0
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn raw_attachment_large_candidate_fallbacks_use_large_result_parts_and_legacy_decoder() {
    let _budget_guard = uploads::byte_budget::test_guard().await;
    let u = setup_raw("raw_large_result_composition").await;
    let auth = authenticate(&u.f).await;
    let mut ordinary = auth.clone();
    ordinary.chat = None;
    // Exercise buffered UTF-8/BOM/charset fallbacks and the unbuffered reader
    // under full budget pressure, through both direct and universal dispatch.
    for (mime, bytes, budget_full) in [
        (
            "application/x-amz-json-1.0",
            format!("{{\"items\":\"{}\"}}", "正文😀".repeat(3500)).into_bytes(),
            false,
        ),
        (
            "application/octet-stream",
            [b"\xef\xbb\xbf".as_slice(), "正文😀".repeat(3500).as_bytes()].concat(),
            false,
        ),
        (
            "application/octet-stream; charset=windows-1252",
            "caf\u{e9}".repeat(4000).into_bytes(),
            false,
        ),
        (
            "application/octet-stream; charset=windows-1252",
            b"caf\xe9".repeat(4000),
            true,
        ),
    ] {
        u.server.reset().await;
        Mock::given(method("GET"))
            .and(path("/open-apis/download"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", mime)
                    .set_body_bytes(bytes),
            )
            .mount(&u.server)
            .await;
        let expected = raw_call(&u, &ordinary, false, "download_file", json!({})).await;
        let budget = if budget_full {
            Some(
                uploads::byte_budget::reserve(uploads::byte_budget::CAPACITY / 2)
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        for meta in [false, true] {
            let result = raw_call(&u, &auth, meta, "download_file", json!({})).await;
            assert_legacy_chat_projection(&u, &auth, meta, &result, text(&expected)).await;
        }
        drop(budget);
    }
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {"origin":"tool"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        u.server.received_requests().await.unwrap().len(),
        3,
        "Part reads must not repeat the provider request"
    );
    u.f.state.db.drop().await.unwrap();
}
