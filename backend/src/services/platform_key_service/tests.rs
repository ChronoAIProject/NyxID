use super::*;
use crate::models::downstream_service::{InferenceWireProtocol, test_helpers::dummy_service};
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
use crate::models::user::UserType;
use crate::services::{
    catalog_service, inference_service, llm_gateway_service, provider_service, proxy_service,
    unified_key_service,
};
use crate::test_utils::{
    connect_transaction_test_database, test_encryption_keys, test_membership, test_user,
    test_user_endpoint, test_user_service,
};
use mongodb::bson;
use std::sync::Arc;

async fn owner(db: &mongodb::Database, kind: UserType) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&id, kind))
        .await
        .unwrap();
    id
}
fn platform_service() -> DownstreamService {
    let mut service = dummy_service();
    service.id = uuid::Uuid::new_v4().to_string();
    service.slug = "platform-test".into();
    service.is_active = true;
    service.service_type = "http".into();
    service.auth_method = "bearer".into();
    service.auth_key_name = "Authorization".into();
    service.platform_key = Some(PlatformKeyConfig {
        enabled: true,
        audience: PlatformKeyAudience::Public,
        allowed_owner_ids: vec![],
    });
    service.credential_encrypted = vec![1];
    service
}

#[test]
fn inference_defaults_and_legacy_binding_are_additive() {
    let mut chrono = platform_service();
    chrono.platform_key = None;
    chrono.slug = "chrono-llm".into();
    chrono.requires_user_credential = true;
    chrono.inference = inference_service::default_inference(&chrono.slug);
    let v = inference_service::view(&chrono, None, has_platform_key(&chrono)).unwrap();
    assert_eq!(v.binding, "user");
    assert_eq!(v.status_slug.as_deref(), Some("chrono-llm"));
    assert_eq!(v.wire_protocol, InferenceWireProtocol::OpenaiCompletions);
    chrono.slug = "chrono-llm-public".into();
    chrono.requires_user_credential = false;
    chrono.service_category = "internal".into();
    chrono.visibility = "public".into();
    let v = inference_service::view(&chrono, None, has_platform_key(&chrono)).unwrap();
    assert_eq!(v.binding, "platform");
    assert!(v.status_slug.is_none());
    assert!(serde_json::to_value(v).unwrap().get("realtime").is_none());
    for slug in [
        "api-github",
        "google-ai",
        "llm-google-ai",
        "llm-cohere",
        "llm-openai-codex",
    ] {
        assert!(inference_service::default_inference(slug).is_none());
    }
    assert!(
        serde_json::from_value::<crate::models::downstream_service::ServiceInference>(
            serde_json::json!({"wire_protocol":"unknown"})
        )
        .is_err()
    );
    let old: crate::models::service_billing::ServiceBilling =
        serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(old.byok_pricing.is_none());
    assert!(old.platform_key_pricing.is_none());
}

#[tokio::test]
async fn platform_acl_checks_people_org_roles_activity_and_revocation() {
    let db = connect_transaction_test_database("platform_acl").await;
    let person = owner(&db, UserType::Person).await;
    let stranger = owner(&db, UserType::Person).await;
    let org = owner(&db, UserType::Org).await;
    let mut service = platform_service();
    assert!(available(&db, &service, &stranger).await.unwrap());
    assert!(
        proxy_service::authorize_master_credential_server_chosen(&db, &service)
            .await
            .is_err()
    );
    service.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Restricted;
    service.platform_key.as_mut().unwrap().allowed_owner_ids = vec![person.clone()];
    assert!(available(&db, &service, &person).await.unwrap());
    assert!(!available(&db, &service, &stranger).await.unwrap());
    service.platform_key.as_mut().unwrap().allowed_owner_ids = vec![org.clone()];
    let membership = test_membership(&org, &person, OrgRole::Member, None);
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    assert!(available(&db, &service, &person).await.unwrap());
    assert!(available(&db, &service, &org).await.unwrap());
    for role in ["viewer", "member"] {
        db.collection::<bson::Document>(MEMBERSHIPS)
            .update_one(doc! {"_id": &membership.id}, doc! {"$set":{"role":role}})
            .await
            .unwrap();
        assert_eq!(
            available(&db, &service, &person).await.unwrap(),
            role == "member"
        );
    }
    db.collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! {"_id": &membership.id},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert!(matches!(
        require(&db, &service, &person).await,
        Err(AppError::NotFound(_))
    ));
    service.platform_key.as_mut().unwrap().enabled = false;
    assert!(!available(&db, &service, &org).await.unwrap());
    service.platform_key = None;
    service.slug = "legacy-shared-service".into();
    service.service_category = "internal".into();
    service.requires_user_credential = false;
    service.visibility = "public".into();
    assert!(available(&db, &service, &stranger).await.unwrap());
    service.platform_key = Some(PlatformKeyConfig::default());
    assert!(!available(&db, &service, &stranger).await.unwrap());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn seeded_provider_platform_binding_resolves_proxy_gateway_and_realtime() {
    let db = connect_transaction_test_database("platform_providers").await;
    let enc = test_encryption_keys();
    let user = owner(&db, UserType::Person).await;
    provider_service::seed_default_providers(&db, &enc)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, &enc)
        .await
        .unwrap();
    inference_service::backfill(&db).await.unwrap();
    let nodes = Arc::new(crate::services::node_ws_manager::NodeWsManager::new(
        30, 100,
    ));
    for (slug, host) in [
        ("xai", "api.x.ai"),
        ("openai", "api.openai.com"),
        ("anthropic", "api.anthropic.com"),
    ] {
        let (mut service, provider) = llm_gateway_service::resolve_llm_service_by_slug(&db, slug)
            .await
            .unwrap();
        assert_eq!(provider.provider_type, "api_key");
        service.platform_key = platform_service().platform_key;
        service.credential_encrypted = enc.encrypt(b"test-platform-secret").await.unwrap();
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .replace_one(doc! {"_id": &service.id}, &service)
            .await
            .unwrap();
        let direct = proxy_service::resolve_proxy_target(
            &db,
            &enc,
            &user,
            &service.id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .await
        .unwrap();
        assert_eq!(direct.credential, "test-platform-secret");
        let connection = unified_key_service::create_platform_key(
            &db,
            &user,
            &user,
            &service.slug,
            "test",
            None,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(connection.api_key.is_none());
        assert_eq!(binding(&connection.service), "platform");
        let resolution = proxy_service::resolve_proxy_target_from_user_service(
            &db,
            &enc,
            &nodes,
            &user,
            Some(&connection.service.slug),
            None,
            proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(resolution.master_credential);
        assert_eq!(resolution.target.credential, "test-platform-secret");
        let entry = catalog_service::get_catalog_entry(&db, &enc, &user, &service.slug)
            .await
            .unwrap();
        assert!(entry.platform_key.available);
        assert_eq!(entry.inference.unwrap().binding, "platform");
        if slug != "anthropic" {
            let url = crate::handlers::proxy::build_downstream_ws_url(
                &resolution.target,
                "realtime",
                None,
                &[],
            )
            .unwrap();
            assert_eq!(url, format!("wss://{host}/v1/realtime"));
            let request = crate::handlers::proxy::build_downstream_ws_request(
                &url,
                &resolution.target,
                &[],
                &[],
                &[],
                None,
            )
            .unwrap();
            assert_eq!(
                request.headers()["authorization"],
                "Bearer test-platform-secret"
            );
            assert!(service.inference.as_ref().unwrap().realtime);
        }
        unified_key_service::switch_credential_binding(
            &db,
            &enc,
            &user,
            &connection.service.id,
            false,
            Some("own-secret"),
            unified_key_service::OauthClientCredentialsInput::None,
        )
        .await
        .unwrap();
        let own = unified_key_service::get_key(&db, &enc, &user, &connection.service.id)
            .await
            .unwrap();
        assert_eq!(own.credential_binding, "user");
        let retained = own.api_key_id.unwrap();
        unified_key_service::switch_credential_binding(
            &db,
            &enc,
            &user,
            &connection.service.id,
            true,
            None,
            unified_key_service::OauthClientCredentialsInput::None,
        )
        .await
        .unwrap();
        assert!(
            db.collection::<crate::models::user_api_key::UserApiKey>(
                crate::models::user_api_key::COLLECTION_NAME
            )
            .find_one(doc! {"_id": &retained})
            .await
            .unwrap()
            .is_some()
        );
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &service.id},
                doc! {"$set":{"platform_key.enabled":false}},
            )
            .await
            .unwrap();
        assert!(matches!(
            proxy_service::resolve_proxy_target_from_user_service(
                &db,
                &enc,
                &nodes,
                &user,
                Some(&connection.service.slug),
                None,
                proxy_service::ProxyExecutionContext::new(
                    None,
                    crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled()
                )
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn inference_backfill_preserves_admin_metadata_and_chrono_catalog_views() {
    let db = connect_transaction_test_database("inference_backfill").await;
    let user = owner(&db, UserType::Person).await;
    let enc = test_encryption_keys();
    for slug in ["chrono-llm", "chrono-llm-public", "api-github"] {
        let mut service = platform_service();
        service.slug = slug.into();
        service.platform_key = None;
        service.visibility = "public".into();
        service.service_category = "internal".into();
        service.requires_user_credential = slug != "chrono-llm-public";
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();
    }
    inference_service::backfill(&db).await.unwrap();
    for (slug, binding) in [("chrono-llm", "user"), ("chrono-llm-public", "platform")] {
        let entry = catalog_service::get_catalog_entry(&db, &enc, &user, slug)
            .await
            .unwrap();
        let inference = entry.inference.unwrap();
        assert_eq!(inference.binding, binding);
        assert_eq!(
            inference.status_slug.as_deref(),
            (binding == "user").then_some(slug)
        );
        assert!(inference.model_list);
    }
    assert!(
        catalog_service::get_catalog_entry(&db, &enc, &user, "api-github")
            .await
            .unwrap()
            .inference
            .is_none()
    );
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"slug":"chrono-llm"},
            doc! {"$set":{"inference.model_list":false}},
        )
        .await
        .unwrap();
    inference_service::backfill(&db).await.unwrap();
    assert!(
        !catalog_service::get_catalog_entry(&db, &enc, &user, "chrono-llm")
            .await
            .unwrap()
            .inference
            .unwrap()
            .model_list
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_auto_connections_reconcile_and_hosted_links_complete_once() {
    use crate::models::user_service::COLLECTION_NAME as CONNECTIONS;
    use crate::services::connect_link_service::{self, CompleteInput, CompleteResult};
    use futures::TryStreamExt;
    let db = connect_transaction_test_database("platform_auto_link").await;
    let user = owner(&db, UserType::Person).await;
    let other = owner(&db, UserType::Person).await;
    let enc = test_encryption_keys();
    let mut catalog = platform_service();
    catalog.credential_encrypted = enc.encrypt(b"platform-secret").await.unwrap();
    catalog.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Restricted;
    catalog.platform_key.as_mut().unwrap().allowed_owner_ids = vec![user.clone()];
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    for id in [&user, &other] {
        unified_key_service::auto_provision_no_auth_services(&db, id)
            .await
            .unwrap();
    }
    let automatic: Vec<UserService> = db
        .collection::<UserService>(CONNECTIONS)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(automatic.len(), 1);
    assert_eq!(automatic[0].user_id, user);
    assert_eq!(binding(&automatic[0]), "platform");
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set":{"platform_key.allowed_owner_ids":[]}},
        )
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &user)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<UserService>(CONNECTIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set":{"platform_key.allowed_owner_ids":[&user]}},
        )
        .await
        .unwrap();
    let link = connect_link_service::create(
        &db,
        connect_link_service::CreateInput {
            user_id: user.clone(),
            service_slug: catalog.slug.clone(),
            use_platform_key: Some(true),
            scopes: vec![],
            label: None,
            requested_by: None,
            callback_url: None,
            ttl_secs: None,
            oauth_client_id: None,
            endpoint_url: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        connect_link_service::complete(
            &db,
            &enc,
            &other,
            &link.raw_token,
            CompleteInput::default(),
            false
        )
        .await,
        Err(AppError::ConnectLinkNotFound)
    ));
    let completed = connect_link_service::complete(
        &db,
        &enc,
        &user,
        &link.raw_token,
        CompleteInput::default(),
        false,
    )
    .await
    .unwrap();
    let CompleteResult::Completed(view) = completed else {
        panic!("platform key requires no OAuth");
    };
    let service_id = view.link.completed_user_service_id.unwrap();
    let key = unified_key_service::get_key(&db, &enc, &user, &service_id)
        .await
        .unwrap();
    assert_eq!(key.credential_binding, "platform");
    assert!(key.platform_key_available);
    assert!(
        db.collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME
        )
        .find_one(doc! {})
        .await
        .unwrap()
        .is_none()
    );
    // Replays observe the existing completion and cannot provision another row.
    let _ = connect_link_service::complete(
        &db,
        &enc,
        &user,
        &link.raw_token,
        CompleteInput::default(),
        false,
    )
    .await;
    assert_eq!(
        db.collection::<UserService>(CONNECTIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_key_http_llm_gateway_and_mcp_use_server_credential_and_live_acl() {
    use crate::services::{
        billing::route_inventory::{
            BillingIngress, BillingRoutePolicy, enforce_billing_egress_classification,
        },
        mcp_service,
    };
    use axum::{
        body::Body,
        extract::{Path, State},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer platform-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices":[{"message":{"role":"assistant","content":"ok"}}],
            "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}
        })))
        .expect(3)
        .mount(&upstream)
        .await;
    let db = connect_transaction_test_database("platform_execution").await;
    let user = owner(&db, UserType::Person).await;
    let state = crate::test_utils::test_app_state(db.clone());
    provider_service::seed_default_providers(&db, &state.encryption_keys)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, &state.encryption_keys)
        .await
        .unwrap();
    let (mut catalog, _) = llm_gateway_service::resolve_llm_service_by_slug(&db, "xai")
        .await
        .unwrap();
    catalog.base_url = format!("{}/v1", upstream.uri());
    catalog.platform_key = platform_service().platform_key;
    catalog.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-secret")
        .await
        .unwrap();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&catalog.id}, &catalog)
        .await
        .unwrap();
    let result = unified_key_service::create_platform_key(
        &db,
        &user,
        &user,
        &catalog.slug,
        "xAI",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    let auth = crate::test_utils::test_auth_user(&user);
    for ingress in [BillingIngress::LlmProvider, BillingIngress::LlmGateway] {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"model":"grok-test","messages":[{"role":"user","content":"hi"}]}"#,
            ))
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(ingress));
        let response = if ingress == BillingIngress::LlmProvider {
            crate::handlers::llm_gateway::llm_proxy_request(
                State(state.clone()),
                auth.clone(),
                Path(("xai".into(), "chat/completions".into())),
                request,
            )
            .await
            .unwrap()
        } else {
            crate::handlers::llm_gateway::gateway_request(
                State(state.clone()),
                auth.clone(),
                Path("chat/completions".into()),
                request,
            )
            .await
            .unwrap()
        };
        assert_eq!(response.status(), 200);
    }
    let endpoint = mcp_service::McpToolEndpoint {
        endpoint_id: "completion".into(),
        name: "completion".into(),
        method: "POST".into(),
        path: "chat/completions".into(),
        request_body_schema: Some(serde_json::json!({"type":"object"})),
        ..Default::default()
    };
    let tool = mcp_service::McpToolService {
        workspace_destinations_pending: false,
        service_id: result.service.id.clone(),
        service_name: "xAI".into(),
        service_slug: result.service.slug.clone(),
        description: None,
        service_category: "internal".into(),
        endpoints: vec![],
        durable_endpoint_metadata: Default::default(),
        source: mcp_service::McpToolSource::UserManaged {
            user_service_id: result.service.id.clone(),
            catalog_service_id: Some(catalog.id.clone()),
            effective_owner_id: user.clone(),
            node_id: None,
            has_server_credential: true,
        },
        executable: true,
        is_generic_proxy: false,
        invalid_openapi_contract: false,
        recommended_skills: vec![],
        recommended_skill_refs: None,
        skills_revision: None,
        proxy_operation_policy: None,
    };
    let prepared = mcp_service::prepare_proxy_tool_call(
        &tool,
        &endpoint,
        &serde_json::json!({"body":{"model":"grok-test"}}),
    )
    .unwrap();
    let ctx = mcp_service::McpExecContext {
        api_key_id: None,
        allow_all_nodes: true,
        allowed_node_ids: &[],
    };
    let permit = || {
        enforce_billing_egress_classification(
            Some(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
            BillingIngress::Mcp,
        )
        .unwrap()
    };
    let response = mcp_service::execute_tool(
        &state.http_client,
        &db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &user,
        &user,
        &tool,
        &endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &ctx,
        permit(),
    )
    .await
    .unwrap();
    assert_eq!(response.0, 200);
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set":{"platform_key.enabled":false}},
        )
        .await
        .unwrap();
    let prepared = mcp_service::prepare_proxy_tool_call(
        &tool,
        &endpoint,
        &serde_json::json!({"body":{"model":"grok-test"}}),
    )
    .unwrap();
    assert!(matches!(
        mcp_service::execute_tool(
            &state.http_client,
            &db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &state.billing,
            &user,
            &user,
            &tool,
            &endpoint,
            prepared,
            &state.jwt_keys,
            &state.config,
            &state.connection_expiry_notifier,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            &ctx,
            permit()
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_auto_platform_rows_inherit_acl_and_agent_union_tracks_binding() {
    let db = connect_transaction_test_database("platform_org_auto_scope").await;
    let enc = test_encryption_keys();
    let user = owner(&db, UserType::Person).await;
    let org = owner(&db, UserType::Org).await;
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &user, OrgRole::Member, None))
        .await
        .unwrap();
    let mut service = platform_service();
    service.credential_encrypted = enc.encrypt(b"platform-secret").await.unwrap();
    service.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Restricted;
    service.platform_key.as_mut().unwrap().allowed_owner_ids = vec![org.clone()];
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &user)
        .await
        .unwrap();
    let rows = crate::services::key_service::active_auto_connected_service_ids(&db, &org)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let org_view = unified_key_service::get_key(&db, &enc, &org, &rows[0])
        .await
        .unwrap();
    assert!(org_view.auto_connected);
    assert_eq!(org_view.credential_binding, "platform");

    assert!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &user)
            .await
            .unwrap()
            .is_empty()
    );
    // An explicit personal binding is also in the agent auto-connected grant.
    let personal = unified_key_service::create_platform_key(
        &db,
        &user,
        &user,
        &service.slug,
        "Personal",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &user)
            .await
            .unwrap(),
        vec![personal.service.id.clone()]
    );
    unified_key_service::set_platform_connection_active(&db, &user, &personal.service.id, false)
        .await
        .unwrap();
    assert!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &user)
            .await
            .unwrap()
            .is_empty()
    );
    unified_key_service::set_platform_connection_active(&db, &user, &personal.service.id, true)
        .await
        .unwrap();
    unified_key_service::switch_credential_binding(
        &db,
        &enc,
        &user,
        &personal.service.id,
        false,
        Some("my-secret"),
        unified_key_service::OauthClientCredentialsInput::None,
    )
    .await
    .unwrap();
    assert!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &user)
            .await
            .unwrap()
            .is_empty()
    );
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! { "_id": &service.id },
            doc! { "$set": { "platform_key.allowed_owner_ids": [] } },
        )
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &user)
        .await
        .unwrap();
    assert!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &org)
            .await
            .unwrap()
            .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn public_platform_auto_connection_is_personal_only_and_reconciles_org_rows() {
    let db = connect_transaction_test_database("platform_public_personal_only").await;
    let enc = test_encryption_keys();
    let person = owner(&db, UserType::Person).await;
    let org = owner(&db, UserType::Org).await;
    let member_org = owner(&db, UserType::Org).await;
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&member_org, &person, OrgRole::Member, None))
        .await
        .unwrap();
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &person, OrgRole::Admin, None))
        .await
        .unwrap();

    let mut catalog = platform_service();
    catalog.credential_encrypted = enc.encrypt(b"platform-secret").await.unwrap();
    catalog.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Public;
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();

    unified_key_service::auto_provision_no_auth_services(&db, &person)
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &org)
        .await
        .unwrap();
    let listed = unified_key_service::list_keys(&db, &enc, &person, &HashMap::new())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].auto_connected);
    assert_eq!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &person)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &member_org)
            .await
            .unwrap()
            .is_empty()
    );
    let rows = db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME);
    assert_eq!(
        rows.count_documents(doc! { "user_id": &person, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        1,
        "public platform services must auto-connect the personal owner"
    );
    assert_eq!(
        rows.count_documents(doc! { "user_id": &org, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        0,
        "public platform services must not fan out into organizations"
    );

    // Simulate a row created by the pre-fix org traversal. Reconciliation on
    // the next listing must remove it and its endpoint before provisioning.
    let wrong_service_id = uuid::Uuid::new_v4().to_string();
    let wrong_endpoint_id = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user_endpoint::UserEndpoint>(
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .insert_one(test_user_endpoint(
        &wrong_endpoint_id,
        &org,
        "DeepSeek",
        "https://api.example.com",
        None,
        Some(&catalog.id),
    ))
    .await
    .unwrap();
    let mut wrong = test_user_service(
        &wrong_service_id,
        &org,
        &catalog.slug,
        &wrong_endpoint_id,
        Some(&catalog.id),
        None,
    );
    wrong.source = Some(crate::models::user_service::AUTO_PROVISION_SOURCE.to_string());
    wrong.source_id = Some(format!("{org}:{}", catalog.id));
    wrong.credential_binding = Some("platform".to_string());
    wrong.auth_method = catalog.auth_method.clone();
    wrong.auth_key_name = catalog.auth_key_name.clone();
    rows.insert_one(wrong).await.unwrap();

    unified_key_service::auto_provision_no_auth_services(&db, &person)
        .await
        .unwrap();
    assert_eq!(
        rows.count_documents(doc! { "user_id": &org, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        0
    );
    assert!(
        db.collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
            .find_one(doc! { "_id": &wrong_endpoint_id })
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn inactive_personal_rows_preserve_disabled_connections_and_replace_tombstones() {
    for state in ["deleted", "disabled", "inactive-auto"] {
        assert_inactive_connection_provisioning(UserType::Person, state).await;
    }
}

#[tokio::test]
async fn inactive_org_rows_preserve_disabled_connections_and_replace_tombstones() {
    for state in ["deleted", "disabled", "inactive-auto"] {
        assert_inactive_connection_provisioning(UserType::Org, state).await;
    }
}

async fn assert_inactive_connection_provisioning(kind: UserType, state: &str) {
    let db = connect_transaction_test_database("platform_inactive_connection").await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let is_org = kind == UserType::Org;
    let user = owner(&db, kind).await;
    let enc = test_encryption_keys();
    let mut catalog = platform_service();
    let actor = if is_org {
        let actor = owner(&db, UserType::Person).await;
        db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(&user, &actor, OrgRole::Admin, None))
            .await
            .unwrap();
        catalog.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Restricted;
        catalog.platform_key.as_mut().unwrap().allowed_owner_ids = vec![user.clone()];
        actor
    } else {
        user.clone()
    };
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let old_id = uuid::Uuid::new_v4().to_string();
    let endpoint_id = uuid::Uuid::new_v4().to_string();
    if state != "deleted" {
        db.collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(test_user_endpoint(
            &endpoint_id,
            &user,
            "Old connection",
            &catalog.base_url,
            None,
            Some(&catalog.id),
        ))
        .await
        .unwrap();
    }
    let mut old = test_user_service(
        &old_id,
        &user,
        &catalog.slug,
        &endpoint_id,
        Some(&catalog.id),
        None,
    );
    old.is_active = state == "disabled";
    if state == "inactive-auto" {
        old.source = Some(crate::models::user_service::AUTO_PROVISION_SOURCE.into());
        old.source_id = Some(format!("{user}:{}", catalog.id));
    }
    db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .insert_one(old)
        .await
        .unwrap();

    if state == "disabled" {
        unified_key_service::switch_credential_binding(
            &db,
            &enc,
            &user,
            &old_id,
            false,
            Some("own-secret"),
            unified_key_service::OauthClientCredentialsInput::None,
        )
        .await
        .unwrap();
        crate::services::user_service_service::update_user_service(
            &db,
            &user,
            &actor,
            &old_id,
            None,
            None,
            None,
            None,
            Some(false),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    for _ in 0..2 {
        let listed = unified_key_service::list_keys(&db, &enc, &actor, &HashMap::new())
            .await
            .unwrap();
        if state == "disabled" {
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, old_id);
            assert!(!listed[0].is_active);
            assert!(!listed[0].auto_connected);
            assert!(!listed[0].credential_missing);
        } else {
            let active: Vec<_> = listed.iter().filter(|row| row.is_active).collect();
            assert_eq!(active.len(), 1, "{state}");
            assert!(active[0].auto_connected);
            assert_eq!(
                active[0].slug, catalog.slug,
                "reuse the base slug for {state}"
            );
        }
    }
    let rows = rows_for_catalog(&db, &user, &catalog.id).await;
    if state == "disabled" {
        assert_eq!(rows.len(), 1);
        let credential_id = rows[0]
            .api_key_id
            .clone()
            .expect("BYOK credential retained");
        crate::services::user_service_service::update_user_service(
            &db,
            &user,
            &actor,
            &old_id,
            None,
            None,
            None,
            None,
            Some(true),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("Enable must succeed after listing a disabled BYOK connection");
        let listed = unified_key_service::list_keys(&db, &enc, &actor, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, old_id);
        assert_eq!(listed[0].slug, catalog.slug);
        assert!(listed[0].is_active);
        assert!(!listed[0].auto_connected);
        assert_eq!(listed[0].credential_binding, "user");
        let enabled = rows_for_catalog(&db, &user, &catalog.id).await;
        assert_eq!(
            enabled[0].api_key_id.as_deref(),
            Some(credential_id.as_str())
        );
        assert_eq!(enabled[0].endpoint_id, endpoint_id);
    } else {
        assert_eq!(rows.len(), if state == "inactive-auto" { 1 } else { 2 });
        assert_ne!(rows.iter().find(|row| row.is_active).unwrap().id, old_id);
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn active_personal_byok_stays_visible_without_duplicate_auto_connection() {
    let db = connect_transaction_test_database("platform_public_byok").await;
    let person = owner(&db, UserType::Person).await;
    let enc = test_encryption_keys();
    let catalog = platform_service();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = unified_key_service::create_platform_key(
        &db,
        &person,
        &person,
        &catalog.slug,
        "Personal",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    unified_key_service::switch_credential_binding(
        &db,
        &enc,
        &person,
        &connection.service.id,
        false,
        Some("own-secret"),
        unified_key_service::OauthClientCredentialsInput::None,
    )
    .await
    .unwrap();
    let listed = unified_key_service::list_keys(&db, &enc, &person, &HashMap::new())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, connection.service.id);
    assert_eq!(listed[0].credential_binding, "user");
    assert!(listed[0].platform_key_available);
    assert!(!listed[0].auto_connected);
    db.drop().await.unwrap();
}

#[test]
fn automatic_platform_grants_require_active_direct_owners() {
    let grants = OwnerGrants {
        actor_id: "person".into(),
        active_owner_ids: ["person".into(), "org".into()].into(),
        readable_owner_ids: ["person".into(), "org".into()].into(),
        org_owner_ids: ["org".into()].into(),
        memberships: vec![],
    };
    let mut catalog = platform_service();
    assert!(auto_provisionable_with_grants(
        &catalog, None, "person", &grants
    ));
    assert!(!auto_provisionable_with_grants(
        &catalog, None, "org", &grants
    ));
    assert!(!auto_provisionable_with_grants(
        &catalog, None, "inactive", &grants
    ));
    catalog.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Restricted;
    catalog.platform_key.as_mut().unwrap().allowed_owner_ids = vec!["org".into()];
    assert!(!auto_provisionable_with_grants(
        &catalog, None, "person", &grants
    ));
    assert!(auto_provisionable_with_grants(
        &catalog, None, "org", &grants
    ));
    assert!(available_with_grants(&catalog, None, "person", &grants));
    catalog.platform_key.as_mut().unwrap().allowed_owner_ids = vec!["person".into()];
    assert!(auto_provisionable_with_grants(
        &catalog, None, "person", &grants
    ));
    assert!(!auto_provisionable_with_grants(
        &catalog, None, "org", &grants
    ));
}

#[tokio::test]
async fn public_platform_explicit_org_binding_keeps_execution_and_agent_access() {
    let db = connect_transaction_test_database("platform_public_explicit_org").await;
    let enc = test_encryption_keys();
    let person = owner(&db, UserType::Person).await;
    let org = owner(&db, UserType::Org).await;
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &person, OrgRole::Admin, None))
        .await
        .unwrap();
    let mut catalog = platform_service();
    catalog.credential_encrypted = enc.encrypt(b"platform-secret").await.unwrap();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let explicit = unified_key_service::create_platform_key(
        &db,
        &org,
        &person,
        &catalog.slug,
        "Explicit org",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert_ne!(
        explicit.service.source.as_deref(),
        Some(crate::models::user_service::AUTO_PROVISION_SOURCE)
    );
    // Both direct org invocation and the member's listing preserve explicit rows.
    unified_key_service::auto_provision_no_auth_services(&db, &org)
        .await
        .unwrap();
    unified_key_service::list_keys(&db, &enc, &person, &HashMap::new())
        .await
        .unwrap();
    crate::services::user_service_service::cleanup_public_org_auto_provisions(&db)
        .await
        .unwrap();
    assert_eq!(
        crate::services::key_service::active_auto_connected_service_ids(&db, &org)
            .await
            .unwrap(),
        vec![explicit.service.id.clone()],
    );
    let target = proxy_service::resolve_proxy_target_from_user_service(
        &db,
        &enc,
        &Arc::new(crate::services::node_ws_manager::NodeWsManager::new(
            30, 100,
        )),
        &org,
        Some(&explicit.service.slug),
        None,
        proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        ),
    )
    .await
    .unwrap();
    let target = target.expect("public explicit org binding remains executable");
    assert!(target.master_credential);
    assert_eq!(target.target.credential, "platform-secret");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_audience_switch_reconciles_org_auto_rows_in_both_directions() {
    let db = connect_transaction_test_database("platform_audience_switch").await;
    let enc = test_encryption_keys();
    let person = owner(&db, UserType::Person).await;
    let org = owner(&db, UserType::Org).await;
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &person, OrgRole::Member, None))
        .await
        .unwrap();

    let mut catalog = platform_service();
    catalog.credential_encrypted = enc.encrypt(b"platform-secret").await.unwrap();
    catalog.platform_key.as_mut().unwrap().audience = PlatformKeyAudience::Restricted;
    catalog.platform_key.as_mut().unwrap().allowed_owner_ids = vec![org.clone()];
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();

    unified_key_service::auto_provision_no_auth_services(&db, &person)
        .await
        .unwrap();
    let rows = db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME);
    assert_eq!(
        rows.count_documents(doc! { "user_id": &org, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        1,
        "restricted org grants still create the org auto-connected row"
    );

    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! { "_id": &catalog.id },
            doc! { "$set": { "platform_key.audience": "public" } },
        )
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &person)
        .await
        .unwrap();
    assert_eq!(
        rows.count_documents(doc! { "user_id": &org, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        0,
        "switching restricted to public removes the org auto row"
    );
    assert_eq!(
        rows.count_documents(doc! { "user_id": &person, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        1,
        "switching restricted to public retains personal auto provisioning"
    );

    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! { "_id": &catalog.id },
            doc! {
                "$set": {
                    "platform_key.audience": "restricted",
                    "platform_key.allowed_owner_ids": [&org],
                }
            },
        )
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &person)
        .await
        .unwrap();
    assert_eq!(
        rows.count_documents(doc! { "user_id": &org, "catalog_service_id": &catalog.id })
            .await
            .unwrap(),
        1,
        "switching public to restricted recreates the granted org row"
    );
    assert!(
        rows_for_catalog(&db, &person, &catalog.id).await.is_empty(),
        "an inherited org execution grant must not retain an automatic personal row"
    );
    unified_key_service::auto_provision_no_auth_services(&db, &person)
        .await
        .unwrap();
    assert_eq!(
        rows_for_catalog(&db, &org, &catalog.id).await.len(),
        1,
        "a directly granted restricted org row survives reconciliation"
    );
    db.drop().await.unwrap();
}

async fn rows_for_catalog(
    db: &mongodb::Database,
    user_id: &str,
    catalog_id: &str,
) -> Vec<UserService> {
    db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "catalog_service_id": catalog_id })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
async fn platform_to_oauth_uses_existing_unified_placeholder_flow() {
    let db = connect_transaction_test_database("platform_oauth_switch").await;
    let enc = test_encryption_keys();
    let user = owner(&db, UserType::Person).await;
    provider_service::seed_default_providers(&db, &enc)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, &enc)
        .await
        .unwrap();
    let provider = db
        .collection::<crate::models::provider_config::ProviderConfig>(
            crate::models::provider_config::COLLECTION_NAME,
        )
        .find_one(doc! { "slug": "github" })
        .await
        .unwrap()
        .unwrap();
    let mut service = platform_service();
    service.provider_config_id = Some(provider.id);
    service.credential_encrypted = enc.encrypt(b"platform-secret").await.unwrap();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    let connection = unified_key_service::create_platform_key(
        &db,
        &user,
        &user,
        &service.slug,
        "OAuth",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    unified_key_service::switch_credential_binding(
        &db,
        &enc,
        &user,
        &connection.service.id,
        false,
        None,
        unified_key_service::OauthClientCredentialsInput::Raw {
            client_id: "custom-app",
            client_secret: "custom-secret",
        },
    )
    .await
    .unwrap();
    let own = unified_key_service::get_key(&db, &enc, &user, &connection.service.id)
        .await
        .unwrap();
    assert_eq!(own.credential_binding, "user");
    assert_eq!(own.status, "pending_auth");
    let key = db
        .collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(doc! { "_id": own.api_key_id.unwrap() })
        .await
        .unwrap()
        .unwrap();
    assert!(key.connection_id.is_some());
    assert_eq!(key.credential_type, "oauth2");
    assert_eq!(
        enc.decrypt(key.user_oauth_client_id_encrypted.as_ref().unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"custom-app"
    );
    assert_eq!(
        enc.decrypt(key.user_oauth_client_secret_encrypted.as_ref().unwrap())
            .await
            .unwrap()
            .as_slice(),
        b"custom-secret"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn inherited_org_provisioning_preserves_legacy_personal_only_rows() {
    let db = connect_transaction_test_database("platform_org_legacy").await;
    let user = owner(&db, UserType::Person).await;
    let org = owner(&db, UserType::Org).await;
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &user, OrgRole::Member, None))
        .await
        .unwrap();
    let mut legacy = platform_service();
    legacy.platform_key = None;
    legacy.auth_method = "none".into();
    legacy.requires_user_credential = false;
    legacy.slug = "legacy-shared-service".into();
    legacy.service_category = "internal".into();
    legacy.visibility = "public".into();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&legacy)
        .await
        .unwrap();
    unified_key_service::auto_provision_no_auth_services(&db, &user)
        .await
        .unwrap();
    let rows = db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME);
    assert_eq!(
        rows.count_documents(doc! { "user_id": &user, "catalog_service_id": &legacy.id })
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        rows.count_documents(doc! { "user_id": &org, "catalog_service_id": &legacy.id })
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn server_chosen_accepts_implicit_and_explicit_public_only() {
    let db = connect_transaction_test_database("review_server_selected").await;
    let mut service = platform_service();
    service.slug = "legacy-shared-service".into();
    service.requires_user_credential = false;
    service.service_category = "internal".into();
    service.visibility = "public".into();
    for config in [
        None,
        service.platform_key.clone(),
        Some(PlatformKeyConfig {
            enabled: true,
            audience: PlatformKeyAudience::Restricted,
            allowed_owner_ids: vec![],
        }),
        Some(PlatformKeyConfig::default()),
    ] {
        let expected = config
            .as_ref()
            .is_none_or(|c| c.enabled && c.audience == PlatformKeyAudience::Public);
        service.platform_key = config;
        assert_eq!(
            proxy_service::authorize_master_credential_server_chosen(&db, &service)
                .await
                .is_ok(),
            expected
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn owner_grant_intersection_is_independent_of_allowlist_size() {
    let grants = OwnerGrants {
        org_owner_ids: HashSet::new(),
        actor_id: "person".into(),
        memberships: vec![],
        readable_owner_ids: HashSet::new(),
        active_owner_ids: ["person", "org-1", "org-2", "org-3"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    };
    let mut allowed: Vec<String> = (0..1000).map(|i| format!("unrelated-{i}")).collect();
    assert!(!grants.permits("person", &allowed));
    for org in ["org-1", "org-2", "org-3"] {
        allowed.push(org.into());
        assert!(grants.permits("person", &allowed));
        assert!(grants.permits(org, &allowed));
        allowed.pop();
    }
    assert!(!grants.permits("inactive-org", &["inactive-org".into()]));
    assert!(!grants.permits("org-1", &["person".into(), "org-2".into()]));
}

#[tokio::test]
async fn gateway_url_providers_reject_platform_enable_and_runtime_resolution() {
    let db = connect_transaction_test_database("review_platform_gateway_url").await;
    let enc = test_encryption_keys();
    let user = owner(&db, UserType::Person).await;
    provider_service::seed_default_providers(&db, &enc)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, &enc)
        .await
        .unwrap();
    let provider = db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"slug":"openclaw"})
        .await
        .unwrap()
        .unwrap();
    assert!(provider.requires_gateway_url);
    let mut service = platform_service();
    service.provider_config_id = Some(provider.id.clone());
    assert!(
        validate_config(
            &db,
            service.platform_key.as_mut().unwrap(),
            Some(&provider.id)
        )
        .await
        .is_err()
    );
    assert!(!available(&db, &service, &user).await.unwrap());
    service.platform_key.as_mut().unwrap().enabled = false;
    validate_config(
        &db,
        service.platform_key.as_mut().unwrap(),
        Some(&provider.id),
    )
    .await
    .unwrap();
    let mut config = PlatformKeyConfig {
        allowed_owner_ids: vec![user.clone(), user],
        ..Default::default()
    };
    validate_config(&db, &mut config, None).await.unwrap();
    assert_eq!(config.allowed_owner_ids.len(), 1);
    config
        .allowed_owner_ids
        .push(uuid::Uuid::new_v4().to_string());
    assert!(validate_config(&db, &mut config, None).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn explicit_platform_connections_allow_cosmetics_and_name_locked_fields() {
    use axum::{
        Json,
        extract::{Path, State},
    };
    let db = connect_transaction_test_database("review_platform_cosmetics").await;
    let user = owner(&db, UserType::Person).await;
    let state = crate::test_utils::test_app_state(db.clone());
    let mut catalog = platform_service();
    catalog.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-secret")
        .await
        .unwrap();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = unified_key_service::create_platform_key(
        &db,
        &user,
        &user,
        &catalog.slug,
        "Initial",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    let auth = crate::test_utils::test_auth_user(&user);
    let request = serde_json::json!({"label":"My platform connection", "admin_only":true, "recommended_skills":["read-docs"], "custom_user_agent":"MyAgent/1", "default_request_headers":[{"name":"X-Project", "value":"test"}]});
    let Json(response) = crate::handlers::keys::update_key(
        State(state.clone()),
        auth.clone(),
        Path(connection.service.id.clone()),
        Json(serde_json::from_value(request).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(response.label, "My platform connection");
    let stored =
        crate::services::user_service_service::get_user_service(&db, &user, &connection.service.id)
            .await
            .unwrap();
    assert!(stored.admin_only);
    assert_eq!(stored.custom_user_agent.as_deref(), Some("MyAgent/1"));
    assert_eq!(stored.default_request_headers.unwrap()[0].name, "X-Project");
    let ep = db
        .collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &connection.endpoint.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ep.recommended_skills.unwrap(), vec!["read-docs"]);
    assert_eq!(ep.url, connection.endpoint.url);
    for (field, value) in [
        (
            "endpoint_url",
            serde_json::json!("https://elsewhere.example"),
        ),
        ("auth_method", serde_json::json!("none")),
        ("auth_key_name", serde_json::json!("X-Key")),
        ("node_id", serde_json::json!("")),
        ("credential", serde_json::json!("own")),
        ("openapi_spec_url", serde_json::json!("")),
        ("identity_propagation_mode", serde_json::json!("none")),
        ("identity_include_user_id", serde_json::json!(false)),
        ("identity_include_email", serde_json::json!(false)),
        ("identity_include_name", serde_json::json!(false)),
        ("identity_jwt_audience", serde_json::json!("other")),
        ("forward_access_token", serde_json::json!(false)),
        ("inject_delegation_token", serde_json::json!(false)),
        ("delegation_token_scope", serde_json::json!("proxy")),
        ("oauth_client_id", serde_json::json!("app")),
        ("oauth_client_secret", serde_json::json!("secret")),
        ("copy_oauth_client_from", serde_json::json!("source")),
    ] {
        let body = serde_json::from_value(serde_json::json!({field: value})).unwrap();
        let err = crate::handlers::keys::update_key(
            State(state.clone()),
            auth.clone(),
            Path(connection.service.id.clone()),
            Json(body),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, AppError::ValidationError(ref message) if message == &format!("Switch to your own key to change {field}")),
            "{field}: {err}"
        );
    }
    // The lower-level service route enforces the same boundary.
    let err = crate::services::user_service_service::update_user_service(
        &db,
        &user,
        &user,
        &connection.service.id,
        None,
        None,
        Some(""),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Switch to your own key to change node_id")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn byok_create_response_reports_available_platform_key_for_both_create_paths() {
    use axum::{Json, extract::State};
    let db = connect_transaction_test_database("review_byok_create_availability").await;
    let user = owner(&db, UserType::Person).await;
    let state = crate::test_utils::test_app_state(db.clone());
    let mut catalog = platform_service();
    catalog.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-secret")
        .await
        .unwrap();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    for reserved in [None, Some(uuid::Uuid::new_v4().to_string())] {
        let body = serde_json::from_value(serde_json::json!({"service_slug": &catalog.slug, "label":"BYOK", "credential":"own-secret"})).unwrap();
        let Json(created) = crate::handlers::keys::create_key_with_service_id(
            State(state.clone()),
            crate::test_utils::test_auth_user(&user),
            Default::default(),
            Json(body),
            reserved.as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(created.credential_binding, "user");
        assert!(created.platform_key_available);
        assert!(
            !serde_json::to_string(&created)
                .unwrap()
                .contains("own-secret")
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn non_llm_slug_token_lane_settles_reported_json_and_sse_usage() {
    use crate::models::service_billing::{
        BillingMetric, LanePricing, PricingSyncStatus, ServiceBilling,
    };
    use crate::models::usage_meter::{COLLECTION_NAME as METER, UsageMeterRow};
    use axum::{
        body::{Body, to_bytes},
        extract::{Path, State},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    let db = connect_transaction_test_database("review_non_llm_token_lane").await;
    let user = owner(&db, UserType::Person).await;
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = true;
    let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let upstream = MockServer::start().await;
    let mut catalog = platform_service();
    catalog.slug = "chrono-llm-public".into();
    catalog.base_url = upstream.uri();
    catalog.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-secret")
        .await
        .unwrap();
    catalog.billing = Some(ServiceBilling {
        platform_key_pricing: Some(LanePricing {
            components: Vec::new(),
            metric: BillingMetric::Tokens,
            credits_per_unit: "0.01".into(),
            lago_metric_code: "platform_svc_chrono-llm-public_pk".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
        }),
        ..Default::default()
    });
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = unified_key_service::create_platform_key(
        &db,
        &user,
        &user,
        &catalog.slug,
        "Chrono",
        None,
        false,
        None,
    )
    .await
    .unwrap();
    for (route, content_type, response_body) in [
        (
            "json",
            "application/json",
            r#"{"choices":[],"usage":{"prompt_tokens":1234,"completion_tokens":321,"total_tokens":1555}}"#,
        ),
        (
            "sse",
            "text/event-stream",
            "data: {\"usage\":{\"prompt_tokens\":1234,\"completion_tokens\":321,\"total_tokens\":1555}}\n\ndata: [DONE]\n\n",
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/{route}")))
            .and(header("authorization", "Bearer platform-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(response_body, content_type))
            .expect(1)
            .mount(&upstream)
            .await;
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/proxy/s/{}/{route}",
                connection.service.slug
            ))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"model":"test","messages":[]}"#))
            .unwrap();
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        let response = Box::pin(crate::handlers::proxy::proxy_request_by_slug(
            State(state.clone()),
            crate::test_utils::test_auth_user(&user),
            Default::default(),
            Path((connection.service.slug.clone(), route.into())),
            request,
        ))
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        to_bytes(response.into_body(), 10_000).await.unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let rows: Vec<UsageMeterRow> = db
                .collection::<UsageMeterRow>(METER)
                .find(doc! {"status":"finalized"})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if rows.len() == 2 {
                for row in rows {
                    assert_eq!(row.quantity, Some(1555));
                    assert_eq!(row.metric, BillingMetric::Tokens);
                    assert_eq!(row.lago_metric_code, "platform_svc_chrono-llm-public_pk");
                    assert_eq!(
                        row.credential_class,
                        crate::models::usage_meter::CredentialClass::NyxidManagedMaster
                    );
                }
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn llm_status_fetches_memberships_once_for_many_providers_and_owners() {
    let (db, counts) = monitored_listing_database("review_status_membership_bound").await;
    let enc = test_encryption_keys();
    let person = owner(&db, UserType::Person).await;
    let mut orgs = vec![];
    for _ in 0..3 {
        let org = owner(&db, UserType::Org).await;
        db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(&org, &person, OrgRole::Member, None))
            .await
            .unwrap();
        orgs.push(org);
    }
    provider_service::seed_default_providers(&db, &enc)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, &enc)
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME).update_many(doc! {"slug":{"$in":["llm-openai","llm-xai","llm-deepseek"]}}, doc! {"$set": {
        "platform_key": {"enabled":true,"audience":"restricted","allowed_owner_ids": &orgs},
        "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: enc.encrypt(b"secret").await.unwrap() },
    }}).await.unwrap();
    let statuses = assert_listing_queries(
        &counts,
        llm_gateway_service::get_llm_status(&db, &person, "https://nyx.example"),
    )
    .await;
    for slug in ["openai", "xai", "deepseek"] {
        assert_eq!(
            statuses
                .providers
                .iter()
                .find(|s| s.provider_slug == slug)
                .unwrap()
                .status,
            "ready"
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn legacy_and_explicit_master_credentials_cannot_use_owner_node_routes() {
    let mut service = platform_service();
    service.requires_user_credential = false;
    let mut target = proxy_service::ProxyTarget {
        workspace_destinations_pending: false,
        target_id: None,
        base_url: service.base_url.clone(),
        auth_method: "bearer".into(),
        auth_key_name: "Authorization".into(),
        credential: "server-secret".into(),
        service,
        catalog_default_headers: vec![],
        user_service_default_headers: vec![],
        ws_frame_injections: vec![],
        connection_id: None,
    };
    assert!(proxy_service::uses_server_held_master(&target));
    target.service.platform_key = None;
    assert!(proxy_service::uses_server_held_master(&target));
    target.service.requires_user_credential = true;
    assert!(!proxy_service::uses_server_held_master(&target));
    target.service.requires_user_credential = false;
    target.auth_method = "none".into();
    assert!(!proxy_service::uses_server_held_master(&target));
}

#[derive(Default)]
struct ListingCommands {
    database: String,
    memberships: usize,
    providers: usize,
}

type ListingRecorder = Arc<std::sync::Mutex<ListingCommands>>;

async fn monitored_listing_database(prefix: &str) -> (mongodb::Database, ListingRecorder) {
    let counts = Arc::new(std::sync::Mutex::new(ListingCommands::default()));
    let recorded = counts.clone();
    let handler = mongodb::event::EventHandler::callback(move |event| {
        if let mongodb::event::command::CommandEvent::Started(event) = event {
            let mut counts = recorded.lock().unwrap();
            if event.db != counts.database {
                return;
            }
            match event.command.get_str("find").ok() {
                Some(MEMBERSHIPS) => counts.memberships += 1,
                Some(PROVIDERS) => counts.providers += 1,
                _ => {}
            }
        }
    });
    let db =
        crate::test_utils::connect_transaction_test_database_with_command_handler(prefix, handler)
            .await;
    counts.lock().unwrap().database = db.name().to_string();
    (db, counts)
}

// Driver callbacks run before the command completes, avoiding the server profiler's
// capped storage and delivery behavior. Each recorder belongs to one test/client.
async fn assert_listing_queries<T>(
    counts: &ListingRecorder,
    request: impl std::future::Future<Output = AppResult<T>>,
) -> T {
    let before = {
        let counts = counts.lock().unwrap();
        (counts.memberships, counts.providers)
    };
    let result = request.await.unwrap();
    let after = counts.lock().unwrap();
    assert_eq!(
        after.memberships - before.0,
        1,
        "each request must fetch memberships once, independent of service/owner count"
    );
    assert_eq!(
        after.providers - before.1,
        1,
        "each request must batch providers once"
    );
    result
}

async fn listing_fixture(db: &mongodb::Database) -> (String, Vec<String>, Vec<DownstreamService>) {
    let person = owner(db, UserType::Person).await;
    let mut orgs = vec![];
    for role in [
        OrgRole::Member,
        OrgRole::Admin,
        OrgRole::Member,
        OrgRole::Viewer,
    ] {
        let org = owner(db, UserType::Org).await;
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(&org, &person, role, None))
            .await
            .unwrap();
        orgs.push(org);
    }
    let mut services = vec![];
    for slug in ["openai", "xai", "deepseek"] {
        let now = bson::DateTime::now();
        let provider: ProviderConfig = bson::from_document(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "slug": slug,
            "name": slug, "provider_type": "api_key", "is_active": true,
            "created_by": "test", "created_at": now, "updated_at": now,
        })
        .unwrap();
        db.collection::<ProviderConfig>(PROVIDERS)
            .insert_one(&provider)
            .await
            .unwrap();
        let mut service = platform_service();
        service.slug = format!("llm-{slug}");
        service.name = slug.into();
        service.provider_config_id = Some(provider.id);
        service.requires_user_credential = true;
        service.visibility = "private".into();
        service.service_category = "connection".into();
        service.inference = inference_service::default_inference(&service.slug);
        let config = service.platform_key.as_mut().unwrap();
        config.audience = PlatformKeyAudience::Restricted;
        config.allowed_owner_ids = orgs.clone();
        config.allowed_owner_ids.push(person.clone());
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();
        services.push(service);
    }
    (person, orgs, services)
}

#[tokio::test]
async fn catalog_list_fetches_memberships_and_providers_once_for_many_platform_entries() {
    let (db, counts) = monitored_listing_database("catalog_grant_batch").await;
    let enc = test_encryption_keys();
    let (person, orgs, _) = listing_fixture(&db).await;
    // Only org grants: the actor must inherit these via its membership snapshot.
    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$set": {"platform_key.allowed_owner_ids": &orgs}},
        )
        .await
        .unwrap();
    let entries =
        assert_listing_queries(&counts, catalog_service::list_catalog(&db, &enc, &person)).await;
    assert_eq!(entries.len(), 3);
    assert!(
        entries.iter().all(
            |e| e.platform_key.available && e.inference.as_ref().unwrap().binding == "platform"
        )
    );

    // Provider eligibility is refreshed on the next request, without a per-row read.
    db.collection::<ProviderConfig>(PROVIDERS)
        .update_one(
            doc! {"slug": "xai"},
            doc! {"$set": {"requires_gateway_url": true}},
        )
        .await
        .unwrap();
    let entries = assert_listing_queries(
        &counts,
        catalog_service::list_catalog_all(&db, &enc, &person),
    )
    .await;
    assert_eq!(
        entries.len(),
        2,
        "ineligible private platform entry must stay hidden"
    );

    // The sole remaining membership is a viewer: it cannot inherit a grant.
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .update_many(
            doc! {"role": {"$ne": "viewer"}},
            doc! {"$set": {"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let entries =
        assert_listing_queries(&counts, catalog_service::list_catalog(&db, &enc, &person)).await;
    assert!(entries.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn list_keys_shares_one_grant_and_provider_batch_with_org_provisioning_and_reconciliation() {
    use axum::extract::State;
    let (db, counts) = monitored_listing_database("keys_grant_batch").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let (person, orgs, _) = listing_fixture(&db).await;
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id": &orgs[1], "member_user_id": &person},
            doc! {"$set": {"scope_source": "inherit"}},
        )
        .await
        .unwrap();
    let keys = assert_listing_queries(&counts, async {
        let providers = load_providers(&db).await?;
        unified_key_service::list_keys(&db, &state.encryption_keys, &person, &providers).await
    })
    .await;
    assert_eq!(
        keys.len(),
        12,
        "three services for the person and each of three proxy-capable orgs"
    );
    assert!(keys.iter().all(|k| k.platform_key_available
        && k.auto_connected
        && k.credential_binding == "platform"));

    // Exercise the HTTP entry point too, including already-provisioned row reconciliation.
    let response = assert_listing_queries(
        &counts,
        crate::handlers::keys::list_keys(
            State(state.clone()),
            crate::test_utils::test_auth_user(&person),
        ),
    )
    .await;
    assert_eq!(response.0.keys.len(), 12);
    assert_eq!(
        response
            .0
            .keys
            .iter()
            .filter(|key| key.authorship.is_some())
            .count(),
        6,
        "only personal services and the admin's org expose history summaries"
    );

    let mut inventory_auth = crate::test_utils::test_auth_user(&person);
    inventory_auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    let response = assert_listing_queries(
        &counts,
        crate::handlers::keys::list_keys(State(state.clone()), inventory_auth),
    )
    .await;
    assert_eq!(response.0.keys.len(), 12);
    assert_eq!(
        response
            .0
            .keys
            .iter()
            .filter(|key| key.authorship.is_some())
            .count(),
        6
    );

    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$pull": {"platform_key.allowed_owner_ids": &orgs[0]}},
        )
        .await
        .unwrap();
    let response = assert_listing_queries(
        &counts,
        crate::handlers::keys::list_keys(
            State(state.clone()),
            crate::test_utils::test_auth_user(&person),
        ),
    )
    .await;
    assert_eq!(
        response.0.keys.len(),
        9,
        "revoked org rows must be reconciled without another membership fetch"
    );
    assert_eq!(
        db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
            .count_documents(doc! {"user_id": &orgs[0]})
            .await
            .unwrap(),
        0
    );
    crate::services::org_role_scope_service::set_scope(
        &db,
        &orgs[1],
        OrgRole::Admin,
        Some(vec![]),
        &person,
    )
    .await
    .unwrap();
    let response = assert_listing_queries(
        &counts,
        crate::handlers::keys::list_keys(
            State(state.clone()),
            crate::test_utils::test_auth_user(&person),
        ),
    )
    .await;
    assert_eq!(
        response
            .0
            .keys
            .iter()
            .filter(|key| key.authorship.is_some())
            .count(),
        3,
        "a live admin scope restriction hides org history without another membership read"
    );
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id": &orgs[1], "member_user_id": &person},
            doc! {"$set": {"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let response = assert_listing_queries(
        &counts,
        crate::handlers::keys::list_keys(State(state), crate::test_utils::test_auth_user(&person)),
    )
    .await;
    assert_eq!(
        response.0.keys.len(),
        6,
        "the next request observes membership revocation"
    );
    assert_eq!(
        response
            .0
            .keys
            .iter()
            .filter(|key| key.authorship.is_some())
            .count(),
        3
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mcp_discovery_and_callable_services_share_grants_and_providers_per_request() {
    use crate::services::{mcp_service, node_ws_manager::NodeWsManager};
    let (db, counts) = monitored_listing_database("mcp_grant_batch").await;
    let (person, _, catalogs) = listing_fixture(&db).await;
    let discovered = assert_listing_queries(
        &counts,
        mcp_service::discover_services(&db, &person, None, None),
    )
    .await;
    assert_eq!(discovered["count"], 3);
    assert!(
        discovered["services"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["platform_key"]["available"] == true
                && s["inference"]["binding"] == "platform")
    );
    for catalog in catalogs {
        unified_key_service::create_platform_key(
            &db,
            &person,
            &person,
            &catalog.slug,
            &catalog.name,
            None,
            false,
            None,
        )
        .await
        .unwrap();
    }
    let manager = NodeWsManager::new(30, 100);
    let tools = assert_listing_queries(
        &counts,
        mcp_service::load_user_tools(&db, &manager, &person),
    )
    .await;
    assert_eq!(tools.len(), 3);
    assert!(tools.iter().all(|t| t.executable));
    // Scoped operation catalog computes both visible and pre-node-scope views;
    // even those two passes must share a single request snapshot.
    assert_listing_queries(
        &counts,
        mcp_service::load_operation_catalog(
            &db,
            &manager,
            &person,
            mcp_service::NodeScope::Allowed(&[]),
            mcp_service::ServiceScope::Unrestricted,
        ),
    )
    .await;

    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$set": {"platform_key.allowed_owner_ids": []}},
        )
        .await
        .unwrap();
    let tools = assert_listing_queries(
        &counts,
        mcp_service::load_user_tools(&db, &manager, &person),
    )
    .await;
    assert!(
        tools.is_empty(),
        "a new request must recheck grants for explicit platform rows"
    );
    db.drop().await.unwrap();
}
