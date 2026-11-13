//! Real catalog, conversation-key admission and provider requests for Lark wiki document resolution.
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
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct WikiFixture {
    f: Fixture,
    server: MockServer,
    instance: String,
    catalog: DownstreamService,
}
async fn setup(name: &str, specialist: bool) -> WikiFixture {
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
    catalog.requires_user_credential = true;
    catalog.base_url = format!("{}/open-apis", server.uri());
    f.state
        .db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let instance = connected(&f.state.db, &f.owner, "lark", &catalog.base_url).await;
    use crate::services::user_api_key_service::{self, CreateApiKeyParams};
    let credential = user_api_key_service::create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        CreateApiKeyParams {
            label: "Wiki mock identity",
            credential_type: "bearer",
            credential: "wiki-mock-owner-token",
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
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&instance},
            doc! {"$set":{"catalog_service_id":&catalog.id,"api_key_id":&credential.id,"auth_method":"bearer"}},
        )
        .await
        .unwrap();
    for input in catalog_spec_sync::destination_endpoint_inputs(
        &catalog,
        &catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap(),
    )
    .unwrap()
    {
        if !matches!(input.name.as_str(), "wiki_get_node" | "docx_raw_content") {
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
    WikiFixture {
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
async fn rpc(u: &WikiFixture, auth: &McpAuthContext, method: &str, params: Value) -> Value {
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
    u: &WikiFixture,
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
fn result(value: &Value) -> Value {
    serde_json::from_str(value["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn node(kind: &str) -> Value {
    json!({"code":0,"data":{"node":{"obj_type":kind,"obj_token":"document-token","space_id":"space","node_token":"wiki-token"}}})
}

#[tokio::test]
async fn wiki_resolve_then_read_same_connection_and_direct_docx_bytes_unchanged() {
    let u = setup("lark_wiki_read", false).await;
    let auth = auth(&u.f).await;
    Mock::given(method("GET"))
        .and(path("/open-apis/wiki/v2/spaces/get_node"))
        .and(wiremock::matchers::query_param("token", "wiki-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(node("docx")))
        .expect(1)
        .mount(&u.server)
        .await;
    let raw = "{ \"code\": 0, \"data\": {\"content\":\"Menu version 7\\nPickup 10:30\"} }";
    Mock::given(method("GET"))
        .and(path(
            "/open-apis/docx/v1/documents/document-token/raw_content",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw, "application/json"))
        .expect(2)
        .mount(&u.server)
        .await;
    let search = rpc(
        &u,
        &auth,
        "tools/call",
        json!({"name":"nyx__search_tools","arguments":{"query":"Lark wiki document"}}),
    )
    .await;
    assert!(
        result(&search)["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "lark__wiki_get_node")
    );
    let resolved = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"wiki-token"}),
        true,
    )
    .await;
    assert_eq!(resolved["result"]["isError"], false, "{resolved}");
    let resolved = result(&resolved);
    assert_eq!(resolved["nyxid_read"]["operation"], "docx_raw_content");
    assert_eq!(
        resolved["nyxid_read"]["failure_guidance"]["1770032"]["code"],
        "lark_document_acl_denied"
    );
    assert!(
        resolved["nyxid_read"]["failure_guidance"]["99991672"]["message"]
            .as_str()
            .unwrap()
            .contains("docx:document:readonly")
    );
    for args in [
        json!({"document_id":resolved["data"]["node"]["obj_token"],"lang":1}),
        json!({"document_id":"document-token","lang":1}),
    ] {
        let read = call(&u, &auth, "docx_raw_content", args, false).await;
        assert_eq!(read["result"]["isError"], false, "{read}");
        assert_eq!(read["result"]["content"][0]["text"], raw);
    }
    let requests = u.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer wiki-mock-owner-token"
    );
    assert_eq!(requests[1].url, requests[2].url);
    assert_eq!(requests[1].body, requests[2].body);
    assert_eq!(
        requests[1].headers.get("authorization"),
        requests[2].headers.get("authorization")
    );
    assert_eq!(
        requests[0].headers.get("authorization"),
        requests[1].headers.get("authorization")
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_failures_are_closed_metadata_and_supported_base_is_explicit() {
    let u = setup("lark_wiki_failures", false).await;
    let auth = auth(&u.f).await;
    for (body, expected) in [
        (
            json!({"code":99991672,"msg":"secret-private-url","error":{"helps":[{"url":"https://evil.test/secret"}]}}),
            "lark_scope_missing",
        ),
        (
            json!({"code":131006,"msg":"secret-private-title"}),
            "lark_document_acl_denied",
        ),
        (json!({"code":131005}), "lark_wiki_node_not_found"),
        (node("sheet"), "lark_wiki_unsupported_type"),
        (node("untrusted-secret-type"), "lark_wiki_unsupported_type"),
        (
            json!({"code":0,"data":{"node":{"obj_type":"docx"}}}),
            "lark_wiki_invalid_response",
        ),
        (
            json!({"code":0,"data":{"node":{"obj_token":"document-token"}}}),
            "lark_wiki_invalid_response",
        ),
    ] {
        u.server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&u.server)
            .await;
        let reply = call(
            &u,
            &auth,
            "wiki_get_node",
            json!({"token":"wiki-token"}),
            true,
        )
        .await;
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        // HTTP error decoration surrounds the structured diagnostic.
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(expected), "{reply}");
        assert!(!text.contains("secret"), "{reply}");
        assert!(text.contains("setup_url"));
        assert_eq!(u.server.received_requests().await.unwrap().len(), 1);
    }
    u.server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(node("bitable")))
        .mount(&u.server)
        .await;
    let reply = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"wiki-token"}),
        false,
    )
    .await;
    assert_eq!(
        result(&reply)["nyxid_read"]["operation"],
        "bitable_tables_list"
    );
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_missing_specialist_grant_links_to_grants_and_guest_url_grants_nothing() {
    let u = setup("lark_wiki_authority", true).await;
    let mut auth = auth(&u.f).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(node("docx")))
        .expect(0)
        .mount(&u.server)
        .await;
    let refusal = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"owner-private-node"}),
        false,
    )
    .await;
    assert_eq!(refusal["result"]["isError"], true, "{refusal}");
    let text = refusal.to_string();
    assert!(text.contains("missing_access"), "{text}");
    assert!(text.contains("details=grants"), "{text}");
    let search = rpc(
        &u,
        &auth,
        "tools/call",
        json!({"name":"nyx__search_tools","arguments":{"query":"wiki_get_node"}}),
    )
    .await;
    assert_eq!(
        result(&search)["lark_setup"]["code"],
        "lark_specialist_grant_required"
    );
    // A customer cannot turn an owner's URL into a service grant.
    auth.chat.as_mut().unwrap().guest = true;
    let refusal = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"owner-private-node"}),
        true,
    )
    .await;
    assert_eq!(refusal["result"]["isError"], true, "{refusal}");
    let search = rpc(
        &u,
        &auth,
        "tools/call",
        json!({"name":"nyx__search_tools","arguments":{"query":"wiki_get_node"}}),
    )
    .await;
    assert!(result(&search).get("lark_setup").is_none());
    assert!(u.server.received_requests().await.unwrap().is_empty());
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_owner_without_connection_gets_hosted_connect_instructions() {
    let u = setup("lark_wiki_no_connection", false).await;
    u.f.state
        .db
        .collection::<bson::Document>("user_services")
        .delete_one(doc! {"_id":&u.instance})
        .await
        .unwrap();
    let auth = auth(&u.f).await;
    let search = rpc(
        &u,
        &auth,
        "tools/call",
        json!({"name":"nyx__search_tools","arguments":{"query":"Lark wiki"}}),
    )
    .await;
    let guidance = &result(&search)["lark_setup"];
    assert_eq!(guidance["code"], "lark_connection_required");
    assert!(
        guidance["message"]
            .as_str()
            .unwrap()
            .contains("nyx__connect_service")
    );
    assert!(
        guidance["message"]
            .as_str()
            .unwrap()
            .contains("wiki:wiki:readonly")
    );
    assert!(
        guidance["message"]
            .as_str()
            .unwrap()
            .contains("docx:document:readonly")
    );
    assert!(
        guidance["setup_url"]
            .as_str()
            .unwrap()
            .ends_with("/assistant")
    );
    // Follow the discovery action through the real hosted-link API, without
    // contacting Lark or completing consent. Search itself creates no link.
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>("connect_links")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let connect = rpc(
        &u,
        &auth,
        "tools/call",
        json!({"name":"nyx__connect_service","arguments":{"service_slug":"api-lark"}}),
    )
    .await;
    assert_eq!(connect["result"]["isError"], false);
    let connected = result(&connect);
    assert_eq!(connected["status"], "pending_connection");
    assert!(
        connected["connect_url"]
            .as_str()
            .is_some_and(|link| link.starts_with(&u.f.state.config.frontend_url))
    );
    assert!(u.server.received_requests().await.unwrap().is_empty());
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_is_read_and_scopes_select_only_lookup_without_authorizing_docx() {
    let u = setup("lark_wiki_operations", false).await;
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
        .find(|e| e.name == "wiki_get_node")
        .unwrap();
    let prepared =
        mcp_service::prepare_proxy_tool_call(service, endpoint, &json!({"token":"wiki-token"}))
            .unwrap();
    let effects = prepared.guest_effects(service.durable_endpoint_metadata[&endpoint.endpoint_id]);
    assert!(effects.reads && !effects.destructive);
    auth.assistant_operation_scopes.insert(u.instance.clone(),serde_json::from_value(json!({"revision":1,"catalog_service_id":u.catalog.id,"operations":[{"endpoint_id":endpoint.endpoint_id,"rule":{"method":"GET","path_template":endpoint.path}}]})).unwrap());
    assert!(
        crate::services::agent_operation_scope_service::endpoint_visible(
            &auth.assistant_operation_scopes,
            service,
            endpoint
        )
    );
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(node("docx")))
        .expect(0)
        .mount(&u.server)
        .await;
    let refusal = call(
        &u,
        &auth,
        "docx_raw_content",
        json!({"document_id":"document-token"}),
        false,
    )
    .await;
    assert_eq!(refusal["result"]["isError"], true, "{refusal}");
    // The read level can admit this operation, but absent/none access cannot.
    auth.chat.as_mut().unwrap().guest = true;
    for (level, allowed) in [("none", false), ("read", true)] {
        u.f.state
            .db
            .collection::<bson::Document>("assistant_agents")
            .update_one(
                doc! {"_id":&u.f.chat.agent_id},
                doc! {"$set":{format!("guest_access.{}",u.instance):level}},
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
    assert!(u.server.received_requests().await.unwrap().is_empty());
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_lookup_does_not_follow_provider_redirects() {
    let u = setup("lark_wiki_redirect", false).await;
    let auth = auth(&u.f).await;
    Mock::given(path("/open-apis/wiki/v2/spaces/get_node"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/redirect-target", u.server.uri())),
        )
        .expect(1)
        .mount(&u.server)
        .await;
    Mock::given(path("/redirect-target"))
        .respond_with(ResponseTemplate::new(200).set_body_json(node("docx")))
        .expect(0)
        .mount(&u.server)
        .await;
    let reply = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"wiki-token"}),
        false,
    )
    .await;
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    assert!(reply.to_string().contains("lark_wiki_lookup_failed"));
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_lookup_response_is_bounded_before_model_delivery() {
    let u = setup("lark_wiki_bounded", false).await;
    let auth = auth(&u.f).await;
    let mut body = node("docx");
    body["data"]["node"]["title"] = json!("x".repeat(300 * 1024));
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&u.server)
        .await;
    let reply = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"wiki-token"}),
        false,
    )
    .await;
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    assert!(reply.to_string().len() < 4096);
    assert!(!reply.to_string().contains("document-token"));
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_guest_read_level_keeps_owner_document_input_fences() {
    use crate::services::assistant_team_service::{self, GrantChange};
    let u = setup("lark_wiki_guest_inputs", true).await;
    assistant_team_service::set_grants(
        &u.f.state.db,
        &u.f.owner,
        &u.f.chat.agent_id,
        GrantChange::Add(crate::models::assistant_agent::AgentGrants {
            service_ids: vec![u.instance.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let mut auth = auth(&u.f).await;
    let services = load_all_services_for_meta_tools(&u.f.state, &auth)
        .await
        .unwrap();
    let service = services
        .iter()
        .find(|s| s.service_id == u.instance)
        .unwrap();
    let wiki = service
        .endpoints
        .iter()
        .find(|e| e.name == "wiki_get_node")
        .unwrap();
    let docx = service
        .endpoints
        .iter()
        .find(|e| e.name == "docx_raw_content")
        .unwrap();
    use nyxid_permissions::{ValueRule, google::QueryParameter, values::InputRules};
    let wiki_inputs = InputRules {
        query: Some(
            [(
                "token".into(),
                QueryParameter {
                    required: true,
                    rule: ValueRule::Exact {
                        value: json!("customer-wiki"),
                    },
                },
            )]
            .into(),
        ),
        ..Default::default()
    };
    let docx_inputs = InputRules {
        path: [(
            "document_id".into(),
            ValueRule::Exact {
                value: json!("customer-docx"),
            },
        )]
        .into(),
        ..Default::default()
    };
    auth.assistant_operation_scopes.insert(u.instance.clone(),serde_json::from_value(json!({"revision":1,"catalog_service_id":u.catalog.id,"operations":[
        {"endpoint_id":wiki.endpoint_id,"rule":{"method":"GET","path_template":wiki.path},"inputs":wiki_inputs},
        {"endpoint_id":docx.endpoint_id,"rule":{"method":"GET","path_template":docx.path},"inputs":docx_inputs}
    ]})).unwrap());
    auth.chat.as_mut().unwrap().guest = true;
    u.f.state
        .db
        .collection::<bson::Document>("assistant_agents")
        .update_one(
            doc! {"_id":&u.f.chat.agent_id},
            doc! {"$set":{format!("guest_access.{}",u.instance):"read"}},
        )
        .await
        .unwrap();
    // Known owner tokens remain denied even with both read operations and a
    // service grant. Resolver output cannot bypass the subsequent docx fence.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(node("docx")))
        .expect(0)
        .mount(&u.server)
        .await;
    for (op, args) in [
        ("wiki_get_node", json!({"token":"owner-private-wiki"})),
        (
            "docx_raw_content",
            json!({"document_id":"owner-private-docx"}),
        ),
    ] {
        let endpoint = service.endpoints.iter().find(|e| e.name == op).unwrap();
        let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &args).unwrap();
        assert!(
            guest_service_refusal(&u.f.state, &auth, service, endpoint, &prepared, None)
                .await
                .is_none()
        );
        assert!(
            prepared
                .authorize_agent_operations(&auth.assistant_operation_scopes, service, endpoint)
                .is_err()
        );
        let reply = call(&u, &auth, op, args, true).await;
        assert_eq!(reply["result"]["isError"], true, "{reply}");
    }
    assert!(u.server.received_requests().await.unwrap().is_empty());
    u.server.reset().await;
    // Even an allowed node lookup cannot authorize the returned object when
    // that object is outside the independently granted docx input scope.
    let mut metadata = node("docx");
    metadata["data"]["node"]["obj_token"] = json!("owner-private-docx");
    Mock::given(method("GET"))
        .and(path("/open-apis/wiki/v2/spaces/get_node"))
        .and(wiremock::matchers::query_param("token", "customer-wiki"))
        .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
        .expect(1)
        .mount(&u.server)
        .await;
    let lookup = call(
        &u,
        &auth,
        "wiki_get_node",
        json!({"token":"customer-wiki"}),
        true,
    )
    .await;
    assert_eq!(lookup["result"]["isError"], false, "{lookup}");
    let object = result(&lookup)["data"]["node"]["obj_token"].clone();
    assert_eq!(object, "owner-private-docx");
    let refused = call(
        &u,
        &auth,
        "docx_raw_content",
        json!({"document_id":object}),
        true,
    )
    .await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert_eq!(u.server.received_requests().await.unwrap().len(), 1);
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn wiki_discovery_respects_existing_platform_grants() {
    let u = setup("lark_wiki_platform_grant", true).await;
    let auth = auth(&u.f).await;
    let mut services = load_all_services_for_meta_tools(&u.f.state, &auth)
        .await
        .unwrap();
    services.retain(|s| s.service_id == u.instance);
    services[0].service_id = u.catalog.id.clone();
    services[0].source = mcp_service::McpToolSource::Platform {
        downstream_service_id: u.catalog.id.clone(),
    };
    let guidance = crate::services::lark_wiki::discovery(
        &u.f.state.db,
        &services,
        &u.f.chat,
        &[],
        std::slice::from_ref(&u.catalog.id),
        &u.f.state.config.frontend_url,
    )
    .await
    .unwrap();
    assert!(guidance.is_none());
    u.f.state.db.drop().await.unwrap();
}
