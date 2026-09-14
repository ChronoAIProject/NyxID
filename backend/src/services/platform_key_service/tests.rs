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
            use_platform_key: Some(true),
            user_id: user.clone(),
            service_slug: catalog.slug.clone(),
            scopes: vec![],
            label: None,
            requested_by: None,
            callback_url: None,
            ttl_secs: None,
            oauth_client_id: None,
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
        .insert_one(test_membership(&org, &user, OrgRole::Admin, None))
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
