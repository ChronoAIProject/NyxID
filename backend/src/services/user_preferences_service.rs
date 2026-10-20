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
    validate_filters(&mut preferences)?;
    let filters = bson::to_bson(&preferences)
        .map_err(|_| AppError::Internal("Could not encode service view preferences".into()))?;
    let result = db.collection::<User>(COLLECTION_NAME)
        .update_one(doc! { "_id": user_id }, vec![doc! { "$set": {
            "profile_config.services_view": { "$literal": filters.clone() },
            "profile_config.service_views": { "$cond": [
                { "$eq": [{ "$type": "$profile_config.service_views" }, "object"] },
                { "$mergeObjects": ["$profile_config.service_views", { "views": {
                    "$map": { "input": "$profile_config.service_views.views", "as": "view", "in": {
                        "$cond": [
                            { "$eq": ["$$view.id", "$profile_config.service_views.default_id"] },
                            { "$mergeObjects": ["$$view", { "filters": { "$literal": filters } }] },
                            "$$view"
                        ]
                    } }
                } }] },
                "$$REMOVE"
            ] },
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        } }])
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("User not found".to_string()));
    }
    Ok(preferences)
}

fn validate_filters(preferences: &mut ServiceViewPreferences) -> AppResult<()> {
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
    Ok(())
}

pub async fn save_service_views(
    db: &Database,
    user_id: &str,
    mut preferences: crate::models::user::ServiceViewsPreferences,
) -> AppResult<crate::models::user::ServiceViewsPreferences> {
    let default = validate_views(&mut preferences)?;
    let result = db
        .collection::<User>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": user_id },
            doc! { "$set": {
                "profile_config.service_views": bson::to_bson(&preferences)
                    .map_err(|_| AppError::Internal("Could not encode saved views".into()))?,
                "profile_config.services_view": bson::to_bson(&default)
                    .map_err(|_| AppError::Internal("Could not encode default view".into()))?,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }
    Ok(preferences)
}

fn validate_views(
    preferences: &mut crate::models::user::ServiceViewsPreferences,
) -> AppResult<Option<ServiceViewPreferences>> {
    if preferences.views.len() > 20 {
        return Err(AppError::ValidationError(
            "Save up to 20 service views".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for view in &mut preferences.views {
        view.name = view.name.trim().to_string();
        if view.name.is_empty()
            || view.name.chars().count() > 100
            || view.id.trim().is_empty()
            || view.id.chars().count() > 128
            || !ids.insert(view.id.clone())
        {
            return Err(AppError::ValidationError(
                "Views require unique IDs and names of 1 to 100 characters".into(),
            ));
        }
        validate_filters(&mut view.filters)?;
    }
    let default = match preferences.default_id.as_ref() {
        Some(id) => Some(
            preferences
                .views
                .iter()
                .find(|view| &view.id == id)
                .ok_or_else(|| {
                    AppError::ValidationError("Default view must be a saved view".into())
                })?
                .filters
                .clone(),
        ),
        None => None,
    };
    Ok(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{SavedServiceView, ServiceViewsPreferences};

    fn workspace() -> ServiceViewsPreferences {
        serde_json::from_value(serde_json::json!({
            "views": [{ "id": "one", "name": " First ", "filters": {
                "search": "", "organization_ids": ["b", "a", "a"],
                "service_group_ids": [], "source": "personal", "state": "all",
                "service_type": "all", "show_auto_connected": false
            }}], "default_id": "one"
        }))
        .unwrap()
    }

    #[test]
    fn validates_normalizes_and_resolves_default() {
        let mut preferences = workspace();
        let default = validate_views(&mut preferences).unwrap().unwrap();
        assert_eq!(preferences.views[0].name, "First");
        assert_eq!(default.organization_ids, vec!["a", "b"]);
        preferences.default_id = None;
        assert!(validate_views(&mut preferences).unwrap().is_none());
    }

    #[test]
    fn rejects_invalid_views_and_default_references() {
        let mut preferences = workspace();
        preferences.default_id = Some("missing".into());
        assert!(validate_views(&mut preferences).is_err());
        preferences = workspace();
        preferences.views.push(preferences.views[0].clone());
        assert!(validate_views(&mut preferences).is_err());
        for (name, search) in [
            (" ".into(), "".into()),
            ("x".repeat(101), "".into()),
            ("Valid".into(), "x".repeat(201)),
        ] {
            preferences = workspace();
            preferences.views[0].name = name;
            preferences.views[0].filters.search = search;
            assert!(validate_views(&mut preferences).is_err());
        }
        preferences = workspace();
        let template = preferences.views[0].clone();
        preferences.views = (0..21)
            .map(|index| SavedServiceView {
                id: index.to_string(),
                ..template.clone()
            })
            .collect();
        preferences.default_id = None;
        assert!(validate_views(&mut preferences).is_err());
    }
}
