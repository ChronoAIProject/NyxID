//! Settings-only, read-only readiness. Never a substitute for execution admission.
use super::model_diagnostic::Reason;
use super::*;
use crate::models::{
    assistant_agent::AssistantAgent, assistant_conversation::AssistantConversation,
    downstream_service::DownstreamService,
};

#[derive(Clone, serde::Serialize)]
pub struct GuestReadiness {
    pub status: &'static str,
    pub reason: Option<String>,
    pub fix: Option<String>,
}
impl GuestReadiness {
    fn ready() -> Self {
        Self {
            status: "ready",
            reason: None,
            fix: None,
        }
    }
    pub(crate) fn refused(reason: Reason) -> Self {
        Self {
            status: "not_ready",
            reason: Some(reason.code()),
            fix: Some(reason.fix().into()),
        }
    }
}

#[derive(serde::Serialize)]
pub struct GuestReadinessSettings {
    #[serde(flatten)]
    readiness: GuestReadiness,
    /// Only these chats need a separate settings read; ordinary customer chats
    /// reuse the agent result. Bound the settings payload, with safe fallback.
    conversation_overrides: Vec<String>,
    conversation_overrides_truncated: bool,
}

pub async fn guest_readiness_settings(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    owner: &str,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> AppResult<GuestReadinessSettings> {
    let not_found = || AppError::NotFound("Agent or conversation not found".into());
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":agent_id,"user_id":owner})
        .await?
        .ok_or_else(not_found)?;
    if let Some(id) = conversation_id
        && db
            .collection::<Document>("assistant_conversations")
            .find_one(doc! {"_id":id,"user_id":owner,"agent_id":agent_id})
            .projection(doc! {"_id":1})
            .await?
            .is_none()
    {
        return Err(not_found());
    }
    let readiness = guest_readiness(db, keys, owner, Some(agent_id), conversation_id).await?;
    let mut conversation_overrides = Vec::new();
    if conversation_id.is_none() {
        let bound = bson::to_bson(
            &agent
                .owner_model_route
                .as_ref()
                .filter(|route| route.profile == agent.model),
        )
        .map_err(|_| AppError::Internal("Invalid model route".into()))?;
        let rows: Vec<Document> = db.collection::<Document>("assistant_conversations")
            .find(doc! {"user_id":owner,"agent_id":agent_id,"$or":[
                {"model":{"$ne":&agent.model}},
                {"owner_model_route":{"$ne":bson::Bson::Null},"$expr":{"$ne":["$owner_model_route",{"$literal":bound}]}}
            ]})
            .projection(doc! {"_id":1}).limit(513).await?.try_collect().await?;
        conversation_overrides = rows
            .iter()
            .filter_map(|r| r.get_str("_id").ok().map(str::to_owned))
            .collect();
    }
    let conversation_overrides_truncated = conversation_overrides.len() > 512;
    conversation_overrides.truncate(512);
    Ok(GuestReadinessSettings {
        readiness,
        conversation_overrides,
        conversation_overrides_truncated,
    })
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static READS: std::sync::Arc<std::sync::atomic::AtomicUsize>;
}

pub async fn guest_readiness(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    owner: &str,
    agent_id: Option<&str>,
    conversation_id: Option<&str>,
) -> AppResult<GuestReadiness> {
    #[cfg(test)]
    let _ = READS.try_with(|reads| reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let mut filter = doc! {"user_id":owner,"destroyed_at":null};
    if let Some(id) = agent_id {
        filter.insert("_id", id);
    } else {
        filter.insert("kind", "nyxbot");
    }
    let Some(agent) = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(filter)
        .await?
    else {
        return Ok(GuestReadiness::refused(Reason::AgentUnavailable));
    };
    let row = if let Some(id) = conversation_id.or(agent.home_conversation_id.as_deref()) {
        db.collection::<AssistantConversation>("assistant_conversations")
            .find_one(doc! {"_id":id,"user_id":owner,"agent_id":&agent.id})
            .await?
    } else {
        None
    };
    let profile = row
        .as_ref()
        .filter(|_| conversation_id.is_some())
        .map_or(agent.model.as_str(), |r| &r.model);
    let bound = row
        .as_ref()
        .filter(|_| conversation_id.is_some())
        .and_then(|r| r.owner_model_route.as_deref())
        .or_else(|| {
            agent
                .owner_model_route
                .as_deref()
                .filter(|r| r.profile == profile)
        });
    // A recorded BYOK route remains eligible only while its exact live identity
    // is unchanged. Read-only resolution never refreshes/decrypts credentials.
    if let (Some(bound), Some(row)) = (bound, row.as_ref()) {
        let chat = acks::for_key(db, owner, Some(&row.credential_api_key_id)).await?;
        if let Some(chat) = chat {
            let target = match bound_target(db, keys, owner, bound).await {
                Err(error @ AppError::DatabaseError(_)) => return Err(error),
                Ok(target) => target,
                Err(_) => None,
            };
            if let Some(target) = target {
                let current = super::guest_model::route(
                    db,
                    &chat,
                    profile,
                    bound.connection_id.as_deref(),
                    &target,
                    &json!({}),
                )
                .await;
                if current.is_ok_and(|r| {
                    bound.profile == profile
                        && r.execution == bound.execution
                        && r.catalog_service_id == bound.catalog_service_id
                }) {
                    return Ok(GuestReadiness::ready());
                }
            }
        }
    }
    let Some(config) = super::guest_model_defaults::ROUTES.get(profile) else {
        return Ok(GuestReadiness::refused(Reason::ProfileUnknown));
    };
    let Some(catalog) = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! {"slug":&config.service_slug,"is_active":true})
        .await?
    else {
        return Ok(GuestReadiness::refused(Reason::CatalogInactiveOrNotMaster));
    };
    let authority = match crate::services::proxy_service::read_default_model_authority(
        db,
        owner,
        &config.service_slug,
    )
    .await
    {
        Ok(value) => value,
        Err(error @ AppError::DatabaseError(_)) => return Err(error),
        Err(_) => return Ok(GuestReadiness::refused(Reason::ConnectionNotPlatform)),
    };
    if authority.as_ref().is_some_and(|a| !a.master_credential) {
        return Ok(GuestReadiness::refused(Reason::ConnectionNotPlatform));
    }
    if catalog.inference.is_none()
        || catalog.credential_encrypted.is_empty()
        || !(crate::services::platform_key_service::has_platform_key(&catalog)
            || catalog.service_category == "internal" && !catalog.requires_user_credential)
    {
        return Ok(GuestReadiness::refused(Reason::CatalogInactiveOrNotMaster));
    }
    if let Some(authority) = authority {
        if !authority.master_credential
            || authority.node_id.is_some()
            || authority.pool_selection.is_some()
        {
            return Ok(GuestReadiness::refused(Reason::ConnectionNotPlatform));
        }
        if authority.target.service.id != catalog.id
            || authority.target.base_url != catalog.base_url
            || authority.target.target_id.is_some()
        {
            return Ok(GuestReadiness::refused(Reason::TargetMismatch));
        }
        if let Some(row) = row.as_ref() {
            let override_key =
                crate::services::proxy_service::read_agent_credential_override_identity(
                    db,
                    owner,
                    &row.credential_api_key_id,
                    &authority.user_service_id,
                    &authority.target,
                )
                .await?;
            if override_key.is_some() {
                return Ok(GuestReadiness::refused(Reason::ConnectionOverride));
            }
        }
        let connection = db
            .collection::<UserService>("user_services")
            .find_one(doc! {"_id":&authority.user_service_id})
            .await?;
        if connection.is_some_and(|c| {
            c.default_request_headers
                .as_ref()
                .is_some_and(|h| !h.is_empty())
        }) {
            return Ok(GuestReadiness::refused(Reason::HeaderMismatch));
        }
    } else {
        // Same ACL as catalog execution, without materializing credentials.
        if crate::services::proxy_service::authorize_master_credential(
            db,
            &catalog,
            &crate::services::proxy_service::EffectiveActor::from_user_id(owner),
        )
        .await
        .is_err()
        {
            return Ok(GuestReadiness::refused(Reason::ConnectionNotPlatform));
        }
    }
    // A stale recorded route takes precedence over bootstrap at execution too.
    if bound.is_some() {
        return Ok(GuestReadiness::refused(Reason::RouteMismatch));
    }
    Ok(GuestReadiness::ready())
}

async fn bound_target(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    owner: &str,
    bound: &crate::models::assistant_agent::OwnerModelRoute,
) -> AppResult<Option<crate::services::proxy_service::ProxyTarget>> {
    use crate::services::{default_header_service, platform_key_service, proxy_service};
    if let Some(id) = &bound.connection_id {
        return Ok(
            proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                db, keys, owner, id, None,
            )
            .await?
            .map(|a| a.target),
        );
    }
    let Some(mut service) = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! {"_id":&bound.catalog_service_id,"is_active":true})
        .await?
    else {
        return Ok(None);
    };
    if proxy_service::read_default_model_authority(db, owner, &service.slug)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    proxy_service::authorize_master_credential(
        db,
        &service,
        &proxy_service::EffectiveActor::from_user_id(owner),
    )
    .await?;
    if platform_key_service::has_platform_key(&service) {
        let (method, name) = platform_key_service::effective_auth(db, &service).await?;
        service.auth_method = method;
        service.auth_key_name = name;
        service.requires_user_credential = false;
    }
    Ok(Some(proxy_service::ProxyTarget {
        workspace_destinations_pending:
            crate::services::destination_routing::workspace_destinations_pending(&service),
        target_id: None,
        base_url: service.base_url.clone(),
        auth_method: service.auth_method.clone(),
        auth_key_name: service.auth_key_name.clone(),
        credential: Default::default(),
        catalog_default_headers: default_header_service::catalog(Some(keys), &service).await?,
        user_service_default_headers: Vec::new(),
        ws_frame_injections: service.ws_frame_injections.clone(),
        connection_id: None,
        service,
    }))
}
