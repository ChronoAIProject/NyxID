use mongodb::{
    ClientSession, Database,
    bson::{Document, doc},
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        role::COLLECTION_NAME as ROLES,
        service_account::{ServiceAccount, ServiceAccountPurpose},
    },
};

pub const READ_PERMISSION: &str = "nyxid:catalog:skills:read";
pub const WRITE_PERMISSION: &str = "nyxid:catalog:skills:write";

pub fn has_catalog_scopes(scopes: &str) -> bool {
    scopes
        .split_whitespace()
        .any(|scope| matches!(scope, "catalog:skills:read" | "catalog:skills:write"))
}

pub async fn role_has_editor_permissions(db: &Database, role_ids: &[String]) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(ROLES)
        .find_one(doc! {
            "_id": {"$in": role_ids}, "client_id": null,
            "permissions": {"$in": [READ_PERMISSION, WRITE_PERMISSION]},
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

fn permission(
    sa: &ServiceAccount,
    token_scope: &str,
    required_scope: &str,
) -> AppResult<&'static str> {
    if !sa.is_active || !sa.platform_protected || sa.purpose != ServiceAccountPurpose::CatalogEditor
    {
        return Err(AppError::Forbidden(
            "Catalog editor authority required".into(),
        ));
    }
    let required_permission = match required_scope {
        "catalog:skills:read" => READ_PERMISSION,
        "catalog:skills:write" => WRITE_PERMISSION,
        _ => {
            return Err(AppError::Forbidden(
                "Unsupported catalog editor scope".into(),
            ));
        }
    };
    super::curation_grant_service::require_scope(sa, token_scope, required_scope)?;
    Ok(required_permission)
}

pub async fn authorize(
    db: &Database,
    sa: &ServiceAccount,
    token_scope: &str,
    required_scope: &str,
) -> AppResult<()> {
    let required_permission = permission(sa, token_scope, required_scope)?;
    if sa.catalog_scope_authorized {
        return Ok(());
    }
    let allowed = db
        .collection::<Document>(ROLES)
        .find_one(doc! {
            "_id": {"$in": &sa.role_ids}, "client_id": null, "permissions": required_permission,
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some();
    if !allowed {
        return Err(AppError::Forbidden(format!(
            "{required_permission} role permission required"
        )));
    }
    Ok(())
}

pub async fn authorize_in_session(
    db: &Database,
    session: &mut ClientSession,
    sa: &ServiceAccount,
    token_scope: &str,
    required_scope: &str,
) -> AppResult<()> {
    let required_permission = permission(sa, token_scope, required_scope)?;
    if sa.catalog_scope_authorized {
        return Ok(());
    }
    db.collection::<Document>(ROLES)
        .find_one(doc! {
            "_id": {"$in": &sa.role_ids}, "client_id": null, "permissions": required_permission,
        })
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .ok_or_else(|| {
            AppError::Forbidden(format!("{required_permission} role permission required"))
        })?;
    #[cfg(test)]
    if let Ok((reached, resume, paused)) =
        super::catalog_skill_service::AUTHORITY_PAUSE.try_with(Clone::clone)
        && !paused.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        reached.wait().await;
        resume.wait().await;
    }
    Ok(())
}

pub async fn fence_write_in_session(
    db: &Database,
    session: &mut ClientSession,
    sa: &ServiceAccount,
) -> AppResult<()> {
    if sa.catalog_scope_authorized {
        return Ok(());
    }
    let result = db
        .collection::<Document>(ROLES)
        .update_one(
            doc! {"_id": {"$in": &sa.role_ids}, "client_id": null, "permissions": WRITE_PERMISSION},
            doc! {"$inc": {"catalog_editor_write_fence": 1_i64}},
        )
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Forbidden(
            "Catalog editor role permission changed".into(),
        ));
    }
    Ok(())
}

pub fn validate_scopes(scopes: &str) -> AppResult<()> {
    if scopes.split_whitespace().next().is_none()
        || scopes.split_whitespace().any(|scope| {
            !matches!(
                scope,
                "catalog:skills:read" | "catalog:skills:write" | "user-services:read" | "proxy"
            )
        })
    {
        return Err(AppError::ValidationError("Catalog editor scopes must be catalog:skills:read, catalog:skills:write, user-services:read, or proxy".into()));
    }
    Ok(())
}
