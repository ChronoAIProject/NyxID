use std::collections::HashSet;

use axum::{
    Json,
    extract::{Query, State},
    http::header,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
    services::{
        org_service, service_insights_activity, service_insights_billing, user_service_service,
    },
};

#[derive(Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct InsightsQuery {
    /// Comma-separated exact connection UUIDs, at most 100.
    pub ids: String,
    /// Optional managed agent key whose execution context should be explained.
    pub api_key_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ConnectionInsightResponse {
    pub service_id: String,
    pub billing: Option<service_insights_billing::ServiceBillingExplanation>,
    pub usage: Option<service_insights_activity::ConnectionActivity>,
}

#[derive(Serialize, ToSchema)]
pub struct InsightsResponse {
    pub connections: Vec<ConnectionInsightResponse>,
}

fn connection_ids(raw: &str) -> AppResult<HashSet<String>> {
    let parts: Vec<_> = raw.split(',').collect();
    if parts.is_empty() || parts.len() > 100 || raw.len() > 3_699 {
        return Err(AppError::ValidationError(
            "Choose between 1 and 100 connection IDs".into(),
        ));
    }
    parts
        .into_iter()
        .map(|id| {
            Uuid::parse_str(id)
                .map(|id| id.to_string())
                .map_err(|_| AppError::ValidationError("Connection IDs must be UUIDs".into()))
        })
        .collect()
}

#[utoipa::path(
    get,
    path = "/api/v1/service-insights",
    params(InsightsQuery),
    responses((status = 200, body = InsightsResponse), (status = 400, description = "Invalid connection IDs"), (status = 403, description = "Account management session required")),
    security(("bearer_auth" = [])),
    tag = "AI Services"
)]
pub async fn get_insights(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<InsightsQuery>,
) -> AppResult<(
    [(header::HeaderName, &'static str); 1],
    Json<InsightsResponse>,
)> {
    if !matches!(
        auth.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken | AuthMethod::Delegated
    ) {
        return Err(AppError::Forbidden(
            "Connection insights require an account management session".into(),
        ));
    }
    let requested = connection_ids(&query.ids)?;
    let agent_key_id = query
        .api_key_id
        .as_deref()
        .map(|id| {
            Uuid::parse_str(id)
                .map(|id| id.to_string())
                .map_err(|_| AppError::ValidationError("Agent key ID must be a UUID".into()))
        })
        .transpose()?;
    let actor = auth.user_id.to_string();
    let memberships = org_service::list_memberships_for_member(&state.db, &actor, false).await?;
    let services = user_service_service::list_user_services_with_sources_including_disabled(
        &state.db,
        &actor,
        &memberships,
    )
    .await?
    .into_iter()
    .filter(|row| requested.contains(&row.service.id))
    .filter(|row| auth.allow_all_services || auth.allowed_service_ids.contains(&row.service.id))
    .map(|row| row.service)
    .collect::<Vec<_>>();
    // Each optional projection can fail without suppressing the other. No cached
    // privilege-bearing result is substituted after an authorization failure.
    let billing_principal = auth.proxy_resolution_user_id();
    let (billing, activity) = tokio::join!(
        async {
            match agent_key_id.as_deref() {
                Some(key_id) => {
                    service_insights_billing::explain_for_agent_key(
                        &state.db,
                        &state.billing,
                        &actor,
                        &services,
                        key_id,
                    )
                    .await
                }
                None => {
                    service_insights_billing::explain_connections(
                        &state.db,
                        &state.billing,
                        &billing_principal,
                        &actor,
                        &services,
                    )
                    .await
                }
            }
        },
        service_insights_activity::insights(&state.db, &actor, &services),
    );
    if let Err(error) = &billing {
        tracing::warn!(%error, "Service billing insights unavailable");
    }
    if let Err(error) = &activity {
        tracing::warn!(%error, "Service caller insights unavailable");
    }
    let mut billing = billing.unwrap_or_default();
    let mut activity = activity.unwrap_or_default();
    let connections = services
        .into_iter()
        .map(|service| ConnectionInsightResponse {
            billing: billing.remove(&service.id),
            usage: activity.remove(&service.id),
            service_id: service.id,
        })
        .collect();
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(InsightsResponse { connections }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_insights_rejects_invalid_or_unbounded_ids() {
        for ids in [
            "".to_string(),
            "slug".into(),
            format!("{},", Uuid::new_v4()),
            vec![Uuid::new_v4().to_string(); 101].join(","),
        ] {
            assert!(connection_ids(&ids).is_err());
        }
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            connection_ids(&format!("{id},{id}")).unwrap(),
            HashSet::from([id])
        );
    }
}
