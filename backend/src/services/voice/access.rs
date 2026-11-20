//! One catalog contract, with a separately authorized connection for each Bot.
//! llm-nyx remains a stateless caller-Key service with its original contract.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    mw::auth::AuthUser,
    services::key_service,
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

pub const SERVICE_SLUG: &str = "nyxbot-voice";
/// Server-owned source identity. A generic catalog connection grants no Bot access.
pub const BOUND_SOURCE: &str = "nyxbot_voice";

pub(super) async fn active_owner(db: &Database, owner: &str) -> AppResult<()> {
    let user = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":owner})
        .await?;
    if !user.is_some_and(|user| user.is_active) {
        return Err(AppError::ApiKeyScopeForbidden(
            "Voice owner is unavailable".into(),
        ));
    }
    Ok(())
}

/// A device acts as its Key owner, including an organization principal.
/// Cross-owner Bots retain their existing live organization membership ACL.
pub(super) async fn agent_access(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<()> {
    if agent.destroyed_at.is_some()
        || (agent.user_id != actor && agent.is_nyxbot())
        || !crate::services::org_agent_service::can_use(
            &crate::services::org_agent_service::access(db, actor, &agent.user_id).await?,
        )
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "Voice Bot is unavailable".into(),
        ));
    }
    Ok(())
}

async fn service_access(db: &Database, auth: &AuthUser, service: &UserService) -> AppResult<()> {
    let access =
        crate::services::org_agent_service::access(db, &auth.user_id.to_string(), &service.user_id)
            .await?;
    if !crate::services::org_agent_service::can_use(&access)
        || !access.allows_resource(&service.id)
        || (service.admin_only && !access.can_write())
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "Voice connection is unavailable".into(),
        ));
    }
    key_service::ensure_api_key_service_scope(
        auth.api_key_service_scope(),
        std::slice::from_ref(&service.id),
    )
}

pub async fn services(db: &Database, auth: &AuthUser) -> AppResult<Vec<UserService>> {
    active_owner(db, &auth.user_id.to_string()).await?;
    let denied = || AppError::ApiKeyScopeForbidden("This Key cannot use NyxBot Voice".into());
    let catalog = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"slug":SERVICE_SLUG,"is_active":true})
        .await?
        .ok_or_else(denied)?;
    let owners =
        crate::services::org_agent_service::visible_owners(db, &auth.user_id.to_string()).await?;
    let services: Vec<UserService> = db.collection::<UserService>(SERVICES).find(doc! {
        "user_id":{"$in":owners},"catalog_service_id":&catalog.id,
        "source":BOUND_SOURCE,"source_id":{"$type":"string"},
        "is_active":true,"deleted_at":mongodb::bson::Bson::Null,"node_id":mongodb::bson::Bson::Null,
    }).await?.try_collect().await?;
    let mut allowed = Vec::new();
    for service in services {
        match service_access(db, auth, &service).await {
            Ok(()) => allowed.push(service),
            Err(error) if super::client_key::permanent_authority_failure(&error) => {}
            Err(error) => return Err(error),
        }
    }
    if allowed.is_empty() {
        return Err(denied());
    }
    Ok(allowed)
}
pub async fn authorize(
    db: &Database,
    auth: &AuthUser,
    agent: &AssistantAgent,
) -> AppResult<UserService> {
    active_owner(db, &auth.user_id.to_string()).await?;
    agent_access(db, &auth.user_id.to_string(), agent).await?;
    let catalog = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"slug":SERVICE_SLUG,"is_active":true})
        .await?
        .ok_or_else(|| AppError::ApiKeyScopeForbidden("NyxBot Voice is unavailable".into()))?;
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {
            "user_id":&agent.user_id,"catalog_service_id":&catalog.id,
            "source":BOUND_SOURCE,"source_id":&agent.id,"is_active":true,
            "deleted_at":mongodb::bson::Bson::Null,"node_id":mongodb::bson::Bson::Null,
        })
        .await?
        .ok_or_else(|| {
            AppError::ApiKeyScopeForbidden(
                "This Key has no voice connection for the selected Bot".into(),
            )
        })?;
    service_access(db, auth, &service).await?;
    Ok(service)
}

/// Resolve a selected connection directly; discovery alone scans the set.
pub(crate) async fn connection(db: &Database, auth: &AuthUser, id: &str) -> AppResult<UserService> {
    active_owner(db, &auth.user_id.to_string()).await?;
    key_service::ensure_api_key_service_scope(auth.api_key_service_scope(), &[id.to_owned()])?;
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {
            "_id":id,"source":BOUND_SOURCE,
            "source_id":{"$type":"string"},"is_active":true,
            "deleted_at":mongodb::bson::Bson::Null,"node_id":mongodb::bson::Bson::Null,
        })
        .await?
        .ok_or_else(|| AppError::ApiKeyScopeForbidden("Voice connection is unavailable".into()))?;
    service_access(db, auth, &service).await?;
    let catalog = db.collection::<DownstreamService>(CATALOG).find_one(doc! {
        "_id":service.catalog_service_id.as_deref().unwrap_or_default(),"slug":SERVICE_SLUG,"is_active":true,
    }).await?.ok_or_else(|| AppError::ApiKeyScopeForbidden("NyxBot Voice is unavailable".into()))?;
    crate::services::shutdown::diagnostics::resolved_catalog(&catalog);
    Ok(service)
}
/// Route and authority checks shared by ingress and ongoing call revalidation.
/// The HTTP ingress checks value limits against the actual request separately.
pub async fn policy(
    db: &Database,
    auth: &AuthUser,
    service: &UserService,
    method: &str,
    path: &str,
    websocket: bool,
) -> AppResult<()> {
    let catalog = db.collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"_id":service.catalog_service_id.as_deref().unwrap_or_default(),"slug":SERVICE_SLUG,"is_active":true})
        .await?.ok_or_else(|| AppError::ApiKeyScopeForbidden("NyxBot Voice is unavailable".into()))?;
    let operation =
        crate::services::operation_descriptor::build_http_descriptor(method, path, None);
    if crate::services::approval_service::evaluate_deny_only(
        db,
        &auth.user_id.to_string(),
        &service.user_id,
        &catalog.id,
        &operation,
        false,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            "Operation denied by approval policy".into(),
        ));
    }
    let canonical = crate::services::proxy_authorization::CanonicalPath::from_mcp_literal(path)?;
    crate::services::proxy_authorization::authorize_proxy_operation(&catalog, method, &canonical)?;
    crate::services::agent_operation_scope_service::authorize(
        &auth.assistant_operation_scopes,
        &service.id,
        Some(&catalog.id),
        None,
        method,
        &canonical,
        false,
        websocket,
    )?;
    Ok(())
}
