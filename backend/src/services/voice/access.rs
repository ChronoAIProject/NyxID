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
    let service = services(db, auth)
        .await?
        .into_iter()
        .find(|service| service.source_id.as_deref() == Some(agent.id.as_str()))
        .ok_or_else(|| {
            AppError::ApiKeyScopeForbidden(
                "This Key has no voice connection for the selected Bot".into(),
            )
        })?;
    // Opening the call includes a WebSocket. Preserve B1's existing rule that
    // operation-scoped services cannot open one; no partial-scope bypass.
    operation(db, auth, &service, "POST", "/voice/sessions", true).await?;
    Ok(service)
}
pub async fn operation(
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
    crate::services::agent_operation_scope_service::check_inputs(
        &auth.assistant_operation_scopes,
        &service.id,
        Some(&catalog.id),
        None,
        method,
        &canonical,
        None,
        &[],
        None,
    )?;
    approval(db, auth, service, method, path).await
}
pub async fn approval(
    db: &Database,
    auth: &AuthUser,
    service: &UserService,
    method: &str,
    path: &str,
) -> AppResult<()> {
    let operation =
        crate::services::operation_descriptor::build_http_descriptor(method, path, None);
    // Approval settings for catalog-backed connections use the catalog ID,
    // matching /approvals/service-configs and the existing proxy evaluator.
    // The exact Bot's connection grant has already been checked separately.
    let approval_service_id = service
        .catalog_service_id
        .as_deref()
        .ok_or_else(|| AppError::ApiKeyScopeForbidden("NyxBot Voice is unavailable".into()))?;
    let (outcome, _) = crate::services::approval_service::evaluate_for_chat(
        db,
        &auth.user_id.to_string(),
        &service.user_id,
        approval_service_id,
        &operation,
        Some("api_key"),
        auth.api_key_id.as_deref().unwrap_or_default(),
        false,
        service.source.as_deref() == Some(crate::models::user_service::AUTO_PROVISION_SOURCE),
        None,
    )
    .await?;
    if !matches!(
        outcome,
        crate::services::approval_service::ApprovalOutcome::Allowed { .. }
    ) {
        return Err(AppError::Forbidden(
            "NyxBot Voice requires approval under your existing service policy".into(),
        ));
    }
    Ok(())
}
