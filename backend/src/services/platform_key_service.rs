//! Live authorization for catalog-held credentials. No credential material leaves
//! this module's callers except through the authorized proxy transport.
use crate::models::org_membership::OrgMembership;
use crate::models::provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig};
use futures::TryStreamExt;
use mongodb::bson::doc;
use std::collections::{HashMap, HashSet};

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    DownstreamService, PlatformKeyAudience, PlatformKeyConfig,
};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service::{AUTO_PROVISION_SOURCE, UserService};
use crate::services::org_service;

/// Read-only status for management responses. Old creators encrypted even an
/// absent credential. Decrypt through the version-aware API, never infer usable
/// material from ciphertext length. None means stored material is unreadable.
pub async fn credential_configured(
    keys: &crate::crypto::aes::EncryptionKeys,
    service: &DownstreamService,
) -> Option<bool> {
    if service.credential_encrypted.is_empty() {
        return Some(false);
    }
    match keys.decrypt(&service.credential_encrypted).await {
        Ok(material) => Some(!zeroize::Zeroizing::new(material).is_empty()),
        Err(_) => None,
    }
}

pub fn legacy_master_credential(service: &DownstreamService) -> bool {
    !super::retired_service_service::is_retired(service)
        && service.service_category == "internal"
        && service.auth_method != "none"
        && !service.requires_user_credential
        && service.service_type == "http"
        && !service.credential_encrypted.is_empty()
        && service.provider_config_id.is_none()
}

pub fn legacy_public_master(service: &DownstreamService) -> bool {
    !super::retired_service_service::is_retired(service)
        && service.visibility == "public"
        && service.service_category == "internal"
        && !matches!(service.auth_method.as_str(), "none" | "token_exchange")
        && !service.requires_user_credential
        && service.service_type == "http"
        && service.is_active
        && !service.credential_encrypted.is_empty()
        && service.provider_config_id.is_none()
}

pub fn has_platform_key(service: &DownstreamService) -> bool {
    if super::retired_service_service::is_retired(service) {
        return false;
    }
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

/// One request's active personal/org grants. Never cache across requests: access
/// removal must take effect at the next credential resolution.
pub struct OwnerGrants {
    actor_id: String,
    active_owner_ids: HashSet<String>,
    readable_owner_ids: HashSet<String>,
    org_owner_ids: HashSet<String>,
    memberships: Vec<OrgMembership>,
}

impl OwnerGrants {
    pub async fn load(db: &mongodb::Database, owner_id: &str) -> AppResult<Self> {
        let memberships = org_service::find_active_memberships_with_timeout(db, owner_id).await?;
        Self::from_memberships(db, owner_id, &memberships).await
    }

    /// Listings already need memberships for org rows, including viewers. Keep
    /// that snapshot with the grants so provisioning and rendering can share it.
    pub async fn load_for_listing(db: &mongodb::Database, owner_id: &str) -> AppResult<Self> {
        let memberships = org_service::list_memberships_for_member(db, owner_id, false).await?;
        Self::from_memberships(db, owner_id, &memberships).await
    }

    pub fn readable_owner_ids(&self) -> &HashSet<String> {
        &self.readable_owner_ids
    }

    pub fn memberships(&self) -> &[OrgMembership] {
        &self.memberships
    }

    /// Reuse an existing request membership snapshot (e.g. LLM status).
    pub async fn from_memberships(
        db: &mongodb::Database,
        owner_id: &str,
        memberships: &[OrgMembership],
    ) -> AppResult<Self> {
        let mut ids = vec![owner_id.to_string()];
        ids.extend(
            memberships
                .iter()
                .filter(|m| m.revoked_at.is_none() && m.member_user_id == owner_id)
                .map(|m| m.org_user_id.clone()),
        );
        let owners: Vec<User> = db
            .collection::<User>(USERS)
            .find(doc! { "_id": { "$in": &ids }, "is_active": true })
            .await?
            .try_collect()
            .await?;
        let actor_active = owners.iter().any(|u| u.id == owner_id);
        let org_owner_ids = owners
            .iter()
            .filter(|u| u.user_type.is_org())
            .map(|u| u.id.clone())
            .collect();
        let readable_owner_ids: HashSet<String> = owners
            .into_iter()
            .filter(|u| actor_active && (u.id == owner_id || u.user_type.is_org()))
            .map(|u| u.id)
            .collect();
        let active_owner_ids = readable_owner_ids
            .iter()
            .filter(|id| {
                id.as_str() == owner_id
                    || memberships.iter().any(|m| {
                        m.org_user_id == **id
                            && m.member_user_id == owner_id
                            && m.revoked_at.is_none()
                            && m.role.can_proxy()
                    })
            })
            .cloned()
            .collect();
        Ok(Self {
            actor_id: owner_id.to_string(),
            active_owner_ids,
            readable_owner_ids,
            org_owner_ids,
            memberships: memberships.to_vec(),
        })
    }

    fn permits(&self, owner_id: &str, allowed: &[String]) -> bool {
        self.active_owner_ids.contains(owner_id)
            && allowed.iter().any(|id| {
                id == owner_id || (owner_id == self.actor_id && self.active_owner_ids.contains(id))
            })
    }
}

async fn provider_supports_platform_key(
    db: &mongodb::Database,
    provider_id: Option<&str>,
) -> AppResult<bool> {
    let Some(id) = provider_id else {
        return Ok(true);
    };
    Ok(db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! { "_id": id })
        .await?
        .is_some_and(|p| !p.requires_gateway_url))
}

pub async fn available(
    db: &mongodb::Database,
    service: &DownstreamService,
    owner_id: &str,
) -> AppResult<bool> {
    if !has_platform_key(service)
        || !provider_supports_platform_key(db, service.provider_config_id.as_deref()).await?
    {
        return Ok(false);
    }
    let Some(config) = &service.platform_key else {
        return Ok(true);
    };
    if config.audience == PlatformKeyAudience::Public {
        return Ok(true);
    }
    Ok(OwnerGrants::load(db, owner_id)
        .await?
        .permits(owner_id, &config.allowed_owner_ids))
}

/// Load provider configuration once for a listing/provisioning request, shared
/// across its personal and org owners. Never retain this snapshot across requests.
/// Catalog/status callers reuse their existing targeted provider batch instead.
pub async fn load_providers(db: &mongodb::Database) -> AppResult<HashMap<String, ProviderConfig>> {
    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(PROVIDERS)
        .find(doc! {})
        .await?
        .try_collect()
        .await?;
    Ok(providers.into_iter().map(|p| (p.id.clone(), p)).collect())
}

/// Pure authorization against this request's catalog, provider and owner
/// snapshots. A missing/mismatched provider fails closed for linked services.
pub fn available_with_grants(
    service: &DownstreamService,
    provider: Option<&ProviderConfig>,
    owner_id: &str,
    grants: &OwnerGrants,
) -> bool {
    if !has_platform_key(service)
        || service
            .provider_config_id
            .as_deref()
            .is_some_and(|id| !provider.is_some_and(|p| p.id == id && !p.requires_gateway_url))
    {
        return false;
    }
    service.platform_key.as_ref().is_none_or(|config| {
        config.audience == PlatformKeyAudience::Public
            || grants.permits(owner_id, &config.allowed_owner_ids)
    })
}

/// Automatic connections have narrower ownership rules than explicit bindings:
/// public keys belong to active people; restricted keys require a direct owner
/// grant. Keep this separate from the execution ACL used by explicit org rows.
pub fn auto_provisionable_with_grants(
    service: &DownstreamService,
    provider: Option<&ProviderConfig>,
    owner_id: &str,
    grants: &OwnerGrants,
) -> bool {
    let Some(config) = &service.platform_key else {
        return false;
    };
    grants.active_owner_ids.contains(owner_id)
        && match config.audience {
            PlatformKeyAudience::Public => !grants.org_owner_ids.contains(owner_id),
            PlatformKeyAudience::Restricted => {
                config.allowed_owner_ids.iter().any(|id| id == owner_id)
            }
        }
        && available_with_grants(service, provider, owner_id, grants)
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
        .find_one(crate::services::provider_link_service::primary_requirement_filter(service))
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
    provider_id: Option<&str>,
) -> AppResult<()> {
    if config.allowed_owner_ids.len() > 1000 {
        return Err(AppError::ValidationError(
            "platform_key allows at most 1000 owners".to_string(),
        ));
    }
    config.allowed_owner_ids.sort();
    config.allowed_owner_ids.dedup();
    if config.enabled && !provider_supports_platform_key(db, provider_id).await? {
        return Err(AppError::ValidationError(
            "Platform keys are unavailable for providers that require a user gateway URL".into(),
        ));
    }
    let invalid_id = config
        .allowed_owner_ids
        .iter()
        .any(|id| uuid::Uuid::parse_str(id).is_err());
    let owners = if config.allowed_owner_ids.is_empty() {
        0
    } else {
        db.collection::<User>(USERS)
            .count_documents(
                doc! { "_id": { "$in": &config.allowed_owner_ids }, "is_active": true },
            )
            .await?
    };
    if invalid_id || owners as usize != config.allowed_owner_ids.len() {
        return Err(AppError::ValidationError(
            "platform_key owner must identify an active person or organization".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
