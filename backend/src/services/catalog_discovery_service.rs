//! Read-only instance visibility shared by REST catalog and MCP discovery.

use crate::{
    errors::{AppError, AppResult},
    models::{downstream_service::DownstreamService, user_service::UserService},
};

/// Match API-key inventory reads: effective key scope, active services and
/// Member/Admin organization access. An org-owned key acts as that org.
pub async fn agent_services(
    db: &mongodb::Database,
    actor_id: &str,
    allowed_service_ids: Option<&[String]>,
) -> AppResult<Vec<UserService>> {
    let memberships = super::org_service::list_memberships_for_member(db, actor_id, false).await?;
    agent_services_with_memberships(db, actor_id, allowed_service_ids, &memberships).await
}

pub async fn agent_services_with_memberships(
    db: &mongodb::Database,
    actor_id: &str,
    allowed_service_ids: Option<&[String]>,
    memberships: &[crate::models::org_membership::OrgMembership],
) -> AppResult<Vec<UserService>> {
    Ok(
        super::user_service_service::list_user_services_with_sources_and_memberships(
            db,
            actor_id,
            /* include_scope_denied */ false,
            /* include_disabled */ false,
            memberships,
        )
        .await?
        .into_iter()
        .filter(|row| !row.source.is_viewer_org())
        .filter(|row| allowed_service_ids.is_none_or(|ids| ids.contains(&row.service.id)))
        .map(|row| row.service)
        .collect(),
    )
}

/// Public/creator/granted templates do not depend on an instance. Private
/// template access inherited through a UserService must use the key's visible
/// inventory, so guessing a slug cannot reveal an out-of-scope connection.
pub async fn get_catalog_service(
    db: &mongodb::Database,
    actor_id: &str,
    slug: &str,
    allowed_service_ids: Option<&[String]>,
) -> AppResult<DownstreamService> {
    let service =
        super::catalog_service::get_downstream_service_by_slug(db, slug, actor_id).await?;
    if service.visibility == "private"
        && service.created_by != actor_id
        && !super::platform_key_service::available(db, &service, actor_id).await?
        && !agent_services(db, actor_id, allowed_service_ids)
            .await?
            .iter()
            .any(|instance| instance.catalog_service_id.as_deref() == Some(&service.id))
    {
        return Err(AppError::NotFound("Catalog entry not found".to_string()));
    }
    Ok(service)
}
