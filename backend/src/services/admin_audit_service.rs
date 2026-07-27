//! Server-side table controls for the platform-admin audit log.
//!
//! Mirrors the bounded search/filter/sort contract of
//! [`crate::services::oauth_client_service`]'s admin client list: every control
//! is validated against a fixed domain (or a fixed shape, for the free-text
//! ones) before it reaches MongoDB, and every user-supplied string that ends up
//! in a `$regex` is escaped first.
//!
//! The audit collection is append-only and can grow without bound, so each
//! filter is chosen to ride an existing index (`user_id`, `event_type`,
//! `api_key_id`, each compounded with `created_at`) wherever possible.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};
use serde::Deserialize;

use crate::errors::{AppError, AppResult};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};

/// HTTP status buckets offered by the `status` filter. `none` covers rows whose
/// `event_data` carries no `response_status` at all (most non-proxy events).
pub const ADMIN_STATUS_FILTERS: &[&str] = &["2xx", "3xx", "4xx", "5xx", "none"];

/// Who the row is attributed to. Derived from `user_id` / `api_key_id`
/// presence rather than a stored column.
pub const ADMIN_ACTOR_FILTERS: &[&str] = &["user", "agent", "anonymous"];

/// Filters that also accept free text, mapped to the stored field the text is
/// matched against with a case-insensitive `contains`. Only filters backed by a
/// single string column can appear here: `status` and `actor` are derived, and
/// `created_at` is a date, so none of them has a column a substring could match.
///
/// Everything after `event_type` is a *text-only* filter: its column is too
/// high-cardinality to enumerate as checkbox options (UUIDs, IPs, User-Agent
/// strings), so free text is the only way to filter it.
pub const ADMIN_CUSTOM_TEXT_FILTERS: &[(&str, &str)] = &[
    ("event_type", "event_type"),
    ("api_key_name", "api_key_name"),
    ("user_id", "user_id"),
    ("api_key_id", "api_key_id"),
    ("ip_address", "ip_address"),
    ("user_agent", "user_agent"),
];

/// Stored field a custom-text filter searches, or `None` when the filter takes
/// no free text.
pub fn admin_custom_text_field(filter: &str) -> Option<&'static str> {
    ADMIN_CUSTOM_TEXT_FILTERS
        .iter()
        .find(|(key, _)| *key == filter)
        .map(|(_, stored)| *stored)
}

/// Fields the scoped search box can target. `api_key` spans both the key ID and
/// its human-readable name so one term finds an agent either way.
pub const ADMIN_SEARCH_FIELDS: &[(&str, &str)] = &[
    ("event_type", "Event type"),
    ("user_id", "User ID"),
    ("api_key", "Agent / API key"),
    ("ip_address", "IP address"),
    ("user_agent", "User agent"),
];

pub const ADMIN_SORT_OPTIONS: &[&str] = &[
    "-created_at",
    "created_at",
    "event_type",
    "-event_type",
    "api_key_name",
    "-api_key_name",
    "api_key_id",
    "-api_key_id",
    "user_id",
    "-user_id",
    "ip_address",
    "-ip_address",
    "user_agent",
    "-user_agent",
    "status",
    "-status",
];

/// Nested key the `status` column sorts and filters on.
const RESPONSE_STATUS_FIELD: &str = "event_data.response_status";

const ADMIN_SEARCH_MAX_CHARS: usize = 256;
const ADMIN_SEARCH_FILTER_MAX_GROUPS: usize = 5;
/// Must stay >= [`ADMIN_CUSTOM_TEXT_FILTERS`], or a user who types text into
/// every text filter at once gets rejected.
const ADMIN_CUSTOM_FILTER_MAX_GROUPS: usize = 8;
const ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP: usize = 8;
const ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES: usize = 32;
const ADMIN_CREATED_DATES_MAX_VALUES: usize = 32;
const ADMIN_EVENT_TYPE_MAX_VALUES: usize = 32;

/// Upper bound on the event types advertised as checkbox options. The vocabulary
/// is open (any call site may log a new one), so the list is discovered from the
/// data; past this many the filter's free-text box is the way to reach the tail.
pub const ADMIN_EVENT_TYPE_OPTION_LIMIT: usize = 200;

/// Validated table controls for the platform-admin audit-log list.
pub struct AdminAuditLogListParams<'a> {
    pub page: u64,
    pub per_page: u64,
    pub search: Option<&'a str>,
    pub search_filters: Option<&'a str>,
    pub custom_filters: Option<&'a str>,
    pub event_type: Option<&'a str>,
    pub status: Option<&'a str>,
    pub actor: Option<&'a str>,
    /// Legacy exact-match param, kept for existing API clients.
    pub user_id: Option<&'a str>,
    /// Legacy exact-match param, kept for existing API clients.
    pub api_key_id: Option<&'a str>,
    pub created_dates: Option<&'a str>,
    pub created_from: Option<&'a str>,
    pub created_to: Option<&'a str>,
    pub sort: &'a str,
}

/// List audit entries using bounded, server-side admin-table controls.
pub async fn list_entries(
    db: &mongodb::Database,
    params: AdminAuditLogListParams<'_>,
) -> AppResult<(Vec<AuditLog>, u64)> {
    if params.page == 0 || !(1..=100).contains(&params.per_page) {
        return Err(AppError::ValidationError(
            "page must be at least 1 and per_page must be between 1 and 100".to_string(),
        ));
    }

    let offset = params
        .page
        .checked_sub(1)
        .and_then(|page| page.checked_mul(params.per_page))
        .ok_or_else(|| AppError::ValidationError("page is too large".to_string()))?;
    let filter = admin_audit_filter(&params)?;
    let sort = admin_audit_sort(params.sort)?;
    let collection = db.collection::<AuditLog>(AUDIT_LOG);

    let total = collection.count_documents(filter.clone()).await?;
    if offset >= total {
        return Ok((Vec::new(), total));
    }
    let limit = i64::try_from(params.per_page)
        .map_err(|_| AppError::ValidationError("per_page is too large".to_string()))?;
    if i64::try_from(offset).is_err() {
        return Err(AppError::ValidationError("page is too large".to_string()));
    }

    let entries: Vec<AuditLog> = collection
        .find(filter)
        .sort(sort)
        .skip(offset)
        .limit(limit)
        .await?
        .try_collect()
        .await?;

    Ok((entries, total))
}

/// Event types present in the collection, for the `event_type` filter's
/// checkbox options.
///
/// Reads distinct values off the `event_type` index. The vocabulary is open, so
/// the result is capped at [`ADMIN_EVENT_TYPE_OPTION_LIMIT`]; anything past the
/// cap stays reachable through the filter's free-text box, which runs a
/// `contains` against the same column.
pub async fn distinct_event_types(db: &mongodb::Database) -> AppResult<Vec<String>> {
    let values = db
        .collection::<AuditLog>(AUDIT_LOG)
        .distinct("event_type", doc! {})
        .await?;

    let mut event_types: Vec<String> = values
        .into_iter()
        .filter_map(|value| match value {
            Bson::String(event_type) if !event_type.trim().is_empty() => Some(event_type),
            _ => None,
        })
        .collect();
    event_types.sort_unstable();
    event_types.dedup();
    event_types.truncate(ADMIN_EVENT_TYPE_OPTION_LIMIT);
    Ok(event_types)
}

fn admin_audit_filter(params: &AdminAuditLogListParams<'_>) -> AppResult<Document> {
    let mut clauses = Vec::new();

    if let Some(search) = params
        .search
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if search.chars().count() > ADMIN_SEARCH_MAX_CHARS {
            return Err(AppError::ValidationError(format!(
                "search must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
            )));
        }
        let escaped = regex::escape(search);
        clauses.push(doc! {
            "$or": [
                { "event_type": { "$regex": &escaped, "$options": "i" } },
                { "user_id": { "$regex": &escaped, "$options": "i" } },
                { "api_key_id": { "$regex": &escaped, "$options": "i" } },
                { "api_key_name": { "$regex": &escaped, "$options": "i" } },
                { "ip_address": { "$regex": &escaped, "$options": "i" } },
                { "user_agent": { "$regex": &escaped, "$options": "i" } },
                { "_id": { "$regex": &escaped, "$options": "i" } },
            ]
        });
    }

    if let Some(search_filters) = params.search_filters {
        clauses.extend(admin_search_filter_clauses(search_filters)?);
    }

    // Free text typed into a filter widens that filter rather than narrowing it:
    // the custom `contains` branches are OR'd into the same clause as that
    // filter's checked options, so `Event is login or contains "mfa"` works.
    let mut custom = admin_custom_filter_branches(params.custom_filters)?;
    let mut push_filter = |clauses: &mut Vec<Document>, key: &str, options: Vec<Document>| {
        let mut branches = options;
        branches.extend(custom.remove(key).unwrap_or_default());
        if !branches.is_empty() {
            clauses.push(one_or_many(branches));
        }
    };

    push_filter(
        &mut clauses,
        "event_type",
        match params.event_type {
            None => Vec::new(),
            // The event-type vocabulary is open, so values are matched exactly
            // rather than validated against a fixed domain.
            Some(event_type) => admin_open_csv_values("event_type", event_type)?
                .into_iter()
                .map(|value| doc! { "event_type": value })
                .collect(),
        },
    );

    // Text-only filters have no checkable options to merge with, so their
    // `contains` branches are the whole clause. Drained in a fixed order so the
    // same request always builds the same query.
    for (key, _) in ADMIN_CUSTOM_TEXT_FILTERS {
        if let Some(branches) = custom.remove(key) {
            clauses.push(one_or_many(branches));
        }
    }
    debug_assert!(
        custom.is_empty(),
        "every parsed custom-text filter must reach the query",
    );

    if let Some(status) = params.status {
        let branches = admin_csv_filter_values("status", status, ADMIN_STATUS_FILTERS)?
            .into_iter()
            .map(status_bucket_filter)
            .collect();
        clauses.push(one_or_many(branches));
    }

    if let Some(actor) = params.actor {
        let branches = admin_csv_filter_values("actor", actor, ADMIN_ACTOR_FILTERS)?
            .into_iter()
            .map(actor_filter)
            .collect();
        clauses.push(one_or_many(branches));
    }

    // Legacy exact-match params, still documented for API clients.
    if let Some(user_id) = params.user_id.map(str::trim).filter(|v| !v.is_empty()) {
        clauses.push(doc! { "user_id": user_id });
    }
    if let Some(api_key_id) = params.api_key_id.map(str::trim).filter(|v| !v.is_empty()) {
        clauses.push(doc! { "api_key_id": api_key_id });
    }

    if let Some(created_at) =
        admin_created_at_filter(params.created_dates, params.created_from, params.created_to)?
    {
        clauses.push(created_at);
    }

    Ok(match clauses.len() {
        0 => doc! {},
        1 => clauses.remove(0),
        _ => doc! { "$and": clauses },
    })
}

/// `{"$gte": 200, "$lt": 300}`-style bounds on the proxied response status.
///
/// `none` matches rows with no `response_status` at all: Mongo treats
/// `{field: null}` as "missing or null", which is exactly the set of audit rows
/// that never carried a status.
fn status_bucket_filter(bucket: &str) -> Document {
    let bounds = match bucket {
        "2xx" => (200, 300),
        "3xx" => (300, 400),
        "4xx" => (400, 500),
        "5xx" => (500, 600),
        _ => return doc! { RESPONSE_STATUS_FIELD: Bson::Null },
    };
    doc! { RESPONSE_STATUS_FIELD: { "$gte": bounds.0, "$lt": bounds.1 } }
}

fn actor_filter(actor: &str) -> Document {
    match actor {
        "agent" => doc! { "api_key_id": { "$exists": true, "$ne": Bson::Null } },
        "user" => doc! {
            "user_id": { "$exists": true, "$ne": Bson::Null },
            "api_key_id": Bson::Null,
        },
        // `{field: null}` matches both an explicit null and a missing field.
        _ => doc! { "user_id": Bson::Null },
    }
}

/// Parse the `custom_filters` query param into per-filter `contains` branches.
///
/// Shape: `{"event_type":["mfa"]}`. Values are regex-escaped and matched
/// case-insensitively against the stored field the filter maps to
/// ([`ADMIN_CUSTOM_TEXT_FILTERS`]). Bounds mirror `search_filters`.
fn admin_custom_filter_branches(
    raw: Option<&str>,
) -> AppResult<HashMap<&'static str, Vec<Document>>> {
    let mut branches: HashMap<&'static str, Vec<Document>> = HashMap::new();
    let Some(raw) = raw else {
        return Ok(branches);
    };

    let groups: HashMap<String, Vec<String>> = serde_json::from_str(raw).map_err(|_| {
        AppError::ValidationError(
            "custom_filters must be a JSON object of filter keys to value arrays".to_string(),
        )
    })?;

    if groups.len() > ADMIN_CUSTOM_FILTER_MAX_GROUPS {
        return Err(AppError::ValidationError(format!(
            "custom_filters must contain at most {ADMIN_CUSTOM_FILTER_MAX_GROUPS} filters"
        )));
    }

    let mut total_values = 0usize;
    for (filter, values) in groups {
        let Some(stored_field) = admin_custom_text_field(&filter) else {
            return Err(AppError::ValidationError(format!(
                "custom_filters filter must be one of: {}",
                ADMIN_CUSTOM_TEXT_FILTERS
                    .iter()
                    .map(|(key, _)| *key)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        if values.is_empty() {
            return Err(AppError::ValidationError(format!(
                "custom_filters values for {filter} must not be empty"
            )));
        }
        if values.len() > ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP {
            return Err(AppError::ValidationError(format!(
                "custom_filters values for {filter} must contain at most {ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP} entries"
            )));
        }
        total_values = total_values.checked_add(values.len()).ok_or_else(|| {
            AppError::ValidationError("custom_filters contains too many values".to_string())
        })?;
        if total_values > ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES {
            return Err(AppError::ValidationError(format!(
                "custom_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES} values"
            )));
        }

        let entry = branches.entry(stored_field_key(&filter)).or_default();
        for raw_value in values {
            let value = raw_value.trim();
            if value.is_empty() {
                return Err(AppError::ValidationError(format!(
                    "custom_filters values for {filter} must not be empty"
                )));
            }
            if value.chars().count() > ADMIN_SEARCH_MAX_CHARS {
                return Err(AppError::ValidationError(format!(
                    "custom_filters values must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
                )));
            }
            entry.push(doc! {
                stored_field: {
                    "$regex": regex::escape(value),
                    "$options": "i",
                }
            });
        }
    }

    Ok(branches)
}

/// Borrow the `'static` filter key so the branch map outlives the parsed JSON.
fn stored_field_key(filter: &str) -> &'static str {
    ADMIN_CUSTOM_TEXT_FILTERS
        .iter()
        .find(|(key, _)| *key == filter)
        .map(|(key, _)| *key)
        .expect("filter was validated against ADMIN_CUSTOM_TEXT_FILTERS")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminSearchFilterInput {
    field: String,
    values: Vec<String>,
}

fn admin_search_filter_clauses(raw: &str) -> AppResult<Vec<Document>> {
    let groups: Vec<AdminSearchFilterInput> = serde_json::from_str(raw).map_err(|_| {
        AppError::ValidationError(
            "search_filters must be a JSON array of field and values objects".to_string(),
        )
    })?;

    if groups.is_empty() {
        return Err(AppError::ValidationError(
            "search_filters must contain at least one field group".to_string(),
        ));
    }
    if groups.len() > ADMIN_SEARCH_FILTER_MAX_GROUPS {
        return Err(AppError::ValidationError(format!(
            "search_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_GROUPS} field groups"
        )));
    }

    let mut seen_fields = HashSet::new();
    let mut total_values = 0usize;
    let mut clauses = Vec::with_capacity(groups.len());

    for group in groups {
        // `api_key` spans two columns, so it has no single stored field.
        let stored_field = match group.field.as_str() {
            "api_key" => None,
            "event_type" => Some("event_type"),
            "user_id" => Some("user_id"),
            "ip_address" => Some("ip_address"),
            "user_agent" => Some("user_agent"),
            _ => {
                return Err(AppError::ValidationError(format!(
                    "search_filters field must be one of: {}",
                    ADMIN_SEARCH_FIELDS
                        .iter()
                        .map(|(key, _)| *key)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };

        if !seen_fields.insert(group.field.clone()) {
            return Err(AppError::ValidationError(format!(
                "search_filters must not repeat field {}",
                group.field
            )));
        }
        if group.values.is_empty() {
            return Err(AppError::ValidationError(format!(
                "search_filters values for {} must not be empty",
                group.field
            )));
        }
        if group.values.len() > ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP {
            return Err(AppError::ValidationError(format!(
                "search_filters values for {} must contain at most {ADMIN_SEARCH_FILTER_MAX_VALUES_PER_GROUP} entries",
                group.field
            )));
        }

        total_values = total_values
            .checked_add(group.values.len())
            .ok_or_else(|| {
                AppError::ValidationError("search_filters contains too many values".to_string())
            })?;
        if total_values > ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES {
            return Err(AppError::ValidationError(format!(
                "search_filters must contain at most {ADMIN_SEARCH_FILTER_MAX_TOTAL_VALUES} values"
            )));
        }

        let mut value_clauses = Vec::with_capacity(group.values.len());
        for raw_value in group.values {
            let value = raw_value.trim();
            if value.is_empty() {
                return Err(AppError::ValidationError(format!(
                    "search_filters values for {} must not be empty",
                    group.field
                )));
            }
            if value.chars().count() > ADMIN_SEARCH_MAX_CHARS {
                return Err(AppError::ValidationError(format!(
                    "search_filters values must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
                )));
            }

            let regex = doc! {
                "$regex": regex::escape(value),
                "$options": "i",
            };
            value_clauses.push(match stored_field {
                Some(field) => doc! { field: regex },
                None => doc! {
                    "$or": [
                        { "api_key_id": regex.clone() },
                        { "api_key_name": regex },
                    ]
                },
            });
        }
        clauses.push(one_or_many(value_clauses));
    }

    Ok(clauses)
}

fn admin_created_at_filter(
    created_dates: Option<&str>,
    created_from: Option<&str>,
    created_to: Option<&str>,
) -> AppResult<Option<Document>> {
    if let Some(created_dates) = created_dates {
        if created_from.is_some() || created_to.is_some() {
            return Err(AppError::ValidationError(
                "created_dates cannot be combined with created_from or created_to".to_string(),
            ));
        }

        let raw_dates = created_dates.split(',').collect::<Vec<_>>();
        if raw_dates.len() > ADMIN_CREATED_DATES_MAX_VALUES {
            return Err(AppError::ValidationError(format!(
                "created_dates must contain at most {ADMIN_CREATED_DATES_MAX_VALUES} values"
            )));
        }

        let mut seen = HashSet::new();
        let mut date_filters = Vec::new();
        for raw_date in raw_dates {
            let value = raw_date.trim();
            if value.is_empty() {
                return Err(AppError::ValidationError(
                    "created_dates must not contain empty values".to_string(),
                ));
            }
            let date = parse_admin_calendar_date("created_dates", value)?;
            if !seen.insert(date) {
                continue;
            }
            let exclusive_to = date.succ_opt().ok_or_else(|| {
                AppError::ValidationError(
                    "created_dates contains a date outside the supported range".to_string(),
                )
            })?;
            date_filters.push(doc! {
                "created_at": {
                    "$gte": midnight_utc(date),
                    "$lt": midnight_utc(exclusive_to),
                }
            });
        }

        return Ok(Some(one_or_many(date_filters)));
    }

    let created_from = created_from
        .map(|value| parse_admin_calendar_date("created_from", value))
        .transpose()?;
    let created_to = created_to
        .map(|value| parse_admin_calendar_date("created_to", value))
        .transpose()?;

    if created_from.is_some_and(|from| created_to.is_some_and(|to| from > to)) {
        return Err(AppError::ValidationError(
            "created_from must be on or before created_to".to_string(),
        ));
    }

    let mut bounds = Document::new();
    if let Some(from) = created_from {
        bounds.insert("$gte", midnight_utc(from));
    }
    if let Some(to) = created_to {
        let exclusive_to = to.succ_opt().ok_or_else(|| {
            AppError::ValidationError("created_to is outside the supported date range".to_string())
        })?;
        bounds.insert("$lt", midnight_utc(exclusive_to));
    }

    Ok((!bounds.is_empty()).then(|| doc! { "created_at": bounds }))
}

fn parse_admin_calendar_date(field: &str, value: &str) -> AppResult<NaiveDate> {
    let bytes = value.as_bytes();
    let has_exact_format = bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if !has_exact_format {
        return Err(AppError::ValidationError(format!(
            "{field} must be a valid date in YYYY-MM-DD format"
        )));
    }

    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        AppError::ValidationError(format!("{field} must be a valid date in YYYY-MM-DD format"))
    })
}

fn midnight_utc(date: NaiveDate) -> bson::DateTime {
    bson::DateTime::from_chrono(
        date.and_hms_opt(0, 0, 0)
            .expect("midnight is valid for every calendar date")
            .and_utc(),
    )
}

fn admin_csv_filter_values<'a>(
    field: &str,
    raw: &'a str,
    allowed: &[&str],
) -> AppResult<Vec<&'a str>> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();

    for raw_value in raw.split(',') {
        let value = raw_value.trim();
        if value.is_empty() {
            return Err(AppError::ValidationError(format!(
                "{field} must not contain empty values"
            )));
        }
        if !allowed.contains(&value) {
            return Err(AppError::ValidationError(format!(
                "{field} must contain only: {}",
                allowed.join(", ")
            )));
        }
        if seen.insert(value) {
            values.push(value);
        }
    }

    Ok(values)
}

/// CSV values for a filter whose domain is open (discovered from the data, not
/// a fixed list). Bounded by count and length, deduplicated, and matched
/// exactly -- never interpolated into a regex.
fn admin_open_csv_values<'a>(field: &str, raw: &'a str) -> AppResult<Vec<&'a str>> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();

    for raw_value in raw.split(',') {
        let value = raw_value.trim();
        if value.is_empty() {
            return Err(AppError::ValidationError(format!(
                "{field} must not contain empty values"
            )));
        }
        if value.chars().count() > ADMIN_SEARCH_MAX_CHARS {
            return Err(AppError::ValidationError(format!(
                "{field} values must be {ADMIN_SEARCH_MAX_CHARS} characters or less"
            )));
        }
        if seen.insert(value) {
            values.push(value);
        }
    }

    if values.len() > ADMIN_EVENT_TYPE_MAX_VALUES {
        return Err(AppError::ValidationError(format!(
            "{field} must contain at most {ADMIN_EVENT_TYPE_MAX_VALUES} values"
        )));
    }

    Ok(values)
}

fn one_or_many(mut filters: Vec<Document>) -> Document {
    if filters.len() == 1 {
        filters.remove(0)
    } else {
        doc! { "$or": filters }
    }
}

fn admin_audit_sort(sort: &str) -> AppResult<Document> {
    let (field, direction) = match sort.strip_prefix('-') {
        Some(field) => (field, -1),
        None => (sort, 1),
    };
    if !matches!(
        field,
        "created_at"
            | "event_type"
            | "api_key_name"
            | "api_key_id"
            | "user_id"
            | "ip_address"
            | "user_agent"
            | "status"
    ) {
        return Err(AppError::ValidationError(
            "sort must target created_at, event_type, api_key_name, api_key_id, user_id, ip_address, user_agent, or status".to_string(),
        ));
    }

    let mut sort_doc = Document::new();
    let stored_field = if field == "status" {
        RESPONSE_STATUS_FIELD
    } else {
        field
    };
    sort_doc.insert(stored_field, direction);
    // Audit rows are written in bursts, so a secondary key keeps paging stable
    // when the primary key ties.
    if field != "created_at" {
        sort_doc.insert("created_at", direction);
    }
    sort_doc.insert("_id", direction);
    Ok(sort_doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params<'a>() -> AdminAuditLogListParams<'a> {
        AdminAuditLogListParams {
            page: 1,
            per_page: 25,
            search: None,
            search_filters: None,
            custom_filters: None,
            event_type: None,
            status: None,
            actor: None,
            user_id: None,
            api_key_id: None,
            created_dates: None,
            created_from: None,
            created_to: None,
            sort: "-created_at",
        }
    }

    #[test]
    fn empty_controls_produce_an_empty_filter() {
        assert_eq!(admin_audit_filter(&params()).expect("filter"), doc! {});
    }

    #[test]
    fn sort_options_all_parse() {
        for sort in ADMIN_SORT_OPTIONS {
            admin_audit_sort(sort).unwrap_or_else(|_| panic!("{sort} should parse"));
        }
    }

    #[test]
    fn sort_defaults_to_newest_first_with_stable_tiebreak() {
        assert_eq!(
            admin_audit_sort("-created_at").expect("sort"),
            doc! { "created_at": -1, "_id": -1 }
        );
    }

    #[test]
    fn status_sorts_on_the_nested_response_status() {
        assert_eq!(
            admin_audit_sort("status").expect("sort"),
            doc! { "event_data.response_status": 1, "created_at": 1, "_id": 1 }
        );
    }

    #[test]
    fn unknown_sort_is_rejected() {
        assert!(admin_audit_sort("event_data").is_err());
        assert!(admin_audit_sort("-password").is_err());
    }

    #[test]
    fn search_escapes_regex_metacharacters() {
        let mut p = params();
        p.search = Some("a.*b");
        let filter = admin_audit_filter(&p).expect("filter");
        let branches = filter.get_array("$or").expect("$or");
        let first = branches[0].as_document().expect("document");
        let regex = first
            .get_document("event_type")
            .expect("event_type")
            .get_str("$regex")
            .expect("$regex");
        assert!(regex.contains(r"\."), "the dot should be escaped: {regex}");
        assert!(!regex.contains(".*"), "no raw wildcard should survive");
    }

    #[test]
    fn oversized_search_is_rejected() {
        let long = "a".repeat(ADMIN_SEARCH_MAX_CHARS + 1);
        let mut p = params();
        p.search = Some(&long);
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn status_buckets_map_to_half_open_ranges() {
        assert_eq!(
            status_bucket_filter("4xx"),
            doc! { "event_data.response_status": { "$gte": 400, "$lt": 500 } }
        );
        assert_eq!(
            status_bucket_filter("none"),
            doc! { "event_data.response_status": Bson::Null }
        );
    }

    #[test]
    fn status_filter_ors_multiple_buckets() {
        let mut p = params();
        p.status = Some("4xx,5xx");
        let filter = admin_audit_filter(&p).expect("filter");
        assert_eq!(filter.get_array("$or").expect("$or").len(), 2);
    }

    #[test]
    fn unknown_status_bucket_is_rejected() {
        let mut p = params();
        p.status = Some("6xx");
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn actor_user_excludes_agent_key_rows() {
        assert_eq!(
            actor_filter("user"),
            doc! {
                "user_id": { "$exists": true, "$ne": Bson::Null },
                "api_key_id": Bson::Null,
            }
        );
    }

    #[test]
    fn unknown_actor_is_rejected() {
        let mut p = params();
        p.actor = Some("robot");
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn event_type_values_are_matched_exactly() {
        let mut p = params();
        p.event_type = Some("admin.user.created,login");
        let filter = admin_audit_filter(&p).expect("filter");
        let branches = filter.get_array("$or").expect("$or");
        assert_eq!(branches.len(), 2);
        assert_eq!(
            branches[0].as_document().expect("doc"),
            &doc! { "event_type": "admin.user.created" }
        );
    }

    #[test]
    fn event_type_rejects_too_many_values() {
        let raw = (0..ADMIN_EVENT_TYPE_MAX_VALUES + 1)
            .map(|i| format!("e{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut p = params();
        p.event_type = Some(&raw);
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn custom_text_widens_the_event_type_filter() {
        let mut p = params();
        p.event_type = Some("login");
        p.custom_filters = Some(r#"{"event_type":["mfa"]}"#);
        let filter = admin_audit_filter(&p).expect("filter");
        let branches = filter.get_array("$or").expect("$or");
        assert_eq!(branches.len(), 2, "checked option OR custom contains");
        assert!(
            branches[1]
                .as_document()
                .expect("doc")
                .get_document("event_type")
                .expect("event_type")
                .contains_key("$regex")
        );
    }

    #[test]
    fn custom_filters_reject_a_derived_filter() {
        let mut p = params();
        p.custom_filters = Some(r#"{"status":["4xx"]}"#);
        assert!(admin_audit_filter(&p).is_err());
    }

    /// Regression: the text-only filters were parsed but never drained into the
    /// query, so filtering by User ID or IP alone silently returned everything.
    #[test]
    fn every_text_only_filter_reaches_the_query_on_its_own() {
        for (key, stored_field) in ADMIN_CUSTOM_TEXT_FILTERS
            .iter()
            .filter(|(key, _)| *key != "event_type")
        {
            let raw = format!(r#"{{"{key}":["needle"]}}"#);
            let mut p = params();
            p.custom_filters = Some(&raw);
            let filter = admin_audit_filter(&p).expect("filter");

            assert_eq!(
                filter
                    .get_document(stored_field)
                    .unwrap_or_else(|_| panic!("{key} should filter on {stored_field}"))
                    .get_str("$regex")
                    .expect("$regex"),
                "needle",
                "{key} must narrow the query, not vanish",
            );
        }
    }

    #[test]
    fn text_filters_combine_as_an_and() {
        let mut p = params();
        p.custom_filters = Some(r#"{"user_id":["u1"],"ip_address":["10."]}"#);
        let filter = admin_audit_filter(&p).expect("filter");
        let clauses = filter.get_array("$and").expect("$and");
        assert_eq!(clauses.len(), 2);
    }

    #[test]
    fn text_filter_values_are_regex_escaped() {
        let mut p = params();
        p.custom_filters = Some(r#"{"ip_address":["10.0.0.1"]}"#);
        let filter = admin_audit_filter(&p).expect("filter");
        let regex = filter
            .get_document("ip_address")
            .expect("ip_address")
            .get_str("$regex")
            .expect("$regex");
        assert_eq!(regex, r"10\.0\.0\.1", "dots must not stay wildcards");
    }

    #[test]
    fn text_filters_accept_one_entry_per_filter_at_once() {
        let raw = r#"{"event_type":["a"],"api_key_name":["b"],"user_id":["c"],"api_key_id":["d"],"ip_address":["e"],"user_agent":["f"]}"#;
        let mut p = params();
        p.custom_filters = Some(raw);
        let filter = admin_audit_filter(&p).expect("all six text filters at once");
        assert_eq!(filter.get_array("$and").expect("$and").len(), 6);
    }

    #[test]
    fn scoped_api_key_search_spans_id_and_name() {
        let mut p = params();
        p.search_filters = Some(r#"[{"field":"api_key","values":["coding"]}]"#);
        let filter = admin_audit_filter(&p).expect("filter");
        let branches = filter.get_array("$or").expect("$or");
        assert_eq!(branches.len(), 2);
        assert!(
            branches[0]
                .as_document()
                .expect("doc")
                .contains_key("api_key_id")
        );
        assert!(
            branches[1]
                .as_document()
                .expect("doc")
                .contains_key("api_key_name")
        );
    }

    #[test]
    fn scoped_search_rejects_an_unknown_field() {
        let mut p = params();
        p.search_filters = Some(r#"[{"field":"event_data","values":["x"]}]"#);
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn legacy_exact_params_still_filter() {
        let mut p = params();
        p.user_id = Some("user-1");
        p.api_key_id = Some("key-1");
        let filter = admin_audit_filter(&p).expect("filter");
        let clauses = filter.get_array("$and").expect("$and");
        assert_eq!(clauses.len(), 2);
        assert_eq!(
            clauses[0].as_document().expect("doc"),
            &doc! { "user_id": "user-1" }
        );
    }

    #[test]
    fn created_dates_and_range_cannot_combine() {
        let mut p = params();
        p.created_dates = Some("2026-07-01");
        p.created_from = Some("2026-07-01");
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn created_range_rejects_an_inverted_window() {
        let mut p = params();
        p.created_from = Some("2026-07-10");
        p.created_to = Some("2026-07-01");
        assert!(admin_audit_filter(&p).is_err());
    }

    #[test]
    fn created_to_is_exclusive_of_the_next_midnight() {
        let filter = admin_created_at_filter(None, None, Some("2026-07-01"))
            .expect("filter")
            .expect("some");
        let bounds = filter.get_document("created_at").expect("created_at");
        assert_eq!(
            bounds.get_datetime("$lt").expect("$lt").to_string(),
            midnight_utc(NaiveDate::from_ymd_opt(2026, 7, 2).expect("valid date")).to_string()
        );
    }

    #[test]
    fn malformed_calendar_date_is_rejected() {
        assert!(admin_created_at_filter(None, Some("07/01/2026"), None).is_err());
        assert!(admin_created_at_filter(None, Some("2026-7-1"), None).is_err());
    }
}
