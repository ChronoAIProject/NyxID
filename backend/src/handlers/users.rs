use axum::{extract::State, Json};
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::AppState;

// --- Request / Response types ---

#[derive(Debug, Serialize)]
pub struct UserProfileResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    pub mfa_enabled: bool,
    pub is_admin: bool,
    pub is_active: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdateProfileResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/users/me
///
/// Returns the profile of the currently authenticated user.
pub async fn get_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserProfileResponse>> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    Ok(Json(UserProfileResponse {
        id: user_model.id,
        email: user_model.email,
        display_name: user_model.display_name,
        avatar_url: user_model.avatar_url,
        email_verified: user_model.email_verified,
        mfa_enabled: user_model.mfa_enabled,
        is_admin: user_model.is_admin,
        is_active: user_model.is_active,
        created_at: user_model.created_at.to_rfc3339(),
        last_login_at: user_model.last_login_at.map(|t| t.to_rfc3339()),
    }))
}

/// PUT /api/v1/users/me
///
/// Update the profile of the currently authenticated user.
pub async fn update_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> AppResult<Json<UpdateProfileResponse>> {
    let user_id = auth_user.user_id.to_string();

    // Verify user exists
    let _existing = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let mut set_doc = doc! {};

    if let Some(ref name) = body.display_name {
        if name.len() > 200 {
            return Err(AppError::ValidationError(
                "Display name must be 200 characters or less".to_string(),
            ));
        }
        set_doc.insert("display_name", name);
    }

    if let Some(ref url) = body.avatar_url {
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Avatar URL must be 2048 characters or less".to_string(),
            ));
        }
        // Validate URL scheme to prevent javascript: and data: URI injection
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(AppError::ValidationError(
                "Avatar URL must use https:// or http:// scheme".to_string(),
            ));
        }
        set_doc.insert("avatar_url", url);
    }

    let now = chrono::Utc::now();
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));

    state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user_id },
            doc! { "$set": set_doc },
        )
        .await?;

    // Re-fetch the updated user
    let updated = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::Internal("User disappeared after update".to_string()))?;

    Ok(Json(UpdateProfileResponse {
        id: updated.id,
        email: updated.email,
        display_name: updated.display_name,
        avatar_url: updated.avatar_url,
        message: "Profile updated successfully".to_string(),
    }))
}
