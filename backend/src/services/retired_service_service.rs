//! Tombstones for credential stores left by the removed Platform Operations feature.
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{COLLECTION_NAME, DownstreamService};

pub const RETIRED_CATEGORY: &str = "retired_platform_vendor";

pub fn is_retired(service: &DownstreamService) -> bool {
    service.service_category == RETIRED_CATEGORY
        || (service.service_category == "internal" && service.slug.starts_with("platform-"))
}

pub fn require_available(service: &DownstreamService) -> AppResult<()> {
    if is_retired(service) {
        Err(AppError::NotFound("Service not found".into()))
    } else {
        Ok(())
    }
}

pub fn exclusion_filter() -> Document {
    doc! { "$nor": [
        { "service_category": RETIRED_CATEGORY },
        { "service_category": "internal", "slug": { "$regex": "^platform-" } },
    ] }
}

/// Runs before serving traffic. The namespace guard also rejects rows created
/// after this sweep by an older replica. No credential or historical row moves.
pub async fn retire_legacy_vendors(db: &mongodb::Database) -> AppResult<()> {
    let services = db.collection::<Document>(COLLECTION_NAME);
    let mut cursor = services
        .find(doc! { "$or": [
            { "service_category": "internal", "slug": { "$regex": "^platform-" } },
            { "service_category": RETIRED_CATEGORY },
        ] })
        .projection(doc! { "_id": 1 })
        .await?;
    while let Some(row) = cursor.try_next().await? {
        let id = row
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid retired service ID".into()))?;
        services.update_one(doc! { "_id": id }, doc! { "$set": {
            "service_category": RETIRED_CATEGORY,
            "is_active": false,
            "platform_key": { "enabled": false, "audience": "restricted", "allowed_owner_ids": [] },
            "proxy_operation_policy": { "rules": [] },
        }}).await?;
        for (collection, field) in [
            (
                crate::models::user_service::COLLECTION_NAME,
                "catalog_service_id",
            ),
            (
                crate::models::user_service_connection::COLLECTION_NAME,
                "service_id",
            ),
        ] {
            db.collection::<Document>(collection)
                .update_many(
                    doc! { field: id, "is_active": true },
                    doc! { "$set": { "is_active": false } },
                )
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_service::{COLLECTION_NAME as BINDINGS, UserService};
    use crate::services::{
        catalog_service, platform_key_service, proxy_service, unified_key_service,
        user_service_service,
    };
    use crate::test_utils::{
        connect_test_database, test_auto_connected_catalog_service, test_encryption_keys,
        test_user, test_user_service,
    };

    #[tokio::test]
    async fn retirement_is_idempotent_preserves_material_and_blocks_old_writers() {
        let Some(db) = connect_test_database("retired_vendors").await else {
            return;
        };
        let keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let mut vendor = test_auto_connected_catalog_service();
        vendor.slug = "platform-telnyx".into();
        vendor.service_category = "internal".into();
        vendor.auth_method = "bearer".into();
        vendor.credential_encrypted = keys.encrypt(b"legacy-fixture").await.unwrap();
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(&vendor)
            .await
            .unwrap();
        let binding = test_user_service(
            "legacy-binding",
            &user_id,
            "legacy-alias",
            "endpoint",
            Some(&vendor.id),
            None,
        );
        db.collection::<UserService>(BINDINGS)
            .insert_one(&binding)
            .await
            .unwrap();
        let mut ordinary = vendor.clone();
        ordinary.id = uuid::Uuid::new_v4().to_string();
        ordinary.slug = "platform-test".into();
        ordinary.service_category = "connection".into();
        ordinary.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
            enabled: true,
            audience: crate::models::downstream_service::PlatformKeyAudience::Public,
            allowed_owner_ids: vec![],
        });
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(&ordinary)
            .await
            .unwrap();
        retire_legacy_vendors(&db).await.unwrap();
        retire_legacy_vendors(&db).await.unwrap();
        let stored = db
            .collection::<DownstreamService>(COLLECTION_NAME)
            .find_one(doc! { "_id": &vendor.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_active);
        assert_eq!(stored.service_category, RETIRED_CATEGORY);
        assert_eq!(stored.credential_encrypted, vendor.credential_encrypted);
        assert!(
            !db.collection::<UserService>(BINDINGS)
                .find_one(doc! { "_id": &binding.id })
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        assert!(platform_key_service::has_platform_key(&ordinary));
        db.collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_user_endpoint(
            "endpoint",
            &user_id,
            "fixture",
            "https://example.com",
            None,
            Some(&vendor.id),
        ))
        .await
        .unwrap();
        // Simulate an old writer after startup, including an explicit binding.
        vendor.id = uuid::Uuid::new_v4().to_string();
        vendor.slug = "platform-old-writer".into();
        vendor.anonymous_endpoints =
            vec![crate::models::downstream_service::AnonymousEndpointRule {
                id: uuid::Uuid::new_v4().to_string(),
                enabled: true,
                method: "GET".into(),
                path_pattern: "/models".into(),
                daily_quota: 10,
            }];
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(&vendor)
            .await
            .unwrap();
        let binding = test_user_service(
            "new-legacy-binding",
            &user_id,
            "renamed-alias",
            "endpoint",
            Some(&vendor.id),
            None,
        );
        db.collection::<UserService>(BINDINGS)
            .insert_one(&binding)
            .await
            .unwrap();
        assert!(!platform_key_service::has_platform_key(&vendor));
        assert!(matches!(
            crate::services::anonymous_endpoint_service::find_matching_enabled_rule(
                &db,
                &vendor.slug,
                "GET",
                "/models"
            )
            .await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            proxy_service::authorize_master_credential_server_chosen(&db, &vendor).await,
            Err(AppError::NotFound(_))
        ));
        let limit = crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(0, 10);
        assert!(matches!(
            proxy_service::resolve_proxy_target_lenient(&db, &keys, &user_id, &vendor.id, limit)
                .await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            proxy_service::resolve_proxy_target_by_user_service_id(
                &db,
                &keys,
                &user_id,
                &binding.id,
                Some(&binding.slug),
                None,
                proxy_service::ProxyExecutionContext::new(None, limit)
            )
            .await,
            Err(AppError::NotFound(_))
        ));
        let state = crate::test_utils::test_app_state(db.clone());
        for service_id in [&vendor.id, &binding.id] {
            assert!(matches!(
                crate::handlers::services_helpers::resolve_service_or_user_service(
                    &state, service_id, &user_id
                )
                .await,
                Err(AppError::NotFound(_))
            ));
        }
        let catalog = catalog_service::list_catalog_all(&db, &keys, &user_id)
            .await
            .unwrap();
        assert!(
            catalog
                .iter()
                .all(|s| s.slug != vendor.slug && s.slug != stored.slug)
        );
        assert!(catalog.iter().any(|s| s.slug == ordinary.slug));
        assert!(
            user_service_service::list_user_services_with_sources(&db, &user_id)
                .await
                .unwrap()
                .is_empty()
        );
        unified_key_service::auto_provision_no_auth_services(&db, &user_id)
            .await
            .unwrap();
        assert_eq!(
            db.collection::<UserService>(BINDINGS)
                .count_documents(
                    doc! { "catalog_service_id": &vendor.id, "source": "auto_provision" }
                )
                .await
                .unwrap(),
            0
        );
        let mut renamed = stored;
        renamed.slug = "renamed-retired".into();
        renamed.is_active = true;
        assert!(is_retired(&renamed));
        assert!(matches!(
            proxy_service::authorize_master_credential_server_chosen(&db, &renamed).await,
            Err(AppError::NotFound(_))
        ));

        // User aliases are not historical catalog identities. The real catalog
        // guard must allow this internal service while still enforcing its policy.
        let mut internal = ordinary.clone();
        internal.id = uuid::Uuid::new_v4().to_string();
        internal.slug = "ordinary-internal".into();
        internal.service_category = "internal".into();
        internal.auth_method = "none".into();
        internal.platform_key = None;
        internal.credential_encrypted.clear();
        internal.proxy_operation_policy =
            Some(crate::models::downstream_service::ProxyOperationPolicy {
                rules: vec![crate::models::downstream_service::ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/models".into(),
                }],
            });
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(&internal)
            .await
            .unwrap();
        let alias = test_user_service(
            "internal-alias",
            &user_id,
            "platform-personal",
            "endpoint",
            Some(&internal.id),
            None,
        );
        db.collection::<UserService>(BINDINGS)
            .insert_one(&alias)
            .await
            .unwrap();
        let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
            &db,
            &keys,
            &user_id,
            &alias.id,
            Some(&alias.slug),
            None,
            proxy_service::ProxyExecutionContext::new(None, limit),
        )
        .await
        .unwrap()
        .unwrap();
        let path = crate::services::proxy_authorization::CanonicalPath::from_mcp_literal("/models")
            .unwrap();
        crate::services::proxy_authorization::authorize_proxy_operation(
            &resolved.target.service,
            "GET",
            &path,
        )
        .unwrap();
        assert!(
            crate::services::proxy_authorization::authorize_proxy_operation(
                &resolved.target.service,
                "POST",
                &path
            )
            .is_err()
        );
        db.drop().await.unwrap();
    }
}
