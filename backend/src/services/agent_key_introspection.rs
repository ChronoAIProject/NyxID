//! Service-bound RFC 7662 evidence for ordinary agent API keys.
//! The confidential resource server must be registered on the target catalog
//! service. Neither a public OAuth client nor a key's owner identity alone
//! authorizes a downstream service.

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::ApiKeyPurpose,
        api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CREDENTIALS},
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService, OfferingKind},
        oauth_client::OauthClient,
        service_approval_config::{
            ApprovalEffect, COLLECTION_NAME as APPROVAL_CONFIGS, ServiceApprovalConfig,
        },
        user_service::{AUTO_PROVISION_SOURCE, COLLECTION_NAME as CONNECTIONS, UserService},
    },
    services::{key_service, platform_key_service, proxy_service, rbac_helpers},
};

pub struct Evidence {
    pub subject: String,
    pub service_id: String,
    pub key_id: String,
    pub scope: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub permissions: Vec<String>,
}

fn refused() -> AppError {
    AppError::Unauthorized("Agent key is not active for this resource server".into())
}

pub async fn introspect(
    db: &Database,
    client: &OauthClient,
    service_id: &str,
    raw_key: &str,
) -> AppResult<Evidence> {
    if client.client_type != "confidential" || !client.is_active {
        return Err(refused());
    }
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": service_id, "is_active": true})
        .await?
        .ok_or_else(refused)?;
    if !service
        .introspection_client_ids
        .as_ref()
        .is_some_and(|ids| ids.contains(&client.id))
    {
        return Err(refused());
    }
    // Service-wide evidence cannot express per-operation publication or route
    // policy. Only an ordinary HTTP resource server can use this contract.
    if service.service_type != "http"
        || service.service_category == "provider"
        || service.offering_kind == OfferingKind::Tool
        || service.proxy_operation_policy.is_some()
        || super::retired_service_service::is_retired(&service)
        || !platform_key_service::no_auth_platform_service(&service)
        || service.requires_user_credential
    {
        return Err(refused());
    }
    // Whole-service evidence is for resource servers that authenticate callers
    // themselves. Never substitute it for NyxID's credential/provider admission
    // (including the fail-closed master-credential operation policy).
    if db
        .collection::<mongodb::bson::Document>(
            crate::models::service_provider_requirement::COLLECTION_NAME,
        )
        .find_one(doc! {"service_id": service_id})
        .projection(doc! {"_id": 1})
        .await?
        .is_some()
    {
        return Err(refused());
    }
    let (subject, key, child_id) = key_service::validate_api_key(db, raw_key).await?;
    // Those credentials carry narrower authority than a service audience and
    // owner RBAC can encode. A request-bound protocol must preserve their live
    // operation, input, organization, turn, guest and approval fences first.
    if key.purpose != ApiKeyPurpose::General
        || !key.scopes.split_whitespace().any(|s| s == "proxy")
        || !key.assistant_operation_scopes.is_empty()
        || key.assistant_agent_owner_id.is_some()
        || key.assistant_group_id.is_some()
    {
        return Err(refused());
    }
    // Reuse the same live key owner, purpose and grant construction as proxy
    // authentication; never reconstruct a human from the key's subject alone.
    let authority = crate::mw::auth::api_key_auth_user(db, &key, child_id.clone(), None, None)
        .await
        .map_err(authority_error)?;
    if authority.user_id.to_string() != subject
        || authority.assistant_chat.is_some()
        || authority.assistant_turn_fence.is_some()
        || authority.org_agent_access.is_some()
    {
        return Err(refused());
    }
    authority
        .ensure_rest_proxy_access()
        .map_err(authority_error)?;
    let mut filter = doc! {"user_id": &subject, "catalog_service_id": service_id, "is_active": true, "deleted_at": null};
    if !authority.allow_all_services
        && !authority
            .allowed_service_ids
            .iter()
            .any(|id| id == service_id)
    {
        filter.insert("_id", doc! {"$in": &authority.allowed_service_ids});
    }
    // An active, owner-bound service connection must exist. Introspection never
    // provisions a connection or widens an agent's allowlist. Catalog grants
    // and UserService IDs are distinct identities in the effective allowlist.
    let mut connections = db
        .collection::<UserService>(CONNECTIONS)
        .find(filter)
        .await?;
    let mut permitted = false;
    while let Some(connection) = connections.try_next().await? {
        if connection.service_type != "http"
            || connection.node_id.is_some()
            || connection.auth_method != "none"
            || connection.api_key_id.is_some()
        {
            continue;
        }
        match proxy_service::verify_auto_provision_eligibility(db, &connection, &subject).await {
            Ok(()) => {}
            Err(error) if is_unavailable(&error) => return Err(error),
            Err(_) => continue,
        }
        if !allows_service_wide_approval(db, &subject, &service.id, &connection).await? {
            continue;
        }
        if service.platform_key.is_some() {
            match platform_key_service::require(db, &service, &subject).await {
                Ok(()) => {}
                Err(error) if is_unavailable(&error) => return Err(error),
                Err(_) => continue,
            }
        }
        permitted = true;
        break;
    }
    if !permitted {
        return Err(refused());
    }
    let mut expires_at = key.expires_at;
    if let Some(child_id) = child_id {
        let child = db.collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id": child_id, "api_key_id": &key.id, "user_id": &subject, "is_active": true})
            .await?.ok_or_else(refused)?;
        expires_at = earliest_expiry(expires_at, child.expires_at);
    }
    if expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(refused());
    }
    let rbac = rbac_helpers::resolve_user_rbac(db, &subject).await?;
    Ok(Evidence {
        subject,
        service_id: service.id,
        key_id: key.id,
        scope: key.scopes,
        expires_at,
        created_at: key.created_at,
        permissions: rbac.permissions,
    })
}

/// Audience-only evidence cannot carry a pending approval or operation rules.
/// Inspect both identities so direct catalog access cannot evade a connection's
/// policy, and use the same live global preference as ordinary proxy requests.
async fn allows_service_wide_approval(
    db: &Database,
    owner: &str,
    catalog_id: &str,
    connection: &UserService,
) -> AppResult<bool> {
    let configs: Vec<ServiceApprovalConfig> = db
        .collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
        .find(doc! {"user_id": owner, "service_id": {"$in": [catalog_id, &connection.id]}})
        .await?
        .try_collect()
        .await?;
    if configs.iter().any(|config| {
        !config.rules.is_empty()
            || match config.default_effect {
                Some(ApprovalEffect::AutoAllow) => false,
                Some(_) => true,
                None => config.approval_required,
            }
    }) {
        return Ok(false);
    }
    if connection.source.as_deref() == Some(AUTO_PROVISION_SOURCE) {
        return Ok(true);
    }
    // An explicit unrestricted connection policy overrides the global fallback.
    if configs
        .iter()
        .any(|config| config.service_id == connection.id)
    {
        return Ok(true);
    }
    Ok(!super::approval_service::user_requires_approval(db, owner).await?)
}

fn is_unavailable(error: &AppError) -> bool {
    matches!(
        error,
        AppError::DatabaseError(_) | AppError::Internal(_) | AppError::OrgQueryTimeout
    )
}

fn authority_error(error: AppError) -> AppError {
    if is_unavailable(&error) {
        error
    } else {
        refused()
    }
}

fn earliest_expiry(
    parent: Option<DateTime<Utc>>,
    child: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (parent, child) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests;
