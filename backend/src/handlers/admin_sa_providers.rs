use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, service_account_service, user_token_service};

// --- Request types ---

#[derive(Deserialize)]
pub struct AdminConnectApiKeyRequest {
    pub api_key: String,
    pub label: Option<String>,
}

impl std::fmt::Debug for AdminConnectApiKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminConnectApiKeyRequest")
            .field("api_key", &"[REDACTED]")
            .field("label", &self.label)
            .finish()
    }
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct AdminSaProviderTokenResponse {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub provider_type: String,
    pub status: String,
    pub label: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaProviderListResponse {
    pub tokens: Vec<AdminSaProviderTokenResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminSaProviderActionResponse {
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaOAuthInitiateResponse {
    pub authorization_url: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaDeviceCodeInitiateResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub state: String,
    pub expires_in: i64,
    pub interval: i32,
}

#[derive(Debug, Deserialize)]
pub struct AdminSaDeviceCodePollRequest {
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaDeviceCodePollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<i32>,
}

// --- Handlers ---

/// GET /api/v1/admin/service-accounts/{sa_id}/providers
pub async fn list_sa_providers(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(sa_id): Path<String>,
) -> AppResult<Json<AdminSaProviderListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.service_accounts.providers.list").await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    let summaries = user_token_service::list_user_tokens(&state.db, &sa_id).await?;

    let tokens: Vec<AdminSaProviderTokenResponse> = summaries
        .into_iter()
        .map(|s| AdminSaProviderTokenResponse {
            provider_id: s.provider_config_id,
            provider_name: s.provider_name,
            provider_slug: s.provider_slug,
            provider_type: s.provider_type,
            status: s.status,
            label: s.label,
            expires_at: s.expires_at,
            last_used_at: s.last_used_at,
            connected_at: s.connected_at,
        })
        .collect();

    Ok(Json(AdminSaProviderListResponse { tokens }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/api-key
pub async fn connect_api_key_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
    Json(body): Json<AdminConnectApiKeyRequest>,
) -> AppResult<Json<AdminSaProviderActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists and is active
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    if body.api_key.is_empty() {
        return Err(AppError::ValidationError(
            "API key must not be empty".to_string(),
        ));
    }
    if body.api_key.len() > 4096 {
        return Err(AppError::ValidationError(
            "API key exceeds maximum length".to_string(),
        ));
    }

    // Reuse existing service -- pass sa.id as user_id
    user_token_service::store_api_key(
        &state.db,
        &state.encryption_keys,
        &sa_id,
        &provider_id,
        &body.api_key,
        body.label.as_deref(),
        None, // service accounts don't use gateway URLs
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.provider_connected",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
            "token_type": "api_key",
        })),
    );

    Ok(Json(AdminSaProviderActionResponse {
        status: "connected".to_string(),
        message: "API key stored for service account".to_string(),
    }))
}

/// DELETE /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/disconnect
pub async fn disconnect_sa_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaProviderActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    user_token_service::disconnect_provider(
        &state.db,
        &state.encryption_keys,
        &sa_id,
        &provider_id,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.provider_disconnected",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
    );

    Ok(Json(AdminSaProviderActionResponse {
        status: "disconnected".to_string(),
        message: "Provider disconnected from service account".to_string(),
    }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/oauth
/// (legacy: GET — kept one release for back-compat, see `routes.rs`)
///
/// Admin initiates an OAuth redirect flow on behalf of a service account.
/// Returns the authorization URL for the admin to redirect to.
///
/// This is a state-mutating action (creates an OAuth state row, emits an
/// audit entry), so it MUST stay behind `require_admin` — never weaken to
/// `require_admin_or_operator`. The route is mounted as POST under the
/// canonical name; the GET fallback exists only so older clients keep
/// working during a rolling deploy.
pub async fn initiate_oauth_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaOAuthInitiateResponse>> {
    require_admin(&state, &auth_user).await?;

    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let admin_id = auth_user.user_id.to_string();
    let redirect_path = format!("/admin/service-accounts/{}", &sa_id);

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        &admin_id,
        &provider_id,
        Some(&sa_id),
        Some(&redirect_path),
        &[],
        None, // no scope_override: SA-connect picker is a later pass (NyxID#917)
        None, // admin-on-behalf-of flow stays single-tenant per SA
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.oauth_initiated",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
    );

    Ok(Json(AdminSaOAuthInitiateResponse {
        authorization_url: auth_url,
    }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/initiate
///
/// Admin initiates a device code flow on behalf of a service account.
/// Returns user_code and verification_uri for the admin to authenticate.
pub async fn initiate_device_code_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaDeviceCodeInitiateResponse>> {
    require_admin(&state, &auth_user).await?;

    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let admin_id = auth_user.user_id.to_string();

    let result = user_token_service::request_device_code(
        &state.db,
        &state.encryption_keys,
        &admin_id,
        &provider_id,
        Some(&sa_id),
        &[],
        None, // no scope_override: SA-connect picker is a later pass (NyxID#917)
        None, // admin-on-behalf-of flow stays single-tenant per SA
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.sa.device_code_initiated",
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
    );

    Ok(Json(AdminSaDeviceCodeInitiateResponse {
        user_code: result.user_code,
        verification_uri: result.verification_uri,
        state: result.state,
        expires_in: result.expires_in,
        interval: result.interval,
    }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/poll
///
/// Admin polls device code status for a service account flow.
/// Returns status: "pending", "slow_down", "expired", "denied", or "complete".
pub async fn poll_device_code_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
    Json(body): Json<AdminSaDeviceCodePollRequest>,
) -> AppResult<Json<AdminSaDeviceCodePollResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists (no active check needed for polling)
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    let admin_id = auth_user.user_id.to_string();

    let result = user_token_service::poll_device_code(
        &state.db,
        &state.encryption_keys,
        &admin_id,
        &provider_id,
        &body.state,
    )
    .await?;

    if result.status == "complete" {
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "admin.sa.provider_connected",
            Some(serde_json::json!({
                "target_sa_id": &sa_id,
                "provider_id": &provider_id,
                "token_type": "device_code",
            })),
        );
    }

    Ok(Json(AdminSaDeviceCodePollResponse {
        status: result.status,
        interval: result.interval,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, State};
    use axum::http::HeaderMap;
    use chrono::Utc;
    use uuid::Uuid;

    async fn seed_admin(db: &mongodb::Database) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed roles");
        let ids = role_service::get_platform_role_ids(db)
            .await
            .expect("role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.role_ids.push(ids.admin);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert admin");
        id
    }

    fn fixture_sa(admin_id: &str) -> ServiceAccount {
        let now = Utc::now();
        ServiceAccount {
            id: Uuid::new_v4().to_string(),
            name: "Test SA".to_string(),
            description: None,
            client_id: format!("sa_{}", hex::encode([1u8; 12])),
            client_secret_hash: "0".repeat(64),
            secret_prefix: "sas_test".to_string(),
            role_ids: vec![],
            allowed_scopes: "proxy:*".to_string(),
            is_active: true,
            rate_limit_override: None,
            created_by: admin_id.to_string(),
            owner_user_id: None,
            created_at: now,
            updated_at: now,
            last_authenticated_at: None,
        }
    }

    #[tokio::test]
    async fn test_list_sa_providers_empty() {
        let Some(db) = connect_test_database("h_sa_prov_list").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let Json(resp) = list_sa_providers(State(state), auth, Path(sa_id))
            .await
            .expect("list should succeed");

        assert!(resp.tokens.is_empty());
    }

    #[tokio::test]
    async fn test_list_sa_providers_sa_not_found() {
        let Some(db) = connect_test_database("h_sa_prov_list_404").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let err = list_sa_providers(State(state), auth, Path(Uuid::new_v4().to_string()))
            .await
            .expect_err("missing SA should fail");

        assert!(matches!(err, AppError::ServiceAccountNotFound(_)));
    }

    #[tokio::test]
    async fn test_connect_api_key_empty_rejected() {
        let Some(db) = connect_test_database("h_sa_prov_connect_empty").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let err = connect_api_key_for_sa(
            State(state),
            auth,
            HeaderMap::new(),
            Path((sa_id, Uuid::new_v4().to_string())),
            Json(AdminConnectApiKeyRequest {
                api_key: String::new(),
                label: None,
            }),
        )
        .await
        .expect_err("empty key should be rejected");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn test_connect_api_key_inactive_sa_rejected() {
        let Some(db) = connect_test_database("h_sa_prov_connect_inactive").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let mut sa = fixture_sa(&admin_id);
        sa.is_active = false;
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let err = connect_api_key_for_sa(
            State(state),
            auth,
            HeaderMap::new(),
            Path((sa_id, Uuid::new_v4().to_string())),
            Json(AdminConnectApiKeyRequest {
                api_key: "sk-test-key".to_string(),
                label: None,
            }),
        )
        .await
        .expect_err("inactive SA should be rejected");

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_disconnect_sa_provider_not_found() {
        let Some(db) = connect_test_database("h_sa_prov_disconnect_404").await else {
            return;
        };
        let admin_id = seed_admin(&db).await;
        let sa = fixture_sa(&admin_id);
        let sa_id = sa.id.clone();
        db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .insert_one(&sa)
            .await
            .expect("insert sa");

        let state = test_app_state(db);
        let auth = test_auth_user(&admin_id);

        let err = disconnect_sa_provider(
            State(state),
            auth,
            HeaderMap::new(),
            Path((sa_id, Uuid::new_v4().to_string())),
        )
        .await
        .expect_err("disconnect non-existent provider should fail");

        assert!(matches!(err, AppError::NotFound(_)));
    }
}
