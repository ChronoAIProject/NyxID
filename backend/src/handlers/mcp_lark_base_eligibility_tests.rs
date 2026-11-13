use super::*;

const SLUGS: [&str; 4] = ["api-lark", "api-lark-bot", "api-feishu", "api-feishu-bot"];
const OPERATIONS: [&str; 4] = [
    attachment::COMPOSITE_OPERATION,
    attachment::FIELDS_OPERATION,
    attachment::UPLOAD_OPERATION,
    attachment::CREATE_OPERATION,
];

async fn second_connection(slug: &'static str) -> BaseFixture {
    let mut u = setup_catalog("lark_2327", slug).await;
    u.catalog.slug = slug.into();
    u.f.state
        .db
        .collection::<bson::Document>(downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id":&u.catalog.id}, doc! {"$set":{"slug":slug}})
        .await
        .unwrap();
    u.f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&u.instance},
            doc! {"$set":{"slug":format!("{slug}-2")}},
        )
        .await
        .unwrap();
    // A managed OAuth app still grants the user's own credential. It is not a
    // UserService platform-key binding and must remain eligible.
    u.f.state
        .db
        .collection::<bson::Document>("user_api_keys")
        .update_many(
            doc! {"user_id":&u.f.owner,"credential_type":"oauth2"},
            doc! {"$set":{"credential_source":"platform"}},
        )
        .await
        .unwrap();
    let endpoints =
        u.f.state
            .db
            .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME);
    endpoints
        .delete_many(doc! {"service_id":&u.catalog.id})
        .await
        .unwrap();
    for input in catalog_spec_sync::destination_endpoint_inputs(
        &u.catalog,
        &catalog_spec_registry::spec_for_slug(slug).unwrap(),
    )
    .unwrap()
    {
        if OPERATIONS.contains(&input.name.as_str()) {
            let row = service_endpoint_service::endpoint_from_input(
                &u.catalog.id,
                input,
                PublicationState::Published,
            );
            endpoints.insert_one(&row).await.unwrap();
            if row.name == attachment::COMPOSITE_OPERATION {
                u.endpoint = row;
            }
        }
    }
    u
}

async fn raw(u: &BaseFixture, auth: &McpAuthContext) -> mcp_service::McpToolService {
    mcp_service::load_operation_catalog(
        &u.f.state.db,
        u.f.state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(auth),
        mcp_service_scope(auth),
    )
    .await
    .unwrap()
    .services
    .into_iter()
    .find(|s| s.service_id == u.instance)
    .unwrap()
}

async fn assert_parity(
    u: &BaseFixture,
    auth: &McpAuthContext,
    service: mcp_service::McpToolService,
    reason: Option<&str>,
) {
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == attachment::COMPOSITE_OPERATION)
        .unwrap();
    let mut input = args(&u.attachment_id);
    input["fields"] = json!({});
    let execution = mcp_service::prepare_chat_proxy_tool_call(
        &u.f.state.db,
        &u.f.state.encryption_keys,
        auth.chat.as_ref(),
        &service,
        endpoint,
        &input,
    )
    .await;
    if let Some(reason) = reason {
        let error = match execution {
            Ok(_) => panic!("unexpected admission: {reason}"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(reason),
            "expected {reason}, got {error}"
        );
    } else {
        assert!(execution.is_ok(), "{}", execution.err().unwrap());
    }
    let mut services = vec![service];
    attachment::filter_discovery(&u.f.state.db, auth.chat.as_ref(), &mut services)
        .await
        .unwrap();
    let advertised = services[0]
        .endpoints
        .iter()
        .any(|ep| ep.name == attachment::COMPOSITE_OPERATION);
    assert_eq!(
        advertised,
        reason.is_none(),
        "discovery/execution disagree: {reason:?}"
    );
    assert!(requests(u).await.is_empty());
}

#[tokio::test]
async fn legacy_publication_second_connections_four_slugs_have_discovery_execution_parity() {
    for slug in SLUGS {
        let u = second_connection(slug).await;
        let auth = authenticate(&u.f).await;
        // Each missing dependency used to break save while discovery advertised it.
        for operation in OPERATIONS {
            u.f.state
                .db
                .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
                .update_one(
                    doc! {"service_id":&u.catalog.id,"name":operation},
                    doc! {"$unset":{"publication":"","operation_generation":""}},
                )
                .await
                .unwrap();
            let service = raw(&u, &auth).await;
            assert!(service.executable);
            assert_eq!(service.service_slug, format!("{slug}-2"));
            assert_parity(&u, &auth, service, None).await;
        }
        let search = rpc(
            &u.f,
            &auth,
            "tools/call",
            Some(json!({"name":"nyx__search_tools",
            "arguments":{"query":format!("{slug}-2__{}", attachment::COMPOSITE_OPERATION)}})),
        )
        .await;
        let result = payload(&search);
        let matched = result["matches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == format!("{slug}-2__{}", attachment::COMPOSITE_OPERATION))
            .unwrap();
        assert_eq!(matched["executable"], true);
        assert_eq!(matched["chat_access"], "granted");
        let config =
            crate::handlers::mcp::get_mcp_config(State(u.f.state.clone()), u.f.auth.clone())
                .await
                .unwrap()
                .0;
        assert!(config.services.iter().flat_map(|s| &s.endpoints).all(|ep| {
            ![attachment::COMPOSITE_OPERATION, UPLOAD_OPERATION].contains(&ep.name.as_str())
        }));
        cleanup(u).await;
    }
}

#[tokio::test]
async fn all_four_slugs_refuse_each_live_authority_and_contract_failure_in_both_paths() {
    for slug in SLUGS {
        let u = second_connection(slug).await;
        let mut auth = authenticate(&u.f).await;
        let original = auth.chat.clone();
        for (field, reason) in [
            ("guest", "attachment_write_guest_turn"),
            ("stopped", "attachment_write_not_live_owner_chat"),
            ("absent", "attachment_write_not_live_owner_chat"),
        ] {
            let service = raw(&u, &auth).await;
            match field {
                "guest" => auth.chat.as_mut().unwrap().guest = true,
                "stopped" => auth.chat.as_mut().unwrap().turn_live = false,
                _ => auth.chat = None,
            }
            assert_parity(&u, &auth, service, Some(reason)).await;
            auth.chat = original.clone();
        }
        let service = raw(&u, &auth).await;
        writes(&u.f, false).await;
        assert_parity(&u, &auth, service, Some("attachment_write_flag_off")).await;
        writes(&u.f, true).await;
        let connection =
            u.f.state
                .db
                .collection::<bson::Document>("user_services")
                .find_one(doc! {"_id":&u.instance})
                .await
                .unwrap()
                .unwrap();
        let mount = connection.get_str("endpoint_id").unwrap();
        let credential = connection.get_str("api_key_id").unwrap();
        let cases = [
            (
                "user_services",
                doc! {"_id":&u.instance},
                doc! {"$set":{"credential_binding":"platform"}},
                doc! {"$unset":{"credential_binding":""}},
                "connection_credential_required",
            ),
            (
                "user_services",
                doc! {"_id":&u.instance},
                doc! {"$set":{"node_id":"node"}},
                doc! {"$unset":{"node_id":""}},
                "hosted_connection_required",
            ),
            (
                "user_services",
                doc! {"_id":&u.instance},
                doc! {"$unset":{"api_key_id":""}},
                doc! {"$set":{"api_key_id":credential}},
                "credential_unavailable",
            ),
            (
                "user_services",
                doc! {"_id":&u.instance},
                doc! {"$set":{"auth_method":"none"}},
                doc! {"$set":{"auth_method":connection.get_str("auth_method").unwrap()}},
                "credential_unavailable",
            ),
            (
                "user_api_keys",
                doc! {"_id":credential},
                doc! {"$set":{"status":"revoked"}},
                doc! {"$set":{"status":"active"}},
                "credential_unavailable",
            ),
            (
                "api_keys",
                doc! {"_id":&auth.chat.as_ref().unwrap().api_key_id},
                doc! {"$set":{"is_active":false}},
                doc! {"$set":{"is_active":true}},
                "key_ineligible",
            ),
            (
                "api_keys",
                doc! {"_id":&auth.chat.as_ref().unwrap().api_key_id},
                doc! {"$set":{"expires_at":bson::DateTime::from_millis(1)}},
                doc! {"$set":{"expires_at":bson::Bson::Null}},
                "key_ineligible",
            ),
            (
                "assistant_agent_credentials",
                doc! {"api_key_id":&auth.chat.as_ref().unwrap().api_key_id},
                doc! {"$set":{"conversation_id":"wrong"}},
                doc! {"$set":{"conversation_id":&u.f.row.id}},
                "key_ineligible",
            ),
            (
                "assistant_conversations",
                doc! {"_id":&u.f.row.id},
                doc! {"$set":{"active_turn.stop_requested":true}},
                doc! {"$set":{"active_turn.stop_requested":false}},
                "not_live_owner_chat",
            ),
            (
                "downstream_services",
                doc! {"_id":&u.catalog.id},
                doc! {"$set":{"is_active":false}},
                doc! {"$set":{"is_active":true}},
                "catalog_unavailable",
            ),
            (
                "user_services",
                doc! {"_id":&u.instance},
                doc! {"$set":{"is_active":false}},
                doc! {"$set":{"is_active":true}},
                "connection_unavailable",
            ),
            (
                "user_services",
                doc! {"_id":&u.instance},
                doc! {"$set":{"catalog_service_id":"other"}},
                doc! {"$set":{"catalog_service_id":&u.catalog.id}},
                "catalog_binding_changed",
            ),
            (
                "user_endpoints",
                doc! {"_id":mount},
                doc! {"$set":{"openapi_spec_url":"https://untrusted.invalid/spec.json"}},
                doc! {"$unset":{"openapi_spec_url":""}},
                "untrusted_spec_override",
            ),
            (
                "service_endpoints",
                doc! {"_id":&u.endpoint.id},
                doc! {"$set":{"publication":"paused"}},
                doc! {"$set":{"publication":"published"}},
                "operation_not_published",
            ),
            (
                "service_endpoints",
                doc! {"_id":&u.endpoint.id},
                doc! {"$set":{"operation_generation":0_i64}},
                doc! {"$set":{"operation_generation":1_i64}},
                "invalid_operation_generation",
            ),
            (
                "service_endpoints",
                doc! {"service_id":&u.catalog.id,"name":attachment::FIELDS_OPERATION},
                doc! {"$set":{"operation_generation":2_i64}},
                doc! {"$set":{"operation_generation":1_i64}},
                "operation_generation_changed",
            ),
            (
                "service_endpoints",
                doc! {"service_id":&u.catalog.id,"name":attachment::UPLOAD_OPERATION},
                doc! {"$set":{"method":"PUT"}},
                doc! {"$set":{"method":"POST"}},
                "untrusted_contract",
            ),
            (
                "downstream_services",
                doc! {"_id":&u.catalog.id},
                doc! {"$set":{"created_by":"custom"}},
                doc! {"$set":{"created_by":"system"}},
                "untrusted_contract",
            ),
            (
                "downstream_services",
                doc! {"_id":&u.catalog.id},
                doc! {"$set":{"tools_manager_created":true}},
                doc! {"$set":{"tools_manager_created":false}},
                "untrusted_contract",
            ),
        ];
        for (collection, filter, change, restore, reason) in cases {
            let service = raw(&u, &auth).await;
            u.f.state
                .db
                .collection::<bson::Document>(collection)
                .update_one(filter.clone(), change)
                .await
                .unwrap();
            assert_parity(
                &u,
                &auth,
                service,
                Some(&format!("attachment_write_{reason}")),
            )
            .await;
            u.f.state
                .db
                .collection::<bson::Document>(collection)
                .update_one(filter, restore)
                .await
                .unwrap();
        }
        let service = raw(&u, &auth).await;
        let row =
            u.f.state
                .db
                .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
                .find_one_and_delete(
                    doc! {"service_id":&u.catalog.id,"name":attachment::CREATE_OPERATION},
                )
                .await
                .unwrap()
                .unwrap();
        assert_parity(
            &u,
            &auth,
            service,
            Some("attachment_write_required_operation_missing"),
        )
        .await;
        u.f.state
            .db
            .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
            .insert_one(row)
            .await
            .unwrap();
        // Clearing the override and restoring authority returns the capability.
        assert_parity(&u, &auth, raw(&u, &auth).await, None).await;
        cleanup(u).await;
    }
}

#[tokio::test]
async fn managed_source_async_and_contract_snapshot_refusals_are_named() {
    let u = second_connection("api-lark").await;
    let auth = authenticate(&u.f).await;
    for node in [false, true] {
        let mut service = raw(&u, &auth).await;
        if let mcp_service::McpToolSource::UserManaged {
            node_id,
            has_server_credential,
            ..
        } = &mut service.source
        {
            if node {
                *node_id = Some("node".into());
            } else {
                *has_server_credential = false;
            }
        }
        assert_parity(
            &u,
            &auth,
            service,
            Some("attachment_write_hosted_connection_required"),
        )
        .await;
    }
    let mut service = raw(&u, &auth).await;
    // The general resolver and executor normalize legacy empty node IDs.
    if let mcp_service::McpToolSource::UserManaged { node_id, .. } = &mut service.source {
        *node_id = Some(String::new());
    }
    assert_parity(&u, &auth, service, None).await;
    let mut service = raw(&u, &auth).await;
    service.source = mcp_service::McpToolSource::Platform {
        downstream_service_id: u.catalog.id.clone(),
    };
    assert_parity(
        &u,
        &auth,
        service,
        Some("attachment_write_catalog_connection_required"),
    )
    .await;
    let mut service = raw(&u, &auth).await;
    service
        .endpoints
        .iter_mut()
        .find(|ep| ep.name == attachment::COMPOSITE_OPERATION)
        .unwrap()
        .response_description = Some("changed snapshot".into());
    // Response description is presentation-only; use a contract-bearing field.
    service
        .endpoints
        .iter_mut()
        .find(|ep| ep.name == attachment::COMPOSITE_OPERATION)
        .unwrap()
        .response
        .binary_artifact = Some(true);
    assert_parity(
        &u,
        &auth,
        service,
        Some("attachment_write_contract_changed"),
    )
    .await;
    let mut service = raw(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter_mut()
        .find(|ep| ep.name == attachment::COMPOSITE_OPERATION)
        .unwrap();
    endpoint.async_operation = Some(serde_json::from_value(json!({"status_operation":"status","result_operation":"result","cancel_operation":"cancel","id_field":"id","id_parameter":"id","status_field":"status","success_states":["done"],"failure_states":["error"],"error_field":null})).unwrap());
    assert_parity(
        &u,
        &auth,
        service,
        Some("attachment_write_async_unsupported"),
    )
    .await;
    cleanup(u).await;
}

#[tokio::test]
async fn legacy_second_connection_empty_fields_uploads_original_bytes_creates_once_and_replays() {
    for slug in SLUGS {
        let u = second_connection(slug).await;
        let auth = authenticate(&u.f).await;
        u.f.state
            .db
            .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
            .update_many(
                doc! {"service_id":&u.catalog.id},
                doc! {"$unset":{"publication":""}},
            )
            .await
            .unwrap();
        let input = json!({"app_token":"base","table_id":"table","fields":{},"attachments":{"Receipt":[u.attachment_id]}});
        let name = format!("{slug}-2__{}", attachment::COMPOSITE_OPERATION);
        for replay in [false, true] {
            let response = rpc(
                &u.f,
                &auth,
                "tools/call",
                Some(json!({"name":name,"arguments":input})),
            )
            .await;
            assert_eq!(response["result"]["isError"], false, "{response}");
            assert_eq!(payload(&response)["record_id"], "rec_receipt");
            assert_eq!(payload(&response)["already_created"], replay);
        }
        let requests = requests(&u).await;
        assert_eq!(requests.len(), if slug.ends_with("-bot") { 4 } else { 3 });
        let upload = requests
            .iter()
            .find(|r| r.url.path().ends_with("upload_all"))
            .unwrap();
        assert!(
            upload
                .body
                .windows(b"receipt-a bytes".len())
                .any(|bytes| bytes == b"receipt-a bytes")
        );
        let create = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records"))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&create.body).unwrap(),
            json!({"fields":{"Receipt":[{"file_token":"file_receipt"}]}})
        );
        assert_eq!(
            ledger(&u).await.state,
            crate::models::lark_base_write::WriteState::Succeeded
        );
        cleanup(u).await;
    }
}

#[tokio::test]
async fn direct_and_meta_calls_name_override_and_unpublished_dependency_then_allow_cleared_override()
 {
    for meta in [false, true] {
        let u = setup("lark_2327_public_errors").await;
        let auth = authenticate(&u.f).await;
        let connection =
            u.f.state
                .db
                .collection::<bson::Document>("user_services")
                .find_one(doc! {"_id":&u.instance})
                .await
                .unwrap()
                .unwrap();
        let mounts = u.f.state.db.collection::<bson::Document>("user_endpoints");
        let mount = connection.get_str("endpoint_id").unwrap();
        let spec = MockServer::start().await;
        Mock::given(path("/spec.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                (*catalog_spec_registry::spec_for_slug("api-lark").unwrap()).clone(),
            ))
            .mount(&spec)
            .await;
        mounts
            .update_one(
                doc! {"_id":mount},
                doc! {"$set":{"openapi_spec_url":format!("{}/spec.json", spec.uri())}},
            )
            .await
            .unwrap();
        let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_error(&response);
        assert!(
            text(&response).contains("attachment_write_untrusted_spec_override"),
            "{response}"
        );
        let listed = rpc(&u.f, &auth, "tools/list", None).await.to_string();
        assert!(!listed.contains("receipts__bitable_record_create_with_attachments"));
        mounts
            .update_one(doc! {"_id":mount}, doc! {"$unset":{"openapi_spec_url":""}})
            .await
            .unwrap();

        let endpoints =
            u.f.state
                .db
                .collection::<bson::Document>(service_endpoint::COLLECTION_NAME);
        let dependency = doc! {"service_id":&u.catalog.id,"name":attachment::CREATE_OPERATION};
        endpoints
            .update_one(dependency.clone(), doc! {"$set":{"publication":"paused"}})
            .await
            .unwrap();
        let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_error(&response);
        assert!(
            text(&response).contains("attachment_write_operation_not_published"),
            "{response}"
        );
        assert!(requests(&u).await.is_empty());
        endpoints
            .update_one(dependency, doc! {"$set":{"publication":"published"}})
            .await
            .unwrap();
        let listed = rpc(&u.f, &auth, "tools/list", None).await.to_string();
        assert!(listed.contains("receipts__bitable_record_create_with_attachments"));
        let mut input = args(&u.attachment_id);
        input["fields"] = json!({});
        let response = call(&u, &auth, meta, input).await;
        assert_eq!(payload(&response)["record_id"], "rec_receipt", "{response}");
        assert_eq!(requests(&u).await.len(), 3);
        cleanup(u).await;
    }
}
