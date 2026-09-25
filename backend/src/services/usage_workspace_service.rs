use super::admin_usage_service::analytics::AnalyticsQuery;
use crate::errors::{AppError, AppResult};
use crate::models::usage_workspace::{COLLECTION_NAME, UsageWorkspace};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFilters {
    pub period: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub services: Vec<String>,
    pub actors: Vec<String>,
    pub owners: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePanel {
    pub id: String,
    pub title: String,
    pub chart: String,
    pub measure: String,
    pub metric: String,
    pub breakdown: String,
    pub top: u32,
    pub wide: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceView {
    pub id: String,
    pub name: String,
    pub layout: String,
    pub filters: WorkspaceFilters,
    pub panels: Vec<WorkspacePanel>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub version: u32,
    pub draft: WorkspaceView,
    pub saved_views: Vec<WorkspaceView>,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceResponse {
    pub revision: i64,
    pub config: Option<WorkspaceConfig>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveWorkspaceRequest {
    pub revision: i64,
    pub config: WorkspaceConfig,
}
fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

pub fn validate(config: &WorkspaceConfig) -> AppResult<()> {
    if config.version != 1 || config.saved_views.len() > 20 {
        return Err(invalid(
            "Unsupported workspace version or more than 20 saved views",
        ));
    }
    if serde_json::to_vec(config)
        .map_err(|_| invalid("Invalid workspace"))?
        .len()
        > 1_048_576
    {
        return Err(invalid("Workspace settings must fit within 1 MiB"));
    }
    let mut ids = HashSet::new();
    for view in &config.saved_views {
        if !ids.insert(&view.id) {
            return Err(invalid("Saved view ids must be unique"));
        }
    }
    for view in std::iter::once(&config.draft).chain(&config.saved_views) {
        if uuid::Uuid::parse_str(&view.id).is_err()
            || view.name.trim().is_empty()
            || view.name.len() > 100
        {
            return Err(invalid(
                "Each view needs a UUID and a name of 1 to 100 bytes",
            ));
        }
        if !["overview", "operations", "explorer"].contains(&view.layout.as_str())
            || view.panels.is_empty()
        {
            return Err(invalid("Choose a valid layout with at least one panel"));
        }
        let filters = &view.filters;
        if !["24h", "7d", "30d", "custom"].contains(&filters.period.as_str()) {
            return Err(invalid("Unknown period"));
        }
        if filters.period != "custom" && (filters.from.is_some() || filters.to.is_some()) {
            return Err(invalid("Relative periods must not include absolute dates"));
        }
        let csv = |values: &[String]| -> AppResult<Option<String>> {
            if values.len() > 20 || values.iter().any(|v| v.contains(',')) {
                return Err(invalid("Invalid filter list"));
            }
            Ok((!values.is_empty()).then(|| values.join(",")))
        };
        let mut panels = HashSet::new();
        for panel in &view.panels {
            if uuid::Uuid::parse_str(&panel.id).is_err()
                || !panels.insert(&panel.id)
                || panel.title.trim().is_empty()
                || panel.title.len() > 100
            {
                return Err(invalid(
                    "Panels need unique UUIDs and titles of 1 to 100 bytes",
                ));
            }
            if !["line", "bar", "pie", "combo"].contains(&panel.chart.as_str())
                || (panel.chart == "combo" && panel.measure == "requests")
            {
                return Err(invalid("Unsupported chart type"));
            }
            if panel.span.is_some_and(|span| !(1..=3).contains(&span))
                || panel
                    .height
                    .as_deref()
                    .is_some_and(|height| !["compact", "standard", "tall"].contains(&height))
            {
                return Err(invalid("Choose 1 to 3 columns and a valid panel height"));
            }
            AnalyticsQuery {
                period: (filters.period != "custom").then(|| filters.period.clone()),
                from: filters.from.clone(),
                to: filters.to.clone(),
                services: csv(&filters.services)?,
                actors: csv(&filters.actors)?,
                owners: csv(&filters.owners)?,
                measure: Some(panel.measure.clone()),
                metric: Some(panel.metric.clone()),
                breakdown: Some(panel.breakdown.clone()),
                top: Some(panel.top),
                interval: panel.interval.clone(),
            }
            .validate(Utc::now())?;
            if filters.period == "custom" && (filters.from.is_none() || filters.to.is_none()) {
                return Err(invalid("Custom periods need start and end times"));
            }
        }
    }
    Ok(())
}

pub async fn get(db: &mongodb::Database, user_id: &str) -> AppResult<WorkspaceResponse> {
    let row = db
        .collection::<UsageWorkspace>(COLLECTION_NAME)
        .find_one(doc! { "_id": user_id })
        .await?;
    match row {
        None => Ok(WorkspaceResponse {
            revision: 0,
            config: None,
        }),
        Some(row) => Ok(WorkspaceResponse {
            revision: row.revision,
            config: Some(
                serde_json::from_value(row.config)
                    .map_err(|_| AppError::Internal("Invalid stored usage workspace".into()))?,
            ),
        }),
    }
}

pub async fn save(
    db: &mongodb::Database,
    user_id: &str,
    request: SaveWorkspaceRequest,
) -> AppResult<WorkspaceResponse> {
    validate(&request.config)?;
    if !(0..i64::MAX).contains(&request.revision) {
        return Err(invalid("Invalid workspace revision"));
    }
    let revision = request.revision + 1;
    let collection = db.collection::<UsageWorkspace>(COLLECTION_NAME);
    let config = serde_json::to_value(&request.config).map_err(|_| invalid("Invalid workspace"))?;
    let row = UsageWorkspace {
        user_id: user_id.into(),
        revision,
        config,
        updated_at: Utc::now(),
    };
    let conflict = || {
        AppError::Conflict("This workspace changed in another tab. Reload it before saving.".into())
    };
    if request.revision == 0 {
        if let Err(error) = collection.insert_one(&row).await {
            if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000)
            {
                return Err(conflict());
            }
            return Err(error.into());
        }
    } else {
        let result = collection.update_one(doc! { "_id": user_id, "revision": request.revision }, doc! { "$set": {
            "revision": revision, "config": bson::to_bson(&row.config).map_err(|_| AppError::Internal("Invalid workspace serialization".into()))?, "updated_at": bson::DateTime::from_chrono(row.updated_at),
        } }).await?;
        if result.matched_count == 0 {
            return Err(conflict());
        }
    }
    Ok(WorkspaceResponse {
        revision,
        config: Some(request.config),
    })
}

#[cfg(test)]
mod tests;
