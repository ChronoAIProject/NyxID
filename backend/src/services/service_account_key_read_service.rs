use std::collections::HashSet;

use chrono::{DateTime, Utc};
use mongodb::{Database, bson::doc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        catalog_skill_revision::SkillState,
        downstream_service::COLLECTION_NAME as CATALOG,
        service_account::{ServiceAccount, ServiceAccountPurpose},
        service_account_key_read_grant::{
            COLLECTION_NAME as GRANTS, KeyReadTarget, ServiceAccountKeyReadGrant,
        },
        user::COLLECTION_NAME as USERS,
        user_endpoint::COLLECTION_NAME as ENDPOINTS,
        user_service::COLLECTION_NAME as SERVICES,
    },
    services::{curation_grant_service, org_service, service_account_service, service_history},
};

pub const READ_SCOPE: &str = "user-services:read";

pub fn is_key_metadata_path(path: &str) -> bool {
    path.strip_prefix("/api/v1/keys/")
        .is_some_and(|id| id.len() == 36 && Uuid::parse_str(id).is_ok())
}

#[derive(Deserialize)]
struct ServiceRow {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    endpoint_id: String,
    slug: String,
    #[serde(default = "http_service_type")]
    service_type: String,
    is_active: bool,
    catalog_service_id: Option<String>,
}

fn http_service_type() -> String {
    "http".into()
}

#[derive(Deserialize)]
struct EndpointRow {
    label: String,
    recommended_skills: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct CatalogRow {
    slug: String,
    name: String,
    #[serde(flatten)]
    skills: SkillState,
    #[serde(default)]
    skills_revision: i64,
}

pub struct KeyMetadata {
    pub id: String,
    pub slug: String,
    pub label: String,
    pub service_type: String,
    pub is_active: bool,
    pub catalog_service_id: Option<String>,
    pub catalog_service_slug: Option<String>,
    pub catalog_service_name: Option<String>,
    pub skills: SkillState,
    pub skills_revision: Option<i64>,
}

fn missing() -> AppError {
    AppError::NotFound("Key not found".into())
}

async fn service(db: &Database, id: &str) -> AppResult<ServiceRow> {
    if id.len() != 36 || Uuid::parse_str(id).is_err() {
        return Err(missing());
    }
    service_history::collection::<ServiceRow>(db, SERVICES)
        .find_one(doc! {"_id": id, "deleted_at": null})
        .projection(doc! {"_id": 1, "user_id": 1, "endpoint_id": 1, "slug": 1,
        "service_type": 1, "is_active": 1, "catalog_service_id": 1})
        .await?
        .ok_or_else(missing)
}

async fn endpoint(db: &Database, row: &ServiceRow) -> AppResult<EndpointRow> {
    service_history::collection::<EndpointRow>(db, ENDPOINTS)
        .find_one(doc! {"_id": &row.endpoint_id, "user_id": &row.user_id, "deleted_at": null})
        .projection(doc! {"label": 1, "recommended_skills": 1})
        .await?
        .ok_or_else(missing)
}

async fn check_owner_access(db: &Database, actor: &str, row: &ServiceRow) -> AppResult<()> {
    for owner in [actor, row.user_id.as_str()] {
        if db
            .collection::<mongodb::bson::Document>(USERS)
            .find_one(doc! {"_id": owner, "is_active": true})
            .projection(doc! {"_id": 1})
            .await?
            .is_none()
        {
            return Err(missing());
        }
    }
    let access = org_service::resolve_owner_access(db, actor, &row.user_id).await?;
    if !access.can_read() || !access.allows_resource(&row.id) {
        return Err(missing());
    }
    Ok(())
}

pub async fn get_grant(
    db: &Database,
    sa_id: &str,
) -> AppResult<Option<ServiceAccountKeyReadGrant>> {
    Ok(db
        .collection::<ServiceAccountKeyReadGrant>(GRANTS)
        .find_one(doc! {"_id": sa_id})
        .await?)
}

pub async fn issue(
    db: &Database,
    sa_id: &str,
    actor: &str,
    ids: &[String],
    expires_at: Option<DateTime<Utc>>,
) -> AppResult<ServiceAccountKeyReadGrant> {
    if ids.is_empty()
        || ids.len() > 100
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        || ids
            .iter()
            .any(|id| id.len() != 36 || Uuid::parse_str(id).is_err())
        || expires_at.is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(AppError::ValidationError(
            "Key read grant requires 1–100 distinct user-service UUIDs and a future expiry when supplied".into(),
        ));
    }
    let sa = service_account_service::get_service_account(db, sa_id).await?;
    let owner = sa.effective_owner_user_id();
    let mut targets = Vec::with_capacity(ids.len());
    for id in ids {
        let row = service(db, id).await?;
        check_owner_access(db, owner, &row).await?;
        endpoint(db, &row).await?;
        targets.push(KeyReadTarget {
            user_service_id: row.id,
            owner_id: row.user_id,
        });
    }
    let grant = ServiceAccountKeyReadGrant {
        service_account_id: sa_id.into(),
        owner_id: owner.into(),
        targets,
        issued_by: actor.into(),
        issued_at: Utc::now(),
        expires_at,
    };
    db.collection::<ServiceAccountKeyReadGrant>(GRANTS)
        .replace_one(doc! {"_id": sa_id}, &grant)
        .upsert(true)
        .await?;
    Ok(grant)
}

pub async fn revoke(db: &Database, sa_id: &str) -> AppResult<Option<ServiceAccountKeyReadGrant>> {
    Ok(db
        .collection::<ServiceAccountKeyReadGrant>(GRANTS)
        .find_one_and_delete(doc! {"_id": sa_id})
        .await?)
}

fn authorize_scope(sa: &ServiceAccount, token_scope: &str) -> AppResult<()> {
    if !sa.is_active {
        return Err(AppError::Unauthorized("Service account is inactive".into()));
    }
    curation_grant_service::require_scope(sa, token_scope, READ_SCOPE)?;
    if sa.purpose == ServiceAccountPurpose::Curation {
        curation_grant_service::live_grant(sa)?;
    }
    Ok(())
}

/// Reads only projected metadata; this path never loads or decrypts credentials.
pub async fn read(
    db: &Database,
    sa_id: &str,
    token_scope: &str,
    id: &str,
) -> AppResult<KeyMetadata> {
    let sa = service_account_service::get_service_account(db, sa_id).await?;
    authorize_scope(&sa, token_scope)?;
    let grant = get_grant(db, sa_id)
        .await?
        .filter(|g| g.owner_id == sa.effective_owner_user_id())
        .filter(|g| g.expires_at.is_none_or(|expiry| expiry > Utc::now()))
        .ok_or_else(|| AppError::Forbidden("Live key read grant required".into()))?;
    let target = grant
        .targets
        .iter()
        .find(|t| t.user_service_id == id)
        .ok_or_else(missing)?;
    let row = service(db, id).await?;
    if row.user_id != target.owner_id {
        return Err(missing());
    }
    check_owner_access(db, &grant.owner_id, &row).await?;
    let endpoint = endpoint(db, &row).await?;
    let catalog = if let Some(id) = &row.catalog_service_id {
        db.collection::<CatalogRow>(CATALOG)
            .find_one(doc! {"_id": id})
            .projection(doc! {"slug": 1, "name": 1, "recommended_skills": 1,
            "recommended_skill_refs": 1, "skills_revision": 1})
            .await?
    } else {
        None
    };
    let (skills, skills_revision) = if endpoint.recommended_skills.is_some() {
        (
            SkillState {
                recommended_skills: endpoint.recommended_skills,
                recommended_skill_refs: None,
            },
            None,
        )
    } else if let Some(catalog) = &catalog {
        (catalog.skills.clone(), Some(catalog.skills_revision))
    } else {
        (SkillState::default(), None)
    };
    Ok(KeyMetadata {
        id: row.id,
        slug: row.slug,
        label: endpoint.label,
        service_type: row.service_type,
        is_active: row.is_active,
        catalog_service_id: row.catalog_service_id,
        catalog_service_slug: catalog.as_ref().map(|c| c.slug.clone()),
        catalog_service_name: catalog.map(|c| c.name),
        skills,
        skills_revision,
    })
}
