use std::collections::HashSet;

use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        service_account::{
            COLLECTION_NAME as ACCOUNTS, CurationGrant, ServiceAccount, ServiceAccountPurpose,
        },
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub const READ_SCOPE: &str = "catalog:skills:read";
pub const WRITE_SCOPE: &str = "catalog:skills:write";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueCurationGrant {
    pub service_ids: Vec<String>,
    pub ornn_proxy_service_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_writes: i64,
    pub window_seconds: i64,
}

pub fn validate_scopes(scopes: &str, ornn_target: Option<&str>) -> AppResult<()> {
    if scopes.split_whitespace().next().is_none()
        || scopes.split_whitespace().any(|s| {
            s != READ_SCOPE
                && s != WRITE_SCOPE
                && s != super::service_account_key_read_service::READ_SCOPE
                && !(s == "proxy" && ornn_target.is_some())
        })
    {
        return Err(AppError::ValidationError("Curation scopes must be catalog:skills:read, catalog:skills:write, user-services:read, or proxy with an Ornn target".into()));
    }
    Ok(())
}

pub fn live_grant(sa: &ServiceAccount) -> AppResult<&CurationGrant> {
    if !sa.is_active || sa.purpose != ServiceAccountPurpose::Curation || !sa.platform_protected {
        return Err(AppError::Forbidden("Curation authority required".into()));
    }
    sa.curation_grant
        .as_ref()
        .filter(|g| g.expires_at.is_none_or(|expiry| expiry > Utc::now()))
        .ok_or_else(|| AppError::Forbidden("Live curation grant required".into()))
}

pub fn require_scope(sa: &ServiceAccount, token_scope: &str, required: &str) -> AppResult<()> {
    if !token_scope.split_whitespace().any(|s| s == required)
        || !sa.allowed_scopes.split_whitespace().any(|s| s == required)
    {
        return Err(AppError::Forbidden(format!("{required} scope required")));
    }
    Ok(())
}

pub fn require_service<'a>(
    sa: &'a ServiceAccount,
    scope: &str,
    required: &str,
    service_id: &str,
) -> AppResult<&'a CurationGrant> {
    let grant = live_grant(sa)?;
    require_scope(sa, scope, required)?;
    if !grant.service_ids.iter().any(|id| id == service_id) {
        return Err(AppError::NotFound("Service not found".into()));
    }
    Ok(grant)
}

pub async fn issue(
    db: &Database,
    sa_id: &str,
    actor_id: &str,
    input: IssueCurationGrant,
) -> AppResult<ServiceAccount> {
    if input.service_ids.is_empty()
        || input.service_ids.len() > 100
        || input.max_writes < 1
        || input.max_writes > 10_000
        || !(60..=86_400).contains(&input.window_seconds)
        || input.expires_at.is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(AppError::ValidationError("Grant requires 1-100 services, 1-10000 writes per 60-86400 second window, and a future expiry when supplied".into()));
    }
    let ids: HashSet<_> = input.service_ids.iter().collect();
    if ids.len() != input.service_ids.len()
        || ids.iter().any(|id| Uuid::parse_str(id).is_err())
        || input
            .ornn_proxy_service_id
            .as_ref()
            .is_some_and(|id| Uuid::parse_str(id).is_err())
    {
        return Err(AppError::ValidationError(
            "Grant service IDs must be distinct UUIDs".into(),
        ));
    }
    let mut targets = input.service_ids.clone();
    if let Some(id) = &input.ornn_proxy_service_id {
        targets.push(id.clone());
    }
    targets.sort();
    targets.dedup();
    if db
        .collection::<DownstreamService>(SERVICES)
        .count_documents(doc! {"_id": {"$in": &targets}})
        .await?
        != targets.len() as u64
    {
        return Err(AppError::ValidationError(
            "Every grant target must be an existing catalog service".into(),
        ));
    }
    if let Some(target) = &input.ornn_proxy_service_id
        && db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {
                "_id": target, "is_active": true, "service_type": "http",
                "proxy_operation_policy": {"$ne": bson::Bson::Null}
            })
            .await?
            .is_none()
    {
        return Err(AppError::ValidationError("Curation proxy target requires an active HTTP catalog service with an explicit proxy operation policy".into()));
    }
    let sa = super::service_account_service::get_service_account(db, sa_id).await?;
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": sa.effective_owner_user_id()})
        .await?
        .ok_or_else(|| AppError::ValidationError("Service account owner not found".into()))?;
    if owner.user_type.is_org() {
        return Err(AppError::Forbidden(
            "Org-owned accounts cannot receive curation grants".into(),
        ));
    }
    validate_scopes(&sa.allowed_scopes, input.ornn_proxy_service_id.as_deref())?;
    let now = Utc::now();
    let grant = CurationGrant {
        id: Uuid::new_v4().to_string(),
        service_ids: input.service_ids,
        ornn_proxy_service_id: input.ornn_proxy_service_id,
        issued_by: actor_id.to_owned(),
        issued_at: now,
        expires_at: input.expires_at,
        max_writes: input.max_writes,
        window_seconds: input.window_seconds,
        window_started_at: now,
        writes_used: 0,
    };
    db.collection::<ServiceAccount>(ACCOUNTS).find_one_and_update(
        doc! {"_id": sa_id, "allowed_scopes": &sa.allowed_scopes, "owner_user_id": bson::to_bson(&sa.owner_user_id).map_err(|e| AppError::Internal(e.to_string()))?},
        doc! {"$set": {"purpose": "curation", "platform_protected": true,
            "curation_grant": bson::to_bson(&grant).map_err(|e| AppError::Internal(e.to_string()))?, "updated_at": bson::DateTime::from_chrono(now)}})
        .return_document(mongodb::options::ReturnDocument::After).await?
        .ok_or_else(|| AppError::Conflict("Service account changed; reload before issuing grant".into()))
}

pub async fn revoke(db: &Database, sa_id: &str) -> AppResult<ServiceAccount> {
    db.collection::<ServiceAccount>(ACCOUNTS).find_one_and_update(doc! {"_id": sa_id},
        doc! {"$unset": {"curation_grant": ""}, "$set": {"updated_at": bson::DateTime::from_chrono(Utc::now())}})
        .return_document(mongodb::options::ReturnDocument::After).await?
        .ok_or_else(|| AppError::ServiceAccountNotFound(sa_id.into()))
}
