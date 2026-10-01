//! Read-only slug metadata selection shared by proxy ingress and alias discovery.
//! Concrete credential resolution always follows this selection and rechecks ACLs.
use super::{proxy_service, service_pool_service};
use crate::errors::{AppError, AppResult};

pub enum SlugMetadataRoute {
    Service(String),
    Pool(Box<crate::models::service_pool::ServicePool>),
    Legacy,
}

/// Batch pool discovery from the catalog resolver's authorized instance snapshot.
/// A pool grants no member authority: at least one enabled, same-owner member
/// must already be visible under the caller's role/service/node scopes. Proxy
/// execution revalidates each member through its ordinary live resolver.
pub async fn agent_pools_with_services(
    db: &mongodb::Database,
    actor: &str,
    services: &[crate::models::user_service::UserService],
    allowed_nodes: Option<&[String]>,
) -> AppResult<Vec<crate::models::service_pool::ServicePool>> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    let eligible: std::collections::HashMap<_, _> = services
        .iter()
        .filter(|row| {
            row.node_id
                .as_ref()
                .is_none_or(|node| allowed_nodes.is_none_or(|ids| ids.contains(node)))
        })
        .map(|row| (row.id.as_str(), row.user_id.as_str()))
        .collect();
    let owners: std::collections::HashSet<_> = eligible.values().copied().collect();
    let mut pools: Vec<crate::models::service_pool::ServicePool> = db
        .collection("service_pools")
        .find(doc! {
            "user_id": { "$in": owners.into_iter().collect::<Vec<_>>() },
            "is_active": true,
            "members": { "$elemMatch": {
                "user_service_id": { "$in": eligible.keys().copied().collect::<Vec<_>>() },
                "enabled": { "$ne": false },
            } },
        })
        .await?
        .try_collect()
        .await?;
    pools.retain(|pool| {
        pool.members.iter().any(|member| {
            member.enabled
                && eligible
                    .get(member.user_service_id.as_str())
                    .is_some_and(|owner| *owner == pool.user_id)
        })
    });
    pools.sort_by_key(|pool| (pool.user_id != actor, pool.slug.clone(), pool.id.clone()));
    Ok(pools)
}

pub async fn select_slug_metadata(
    db: &mongodb::Database,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    allowed_service_ids: Option<&[String]>,
    allowed_node_ids: Option<&[String]>,
    actor_user_id: &str,
    slug: &str,
) -> AppResult<SlugMetadataRoute> {
    // Preserve the established proxy cascade: an active personal UserService
    // with this slug owns the route before any pool can intercept it.
    if let Some(service) =
        crate::services::user_service_service::find_by_slug(db, actor_user_id, slug).await?
    {
        return Ok(SlugMetadataRoute::Service(service.id));
    }
    if let Some(pool) = service_pool_service::find_pool_by_slug(db, actor_user_id, slug).await? {
        return Ok(SlugMetadataRoute::Pool(Box::new(pool)));
    }
    if proxy_service::user_has_legacy_personal_connection(db, actor_user_id, Some(slug), None)
        .await?
    {
        return Ok(SlugMetadataRoute::Legacy);
    }
    let memberships =
        match crate::services::org_service::find_active_memberships_with_timeout(db, actor_user_id)
            .await
        {
            Ok(rows) => rows,
            Err(AppError::NotFound(_)) => Vec::new(),
            Err(error) => return Err(error),
        };
    let mut role_denied = false;
    for membership in memberships {
        if let Some(service) =
            crate::services::user_service_service::find_by_slug(db, &membership.org_user_id, slug)
                .await?
        {
            if !crate::services::user_service_service::role_can_proxy_service(
                membership.role,
                &service,
            ) {
                role_denied = true;
                continue;
            }
            let scope = crate::services::org_role_scope_service::effective_scope_for_membership(
                db,
                &membership,
            )
            .await?;
            if !crate::services::org_role_scope_service::scope_allows(&scope, &service.id) {
                role_denied = true;
                continue;
            }
            return Ok(SlugMetadataRoute::Service(service.id));
        }
        let Some(pool) =
            service_pool_service::find_pool_by_slug(db, &membership.org_user_id, slug).await?
        else {
            continue;
        };
        if !membership.role.can_proxy() {
            role_denied = true;
            continue;
        }
        let mut eligible = false;
        for member in pool.members.iter().filter(|member| member.enabled) {
            if allowed_service_ids.is_some_and(|ids| !ids.contains(&member.user_service_id)) {
                continue;
            }
            match proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                db,
                encryption_keys,
                actor_user_id,
                &member.user_service_id,
                None,
            )
            .await
            {
                Ok(Some(resolution)) => {
                    if resolution
                        .node_id
                        .as_ref()
                        .is_some_and(|id| allowed_node_ids.is_some_and(|ids| !ids.contains(id)))
                    {
                        continue;
                    }
                    eligible = true;
                    break;
                }
                Ok(None) => {}
                Err(error) if service_pool_service::member_unavailable(&error) => {}
                Err(error) => return Err(error),
            }
        }
        if eligible {
            return Ok(SlugMetadataRoute::Pool(Box::new(pool)));
        }
        role_denied = true;
    }
    if role_denied {
        return Err(AppError::OrgRoleInsufficient(
            "your role in the owning org does not permit using this service pool".to_string(),
        ));
    }
    Ok(SlugMetadataRoute::Legacy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn non_user_resolution_id_retains_ordinary_legacy_fallback() {
        let db = crate::test_utils::connect_transaction_test_database("pool_nonuser_route").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let actor = uuid::Uuid::new_v4().to_string();
        assert!(matches!(
            select_slug_metadata(
                &db,
                &state.encryption_keys,
                None,
                None,
                &actor,
                "ordinary-legacy"
            )
            .await
            .unwrap(),
            SlugMetadataRoute::Legacy
        ));
        assert!(
            crate::services::proxy_service::resolve_proxy_target_from_user_service(
                &db,
                &state.encryption_keys,
                &state.node_ws_manager,
                &actor,
                Some("ordinary-legacy"),
                None,
                crate::services::proxy_service::ProxyExecutionContext::new(
                    None,
                    state.platform_user_rate_limit
                )
            )
            .await
            .unwrap()
            .is_none()
        );
        db.drop().await.unwrap();
    }
}
