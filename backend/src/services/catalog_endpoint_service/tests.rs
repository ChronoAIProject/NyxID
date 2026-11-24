use super::*;
use crate::{
    services::{proxy_service, unified_key_service},
    test_utils::*,
};
use std::{collections::HashMap, sync::Arc};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Fixture {
    db: Database,
    catalog: DownstreamService,
    connection: UserService,
    endpoint: UserEndpoint,
    owner: String,
}
impl Fixture {
    async fn new(slug: &str) -> Self {
        let db = connect_transaction_test_database("catalog_endpoint_2374").await;
        Self::in_db(db, slug).await
    }
    async fn in_db(db: Database, slug: &str) -> Self {
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>("users")
            .insert_one(test_user(&owner, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = slug.into();
        catalog.identity_propagation_mode = "jwt".into();
        catalog.base_url = "https://current.example".into();
        catalog.openapi_spec_url =
            Some("https://current.example/agent-manager/openapi.json".into());
        db.collection::<DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        // Provision through the real pipeline, then model a catalog migration.
        unified_key_service::auto_provision_no_auth_services(&db, &owner)
            .await
            .unwrap();
        let connection = db
            .collection::<UserService>(CONNECTIONS)
            .find_one(doc! {"user_id":&owner,"catalog_service_id":&catalog.id})
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>(ENDPOINTS).update_one(doc! {"_id":&connection.endpoint_id}, doc! {"$set":{"url":"https://old.example","openapi_spec_url":"https://old.example/openapi.json"}}).await.unwrap();
        let endpoint = db
            .collection::<UserEndpoint>(ENDPOINTS)
            .find_one(doc! {"_id":&connection.endpoint_id})
            .await
            .unwrap()
            .unwrap();
        Self {
            db,
            catalog,
            connection,
            endpoint,
            owner,
        }
    }
    async fn endpoint(&self) -> UserEndpoint {
        self.db
            .collection::<UserEndpoint>(ENDPOINTS)
            .find_one(doc! {"_id":&self.endpoint.id})
            .await
            .unwrap()
            .unwrap()
    }
    async fn wait_for_target(&self, url: &str, spec: &str) -> UserEndpoint {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let endpoint = self.endpoint().await;
                if endpoint.url == url && endpoint.openapi_spec_url.as_deref() == Some(spec) {
                    return endpoint;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("background repair must converge")
    }
    async fn resolve(&self) -> proxy_service::UserServiceResolution {
        proxy_service::resolve_proxy_target_from_user_service(
            &self.db,
            &test_encryption_keys(),
            &Arc::new(crate::services::node_ws_manager::NodeWsManager::new(
                30, 100,
            )),
            &self.owner,
            Some(&self.catalog.slug),
            None,
            proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ),
        )
        .await
        .unwrap()
        .unwrap()
    }
}

#[tokio::test]
async fn stale_auto_cma_resolves_live_then_lists_repaired_target_and_spec_same_ids() {
    let f = Fixture::new("cma").await;
    let resolution = f.resolve().await;
    assert_eq!(resolution.target.base_url, f.catalog.base_url);
    assert_eq!(resolution.target.service.base_url, f.catalog.base_url);
    assert_eq!(resolution.user_service_id, f.connection.id);
    assert_eq!(
        f.endpoint().await.url,
        f.endpoint.url,
        "read-only resolution does not mutate storage"
    );
    let rows =
        unified_key_service::list_keys(&f.db, &test_encryption_keys(), &f.owner, &HashMap::new())
            .await
            .unwrap();
    let row = rows.iter().find(|row| row.id == f.connection.id).unwrap();
    assert_eq!(row.endpoint_id, f.endpoint.id);
    assert_eq!(row.endpoint_url, f.catalog.base_url);
    assert_eq!(row.openapi_spec_url, f.catalog.openapi_spec_url);
    let after =
        f.db.collection::<UserService>(CONNECTIONS)
            .find_one(doc! {"_id":&f.connection.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        bson::to_document(&after).unwrap(),
        bson::to_document(&f.connection).unwrap(),
        "no connection/key/identity version changes"
    );
    assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 0);
    let audits: Vec<Document> =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":"catalog_endpoint_reconciled"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 1);
    let encoded = serde_json::to_string(&audits).unwrap();
    assert!(!encoded.contains("https://"));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cmaeg_and_other_auto_connections_follow_catalog_moves_without_slug_rules() {
    for slug in ["cmaeg", "ordinary-new-service"] {
        let f = Fixture::new(slug).await;
        assert_eq!(f.resolve().await.target.base_url, f.catalog.base_url);
        assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 1);
        assert_eq!(f.endpoint().await.url, f.catalog.base_url);
        f.db.collection::<Document>(CATALOG)
            .update_one(
                doc! {"_id":&f.catalog.id},
                doc! {"$set":{"base_url":"https://next.example","openapi_spec_url":null}},
            )
            .await
            .unwrap();
        assert_eq!(
            reconcile(&f.db, None, Some(&f.catalog.id)).await.unwrap(),
            1
        );
        assert_eq!(f.endpoint().await.url, "https://next.example");
        assert!(f.endpoint().await.openapi_spec_url.is_none());
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn custom_edited_node_credential_and_explicit_org_rows_are_untouched() {
    for changes in [
        doc! {"source":null},
        doc! {"catalog_service_id":null},
        doc! {"node_id":"node"},
        doc! {"api_key_id":"user-key"},
        doc! {"source":"manual","user_id":"org-owner"},
    ] {
        let f = Fixture::new("cma").await;
        f.db.collection::<Document>(CONNECTIONS)
            .update_one(doc! {"_id":&f.connection.id}, doc! {"$set":changes})
            .await
            .unwrap();
        assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 0);
        assert_eq!(
            bson::to_document(&f.endpoint().await).unwrap(),
            bson::to_document(&f.endpoint).unwrap()
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn gateway_provider_and_shared_endpoint_are_never_repaired() {
    let f = Fixture::new("cmaeg").await;
    // Unknown provider is deliberately fail closed too.
    f.db.collection::<Document>(CATALOG)
        .update_one(
            doc! {"_id":&f.catalog.id},
            doc! {"$set":{"provider_config_id":"gateway"}},
        )
        .await
        .unwrap();
    assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 0);
    let mut provider = doc! {"slug":"gateway","name":"Gateway","provider_type":"api_key","is_active":true,"created_by":"system","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()};
    provider.insert("_id", "gateway");
    provider.insert("requires_gateway_url", true);
    f.db.collection::<Document>(PROVIDERS)
        .insert_one(provider)
        .await
        .unwrap();
    assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 0);
    assert_eq!(f.resolve().await.target.base_url, f.endpoint.url);
    f.db.collection::<Document>(CATALOG)
        .update_one(
            doc! {"_id":&f.catalog.id},
            doc! {"$unset":{"provider_config_id":""}},
        )
        .await
        .unwrap();
    let mut other = f.connection.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    other.source = None;
    other.slug = "explicit".into();
    f.db.collection::<UserService>(CONNECTIONS)
        .insert_one(other)
        .await
        .unwrap();
    assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 0);
    assert_eq!(f.endpoint().await.url, f.endpoint.url);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_replicas_repair_once_and_do_not_change_catalog_fingerprint() {
    let f = Fixture::new("cma").await;
    let before = crate::services::assistant_cma_gateway::catalog_execution_fingerprint(&f.catalog);
    let (a, b) = tokio::join!(reconcile(&f.db, None, None), reconcile(&f.db, None, None));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    let catalog =
        f.db.collection::<DownstreamService>(CATALOG)
            .find_one(doc! {"_id":&f.catalog.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        crate::services::assistant_cma_gateway::catalog_execution_fingerprint(&catalog),
        before
    );
    assert_eq!(f.endpoint().await.url, f.catalog.base_url);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn repaired_auto_cma_passes_admission_before_send_and_mock_proxy_200() {
    use crate::services::service_dispatch_registry as registry;
    let f = Fixture::new("cma").await;
    let server = MockServer::start().await;
    f.db.collection::<Document>(CATALOG)
        .update_one(
            doc! {"_id":&f.catalog.id},
            doc! {"$set":{"base_url":server.uri()}},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/agents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
        .expect(1)
        .mount(&server)
        .await;
    let target = f.resolve().await.target;
    let caller = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &f.owner,
        user_service_id: Some(&f.connection.id),
    };
    let admission = registry::admit(
        &f.db,
        &test_encryption_keys(),
        &cached_test_jwt_keys(),
        &test_app_config(),
        &caller,
        true,
        &target,
        "GET",
        "/api/v1/agents",
        None,
        b"",
        || Ok(vec![].into()),
        false,
    )
    .await
    .unwrap();
    let guard = registry::verify(Some(&admission), &target, Some(&caller))
        .unwrap()
        .unwrap();
    let client = reqwest::Client::new();
    let request = client
        .get(format!("{}/api/v1/agents", target.base_url))
        .build()
        .unwrap();
    guard.before_send(&request).await.unwrap();
    assert_eq!(
        guard
            .observe_response(client.execute(request).await.unwrap())
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(reconcile(&f.db, Some(&f.owner), None).await.unwrap(), 1);
    assert_eq!(f.endpoint().await.id, f.endpoint.id);
    // An explicit/custom connection must still refuse and identify itself.
    f.db.collection::<Document>(CONNECTIONS)
        .update_one(doc! {"_id":&f.connection.id}, doc! {"$set":{"source":null}})
        .await
        .unwrap();
    f.db.collection::<Document>(ENDPOINTS)
        .update_one(
            doc! {"_id":&f.endpoint.id},
            doc! {"$set":{"url":"https://custom.example"}},
        )
        .await
        .unwrap();
    let custom = f.resolve().await.target;
    assert_eq!(custom.base_url, "https://custom.example");
    let error = registry::admit(
        &f.db,
        &test_encryption_keys(),
        &cached_test_jwt_keys(),
        &test_app_config(),
        &caller,
        true,
        &custom,
        "GET",
        "/api/v1/agents",
        None,
        b"",
        || Ok(vec![].into()),
        false,
    )
    .await
    .err()
    .unwrap();
    assert!(
        matches!(&error,AppError::Conflict(message) if message.starts_with("cma_configuration_required:") && message.contains(&f.connection.id))
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_catalog_update_propagates_target_and_inherited_spec() {
    use crate::{
        handlers::services::{UpdateServiceRequest, update_service_inner},
        services::role_service,
    };
    let f = Fixture::new("cma").await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    role_service::seed_system_roles(&f.db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&f.db).await.unwrap();
    f.db.collection::<Document>("users")
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"role_ids":[roles.admin]}},
        )
        .await
        .unwrap();
    let spec = format!("{}/new/openapi.json", server.uri());
    let body: UpdateServiceRequest = serde_json::from_value(
        serde_json::json!({"base_url":server.uri(),"openapi_spec_url":spec}),
    )
    .unwrap();
    let response = update_service_inner(
        test_app_state(f.db.clone()),
        test_auth_user(&f.owner),
        crate::telemetry::TelemetryContext::default(),
        f.catalog.id.clone(),
        body,
    )
    .await
    .unwrap();
    assert_eq!(response.0.base_url, server.uri());
    let endpoint = f.wait_for_target(&server.uri(), &spec).await;
    assert_eq!(endpoint.url, server.uri());
    assert_eq!(endpoint.openapi_spec_url.as_deref(), Some(spec.as_str()));
    assert_eq!(endpoint.id, f.endpoint.id);
    assert_eq!(f.resolve().await.user_service_id, f.connection.id);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_catalog_move_and_repair_converge_to_current_target() {
    let f = Fixture::new("cma").await;
    let (repair, update) = tokio::join!(reconcile(&f.db, None, None), async {
        let catalog = f.catalog.clone();
        crate::services::transaction_service::run_db_only(&f.db,&(f.db.clone(),catalog),|tx,(db,catalog)|Box::pin(async move {
            crate::services::catalog_mutation_service::fence(db,tx,catalog,None).await?;
            db.collection::<Document>(CATALOG).update_one(doc! {"_id":&catalog.id},doc! {"$set":{"base_url":"https://newest.example","openapi_spec_url":null}}).session(tx).await?;
            Ok(())
        })).await
    });
    repair.unwrap();
    update.unwrap();
    // This is the catalog handler's post-commit reconciliation hook.
    reconcile(&f.db, None, Some(&f.catalog.id)).await.unwrap();
    assert_eq!(f.endpoint().await.url, "https://newest.example");
    assert_eq!(f.resolve().await.target.base_url, "https://newest.example");
    assert!(f.endpoint().await.openapi_spec_url.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn consistent_owner_and_paged_sweep_use_only_batched_reads() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::Mutex;
    let commands = Arc::new(Mutex::new(Vec::new()));
    let observed = commands.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            observed.lock().unwrap().push((
                event.command_name.clone(),
                collection.to_owned(),
                event.command.contains_key("autocommit"),
            ));
        }
    });
    let db =
        connect_transaction_test_database_with_command_handler("catalog_endpoint_reads", handler)
            .await;
    let f = Fixture::in_db(db, "cma").await;
    reconcile(&f.db, Some(&f.owner), None).await.unwrap();
    commands.lock().unwrap().clear();
    assert_eq!(reconcile(&f.db, Some(&f.owner), None).await.unwrap(), 0);
    assert_eq!(
        *commands.lock().unwrap(),
        vec![
            ("find".into(), CONNECTIONS.into(), false),
            ("find".into(), ENDPOINTS.into(), false),
            ("find".into(), CATALOG.into(), false),
        ]
    );
    // Cross a complete page; every row is consistent and has its own endpoint.
    let endpoint = f.endpoint().await;
    let mut endpoints = Vec::new();
    let mut connections = Vec::new();
    for _ in 0..PAGE_SIZE {
        let mut ep = endpoint.clone();
        ep.id = uuid::Uuid::new_v4().to_string();
        let mut row = f.connection.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.endpoint_id = ep.id.clone();
        row.slug = row.id.clone();
        row.source_id = Some(row.id.clone());
        endpoints.push(ep);
        connections.push(row);
    }
    f.db.collection::<UserEndpoint>(ENDPOINTS)
        .insert_many(endpoints)
        .await
        .unwrap();
    f.db.collection::<UserService>(CONNECTIONS)
        .insert_many(connections)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    assert_eq!(reconcile(&f.db, None, None).await.unwrap(), 0);
    let measured = commands.lock().unwrap().clone();
    assert_eq!(
        measured.len(),
        6,
        "two pages, three queries each: {measured:?}"
    );
    assert!(
        measured
            .iter()
            .all(|(name, _, transaction)| name == "find" && !transaction)
    );
    for collection in [CONNECTIONS, ENDPOINTS, CATALOG] {
        assert_eq!(
            measured.iter().filter(|(_, c, _)| c == collection).count(),
            2
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn failed_endpoint_repair_does_not_fail_key_listing() {
    let f = Fixture::new("cma").await;
    // A database-local validator rejects the endpoint mutation; no global
    // failpoint or test-only success path masks the actual transaction failure.
    f.db.run_command(doc! {"collMod":ENDPOINTS,"validator":{"url":&f.endpoint.url},"validationLevel":"strict","validationAction":"error"}).await.unwrap();
    assert!(reconcile(&f.db, Some(&f.owner), None).await.is_err());
    let rows =
        unified_key_service::list_keys(&f.db, &test_encryption_keys(), &f.owner, &HashMap::new())
            .await
            .expect("maintenance failure cannot break key listing");
    assert!(rows.iter().any(|row| row.id == f.connection.id));
    assert_eq!(
        f.endpoint().await.url,
        f.endpoint.url,
        "failed transaction must not change storage"
    );
    assert_eq!(
        f.resolve().await.target.base_url,
        f.catalog.base_url,
        "live target repair still applies"
    );
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"catalog_endpoint_reconciled"})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_response_returns_while_repair_is_paused_then_rows_converge() {
    use crate::{
        handlers::services::{UpdateServiceRequest, update_service_inner},
        services::role_service,
    };
    let f = Fixture::new("cma").await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    role_service::seed_system_roles(&f.db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&f.db).await.unwrap();
    f.db.collection::<Document>("users")
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"role_ids":[roles.admin]}},
        )
        .await
        .unwrap();
    let spec = format!("{}/new/openapi.json", server.uri());
    let body: UpdateServiceRequest = serde_json::from_value(
        serde_json::json!({"base_url":server.uri(),"openapi_spec_url":spec}),
    )
    .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        BACKGROUND_PAUSE.scope(
            (entered.clone(), release.clone()),
            update_service_inner(
                test_app_state(f.db.clone()),
                test_auth_user(&f.owner),
                crate::telemetry::TelemetryContext::default(),
                f.catalog.id.clone(),
                body,
            ),
        ),
    )
    .await
    .expect("admin response must not await repair")
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("background repair spawned");
    assert_eq!(response.0.base_url, server.uri());
    assert_eq!(
        f.endpoint().await.url,
        f.endpoint.url,
        "repair is still paused after response"
    );
    assert_eq!(
        f.resolve().await.target.base_url,
        server.uri(),
        "live resolution closes the background window"
    );
    release.notify_one();
    let endpoint = f.wait_for_target(&server.uri(), &spec).await;
    assert_eq!(endpoint.id, f.endpoint.id);
    f.db.drop().await.unwrap();
}
