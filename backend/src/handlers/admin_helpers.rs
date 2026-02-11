use axum::http::{header, HeaderMap};
use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::mw::auth::AuthUser;
use crate::AppState;

/// Check that the authenticated user has admin privileges.
///
/// Admin access is determined by the `is_admin` flag on the user record.
/// This is the canonical admin check. The "admin" RBAC role is informational
/// and used for claim injection into tokens; it does not replace this flag.
pub async fn require_admin(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();
    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    Ok(())
}

pub fn extract_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Validate that a slug matches the required format: lowercase alphanumeric,
/// hyphens, and underscores only.
pub fn validate_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(AppError::ValidationError(
            "Slug must contain only lowercase alphanumeric characters, hyphens, or underscores"
                .to_string(),
        ));
    }
    Ok(())
}
