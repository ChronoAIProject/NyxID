use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ServiceCapabilities,
};
#[cfg(test)]
use crate::models::org_membership::OrgMembership;
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::org_service;

/// A catalog entry combining DownstreamService + ProviderConfig info.
pub struct CatalogEntry {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub provider_config_id: Option<String>,
    pub provider_type: Option<String>,
    pub requires_gateway_url: bool,
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
    pub credential_mode: Option<String>,
    // SSH fields
    pub service_type: String,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_ca_public_key: Option<String>,
    pub ssh_allowed_principals: Option<Vec<String>>,
    pub ssh_certificate_ttl_minutes: Option<u32>,
    // OAuth config fields (for node-native OAuth)
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub device_code_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub device_token_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub supports_pkce: bool,
    pub device_code_format: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub extra_auth_params: Option<HashMap<String, String>>,
    pub oauth_client_id: Option<String>,
    pub client_id_param_name: Option<String>,
    /// Whether this catalog entry needs credential setup instead of direct no-auth access.
    pub requires_credential: bool,
    // --- Rich metadata for AI agent discovery ---
    pub openapi_spec_url: Option<String>,
    pub asyncapi_spec_url: Option<String>,
    pub homepage_url: Option<String>,
    pub repository_url: Option<String>,
    pub issues_url: Option<String>,
    pub capabilities: Option<ServiceCapabilities>,
    pub auth_notes: Option<String>,
    pub known_limitations: Option<String>,
    pub required_permissions: Option<Vec<String>>,
    pub examples_url: Option<String>,
    pub recommended_skills: Option<Vec<String>>,
    /// Declared credential fields for `token_exchange` services. When set,
    /// clients should render one input per field (text vs password per the
    /// `secret` flag) and compose a JSON object from the values before
    /// submitting. The full `TokenExchangeConfig` stays server-side --
    /// clients never see the endpoint URL, request template, or injection
    /// format, only what to collect from the user.
    pub token_exchange_credential_fields:
        Option<Vec<crate::models::downstream_service::CredentialFieldSpec>>,
    /// Admin-configured default HTTP headers declared on the catalog
    /// `DownstreamService`. Exposed read-only so the per-user AI Services
    /// UI can show catalog inheritance next to the user's overrides
    /// (NyxID#356). `None` when the catalog entry has no defaults.
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
}

fn build_catalog_entry(
    svc: DownstreamService,
    provider: Option<&ProviderConfig>,
    spr: Option<&ServiceProviderRequirement>,
    oauth_client_id: Option<String>,
) -> CatalogEntry {
    // A service requires a credential if:
    // 1. It requires per-user credentials (connection services), OR
    // 2. It has an actual auth method (not "none"), OR
    // 3. It has auth_method "none" but an SPR exists (uses master credentials)
    let requires_credential =
        svc.requires_user_credential || svc.auth_method != "none" || spr.is_some();
    CatalogEntry {
        service_type: svc.service_type.clone(),
        ssh_host: svc.ssh_config.as_ref().map(|c| c.host.clone()),
        ssh_port: svc.ssh_config.as_ref().map(|c| c.port),
        ssh_ca_public_key: svc
            .ssh_config
            .as_ref()
            .and_then(|c| c.ca_public_key.clone()),
        ssh_allowed_principals: svc
            .ssh_config
            .as_ref()
            .map(|c| c.allowed_principals.clone()),
        ssh_certificate_ttl_minutes: svc.ssh_config.as_ref().map(|c| c.certificate_ttl_minutes),
        slug: svc.slug,
        name: svc.name,
        description: svc.description,
        base_url: svc.base_url,
        // For internal services (auth_method="none"), resolve actual injection
        // from ServiceProviderRequirement (e.g., bearer/Authorization, header/x-api-key, query/key)
        auth_method: if svc.auth_method == "none" {
            spr.map(|r| r.injection_method.clone())
                .unwrap_or_else(|| svc.auth_method)
        } else {
            svc.auth_method
        },
        auth_key_name: if svc.auth_key_name.is_empty() {
            spr.and_then(|r| r.injection_key.clone())
                .unwrap_or_else(|| "Authorization".to_string())
        } else {
            svc.auth_key_name
        },
        provider_config_id: provider.map(|p| p.id.clone()),
        provider_type: provider.map(|p| p.provider_type.clone()),
        requires_gateway_url: provider.is_some_and(|p| p.requires_gateway_url),
        api_key_instructions: provider.and_then(|p| p.api_key_instructions.clone()),
        api_key_url: provider.and_then(|p| p.api_key_url.clone()),
        icon_url: provider.and_then(|p| p.icon_url.clone()),
        documentation_url: provider.and_then(|p| p.documentation_url.clone()),
        credential_mode: provider.map(|p| p.credential_mode.clone()),
        // OAuth config
        authorization_url: provider.and_then(|p| p.authorization_url.clone()),
        token_url: provider.and_then(|p| p.token_url.clone()),
        device_code_url: provider.and_then(|p| p.device_code_url.clone()),
        device_verification_url: provider.and_then(|p| p.device_verification_url.clone()),
        device_token_url: provider.and_then(|p| p.device_token_url.clone()),
        default_scopes: provider.and_then(|p| p.default_scopes.clone()),
        supports_pkce: provider.is_some_and(|p| p.supports_pkce),
        device_code_format: provider.map(|p| p.device_code_format.clone()),
        token_endpoint_auth_method: provider.map(|p| p.token_endpoint_auth_method.clone()),
        extra_auth_params: provider.and_then(|p| p.extra_auth_params.clone()),
        oauth_client_id,
        client_id_param_name: provider.and_then(|p| p.client_id_param_name.clone()),
        requires_credential,
        openapi_spec_url: svc.openapi_spec_url,
        asyncapi_spec_url: svc.asyncapi_spec_url,
        homepage_url: svc.homepage_url,
        repository_url: svc.repository_url,
        issues_url: svc.issues_url,
        capabilities: svc.capabilities,
        auth_notes: svc.auth_notes,
        known_limitations: svc.known_limitations,
        required_permissions: svc.required_permissions,
        examples_url: svc.examples_url,
        recommended_skills: svc.recommended_skills,
        token_exchange_credential_fields: svc.token_exchange_config.map(|c| c.credential_fields),
        default_request_headers: svc.default_request_headers,
    }
}

async fn decrypt_provider_client_id(
    provider: &ProviderConfig,
    encryption_keys: &EncryptionKeys,
) -> AppResult<Option<String>> {
    let Some(encrypted) = provider.client_id_encrypted.as_ref() else {
        return Ok(None);
    };

    let decrypted = encryption_keys.decrypt(encrypted).await?;
    let client_id = String::from_utf8(decrypted)
        .map_err(|_| AppError::Internal("Failed to decode provider client_id".to_string()))?;
    if client_id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(client_id))
    }
}

/// MongoDB filter for visibility that hides private services from non-owners.
/// Public services and legacy documents without a visibility field are visible to all.
fn visibility_filter(user_id: &str) -> mongodb::bson::Document {
    doc! {
        "$or": [
            { "visibility": { "$ne": "private" } },
            { "visibility": { "$exists": false } },
            { "visibility": "private", "created_by": user_id },
        ],
    }
}

/// MongoDB filter for service_category that handles legacy documents
/// created before the field was added (defaults to "connection").
fn legacy_service_category_filter(categories: &[&str]) -> mongodb::bson::Document {
    doc! {
        "$or": categories.iter().map(|c| doc! { "service_category": c }).chain(
            std::iter::once(doc! { "service_category": { "$exists": false } })
        ).collect::<Vec<_>>(),
    }
}

/// List catalog entries available for user key creation.
/// Filters to connection-category + provider-linked services.
///
/// Enforces visibility: private services are only visible to their
/// creator (admin overrides happen at the handler layer if needed).
/// Without this filter, the response would include
/// `default_request_headers` and other metadata for private services,
/// which the slug endpoint already restricts — the list path used to
/// undo that restriction.
pub async fn list_catalog(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
) -> AppResult<Vec<CatalogEntry>> {
    // Legacy documents may lack requires_user_credential (defaults to true)
    // and service_category (defaults to "connection").
    list_catalog_filtered(
        db,
        encryption_keys,
        doc! {
            "service_type": "http",
            "is_active": true,
            "$and": [
                {
                    "$or": [
                        { "requires_user_credential": true },
                        { "requires_user_credential": { "$exists": false } },
                        { "provider_config_id": { "$ne": null } },
                    ],
                },
                legacy_service_category_filter(&["connection", "internal"]),
                visibility_filter(user_id),
            ],
        },
    )
    .await
}

/// List ALL active catalog entries for discovery (includes system services without auth).
/// Enforces visibility: private services only visible to their creator.
pub async fn list_catalog_all(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
) -> AppResult<Vec<CatalogEntry>> {
    let filter = doc! {
        "is_active": true,
        "$and": [
            legacy_service_category_filter(&["connection", "internal"]),
            visibility_filter(user_id),
        ],
    };
    list_catalog_filtered(db, encryption_keys, filter).await
}

async fn list_catalog_filtered(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    filter: mongodb::bson::Document,
) -> AppResult<Vec<CatalogEntry>> {
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    // Batch-load all referenced provider configs
    let provider_ids: Vec<&str> = services
        .iter()
        .filter_map(|s| s.provider_config_id.as_deref())
        .collect();

    let providers: Vec<ProviderConfig> = if provider_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find(doc! { "_id": { "$in": &provider_ids } })
            .await?
            .try_collect()
            .await?
    };

    // Batch-load service provider requirements to get actual auth injection config
    let svc_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let sprs: Vec<ServiceProviderRequirement> = if svc_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
            .find(doc! { "service_id": { "$in": &svc_ids } })
            .await?
            .try_collect()
            .await?
    };

    let mut resolved_entries = Vec::with_capacity(services.len());
    for svc in services {
        let provider = svc
            .provider_config_id
            .as_ref()
            .and_then(|pid| providers.iter().find(|p| &p.id == pid));

        let spr = sprs.iter().find(|r| r.service_id == svc.id);

        let oauth_client_id = match provider {
            Some(provider) if provider.credential_mode != "user" => {
                decrypt_provider_client_id(provider, encryption_keys).await?
            }
            _ => None,
        };

        resolved_entries.push(build_catalog_entry(svc, provider, spr, oauth_client_id));
    }

    Ok(resolved_entries)
}

/// Look up the catalog entry's `required_permissions` list by id.
///
/// Returns an empty `Vec` when the catalog row is missing, has the field
/// unset, or the lookup fails. No visibility check is needed: callers
/// already hold the `catalog_service_id` via their own `UserService`
/// row, which is itself how access is granted to the underlying catalog
/// entry. Used by `handlers/keys.rs` to derive the Lark / Feishu
/// permission setup deep link without reaching directly into the
/// `DownstreamService` collection.
pub async fn get_required_permissions(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> Vec<String> {
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": catalog_service_id })
        .await
        .ok()
        .flatten()
        .and_then(|svc| svc.required_permissions)
        .unwrap_or_default()
}

/// Get the raw DownstreamService by slug (lightweight, no provider/encryption lookup).
///
/// Enforces the same layered visibility rules as `get_catalog_entry`:
/// public services are readable by everyone; private services are
/// readable by the creator, admins, or any user with an active
/// `UserService` (personal or org-owned, membership-scoped) referencing
/// the service. Without this alignment, endpoint-discovery via
/// `/catalog/{slug}/endpoints` would return 404 for private rows that
/// the same caller can access on the parent `/catalog/{slug}`.
pub async fn get_downstream_service_by_slug(
    db: &mongodb::Database,
    slug: &str,
    user_id: &str,
) -> AppResult<DownstreamService> {
    let svc = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog entry not found".to_string()))?;

    enforce_catalog_read_access(db, user_id, &svc).await?;

    Ok(svc)
}

/// Enforce the layered catalog-read access check for a loaded
/// `DownstreamService`. Returns `Err(NotFound)` (with the same shape as
/// a missing slug) when the caller is not permitted to read the entry.
///
/// Callers are responsible for loading `svc` first; both
/// `get_catalog_entry` and `get_downstream_service_by_slug` use this
/// helper so their visibility rules cannot drift.
async fn enforce_catalog_read_access(
    db: &mongodb::Database,
    user_id: &str,
    svc: &DownstreamService,
) -> AppResult<()> {
    if svc.visibility != "private" || svc.created_by == user_id {
        return Ok(());
    }
    let is_admin = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .is_some_and(|u| u.is_admin);
    let has_active_user_service = if is_admin {
        false
    } else {
        has_active_user_service_for_catalog(db, user_id, &svc.id).await?
    };
    if !caller_may_read_catalog_entry(
        &svc.visibility,
        &svc.created_by,
        user_id,
        is_admin,
        has_active_user_service,
    ) {
        return Err(AppError::NotFound("Catalog entry not found".to_string()));
    }
    Ok(())
}

/// Pure-function visibility decision for a single catalog entry.
///
/// Extracted so the rule can be unit-tested without spinning up Mongo.
/// Returns `true` when `user_id` is allowed to read the entry. The
/// caller is responsible for the database lookups that produce
/// `is_admin` and `has_active_user_service` before invoking this.
pub(crate) fn caller_may_read_catalog_entry(
    visibility: &str,
    created_by: &str,
    user_id: &str,
    is_admin: bool,
    has_active_user_service: bool,
) -> bool {
    if visibility != "private" {
        return true;
    }
    if created_by == user_id {
        return true;
    }
    if is_admin {
        return true;
    }
    has_active_user_service
}

/// Get single catalog entry by slug, enforcing visibility against the
/// requesting user.
///
/// Private catalog services were previously readable by any authenticated
/// user who could guess or obtain the slug — exposing field values such as
/// `default_request_headers` (which can carry routing / scope hints).
/// This function now restricts access to:
///   1. anyone, when the service is public / has no visibility field
///   2. the creator of a private service
///   3. admins
///   4. users who already have an active `UserService` referencing this
///      catalog entry — needed so the inherited-defaults panel keeps
///      working for auto-provisioned no-auth services without re-leaking
///      the row to unrelated callers.
///
/// All other lookups return `NotFound` (the same shape as a missing slug,
/// so private services don't even leak existence).
pub async fn get_catalog_entry(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    slug: &str,
) -> AppResult<CatalogEntry> {
    let svc = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog entry not found".to_string()))?;

    enforce_catalog_read_access(db, user_id, &svc).await?;

    let provider = if let Some(ref pid) = svc.provider_config_id {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find_one(doc! { "_id": pid })
            .await?
    } else {
        None
    };

    let spr = db
        .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
        .find_one(doc! { "service_id": &svc.id })
        .await?;

    let oauth_client_id = match provider.as_ref() {
        Some(provider) if provider.credential_mode != "user" => {
            decrypt_provider_client_id(provider, encryption_keys).await?
        }
        _ => None,
    };

    Ok(build_catalog_entry(
        svc,
        provider.as_ref(),
        spr.as_ref(),
        oauth_client_id,
    ))
}

/// Does `user_id` have an active provisioned `UserService` for catalog
/// `catalog_service_id`? Checks personal rows first (common case), then
/// falls back to org-owned rows reachable through an active membership.
///
/// Org services store `UserService.user_id = org_user_id`, so a plain
/// `{user_id}` lookup would miss them and deny catalog visibility to
/// legitimate org members. Uses `OrgMembership.allows_resource` to
/// respect effective member scopes — a viewer with a scoped membership
/// does NOT inherit visibility to services outside their scope.
async fn has_active_user_service_for_catalog(
    db: &mongodb::Database,
    user_id: &str,
    catalog_service_id: &str,
) -> AppResult<bool> {
    // Fast path: personal row.
    let personal = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "user_id": user_id,
            "catalog_service_id": catalog_service_id,
            "is_active": true,
        })
        .await?
        .is_some();
    if personal {
        return Ok(true);
    }

    // Org fallback. If the user has no active memberships the answer is
    // definitively no; skip the second query.
    let memberships = org_service::find_active_memberships_with_timeout(db, user_id).await?;
    if memberships.is_empty() {
        return Ok(false);
    }

    let org_user_ids: Vec<&str> = memberships.iter().map(|m| m.org_user_id.as_str()).collect();
    let candidates: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": { "$in": &org_user_ids },
            "catalog_service_id": catalog_service_id,
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    for us in candidates {
        let Some(membership) = memberships.iter().find(|m| m.org_user_id == us.user_id) else {
            continue;
        };
        let effective_scope =
            crate::services::org_role_scope_service::effective_scope_for_membership(db, membership)
                .await?;
        if crate::services::org_role_scope_service::scope_allows(&effective_scope, &us.id) {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Pure-function matcher: is there a `UserService` in `candidates` whose
/// owning `org_user_id` corresponds to an `OrgMembership` whose
/// `allowed_service_ids` scope covers that `UserService.id`?
///
/// Extracted so the scope-matching logic is unit-testable without Mongo.
///
/// **Caveat:** this compares against `m.allowed_service_ids` directly and
/// does NOT resolve `scope_source = Inherit` via the role-scope collection.
/// Callers constructing test memberships must set `scope_source = Override`
/// with the desired explicit list — otherwise Inherit memberships look
/// like "full access" here even when a stricter role scope exists.
/// Production enforcement always flows through
/// [`has_active_user_service_for_catalog`], which uses
/// `org_role_scope_service::effective_scope_for_membership`.
#[cfg(test)]
pub(crate) fn any_org_service_reachable(
    candidates: &[UserService],
    memberships: &[OrgMembership],
) -> bool {
    candidates.iter().any(|us| {
        memberships.iter().any(|m| {
            m.org_user_id == us.user_id
                && crate::services::org_role_scope_service::scope_allows(
                    &m.allowed_service_ids,
                    &us.id,
                )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{any_org_service_reachable, caller_may_read_catalog_entry};
    use crate::models::org_membership::OrgMembership;
    use crate::models::user_service::UserService;
    use chrono::Utc;

    // The visibility decision is the load-bearing piece of the
    // information-disclosure fix in NyxID#356: any path that broadens
    // it would re-leak `default_request_headers` (and other catalog
    // metadata) for private services. Pin the rules here so a future
    // refactor can't quietly reopen the hole.

    #[test]
    fn public_entries_are_readable_by_everyone() {
        assert!(caller_may_read_catalog_entry(
            "public", "alice", "bob", false, false,
        ));
        // Legacy rows missing the visibility field surface as something
        // other than "private"; this branch must default to allow.
        assert!(caller_may_read_catalog_entry(
            "", "alice", "bob", false, false,
        ));
    }

    #[test]
    fn private_creator_can_always_read() {
        assert!(caller_may_read_catalog_entry(
            "private", "alice", "alice", false, false,
        ));
    }

    #[test]
    fn private_admin_can_read() {
        assert!(caller_may_read_catalog_entry(
            "private", "alice", "bob", true, false,
        ));
    }

    #[test]
    fn private_user_with_active_service_can_read() {
        // Auto-provisioned no-auth keys backed by a private catalog
        // entry need this exception so the inherited-defaults panel
        // works.
        assert!(caller_may_read_catalog_entry(
            "private", "alice", "bob", false, true,
        ));
    }

    #[test]
    fn private_user_without_relationship_is_denied() {
        // Plain authenticated user with no link to the row: the
        // disclosure path Codex flagged. Must stay denied.
        assert!(!caller_may_read_catalog_entry(
            "private", "alice", "bob", false, false,
        ));
    }

    #[test]
    fn private_user_with_inactive_service_is_denied() {
        // Soft-deleted user-service must NOT keep catalog visibility.
        // The handler caller is responsible for filtering
        // `is_active: true` in its lookup; this assertion just pins
        // the contract — `has_active_user_service: false` blocks.
        assert!(!caller_may_read_catalog_entry(
            "private", "alice", "bob", false, false,
        ));
    }

    // ---- org visibility tests for any_org_service_reachable ----

    fn user_service(id: &str, user_id: &str) -> UserService {
        UserService {
            id: id.to_string(),
            user_id: user_id.to_string(),
            slug: "test".to_string(),
            endpoint_id: "ep-1".to_string(),
            api_key_id: None,
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            catalog_service_id: Some("cat-1".to_string()),
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::ProxyOnly,
            ssh_node_keys_stale: false,
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn membership(org_user_id: &str, allowed: Option<Vec<String>>) -> OrgMembership {
        OrgMembership {
            id: format!("mem-{org_user_id}"),
            org_user_id: org_user_id.to_string(),
            member_user_id: "bob".to_string(),
            role: crate::models::org_membership::OrgRole::Member,
            scope_source: crate::models::org_membership::MemberScopeSource::Override,
            allowed_service_ids: allowed,
            created_at: Utc::now(),
            revoked_at: None,
        }
    }

    #[test]
    fn org_member_with_unrestricted_membership_can_see_catalog() {
        // The concrete case that regressed: an org user's UserService
        // is stored under the org's synthetic user_id, not the member's.
        // A membership with no `allowed_service_ids` scope (full access)
        // must grant visibility.
        let svc = user_service("us-1", "org-1");
        let memberships = vec![membership("org-1", None)];
        assert!(any_org_service_reachable(&[svc], &memberships));
    }

    #[test]
    fn org_member_with_matching_scope_can_see_catalog() {
        let svc = user_service("us-1", "org-1");
        let memberships = vec![membership("org-1", Some(vec!["us-1".to_string()]))];
        assert!(any_org_service_reachable(&[svc], &memberships));
    }

    #[test]
    fn org_member_scoped_to_other_services_cannot_see_catalog() {
        // A viewer whose membership is restricted to a different
        // UserService MUST NOT inherit catalog visibility through this
        // path. The scope check is the gate.
        let svc = user_service("us-1", "org-1");
        let memberships = vec![membership("org-1", Some(vec!["us-2".to_string()]))];
        assert!(!any_org_service_reachable(&[svc], &memberships));
    }

    #[test]
    fn empty_scope_denies_all_services() {
        // `allowed_service_ids = Some([])` means explicitly allow
        // nothing — the viewer is gated out entirely.
        let svc = user_service("us-1", "org-1");
        let memberships = vec![membership("org-1", Some(vec![]))];
        assert!(!any_org_service_reachable(&[svc], &memberships));
    }

    #[test]
    fn no_matching_membership_denies_access() {
        // UserService owned by org-1 but the caller only holds a
        // membership in org-2.
        let svc = user_service("us-1", "org-1");
        let memberships = vec![membership("org-2", None)];
        assert!(!any_org_service_reachable(&[svc], &memberships));
    }

    #[test]
    fn no_candidates_means_no_access() {
        let memberships = vec![membership("org-1", None)];
        assert!(!any_org_service_reachable(&[], &memberships));
    }
}
