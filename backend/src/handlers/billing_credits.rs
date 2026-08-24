use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
use crate::models::credit_grant::{CreditGrant, CreditGrantStatus};
use crate::models::usage_allowance::{AllowanceRecurrence, UsageAllowance};
use crate::models::usage_allowance_period::UsageAllowancePeriod;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, billing};

#[derive(Debug, Deserialize, ToSchema)]
pub struct IssueGrantRequest {
    pub amount_credits: i64,
    pub target_kind: BillingTargetKind,
    #[serde(default)]
    pub target_user_ids: Vec<String>,
    pub all_services: bool,
    #[serde(default)]
    pub service_refs: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueGrantResponse {
    pub batch_id: String,
    pub created_count: usize,
    /// Recipients whose immutable issuance ledger entry was confirmed inline.
    pub activated_count: usize,
    /// Recipients that remain unspendable until the reconcile sweep journals
    /// their issuance. Large batches deliberately use this bounded path.
    pub pending_activation_count: usize,
    /// Per-recipient visibility and activation signals for the completed
    /// issuance. This lets the admin surface warn without consulting Lago.
    pub recipients: Vec<IssueGrantRecipientResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueGrantRecipientResponse {
    pub recipient_user_id: String,
    pub recipient_billing_enabled: bool,
    pub activation_state: CreditGrantActivationState,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditGrantActivationState {
    Active,
    PendingActivation,
}

#[derive(Debug, Deserialize)]
pub struct GrantListQuery {
    pub recipient_user_id: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct BillingBenefitsQuery {
    /// Personal owner when omitted; organization user id for an authorized member.
    pub owner_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreditGrantResponse {
    pub id: String,
    pub batch_id: String,
    pub recipient_user_id: String,
    pub recipient_email: Option<String>,
    pub recipient_display_name: Option<String>,
    /// Present on admin grant listings. User reads are already rollout-gated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_billing_enabled: Option<bool>,
    pub activation_state: CreditGrantActivationState,
    pub target_kind: BillingTargetKind,
    pub amount_credits: i64,
    pub amount_micros: i64,
    pub remaining_micros: i64,
    pub reserved_micros: i64,
    pub scope: BillingServiceScope,
    pub expires_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub granted_by: String,
    pub status: CreditGrantStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub expired_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreditGrantListResponse {
    pub grants: Vec<CreditGrantResponse>,
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAllowanceRequest {
    pub service_ref: String,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
    pub target_kind: BillingTargetKind,
    #[serde(default)]
    pub target_user_ids: Vec<String>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct UpdateAllowanceRequest {
    pub service_ref: Option<String>,
    pub quantity: Option<i64>,
    pub recurrence: Option<AllowanceRecurrence>,
    pub target_kind: Option<BillingTargetKind>,
    pub target_user_ids: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UsageAllowanceResponse {
    pub id: String,
    pub service_id: String,
    pub service_slug: String,
    pub metric: crate::models::service_billing::BillingMetric,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub is_active: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UsageAllowanceListResponse {
    pub allowances: Vec<UsageAllowanceResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserAllowanceBalanceResponse {
    pub allowance: UsageAllowanceResponse,
    pub period_start: DateTime<Utc>,
    pub period_end: Option<DateTime<Utc>>,
    pub consumed_quantity: i64,
    pub reserved_quantity: i64,
    pub remaining_quantity: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserAllowanceListResponse {
    pub allowances: Vec<UserAllowanceBalanceResponse>,
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/credits/grants",
    tag = "Admin Credits",
    request_body = IssueGrantRequest,
    responses((status = 200, body = IssueGrantResponse)),
    security(("bearer_auth" = []))
)]
/// Issue a grant batch. At most 50 recipients are activated inline; larger
/// batches are fully created but report pending activation until reconciliation
/// confirms each remaining hash-chain ledger entry.
pub async fn issue_grant(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<IssueGrantRequest>,
) -> AppResult<Json<IssueGrantResponse>> {
    require_admin(&state, &auth_user).await?;
    let grants = billing::grants::issue_grants(
        &state.db,
        billing::grants::IssueCreditGrantInput {
            amount_credits: body.amount_credits,
            target_kind: body.target_kind,
            target_user_ids: body.target_user_ids,
            all_services: body.all_services,
            service_refs: body.service_refs,
            expires_at: body.expires_at,
            reason: body.reason,
            granted_by: auth_user.user_id.to_string(),
        },
    )
    .await?;
    let batch_id = grants
        .first()
        .map(|grant| grant.batch_id.clone())
        .ok_or_else(|| AppError::Internal("credit grant batch was empty".to_string()))?;
    let activated_count = grants
        .iter()
        .filter(|grant| grant.issued_ledgered_at.is_some())
        .count();
    let rollout_by_recipient = match rollout_for_grants(&state.db, &grants).await {
        Ok(values) => values,
        Err(error) => {
            // The grants already exist at this point. Return a conservative
            // warning signal instead of turning a successful mutation into a
            // retryable-looking error that could duplicate the batch.
            tracing::warn!(%error, "credit grant recipient rollout lookup failed");
            HashMap::new()
        }
    };
    let recipients = grants
        .iter()
        .map(|grant| IssueGrantRecipientResponse {
            recipient_user_id: grant.recipient_user_id.clone(),
            recipient_billing_enabled: rollout_by_recipient
                .get(&grant.recipient_user_id)
                .copied()
                .unwrap_or(false),
            activation_state: activation_state(grant),
        })
        .collect();
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.credit_grant.issued",
        Some(serde_json::json!({
            "batch_id": batch_id,
            "recipient_count": grants.len(),
            "amount_credits_each": body.amount_credits,
        })),
    );
    Ok(Json(IssueGrantResponse {
        batch_id,
        created_count: grants.len(),
        activated_count,
        pending_activation_count: grants.len().saturating_sub(activated_count),
        recipients,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/credits/grants",
    tag = "Admin Credits",
    responses((status = 200, body = CreditGrantListResponse)),
    security(("bearer_auth" = []))
)]
pub async fn admin_list_grants(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<GrantListQuery>,
) -> AppResult<Json<CreditGrantListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.credits.grants.list").await?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 500);
    let (grants, total) = billing::grants::list_grants(
        &state.db,
        query.recipient_user_id.as_deref(),
        i64::from(per_page),
        u64::from((page - 1).saturating_mul(per_page)),
    )
    .await?;
    let users = users_for_grants(&state.db, &grants).await?;
    let rollout_by_recipient = rollout_for_users(&state.db, &users).await?;
    Ok(Json(CreditGrantListResponse {
        grants: grants
            .into_iter()
            .map(|grant| {
                let user = users.get(&grant.recipient_user_id);
                let recipient_billing_enabled = Some(
                    rollout_by_recipient
                        .get(&grant.recipient_user_id)
                        .copied()
                        .unwrap_or(false),
                );
                grant_response(grant, user, recipient_billing_enabled)
            })
            .collect(),
        page,
        per_page,
        total,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/credits/grants/{grant_id}",
    tag = "Admin Credits",
    params(("grant_id" = String, Path)),
    responses((status = 200, body = CreditGrantResponse)),
    security(("bearer_auth" = []))
)]
pub async fn revoke_grant(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(grant_id): Path<String>,
) -> AppResult<Json<CreditGrantResponse>> {
    require_admin(&state, &auth_user).await?;
    let grant = billing::grants::revoke_grant(&state.db, &grant_id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.credit_grant.revoked",
        Some(serde_json::json!({ "grant_id": grant_id })),
    );
    Ok(Json(grant_response(grant, None, None)))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/grants",
    tag = "Billing",
    params(("owner_id" = Option<String>, Query, description = "Billing owner id; authorized organization members may select an organization")),
    responses((status = 200, body = CreditGrantListResponse)),
    security(("bearer_auth" = []))
)]
pub async fn user_list_grants(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<BillingBenefitsQuery>,
) -> AppResult<Json<CreditGrantListResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_benefit_read(&actor_id, query.owner_id.as_deref())
        .await?;
    super::billing::ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;
    let grants =
        billing::grants::list_active_for_user(&state.db, &owner.owner_id, Utc::now()).await?;
    let total = grants.len() as u64;
    Ok(Json(CreditGrantListResponse {
        grants: grants
            .into_iter()
            .map(|grant| grant_response(grant, None, None))
            .collect(),
        page: 1,
        per_page: total.min(u64::from(u32::MAX)) as u32,
        total,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/credits/allowances",
    tag = "Admin Credits",
    request_body = CreateAllowanceRequest,
    responses((status = 200, body = UsageAllowanceResponse)),
    security(("bearer_auth" = []))
)]
pub async fn create_allowance(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateAllowanceRequest>,
) -> AppResult<Json<UsageAllowanceResponse>> {
    require_admin(&state, &auth_user).await?;
    let allowance = billing::allowances::create_allowance(
        &state.db,
        billing::allowances::CreateAllowanceInput {
            service_ref: body.service_ref,
            quantity: body.quantity,
            recurrence: body.recurrence,
            target_kind: body.target_kind,
            target_user_ids: body.target_user_ids,
            created_by: auth_user.user_id.to_string(),
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.usage_allowance.created",
        Some(serde_json::json!({ "allowance_id": allowance.id })),
    );
    Ok(Json(allowance_response(allowance)))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/credits/allowances",
    tag = "Admin Credits",
    responses((status = 200, body = UsageAllowanceListResponse)),
    security(("bearer_auth" = []))
)]
pub async fn admin_list_allowances(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UsageAllowanceListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.credits.allowances.list").await?;
    let allowances = billing::allowances::list_allowances(&state.db, true).await?;
    Ok(Json(UsageAllowanceListResponse {
        allowances: allowances.into_iter().map(allowance_response).collect(),
    }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/admin/credits/allowances/{allowance_id}",
    tag = "Admin Credits",
    params(("allowance_id" = String, Path)),
    request_body = UpdateAllowanceRequest,
    responses((status = 200, body = UsageAllowanceResponse)),
    security(("bearer_auth" = []))
)]
pub async fn update_allowance(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(allowance_id): Path<String>,
    Json(body): Json<UpdateAllowanceRequest>,
) -> AppResult<Json<UsageAllowanceResponse>> {
    require_admin(&state, &auth_user).await?;
    let allowance = billing::allowances::update_allowance(
        &state.db,
        &allowance_id,
        billing::allowances::UpdateAllowanceInput {
            service_ref: body.service_ref,
            quantity: body.quantity,
            recurrence: body.recurrence,
            target_kind: body.target_kind,
            target_user_ids: body.target_user_ids,
            is_active: body.is_active,
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.usage_allowance.updated",
        Some(serde_json::json!({
            "allowance_id": allowance_id,
            "is_active": allowance.is_active,
        })),
    );
    Ok(Json(allowance_response(allowance)))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/allowances",
    tag = "Billing",
    params(("owner_id" = Option<String>, Query, description = "Billing owner id; authorized organization members may select an organization")),
    responses((status = 200, body = UserAllowanceListResponse)),
    security(("bearer_auth" = []))
)]
pub async fn user_list_allowances(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<BillingBenefitsQuery>,
) -> AppResult<Json<UserAllowanceListResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_benefit_read(&actor_id, query.owner_id.as_deref())
        .await?;
    super::billing::ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;
    let balances =
        billing::allowances::list_current_for_user(&state.db, &owner.owner_id, Utc::now()).await?;
    Ok(Json(UserAllowanceListResponse {
        allowances: balances
            .into_iter()
            .map(|(allowance, period)| user_allowance_response(allowance, period))
            .collect(),
    }))
}

fn grant_response(
    grant: CreditGrant,
    user: Option<&User>,
    recipient_billing_enabled: Option<bool>,
) -> CreditGrantResponse {
    let activation_state = activation_state(&grant);
    CreditGrantResponse {
        id: grant.id,
        batch_id: grant.batch_id,
        recipient_user_id: grant.recipient_user_id,
        recipient_email: user.map(|value| value.email.clone()),
        recipient_display_name: user.and_then(|value| value.display_name.clone()),
        recipient_billing_enabled,
        activation_state,
        target_kind: grant.target_kind,
        amount_credits: grant.amount_credits,
        amount_micros: grant.amount_micros,
        remaining_micros: grant.remaining_micros,
        reserved_micros: grant.reserved_micros,
        scope: grant.scope,
        expires_at: grant.expires_at,
        reason: grant.reason,
        granted_by: grant.granted_by,
        status: grant.status,
        created_at: grant.created_at,
        updated_at: grant.updated_at,
        consumed_at: grant.consumed_at,
        expired_at: grant.expired_at,
        revoked_at: grant.revoked_at,
    }
}

fn allowance_response(allowance: UsageAllowance) -> UsageAllowanceResponse {
    UsageAllowanceResponse {
        id: allowance.id,
        service_id: allowance.service_id,
        service_slug: allowance.service_slug,
        metric: allowance.metric,
        quantity: allowance.quantity,
        recurrence: allowance.recurrence,
        target_kind: allowance.target_kind,
        target_user_ids: allowance.target_user_ids,
        is_active: allowance.is_active,
        created_by: allowance.created_by,
        created_at: allowance.created_at,
        updated_at: allowance.updated_at,
    }
}

fn user_allowance_response(
    allowance: UsageAllowance,
    period: UsageAllowancePeriod,
) -> UserAllowanceBalanceResponse {
    UserAllowanceBalanceResponse {
        allowance: allowance_response(allowance),
        period_start: period.period_start,
        period_end: period.period_end,
        consumed_quantity: period.consumed_quantity,
        reserved_quantity: period.reserved_quantity,
        remaining_quantity: billing::allowances::available_period_quantity(&period),
    }
}

async fn users_for_grants(
    db: &mongodb::Database,
    grants: &[CreditGrant],
) -> AppResult<HashMap<String, User>> {
    let ids: Vec<&str> = grants
        .iter()
        .map(|grant| grant.recipient_user_id.as_str())
        .collect();
    let mut users = HashMap::new();
    if ids.is_empty() {
        return Ok(users);
    }
    let mut cursor = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": ids } })
        .await?;
    while let Some(user) = cursor.try_next().await? {
        users.insert(user.id.clone(), user);
    }
    Ok(users)
}

fn activation_state(grant: &CreditGrant) -> CreditGrantActivationState {
    if grant.issued_ledgered_at.is_some() {
        CreditGrantActivationState::Active
    } else {
        CreditGrantActivationState::PendingActivation
    }
}

async fn rollout_for_grants(
    db: &mongodb::Database,
    grants: &[CreditGrant],
) -> AppResult<HashMap<String, bool>> {
    let users = users_for_grants(db, grants).await?;
    rollout_for_users(db, &users).await
}

async fn rollout_for_users(
    db: &mongodb::Database,
    users: &HashMap<String, User>,
) -> AppResult<HashMap<String, bool>> {
    let pairs: Vec<(String, bool)> = futures::stream::iter(users.values().cloned())
        .map(|user| {
            let db = db.clone();
            async move {
                let enabled =
                    crate::services::feature_flag_service::billing_recipient_rollout_enabled(
                        &db, &user,
                    )
                    .await?;
                Ok::<_, AppError>((user.id, enabled))
            }
        })
        .buffer_unordered(16)
        .try_collect()
        .await?;
    Ok(pairs.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::feature_flag_service::{self, BILLING_FLAG_KEY, FlagTarget};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, Query, State};
    use uuid::Uuid;

    async fn setup(prefix: &str) -> Option<(AppState, String, String, String)> {
        let db = connect_test_database(prefix).await?;
        role_service::seed_system_roles(&db).await.ok()?;
        let role_ids = role_service::get_platform_role_ids(&db).await.ok()?;
        let admin_id = Uuid::new_v4().to_string();
        let operator_id = Uuid::new_v4().to_string();
        let user_id = Uuid::new_v4().to_string();
        let mut admin = test_user(&admin_id, UserType::Person);
        admin.role_ids.push(role_ids.admin);
        let mut operator = test_user(&operator_id, UserType::Person);
        operator.role_ids.push(role_ids.operator);
        db.collection(USERS)
            .insert_many([admin, operator, test_user(&user_id, UserType::Person)])
            .await
            .ok()?;
        Some((test_app_state(db), admin_id, operator_id, user_id))
    }

    #[tokio::test]
    async fn operator_reads_credit_admin_surfaces_but_cannot_mutate_them() {
        let Some((state, _admin_id, operator_id, _user_id)) =
            setup("billing_credits_operator_access").await
        else {
            return;
        };
        let operator = test_auth_user(&operator_id);

        let _ = admin_list_grants(
            State(state.clone()),
            operator.clone(),
            Query(GrantListQuery {
                recipient_user_id: None,
                page: None,
                per_page: None,
            }),
        )
        .await
        .expect("operator grant GET should succeed");
        let _ = admin_list_allowances(State(state.clone()), operator.clone())
            .await
            .expect("operator allowance GET should succeed");

        let issue = issue_grant(
            State(state.clone()),
            operator.clone(),
            Json(IssueGrantRequest {
                amount_credits: 1,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
                all_services: true,
                service_refs: Vec::new(),
                expires_at: None,
                reason: None,
            }),
        )
        .await
        .expect_err("operator grant POST should be forbidden");
        let revoke = revoke_grant(
            State(state.clone()),
            operator.clone(),
            Path("grant-1".to_string()),
        )
        .await
        .expect_err("operator grant DELETE should be forbidden");
        let create = create_allowance(
            State(state.clone()),
            operator.clone(),
            Json(CreateAllowanceRequest {
                service_ref: "service-1".to_string(),
                quantity: 1,
                recurrence: AllowanceRecurrence::Daily,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
            }),
        )
        .await
        .expect_err("operator allowance POST should be forbidden");
        let update = update_allowance(
            State(state),
            operator,
            Path("allowance-1".to_string()),
            Json(UpdateAllowanceRequest {
                is_active: Some(false),
                ..Default::default()
            }),
        )
        .await
        .expect_err("operator allowance PATCH should be forbidden");

        for error in [issue, revoke, create, update] {
            assert!(matches!(error, AppError::Forbidden(_)));
        }
    }

    #[tokio::test]
    async fn user_benefit_reads_enforce_billing_rollout() {
        let Some((state, admin_id, _operator_id, user_id)) =
            setup("billing_credits_rollout_gate").await
        else {
            return;
        };
        feature_flag_service::set_platform_override(
            &state.db,
            BILLING_FLAG_KEY,
            &FlagTarget::User(user_id.clone()),
            false,
            &admin_id,
        )
        .await
        .expect("disable billing for user");
        let user = test_auth_user(&user_id);

        let grants = user_list_grants(
            State(state.clone()),
            user.clone(),
            Query(BillingBenefitsQuery::default()),
        )
        .await
        .expect_err("unflagged user grant read should be forbidden");
        let allowances =
            user_list_allowances(State(state), user, Query(BillingBenefitsQuery::default()))
                .await
                .expect_err("unflagged user allowance read should be forbidden");

        assert!(matches!(grants, AppError::Forbidden(_)));
        assert!(matches!(allowances, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn handler_validation_rejects_invalid_grant_and_allowance_payloads() {
        let Some((state, admin_id, _operator_id, _user_id)) =
            setup("billing_credits_handler_validation").await
        else {
            return;
        };
        let admin = test_auth_user(&admin_id);

        let grant = issue_grant(
            State(state.clone()),
            admin.clone(),
            Json(IssueGrantRequest {
                amount_credits: 0,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
                all_services: true,
                service_refs: Vec::new(),
                expires_at: None,
                reason: None,
            }),
        )
        .await
        .expect_err("zero-credit grant should fail validation");
        let allowance = create_allowance(
            State(state),
            admin,
            Json(CreateAllowanceRequest {
                service_ref: "service-1".to_string(),
                quantity: 0,
                recurrence: AllowanceRecurrence::Monthly,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
            }),
        )
        .await
        .expect_err("zero-unit allowance should fail validation");

        assert!(matches!(grant, AppError::ValidationError(_)));
        assert!(matches!(allowance, AppError::ValidationError(_)));
    }
}
