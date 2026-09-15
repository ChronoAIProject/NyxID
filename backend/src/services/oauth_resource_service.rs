use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::org_membership::OrgMembership;
use crate::models::user_service::UserService;
use crate::services::{catalog_service, org_role_scope_service, org_service, user_service_service};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOAuthResources {
    /// Canonical user-service resource URIs, index-paired with `service_ids`.
    pub resource_uris: Vec<String>,
    pub service_ids: Vec<String>,
    /// Canonical `{BASE_URL}/mcp` when the request included the NyxID MCP
    /// endpoint as an RFC 8707 resource (NyxID#1226). Tracked separately so
    /// the `resource_uris`/`service_ids` pairing invariant holds: the MCP
    /// endpoint identifies NyxID itself, not a user service, and is neutral
    /// for service narrowing (mcp-only behaves like omitting `resource`).
    pub mcp_resource_uri: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthTokenResourceScope {
    pub resource_uris: Vec<String>,
    pub allowed_service_ids: Vec<String>,
    pub allow_all_services: bool,
}

pub fn user_service_resource_uri(config: &AppConfig, slug: &str) -> String {
    format!(
        "{}/api/v1/proxy/s/{}",
        config.base_url.trim_end_matches('/'),
        slug
    )
}

/// Canonical RFC 8707 resource URI for the NyxID MCP endpoint itself —
/// exactly what RFC 9728 protected-resource metadata advertises
/// (`handlers/oidc_discovery.rs`), and therefore what spec-compliant MCP
/// clients send back on `/oauth/authorize` and `/oauth/token` (NyxID#1226).
pub fn mcp_resource_uri(config: &AppConfig) -> String {
    format!("{}/mcp", config.base_url.trim_end_matches('/'))
}

/// True when `resource` identifies the NyxID MCP endpoint (trailing-slash
/// tolerant; comparison is on the canonical form).
pub fn is_mcp_resource(config: &AppConfig, resource: &str) -> bool {
    resource.trim_end_matches('/') == mcp_resource_uri(config)
}

/// Canonicalize a requested resource for narrowing comparison: MCP-endpoint
/// variants collapse to the canonical `{BASE_URL}/mcp`; everything else is
/// returned unchanged (user-service URIs are already exact-match).
fn canonicalize_resource(config: &AppConfig, resource: &str) -> String {
    if is_mcp_resource(config, resource) {
        mcp_resource_uri(config)
    } else {
        resource.to_string()
    }
}

pub fn validate_resource_uri(resource: &str) -> AppResult<()> {
    let parsed = url::Url::parse(resource)
        .map_err(|_| AppError::InvalidTarget("resource must be an absolute URI".to_string()))?;

    if parsed.fragment().is_some() {
        return Err(AppError::InvalidTarget(
            "resource must not include a fragment".to_string(),
        ));
    }

    Ok(())
}

pub async fn resolve_requested_resources(
    db: &mongodb::Database,
    config: &AppConfig,
    actor_user_id: &str,
    resources: Option<&[String]>,
) -> AppResult<Option<ResolvedOAuthResources>> {
    let Some(resources) = resources else {
        return Ok(None);
    };

    let mut resource_uris = Vec::new();
    let mut service_ids = Vec::new();
    let mut mcp_resource = None;

    for resource in resources {
        validate_resource_uri(resource)?;
        if is_mcp_resource(config, resource) {
            // The NyxID MCP endpoint itself (NyxID#1226): a valid audience,
            // not a user service — no service narrowing contribution.
            mcp_resource = Some(mcp_resource_uri(config));
            continue;
        }
        let service = resolve_single_resource(db, config, actor_user_id, resource).await?;
        let canonical = user_service_resource_uri(config, &service.slug);
        if !resource_uris.iter().any(|existing| existing == &canonical) {
            resource_uris.push(canonical);
        }
        if !service_ids.iter().any(|existing| existing == &service.id) {
            service_ids.push(service.id);
        }
    }

    Ok(Some(ResolvedOAuthResources {
        resource_uris,
        service_ids,
        mcp_resource_uri: mcp_resource,
    }))
}

pub async fn resolve_resource_service_ids_for_user(
    db: &mongodb::Database,
    config: &AppConfig,
    actor_user_id: &str,
    resources: &[String],
) -> AppResult<Vec<String>> {
    let resolved = resolve_requested_resources(db, config, actor_user_id, Some(resources)).await?;
    Ok(resolved.map(|r| r.service_ids).unwrap_or_default())
}

pub async fn can_grant_user_service(
    db: &mongodb::Database,
    actor_user_id: &str,
    service: &UserService,
) -> AppResult<bool> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &service.user_id).await?;
    Ok(owner_access_can_grant_user_service(&access, service))
}

fn owner_access_can_grant_user_service(
    access: &org_service::OwnerAccess,
    service: &UserService,
) -> bool {
    match access {
        org_service::OwnerAccess::Direct => true,
        org_service::OwnerAccess::AsOrgAdmin { .. } => access.allows_resource(&service.id),
        org_service::OwnerAccess::AsOrgMember { role, .. } => {
            crate::services::user_service_service::role_can_proxy_service(*role, service)
                && access.allows_resource(&service.id)
        }
        org_service::OwnerAccess::Forbidden => false,
    }
}

async fn membership_can_grant_user_service(
    db: &mongodb::Database,
    membership: &OrgMembership,
    service: &UserService,
) -> AppResult<bool> {
    let effective_scope =
        org_role_scope_service::effective_scope_for_membership(db, membership).await?;
    let access = match membership.role {
        crate::models::org_membership::OrgRole::Admin => org_service::OwnerAccess::AsOrgAdmin {
            org_user_id: membership.org_user_id.clone(),
            membership_id: membership.id.clone(),
            allowed_service_ids: effective_scope,
        },
        crate::models::org_membership::OrgRole::Member
        | crate::models::org_membership::OrgRole::Viewer => org_service::OwnerAccess::AsOrgMember {
            org_user_id: membership.org_user_id.clone(),
            membership_id: membership.id.clone(),
            role: membership.role,
            allowed_service_ids: effective_scope,
        },
    };

    Ok(owner_access_can_grant_user_service(&access, service))
}

pub async fn validate_grantable_service_ids(
    db: &mongodb::Database,
    actor_user_id: &str,
    service_ids: &[String],
) -> AppResult<bool> {
    for service_id in service_ids {
        let Some(service) = user_service_service::find_user_service_by_id(db, service_id).await?
        else {
            return Ok(false);
        };
        if !can_grant_user_service(db, actor_user_id, &service).await? {
            return Ok(false);
        }
    }

    Ok(true)
}

pub async fn resolve_token_resource_scope(
    db: &mongodb::Database,
    config: &AppConfig,
    actor_user_id: &str,
    requested_resources: Option<&[String]>,
    grant_resource_uris: &[String],
    grant_allowed_service_ids: &[String],
    grant_allow_all_services: bool,
) -> AppResult<OAuthTokenResourceScope> {
    let requested = requested_resources.filter(|resources| !resources.is_empty());
    if grant_allow_all_services && grant_allowed_service_ids.is_empty() {
        let Some(resources) = requested else {
            return Ok(OAuthTokenResourceScope {
                resource_uris: grant_resource_uris.to_vec(),
                allowed_service_ids: vec![],
                allow_all_services: true,
            });
        };
        let resolved = resolve_requested_resources(db, config, actor_user_id, Some(resources))
            .await?
            .ok_or_else(|| AppError::InvalidTarget("Missing resource selection".into()))?;
        let service_restricting = !resolved.service_ids.is_empty();
        let mut resource_uris = resolved.resource_uris;
        if let Some(mcp) = resolved.mcp_resource_uri {
            resource_uris.push(mcp);
        }
        return Ok(OAuthTokenResourceScope {
            resource_uris,
            allowed_service_ids: resolved.service_ids,
            allow_all_services: !service_restricting,
        });
    }

    // Once a grant has an explicit id boundary (including the empty set), slugs
    // cannot mint new authority. URIs are derived from those ids, never vice versa.
    let mut grant = Vec::new();
    for id in grant_allowed_service_ids {
        let Some(service) = db
            .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
            .find_one(mongodb::bson::doc! { "_id": id })
            .await?
        else {
            continue;
        };
        // Disable is reversible and Delete leaves a tombstone. Retain that ID;
        // execution denies inactive rows, even if another service reuses its slug.
        if service.is_active && !can_grant_user_service(db, actor_user_id, &service).await? {
            continue;
        }
        let resource = user_service_resource_uri(config, &service.slug);
        if service.is_active
            && (selected_service_is_shadowed(db, actor_user_id, &service).await?
                || resolve_single_resource(db, config, actor_user_id, &resource)
                    .await?
                    .id
                    != *id)
        {
            return Err(AppError::InvalidTarget(
                "A granted service name resolves to another connection".into(),
            ));
        }
        grant.push((id.clone(), resource));
    }
    let mut narrowed_ids = Vec::new();
    let mut selected_uris = Vec::new();
    let mut include_mcp = grant_resource_uris
        .iter()
        .any(|r| is_mcp_resource(config, r));
    if let Some(requested) = requested {
        include_mcp = false;
        let mut seen = std::collections::HashSet::new();
        for requested_uri in requested {
            validate_resource_uri(requested_uri)?;
            let uri = canonicalize_resource(config, requested_uri);
            if !seen.insert(uri.clone()) {
                return Err(AppError::InvalidTarget("Duplicate resource".into()));
            }
            if is_mcp_resource(config, &uri) {
                include_mcp = true;
                continue;
            }
            let matching = grant
                .iter()
                .filter(|(_, resource)| resource == &uri)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            if matching.is_empty() {
                return Err(AppError::InvalidTarget(
                    "resource cannot expand beyond the previously granted services".into(),
                ));
            }
            selected_uris.extend(std::iter::repeat_n(uri, matching.len()));
            narrowed_ids.extend(matching);
        }
    }
    // MCP-only requests remain narrowing-neutral; explicit empty grants stay empty.
    if requested.is_none() || (narrowed_ids.is_empty() && include_mcp) {
        (narrowed_ids, selected_uris) = grant.into_iter().unzip();
    }
    if include_mcp {
        selected_uris.push(mcp_resource_uri(config));
    }
    Ok(OAuthTokenResourceScope {
        resource_uris: selected_uris,
        allowed_service_ids: narrowed_ids,
        allow_all_services: false,
    })
}

/// Use the proxy's metadata-only precedence mirror, including personal legacy
/// connections and org priority. A slug is unique within each owner, so a
/// different effective owner means a different selected connection.
pub async fn selected_service_is_shadowed(
    db: &mongodb::Database,
    actor: &str,
    service: &UserService,
) -> AppResult<bool> {
    let owner = crate::services::proxy_service::find_effective_service_owner(
        db,
        actor,
        Some(&service.slug),
        None,
    )
    .await?;
    Ok(owner.is_some_and(|owner| owner != service.user_id))
}

async fn resolve_single_resource(
    db: &mongodb::Database,
    config: &AppConfig,
    actor_user_id: &str,
    resource: &str,
) -> AppResult<UserService> {
    let base = config.base_url.trim_end_matches('/');
    let prefix = format!("{base}/api/v1/proxy/s/");
    let Some(rest) = resource.strip_prefix(&prefix) else {
        return Err(AppError::InvalidTarget(
            "resource does not identify a NyxID user service".to_string(),
        ));
    };

    let slug = rest;
    if slug.is_empty() || slug.contains('/') || slug.contains('?') || slug != slug.trim_matches('/')
    {
        return Err(AppError::InvalidTarget(
            "resource does not identify a NyxID user service".to_string(),
        ));
    }

    if let Some(service) = user_service_service::find_by_slug(db, actor_user_id, slug).await? {
        return Ok(service);
    }

    match resolve_org_service_by_slug(db, actor_user_id, slug).await {
        Ok(service) => Ok(service),
        Err(AppError::InvalidTarget(_)) => {
            match catalog_service::get_downstream_service_by_slug(db, slug, actor_user_id).await {
                Ok(catalog_service) => Err(AppError::RequiredServiceNotConnected {
                    service_slug: catalog_service.slug,
                    service_name: catalog_service.name,
                }),
                Err(AppError::NotFound(_)) => Err(AppError::InvalidTarget(
                    "resource is unknown or not owned by the user".to_string(),
                )),
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

async fn resolve_org_service_by_slug(
    db: &mongodb::Database,
    actor_user_id: &str,
    slug: &str,
) -> AppResult<UserService> {
    let memberships =
        match org_service::find_active_memberships_with_timeout(db, actor_user_id).await {
            Ok(rows) => rows,
            Err(AppError::NotFound(_)) => Vec::new(),
            Err(err) => return Err(err),
        };

    for membership in &memberships {
        if !membership.role.can_proxy() {
            continue;
        }

        let Some(service) =
            user_service_service::find_by_slug(db, &membership.org_user_id, slug).await?
        else {
            continue;
        };

        if !membership_can_grant_user_service(db, membership, &service).await? {
            continue;
        }

        return Ok(service);
    }

    Err(AppError::InvalidTarget(
        "resource is unknown or not owned by the user".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn validates_absolute_resource_without_fragment() {
        assert!(validate_resource_uri("https://nyx.example/api/v1/proxy/s/openai").is_ok());
        assert!(validate_resource_uri("/api/v1/proxy/s/openai").is_err());
        assert!(validate_resource_uri("urn:example:service").is_ok());
        assert!(validate_resource_uri("https://nyx.example/api/v1/proxy/s/openai#part").is_err());
    }

    #[tokio::test]
    async fn mcp_resource_is_accepted_and_service_neutral() {
        // NyxID#1226 repro: RFC 9728 metadata advertises {BASE_URL}/mcp, so
        // spec-compliant MCP clients send it as the RFC 8707 resource. It must
        // resolve without invalid_target and without service narrowing.
        let Some(db) = crate::test_utils::connect_test_database("oauth_mcp_resource").await else {
            return;
        };
        let config = crate::test_utils::test_app_config();
        let mcp = mcp_resource_uri(&config);

        let resolved = resolve_requested_resources(
            &db,
            &config,
            &Uuid::new_v4().to_string(),
            Some(&[mcp.clone(), format!("{mcp}/")]),
        )
        .await
        .expect("mcp resource must resolve")
        .expect("resources were requested");
        assert_eq!(resolved.mcp_resource_uri, Some(mcp));
        assert!(resolved.service_ids.is_empty());
        assert!(resolved.resource_uris.is_empty());

        // Unknown URIs are still rejected — no regression on validation.
        let unknown = format!(
            "{}/api/v2/not-a-service",
            config.base_url.trim_end_matches('/')
        );
        assert!(matches!(
            resolve_requested_resources(
                &db,
                &config,
                &Uuid::new_v4().to_string(),
                Some(&[unknown]),
            )
            .await,
            Err(AppError::InvalidTarget(_))
        ));
    }

    #[tokio::test]
    async fn token_scope_mcp_only_preserves_allow_all_grant() {
        // NyxID#1226: refreshing with only resource={BASE_URL}/mcp must behave
        // like omitting `resource` — the allow-all grant survives and the MCP
        // audience stays in the token's resource list.
        let Some(db) = crate::test_utils::connect_test_database("oauth_mcp_token_scope").await
        else {
            return;
        };
        let config = crate::test_utils::test_app_config();
        let mcp = mcp_resource_uri(&config);

        let scope = resolve_token_resource_scope(
            &db,
            &config,
            &Uuid::new_v4().to_string(),
            Some(std::slice::from_ref(&mcp)),
            std::slice::from_ref(&mcp),
            &[],
            true,
        )
        .await
        .expect("mcp-only refresh must succeed");
        assert!(scope.allow_all_services);
        assert!(scope.allowed_service_ids.is_empty());
        assert_eq!(scope.resource_uris, vec![mcp]);
    }

    #[tokio::test]
    async fn known_catalog_resource_without_user_service_is_actionable() {
        let Some(db) = crate::test_utils::connect_test_database("oauth_missing_resource").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let suffix = Uuid::new_v4().to_string();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.slug = format!("required-{suffix}");
        catalog.name = "Required Test Service".to_string();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .expect("insert catalog service");

        let resource = user_service_resource_uri(&state.config, &catalog.slug);
        let error = resolve_requested_resources(
            &db,
            &state.config,
            &Uuid::new_v4().to_string(),
            Some(&[resource]),
        )
        .await
        .expect_err("missing user service must stop authorization");

        assert!(matches!(
            error,
            AppError::RequiredServiceNotConnected {
                service_slug,
                service_name,
            } if service_slug == catalog.slug && service_name == catalog.name
        ));
    }
}
