//! Exercise the mounted HTTP contract, including the human/CatalogEditor dispatcher.
use super::*;
use crate::{models::user::UserType, test_utils};
use axum::http::StatusCode;
use serde_json::{Value, json};

async fn single_field_updates(catalog_backed: bool) {
    let db = test_utils::connect_transaction_test_database("keys_2098").await;
    let state = test_utils::test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_utils::test_user(&owner.to_string(), UserType::Person))
        .await
        .unwrap();
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "openid profile email",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let mut create = json!({"slug":"custom-bearer", "label":"Original", "auth_method":"bearer",
        "credential":"test-only-secret", "endpoint_url":"https://example.com/original"});
    if catalog_backed {
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = "catalog-bearer".into();
        catalog.base_url = "https://example.com/original".into();
        catalog.auth_method = "bearer".into();
        catalog.auth_key_name = "Authorization".into();
        catalog.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .unwrap();
        create["service_slug"] = json!("catalog-bearer");
    }
    let request = crate::handlers::curation_tests::request;
    let (status, created) = request(&state, "POST", "/api/v1/keys", &token, Some(create)).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["catalog_service_id"].is_string(), catalog_backed);
    let path = format!("/api/v1/keys/{}", created["id"].as_str().unwrap());
    let (status, initial) = request(&state, "GET", &path, &token, None).await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    assert_eq!(initial["id"], created["id"]);
    assert_eq!(initial["endpoint_url"], created["endpoint_url"]);
    for (field, value) in [
        ("endpoint_url", json!("https://example.com/updated")),
        ("label", json!("Updated")),
        ("credential", json!("rotated-test-only-secret")),
        ("custom_user_agent", json!("test-agent/2")),
        (
            "openapi_spec_url",
            json!("https://example.com/openapi.json"),
        ),
        ("is_active", json!(false)),
        ("is_active", json!(true)),
    ] {
        let (status, updated) =
            request(&state, "PUT", &path, &token, Some(json!({field: value}))).await;
        // Read even after a failed PUT so a partial commit is visible in the assertion.
        let (read_status, saved) = request(&state, "GET", &path, &token, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{field}: PUT={updated}, stored={saved}"
        );
        assert_eq!(read_status, StatusCode::OK, "{saved}");
        assert_eq!(updated["id"], created["id"]);
        assert_eq!(updated["catalog_service_id"], created["catalog_service_id"]);
        if field != "credential" {
            assert_eq!(updated[field], value, "{field}");
            assert_eq!(saved[field], value, "{field}");
        } else {
            assert!(updated.get("credential").is_none());
        }
    }
    // Parsing and validation failures must leave the endpoint unchanged, including
    // a valid new URL accompanied by invalid metadata.
    let endpoints = db.collection::<mongodb::bson::Document>(USER_ENDPOINTS);
    let endpoint_id = created["endpoint_id"].as_str().unwrap();
    for (expected_status, body) in [
        (
            StatusCode::BAD_REQUEST,
            json!({"endpoint_url":"ftp://example.com/bad"}),
        ),
        (StatusCode::BAD_REQUEST, json!({"label":""})),
        (
            StatusCode::BAD_REQUEST,
            json!({"endpoint_url":"https://example.com/rejected", "label":""}),
        ),
        (StatusCode::UNPROCESSABLE_ENTITY, json!({"endpoint_url":42})),
    ] {
        let (before_status, before) = request(&state, "GET", &path, &token, None).await;
        assert_eq!(before_status, StatusCode::OK, "{before}");
        let before_endpoint = endpoints
            .find_one(doc! {"_id": endpoint_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            before_endpoint.get_str("url").unwrap(),
            "https://example.com/updated"
        );
        let (status, error) = request(&state, "PUT", &path, &token, Some(body)).await;
        assert_eq!(status, expected_status, "{error}");
        let (after_status, after) = request(&state, "GET", &path, &token, None).await;
        assert_eq!(after_status, StatusCode::OK, "{after}");
        for response in [&before, &after] {
            assert_eq!(response["id"], created["id"]);
            assert_eq!(
                response["endpoint_url"].as_str(),
                Some("https://example.com/updated")
            );
        }
        for field in ["endpoint_url", "label", "is_active", "updated_at"] {
            assert_eq!(after[field], before[field]);
        }
        let after_endpoint = endpoints
            .find_one(doc! {"_id": endpoint_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_endpoint, before_endpoint);
    }
    if !catalog_backed {
        assert_eq!(
            db.collection::<Value>(DOWNSTREAM_SERVICES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn keys_2098_custom_single_field_updates_return_committed_connection_without_catalog() {
    Box::pin(single_field_updates(false)).await;
}

#[tokio::test]
async fn keys_2098_catalog_single_field_updates_return_committed_connection() {
    Box::pin(single_field_updates(true)).await;
}

struct NodeMigrationFixture {
    state: AppState,
    owner: String,
    node: String,
    token: String,
    created: Value,
}
impl NodeMigrationFixture {
    async fn new() -> Self {
        let db = test_utils::connect_transaction_test_database("node_endpoint_2126").await;
        let state = test_utils::test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection("users")
            .insert_one(test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let node = uuid::Uuid::new_v4().to_string();
        db.collection::<mongodb::bson::Document>("nodes").insert_one(doc! {
            "_id": &node, "user_id": &owner, "name": "migration", "status": "online",
            "auth_token_hash": uuid::Uuid::new_v4().to_string(), "is_active": true,
            "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
        }).await.unwrap();
        let token = Self::token(&state, &owner);
        let (status, created) = crate::handlers::curation_tests::request(
            &state,
            "POST",
            "/api/v1/keys",
            &token,
            Some(json!({
                "slug":"monitor", "label":"Monitor", "auth_method":"bearer", "node_id":node,
                "endpoint_url":"http://old-monitor:8080/private?opaque=value"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["credential_type"], "node_managed");
        Self {
            state,
            owner,
            node,
            token,
            created,
        }
    }
    fn token(state: &AppState, owner: &str) -> String {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(owner).unwrap(),
            "openid profile email",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    }
    async fn put(&self, body: Value) -> (StatusCode, Value) {
        crate::handlers::curation_tests::request(
            &self.state,
            "PUT",
            &format!("/api/v1/keys/{}", self.created["id"].as_str().unwrap()),
            &self.token,
            Some(body),
        )
        .await
    }
    async fn connect(
        &self,
        capable: bool,
    ) -> tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::NodeOutboundMessage> {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        test_utils::register_test_node_connection(&self.state, &self.node, tx).await;
        self.state.node_ws_manager.record_capabilities(
            &self.node,
            &crate::services::node_ws_manager::NodeCapabilitiesMsg {
                credential_target_update_v1: capable,
                credential_ack_correlation: true,
                ..Default::default()
            },
        );
        self.state
            .node_ws_manager
            .mark_status_update_received(&self.node);
        rx
    }
}

#[tokio::test]
async fn node_endpoint_2126_offline_preserves_ids_authority_and_node_local_key() {
    let f = NodeMigrationFixture::new().await;
    let db = &f.state.db;
    let service_id = f.created["id"].as_str().unwrap();
    let key_id = f.created["api_key_id"].as_str().unwrap();
    let before_service = db
        .collection::<mongodb::bson::Document>("user_services")
        .find_one(doc! {"_id":service_id})
        .await
        .unwrap()
        .unwrap();
    let before_key = db
        .collection::<mongodb::bson::Document>("user_api_keys")
        .find_one(doc! {"_id":key_id})
        .await
        .unwrap()
        .unwrap();
    // Stand-ins for identity-bound references: migration must never rewrite them.
    let binding =
        doc! {"_id":uuid::Uuid::new_v4().to_string(), "service_id":service_id, "node_id":&f.node};
    db.collection::<mongodb::bson::Document>("node_service_bindings")
        .insert_one(binding.clone())
        .await
        .unwrap();
    let (status, updated) = f
        .put(json!({"endpoint_url":"http://new-monitor:8080/new?private=hidden"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    for field in [
        "id",
        "api_key_id",
        "endpoint_id",
        "node_id",
        "credential_type",
    ] {
        assert_eq!(updated[field], f.created[field]);
    }
    assert_eq!(updated["node_target_sync"][0]["status"], "offline");
    let mut after_service = db
        .collection::<mongodb::bson::Document>("user_services")
        .find_one(doc! {"_id":service_id})
        .await
        .unwrap()
        .unwrap();
    let mut before_service = before_service;
    // The existing service-history journal records the endpoint edit. All
    // identity, authority, routing and credential configuration stays intact.
    let last_change = after_service.remove("last_change");
    assert!(
        last_change.is_some(),
        "the committed URL edit retains its history"
    );
    before_service.remove("last_change");
    assert_eq!(after_service, before_service);
    assert_eq!(
        db.collection::<mongodb::bson::Document>("user_api_keys")
            .find_one(doc! {"_id":key_id})
            .await
            .unwrap()
            .unwrap(),
        before_key
    );
    assert_eq!(
        db.collection::<mongodb::bson::Document>("node_service_bindings")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap(),
        binding
    );
    let audits = db.collection::<mongodb::bson::Document>("audit_log");
    // Audit insertion is asynchronous, so poll only for this metadata event.
    let filter =
        doc! {"event_type":"node_managed_endpoint_updated", "event_data.service_id":service_id};
    for _ in 0..500 {
        if audits.count_documents(filter.clone()).await.unwrap() > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let audit = audits.find_one(filter).await.unwrap().unwrap();
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("old_host").unwrap(), "old-monitor");
    assert_eq!(data.get_str("new_host").unwrap(), "new-monitor");
    assert!(!data.to_string().contains("private"));
    assert!(!data.to_string().contains("hidden"));
    // Clear is special: local fallback cannot be confirmed while disconnected.
    assert_eq!(
        f.put(json!({"endpoint_url":""})).await.0,
        StatusCode::BAD_REQUEST
    );
    let after_failed_clear = db
        .collection::<mongodb::bson::Document>("user_services")
        .find_one(doc! {"_id":service_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_failed_clear.get("last_change"),
        last_change.as_ref(),
        "reserving a revision for a refused clear must not create a history event"
    );
}

#[tokio::test]
async fn node_endpoint_2126_rejects_injection_changes_and_bad_urls_without_promotion() {
    let f = NodeMigrationFixture::new().await;
    for body in [
        json!({"auth_method":"header"}),
        json!({"auth_key_name":"X-Key"}),
        json!({"node_id":""}),
        json!({"endpoint_url":"http://new-monitor", "auth_method":"bearer"}),
        json!({"endpoint_url":"http://new-monitor", "node_id":f.node}),
        json!({"endpoint_url":"ftp://bad"}),
        json!({"endpoint_url":"https://user:password@example.com"}),
    ] {
        assert_eq!(f.put(body).await.0, StatusCode::BAD_REQUEST);
    }
    let endpoint = crate::services::user_endpoint_service::get_endpoint(
        &f.state.db,
        &f.owner,
        f.created["endpoint_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(endpoint.url, f.created["endpoint_url"].as_str().unwrap());
}

#[tokio::test]
async fn node_endpoint_2126_old_agent_gets_no_metadata_frame_and_endpoint_api_matches() {
    let f = NodeMigrationFixture::new().await;
    let mut rx = f.connect(false).await;
    let (status, updated) = f.put(json!({"endpoint_url":"http://new-monitor"})).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["node_target_sync"][0]["status"], "unsupported");
    assert!(rx.try_recv().is_err());
    let path = format!(
        "/api/v1/endpoints/{}",
        f.created["endpoint_id"].as_str().unwrap()
    );
    let (status, updated) = crate::handlers::curation_tests::request(
        &f.state,
        "PUT",
        &path,
        &f.token,
        Some(json!({"url":"ssh://new-monitor:22"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["node_target_sync"][0]["status"], "unsupported");
    assert!(rx.try_recv().is_err());
    assert_eq!(
        crate::handlers::curation_tests::request(
            &f.state,
            "PUT",
            &path,
            &f.token,
            Some(json!({"url":""}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn node_endpoint_2126_capable_node_acks_metadata_and_clear_before_commit() {
    use crate::services::node_ws_manager::{CredentialAckOutcome, NodeOutboundMessage};
    let f = NodeMigrationFixture::new().await;
    let mut rx = f.connect(true).await;
    // Invalid accompanying metadata must fail before a local clear is sent.
    let (status, _) = f
        .put(json!({"endpoint_url":"", "recommended_skills":["x".repeat(4096)]}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(rx.try_recv().is_err());
    let manager = f.state.node_ws_manager.clone();
    let node = f.node.clone();
    let peer = tokio::spawn(async move {
        for expected in ["http://new-monitor", ""] {
            let Some(NodeOutboundMessage::Text(raw)) = rx.recv().await else {
                panic!("missing target frame")
            };
            let frame: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(frame["injection_method"], "target_url");
            assert_eq!(frame["target_url"], expected);
            for field in ["header_name", "header_value", "param_name", "param_value"] {
                assert!(frame.get(field).is_none());
            }
            manager.deliver_credential_ack(
                &node,
                frame["request_id"].as_str().unwrap(),
                CredentialAckOutcome::Ok,
            );
        }
    });
    for url in ["http://new-monitor", ""] {
        let (status, updated) = f.put(json!({"endpoint_url":url})).await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_eq!(updated["node_target_sync"][0]["status"], "synced");
        assert_eq!(updated["endpoint_url"], url);
    }
    peer.await.unwrap();
}

#[tokio::test]
async fn node_endpoint_2126_nonowner_and_org_viewer_cannot_retarget_either_route() {
    use crate::models::org_membership::OrgRole;
    let f = NodeMigrationFixture::new().await;
    let actor = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_utils::test_user(&actor, UserType::Person))
        .await
        .unwrap();
    let token = NodeMigrationFixture::token(&f.state, &actor);
    for org in [false, true] {
        if org {
            f.state
                .db
                .collection::<mongodb::bson::Document>("users")
                .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"user_type":"org"}})
                .await
                .unwrap();
            f.state
                .db
                .collection("org_memberships")
                .insert_one(test_utils::test_membership(
                    &f.owner,
                    &actor,
                    OrgRole::Viewer,
                    None,
                ))
                .await
                .unwrap();
        }
        for (path, body) in [
            (
                format!("/api/v1/keys/{}", f.created["id"].as_str().unwrap()),
                json!({"endpoint_url":"http://attacker"}),
            ),
            (
                format!(
                    "/api/v1/endpoints/{}",
                    f.created["endpoint_id"].as_str().unwrap()
                ),
                json!({"url":"http://attacker"}),
            ),
        ] {
            let (status, _) = crate::handlers::curation_tests::request(
                &f.state,
                "PUT",
                &path,
                &token,
                Some(body),
            )
            .await;
            assert!(matches!(
                status,
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ));
        }
    }
}

#[tokio::test]
async fn node_endpoint_2126_rejected_sync_is_reported_but_clear_does_not_commit() {
    use crate::services::node_ws_manager::{CredentialAckOutcome, NodeOutboundMessage};
    let f = NodeMigrationFixture::new().await;
    let mut rx = f.connect(true).await;
    let manager = f.state.node_ws_manager.clone();
    let node = f.node.clone();
    let peer = tokio::spawn(async move {
        for _ in 0..2 {
            let Some(NodeOutboundMessage::Text(raw)) = rx.recv().await else {
                panic!("missing target frame")
            };
            let frame: Value = serde_json::from_str(&raw).unwrap();
            manager.deliver_credential_ack(
                &node,
                frame["request_id"].as_str().unwrap(),
                CredentialAckOutcome::Err("untrusted node detail".into()),
            );
        }
    });
    let (status, value) = f.put(json!({"endpoint_url":"http://new-monitor"})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["node_target_sync"][0]["status"], "unconfirmed");
    let (status, value) = f.put(json!({"endpoint_url":""})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert!(!value.to_string().contains("untrusted node detail"));
    let ep = crate::services::user_endpoint_service::get_endpoint(
        &f.state.db,
        &f.owner,
        f.created["endpoint_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(ep.url, "http://new-monitor");
    peer.await.unwrap();
}

#[tokio::test]
async fn node_endpoint_2126_newer_migration_fences_an_older_commit() {
    use crate::services::{
        node_target_service::Migration,
        user_endpoint_service::{OpenApiSpecUrlUpdate, RecommendedSkillsUpdate},
    };
    let f = NodeMigrationFixture::new().await;
    let endpoint = f.created["endpoint_id"].as_str().unwrap();
    let old = Migration::prepare(
        &f.state.db,
        &f.owner,
        &f.owner,
        endpoint,
        "http://stale-monitor",
    )
    .await
    .unwrap();
    let new = Migration::prepare(
        &f.state.db,
        &f.owner,
        &f.owner,
        endpoint,
        "http://new-monitor",
    )
    .await
    .unwrap();
    new.commit(
        &f.state.db,
        None,
        OpenApiSpecUrlUpdate::Leave,
        RecommendedSkillsUpdate::Leave,
    )
    .await
    .unwrap();
    assert!(
        old.commit(
            &f.state.db,
            None,
            OpenApiSpecUrlUpdate::Leave,
            RecommendedSkillsUpdate::Leave
        )
        .await
        .is_err()
    );
    let ep = crate::services::user_endpoint_service::get_endpoint(&f.state.db, &f.owner, endpoint)
        .await
        .unwrap();
    assert_eq!(ep.url, "http://new-monitor");
}

#[tokio::test]
async fn node_endpoint_2126_detached_legacy_key_cannot_enable_unconfirmed_fallback() {
    let f = NodeMigrationFixture::new().await;
    f.state
        .db
        .collection::<mongodb::bson::Document>("user_services")
        .update_one(
            doc! {"_id":f.created["id"].as_str().unwrap()},
            doc! {"$unset":{"node_id":""}},
        )
        .await
        .unwrap();
    let (status, value) = f.put(json!({"endpoint_url":"http://127.0.0.1"})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["node_target_sync"][0]["status"], "unsupported");
    assert_eq!(
        f.put(json!({"endpoint_url":""})).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_2098_direct_url_put_returns_its_commit_after_competing_write_and_read_failure() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::Mutex;
    use std::time::Duration;

    let (committed, observed) = tokio::sync::oneshot::channel();
    let (resume, wait) = std::sync::mpsc::channel();
    let barrier = Mutex::new(Some((committed, wait)));
    let paused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let armed = paused.clone();
    let db = test_utils::connect_transaction_test_database_with_command_handler(
        "keys_2098_commit_snapshot",
        EventHandler::callback(move |event| {
            if let CommandEvent::Succeeded(event) = event
                && event.command_name == "commitTransaction"
                && armed.load(std::sync::atomic::Ordering::Acquire)
                && let Some((committed, wait)) = barrier.lock().unwrap().take()
            {
                committed.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(15)).unwrap();
            }
        }),
    )
    .await;
    let state = test_utils::test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let token = NodeMigrationFixture::token(&state, &owner);
    let request = crate::handlers::curation_tests::request;
    let (status, created) = request(
        &state,
        "POST",
        "/api/v1/keys",
        &token,
        Some(json!({
            "slug":"commit-snapshot", "label":"Commit snapshot", "auth_method":"bearer",
            "credential":"fixture-secret", "endpoint_url":"https://example.com/start",
            "openapi_spec_url":"https://example.com/openapi.json",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let endpoint = created["endpoint_id"].as_str().unwrap();
    let id = created["id"].as_str().unwrap();
    let path = format!("/api/v1/keys/{id}");
    let competing = mongodb::Client::with_uri_str(test_utils::selected_test_database_uri())
        .await
        .unwrap()
        .database(db.name());
    let endpoints = competing.collection::<bson::Document>(USER_ENDPOINTS);

    paused.store(true, std::sync::atomic::Ordering::Release);
    let put = tokio::spawn({
        let (state, token, path) = (state.clone(), token.clone(), path.clone());
        async move {
            request(
                &state,
                "PUT",
                &path,
                &token,
                Some(json!({
                    "endpoint_url":"https://example.com/committed",
                })),
            )
            .await
        }
    });
    let observe = async {
        tokio::time::timeout(Duration::from_secs(15), observed)
            .await
            .unwrap()
            .unwrap();
        let saved = endpoints
            .find_one(doc! {"_id":endpoint})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.get_str("url").unwrap(),
            "https://example.com/committed"
        );
        endpoints
            .update_one(
                doc! {"_id":endpoint},
                doc! {"$set":{
                    "url":"https://example.com/competing",
                }},
            )
            .await
            .unwrap();
        // A second read of this response dependency would fail after commit.
        competing
            .collection::<bson::Document>(USER_SERVICES)
            .update_one(doc! {"_id":id}, doc! {"$set":{"slug": 17_i32}})
            .await
            .unwrap();
        resume.send(()).unwrap();
    };
    observe.await;
    let (status, response) = put.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["endpoint_url"], "https://example.com/committed");
    assert_eq!(response["id"], id);
    assert_eq!(response["proxy_url"], created["proxy_url"]);
    assert_eq!(response["proxy_url_slug"], created["proxy_url_slug"]);
    assert_eq!(response["openapi_url"], created["openapi_url"]);
    let (status, _) = request(&state, "GET", &path, &token, None).await;
    assert!(status.is_server_error());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn keys_2098_prepared_edit_rejects_deleted_connection_and_stale_endpoint() {
    use crate::services::node_target_service::Migration;
    let db = test_utils::connect_transaction_test_database("keys_2098_stale_commit").await;
    let state = test_utils::test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let token = NodeMigrationFixture::token(&state, &owner);
    let (status, created) = crate::handlers::curation_tests::request(
        &state,
        "POST",
        "/api/v1/keys",
        &token,
        Some(json!({
            "slug":"prepared-edit", "label":"Prepared edit", "auth_method":"bearer",
            "credential":"fixture-secret", "endpoint_url":"https://example.com/start",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    let endpoint = created["endpoint_id"].as_str().unwrap();
    let version = created["state_version"].as_i64().unwrap();
    let endpoints = db.collection::<bson::Document>(USER_ENDPOINTS);
    for deleted in [false, true] {
        let migration =
            Migration::prepare(&db, &owner, &owner, endpoint, "https://example.com/stale")
                .await
                .unwrap();
        let expected = if deleted {
            db.collection::<bson::Document>(USER_SERVICES)
                .update_one(
                    doc! {"_id":id},
                    doc! {"$set":{"deleted_at":bson::DateTime::now()}},
                )
                .await
                .unwrap();
            "https://example.com/competing"
        } else {
            endpoints
                .update_one(
                    doc! {"_id":endpoint},
                    doc! {"$set":{
                        "url":"https://example.com/competing",
                    }},
                )
                .await
                .unwrap();
            "https://example.com/competing"
        };
        assert!(matches!(
            migration.commit_for_connection(&db, id, version).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            endpoints
                .find_one(doc! {"_id":endpoint})
                .await
                .unwrap()
                .unwrap()
                .get_str("url")
                .unwrap(),
            expected
        );
    }
    db.drop().await.unwrap();
}
