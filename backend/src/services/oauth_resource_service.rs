use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::user_service::UserService;
use crate::services::{org_role_scope_service, org_service, user_service_service};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOAuthResources {
    pub resource_uris: Vec<String>,
    pub service_ids: Vec<String>,
}

pub fn user_service_resource_uri(config: &AppConfig, slug: &str) -> String {
    format!(
        "{}/api/v1/proxy/s/{}",
        config.base_url.trim_end_matches('/'),
        slug
    )
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

pub fn filter_resource_narrowing(
    requested: &[String],
    granted_resources: &[String],
) -> AppResult<Vec<String>> {
    let mut narrowed = Vec::new();
    for resource in requested {
        validate_resource_uri(resource)?;
        if granted_resources.iter().any(|granted| granted == resource)
            && !narrowed.iter().any(|existing| existing == resource)
        {
            narrowed.push(resource.clone());
        }
    }

    if narrowed.len() != requested.len() {
        return Err(AppError::InvalidTarget(
            "resource cannot expand beyond the previously granted resources".to_string(),
        ));
    }

    Ok(narrowed)
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

    for resource in resources {
        validate_resource_uri(resource)?;
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

    resolve_org_service_by_slug(db, actor_user_id, slug).await
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

        if service.admin_only && !membership.role.can_admin() {
            continue;
        }

        let effective_scope =
            org_role_scope_service::effective_scope_for_membership(db, membership).await?;
        if !org_role_scope_service::scope_allows(&effective_scope, &service.id) {
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

    #[test]
    fn validates_absolute_resource_without_fragment() {
        assert!(validate_resource_uri("https://nyx.example/api/v1/proxy/s/openai").is_ok());
        assert!(validate_resource_uri("/api/v1/proxy/s/openai").is_err());
        assert!(validate_resource_uri("urn:example:service").is_ok());
        assert!(validate_resource_uri("https://nyx.example/api/v1/proxy/s/openai#part").is_err());
    }

    #[test]
    fn narrowing_rejects_expansion() {
        let granted = vec![
            "https://nyx.example/api/v1/proxy/s/openai".to_string(),
            "https://nyx.example/api/v1/proxy/s/anthropic".to_string(),
        ];

        let requested = vec!["https://nyx.example/api/v1/proxy/s/openai".to_string()];
        assert_eq!(
            filter_resource_narrowing(&requested, &granted).unwrap(),
            requested
        );

        let expanded = vec!["https://nyx.example/api/v1/proxy/s/cohere".to_string()];
        assert!(matches!(
            filter_resource_narrowing(&expanded, &granted),
            Err(AppError::InvalidTarget(_))
        ));
    }
}
