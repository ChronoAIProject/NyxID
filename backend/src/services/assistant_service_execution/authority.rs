use super::*;

/// A digest only: no key material, downstream arguments or internal authority
/// metadata is exposed to the model or forwarded to a provider.
pub fn connection_binding<'a>(
    db: &'a Database,
    id: &'a str,
    api_key_id: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<String>> {
    Box::pin(async move {
        let service =
            crate::services::service_history::collection::<UserService>(db, "user_services")
                .find_one(doc! {"_id": id, "is_active": true})
                .await?
                .ok_or_else(|| AppError::NotFound("Service connection unavailable".into()))?;
        let endpoint =
            crate::services::service_history::collection::<UserEndpoint>(db, "user_endpoints")
                .find_one(doc! {"_id": &service.endpoint_id, "user_id": &service.user_id})
                .await?
                .ok_or_else(|| AppError::NotFound("Service destination unavailable".into()))?;
        let credential = if let Some(key) = &service.api_key_id {
            let key = db
                .collection::<UserApiKey>("user_api_keys")
                .find_one(doc! {"_id": key, "user_id": &service.user_id})
                .await?
                .ok_or_else(|| AppError::NotFound("Service credential unavailable".into()))?;
            json!({"id": key.id, "owner": key.user_id, "epoch": key.credential_epoch,
            "provider": key.provider_config_id, "connection": key.connection_id, "source": key.credential_source})
        } else {
            Value::Null
        };
        // Catalog updates, including destination/policy/publication/credential changes,
        // invalidate pending consent. Lazy OAuth refresh is intentionally absent.
        let catalog = if let Some(id) = &service.catalog_service_id {
            db.collection::<Document>("downstream_services")
                .find_one(doc! {"_id": id, "is_active": true})
                .await?
                .ok_or_else(|| AppError::NotFound("Catalog service unavailable".into()))?
        } else {
            Document::new()
        };
        let agent_binding = db
            .collection::<Document>("agent_service_bindings")
            .find_one(doc! {
                "api_key_id": api_key_id, "user_service_id": id,
            })
            .await?;
        let override_key = if let Some(key) = agent_binding
            .as_ref()
            .and_then(|row| row.get_str("user_api_key_id").ok())
        {
            db.collection::<Document>("user_api_keys").find_one(doc! {"_id": key}).projection(doc! {
            "_id": 1, "user_id": 1, "credential_epoch": 1, "provider_config_id": 1, "connection_id": 1,
        }).await?
        } else {
            None
        };
        let configured_nodes =
            crate::services::node_routing_service::list_configured_binding_node_ids(
                db,
                &service.user_id,
                service.catalog_service_id.as_deref().unwrap_or(id),
            )
            .await?;
        let key = db
            .collection::<ApiKey>("api_keys")
            .find_one(doc! {"_id":api_key_id,"is_active":true})
            .await?
            .ok_or_else(|| AppError::Forbidden("Assistant credential revoked".into()))?;
        if key.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(AppError::Forbidden("Assistant credential expired".into()));
        }
        let access =
            crate::services::org_agent_service::resolve_credential_access(db, &key).await?;
        let allowed = crate::services::key_service::effective_allowed_service_ids_with_access(
            db,
            &key,
            access.as_deref(),
        )
        .await?;
        let nodes =
            crate::services::org_agent_service::key_nodes(db, &key, access.as_deref()).await?;
        let mode = if let Some(chat) = acks::for_key(db, &key.user_id, Some(api_key_id)).await? {
            acks::effective_access_mode(db, &chat).await?
        } else {
            crate::services::assistant_settings_service::get(db, &key.user_id)
                .await?
                .access_mode
        };
        let shared_policy = if access.as_ref().is_some_and(|a| a.shared().is_some()) {
            let approval: Vec<Document> = db.collection("service_approval_configs")
                .find(doc! {"user_id": {"$in": [&service.user_id, &key.user_id]}, "service_id": {"$in": [Some(service.id.as_str()), service.catalog_service_id.as_deref()]}})
                .sort(doc! {"_id": 1}).await?.try_collect().await?;
            let notifications: Vec<Document> = db
                .collection("notification_channels")
                .find(doc! {"user_id": {"$in": [&service.user_id, &key.user_id]}})
                .projection(doc! {"_id": 1, "approval_required": 1})
                .sort(doc! {"_id": 1})
                .await?
                .try_collect()
                .await?;
            let endpoints: Vec<Document> = db.collection("service_endpoints")
                .find(doc! {"service_id": service.catalog_service_id.as_deref().unwrap_or(id)})
                .projection(doc! {"_id": 1, "method": 1, "path": 1, "risk": 1, "destructive": 1, "changes_existing": 1, "operation_generation": 1, "is_active": 1, "publication": 1})
                .sort(doc! {"_id": 1}).await?.try_collect().await?;
            json!({"approval": approval, "notifications": notifications, "operations": endpoints})
        } else {
            Value::Null
        };
        let authority = json!({"owner":key.user_id,"expires":key.expires_at,"scopes":key.assistant_operation_scopes,
        "allowed":allowed,"all_services":key.allow_all_services,"nodes":nodes,"mode":mode});
        let mut service = serde_json::to_value(service)
            .map_err(|_| AppError::Internal("Connection projection failed".into()))?;
        let mut endpoint = serde_json::to_value(endpoint)
            .map_err(|_| AppError::Internal("Connection projection failed".into()))?;
        let mut catalog = catalog;
        for field in [
            "created_at",
            "updated_at",
            "name",
            "description",
            "icon_url",
            "logo_url",
        ] {
            service.as_object_mut().expect("service").remove(field);
            endpoint.as_object_mut().expect("endpoint").remove(field);
            catalog.remove(field);
        }
        let mut projection = json!({"service": service, "endpoint": endpoint, "credential": credential,
        "catalog": catalog, "authority":authority, "agent_binding": agent_binding, "override_key": override_key,
        "configured_nodes": configured_nodes});
        // Preserve the existing binding byte-for-byte for unrelated lanes.
        if !shared_policy.is_null() {
            projection["shared_policy"] = shared_policy;
        }
        Ok(digest(projection))
    })
}
