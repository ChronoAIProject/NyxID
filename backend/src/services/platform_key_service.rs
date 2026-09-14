//! Live authorization for catalog-held credentials. No credential material leaves
//! this module's callers except through the authorized proxy transport.
use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    DownstreamService, PlatformKeyAudience, PlatformKeyConfig,
};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service::{AUTO_PROVISION_SOURCE, UserService};
use crate::services::org_service::{self, OwnerAccess};

pub fn legacy_public_master(service: &DownstreamService) -> bool {
    service.visibility == "public"
        && service.service_category == "internal"
        && !matches!(service.auth_method.as_str(), "none" | "token_exchange")
        && !service.requires_user_credential
        && service.service_type == "http"
        && service.is_active
        && !service.credential_encrypted.is_empty()
        && service.provider_config_id.is_none()
}

pub fn has_platform_key(service: &DownstreamService) -> bool {
    match &service.platform_key {
        Some(config) => {
            config.enabled
                && service.is_active
                && service.service_type == "http"
                && !service.credential_encrypted.is_empty()
        }
        None => legacy_public_master(service),
    }
}

pub fn binding(service: &UserService) -> &str {
    service.credential_binding.as_deref().unwrap_or_else(|| {
        if service.api_key_id.is_none() && service.source.as_deref() == Some(AUTO_PROVISION_SOURCE)
        {
            "platform"
        } else {
            "user"
        }
    })
}

/// `owner_id` is already selected through the normal personal/org resource ACL.
/// An org grant can also authorize a person's own connection, but never another
/// unrelated owner's connection. Membership lookups validate both users' activity.
pub async fn available(
    db: &mongodb::Database,
    service: &DownstreamService,
    owner_id: &str,
) -> AppResult<bool> {
    if !has_platform_key(service) {
        return Ok(false);
    }
    let Some(config) = &service.platform_key else {
        return Ok(true);
    };
    if config.audience == PlatformKeyAudience::Public {
        return Ok(true);
    }
    if config.allowed_owner_ids.iter().any(|id| id == owner_id) {
        return Ok(db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": owner_id, "is_active": true })
            .await?
            .is_some());
    }
    for allowed in &config.allowed_owner_ids {
        match org_service::resolve_owner_access(db, owner_id, allowed).await? {
            OwnerAccess::AsOrgAdmin { .. } => return Ok(true),
            OwnerAccess::AsOrgMember { role, .. } if role.can_proxy() => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

pub async fn require(
    db: &mongodb::Database,
    service: &DownstreamService,
    owner_id: &str,
) -> AppResult<()> {
    if available(db, service, owner_id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(
            "Service is no longer available".to_string(),
        ))
    }
}

pub async fn effective_auth(
    db: &mongodb::Database,
    service: &DownstreamService,
) -> AppResult<(String, String)> {
    let requirement = db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .find_one(doc! { "service_id": &service.id })
        .await?;
    let auth = super::unified_key_service::derive_effective_auth(service, requirement.as_ref());
    if auth.0 == "none" {
        return Err(AppError::ValidationError(
            "A platform key requires a credential injection method".to_string(),
        ));
    }
    Ok(auth)
}

pub async fn validate_config(
    db: &mongodb::Database,
    config: &mut PlatformKeyConfig,
) -> AppResult<()> {
    if config.allowed_owner_ids.len() > 1000 {
        return Err(AppError::ValidationError(
            "platform_key allows at most 1000 owners".to_string(),
        ));
    }
    config.allowed_owner_ids.sort();
    config.allowed_owner_ids.dedup();
    for id in &config.allowed_owner_ids {
        if uuid::Uuid::parse_str(id).is_err()
            || db
                .collection::<User>(USERS)
                .find_one(doc! { "_id": id, "is_active": true })
                .await?
                .is_none()
        {
            return Err(AppError::ValidationError(
                "platform_key owner must identify an active person or organization".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
