use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::require_admin_or_operator;
use crate::models::billing_target::BillingServiceScope;
use crate::models::service_billing::BillingMetric;
use crate::models::usage_allowance::AllowanceRecurrence;
use crate::mw::auth::AuthUser;
use crate::services::billing;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CreditSummaryQuery {
    /// Personal owner when omitted; an organization id for an authorized
    /// organization member.
    pub owner_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminCreditSummaryResponse {
    /// Outstanding issued promotional credits, including reserved and
    /// pending-activation amounts. This excludes future scheduled issuance
    /// and exact usage-unit allowances.
    pub outstanding_micros: String,
    /// Unreserved, confirmed, unexpired issued value, before recipient-level
    /// suspension, rollout, plan, and service spending rules are applied.
    pub available_micros: String,
    /// Confirmed unheld value unavailable due to reclaim or exhausted service limits.
    pub unavailable_micros: String,
    pub reserved_micros: String,
    pub pending_activation_micros: String,
    pub expiring_within_30_days_micros: String,
    pub restricted_micros: String,
    /// Active grants whose wall-clock expiry has passed but whose terminal
    /// cleanup marker has not been written yet. These credits are excluded
    /// from outstanding and available totals.
    pub expired_awaiting_cleanup_micros: String,
    pub active_grant_count: u64,
    pub pending_activation_count: u64,
    pub recipient_count: u64,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserCreditSummaryResponse {
    pub owner_id: String,
    /// Exact spendable monetary credits. Values are strings so large owner
    /// balances remain lossless in JavaScript clients.
    pub available_micros: String,
    pub wallet_available_micros: String,
    pub issued_available_micros: String,
    pub issued_reserved_micros: String,
    pub pending_activation_micros: String,
    pub restricted_micros: String,
    pub restricted_grant_count: u64,
    pub wallet: Option<UserWalletSummaryResponse>,
    pub grants: Vec<UserGrantSummaryResponse>,
    pub allowances: Vec<UserAllowanceSummaryResponse>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserWalletSummaryResponse {
    pub plan_kind: crate::models::billing_wallet::PlanKind,
    pub collection_state: crate::models::billing_wallet::CollectionState,
    pub balance_credits: String,
    pub reserved_credits: String,
    pub pending_lago_debits: String,
    pub pending_topup_expiry_credits: String,
    /// Negative wallet availability is retained for diagnostics, while the
    /// exact spendable total uses zero as its lower bound.
    pub available_credits: String,
    pub spendable_credits: String,
    pub overdraft_cap_credits: String,
    pub has_payment_instrument: bool,
    pub suspended: bool,
    pub balance_synced_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserGrantSummaryResponse {
    pub id: String,
    pub allocated_micros: String,
    pub used_micros: String,
    pub reclaimed_micros: String,
    pub status: crate::models::credit_grant::CreditGrantStatus,
    pub revoke_requested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub schedule_id: Option<String>,
    pub period_start: Option<DateTime<Utc>>,
    pub service_limits: Vec<super::billing_credits::CreditServiceLimitResponse>,
    pub remaining_micros: String,
    pub reserved_micros: String,
    pub available_micros: String,
    pub scope: BillingServiceScope,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserAllowanceSummaryResponse {
    pub id: String,
    pub service_id: String,
    pub service_slug: String,
    pub metric: BillingMetric,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
    pub period_start: DateTime<Utc>,
    pub period_end: Option<DateTime<Utc>>,
    pub consumed_quantity: i64,
    pub reserved_quantity: i64,
    pub remaining_quantity: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/credits/summary",
    tag = "Admin Credits",
    responses((status = 200, body = AdminCreditSummaryResponse)),
    security(("bearer_auth" = []))
)]
pub async fn admin_credit_summary(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminCreditSummaryResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.credits.summary").await?;
    let summary = billing::summaries::admin_summary(&state.db, Utc::now()).await?;
    Ok(Json(AdminCreditSummaryResponse {
        outstanding_micros: summary.outstanding_micros.to_string(),
        available_micros: summary.available_micros.to_string(),
        reserved_micros: summary.reserved_micros.to_string(),
        unavailable_micros: summary.unavailable_micros.to_string(),
        pending_activation_micros: summary.pending_activation_micros.to_string(),
        expiring_within_30_days_micros: summary.expiring_micros.to_string(),
        restricted_micros: summary.restricted_micros.to_string(),
        expired_awaiting_cleanup_micros: summary.expired_awaiting_cleanup_micros.to_string(),
        active_grant_count: summary.active_grant_count,
        pending_activation_count: summary.pending_activation_count,
        recipient_count: summary.recipient_count,
        generated_at: Utc::now(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/credits",
    tag = "Billing",
    params(("owner_id" = Option<String>, Query, description = "Billing owner id; authorized organization members may select an organization")),
    responses((status = 200, body = UserCreditSummaryResponse)),
    security(("bearer_auth" = []))
)]
pub async fn user_credit_summary(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<CreditSummaryQuery>,
) -> AppResult<Json<UserCreditSummaryResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_benefit_read(&actor_id, query.owner_id.as_deref())
        .await?;
    super::billing::ensure_billing_rollout(&state, &owner.owner_id, &actor_id).await?;
    let now = Utc::now();
    let summary = billing::summaries::user_summary(&state.db, &owner.owner_id, now).await?;
    let wallet_available_micros = summary.wallet_available_micros;
    let available_micros = summary.available_micros;
    let wallet = summary
        .wallet
        .as_ref()
        .map(|wallet| UserWalletSummaryResponse {
            plan_kind: wallet.plan_kind,
            collection_state: wallet.collection_state,
            balance_credits: wallet.balance_credits.to_string(),
            reserved_credits: wallet.reserved_credits.to_string(),
            pending_lago_debits: wallet.pending_lago_debits.to_string(),
            pending_topup_expiry_credits: wallet.pending_topup_expiry_credits.to_string(),
            available_credits: summary.wallet_available_credits.to_string(),
            spendable_credits: summary.wallet_spendable_credits.to_string(),
            overdraft_cap_credits: wallet.overdraft_cap_credits.to_string(),
            has_payment_instrument: wallet.has_payment_instrument,
            suspended: wallet.is_suspended(),
            balance_synced_at: wallet.balance_synced_at,
        });
    let grants = summary
        .grants
        .iter()
        .map(|grant| {
            let remaining = grant.remaining_micros;
            let reserved = billing::service_limits::held_micros(grant);
            UserGrantSummaryResponse {
                id: grant.id.clone(),
                allocated_micros: grant.amount_micros.to_string(),
                used_micros: billing::grants::used_micros(grant).to_string(),
                reclaimed_micros: if grant.status
                    == crate::models::credit_grant::CreditGrantStatus::Revoked
                {
                    grant.terminal_amount_micros
                } else {
                    0
                }
                .to_string(),
                status: grant.status,
                revoke_requested_at: grant.revoke_requested_at,
                created_at: grant.created_at,
                schedule_id: grant
                    .schedule_origin
                    .as_ref()
                    .map(|o| o.schedule_id.clone()),
                period_start: grant.schedule_origin.as_ref().map(|o| o.period_start),
                service_limits: billing::service_limits::rules_with_commitments(grant)
                    .into_iter()
                    .map(super::billing_credits::service_limit_response)
                    .collect(),
                remaining_micros: remaining.to_string(),
                reserved_micros: reserved.to_string(),
                available_micros: billing::grants::eligible_available_micros(grant, now)
                    .to_string(),
                scope: grant.scope.clone(),
                expires_at: grant.expires_at,
            }
        })
        .collect();
    let allowances = summary
        .allowances
        .iter()
        .map(|(allowance, period)| UserAllowanceSummaryResponse {
            id: allowance.id.clone(),
            service_id: allowance.service_id.clone(),
            service_slug: allowance.service_slug.clone(),
            metric: allowance.metric,
            quantity: allowance.quantity,
            recurrence: allowance.recurrence,
            period_start: period.period_start,
            period_end: period.period_end,
            consumed_quantity: period.consumed_quantity,
            reserved_quantity: period.reserved_quantity,
            remaining_quantity: crate::services::billing::allowances::available_period_quantity(
                period,
            ),
        })
        .collect();

    Ok(Json(UserCreditSummaryResponse {
        owner_id: summary.owner_id,
        available_micros: available_micros.to_string(),
        wallet_available_micros: wallet_available_micros.to_string(),
        issued_available_micros: summary.available_grant_micros.to_string(),
        issued_reserved_micros: summary.reserved_grant_micros.to_string(),
        pending_activation_micros: summary.pending_activation_micros.to_string(),
        restricted_micros: summary.restricted_grant_micros.to_string(),
        restricted_grant_count: summary.restricted_grant_count,
        wallet,
        grants,
        allowances,
        generated_at: now,
    }))
}

#[cfg(test)]
mod tests {
    use axum::extract::{Query, State};
    use chrono::Utc;
    use mongodb::bson::doc;
    use uuid::Uuid;

    use super::{CreditSummaryQuery, admin_credit_summary, user_credit_summary};
    use crate::errors::AppError;
    use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant, CreditGrantStatus};
    use crate::models::feature_flag_override::{
        COLLECTION_NAME as FLAGS, FeatureFlagOverride, FlagTargetKind,
    };
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
    use crate::models::user::UserType;
    use crate::services::feature_flag_service::BILLING_FLAG_KEY;
    use crate::services::role_service;
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
    };

    #[tokio::test]
    async fn personal_summary_is_lossless_and_read_only() {
        let db = connect_test_database("credit_handler_personal_summary")
            .await
            .expect("handler integration test requires NYXID_TEST_DATABASE_URL");
        let actor_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let grant = CreditGrant {
            service_limits: Vec::new(),
            revoke_requested_at: None,
            revoke_requested_by: None,

            id: "handler-large-grant".to_string(),
            batch_id: "handler-batch".to_string(),
            schedule_origin: None,
            recipient_user_id: actor_id.clone(),
            target_kind: BillingTargetKind::SelectedUsers,
            amount_credits: i64::MAX,
            amount_micros: i64::MAX,
            remaining_micros: i64::MAX,
            reserved_micros: 0,
            scope: BillingServiceScope {
                all_services: true,
                service_ids: vec![],
                service_slugs: vec![],
            },
            expires_at: None,
            reason: None,
            granted_by: "admin".to_string(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: Some(now),
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now,
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        };
        db.collection::<CreditGrant>(GRANTS)
            .insert_one(&grant)
            .await
            .expect("insert grant");
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(BillingWallet {
                id: "handler-large-wallet".to_string(),
                owner_id: actor_id.clone(),
                lago_customer_id: actor_id.clone(),
                lago_wallet_id: None,
                lago_subscription_id: None,
                plan_kind: PlanKind::Prepaid,
                balance_credits: i64::MAX,
                reserved_credits: 0,
                pending_lago_debits: 0,
                pending_topup_expiry_credits: 0,
                has_payment_instrument: true,
                overdraft_cap_credits: i64::MAX,
                suspended: false,
                collection_state: CollectionState::Good,
                topup_expiry_checked_at: None,
                active_topup_expiry: None,
                balance_synced_at: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert wallet");
        let before_grants = db
            .collection::<mongodb::bson::Document>(GRANTS)
            .count_documents(doc! {})
            .await
            .expect("count grants before");
        let before_wallets = db
            .collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .expect("count wallets before");
        let state = test_app_state(db.clone());
        let response = user_credit_summary(
            State(state),
            test_auth_user(&actor_id),
            Query(CreditSummaryQuery::default()),
        )
        .await
        .expect("personal summary");
        assert_eq!(
            response.0.available_micros,
            (i128::from(i64::MAX) * 1_000_001).to_string()
        );
        assert_eq!(
            response.0.wallet.as_ref().unwrap().balance_credits,
            i64::MAX.to_string()
        );
        assert_eq!(
            response.0.wallet.as_ref().unwrap().spendable_credits,
            i64::MAX.to_string()
        );
        let after_grants = db
            .collection::<mongodb::bson::Document>(GRANTS)
            .count_documents(doc! {})
            .await
            .expect("count grants after");
        assert_eq!(before_grants, after_grants);
        let after_wallets = db
            .collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .expect("count wallets after");
        assert_eq!(before_wallets, after_wallets);
        assert_eq!(
            db.collection::<mongodb::bson::Document>(
                crate::models::usage_allowance_period::COLLECTION_NAME,
            )
            .count_documents(doc! {})
            .await
            .expect("count periods"),
            0
        );

        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .update_one(
                doc! { "_id": "handler-large-wallet" },
                doc! { "$set": { "suspended": true } },
            )
            .await
            .expect("suspend wallet");
        let suspended = user_credit_summary(
            State(test_app_state(db.clone())),
            test_auth_user(&actor_id),
            Query(CreditSummaryQuery::default()),
        )
        .await
        .expect("suspended personal summary");
        assert_eq!(suspended.0.available_micros, "0");
        assert_eq!(suspended.0.issued_available_micros, i64::MAX.to_string());
        assert_eq!(suspended.0.wallet.as_ref().unwrap().spendable_credits, "0");
    }

    #[tokio::test]
    async fn personal_summary_rejects_another_person_and_rollout_disabled_owner() {
        let db = connect_test_database("credit_handler_acl_rollout")
            .await
            .expect("handler integration test requires NYXID_TEST_DATABASE_URL");
        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&other_id, UserType::Person))
            .await
            .expect("insert other person");
        let state = test_app_state(db.clone());
        let forbidden = user_credit_summary(
            State(state.clone()),
            test_auth_user(&actor_id),
            Query(CreditSummaryQuery {
                owner_id: Some(other_id),
            }),
        )
        .await
        .expect_err("another person must not be readable");
        assert!(matches!(forbidden, AppError::Forbidden(_)));

        let now = Utc::now();
        db.collection::<FeatureFlagOverride>(FLAGS)
            .insert_one(FeatureFlagOverride {
                id: Uuid::new_v4().to_string(),
                org_user_id: None,
                flag_key: BILLING_FLAG_KEY.to_string(),
                target_kind: FlagTargetKind::Global,
                target_key: None,
                enabled: false,
                created_at: now,
                updated_at: now,
                updated_by: actor_id.clone(),
            })
            .await
            .expect("disable rollout");
        let gated = user_credit_summary(
            State(state),
            test_auth_user(&actor_id),
            Query(CreditSummaryQuery::default()),
        )
        .await
        .expect_err("rollout gate must deny summary");
        assert!(matches!(gated, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn organization_member_can_read_org_summary_but_non_member_cannot() {
        let db = connect_test_database("credit_handler_org_acl")
            .await
            .expect("handler integration test requires NYXID_TEST_DATABASE_URL");
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let outsider_id = Uuid::new_v4().to_string();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_many([
                test_user(&org_id, UserType::Org),
                test_user(&member_id, UserType::Person),
                test_user(&outsider_id, UserType::Person),
            ])
            .await
            .expect("insert org ACL users");
        db.collection(MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert org membership");
        let member_response = user_credit_summary(
            State(test_app_state(db.clone())),
            test_auth_user(&member_id),
            Query(CreditSummaryQuery {
                owner_id: Some(org_id.clone()),
            }),
        )
        .await
        .expect("authorized organization member read");
        assert_eq!(member_response.0.owner_id, org_id);
        let denied = user_credit_summary(
            State(test_app_state(db)),
            test_auth_user(&outsider_id),
            Query(CreditSummaryQuery {
                owner_id: Some(org_id),
            }),
        )
        .await
        .expect_err("non-member must not read organization summary");
        assert!(matches!(denied, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn admin_summary_allows_operator_read_and_denies_regular_user() {
        let db = connect_test_database("credit_handler_admin_acl")
            .await
            .expect("handler integration test requires NYXID_TEST_DATABASE_URL");
        role_service::seed_system_roles(&db)
            .await
            .expect("seed roles");
        let role_ids = role_service::get_platform_role_ids(&db)
            .await
            .expect("role ids");
        let operator_id = Uuid::new_v4().to_string();
        let regular_id = Uuid::new_v4().to_string();
        let mut operator = test_user(&operator_id, UserType::Person);
        operator.role_ids.push(role_ids.operator);
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_many([operator, test_user(&regular_id, UserType::Person)])
            .await
            .expect("insert admin test users");

        let operator_response = admin_credit_summary(
            State(test_app_state(db.clone())),
            test_auth_user(&operator_id),
        )
        .await
        .expect("operator read");
        assert_eq!(operator_response.0.outstanding_micros, "0");
        let denied = admin_credit_summary(State(test_app_state(db)), test_auth_user(&regular_id))
            .await
            .expect_err("regular user must not read admin summary");
        assert!(matches!(denied, AppError::Forbidden(_)));
    }
}
