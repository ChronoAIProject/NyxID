use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};

use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME, ServiceViewPreferences, User};

pub async fn save_services_view(
    db: &Database,
    user_id: &str,
    mut preferences: ServiceViewPreferences,
) -> AppResult<ServiceViewPreferences> {
    for (name, values) in [
        ("Organization", &mut preferences.organization_ids),
        ("Service", &mut preferences.service_group_ids),
    ] {
        if values.len() > 100 {
            return Err(AppError::ValidationError(format!(
                "{name} filter is limited to 100 selections"
            )));
        }
        if values
            .iter()
            .any(|value| value.trim().is_empty() || value.chars().count() > 128)
        {
            return Err(AppError::ValidationError(format!(
                "{name} filter must contain 1 to 128 characters per selection"
            )));
        }
        values.sort();
        values.dedup();
    }
    if preferences.search.chars().count() > 200 {
        return Err(AppError::ValidationError(
            "Service search must be 200 characters or less".to_string(),
        ));
    }
    let result = db.collection::<User>(COLLECTION_NAME)
        .update_one(doc! { "_id": user_id }, doc! { "$set": {
            "profile_config.services_view": bson::to_bson(&preferences)
                .map_err(|_| AppError::Internal("Could not encode service view preferences".to_string()))?,
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        } })
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("User not found".to_string()));
    }
    Ok(preferences)
}
