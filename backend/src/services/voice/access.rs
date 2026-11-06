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

pub(super) async fn personal_owner(db: &Database, owner: &str) -> AppResult<()> {
    let user = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":owner})
        .await?;
    if !user.is_some_and(|user| {
        user.is_active && user.user_type == crate::models::user::UserType::Person
    }) {
        return Err(AppError::ApiKeyScopeForbidden(
            "Device voice requires a personal owner".into(),
        ));
    }
    Ok(())
}

pub async fn services(db: &Database, auth: &AuthUser) -> AppResult<Vec<UserService>> {
    personal_owner(db, &auth.user_id.to_string()).await?;
    let denied = || AppError::ApiKeyScopeForbidden("This Key cannot use NyxBot Voice".into());
    let catalog = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"slug":SERVICE_SLUG,"is_active":true})
        .await?
        .ok_or_else(denied)?;
    let services: Vec<UserService> = db.collection::<UserService>(SERVICES).find(doc! {
        "user_id":auth.user_id.to_string(),"catalog_service_id":&catalog.id,
        "source":BOUND_SOURCE,"source_id":{"$type":"string"},
        "is_active":true,"deleted_at":mongodb::bson::Bson::Null,"node_id":mongodb::bson::Bson::Null,
    }).await?.try_collect().await?;
    let allowed = services
        .into_iter()
        .filter(|service| {
            key_service::ensure_api_key_service_scope(
                auth.api_key_service_scope(),
                std::slice::from_ref(&service.id),
            )
            .is_ok()
        })
        .collect::<Vec<_>>();
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
    if agent.user_id != auth.user_id.to_string() || agent.destroyed_at.is_some() {
        return Err(AppError::ApiKeyScopeForbidden(
            "This Key cannot invoke the selected Bot".into(),
        ));
    }
    personal_owner(db, &auth.user_id.to_string()).await?;
    let catalog = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"slug":SERVICE_SLUG,"is_active":true})
        .await?
        .ok_or_else(|| AppError::ApiKeyScopeForbidden("NyxBot Voice is unavailable".into()))?;
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {
            "user_id":auth.user_id.to_string(),"catalog_service_id":&catalog.id,
            "source":BOUND_SOURCE,"source_id":&agent.id,"is_active":true,
            "deleted_at":mongodb::bson::Bson::Null,"node_id":mongodb::bson::Bson::Null,
        })
        .await?
        .ok_or_else(|| {
            AppError::ApiKeyScopeForbidden(
                "This Key has no voice connection for the selected Bot".into(),
            )
        })?;
    key_service::ensure_api_key_service_scope(
        auth.api_key_service_scope(),
        std::slice::from_ref(&service.id),
    )?;
    Ok(service)
}

/// Resolve a selected connection directly; discovery alone scans the set.
pub(crate) async fn connection(db: &Database, auth: &AuthUser, id: &str) -> AppResult<UserService> {
    personal_owner(db, &auth.user_id.to_string()).await?;
    key_service::ensure_api_key_service_scope(auth.api_key_service_scope(), &[id.to_owned()])?;
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {
            "_id":id,"user_id":auth.user_id.to_string(),"source":BOUND_SOURCE,
            "source_id":{"$type":"string"},"is_active":true,
            "deleted_at":mongodb::bson::Bson::Null,"node_id":mongodb::bson::Bson::Null,
        })
        .await?
        .ok_or_else(|| AppError::ApiKeyScopeForbidden("Voice connection is unavailable".into()))?;
    if db.collection::<DownstreamService>(CATALOG).find_one(doc! {
        "_id":service.catalog_service_id.as_deref().unwrap_or_default(),"slug":SERVICE_SLUG,"is_active":true,
    }).await?.is_none() {
        return Err(AppError::ApiKeyScopeForbidden("NyxBot Voice is unavailable".into()));
    }
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
