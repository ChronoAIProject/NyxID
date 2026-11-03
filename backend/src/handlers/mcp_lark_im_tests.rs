//! Real catalog, conversation-key admission and provider requests for Lark IM.
use super::*;
use crate::{
    models::{
        downstream_service::{self, DownstreamService},
        service_endpoint::{self, PublicationState, ServiceEndpoint},
    },
    services::{
        assistant_authority_tests::{Fixture, connected, direct_fixture, orchestrator_fixture},
        catalog_spec_registry, catalog_spec_sync, service_endpoint_service,
    },
};
use axum::Json;
use base64::Engine;
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct ImFixture {
    f: Fixture,
    server: MockServer,
    instance: String,
    catalog: DownstreamService,
}
async fn setup(name: &str, specialist: bool) -> ImFixture {
    let f = if specialist {
        direct_fixture(name).await
    } else {
        orchestrator_fixture(name).await
    };
    let server = MockServer::start().await;
    let mut catalog = downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "api-lark".into();
    catalog.created_by = "system".into();
    catalog.base_url = format!("{}/open-apis", server.uri());
    f.state
        .db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let instance = connected(&f.state.db, &f.owner, "lark", &catalog.base_url).await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&instance},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    for input in catalog_spec_sync::destination_endpoint_inputs(
        &catalog,
        &catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap(),
    )
    .unwrap()
    {
        if !matches!(
            input.name.as_str(),
            "im_message_create" | "im_message_reply" | "im_image_create"
        ) {
            continue;
        }
        let row = service_endpoint_service::endpoint_from_input(
            &catalog.id,
            input,
            PublicationState::Published,
        );
        f.state
            .db
            .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
            .insert_one(row)
            .await
            .unwrap();
    }
    ImFixture {
        f,
        server,
        instance,
        catalog,
    }
}
async fn auth(f: &Fixture) -> McpAuthContext {
    let key = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.raw_key.parse().unwrap());
    authenticate_mcp(&f.state, &headers, false).await.unwrap()
}
async fn rpc(u: &ImFixture, auth: &McpAuthContext, method: &str, params: Value) -> Value {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params: Some(params),
    };
    let response = if method == "tools/list" {
        Box::pin(handle_tools_list(&u.f.state, auth, None, &request)).await
    } else {
        Box::pin(handle_tools_call(
            &u.f.state,
            auth,
            None,
            &request,
            false,
            crate::services::billing::route_inventory::internal_node_dispatch_permit(),
        ))
        .await
    };
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn call(
    u: &ImFixture,
    auth: &McpAuthContext,
    operation: &str,
    args: Value,
    meta: bool,
) -> Value {
    let name = format!("lark__{operation}");
    let params = if meta {
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}})
    } else {
        json!({"name":name,"arguments":args})
    };
    rpc(u, auth, "tools/call", params).await
}
fn image_args() -> Value {
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    json!({"image_base64":base64::engine::general_purpose::STANDARD.encode(output.into_inner()),"filename":"card.png"})
}
fn success(value: &Value) {
    assert_eq!(value["result"]["isError"], false, "{value}");
}
fn failure(value: &Value) {
    assert_eq!(value["result"]["isError"], true, "{value}");
}

#[tokio::test]
async fn lark_im_messages_create_reply_raw_typed_and_split_dispatch_once() {
    let u = setup("lark_im_messages", false).await;
    let auth = auth(&u.f).await;
    for p in [
        "/open-apis/im/v1/messages",
        "/open-apis/im/v1/messages/message/reply",
    ] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"sent"}})),
            )
            .mount(&u.server)
            .await;
    }
    let text = "第一行\n第二行 🎉 \"quote\" C:\\path literal\\n";
    for operation in ["im_message_create", "im_message_reply"] {
        for meta in [false, true] {
            for content in [
                json!({"text":text}),
                json!({"content":{"text":text}}),
                json!({"content":format!(" {{ \"text\": {} }} ",json!(text))}),
            ] {
                let mut args = content;
                args["msg_type"] = json!("text");
                args["uuid"] = json!("stable");
                if operation == "im_message_create" {
                    args["receive_id"] = json!("chat");
                    args["receive_id_type"] = json!("chat_id");
                } else {
                    args["message_id"] = json!("message");
                }
                success(&call(&u, &auth, operation, args, meta).await);
            }
        }
    }
    for part in ["第一段\n中文", "第二段\n🙂"] {
        success(&call(&u,&auth,"im_message_create",json!({"receive_id":"chat","receive_id_type":"chat_id","msg_type":"text","text":part}),false).await);
    }
    let sent = u.server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 14);
    for request in &sent[..12] {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(body["content"].as_str().unwrap()).unwrap(),
            json!({"text":text})
        );
        assert!(body.get("text").is_none());
        assert_eq!(body["uuid"], "stable");
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&sent[2].body).unwrap()["content"],
        format!(" {{ \"text\": {} }} ", json!(text))
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_im_invalid_json_never_dispatches_and_230001_is_an_error_without_content() {
    let u = setup("lark_im_validation", false).await;
    let auth = auth(&u.f).await;
    for fields in [
        json!({"text":"secret","content":"{}"}),
        json!({"content":"secret"}),
        json!({"content":"\"double\""}),
    ] {
        let mut args = fields;
        args["receive_id"] = json!("chat");
        args["receive_id_type"] = json!("chat_id");
        args["msg_type"] = json!("text");
        let reply = call(&u, &auth, "im_message_create", args, false).await;
        failure(&reply);
        assert!(reply.to_string().contains("lark_message_content_invalid"));
        assert!(!reply.to_string().contains("secret"));
    }
    assert!(u.server.received_requests().await.unwrap().is_empty());
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":230001,"msg":"private message secret"})),
        )
        .mount(&u.server)
        .await;
    let reply = call(
        &u,
        &auth,
        "im_message_create",
        json!({"receive_id":"chat","receive_id_type":"chat_id","msg_type":"text","text":"secret"}),
        false,
    )
    .await;
    failure(&reply);
    let reply = reply.to_string();
    assert!(!reply.contains("secret"));
    assert!(reply.contains("local_json_parsed"));
    assert!(reply.contains("request_sent"));
    assert_eq!(u.server.received_requests().await.unwrap().len(), 1);
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_im_image_upload_mcp_discovery_multipart_result_and_rejection() {
    let u = setup("lark_im_upload", false).await;
    let auth = auth(&u.f).await;
    let tools = rpc(&u, &auth, "tools/list", json!({})).await;
    assert!(tools.to_string().contains("lark__im_image_create"));
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/images"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"image_key":"img_card"}})),
        )
        .mount(&u.server)
        .await;
    for meta in [false, true] {
        let reply = call(&u, &auth, "im_image_create", image_args(), meta).await;
        success(&reply);
        assert!(reply.to_string().contains("img_card"));
    }
    let sent = u.server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 2);
    for request in sent {
        assert!(
            request.headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with("multipart/form-data; boundary=")
        );
        let text = String::from_utf8_lossy(&request.body);
        assert!(text.contains("name=\"image_type\"\r\n\r\nmessage"));
        assert!(text.contains("name=\"image\"; filename=\"card.png\""));
    }
    let Json(config) = crate::handlers::mcp::get_mcp_config(
        axum::extract::State(u.f.state.clone()),
        crate::test_utils::test_auth_user(&u.f.owner),
    )
    .await
    .unwrap();
    let config = serde_json::to_value(config).unwrap();
    assert!(!config.to_string().contains("im_image_create"));
    assert!(config.to_string().contains("im_message_create"));
    u.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":99991672,"msg":"secret payload"})),
        )
        .mount(&u.server)
        .await;
    let reply = call(&u, &auth, "im_image_create", image_args(), false).await;
    failure(&reply);
    assert!(reply.to_string().contains("lark_image_upload_rejected"));
    assert!(!reply.to_string().contains("secret payload"));
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_im_image_ungranted_agent_and_operation_selection_refuse_before_upload() {
    for specialist in [true, false] {
        let u = setup("lark_im_scope", specialist).await;
        // Use a valid image and a successful provider: a missing authority
        // check must not pass this test merely because an unsafe URL failed.
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"image_key":"should_not_upload"}})),
            )
            .mount(&u.server)
            .await;
        let mut auth = auth(&u.f).await;
        if !specialist {
            let scope = serde_json::from_value(
                json!({"revision":1,"catalog_service_id":u.catalog.id,"operations":[]}),
            )
            .unwrap();
            auth.assistant_operation_scopes
                .insert(u.instance.clone(), scope);
        }
        let reply = call(&u, &auth, "im_image_create", image_args(), false).await;
        failure(&reply);
        assert!(!reply.to_string().contains("lark_image_fetch"));
        assert!(u.server.received_requests().await.unwrap().is_empty());
        u.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn lark_im_image_is_a_guest_change_and_refuses_exact_dispatch() {
    let u = setup("lark_im_guest_change", false).await;
    let mut auth = auth(&u.f).await;
    let services = load_all_services_for_meta_tools(&u.f.state, &auth)
        .await
        .unwrap();
    let service = services
        .iter()
        .find(|s| s.service_id == u.instance)
        .unwrap();
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "im_image_create")
        .unwrap();
    let args = image_args();
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &args).unwrap();
    assert!(mcp_service::build_mcp_operation_descriptor(service, endpoint, &args).is_err());
    assert!(mcp_service::prepare_exact_proxy_tool_call(service, endpoint, &args, None).is_err());
    let effects = prepared.guest_effects(service.durable_endpoint_metadata[&endpoint.endpoint_id]);
    assert!(!effects.reads && effects.uses && !effects.destructive);
    auth.chat.as_mut().unwrap().guest = true;
    for (level, allowed) in [("read", false), ("use", true)] {
        u.f.state
            .db
            .collection::<bson::Document>("assistant_agents")
            .update_one(
                doc! {"_id": &u.f.chat.agent_id},
                doc! {"$set": {format!("guest_access.{}", u.instance): level}},
            )
            .await
            .unwrap();
        assert_eq!(
            guest_service_refusal(&u.f.state, &auth, service, endpoint, &prepared, None)
                .await
                .is_none(),
            allowed
        );
    }
    let guard = crate::services::assistant_service_execution::HttpEgressGuard::with_attachment(
        None,
        &u.f.state.db,
        service,
        endpoint,
        None,
    )
    .unwrap();
    guard.revalidate().await.unwrap();
    u.f.state
        .db
        .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &endpoint.endpoint_id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    assert!(
        guard.revalidate().await.is_err(),
        "catalog withdrawal fences an already-prepared upload"
    );
    assert!(u.server.received_requests().await.unwrap().is_empty());
    u.f.state.db.drop().await.unwrap();
}
