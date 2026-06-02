use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, provider_service, user_credentials_service};
use crate::telemetry::TelemetryContext;

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct SetUserCredentialsRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub label: Option<String>,
}

impl std::fmt::Debug for SetUserCredentialsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetUserCredentialsRequest")
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("label", &self.label)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct UserCredentialsResponse {
    pub provider_config_id: String,
    pub has_credentials: bool,
    pub label: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteCredentialsResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/providers/{provider_id}/credentials
pub async fn get_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<UserCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let metadata = user_credentials_service::get_user_credentials_metadata(
        &state.db,
        &user_id_str,
        &provider_id,
    )
    .await?;

    match metadata {
        Some(m) => Ok(Json(UserCredentialsResponse {
            provider_config_id: m.provider_config_id,
            has_credentials: true,
            label: m.label,
            created_at: Some(m.created_at.to_rfc3339()),
            updated_at: Some(m.updated_at.to_rfc3339()),
        })),
        None => Ok(Json(UserCredentialsResponse {
            provider_config_id: provider_id,
            has_credentials: false,
            label: None,
            created_at: None,
            updated_at: None,
        })),
    }
}

/// PUT /api/v1/providers/{provider_id}/credentials
pub async fn set_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(provider_id): Path<String>,
    Json(body): Json<SetUserCredentialsRequest>,
) -> AppResult<Json<UserCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Validate provider exists and is active
    let provider = provider_service::get_provider(&state.db, &provider_id).await?;
    if !provider.is_active {
        return Err(AppError::BadRequest("Provider is not active".to_string()));
    }

    // Validate credential_mode allows user credentials
    if !user_credentials_service::supports_user_credentials(&provider) {
        return Err(AppError::BadRequest(
            "This provider does not accept user-provided credentials".to_string(),
        ));
    }

    // Validate inputs
    if body.client_id.is_empty() || body.client_id.len() > 500 {
        return Err(AppError::ValidationError(
            "client_id must be between 1 and 500 characters".to_string(),
        ));
    }
    if body
        .client_secret
        .as_ref()
        .is_some_and(|value| value.len() > 2000)
    {
        return Err(AppError::ValidationError(
            "client_secret must be at most 2000 characters".to_string(),
        ));
    }
    let client_secret = body
        .client_secret
        .as_deref()
        .filter(|value| !value.is_empty());

    let cred = user_credentials_service::upsert_user_credentials(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        &body.client_id,
        client_secret,
        body.label.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "user_credentials_set",
        Some(serde_json::json!({
            "provider_id": &provider_id,
        })),
    );

    // Intentionally NO `ServiceConnected` emit here. This endpoint manages
    // per-user OAuth client credentials (client_id / optional client_secret),
    // not the actual `UserService`/provider binding lifecycle. Emitting a
    // service.connected event when users rotate or re-save credentials would
    // overcount connection churn even though the binding is unchanged.
    // `service.connected` / `service.disconnected` are owned by
    // `user_services_handler.rs` and the provider-connect flow handlers.
    let _ = (&provider, &tele);

    Ok(Json(UserCredentialsResponse {
        provider_config_id: cred.provider_config_id,
        has_credentials: true,
        label: cred.label,
        created_at: Some(cred.created_at.to_rfc3339()),
        updated_at: Some(cred.updated_at.to_rfc3339()),
    }))
}

/// DELETE /api/v1/providers/{provider_id}/credentials
pub async fn delete_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(provider_id): Path<String>,
) -> AppResult<Json<DeleteCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    user_credentials_service::delete_user_credentials(&state.db, &user_id_str, &provider_id)
        .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "user_credentials_deleted",
        Some(serde_json::json!({
            "provider_id": &provider_id,
        })),
    );

    // Intentionally NO `ServiceDisconnected` emit here. See the matching
    // note in `set_my_credentials` above: this endpoint only removes the
    // user's per-user OAuth client credentials, not the actual
    // `UserService`/provider binding. `service.disconnected` is owned by
    // the binding lifecycle in `user_services_handler.rs`.
    let _ = &tele;

    Ok(Json(DeleteCredentialsResponse {
        message: "Credentials deleted successfully".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::test_utils::*;
    use axum::extract::{Path, State};

    fn test_provider(provider_id: &str, credential_mode: &str, is_active: bool) -> ProviderConfig {
        let now = chrono::Utc::now();
        ProviderConfig {
            id: provider_id.to_string(),
            slug: format!("test-{}", &provider_id[..8]),
            name: "Test Provider".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: Some("https://auth.example.com/token".to_string()),
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active,
            credential_mode: credential_mode.to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "admin".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn test_get_my_credentials_none() {
        let Some(db) = connect_test_database("h_user_creds_get_none").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);

        let Json(resp) = get_my_credentials(
            State(state),
            test_auth_user(&user_id),
            Path(provider_id.clone()),
        )
        .await
        .unwrap();

        assert!(!resp.has_credentials);
        assert_eq!(resp.provider_config_id, provider_id);
        assert!(resp.created_at.is_none());
    }

    #[tokio::test]
    async fn test_set_and_get_my_credentials() {
        let Some(db) = connect_test_database("h_user_creds_set_get").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(test_provider(&provider_id, "user", true))
            .await
            .unwrap();

        let state = test_app_state(db);

        let Json(set_resp) = set_my_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id.clone()),
            Json(SetUserCredentialsRequest {
                client_id: "my-client-id".to_string(),
                client_secret: Some("my-secret".to_string()),
                label: Some("My App".to_string()),
            }),
        )
        .await
        .unwrap();

        assert!(set_resp.has_credentials);
        assert_eq!(set_resp.provider_config_id, provider_id);
        assert_eq!(set_resp.label.as_deref(), Some("My App"));

        let Json(get_resp) = get_my_credentials(
            State(state),
            test_auth_user(&user_id),
            Path(provider_id.clone()),
        )
        .await
        .unwrap();

        assert!(get_resp.has_credentials);
        assert_eq!(get_resp.label.as_deref(), Some("My App"));
    }

    #[tokio::test]
    async fn test_delete_my_credentials() {
        let Some(db) = connect_test_database("h_user_creds_delete").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(test_provider(&provider_id, "both", true))
            .await
            .unwrap();

        let state = test_app_state(db);

        let _ = set_my_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id.clone()),
            Json(SetUserCredentialsRequest {
                client_id: "cid".to_string(),
                client_secret: None,
                label: None,
            }),
        )
        .await
        .unwrap();

        let Json(del_resp) = delete_my_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(del_resp.message, "Credentials deleted successfully");

        let Json(after) =
            get_my_credentials(State(state), test_auth_user(&user_id), Path(provider_id))
                .await
                .unwrap();
        assert!(!after.has_credentials);
    }

    #[tokio::test]
    async fn test_set_my_credentials_rejects_inactive_provider() {
        let Some(db) = connect_test_database("h_user_creds_inactive_provider").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(test_provider(&provider_id, "user", false))
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = set_my_credentials(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id),
            Json(SetUserCredentialsRequest {
                client_id: "client-id".to_string(),
                client_secret: None,
                label: None,
            }),
        )
        .await
        .expect_err("inactive provider should be rejected");

        assert!(matches!(
            err,
            AppError::BadRequest(message) if message == "Provider is not active"
        ));
    }

    #[tokio::test]
    async fn test_set_my_credentials_rejects_admin_credential_mode() {
        let Some(db) = connect_test_database("h_user_creds_admin_mode").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(test_provider(&provider_id, "admin", true))
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = set_my_credentials(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id),
            Json(SetUserCredentialsRequest {
                client_id: "client-id".to_string(),
                client_secret: None,
                label: None,
            }),
        )
        .await
        .expect_err("admin-only provider should reject user credentials");

        assert!(matches!(
            err,
            AppError::BadRequest(message)
                if message == "This provider does not accept user-provided credentials"
        ));
    }

    #[tokio::test]
    async fn test_set_my_credentials_rejects_invalid_client_id_and_secret_lengths() {
        let Some(db) = connect_test_database("h_user_creds_validation").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(test_provider(&provider_id, "user", true))
            .await
            .unwrap();

        let state = test_app_state(db);
        let empty_client_id = set_my_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id.clone()),
            Json(SetUserCredentialsRequest {
                client_id: String::new(),
                client_secret: None,
                label: None,
            }),
        )
        .await
        .expect_err("empty client_id should fail validation");
        assert!(matches!(
            empty_client_id,
            AppError::ValidationError(message)
                if message == "client_id must be between 1 and 500 characters"
        ));

        let long_secret = set_my_credentials(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(provider_id),
            Json(SetUserCredentialsRequest {
                client_id: "client-id".to_string(),
                client_secret: Some("s".repeat(2001)),
                label: None,
            }),
        )
        .await
        .expect_err("oversized client_secret should fail validation");
        assert!(matches!(
            long_secret,
            AppError::ValidationError(message)
                if message == "client_secret must be at most 2000 characters"
        ));
    }
}
