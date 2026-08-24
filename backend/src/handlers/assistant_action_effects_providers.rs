use axum::{Json, Router, extract::State, routing::post};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::assistant_action_receipt::AssistantActionReceipt;
use crate::models::user_provider_credentials::{
    COLLECTION_NAME as USER_PROVIDER_CREDENTIALS, UserProviderCredentials,
};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_receipts::{
    self, ReceiptOutcome, fingerprint_canonical, fingerprint_sensitive_material, mark_completed,
    normalize_action_request_id,
};
use crate::services::{
    audit_service, connection_service, provider_service, unified_key_service,
    user_credentials_service,
};

const CONNECTION_REVOKE_ACTION: &str = "connection.revoke";
const PROVIDER_DISCONNECT_ACTION: &str = "provider.disconnect";
const PROVIDER_SET_APP_CREDENTIALS_ACTION: &str = "provider.set_app_credentials";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/connection-revoke", post(revoke_connection))
        .route("/provider-disconnect", post(disconnect_provider))
        .route("/set-app-credentials", post(set_provider_app_credentials))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeAssistantConnectionRequest {
    pub action_request_id: String,
    pub service_id: String,
    pub expected_state_version: i64,
    pub confirmed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisconnectAssistantProviderRequest {
    pub action_request_id: String,
    pub provider_id: String,
    pub expected_state_version: i64,
    pub confirmed: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAssistantProviderAppCredentialsRequest {
    pub action_request_id: String,
    pub provider_id: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub expected_state_version: i64,
}

impl std::fmt::Debug for SetAssistantProviderAppCredentialsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetAssistantProviderAppCredentialsRequest")
            .field("action_request_id", &self.action_request_id)
            .field("provider_id", &self.provider_id)
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("expected_state_version", &self.expected_state_version)
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConnectionResource {
    pub service_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantProviderResource {
    pub provider_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantConnectionEffectResponse {
    pub resource: AssistantConnectionResource,
    pub replayed: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantProviderEffectResponse {
    pub resource: AssistantProviderResource,
    pub replayed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionRevokeFingerprint<'a> {
    action: &'static str,
    service_id: &'a str,
    expected_state_version: i64,
    confirmed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderDisconnectFingerprint<'a> {
    action: &'static str,
    provider_id: &'a str,
    expected_state_version: i64,
    confirmed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAppCredentialsFingerprint<'a> {
    action: &'static str,
    provider_id: &'a str,
    client_id_fingerprint: String,
    client_secret_fingerprint: Option<String>,
    expected_state_version: i64,
}

async fn release_receipt(
    db: &mongodb::Database,
    receipt: &AssistantActionReceipt,
) -> AppResult<()> {
    db.collection::<AssistantActionReceipt>(
        crate::models::assistant_action_receipt::COLLECTION_NAME,
    )
    .delete_one(doc! { "_id": &receipt.id })
    .await?;
    Ok(())
}

fn validate_state_version(value: i64) -> AppResult<i64> {
    if value < 0 {
        return Err(AppError::ValidationError(
            "expectedStateVersion must be non-negative".to_string(),
        ));
    }
    Ok(value)
}

fn connection_response(service_id: String, replayed: bool) -> AssistantConnectionEffectResponse {
    AssistantConnectionEffectResponse {
        resource: AssistantConnectionResource { service_id },
        replayed,
    }
}

fn provider_response(provider_id: String, replayed: bool) -> AssistantProviderEffectResponse {
    AssistantProviderEffectResponse {
        resource: AssistantProviderResource { provider_id },
        replayed,
    }
}

async fn load_connection(
    state: &AppState,
    user_id: &str,
    service_id: &str,
) -> AppResult<UserServiceConnection> {
    state
        .db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! { "user_id": user_id, "service_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Connection not found".to_string()))
}

async fn load_provider_token(
    state: &AppState,
    user_id: &str,
    provider_id: &str,
) -> AppResult<UserProviderToken> {
    state
        .db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .sort(doc! { "updated_at": -1_i32 })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider token not found".to_string()))
}

async fn credentials_match(
    state: &AppState,
    credentials: &UserProviderCredentials,
    client_id: &str,
    client_secret: Option<&str>,
) -> AppResult<bool> {
    let Some(encrypted_client_id) = credentials.client_id_encrypted.as_deref() else {
        return Ok(false);
    };
    if state.encryption_keys.decrypt(encrypted_client_id).await? != client_id.as_bytes() {
        return Ok(false);
    }
    match (
        credentials.client_secret_encrypted.as_deref(),
        client_secret,
    ) {
        (None, None) => Ok(true),
        (Some(encrypted), Some(expected)) => {
            Ok(state.encryption_keys.decrypt(encrypted).await? == expected.as_bytes())
        }
        _ => Ok(false),
    }
}

async fn commit_connection_revoke(
    state: &AppState,
    auth_user: &AuthUser,
    user_id: &str,
    service_id: &str,
    expected_state_version: i64,
    receipt: AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let current = match load_connection(state, user_id, service_id).await {
        Ok(current) => current,
        Err(error) => {
            if !was_in_progress {
                release_receipt(&state.db, &receipt).await?;
            }
            return Err(error);
        }
    };
    if was_in_progress && !current.is_active && current.state_version == expected_state_version + 1
    {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    if !current.is_active {
        if !was_in_progress {
            release_receipt(&state.db, &receipt).await?;
        }
        return Err(AppError::NotFound("Connection not found".to_string()));
    }
    match connection_service::disconnect_user_with_expected_state_version(
        &state.db,
        user_id,
        service_id,
        Some(expected_state_version),
    )
    .await
    {
        Ok(_) => {
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "assistant_connection_revoked",
                Some(serde_json::json!({ "service_id": service_id })),
            );
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error @ (AppError::NotFound(_) | AppError::Conflict(_))) => {
            if !was_in_progress {
                release_receipt(&state.db, &receipt).await?;
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

async fn commit_provider_disconnect(
    state: &AppState,
    auth_user: &AuthUser,
    user_id: &str,
    provider_id: &str,
    expected_state_version: i64,
    receipt: AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let current = match load_provider_token(state, user_id, provider_id).await {
        Ok(current) => current,
        Err(error) => {
            if !was_in_progress {
                release_receipt(&state.db, &receipt).await?;
            }
            return Err(error);
        }
    };
    if was_in_progress
        && current.status == "revoked"
        && current.state_version == expected_state_version + 1
    {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    if current.status == "revoked" {
        if !was_in_progress {
            release_receipt(&state.db, &receipt).await?;
        }
        return Err(AppError::NotFound(
            "No active token found for this provider".to_string(),
        ));
    }
    let actor = audit_service::AuditActor::from_auth_user(auth_user);
    let options = unified_key_service::DisconnectOptions {
        cascade_grant: false,
        grant_scope: Some("token".to_string()),
    };
    match unified_key_service::disconnect_credentials(
        &state.db,
        &state.encryption_keys,
        user_id,
        &actor,
        unified_key_service::DisconnectTarget::ProviderWithExpectedStateVersion(
            provider_id,
            expected_state_version,
        ),
        options,
    )
    .await
    {
        Ok(_) => {
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "assistant_provider_disconnected",
                Some(serde_json::json!({ "provider_id": provider_id })),
            );
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error @ (AppError::NotFound(_) | AppError::Conflict(_))) => {
            if !was_in_progress {
                release_receipt(&state.db, &receipt).await?;
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn commit_provider_app_credentials(
    state: &AppState,
    auth_user: &AuthUser,
    user_id: &str,
    provider_id: &str,
    client_id: &str,
    client_secret: Option<&str>,
    expected_state_version: i64,
    receipt: AssistantActionReceipt,
    was_in_progress: bool,
) -> AppResult<bool> {
    let provider = match provider_service::get_provider(&state.db, provider_id).await {
        Ok(provider) => provider,
        Err(error) => {
            if !was_in_progress {
                release_receipt(&state.db, &receipt).await?;
            }
            return Err(error);
        }
    };
    if !provider.is_active || !user_credentials_service::supports_user_credentials(&provider) {
        if !was_in_progress {
            release_receipt(&state.db, &receipt).await?;
        }
        return Err(AppError::BadRequest(if provider.is_active {
            "This provider does not accept user-provided credentials".to_string()
        } else {
            "Provider is not active".to_string()
        }));
    }
    let current = state
        .db
        .collection::<UserProviderCredentials>(USER_PROVIDER_CREDENTIALS)
        .find_one(doc! { "user_id": user_id, "provider_config_id": provider_id })
        .await?;
    if was_in_progress
        && let Some(credentials) = current.as_ref()
        && credentials.state_version == expected_state_version + 1
        && credentials_match(state, credentials, client_id, client_secret).await?
    {
        mark_completed(&state.db, &receipt).await?;
        return Ok(true);
    }
    if current
        .as_ref()
        .map_or(0, |credentials| credentials.state_version)
        != expected_state_version
    {
        if !was_in_progress {
            release_receipt(&state.db, &receipt).await?;
        }
        return Err(AppError::Conflict(
            "the provider credentials changed since this action was prepared".to_string(),
        ));
    }
    match user_credentials_service::upsert_user_credentials_with_expected_state_version(
        &state.db,
        &state.encryption_keys,
        user_id,
        provider_id,
        client_id,
        client_secret,
        None,
        Some(expected_state_version),
    )
    .await
    {
        Ok(_) => {
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "assistant_provider_app_credentials_set",
                Some(serde_json::json!({ "provider_id": provider_id })),
            );
            mark_completed(&state.db, &receipt).await?;
            Ok(false)
        }
        Err(error @ (AppError::NotFound(_) | AppError::Conflict(_))) => {
            if !was_in_progress {
                release_receipt(&state.db, &receipt).await?;
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

pub async fn revoke_connection(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RevokeAssistantConnectionRequest>,
) -> AppResult<Json<AssistantConnectionEffectResponse>> {
    auth_user.ensure_write_scope()?;
    if !body.confirmed {
        return Err(AppError::BadRequest(
            "connection.revoke requires confirmation".to_string(),
        ));
    }
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let service_id = normalize_action_request_id(body.service_id)?;
    let expected_state_version = validate_state_version(body.expected_state_version)?;
    let user_id = auth_user.user_id.to_string();
    let fingerprint = fingerprint_canonical(&ConnectionRevokeFingerprint {
        action: CONNECTION_REVOKE_ACTION,
        service_id: &service_id,
        expected_state_version,
        confirmed: body.confirmed,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &user_id,
        CONNECTION_REVOKE_ACTION,
        &action_request_id,
        &fingerprint,
        service_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(connection_response(receipt.resource_id, true))),
        ReceiptOutcome::Reserved(receipt) => Ok(Json(connection_response(
            service_id.clone(),
            commit_connection_revoke(
                &state,
                &auth_user,
                &user_id,
                &service_id,
                expected_state_version,
                receipt,
                false,
            )
            .await?,
        ))),
        ReceiptOutcome::InProgress(receipt) => Ok(Json(connection_response(
            service_id.clone(),
            commit_connection_revoke(
                &state,
                &auth_user,
                &user_id,
                &service_id,
                expected_state_version,
                receipt,
                true,
            )
            .await?,
        ))),
    }
}

pub async fn disconnect_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<DisconnectAssistantProviderRequest>,
) -> AppResult<Json<AssistantProviderEffectResponse>> {
    auth_user.ensure_write_scope()?;
    if !body.confirmed {
        return Err(AppError::BadRequest(
            "provider.disconnect requires confirmation".to_string(),
        ));
    }
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let provider_id = normalize_action_request_id(body.provider_id)?;
    let expected_state_version = validate_state_version(body.expected_state_version)?;
    let user_id = auth_user.user_id.to_string();
    let fingerprint = fingerprint_canonical(&ProviderDisconnectFingerprint {
        action: PROVIDER_DISCONNECT_ACTION,
        provider_id: &provider_id,
        expected_state_version,
        confirmed: body.confirmed,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &user_id,
        PROVIDER_DISCONNECT_ACTION,
        &action_request_id,
        &fingerprint,
        provider_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(provider_response(receipt.resource_id, true))),
        ReceiptOutcome::Reserved(receipt) => Ok(Json(provider_response(
            provider_id.clone(),
            commit_provider_disconnect(
                &state,
                &auth_user,
                &user_id,
                &provider_id,
                expected_state_version,
                receipt,
                false,
            )
            .await?,
        ))),
        ReceiptOutcome::InProgress(receipt) => Ok(Json(provider_response(
            provider_id.clone(),
            commit_provider_disconnect(
                &state,
                &auth_user,
                &user_id,
                &provider_id,
                expected_state_version,
                receipt,
                true,
            )
            .await?,
        ))),
    }
}

pub async fn set_provider_app_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<SetAssistantProviderAppCredentialsRequest>,
) -> AppResult<Json<AssistantProviderEffectResponse>> {
    auth_user.ensure_write_scope()?;
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let provider_id = normalize_action_request_id(body.provider_id)?;
    let expected_state_version = validate_state_version(body.expected_state_version)?;
    if body.client_id.is_empty() || body.client_id.len() > 500 {
        return Err(AppError::ValidationError(
            "clientId must be between 1 and 500 characters".to_string(),
        ));
    }
    if body
        .client_secret
        .as_ref()
        .is_some_and(|value| value.len() > 2000)
    {
        return Err(AppError::ValidationError(
            "clientSecret must be at most 2000 characters".to_string(),
        ));
    }
    let client_secret = body
        .client_secret
        .as_deref()
        .filter(|value| !value.is_empty());
    let user_id = auth_user.user_id.to_string();
    let fingerprint = fingerprint_canonical(&ProviderAppCredentialsFingerprint {
        action: PROVIDER_SET_APP_CREDENTIALS_ACTION,
        provider_id: &provider_id,
        client_id_fingerprint: fingerprint_sensitive_material(&body.client_id),
        client_secret_fingerprint: client_secret.map(fingerprint_sensitive_material),
        expected_state_version,
    })?;
    match assistant_action_receipts::reserve_or_replay(
        &state.db,
        &user_id,
        PROVIDER_SET_APP_CREDENTIALS_ACTION,
        &action_request_id,
        &fingerprint,
        provider_id.clone(),
    )
    .await?
    {
        ReceiptOutcome::Replay(receipt) => Ok(Json(provider_response(receipt.resource_id, true))),
        ReceiptOutcome::Reserved(receipt) => Ok(Json(provider_response(
            provider_id.clone(),
            commit_provider_app_credentials(
                &state,
                &auth_user,
                &user_id,
                &provider_id,
                &body.client_id,
                client_secret,
                expected_state_version,
                receipt,
                false,
            )
            .await?,
        ))),
        ReceiptOutcome::InProgress(receipt) => Ok(Json(provider_response(
            provider_id.clone(),
            commit_provider_app_credentials(
                &state,
                &auth_user,
                &user_id,
                &provider_id,
                &body.client_id,
                client_secret,
                expected_state_version,
                receipt,
                true,
            )
            .await?,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::{IndexModel, bson::Bson, options::IndexOptions};
    use uuid::Uuid;

    use crate::models::assistant_action_receipt::{
        AssistantActionReceipt, COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
    };
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOGS};
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};

    async fn prepare_database(prefix: &str) -> Option<(mongodb::Database, String)> {
        let db = connect_test_database(prefix).await?;
        db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "action": 1, "action_request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create receipt uniqueness index");
        Some((db, Uuid::new_v4().to_string()))
    }

    fn connection(user_id: &str, service_id: &str) -> UserServiceConnection {
        let now = chrono::Utc::now();
        UserServiceConnection {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            credential_encrypted: Some(vec![1, 2, 3]),
            credential_type: Some("api_key".to_string()),
            credential_label: None,
            metadata: None,
            is_active: true,
            state_version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    fn provider_token(user_id: &str, provider_id: &str) -> UserProviderToken {
        let now = chrono::Utc::now();
        UserProviderToken {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: Some(vec![4, 5, 6]),
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            state_version: 1,
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn provider(provider_id: &str) -> ProviderConfig {
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
            revocation: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "admin".to_string(),
            revocation_seed_version: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn reopen_receipt(db: &mongodb::Database, action: &str, request_id: &str) {
        db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .update_one(
                doc! { "action": action, "action_request_id": request_id },
                doc! { "$set": { "status": "pending", "completed_at": Bson::Null } },
            )
            .await
            .expect("reopen receipt after simulated crash");
    }

    #[tokio::test]
    async fn connection_revoke_happy_path_and_exact_replay() {
        let Some((db, user_id)) = prepare_database("assistant_connection_revoke_replay").await
        else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .insert_one(connection(&user_id, &service_id))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let request = || RevokeAssistantConnectionRequest {
            action_request_id: "connection-revoke-1".to_string(),
            service_id: service_id.clone(),
            expected_state_version: 1,
            confirmed: true,
        };

        let Json(first) = revoke_connection(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request()),
        )
        .await
        .unwrap();
        let Json(replay) =
            revoke_connection(State(state), test_auth_user(&user_id), Json(request()))
                .await
                .unwrap();
        assert!(!first.replayed);
        assert!(replay.replayed);
        assert_eq!(replay.resource.service_id, service_id);
        let stored = db
            .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .find_one(doc! { "service_id": &service_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_active);
        assert_eq!(stored.state_version, 2);
    }

    #[tokio::test]
    async fn connection_revoke_recovers_pending_without_reapplying() {
        let Some((db, user_id)) = prepare_database("assistant_connection_revoke_pending").await
        else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .insert_one(connection(&user_id, &service_id))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let request = || RevokeAssistantConnectionRequest {
            action_request_id: "connection-pending".to_string(),
            service_id: service_id.clone(),
            expected_state_version: 1,
            confirmed: true,
        };
        let _ = revoke_connection(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request()),
        )
        .await
        .unwrap();
        reopen_receipt(&db, CONNECTION_REVOKE_ACTION, "connection-pending").await;
        let Json(recovered) =
            revoke_connection(State(state), test_auth_user(&user_id), Json(request()))
                .await
                .unwrap();
        assert!(recovered.replayed);
        let stored = db
            .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .find_one(doc! { "service_id": &service_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.state_version, 2);
    }

    #[tokio::test]
    async fn connection_revoke_fingerprint_mismatch_conflicts() {
        let Some((db, user_id)) = prepare_database("assistant_connection_revoke_mismatch").await
        else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .insert_one(connection(&user_id, &service_id))
            .await
            .unwrap();
        let state = test_app_state(db);
        let request = |version| RevokeAssistantConnectionRequest {
            action_request_id: "connection-mismatch".to_string(),
            service_id: service_id.clone(),
            expected_state_version: version,
            confirmed: true,
        };
        let _ = revoke_connection(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request(1)),
        )
        .await
        .unwrap();
        assert!(matches!(
            revoke_connection(State(state), test_auth_user(&user_id), Json(request(2)),).await,
            Err(AppError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn provider_disconnect_happy_path_and_exact_replay() {
        let Some((db, user_id)) = prepare_database("assistant_provider_disconnect_replay").await
        else {
            return;
        };
        let provider_id = Uuid::new_v4().to_string();
        let token = provider_token(&user_id, &provider_id);
        let token_id = token.id.clone();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(token)
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let request = || DisconnectAssistantProviderRequest {
            action_request_id: "provider-disconnect-1".to_string(),
            provider_id: provider_id.clone(),
            expected_state_version: 1,
            confirmed: true,
        };
        let Json(first) = disconnect_provider(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request()),
        )
        .await
        .unwrap();
        let Json(replay) =
            disconnect_provider(State(state), test_auth_user(&user_id), Json(request()))
                .await
                .unwrap();
        assert!(!first.replayed);
        assert!(replay.replayed);
        assert_eq!(replay.resource.provider_id, provider_id);
        let stored = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": token_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "revoked");
        assert_eq!(stored.state_version, 2);
        assert!(stored.access_token_encrypted.is_none());
    }

    #[tokio::test]
    async fn provider_disconnect_recovers_pending_without_reapplying() {
        let Some((db, user_id)) = prepare_database("assistant_provider_disconnect_pending").await
        else {
            return;
        };
        let provider_id = Uuid::new_v4().to_string();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(provider_token(&user_id, &provider_id))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let request = || DisconnectAssistantProviderRequest {
            action_request_id: "provider-pending".to_string(),
            provider_id: provider_id.clone(),
            expected_state_version: 1,
            confirmed: true,
        };
        let _ = disconnect_provider(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request()),
        )
        .await
        .unwrap();
        reopen_receipt(&db, PROVIDER_DISCONNECT_ACTION, "provider-pending").await;
        let Json(recovered) =
            disconnect_provider(State(state), test_auth_user(&user_id), Json(request()))
                .await
                .unwrap();
        assert!(recovered.replayed);
        let stored = load_provider_token(&test_app_state(db), &user_id, &provider_id)
            .await
            .unwrap();
        assert_eq!(stored.state_version, 2);
    }

    #[tokio::test]
    async fn provider_disconnect_fingerprint_mismatch_conflicts() {
        let Some((db, user_id)) = prepare_database("assistant_provider_disconnect_mismatch").await
        else {
            return;
        };
        let provider_id = Uuid::new_v4().to_string();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(provider_token(&user_id, &provider_id))
            .await
            .unwrap();
        let state = test_app_state(db);
        let request = |version| DisconnectAssistantProviderRequest {
            action_request_id: "provider-mismatch".to_string(),
            provider_id: provider_id.clone(),
            expected_state_version: version,
            confirmed: true,
        };
        let _ = disconnect_provider(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request(1)),
        )
        .await
        .unwrap();
        assert!(matches!(
            disconnect_provider(State(state), test_auth_user(&user_id), Json(request(2)),).await,
            Err(AppError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn provider_set_app_credentials_happy_replay_and_secret_free_artifacts() {
        let Some((db, user_id)) = prepare_database("assistant_provider_credentials_replay").await
        else {
            return;
        };
        let provider_id = Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(provider(&provider_id))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let client_id = "browser-client-id";
        let secret = "browser-client-secret";
        let request = || SetAssistantProviderAppCredentialsRequest {
            action_request_id: "provider-credentials-1".to_string(),
            provider_id: provider_id.clone(),
            client_id: client_id.to_string(),
            client_secret: Some(secret.to_string()),
            expected_state_version: 0,
        };
        let Json(first) = set_provider_app_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request()),
        )
        .await
        .unwrap();
        let response_json = serde_json::to_string(&first).unwrap();
        assert!(!first.replayed);
        assert!(!response_json.contains(secret));
        assert!(!response_json.contains(client_id));
        let Json(replay) =
            set_provider_app_credentials(State(state), test_auth_user(&user_id), Json(request()))
                .await
                .unwrap();
        assert!(replay.replayed);

        let receipt = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! {
                "action": PROVIDER_SET_APP_CREDENTIALS_ACTION,
                "action_request_id": "provider-credentials-1",
            })
            .await
            .unwrap()
            .unwrap();
        let receipt_json = serde_json::to_string(&receipt).unwrap();
        assert!(!receipt_json.contains(secret));
        assert!(!receipt_json.contains(client_id));

        let mut audit = None;
        for _ in 0..200 {
            audit = db
                .collection::<AuditLog>(AUDIT_LOGS)
                .find_one(doc! {
                    "user_id": &user_id,
                    "event_type": "assistant_provider_app_credentials_set",
                })
                .await
                .unwrap();
            if audit.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let audit_json = serde_json::to_string(&audit.expect("assistant audit event")).unwrap();
        assert!(!audit_json.contains(secret));
        assert!(!audit_json.contains(client_id));
    }

    #[tokio::test]
    async fn provider_set_app_credentials_recovers_pending_without_reapplying() {
        let Some((db, user_id)) = prepare_database("assistant_provider_credentials_pending").await
        else {
            return;
        };
        let provider_id = Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(provider(&provider_id))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let request = || SetAssistantProviderAppCredentialsRequest {
            action_request_id: "credentials-pending".to_string(),
            provider_id: provider_id.clone(),
            client_id: "client-id".to_string(),
            client_secret: Some("client-secret".to_string()),
            expected_state_version: 0,
        };
        let _ = set_provider_app_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request()),
        )
        .await
        .unwrap();
        reopen_receipt(
            &db,
            PROVIDER_SET_APP_CREDENTIALS_ACTION,
            "credentials-pending",
        )
        .await;
        let Json(recovered) =
            set_provider_app_credentials(State(state), test_auth_user(&user_id), Json(request()))
                .await
                .unwrap();
        assert!(recovered.replayed);
        let stored = db
            .collection::<UserProviderCredentials>(USER_PROVIDER_CREDENTIALS)
            .find_one(doc! { "user_id": &user_id, "provider_config_id": &provider_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.state_version, 1);
    }

    #[tokio::test]
    async fn provider_set_app_credentials_fingerprint_mismatch_conflicts() {
        let Some((db, user_id)) = prepare_database("assistant_provider_credentials_mismatch").await
        else {
            return;
        };
        let provider_id = Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(provider(&provider_id))
            .await
            .unwrap();
        let state = test_app_state(db);
        let request = |secret: &str| SetAssistantProviderAppCredentialsRequest {
            action_request_id: "credentials-mismatch".to_string(),
            provider_id: provider_id.clone(),
            client_id: "client-id".to_string(),
            client_secret: Some(secret.to_string()),
            expected_state_version: 0,
        };
        let _ = set_provider_app_credentials(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(request("secret-one")),
        )
        .await
        .unwrap();
        assert!(matches!(
            set_provider_app_credentials(
                State(state),
                test_auth_user(&user_id),
                Json(request("secret-two")),
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
}
