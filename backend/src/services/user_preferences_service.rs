use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};

use crate::errors::{AppError, AppResult};
use crate::models::user::{
    COLLECTION_NAME, QUICK_ACCESS_DEFAULT, QUICK_ACCESS_MAX_ITEMS, QuickAccessDestination,
    QuickAccessItem, QuickAccessPreferences, ServiceViewPreferences, User,
};

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

/// The quick-access list everyone sees until they customise it.
pub fn default_quick_access() -> Vec<QuickAccessItem> {
    QUICK_ACCESS_DEFAULT
        .into_iter()
        .map(|id| QuickAccessItem::Destination { id })
        .collect()
}

/// One requested entry as it arrives: a kind and an id, both still untrusted.
pub struct QuickAccessEntry<'a> {
    pub kind: &'a str,
    pub id: &'a str,
}

/// Turn requested entries into typed items: at most 12; a destination must be
/// one of the closed list, an agent its UUID and a crew its `nyxg-` id (both
/// stored in canonical lowercase form); no entry may appear twice. Anything
/// else is a 400.
pub fn parse_quick_access(entries: &[QuickAccessEntry<'_>]) -> AppResult<Vec<QuickAccessItem>> {
    if entries.len() > QUICK_ACCESS_MAX_ITEMS {
        return Err(AppError::ValidationError(format!(
            "Quick access holds at most {QUICK_ACCESS_MAX_ITEMS} items"
        )));
    }
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let item = match entry.kind {
            "destination" => QuickAccessDestination::parse(entry.id)
                .map(|id| QuickAccessItem::Destination { id })
                .ok_or_else(|| {
                    AppError::ValidationError("Unknown quick access destination".into())
                })?,
            "agent" => QuickAccessItem::Agent {
                id: uuid::Uuid::parse_str(entry.id)
                    .map_err(|_| {
                        AppError::ValidationError(
                            "Quick access agents are identified by their UUID".into(),
                        )
                    })?
                    .to_string(),
            },
            // Crews (assistant groups) carry `nyxg-` ids, not UUIDs.
            "crew" | "group" if crate::services::assistant_group_service::valid_id(entry.id) => {
                QuickAccessItem::Crew {
                    id: entry.id.to_ascii_lowercase(),
                }
            }
            "crew" | "group" => {
                return Err(AppError::ValidationError(
                    "Quick access crews are identified by their crew ID".into(),
                ));
            }
            _ => {
                return Err(AppError::ValidationError(
                    "Unknown quick access item kind".into(),
                ));
            }
        };
        if items.contains(&item) {
            return Err(AppError::ValidationError(
                "Quick access items must be unique".into(),
            ));
        }
        items.push(item);
    }
    Ok(items)
}

struct QuickAccessSnapshot {
    preferences: Option<QuickAccessPreferences>,
    has_unsupported_entries: bool,
    raw: Option<bson::Bson>,
}

async fn quick_access_snapshot(db: &Database, user_id: &str) -> AppResult<QuickAccessSnapshot> {
    // Typed User deserialization deliberately drops unknown entries. Read the
    // narrow raw preference so GET can report that loss and PUT can fence it.
    let user = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! { "_id": user_id })
        .projection(doc! { "user_type": 1, "profile_config.quick_access": 1 })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if user
        .get("user_type")
        .is_some_and(|kind| kind.as_str() != Some("person"))
    {
        return Err(AppError::Forbidden(
            "Quick access requires a person account".into(),
        ));
    }
    let raw = user
        .get_document("profile_config")
        .ok()
        .and_then(|config| config.get("quick_access"))
        .cloned();
    let has_unsupported_entries = raw
        .as_ref()
        .and_then(bson::Bson::as_document)
        .and_then(|value| value.get_array("items").ok())
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| bson::from_bson::<QuickAccessItem>(item.clone()).is_err())
        });
    let preferences = match raw.clone() {
        None | Some(bson::Bson::Null) => None,
        Some(value) => Some(
            bson::from_bson(value)
                .map_err(|_| AppError::Internal("Could not decode quick access".into()))?,
        ),
    };
    Ok(QuickAccessSnapshot {
        preferences,
        has_unsupported_entries,
        raw,
    })
}

/// The saved list, or `None` when the person never customised it.
pub async fn read_quick_access(
    db: &Database,
    user_id: &str,
) -> AppResult<Option<QuickAccessPreferences>> {
    Ok(quick_access_snapshot(db, user_id).await?.preferences)
}

/// Additive compatibility metadata is computed before tolerant model decoding.
pub async fn read_quick_access_with_compatibility(
    db: &Database,
    user_id: &str,
) -> AppResult<(Option<QuickAccessPreferences>, bool)> {
    let snapshot = quick_access_snapshot(db, user_id).await?;
    Ok((snapshot.preferences, snapshot.has_unsupported_entries))
}

fn quick_access_person_filter(user_id: &str) -> bson::Document {
    // Older person records predate user_type. Keep them compatible while
    // fencing writes against organization subjects, including type changes.
    doc! { "_id": user_id, "$or": [
        { "user_type": "person" }, { "user_type": { "$exists": false } }
    ] }
}

pub async fn save_quick_access(
    db: &Database,
    user_id: &str,
    items: Vec<QuickAccessItem>,
) -> AppResult<QuickAccessPreferences> {
    let snapshot = quick_access_snapshot(db, user_id).await?;
    if snapshot.has_unsupported_entries {
        return Err(AppError::Conflict("Quick access contains unsupported entries. Update the app or explicitly reset the list.".into()));
    }
    let mut filter = quick_access_person_filter(user_id);
    // Match the entire raw preference to close the read/write race with a
    // newer binary adding unknown entries. Null also matches an absent field.
    filter.insert(
        "profile_config.quick_access",
        snapshot.raw.unwrap_or(bson::Bson::Null),
    );
    let preferences = QuickAccessPreferences { items };
    let result = db
        .collection::<User>(COLLECTION_NAME)
        .update_one(
            filter,
            doc! { "$set": {
                "profile_config.quick_access": bson::to_bson(&preferences)
                    .map_err(|_| AppError::Internal("Could not encode quick access".into()))?,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;
    if result.matched_count == 0 {
        read_quick_access(db, user_id).await?;
        return Err(AppError::Conflict(
            "Quick access changed. Reload the list before editing.".into(),
        ));
    }
    Ok(preferences)
}

/// Back to the default list (and to any future change of the default).
pub async fn reset_quick_access(db: &Database, user_id: &str) -> AppResult<()> {
    let result = db
        .collection::<User>(COLLECTION_NAME)
        .update_one(
            quick_access_person_filter(user_id),
            doc! {
                "$unset": { "profile_config.quick_access": "" },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    if result.matched_count == 0 {
        read_quick_access(db, user_id).await?;
        return Err(AppError::NotFound("User not found".into()));
    }
    Ok(())
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

    fn entry<'a>(kind: &'a str, id: &'a str) -> QuickAccessEntry<'a> {
        QuickAccessEntry { kind, id }
    }

    #[test]
    fn quick_access_default_is_the_rails_original_five() {
        assert_eq!(
            default_quick_access(),
            [
                QuickAccessDestination::NewChat,
                QuickAccessDestination::Notifications,
                QuickAccessDestination::ScanQr,
                QuickAccessDestination::RecentChats,
                QuickAccessDestination::Settings,
            ]
            .into_iter()
            .map(|id| QuickAccessItem::Destination { id })
            .collect::<Vec<_>>()
        );
        // Every destination has one wire name that parses back to itself.
        for destination in QuickAccessDestination::ALL {
            assert_eq!(
                QuickAccessDestination::parse(destination.as_str()),
                Some(destination)
            );
            assert_eq!(
                serde_json::to_value(destination).unwrap(),
                serde_json::json!(destination.as_str())
            );
        }
    }

    fn crew_id() -> String {
        format!("nyxg-{}", uuid::Uuid::new_v4().simple())
    }

    #[test]
    fn quick_access_parses_destinations_agents_and_crews_in_order() {
        let agent = "6F9619FF-8B86-D011-B42D-00C04FC964FF";
        let crew = crew_id();
        let items = parse_quick_access(&[
            entry("destination", "machines"),
            entry("agent", agent),
            entry("crew", &crew),
            entry("destination", "new_chat"),
        ])
        .unwrap();
        assert_eq!(
            items,
            vec![
                QuickAccessItem::Destination {
                    id: QuickAccessDestination::Machines
                },
                // Stored in canonical lowercase form.
                QuickAccessItem::Agent {
                    id: agent.to_ascii_lowercase()
                },
                QuickAccessItem::Crew { id: crew.clone() },
                QuickAccessItem::Destination {
                    id: QuickAccessDestination::NewChat
                },
            ]
        );
        // `group` is accepted for a crew, and crew ids are canonical
        // lowercase; an empty list is a valid choice.
        assert_eq!(
            parse_quick_access(&[entry(
                "group",
                &crew.to_ascii_uppercase().replace("NYXG-", "nyxg-")
            )])
            .unwrap(),
            vec![QuickAccessItem::Crew { id: crew }]
        );
        assert!(parse_quick_access(&[]).unwrap().is_empty());
        let twelve: Vec<_> = QuickAccessDestination::ALL[..12]
            .iter()
            .map(|destination| entry("destination", destination.as_str()))
            .collect();
        assert_eq!(parse_quick_access(&twelve).unwrap().len(), 12);
    }

    #[test]
    fn quick_access_rejects_unknown_malformed_duplicate_and_oversized_lists() {
        let agent = uuid::Uuid::new_v4().to_string();
        for entries in [
            vec![entry("destination", "billing")],
            vec![entry("destination", "NewChat")],
            vec![entry("page", "notifications")],
            vec![entry("agent", "not-a-uuid")],
            vec![entry("agent", &crew_id())],
            vec![entry("crew", "")],
            vec![entry("crew", &agent)],
            vec![entry("crew", "nyxg-1234")],
            vec![
                entry("destination", "scan_qr"),
                entry("destination", "scan_qr"),
            ],
            vec![
                entry("agent", &agent),
                entry("agent", &agent.to_ascii_uppercase()),
            ],
        ] {
            let error = parse_quick_access(&entries).unwrap_err();
            assert!(matches!(error, AppError::ValidationError(_)), "{error:?}");
            assert_eq!(error.status_code(), axum::http::StatusCode::BAD_REQUEST);
        }
        // A crew pinned twice is a duplicate too.
        let crew = crew_id();
        assert!(matches!(
            parse_quick_access(&[entry("crew", &crew), entry("group", &crew)]),
            Err(AppError::ValidationError(_))
        ));
        let ids: Vec<String> = (0..13).map(|_| uuid::Uuid::new_v4().to_string()).collect();
        let thirteen: Vec<_> = ids.iter().map(|id| entry("agent", id)).collect();
        assert!(matches!(
            parse_quick_access(&thirteen),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn stored_quick_access_round_trips_and_skips_entries_this_binary_does_not_know() {
        let preferences = QuickAccessPreferences {
            items: parse_quick_access(&[
                entry("destination", "activity"),
                entry("agent", "6f9619ff-8b86-d011-b42d-00c04fc964ff"),
            ])
            .unwrap(),
        };
        let stored = bson::to_document(&preferences).unwrap();
        assert_eq!(
            bson::from_document::<QuickAccessPreferences>(stored).unwrap(),
            preferences
        );
        // A newer binary's kind or destination is skipped, never fatal to the
        // user document.
        let newer = doc! { "items": [
            { "kind": "destination", "id": "billing" },
            { "kind": "workflow", "id": "x" },
            { "kind": "destination", "id": "plugins" },
            "garbage",
        ] };
        assert_eq!(
            bson::from_document::<QuickAccessPreferences>(newer)
                .unwrap()
                .items,
            vec![QuickAccessItem::Destination {
                id: QuickAccessDestination::Plugins
            }]
        );
        // A user document without the field keeps the default (None).
        let config: crate::models::user::UserProfileConfig = bson::from_document(doc! {}).unwrap();
        assert_eq!(config.quick_access, None);
    }
}
