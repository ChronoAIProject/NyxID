use crate::errors::{AppError, AppResult};
use chrono::{DateTime, Utc};

pub(crate) fn parse_expires_at(s: &str) -> AppResult<DateTime<Utc>> {
    // Try RFC 3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Try date-only (YYYY-MM-DD) -> end of day UTC
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        && let Some(dt) = date.and_hms_opt(23, 59, 59)
    {
        return Ok(dt.and_utc());
    }
    Err(AppError::ValidationError(
        "Invalid expires_at format. Use RFC 3339 (e.g. 2026-04-01T00:00:00Z) or date-only (e.g. 2026-04-01)".to_string(),
    ))
}

pub(crate) fn resolve_create_allow_all(
    allowed_ids: &[String],
    requested_allow_all: Option<bool>,
    allow_all_field: &str,
    allowed_ids_field: &str,
) -> AppResult<bool> {
    if requested_allow_all == Some(true) && !allowed_ids.is_empty() {
        return Err(AppError::ValidationError(format!(
            "{allow_all_field} cannot be true when {allowed_ids_field} is non-empty"
        )));
    }

    Ok(requested_allow_all.unwrap_or(allowed_ids.is_empty()))
}
