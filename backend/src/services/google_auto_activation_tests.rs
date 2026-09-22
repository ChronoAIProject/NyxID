use super::*;
use base64::Engine;
use futures::TryStreamExt;

#[tokio::test]
async fn google_flagless_fresh_startup_repeats_and_repairs_missing_rows() {
    let db = connect_test_database("google_fresh_auto").await.unwrap();
    seed(&db).await;
    for (slug, count) in [("api-google-drive", 22), ("api-google-workspace", 38)] {
        let services = db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME);
        let service = services
            .find_one(doc! {"slug":slug})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(service.destination_targets, workspace_targets());
        assert_eq!(
            service.proxy_operation_policy,
            Some(
                crate::services::google_workspace::GoogleProduct::from_slug(slug)
                    .unwrap()
                    .operation_policy()
                    .unwrap()
            )
        );
        assert!(service.description.as_deref().unwrap().contains("Slides"));
        let coll =
            db.collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME);
        let rows: Vec<_> = coll
            .find(doc! {"service_id":&service.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), count);
        assert!(rows.iter().all(|row| row.operation_generation == 1));
        let removed = rows
            .iter()
            .find(|row| row.name == "docs_create_document")
            .unwrap();
        coll.delete_one(doc! {"_id":&removed.id}).await.unwrap();
        seed(&db).await;
        let repaired: Vec<_> = coll
            .find(doc! {"service_id":&service.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(repaired.len(), count);
        for row in rows.iter().filter(|row| row.id != removed.id) {
            assert_eq!(
                bson::to_document(row).unwrap(),
                bson::to_document(repaired.iter().find(|new| new.id == row.id).unwrap()).unwrap()
            );
        }
        seed(&db).await;
        for row in repaired {
            assert_eq!(
                bson::to_document(&row).unwrap(),
                bson::to_document(&coll.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap())
                    .unwrap()
            );
        }
    }
}

#[tokio::test]
async fn google_concurrent_sync_inserts_editors_and_updates_old_contracts_once() {
    tokio::time::timeout(std::time::Duration::from_secs(180), async {
        for round in 0..5 {
            let db = connect_test_database("google_concurrent").await.unwrap();
            db.collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
                .create_index(
                    mongodb::IndexModel::builder()
                        .keys(doc! {"service_id":1,"name":1})
                        .options(
                            mongodb::options::IndexOptions::builder()
                                .unique(true)
                                .build(),
                        )
                        .build(),
                )
                .await
                .unwrap();
            seed_legacy(&db).await;
            let coll = db
                .collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME);
            let services = db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME);
            let mut before = Vec::new();
            for slug in ["api-google-drive", "api-google-workspace"] {
                let service = services
                    .find_one(doc! {"slug":slug})
                    .await
                    .unwrap()
                    .unwrap();
                let rows: Vec<service_endpoint::ServiceEndpoint> = coll
                    .find(doc! {"service_id":&service.id})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                before.extend(rows);
            }
            let mut tasks = tokio::task::JoinSet::new();
            for _ in 0..8 {
                let db = db.clone();
                tasks.spawn(async move {
                    provider_service::seed_default_services(&db, &test_encryption_keys()).await?;
                    catalog_spec_sync::sync_seeded_service_endpoints(&db).await
                });
            }
            while let Some(result) = tasks.join_next().await {
                result.unwrap().unwrap();
            }
            for slug in ["api-google-drive", "api-google-workspace"] {
                let service = services
                    .find_one(doc! {"slug":slug})
                    .await
                    .unwrap()
                    .unwrap();
                let rows: Vec<service_endpoint::ServiceEndpoint> = coll
                    .find(doc! {"service_id":&service.id})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                assert_eq!(rows.len(), if slug.ends_with("drive") { 22 } else { 38 });
                let names: std::collections::HashSet<_> =
                    rows.iter().map(|row| &row.name).collect();
                assert_eq!(names.len(), rows.len());
                for row in rows {
                    if let Some(old) = before.iter().find(|old| old.id == row.id) {
                        let changed = GOOGLE_CHANGED_OPERATIONS.contains(&row.name.as_str());
                        assert_eq!(
                            row.operation_generation,
                            old.operation_generation + i64::from(changed),
                            "round {round} {}",
                            row.name
                        );
                        if !changed {
                            assert_eq!(
                                bson::to_document(&row).unwrap(),
                                bson::to_document(old).unwrap()
                            );
                        }
                    } else {
                        assert!(row.target_id.is_some());
                        assert_eq!(row.operation_generation, 1);
                    }
                }
            }
        }
    })
    .await
    .expect("bounded concurrent sync");
}

#[tokio::test]
async fn google_partial_custom_maps_sync_only_available_origins_and_keep_custom_disabled_rows() {
    use tracing::instrument::WithSubscriber;
    let db = connect_test_database("google_partial_maps").await.unwrap();
    seed(&db).await;
    for slug in ["api-google-drive", "api-google-workspace"] {
        let services = db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME);
        let service = services
            .find_one(doc! {"slug":slug})
            .await
            .unwrap()
            .unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let connection = connect(&db, &owner, slug).await;
        let user_rows = db.collection::<bson::Document>(user_service::COLLECTION_NAME);
        let before_connection = user_rows
            .find_one(doc! {"_id":&connection.id})
            .await
            .unwrap()
            .unwrap();

        let credential_rows = db.collection::<bson::Document>(user_api_key::COLLECTION_NAME);
        let endpoint_rows = db.collection::<bson::Document>(user_endpoint::COLLECTION_NAME);
        let before_credential = credential_rows
            .find_one(doc! {"_id":connection.api_key_id.as_ref().unwrap()})
            .await
            .unwrap()
            .unwrap();
        let before_endpoint = endpoint_rows
            .find_one(doc! {"_id":&connection.endpoint_id})
            .await
            .unwrap()
            .unwrap();
        services
            .update_one(
                doc! {"_id":&service.id},
                doc! {"$set":{
                    "destination_targets":{"docs":"https://docs.googleapis.com"},
                    "description":"Operator description", "known_limitations":"Operator limits",
                }},
            )
            .await
            .unwrap();
        let coll =
            db.collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME);
        let disabled = coll
            .find_one(doc! {"service_id":&service.id,"name":"drive_update_file"})
            .await
            .unwrap()
            .unwrap();
        coll.update_one(doc! {"_id":&disabled.id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        let mut custom = disabled.clone();
        custom.id = uuid::Uuid::new_v4().to_string();
        custom.name = "operator_custom".into();
        custom.path = "/custom".into();
        coll.insert_one(&custom).await.unwrap();
        let existing: Vec<_> = coll
            .find(doc! {"service_id":&service.id})
            .await
            .unwrap()
            .try_collect::<Vec<service_endpoint::ServiceEndpoint>>()
            .await
            .unwrap();
        // Force root and mapped-origin repairs, with unavailable editor rows left intact.
        coll.delete_one(doc! {"service_id":&service.id,"name":"drive_list_files"})
            .await
            .unwrap();
        coll.delete_one(doc! {"service_id":&service.id,"name":"docs_get_document"})
            .await
            .unwrap();
        let before_service = services
            .find_one(doc! {"_id":&service.id})
            .await
            .unwrap()
            .unwrap();
        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        async {
            provider_service::seed_default_services(&db, &test_encryption_keys())
                .await
                .unwrap();
            catalog_spec_sync::sync_seeded_service_endpoints(&db)
                .await
                .unwrap();
        }
        .with_subscriber(subscriber)
        .await;
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        assert!(logs.contains("custom destination map retained"));
        let warnings: Vec<_> = logs
            .lines()
            .filter(|line| line.contains(&service.id) && line.contains("skipped_operation_count"))
            .collect();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("skipped_operation_count=10"));
        assert!(warnings[0].contains("sheets"));
        assert!(warnings[0].contains("slides"));
        assert_eq!(
            bson::to_document(&before_service).unwrap(),
            bson::to_document(
                &services
                    .find_one(doc! {"_id":&service.id})
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(
            before_connection,
            user_rows
                .find_one(doc! {"_id":&connection.id})
                .await
                .unwrap()
                .unwrap()
        );

        assert_eq!(
            before_credential,
            credential_rows
                .find_one(doc! {"_id":connection.api_key_id.as_ref().unwrap()})
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            before_endpoint,
            endpoint_rows
                .find_one(doc! {"_id":&connection.endpoint_id})
                .await
                .unwrap()
                .unwrap()
        );
        for old in existing
            .iter()
            .filter(|row| !matches!(row.name.as_str(), "drive_list_files" | "docs_get_document"))
        {
            assert_eq!(
                bson::to_document(old).unwrap(),
                bson::to_document(&coll.find_one(doc! {"_id":&old.id}).await.unwrap().unwrap())
                    .unwrap()
            );
        }
        assert_eq!(
            coll.count_documents(doc! {"service_id":&service.id})
                .await
                .unwrap(),
            if slug.ends_with("drive") { 23 } else { 39 }
        );
    }
}

fn assert_upload_echo(value: &Value, method: &str, query: &str, content_type: &str, body: &[u8]) {
    assert_eq!(value["host"], "www.googleapis.com");
    assert!(value["request"].as_str().unwrap().starts_with(method));
    assert!(value["request"].as_str().unwrap().contains(query));
    let headers = value["headers"].as_str().unwrap();
    assert!(
        headers
            .to_ascii_lowercase()
            .contains(&format!("content-type: {content_type}").to_ascii_lowercase()),
        "{headers}"
    );
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(value["body_base64"].as_str().unwrap())
            .unwrap(),
        body
    );
}

async fn node_upload(
    echo: &Echo,
    slug: &str,
    method: &str,
    path: &str,
    query: &str,
    content_type: &str,
    body: &[u8],
) -> Value {
    use crate::services::node_ws_manager::*;
    let manager = Arc::new(NodeWsManager::new(5, 100));
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    manager.register_connection("upload-node", tx);
    manager.record_capabilities(
        "upload-node",
        &NodeCapabilitiesMsg {
            http_signature_v2: true,
            ..Default::default()
        },
    );
    let client = echo.client.clone();
    let slug_owned = slug.to_owned();
    let downstream = manager.clone();
    let task = tokio::spawn(async move {
        let NodeOutboundMessage::Text(text) = rx.recv().await.unwrap() else {
            panic!("HTTP frame");
        };
        let request: Value = serde_json::from_str(&text).unwrap();
        // Uploads remain root-origin requests and keep the legacy signature shape,
        // even on a v2-capable node; selected editor origins use v2 separately.
        assert!(request.get("target_id").is_none_or(Value::is_null));
        let credentials = nyxid_node_proxy_test::bearer_credentials(
            &slug_owned,
            "https://www.googleapis.com",
            "node-google-token",
        )
        .unwrap();
        let (tx, mut responses) = tokio::sync::mpsc::channel(8);
        nyxid_node_proxy_test::proxy_executor::execute_proxy_request(
            &request,
            &credentials,
            Some(&"11".repeat(32)),
            &tokio::sync::Mutex::new(nyxid_node_proxy_test::ReplayGuard::new()),
            &nyxid_node_proxy_test::NodeMetrics::new(),
            &tx,
            false,
            &client,
        )
        .await;
        let nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(response) =
            responses.recv().await.unwrap()
        else {
            panic!("response frame")
        };
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], 200, "{response}");
        downstream.deliver_proxy_response(
            "upload-node",
            NodeProxyResponse {
                request_id: request["request_id"].as_str().unwrap().into(),
                status: 200,
                headers: vec![],
                body: base64::engine::general_purpose::STANDARD
                    .decode(response["body"].as_str().unwrap())
                    .unwrap(),
            },
        );
    });
    let result = manager
        .send_proxy_request(
            "upload-node",
            NodeProxyRequest {
                target_id: None,
                request_id: uuid::Uuid::new_v4().to_string(),
                service_id: "upload-service".into(),
                service_slug: slug.into(),
                base_url: "https://www.googleapis.com".into(),
                method: method.into(),
                path: path.into(),
                query: Some(query.into()),
                headers: vec![("content-type".into(), content_type.into())],
                body: Some(body.to_vec()),
            },
            Some(&[0x11; 32]),
            permit(crate::services::billing::BillingIngress::Proxy),
        )
        .await
        .unwrap();
    task.await.unwrap();
    let ProxyResponseType::Complete(result) = result else {
        panic!("complete echo response")
    };
    serde_json::from_slice(&result.body).unwrap()
}

#[tokio::test]
async fn google_upload_post_patch_preserve_binary_multipart_and_html_bytes_direct_and_node() {
    let db = connect_test_database("google_upload_bytes").await.unwrap();
    seed(&db).await;
    let echo = Echo::start().await;
    let owner = uuid::Uuid::new_v4().to_string();
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    let spec = crate::services::catalog_spec_registry::spec_for_key("google-drive").unwrap();
    for slug in ["api-google-drive", "api-google-workspace"] {
        connect(&db, &owner, slug).await;
        for (method, path, template) in [
            ("POST", "/upload/drive/v3/files", "/upload/drive/v3/files"),
            (
                "PATCH",
                "/upload/drive/v3/files/file-id",
                "/upload/drive/v3/files/{fileId}",
            ),
        ] {
            let multipart = spec["paths"][template][method.to_lowercase()]["requestBody"]["content"]["multipart/related"]["schema"]["example"].as_str().unwrap().as_bytes();
            let mut binary_multipart = b"--binary_boundary\r\nContent-Type: application/json\r\n\r\n{\"name\":\"binary.dat\"}\r\n--binary_boundary\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
            binary_multipart.extend_from_slice(b"\0\xff\x80binary-part\r\n--binary_boundary--\r\n");
            for (query, content_type, body) in [
                (
                    "uploadType=multipart",
                    "multipart/related; boundary=binary_boundary",
                    binary_multipart.as_slice(),
                ),
                (
                    "uploadType=media",
                    "application/octet-stream",
                    b"\0\xff\x80media\r\n".as_slice(),
                ),
                (
                    "uploadType=multipart",
                    "multipart/related; boundary=nyxid_google_doc",
                    multipart,
                ),
                (
                    "uploadType=media",
                    "text/html; charset=UTF-8",
                    b"<html><body>Replacement</body></html>".as_slice(),
                ),
            ] {
                let mut request = axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/api/v1/proxy/s/{slug}{path}?{query}"))
                    .header("content-type", content_type)
                    .body(axum::body::Body::from(body.to_vec()))
                    .unwrap();
                request.extensions_mut().insert(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::Proxy,
                    ),
                );
                let response = crate::handlers::proxy::proxy_request_by_slug(
                    axum::extract::State(state.clone()),
                    test_auth_user(&owner),
                    Default::default(),
                    axum::extract::Path((slug.into(), path.trim_start_matches('/').into())),
                    request,
                )
                .await
                .unwrap();
                assert_eq!(response.status(), 200);
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                assert_upload_echo(
                    &serde_json::from_slice(&bytes).unwrap(),
                    method,
                    query,
                    content_type,
                    body,
                );
                let captured =
                    node_upload(&echo, slug, method, path, query, content_type, body).await;
                assert_upload_echo(&captured, method, query, content_type, body);
            }
        }
    }
    assert_eq!(echo.calls.lock().unwrap().len(), 32);
}

#[tokio::test]
async fn google_upload_mcp_projection_rejects_before_dispatch_and_keeps_binary_media() {
    let db = connect_test_database("google_mcp_media").await.unwrap();
    seed(&db).await;
    let echo = Echo::start().await;
    let owner = uuid::Uuid::new_v4().to_string();
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    for slug in ["api-google-drive", "api-google-workspace"] {
        connect(&db, &owner, slug).await;
    }
    let catalog = mcp_service::load_operation_catalog(
        &db,
        &state.node_ws_manager,
        &owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    for service in catalog.services.iter().filter(|service| {
        matches!(
            service.service_slug.as_str(),
            "api-google-drive" | "api-google-workspace"
        )
    }) {
        for endpoint in service.endpoints.iter().filter(|endpoint| {
            matches!(
                endpoint.name.as_str(),
                "drive_upload_file" | "drive_upload_file_content"
            )
        }) {
            let payload = b"\0\xff\x80raw-media";
            let mut args = json!({"uploadType":"media","body":base64::engine::general_purpose::STANDARD.encode(payload)});
            if endpoint.method == "PATCH" {
                args["fileId"] = json!("file-id");
            }
            let schema = mcp_service::build_input_schema(endpoint);
            assert_eq!(schema["properties"]["uploadType"]["enum"], json!(["media"]));
            assert_eq!(schema["properties"]["body"]["contentEncoding"], "base64");
            assert_eq!(
                schema["properties"]["body"]["contentMediaType"],
                "application/octet-stream"
            );
            let count = echo.calls.lock().unwrap().len();
            let mut bad_args = args.clone();
            bad_args["uploadType"] = json!("multipart");
            assert!(matches!(
                mcp_call(&state, &owner, service, endpoint, &bad_args).await,
                Err(AppError::BadRequest(_))
            ));
            assert_eq!(echo.calls.lock().unwrap().len(), count);
            for malformed in [
                json!({"name":"uploadType","in":"query","schema":{"enum":["media","multipart"]},"x-nyxid-mcp-enum":["media"]}),
                json!({"name":"uploadType","in":"header","schema":{"enum":["media"]},"x-nyxid-mcp-enum":["media"]}),
                json!({"name":"","in":"query","schema":{"enum":["media"]},"x-nyxid-mcp-enum":["media"]}),
                json!({"name":"uploadType","in":"query","schema":{"enum":["media"]},"x-nyxid-mcp-enum":null}),
                json!({"name":"uploadType","in":"query","schema":{"enum":["media"],"x-nyxid-mcp-enum":["media"]}}),
            ] {
                let mut bad = copy_upload_endpoint(endpoint);
                bad.parameters = Some(json!([malformed]));
                assert!(matches!(
                    mcp_service::build_proxy_args(&bad, &json!({})),
                    Err(AppError::BadRequest(_))
                ));
            }
            let mut blocked = copy_upload_endpoint(endpoint);
            blocked
                .parameters
                .as_mut()
                .unwrap()
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"Content-Type","in":"header","schema":{"type":"string"}}));
            bad_args = args.clone();
            bad_args["Content-Type"] = json!("multipart/related; boundary=evil");
            assert!(matches!(
                mcp_service::build_proxy_args(&blocked, &bad_args),
                Err(AppError::BadRequest(_))
            ));
            let (status, response) = mcp_call(&state, &owner, service, endpoint, &args)
                .await
                .unwrap();
            assert_eq!(status, 200);
            assert_upload_echo(
                &serde_json::from_str(&response).unwrap(),
                &endpoint.method,
                "uploadType=media",
                "application/octet-stream",
                payload,
            );
        }
    }
    assert_eq!(echo.calls.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn google_twelve_contract_digest_generation_changes_match_reviewed_inventory() {
    let db = connect_test_database("google_contract_inventory")
        .await
        .unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../specs/fixtures/google-auto-activation-contract-changes.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../specs/fixtures/google-drive-before-auto-activation.json"
    ))
    .unwrap();
    let coll =
        db.collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME);
    for slug in ["api-google-drive", "api-google-workspace"] {
        let pins: Vec<_> = fixture["operations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|pin| pin["slug"] == slug)
            .collect();
        let service_id = pins[0]["service_id"].as_str().unwrap();
        restore_historical_drive_endpoints(&db, service_id, &old).await;
        for pin in &pins {
            let mut row = coll
                .find_one(doc! {"service_id":service_id,"name":pin["operation"].as_str().unwrap()})
                .await
                .unwrap()
                .unwrap();
            coll.delete_one(doc! {"_id":&row.id}).await.unwrap();
            row.id = pin["endpoint_id"].as_str().unwrap().into();
            row.operation_generation = pin["before_generation"].as_i64().unwrap();
            assert_eq!(
                durable_operation_grant_service::endpoint_contract_digest(&row).unwrap(),
                pin["before_digest"]
            );
            coll.insert_one(row).await.unwrap();
        }
        let current: Value = serde_json::from_str(include_str!(
            "../../specs/catalog/google-drive.openapi.json"
        ))
        .unwrap();
        restore_historical_drive_endpoints(&db, service_id, &current).await;
        for pin in &pins {
            let row = coll
                .find_one(doc! {"_id":pin["endpoint_id"].as_str().unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.operation_generation,
                pin["after_generation"].as_i64().unwrap()
            );
            assert_eq!(
                durable_operation_grant_service::endpoint_contract_digest(&row).unwrap(),
                pin["after_digest"]
            );
        }
        restore_historical_drive_endpoints(&db, service_id, &current).await;
        for pin in pins {
            assert_eq!(
                coll.find_one(doc! {"_id":pin["endpoint_id"].as_str().unwrap()})
                    .await
                    .unwrap()
                    .unwrap()
                    .operation_generation,
                8
            );
        }
    }
}

#[tokio::test]
async fn google_service_disabled_403_passes_through_rest_and_mcp_distinct_from_activation() {
    let db = connect_test_database("google_disabled_api").await.unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let echo = Echo::start().await;
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    for slug in ["api-google-drive", "api-google-workspace"] {
        connect(&db, &owner, slug).await;
    }
    let catalog = mcp_service::load_operation_catalog(
        &db,
        &state.node_ws_manager,
        &owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(echo.client_builder.clone(), async {
            for service in catalog.services.iter().filter(|s| {
                matches!(
                    s.service_slug.as_str(),
                    "api-google-drive" | "api-google-workspace"
                )
            }) {
                let response = rest_call(
                    &state,
                    &owner,
                    &service.service_slug,
                    "/v1/documents/service-disabled:batchUpdate",
                    false,
                )
                .await
                .unwrap();
                assert_eq!(response.status(), 403);
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                let rest: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(rest["error"]["details"][0]["reason"], "SERVICE_DISABLED");
                let endpoint = service
                    .endpoints
                    .iter()
                    .find(|e| e.name == "docs_batch_update_document")
                    .unwrap();
                let (status, body) = mcp_call(
                    &state,
                    &owner,
                    service,
                    endpoint,
                    &json!({"documentId":"service-disabled","requests":[]}),
                )
                .await
                .unwrap();
                assert_eq!(status, 403);
                assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), rest);
                assert!(!body.contains("12300"));
            }
        })
        .await;
    assert_eq!(echo.calls.lock().unwrap().len(), 4);
}

fn copy_upload_endpoint(endpoint: &mcp_service::McpToolEndpoint) -> mcp_service::McpToolEndpoint {
    mcp_service::McpToolEndpoint {
        name: endpoint.name.clone(),
        method: endpoint.method.clone(),
        path: endpoint.path.clone(),
        parameters: endpoint.parameters.clone(),
        request_body_schema: endpoint.request_body_schema.clone(),
        request_content_type: endpoint.request_content_type.clone(),
        request_body_required: endpoint.request_body_required,
        ..Default::default()
    }
}

#[tokio::test]
async fn google_durable_corrected_grant_drifts_unchanged_grant_survives_and_reapproval_works() {
    use crate::models::durable_operation_grant::{
        DurableBodyConstraint, DurableOperationConstraints, DurableOperationSelection,
        DurableParameterConstraint, DurableReplayPolicy, DurableValueConstraint,
    };
    use crate::services::api_key_scope_service;
    let db = connect_test_database("google_durable_drift").await.unwrap();
    seed(&db).await;
    let old: Value = serde_json::from_str(include_str!(
        "../../specs/fixtures/google-drive-before-auto-activation.json"
    ))
    .unwrap();
    let manager = crate::services::node_ws_manager::NodeWsManager::new(30, 100);
    for slug in ["api-google-drive", "api-google-workspace"] {
        let owner = test_user(
            &uuid::Uuid::new_v4().to_string(),
            crate::models::user::UserType::Person,
        );
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(&owner)
            .await
            .unwrap();
        let service = connect(&db, &owner.id, slug).await;
        restore_historical_drive_endpoints(
            &db,
            service.catalog_service_id.as_deref().unwrap(),
            &old,
        )
        .await;
        let coll =
            db.collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME);
        let expiry =
            chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp() + 7200, 0).unwrap();
        let mut selections = Vec::new();
        for name in ["drive_create_file", "docs_create_document"] {
            let body = if name == "drive_create_file" {
                json!({"name":"approved-test-file"})
            } else {
                json!({"title":"approved-test-document"})
            };
            let row = coll
                .find_one(
                    doc! {"service_id":service.catalog_service_id.as_ref().unwrap(),"name":name},
                )
                .await
                .unwrap()
                .unwrap();
            selections.push(DurableOperationSelection {
                user_service_id: service.id.clone(),
                endpoint_id: row.id,
                constraints: DurableOperationConstraints {
                    body: Some(DurableBodyConstraint {
                        fields: std::collections::BTreeMap::from([(
                            "".into(),
                            DurableParameterConstraint {
                                required: true,
                                rule: DurableValueConstraint::Exact {
                                    value: body.clone(),
                                },
                            },
                        )]),
                        allow_additional_fields: false,
                    }),
                    ..Default::default()
                },
                valid_from: (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(),
                expires_at: (expiry - chrono::Duration::minutes(1)).to_rfc3339(),
                total_limit: 10,
                window: None,
                replay_policy: DurableReplayPolicy::NonReplayable,
                client_audit_binding: None,
            });
        }
        let service_ids = vec![service.id.clone()];
        let plan = api_key_scope_service::build_scope_plan_with_operations(
            &db,
            &manager,
            &owner.id,
            None,
            &service_ids,
            &selections,
            Some(expiry),
            false,
        )
        .await
        .unwrap();
        let provisioned = durable_operation_grant_service::provision_scheduled_key(
            &db,
            &manager,
            &owner.id,
            &owner.id,
            "Google test grant",
            expiry,
            None,
            &service_ids,
            &plan.allowed_node_ids,
            None,
            None,
            None,
            &selections,
            &plan.normalized_grant_digest,
        )
        .await
        .unwrap();
        catalog_spec_sync::sync_seeded_service_endpoints(&db)
            .await
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
        for grant in &provisioned.grants {
            let body = if grant.normalized_path_template == "/drive/v3/files" {
                json!({"name":"approved-test-file"})
            } else {
                json!({"title":"approved-test-document"})
            };
            let result = durable_operation_grant_service::authorize_and_reserve(
                &db,
                &manager,
                &owner.id,
                &provisioned.key.id,
                &service.id,
                "POST",
                &grant.normalized_path_template,
                None,
                &headers,
                body.to_string().as_bytes(),
                &grant.id,
                &uuid::Uuid::new_v4().to_string(),
                false,
            )
            .await;
            if grant.normalized_path_template == "/drive/v3/files" {
                assert!(matches!(result, Err(AppError::DurableGrantContractDrift)));
            } else {
                assert!(result.is_ok(), "{result:?}");
            }
        }
        let new_plan = api_key_scope_service::build_scope_plan_with_operations(
            &db,
            &manager,
            &owner.id,
            None,
            &service_ids,
            &selections,
            Some(expiry),
            false,
        )
        .await
        .unwrap();
        assert_ne!(
            plan.normalized_grant_digest,
            new_plan.normalized_grant_digest
        );
        let renewed = durable_operation_grant_service::reauthorize_scheduled_key(
            &db,
            &manager,
            &owner.id,
            &owner.id,
            &provisioned.key.id,
            &selections,
            &new_plan.normalized_grant_digest,
        )
        .await
        .unwrap();
        for grant in renewed {
            let body = if grant.normalized_path_template == "/drive/v3/files" {
                json!({"name":"approved-test-file"})
            } else {
                json!({"title":"approved-test-document"})
            };
            durable_operation_grant_service::authorize_and_reserve(
                &db,
                &manager,
                &owner.id,
                &provisioned.key.id,
                &service.id,
                "POST",
                &grant.normalized_path_template,
                None,
                &headers,
                body.to_string().as_bytes(),
                &grant.id,
                &uuid::Uuid::new_v4().to_string(),
                false,
            )
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn google_incompatible_requirements_and_custom_policies_remain_unchanged() {
    use crate::models::service_provider_requirement;
    use tracing::instrument::WithSubscriber;
    for slug in ["api-google-drive", "api-google-workspace"] {
        for case in [
            "injection",
            "provider",
            "policy",
            "service_type",
            "auth_method",
            "created_by",
        ] {
            let db = connect_test_database("google_custom_auth").await.unwrap();
            seed_legacy(&db).await;
            let services = db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME);
            let mut service = services
                .find_one(doc! {"slug":slug})
                .await
                .unwrap()
                .unwrap();
            let requirements =
                db.collection::<bson::Document>(service_provider_requirement::COLLECTION_NAME);
            match case {
                "injection" => {
                    requirements.update_one(doc! {"service_id":&service.id},doc! {"$set":{"injection_method":"header","injection_key":"custom-key"}}).await.unwrap();
                }
                "provider" => {
                    requirements
                        .update_one(
                            doc! {"service_id":&service.id},
                            doc! {"$set":{"provider_config_id":"operator-provider"}},
                        )
                        .await
                        .unwrap();
                }
                "policy" => {
                    service.proxy_operation_policy.as_mut().unwrap().rules.pop();
                }
                "service_type" => {
                    service.service_type = "grpc".into();
                }
                "auth_method" => {
                    service.auth_method = "header".into();
                }
                "created_by" => {
                    service.created_by = "operator".into();
                }
                _ => unreachable!(),
            }
            services
                .replace_one(doc! {"_id":&service.id}, &service)
                .await
                .unwrap();
            let before_requirement = requirements
                .find_one(doc! {"service_id":&service.id})
                .await
                .unwrap()
                .unwrap();
            let capture = tempfile::NamedTempFile::new().unwrap();
            let writer = capture.reopen().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .without_time()
                .with_writer(move || writer.try_clone().unwrap())
                .finish();
            provider_service::seed_default_services(&db, &test_encryption_keys())
                .with_subscriber(subscriber)
                .await
                .unwrap();
            let after = services
                .find_one(doc! {"_id":&service.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                bson::to_document(&after).unwrap(),
                bson::to_document(&service).unwrap(),
                "{slug} {case}"
            );
            assert_eq!(
                requirements
                    .find_one(doc! {"service_id":&service.id})
                    .await
                    .unwrap()
                    .unwrap(),
                before_requirement
            );
            if case != "created_by" {
                assert!(
                    std::fs::read_to_string(capture.path())
                        .unwrap()
                        .contains("Google editor destinations were not activated"),
                    "{case}"
                );
            }
        }
    }
}

#[tokio::test]
async fn google_generic_mcp_cannot_set_multipart_content_type() {
    let db = connect_test_database("google_generic_json").await.unwrap();
    seed(&db).await;
    let echo = Echo::start().await;
    let owner = uuid::Uuid::new_v4().to_string();
    connect(&db, &owner, "api-google-drive").await;
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    let mut catalog = mcp_service::load_operation_catalog(
        &db,
        &state.node_ws_manager,
        &owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    let service = catalog
        .services
        .iter_mut()
        .find(|s| s.service_slug == "api-google-drive")
        .unwrap();
    service.is_generic_proxy = true;
    let endpoint = mcp_service::McpToolEndpoint {
        endpoint_id: mcp_service::GENERIC_PROXY_ENDPOINT_ID.into(),
        ..Default::default()
    };
    let mime = "--fake\r\nContent-Type: text/html\r\n\r\nHello\r\n--fake--\r\n";
    let (status,response)=mcp_call(&state,&owner,service,&endpoint,&json!({"method":"POST","path":"/upload/drive/v3/files","query":"uploadType=multipart","body":mime,"headers":{"Content-Type":"multipart/related; boundary=fake"}})).await.unwrap();
    assert_eq!(status, 200);
    let captured: Value = serde_json::from_str(&response).unwrap();
    assert!(
        captured["headers"]
            .as_str()
            .unwrap()
            .contains("content-type: application/json")
    );
    let body = base64::engine::general_purpose::STANDARD
        .decode(captured["body_base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(body, mime.as_bytes());
    assert!(
        !captured["headers"]
            .as_str()
            .unwrap()
            .contains("content-type: multipart/related")
    );
}

#[tokio::test]
async fn google_reconciliation_propagates_requirement_read_errors_and_recovers_after_repair() {
    use crate::models::service_provider_requirement;
    let db = connect_test_database("google_reconcile_error")
        .await
        .unwrap();
    seed_legacy(&db).await;
    let services = db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME);
    let service = services
        .find_one(doc! {"slug":"api-google-drive"})
        .await
        .unwrap()
        .unwrap();
    let requirements =
        db.collection::<bson::Document>(service_provider_requirement::COLLECTION_NAME);
    // Deserialization is a MongoDB read error, not a customized auth choice.
    requirements
        .update_one(
            doc! {"service_id":&service.id},
            doc! {"$set":{"injection_method":42}},
        )
        .await
        .unwrap();
    assert!(matches!(
        provider_service::seed_default_services(&db, &test_encryption_keys()).await,
        Err(AppError::DatabaseError(_))
    ));
    assert!(
        services
            .find_one(doc! {"_id":&service.id})
            .await
            .unwrap()
            .unwrap()
            .destination_targets
            .is_empty()
    );
    requirements
        .update_one(
            doc! {"service_id":&service.id},
            doc! {"$set":{"injection_method":"bearer"}},
        )
        .await
        .unwrap();
    seed(&db).await;
    assert_eq!(
        services
            .find_one(doc! {"_id":&service.id})
            .await
            .unwrap()
            .unwrap()
            .destination_targets,
        workspace_targets()
    );
}
