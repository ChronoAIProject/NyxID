use crate::{
    errors::{AppError, AppResult},
    models::{
        service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount},
        user::{COLLECTION_NAME as USERS, User},
    },
    mw::auth::{AuthMethod, AuthUser},
};
use mongodb::{Database, bson::doc};

pub async fn authorize(db: &Database, auth: &AuthUser, write: bool) -> AppResult<()> {
    let scope = if write {
        "catalog:services:write"
    } else {
        "catalog:services:read"
    };
    if auth.auth_method == AuthMethod::ServiceAccount {
        let sa = db
            .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .find_one(doc! {"_id":auth.user_id.to_string()})
            .await?
            .ok_or_else(|| AppError::Forbidden("Catalog editor authority required".into()))?;
        return super::catalog_services_editor_service::authorize(db, &sa, &auth.scope, scope)
            .await;
    }
    if !matches!(
        auth.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken
    ) {
        return Err(AppError::Forbidden(
            "Human or catalog editor authority required".into(),
        ));
    }
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":auth.user_id.to_string()})
        .await?
        .ok_or_else(|| AppError::Forbidden("Catalog editor authority required".into()))?;
    if super::role_service::resolve_platform_role(db, &user)
        .await?
        .is_admin()
    {
        return Ok(());
    }
    let rbac =
        super::rbac_helpers::resolve_rbac_from_ids(db, &user.role_ids, &user.group_ids).await?;
    let permission = if write {
        super::catalog_services_editor_service::WRITE_PERMISSION
    } else {
        super::catalog_services_editor_service::READ_PERMISSION
    };
    if !rbac.permissions.iter().any(|p| p == permission) {
        return Err(AppError::Forbidden(format!("{permission} required")));
    }
    Ok(())
}

pub fn validate_editor_fields(value: &serde_json::Value, create: bool) -> AppResult<()> {
    let allowed = [
        "name",
        "description",
        "visibility",
        "openapi_spec_url",
        "api_spec_url",
        "asyncapi_spec_url",
        "homepage_url",
        "repository_url",
        "issues_url",
        "examples_url",
        "auth_notes",
        "known_limitations",
        "required_permissions",
        "capabilities",
        "topics",
        "supplier",
        "import_source",
        "offering_kind",
    ];
    for field in value
        .as_object()
        .ok_or_else(|| AppError::ValidationError("Expected an object".into()))?
        .keys()
    {
        if !allowed.contains(&field.as_str()) && !(create && field == "slug") {
            return Err(AppError::Forbidden(format!(
                "Field {field} requires platform admin authority"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_fields_reject_admin_fields_even_when_null() {
        for field in [
            "credential",
            "platform_key",
            "billing",
            "base_url",
            "auth_method",
            "auth_key_name",
            "service_category",
            "provider_config_id",
            "destination_targets",
            "forward_access_token",
            "token_exchange_config",
            "anonymous_endpoints",
            "proxy_operation_policy",
            "default_request_headers",
            "ws_frame_injections",
            "developer_app_ids",
        ] {
            let mut value = serde_json::Map::new();
            value.insert(field.into(), serde_json::Value::Null);
            let error =
                validate_editor_fields(&serde_json::Value::Object(value), false).unwrap_err();
            assert!(matches!(error, AppError::Forbidden(message) if message.contains(field)));
        }
        assert!(validate_editor_fields(&serde_json::json!({"offering_kind":"tool","topics":["social"],"supplier":"Example"}), false).is_ok());
        assert!(validate_editor_fields(&serde_json::json!({"slug":"tool"}), true).is_ok());
        assert!(validate_editor_fields(&serde_json::json!({"slug":"tool"}), false).is_err());
    }
}

pub async fn authorize_tool(
    db: &Database,
    auth: &AuthUser,
    service: &crate::models::downstream_service::DownstreamService,
    write: bool,
) -> AppResult<()> {
    if service.offering_kind != crate::models::downstream_service::OfferingKind::Tool {
        if auth.auth_method == AuthMethod::ServiceAccount {
            return Err(AppError::NotFound("Tool not found".into()));
        }
        let user = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id":auth.user_id.to_string()})
            .await?
            .ok_or_else(|| AppError::Forbidden("Admin required".into()))?;
        if !super::role_service::resolve_platform_role(db, &user)
            .await?
            .is_admin()
        {
            return Err(AppError::Forbidden("Admin required".into()));
        }
        return Ok(());
    }
    authorize(db, auth, write).await
}
