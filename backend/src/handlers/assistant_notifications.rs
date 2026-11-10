//! NyxBot Notifications: human-only, engine- and flag-gated feed routes under
//! `/api/v1/assistant/nyxagent/notifications`, plus the read-only list used
//! by the `nyxid__list_notifications` tool.
use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_notification::{NotificationKind, SourceType},
    mw::auth::AuthUser,
    services::{
        assistant_notification_service as notifications,
        assistant_notification_service::{ListPage, ListQuery, ReadScope, Summary},
        assistant_nyxagent as engine,
        audit_service::{self, AuditActor},
    },
};

fn not_found() -> AppError {
    AppError::NotFound("Assistant route not found.".into())
}

/// First-party human, engine on, feed flag on; otherwise not-found shaped.
async fn require_feed(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    let user_id = auth.user_id.to_string();
    let features = engine::require_enabled_features(&state.db, &user_id).await?;
    if !notifications::enabled_in(&features) {
        return Err(not_found());
    }
    Ok(user_id)
}

fn parse_rfc3339(value: &str, field: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| AppError::BadRequest(format!("{field} must be an RFC 3339 timestamp")))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub unread_only: Option<bool>,
    pub pending_only: Option<bool>,
    pub kind: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ListParams>,
) -> AppResult<Json<ListPage>> {
    let user_id = require_feed(&state, &auth).await?;
    let limit = params.limit.unwrap_or(notifications::DEFAULT_LIST_LIMIT);
    if limit == 0 || limit > notifications::MAX_LIST_LIMIT {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {}",
            notifications::MAX_LIST_LIMIT
        )));
    }
    let kinds = params
        .kind
        .as_deref()
        .map(notifications::parse_kinds)
        .transpose()?
        .filter(|kinds| !kinds.is_empty());
    let page = notifications::list(
        &state,
        &user_id,
        &ListQuery {
            cursor: params.cursor,
            limit,
            unread_only: params.unread_only.unwrap_or(false),
            pending_only: params.pending_only.unwrap_or(false),
            kinds,
            since: None,
        },
    )
    .await?;
    Ok(Json(page))
}

pub async fn summary(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Summary>> {
    let user_id = require_feed(&state, &auth).await?;
    Ok(Json(notifications::summary(&state.db, &user_id).await?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub id: String,
}

/// Exactly one of `ids`, `source` or `all` (with `before`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    #[serde(default)]
    pub source: Option<ReadSource>,
    #[serde(default)]
    pub all: Option<bool>,
    #[serde(default)]
    pub before: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReadResponse {
    pub updated: u64,
}

pub async fn read(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReadRequest>,
) -> AppResult<Json<ReadResponse>> {
    let user_id = require_feed(&state, &auth).await?;
    let selected = usize::from(body.ids.is_some())
        + usize::from(body.source.is_some())
        + usize::from(body.all.is_some());
    if selected != 1 {
        return Err(AppError::BadRequest(
            "Provide exactly one of ids, source or all".into(),
        ));
    }
    let scope = if let Some(ids) = body.ids {
        if body.before.is_some() {
            return Err(AppError::BadRequest("before applies only to all".into()));
        }
        if ids.is_empty() || ids.len() > notifications::MAX_READ_IDS {
            return Err(AppError::BadRequest(format!(
                "ids must contain between 1 and {} entries",
                notifications::MAX_READ_IDS
            )));
        }
        if ids.iter().any(|id| id.is_empty() || id.len() > 64) {
            return Err(AppError::BadRequest("Invalid notification id".into()));
        }
        ReadScope::Ids(ids)
    } else if let Some(source) = body.source {
        if body.before.is_some() {
            return Err(AppError::BadRequest("before applies only to all".into()));
        }
        let source_type = SourceType::parse(&source.source_type)
            .ok_or_else(|| AppError::BadRequest("Unknown source type".into()))?;
        if source.id.is_empty() || source.id.len() > 128 {
            return Err(AppError::BadRequest("Invalid source id".into()));
        }
        ReadScope::Source {
            source_type,
            id: source.id,
        }
    } else {
        if body.all != Some(true) {
            return Err(AppError::BadRequest("all must be true".into()));
        }
        let before = body
            .before
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("all requires before".into()))?;
        ReadScope::All {
            before: parse_rfc3339(before, "before")?,
        }
    };
    let audit_all = matches!(scope, ReadScope::All { .. });
    let updated = notifications::mark_read(&state.db, &user_id, scope).await?;
    if audit_all {
        let _ = audit_service::log_actor_event(
            state.db.clone(),
            &AuditActor::from_auth_user(&auth),
            "assistant_notifications_read",
            Some(json!({"count": updated, "scope": "all"})),
        )
        .await;
    }
    Ok(Json(ReadResponse { updated }))
}

/// Read-only list for the NyxBot tool: same DTOs and list path, never marks
/// read. `since` RFC 3339, `kinds` subset, `unread_only` default true,
/// `limit` 1..=50. Callers authorize the acting chat before this.
pub async fn list_for_tool(state: &AppState, user_id: &str, args: &Value) -> AppResult<Value> {
    engine::require_enabled(&state.db, user_id).await?;
    if !notifications::enabled(&state.db, user_id).await? {
        return Err(not_found());
    }
    let since = args
        .get("since")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| AppError::BadRequest("since must be a string".into()))
                .and_then(|value| parse_rfc3339(value, "since"))
        })
        .transpose()?;
    let kinds = match args.get("kinds").filter(|value| !value.is_null()) {
        None => None,
        Some(Value::Array(values)) => {
            let mut kinds = Vec::new();
            for value in values {
                let name = value
                    .as_str()
                    .ok_or_else(|| AppError::BadRequest("kinds must be strings".into()))?;
                let kind = NotificationKind::parse(name).ok_or_else(|| {
                    AppError::BadRequest(format!("Unknown notification kind: {name}"))
                })?;
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            Some(kinds)
        }
        Some(Value::String(value)) => Some(notifications::parse_kinds(value)?),
        Some(_) => return Err(AppError::BadRequest("kinds must be a list".into())),
    };
    let limit = match args.get("limit").filter(|value| !value.is_null()) {
        None => notifications::DEFAULT_LIST_LIMIT,
        Some(value) => {
            let limit = value
                .as_u64()
                .ok_or_else(|| AppError::BadRequest("limit must be a positive integer".into()))?;
            if limit == 0 || limit > notifications::MAX_LIST_LIMIT as u64 {
                return Err(AppError::BadRequest(format!(
                    "limit must be between 1 and {}",
                    notifications::MAX_LIST_LIMIT
                )));
            }
            limit as usize
        }
    };
    let unread_only = match args.get("unread_only").filter(|value| !value.is_null()) {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AppError::BadRequest("unread_only must be a boolean".into()))?,
    };
    let page = notifications::list(
        state,
        user_id,
        &ListQuery {
            cursor: None,
            limit,
            unread_only,
            pending_only: false,
            kinds,
            since,
        },
    )
    .await?;
    Ok(serde_json::to_value(page)
        .map_err(|_| AppError::Internal("Notification list unavailable".into()))?)
}
