use mongodb::{Database, bson::doc};

use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        service_account::{ServiceAccount, ServiceAccountPurpose},
    },
};

/// Retains an admin-selected Ornn target when converting a legacy editor.
/// Fresh editors use the platform's canonical Ornn service.
pub async fn authorized_target(
    db: &Database,
    sa: &ServiceAccount,
    scope: &str,
) -> AppResult<String> {
    if !sa.is_active || !sa.platform_protected || sa.purpose != ServiceAccountPurpose::CatalogEditor
    {
        return Err(AppError::Forbidden(
            "Catalog editor authority required".into(),
        ));
    }
    super::curation_grant_service::require_scope(sa, scope, "proxy")?;
    if !super::catalog_editor_service::role_has_editor_permissions(db, &sa.role_ids).await? {
        return Err(AppError::Forbidden(
            "Catalog editor role permission required".into(),
        ));
    }
    let mut filter = doc! {"is_active": true, "service_type": "http"};
    if let Some(id) = sa
        .curation_grant
        .as_ref()
        .and_then(|g| g.ornn_proxy_service_id.as_ref())
    {
        filter.insert("_id", id);
    } else {
        filter.insert("slug", "ornn-api");
    }
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::Forbidden("Active Ornn catalog service required".into()))?;
    if service.proxy_operation_policy.is_none() {
        return Err(AppError::Forbidden(
            "Ornn requires an explicit proxy operation policy".into(),
        ));
    }
    Ok(service.id)
}
