use super::*;
use crate::{
    models::downstream_service::{
        DownstreamService, PlatformKeyAudience, PlatformKeyConfig, PlatformKeyPurpose,
    },
    services::{feature_flag_service as flags, org_agent_tests::Fixture},
    test_utils::{test_auth_user, test_auto_connected_catalog_service, test_service_endpoint},
};
use bson::{Document, doc};

async fn fixture(name: &str) -> Fixture {
    let f = Fixture::new(name).await;
    for flag in [flags::NYXAGENT_ENGINE_FLAG_KEY, "assistant:org-agents"] {
        flags::set_platform_override(
            &f.state.db,
            flag,
            &flags::FlagTarget::Global,
            true,
            &f.admin,
        )
        .await
        .unwrap();
    }
    f
}
async fn catalog(
    f: &Fixture,
    slug: &str,
    audience: Option<PlatformKeyAudience>,
    owners: Vec<String>,
) -> DownstreamService {
    let mut row = test_auto_connected_catalog_service();
    row.slug = slug.into();
    row.name = format!("Display {slug}");
    row.service_category = "internal".into();
    row.auth_method = "bearer".into();
    row.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"fixture-only")
        .await
        .unwrap();
    row.platform_key = audience.map(|audience| PlatformKeyConfig {
        purpose: Default::default(),
        enabled: true,
        audience,
        allowed_owner_ids: owners,
    });
    row.inference = Some(
        serde_json::from_value(json!({"wire_protocol":"openai_responses","model_list":true}))
            .unwrap(),
    );
    f.state
        .db
        .collection::<DownstreamService>("downstream_services")
        .insert_one(&row)
        .await
        .unwrap();
    row
}
async fn inventory(f: &Fixture, actor: &str, org: Option<&str>) -> Value {
    grant_services(
        State(f.state.clone()),
        test_auth_user(actor),
        Query(GrantServicesQuery {
            org: org.map(str::to_owned),
        }),
    )
    .await
    .unwrap()
    .0["services"]
        .clone()
}
async fn create(f: &Fixture, org: Option<&str>, name: &str, services: Value) -> Value {
    Box::pin(create_agent(
        State(f.state.clone()),
        test_auth_user(&f.admin),
        Json(
            serde_json::from_value(
                json!({"org":org,"name":name,"description":"Grant round trip","services":services}),
            )
            .unwrap(),
        ),
    ))
    .await
    .unwrap()
    .1
    .0
}
async fn save(f: &Fixture, actor: &str, agent: &str, services: Value) -> AppResult<Json<Value>> {
    Box::pin(set_agent_grants(
        State(f.state.clone()),
        test_auth_user(actor),
        Path(agent.into()),
        Json(serde_json::from_value(json!({"services":services,"account_read":false})).unwrap()),
    ))
    .await
}

#[tokio::test]
async fn grant_services_personal_list_create_save_round_trip_includes_unpublished_platform_inference()
 {
    let f = fixture("grant_services_personal_roundtrip").await;
    let db = &f.state.db;
    let public = catalog(
        &f,
        "public-inference",
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    catalog(&f, "legacy-inference", None, vec![]).await;
    catalog(
        &f,
        "restricted-person",
        Some(PlatformKeyAudience::Restricted),
        vec![f.admin.clone()],
    )
    .await;
    let published = catalog(
        &f,
        "published-api",
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    db.collection("service_endpoints")
        .insert_one(test_service_endpoint(
            &published.id,
            "status",
            "GET",
            "/status",
        ))
        .await
        .unwrap();
    let custom = f.service(&f.admin, "custom-http").await;
    db.collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&public.id},
            doc! {"$set":{"icon_url":"https://example.test/catalog.svg"}},
        )
        .await
        .unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&custom},
            doc! {"$set":{"icon_url":"https://example.test/custom.svg"}},
        )
        .await
        .unwrap();
    let list = inventory(&f, &f.admin, None).await;
    assert_eq!(list.as_array().unwrap().len(), 5, "{list}");
    let public_metadata = list
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["slug"] == "public-inference")
        .unwrap();
    assert_eq!(public_metadata["catalog_slug"], "public-inference");
    assert_eq!(
        public_metadata["icon_url"],
        "https://example.test/catalog.svg"
    );
    let custom_metadata = list
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == custom)
        .unwrap();
    assert!(custom_metadata.get("catalog_slug").is_none());
    assert_eq!(
        custom_metadata["icon_url"],
        "https://example.test/custom.svg"
    );
    let slugs: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["slug"].as_str().unwrap().to_string())
        .collect();
    let created = create(&f, None, "roundtrip", json!(slugs)).await;
    let agent = team::agent(db, &f.admin, created["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(agent.grants.service_ids.len(), 5);
    assert!(agent.grants.platform_service_ids.is_empty());
    let ids: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    let _ = save(&f, &f.admin, &agent.id, json!(ids)).await.unwrap();
    let thread = engine::get(
        db,
        &f.admin,
        created["home_conversation_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    let key = f.key(&thread).await;
    assert!(!key.allow_all_services && !key.allow_auto_connected_services);
    assert_eq!(key.allowed_service_ids.len(), 5);
    // It is the personal auto-connected row, not an unbound catalog grant.
    let connection = db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"user_id":&f.admin,"catalog_service_id":&public.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.source.as_deref(), Some("auto_provision"));
    assert!(key.allowed_service_ids.contains(&connection.id));
    assert_eq!(
        db.collection::<Document>("user_services")
            .count_documents(doc! {"user_id":&f.org,"catalog_service_id":&public.id})
            .await
            .unwrap(),
        0
    );
    // MCP still publishes no invented tools for no-endpoint inference.
    let tools = crate::services::mcp_service::load_operation_catalog(
        db,
        &f.state.node_ws_manager,
        &f.admin,
        crate::services::mcp_service::NodeScope::Unrestricted,
        crate::services::mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    assert!(!tools.services.iter().any(|s| s.service_id == connection.id));
    assert!(
        tools
            .unpublished_services
            .iter()
            .any(|s| s.service_id == connection.id)
    );
    let before = agent.grants.clone();
    assert!(
        save(&f, &f.admin, &agent.id, json!(["missing"]))
            .await
            .is_err()
    );
    assert_eq!(
        team::agent(db, &f.admin, &agent.id).await.unwrap().grants,
        before
    );
    let before_count = db
        .collection::<Document>("assistant_agents")
        .count_documents(doc! {})
        .await
        .unwrap();
    assert!(Box::pin(create_agent(State(f.state.clone()), test_auth_user(&f.admin), Json(serde_json::from_value(json!({"name":"bad","description":"bad service","services":["public-inference","missing"]})).unwrap()))).await.is_err());
    assert_eq!(
        db.collection::<Document>("assistant_agents")
            .count_documents(doc! {})
            .await
            .unwrap(),
        before_count
    );
    let (valid, refused) = team::resolve_each_target(
        db,
        &f.state.node_ws_manager,
        &f.admin,
        &f.admin,
        &["public-inference".into(), "missing".into()],
    )
    .await
    .unwrap();
    assert_eq!(valid.service_ids, vec![connection.id]);
    assert_eq!(refused.len(), 1);
    let nyxbot = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &f.admin,
        "Grant the model connection",
    )
    .await;
    let chat =
        crate::services::assistant_authority_tests::chat_for_row(&f.state, &f.admin, &nyxbot).await;
    let specialist = create(&f, None, "native-grant", json!([])).await;
    let (result, error) = Box::pin(execute_tool(
        &f.state,
        &chat,
        "nyxid__grant_subagent",
        &json!({"subagent":specialist["id"],"services":["public-inference","llm-nyx","unknown"]}),
    ))
    .await;
    assert!(!error, "{result}");
    assert_eq!(result["not_granted"].as_array().unwrap().len(), 2);
    let saved = team::agent(db, &f.admin, specialist["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(saved.grants.service_ids, valid.service_ids);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_services_org_round_trip_public_explicit_restricted_auto_and_live_acl() {
    let f = fixture("grant_services_org_roundtrip").await;
    let db = &f.state.db;
    let public = catalog(
        &f,
        "public-inference",
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    catalog(
        &f,
        "restricted-org",
        Some(PlatformKeyAudience::Restricted),
        vec![f.org.clone()],
    )
    .await;
    let personal = f.service(&f.admin, "personal-only").await;
    let explicit = f.service(&f.org, "explicit-platform").await;
    db.collection::<Document>("user_services").update_one(doc! {"_id":&explicit}, doc! {"$set":{"catalog_service_id":&public.id,"credential_binding":"platform","auth_method":"bearer"}}).await.unwrap();
    let custom = f.service(&f.org, "custom-org").await;
    db.collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&public.id},
            doc! {"$set":{"icon_url":"https://example.test/catalog.svg"}},
        )
        .await
        .unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&explicit},
            doc! {"$set":{"icon_url":"https://example.test/override.svg"}},
        )
        .await
        .unwrap();
    let list = inventory(&f, &f.admin, Some(&f.org)).await;
    assert_eq!(list.as_array().unwrap().len(), 3, "{list}");
    let metadata = list
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == explicit)
        .unwrap();
    assert_eq!(metadata["slug"], "explicit-platform");
    assert_eq!(metadata["catalog_slug"], "public-inference");
    assert_eq!(metadata["icon_url"], "https://example.test/override.svg");
    assert!(!list.as_array().unwrap().iter().any(|s| s["id"] == personal));
    let ids: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    let created = create(&f, Some(&f.org), "org-roundtrip", json!(ids)).await;
    let agent_id = created["id"].as_str().unwrap();
    let _ = save(&f, &f.member, agent_id, json!(ids)).await.unwrap();
    let agent = team::maintained_agent(db, &f.member, agent_id)
        .await
        .unwrap();
    assert_eq!(agent.grants.service_ids.len(), 3);
    assert!(agent.grants.platform_service_ids.is_empty());
    assert!(
        save(&f, &f.member, agent_id, json!([personal]))
            .await
            .is_err()
    );
    for actor in [&f.viewer, &f.outsider] {
        assert!(
            grant_services(
                State(f.state.clone()),
                test_auth_user(actor),
                Query(GrantServicesQuery {
                    org: Some(f.org.clone())
                })
            )
            .await
            .is_err()
        );
    }
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":&custom}, doc! {"$set":{"admin_only":true}})
        .await
        .unwrap();
    let member_list = inventory(&f, &f.member, Some(&f.org)).await;
    assert!(
        !member_list
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == custom)
    );
    assert!(
        save(&f, &f.member, agent_id, json!([custom]))
            .await
            .is_err()
    );
    db.collection::<Document>("downstream_services")
        .update_one(
            doc! {"slug":"restricted-org"},
            doc! {"$set":{"platform_key.allowed_owner_ids":[]}},
        )
        .await
        .unwrap();
    let list = inventory(&f, &f.admin, Some(&f.org)).await;
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["slug"] == "restricted-org")
    );
    assert!(
        save(&f, &f.admin, agent_id, json!(["restricted-org"]))
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_services_ungrantable_infrastructure_disabled_and_foreign_are_never_offered() {
    let f = fixture("grant_services_exclusions").await;
    let db = &f.state.db;
    let engine = catalog(
        &f,
        engine::SERVICE_SLUG,
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    let hidden = catalog(
        &f,
        "private-model-route",
        Some(PlatformKeyAudience::Restricted),
        vec![],
    )
    .await;
    db.collection::<Document>("downstream_services").update_one(doc! {"_id":&hidden.id}, doc! {"$set":{"platform_key.purpose":bson::to_bson(&PlatformKeyPurpose::ModelRoute).unwrap()}}).await.unwrap();
    catalog(
        &f,
        "unavailable-platform",
        Some(PlatformKeyAudience::Restricted),
        vec![f.outsider.clone()],
    )
    .await;
    let disabled = f.service(&f.admin, "disabled").await;
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":&disabled}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let foreign = f.service(&f.outsider, "foreign").await;
    let private = catalog(&f, "private-no-consent", None, vec![]).await;
    db.collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&private.id},
            doc! {"$set":{"visibility":"private","developer_app_ids":["unconsented-app"]}},
        )
        .await
        .unwrap();
    let retired = catalog(&f, "retired-catalog", None, vec![]).await;
    db.collection::<Document>("downstream_services").update_one(doc!{"_id":&retired.id},
        doc!{"$set":{"service_category":crate::services::retired_service_service::RETIRED_CATEGORY}}).await.unwrap();
    let list = inventory(&f, &f.admin, None).await;
    assert_eq!(list, json!([]));
    let ids = vec![
        engine.id,
        hidden.id,
        "unavailable-platform".into(),
        private.id,
        retired.id,
        disabled,
        foreign,
        engine::SERVICE_SLUG.into(),
    ];
    let (resolved, refused) =
        team::resolve_each_target(db, &f.state.node_ws_manager, &f.admin, &f.admin, &ids)
            .await
            .unwrap();
    assert!(resolved.service_ids.is_empty() && resolved.platform_service_ids.is_empty());
    assert_eq!(refused.len(), ids.len());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_services_saved_public_inference_executes_with_the_person_as_platform_payer() {
    use crate::models::usage_meter::CredentialClass;
    use crate::services::billing::owner_resolver::{BillingOwnerResolver, PaysFrom};
    use crate::services::proxy_service::{self, ProxyExecutionContext};
    use axum::{Router, body::Body, http::Request, routing::post};
    let f = fixture("grant_services_platform_execution").await;
    let db = &f.state.db;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = Router::new().route("/responses", post(|headers: axum::http::HeaderMap| async move {
        assert_eq!(headers.get("authorization").and_then(|v| v.to_str().ok()), Some("Bearer fixture-only"));
        Json(json!({"id":"fixture_response","object":"response","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}))
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let model = catalog(
        &f,
        "public-inference",
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    db.collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&model.id},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let org_connection = f.service(&f.org, "org-inference").await;
    db.collection::<Document>("user_services").update_one(doc!{"_id":&org_connection}, doc!{"$set":{"catalog_service_id":&model.id,"credential_binding":"platform","auth_method":"bearer"}}).await.unwrap();
    for org in [None, Some(f.org.as_str())] {
        let inventory = inventory(&f, &f.admin, org).await;
        let connection = inventory
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                if org.is_some() {
                    r["id"] == org_connection
                } else {
                    r["slug"] == "public-inference"
                }
            })
            .unwrap();
        let created = create(
            &f,
            org,
            if org.is_some() {
                "org-execution"
            } else {
                "personal-execution"
            },
            json!([connection["id"]]),
        )
        .await;
        // The actor has both personal and org connections to this catalog.
        // Select the intended account through the normal owner-input path;
        // granting a connection does not bypass account-selection policy.
        let input: engine::TurnRequest = serde_json::from_value(json!({
            "conversation_id":created["home_conversation_id"],
            "text":format!("/service {}\nUse the granted inference service", connection["id"].as_str().unwrap()),
        })).unwrap();
        let thread = Box::pin(engine::begin_turn(
            db,
            &f.admin,
            &input,
            &f.state.encryption_keys,
        ))
        .await
        .unwrap();
        let key = f.key(&thread).await;
        let auth = crate::mw::auth::api_key_auth_user(db, &key, None, None, None)
            .await
            .unwrap();
        let slug = connection["slug"].as_str().unwrap();
        let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
            db,
            &f.state.encryption_keys,
            &f.admin,
            connection["id"].as_str().unwrap(),
            Some(slug),
            Some(&model.id),
            ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(resolved.master_credential);
        let payer = BillingOwnerResolver::new(db.clone())
            .resolve_for_ordinary_execution(
                &f.admin,
                org.unwrap_or(&f.admin),
                CredentialClass::NyxidManagedMaster,
            )
            .await
            .unwrap();
        assert_eq!(payer.owner_id, f.admin);
        assert_eq!(payer.pays, PaysFrom::Personal);
        let uri = format!("/api/v1/proxy/s/{slug}/responses");
        let mut request = Request::builder()
            .method("POST")
            .uri(&uri)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"model":"fixture-model","input":"fixture"}).to_string(),
            ))
            .unwrap();
        request
            .extensions_mut()
            .insert(axum::extract::OriginalUri(uri.parse().unwrap()));
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        let response = Box::pin(crate::handlers::proxy::proxy_request_by_slug_inner(
            &f.state,
            &auth,
            slug,
            "responses",
            request,
            &mut String::new(),
        ))
        .await
        .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "refusal code: {}",
            result["error"]
        );
        assert_eq!(result["id"], "fixture_response");
    }
    server.abort();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_services_catalog_id_remains_supported_before_personal_provisioning() {
    let f = fixture("grant_services_catalog_id").await;
    let catalog = catalog(
        &f,
        "direct-platform",
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    let created = create(&f, None, "catalog-id", json!([catalog.id])).await;
    let agent = team::agent(&f.state.db, &f.admin, created["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(agent.grants.platform_service_ids, vec![catalog.id]);
    let home = engine::get(
        &f.state.db,
        &f.admin,
        created["home_conversation_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        f.key(&home).await.allowed_platform_service_ids,
        agent.grants.platform_service_ids
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>("user_services")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "resolving grants does not provision connections"
    );
    let list = inventory(&f, &f.admin, None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let _ = save(&f, &f.admin, &agent.id, json!([list[0]["id"]]))
        .await
        .unwrap();
    let agent = team::agent(&f.state.db, &f.admin, &agent.id).await.unwrap();
    assert_eq!(
        agent.grants.service_ids,
        vec![list[0]["id"].as_str().unwrap()]
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_services_offline_user_credential_keeps_identity_over_the_platform_copy() {
    let f = fixture("grant_services_offline_identity").await;
    let model = catalog(
        &f,
        "configured-inference",
        Some(PlatformKeyAudience::Public),
        vec![],
    )
    .await;
    let service = f.service(&f.admin, "configured-inference").await;
    f.state.db.collection::<Document>("user_services").update_one(doc!{"_id":&service},
        doc!{"$set":{"catalog_service_id":&model.id,"auth_method":"bearer","credential_binding":"user"}}).await.unwrap();
    let list = inventory(&f, &f.admin, None).await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["id"], service);
    assert_eq!(list[0]["executable"], false);
    let created = create(&f, None, "offline-grant", json!([list[0]["slug"]])).await;
    let agent = team::agent(&f.state.db, &f.admin, created["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(agent.grants.service_ids, vec![service]);
    assert!(agent.grants.platform_service_ids.is_empty());
    f.state.db.drop().await.unwrap();
}
