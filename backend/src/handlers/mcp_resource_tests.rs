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
    let bytes = axum::body::to_bytes(response.into_body(), 3 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn resource_source_hook_direct_and_wrapped_calls_retain_one_image_and_recheck_source() {
    let f = crate::services::assistant_authority_tests::orchestrator_fixture("resource_transport")
        .await;
    let server = MockServer::start().await;
    let _origin = resource_adapter::register_test_origin(&server.uri());
    let block_id = Uuid::new_v4().to_string();
    let payload = json!({"object":"list","results":[{"object":"block","id":block_id,"type":"image","last_edited_time":"2026-10-17T00:00:00Z","image":{"type":"file","file":{"url":format!("{}/image?signature=private",server.uri())}}}],"padding":"x".repeat(12000)});
    Mock::given(path(format!("/v1/blocks/{block_id}/children")))
        .and(header("authorization", "Bearer fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&payload))
        .expect(3)
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
        .expect(1)
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
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(b"replacement-token")
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME).update_one(doc!{"_id":&key.id},doc!{"$set":{"credential_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
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
