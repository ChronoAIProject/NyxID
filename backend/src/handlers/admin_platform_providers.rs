use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, platform_credential_service};

#[derive(Debug, Serialize)]
pub struct AdminPlatformProviderListResponse {
    pub providers: Vec<AdminPlatformProviderResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformProviderResponse {
    pub catalog_service_id: String,
    pub catalog_service_slug: String,
    pub catalog_service_name: String,
    pub catalog_service_active: bool,
    pub eligible: bool,
    pub eligibility_reason: Option<String>,
    pub promoted: bool,
    pub promoted_at: Option<String>,
    pub promoted_by: Option<String>,
    pub vendor_terms_accepted_at: Option<String>,
    pub vendor_terms_accepted_by: Option<String>,
    pub credential: AdminPlatformCredentialResponse,
    pub enabled_operation_count: u64,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformCredentialResponse {
    pub configured: bool,
    pub id: Option<String>,
    pub auth_method: Option<String>,
    pub auth_key_name: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotePlatformProviderRequest {
    pub vendor_terms_accepted: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPlatformCredentialRequest {
    pub credential: String,
}

impl std::fmt::Debug for SetPlatformCredentialRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SetPlatformCredentialRequest")
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

fn response(
    status: platform_credential_service::PlatformProviderLifecycleStatus,
) -> AdminPlatformProviderResponse {
    let promotion = status.promotion;
    let credential = status.credential;
    AdminPlatformProviderResponse {
        catalog_service_id: status.catalog_service.id,
        catalog_service_slug: status.catalog_service.slug,
        catalog_service_name: status.catalog_service.name,
        catalog_service_active: status.catalog_service.is_active,
        eligible: status.eligible,
        eligibility_reason: status.eligibility_reason,
        promoted: promotion.is_some(),
        promoted_at: promotion.as_ref().map(|row| row.promoted_at.to_rfc3339()),
        promoted_by: promotion.as_ref().map(|row| row.promoted_by.clone()),
        vendor_terms_accepted_at: promotion
            .as_ref()
            .map(|row| row.vendor_terms_accepted_at.to_rfc3339()),
        vendor_terms_accepted_by: promotion
            .as_ref()
            .map(|row| row.vendor_terms_accepted_by.clone()),
        credential: AdminPlatformCredentialResponse {
            configured: credential.is_some(),
            id: credential.as_ref().map(|row| row.id.clone()),
            auth_method: credential.as_ref().map(|row| row.auth_method.clone()),
            auth_key_name: credential.as_ref().map(|row| row.auth_key_name.clone()),
            created_at: credential.as_ref().map(|row| row.created_at.to_rfc3339()),
            updated_at: credential.map(|row| row.updated_at.to_rfc3339()),
        },
        enabled_operation_count: status.enabled_operation_count,
    }
}

/// GET /api/v1/admin/platform-providers
pub async fn list_platform_providers(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminPlatformProviderListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.platform_providers.list").await?;
    let providers = platform_credential_service::list_provider_lifecycle_statuses(&state.db)
        .await?
        .into_iter()
        .map(response)
        .collect();
    Ok(Json(AdminPlatformProviderListResponse { providers }))
}

/// GET /api/v1/admin/platform-providers/{catalog_service_id}
pub async fn get_platform_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
) -> AppResult<Json<AdminPlatformProviderResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.platform_providers.get").await?;
    Ok(Json(response(
        platform_credential_service::provider_lifecycle_status(&state.db, &catalog_service_id)
            .await?,
    )))
}

/// PUT /api/v1/admin/platform-providers/{catalog_service_id}
pub async fn promote_platform_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
    Json(body): Json<PromotePlatformProviderRequest>,
) -> AppResult<Json<AdminPlatformProviderResponse>> {
    require_admin(&state, &auth_user).await?;
    let status = platform_credential_service::promote_provider(
        &state.db,
        &catalog_service_id,
        body.vendor_terms_accepted,
        &auth_user.user_id.to_string(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_provider_promoted",
        Some(serde_json::json!({
            "catalog_service_id": catalog_service_id,
            "catalog_service_slug": status.catalog_service.slug,
            "vendor_terms_accepted": true,
        })),
    );
    Ok(Json(response(status)))
}

/// DELETE /api/v1/admin/platform-providers/{catalog_service_id}
pub async fn demote_platform_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
) -> AppResult<Json<AdminPlatformProviderResponse>> {
    require_admin(&state, &auth_user).await?;
    let status =
        platform_credential_service::demote_provider(&state.db, &catalog_service_id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_provider_demoted",
        Some(serde_json::json!({
            "catalog_service_id": catalog_service_id,
            "catalog_service_slug": status.catalog_service.slug,
        })),
    );
    Ok(Json(response(status)))
}

/// PUT /api/v1/admin/platform-providers/{catalog_service_id}/credential
pub async fn set_platform_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
    Json(body): Json<SetPlatformCredentialRequest>,
) -> AppResult<Json<AdminPlatformProviderResponse>> {
    require_admin(&state, &auth_user).await?;
    platform_credential_service::set_credential(
        &state.db,
        &state.encryption_keys,
        &catalog_service_id,
        &body.credential,
        &auth_user.user_id.to_string(),
    )
    .await?;
    let status =
        platform_credential_service::provider_lifecycle_status(&state.db, &catalog_service_id)
            .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_credential_set",
        Some(serde_json::json!({
            "catalog_service_id": catalog_service_id,
            "catalog_service_slug": status.catalog_service.slug,
            "configured": true,
        })),
    );
    Ok(Json(response(status)))
}

/// DELETE /api/v1/admin/platform-providers/{catalog_service_id}/credential
pub async fn delete_platform_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(catalog_service_id): Path<String>,
) -> AppResult<Json<AdminPlatformProviderResponse>> {
    require_admin(&state, &auth_user).await?;
    let status =
        platform_credential_service::delete_credential(&state.db, &catalog_service_id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_credential_deleted",
        Some(serde_json::json!({
            "catalog_service_id": catalog_service_id,
            "catalog_service_slug": status.catalog_service.slug,
            "configured": false,
        })),
    );
    Ok(Json(response(status)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::platform_credential::{
        COLLECTION_NAME as PLATFORM_CREDENTIALS, PlatformCredential,
    };
    use crate::models::platform_provider_promotion::{
        COLLECTION_NAME as PLATFORM_PROVIDER_PROMOTIONS, PlatformProviderPromotion,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use mongodb::bson::doc;

    fn provider() -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.name = "Duffel".to_string();
        service.slug = "duffel".to_string();
        service.auth_method = "bearer".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.requires_user_credential = true;
        service
    }

    async fn insert_actor(db: &mongodb::Database, admin: bool) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let roles = role_service::get_platform_role_ids(db)
            .await
            .expect("load platform role ids");
        let id = uuid::Uuid::new_v4().to_string();
        let mut actor = test_user(&id, UserType::Person);
        actor
            .role_ids
            .push(if admin { roles.admin } else { roles.operator });
        db.collection::<User>(USERS)
            .insert_one(&actor)
            .await
            .expect("insert platform actor");
        id
    }

    #[test]
    fn credential_request_debug_redacts_plaintext() {
        let request = SetPlatformCredentialRequest {
            credential: "vendor-secret-that-must-not-leak".to_string(),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("vendor-secret-that-must-not-leak"));
    }

    #[tokio::test]
    async fn operator_can_read_but_cannot_promote_or_write_a_credential() {
        let Some(db) = connect_test_database("admin_platform_provider_operator").await else {
            return;
        };
        let operator_id = insert_actor(&db, false).await;
        let service = provider();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert provider");
        let state = test_app_state(db.clone());
        let auth = test_auth_user(&operator_id);

        let listing = list_platform_providers(State(state.clone()), auth.clone())
            .await
            .expect("operator can list platform providers");
        assert_eq!(listing.0.providers.len(), 1);
        assert!(!listing.0.providers[0].promoted);

        let promote_error = promote_platform_provider(
            State(state.clone()),
            auth.clone(),
            Path(service.id.clone()),
            Json(PromotePlatformProviderRequest {
                vendor_terms_accepted: true,
            }),
        )
        .await
        .expect_err("operator must not promote providers");
        assert!(matches!(
            promote_error,
            crate::errors::AppError::Forbidden(_)
        ));
        let credential_error = set_platform_credential(
            State(state),
            auth,
            Path(service.id.clone()),
            Json(SetPlatformCredentialRequest {
                credential: "operator-secret".to_string(),
            }),
        )
        .await
        .expect_err("operator must not set credentials");
        assert!(matches!(
            credential_error,
            crate::errors::AppError::Forbidden(_)
        ));
        assert_eq!(
            db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
                .count_documents(doc! {})
                .await
                .expect("count promotions"),
            0
        );
        assert_eq!(
            db.collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
                .count_documents(doc! {})
                .await
                .expect("count credentials"),
            0
        );
    }

    #[tokio::test]
    async fn admin_lifecycle_response_is_metadata_only_and_replacement_keeps_one_row() {
        let Some(db) = connect_test_database("admin_platform_provider_lifecycle").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("create platform provider indexes");
        let admin_id = insert_actor(&db, true).await;
        let service = provider();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert provider");
        let state = test_app_state(db.clone());
        let auth = test_auth_user(&admin_id);

        let promoted = promote_platform_provider(
            State(state.clone()),
            auth.clone(),
            Path(service.id.clone()),
            Json(PromotePlatformProviderRequest {
                vendor_terms_accepted: true,
            }),
        )
        .await
        .expect("promote provider")
        .0;
        assert!(promoted.promoted);
        assert!(promoted.vendor_terms_accepted_at.is_some());
        assert!(!promoted.credential.configured);
        assert_eq!(promoted.enabled_operation_count, 0);

        let first = set_platform_credential(
            State(state.clone()),
            auth.clone(),
            Path(service.id.clone()),
            Json(SetPlatformCredentialRequest {
                credential: "first-admin-secret".to_string(),
            }),
        )
        .await
        .expect("set credential")
        .0;
        let second = set_platform_credential(
            State(state),
            auth,
            Path(service.id.clone()),
            Json(SetPlatformCredentialRequest {
                credential: "second-admin-secret".to_string(),
            }),
        )
        .await
        .expect("replace credential")
        .0;
        assert!(second.credential.configured);
        assert_eq!(first.credential.id, second.credential.id);
        let serialized = serde_json::to_string(&second).expect("serialize metadata response");
        assert!(!serialized.contains("first-admin-secret"));
        assert!(!serialized.contains("second-admin-secret"));
        assert!(!serialized.contains("credential_encrypted"));
        assert_eq!(
            db.collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
                .count_documents(doc! { "catalog_service_id": &service.id })
                .await
                .expect("count credentials"),
            1
        );
    }
}
