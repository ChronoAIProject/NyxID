use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::mfa_service;
use crate::AppState;

// --- Request / Response types ---

#[derive(Debug, Serialize)]
pub struct MfaSetupResponse {
    pub factor_id: String,
    pub secret: String,
    pub qr_code_url: String,
}

#[derive(Debug, Deserialize)]
pub struct MfaVerifySetupRequest {
    pub factor_id: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct MfaVerifySetupResponse {
    pub message: String,
    pub recovery_codes: Vec<String>,
}

// --- Handlers ---

/// POST /api/v1/mfa/setup
///
/// Begin TOTP enrollment. Returns the secret and QR code URL.
pub async fn setup(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<MfaSetupResponse>> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    // Look up user email for the TOTP account name
    let user = state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let result = mfa_service::setup_totp(
        &state.db,
        &encryption_key,
        &user_id_str,
        &user.email,
    )
    .await?;

    Ok(Json(MfaSetupResponse {
        factor_id: result.factor_id,
        secret: result.secret,
        qr_code_url: result.qr_code_url,
    }))
}

/// POST /api/v1/mfa/verify-setup
///
/// Complete TOTP enrollment by verifying a code. Returns recovery codes.
pub async fn verify_setup(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<MfaVerifySetupRequest>,
) -> AppResult<Json<MfaVerifySetupResponse>> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    let recovery_codes = mfa_service::verify_totp_setup(
        &state.db,
        &encryption_key,
        &body.factor_id,
        &user_id_str,
        &body.code,
    )
    .await?;

    // Enable MFA on the user account
    let now = chrono::Utc::now();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .update_one(
            mongodb::bson::doc! { "_id": &user_id_str },
            mongodb::bson::doc! { "$set": {
                "mfa_enabled": true,
                "updated_at": mongodb::bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(Json(MfaVerifySetupResponse {
        message: "MFA enabled successfully. Save your recovery codes.".to_string(),
        recovery_codes,
    }))
}
