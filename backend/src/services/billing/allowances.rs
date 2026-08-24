use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::billing_target::BillingTargetKind;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_allowance::{
    AllowanceRecurrence, COLLECTION_NAME as USAGE_ALLOWANCES, UsageAllowance,
};
use crate::models::usage_allowance_period::{
    COLLECTION_NAME as USAGE_ALLOWANCE_PERIODS, UsageAllowancePeriod,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};

pub const MAX_ALLOWANCE_QUANTITY: i64 = 1_000_000_000_000;
pub const MAX_ALLOWANCE_TARGET_USERS: usize = 500;

#[derive(Clone, Debug)]
pub struct CreateAllowanceInput {
    pub service_ref: String,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub created_by: String,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateAllowanceInput {
    pub service_ref: Option<String>,
    pub quantity: Option<i64>,
    pub recurrence: Option<AllowanceRecurrence>,
    pub target_kind: Option<BillingTargetKind>,
    pub target_user_ids: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllowanceWindow {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

pub async fn create_allowance(
    db: &mongodb::Database,
    input: CreateAllowanceInput,
) -> AppResult<UsageAllowance> {
    validate_quantity(input.quantity)?;
    validate_targets(db, input.target_kind, &input.target_user_ids).await?;
    let service = resolve_service(db, &input.service_ref).await?;
    let metric = super::metric_resolution::effective_platform_metric(&service);
    let now = Utc::now();
    let allowance = UsageAllowance {
        id: Uuid::new_v4().to_string(),
        service_id: service.id,
        service_slug: service.slug.clone(),
        metric,
        quantity: input.quantity,
        recurrence: input.recurrence,
        target_kind: input.target_kind,
        target_user_ids: input.target_user_ids,
        is_active: true,
        created_by: input.created_by,
        created_at: now,
        updated_at: now,
    };
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .insert_one(&allowance)
        .await?;
    Ok(allowance)
}

pub async fn update_allowance(
    db: &mongodb::Database,
    allowance_id: &str,
    input: UpdateAllowanceInput,
) -> AppResult<UsageAllowance> {
    let current = db
        .collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .find_one(doc! { "_id": allowance_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Usage allowance not found".to_string()))?;
    let service = match input.service_ref.as_deref() {
        Some(reference) => Some(resolve_service(db, reference).await?),
        None => None,
    };
    let quantity = input.quantity.unwrap_or(current.quantity);
    validate_quantity(quantity)?;
    let target_kind = input.target_kind.unwrap_or(current.target_kind);
    let target_user_ids = input
        .target_user_ids
        .unwrap_or_else(|| current.target_user_ids.clone());
    validate_targets(db, target_kind, &target_user_ids).await?;

    let mut set = doc! {
        "quantity": quantity,
        "target_kind": bson::to_bson(&target_kind).map_err(|error| {
            AppError::Internal(format!("failed to encode allowance target: {error}"))
        })?,
        "target_user_ids": bson::to_bson(&target_user_ids).map_err(|error| {
            AppError::Internal(format!("failed to encode allowance targets: {error}"))
        })?,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    if let Some(recurrence) = input.recurrence {
        set.insert(
            "recurrence",
            bson::to_bson(&recurrence).map_err(|error| {
                AppError::Internal(format!("failed to encode allowance recurrence: {error}"))
            })?,
        );
    }
    if let Some(is_active) = input.is_active {
        set.insert("is_active", is_active);
    }
    if let Some(service) = service {
        let metric = super::metric_resolution::effective_platform_metric(&service);
        set.insert("service_id", service.id);
        set.insert("service_slug", service.slug.clone());
        set.insert(
            "metric",
            bson::to_bson(&metric).map_err(|error| {
                AppError::Internal(format!("failed to encode allowance metric: {error}"))
            })?,
        );
    }
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .update_one(doc! { "_id": allowance_id }, doc! { "$set": set })
        .await?;
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .find_one(doc! { "_id": allowance_id })
        .await?
        .ok_or_else(|| AppError::Internal("updated usage allowance disappeared".to_string()))
}

pub async fn list_allowances(
    db: &mongodb::Database,
    include_disabled: bool,
) -> AppResult<Vec<UsageAllowance>> {
    let filter = if include_disabled {
        doc! {}
    } else {
        doc! { "is_active": true }
    };
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .find(filter)
        .sort(doc! { "created_at": -1, "_id": -1 })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub async fn list_current_for_user(
    db: &mongodb::Database,
    owner_user_id: &str,
    now: DateTime<Utc>,
) -> AppResult<Vec<(UsageAllowance, UsageAllowancePeriod)>> {
    let definitions: Vec<UsageAllowance> = db
        .collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .find(doc! {
            "is_active": true,
            "$or": [
                { "target_kind": "all_users" },
                {
                    "target_kind": "selected_users",
                    "target_user_ids": owner_user_id,
                },
            ],
        })
        .sort(doc! { "service_slug": 1, "created_at": 1 })
        .await?
        .try_collect()
        .await?;
    let mut balances = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let period = ensure_current_period(db, &definition, owner_user_id, now).await?;
        balances.push((definition, period));
    }
    Ok(balances)
}

pub async fn applicable_allowances(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: Option<&str>,
    service_slug: Option<&str>,
    metric: BillingMetric,
) -> AppResult<Vec<UsageAllowance>> {
    let mut service_match = Vec::new();
    if let Some(service_id) = service_id {
        service_match.push(doc! { "service_id": service_id });
    }
    if let Some(service_slug) = service_slug {
        service_match.push(doc! { "service_slug": service_slug });
    }
    if service_match.is_empty() {
        return Ok(Vec::new());
    }
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .find(doc! {
            "is_active": true,
            "metric": bson::to_bson(&metric).map_err(|error| {
                AppError::Internal(format!("failed to encode billing metric: {error}"))
            })?,
            "$and": [
                { "$or": service_match },
                { "$or": [
                    { "target_kind": "all_users" },
                    {
                        "target_kind": "selected_users",
                        "target_user_ids": owner_user_id,
                    },
                ] },
            ],
        })
        .sort(doc! { "created_at": 1 })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub async fn ensure_current_period(
    db: &mongodb::Database,
    allowance: &UsageAllowance,
    owner_user_id: &str,
    now: DateTime<Utc>,
) -> AppResult<UsageAllowancePeriod> {
    let window = allowance_window(allowance.recurrence, allowance.created_at, now);
    let period_id = period_id(&allowance.id, owner_user_id, window.start);
    let created_at = Utc::now();
    let period_end = window.end.map_or(bson::Bson::Null, |value| {
        bson::DateTime::from_chrono(value).into()
    });
    let update = vec![doc! { "$set": {
        "allowance_id": { "$ifNull": ["$allowance_id", &allowance.id] },
        "owner_user_id": { "$ifNull": ["$owner_user_id", owner_user_id] },
        "consumed_quantity": { "$ifNull": ["$consumed_quantity", 0_i64] },
        "reserved_quantity": { "$ifNull": ["$reserved_quantity", 0_i64] },
        // Admin reductions apply to the remaining current-period balance but
        // can never invalidate consumption or an in-flight reservation
        // admitted under the old definition.
        "total_quantity": { "$max": [
            allowance.quantity,
            { "$add": [
                { "$ifNull": ["$consumed_quantity", 0_i64] },
                { "$ifNull": ["$reserved_quantity", 0_i64] },
            ] },
        ] },
        "period_start": { "$ifNull": [
            "$period_start",
            bson::DateTime::from_chrono(window.start),
        ] },
        "period_end": period_end,
        "created_at": { "$ifNull": [
            "$created_at",
            bson::DateTime::from_chrono(created_at),
        ] },
        "updated_at": bson::DateTime::from_chrono(created_at),
    } }];
    let periods = db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS);
    let result = periods
        .update_one(doc! { "_id": &period_id }, update.clone())
        .upsert(true)
        .await;
    match result {
        Ok(_) => {}
        // Concurrent first use can make both callers decide to upsert the
        // deterministic period id. The loser retries against the row the
        // winner created, preserving an idempotent first-use path.
        Err(error) if is_duplicate_key_error(&error) => {
            periods
                .update_one(doc! { "_id": &period_id }, update)
                .await?;
        }
        Err(error) => return Err(error.into()),
    }
    periods
        .find_one(doc! { "_id": &period_id })
        .await?
        .ok_or_else(|| AppError::Internal("usage allowance period disappeared".to_string()))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

pub fn allowance_window(
    recurrence: AllowanceRecurrence,
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AllowanceWindow {
    if recurrence == AllowanceRecurrence::OneTime {
        return AllowanceWindow {
            start: created_at,
            end: None,
        };
    }
    let day = now.date_naive();
    let day_start = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight is valid"));
    match recurrence {
        AllowanceRecurrence::OneTime => unreachable!(),
        AllowanceRecurrence::Daily => AllowanceWindow {
            start: day_start,
            end: Some(day_start + Duration::days(1)),
        },
        AllowanceRecurrence::Weekly => {
            let start = day_start - Duration::days(i64::from(day.weekday().num_days_from_monday()));
            AllowanceWindow {
                start,
                end: Some(start + Duration::days(7)),
            }
        }
        AllowanceRecurrence::Monthly => {
            let start_date = day.with_day(1).expect("every month has a first day");
            let (next_year, next_month) = if day.month() == 12 {
                (day.year() + 1, 1)
            } else {
                (day.year(), day.month() + 1)
            };
            let next_date = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
                .expect("next month is valid");
            AllowanceWindow {
                start: Utc.from_utc_datetime(
                    &start_date.and_hms_opt(0, 0, 0).expect("midnight is valid"),
                ),
                end: Some(Utc.from_utc_datetime(
                    &next_date.and_hms_opt(0, 0, 0).expect("midnight is valid"),
                )),
            }
        }
    }
}

pub fn period_id(allowance_id: &str, owner_user_id: &str, start: DateTime<Utc>) -> String {
    format!(
        "{allowance_id}:{owner_user_id}:{}",
        start.timestamp_millis()
    )
}

pub fn available_period_quantity(period: &UsageAllowancePeriod) -> i64 {
    period
        .total_quantity
        .saturating_sub(period.consumed_quantity)
        .saturating_sub(period.reserved_quantity)
        .max(0)
}

fn validate_quantity(quantity: i64) -> AppResult<()> {
    if !(1..=MAX_ALLOWANCE_QUANTITY).contains(&quantity) {
        return Err(AppError::ValidationError(format!(
            "allowance quantity must be between 1 and {MAX_ALLOWANCE_QUANTITY}"
        )));
    }
    Ok(())
}

async fn validate_targets(
    db: &mongodb::Database,
    target_kind: BillingTargetKind,
    target_user_ids: &[String],
) -> AppResult<()> {
    if target_kind == BillingTargetKind::AllUsers {
        if !target_user_ids.is_empty() {
            return Err(AppError::ValidationError(
                "all-users allowances must not include target_user_ids".to_string(),
            ));
        }
        return Ok(());
    }
    if target_user_ids.is_empty() || target_user_ids.len() > MAX_ALLOWANCE_TARGET_USERS {
        return Err(AppError::ValidationError(format!(
            "selected allowances require 1-{MAX_ALLOWANCE_TARGET_USERS} target users"
        )));
    }
    let unique: std::collections::BTreeSet<&str> = target_user_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect();
    if unique.len() != target_user_ids.len() {
        return Err(AppError::ValidationError(
            "target_user_ids must be unique and non-empty".to_string(),
        ));
    }
    let ids: Vec<&str> = unique.into_iter().collect();
    let count = db
        .collection::<User>(USERS)
        .count_documents(doc! {
            "_id": { "$in": &ids },
            "is_active": true,
        })
        .await?;
    if count != ids.len() as u64 {
        return Err(AppError::ValidationError(
            "one or more target users do not exist or are inactive".to_string(),
        ));
    }
    Ok(())
}

async fn resolve_service(db: &mongodb::Database, reference: &str) -> AppResult<DownstreamService> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(AppError::ValidationError(
            "service_ref is required".to_string(),
        ));
    }
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "$or": [{ "_id": reference }, { "slug": reference }] })
        .await?
        .ok_or_else(|| AppError::ValidationError("allowance service was not found".to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use crate::models::billing_target::BillingTargetKind;
    use crate::models::user::{UserProfileConfig, UserType};
    use crate::test_utils::connect_test_database;

    use super::*;

    #[test]
    fn utc_windows_cover_daily_weekly_monthly_and_one_time() {
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 13, 45, 0).unwrap();
        let created = Utc.with_ymd_and_hms(2026, 7, 2, 5, 0, 0).unwrap();
        let daily = allowance_window(AllowanceRecurrence::Daily, created, now);
        assert_eq!(
            daily.start,
            Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap()
        );
        assert_eq!(
            daily.end,
            Some(Utc.with_ymd_and_hms(2026, 8, 22, 0, 0, 0).unwrap())
        );

        let weekly = allowance_window(AllowanceRecurrence::Weekly, created, now);
        assert_eq!(
            weekly.start,
            Utc.with_ymd_and_hms(2026, 8, 17, 0, 0, 0).unwrap()
        );
        assert_eq!(
            weekly.end,
            Some(Utc.with_ymd_and_hms(2026, 8, 24, 0, 0, 0).unwrap())
        );

        let monthly = allowance_window(AllowanceRecurrence::Monthly, created, now);
        assert_eq!(
            monthly.start,
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(
            monthly.end,
            Some(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap())
        );

        let one_time = allowance_window(AllowanceRecurrence::OneTime, created, now);
        assert_eq!(
            one_time,
            AllowanceWindow {
                start: created,
                end: None
            }
        );
    }

    #[test]
    fn december_monthly_window_rolls_into_next_year() {
        let now = Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 0).unwrap();
        let window = allowance_window(AllowanceRecurrence::Monthly, now, now);
        assert_eq!(
            window.end,
            Some(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap())
        );
    }

    #[tokio::test]
    async fn current_period_reduction_never_invalidates_used_or_reserved_units() {
        let Some(db) = connect_test_database("allowance_period_reduction_clamp").await else {
            return;
        };
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 13, 45, 0).unwrap();
        let mut allowance = UsageAllowance {
            id: "allowance-1".to_string(),
            service_id: "service-1".to_string(),
            service_slug: "llm-one".to_string(),
            metric: BillingMetric::Tokens,
            quantity: 10,
            recurrence: AllowanceRecurrence::Daily,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: Vec::new(),
            is_active: true,
            created_by: "admin-1".to_string(),
            created_at: now - Duration::days(2),
            updated_at: now,
        };
        let period = ensure_current_period(&db, &allowance, "owner-1", now)
            .await
            .expect("create period");
        db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .update_one(
                doc! { "_id": &period.id },
                doc! { "$set": { "consumed_quantity": 6_i64, "reserved_quantity": 3_i64 } },
            )
            .await
            .expect("seed period usage");
        allowance.quantity = 4;

        let clamped = ensure_current_period(&db, &allowance, "owner-1", now)
            .await
            .expect("refresh period");

        assert_eq!(clamped.total_quantity, 9);
        assert_eq!(clamped.consumed_quantity, 6);
        assert_eq!(clamped.reserved_quantity, 3);
        assert_eq!(available_period_quantity(&clamped), 0);
    }

    #[tokio::test]
    async fn concurrent_first_use_converges_on_one_period() {
        let Some(db) = connect_test_database("allowance_period_concurrent_first_use").await else {
            return;
        };
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 13, 45, 0).unwrap();
        let allowance = UsageAllowance {
            id: "allowance-concurrent".to_string(),
            service_id: "service-1".to_string(),
            service_slug: "llm-one".to_string(),
            metric: BillingMetric::Tokens,
            quantity: 100,
            recurrence: AllowanceRecurrence::Daily,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: Vec::new(),
            is_active: true,
            created_by: "admin-1".to_string(),
            created_at: now - Duration::days(2),
            updated_at: now,
        };
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let db = db.clone();
            let allowance = allowance.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                ensure_current_period(&db, &allowance, "owner-1", now).await
            }));
        }

        let mut ids = std::collections::BTreeSet::new();
        for task in tasks {
            ids.insert(task.await.expect("period task").expect("ensure period").id);
        }
        assert_eq!(ids.len(), 1);
        assert_eq!(
            db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
                .count_documents(doc! {})
                .await
                .expect("count periods"),
            1
        );
    }

    #[tokio::test]
    async fn selected_allowance_targets_accept_organization_owners() {
        let Some(db) = connect_test_database("allowance_org_target").await else {
            return;
        };
        let now = Utc::now();
        db.collection::<User>(USERS)
            .insert_one(User {
                id: "org-1".to_string(),
                email: "org-1@invalid.local".to_string(),
                password_hash: None,
                display_name: Some("Org One".to_string()),
                slug: Some("org-one".to_string()),
                avatar_url: None,
                email_verified: true,
                email_verification_token: None,
                password_reset_token: None,
                password_reset_expires_at: None,
                is_active: true,
                is_admin: false,
                is_operator: false,
                role_ids: Vec::new(),
                group_ids: Vec::new(),
                invite_code_id: None,
                mfa_enabled: false,
                social_provider: None,
                social_provider_id: None,
                user_type: UserType::Org,
                primary_org_id: None,
                created_at: now,
                updated_at: now,
                last_login_at: None,
                profile_config: UserProfileConfig::default(),
            })
            .await
            .expect("insert organization owner");

        validate_targets(
            &db,
            BillingTargetKind::SelectedUsers,
            &["org-1".to_string()],
        )
        .await
        .expect("organization owner is eligible");
    }
}
