use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_message_feedback::{AssistantMessageFeedback, COLLECTION_NAME},
    mw::auth::AuthUser,
    services::{
        assistant_feedback_service as feedback_service, assistant_nyxagent as engine, audit_service,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, SecondsFormat, Utc};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct FeedbackInput {
    pub vote: String,
    pub comment: Option<String>,
}
#[derive(Serialize, Clone)]
pub struct FeedbackResponse {
    pub vote: String,
    pub comment: Option<String>,
    pub updated_at: DateTime<Utc>,
}
impl From<AssistantMessageFeedback> for FeedbackResponse {
    fn from(v: AssistantMessageFeedback) -> Self {
        Self {
            vote: v.vote,
            comment: v.comment,
            updated_at: v.updated_at,
        }
    }
}

fn sanitize_comment(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    *c,
                    '\u{200b}'
                        | '\u{200c}'
                        | '\u{200d}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'
                        | '\u{202b}'
                        | '\u{202c}'
                        | '\u{202d}'
                        | '\u{202e}'
                        | '\u{2060}'
                        | '\u{2066}'
                        | '\u{2067}'
                        | '\u{2069}'
                )
        })
        .collect()
}

fn validate(input: &FeedbackInput) -> AppResult<Option<String>> {
    if input.vote != "up" && input.vote != "down" {
        return Err(AppError::BadRequest("feedback_vote_invalid".into()));
    }
    let comment = input
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = sanitize_comment(s);
            if s.chars().count() > 1000 {
                return Err(AppError::BadRequest("feedback_comment_too_long".into()));
            }
            Ok(s.to_string())
        })
        .transpose()?;
    Ok(comment)
}
async fn enforce_feedback_limit(state: &AppState, user_id: &str) -> AppResult<()> {
    let limiter = crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "assistant_feedback",
        30,
        60,
    );
    if !limiter
        .check_shared(&format!("assistant-feedback:{user_id}"))
        .await?
    {
        return Err(AppError::RateLimited);
    }
    Ok(())
}
async fn upsert(
    state: &AppState,
    auth: &AuthUser,
    message_id: &str,
    conversation_id: Option<String>,
    group_id: Option<String>,
    input: FeedbackInput,
) -> AppResult<Json<FeedbackResponse>> {
    let comment = validate(&input)?;
    let user_id = auth.user_id.to_string();
    enforce_feedback_limit(state, &user_id).await?;
    engine::require_enabled(&state.db, &user_id).await?;
    let (turn_id, eligible) = if let Some(cid) = conversation_id.as_deref() {
        let (conversation, _) = engine::get_authorized(&state.db, &user_id, cid).await?;
        engine::authorize_thread_read(&state.db, &conversation).await?;
        let row = feedback_service::conversation_message(&state.db, &user_id, cid, message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("message not found".into()))?;
        (
            Some(row.turn_id),
            row.role == "assistant" && row.status == "completed",
        )
    } else {
        let gid = group_id.as_deref().unwrap();
        let access =
            crate::services::org_group_service::get(&state.db, &user_id, gid, None).await?;
        let row = feedback_service::group_message(&state.db, gid, message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("message not found".into()))?;
        (None, row.role == "agent")
    };
    if !eligible {
        return Err(AppError::BadRequest("assistant_feedback_ineligible".into()));
    }
    let (row, existing) = feedback_service::upsert_feedback(
        &state.db,
        &user_id,
        message_id,
        conversation_id.clone(),
        group_id.clone(),
        turn_id.clone(),
        &input.vote,
        comment.clone(),
        None,
    )
    .await
    .map_err(AppError::from)?;
    if existing
        .as_ref()
        .map(|r| {
            r.vote.as_str() == input.vote.as_str()
                && (input.comment.is_none() || r.comment == comment)
        })
        .unwrap_or(false)
    {
        return Ok(Json(row.into()));
    }
    let event = if existing.is_none() {
        "submitted"
    } else if existing.as_ref().map(|r| r.vote.as_str()) != Some(input.vote.as_str()) {
        "changed"
    } else {
        "comment_updated"
    };
    tracing::info!(target: "nyxid::assistant_feedback", event, feedback_id = %row.id, user_id = %user_id, message_id, vote = %input.vote, previous_vote = existing.as_ref().map(|r| r.vote.as_str()).unwrap_or(""), has_comment = input.comment.is_some(), comment_chars = comment.as_ref().map(|v| v.chars().count()).unwrap_or(0), "assistant feedback state changed");
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        "assistant_feedback_submitted",
        Some(
            serde_json::json!({"message_id":message_id,"vote":input.vote,"conversation_id":conversation_id,"group_id":group_id}),
        ),
    );
    Ok(Json(row.into()))
}
pub async fn put_conversation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
    Json(input): Json<FeedbackInput>,
) -> AppResult<Json<FeedbackResponse>> {
    upsert(&state, &auth, &message_id, Some(id), None, input).await
}
pub async fn put_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
    Json(input): Json<FeedbackInput>,
) -> AppResult<Json<FeedbackResponse>> {
    upsert(&state, &auth, &message_id, None, Some(id), input).await
}
async fn withdraw(
    state: &AppState,
    auth: &AuthUser,
    message_id: String,
    scope: Option<String>,
    group: bool,
) -> AppResult<StatusCode> {
    let user_id = auth.user_id.to_string();
    enforce_feedback_limit(state, &user_id).await?;
    engine::require_enabled(&state.db, &user_id).await?;
    if let Some(cid) = scope.as_deref() {
        if group {
            crate::services::org_group_service::get(&state.db, &user_id, cid, None).await?;
            if feedback_service::group_message(&state.db, cid, &message_id)
                .await?
                .is_none()
            {
                return Err(AppError::NotFound("message not found".into()));
            }
        } else {
            let (conversation, _) = engine::get_authorized(&state.db, &user_id, cid).await?;
            engine::authorize_thread_read(&state.db, &conversation).await?;
            let row = feedback_service::conversation_message(&state.db, &user_id, cid, &message_id)
                .await?
                .ok_or_else(|| AppError::NotFound("message not found".into()))?;
            if row.role != "assistant" || row.status != "completed" {
                return Err(AppError::BadRequest("assistant_feedback_ineligible".into()));
            }
        }
    }
    let filter = doc! {"user_id":&user_id,"message_id":&message_id};
    let deleted = feedback_service::delete_feedback(&state.db, &user_id, &message_id).await?;
    if deleted.is_none() {
        return Ok(StatusCode::NO_CONTENT);
    }
    let context = deleted.as_ref().and_then(|row| row.context.as_ref());
    tracing::info!(target: "nyxid::assistant_feedback", event = "withdrawn", user_id = %user_id, message_id = %message_id,
        surface = ?context.and_then(|c| c.surface.as_deref()), agent_id = ?context.and_then(|c| c.agent_id.as_deref()), agent_kind = ?context.and_then(|c| c.agent_kind.as_deref()), model = ?context.and_then(|c| c.model.as_deref()), access_mode = ?context.and_then(|c| c.access_mode.as_deref()), reply_status = ?context.and_then(|c| c.reply_status.as_deref()), error_code = ?context.and_then(|c| c.error_code.as_deref()), origin = ?context.and_then(|c| c.origin.as_deref()), via = ?context.and_then(|c| c.via.as_deref()), turn_id = ?context.and_then(|c| c.turn_id.as_deref()), reply_seq = ?context.and_then(|c| c.reply_seq), latency_ms = ?context.and_then(|c| c.latency_ms), tool_calls = context.map(|c| c.tool_calls).unwrap_or(0), tool_errors = context.map(|c| c.tool_errors).unwrap_or(0), machine_used = context.map(|c| c.machine_used).unwrap_or(false), attachments = context.map(|c| c.attachments).unwrap_or(0), steered = context.map(|c| c.steered).unwrap_or(false), voice = context.map(|c| c.voice).unwrap_or(false), reply_chars = context.map(|c| c.reply_chars).unwrap_or(0), context_reset_since_reply = context.map(|c| c.context_reset_since_reply).unwrap_or(false), "assistant feedback state changed");
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        "assistant_feedback_withdrawn",
        Some(
            serde_json::json!({"message_id":message_id,"group":group,"conversation_id": if group { serde_json::Value::Null } else { serde_json::Value::String(scope.clone().unwrap_or_default()) },"group_id": if group { serde_json::Value::String(scope.clone().unwrap_or_default()) } else { serde_json::Value::Null }}),
        ),
    );
    Ok(StatusCode::NO_CONTENT)
}
use axum::http::StatusCode;
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    withdraw(&state, &auth, message_id, Some(id), false).await
}
pub async fn delete_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    withdraw(&state, &auth, message_id, Some(id), true).await
}

#[derive(Default, Deserialize)]
pub struct AdminQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub vote: Option<String>,
    pub cursor: Option<String>,
}
#[derive(Serialize)]
pub struct AdminResponse {
    pub counts: Vec<feedback_service::AdminCount>,
    pub recent: Vec<AdminFeedbackRow>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize)]
pub struct AdminFeedbackRow {
    pub message_id: String,
    pub vote: String,
    pub comment: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
pub async fn admin(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<AdminQuery>,
) -> AppResult<Json<AdminResponse>> {
    super::admin_helpers::require_admin_or_operator(&state, &auth, "admin.assistant.feedback")
        .await?;
    let mut filter = doc! {};
    if let Some(v) = q.vote {
        filter.insert("vote", v);
    }
    if let Some(f) = q.from {
        filter.insert("created_at", doc! {"$gte":bson::DateTime::from_chrono(f)});
    }
    if let Some(t) = q.to {
        filter
            .entry("created_at".to_string())
            .or_insert(doc! {}.into());
        if let Ok(b) = filter.get_document_mut("created_at") {
            b.insert("$lte", bson::DateTime::from_chrono(t));
        }
    }
    if let Some(cursor) = q.cursor.as_deref() {
        let (dt_raw, id) = cursor.split_once('|').unwrap_or(("", ""));
        let dt = (!dt_raw.is_empty())
            .then_some(dt_raw)
            .and_then(|v| DateTime::parse_from_rfc3339(v).ok());
        let id = (!id.is_empty()).then_some(id);
        let (dt, id) = match (dt, id) {
            (Some(dt), Some(id)) => (dt.with_timezone(&Utc), id),
            _ => return Err(AppError::BadRequest("feedback_cursor_invalid".into())),
        };
        filter.insert(
            "$or",
            vec![
                doc! {"created_at": {"$lt": bson::DateTime::from_chrono(dt)}},
                doc! {"created_at": bson::DateTime::from_chrono(dt), "_id": {"$lt": id}},
            ],
        );
    }
    let mut aggregate_filter = filter.clone();
    aggregate_filter.remove("$or");
    let counts = feedback_service::admin_counts(&state.db, aggregate_filter).await?;
    let rows = feedback_service::admin_recent(&state.db, filter, 100).await?;
    let recent = rows
        .iter()
        .map(|r| AdminFeedbackRow {
            message_id: r.message_id.clone(),
            vote: r.vote.clone(),
            comment: r.comment.clone(),
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    let next_cursor = (rows.len() == 100)
        .then(|| {
            rows.last().map(|r| {
                format!(
                    "{}|{}",
                    r.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    r.id
                )
            })
        })
        .flatten();
    Ok(Json(AdminResponse {
        counts,
        recent,
        next_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn per_user_feedback_burst_is_limited_independently() {
        let limiter = crate::mw::rate_limit::PerKeyRateLimiter::new(2, 60);
        assert!(
            limiter
                .check_shared("assistant-feedback:user-a")
                .await
                .unwrap()
        );
        assert!(
            limiter
                .check_shared("assistant-feedback:user-a")
                .await
                .unwrap()
        );
        assert!(
            !limiter
                .check_shared("assistant-feedback:user-a")
                .await
                .unwrap()
        );
        assert!(
            limiter
                .check_shared("assistant-feedback:user-b")
                .await
                .unwrap()
        );
    }

    #[test]
    fn validate_accepts_votes_and_bounds_comments() {
        let input = FeedbackInput {
            vote: "up".into(),
            comment: Some(format!("  {}  ", "x".repeat(1_000))),
        };
        let comment = validate(&input).unwrap().unwrap();
        assert_eq!(comment.len(), 1_000);
        assert!(comment.chars().all(|c| c == 'x'));
    }

    #[test]
    fn validate_rejects_comments_over_bound() {
        let error = validate(&FeedbackInput {
            vote: "up".into(),
            comment: Some("x".repeat(1_001)),
        })
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(code) if code == "feedback_comment_too_long"));
    }

    #[test]
    fn validate_rejects_unknown_vote_with_stable_code() {
        let error = validate(&FeedbackInput {
            vote: "maybe".into(),
            comment: None,
        })
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(code) if code == "feedback_vote_invalid"));
    }

    #[test]
    fn empty_comment_is_omitted() {
        assert!(
            validate(&FeedbackInput {
                vote: "down".into(),
                comment: Some("  \n".into())
            })
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn response_does_not_expose_owner_or_message_text() {
        let now = Utc::now();
        let response: FeedbackResponse = AssistantMessageFeedback {
            id: Uuid::new_v4().to_string(),
            user_id: "u".into(),
            conversation_id: Some("c".into()),
            group_id: None,
            message_id: "m".into(),
            turn_id: Some("t".into()),
            vote: "up".into(),
            comment: None,
            created_at: now,
            updated_at: now,
        }
        .into();
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json.get("vote").and_then(|v| v.as_str()), Some("up"));
        assert!(json.get("user_id").is_none());
        assert!(json.get("message_id").is_none());
    }
}

#[cfg(test)]
mod input_tests {
    use super::sanitize_comment;
    #[test]
    fn sanitize_comment_strips_controls_and_bidi() {
        assert_eq!(sanitize_comment("ok\u{0007}\u{202e}text"), "oktext");
    }
}
