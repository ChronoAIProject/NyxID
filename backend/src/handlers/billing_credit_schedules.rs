use std::collections::{BTreeSet, HashMap};

use axum::{
    Json,
    extract::{Path, State},
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
use crate::models::credit_schedule::{
    CreditExpiryPolicy, CreditSchedule, ScheduleRecurrence, ScheduleTiming,
};
use crate::models::credit_schedule_period::{
    COLLECTION_NAME as CREDIT_SCHEDULE_PERIODS, CreditSchedulePeriod, SchedulePeriodStatus,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, billing};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCreditScheduleRequest {
    pub amount: Option<crate::models::credits::Credits>,
    #[serde(default)]
    pub service_limits: Vec<crate::models::credit_service_limit::CreditServiceLimitInput>,
    pub request_id: Option<String>,
    /// Legacy whole-credit input. Exact `amount`, when present, takes precedence.
    #[serde(default)]
    pub amount_credits: i64,
    pub recurrence: ScheduleRecurrence,
    #[serde(default)]
    pub timing: ScheduleTiming,
    pub expiry: CreditExpiryPolicy,
    pub target_kind: BillingTargetKind,
    #[serde(default)]
    pub target_user_ids: Vec<String>,
    #[serde(default)]
    pub target_org_ids: Vec<String>,
    #[serde(default)]
    pub target_group_ids: Vec<String>,
    pub all_services: bool,
    #[serde(default)]
    pub service_refs: Vec<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCreditScheduleRequest {
    pub amount: Option<crate::models::credits::Credits>,
    pub service_limits: Option<Vec<crate::models::credit_service_limit::CreditServiceLimitInput>>,
    pub amount_credits: Option<i64>,
    pub expiry: Option<CreditExpiryPolicy>,
    pub target_kind: Option<BillingTargetKind>,
    pub target_user_ids: Option<Vec<String>>,
    pub target_org_ids: Option<Vec<String>>,
    pub target_group_ids: Option<Vec<String>>,
    pub all_services: Option<bool>,
    pub service_refs: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub reason: Option<Option<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreditSchedulePeriodResponse {
    pub service_limits: Vec<super::billing_credits::ServiceLimitResponse>,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub target_org_ids: Vec<String>,
    pub target_group_ids: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub status: SchedulePeriodStatus,
    pub disbursed_count: u64,
    pub amount_micros: i64,
    pub amount: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreditScheduleRecipientResponse {
    pub recipient_user_id: String,
    pub recipient_email: Option<String>,
    pub recipient_display_name: Option<String>,
    pub recipient_billing_enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreditScheduleResponse {
    pub service_limits: Vec<super::billing_credits::ServiceLimitResponse>,
    pub id: String,
    pub amount_credits: i64,
    pub amount_micros: i64,
    pub amount: String,
    pub recurrence: ScheduleRecurrence,
    pub timing: ScheduleTiming,
    pub expiry: CreditExpiryPolicy,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub target_org_ids: Vec<String>,
    pub target_group_ids: Vec<String>,
    pub scope: BillingServiceScope,
    pub reason: Option<String>,
    pub is_active: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_period_start: Option<DateTime<Utc>>,
    pub last_disbursed_at: Option<DateTime<Utc>>,
    pub skipped_periods: u64,
    pub current_period: Option<CreditSchedulePeriodResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipients: Option<Vec<CreditScheduleRecipientResponse>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreditScheduleListResponse {
    pub schedules: Vec<CreditScheduleResponse>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SeedCreditScheduleRequest {
    pub expected_updated_at: Option<DateTime<Utc>>,
    pub period_start: Option<DateTime<Utc>>,
    pub after_user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SeedCreditScheduleResponse {
    pub period_start: DateTime<Utc>,
    pub created_count: usize,
    pub existing_count: usize,
    pub pending_activation_count: usize,
    pub next_user_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/credits/schedules/{schedule_id}/seed",
    tag = "Admin Credits",
    params(("schedule_id" = String, Path)),
    request_body = SeedCreditScheduleRequest,
    responses((status = 200, body = SeedCreditScheduleResponse)),
    security(("bearer_auth" = []))
)]
pub async fn seed_schedule(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(schedule_id): Path<String>,
    Json(body): Json<SeedCreditScheduleRequest>,
) -> AppResult<Json<SeedCreditScheduleResponse>> {
    require_admin(&state, &auth_user).await?;
    let result = billing::schedules::seeding::seed_missing(
        &state.db,
        &schedule_id,
        billing::schedules::seeding::SeedScheduleInput {
            actor: Some(audit_service::AuditActor::from_auth_user(&auth_user)),
            expected_updated_at: body.expected_updated_at,
            period_start: body.period_start,
            after_user_id: body.after_user_id,
        },
        Utc::now(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.credit_schedule.seeded",
        Some(serde_json::json!({
            "schedule_id": schedule_id,
            "period_start": result.period_start,
            "created_count": result.created_count,
            "existing_count": result.existing_count,
            "pending_activation_count": result.pending_activation_count,
        })),
    );
    Ok(Json(SeedCreditScheduleResponse {
        period_start: result.period_start,
        created_count: result.created_count,
        existing_count: result.existing_count,
        pending_activation_count: result.pending_activation_count,
        next_user_id: result.next_user_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/credits/schedules",
    tag = "Admin Credits",
    request_body = CreateCreditScheduleRequest,
    responses((status = 200, body = CreditScheduleResponse)),
    security(("bearer_auth" = []))
)]
pub async fn create_schedule(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateCreditScheduleRequest>,
) -> AppResult<Json<CreditScheduleResponse>> {
    require_admin(&state, &auth_user).await?;
    if !body.service_limits.is_empty() || body.timing == ScheduleTiming::Recipient {
        super::billing_credits::require_allocation_capability(&state)?;
    }
    if body.timing == ScheduleTiming::Recipient {
        crate::services::security_readiness_service::require_serving(
            &state.db,
            billing::schedules::recipients::READINESS,
        )
        .await?;
    }
    let schedule = billing::schedules::create_schedule(
        &state.db,
        billing::schedules::CreateScheduleInput {
            amount: body.amount,
            service_limits: body.service_limits,
            request_id: body.request_id,
            amount_credits: body.amount_credits,
            recurrence: body.recurrence,
            timing: body.timing,
            expiry: body.expiry,
            target_kind: body.target_kind,
            target_user_ids: body.target_user_ids,
            target_org_ids: body.target_org_ids,
            target_group_ids: body.target_group_ids,
            all_services: body.all_services,
            service_refs: body.service_refs,
            reason: body.reason,
            created_by: auth_user.user_id.to_string(),
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.credit_schedule.created",
        Some(serde_json::json!({ "schedule_id": schedule.id,
            "target_kind": schedule.target_kind,
            "timing": schedule.timing,
            "recurrence": schedule.recurrence,
            "target_user_count": schedule.target_user_ids.len(),
            "target_org_count": schedule.target_org_ids.len(),
            "target_group_count": schedule.target_group_ids.len(),
        })),
    );
    Ok(Json(response_after_mutation(&state.db, schedule).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/credits/schedules",
    tag = "Admin Credits",
    responses((status = 200, body = CreditScheduleListResponse)),
    security(("bearer_auth" = []))
)]
pub async fn list_schedules(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<CreditScheduleListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.credits.schedules.list").await?;
    let rows = billing::schedules::list_schedules(&state.db, Utc::now()).await?;
    let schedules: Vec<&CreditSchedule> = rows.iter().map(|row| &row.schedule).collect();
    let (users, rollout) = rollout_context(&state.db, &schedules).await?;
    Ok(Json(CreditScheduleListResponse {
        schedules: rows
            .into_iter()
            .map(|row| schedule_response(row.schedule, row.current_period, &users, &rollout))
            .collect(),
    }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/admin/credits/schedules/{schedule_id}",
    tag = "Admin Credits",
    params(("schedule_id" = String, Path)),
    request_body = UpdateCreditScheduleRequest,
    responses((status = 200, body = CreditScheduleResponse)),
    security(("bearer_auth" = []))
)]
pub async fn update_schedule(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(schedule_id): Path<String>,
    Json(body): Json<UpdateCreditScheduleRequest>,
) -> AppResult<Json<CreditScheduleResponse>> {
    require_admin(&state, &auth_user).await?;
    if body.service_limits.is_some() {
        super::billing_credits::require_allocation_capability(&state)?;
    }
    if body.is_active == Some(true) {
        // Resuming is authoring: an older replica would walk a recipient
        // schedule as calendar periods and issue a second grant set.
        let recipient_schedule = state
            .db
            .collection::<CreditSchedule>(crate::models::credit_schedule::COLLECTION_NAME)
            .find_one(doc! { "_id": &schedule_id, "timing": "recipient" })
            .await?
            .is_some();
        // Gate every request that can enable recipient issuance. A concurrent
        // pause may happen before the service loads its own CAS snapshot.
        if recipient_schedule {
            super::billing_credits::require_allocation_capability(&state)?;
            crate::services::security_readiness_service::require_serving(
                &state.db,
                billing::schedules::recipients::READINESS,
            )
            .await?;
        }
    }
    let schedule = billing::schedules::update_schedule(
        &state.db,
        &schedule_id,
        billing::schedules::UpdateScheduleInput {
            amount: body.amount,
            service_limits: body.service_limits,
            amount_credits: body.amount_credits,
            expiry: body.expiry,
            target_kind: body.target_kind,
            target_user_ids: body.target_user_ids,
            target_org_ids: body.target_org_ids,
            target_group_ids: body.target_group_ids,
            all_services: body.all_services,
            service_refs: body.service_refs,
            reason: body.reason,
            is_active: body.is_active,
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "billing.credit_schedule.updated",
        Some(serde_json::json!({
            "schedule_id": schedule.id,
            "target_kind": schedule.target_kind,
            "timing": schedule.timing,
            "recurrence": schedule.recurrence,
            "target_user_count": schedule.target_user_ids.len(),
            "target_org_count": schedule.target_org_ids.len(),
            "target_group_count": schedule.target_group_ids.len(),
            "is_active": schedule.is_active,
        })),
    );
    Ok(Json(response_after_mutation(&state.db, schedule).await?))
}

async fn response_after_mutation(
    db: &mongodb::Database,
    schedule: CreditSchedule,
) -> AppResult<CreditScheduleResponse> {
    let period = current_period(db, &schedule).await?;
    let (users, rollout) = match rollout_context(db, &[&schedule]).await {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(%error, schedule_id = %schedule.id, "credit schedule rollout lookup failed");
            (HashMap::new(), HashMap::new())
        }
    };
    Ok(schedule_response(schedule, period, &users, &rollout))
}

async fn current_period(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
) -> AppResult<Option<CreditSchedulePeriod>> {
    let period = billing::schedules::schedule_period(schedule.recurrence, Utc::now());
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! {
            "_id": billing::schedules::period_id(&schedule.id, period.start),
        })
        .await
        .map_err(Into::into)
}

fn schedule_response(
    schedule: CreditSchedule,
    period: Option<CreditSchedulePeriod>,
    users: &HashMap<String, User>,
    rollout: &HashMap<String, bool>,
) -> CreditScheduleResponse {
    let recipients = (schedule.target_kind == BillingTargetKind::SelectedUsers).then(|| {
        schedule
            .target_user_ids
            .iter()
            .map(|id| CreditScheduleRecipientResponse {
                recipient_user_id: id.clone(),
                recipient_email: users.get(id).map(|user| user.email.clone()),
                recipient_display_name: users.get(id).and_then(|user| user.display_name.clone()),
                recipient_billing_enabled: rollout.get(id).copied().unwrap_or(false),
            })
            .collect()
    });
    CreditScheduleResponse {
        id: schedule.id,
        amount_credits: schedule.amount_credits,
        amount_micros: schedule.amount.display_micros(),
        amount: schedule.amount.to_string(),
        service_limits: schedule
            .service_limits
            .iter()
            .map(super::billing_credits::ServiceLimitResponse::from)
            .collect(),
        recurrence: schedule.recurrence,
        timing: schedule.timing,
        expiry: schedule.expiry,
        target_kind: schedule.target_kind,
        target_user_ids: schedule.target_user_ids,
        target_org_ids: schedule.target_org_ids,
        target_group_ids: schedule.target_group_ids,
        scope: schedule.scope,
        reason: schedule.reason,
        is_active: schedule.is_active,
        created_by: schedule.created_by,
        created_at: schedule.created_at,
        updated_at: schedule.updated_at,
        last_period_start: schedule.last_period_start,
        last_disbursed_at: schedule.last_disbursed_at,
        skipped_periods: schedule.skipped_periods,
        current_period: period.map(|period| CreditSchedulePeriodResponse {
            target_kind: period.target_kind,
            target_user_ids: period.target_user_ids,
            target_org_ids: period.target_org_ids,
            target_group_ids: period.target_group_ids,
            start: period.period_start,
            end: period.period_end,
            status: period.status,
            disbursed_count: period.disbursed_count,
            amount_micros: period.amount.display_micros(),
            amount: period.amount.to_string(),
            service_limits: period
                .service_limits
                .iter()
                .map(super::billing_credits::ServiceLimitResponse::from)
                .collect(),
            expires_at: period.expires_at,
            completed_at: period.completed_at,
        }),
        recipients,
    }
}

async fn rollout_context(
    db: &mongodb::Database,
    schedules: &[&CreditSchedule],
) -> AppResult<(HashMap<String, User>, HashMap<String, bool>)> {
    let ids: BTreeSet<String> = schedules
        .iter()
        .filter(|schedule| schedule.target_kind == BillingTargetKind::SelectedUsers)
        .flat_map(|schedule| schedule.target_user_ids.iter().cloned())
        .collect();
    if ids.is_empty() {
        return Ok((HashMap::new(), HashMap::new()));
    }
    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": ids.iter().collect::<Vec<_>>() } })
        .await?
        .try_collect()
        .await?;
    let rollout: Vec<(String, bool)> = futures::stream::iter(users.iter().cloned())
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
    Ok((
        users
            .into_iter()
            .map(|user| (user.id.clone(), user))
            .collect(),
        rollout.into_iter().collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::billing_credits::{GrantListQuery, admin_list_grants};
    use crate::models::audit_log::COLLECTION_NAME as AUDIT_LOG;
    use crate::models::billing_target::BillingTargetKind;
    use crate::models::credit_grant::{COLLECTION_NAME as CREDIT_GRANTS, CreditGrant};
    use crate::models::user::UserType;
    use crate::services::billing::grants::IssueCreditGrantInput;
    use crate::services::feature_flag_service::{self, BILLING_FLAG_KEY, FlagTarget};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::Query;
    use uuid::Uuid;

    async fn setup(prefix: &str) -> Option<(AppState, String, String, String)> {
        let db = connect_test_database(prefix).await?;
        role_service::seed_system_roles(&db).await.ok()?;
        let roles = role_service::get_platform_role_ids(&db).await.ok()?;
        let admin_id = Uuid::new_v4().to_string();
        let operator_id = Uuid::new_v4().to_string();
        let user_id = Uuid::new_v4().to_string();
        let mut admin = test_user(&admin_id, UserType::Person);
        admin.role_ids.push(roles.admin);
        let mut operator = test_user(&operator_id, UserType::Person);
        operator.role_ids.push(roles.operator);
        db.collection(USERS)
            .insert_many([admin, operator, test_user(&user_id, UserType::Person)])
            .await
            .ok()?;
        Some((test_app_state(db), admin_id, operator_id, user_id))
    }

    fn create_request(
        target_kind: BillingTargetKind,
        targets: Vec<String>,
    ) -> CreateCreditScheduleRequest {
        CreateCreditScheduleRequest {
            amount: None,
            service_limits: Vec::new(),
            request_id: None,
            amount_credits: 10,
            recurrence: ScheduleRecurrence::Monthly,
            timing: crate::models::credit_schedule::ScheduleTiming::Calendar,
            expiry: CreditExpiryPolicy::EndOfPeriod,
            target_kind,
            target_user_ids: targets,
            target_org_ids: Vec::new(),
            target_group_ids: Vec::new(),
            all_services: true,
            service_refs: Vec::new(),
            reason: Some("Monthly credits".to_string()),
        }
    }

    #[tokio::test]
    async fn recipient_schedules_require_the_new_fleet_readiness_even_with_allocations_enabled() {
        let (mut state, admin, _, _) = setup("schedule_recipient_readiness").await.unwrap();
        state.config.billing_allocations_enabled = true;
        for allocation_route in [false, true] {
            let mut body = create_request(BillingTargetKind::AllUsers, vec![]);
            body.timing = ScheduleTiming::Recipient;
            let result = if allocation_route {
                allocate_recurring(State(state.clone()), test_auth_user(&admin), Json(body)).await
            } else {
                create_schedule(State(state.clone()), test_auth_user(&admin), Json(body)).await
            };
            assert!(matches!(
                result,
                Err(AppError::SecurityReadinessUnavailable)
            ));
        }
        assert_eq!(
            state
                .db
                .collection::<CreditSchedule>(crate::models::credit_schedule::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        let mut body = create_request(BillingTargetKind::AllUsers, vec![]);
        body.timing = ScheduleTiming::Recipient;
        let Json(schedule) =
            allocate_recurring(State(state.clone()), test_auth_user(&admin), Json(body))
                .await
                .unwrap();
        assert_eq!(schedule.timing, ScheduleTiming::Recipient);
        // A concurrently serving legacy process must close authoring again.
        state
            .db
            .collection::<bson::Document>("security_replicas")
            .insert_one(doc! { "_id": "old-worker", "generation_id": "old-worker",
            "instance_name": "old-worker", "protocol": 1, "capabilities": [],
            "expected_replicas": ["test-backend"], "drained": true,
            "platform_policy_required": false, "observed_at": bson::DateTime::now() })
            .await
            .unwrap();
        let mut body = create_request(BillingTargetKind::AllUsers, vec![]);
        body.timing = ScheduleTiming::Recipient;
        assert!(matches!(
            create_schedule(State(state), test_auth_user(&admin), Json(body)).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
    }

    #[tokio::test]
    async fn resuming_a_recipient_schedule_requires_the_fleet_readiness_again() {
        let Some((mut state, admin, _, _)) = setup("schedule_recipient_resume").await else {
            return;
        };
        state.config.billing_allocations_enabled = true;
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        let mut body = create_request(BillingTargetKind::AllUsers, vec![]);
        body.timing = ScheduleTiming::Recipient;
        let Json(schedule) =
            create_schedule(State(state.clone()), test_auth_user(&admin), Json(body))
                .await
                .unwrap();
        let toggle = |is_active| UpdateCreditScheduleRequest {
            is_active: Some(is_active),
            ..Default::default()
        };
        let Json(paused) = update_schedule(
            State(state.clone()),
            test_auth_user(&admin),
            Path(schedule.id.clone()),
            Json(toggle(false)),
        )
        .await
        .unwrap();
        assert!(!paused.is_active);
        // A legacy process rejoins while the schedule is paused.
        state
            .db
            .collection::<bson::Document>("security_replicas")
            .insert_one(doc! { "_id": "old-worker", "generation_id": "old-worker",
            "instance_name": "old-worker", "protocol": 1, "capabilities": [],
            "expected_replicas": ["test-backend"], "drained": true,
            "platform_policy_required": false, "observed_at": bson::DateTime::now() })
            .await
            .unwrap();
        assert!(matches!(
            update_schedule(
                State(state.clone()),
                test_auth_user(&admin),
                Path(schedule.id.clone()),
                Json(toggle(true)),
            )
            .await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        let stored = state
            .db
            .collection::<CreditSchedule>(crate::models::credit_schedule::COLLECTION_NAME)
            .find_one(doc! { "_id": &schedule.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_active);
        // Pausing and sparse edits remain available while the fleet is mixed.
        let Json(repaused) = update_schedule(
            State(state),
            test_auth_user(&admin),
            Path(schedule.id),
            Json(toggle(false)),
        )
        .await
        .unwrap();
        assert!(!repaused.is_active);
    }

    #[tokio::test]
    async fn enabling_an_active_recipient_schedule_still_requires_authoring_readiness() {
        let (mut state, admin, _, _) = setup("schedule_active_enable_readiness").await.unwrap();
        state.config.billing_allocations_enabled = true;
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        let mut body = create_request(BillingTargetKind::AllUsers, vec![]);
        body.timing = ScheduleTiming::Recipient;
        let Json(schedule) =
            create_schedule(State(state.clone()), test_auth_user(&admin), Json(body))
                .await
                .unwrap();
        let enable = || UpdateCreditScheduleRequest {
            is_active: Some(true),
            ..Default::default()
        };
        // Even an apparently redundant enable can race a concurrent pause.
        state.config.billing_allocations_enabled = false;
        assert!(
            update_schedule(
                State(state.clone()),
                test_auth_user(&admin),
                Path(schedule.id.clone()),
                Json(enable())
            )
            .await
            .is_err()
        );
        state.config.billing_allocations_enabled = true;
        state
            .db
            .collection::<bson::Document>("security_replicas")
            .insert_one(doc! { "_id": "old-worker", "generation_id": "old-worker",
            "instance_name": "old-worker", "protocol": 1, "capabilities": [],
            "expected_replicas": ["test-backend"], "drained": true,
            "platform_policy_required": false, "observed_at": bson::DateTime::now() })
            .await
            .unwrap();
        assert!(matches!(
            update_schedule(
                State(state.clone()),
                test_auth_user(&admin),
                Path(schedule.id.clone()),
                Json(enable())
            )
            .await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        let stored = state
            .db
            .collection::<CreditSchedule>(crate::models::credit_schedule::COLLECTION_NAME)
            .find_one(doc! { "_id": &schedule.id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.is_active);
    }

    #[tokio::test]
    async fn seeding_is_admin_only_and_retries_do_not_refill_existing_grants() {
        let Some((state, admin_id, operator_id, _)) = setup("schedule_handler_seed").await else {
            return;
        };
        let Json(schedule) = create_schedule(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(create_request(BillingTargetKind::AllUsers, vec![])),
        )
        .await
        .unwrap();
        assert!(matches!(
            seed_schedule(
                State(state.clone()),
                test_auth_user(&operator_id),
                Path(schedule.id.clone()),
                Json(SeedCreditScheduleRequest::default()),
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        let Json(first) = seed_schedule(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(schedule.id.clone()),
            Json(SeedCreditScheduleRequest::default()),
        )
        .await
        .unwrap();
        assert_eq!(first.created_count, 3);
        let Json(retry) = seed_schedule(
            State(state),
            test_auth_user(&admin_id),
            Path(schedule.id),
            Json(SeedCreditScheduleRequest::default()),
        )
        .await
        .unwrap();
        assert_eq!((retry.created_count, retry.existing_count), (0, 3));
    }

    #[tokio::test]
    async fn schedule_crud_enforces_roles_writes_audits_and_rejects_recurrence_updates() {
        let Some((state, admin_id, operator_id, user_id)) = setup("schedule_handler_crud").await
        else {
            return;
        };
        let operator = test_auth_user(&operator_id);
        let user = test_auth_user(&user_id);
        let _ = list_schedules(State(state.clone()), operator.clone())
            .await
            .expect("operator lists schedules");
        assert!(matches!(
            list_schedules(State(state.clone()), user).await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            create_schedule(
                State(state.clone()),
                operator.clone(),
                Json(create_request(BillingTargetKind::AllUsers, Vec::new())),
            )
            .await,
            Err(AppError::Forbidden(_))
        ));

        let created_audit = audit_service::notify_on_audit_write_for_user(
            "billing.credit_schedule.created",
            &admin_id,
        );
        let Json(created) = create_schedule(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(create_request(BillingTargetKind::AllUsers, Vec::new())),
        )
        .await
        .expect("admin creates schedule");
        tokio::time::timeout(std::time::Duration::from_secs(2), created_audit)
            .await
            .expect("created audit timeout")
            .expect("created audit watcher");

        assert!(
            serde_json::from_value::<UpdateCreditScheduleRequest>(serde_json::json!({
                "recurrence": "daily"
            }))
            .is_err()
        );
        let updated_audit = audit_service::notify_on_audit_write_for_user(
            "billing.credit_schedule.updated",
            &admin_id,
        );
        let Json(updated) = update_schedule(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(created.id.clone()),
            Json(UpdateCreditScheduleRequest {
                is_active: Some(false),
                ..Default::default()
            }),
        )
        .await
        .expect("admin pauses schedule");
        assert!(!updated.is_active);
        tokio::time::timeout(std::time::Duration::from_secs(2), updated_audit)
            .await
            .expect("updated audit timeout")
            .expect("updated audit watcher");
        assert!(matches!(
            update_schedule(
                State(state.clone()),
                operator,
                Path(created.id),
                Json(UpdateCreditScheduleRequest {
                    is_active: Some(true),
                    ..Default::default()
                }),
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        assert_eq!(
            state
                .db
                .collection::<mongodb::bson::Document>(AUDIT_LOG)
                .count_documents(doc! { "event_type": { "$in": [
                    "billing.credit_schedule.created",
                    "billing.credit_schedule.updated",
                ] } })
                .await
                .expect("count schedule audits"),
            2
        );
    }

    #[tokio::test]
    async fn selected_schedule_reports_rollout_and_grant_listing_filters_by_schedule() {
        let Some((state, admin_id, _operator_id, user_id)) =
            setup("schedule_handler_rollout").await
        else {
            return;
        };
        crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        feature_flag_service::set_platform_override(
            &state.db,
            BILLING_FLAG_KEY,
            &FlagTarget::User(user_id.clone()),
            false,
            &admin_id,
        )
        .await
        .expect("disable recipient billing rollout");
        let Json(created) = create_schedule(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(create_request(
                BillingTargetKind::SelectedUsers,
                vec![user_id.clone()],
            )),
        )
        .await
        .expect("create selected schedule");
        let recipient = created
            .recipients
            .as_ref()
            .and_then(|recipients| recipients.first())
            .expect("selected rollout recipient");
        assert_eq!(recipient.recipient_user_id, user_id);
        assert!(!recipient.recipient_billing_enabled);

        billing::schedules::disburse_due(
            &state.db,
            Utc::now(),
            billing::schedules::MAX_RECIPIENTS_PER_TICK,
        )
        .await
        .expect("disburse selected schedule");
        billing::grants::issue_grants(
            &state.db,
            IssueCreditGrantInput {
                amount: None,
                service_limits: Vec::new(),
                request_id: None,
                amount_credits: 1,
                target_kind: BillingTargetKind::SelectedUsers,
                target_user_ids: vec![user_id],
                target_org_ids: Vec::new(),
                target_group_ids: Vec::new(),
                all_services: true,
                service_refs: Vec::new(),
                expires_at: None,
                reason: None,
                granted_by: admin_id.clone(),
            },
        )
        .await
        .expect("issue unrelated ordinary grant");

        let Json(filtered) = admin_list_grants(
            State(state.clone()),
            test_auth_user(&admin_id),
            Query(GrantListQuery {
                recipient_user_id: None,
                schedule_id: Some(created.id.clone()),
                page: Some(1),
                per_page: Some(50),
            }),
        )
        .await
        .expect("filter grants by schedule");
        assert_eq!(filtered.total, 1);
        assert_eq!(
            filtered.grants[0].schedule_id.as_deref(),
            Some(created.id.as_str())
        );
        assert!(filtered.grants[0].period_start.is_some());
        assert_eq!(
            state
                .db
                .collection::<CreditGrant>(CREDIT_GRANTS)
                .count_documents(doc! {})
                .await
                .expect("count scheduled and ordinary grants"),
            2
        );
    }
}

#[cfg(test)]
mod target_tests;

#[utoipa::path(post, path = "/api/v1/admin/credits/allocations/schedules", tag = "Admin Credits", request_body = CreateCreditScheduleRequest, responses((status = 200, body = CreditScheduleResponse)), security(("bearer_auth" = [])))]
pub async fn allocate_recurring(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateCreditScheduleRequest>,
) -> AppResult<Json<CreditScheduleResponse>> {
    super::billing_credits::require_allocation_capability(&state)?;
    create_schedule(State(state), auth, Json(body)).await
}
#[utoipa::path(patch, path = "/api/v1/admin/credits/allocations/schedules/{schedule_id}", tag = "Admin Credits", params(("schedule_id" = String, Path)), request_body = UpdateCreditScheduleRequest, responses((status = 200, body = CreditScheduleResponse)), security(("bearer_auth" = [])))]
pub async fn edit_allocation_recurring(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateCreditScheduleRequest>,
) -> AppResult<Json<CreditScheduleResponse>> {
    super::billing_credits::require_allocation_capability(&state)?;
    update_schedule(State(state), auth, Path(id), Json(body)).await
}
