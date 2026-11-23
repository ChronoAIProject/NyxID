// Reuse the proxy resolver's real catalog/custom connection fixtures.
#[tokio::test]
async fn shutdown_service_labels_use_resolved_catalog_identity_for_slug_and_uuid_routes() {
    use tower::ServiceExt;
    let db = crate::test_utils::connect_transaction_test_database("drain_service_labels").await;
    let actor = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    crate::test_utils::set_test_approval_preference(&db, &actor, false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/{*path}",
                any(|| async {
                    Response::builder()
                        .header("content-type", "text/event-stream; charset=utf-8")
                        .body(Body::from_stream(
                            futures::stream::once(async {
                                Ok::<_, io::Error>(Bytes::from_static(b"data: ready\n\n"))
                            })
                            .chain(futures::stream::pending()),
                        ))
                        .unwrap()
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "catalog-drain-fixture".into();
    catalog.base_url = base.clone();
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    let linked = insert_user_service(
        &db,
        &actor,
        "private-catalog-alias",
        &base,
        Some(&catalog.id),
    )
    .await;
    let custom = insert_user_service(&db, &actor, "private-custom-slug", &base, None).await;
    let state = test_app_state(db.clone());
    let auth = crate::test_utils::test_auth_user(&actor);
    let slug_auth = auth.clone();
    let app = Router::new()
        .route(
            "/api/v1/proxy/s/{slug}/{*path}",
            post(
                move |State(state): State<AppState>,
                      Path((slug, path)): Path<(String, String)>,
                      request: Request<Body>| {
                    let auth = slug_auth.clone();
                    async move {
                        proxy_request_by_slug_inner(
                            &state,
                            &auth,
                            &slug,
                            &path,
                            request,
                            &mut String::new(),
                        )
                        .await
                    }
                },
            ),
        )
        .route(
            "/api/v1/proxy/{service_id}/{*path}",
            post(
                move |State(state): State<AppState>,
                      Path((id, path)): Path<(String, String)>,
                      request: Request<Body>| {
                    let auth = auth.clone();
                    async move {
                        proxy_request_inner(&state, &auth, &id, &path, request, &mut String::new())
                            .await
                    }
                },
            ),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ))
        .with_state(state.clone());
    let mut responses = Vec::new();
    for path in [
        format!("s/{}", linked.slug),
        format!("s/{}", custom.slug),
        catalog.id.clone(),
    ] {
        let response = app
            .clone()
            .oneshot(proxy_json_request(
                &format!("/api/v1/proxy/{path}/events"),
                "{}",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        responses.push(response);
    }
    let summary = serde_json::to_value(state.shutdown.open_http()).unwrap();
    let entries = summary["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        3,
        "catalog and custom streams must be separate groups"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e["service"] == catalog.slug)
            .count(),
        2
    );
    assert_eq!(
        entries.iter().filter(|e| e["service"] == "custom").count(),
        1
    );
    assert!(
        entries
            .iter()
            .all(|e| e["content_type"] == "event-stream" && e["kind"] == "stream")
    );
    let encoded = summary.to_string();
    for private in [
        &actor,
        &catalog.id,
        &linked.id,
        &linked.slug,
        &custom.id,
        &custom.slug,
    ] {
        assert!(!encoded.contains(private));
    }
    drop(responses);
    assert!(state.shutdown.open_http().entries.is_empty());
    upstream.abort();
    db.drop().await.unwrap();
}

async fn shutdown_catalog_actors(db: &mongodb::Database) -> (String, String, String) {
    use crate::services::role_service;
    role_service::seed_system_roles(db).await.unwrap();
    let roles = role_service::get_platform_role_ids(db).await.unwrap();
    let manager_role = db
        .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
        .find_one(doc! {"slug": crate::models::role::CATALOG_MANAGER_ROLE_SLUG})
        .await
        .unwrap()
        .unwrap();
    let actor = Uuid::new_v4().to_string();
    let admin = Uuid::new_v4().to_string();
    let manager = Uuid::new_v4().to_string();
    let mut admin_user = test_user(&admin, UserType::Person);
    admin_user.role_ids.push(roles.admin);
    let mut manager_user = test_user(&manager, UserType::Person);
    manager_user
        .role_ids
        .extend([roles.operator, manager_role.id]);
    db.collection(USERS)
        .insert_many([
            test_user(&actor, UserType::Person),
            admin_user,
            manager_user,
        ])
        .await
        .unwrap();
    (actor, admin, manager)
}

// Snapshot inside the real handler's caller, while the middleware request guard
// is still open after a successful resolution and response.
async fn shutdown_public_catalog_privacy_route(kind: &'static str) {
    use crate::models::downstream_service::{AnonymousEndpointRule, DownstreamService};
    use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
    use tower::ServiceExt;
    let db = crate::test_utils::connect_transaction_test_database("drain_catalog_privacy").await;
    let (actor, admin, manager) = shutdown_catalog_actors(&db).await;
    crate::test_utils::set_test_approval_preference(&db, &actor, false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/{*path}",
                any(|| async { Json(serde_json::json!({"ok":true})) }),
            ),
        )
        .await
        .unwrap();
    });
    let provider = Uuid::new_v4().to_string();
    db.collection::<bson::Document>("provider_configs").insert_one(doc! {
        "_id": &provider, "slug":"openai", "name":"OpenAI", "provider_type":"api_key",
        "is_active":true, "created_by":"system", "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
    }).await.unwrap();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "llm-readable-customer-project".into();
    catalog.base_url = base.clone();
    catalog.provider_config_id = Some(provider);
    catalog.anonymous_endpoints = vec![AnonymousEndpointRule {
        id: Uuid::new_v4().to_string(),
        enabled: true,
        method: "POST".into(),
        path_pattern: "/perform".into(),
        daily_quota: 100,
    }];
    let linked = insert_user_service(&db, &actor, "private-alias", &base, Some(&catalog.id)).await;
    if kind == "uuid_direct" {
        // Keep the UUID fallback on a historical explicit connection.
        db.collection::<crate::models::user_service_connection::UserServiceConnection>(
            crate::models::user_service_connection::COLLECTION_NAME,
        ).insert_one(crate::models::user_service_connection::UserServiceConnection {
            id: Uuid::new_v4().to_string(),
            user_id: actor.clone(),
            service_id: catalog.id.clone(),
            credential_encrypted: None,
            secret_context_versions: Default::default(),
            credential_type: None,
            credential_label: None,
            metadata: None,
            is_active: true,
            state_version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }).await.unwrap();
        db.collection::<bson::Document>("user_services")
            .delete_one(doc! {"_id":&linked.id})
            .await
            .unwrap();
    }
    if kind == "pool" {
        db.collection::<bson::Document>("service_pools").insert_one(doc! {
            "_id":Uuid::new_v4().to_string(), "user_id":&actor, "slug":"private-pool", "name":"Pool",
            "strategy":"priority", "member_contract":"same_api", "config_revision":0_i64, "is_active":true,
            "members":[{"user_service_id":&linked.id,"enabled":true,"weight":1,"priority":0}],
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).await.unwrap();
    }
    let state = test_app_state(db.clone());
    let auth = crate::test_utils::test_auth_user(&actor);
    let (template, address) = match kind {
        "slug" => (
            "/api/v1/proxy/s/{slug}/{*path}",
            format!("/api/v1/proxy/s/{}/perform", linked.slug),
        ),
        "uuid" | "uuid_direct" => (
            "/api/v1/proxy/{service_id}/{*path}",
            format!("/api/v1/proxy/{}/perform", catalog.id),
        ),
        "pool" => (
            "/api/v1/proxy/s/{slug}/{*path}",
            "/api/v1/proxy/s/private-pool/perform".into(),
        ),
        "public" => (
            "/public/s/{slug}/{*path}",
            format!("/public/s/{}/perform", catalog.slug),
        ),
        "llm_provider" => (
            "/api/v1/llm/{provider_slug}/v1/{*path}",
            "/api/v1/llm/openai/v1/perform".into(),
        ),
        "llm_gateway" => (
            "/api/v1/llm/gateway/v1/{*path}",
            "/api/v1/llm/gateway/v1/chat/completions".into(),
        ),
        _ => unreachable!(),
    };
    let observed = Arc::new(Mutex::new(None));
    let capture = observed.clone();
    let id = catalog.id.clone();
    let slug = catalog.slug.clone();
    let alias = linked.slug.clone();
    let app = Router::new()
        .route(
            template,
            post(
                move |State(state): State<AppState>, mut request: Request<Body>| {
                    let (auth, capture, id, slug, alias) = (
                        auth.clone(),
                        capture.clone(),
                        id.clone(),
                        slug.clone(),
                        alias.clone(),
                    );
                    async move {
                        request.extensions_mut().insert(match kind {
                            "public" => BillingRoutePolicy::Exempt("anonymous proxy"),
                            "llm_provider" => {
                                BillingRoutePolicy::Metered(BillingIngress::LlmProvider)
                            }
                            "llm_gateway" => {
                                BillingRoutePolicy::Metered(BillingIngress::LlmGateway)
                            }
                            _ => BillingRoutePolicy::Metered(BillingIngress::Proxy),
                        });
                        let response = match kind {
                            "slug" | "pool" => {
                                Box::pin(proxy_request_by_slug_inner(
                                    &state,
                                    &auth,
                                    if kind == "pool" {
                                        "private-pool"
                                    } else {
                                        &alias
                                    },
                                    "perform",
                                    request,
                                    &mut String::new(),
                                ))
                                .await
                            }
                            "uuid" | "uuid_direct" => {
                                Box::pin(proxy_request_inner(
                                    &state,
                                    &auth,
                                    &id,
                                    "perform",
                                    request,
                                    &mut String::new(),
                                ))
                                .await
                            }
                            "public" => {
                                Box::pin(crate::handlers::public_proxy::public_proxy_request(
                                    State(state.clone()),
                                    axum::extract::ConnectInfo("127.0.0.1:12345".parse().unwrap()),
                                    Path((slug, "perform".into())),
                                    request,
                                ))
                                .await
                            }
                            "llm_provider" => {
                                Box::pin(crate::handlers::llm_gateway::llm_proxy_request(
                                    State(state.clone()),
                                    auth,
                                    Path(("openai".into(), "perform".into())),
                                    request,
                                ))
                                .await
                            }
                            "llm_gateway" => {
                                crate::handlers::llm_gateway::gateway_request(
                                    State(state.clone()),
                                    auth,
                                    Path("chat/completions".into()),
                                    request,
                                )
                                .await
                            }
                            _ => unreachable!(),
                        };
                        *capture.lock().unwrap() = Some(state.shutdown.open_http());
                        response
                    }
                },
            ),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.shutdown.clone(),
            crate::mw::shutdown::track,
        ))
        .with_state(state.clone());
    let mut failures = Vec::new();
    for (case, visibility, owner, creator, expected) in [
        (
            "seeded public",
            "public",
            None,
            "system",
            catalog.slug.as_str(),
        ),
        (
            "admin-created public",
            "public",
            None,
            admin.as_str(),
            catalog.slug.as_str(),
        ),
        (
            "manager-created public",
            "public",
            None,
            manager.as_str(),
            catalog.slug.as_str(),
        ),
        (
            "legacy creator published",
            "public",
            None,
            actor.as_str(),
            catalog.slug.as_str(),
        ),
        ("private", "private", None, actor.as_str(), "custom"),
        (
            "owned public",
            "public",
            Some(actor.as_str()),
            actor.as_str(),
            "custom",
        ),
        (
            "transferred public",
            "public",
            Some(actor.as_str()),
            admin.as_str(),
            "custom",
        ),
    ] {
        catalog.visibility = visibility.into();
        catalog.owner_user_id = owner.map(str::to_owned);
        catalog.created_by = creator.into();
        catalog.catalog_manager_created = creator == manager;
        db.collection::<DownstreamService>("downstream_services")
            .replace_one(doc! {"_id":&catalog.id}, &catalog)
            .upsert(true)
            .await
            .unwrap();
        let request = Request::builder()
            .method(Method::POST)
            .uri(&address)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"model":"gpt-4o","messages":[]}"#))
            .unwrap();
        // The snapshot is taken inside the handler, while its guard is live.
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        assert_eq!(status, StatusCode::OK, "{kind}: {case}");
        to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let summary = observed.lock().unwrap().take().unwrap();
        assert_eq!(summary.entries.len(), 1, "{kind}: {case}, status={status}");
        let entry = &summary.entries[0];
        assert_eq!(entry.kind, "request");
        if entry.service.as_deref() != Some(expected) {
            failures.push(format!(
                "{case}: {:?} instead of {expected}, status={status}",
                entry.service
            ));
        }
        let encoded = serde_json::to_string(&summary).unwrap();
        assert!(
            !encoded.contains(&actor)
                && !encoded.contains(&admin)
                && !encoded.contains(&manager)
                && !encoded.contains(&catalog.id)
                && !encoded.contains(&linked.slug)
        );
    }
    upstream.abort();
    db.drop().await.unwrap();
    assert!(failures.is_empty(), "{kind}: {}", failures.join("; "));
}

#[tokio::test]
async fn shutdown_public_catalog_privacy_slug_route() {
    Box::pin(shutdown_public_catalog_privacy_route("slug")).await;
}
#[tokio::test]
async fn shutdown_public_catalog_privacy_uuid_route() {
    Box::pin(shutdown_public_catalog_privacy_route("uuid")).await;
}
#[tokio::test]
async fn shutdown_public_catalog_privacy_public_route() {
    Box::pin(shutdown_public_catalog_privacy_route("public")).await;
}
#[tokio::test]
async fn shutdown_public_catalog_privacy_llm_provider_route() {
    Box::pin(shutdown_public_catalog_privacy_route("llm_provider")).await;
}
#[tokio::test]
async fn shutdown_public_catalog_privacy_llm_gateway_route() {
    Box::pin(shutdown_public_catalog_privacy_route("llm_gateway")).await;
}
#[tokio::test]
async fn shutdown_public_catalog_privacy_pool_route() {
    Box::pin(shutdown_public_catalog_privacy_route("pool")).await;
}

#[tokio::test]
async fn shutdown_public_catalog_privacy_uuid_direct_route() {
    Box::pin(shutdown_public_catalog_privacy_route("uuid_direct")).await;
}

// Listing parity includes the stricter diagnostic exclusions for explicit owners
// and empty/UUID-shaped slugs; public owned rows remain discoverable.
#[tokio::test]
async fn shutdown_public_catalog_listing_parity() {
    use crate::models::downstream_service::{
        DownstreamService, PlatformKeyConfig, PlatformKeyPurpose,
    };
    use crate::services::shutdown::{Runtime, diagnostics};
    use crate::telemetry::TelemetryContext;
    use tower::ServiceExt;
    let db = crate::test_utils::connect_transaction_test_database("drain_listing_parity").await;
    let (actor, admin, manager) = shutdown_catalog_actors(&db).await;
    let reader = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&reader, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let runtime = Runtime::default();
    let mut failures = Vec::new();
    for (case, expected_listed) in [
        ("admin-created-public", true),
        ("manager-created-public", true),
        ("seeded-public", true),
        ("internal-public", true),
        ("inactive", false),
        ("platform-internal", false),
        ("other-category", false),
        ("model-route", false),
        ("model-offering", false),
        ("private", false),
        ("owned", true),
        ("transferred", true),
        ("legacy-creator-published", true),
        ("empty-slug", true),
        ("uuid-slug", true),
    ] {
        let mut service = crate::test_utils::test_auto_connected_catalog_service();
        service.slug = case.into();
        service.base_url = "http://127.0.0.1:9".into();
        match case {
            "admin-created-public" => service.created_by = admin.clone(),
            "manager-created-public" => {
                service.created_by = manager.clone();
                service.catalog_manager_created = true;
            }
            "internal-public" | "platform-internal" => service.service_category = "internal".into(),
            "inactive" => service.is_active = false,
            "other-category" => service.service_category = "other".into(),
            "model-route" | "model-offering" => {
                service.platform_key = Some(PlatformKeyConfig {
                    purpose: if case == "model-route" {
                        PlatformKeyPurpose::ModelRoute
                    } else {
                        PlatformKeyPurpose::ModelOffering
                    },
                    ..Default::default()
                })
            }
            "private" | "legacy-creator-published" => {
                service.created_by = actor.clone();
                service.visibility = "private".into();
            }
            "owned" => {
                service.created_by = actor.clone();
                service.owner_user_id = Some(actor.clone());
            }
            "transferred" => {
                service.created_by = admin.clone();
                service.owner_user_id = Some(actor.clone());
            }
            "empty-slug" => service.slug.clear(),
            "uuid-slug" => service.slug = Uuid::new_v4().to_string(),
            _ => {}
        }
        db.collection::<DownstreamService>("downstream_services")
            .insert_one(&service)
            .await
            .unwrap();
        if case == "legacy-creator-published" {
            let Json(_) = crate::handlers::services::update_service_request(
                State(state.clone()),
                crate::test_utils::test_auth_user(&actor),
                TelemetryContext::default(),
                Path(service.id.clone()),
                Json(serde_json::json!({"visibility":"public"})),
            )
            .await
            .unwrap();
            service = db
                .collection::<DownstreamService>("downstream_services")
                .find_one(doc! {"_id":&service.id})
                .await
                .unwrap()
                .unwrap();
        }
        let entries = crate::handlers::catalog::list_catalog(
            State(state.clone()),
            crate::test_utils::test_auth_user(&reader),
            TelemetryContext::default(),
            axum::extract::Query(crate::handlers::catalog::CatalogListQuery {
                offering_kind: None,
                include_all: true,
            }),
        )
        .await
        .unwrap()
        .0
        .entries;
        let listed = entries.iter().any(|entry| entry.slug == service.slug);
        assert_eq!(listed, expected_listed, "listing: {case}");
        let expected = if listed
            && service.owner_user_id.is_none()
            && !service.slug.is_empty()
            && Uuid::try_parse(&service.slug).is_err()
        {
            service.slug.as_str()
        } else {
            "custom"
        };
        let resolved = service.clone();
        let app = Router::new()
            .route(
                "/api/v1/proxy/s/{slug}/{*path}",
                get(move || {
                    let resolved = resolved.clone();
                    async move {
                        diagnostics::resolved_catalog(&resolved);
                        Body::from_stream(futures::stream::pending::<
                            Result<Bytes, std::convert::Infallible>,
                        >())
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                runtime.clone(),
                crate::mw::shutdown::track,
            ));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/proxy/s/private-alias/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let summary = runtime.open_http();
        assert_eq!(summary.entries.len(), 1, "{case}");
        let entry = &summary.entries[0];
        assert_eq!(entry.kind, "stream");
        if entry.service.as_deref() != Some(expected) {
            failures.push(format!(
                "{case}: listed={listed}, label={:?}, expected={expected}",
                entry.service
            ));
        }
        drop(response);
        assert!(runtime.open_http().entries.is_empty());
        db.collection::<DownstreamService>("downstream_services")
            .delete_one(doc! {"_id":&service.id})
            .await
            .unwrap();
    }
    db.drop().await.unwrap();
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
