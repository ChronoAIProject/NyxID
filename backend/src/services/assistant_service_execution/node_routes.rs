use super::*;

/// Validate held primary/fallback identities against current configuration.
/// Online state can change without changing authority; removed routes cannot run.
pub fn verify_node_route<'a>(
    db: &'a Database,
    chat: &'a acks::ChatAuthority,
    id: &'a str,
    route: Option<&'a crate::services::node_routing_service::NodeRoute>,
    target: &'a crate::services::proxy_service::ProxyTarget,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let service = crate::services::user_service_service::find_user_service_by_id(db, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Service unavailable".into()))?;
        let mut allowed = crate::services::node_routing_service::list_configured_binding_node_ids(
            db,
            &service.user_id,
            service.catalog_service_id.as_deref().unwrap_or(id),
        )
        .await?;
        if let Some(primary) = service.node_id.as_ref().filter(|id| !id.is_empty()) {
            allowed.push(primary.clone());
        }
        let key = db
            .collection::<ApiKey>("api_keys")
            .find_one(doc! {"_id":&chat.api_key_id,"is_active":true})
            .await?
            .ok_or_else(|| AppError::Forbidden("Assistant credential revoked".into()))?;
        let access =
            crate::services::org_agent_service::resolve_credential_access(db, &key).await?;
        let (all, nodes) =
            crate::services::org_agent_service::key_nodes(db, &key, access.as_deref()).await?;
        if let Some(route) = route {
            if crate::services::proxy_service::uses_server_held_master(target)
                || std::iter::once(&route.node_id)
                    .chain(route.fallback_node_ids.iter())
                    .any(|node| !allowed.contains(node) || (!all && !nodes.contains(node)))
            {
                return Err(AppError::Conflict(
                    "Resolved credential node authority changed".into(),
                ));
            }
        } else if !crate::services::proxy_service::uses_server_held_master(target)
            && service.node_id.as_ref().is_some_and(|id| !id.is_empty())
        {
            return Err(AppError::Conflict(
                "Resolved credential node route changed".into(),
            ));
        }
        Ok(())
    })
}
