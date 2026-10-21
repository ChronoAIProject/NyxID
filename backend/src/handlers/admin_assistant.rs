use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::admin_assistant_analytics as analytics,
};
use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
pub struct AnalyticsQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl AnalyticsQuery {
    fn window(&self, now: DateTime<Utc>) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
        let to = self.to.unwrap_or(now);
        let from = self.from.unwrap_or(to - chrono::Duration::days(7));
        if to <= from || to - from > chrono::Duration::days(31) {
            return Err(AppError::BadRequest(
                "Choose a positive range of at most 31 days.".into(),
            ));
        }
        Ok((from, to))
    }
}

#[derive(Serialize)]
pub struct AnalyticsResponse {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub generated_at: DateTime<Utc>,
    pub active_sessions: i64,
    pub active_accounts: i64,
    pub running_now: u64,
    pub outcomes: analytics::TurnCounts,
    pub daily_sessions: Vec<analytics::ActivityDay>,
    pub daily_turns: Vec<analytics::TurnDay>,
}

pub async fn analytics(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<AnalyticsQuery>,
) -> AppResult<Json<AnalyticsResponse>> {
    super::admin_helpers::require_admin_or_operator(&state, &auth, "admin.assistant.analytics")
        .await?;
    let now = Utc::now();
    let (from, to) = query.window(now)?;
    let summary = analytics::summary(&state.db, from, to, now).await?;
    Ok(Json(AnalyticsResponse {
        from,
        to,
        generated_at: now,
        active_sessions: summary.sessions,
        active_accounts: summary.accounts,
        running_now: summary.running_now,
        outcomes: summary.outcomes,
        daily_sessions: summary.daily_sessions,
        daily_turns: summary.daily_turns,
    }))
}

/// Kept behind the assistant admin/operator guard, independently of Usage Viewer.
pub async fn model_usage(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<crate::services::admin_usage_service::AdminUsageQuery>,
) -> AppResult<Json<crate::services::admin_usage_service::nyxbot::Response>> {
    super::admin_helpers::require_admin_or_operator(&state, &auth, "admin.assistant.model_usage")
        .await?;
    Ok(Json(
        crate::services::admin_usage_service::nyxbot::get(&state.db, query.validate(Utc::now())?)
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_analytics_window() {
        let now = Utc::now();
        assert_eq!(
            AnalyticsQuery::default().window(now).unwrap(),
            (now - chrono::Duration::days(7), now)
        );
        for from in [
            now,
            now + chrono::Duration::seconds(1),
            now - chrono::Duration::days(32),
        ] {
            assert!(
                AnalyticsQuery {
                    from: Some(from),
                    to: Some(now)
                }
                .window(now)
                .is_err()
            );
        }
    }
}
