use chrono::{DateTime, Utc};
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
#[cfg(test)]
use crate::models::user::{COLLECTION_NAME as USERS, User};

pub const MAX_ALLOWANCE_QUANTITY: i64 = 1_000_000_000_000;

#[derive(Clone, Debug)]
pub struct CreateAllowanceInput {
    pub service_ref: String,
    pub metric: Option<BillingMetric>,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub target_org_ids: Vec<String>,
    pub target_group_ids: Vec<String>,
    pub created_by: String,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateAllowanceInput {
    pub service_ref: Option<String>,
    pub metric: Option<BillingMetric>,
    pub quantity: Option<i64>,
    pub recurrence: Option<AllowanceRecurrence>,
    pub target_kind: Option<BillingTargetKind>,
    pub target_user_ids: Option<Vec<String>>,
    pub target_org_ids: Option<Vec<String>>,
    pub target_group_ids: Option<Vec<String>>,
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
    let allowance = prepare_allowance(db, input).await?;
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .insert_one(&allowance)
        .await?;
    Ok(allowance)
}

async fn prepare_allowance(
    db: &mongodb::Database,
    input: CreateAllowanceInput,
) -> AppResult<UsageAllowance> {
    validate_quantity(input.quantity)?;
    super::targets::validate(
        db,
        input.target_kind,
        &input.target_user_ids,
        &input.target_org_ids,
        &input.target_group_ids,
    )
    .await?;
    let service = resolve_service(db, &input.service_ref).await?;
    let metric = super::metric_resolution::allowance_metric(&service, input.metric)?;
    let now = Utc::now();
    let allowance = UsageAllowance {
        bundle_id: None,
        id: Uuid::new_v4().to_string(),
        service_id: service.id,
        service_slug: service.slug.clone(),
        metric,
        quantity: input.quantity,
        recurrence: input.recurrence,
        target_kind: input.target_kind,
        target_user_ids: input.target_user_ids,
        target_org_ids: input.target_org_ids,
        target_group_ids: input.target_group_ids,
        is_active: true,
        created_by: input.created_by,
        created_at: now,
        updated_at: now,
    };
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
    let set = prepare_update(db, &current, input).await?;
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .update_one(doc! { "_id": allowance_id }, doc! { "$set": set })
        .await?;
    db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .find_one(doc! { "_id": allowance_id })
        .await?
        .ok_or_else(|| AppError::Internal("updated usage allowance disappeared".into()))
}

async fn prepare_update(
    db: &mongodb::Database,
    current: &UsageAllowance,
    input: UpdateAllowanceInput,
) -> AppResult<bson::Document> {
    let service = match input.service_ref.as_deref() {
        Some(reference) => Some(resolve_service(db, reference).await?),
        None if input.metric.is_some() => Some(resolve_service(db, &current.service_id).await?),
        None => None,
    };
    let quantity = input.quantity.unwrap_or(current.quantity);
    validate_quantity(quantity)?;
    let target_kind = input.target_kind.unwrap_or(current.target_kind);
    let [target_user_ids, target_org_ids, target_group_ids] = super::targets::updated_lists(
        current.target_kind,
        target_kind,
        [
            &current.target_user_ids,
            &current.target_org_ids,
            &current.target_group_ids,
        ],
        [
            input.target_user_ids,
            input.target_org_ids,
            input.target_group_ids,
        ],
    )?;
    super::targets::validate_existing(
        db,
        target_kind,
        &target_user_ids,
        &target_org_ids,
        &target_group_ids,
    )
    .await?;

    let mut set = doc! {
        "quantity": quantity,
        "target_kind": bson::to_bson(&target_kind).map_err(|error| {
            AppError::Internal(format!("failed to encode allowance target: {error}"))
        })?,
        "target_user_ids": bson::to_bson(&target_user_ids).map_err(|error| {
            AppError::Internal(format!("failed to encode allowance targets: {error}"))
        })?,
        "target_org_ids": &target_org_ids,
        "target_group_ids": &target_group_ids,
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
        let metric = if input.metric.is_none() && service.id == current.service_id {
            // Full-form saves from older clients repeat the service reference
            // without a metric. Preserve the allowance's existing unit.
            current.metric
        } else {
            super::metric_resolution::allowance_metric(&service, input.metric)?
        };
        set.insert("service_id", service.id);
        set.insert("service_slug", service.slug.clone());
        set.insert(
            "metric",
            bson::to_bson(&metric).map_err(|error| {
                AppError::Internal(format!("failed to encode allowance metric: {error}"))
            })?,
        );
    }
    Ok(set)
}

#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AllowanceUnitInput {
    pub metric: BillingMetric,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
}

#[derive(Clone, Debug)]
pub struct AllowanceBundleInput {
    pub service_ref: String,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub target_org_ids: Vec<String>,
    pub target_group_ids: Vec<String>,
    pub units: Vec<AllowanceUnitInput>,
    pub created_by: String,
}

async fn validate_bundle(
    db: &mongodb::Database,
    input: &AllowanceBundleInput,
) -> AppResult<DownstreamService> {
    if !(1..=16).contains(&input.units.len()) {
        return Err(AppError::ValidationError(
            "Select 1 to 16 allowance units".into(),
        ));
    }
    let service = resolve_service(db, &input.service_ref).await?;
    let mut metrics = std::collections::HashSet::new();
    for unit in &input.units {
        if !metrics.insert(unit.metric.as_str()) {
            return Err(AppError::ValidationError(
                "Allowance units must have unique metrics".into(),
            ));
        }
        validate_quantity(unit.quantity)?;
        super::metric_resolution::allowance_metric(&service, Some(unit.metric))?;
    }
    super::targets::validate(
        db,
        input.target_kind,
        &input.target_user_ids,
        &input.target_org_ids,
        &input.target_group_ids,
    )
    .await?;
    Ok(service)
}

fn bundle_filter(key: &str) -> bson::Document {
    doc! { "$or": [{ "bundle_id": key }, { "_id": key, "bundle_id": null }] }
}

pub async fn create_allowance_bundle(
    db: &mongodb::Database,
    input: AllowanceBundleInput,
) -> AppResult<Vec<UsageAllowance>> {
    validate_bundle(db, &input).await?;
    let bundle_id = Uuid::new_v4().to_string();
    let mut rows = Vec::new();
    for unit in &input.units {
        let mut row = prepare_allowance(
            db,
            CreateAllowanceInput {
                service_ref: input.service_ref.clone(),
                metric: Some(unit.metric),
                quantity: unit.quantity,
                recurrence: unit.recurrence,
                target_kind: input.target_kind,
                target_user_ids: input.target_user_ids.clone(),
                target_org_ids: input.target_org_ids.clone(),
                target_group_ids: input.target_group_ids.clone(),
                created_by: input.created_by.clone(),
            },
        )
        .await?;
        row.bundle_id = Some(bundle_id.clone());
        rows.push(row);
    }
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let inserted = rows.clone();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<()> = async {
                db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
                    .insert_many(&inserted)
                    .session(session)
                    .await?;
                Ok(())
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    Ok(rows)
}

/// Like the repository's other atomic multi-document mutations, bundle writes
/// require transactions and fail closed on standalone MongoDB.
pub async fn replace_allowance_bundle(
    db: &mongodb::Database,
    key: &str,
    input: AllowanceBundleInput,
) -> AppResult<Vec<UsageAllowance>> {
    let service = validate_bundle(db, &input).await?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let key = key.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<Vec<UsageAllowance>> = async {
                let collection = db.collection::<UsageAllowance>(USAGE_ALLOWANCES);
                let mut cursor = collection
                    .find(bundle_filter(&key))
                    .session(&mut *session)
                    .await?;
                let current: Vec<UsageAllowance> =
                    cursor.stream(&mut *session).try_collect().await?;
                if current.is_empty() {
                    return Err(AppError::NotFound(
                        "Usage allowance bundle not found".into(),
                    ));
                }
                if current.iter().any(|row| row.service_id != service.id) {
                    return Err(AppError::ValidationError(
                        "A bundle's service cannot be changed; create a new bundle".into(),
                    ));
                }
                let mut result = Vec::new();
                for row in &current {
                    let unit = input.units.iter().find(|unit| unit.metric == row.metric);
                    let mut set = prepare_update(
                        &db,
                        row,
                        UpdateAllowanceInput {
                            quantity: unit.map(|u| u.quantity),
                            recurrence: unit.map(|u| u.recurrence),
                            target_kind: Some(input.target_kind),
                            target_user_ids: Some(input.target_user_ids.clone()),
                            target_org_ids: Some(input.target_org_ids.clone()),
                            target_group_ids: Some(input.target_group_ids.clone()),
                            is_active: Some(unit.is_some()),
                            ..Default::default()
                        },
                    )
                    .await?;
                    set.insert("bundle_id", &key);
                    collection
                        .update_one(doc! { "_id": &row.id }, doc! { "$set": set })
                        .session(&mut *session)
                        .await?;
                    result.push(
                        collection
                            .find_one(doc! { "_id": &row.id })
                            .session(&mut *session)
                            .await?
                            .ok_or_else(|| AppError::Internal("Allowance disappeared".into()))?,
                    );
                }
                for unit in &input.units {
                    if current.iter().any(|row| row.metric == unit.metric) {
                        continue;
                    }
                    let mut row = prepare_allowance(
                        &db,
                        CreateAllowanceInput {
                            service_ref: service.id.clone(),
                            metric: Some(unit.metric),
                            quantity: unit.quantity,
                            recurrence: unit.recurrence,
                            target_kind: input.target_kind,
                            target_user_ids: input.target_user_ids.clone(),
                            target_org_ids: input.target_org_ids.clone(),
                            target_group_ids: input.target_group_ids.clone(),
                            created_by: input.created_by.clone(),
                        },
                    )
                    .await?;
                    row.bundle_id = Some(key.to_owned());
                    collection.insert_one(&row).session(&mut *session).await?;
                    result.push(row);
                }
                Ok(result)
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn set_bundle_active(
    db: &mongodb::Database,
    key: &str,
    active: bool,
) -> AppResult<Vec<UsageAllowance>> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let key = key.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<Vec<UsageAllowance>> = async {
            let collection = db.collection::<UsageAllowance>(USAGE_ALLOWANCES);
            let result = collection.update_many(bundle_filter(&key), doc! { "$set": { "is_active": active, "updated_at": bson::DateTime::from_chrono(Utc::now()) } }).session(&mut *session).await?;
            if result.matched_count == 0 { return Err(AppError::NotFound("Usage allowance bundle not found".into())); }
            let mut cursor = collection.find(bundle_filter(&key)).session(&mut *session).await?;
            Ok(cursor.stream(&mut *session).try_collect().await?)
        }.await;
        crate::services::api_key_mutation_service::transaction_result(operation)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
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
            "$or": super::targets::allowance_clauses(db, owner_user_id).await?,
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
                { "$or": super::targets::allowance_clauses(db, owner_user_id).await? },
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
    let recurrence = match recurrence {
        AllowanceRecurrence::OneTime => unreachable!(),
        AllowanceRecurrence::Daily => super::periods::RecurringUtcPeriod::Daily,
        AllowanceRecurrence::Weekly => super::periods::RecurringUtcPeriod::Weekly,
        AllowanceRecurrence::Monthly => super::periods::RecurringUtcPeriod::Monthly,
    };
    let window = super::periods::recurring_utc_window(recurrence, now);
    AllowanceWindow {
        start: window.start,
        end: Some(window.end),
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
    use chrono::{Duration, TimeZone};

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
            bundle_id: None,
            id: "allowance-1".to_string(),
            service_id: "service-1".to_string(),
            service_slug: "llm-one".to_string(),
            metric: BillingMetric::Tokens,
            quantity: 10,
            recurrence: AllowanceRecurrence::Daily,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: Vec::new(),
            target_org_ids: Vec::new(),
            target_group_ids: Vec::new(),
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
            bundle_id: None,
            id: "allowance-concurrent".to_string(),
            service_id: "service-1".to_string(),
            service_slug: "llm-one".to_string(),
            metric: BillingMetric::Tokens,
            quantity: 100,
            recurrence: AllowanceRecurrence::Daily,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: Vec::new(),
            target_org_ids: Vec::new(),
            target_group_ids: Vec::new(),
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

        super::super::targets::validate(
            &db,
            BillingTargetKind::SelectedUsers,
            &["org-1".to_string()],
            &[],
            &[],
        )
        .await
        .expect("organization owner is eligible");
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    use crate::test_utils::{
        connect_transaction_test_database, test_auto_connected_catalog_service,
    };

    async fn setup() -> (mongodb::Database, AllowanceBundleInput) {
        let db = connect_transaction_test_database("allowance_bundles").await;
        let mut service = test_auto_connected_catalog_service();
        service.billing = Some(serde_json::from_value(serde_json::json!({
            "byok_pricing": { "metric": "input_tokens", "credits_per_unit": "1", "sync_status": "synced",
                "components": [{ "metric": "output_tokens", "credits_per_unit": "2", "sync_status": "synced" }, { "metric": "images", "credits_per_unit": "1", "sync_status": "synced" }] }
        })).unwrap());
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        (
            db,
            AllowanceBundleInput {
                service_ref: service.id,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: vec![],
                target_org_ids: vec![],
                target_group_ids: vec![],
                created_by: "admin".into(),
                units: vec![
                    AllowanceUnitInput {
                        metric: BillingMetric::InputTokens,
                        quantity: 100,
                        recurrence: AllowanceRecurrence::Daily,
                    },
                    AllowanceUnitInput {
                        metric: BillingMetric::OutputTokens,
                        quantity: 200,
                        recurrence: AllowanceRecurrence::Monthly,
                    },
                ],
            },
        )
    }

    #[tokio::test]
    async fn bundle_writes_fail_closed_on_standalone() {
        let db = crate::test_utils::connect_test_database("bundle_standalone")
            .await
            .unwrap();
        if super::super::usage_rollup::supports_transactions(&db)
            .await
            .unwrap()
        {
            db.drop().await.unwrap();
            return;
        }
        let service = test_auto_connected_catalog_service();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        let input = AllowanceBundleInput {
            service_ref: service.id.clone(),
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: vec![],
            target_org_ids: vec![],
            target_group_ids: vec![],
            created_by: "admin".into(),
            units: vec![AllowanceUnitInput {
                metric: BillingMetric::Requests,
                quantity: 10,
                recurrence: AllowanceRecurrence::Daily,
            }],
        };
        assert!(matches!(
            create_allowance_bundle(&db, input.clone()).await,
            Err(AppError::DatabaseError(_))
        ));
        assert!(list_allowances(&db, true).await.unwrap().is_empty());
        let row = create_allowance(
            &db,
            CreateAllowanceInput {
                service_ref: service.id,
                metric: Some(BillingMetric::Requests),
                quantity: 10,
                recurrence: AllowanceRecurrence::Daily,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: vec![],
                target_org_ids: vec![],
                target_group_ids: vec![],
                created_by: "admin".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            replace_allowance_bundle(&db, &row.id, input).await,
            Err(AppError::DatabaseError(_))
        ));
        assert!(matches!(
            set_bundle_active(&db, &row.id, false).await,
            Err(AppError::DatabaseError(_))
        ));
        let saved = list_allowances(&db, true).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].is_active);
        assert_eq!(saved[0].bundle_id, None);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn bundle_create_replace_toggle_preserves_row_and_period_identities() {
        let (db, mut input) = setup().await;
        let created = create_allowance_bundle(&db, input.clone()).await.unwrap();
        let key = created[0].bundle_id.clone().unwrap();
        assert!(Uuid::parse_str(&key).unwrap().get_version_num() == 4);
        assert!(created.iter().all(|r| r.bundle_id.as_deref() == Some(&key)));
        let period = ensure_current_period(&db, &created[0], "owner", Utc::now())
            .await
            .unwrap();
        input.units[0].quantity = 50;
        input.units[1].metric = BillingMetric::Images;
        let updated = replace_allowance_bundle(&db, &key, input).await.unwrap();
        assert_eq!(updated.len(), 3);
        assert_eq!(
            updated
                .iter()
                .find(|r| r.metric == BillingMetric::InputTokens)
                .unwrap()
                .id,
            created[0].id
        );
        assert!(
            !updated
                .iter()
                .find(|r| r.metric == BillingMetric::OutputTokens)
                .unwrap()
                .is_active
        );
        let unchanged = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .find_one(doc! { "_id": &period.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.allowance_id, created[0].id);
        assert!(
            set_bundle_active(&db, &key, false)
                .await
                .unwrap()
                .iter()
                .all(|r| !r.is_active)
        );
        assert!(
            set_bundle_active(&db, &key, true)
                .await
                .unwrap()
                .iter()
                .all(|r| r.is_active)
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn bundle_validation_is_atomic_and_rejects_service_change() {
        let (db, input) = setup().await;
        for invalid in [
            vec![],
            vec![input.units[0].clone(); 2],
            vec![input.units[0].clone(); 17],
            vec![AllowanceUnitInput {
                quantity: 0,
                ..input.units[0].clone()
            }],
            vec![AllowanceUnitInput {
                metric: BillingMetric::Bytes,
                ..input.units[0].clone()
            }],
        ] {
            assert!(matches!(
                create_allowance_bundle(
                    &db,
                    AllowanceBundleInput {
                        units: invalid,
                        ..input.clone()
                    }
                )
                .await,
                Err(AppError::ValidationError(_))
            ));
            assert_eq!(list_allowances(&db, true).await.unwrap().len(), 0);
        }
        let rows = create_allowance_bundle(&db, input.clone()).await.unwrap();
        let key = rows[0].bundle_id.as_deref().unwrap();
        let mut other = test_auto_connected_catalog_service();
        other.slug = "other".into();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&other)
            .await
            .unwrap();
        let invalid = AllowanceBundleInput {
            service_ref: other.id,
            units: vec![AllowanceUnitInput {
                metric: BillingMetric::Requests,
                ..input.units[0].clone()
            }],
            ..input.clone()
        };
        assert!(replace_allowance_bundle(&db, key, invalid).await.is_err());
        let invalid = AllowanceBundleInput {
            units: vec![AllowanceUnitInput {
                quantity: 0,
                ..input.units[0].clone()
            }],
            ..input
        };
        assert!(replace_allowance_bundle(&db, key, invalid).await.is_err());
        assert_eq!(list_allowances(&db, true).await.unwrap().len(), 2);
        assert!(
            list_allowances(&db, true)
                .await
                .unwrap()
                .iter()
                .all(|r| r.is_active)
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_singleton_upgrade_keeps_id_and_compatibility_writes() {
        let (db, input) = setup().await;
        let legacy = create_allowance(
            &db,
            CreateAllowanceInput {
                service_ref: input.service_ref.clone(),
                metric: Some(input.units[0].metric),
                quantity: 7,
                recurrence: AllowanceRecurrence::Daily,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: vec![],
                target_org_ids: vec![],
                target_group_ids: vec![],
                created_by: "admin".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(legacy.bundle_id, None);
        let rows = replace_allowance_bundle(&db, &legacy.id, input)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.bundle_id.as_deref() == Some(&legacy.id))
        );
        let updated = update_allowance(
            &db,
            &legacy.id,
            UpdateAllowanceInput {
                quantity: Some(42),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.quantity, 42);
        assert_eq!(updated.bundle_id.as_deref(), Some(legacy.id.as_str()));
        db.drop().await.unwrap();
    }
}
