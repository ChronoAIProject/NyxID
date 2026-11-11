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
    pub threshold: Option<u32>,
}

#[derive(Serialize)]
pub struct MediaDrainResponse {
    pub converted: u64,
}

/// Early queue conversion for a rollback drain; repeat until converted is zero.
pub async fn drain_channel_media(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<MediaDrainResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    let converted =
        crate::services::assistant_channel_media::queue::convert_queued(&state.db, Utc::now())
            .await?;
    Ok(Json(MediaDrainResponse { converted }))
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
    pub real_work: Option<analytics::real_work::Weekly>,
    pub real_work_unavailable: bool,
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
    super::admin_helpers::require_admin_usage_read(&state, &auth, "admin.assistant.analytics")
        .await?;
    let now = Utc::now();
    let (from, to) = query.window(now)?;
    let threshold = analytics::real_work::threshold(query.threshold)?;
    let (summary, real_work) = tokio::join!(
        analytics::summary(&state.db, from, to, now),
        analytics::real_work::weekly(&state.db, from, to, threshold),
    );
    let summary = summary?;
    let real_work = match real_work {
        Ok(work) => Some(work),
        Err(error) => {
            tracing::warn!(
                error_code = error.error_code(),
                "NyxBot real-work analytics unavailable"
            );
            None
        }
    };
    Ok(Json(AnalyticsResponse {
        from,
        to,
        real_work_unavailable: real_work.is_none(),
        real_work,
        generated_at: now,
        active_sessions: summary.sessions,
        active_accounts: summary.accounts,
        running_now: summary.running_now,
        outcomes: summary.outcomes,
        daily_sessions: summary.daily_sessions,
        daily_turns: summary.daily_turns,
    }))
}

/// Same read access as the rest of Usage, including the Usage Viewer role.
pub async fn model_usage(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<crate::services::admin_usage_service::AdminUsageQuery>,
) -> AppResult<Json<crate::services::admin_usage_service::nyxbot::Response>> {
    super::admin_helpers::require_admin_usage_read(&state, &auth, "admin.assistant.model_usage")
        .await?;
    Ok(Json(
        crate::services::admin_usage_service::nyxbot::get(&state.db, query.validate(Utc::now())?)
            .await?,
    ))
}

/// Same read audience as Usage. Internal classification never exposes content.
pub async fn internal_owners(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<analytics::real_work::InternalOwners>> {
    super::admin_helpers::require_admin_usage_read(
        &state,
        &auth,
        "admin.assistant.internal_owners",
    )
    .await?;
    Ok(Json(analytics::real_work::internal(&state.db).await?))
}

pub async fn set_internal_owners(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<analytics::real_work::InternalOwners>,
) -> AppResult<Json<analytics::real_work::InternalOwners>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_reconciliation_operator(&state, &auth).await?;
    Ok(Json(
        analytics::real_work::set_internal(
            &state.db,
            &crate::services::audit_service::AuditActor::from_auth_user(&auth),
            input,
        )
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
                    to: Some(now),
                    threshold: None,
                }
                .window(now)
                .is_err()
            );
        }
    }
}
