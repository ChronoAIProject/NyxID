use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;

use super::{api_key_mutation_service as transactions, retired_service_service};
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};

pub fn primary_requirement_filter(service: &DownstreamService) -> mongodb::bson::Document {
    let mut filter = doc! { "service_id": &service.id };
    if let Some(provider_id) = &service.provider_config_id {
        filter.insert("provider_config_id", provider_id);
    }
    filter
}

pub async fn list_linked(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<Vec<DownstreamService>> {
    let requirements: Vec<ServiceProviderRequirement> = db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .find(doc! { "provider_config_id": provider_id })
        .await?
        .try_collect()
        .await?;
    let ids: Vec<_> = requirements.iter().map(|r| r.service_id.as_str()).collect();
    Ok(db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! { "$and": [
            { "$or": [{ "provider_config_id": provider_id }, { "_id": { "$in": ids } }] },
            retired_service_service::exclusion_filter(),
        ] })
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?)
}

/// The service and its requirement change in one transaction. Repeating the
/// same link is safe; replacing another provider requires a separate decision.
pub async fn link(
    db: &mongodb::Database,
    provider_id: &str,
    service_id: &str,
    new_service: Option<&DownstreamService>,
) -> AppResult<()> {
    let db = db.clone();
    let provider_id = provider_id.to_string();
    let service_id = service_id.to_string();
    let new_service = new_service.cloned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let provider = db.collection::<ProviderConfig>(PROVIDERS)
                .find_one(doc! { "_id": &provider_id, "is_active": true }).session(&mut *session).await?
                .ok_or_else(|| AppError::NotFound("Provider not found or inactive".into()))?;
            if !matches!(provider.provider_type.as_str(), "api_key" | "oauth2" | "device_code") {
                return Err(AppError::ValidationError("This provider does not supply service credentials".into()));
            }
            let services = db.collection::<DownstreamService>(SERVICES);
            if let Some(service) = new_service.as_ref() {
                services.insert_one(service).session(&mut *session).await?;
            }
            let service = services.find_one(doc! { "_id": &service_id }).session(&mut *session).await?
                .ok_or_else(|| AppError::NotFound("Service not found".into()))?;
            retired_service_service::require_available(&service)?;
            if service.service_type != "http" || service.auth_method == "oidc" {
                return Err(AppError::ValidationError("Only downstream HTTP services can be linked".into()));
            }
            if service.provider_config_id.as_deref().is_some_and(|id| id != provider_id.as_str()) {
                return Err(AppError::Conflict("Service is already linked to another provider".into()));
            }
            if provider.requires_gateway_url && service.platform_key.as_ref().is_some_and(|c| c.enabled) {
                return Err(AppError::ValidationError("Gateway providers cannot use a platform key".into()));
            }
            if new_service.is_none() && service.provider_config_id.as_deref() == Some(provider_id.as_str()) {
                return Ok(());
            }
            if service.platform_key.is_none() && !service.credential_encrypted.is_empty() {
                return Err(AppError::ValidationError("Configure an explicit platform key policy before linking a service with a shared credential".into()));
            }
            let requirements = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);
            let mut cursor = requirements.find(doc! { "service_id": &service_id }).session(&mut *session).await?;
            let mut existing = None;
            while let Some(requirement) = cursor.next(&mut *session).await.transpose()? {
                if requirement.provider_config_id != provider_id.as_str() || existing.is_some() {
                    return Err(AppError::Conflict("Service has other provider requirements; reconcile them explicitly before linking".into()));
                }
                existing = Some(requirement);
            }
            if service.auth_method == "none" {
                let requirement = existing.as_ref().ok_or_else(|| AppError::ValidationError(
                    "Provider-backed auth requires an existing provider requirement; choose a direct injection method or configure the requirement first".into()))?;
                if !requirement.required || !matches!(requirement.injection_method.as_str(), "bearer" | "header" | "query" | "path") {
                    return Err(AppError::ValidationError("The primary provider requirement must be required and use a supported injection method".into()));
                }
            } else if existing.is_some() {
                return Err(AppError::Conflict("Direct-auth services cannot have provider requirements".into()));
            }
            services.update_one(doc! { "_id": &service_id }, doc! { "$set": {
                "provider_config_id": &provider_id,
                "requires_user_credential": true,
                "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
            }}).session(&mut *session).await?;
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn add_requirement(
    db: &mongodb::Database,
    requirement: &ServiceProviderRequirement,
) -> AppResult<()> {
    mutate_requirement(
        db,
        &requirement.service_id,
        Some(requirement),
        &requirement.id,
    )
    .await
}

pub async fn remove_requirement(
    db: &mongodb::Database,
    service_id: &str,
    requirement_id: &str,
) -> AppResult<()> {
    mutate_requirement(db, service_id, None, requirement_id).await
}

async fn mutate_requirement(
    db: &mongodb::Database,
    service_id: &str,
    addition: Option<&ServiceProviderRequirement>,
    requirement_id: &str,
) -> AppResult<()> {
    let db = db.clone();
    let service_id = service_id.to_string();
    let requirement_id = requirement_id.to_string();
    let addition = addition.cloned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let services = db.collection::<DownstreamService>(SERVICES);
            let service = services.find_one(doc! { "_id": &service_id }).session(&mut *session).await?
                .ok_or_else(|| AppError::NotFound("Service not found".into()))?;
            retired_service_service::require_available(&service)?;
            if service.provider_config_id.is_some() && service.auth_method != "none" {
                return Err(AppError::Conflict("Direct-auth services cannot have provider requirements".into()));
            }
            let requirements = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);
            if addition.is_none() {
                let requirement = requirements.find_one(doc! { "_id": &requirement_id, "service_id": &service_id }).session(&mut *session).await?
                    .ok_or_else(|| AppError::NotFound("Requirement not found".into()))?;
                if service.provider_config_id.as_deref() == Some(requirement.provider_config_id.as_str()) {
                    return Err(AppError::Conflict("The primary provider requirement cannot be removed while the service is linked".into()));
                }
            } else if let Some(requirement) = addition.as_ref()
                && service.provider_config_id.as_deref() == Some(requirement.provider_config_id.as_str())
                && !requirement.required
            {
                return Err(AppError::ValidationError("The primary provider requirement must be required".into()));
            }
            // Serialize with linking, including attempts that started before a
            // canonical link existed. Mongo retries and rechecks this gate.
            services.update_one(doc! { "_id": &service_id }, doc! { "$set": {
                "updated_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
            }, "$inc": { "provider_link_revision": 1 } }).session(&mut *session).await?;
            let requirements = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);
            if let Some(requirement) = addition.as_ref() {
                if requirements.find_one(doc! { "service_id": &service_id, "provider_config_id": &requirement.provider_config_id }).session(&mut *session).await?.is_some() {
                    return Err(AppError::Conflict("This provider requirement already exists".into()));
                }
                requirements.insert_one(requirement).session(&mut *session).await?;
            } else if requirements.delete_one(doc! { "_id": &requirement_id, "service_id": &service_id }).session(&mut *session).await?.deleted_count == 0 {
                return Err(AppError::NotFound("Requirement not found".into()));
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::{
        delegation_service, provider_service, proxy_service, unified_key_service,
    };
    use crate::test_utils::{connect_test_database, test_encryption_keys, test_user};

    #[tokio::test]
    async fn seeded_direct_and_delegated_links_preserve_auth_and_secondary_requirements() {
        let Some(db) = connect_test_database("provider_link_seeded").await else {
            return;
        };
        let keys = test_encryption_keys();
        provider_service::seed_default_providers(&db, &keys)
            .await
            .unwrap();
        provider_service::seed_default_services(&db, &keys)
            .await
            .unwrap();
        let services = db.collection::<DownstreamService>(SERVICES);
        let telnyx = services
            .find_one(doc! { "slug": "api-telnyx" })
            .await
            .unwrap()
            .unwrap();
        let provider_id = telnyx.provider_config_id.as_deref().unwrap();
        link(&db, provider_id, &telnyx.id, None).await.unwrap();
        link(&db, provider_id, &telnyx.id, None).await.unwrap();
        let after = services
            .find_one(doc! { "_id": &telnyx.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            mongodb::bson::to_document(&telnyx).unwrap(),
            mongodb::bson::to_document(&after).unwrap()
        );
        assert_eq!(
            db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
                .count_documents(doc! { "service_id": &telnyx.id })
                .await
                .unwrap(),
            0
        );
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let created = unified_key_service::create_key(
            &db,
            &keys,
            &user_id,
            &user_id,
            Some("api-telnyx"),
            None,
            "byok-fixture",
            "Telnyx",
            None,
            None,
            None,
            None,
            None,
            None,
            unified_key_service::OpenApiSpecUrlInput::Inherit,
            None,
            false,
            unified_key_service::OauthClientCredentialsInput::None,
            false,
        )
        .await
        .unwrap();
        let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
            &db,
            &keys,
            &user_id,
            &created.service.id,
            Some(&created.service.slug),
            None,
            proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(0, 10),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(resolved.target.service.auth_method, "bearer");
        assert!(
            delegation_service::resolve_delegated_credentials(
                &db, &keys, &user_id, &telnyx.id, None
            )
            .await
            .unwrap()
            .is_empty()
        );
        let delegated = services
            .find_one(doc! { "provider_config_id": { "$ne": null }, "auth_method": "none" })
            .await
            .unwrap()
            .unwrap();
        let primary_id = delegated.provider_config_id.as_deref().unwrap();
        let primary = db
            .collection::<ServiceProviderRequirement>(REQUIREMENTS)
            .find_one(primary_requirement_filter(&delegated))
            .await
            .unwrap()
            .unwrap();
        let mut secondary = primary.clone();
        secondary.id = uuid::Uuid::new_v4().to_string();
        secondary.provider_config_id = provider_id.into();
        secondary.required = false;
        secondary.injection_method = "header".into();
        secondary.injection_key = Some("X-Secondary-Key".into());
        add_requirement(&db, &secondary).await.unwrap();
        link(&db, primary_id, &delegated.id, None).await.unwrap();
        assert_eq!(
            crate::services::platform_key_service::effective_auth(&db, &delegated)
                .await
                .unwrap(),
            unified_key_service::derive_effective_auth(&delegated, Some(&primary))
        );
        remove_requirement(&db, &delegated.id, &secondary.id)
            .await
            .unwrap();
        assert!(matches!(
            remove_requirement(&db, &delegated.id, &primary.id).await,
            Err(AppError::Conflict(_))
        ));
        secondary.service_id = telnyx.id.clone();
        assert!(matches!(
            add_requirement(&db, &secondary).await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            link(&db, primary_id, &telnyx.id, None).await,
            Err(AppError::Conflict(_))
        ));
        // Failed creation rolls back the service insert as well as any link.
        let mut invalid = telnyx.clone();
        invalid.id = uuid::Uuid::new_v4().to_string();
        invalid.slug = "rollback-service".into();
        assert!(
            link(&db, primary_id, &invalid.id, Some(&invalid))
                .await
                .is_err()
        );
        assert!(
            services
                .find_one(doc! { "_id": &invalid.id })
                .await
                .unwrap()
                .is_none()
        );
        // Linking and requirement writes contend on the catalog row. Either
        // order is valid, but direct authentication must never acquire an SPR.
        for _ in 0..4 {
            let mut direct = telnyx.clone();
            direct.id = uuid::Uuid::new_v4().to_string();
            direct.slug = format!("direct-{}", direct.id);
            direct.provider_config_id = None;
            services.insert_one(&direct).await.unwrap();
            let mut requirement = primary.clone();
            requirement.id = uuid::Uuid::new_v4().to_string();
            requirement.service_id = direct.id.clone();
            let (linked, added) = tokio::join!(
                link(&db, provider_id, &direct.id, None),
                add_requirement(&db, &requirement),
            );
            assert_ne!(linked.is_ok(), added.is_ok());
            let stored = services
                .find_one(doc! { "_id": &direct.id })
                .await
                .unwrap()
                .unwrap();
            let count = db
                .collection::<ServiceProviderRequirement>(REQUIREMENTS)
                .count_documents(doc! { "service_id": &direct.id })
                .await
                .unwrap();
            assert_eq!(stored.provider_config_id.is_some(), count == 0);
        }
        db.drop().await.unwrap();
    }
}
