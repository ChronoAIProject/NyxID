use super::*;
use crate::{
    models::{user::UserType, user_api_key::UserApiKey},
    test_utils::*,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::{any, post},
};
use mongodb::bson::{Document, doc};
use nyxid_permissions::{drive::example_policy, hooks::HookDefinition, mock::MockDrive};
use tower::ServiceExt;

async fn fixture() -> (crate::AppState, String, String) {
    let db = connect_transaction_test_database("permission_keys").await;
    let state = test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "api-google-drive".into();
    catalog.base_url = "https://www.googleapis.com".into();
    catalog.auth_method = "bearer".into();
    catalog.requires_user_credential = true;
    db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    let endpoint_id = uuid::Uuid::new_v4().to_string();
    db.collection("user_endpoints")
        .insert_one(test_user_endpoint(
            &endpoint_id,
            &owner,
            "Drive",
            &catalog.base_url,
            None,
            Some(&catalog.id),
        ))
        .await
        .unwrap();
    let credential_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let credential = UserApiKey {
        secret_context_versions: Default::default(),
        id: credential_id.clone(),
        user_id: owner.clone(),
        label: "Drive test".into(),
        credential_type: "oauth2".into(),
        credential_encrypted: None,
        access_token_encrypted: Some(
            state
                .encryption_keys
                .encrypt(b"test-token-never-dispatched")
                .await
                .unwrap(),
        ),
        refresh_token_encrypted: None,
        token_scopes: None,
        expires_at: None,
        provider_config_id: None,
        connection_id: None,
        oauth_attempt_nonce: None,
        user_oauth_client_id_encrypted: None,
        user_oauth_client_secret_encrypted: None,
        credential_source: None,
        status: "active".into(),
        last_used_at: None,
        last_authorized_at: None,
        error_message: None,
        source: None,
        source_id: None,
        credential_epoch: 1,
        oauth_app_observation: None,
        created_at: now,
        updated_at: now,
    };
    db.collection("user_api_keys")
        .insert_one(credential)
        .await
        .unwrap();
    let connection_id = uuid::Uuid::new_v4().to_string();
    let mut connection = test_user_service(
        &connection_id,
        &owner,
        &catalog.slug,
        &endpoint_id,
        Some(&catalog.id),
        None,
    );
    connection.slug = "my-google-drive-connection".into();
    connection.api_key_id = Some(credential_id);
    connection.auth_method = "bearer".into();
    connection.auth_key_name = "Authorization".into();
    db.collection("user_services")
        .insert_one(connection)
        .await
        .unwrap();
    (state, owner, connection_id)
}

async fn issue(
    state: &crate::AppState,
    owner: &str,
    connection: &str,
) -> (key_service::CreatedApiKey, PermissionPolicy) {
    create(
        &state.db,
        &state.encryption_keys,
        owner,
        "Project folder",
        connection,
        Utc::now() + chrono::Duration::days(1),
        example_policy(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn permission_keys_persist_atomically_and_live_pause_revocation_reach_engine() {
    let (state, owner, connection) = fixture().await;
    let (key, binding) = issue(&state, &owner, &connection).await;
    let (_, stored_key, _) = key_service::validate_api_key(&state.db, &key.full_key)
        .await
        .unwrap();
    assert_eq!(stored_key.purpose, ApiKeyPurpose::PermissionBound);
    assert!(stored_key.expires_at.is_some());
    assert_eq!(key.expires_at, stored_key.expires_at);
    assert_eq!(stored_key.allowed_service_ids, [connection]);
    assert!(!stored_key.allow_all_nodes && !stored_key.allow_all_services);
    let stored = get(&state.db, &owner, &key.id).await.unwrap();
    let bson = state
        .db
        .collection::<Document>(POLICIES)
        .find_one(doc! {"_id": &key.id})
        .await
        .unwrap()
        .unwrap();
    assert!(bson.get_datetime("created_at").is_ok());
    let evaluator = engine(state.db.clone(), stored, Arc::new(MockDrive::default())).unwrap();
    let request = nyxid_permissions::Request::get("/drive/v3/files/report");
    assert!(evaluator.execute(request.clone()).await.is_ok());
    assert!(
        key_service::rotate_api_key(&state.db, &state.encryption_keys, &owner, &key.id)
            .await
            .is_err()
    );
    let ordinary_mcp = Router::new()
        .route(
            "/mcp",
            post(crate::handlers::mcp_transport::mcp_post).layer(axum::Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Mcp,
                ),
            )),
        )
        .with_state(state.clone());
    let rejected = ordinary_mcp
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("x-api-key", &key.full_key)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let paused = set_paused(&state.db, &owner, &key.id, 1, true)
        .await
        .unwrap();
    assert_eq!(paused.revision, 2);
    assert!(
        set_paused(&state.db, &owner, &key.id, 1, false)
            .await
            .is_err()
    );
    assert!(evaluator.execute(request.clone()).await.is_err());
    let resumed = set_paused(&state.db, &owner, &key.id, 2, false)
        .await
        .unwrap();
    assert!(
        evaluator.execute(request.clone()).await.is_err(),
        "old requests stay fenced after resume"
    );
    let evaluator = engine(state.db.clone(), resumed, Arc::new(MockDrive::default())).unwrap();
    assert!(evaluator.execute(request.clone()).await.is_ok());
    key_service::delete_api_key(&state.db, &owner, &binding.id)
        .await
        .unwrap();
    assert!(evaluator.execute(request).await.is_err());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn permission_keys_bind_credential_epoch_and_reject_other_owners() {
    let (state, owner, connection) = fixture().await;
    let (_, binding) = issue(&state, &owner, &connection).await;
    assert!(get(&state.db, "someone-else", &binding.id).await.is_err());
    assert!(
        create(
            &state.db,
            &state.encryption_keys,
            "someone-else",
            "bad",
            &connection,
            Utc::now() + chrono::Duration::days(1),
            example_policy()
        )
        .await
        .is_err()
    );
    let snapshot = || {
        proxy_service::read_proxy_authority_snapshot_by_user_service_id(
            &state.db,
            &state.encryption_keys,
            &owner,
            &connection,
            None,
        )
    };
    let before = snapshot().await.unwrap().unwrap();
    assert_eq!(
        authority(&before).unwrap(),
        binding.execution_authority_digest
    );
    state
        .db
        .collection::<Document>("user_api_keys")
        .update_one(
            doc! {"_id": &before.api_key_id},
            vec![doc! {"$set": {"credential_epoch": {"$add": ["$credential_epoch", 1]}}}],
        )
        .await
        .unwrap();
    assert_ne!(
        authority(&snapshot().await.unwrap().unwrap()).unwrap(),
        binding.execution_authority_digest
    );
    state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id": &connection},
            doc! {"$set": {"node_id": "new-node"}},
        )
        .await
        .unwrap();
    assert!(check_live(&state.db, &binding).await.is_err());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn permission_keys_http_auth_blocks_alternate_routes_and_missing_policy() {
    let (state, owner, connection) = fixture().await;
    let (key, binding) = issue(&state, &owner, &connection).await;
    async fn sentinel(_: crate::mw::auth::AuthUser) -> StatusCode {
        StatusCode::NO_CONTENT
    }
    let router = Router::new()
        .route("/{*path}", any(sentinel))
        .with_state(state.clone());
    for path in [
        "/api/v1/proxy/id/drive/v3/files/report",
        "/api/v1/mcp",
        "/api/v1/llm/chat/completions",
        "/api/v1/auth/agent-key",
        "/api/v1/keys",
        "/api/v1/permission-keys",
        "/api/v1/permission-execution/restish",
    ] {
        for header in ["authorization", "x-api-key"] {
            let value = if header == "authorization" {
                format!("Bearer {}", key.full_key)
            } else {
                key.full_key.clone()
            };
            let r = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header(header, value)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{header} {path}");
        }
    }
    let rest_router = Router::new()
        .route(
            "/api/v1/permission-execution/rest/{*path}",
            any(crate::handlers::permission_keys::rest).layer(axum::Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            )),
        )
        .with_state(state.clone());
    let denied = rest_router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/permission-execution/rest/drive/v3/files/report")
                .header("authorization", format!("Bearer {}", key.full_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let native = Router::new()
        .route(
            "/api/v1/permission-execution/mcp",
            post(crate::handlers::permission_keys::mcp).layer(axum::Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            )),
        )
        .with_state(state.clone());
    let ping = || {
        Request::builder()
            .method("POST")
            .uri("/api/v1/permission-execution/mcp")
            .header("authorization", format!("Bearer {}", key.full_key))
            .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
            .unwrap()
    };
    assert_eq!(
        native.clone().oneshot(ping()).await.unwrap().status(),
        StatusCode::OK
    );
    state
        .db
        .collection::<PermissionPolicy>(POLICIES)
        .delete_one(doc! {"_id": binding.id})
        .await
        .unwrap();
    assert_eq!(
        native.oneshot(ping()).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn permission_keys_mcp_uses_same_folder_and_response_hooks() {
    let (state, owner, connection) = fixture().await;
    let (_, mut binding) = issue(&state, &owner, &connection).await;
    binding.policy.hooks.push(
        serde_json::from_value::<HookDefinition>(serde_json::json!({
            "name": "hide-name", "handler": "response_markers", "stage": "after_response",
            "config": {"markers": ["report"]}
        }))
        .unwrap(),
    );
    let evaluator = engine(state.db.clone(), binding, Arc::new(MockDrive::default())).unwrap();
    for file in ["report", "salary"] {
        let message = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call", "params": {
            "name":"drive_request", "arguments":{"method":"GET","path":format!("/drive/v3/files/{file}")}
        }});
        let response = nyxid_permissions::server::handle_mcp(
            &evaluator,
            serde_json::to_vec(&message).unwrap().into(),
        )
        .await;
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["result"]["isError"], true, "{file}");
    }
    state.db.drop().await.unwrap();
}

#[test]
fn permission_keys_native_hooks_cannot_access_host_files() {
    let mut policy = example_policy();
    policy.hooks.push(serde_json::from_value(serde_json::json!({
        "name":"pause", "handler":"pause_switch", "stage":"before_execute", "config":{"path":"/etc/passwd"}
    })).unwrap());
    assert!(validate_policy(&policy).is_err());
}

#[tokio::test]
async fn permission_keys_pause_after_write_withholds_response_without_replay() {
    struct PauseAfterWrite {
        db: Database,
        binding: PermissionPolicy,
        drive: Arc<MockDrive>,
    }
    #[async_trait]
    impl Transport for PauseAfterWrite {
        async fn send(
            &self,
            request: &nyxid_permissions::Request,
        ) -> Result<nyxid_permissions::Response, Error> {
            let response = self.drive.send(request).await?;
            if request.method == axum::http::Method::POST {
                set_paused(
                    &self.db,
                    &self.binding.user_id,
                    &self.binding.id,
                    self.binding.revision,
                    true,
                )
                .await
                .map_err(|_| Error::Verification)?;
            }
            Ok(response)
        }
    }
    let (state, owner, connection) = fixture().await;
    let (_, binding) = issue(&state, &owner, &connection).await;
    let drive = Arc::new(MockDrive::default());
    let transport = Arc::new(PauseAfterWrite {
        db: state.db.clone(),
        binding: binding.clone(),
        drive: drive.clone(),
    });
    let evaluator = engine(state.db.clone(), binding, transport).unwrap();
    let result = evaluator.execute(nyxid_permissions::Request {
        method: axum::http::Method::POST, path: "/drive/v3/files".into(), query: Default::default(),
        body: serde_json::to_vec(&serde_json::json!({"name":"New folder", "mimeType":nyxid_permissions::drive::FOLDER, "parents":["client-a"]})).unwrap().into(),
        content_type: Some("application/json".into()),
    }).await;
    assert!(matches!(result, Err(Error::WriteAcceptedResponseWithheld)));
    assert_eq!(
        drive
            .requests()
            .iter()
            .filter(|r| r.method == axum::http::Method::POST)
            .count(),
        1
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn permission_keys_invalid_creation_leaves_no_key_or_policy() {
    let (state, owner, connection) = fixture().await;
    for expires in [
        Utc::now() - chrono::Duration::hours(1),
        Utc::now() + chrono::Duration::days(91),
    ] {
        assert!(
            create(
                &state.db,
                &state.encryption_keys,
                &owner,
                "Key",
                &connection,
                expires,
                example_policy()
            )
            .await
            .is_err()
        );
    }
    assert!(
        create(
            &state.db,
            &state.encryption_keys,
            &owner,
            "",
            &connection,
            Utc::now() + chrono::Duration::days(1),
            example_policy()
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(KEYS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(POLICIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

fn workspace_policy() -> Policy {
    serde_json::from_str(include_str!(
        "../../../permissions/examples/google-workspace-policy.json"
    ))
    .unwrap()
}

#[tokio::test]
async fn permission_keys_google_workspace_routes_only_to_the_policy_origin() {
    use crate::services::{
        destination_routing, google_workspace::GoogleProduct, proxy_authorization::CanonicalPath,
    };
    let (state, owner, connection) = fixture().await;
    let service = state
        .db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {"_id": &connection})
        .await
        .unwrap()
        .unwrap();
    state.db.collection::<Document>("downstream_services").update_one(
        doc! {"_id": &service.catalog_service_id},
        doc! {"$set": {"slug": "api-google-workspace",
            "destination_targets": bson::to_bson(&destination_routing::workspace_targets()).unwrap(),
            "proxy_operation_policy": bson::to_bson(&GoogleProduct::Workspace.operation_policy().unwrap()).unwrap()}},
    ).await.unwrap();
    let (_, binding) = create(
        &state.db,
        &state.encryption_keys,
        &owner,
        "Workspace",
        &connection,
        Utc::now() + chrono::Duration::days(1),
        workspace_policy(),
    )
    .await
    .unwrap();
    let resolved = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
        &state.db,
        &state.encryption_keys,
        &owner,
        &connection,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        authority(&resolved).unwrap(),
        binding.execution_authority_digest
    );
    for (path, origin) in [
        ("/drive/v3/files/report-file", "https://www.googleapis.com"),
        ("/gmail/v1/users/me/messages", "https://www.googleapis.com"),
        (
            "/calendar/v3/calendars/team@example.com/events",
            "https://www.googleapis.com",
        ),
        (
            "/v1/documents/project-document",
            "https://docs.googleapis.com",
        ),
        (
            "/v4/spreadsheets/project-sheet/values/Sheet1!A1:B10",
            "https://sheets.googleapis.com",
        ),
        (
            "/v1/presentations/project-slides",
            "https://slides.googleapis.com",
        ),
    ] {
        let mut target = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
            &state.db,
            &state.encryption_keys,
            &owner,
            &connection,
            None,
        )
        .await
        .unwrap()
        .unwrap()
        .target;
        destination_routing::resolve_target(
            &mut target,
            "GET",
            &CanonicalPath::from_mcp_literal(path).unwrap(),
            None,
        )
        .unwrap();
        validate_execution_target(&target, Some(origin)).unwrap();
        assert!(validate_execution_target(&target, Some("https://other.googleapis.com")).is_err());
        if origin != "https://www.googleapis.com" {
            assert!(
                validate_execution_target(&target, None).is_err(),
                "Drive policy cannot reach editors"
            );
        }
    }
    let mut wrong_policy = workspace_policy();
    let ResourceBoundary::GoogleApi { operations } = &mut wrong_policy.resource else {
        unreachable!()
    };
    operations[0].origin = "https://storage.googleapis.com".into();
    assert!(
        create(
            &state.db,
            &state.encryption_keys,
            &owner,
            "Wrong recipient",
            &connection,
            Utc::now() + chrono::Duration::days(1),
            wrong_policy
        )
        .await
        .is_err()
    );
    state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id": &service.catalog_service_id},
            doc! {"$set": {"destination_targets.docs": "https://sheets.googleapis.com"}},
        )
        .await
        .unwrap();
    let drift = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
        &state.db,
        &state.encryption_keys,
        &owner,
        &connection,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(
        authority(&drift).unwrap(),
        binding.execution_authority_digest
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn permission_keys_google_supports_registered_apis_and_private_api_keys() {
    let (state, owner, connection) = fixture().await;
    let service = state
        .db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {"_id": &connection})
        .await
        .unwrap()
        .unwrap();
    for (slug, base, auth, auth_name, credential_type, source) in [
        (
            "my-google-cloud",
            "https://cloudresourcemanager.googleapis.com",
            "bearer",
            "Authorization",
            "oauth2",
            include_str!("../../../permissions/examples/google-cloud-policy.json"),
        ),
        (
            "my-youtube",
            "https://www.googleapis.com",
            "query",
            "key",
            "api_key",
            include_str!("../../../permissions/examples/google-youtube-policy.json"),
        ),
        (
            "llm-google-ai",
            "https://generativelanguage.googleapis.com/v1beta",
            "header",
            "x-goog-api-key",
            "api_key",
            include_str!("../../../permissions/examples/google-gemini-policy.json"),
        ),
    ] {
        state.db.collection::<Document>("downstream_services").update_one(doc! {"_id": &service.catalog_service_id},
            doc! {"$set": {"slug": slug, "base_url": base, "auth_method": auth, "auth_key_name": auth_name}}).await.unwrap();
        state
            .db
            .collection::<Document>("user_endpoints")
            .update_one(
                doc! {"_id": &service.endpoint_id},
                doc! {"$set": {"url": base}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(SERVICES)
            .update_one(
                doc! {"_id": &connection},
                doc! {"$set": {"auth_method": auth, "auth_key_name": auth_name}},
            )
            .await
            .unwrap();
        state.db.collection::<Document>(CREDENTIALS).update_one(doc! {"_id": &service.api_key_id},
            doc! {"$set": {"credential_type": credential_type, "credential_encrypted": bson::to_bson(&state.encryption_keys.encrypt(b"fixture-key-never-dispatched").await.unwrap()).unwrap()}}).await.unwrap();
        let (_, binding) = create(
            &state.db,
            &state.encryption_keys,
            &owner,
            "Google API",
            &connection,
            Utc::now() + chrono::Duration::days(1),
            serde_json::from_str(source).unwrap(),
        )
        .await
        .unwrap();
        check_live(&state.db, &binding).await.unwrap();
        let resolved = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
            &state.db,
            &state.encryption_keys,
            &owner,
            &connection,
            None,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            authority(&resolved).unwrap(),
            binding.execution_authority_digest
        );
        let ResourceBoundary::GoogleApi { operations } = &binding.policy.resource else {
            unreachable!()
        };
        validate_execution_target(&resolved.target, Some(&operations[0].origin)).unwrap();
        assert!(validate_policy_connection(&example_policy(), &resolved).is_err());
    }
    state.db.drop().await.unwrap();
}

#[test]
fn permission_keys_google_bases_cannot_hide_recipient_changes() {
    assert_eq!(
        google_origin("https://generativelanguage.googleapis.com/v1beta").as_deref(),
        Some("https://generativelanguage.googleapis.com")
    );
    for base in [
        "https://www.googleapis.com.evil.test",
        "https://user@www.googleapis.com",
        "https://www.googleapis.com:8443",
        "https://www.googleapis.com/../v1",
        "https://www.googleapis.com/v1?key=caller",
        "https://www.googleapis.com/v1#x",
        "https://www.googleapis.com/%2fother",
        "http://www.googleapis.com",
    ] {
        assert!(google_origin(base).is_none(), "{base}");
    }
}

#[tokio::test]
async fn permission_keys_google_native_engine_retains_live_pause_hooks() {
    struct Google;
    #[async_trait]
    impl Transport for Google {
        async fn send(
            &self,
            _: &nyxid_permissions::Request,
        ) -> Result<nyxid_permissions::Response, Error> {
            panic!("origin required")
        }
        async fn send_to(
            &self,
            _: &nyxid_permissions::Request,
            origin: Option<&str>,
        ) -> Result<nyxid_permissions::Response, Error> {
            assert_eq!(origin, Some("https://www.googleapis.com"));
            Ok(nyxid_permissions::Response {
                status: 200,
                content_type: "application/json".into(),
                body: br#"{"items":[]}"#.as_slice().into(),
            })
        }
    }
    let (state, owner, connection) = fixture().await;
    let (_, binding) = create(
        &state.db,
        &state.encryption_keys,
        &owner,
        "One video",
        &connection,
        Utc::now() + chrono::Duration::days(1),
        serde_json::from_str(include_str!(
            "../../../permissions/examples/google-youtube-policy.json"
        ))
        .unwrap(),
    )
    .await
    .unwrap();
    let evaluator = engine(state.db.clone(), binding.clone(), Arc::new(Google)).unwrap();
    let mut request = nyxid_permissions::Request::get("/youtube/v3/videos");
    request.query.insert("id".into(), "VIDEO_ID".into());
    request
        .query
        .insert("part".into(), "snippet,statistics".into());
    evaluator.execute(request.clone()).await.unwrap();
    set_paused(&state.db, &owner, &binding.id, 1, true)
        .await
        .unwrap();
    assert!(matches!(
        evaluator.execute(request).await,
        Err(Error::HookDenied { .. })
    ));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn execution_permission_native_verified_plan_separates_primary_claim_from_verification() {
    for global in [false, true] {
        use crate::services::execution_permission_dispatch::{self as dispatch, phase_b_tests};
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Native {
            state: crate::AppState,
            auth: crate::mw::auth::AuthUser,
            binding: PermissionPolicy,
            drive: MockDrive,
            primary: AtomicUsize,
            verification: AtomicUsize,
        }
        #[async_trait]
        impl Transport for Native {
            async fn send(
                &self,
                request: &nyxid_permissions::Request,
            ) -> Result<nyxid_permissions::Response, Error> {
                self.verification.fetch_add(1, Ordering::SeqCst);
                assert_ne!(request.method, axum::http::Method::PATCH);
                self.drive.send(request).await
            }
            async fn send_verified(
                &self,
                verified: nyxid_permissions::VerifiedPlan<'_>,
                mark: nyxid_permissions::DispatchMark,
            ) -> Result<nyxid_permissions::Response, Error> {
                let mut auth = self.auth.clone();
                auth.execution_permission = Some(
                    auth.execution_permission
                        .as_ref()
                        .unwrap()
                        .for_native(&self.binding, &verified, mark.clone())
                        .unwrap(),
                );
                let request = &verified.plan().request;
                // Drive adds its response-fields restriction to the verified plan.
                // Pass those exact query bytes just as BackendTransport does.
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(&request.query)
                    .finish();
                let query = (!query.is_empty()).then_some(query.as_str());
                let mut target = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                    &self.state.db,
                    &self.state.encryption_keys,
                    &self.binding.user_id,
                    &self.binding.user_service_id,
                    None,
                )
                .await
                .unwrap()
                .unwrap()
                .target;
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(
                    "content-type",
                    request.content_type.as_ref().unwrap().parse().unwrap(),
                );
                let descriptor = crate::services::operation_descriptor::build_http_descriptor(
                    request.method.as_str(),
                    request.path.trim_start_matches('/'),
                    Some(request.body.as_ref()),
                );
                dispatch::admit_http(
                    &self.state,
                    &auth,
                    Some(&self.binding.user_service_id),
                    Some(&self.binding.user_id),
                    &mut target,
                    request.method.as_str(),
                    request.path.trim_start_matches('/'),
                    query,
                    &request.body,
                    &headers,
                    None,
                    &descriptor,
                    false,
                    None,
                )
                .await
                .map_err(|error| match error {
                    AppError::ApprovalRequired { request_id } => {
                        Error::ApprovalPending { request_id }
                    }
                    _ => Error::Denied("execution refused"),
                })?;
                // A gap uses the legacy personal review, not a new proposal or send.
                let Some(permit) = &target.execution_permission else {
                    return Err(Error::Denied("legacy review required"));
                };
                permit
                    .before_send(&target)
                    .await
                    .map_err(|_| Error::Denied("live admission refused"))?;
                auth.execution_permission
                    .as_ref()
                    .unwrap()
                    .native_dispatched();
                self.primary.fetch_add(1, Ordering::SeqCst);
                let response = self.drive.send(request).await;
                permit.complete(response.is_ok()).await.unwrap();
                response
            }
        }
        for outcome in ["covered", "gap", "prohibited", "proposal"] {
            let (state, owner, connection) = fixture().await;
            let (key, binding) = issue(&state, &owner, &connection).await;
            phase_b_tests::enroll_cohort(&state.db, &owner, global).await;
            let (_, key, _) = key_service::validate_api_key(&state.db, &key.full_key)
                .await
                .unwrap();
            let auth = crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None)
                .await
                .unwrap();
            state.db.collection::<Document>("service_approval_configs").insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"service_id":&binding.catalog_service_id,"service_name":"Drive","approval_required":true,"approval_mode":"per_request","rules":[],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
            if !matches!(outcome, "gap" | "proposal") {
                let mut grant = phase_b_tests::grant(
                    &state,
                    &auth,
                    &connection,
                    "PATCH",
                    "/drive/v3/files/report",
                )
                .await;
                let execution = grant.execution.as_mut().unwrap();
                execution.v1.as_mut().unwrap().resource_policy = Some(binding.policy.clone());
                if outcome == "prohibited" {
                    execution.version = 99;
                }
                state
                    .db
                    .collection("approval_grants")
                    .insert_one(grant)
                    .await
                    .unwrap();
            }
            if outcome == "proposal" {
                auth.execution_permission.as_ref().unwrap().retain_adapter(
                    crate::models::execution_proposal::PreparedAdapter::Native {
                        uri: "/api/v1/permission-execution/rest/drive/v3/files/report".into(),
                        method: "PATCH".into(),
                        body: r#"{"name":"Reviewed"}"#.into(),
                        content_type: Some("application/json".into()),
                        mcp: false,
                    },
                    &[(
                        axum::http::HeaderName::from_static("x-nyxid-approval-mode"),
                        axum::http::HeaderValue::from_static("async"),
                    )]
                    .into_iter()
                    .collect(),
                );
            }
            let native = Arc::new(Native {
                state: state.clone(),
                auth,
                binding: binding.clone(),
                drive: MockDrive::default(),
                primary: AtomicUsize::new(0),
                verification: AtomicUsize::new(0),
            });
            let native_engine = engine(state.db.clone(), binding.clone(), native.clone()).unwrap();
            let result = native_engine
                .execute(nyxid_permissions::Request {
                    method: axum::http::Method::PATCH,
                    path: "/drive/v3/files/report".into(),
                    query: Default::default(),
                    body: serde_json::to_vec(&serde_json::json!({"name":"Reviewed"}))
                        .unwrap()
                        .into(),
                    content_type: Some("application/json".into()),
                })
                .await;
            if outcome == "covered" {
                assert!(result.is_ok(), "{result:?}");
            } else if outcome == "proposal" {
                assert!(
                    matches!(result, Err(Error::ApprovalPending { .. })),
                    "{result:?}"
                );
            } else {
                assert!(matches!(result, Err(Error::Denied(_))), "{result:?}");
            }
            assert_eq!(
                native.primary.load(Ordering::SeqCst),
                usize::from(outcome == "covered")
            );
            assert!(native.verification.load(Ordering::SeqCst) > 0);
            assert_eq!(
                state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                u64::from(outcome == "proposal")
            );
            if outcome == "proposal" {
                use crate::services::execution_permission_dispatch::proposals::{self, v2};
                let row = state
                    .db
                    .collection::<crate::models::approval_request::ApprovalRequest>(
                        "approval_requests",
                    )
                    .find_one(doc! {"execution_proposal.version":2})
                    .await
                    .unwrap()
                    .unwrap();
                let proposal = v2::proposal(&row).unwrap();
                let choice = proposal
                    .choices
                    .iter()
                    .find(|c| c.id == "operation")
                    .unwrap();
                let approved = proposals::decide(
                    &state,
                    &owner,
                    &row.id,
                    &proposals::Decision {
                        approved: true,
                        expected_revision: proposal.revision,
                        choice_id: Some(choice.id.clone()),
                        choice_digest: Some(choice.digest.clone()),
                        idempotency_key: uuid::Uuid::new_v4().to_string(),
                    },
                )
                .await
                .unwrap();
                let (resume, _) = v2::claim_for_test(&state, &approved)
                    .await
                    .unwrap()
                    .unwrap();
                let mut auth = native.auth.clone();
                dispatch::RequestContext::for_resume(&mut auth, resume).unwrap();
                let replay = Arc::new(Native {
                    state: state.clone(),
                    auth,
                    binding: binding.clone(),
                    drive: MockDrive::default(),
                    primary: AtomicUsize::new(0),
                    verification: AtomicUsize::new(0),
                });
                let replay_engine =
                    engine(state.db.clone(), binding.clone(), replay.clone()).unwrap();
                let result = replay_engine
                    .execute(nyxid_permissions::Request {
                        method: axum::http::Method::PATCH,
                        path: "/drive/v3/files/report".into(),
                        query: Default::default(),
                        body: serde_json::to_vec(&serde_json::json!({"name":"Reviewed"}))
                            .unwrap()
                            .into(),
                        content_type: Some("application/json".into()),
                    })
                    .await;
                assert!(result.is_ok(), "{result:?}");
                assert_eq!(replay.primary.load(Ordering::SeqCst), 1);
                assert!(replay.verification.load(Ordering::SeqCst) > 0);
                let sent = state
                    .db
                    .collection::<crate::models::approval_request::ApprovalRequest>(
                        "approval_requests",
                    )
                    .find_one(doc! {"_id":&row.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(v2::proposal(&sent).unwrap().state, "dispatched");
                set_paused(&state.db, &owner, &binding.id, binding.revision, true)
                    .await
                    .unwrap();
                assert!(matches!(
                    v2::readback(&state, &native.auth, &sent).await,
                    Err(AppError::NotFound(_))
                ));
            }
            state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn execution_permission_native_rest_and_mcp_gaps_and_denials_exit_before_primary_send() {
    for global in [false, true] {
        use crate::services::execution_permission_dispatch::phase_b_tests;
        for denied in [false, true] {
            let (mut state, owner, connection) = fixture().await;
            let upstream = wiremock::MockServer::start().await;
            // All attempted network traffic is observable; no real Google traffic.
            state.http_client = reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(upstream.uri()).unwrap())
                .build()
                .unwrap();
            let (key, binding) = create(
                &state.db,
                &state.encryption_keys,
                &owner,
                "One video",
                &connection,
                Utc::now() + chrono::Duration::days(1),
                serde_json::from_str(include_str!(
                    "../../../permissions/examples/google-youtube-policy.json"
                ))
                .unwrap(),
            )
            .await
            .unwrap();
            phase_b_tests::enroll_cohort(&state.db, &owner, global).await;
            state.db.collection::<Document>("service_approval_configs").insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"service_id":&binding.catalog_service_id,
            "service_name":"Google","approval_required":true,"approval_mode":"per_request","rules":[],
            "default_effect":if denied {"deny"} else {"require_approval"},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
            let channel =
                crate::services::notification_service::get_or_create_channel(&state.db, &owner)
                    .await
                    .unwrap();
            state
                .db
                .collection::<Document>("notification_channels")
                .update_one(
                    doc! {"_id":channel.id},
                    doc! {"$set":{"approval_timeout_secs":1}},
                )
                .await
                .unwrap();
            let billing = axum::Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            );
            let router = Router::new()
                .route(
                    "/api/v1/permission-execution/rest/{*path}",
                    any(crate::handlers::permission_keys::rest).layer(billing),
                )
                .route(
                    "/api/v1/permission-execution/mcp",
                    post(crate::handlers::permission_keys::mcp).layer(billing),
                )
                .route(
                    "/api/v1/approvals/requests/{id}/status",
                    axum::routing::get(crate::handlers::approvals::get_request_status),
                )
                .with_state(state.clone());
            let rest = router.clone().oneshot(Request::builder().uri("/api/v1/permission-execution/rest/youtube/v3/videos?id=VIDEO_ID&part=snippet%2Cstatistics")
            .header("x-nyxid-approval-mode","async").header("authorization",format!("Bearer {}",key.full_key)).body(Body::empty()).unwrap()).await.unwrap();
            let status = rest.status();
            let text = String::from_utf8(
                to_bytes(rest.into_body(), 1024 * 1024)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
            assert!(!text.contains("uncertain"), "{text}");
            let response = router.clone().oneshot(Request::builder().method("POST").uri("/api/v1/permission-execution/mcp")
            .header("x-nyxid-approval-mode","async").header("authorization",format!("Bearer {}",key.full_key)).header("content-type","application/json")
            .body(Body::from(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"google_request","arguments":{"method":"GET","path":"/youtube/v3/videos","query":{"id":"VIDEO_ID","part":"snippet,statistics"}}}}).to_string())).unwrap()).await.unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                    .unwrap();
            assert_eq!(value["result"]["isError"], true, "{value}");
            assert!(!value.to_string().contains("uncertain"));
            assert!(upstream.received_requests().await.unwrap().is_empty());
            if !denied {
                let pending: serde_json::Value = serde_json::from_str(&text).unwrap();
                let id = pending["request_id"].as_str().unwrap();
                let receipt = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(format!("/api/v1/approvals/requests/{id}/status"))
                            .header("authorization", format!("Bearer {}", key.full_key))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(receipt.status(), StatusCode::OK);
                let receipt: serde_json::Value = serde_json::from_slice(
                    &to_bytes(receipt.into_body(), 1024 * 1024).await.unwrap(),
                )
                .unwrap();
                assert_eq!(receipt["execution"]["state"], "pending");
                assert!(receipt.get("execution_proposal").is_none());
                assert!(
                    upstream.received_requests().await.unwrap().is_empty(),
                    "polling must never dispatch"
                );
                assert!(text.contains("request_id"), "{text}");
                assert!(
                    value["result"]["execution"]["request_id"].is_string(),
                    "{value}"
                );
            }

            assert_eq!(
                state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {"execution_proposal.version":2})
                    .await
                    .unwrap(),
                if denied { 0 } else { 2 }
            );
            if denied {
                assert_eq!(
                    state
                        .db
                        .collection::<Document>("approval_requests")
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    0
                );
            }
            state.db.drop().await.unwrap();
        }
    }
}
