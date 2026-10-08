use mongodb::{
    Database,
    bson::{Document, doc},
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        role::COLLECTION_NAME as ROLES,
        service_account::{ServiceAccount, ServiceAccountPurpose},
    },
};

pub const READ_PERMISSION: &str = "nyxid:catalog:services:read";
pub const WRITE_PERMISSION: &str = "nyxid:catalog:services:write";

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
        "catalog:services:read" => READ_PERMISSION,
        "catalog:services:write" => WRITE_PERMISSION,
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
