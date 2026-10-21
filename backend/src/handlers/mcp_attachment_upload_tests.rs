//! Real chat dispatch tests for the stored-catalog attachment capability.
use super::*;
use crate::models::{
    downstream_service::{self, DownstreamService},
    service_endpoint::{self, PublicationState, ServiceEndpoint},
};
use crate::services::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    assistant_upload_service as uploads, catalog_spec_registry, catalog_spec_sync,
    lark_attachment_tool::{self as attachment, UPLOAD_OPERATION},
    service_endpoint_service,
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct UploadFixture {
    f: Fixture,
    server: MockServer,
    catalog: DownstreamService,
    instance: String,
    endpoint: ServiceEndpoint,
    attachment_id: String,
}

async fn replace_upload_bytes(u: &UploadFixture, bytes: &[u8]) {
    let chunks =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    let chunk = chunks
        .find_one(doc! {"parent_attachment_id":&u.attachment_id,"chunk_index":0_i64})
        .await
        .unwrap()
        .unwrap();
    let chunk_id = chunk.get_str("_id").unwrap();
    let encrypted = crate::services::assistant_record_service::encrypt_chunk(
        &u.f.state.encryption_keys,
        chunk_id,
        &u.f.owner,
        &u.attachment_id,
        0,
        bytes,
    )
    .await
    .unwrap();
    let updated = chunks
        .update_one(
            doc! {"_id":chunk_id},
            doc! {"$set":{"data_encrypted":bson::Binary {
                subtype:bson::spec::BinarySubtype::Generic,
                bytes:encrypted,
            }}},
        )
        .await
        .unwrap();
    assert_eq!(updated.matched_count, 1);
}

async fn flag(f: &Fixture, key: &str, enabled: bool) {
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>("feature_flag_overrides")
        .update_one(
            doc! {"flag_key":key,"target_kind":"user","target_key":&f.owner},
            doc! {"$set":{"enabled":enabled,"updated_at":now},"$setOnInsert":{
            "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
            "updated_by":"test","created_at":now}},
        )
        .upsert(true)
        .await
        .unwrap();
}
async fn writes(f: &Fixture, enabled: bool) {
    flag(
        f,
        crate::services::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
        enabled,
    )
    .await;
}
async fn bind_upload(f: &Fixture, bytes: &[u8]) -> String {
    let item = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
        "receipt.txt",
        bytes.to_vec(),
    )
    .await
    .unwrap();
    let message = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"conversation_id":&f.row.id,"role":"user"})
        .await
        .unwrap()
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    uploads::bind(
        &f.state.db,
        &f.owner,
        &f.row.id,
        message.get_str("_id").unwrap(),
        std::slice::from_ref(&item.id),
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    item.id
}
fn setup(name: &'static str) -> futures::future::BoxFuture<'static, UploadFixture> {
    Box::pin(async move {
        let f = orchestrator_fixture(name).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/open-apis/drive/v1/medias/upload_all"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"file_token":"file_receipt"}})),
            )
            .mount(&server)
            .await;
        let mut catalog = downstream_service::test_helpers::dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.name = "Lark".into();
        catalog.slug = "api-lark".into();
        catalog.created_by = "system".into();
        let base_url = format!("{}/open-apis", server.uri());
        catalog.base_url = base_url.clone();
        f.state
            .db
            .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        let instance = connected(&f.state.db, &f.owner, "receipts", &base_url).await;
        f.state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&instance},
                doc! {"$set":{"catalog_service_id":&catalog.id}},
            )
            .await
            .unwrap();
        let spec = catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap();
        let mut endpoint = None;
        for input in catalog_spec_sync::destination_endpoint_inputs(&catalog, &spec).unwrap() {
            if !matches!(
                input.name.as_str(),
                UPLOAD_OPERATION | "bitable_record_create"
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
                .insert_one(&row)
                .await
                .unwrap();
            if row.name == UPLOAD_OPERATION {
                endpoint = Some(row);
            }
        }
        writes(&f, true).await;
        let attachment_id = bind_upload(&f, b"receipt bytes").await;
        UploadFixture {
            f,
            server,
            catalog,
            instance,
            endpoint: endpoint.unwrap(),
            attachment_id,
        }
    })
}
async fn authenticate(f: &Fixture) -> McpAuthContext {
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
async fn rpc(f: &Fixture, auth: &McpAuthContext, method: &str, params: Option<Value>) -> Value {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params,
    };
    let response = if method == "tools/list" {
        Box::pin(handle_tools_list(&f.state, auth, None, &request)).await
    } else {
        Box::pin(handle_tools_call(
            &f.state,
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
async fn call(u: &UploadFixture, auth: &McpAuthContext, meta: bool, args: Value) -> Value {
    let name = "receipts__bitable_media_upload";
    let params = if meta {
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}})
    } else {
        json!({"name":name,"arguments":args})
    };
    rpc(&u.f, auth, "tools/call", Some(params)).await
}
fn args(id: &str) -> Value {
    json!({"attachment_id":id,"parent_type":"bitable_file","parent_node":"base"})
}
fn text(body: &Value) -> &str {
    body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{body}"))
}
fn payload(body: &Value) -> Value {
    serde_json::from_str(text(body)).unwrap_or_else(|e| panic!("{e}: {body}"))
}
fn assert_error(body: &Value) {
    assert_eq!(body["result"]["isError"], true, "{body}");
}
async fn assert_no_requests(u: &UploadFixture) {
    assert!(u.server.received_requests().await.unwrap().is_empty());
}
async fn loaded(u: &UploadFixture, auth: &McpAuthContext) -> mcp_service::McpToolService {
    load_all_services_for_meta_tools(&u.f.state, auth)
        .await
        .unwrap()
        .into_iter()
        .find(|service| service.service_id == u.instance)
        .unwrap()
}

#[tokio::test]
async fn lark_attachment_dispatch_direct_and_meta_send_canonical_parts_and_return_file_token() {
    let u = setup("attachment_dispatch").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == UPLOAD_OPERATION)
        .unwrap();
    assert!(matches!(
        endpoint.attachment_input,
        Some(attachment::Projection::Stored(_))
    ));
    let schema = mcp_service::build_input_schema(endpoint);
    assert!(schema["properties"].get("file").is_none());
    assert_eq!(schema["properties"]["attachment_id"]["type"], "string");
    let prepared = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args(&u.attachment_id),
    )
    .await
    .unwrap();
    let expected_identity = prepared
        .attachment_confirmation_arguments(endpoint, &args(&u.attachment_id))
        .unwrap();
    drop(prepared);
    for meta in [false, true] {
        let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_eq!(response["result"]["isError"], false, "{response}");
        assert!(text(&response).contains("file_receipt"));
    }
    let requests = u.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    for request in requests {
        let ct = request
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(Some(ct), expected_identity["content_type"].as_str());
        let boundary = ct
            .strip_prefix("multipart/form-data; boundary=nyxid-")
            .unwrap();
        assert_eq!(boundary.len(), 32);
        let body = String::from_utf8(request.body).unwrap();
        for (key, value) in [
            ("file_name", "receipt.txt"),
            ("parent_type", "bitable_file"),
            ("parent_node", "base"),
            ("size", "13"),
            ("extra", "{\"drive_route_token\":\"base\"}"),
        ] {
            assert!(
                body.contains(&format!("name=\"{key}\"\r\n\r\n{value}\r\n")),
                "{body}"
            );
        }
        assert!(body.contains("name=\"file\"; filename=\"receipt.txt\"\r\nContent-Type: text/plain\r\n\r\nreceipt bytes\r\n"));
    }
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_provenance_spoofs_and_edited_stored_inputs_refuse_before_provider() {
    let u = setup("attachment_provenance").await;
    let auth = authenticate(&u.f).await;
    let original = bson::to_document(&u.endpoint).unwrap();
    for (field, value) in [
        ("path", bson::Bson::String("/edited".into())),
        (
            "request_body_schema",
            bson::to_bson(&json!({"type":"object","properties":{"file":{"type":"string"}}}))
                .unwrap(),
        ),
        (
            "parameters",
            bson::to_bson(
                &json!([{"name":"x","in":"query","required":true,"schema":{"type":"string"}}]),
            )
            .unwrap(),
        ),
        ("request_body_required", bson::Bson::Boolean(false)),
        ("target_id", bson::Bson::String("foreign".into())),
    ] {
        let mut changed = original.clone();
        changed.insert(field, value);
        u.f.state
            .db
            .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
            .replace_one(doc! {"_id":&u.endpoint.id}, changed)
            .await
            .unwrap();
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
        }
        assert_no_requests(&u).await;
    }
    u.f.state
        .db
        .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
        .replace_one(doc! {"_id":&u.endpoint.id}, original)
        .await
        .unwrap();
    for (creator, slug) in [
        ("user", "api-lark"),
        ("system", "custom-lark"),
        ("user", "api-feishu"),
    ] {
        u.f.state
            .db
            .collection::<bson::Document>(downstream_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&u.catalog.id},
                doc! {"$set":{"created_by":creator,"slug":slug}},
            )
            .await
            .unwrap();
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
        }
        assert_no_requests(&u).await;
    }
    // A user-owned service may copy both the operation name and full schema.
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.instance},
            doc! {"$unset":{"catalog_service_id":""}},
        )
        .await
        .unwrap();
    let spec_server = MockServer::start().await;
    Mock::given(path("/custom.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(&*catalog_spec_registry::spec_for_slug("api-lark").unwrap()),
        )
        .mount(&spec_server)
        .await;
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
            doc! {"$set":{"openapi_spec_url":format!("{}/custom.json",spec_server.uri())}},
        )
        .await
        .unwrap();
    for meta in [false, true] {
        assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_mounted_remote_and_exact_hosted_overlays_never_gain_projection() {
    let u = setup("attachment_mounted").await;
    let auth = authenticate(&u.f).await;
    let instance =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .find_one(doc! {"_id":&u.instance})
            .await
            .unwrap()
            .unwrap();
    let spec_server = MockServer::start().await;
    for exact in [false, true] {
        let mut spec = (*catalog_spec_registry::spec_for_slug("api-lark").unwrap()).clone();
        if !exact {
            spec["servers"] = json!([{"url":format!("{}/open-apis",u.server.uri())}]);
        }
        let route = if exact { "/exact.json" } else { "/remote.json" };
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(spec))
            .mount(&spec_server)
            .await;
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
            .update_one(
                doc! {"_id":instance.get_str("endpoint_id").unwrap()},
                doc! {"$set":{"openapi_spec_url":format!("{}{route}",spec_server.uri())}},
            )
            .await
            .unwrap();
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
        }
        assert_no_requests(&u).await;
    }
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_discovery_failure_returns_fixed_public_error() {
    let u = setup("attachment_discovery_failure").await;
    let auth = authenticate(&u.f).await;
    u.f.state
        .db
        .collection::<bson::Document>("feature_flag_overrides")
        .update_one(
            doc! {"flag_key":crate::services::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
                "target_kind":"user","target_key":&u.f.owner},
            doc! {"$set":{"enabled":"private override detail"}},
        )
        .await
        .unwrap();
    let mut services = mcp_service::load_operation_catalog(
        &u.f.state.db,
        u.f.state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(&auth),
        mcp_service_scope(&auth),
    )
    .await
    .unwrap()
    .services;
    let error = attachment::filter_discovery(&u.f.state.db, auth.chat.as_ref(), &mut services)
        .await
        .unwrap_err();
    assert!(
        matches!(error, crate::errors::AppError::DatabaseError(_)),
        "{error}"
    );
    assert_eq!(
        rpc(&u.f, &auth, "tools/list", None).await,
        json!({"jsonrpc":"2.0","id":1,
            "error":{"code":-32603,"message":"Failed to load tools"}})
    );
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_discovery_chat_flag_matrix_config_and_exact_keep_plain_create() {
    let u = setup("attachment_discovery").await;
    let mut auth = authenticate(&u.f).await;
    for media in [false, true] {
        flag(
            &u.f,
            crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
            media,
        )
        .await;
        for enabled in [false, true] {
            writes(&u.f, enabled).await;
            let services = load_all_services_for_meta_tools(&u.f.state, &auth)
                .await
                .unwrap();
            let service = services
                .iter()
                .find(|service| service.service_id == u.instance)
                .unwrap();
            assert_eq!(
                service
                    .endpoints
                    .iter()
                    .any(|ep| ep.name == UPLOAD_OPERATION),
                enabled
            );
            assert!(
                service
                    .endpoints
                    .iter()
                    .any(|ep| ep.name == "bitable_record_create")
            );
            let list = rpc(&u.f, &auth, "tools/list", None).await;
            assert_eq!(
                list["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == "receipts__bitable_media_upload"),
                enabled,
                "{list}"
            );
            let search = rpc(&u.f,&auth,"tools/call",Some(json!({"name":"nyx__search_tools","arguments":{"query":"bitable_media_upload"}}))).await;
            assert_eq!(
                text(&search).contains("receipts__bitable_media_upload"),
                enabled,
                "{search}"
            );
            let before = u.server.received_requests().await.unwrap().len();
            for meta in [false, true] {
                let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
                assert_eq!(response["result"]["isError"], !enabled, "{response}");
            }
            assert_eq!(
                u.server.received_requests().await.unwrap().len(),
                before + usize::from(enabled) * 2
            );
        }
    }
    let config = crate::handlers::mcp::get_mcp_config(State(u.f.state.clone()), u.f.auth.clone())
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
            .all(|ep| ep.name != UPLOAD_OPERATION)
    );
    assert!(
        configured
            .endpoints
            .iter()
            .any(|ep| ep.name == "bitable_record_create")
    );
    let services = load_all_services_for_meta_tools(&u.f.state, &auth)
        .await
        .unwrap();
    let exact = serde_json::to_value(mcp_service::exact_operation_view(&services)).unwrap();
    assert!(!exact.to_string().contains(UPLOAD_OPERATION));
    assert!(exact.to_string().contains("bitable_record_create"));
    auth.chat = None;
    let services = load_all_services_for_meta_tools(&u.f.state, &auth)
        .await
        .unwrap();
    assert!(
        services
            .iter()
            .flat_map(|s| &s.endpoints)
            .all(|ep| ep.name != UPLOAD_OPERATION)
    );
    for meta in [false, true] {
        assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
    }
    assert_eq!(u.server.received_requests().await.unwrap().len(), 4);
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_origin_access_refuses_cross_owner_conversation_guest_stopped_revoked() {
    let u = setup("attachment_access").await;
    let auth = authenticate(&u.f).await;
    let original =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .find_one(doc! {"_id":&u.attachment_id})
            .await
            .unwrap()
            .unwrap();
    for (field, value) in [
        ("user_id", "other"),
        ("conversation_id", "nyxa-other"),
        ("origin", "tool"),
        ("origin", "unknown"),
    ] {
        let mut row = original.clone();
        row.insert(field, value);
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .replace_one(doc! {"_id":&u.attachment_id}, row)
            .await
            .unwrap();
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
        }
        assert_no_requests(&u).await;
    }
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .replace_one(doc! {"_id":&u.attachment_id}, original)
        .await
        .unwrap();
    let mut guest = authenticate(&u.f).await;
    guest.chat.as_mut().unwrap().guest = true;
    for meta in [false, true] {
        assert_error(&call(&u, &guest, meta, args(&u.attachment_id)).await);
    }
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.f.row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    for meta in [false, true] {
        let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_eq!(response["error"]["code"], -32603, "{response}");
        assert_eq!(
            response["error"]["message"], "Assistant turn is no longer active",
            "{response}"
        );
    }
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.f.row.id},
            doc! {"$set":{"active_turn.stop_requested":false}},
        )
        .await
        .unwrap();
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.f.chat.api_key_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    for meta in [false, true] {
        assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_tool_images_keep_producing_turn_and_origin_without_fallback() {
    let mut u = setup("attachment_tool_image").await;
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(true);
    keys.set_allow_legacy_record_reads(false);
    u.f.state.encryption_keys = std::sync::Arc::new(keys);
    let auth = authenticate(&u.f).await;
    let mut image = crate::models::assistant_attachment::AssistantAttachment {
        secret_context_versions: Default::default(),
        id: Uuid::new_v4().to_string(),
        origin: "tool".into(),
        user_id: u.f.owner.clone(),
        conversation_id: u.f.row.id.clone(),
        turn_id: u.f.chat.turn_id.clone().unwrap(),
        content_type: "image/jpeg".into(),
        size: 4,
        data_encrypted: Vec::new(),
        created_at: chrono::Utc::now(),
    };
    image.data_encrypted = crate::services::assistant_record_service::encrypt_image(
        &u.f.state.encryption_keys,
        &image,
        b"JPEG",
    )
    .await
    .unwrap();
    crate::services::credential_record_service::mark_version(
        &mut image.secret_context_versions,
        &u.f.state.encryption_keys,
        "data_encrypted",
    );
    u.f.state
        .db
        .collection::<crate::models::assistant_attachment::AssistantAttachment>(
            crate::models::assistant_upload::COLLECTION_NAME,
        )
        .insert_one(&image)
        .await
        .unwrap();
    assert!(matches!(
        uploads::for_egress(&u.f.state.db, auth.chat.as_ref().unwrap(), &image.id)
            .await
            .unwrap(),
        uploads::EgressAttachment::Tool(_)
    ));
    let mut original = bson::to_document(&image).unwrap();
    for (field, value) in [("turn_id", "prior-turn"), ("origin", "user_upload")] {
        let mut row = original.clone();
        row.insert(field, value);
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .replace_one(doc! {"_id":&image.id}, row)
            .await
            .unwrap();
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, args(&image.id)).await);
        }
    }
    // The legacy absent origin is a tool image, never a fallback to user upload.
    original.remove("origin");
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .replace_one(doc! {"_id":&image.id}, original)
        .await
        .unwrap();
    assert!(
        uploads::for_egress(&u.f.state.db, auth.chat.as_ref().unwrap(), &image.id)
            .await
            .is_ok()
    );
    assert_no_requests(&u).await;
    for meta in [false, true] {
        let response = call(&u, &auth, meta, args(&image.id)).await;
        assert_eq!(response["result"]["isError"], false, "{response}");
    }
    let requests = u.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let body = String::from_utf8(request.body).unwrap();
        assert!(
            body.contains("name=\"file_name\"\r\n\r\ntool-image.jpg\r\n"),
            "{body}"
        );
        assert!(
            body.contains(
                "filename=\"tool-image.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nJPEG\r\n"
            ),
            "{body}"
        );
    }
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_cards_bind_bytes_filename_parent_type_and_destination_on_both_dispatch_paths()
 {
    let u = setup("attachment_cards").await;
    crate::services::assistant_settings_service::update(
        &u.f.state.db,
        &u.f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let auth = authenticate(&u.f).await;
    for meta in [false, true] {
        let request = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_error(&request);
        let card = payload(&request);
        assert_eq!(card["error"], "acknowledgement_required", "{card}");
        let ack = card["acknowledgement_id"].as_str().unwrap();
        let history = acks::history(&u.f.state.db, &u.f.owner, &u.f.row.id)
            .await
            .unwrap();
        let digest = history
            .iter()
            .find(|a| a.id == ack)
            .unwrap()
            .arguments_digest
            .clone();
        let again = payload(&call(&u, &auth, meta, args(&u.attachment_id)).await);
        assert_eq!(again["acknowledgement_id"], ack);
        assert_eq!(
            acks::history(&u.f.state.db, &u.f.owner, &u.f.row.id)
                .await
                .unwrap()
                .iter()
                .find(|a| a.id == ack)
                .unwrap()
                .arguments_digest,
            digest
        );
        acks::decide(&u.f.state.db, &u.f.owner, &u.f.row.id, ack, true)
            .await
            .unwrap();
        let mut confirmed = args(&u.attachment_id);
        confirmed["acknowledgement_id"] = json!(ack);
        for (field, value) in [
            ("file_name", "other.txt"),
            ("parent_node", "other-base"),
            ("parent_type", "bitable_image"),
        ] {
            let mut changed = confirmed.clone();
            changed[field] = json!(value);
            let response = call(&u, &auth, meta, changed).await;
            assert_error(&response);
            assert_eq!(payload(&response)["error"], "acknowledgement_invalid");
        }
        replace_upload_bytes(&u, b"changed bytes").await;
        let changed = call(&u, &auth, meta, confirmed.clone()).await;
        assert_error(&changed);
        assert_eq!(payload(&changed)["error"], "acknowledgement_invalid");
        replace_upload_bytes(&u, b"receipt bytes").await;
        let sent = call(&u, &auth, meta, confirmed).await;
        assert_eq!(sent["result"]["isError"], false, "{sent}");
    }
    assert_eq!(u.server.received_requests().await.unwrap().len(), 2);
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_body_scopes_and_unsupported_preparation_fail_closed_before_effects() {
    let u = setup("attachment_scopes").await;
    let mut auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == UPLOAD_OPERATION)
        .unwrap();
    assert!(
        mcp_service::prepare_proxy_tool_call(&service, endpoint, &args(&u.attachment_id)).is_err()
    );
    assert!(
        mcp_service::prepare_exact_proxy_tool_call(
            &service,
            endpoint,
            &args(&u.attachment_id),
            Some("exact")
        )
        .is_err()
    );
    assert!(
        mcp_service::build_mcp_operation_descriptor(&service, endpoint, &args(&u.attachment_id))
            .is_err()
    );
    let mut async_endpoint = endpoint.clone();
    async_endpoint.async_operation=Some(serde_json::from_value(json!({"status_operation":"status","result_operation":"result","cancel_operation":"cancel","id_field":"id","id_parameter":"id","status_field":"status","success_states":["done"],"failure_states":["error"],"error_field":null})).unwrap());
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            &async_endpoint,
            &args(&u.attachment_id)
        )
        .await
        .is_err()
    );
    let scope:crate::models::agent_operation_scope::AgentOperationScope=serde_json::from_value(json!({
        "revision":1,"catalog_service_id":u.catalog.id,"operations":[{"endpoint_id":endpoint.endpoint_id,
        "rule":{"method":"POST","path_template":endpoint.path},"inputs":{"body":{"type":"object","properties":{},"required":[]}}}]
    })).unwrap();
    auth.assistant_operation_scopes
        .insert(u.instance.clone(), scope);
    let prepared = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args(&u.attachment_id),
    )
    .await
    .unwrap();
    let error = prepared
        .authorize_agent_operations(&auth.assistant_operation_scopes, &service, endpoint)
        .unwrap_err();
    assert!(
        error.to_string().contains("Operation scope input limit"),
        "{error}"
    );
    for meta in [false, true] {
        assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_dispatch_refuses_malformed_fields_and_size_before_provider() {
    let u = setup("attachment_malformed").await;
    let auth = authenticate(&u.f).await;
    for (key, value) in [
        ("file_name", json!("../receipt.txt")),
        ("file_name", json!("a\r\nb")),
        ("file_name", json!("nul\0name")),
        ("parent_node", json!("x".repeat(1025))),
        ("size", json!(12)),
        ("extra", json!("{\"drive_route_token\":\"different\"}")),
        ("file", json!("raw bytes")),
        ("_method", json!("DELETE")),
    ] {
        let mut value_args = args(&u.attachment_id);
        value_args[key] = value;
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, value_args.clone()).await);
        }
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_budget_precedes_decryption_and_is_held_by_prepared_call_until_drop() {
    use crate::services::assistant_upload_service::byte_budget::{CAPACITY, reserve};
    use std::time::Duration;
    let u = setup("attachment_egress_budget").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == UPLOAD_OPERATION)
        .unwrap();
    let original =
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .find_one(doc! {"parent_attachment_id":&u.attachment_id})
            .await
            .unwrap()
            .unwrap();
    // A malformed encrypted payload would fail immediately if decrypted before reserving.
    u.f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .update_one(doc! {"_id":original.get_str("_id").unwrap()},doc! {"$set":{"data_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0]}}}).await.unwrap();
    let full = reserve(CAPACITY / 2).await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            mcp_service::prepare_chat_proxy_tool_call(
                &u.f.state.db,
                &u.f.state.encryption_keys,
                auth.chat.as_ref(),
                &service,
                endpoint,
                &args(&u.attachment_id)
            )
        )
        .await
        .is_err()
    );
    drop(full);
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            endpoint,
            &args(&u.attachment_id)
        )
        .await
        .is_err()
    );
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .replace_one(doc! {"_id":original.get_str("_id").unwrap()}, &original)
        .await
        .unwrap();
    let prepared = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &args(&u.attachment_id),
    )
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), reserve(CAPACITY / 2))
            .await
            .is_err()
    );
    drop(prepared);
    drop(
        tokio::time::timeout(Duration::from_secs(1), reserve(CAPACITY / 2))
            .await
            .unwrap()
            .unwrap(),
    );
    assert_no_requests(&u).await;
    u.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(json!({"code":0,"data":{"file_token":"token"}})),
        )
        .mount(&u.server)
        .await;
    let (response, ()) = tokio::join!(call(&u, &auth, false, args(&u.attachment_id)), async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while u.server.received_requests().await.unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), reserve(CAPACITY / 2))
                .await
                .is_err()
        );
    });
    assert_eq!(response["result"]["isError"], false, "{response}");
    drop(
        tokio::time::timeout(Duration::from_secs(1), reserve(CAPACITY / 2))
            .await
            .unwrap()
            .unwrap(),
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_upload_requires_http_and_business_success_with_nonempty_file_token() {
    let u = setup("attachment_response").await;
    let auth = authenticate(&u.f).await;
    for (status, body) in [
        (500, json!({"code":0,"data":{"file_token":"token"}})),
        (200, json!({"code":999,"data":{"file_token":"token"}})),
        (200, json!({"code":0,"data":{}})),
        (200, json!({"code":0,"data":{"file_token":""}})),
    ] {
        u.server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&u.server)
            .await;
        for meta in [false, true] {
            assert_error(&call(&u, &auth, meta, args(&u.attachment_id)).await);
        }
        assert_eq!(u.server.received_requests().await.unwrap().len(), 2);
    }
    // The standalone operation never sends a Base-record create.
    assert!(
        u.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() == "/open-apis/drive/v1/medias/upload_all")
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_platform_cards_bind_materialized_bytes_and_boundary_content_type() {
    let u = setup("attachment_platform_card").await;
    crate::services::assistant_settings_service::update(
        &u.f.state.db,
        &u.f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let auth = authenticate(&u.f).await;
    let mut service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == UPLOAD_OPERATION)
        .unwrap()
        .clone();
    service.source = mcp_service::McpToolSource::Platform {
        downstream_service_id: u.catalog.id.clone(),
    };
    service.service_id = u.catalog.id.clone();
    let original = args(&u.attachment_id);
    let prepared = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        &endpoint,
        &original,
    )
    .await
    .unwrap();
    let refused = webhook_service_gate(
        &u.f.state,
        &auth,
        &service,
        &endpoint,
        &prepared,
        "platform-upload",
        &original,
        None,
    )
    .await
    .unwrap_err();
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(refused.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    let card = payload(&body);
    let id = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&u.f.state.db, &u.f.owner, &u.f.row.id, id, true)
        .await
        .unwrap();
    let mut retry = original.clone();
    retry["acknowledgement_id"] = json!(id);
    replace_upload_bytes(&u, b"changed bytes").await;
    let changed = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        &endpoint,
        &original,
    )
    .await
    .unwrap();
    let refused = webhook_service_gate(
        &u.f.state,
        &auth,
        &service,
        &endpoint,
        &changed,
        "platform-upload",
        &retry,
        None,
    )
    .await
    .unwrap_err();
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(refused.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload(&body)["error"], "acknowledgement_invalid");
    assert!(
        webhook_service_gate(
            &u.f.state,
            &auth,
            &service,
            &endpoint,
            &prepared,
            "platform-upload",
            &retry,
            None
        )
        .await
        .is_ok()
    );
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_actual_send_revalidates_after_delayed_credential_preparation() {
    use std::time::Duration;
    for drift in ["attachment", "key", "contract", "stop"] {
        let u = setup("attachment_final_send").await;
        Mock::given(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(500))
                    .set_body_json(json!({"token":"test-token","expires_in":60})),
            )
            .mount(&u.server)
            .await;
        let key = Uuid::new_v4().to_string();
        let encrypted = u.f.state.encryption_keys.encrypt(b"{}").await.unwrap();
        u.f.state.db.collection::<bson::Document>("user_api_keys").insert_one(doc! {
            "_id":&key,"user_id":&u.f.owner,"label":"exchange","credential_type":"api_key","status":"active",
            "credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
        u.f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&u.instance},
                doc! {"$set":{"api_key_id":key,"auth_method":"token_exchange"}},
            )
            .await
            .unwrap();
        let config = crate::models::downstream_service::TokenExchangeConfig {
            endpoint: format!("{}/token", u.server.uri()),
            request_encoding: "json".into(),
            request_template: json!({}),
            token_response_path: "token".into(),
            ttl_response_path: Some("expires_in".into()),
            default_ttl_secs: 60,
            injection: "bearer".into(),
            error_code_path: None,
            error_message_path: None,
            credential_fields: vec![],
        };
        u.f.state
            .db
            .collection::<bson::Document>(downstream_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&u.catalog.id},
                doc! {"$set":{"token_exchange_config":bson::to_bson(&config).unwrap()}},
            )
            .await
            .unwrap();
        let auth = authenticate(&u.f).await;
        let (response, ()) = tokio::join!(call(&u, &auth, false, args(&u.attachment_id)), async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while !u
                    .server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.url.path() == "/token")
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("credential exchange reached");
            let (collection, filter, change) = match drift {
                "attachment" => (
                    crate::models::assistant_upload::COLLECTION_NAME,
                    doc! {"_id":&u.attachment_id},
                    doc! {"$set":{"message_id":bson::Bson::Null}},
                ),
                "key" => (
                    crate::models::api_key::COLLECTION_NAME,
                    doc! {"_id":&u.f.chat.api_key_id},
                    doc! {"$set":{"is_active":false}},
                ),
                "contract" => (
                    service_endpoint::COLLECTION_NAME,
                    doc! {"_id":&u.endpoint.id},
                    doc! {"$set":{"path":"/changed"}},
                ),
                "stop" => (
                    crate::models::assistant_conversation::COLLECTION_NAME,
                    doc! {"_id":&u.f.row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                ),
                _ => unreachable!(),
            };
            u.f.state
                .db
                .collection::<bson::Document>(collection)
                .update_one(filter, change)
                .await
                .unwrap();
        });
        assert_error(&response);
        assert!(
            u.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.url.path() == "/token"),
            "{drift}: {response}"
        );
        u.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn lark_attachment_unmarked_multipart_keeps_existing_local_refusal_on_both_dispatch_paths() {
    let u = setup("attachment_unmarked_multipart").await;
    let auth = authenticate(&u.f).await;
    let mut ordinary = u.endpoint.clone();
    ordinary.id = Uuid::new_v4().to_string();
    ordinary.name = "ordinary_upload".into();
    ordinary.path = "/ordinary".into();
    u.f.state
        .db
        .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .insert_one(&ordinary)
        .await
        .unwrap();
    for meta in [false, true] {
        let name = "receipts__ordinary_upload";
        let arguments = json!({"body":"raw multipart"});
        let params = if meta {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":arguments.to_string()}})
        } else {
            json!({"name":name,"arguments":arguments})
        };
        let response = rpc(&u.f, &auth, "tools/call", Some(params)).await;
        assert_error(&response);
        assert!(
            text(&response).contains("multipart/form-data request bodies are not yet supported"),
            "{response}"
        );
    }
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_attachment_public_discovery_excludes_upload_and_keeps_ordinary_endpoints() {
    let u = setup("attachment_public_discovery").await;
    u.f.state.db.collection::<bson::Document>(downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id":&u.catalog.id},doc! {"$set":{"anonymous_endpoints":[
            {"id":"upload","enabled":true,"method":"POST","path_pattern":"/open-apis/drive/v1/medias/upload_all","daily_quota":100},
            {"id":"ordinary","enabled":true,"method":"GET","path_pattern":"/ordinary","daily_quota":100}
        ]}}).await.unwrap();
    let services = mcp_service::load_public_tools(&u.f.state.db).await.unwrap();
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].endpoints.len(), 1);
    assert_eq!(services[0].endpoints[0].path, "/ordinary");
    assert_no_requests(&u).await;
    u.f.state.db.drop().await.unwrap();
}
