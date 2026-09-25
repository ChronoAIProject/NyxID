use super::*;
use crate::models::{
    downstream_service, service_endpoint, user_api_key, user_endpoint, user_service,
};
use crate::services::{
    catalog_spec_sync, durable_operation_grant_service, mcp_service, provider_service,
};
use crate::test_utils::*;
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// HTTPS runs locally through CONNECT while URLs, Host, SNI, and certificate
/// verification retain the actual provider names. No external provider is called.
pub(crate) struct Echo {
    pub client: reqwest::Client,
    pub client_builder: Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync>,
    pub calls: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Echo {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Echo {
    pub async fn start() -> Self {
        let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key = rcgen::generate_simple_self_signed(vec![
            "docs.googleapis.com".into(),
            "sheets.googleapis.com".into(),
            "slides.googleapis.com".into(),
            "www.googleapis.com".into(),
        ])
        .unwrap();
        let cert = reqwest::Certificate::from_pem(key.cert.pem().as_bytes()).unwrap();
        let config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![key.cert.der().clone()],
                tokio_rustls::rustls::pki_types::PrivateKeyDer::Pkcs8(
                    key.signing_key.serialize_der().into(),
                ),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        let client_builder: Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync> =
            Arc::new(move || {
                reqwest::Client::builder()
                    .proxy(reqwest::Proxy::all(&proxy_url).unwrap())
                    .add_root_certificate(cert.clone())
            });
        let client = client_builder().build().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                let calls = captured.clone();
                tokio::spawn(async move {
                    let mut connect = Vec::new();
                    while !connect.ends_with(b"\r\n\r\n") {
                        connect.push(stream.read_u8().await.unwrap());
                    }
                    assert!(connect.starts_with(b"CONNECT "));
                    stream
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .unwrap();
                    let mut tls = acceptor.accept(stream).await.unwrap();
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") {
                        headers.push(tls.read_u8().await.unwrap());
                    }
                    let headers = String::from_utf8(headers).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    tls.read_exact(&mut body).await.unwrap();
                    let host = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("host:")
                                .map(|value| value.trim().to_string())
                        })
                        .unwrap();
                    let value = json!({"host": host, "request": headers.lines().next().unwrap(), "headers": headers, "body": String::from_utf8_lossy(&body), "body_base64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body)});
                    let redirect = value["request"].as_str().unwrap().contains("/redirect:");
                    calls.lock().unwrap().push(value.clone());
                    let disabled = value["request"]
                        .as_str()
                        .unwrap()
                        .contains("service-disabled");
                    let body = if disabled {
                        json!({"error":{"code":403,"status":"PERMISSION_DENIED","details":[{"reason":"SERVICE_DISABLED","metadata":{"service":"docs.googleapis.com","consumer":"projects/test-project"}}]}}).to_string()
                    } else {
                        value.to_string()
                    };
                    let status = if disabled {
                        "403 Forbidden"
                    } else if redirect {
                        "302 Found\r\nLocation: https://sheets.googleapis.com/redirected"
                    } else {
                        "200 OK"
                    };
                    tls.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    tls.shutdown().await.unwrap();
                });
            }
        });
        Self {
            client,
            client_builder,
            calls,
            task,
        }
    }
}

pub(crate) const GOOGLE_CHANGED_OPERATIONS: &[&str] = &[
    "drive_get_file",
    "drive_create_file",
    "drive_update_file",
    "drive_copy_file",
    "drive_upload_file",
    "drive_upload_file_content",
];

pub(crate) async fn seed(db: &mongodb::Database) {
    let keys = test_encryption_keys();
    provider_service::seed_default_providers(db, &keys)
        .await
        .unwrap();
    provider_service::seed_default_services(db, &keys)
        .await
        .unwrap();
    catalog_spec_sync::sync_seeded_service_endpoints(db)
        .await
        .unwrap();
}

pub(crate) async fn seed_legacy(db: &mongodb::Database) {
    let keys = test_encryption_keys();
    provider_service::seed_default_providers(db, &keys)
        .await
        .unwrap();
    provider_service::seed_default_services(db, &keys)
        .await
        .unwrap();
    for slug in ["api-google-drive", "api-google-workspace"] {
        let product = super::super::google_workspace::GoogleProduct::from_slug(slug).unwrap();
        let (description, limitations) = if slug == "api-google-drive" {
            (
                "Read, upload, create, edit, export, and delete Google Drive files and folders.",
                "API access is limited to the published Drive operations. Native Docs/Sheets document editing requires their separate APIs. Google may revoke sibling connections using the same account and client together.",
            )
        } else {
            (
                "Google Drive files and folders, calendars, events, availability, and Gmail read/send access.",
                "Workspace bundles Drive, Calendar, and Gmail read/send access. Gmail deletion, trash, mailbox changes, and draft management are not supported. Docs editing, Sheets editing, and Workspace administration are not included. API access is limited to the published operations. Google may revoke sibling connections using the same account and client together.",
            )
        };

        db.collection::<bson::Document>(downstream_service::COLLECTION_NAME)
            .update_one(doc! {"slug": slug}, doc! {"$set": {
                "proxy_operation_policy": bson::to_bson(&product.legacy_operation_policy().unwrap()).unwrap(),
                "destination_targets": {}, "description":description, "known_limitations":limitations,
            }}).await.unwrap();
    }
    catalog_spec_sync::sync_seeded_service_endpoints(db)
        .await
        .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../specs/fixtures/google-drive-before-auto-activation.json"
    ))
    .unwrap();
    for slug in ["api-google-drive", "api-google-workspace"] {
        let service = db
            .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
            .find_one(doc! {"slug": slug})
            .await
            .unwrap()
            .unwrap();
        restore_historical_drive_endpoints(db, &service.id, &old).await;
    }
}

pub(crate) async fn restore_historical_drive_endpoints(
    db: &mongodb::Database,
    service_id: &str,
    old: &Value,
) {
    let inputs = crate::services::openapi_parser::parse_openapi_spec_value(old)
        .unwrap()
        .into_iter()
        .map(
            |endpoint| crate::services::service_endpoint_service::EndpointInput {
                name: endpoint.name,
                description: endpoint.description,
                method: endpoint.method,
                path: endpoint.path,
                target_id: None,
                parameters: endpoint.parameters,
                request_body_schema: endpoint.request_body_schema,
                request_content_type: endpoint.request_content_type,
                request_body_required: endpoint.request_body_required,
                response_description: None,
                response: endpoint.response,
                risk: endpoint.risk,
                supports_idempotency_key: endpoint.supports_idempotency_key,
            },
        )
        .collect();
    crate::services::service_endpoint_service::upsert_endpoints_additive(db, service_id, inputs)
        .await
        .unwrap();
}

pub(crate) async fn connect(
    db: &mongodb::Database,
    owner: &str,
    slug: &str,
) -> user_service::UserService {
    let catalog = db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .find_one(doc! {"slug": slug})
        .await
        .unwrap()
        .unwrap();
    let endpoint = test_user_endpoint(
        &uuid::Uuid::new_v4().to_string(),
        owner,
        "Google",
        &catalog.base_url,
        None,
        Some(&catalog.id),
    );
    db.collection::<user_endpoint::UserEndpoint>(user_endpoint::COLLECTION_NAME)
        .insert_one(&endpoint)
        .await
        .unwrap();
    let encrypted = test_encryption_keys()
        .encrypt(b"test-google-token")
        .await
        .unwrap();
    let key: user_api_key::UserApiKey = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": owner, "label": "Google", "credential_type": "oauth2", "status": "active", "access_token_encrypted": bson::Binary{ subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted }, "provider_config_id": &catalog.provider_config_id,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).unwrap();
    db.collection::<user_api_key::UserApiKey>(user_api_key::COLLECTION_NAME)
        .insert_one(&key)
        .await
        .unwrap();
    let mut service = test_user_service(
        &uuid::Uuid::new_v4().to_string(),
        owner,
        slug,
        &endpoint.id,
        Some(&catalog.id),
        None,
    );
    service.auth_method = "bearer".into();
    service.auth_key_name = "Authorization".into();
    service.api_key_id = Some(key.id);
    db.collection::<user_service::UserService>(user_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    service
}

#[tokio::test]
async fn drive_and_workspace_activation_changes_only_six_contracts_and_preserves_other_rows() {
    let db = connect_test_database("workspace_activation").await.unwrap();
    seed_legacy(&db).await;
    let services = db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME);
    let legacy_drive = services
        .find_one(doc! {"slug": "api-google-drive"})
        .await
        .unwrap()
        .unwrap();
    for (slug, old_count, new_count) in [
        ("api-google-workspace", 25, 38),
        ("api-google-drive", 9, 22),
    ] {
        // Workspace has already activated when Drive reaches this iteration.
        if slug == "api-google-drive" {
            services
                .replace_one(doc! {"_id": &legacy_drive.id}, &legacy_drive)
                .await
                .unwrap();
            db.collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
                .delete_many(
                    doc! {"service_id": &legacy_drive.id, "target_id": {"$type": "string"}},
                )
                .await
                .unwrap();
            let historical: Value = serde_json::from_str(include_str!(
                "../../specs/fixtures/google-drive-before-auto-activation.json"
            ))
            .unwrap();
            restore_historical_drive_endpoints(&db, &legacy_drive.id, &historical).await;
            let workspace = services
                .find_one(doc! {"slug": "api-google-workspace"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(workspace.destination_targets, workspace_targets());
        }
        let service = db
            .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
            .find_one(doc! {"slug": slug})
            .await
            .unwrap()
            .unwrap();
        assert!(workspace_destinations_pending(&service));
        let before = crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
            .await
            .unwrap();
        assert_eq!(before.len(), old_count);
        seed(&db).await;
        let after = crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
            .await
            .unwrap();
        assert_eq!(after.len(), new_count);
        for endpoint in after.iter().filter(|row| row.target_id.is_some()) {
            let expected = if endpoint.name.starts_with("docs_") {
                "docs"
            } else if endpoint.name.starts_with("sheets_") {
                "sheets"
            } else {
                "slides"
            };
            assert_eq!(endpoint.target_id.as_deref(), Some(expected));
        }
        let active = db
            .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
            .find_one(doc! {"_id": &service.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.destination_targets, workspace_targets());
        assert!(!workspace_destinations_pending(&active));
        for old in before {
            let new = after.iter().find(|row| row.id == old.id).unwrap();
            if GOOGLE_CHANGED_OPERATIONS.contains(&old.name.as_str()) {
                assert_eq!(
                    new.operation_generation,
                    old.operation_generation + 1,
                    "{}",
                    old.name
                );
                assert_ne!(
                    durable_operation_grant_service::endpoint_contract_digest(new).unwrap(),
                    durable_operation_grant_service::endpoint_contract_digest(&old).unwrap(),
                    "{}",
                    old.name
                );
                continue;
            }
            assert_eq!(
                new.operation_generation, old.operation_generation,
                "{}",
                old.name
            );
            assert_eq!(
                durable_operation_grant_service::endpoint_contract_digest(new).unwrap(),
                durable_operation_grant_service::endpoint_contract_digest(&old).unwrap(),
                "{}",
                old.name
            );
            assert_eq!(
                bson::to_document(new).unwrap(),
                bson::to_document(&old).unwrap(),
                "untouched row {}",
                old.name
            );
        }
        seed(&db).await;
        assert_eq!(
            crate::services::service_endpoint_service::list_endpoints(&db, &service.id)
                .await
                .unwrap()
                .len(),
            new_count
        );
    }
}

#[path = "test_fixtures/pre_multi_origin_endpoint_writer.rs"]
mod old_writer;

#[tokio::test]
async fn workspace_old_binary_upsert_preserves_top_level_target_while_replacing_parameters() {
    let db = connect_test_database("workspace_old_writer").await.unwrap();
    seed(&db).await;
    let row = db
        .collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .find_one(doc! {"name": "docs_batch_update_document", "target_id": "docs"})
        .await
        .unwrap()
        .unwrap();
    let old: old_writer::model::ServiceEndpoint =
        bson::from_document(bson::to_document(&row).unwrap()).unwrap();
    let input = old_writer::EndpointInput {
        name: old.name,
        description: Some("force old writer update".into()),
        method: old.method,
        path: old.path,
        parameters: Some(json!({"old_shape": true})),
        request_body_schema: old.request_body_schema,
        request_content_type: old.request_content_type,
        request_body_required: old.request_body_required,
        response_description: old.response_description,
        response: old.response,
        risk: old.risk,
        supports_idempotency_key: old.supports_idempotency_key,
    };
    old_writer::upsert_one_endpoint(
        &db.collection(service_endpoint::COLLECTION_NAME),
        &row.service_id,
        input,
        chrono::Utc::now(),
        old_writer::EndpointSyncActivation::PreserveExisting,
    )
    .await
    .unwrap();
    let updated = db
        .collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.target_id.as_deref(), Some("docs"));
    assert_eq!(updated.parameters, Some(json!({"old_shape": true})));
}

#[test]
fn workspace_target_map_validation_is_exact_and_bearer_only() {
    for origin in [
        "http://docs.googleapis.com",
        "https://evil.googleapis.com",
        "https://docs.googleapis.com.evil.test",
        "https://docs.googleapis.com:444",
        "https://user:pass@docs.googleapis.com",
        "https://docs.googleapis.com/path",
        "https://docs.googleapis.com?key=secret",
        "https://*.googleapis.com",
    ] {
        assert!(
            normalize_targets(
                "api-google-workspace",
                "bearer",
                "http",
                [("docs".into(), origin.into())].into(),
                Some(&ProxyOperationPolicy { rules: vec![] })
            )
            .is_err(),
            "{origin}"
        );
    }
    assert!(
        normalize_targets(
            "api-google-workspace",
            "token_exchange",
            "http",
            workspace_targets(),
            Some(&ProxyOperationPolicy { rules: vec![] })
        )
        .is_err()
    );
    assert_eq!(
        normalize_origin("https://DOCS.googleapis.com:443/").unwrap(),
        "https://docs.googleapis.com"
    );
}

fn permit(
    ingress: crate::services::billing::BillingIngress,
) -> crate::services::billing::route_inventory::BillingEgressPermit {
    use crate::services::billing::route_inventory::*;
    enforce_billing_egress_classification(Some(BillingRoutePolicy::Metered(ingress)), ingress)
        .unwrap()
}

pub(crate) async fn mcp_call(
    state: &crate::AppState,
    owner: &str,
    service: &mcp_service::McpToolService,
    endpoint: &mcp_service::McpToolEndpoint,
    args: &Value,
) -> AppResult<(u16, String)> {
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, args)?;
    mcp_service::execute_tool(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        owner,
        owner,
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &mcp_service::McpExecContext {
            api_key_id: None,
            allow_all_nodes: true,
            allowed_node_ids: &[],
        },
        permit(crate::services::billing::BillingIngress::Mcp),
    )
    .await
}

pub(crate) async fn rest_call(
    state: &crate::AppState,
    owner: &str,
    slug: &str,
    path: &str,
    websocket: bool,
) -> AppResult<axum::response::Response> {
    let mut request = axum::http::Request::builder()
        .method(if websocket { "GET" } else { "POST" })
        .uri(format!("/api/v1/proxy/s/{slug}{path}"));
    if websocket {
        request = request
            .header("connection", "upgrade")
            .header("upgrade", "websocket");
    }
    let mut request = request
        .header("content-type", "application/json")
        .body(axum::body::Body::from(r#"{"requests":[]}"#))
        .unwrap();
    request.extensions_mut().insert(
        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        ),
    );
    crate::handlers::proxy::proxy_request_by_slug(
        axum::extract::State(state.clone()),
        test_auth_user(owner),
        Default::default(),
        axum::extract::Path((slug.into(), path.trim_start_matches('/').into())),
        request,
    )
    .await
}

#[tokio::test]
async fn drive_and_workspace_route_all_editors_over_rest_typed_and_generic_mcp() {
    let db = connect_test_database("drive_workspace_editors")
        .await
        .unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    for slug in ["api-google-drive", "api-google-workspace"] {
        connect(&db, &owner, slug).await;
    }
    let echo = Echo::start().await;
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
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(echo.client_builder.clone(), async {
            for slug in ["api-google-drive", "api-google-workspace"] {
                let service = catalog
                    .services
                    .iter_mut()
                    .find(|s| s.service_slug == slug)
                    .unwrap();
                for (operation, path, parameter, host) in [
                    (
                        "docs_batch_update_document",
                        "/v1/documents/editor-id:batchUpdate",
                        "documentId",
                        "docs.googleapis.com",
                    ),
                    (
                        "sheets_batch_update_spreadsheet",
                        "/v4/spreadsheets/editor-id:batchUpdate",
                        "spreadsheetId",
                        "sheets.googleapis.com",
                    ),
                    (
                        "slides_batch_update_presentation",
                        "/v1/presentations/editor-id:batchUpdate",
                        "presentationId",
                        "slides.googleapis.com",
                    ),
                ] {
                    let response = rest_call(&state, &owner, slug, path, false).await.unwrap();
                    assert_eq!(response.status(), 200);
                    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    assert_eq!(
                        serde_json::from_slice::<Value>(&bytes).unwrap()["host"],
                        host
                    );
                    let endpoint = service
                        .endpoints
                        .iter()
                        .find(|e| e.name == operation)
                        .unwrap();
                    let (status, body) = mcp_call(
                        &state,
                        &owner,
                        service,
                        endpoint,
                        &json!({parameter: "editor-id", "requests": []}),
                    )
                    .await
                    .unwrap();
                    assert_eq!(status, 200);
                    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["host"], host);
                    let index = service
                        .endpoints
                        .iter()
                        .position(|e| e.name == operation)
                        .unwrap();
                    let mut generic_endpoint = service.endpoints.remove(index);
                    service.is_generic_proxy = true;
                    generic_endpoint.endpoint_id = mcp_service::GENERIC_PROXY_ENDPOINT_ID.into();
                    generic_endpoint.target_id = None;
                    let (status, body) = mcp_call(
                        &state,
                        &owner,
                        service,
                        &generic_endpoint,
                        &json!({"method": "POST", "path": path, "body": {"requests": []}}),
                    )
                    .await
                    .unwrap();
                    assert_eq!(status, 200);
                    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["host"], host);
                    service.is_generic_proxy = false;
                }
            }
        })
        .await;
    let calls = echo.calls.lock().unwrap();
    assert_eq!(calls.len(), 18);
    for call in calls.iter() {
        assert!(
            call["headers"]
                .as_str()
                .unwrap()
                .contains("Bearer test-google-token")
        );
        assert_eq!(call["body"], r#"{"requests":[]}"#);
    }
}

#[tokio::test]
async fn workspace_rest_typed_generic_mcp_and_product_service_reach_real_host_without_redirects() {
    let db = connect_test_database("workspace_echo").await.unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    connect(&db, &owner, "api-google-workspace").await;
    connect(&db, &owner, "api-google-docs").await;
    let echo = Echo::start().await;
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER.scope(echo.client_builder.clone(), async {
        let response = rest_call(&state,&owner,"api-google-workspace","/v1/documents/document-1:batchUpdate",false).await.unwrap();
        assert_eq!(response.status(),200);
        let response = axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&response).unwrap()["host"],"docs.googleapis.com");
        let mut catalog = mcp_service::load_operation_catalog(&db,&state.node_ws_manager,&owner,mcp_service::NodeScope::Unrestricted,mcp_service::ServiceScope::Unrestricted).await.unwrap();
        for slug in ["api-google-workspace","api-google-docs"] {
            let service = catalog.services.iter().find(|service| service.service_slug == slug).unwrap();
            let endpoint = service.endpoints.iter().find(|endpoint| endpoint.name == "docs_batch_update_document").unwrap();
            let (status, body) = mcp_call(&state,&owner,service,endpoint,&json!({"documentId":"document-2", "requests":[]})).await.unwrap();
            assert_eq!(status,200); assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["host"],"docs.googleapis.com");
        }
        let service = catalog.services.iter_mut().find(|service| service.service_slug == "api-google-workspace").unwrap();
        service.is_generic_proxy = true;
        let mut generic = service.endpoints.remove(0); generic.endpoint_id = mcp_service::GENERIC_PROXY_ENDPOINT_ID.into(); generic.target_id = None;
        let (status,body) = mcp_call(&state,&owner,service,&generic,&json!({"method":"POST","path":"/v1/documents/document-3:batchUpdate","body":{"requests":[]}})).await.unwrap();
        assert_eq!(status,200); assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["host"],"docs.googleapis.com");
        let response = rest_call(&state,&owner,"api-google-workspace","/v1/documents/redirect:batchUpdate",false).await.unwrap();
        assert_eq!(response.status(),302);
        let error = rest_call(&state,&owner,"api-google-workspace","/v1/documents/document-1",true).await.unwrap_err();
        assert!(matches!(error, AppError::BadRequest(ref message) if message == "Target-selected operations support HTTP only"));
    }).await;
    {
        let calls = echo.calls.lock().unwrap();
        assert_eq!(calls.len(), 5);
        for call in calls.iter() {
            assert_eq!(call["host"], "docs.googleapis.com");
            assert!(
                call["headers"]
                    .as_str()
                    .unwrap()
                    .contains("Bearer test-google-token")
            );
        }
    }
    let response = rest_call(
        &state,
        &owner,
        "api-google-docs",
        "/v1/documents/redirect:batchUpdate",
        false,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200, "existing product redirect behavior");
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["host"],
        "sheets.googleapis.com"
    );
    assert_eq!(echo.calls.lock().unwrap().len(), 7);
}

#[tokio::test]
async fn drive_and_workspace_inactive_errors_are_actionable_on_rest_and_mcp() {
    let db = connect_test_database("workspace_inactive").await.unwrap();
    seed_legacy(&db).await;
    for slug in ["api-google-drive", "api-google-workspace"] {
        let owner = uuid::Uuid::new_v4().to_string();
        connect(&db, &owner, slug).await;
        let state = test_app_state(db.clone());
        let error = rest_call(
            &state,
            &owner,
            slug,
            "/v1/documents/document:batchUpdate",
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::WorkspaceDestinationsNotActivated));

        assert_eq!(error.error_code(), 12300);
        assert!(
            error
                .to_string()
                .contains("Startup reconciliation did not complete")
        );
        assert!(
            !error
                .to_string()
                .contains("GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED")
        );
        assert_eq!(
            axum::response::IntoResponse::into_response(error).status(),
            503
        );
        let mut catalog = mcp_service::load_operation_catalog(
            &db,
            &state.node_ws_manager,
            &owner,
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .unwrap();
        assert!(mcp_service::inactive_workspace_tool(
            &format!("{slug}__docs_batch_update_document"),
            &catalog.services
        ));
        let service = catalog
            .services
            .iter_mut()
            .find(|service| service.service_slug == slug)
            .unwrap();
        service.is_generic_proxy = true;
        let mut endpoint = service.endpoints.remove(0);
        endpoint.endpoint_id = mcp_service::GENERIC_PROXY_ENDPOINT_ID.into();
        assert!(matches!(
            mcp_service::prepare_proxy_tool_call(
                service,
                &endpoint,
                &json!({"method":"POST","path":"/v1/documents/doc:batchUpdate","body":{"requests":[]}})
            ),
            Err(AppError::WorkspaceDestinationsNotActivated)
        ));
    }
}

#[tokio::test]
async fn workspace_node_v2_reaches_target_and_refuses_legacy_capability() {
    use crate::services::node_ws_manager::*;
    use base64::Engine;
    let echo = Echo::start().await;
    let manager = Arc::new(NodeWsManager::new(5, 100));
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    manager.register_connection("node", tx);
    manager.record_capabilities("node", &NodeCapabilitiesMsg::default());
    let request = NodeProxyRequest {
        target_id: Some("docs".into()),
        request_id: uuid::Uuid::new_v4().to_string(),
        service_id: "workspace-id".into(),
        service_slug: "api-google-workspace".into(),
        base_url: "https://docs.googleapis.com".into(),
        method: "POST".into(),
        path: "/v1/documents/doc:batchUpdate".into(),
        query: None,
        headers: vec![],
        body: Some(br#"{"requests":[]}"#.to_vec()),
    };
    let failure = manager
        .send_proxy_request_classified(
            "node",
            request.clone(),
            Some(&[0x11; 32]),
            permit(crate::services::billing::BillingIngress::Proxy),
        )
        .await
        .err()
        .unwrap();
    assert!(matches!(
        failure.error,
        AppError::NodeHttpSignatureUnsupported
    ));
    assert!(!failure.dispatched);
    assert!(rx.try_recv().is_err());
    manager.record_capabilities(
        "node",
        &NodeCapabilitiesMsg {
            http_signature_v2: true,
            ..Default::default()
        },
    );
    let downstream_manager = manager.clone();
    let client = echo.client.clone();
    let client_builder = echo.client_builder.clone();
    let executor = tokio::spawn(async move {
        let Some(NodeOutboundMessage::Text(text)) = rx.recv().await else {
            panic!("HTTP frame required")
        };
        let request: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(request["signature_version"], 2);
        assert_eq!(request["target_id"], "docs");
        let credentials = nyxid_node_proxy_test::bearer_credentials(
            "api-google-workspace",
            "https://www.googleapis.com",
            "node-google-token",
        )
        .unwrap();
        let replay = tokio::sync::Mutex::new(nyxid_node_proxy_test::ReplayGuard::new());
        let metrics = nyxid_node_proxy_test::NodeMetrics::new();
        let (tx, mut responses) = tokio::sync::mpsc::channel(16);
        nyxid_node_proxy_test::proxy_executor::TARGET_HTTP_CLIENT_BUILDER
            .scope(client_builder, async {
                // A signature cannot be transplanted to another origin, service,
                // target, path, query, or body, nor downgraded to v1.
                for (field, value) in [
                    ("base_url", json!("https://sheets.googleapis.com")),
                    ("service_id", json!("other")),
                    ("service_slug", json!("other")),
                    ("target_id", json!("sheets")),
                    ("path", json!("/other")),
                    ("query", json!("key=changed")),
                    ("body", json!("eA==")),
                    ("signature_version", json!(1)),
                ] {
                    let mut tampered = request.clone();
                    tampered[field] = value;
                    nyxid_node_proxy_test::proxy_executor::execute_proxy_request(
                        &tampered,
                        &credentials,
                        Some(&"11".repeat(32)),
                        &tokio::sync::Mutex::new(nyxid_node_proxy_test::ReplayGuard::new()),
                        &metrics,
                        &tx,
                        false,
                        &client,
                    )
                    .await;
                    let nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(response) =
                        responses.recv().await.unwrap()
                    else {
                        panic!("error frame")
                    };
                    assert_eq!(
                        serde_json::from_str::<Value>(&response).unwrap()["status"],
                        403,
                        "tampered {field}"
                    );
                }
                nyxid_node_proxy_test::proxy_executor::execute_proxy_request(
                    &request,
                    &credentials,
                    Some(&"11".repeat(32)),
                    &replay,
                    &metrics,
                    &tx,
                    false,
                    &client,
                )
                .await;
            })
            .await;
        let nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(response) =
            responses.recv().await.unwrap()
        else {
            panic!("response frame")
        };
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], 200, "{response}");
        downstream_manager.deliver_proxy_response(
            "node",
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
            "node",
            request,
            Some(&[0x11; 32]),
            permit(crate::services::billing::BillingIngress::Proxy),
        )
        .await
        .unwrap();
    let ProxyResponseType::Complete(response) = result else {
        panic!("complete JSON response")
    };
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap()["host"],
        "docs.googleapis.com"
    );
    executor.await.unwrap();
    assert_eq!(echo.calls.lock().unwrap().len(), 1);
}

#[test]
fn workspace_instance_nested_servers_are_removed_at_every_openapi_scope() {
    let evil = json!([{"url":"https://outside.test"}]);
    let mut spec = json!({"openapi":"3.1.0", "servers":evil, "paths":{"/test":{"servers":evil,"post":{"servers":evil,"callbacks":{"event":{"{$request.body#/url}":{"servers":evil,"post":{"servers":evil}}}}}}},"webhooks":{"event":{"servers":evil,"post":{"servers":evil}}},"components":{"pathItems":{"test":{"servers":evil,"get":{"servers":evil}}}}});
    crate::services::api_docs_service::rewrite_openapi_servers(
        &mut spec,
        "https://nyxid.test/api/v1/proxy/s/workspace",
    );
    assert_eq!(
        spec["servers"][0]["url"],
        "https://nyxid.test/api/v1/proxy/s/workspace"
    );
    assert!(!spec.to_string().contains("outside.test"));
}

#[tokio::test]
async fn workspace_hosted_route_publishes_38_standard_openapi_origins() {
    use tower::ServiceExt;
    let app = axum::Router::new().route(
        "/api/v1/catalog-specs/{spec_key}/openapi.json",
        axum::routing::get(crate::handlers::docs::catalog_spec_json),
    );
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/catalog-specs/google-workspace/openapi.json")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let spec: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(spec["servers"][0]["url"], "https://www.googleapis.com");
    let operations = crate::services::openapi_parser::parse_openapi_spec_value(&spec).unwrap();
    assert_eq!(operations.len(), 38);
    for operation in operations {
        let expected = if operation.name.starts_with("docs_") {
            "docs"
        } else if operation.name.starts_with("sheets_") {
            "sheets"
        } else if operation.name.starts_with("slides_") {
            "slides"
        } else {
            "www"
        };
        assert_eq!(
            operation.origin,
            Some(format!("https://{expected}.googleapis.com")),
            "{}",
            operation.name
        );
        if expected != "www" {
            assert_eq!(
                spec["paths"][&operation.path]["servers"][0]["url"],
                format!("https://{expected}.googleapis.com")
            );
        }
    }
}

#[test]
fn workspace_parser_server_precedence_and_policy_target_ambiguity() {
    let spec = json!({"openapi":"3.1.0","info":{"title":"test","version":"1"},"servers":[{"url":"https://www.googleapis.com"}],"paths":{"/test":{"servers":[{"url":"https://sheets.googleapis.com"}],"get":{"operationId":"read","servers":[{"url":"https://docs.googleapis.com"}],"responses":{"200":{"description":"ok"}}},"post":{"operationId":"write","responses":{"200":{"description":"ok"}}}}}});
    let endpoints = crate::services::openapi_parser::parse_openapi_spec_value(&spec).unwrap();
    assert_eq!(
        endpoints
            .iter()
            .find(|ep| ep.method == "GET")
            .unwrap()
            .origin
            .as_deref(),
        Some("https://docs.googleapis.com")
    );
    assert_eq!(
        endpoints
            .iter()
            .find(|ep| ep.method == "POST")
            .unwrap()
            .origin
            .as_deref(),
        Some("https://sheets.googleapis.com")
    );
    let policy = serde_json::from_value(json!({"rules":[{"method":"GET","path_template":"/test","target_id":"docs"},{"method":"get","path_template":"/test","target_id":"sheets"}]})).unwrap();
    assert!(crate::services::proxy_authorization::normalize_policy(policy).is_err());
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.destination_targets = workspace_targets();
    service.proxy_operation_policy = Some(
        serde_json::from_value(
            json!({"rules":[{"method":"GET","path_template":"/test","target_id":"missing"}]}),
        )
        .unwrap(),
    );
    assert!(
        select_target(
            &service,
            "GET",
            &CanonicalPath::from_mcp_literal("/test").unwrap()
        )
        .is_err()
    );
    service.proxy_operation_policy = None;
    assert!(
        select_target(
            &service,
            "GET",
            &CanonicalPath::from_mcp_literal("/test").unwrap()
        )
        .is_err()
    );
}

#[tokio::test]
async fn workspace_admin_policy_is_not_overwritten_and_override_recipient_is_checked() {
    let db = connect_test_database("workspace_policy_override")
        .await
        .unwrap();
    seed_legacy(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let service = connect(&db, &owner, "api-google-workspace").await;
    let catalog_id = service.catalog_service_id.as_ref().unwrap();
    let catalog = db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":catalog_id})
        .await
        .unwrap()
        .unwrap();
    let mut edited = catalog.proxy_operation_policy.clone().unwrap();
    edited.rules.pop();
    db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":catalog_id},
            doc! {"$set":{"proxy_operation_policy":bson::to_bson(&edited).unwrap()}},
        )
        .await
        .unwrap();
    seed(&db).await;
    let preserved = db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":catalog_id})
        .await
        .unwrap()
        .unwrap();
    assert!(preserved.destination_targets.is_empty());
    assert_eq!(preserved.proxy_operation_policy, Some(edited));
    db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME).update_one(doc! {"_id":catalog_id},doc! {"$set":{"proxy_operation_policy":bson::to_bson(&catalog.proxy_operation_policy).unwrap()}}).await.unwrap();
    seed(&db).await;
    let mut key = db
        .collection::<user_api_key::UserApiKey>(user_api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":service.api_key_id.as_ref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    validate_override_recipient(&db, &service.id, &key)
        .await
        .unwrap();
    key.id = uuid::Uuid::new_v4().to_string();
    key.provider_config_id = Some("outside-provider".into());
    assert!(
        validate_override_recipient(&db, &service.id, &key)
            .await
            .is_err()
    );
}

#[test]
fn workspace_selected_origins_separate_cache_entries() {
    use crate::services::cloud_response_cache::CloudResponseCache;
    let a = CloudResponseCache::key(
        "bearer",
        "token-fingerprint",
        "https://docs.googleapis.com",
        "GET",
        "/same",
        &[],
        &[],
    );
    let b = CloudResponseCache::key(
        "bearer",
        "token-fingerprint",
        "https://sheets.googleapis.com",
        "GET",
        "/same",
        &[],
        &[],
    );
    assert_ne!(a, b);
}

#[tokio::test]
async fn workspace_node_redirect_policy_is_applied_by_the_production_client_builder() {
    use crate::services::node_ws_manager::{NodeProxyRequest, sign_proxy_request};
    use nyxid_node_proxy_test::{
        NodeMetrics, ReplayGuard, proxy_executor, ws_client::NodeWsMessage,
    };
    let echo = Echo::start().await;
    let request = NodeProxyRequest {
        target_id: Some("docs".into()),
        request_id: uuid::Uuid::new_v4().to_string(),
        service_id: "workspace-id".into(),
        service_slug: "api-google-workspace".into(),
        base_url: "https://docs.googleapis.com".into(),
        method: "POST".into(),
        path: "/v1/documents/redirect:batchUpdate".into(),
        query: None,
        headers: vec![],
        body: Some(br#"{"requests":[]}"#.to_vec()),
    };
    let signature = sign_proxy_request(&[0x11; 32], &request);
    let mut wire = serde_json::to_value(&request).unwrap();
    wire["signature_version"] = 2.into();
    wire["signature"] = signature.signature.into();
    wire["timestamp"] = signature.timestamp.into();
    wire["nonce"] = signature.nonce.into();
    let credentials = nyxid_node_proxy_test::bearer_credentials(
        "api-google-workspace",
        "https://www.googleapis.com",
        "node-google-token",
    )
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    proxy_executor::TARGET_HTTP_CLIENT_BUILDER
        .scope(echo.client_builder.clone(), async {
            proxy_executor::execute_proxy_request(
                &wire,
                &credentials,
                Some(&"11".repeat(32)),
                &tokio::sync::Mutex::new(ReplayGuard::new()),
                &NodeMetrics::new(),
                &tx,
                false,
                &echo.client,
            )
            .await;
        })
        .await;
    let NodeWsMessage::Text(response) = rx.recv().await.unwrap() else {
        panic!("response frame")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["status"],
        302
    );
    assert_eq!(echo.calls.lock().unwrap().len(), 1);
    // The ordinary client from the same fixture follows the redirect. Only
    // the production target client applies the no-redirect restriction.
    let response = echo
        .client
        .post("https://docs.googleapis.com/v1/documents/redirect:batchUpdate")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.url().host_str(), Some("sheets.googleapis.com"));
    assert_eq!(echo.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn workspace_resolved_target_cannot_enter_a_specialized_transport_after_selection() {
    let db = connect_test_database("workspace_specialized_transport")
        .await
        .unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let service = connect(&db, &owner, "api-google-workspace").await;
    // An admin-selected generic HTTP operation must retain its recipient even
    // if the user's connection slug coincides with a specialized transport.
    db.collection::<user_service::UserService>(user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &service.id},
            doc! {"$set":{"slug":"llm-openai-codex"}},
        )
        .await
        .unwrap();
    db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id":service.catalog_service_id.as_ref().unwrap()},
            doc! {"$push":{"proxy_operation_policy.rules":{"method":"POST","path_template":"/responses","target_id":"docs"}}})
        .await.unwrap();
    let echo = Echo::start().await;
    let mut state = test_app_state(db);
    state.http_client = echo.client.clone();
    let response = crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(
            echo.client_builder.clone(),
            rest_call(&state, &owner, "llm-openai-codex", "/responses", false),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["host"],
        "docs.googleapis.com"
    );
    assert_eq!(echo.calls.lock().unwrap().len(), 1);
}

#[test]
fn workspace_instance_routing_refs_are_sanitized_without_changing_schema_refs() {
    let outside = json!([{"url":"https://outside.test"}]);
    let schema = json!({"$ref":"https://schema.example/response.json"});
    let mut spec = json!({
        "openapi":"3.1.0", "paths":{
            "/inline":{"servers":outside,"get":{"servers":outside,"responses":{"200":{"content":{"application/json":{"schema":schema}}}},"callbacks":{
                "local":{"$ref":"#/components/callbacks/local"},
                "x-external":{"$ref":"https://outside.test/callback.json?credential=secret"}
            }}},
            "/local":{"$ref":"#/components/pathItems/shared"},
            "/external":{"$ref":"https://outside.test/path.json?credential=secret","get":{"servers":outside}},
            "/cyclic":{"$ref":"#/paths/~1cyclic","servers":outside}
        },
        "components":{
            "pathItems":{"shared":{"servers":outside,"get":{"servers":outside}}},
            "callbacks":{"local":{"{$request.body#/callback}":{"servers":outside,"post":{"servers":outside}}}}
        }
    });
    crate::services::api_docs_service::rewrite_openapi_servers(
        &mut spec,
        "https://nyxid.test/proxy",
    );
    assert_eq!(spec["servers"][0]["url"], "https://nyxid.test/proxy");
    assert!(spec["paths"]["/inline"].get("servers").is_none());
    assert!(spec["paths"]["/inline"]["get"].get("servers").is_none());
    assert_eq!(
        spec["paths"]["/local"]["$ref"],
        "#/components/pathItems/shared"
    );
    assert!(
        spec["components"]["pathItems"]["shared"]
            .get("servers")
            .is_none()
    );
    assert!(
        spec["components"]["pathItems"]["shared"]["get"]
            .get("servers")
            .is_none()
    );
    assert!(
        spec["components"]["callbacks"]["local"]["{$request.body#/callback}"]["post"]
            .get("servers")
            .is_none()
    );
    for omitted in [
        &spec["paths"]["/external"],
        &spec["paths"]["/inline"]["get"]["callbacks"]["x-external"],
    ] {
        assert_eq!(
            omitted,
            &json!({"x-nyxid-omitted-external-ref":{"reason":"external routing reference is not served through the proxy"}})
        );
    }
    assert_eq!(
        spec["paths"]["/inline"]["get"]["responses"]["200"]["content"]["application/json"]["schema"],
        schema
    );
    assert!(!spec.to_string().contains("outside.test"));
    assert!(!spec.to_string().contains("credential=secret"));
}

#[tokio::test]
async fn workspace_served_external_routing_omission_preserves_instance_catalog_precedence() {
    use crate::services::api_docs_service::{SpecCacheTestGuard, cache_test_spec};
    let db = connect_test_database("workspace_instance_ref_precedence")
        .await
        .unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let service = connect(&db, &owner, "api-google-workspace").await;
    let spec_url = "https://nyxid.test/api/v1/catalog-specs/google-workspace/openapi.json";
    db.collection::<user_endpoint::UserEndpoint>(user_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&service.endpoint_id},
            doc! {"$set":{"openapi_spec_url":spec_url}},
        )
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let _guard = SpecCacheTestGuard::acquire();
    for has_inline_operation in [false, true] {
        let mut spec = json!({"openapi":"3.1.0","info":{"title":"Instance","version":"1"},"paths":{
            "/external":{"$ref":"https://outside.test/path.json"}
        },"components":{"responses":{"shared":{"description":"ok","links":{
            "next":{"operationRef":"https://outside.test/operation.json","server":{"url":"https://outside.test"}}
        }}}}});
        if has_inline_operation {
            spec["paths"]["/v1/documents/{documentId}"] = json!({"get":{"operationId":"instance_read_document","responses":{"200":{"$ref":"#/components/responses/shared"}}}});
        }
        cache_test_spec(spec_url, Some(&owner), spec);
        let before = mcp_service::load_operation_catalog(
            &db,
            &state.node_ws_manager,
            &owner,
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .unwrap();
        let before = before
            .services
            .iter()
            .find(|entry| entry.service_id == service.id)
            .unwrap();
        let expected: Vec<_> = before
            .endpoints
            .iter()
            .map(|endpoint| (endpoint.endpoint_id.clone(), endpoint.name.clone()))
            .collect();
        assert_eq!(expected.len(), if has_inline_operation { 1 } else { 38 });
        let response = crate::handlers::docs::service_openapi_json(
            axum::extract::State(state.clone()),
            test_auth_user(&owner),
            axum::extract::Path(service.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let served: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            served["paths"]["/external"]
                .get("x-nyxid-omitted-external-ref")
                .is_some()
        );
        assert!(!served.to_string().contains("outside.test"));
        assert_eq!(
            served["components"]["responses"]["shared"]["links"]["next"]["x-nyxid-omitted-external-ref"]
                ["reason"],
            "external link reference is not served through the proxy"
        );
        let after = mcp_service::load_operation_catalog(
            &db,
            &state.node_ws_manager,
            &owner,
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .unwrap();
        let after = after
            .services
            .iter()
            .find(|entry| entry.service_id == service.id)
            .unwrap();
        assert_eq!(
            after
                .endpoints
                .iter()
                .map(|endpoint| (endpoint.endpoint_id.clone(), endpoint.name.clone()))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn workspace_instance_links_keep_local_metadata_and_omit_external_routing() {
    let local = json!({"operationRef":"#/paths/~1inline/get","server":{"url":"https://outside.test"},
        "parameters":{"id":"$response.body#/id"},"description":"Next operation","requestBody":{"keep":true}});
    let by_id = json!({"operationId":"next","server":{"url":"https://outside.test"},"description":"Next by ID"});
    let mut spec = json!({"openapi":"3.1.0","paths":{
        "/inline":{"get":{"responses":{"200":{"description":"ok","links":{
            "external":{"operationRef":"https://outside.test/openapi.json#/paths/~1next/get","server":{"url":"https://outside.test"}},
            "external_link_ref":{"$ref":"https://outside.test/link.json"},
            "local":local,"by_id":by_id
        }}}}},
        "/response-ref":{"get":{"responses":{"200":{"$ref":"#/components/responses/shared"}}}}
    },"components":{
        "responses":{"shared":{"description":"Shared response","links":{"next":{"$ref":"#/components/links/shared"}}}},
        "links":{"shared":local,"cyclic":{"$ref":"#/components/links/cyclic","server":{"url":"https://outside.test"}}}
    }});
    crate::services::api_docs_service::rewrite_openapi_servers(
        &mut spec,
        "https://nyxid.test/proxy",
    );
    let links = &spec["paths"]["/inline"]["get"]["responses"]["200"]["links"];
    let omitted = json!({"x-nyxid-omitted-external-ref":{"reason":"external link reference is not served through the proxy"}});
    assert_eq!(links["external"], omitted);
    assert_eq!(links["external_link_ref"], omitted);
    let mut expected_local = local;
    expected_local.as_object_mut().unwrap().remove("server");
    assert_eq!(links["local"], expected_local);
    assert_eq!(spec["components"]["links"]["shared"], expected_local);
    let mut expected_id = by_id;
    expected_id.as_object_mut().unwrap().remove("server");
    assert_eq!(links["by_id"], expected_id);
    assert_eq!(
        spec["paths"]["/response-ref"]["get"]["responses"]["200"]["$ref"],
        "#/components/responses/shared"
    );
    assert_eq!(
        spec["components"]["responses"]["shared"]["links"]["next"]["$ref"],
        "#/components/links/shared"
    );
    assert!(!spec.to_string().contains("outside.test"));
}

#[tokio::test]
async fn workspace_external_response_omission_preserves_template_tools_and_cached_spec() {
    use crate::services::api_docs_service::{SpecCacheTestGuard, cache_test_spec};
    let db = connect_test_database("workspace_external_response")
        .await
        .unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let connection = connect(&db, &owner, "api-google-workspace").await;
    let catalog_id = connection.catalog_service_id.as_deref().unwrap();
    // Hosted test URLs enter the existing cache path without a DNS lookup.
    // Arbitrary remote URLs are validated before cache lookup in production.
    let spec_url = "https://nyxid.test/api/v1/catalog-specs/google-workspace/openapi.json";
    db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":catalog_id},
            doc! {"$set":{"openapi_spec_url":spec_url}},
        )
        .await
        .unwrap();
    let original = json!({"openapi":"3.1.0","info":{"title":"Workspace","version":"1"},"paths":{
        "/v1/documents/{documentId}:batchUpdate":{"post":{"operationId":"docs_batch_update_document",
            "requestBody":{"content":{"application/json":{"schema":{"$ref":"https://schema.example/request.json"}}}},
            "responses":{"200":{"$ref":"https://outside.test/response.json"}}
        }}
    }});
    let state = test_app_state(db.clone());
    let _guard = SpecCacheTestGuard::acquire();
    cache_test_spec(spec_url, None, original.clone());
    let before = mcp_service::load_operation_catalog(
        &db,
        &state.node_ws_manager,
        &owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    let tool_service = before
        .services
        .iter()
        .find(|service| service.service_id == connection.id)
        .unwrap();
    assert_eq!(tool_service.endpoints.len(), 38);
    let endpoint = tool_service
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == "docs_batch_update_document")
        .unwrap();
    assert!(
        mcp_service::producer_operation_generation(tool_service, endpoint).is_some(),
        "template endpoint with durable producer generation"
    );
    let before_digest = mcp_service::operation_catalog_digest(&before.services);
    let response = crate::handlers::docs::service_openapi_json(
        axum::extract::State(state.clone()),
        test_auth_user(&owner),
        axum::extract::Path(catalog_id.to_string()),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let served: Value = serde_json::from_slice(&bytes).unwrap();
    let operation = &served["paths"]["/v1/documents/{documentId}:batchUpdate"]["post"];
    assert_eq!(operation["operationId"], "docs_batch_update_document");
    assert_eq!(
        operation["responses"]["200"],
        json!({"description":"External response documentation omitted by NyxID","x-nyxid-omitted-external-ref":{"reason":"external response reference is not served through the proxy"}})
    );
    assert_eq!(
        operation["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "https://schema.example/request.json"
    );
    assert_eq!(
        crate::services::api_docs_service::fetch_spec_json(spec_url)
            .await
            .unwrap()
            .as_ref(),
        &original
    );
    let after = mcp_service::load_operation_catalog(
        &db,
        &state.node_ws_manager,
        &owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    assert_eq!(
        mcp_service::operation_catalog_digest(&after.services),
        before_digest
    );
}

#[tokio::test]
async fn workspace_platform_keys_are_rejected_before_target_selection_and_catalog_projection() {
    let db = connect_test_database("workspace_platform_recipient")
        .await
        .unwrap();
    seed(&db).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let connection = connect(&db, &owner, "api-google-workspace").await;
    let state = test_app_state(db.clone());
    let catalog_id = connection.catalog_service_id.as_ref().unwrap();
    let catalog = db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id": catalog_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        effective_catalog_auth(&db, &catalog).await.unwrap(),
        "bearer"
    );
    let mut configured = catalog.clone();
    configured.platform_key = Some(downstream_service::PlatformKeyConfig {
        enabled: true,
        ..Default::default()
    });
    let mut disabled = configured.clone();
    disabled.platform_key.as_mut().unwrap().enabled = false;
    let mut legacy = catalog.clone();
    legacy.auth_method = "bearer".into();
    legacy.requires_user_credential = false;
    legacy.service_category = "internal".into();
    legacy.provider_config_id = None;
    legacy.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-token")
        .await
        .unwrap();
    for service in [configured, disabled, legacy] {
        assert!(matches!(
            crate::services::proxy_service::authorize_master_credential_server_chosen(&db, &service).await,
            Err(AppError::ValidationError(message)) if message == "Destination targets do not support platform keys"
        ));
        assert!(matches!(effective_catalog_auth(&db, &service).await,
            Err(AppError::ValidationError(message)) if message == "Destination targets do not support platform keys"));
        let mut target = ProxyTarget {
            workspace_destinations_pending: false,
            target_id: None,
            base_url: catalog.base_url.clone(),
            auth_method: "bearer".into(),
            auth_key_name: "Authorization".into(),
            credential: "platform-token".into(),
            service: service.clone(),
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: None,
        };
        let path = CanonicalPath::from_mcp_literal("/v1/documents/doc:batchUpdate").unwrap();
        assert!(
            matches!(resolve_target(&mut target, "POST", &path, Some(Some("docs"))),
            Err(AppError::ValidationError(message)) if message == "Destination targets do not support platform keys")
        );
        assert_eq!(target.base_url, catalog.base_url);
        assert!(target.target_id.is_none());
        db.collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
            .replace_one(doc! {"_id": catalog_id}, &service)
            .await
            .unwrap();
        let error = rest_call(
            &state,
            &owner,
            &connection.slug,
            "/v1/documents/doc:batchUpdate",
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, AppError::ValidationError(ref message) if message == "Destination targets do not support platform keys"),
            "{error:?}"
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn user_owned_bearer_target_does_not_become_platform_by_catalog_category() {
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.service_category = "internal".into();
    service.requires_user_credential = false;
    service.auth_method = "bearer".into();
    service.destination_targets = workspace_targets();
    service.proxy_operation_policy = Some(
        crate::services::google_workspace::GoogleProduct::Workspace
            .operation_policy()
            .unwrap(),
    );
    // A resolved UserService carries its own credential, not encrypted catalog
    // master material. Catalog category/requirement flags alone do not select a key.
    assert!(service.credential_encrypted.is_empty());
    let mut target = ProxyTarget {
        workspace_destinations_pending: false,
        target_id: None,
        base_url: "https://www.googleapis.com".into(),
        auth_method: "bearer".into(),
        auth_key_name: "Authorization".into(),
        credential: "user-owned-token".into(),
        service,
        catalog_default_headers: vec![],
        user_service_default_headers: vec![],
        ws_frame_injections: vec![],
        connection_id: None,
    };
    let path = CanonicalPath::from_mcp_literal("/v1/documents/doc:batchUpdate").unwrap();
    resolve_target(&mut target, "POST", &path, Some(Some("docs"))).unwrap();
    assert_eq!(target.target_id.as_deref(), Some("docs"));
    assert_eq!(target.base_url, "https://docs.googleapis.com");
    validate_outbound_destination(&target, "POST", "/v1/documents/doc:batchUpdate", &[]).unwrap();
}

#[path = "google_auto_activation_tests.rs"]
mod auto_activation;
