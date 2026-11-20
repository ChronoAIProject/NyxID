use super::*;
use crate::models::{
    service_provider_requirement::{COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement},
    user_endpoint::COLLECTION_NAME as ENDPOINTS,
    user_service::COLLECTION_NAME as SERVICES,
};
use crate::services::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    service_history::{self, transaction::Transaction},
};

pub const CATALOG_OPT_IN_READINESS: &str = "catalog:explicit-connections-v1";
pub const CATALOG_OPT_IN_SOURCE: &str = "catalog_opt_in";

/// Shared personal Connect action. Retrying retains connection identity and its
/// enabled state; automatic rows require this action before execution.
pub async fn connect_catalog_service(
    db: &mongodb::Database,
    actor: &str,
    owner: &str,
    slug: &str,
) -> AppResult<UserService> {
    if actor != owner {
        return Err(AppError::Forbidden(
            "Catalog opt-in is a personal connection action".into(),
        ));
    }
    let principal = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! { "_id": owner, "is_active": true, "user_type": "person" })
        .await?;
    if principal.is_none() {
        return Err(AppError::Forbidden(
            "An active personal owner is required".into(),
        ));
    }
    super::super::security_readiness_service::require_serving(db, CATALOG_OPT_IN_READINESS).await?;
    let catalog =
        super::super::catalog_service::get_downstream_service_by_slug(db, slug, actor).await?;
    let requirements: Vec<ServiceProviderRequirement> = db
        .collection(REQUIREMENTS)
        .find(doc! { "service_id": &catalog.id })
        .await?
        .try_collect()
        .await?;
    if catalog.offering_kind != crate::models::downstream_service::OfferingKind::AiService
        || !is_auto_provisionable_catalog_service(
            &catalog,
            !requirements.is_empty(),
            requirements.iter().any(|requirement| requirement.required),
        )
    {
        return Err(AppError::BadRequest(
            "This service requires credential setup".into(),
        ));
    }
    let platform =
        catalog.platform_key.is_some() || platform_key_service::has_platform_key(&catalog);
    let (auth_method, auth_key_name) = if platform {
        platform_key_service::require(db, &catalog, owner).await?;
        platform_key_service::effective_auth(db, &catalog).await?
    } else {
        let (method, name) = auto_provision_auth_snapshot(&catalog);
        (method.to_owned(), name.to_owned())
    };
    user_endpoint_service::validate_endpoint_url(&catalog.base_url)?;
    let now = Utc::now();
    let endpoint = UserEndpoint {
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        label: catalog.name.clone(),
        url: catalog.base_url.clone(),
        catalog_service_id: Some(catalog.id.clone()),
        openapi_spec_url: catalog.openapi_spec_url.clone(),
        recommended_skills: None,
        created_at: now,
        updated_at: now,
    };
    let identity = identity_config_from_downstream_service(&catalog);
    let mut service = UserService {
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        deleted_at: None,
        created_by: None,
        last_change: None,
        slug: catalog.slug.clone(),
        endpoint_id: endpoint.id.clone(),
        api_key_id: None,
        credential_binding: Some(if platform { "platform" } else { "user" }.into()),
        auth_method,
        auth_key_name,
        catalog_service_id: Some(catalog.id.clone()),
        icon_url: None,
        node_id: None,
        node_priority: 0,
        service_type: "http".into(),
        ssh_auth_mode: SshAuthMode::ProxyOnly,
        admin_only: false,
        ssh_node_keys_stale: false,
        identity_propagation_mode: identity.identity_propagation_mode,
        identity_include_user_id: identity.identity_include_user_id,
        identity_include_email: identity.identity_include_email,
        identity_include_name: identity.identity_include_name,
        identity_jwt_audience: identity.identity_jwt_audience,
        forward_access_token: identity.forward_access_token,
        inject_delegation_token: identity.inject_delegation_token,
        delegation_token_scope: identity.delegation_token_scope,
        custom_user_agent: None,
        default_request_headers: None,
        ws_frame_injections: Vec::new(),
        is_active: true,
        source: Some(CATALOG_OPT_IN_SOURCE.into()),
        source_id: Some(format!("{owner}:{}", catalog.id)),
        source_app_id: None,
        created_at: now,
        updated_at: now,
        state_version: 1,
        rotation_predecessor_id: None,
        credential_verification: None,
    };
    super::super::proxy_service::verify_auto_provision_eligibility(db, &service, owner).await?;

    // IDs remain fixed across retries. The source identity index serializes
    // simultaneous Connect actions; endpoint and connection commit together.
    for attempt in 0..3 {
        let existing = service_history::collection::<UserService>(db, SERVICES)
            .find_one(
                doc! { "user_id": owner, "catalog_service_id": &catalog.id, "deleted_at": null },
            )
            .await?;
        if existing.is_none() {
            service.slug = resolve_unique_slug(
                db,
                owner,
                &catalog.slug,
                SlugCollisionStrategy::AutoDisambiguate,
            )
            .await?;
        }
        let context = (db, &service, &endpoint);
        let result =
            service_history::transaction::run_send(db, &context, |tx, &(db, service, endpoint)| {
                Box::pin(apply_opt_in(db, tx, service, endpoint))
            })
            .await;
        match result {
            Ok(row) => return Ok(row),
            Err(error) if is_duplicate_key_error(&error) && attempt < 2 => continue,
            Err(error) => return Err(map_transaction_error(error)),
        }
    }
    unreachable!()
}

async fn apply_opt_in(
    db: &mongodb::Database,
    tx: &mut Transaction,
    template: &UserService,
    endpoint: &UserEndpoint,
) -> mongodb::error::Result<UserService> {
    let services = service_history::collection::<UserService>(db, SERVICES);
    let owner = &template.user_id;
    let catalog = template
        .catalog_service_id
        .as_deref()
        .expect("catalog connection");
    services
        .update_many(
            doc! {
                "user_id": owner, "catalog_service_id": catalog,
                "source": CATALOG_OPT_IN_SOURCE, "source_id": { "$type": "string" },
                "deleted_at": { "$ne": null },
            },
            doc! { "$unset": { "source_id": "" } },
        )
        .session(&mut *tx)
        .await?;
    let existing = services
        .find_one(doc! {
            "user_id": owner, "catalog_service_id": catalog,
            "deleted_at": null, "source": { "$ne": AUTO_PROVISION_SOURCE },
        })
        .sort(doc! { "created_at": 1, "_id": 1 })
        .session(&mut *tx)
        .await?;
    if let Some(mut existing) = existing {
        if existing.source.as_deref() == Some(CATALOG_OPT_IN_SOURCE) {
            let repaired = repair_endpoint(db, tx, &existing, endpoint, false).await?;
            if repaired != existing.endpoint_id {
                services.update_one(
                    doc! { "_id": &existing.id, "user_id": owner },
                    doc! {
                        "$set": { "endpoint_id": &repaired, "updated_at": bson::DateTime::now() },
                        "$inc": { "state_version": 1_i64 },
                    },
                ).session(&mut *tx).await?;
                existing.endpoint_id = repaired;
                existing.state_version += 1;
            }
        }
        return Ok(existing);
    }
    let automatic = services
        .find_one(doc! {
            "user_id": owner, "catalog_service_id": catalog,
            "source": AUTO_PROVISION_SOURCE, "deleted_at": null,
        })
        .sort(doc! { "created_at": 1, "_id": 1 })
        .session(&mut *tx)
        .await?;
    if let Some(mut existing) = automatic {
        let repaired = repair_endpoint(db, tx, &existing, endpoint, true).await?;
        services
            .update_one(
                doc! { "_id": &existing.id, "user_id": owner },
                doc! {
                    "$set": {
                        "source": CATALOG_OPT_IN_SOURCE, "source_id": &template.source_id,
                        "endpoint_id": &repaired, "auth_method": &template.auth_method,
                        "auth_key_name": &template.auth_key_name,
                        "credential_binding": &template.credential_binding,
                        "updated_at": bson::DateTime::now(),
                    },
                    "$inc": { "state_version": 1_i64 },
                },
            )
            .session(&mut *tx)
            .await?;
        existing.source = template.source.clone();
        existing.source_id = template.source_id.clone();
        existing.endpoint_id = repaired;
        existing.auth_method = template.auth_method.clone();
        existing.auth_key_name = template.auth_key_name.clone();
        existing.credential_binding = template.credential_binding.clone();
        existing.state_version += 1;
        return Ok(existing);
    }
    transaction_result(
        async {
            service_history::collection::<UserEndpoint>(db, ENDPOINTS)
                .insert_one(endpoint)
                .session(&mut *tx)
                .await?;
            services.insert_one(template).session(&mut *tx).await?;
            Ok(template.clone())
        }
        .await,
    )
}

/// Repair only catalog-owned rows. Adoption refreshes snapshots; an ordinary
/// retry repairs a missing endpoint without replacing an explicit live target.
async fn repair_endpoint(
    db: &mongodb::Database,
    tx: &mut Transaction,
    connection: &UserService,
    template: &UserEndpoint,
    refresh: bool,
) -> mongodb::error::Result<String> {
    let row = db
        .collection::<bson::Document>(ENDPOINTS)
        .find_one(doc! { "_id": &connection.endpoint_id })
        .session(&mut *tx)
        .await?;
    let owned_live = row.as_ref().is_some_and(|row| {
        row.get_str("user_id").ok() == Some(connection.user_id.as_str())
            && row
                .get("deleted_at")
                .is_none_or(|value| matches!(value, bson::Bson::Null))
    });
    if owned_live && !refresh {
        return Ok(connection.endpoint_id.clone());
    }
    let shared = service_history::collection::<UserService>(db, SERVICES)
        .find_one(doc! {
            "_id": { "$ne": &connection.id }, "endpoint_id": &connection.endpoint_id,
            "deleted_at": null,
        })
        .session(&mut *tx)
        .await?
        .is_some();
    let mut repair = template.clone();
    if !shared && (row.is_none() || owned_live) {
        repair.id = connection.endpoint_id.clone();
    }
    let endpoints = service_history::collection::<UserEndpoint>(db, ENDPOINTS);
    if owned_live && !shared {
        endpoints.update_one(
            doc! { "_id": &repair.id, "user_id": &connection.user_id },
            doc! { "$set": {
                "url": &repair.url, "catalog_service_id": &repair.catalog_service_id,
                "openapi_spec_url": &repair.openapi_spec_url, "updated_at": bson::DateTime::now(),
            } },
        ).session(&mut *tx).await?;
    } else {
        endpoints.insert_one(&repair).session(&mut *tx).await?;
    }
    Ok(repair.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::{mcp_service, proxy_service};
    use crate::test_utils::{
        connect_transaction_test_database, test_auto_connected_catalog_service,
        test_encryption_keys, test_user,
    };

    async fn fixture(name: &str) -> (mongodb::Database, String, DownstreamService) {
        let db = connect_transaction_test_database(name).await;
        crate::services::security_readiness_service::ready_test_replica(&db).await;
        let owner = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let mut catalog = test_auto_connected_catalog_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.slug = format!("opt-in-{}", Uuid::new_v4());
        catalog.identity_propagation_mode = "headers".into();
        catalog.identity_include_email = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();
        (db, owner, catalog)
    }

    async fn callable(db: &mongodb::Database, owner: &str, catalog: &DownstreamService) -> bool {
        let manager = std::sync::Arc::new(crate::services::node_ws_manager::NodeWsManager::new(
            30, 100,
        ));
        let result = proxy_service::resolve_proxy_target_from_user_service(
            db,
            &test_encryption_keys(),
            &manager,
            owner,
            Some(&catalog.slug),
            Some(&catalog.id),
            proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(10, 10),
            ),
        )
        .await;
        matches!(result, Ok(Some(_)))
    }

    async fn tools(db: &mongodb::Database, owner: &str) -> usize {
        let manager = crate::services::node_ws_manager::NodeWsManager::new(30, 100);
        mcp_service::load_user_tools_all(db, &manager, owner)
            .await
            .unwrap()
            .len()
    }

    #[tokio::test]
    async fn opted_in_destination_is_managed_across_key_and_assistant_updates() {
        use axum::{
            Json,
            extract::{Path, State},
        };
        let (db, owner, catalog) = fixture("opt_in_managed_destination").await;
        let connection = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let auth = crate::test_utils::test_auth_user(&owner);
        let key_update = Box::pin(crate::handlers::keys::update_key(
            State(state.clone()),
            auth.clone(),
            Path(connection.id.clone()),
            Json(
                serde_json::from_value(
                    serde_json::json!({"endpoint_url": "https://other.example"}),
                )
                .unwrap(),
            ),
        ))
        .await;
        assert!(matches!(key_update, Err(AppError::ValidationError(_))));
        let assistant_update = Box::pin(
            crate::handlers::assistant_action_effects_services::update_service(
                State(state),
                auth,
                Json(
                    serde_json::from_value(serde_json::json!({
                        "actionRequestId": Uuid::new_v4().to_string(),
                        "userServiceId": connection.id,
                        "endpointUrl": "https://other.example",
                    }))
                    .unwrap(),
                ),
            ),
        )
        .await;
        assert!(matches!(
            assistant_update,
            Err(AppError::ValidationError(_))
        ));
        let endpoint = db
            .collection::<UserEndpoint>(ENDPOINTS)
            .find_one(doc! {"_id": &connection.endpoint_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(endpoint.url, catalog.base_url);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn http_and_mcp_require_opt_in_then_honor_disable() {
        let (db, owner, catalog) = fixture("catalog_opt_in_lifecycle").await;
        assert!(!callable(&db, &owner, &catalog).await);
        let limit = crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(10, 10);
        assert!(
            proxy_service::resolve_proxy_target(
                &db,
                &test_encryption_keys(),
                &owner,
                &catalog.id,
                limit
            )
            .await
            .is_err()
        );
        assert_eq!(tools(&db, &owner).await, 0);
        let row = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        assert!(callable(&db, &owner, &catalog).await);
        assert_eq!(tools(&db, &owner).await, 1);
        assert_eq!(row.identity_propagation_mode, "headers");
        assert!(row.identity_include_email);
        crate::services::service_history::collection::<UserService>(&db, SERVICES)
            .update_one(doc! {"_id": &row.id}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        assert!(!callable(&db, &owner, &catalog).await);
        assert_eq!(tools(&db, &owner).await, 0);
        let retry = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        assert_eq!(row.id, retry.id);
        assert!(!retry.is_active);
        crate::services::service_history::collection::<UserService>(&db, SERVICES)
            .update_one(doc! {"_id": &row.id}, doc! {"$set": {"is_active": true}})
            .await
            .unwrap();
        assert!(callable(&db, &owner, &catalog).await);
        assert_eq!(tools(&db, &owner).await, 1);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_retries_create_one_connection_and_no_orphan_endpoint() {
        let (db, owner, catalog) = fixture("catalog_opt_in_concurrent").await;
        let (a, b) = tokio::join!(
            connect_catalog_service(&db, &owner, &owner, &catalog.slug),
            connect_catalog_service(&db, &owner, &owner, &catalog.slug)
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(
            db.collection::<bson::Document>(SERVICES)
                .count_documents(doc! {"user_id": &owner,"deleted_at": null})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<bson::Document>(ENDPOINTS)
                .count_documents(doc! {"user_id": &owner,"deleted_at": null})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn adopts_automatic_identity_repairs_snapshot_and_reconnects_tombstone() {
        let (db, owner, catalog) = fixture("catalog_opt_in_adoption").await;
        let mut automatic = crate::test_utils::test_user_service(
            &Uuid::new_v4().to_string(),
            &owner,
            &catalog.slug,
            &Uuid::new_v4().to_string(),
            Some(&catalog.id),
            None,
        );
        automatic.source = Some(AUTO_PROVISION_SOURCE.into());
        automatic.source_id = Some(format!("{owner}:{}", catalog.id));
        automatic.auth_method = "obsolete".into();
        db.collection::<UserService>(SERVICES)
            .insert_one(&automatic)
            .await
            .unwrap();
        assert!(!callable(&db, &owner, &catalog).await);
        assert_eq!(tools(&db, &owner).await, 0);
        let adopted = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        assert_eq!(adopted.id, automatic.id);
        assert_eq!(adopted.endpoint_id, automatic.endpoint_id);
        assert_eq!(adopted.auth_method, "none");
        assert!(callable(&db, &owner, &catalog).await);
        crate::services::service_history::collection::<UserService>(&db, SERVICES)
            .delete_one(doc! {"_id": &adopted.id})
            .await
            .unwrap();
        let replacement = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        assert_ne!(replacement.id, adopted.id);
        assert!(callable(&db, &owner, &catalog).await);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn inventory_login_and_grant_listing_do_not_create_connections() {
        let (db, owner, _) = fixture("catalog_opt_in_read_only").await;
        list_keys(&db, &test_encryption_keys(), &owner, &HashMap::new())
            .await
            .unwrap();
        crate::services::auth_agent_key_login_service::options_for_actor(&db, &owner)
            .await
            .unwrap();
        assert_eq!(
            db.collection::<bson::Document>(SERVICES)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<bson::Document>(ENDPOINTS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn refuses_inactive_owner_and_org_creation() {
        let (db, owner, catalog) = fixture("catalog_opt_in_owner").await;
        assert!(
            connect_catalog_service(&db, &owner, "another-owner", &catalog.slug)
                .await
                .is_err()
        );
        db.collection::<bson::Document>(USERS)
            .update_one(doc! {"_id": &owner}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        assert!(
            connect_catalog_service(&db, &owner, &owner, &catalog.slug)
                .await
                .is_err()
        );
        assert_eq!(
            db.collection::<bson::Document>(ENDPOINTS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn legacy_master_and_platform_adoption_keep_catalog_credentials() {
        for explicit_platform in [false, true] {
            let (db, owner, mut catalog) = fixture(if explicit_platform {
                "catalog_adopt_platform"
            } else {
                "catalog_adopt_master"
            })
            .await;
            catalog.auth_method = "bearer".into();
            catalog.auth_key_name = "Authorization".into();
            catalog.service_category = "internal".into();
            catalog.visibility = "public".into();
            catalog.provider_config_id = None;
            catalog.credential_encrypted = test_encryption_keys()
                .encrypt(b"catalog-secret")
                .await
                .unwrap();
            if explicit_platform {
                catalog.platform_key=Some(serde_json::from_value(serde_json::json!({"enabled": true,"audience": "public","allowed_owner_ids": []})).unwrap());
            }
            db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .replace_one(doc! {"_id": &catalog.id}, &catalog)
                .await
                .unwrap();
            let mut automatic = crate::test_utils::test_user_service(
                &Uuid::new_v4().to_string(),
                &owner,
                &catalog.slug,
                &Uuid::new_v4().to_string(),
                Some(&catalog.id),
                None,
            );
            automatic.source = Some(AUTO_PROVISION_SOURCE.into());
            automatic.source_id = Some(format!("{owner}:{}", catalog.id));
            automatic.auth_method = "none".into();
            db.collection::<UserService>(SERVICES)
                .insert_one(&automatic)
                .await
                .unwrap();
            let adopted = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
                .await
                .unwrap();
            assert_eq!(adopted.id, automatic.id);
            assert_eq!(adopted.credential_binding.as_deref(), Some("platform"));
            assert_eq!(adopted.auth_method, "bearer");
            assert!(callable(&db, &owner, &catalog).await);
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn private_consent_discovery_is_narrow_and_rechecked_live() {
        let (db, owner, mut catalog) = fixture("catalog_opt_in_private_consent").await;
        let app = Uuid::new_v4().to_string();
        catalog.visibility = "private".into();
        catalog.developer_app_ids = Some(vec![app.clone()]);
        catalog.created_by = "another-owner".into();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! {"_id": &catalog.id}, &catalog)
            .await
            .unwrap();
        assert!(
            crate::services::catalog_service::get_downstream_service_by_slug(
                &db,
                &catalog.slug,
                &owner
            )
            .await
            .is_err()
        );
        let now = bson::DateTime::now();
        db.collection::<bson::Document>(crate::models::oauth_client::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &app,
                "client_name": "Fixture",
                "client_secret_hash": "unused",
                "redirect_uris": [],
                "allowed_scopes": "openid",
                "grant_types": "authorization_code",
                "client_type": "public",
                "is_active": true,
                "created_at": now,
                "updated_at": now
            })
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::consent::COLLECTION_NAME).insert_one(doc! {"_id": Uuid::new_v4().to_string(),"user_id": &owner,"client_id": &app,"scopes": "openid","granted_at": now}).await.unwrap();
        for kind in ["ssh", "credential"] {
            let mut unrelated = catalog.clone();
            unrelated.id = Uuid::new_v4().to_string();
            unrelated.slug = format!("private-{kind}");
            if kind == "ssh" {
                unrelated.service_type = "ssh".into();
            } else {
                unrelated.requires_user_credential = true;
            }
            db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .insert_one(&unrelated)
                .await
                .unwrap();
            assert!(
                crate::services::catalog_service::get_downstream_service_by_slug(
                    &db,
                    &unrelated.slug,
                    &owner
                )
                .await
                .is_err()
            );
        }
        let entries = crate::services::catalog_service::list_catalog_all(
            &db,
            &test_encryption_keys(),
            &owner,
        )
        .await
        .unwrap();
        assert!(entries.iter().any(|entry| entry.slug == catalog.slug));
        assert!(
            !entries
                .iter()
                .any(|entry| entry.slug.starts_with("private-"))
        );
        let row = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        assert!(callable(&db, &owner, &catalog).await);
        db.collection::<bson::Document>(crate::models::consent::COLLECTION_NAME)
            .delete_many(doc! {"user_id": &owner})
            .await
            .unwrap();
        assert!(!callable(&db, &owner, &catalog).await);
        assert!(
            db.collection::<UserService>(SERVICES)
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn catalog_changes_suspend_execution_without_recreating_explicit_identity() {
        let (db, owner, catalog) = fixture("catalog_opt_in_live_catalog").await;
        let row = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        for update in [
            doc! {"is_active": false},
            doc! {"is_active": true,"auth_method": "bearer"},
        ] {
            db.collection::<bson::Document>(DOWNSTREAM_SERVICES)
                .update_one(doc! {"_id": &catalog.id}, doc! {"$set": update})
                .await
                .unwrap();
            assert!(!callable(&db, &owner, &catalog).await);
            assert!(
                db.collection::<UserService>(SERVICES)
                    .find_one(doc! {"_id": &row.id})
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        db.collection::<bson::Document>(DOWNSTREAM_SERVICES)
            .update_one(
                doc! {"_id": &catalog.id},
                doc! {"$set": {"auth_method": "none"}},
            )
            .await
            .unwrap();
        let retry = connect_catalog_service(&db, &owner, &owner, &catalog.slug)
            .await
            .unwrap();
        assert_eq!(row.id, retry.id);
        assert!(callable(&db, &owner, &catalog).await);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn legacy_connection_disable_wins_and_explicit_nodes_remain_available() {
        let (db, owner, catalog) = fixture("catalog_legacy_node_precedence").await;
        db.collection::<bson::Document>(crate::models::node_service_binding::COLLECTION_NAME).insert_one(doc! {"_id": "binding","user_id": &owner,"service_id": &catalog.id,"node_id": "node","is_active": true}).await.unwrap();
        let legacy = crate::models::user_service_connection::UserServiceConnection {
            id: "legacy".into(),
            user_id: owner.clone(),
            service_id: catalog.id.clone(),
            credential_encrypted: None,
            secret_context_versions: Default::default(),
            credential_type: None,
            credential_label: None,
            metadata: None,
            is_active: false,
            state_version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.collection::<crate::models::user_service_connection::UserServiceConnection>(
            crate::models::user_service_connection::COLLECTION_NAME,
        )
        .insert_one(&legacy)
        .await
        .unwrap();
        let limit = crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(10, 10);
        assert!(matches!(
            proxy_service::resolve_proxy_target_lenient(
                &db,
                &test_encryption_keys(),
                &owner,
                &catalog.id,
                limit
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        db.collection::<bson::Document>(crate::models::user_service_connection::COLLECTION_NAME)
            .delete_one(doc! {"_id": "legacy"})
            .await
            .unwrap();
        proxy_service::resolve_proxy_target_lenient(
            &db,
            &test_encryption_keys(),
            &owner,
            &catalog.id,
            limit,
        )
        .await
        .unwrap();
        let mut automatic = crate::test_utils::test_user_service(
            "automatic",
            &owner,
            &catalog.slug,
            "missing",
            Some(&catalog.id),
            None,
        );
        automatic.source = Some(AUTO_PROVISION_SOURCE.into());
        db.collection::<UserService>(SERVICES)
            .insert_one(&automatic)
            .await
            .unwrap();
        let manager = std::sync::Arc::new(crate::services::node_ws_manager::NodeWsManager::new(
            30, 100,
        ));
        let result = proxy_service::resolve_proxy_target_from_user_service(
            &db,
            &test_encryption_keys(),
            &manager,
            &owner,
            Some(&catalog.slug),
            Some(&catalog.id),
            proxy_service::ProxyExecutionContext::new(None, limit),
        )
        .await
        .unwrap();
        assert!(
            result.is_none(),
            "explicit legacy node routing must retain its own resolver"
        );
        db.drop().await.unwrap();
    }
}
