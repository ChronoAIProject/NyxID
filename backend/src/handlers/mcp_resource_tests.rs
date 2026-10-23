//! Mocked source execution through the real MCP dispatcher and encrypted attachments.
use super::*;
use crate::{
    models::{
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
        assistant_resource::{AssistantResource, COLLECTION_NAME as RESOURCES},
    },
    services::{assistant_nyxagent as engine, resource_adapter, user_api_key_service},
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, path},
};

async fn call(state: &AppState, auth: &McpAuthContext, name: &str, arguments: Value) -> Value {
    let response = Box::pin(handle_tools_call(
        state,
        auth,
        None,
        &JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: Some(json!({"name":name,"arguments":arguments})),
        },
        false,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    ))
    .await;
    let classified_failure = tool_failure(&response).is_some();
    let bytes = axum::body::to_bytes(response.into_body(), 3 * 1024 * 1024)
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    if let Some(text) = result["result"]["content"][0]["text"].as_str()
        && let Ok(policy) = serde_json::from_str::<Value>(text)
        && matches!(
            policy["error"].as_str(),
            Some("acknowledgement_required" | "resource_confirmation_unavailable")
        )
    {
        assert!(
            !classified_failure,
            "Policy refusal must not count as a tool failure: {result}"
        );
    }
    result
}

struct SourceFixture {
    f: crate::services::assistant_authority_tests::Fixture,
    server: MockServer,
    _origin: resource_adapter::TestOrigin,
    block_id: String,
    pixels: Vec<u8>,
    catalog: crate::models::downstream_service::DownstreamService,
    endpoint: crate::models::service_endpoint::ServiceEndpoint,
    key: crate::models::user_api_key::UserApiKey,
    instance_id: String,
    endpoint_id: String,
}

async fn move_source_to_org(
    f: &crate::services::assistant_authority_tests::Fixture,
    org: &str,
    instance: &str,
    endpoint: &str,
    key: &crate::models::user_api_key::UserApiKey,
) {
    use crate::services::{credential_record_service as records, service_history};
    let mut rebound = records::rebind_key_owner(&f.state.encryption_keys, key, org)
        .await
        .unwrap();
    rebound.insert("user_id", org);
    service_history::transaction::run(&f.state.db, async |tx| {
        let keys = service_history::collection::<bson::Document>(&f.state.db, "user_api_keys")
            .update_one(
                doc! {"_id":&key.id,"user_id":&key.user_id},
                doc! {"$set":rebound.clone()},
            )
            .session(tx)
            .await?;
        assert_eq!(keys.matched_count, 1);
        for (collection, id) in [("user_services", instance), ("user_endpoints", endpoint)] {
            service_history::collection::<bson::Document>(&f.state.db, collection)
                .update_one(
                    doc! {"_id":id,"user_id":&key.user_id},
                    doc! {"$set":{"user_id":org}},
                )
                .session(tx)
                .await?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let moved = user_api_key_service::get_api_key(&f.state.db, org, &key.id)
        .await
        .unwrap();
    let raw = records::CredentialRecord::key(&moved)
        .text(
            &f.state.encryption_keys,
            "credential_encrypted",
            moved.credential_encrypted.as_deref().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw.as_str(), "fixture-token");
    assert!(
        records::CredentialRecord::key(key)
            .text(
                &f.state.encryption_keys,
                "credential_encrypted",
                moved.credential_encrypted.as_deref().unwrap()
            )
            .await
            .is_err()
    );
}

async fn source_fixture(prefix: &str, source_reads: u64, media_reads: u64) -> SourceFixture {
    let f = crate::services::assistant_authority_tests::orchestrator_fixture(prefix).await;
    let server = MockServer::start().await;
    let _origin = resource_adapter::register_test_origin(&server.uri());
    let block_id = Uuid::new_v4().to_string();
    let payload = json!({"object":"list","has_more":false,"next_cursor":null,"results":[{"object":"block","id":block_id,"type":"image","last_edited_time":"2026-10-17T00:00:00Z","image":{"type":"file","caption":[{"plain_text":"Current menu"}],"file":{"url":format!("{}/image?signature=private",server.uri())}}}],"padding":"x".repeat(12000)});
    Mock::given(path(format!("/v1/blocks/{block_id}/children")))
        .and(header("authorization", "Bearer fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .expect(source_reads)
        .mount(&server)
        .await;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let pixels = png.into_inner();
    Mock::given(path("/image"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(pixels.clone()),
        )
        .expect(media_reads)
        .mount(&server)
        .await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "api-notion".into();
    catalog.base_url = server.uri();
    catalog.auth_method = "bearer".into();
    catalog.requires_user_credential = true;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let mut endpoint = crate::test_utils::test_service_endpoint(
        &catalog.id,
        "notion_list_block_children",
        "GET",
        "/v1/blocks/{block_id}/children",
    );
    endpoint.parameters =
        Some(json!([{"name":"block_id","in":"path","required":true,"schema":{"type":"string"}}]));
    f.state
        .db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(&endpoint)
        .await
        .unwrap();
    let endpoint_id = Uuid::new_v4().to_string();
    let instance_id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint_id,
            &f.owner,
            "Renamed Notion",
            &server.uri(),
            None,
            None,
        ))
        .await
        .unwrap();
    let key = user_api_key_service::create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        user_api_key_service::CreateApiKeyParams {
            label: "Notion",
            credential_type: "bearer",
            credential: "fixture-token",
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    let mut instance = crate::test_utils::test_user_service(
        &instance_id,
        &f.owner,
        "my-notion-2",
        &endpoint_id,
        Some(&catalog.id),
        None,
    );
    instance.api_key_id = Some(key.id.clone());
    instance.auth_method = "bearer".into();
    instance.auth_key_name = "Authorization".into();
    f.state
        .db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(&instance)
        .await
        .unwrap();
    SourceFixture {
        f,
        server,
        _origin,
        block_id,
        pixels,
        catalog,
        endpoint,
        key,
        instance_id,
        endpoint_id,
    }
}

#[tokio::test]
async fn resource_source_query_is_presentation_only_preserves_image_refs_and_checks_live_access() {
    let SourceFixture {
        f,
        server,
        _origin,
        block_id,
        instance_id,
        ..
    } = source_fixture("resource_source_query", 1, 0).await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let args = json!({"tool_name":"my-notion-2__notion_list_block_children","arguments_json":json!({"block_id":block_id}).to_string(),"result_query":"menu"});
    let response = call(&f.state, &auth, "nyx__call_tool", args.clone()).await;
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("NyxID image resources"));
    let evidence: serde_json::Value =
        serde_json::from_str(text.split_once("\n\n").unwrap().1).unwrap();
    assert_eq!(evidence["query"], "menu");
    assert_eq!(evidence["matched_sections"], 1);
    assert_eq!(evidence["sections"][0]["blocks"][0]["text"], "Current menu");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(
        server.received_requests().await.unwrap()[0]
            .url
            .query()
            .is_none()
    );
    let mut non_chat = auth.clone();
    non_chat.chat = None;
    let denied = call(&f.state, &non_chat, "nyx__call_tool", args.clone()).await;
    assert_eq!(denied["result"]["isError"], true);
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! {"_id":instance_id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let revoked = call(&f.state, &auth, "nyx__call_tool", args).await;
    assert_eq!(revoked["result"]["isError"], true, "{revoked}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn resource_source_hook_direct_and_wrapped_calls_retain_one_image_and_recheck_source() {
    let SourceFixture {
        f,
        server,
        _origin,
        block_id,
        pixels,
        endpoint,
        key,
        ..
    } = source_fixture("resource_transport", 3, 1).await;
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    let source = call(
        &f.state,
        &auth,
        "my-notion-2__notion_list_block_children",
        json!({"block_id":block_id}),
    )
    .await;
    assert_eq!(source["result"]["isError"], false, "{source}");
    let text = source["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("NyxID image resources"), "{text}");
    let evidence: serde_json::Value =
        serde_json::from_str(text.split_once("\n\n").unwrap().1).unwrap();
    assert_eq!(evidence["scope"], "returned_block_page");
    assert_eq!(evidence["sections"][0]["blocks"][0]["type"], "image");
    assert!(text.len() < 6000);
    let row = f
        .state
        .db
        .collection::<AssistantResource>(RESOURCES)
        .find_one(doc! {"conversation_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(text[..text.find("\n\n").unwrap()].contains(&row.id));
    let direct = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    assert_eq!(direct["result"]["isError"], false, "{direct}");
    let current = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let first = &current.active_turn.as_ref().unwrap().attachments[0];
    assert_eq!(first.chat_delivery, Some(true));
    assert_eq!(first.image_inspection, Some(false));
    let wrapped=call(&f.state,&auth,"nyx__call_tool",json!({"tool_name":"nyx__resource_get","arguments_json":json!({"ref":row.id,"output":"inspect"}).to_string()})).await;
    assert_eq!(wrapped["result"]["isError"], false, "{wrapped}");
    let current = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let attachments = &current.active_turn.as_ref().unwrap().attachments;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].image_inspection, Some(true));
    assert_eq!(attachments[0].chat_delivery, Some(true));
    let (_, stored) = engine::read_attachment(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
        &row.attachment_id,
    )
    .await
    .unwrap();
    assert_eq!(stored, pixels);
    let media_request = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == "/image")
        .unwrap();
    assert!(media_request.headers.get("authorization").is_none());
    // A republished endpoint cannot redeem an older source handle.
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&endpoint.id},
            doc! {"$inc":{"operation_generation":1}},
        )
        .await
        .unwrap();
    let drift = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    assert_eq!(drift["result"]["isError"], true, "{drift}");
    assert!(drift.to_string().contains("resource_stale"));
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&endpoint.id},
            doc! {"$inc":{"operation_generation":-1}},
        )
        .await
        .unwrap();
    user_api_key_service::update_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &key.id,
        None,
        Some("replacement-token"),
    )
    .await
    .unwrap();
    let drift = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    assert_eq!(drift["result"]["isError"], true, "{drift}");
    assert!(drift.to_string().contains("resource_stale"));
    let mut restricted = auth.clone();
    restricted.allow_all_services = false;
    restricted.allowed_service_ids.clear();
    restricted.allowed_platform_service_ids.clear();
    let revoked = call(
        &f.state,
        &restricted,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    assert_eq!(revoked["result"]["isError"], true, "{revoked}");
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .find_one(doc! {"_id":&f.row.id})
            .await
            .unwrap()
            .unwrap()
            .get_document("active_turn")
            .unwrap()
            .get_array("attachments")
            .unwrap()
            .len(),
        1
    );
    // Both public invocation forms refuse NyxBot guest calls before source execution.
    let mut guest = auth.clone();
    guest.chat.as_mut().unwrap().guest = true;
    for (name, args) in [
        ("nyx__resource_get", json!({"ref":row.id,"output":"attach"})),
        (
            "nyx__call_tool",
            json!({"tool_name":"nyx__resource_get","arguments":{"ref":row.id,"output":"attach"}}),
        ),
    ] {
        let refused = call(&f.state, &guest, name, args).await;
        assert_eq!(refused["result"]["isError"], true, "{refused}");
    }
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

fn policy(result: &Value) -> Value {
    assert_eq!(result["result"]["isError"], true, "{result}");
    serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn select_source(
    f: &crate::services::assistant_authority_tests::Fixture,
    auth: &McpAuthContext,
    instance: &str,
    owner: &str,
    catalog: &str,
) {
    let chat = auth.chat.as_ref().unwrap();
    f.state.db.collection::<bson::Document>("assistant_service_execution").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"kind":"selection","user_id":&f.owner,
        "conversation_id":&f.row.id,"api_key_id":&chat.api_key_id,"turn_id":&chat.turn_id,
        "connection_id":instance,"selection_source":"owner","owner_id":owner,
        "catalog_service_id":catalog,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(10)),
    }).await.unwrap();
}

async fn next_resource_turn(
    f: &crate::services::assistant_authority_tests::Fixture,
    acknowledgement_id: Option<&str>,
) -> McpAuthContext {
    try_next_resource_turn(f, acknowledgement_id).await.unwrap()
}

async fn try_next_resource_turn(
    f: &crate::services::assistant_authority_tests::Fixture,
    acknowledgement_id: Option<&str>,
) -> crate::errors::AppResult<McpAuthContext> {
    // Simulate completed settlement, then use real owner admission to validate
    // the card and write the continuation message for the next turn.
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&f.row.id}, doc! {"$unset":{"active_turn":""}})
        .await?;
    engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            acknowledgement_id: acknowledgement_id.map(str::to_owned),
            retry_of: None,
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: Some(f.row.id.clone()),
            text: "Continue the image request".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await?;
    Ok(super::machine_mcp_tests::authenticated_machine_chat(f).await)
}

#[tokio::test]
async fn resource_org_auto_confirmation_preserves_policy_and_resumes_only_its_exact_intent() {
    Box::pin(org_resource_journey(
        crate::models::assistant_settings::AccessMode::Nyxmode,
        false,
    ))
    .await;
}
#[tokio::test]
async fn resource_org_ask_confirmation_preserves_policy_and_resumes_only_its_exact_intent() {
    Box::pin(org_resource_journey(
        crate::models::assistant_settings::AccessMode::AskEveryTime,
        false,
    ))
    .await;
}
#[tokio::test]
async fn resource_org_auto_confirmation_rejects_mode_drift_and_recovers_in_full() {
    Box::pin(org_resource_journey(
        crate::models::assistant_settings::AccessMode::Nyxmode,
        true,
    ))
    .await;
}
#[tokio::test]
async fn resource_org_ask_confirmation_rejects_mode_drift_and_recovers_in_full() {
    Box::pin(org_resource_journey(
        crate::models::assistant_settings::AccessMode::AskEveryTime,
        true,
    ))
    .await;
}
fn org_resource_journey(
    mode: crate::models::assistant_settings::AccessMode,
    switch_to_full: bool,
) -> impl std::future::Future<Output = ()> {
    // Keep the large journey fixture out of the test runtime's caller frames.
    Box::pin(org_resource_journey_inner(mode, switch_to_full))
}

async fn org_resource_journey_inner(
    mode: crate::models::assistant_settings::AccessMode,
    switch_to_full: bool,
) {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_settings_service as settings,
    };
    let SourceFixture {
        f,
        server,
        _origin,
        block_id,
        catalog,
        instance_id,
        endpoint_id,
        key,
        ..
    } = source_fixture(
        &format!("resource_org_confirmation_{mode:?}"),
        if switch_to_full { 4 } else { 3 },
        2,
    )
    .await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(crate::test_utils::test_user(
            &org,
            crate::models::user::UserType::Org,
        ))
        .await
        .unwrap();
    f.state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org,
            &f.owner,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    move_source_to_org(&f, &org, &instance_id, &endpoint_id, &key).await;
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            access_mode: Some(mode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
    select_source(&f, &auth, &instance_id, &org, &catalog.id).await;
    let source = call(
        &f.state,
        &auth,
        "my-notion-2__notion_list_block_children",
        json!({"block_id":block_id}),
    )
    .await;
    let initial = policy(&source);
    assert_eq!(initial["error"], "acknowledgement_required", "{initial}");
    let initial_ack = initial["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.row.id, initial_ack, true)
        .await
        .unwrap();
    let auth = next_resource_turn(&f, Some(initial_ack)).await;
    let source = call(
        &f.state,
        &auth,
        "my-notion-2__notion_list_block_children",
        json!({"block_id":block_id}),
    )
    .await;
    assert_eq!(source["result"]["isError"], false, "{source}");
    let row = f
        .state
        .db
        .collection::<AssistantResource>(RESOURCES)
        .find_one(doc! {"conversation_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    let refused = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    let refusal = policy(&refused);
    assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    let ack = refusal["acknowledgement_id"].as_str().unwrap();
    let conflicting = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"inspect"}),
    )
    .await;
    let conflict = policy(&conflicting);
    assert_eq!(
        conflict["error"], "resource_confirmation_unavailable",
        "{conflict}"
    );
    assert_eq!(
        conflict["source_policy"]["error"],
        "acknowledgement_required"
    );
    assert_eq!(conflict["source_policy"]["acknowledgement_id"], ack);
    let pending = f
        .state
        .db
        .collection::<AssistantResource>(RESOURCES)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap()
        .pending_confirmation
        .unwrap();
    assert_eq!(
        pending.output,
        crate::models::assistant_resource::ResourceOutput::Attach
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    acks::decide(&f.state.db, &f.owner, &f.row.id, ack, true)
        .await
        .unwrap();
    let wrong_intent = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"inspect","acknowledgement_id":ack}),
    )
    .await;
    assert_eq!(wrong_intent["result"]["isError"], true);
    let attached = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach","acknowledgement_id":ack}),
    )
    .await;
    assert_eq!(attached["result"]["isError"], false, "{attached}");
    let used = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach","acknowledgement_id":ack}),
    )
    .await;
    assert_eq!(policy(&used)["error"], "acknowledgement_invalid");
    let refused = call(
        &f.state,
        &auth,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    let refusal = policy(&refused);
    assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
    let next_ack = refusal["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.row.id, next_ack, true)
        .await
        .unwrap();
    let unrelated = next_resource_turn(&f, None).await;
    let denied = call(
        &f.state,
        &unrelated,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    if switch_to_full {
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let error = match try_next_resource_turn(&f, Some(next_ack)).await {
            Err(error) => error,
            Ok(_) => panic!("A changed access mode must invalidate the saved service binding"),
        };
        assert!(
            matches!(error, crate::errors::AppError::Conflict(ref message)
            if message == "Confirmed service changed; request a new confirmation")
        );
        let current = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        assert!(current.active_turn.is_none());
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
        let cards = f
            .state
            .db
            .collection::<bson::Document>("assistant_acknowledgements");
        let card_count = cards
            .count_documents(doc! {"conversation_id":&f.row.id,"decided_by":{"$ne":"access_mode"}})
            .await
            .unwrap();
        assert_eq!(
            cards
                .find_one(doc! {"_id":next_ack})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "allowed"
        );

        let fresh = next_resource_turn(&f, None).await;
        let denied = call(
            &f.state,
            &fresh,
            "nyx__resource_get",
            json!({"ref":row.id,"output":"attach"}),
        )
        .await;
        assert_eq!(denied["result"]["isError"], true, "{denied}");
        let source = call(
            &f.state,
            &fresh,
            "my-notion-2__notion_list_block_children",
            json!({"block_id":block_id}),
        )
        .await;
        assert_eq!(source["result"]["isError"], false, "{source}");
        let fresh_row = f
            .state
            .db
            .collection::<AssistantResource>(RESOURCES)
            .find_one(
                doc! {"conversation_id":&f.row.id,"turn_id":&fresh.chat.as_ref().unwrap().turn_id},
            )
            .await
            .unwrap()
            .unwrap();
        assert_ne!(fresh_row.id, row.id);
        assert_ne!(fresh_row.attachment_id, row.attachment_id);
        let attached = call(
            &f.state,
            &fresh,
            "nyx__call_tool",
            json!({"tool_name":"nyx__resource_get","arguments":{"ref":fresh_row.id,"output":"attach"}}),
        )
        .await;
        assert_eq!(attached["result"]["isError"], false, "{attached}");
        let current = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        let images = &current.active_turn.as_ref().unwrap().attachments;
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].id, fresh_row.attachment_id);
        assert_eq!(images[0].chat_delivery, Some(true));
        assert_eq!(
            cards
                .count_documents(
                    doc! {"conversation_id":&f.row.id,"decided_by":{"$ne":"access_mode"}}
                )
                .await
                .unwrap(),
            card_count
        );
        assert_eq!(
            cards
                .find_one(doc! {"_id":next_ack})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "allowed"
        );
        server.verify().await;
        f.state.db.drop().await.unwrap();
        return;
    }
    let continued = next_resource_turn(&f, Some(next_ack)).await;
    let wrong_intent = call(
        &f.state,
        &continued,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"inspect"}),
    )
    .await;
    assert_eq!(wrong_intent["result"]["isError"], true);
    let attached = call(
        &f.state,
        &continued,
        "nyx__call_tool",
        json!({"tool_name":"nyx__resource_get","arguments":{"ref":row.id,"output":"attach"}}),
    )
    .await;
    assert_eq!(attached["result"]["isError"], false, "{attached}");
    let consumed = f
        .state
        .db
        .collection::<bson::Document>("assistant_acknowledgements")
        .find_one(doc! {"_id":next_ack})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(consumed.get_str("status").unwrap(), "used");
    let current = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let images = &current.active_turn.as_ref().unwrap().attachments;
    assert_eq!(images.len(), 1);
    assert_ne!(images[0].id, row.attachment_id);
    assert_eq!(images[0].chat_delivery, Some(true));
    let used = call(
        &f.state,
        &continued,
        "nyx__resource_get",
        json!({"ref":row.id,"output":"attach"}),
    )
    .await;
    assert_eq!(used["result"]["isError"], true, "{used}");
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn resource_explicit_org_card_remains_authority_bound_in_full_access() {
    use crate::{
        models::assistant_settings::AccessMode,
        services::{
            assistant_acknowledgement_service as acks, assistant_settings_service as settings,
        },
    };

    for (mode, wrapped) in [
        (AccessMode::Nyxmode, false),
        (AccessMode::AskEveryTime, true),
    ] {
        let SourceFixture {
            f,
            server,
            _origin,
            block_id,
            catalog,
            instance_id,
            endpoint_id,
            key,
            ..
        } = source_fixture(&format!("resource_same_turn_mode_drift_{mode:?}"), 1, 0).await;
        let org = Uuid::new_v4().to_string();
        f.state
            .db
            .collection("users")
            .insert_one(crate::test_utils::test_user(
                &org,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        f.state
            .db
            .collection("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org,
                &f.owner,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        move_source_to_org(&f, &org, &instance_id, &endpoint_id, &key).await;
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                access_mode: Some(AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let auth = super::machine_mcp_tests::authenticated_machine_chat(&f).await;
        select_source(&f, &auth, &instance_id, &org, &catalog.id).await;
        let source = call(
            &f.state,
            &auth,
            "my-notion-2__notion_list_block_children",
            json!({"block_id":block_id}),
        )
        .await;
        assert_eq!(source["result"]["isError"], false, "{source}");
        let row = f
            .state
            .db
            .collection::<AssistantResource>(RESOURCES)
            .find_one(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap()
            .unwrap();
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                access_mode: Some(mode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let refused = call(
            &f.state,
            &auth,
            "nyx__resource_get",
            json!({"ref":row.id,"output":"attach"}),
        )
        .await;
        let refusal = policy(&refused);
        assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
        let ack = refusal["acknowledgement_id"].as_str().unwrap();
        acks::decide(&f.state.db, &f.owner, &f.row.id, ack, true)
            .await
            .unwrap();
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                access_mode: Some(AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let arguments = json!({"ref":row.id,"output":"attach","acknowledgement_id":ack});
        let (name, arguments) = if wrapped {
            (
                "nyx__call_tool",
                json!({"tool_name":"nyx__resource_get","arguments":arguments}),
            )
        } else {
            ("nyx__resource_get", arguments)
        };
        let refused = call(&f.state, &auth, name, arguments).await;
        assert_eq!(
            policy(&refused)["error"],
            "acknowledgement_invalid",
            "{mode:?}: {refused}"
        );
        let card = f
            .state
            .db
            .collection::<bson::Document>("assistant_acknowledgements")
            .find_one(doc! {"_id":ack})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.get_str("status").unwrap(), "allowed");
        let current = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
        assert!(current.active_turn.as_ref().unwrap().attachments.is_empty());
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
                .count_documents(doc! {"conversation_id":&f.row.id})
                .await
                .unwrap(),
            0
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        server.verify().await;
        f.state.db.drop().await.unwrap();
    }
}
