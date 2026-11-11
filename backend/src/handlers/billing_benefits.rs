//! Current benefit preview. History pagination never contributes to totals.
use super::billing_credits::{CreditGrantResponse, UserAllowanceBalanceResponse};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::billing,
};
use axum::{
    Json,
    extract::{Query, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BenefitsQuery {
    pub owner_id: Option<String>,
    pub page: Option<u32>,
}
#[derive(Serialize, ToSchema)]
pub struct BenefitsResponse {
    pub summary: billing::summaries::AllocationSummary,
    pub grants: Vec<CreditGrantResponse>,
    pub total_grants: u64,
    pub page: u32,
    pub per_page: u32,
    pub allowances: Vec<UserAllowanceBalanceResponse>,
}
#[utoipa::path(get, path = "/api/v1/billing/benefits", tag = "Billing", params(BenefitsQuery), responses((status = 200, body = BenefitsResponse)), security(("bearer_auth" = [])))]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<BenefitsQuery>,
) -> AppResult<Json<BenefitsResponse>> {
    let actor = auth.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_benefit_read(&actor, q.owner_id.as_deref())
        .await?;
    super::billing::ensure_billing_rollout(&state, &owner.owner_id, &actor).await?;
    let page = q.page.unwrap_or(1);
    if page == 0 || page > 100_000 {
        return Err(AppError::BadRequest(
            "page must be between 1 and 100000".into(),
        ));
    }
    let now = Utc::now();
    let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
        let summary =
            billing::summaries::allocations(&state.db, Some(&owner.owner_id), now).await?;
        let (grants, total_grants) = billing::grants::list_grants(
            &state.db,
            Some(&owner.owner_id),
            None,
            50,
            u64::from(page - 1) * 50,
        )
        .await?;
        let allowances =
            billing::allowances::list_current_for_user(&state.db, &owner.owner_id, now).await?;
        Ok::<_, AppError>(BenefitsResponse {
            summary,
            grants: grants
                .into_iter()
                .map(|g| super::billing_credits::grant_response(g, None, None))
                .collect::<AppResult<_>>()?,
            total_grants,
            page,
            per_page: 50,
            allowances: allowances
                .into_iter()
                .map(|(a, p)| super::billing_credits::user_allowance_response(a, p))
                .collect(),
        })
    })
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)??;
    Ok(Json(result))
}
