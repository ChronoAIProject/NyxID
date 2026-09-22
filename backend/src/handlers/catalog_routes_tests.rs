use crate::{
    AppState,
    crypto::jwt,
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        downstream_service::{
            COLLECTION_NAME as CATALOG, DownstreamService, PlatformKeyAudience, PlatformKeyConfig,
        },
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
        user::{COLLECTION_NAME as USERS, User, UserType},
        user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    services::{
        api_docs_service::{SpecCacheTestGuard, cache_test_spec},
        key_service::{self, CreatedApiKey},
        mcp_service,
    },
    test_utils::{
        connect_test_database, connect_test_database_with_command_handler, test_app_state,
        test_auto_connected_catalog_service, test_membership, test_user, test_user_endpoint,
        test_user_service,
    },
};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use mongodb::{
    bson::{Document, doc},
    event::{EventHandler, command::CommandEvent},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    state: AppState,
    owner: String,
}

struct KeySpec<'a> {
    allowed: Option<&'a [String]>,
    allow_auto_connected: bool,
    allow_all_nodes: bool,
    platform: Option<&'a str>,
}
impl Default for KeySpec<'_> {
    fn default() -> Self {
        Self {
            allowed: None,
            allow_auto_connected: false,
            allow_all_nodes: true,
            platform: None,
        }
    }
}
impl Fixture {
    async fn new(name: &str) -> Option<Self> {
        Some(Self::with_db(connect_test_database(name).await?).await)
    }
    async fn with_db(db: mongodb::Database) -> Self {
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let owner = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        Self {
            state: test_app_state(db),
            owner,
        }
    }
    async fn key(&self, spec: KeySpec<'_>) -> CreatedApiKey {
        key_service::create_api_key_with_scope_authorization(
            &self.state.db,
            &self.owner,
            /* scope_actor_user_id */ Some(&self.owner),
            "Catalog discovery",
            "read proxy",
            /* expires_at */ None,
            /* description */ None,
            spec.allowed,
            /* allowed_node_ids */ None,
            /* allow_all_services */ Some(spec.allowed.is_none()),
            /* allow_auto_connected_services */ Some(spec.allow_auto_connected),
            /* allow_all_nodes */ Some(spec.allow_all_nodes),
            /* rate_limit_per_second */ None,
            /* rate_limit_burst */ None,
            spec.platform,
            /* callback_url */ None,
            /* scope_plan_digest */ None,
        )
        .await
        .unwrap()
    }
    async fn catalog(&self, slug: &str, private: bool) -> DownstreamService {
        let mut service = test_auto_connected_catalog_service();
        service.slug = slug.to_string();
        service.requires_user_credential = true;
        service.visibility = if private { "private" } else { "public" }.to_string();
        service.openapi_spec_url = Some(format!("https://example.com/{}/openapi.json", service.id));
        service.inference = crate::services::inference_service::default_inference("llm-openai");
        service.recommended_skills = Some(vec!["template-skill".to_string()]);
        self.state
            .db
            .collection::<DownstreamService>(CATALOG)
            .insert_one(&service)
            .await
            .unwrap();
        // Public overlay cache is keyed by the stored admin URL, not the actor.
        // A private instance spec below uses the separate owner-scoped cache.
        cache_test_spec(
            service.openapi_spec_url.as_ref().unwrap(),
            None,
            serde_json::from_str(include_str!("../../specs/catalog/github.openapi.json")).unwrap(),
        );
        service
    }
    async fn mount(
        &self,
        owner: &str,
        slug: &str,
        catalog: Option<&DownstreamService>,
    ) -> UserService {
        let id = Uuid::new_v4().to_string();
        let spec_url = format!("https://example.com/{id}/instance.json");
        let mut endpoint = test_user_endpoint(
            &id,
            owner,
            slug,
            "https://example.com",
            Some(&spec_url),
            catalog.map(|s| s.id.as_str()),
        );
        endpoint.recommended_skills = Some(vec![format!("private-{slug}")]);
        self.state
            .db
            .collection::<UserEndpoint>(ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        let service = test_user_service(
            &Uuid::new_v4().to_string(),
            owner,
            slug,
            &id,
            catalog.map(|s| s.id.as_str()),
            None,
        );
        self.state
            .db
            .collection::<UserService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        cache_test_spec(
            &spec_url,
            Some(owner),
            json!({
                "openapi": "3.1.0", "info": {"title": "Private instance", "version": "1"},
                "paths": {"/instance": {"get": {"operationId": "private_instance_op", "responses": {"200": {"description": "ok"}}}}}
            }),
        );
        service
    }
}
fn headers(token: &str, bearer: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if bearer {
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    } else {
        headers.insert("x-api-key", token.parse().unwrap());
    }
    headers
}
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: Value,
) -> (StatusCode, Value) {
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let mut req = Request::builder().method(method).uri(path);
    *req.headers_mut().unwrap() = headers.clone();
    let response = private
        .with_state(state.clone())
        .oneshot(
            req.header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn get(f: &Fixture, path: &str, headers: &HeaderMap) -> (StatusCode, Value) {
    request(&f.state, "GET", path, headers, Value::Null).await
}
fn paths(slug: &str) -> [String; 3] {
    [
        "/api/v1/catalog".to_string(),
        format!("/api/v1/catalog/{slug}"),
        format!("/api/v1/catalog/{slug}/endpoints"),
    ]
}
async fn discover(f: &Fixture, key: &CreatedApiKey) -> Value {
    let (status, response) = request(&f.state, "POST", "/mcp", &headers(&key.full_key, false),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "nyx__discover_services", "arguments": {}}})).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn real_key_formats_read_all_catalog_routes_and_cached_overlay_operations() {
    let Some(f) = Fixture::new("catalog_routes_real_keys").await else {
        return;
    };
    let _cache = SpecCacheTestGuard::acquire();
    let catalog = f.catalog("github", false).await;
    for (prefix, allow_all_nodes, platform) in
        [("nyx_", true, None), ("nyxid_ag_", false, Some("codex"))]
    {
        let key = f
            .key(KeySpec {
                allow_all_nodes,
                platform,
                ..Default::default()
            })
            .await;
        assert!(key.full_key.starts_with(prefix));
        // Catalog reads do not require proxy scope.
        f.state
            .db
            .collection::<ApiKey>(KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": {"scopes": "read"}})
            .await
            .unwrap();
        for bearer in [false, true] {
            for path in paths(&catalog.slug) {
                let (status, body) = get(&f, &path, &headers(&key.full_key, bearer)).await;
                assert_eq!(status, StatusCode::OK, "{path}: {body}");
                if path.ends_with("/endpoints") {
                    assert!(body["endpoints"].as_array().unwrap().len() > 1);
                    assert!(
                        body["endpoints"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|op| op["path"].as_str().unwrap().contains("/repos/"))
                    );
                }
            }
        }
        let (status, body) = request(
            &f.state,
            "POST",
            "/api/v1/services",
            &headers(&key.full_key, false),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("API keys cannot access this endpoint")
        );
    }
}

#[tokio::test]
async fn restricted_keys_hide_instances_but_keep_unconnected_template_metadata() {
    let Some(f) = Fixture::new("catalog_routes_scope").await else {
        return;
    };
    let _cache = SpecCacheTestGuard::acquire();
    let visible = f.catalog("allowed", false).await;
    let hidden = f.catalog("hidden", false).await;
    let allowed = f.mount(&f.owner, "allowed-instance", Some(&visible)).await;
    let ids = [allowed.id.clone()];
    let key = f
        .key(KeySpec {
            allowed: Some(&ids),
            ..Default::default()
        })
        .await;
    let h = headers(&key.full_key, true);
    let before = get(&f, "/api/v1/catalog/hidden", &h).await;
    let before_list = get(&f, "/api/v1/catalog?include_all=true", &h).await;
    let hidden_instance = f.mount(&f.owner, "hidden-instance", Some(&hidden)).await;
    // Compare every field, including inference binding/status_slug, platform
    // availability, resource_uri and skill names/refs/digests. These are template
    // metadata/live grants, not instance overrides or connected-state flags.
    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(before, get(&f, "/api/v1/catalog/hidden", &h).await);
    assert_eq!(
        before_list,
        get(&f, "/api/v1/catalog?include_all=true", &h).await
    );
    assert_eq!(
        get(&f, "/api/v1/catalog/allowed-instance/endpoints", &h)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        get(&f, "/api/v1/catalog/hidden-instance/endpoints", &h)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, keys) = get(&f, "/api/v1/keys", &h).await;
    assert_eq!(status, StatusCode::OK, "{keys}");
    assert!(keys.to_string().contains(&allowed.id));
    assert!(!keys.to_string().contains(&hidden_instance.id));
    let discovered = discover(&f, &key).await;
    let services = discovered["services"].as_array().unwrap();
    assert!(services.iter().any(|s| s["slug"] == "hidden"));
    assert!(!services.iter().any(|s| s["slug"] == "allowed"));
    // A private catalog row must not use an out-of-scope instance to authorize
    // either details or endpoint discovery, even when its slug is known.
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .update_many(doc! {}, doc! {"$set": {"visibility": "private"}})
        .await
        .unwrap();
    // List visibility has no instance join, even for the allowed private row.
    for path in ["/api/v1/catalog", "/api/v1/catalog?include_all=true"] {
        let (status, body) = get(&f, path, &h).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["entries"], json!([]), "{body}");
    }
    for suffix in ["", "/endpoints"] {
        let (status, body) = get(&f, &format!("/api/v1/catalog/hidden{suffix}"), &h).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        let (status, body) = get(&f, &format!("/api/v1/catalog/allowed{suffix}"), &h).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert!(
        discover(&f, &key).await["services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn org_owned_and_personal_keys_use_inventory_membership_and_effective_scopes() {
    let Some(mut f) = Fixture::new("catalog_routes_org").await else {
        return;
    };
    let _cache = SpecCacheTestGuard::acquire();
    let person = f.owner.clone();
    let mut services = vec![];
    for (slug, role) in [
        ("member", OrgRole::Member),
        ("admin", OrgRole::Admin),
        ("viewer", OrgRole::Viewer),
    ] {
        let org = Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        let catalog = f.catalog(slug, false).await;
        let service = f
            .mount(&org, &format!("{slug}-instance"), Some(&catalog))
            .await;
        f.state
            .db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(
                &org,
                &person,
                if role == OrgRole::Viewer {
                    OrgRole::Member
                } else {
                    role
                },
                Some(vec![service.id.clone()]),
            ))
            .await
            .unwrap();
        services.push(service);
    }
    // Unrestricted keys match sessions with org, no-auth and private templates.
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .insert_one(test_auto_connected_catalog_service())
        .await
        .unwrap();
    f.catalog("private-template", true).await;
    let unrestricted = f.key(KeySpec::default()).await;
    assert_eq!(
        discover(&f, &unrestricted).await,
        mcp_service::discover_services(&f.state.db, &f.owner, None, None)
            .await
            .unwrap()
    );
    let ids: Vec<String> = services.iter().map(|s| s.id.clone()).collect();
    let key = f
        .key(KeySpec {
            allowed: Some(&ids),
            ..Default::default()
        })
        .await;
    // The key was validly granted while the actor was a Member. A live
    // downgrade to Viewer must hide inventory despite that persisted grant.
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id": &services[2].user_id},
            doc! {"$set": {"role": "viewer"}},
        )
        .await
        .unwrap();
    let h = headers(&key.full_key, false);
    for (slug, expected) in [
        ("member", StatusCode::OK),
        ("admin", StatusCode::OK),
        ("viewer", StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            get(
                &f,
                &format!("/api/v1/catalog/{slug}-instance/endpoints"),
                &h
            )
            .await
            .0,
            expected
        );
    }
    let discovered = discover(&f, &key).await;
    assert_eq!(discovered["services"].as_array().unwrap().len(), 1);
    assert_eq!(discovered["services"][0]["slug"], "viewer");
    // The same org with an org-owned key acts directly as owner, not as the
    // human's Viewer membership, and cannot inherit the human's other orgs.
    f.owner = services[2].user_id.clone();
    let org_key = f.key(KeySpec::default()).await;
    let h = headers(&org_key.full_key, true);
    assert_eq!(
        get(&f, "/api/v1/catalog/viewer-instance/endpoints", &h)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        get(&f, "/api/v1/catalog/member-instance/endpoints", &h)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let discovered = discover(&f, &org_key).await;
    assert!(
        !discovered["services"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["slug"] == "viewer")
    );
}

#[tokio::test]
async fn auto_connected_expansion_applies_without_provisioning() {
    let Some(f) = Fixture::new("catalog_routes_auto").await else {
        return;
    };
    let _cache = SpecCacheTestGuard::acquire();
    let catalog = f.catalog("auto", false).await;
    let instance = f.mount(&f.owner, "auto-instance", Some(&catalog)).await;
    f.state
        .db
        .collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &instance.id},
            doc! {"$set": {"source": "auto_provision"}},
        )
        .await
        .unwrap();
    for enabled in [false, true] {
        let key = f
            .key(KeySpec {
                allowed: Some(&[]),
                allow_auto_connected: enabled,
                ..Default::default()
            })
            .await;
        assert_eq!(
            get(
                &f,
                "/api/v1/catalog/auto-instance/endpoints",
                &headers(&key.full_key, false)
            )
            .await
            .0,
            if enabled {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
        let discovered = discover(&f, &key).await;
        assert_eq!(
            discovered["services"].as_array().unwrap().is_empty(),
            enabled
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn scheduled_relay_and_unprivileged_delegated_tokens_are_rejected() {
    let Some(f) = Fixture::new("catalog_routes_denied").await else {
        return;
    };
    let key = f.key(KeySpec::default()).await;
    let owner = Uuid::parse_str(&f.owner).unwrap();
    let relay = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "proxy account:read",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id.clone(),
            api_key_name: key.name.clone(),
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
        },
    )
    .unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "proxy catalog:read",
        "catalog-test",
        60,
        None,
    )
    .unwrap();
    f.state
        .db
        .collection::<ApiKey>(KEYS)
        .update_one(
            doc! {"_id": &key.id},
            doc! {"$set": {"purpose": "scheduled_invocation", "scheduled_write_enabled": true}},
        )
        .await
        .unwrap();
    for token in [&key.full_key, &relay, &delegated] {
        for path in paths("any") {
            assert_eq!(
                get(&f, &path, &headers(token, true)).await.0,
                StatusCode::FORBIDDEN
            );
        }
    }
}

#[tokio::test]
async fn delegated_account_read_catalog_parity_is_unchanged() {
    let Some(f) = Fixture::new("catalog_routes_delegated").await else {
        return;
    };
    let _cache = SpecCacheTestGuard::acquire();
    f.catalog("public", false).await;
    let token = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.owner).unwrap(),
        "account:read",
        "catalog-test",
        60,
        None,
    )
    .unwrap();
    for path in paths("public") {
        let (status, body) = get(&f, &path, &headers(&token, true)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

#[tokio::test]
async fn service_accounts_retain_rejection_because_detail_requires_a_user_actor() {
    let Some(f) = Fixture::new("catalog_routes_sa").await else {
        return;
    };
    let _cache = SpecCacheTestGuard::acquire();
    let catalog = f.catalog("public", false).await;
    // Restricted platform grants to the human owner must not transfer to SA.
    f.state.db.collection::<DownstreamService>(CATALOG).update_one(doc! {"_id": &catalog.id}, doc! {"$set": {
        "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
        "platform_key": bson::to_bson(&PlatformKeyConfig { enabled: true, audience: PlatformKeyAudience::Restricted, allowed_owner_ids: vec![f.owner.clone()] }).unwrap()
    }}).await.unwrap();
    f.mount(&f.owner, "owner-private", None).await;
    use crate::services::service_account_service as accounts;
    let (sa, secret) = accounts::create_service_account(
        &f.state.db,
        "Discovery",
        None,
        "proxy read",
        &[],
        None,
        &f.owner,
    )
    .await
    .unwrap();
    let token = accounts::authenticate_client_credentials(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &sa.client_id,
        &secret,
        Some("proxy"),
    )
    .await
    .unwrap()
    .access_token;
    let h = headers(&token, true);
    for path in paths("public") {
        let (status, body) = get(&f, &path, &h).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("Service accounts cannot access")
        );
    }
    // MCP's listing uses load_for_listing and tolerates an SA actor. The
    // catalog detail's restricted-grant lookup uses load and requires a User.
    let detail = crate::services::catalog_service::get_catalog_entry(
        &f.state.db,
        &f.state.encryption_keys,
        &sa.id,
        "public",
    )
    .await;
    assert!(matches!(detail, Err(crate::errors::AppError::NotFound(_))));
    let discovery = mcp_service::discover_services(&f.state.db, &sa.id, None, None)
        .await
        .unwrap();
    assert_eq!(discovery["services"][0]["platform_key"]["available"], false);
    f.state.db.collection::<Document>(crate::models::service_account::COLLECTION_NAME).update_one(doc! {"_id": &sa.id}, doc! {"$set": {"purpose": "curation", "platform_protected": true, "curation_grant": {
        "id": Uuid::new_v4().to_string(), "service_ids": [], "issued_by": &f.owner, "issued_at": bson::DateTime::now(), "max_writes": 10_i64, "window_seconds": 3600_i64, "window_started_at": bson::DateTime::now(), "writes_used": 0_i64
    }}}).await.unwrap();
    for path in paths("public") {
        assert_eq!(get(&f, &path, &h).await.0, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn catalog_discovery_issues_no_writes_beyond_api_key_authentication_bookkeeping() {
    let commands = Arc::new(Mutex::new(Vec::<(String, Document)>::new()));
    let observed = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            observed
                .lock()
                .unwrap()
                .push((event.command_name, event.command));
        }
    });
    let Some(db) =
        connect_test_database_with_command_handler("catalog_routes_read_only", handler).await
    else {
        return;
    };
    let f = Fixture::with_db(db).await;
    let _cache = SpecCacheTestGuard::acquire();
    let catalog = f.catalog("auto-candidate", false).await;
    // This catalog row is eligible for provisioning, but has no UserService.
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set": {"requires_user_credential": false}},
        )
        .await
        .unwrap();
    let key = f.key(KeySpec::default()).await;
    let h = headers(&key.full_key, false);
    for path in [
        "/api/v1/catalog",
        "/api/v1/catalog?include_all=true",
        "/api/v1/catalog/auto-candidate",
        "/api/v1/catalog/auto-candidate/endpoints",
    ] {
        commands.lock().unwrap().clear();
        let (status, body) = get(&f, path, &h).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let observed = commands.lock().unwrap();
        for (name, command) in observed.iter() {
            match name.as_str() {
                "update" => {
                    // AuthUser validates the key on every request and records
                    // last_used_at through the versioned mutation service.
                    assert_eq!(command.get_str("update").unwrap(), KEYS);
                    for update in command.get_array("updates").unwrap() {
                        let update = update.as_document().unwrap();
                        assert_eq!(update.get_document("q").unwrap(), &doc! {"_id": &key.id});
                        let mutation = update.get_document("u").unwrap();
                        assert_eq!(mutation.len(), 2);
                        let set = mutation.get_document("$set").unwrap();
                        assert_eq!(set.len(), 2);
                        assert!(set.contains_key("last_used_at") && set.contains_key("updated_at"));
                        assert_eq!(
                            mutation.get_document("$inc").unwrap(),
                            &doc! {"state_version": 1_i64}
                        );
                    }
                }
                "insert" | "delete" | "findAndModify" | "bulkWrite" => {
                    panic!("unexpected discovery write: {name}")
                }
                _ => {}
            }
        }
    }
    // Independently observe the handler after real extraction: zero write
    // commands, including to api_keys. Authentication is outside this boundary.
    use axum::extract::{FromRequestParts, Query, State};
    let (mut parts, _) = Request::builder()
        .uri("/api/v1/catalog")
        .header("x-api-key", &key.full_key)
        .body(())
        .unwrap()
        .into_parts();
    let auth = crate::mw::auth::AuthUser::from_request_parts(&mut parts, &f.state)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let axum::Json(listing) = super::list_catalog(
        State(f.state.clone()),
        auth,
        Default::default(),
        Query(super::CatalogListQuery { include_all: true }),
    )
    .await
    .unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert!(commands.lock().unwrap().iter().all(|(name, _)| {
        !["insert", "update", "delete", "findAndModify", "bulkWrite"].contains(&name.as_str())
    }));
    assert_eq!(
        f.state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<UserEndpoint>(ENDPOINTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    // The scoped MCP discovery loader is read-only as well.
    commands.lock().unwrap().clear();
    mcp_service::discover_services_with_scope(&f.state.db, &f.owner, None, None, Some(&[]))
        .await
        .unwrap();
    assert!(commands.lock().unwrap().iter().all(|(name, _)| {
        !["insert", "update", "delete", "findAndModify", "bulkWrite"].contains(&name.as_str())
    }));
}
