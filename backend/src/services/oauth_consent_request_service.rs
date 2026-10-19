use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};

use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::oauth_consent_request::{COLLECTION_NAME, OauthConsentRequest};

pub async fn create(
    db: &mongodb::Database,
    user_id: &str,
    signed_request: &str,
) -> AppResult<String> {
    let handle = generate_random_token();
    db.collection::<OauthConsentRequest>(COLLECTION_NAME)
        .insert_one(OauthConsentRequest {
            id: hash_token(&handle),
            user_id: user_id.to_string(),
            signed_request: signed_request.to_string(),
            expires_at: Utc::now() + Duration::hours(24),
        })
        .await?;
    Ok(handle)
}

pub async fn get(db: &mongodb::Database, user_id: &str, handle: &str) -> AppResult<String> {
    if handle.len() != 64 || !handle.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::NotFound("Consent request not found".to_string()));
    }
    let request = db
        .collection::<OauthConsentRequest>(COLLECTION_NAME)
        .find_one(doc! {
            "_id": hash_token(handle),
            "user_id": user_id,
            "expires_at": { "$gt": bson::DateTime::now() },
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Consent request not found".to_string()))?;
    Ok(request.signed_request)
}
