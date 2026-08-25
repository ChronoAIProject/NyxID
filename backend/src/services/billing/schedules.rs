use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::billing_ledger::BillingLedgerEventType;
use crate::models::billing_target::BillingTargetKind;
use crate::models::credit_grant::{
    COLLECTION_NAME as CREDIT_GRANTS, CreditGrant, CreditGrantScheduleOrigin, CreditGrantStatus,
};
use crate::models::credit_schedule::{
    COLLECTION_NAME as CREDIT_SCHEDULES, CreditExpiryPolicy, CreditSchedule, SchedulePeriod,
    ScheduleRecurrence,
};
use crate::models::credit_schedule_period::{
    COLLECTION_NAME as CREDIT_SCHEDULE_PERIODS, CreditSchedulePeriod, SchedulePeriodStatus,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};

use super::grants::{CREDIT_MICROS, MAX_GRANT_CREDITS, MAX_GRANT_REASON_LEN, MAX_SELECTED_USERS};

pub const DISBURSEMENT_CHUNK: usize = 200;
pub const MAX_RECIPIENTS_PER_TICK: usize = 5_000;
pub const PERIOD_LEASE_SECS: i64 = 300;
const MAX_SCHEDULES_PER_TICK: i64 = 500;

#[derive(Clone, Debug)]
pub struct CreateScheduleInput {
    pub amount_credits: i64,
    pub recurrence: ScheduleRecurrence,
    pub expiry: CreditExpiryPolicy,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub all_services: bool,
    pub service_refs: Vec<String>,
    pub reason: Option<String>,
    pub created_by: String,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateScheduleInput {
    pub amount_credits: Option<i64>,
    pub expiry: Option<CreditExpiryPolicy>,
    pub target_kind: Option<BillingTargetKind>,
    pub target_user_ids: Option<Vec<String>>,
    pub all_services: Option<bool>,
    pub service_refs: Option<Vec<String>>,
    pub reason: Option<Option<String>>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct ScheduleWithCurrentPeriod {
    pub schedule: CreditSchedule,
    pub current_period: Option<CreditSchedulePeriod>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisbursementStats {
    pub schedules_examined: usize,
    pub periods_claimed: usize,
    pub periods_completed: usize,
    pub periods_abandoned: usize,
    pub recipients_examined: usize,
    pub grants_created: usize,
    pub already_disbursed: usize,
    pub grants_ledgered: usize,
    pub budget_exhausted: bool,
}

pub async fn create_schedule(
    db: &mongodb::Database,
    input: CreateScheduleInput,
) -> AppResult<CreditSchedule> {
    let amount_micros = validate_amount(input.amount_credits)?;
    validate_expiry(&input.expiry)?;
    let target_user_ids = validate_targets(db, input.target_kind, &input.target_user_ids).await?;
    let scope =
        super::grants::resolve_service_scope(db, input.all_services, &input.service_refs).await?;
    let now = Utc::now();
    let schedule = CreditSchedule {
        id: Uuid::new_v4().to_string(),
        amount_credits: input.amount_credits,
        amount_micros,
        recurrence: input.recurrence,
        expiry: input.expiry,
        target_kind: input.target_kind,
        target_user_ids,
        scope,
        reason: normalize_reason(input.reason)?,
        is_active: true,
        created_by: input.created_by,
        created_at: now,
        updated_at: now,
        last_period_start: None,
        last_disbursed_at: None,
        skipped_periods: 0,
    };
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .insert_one(&schedule)
        .await?;
    Ok(schedule)
}

pub async fn update_schedule(
    db: &mongodb::Database,
    schedule_id: &str,
    input: UpdateScheduleInput,
) -> AppResult<CreditSchedule> {
    let current = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find_one(doc! { "_id": schedule_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Credit schedule not found".to_string()))?;
    let amount_credits = input.amount_credits.unwrap_or(current.amount_credits);
    let amount_micros = validate_amount(amount_credits)?;
    let expiry = input.expiry.unwrap_or_else(|| current.expiry.clone());
    validate_expiry(&expiry)?;
    let target_kind = input.target_kind.unwrap_or(current.target_kind);
    let requested_targets = input
        .target_user_ids
        .unwrap_or_else(|| current.target_user_ids.clone());
    let target_user_ids = validate_targets(db, target_kind, &requested_targets).await?;

    let scope = if input.all_services.is_some() || input.service_refs.is_some() {
        let all_services = input.all_services.unwrap_or(current.scope.all_services);
        let references = input.service_refs.unwrap_or_else(|| {
            if all_services {
                Vec::new()
            } else {
                current.scope.service_ids.clone()
            }
        });
        super::grants::resolve_service_scope(db, all_services, &references).await?
    } else {
        current.scope.clone()
    };
    let reason = match input.reason {
        Some(reason) => normalize_reason(reason)?,
        None => current.reason.clone(),
    };
    let now = Utc::now();
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .update_one(
            doc! { "_id": schedule_id },
            doc! { "$set": {
                "amount_credits": amount_credits,
                "amount_micros": amount_micros,
                "expiry": encode(&expiry, "schedule expiry")?,
                "target_kind": encode(&target_kind, "schedule target kind")?,
                "target_user_ids": encode(&target_user_ids, "schedule targets")?,
                "scope": encode(&scope, "schedule scope")?,
                "reason": reason.as_ref().map_or(Bson::Null, |value| value.clone().into()),
                "is_active": input.is_active.unwrap_or(current.is_active),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find_one(doc! { "_id": schedule_id })
        .await?
        .ok_or_else(|| AppError::Internal("updated credit schedule disappeared".to_string()))
}

pub async fn list_schedules(
    db: &mongodb::Database,
    now: DateTime<Utc>,
) -> AppResult<Vec<ScheduleWithCurrentPeriod>> {
    let schedules: Vec<CreditSchedule> = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find(doc! {})
        .sort(doc! { "created_at": -1, "_id": -1 })
        .await?
        .try_collect()
        .await?;
    if schedules.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = schedules
        .iter()
        .map(|schedule| {
            period_id(
                &schedule.id,
                schedule_period(schedule.recurrence, now).start,
            )
        })
        .collect();
    let periods: Vec<CreditSchedulePeriod> = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find(doc! { "_id": { "$in": &ids } })
        .await?
        .try_collect()
        .await?;
    let by_id: std::collections::HashMap<String, CreditSchedulePeriod> = periods
        .into_iter()
        .map(|period| (period.id.clone(), period))
        .collect();
    Ok(schedules
        .into_iter()
        .zip(ids)
        .map(|(schedule, id)| ScheduleWithCurrentPeriod {
            schedule,
            current_period: by_id.get(&id).cloned(),
        })
        .collect())
}

pub async fn disburse_due(
    db: &mongodb::Database,
    now: DateTime<Utc>,
    budget: usize,
) -> AppResult<DisbursementStats> {
    let mut stats = DisbursementStats::default();
    abandon_elapsed_periods(db, now, &mut stats).await?;
    if budget == 0 {
        stats.budget_exhausted = true;
        return Ok(stats);
    }

    let in_flight_schedule_ids: Vec<String> = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .distinct("schedule_id", doc! { "status": "disbursing", "period_end": { "$gt": bson::DateTime::from_chrono(now) } })
        .await?
        .into_iter()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect();
    let schedules: Vec<CreditSchedule> = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find(doc! { "$or": [
            { "is_active": true },
            { "_id": { "$in": &in_flight_schedule_ids } },
        ] })
        .sort(doc! { "_id": 1 })
        .limit(MAX_SCHEDULES_PER_TICK)
        .await?
        .try_collect()
        .await?;

    let mut remaining_budget = budget.min(MAX_RECIPIENTS_PER_TICK);
    for schedule in schedules {
        if remaining_budget == 0 {
            stats.budget_exhausted = true;
            break;
        }
        stats.schedules_examined += 1;
        let window = schedule_period(schedule.recurrence, now);
        let claim = claim_period(db, &schedule, window, now, schedule.is_active).await?;
        let mut period = match claim {
            PeriodClaim::Acquired(period) => {
                stats.periods_claimed += 1;
                *period
            }
            PeriodClaim::Busy | PeriodClaim::Complete => continue,
        };

        loop {
            if remaining_budget == 0 {
                release_period_lease(db, &period.id, now).await?;
                stats.budget_exhausted = true;
                break;
            }
            let chunk_limit = DISBURSEMENT_CHUNK.min(remaining_budget);
            let recipients = next_recipients(db, &period, chunk_limit).await?;
            if recipients.is_empty() {
                complete_period(db, &schedule.id, &period, now).await?;
                stats.periods_completed += 1;
                break;
            }
            let last_recipient = recipients.last().cloned().expect("chunk is non-empty");
            let grants = grants_for_recipients(&schedule, &period, recipients, Utc::now());
            let outcome = insert_grant_chunk(db, &grants).await?;
            stats.recipients_examined += grants.len();
            stats.grants_created += outcome.inserted_indices.len();
            stats.already_disbursed += outcome.duplicate_count;
            for index in outcome.inserted_indices {
                let grant = &grants[index];
                if super::ledger::record_grant_event(
                    db,
                    BillingLedgerEventType::GrantIssued,
                    &grant.recipient_user_id,
                    &grant.id,
                    grant.amount_micros,
                    None,
                    format!("grant-issued:{}", grant.id),
                )
                .await
                {
                    super::grants::mark_issued_ledgered(db, &grant.id, Utc::now()).await?;
                    stats.grants_ledgered += 1;
                }
            }
            advance_cursor(
                db,
                &schedule.id,
                &period.id,
                &last_recipient,
                outcome.inserted_count,
                now,
            )
            .await?;
            period.cursor_user_id = Some(last_recipient);
            period.disbursed_count = period
                .disbursed_count
                .saturating_add(outcome.inserted_count as u64);
            remaining_budget -= grants.len();
        }
    }
    Ok(stats)
}

pub fn schedule_period(recurrence: ScheduleRecurrence, now: DateTime<Utc>) -> SchedulePeriod {
    let recurrence = match recurrence {
        ScheduleRecurrence::Daily => super::periods::RecurringUtcPeriod::Daily,
        ScheduleRecurrence::Weekly => super::periods::RecurringUtcPeriod::Weekly,
        ScheduleRecurrence::Monthly => super::periods::RecurringUtcPeriod::Monthly,
    };
    let window = super::periods::recurring_utc_window(recurrence, now);
    SchedulePeriod {
        start: window.start,
        end: window.end,
    }
}

pub fn resolve_expiry(
    policy: &CreditExpiryPolicy,
    period: &SchedulePeriod,
    created_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match policy {
        CreditExpiryPolicy::EndOfPeriod => Some(period.end),
        CreditExpiryPolicy::AfterDays { days } => {
            Some(created_at + Duration::days(i64::from(*days)))
        }
        CreditExpiryPolicy::Never => None,
    }
}

pub fn period_id(schedule_id: &str, period_start: DateTime<Utc>) -> String {
    format!("{schedule_id}:{}", period_start.timestamp_millis())
}

fn grant_id(schedule_id: &str, period_start: DateTime<Utc>, recipient: &str) -> String {
    let identity = format!(
        "{schedule_id}:{}:{recipient}",
        period_start.timestamp_millis()
    );
    Uuid::new_v5(&Uuid::NAMESPACE_URL, identity.as_bytes()).to_string()
}

enum PeriodClaim {
    Acquired(Box<CreditSchedulePeriod>),
    Busy,
    Complete,
}

async fn claim_period(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    period: SchedulePeriod,
    now: DateTime<Utc>,
    allow_insert: bool,
) -> AppResult<PeriodClaim> {
    let id = period_id(&schedule.id, period.start);
    let claim_time = Utc::now();
    let update = doc! {
        "$setOnInsert": {
            "schedule_id": &schedule.id,
            "period_start": bson::DateTime::from_chrono(period.start),
            "period_end": bson::DateTime::from_chrono(period.end),
            "status": "disbursing",
            "amount_micros": schedule.amount_micros,
            "expires_at": resolve_expiry(&schedule.expiry, &period, claim_time)
                .map_or(Bson::Null, |value| bson::DateTime::from_chrono(value).into()),
            "target_kind": encode(&schedule.target_kind, "period target kind")?,
            "target_user_ids": encode(&schedule.target_user_ids, "period targets")?,
            "scope": encode(&schedule.scope, "period scope")?,
            "reason": schedule.reason.as_ref().map_or(Bson::Null, |value| value.clone().into()),
            "cursor_user_id": Bson::Null,
            "disbursed_count": 0_i64,
            "created_at": bson::DateTime::from_chrono(claim_time),
            "completed_at": Bson::Null,
        },
        "$set": {
            "lease_expires_at": bson::DateTime::from_chrono(now + Duration::seconds(PERIOD_LEASE_SECS)),
            "updated_at": bson::DateTime::from_chrono(now),
        },
    };
    let filter = doc! {
        "_id": &id,
        "status": "disbursing",
        "$or": [
            { "lease_expires_at": Bson::Null },
            { "lease_expires_at": { "$exists": false } },
            { "lease_expires_at": { "$lte": bson::DateTime::from_chrono(now) } },
        ],
    };
    let result = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one_and_update(filter, update)
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(allow_insert)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;
    match result {
        Ok(Some(period)) => Ok(PeriodClaim::Acquired(Box::new(period))),
        Ok(None) => Ok(PeriodClaim::Busy),
        Err(error) if is_duplicate_key_error(&error) => {
            let existing = db
                .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
                .find_one(doc! { "_id": id })
                .await?;
            Ok(match existing.map(|period| period.status) {
                Some(SchedulePeriodStatus::Complete) => PeriodClaim::Complete,
                _ => PeriodClaim::Busy,
            })
        }
        Err(error) => Err(error.into()),
    }
}

async fn next_recipients(
    db: &mongodb::Database,
    period: &CreditSchedulePeriod,
    limit: usize,
) -> AppResult<Vec<String>> {
    match period.target_kind {
        BillingTargetKind::AllUsers => {
            // The claim timestamp excludes later signups. Re-checking
            // is_active while paging deliberately permits an existing owner
            // activated ahead of the cursor to receive this period.
            let mut filter = doc! {
                "is_active": true,
                "created_at": { "$lte": bson::DateTime::from_chrono(period.created_at) },
            };
            if let Some(cursor) = period.cursor_user_id.as_deref() {
                filter.insert("_id", doc! { "$gt": cursor });
            }
            let users: Vec<User> = db
                .collection::<User>(USERS)
                .find(filter)
                .sort(doc! { "_id": 1 })
                .limit(limit as i64)
                .await?
                .try_collect()
                .await?;
            Ok(users.into_iter().map(|user| user.id).collect())
        }
        BillingTargetKind::SelectedUsers => {
            let mut targets = period.target_user_ids.clone();
            targets.sort();
            Ok(targets
                .into_iter()
                .filter(|id| {
                    period
                        .cursor_user_id
                        .as_deref()
                        .is_none_or(|cursor| id.as_str() > cursor)
                })
                .take(limit)
                .collect())
        }
    }
}

fn grants_for_recipients(
    schedule: &CreditSchedule,
    period: &CreditSchedulePeriod,
    recipients: Vec<String>,
    now: DateTime<Utc>,
) -> Vec<CreditGrant> {
    let batch_id = period_id(&schedule.id, period.period_start);
    recipients
        .into_iter()
        .map(|recipient_user_id| CreditGrant {
            id: grant_id(&schedule.id, period.period_start, &recipient_user_id),
            batch_id: batch_id.clone(),
            schedule_origin: Some(CreditGrantScheduleOrigin {
                schedule_id: schedule.id.clone(),
                period_start: period.period_start,
            }),
            recipient_user_id,
            target_kind: period.target_kind,
            amount_credits: period.amount_micros / CREDIT_MICROS,
            amount_micros: period.amount_micros,
            remaining_micros: period.amount_micros,
            reserved_micros: 0,
            scope: period.scope.clone(),
            expires_at: period.expires_at,
            reason: period.reason.clone(),
            granted_by: schedule.created_by.clone(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: None,
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now,
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        })
        .collect()
}

struct InsertChunkOutcome {
    inserted_indices: Vec<usize>,
    inserted_count: usize,
    duplicate_count: usize,
}

async fn insert_grant_chunk(
    db: &mongodb::Database,
    grants: &[CreditGrant],
) -> AppResult<InsertChunkOutcome> {
    match db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_many(grants)
        .ordered(false)
        .await
    {
        Ok(_) => Ok(InsertChunkOutcome {
            inserted_indices: (0..grants.len()).collect(),
            inserted_count: grants.len(),
            duplicate_count: 0,
        }),
        Err(error) => {
            let mongodb::error::ErrorKind::InsertMany(failure) = error.kind.as_ref() else {
                return Err(error.into());
            };
            if failure.write_concern_error.is_some() {
                return Err(error.into());
            }
            let Some(write_errors) = failure.write_errors.as_ref() else {
                return Err(error.into());
            };
            if write_errors.is_empty() || write_errors.iter().any(|item| item.code != 11000) {
                return Err(error.into());
            }
            let duplicate_indices: HashSet<usize> =
                write_errors.iter().map(|item| item.index).collect();
            let inserted_indices: Vec<usize> = (0..grants.len())
                .filter(|index| !duplicate_indices.contains(index))
                .collect();
            Ok(InsertChunkOutcome {
                inserted_count: inserted_indices.len(),
                inserted_indices,
                duplicate_count: duplicate_indices.len(),
            })
        }
    }
}

async fn advance_cursor(
    db: &mongodb::Database,
    schedule_id: &str,
    period_id: &str,
    cursor: &str,
    inserted_count: usize,
    now: DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .update_one(
            doc! { "_id": period_id, "status": "disbursing" },
            doc! {
                "$max": { "cursor_user_id": cursor },
                "$inc": { "disbursed_count": inserted_count as i64 },
                "$set": {
                    "lease_expires_at": bson::DateTime::from_chrono(now + Duration::seconds(PERIOD_LEASE_SECS)),
                    "updated_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .await?;
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .update_one(
            doc! { "_id": schedule_id },
            doc! { "$set": {
                "last_period_start": period_id
                    .rsplit_once(':')
                    .and_then(|(_, millis)| millis.parse::<i64>().ok())
                    .and_then(DateTime::from_timestamp_millis)
                    .map_or(Bson::Null, |value| bson::DateTime::from_chrono(value).into()),
                "last_disbursed_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    Ok(())
}

async fn complete_period(
    db: &mongodb::Database,
    schedule_id: &str,
    period: &CreditSchedulePeriod,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let disbursed_count = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .count_documents(doc! {
            "schedule_origin.schedule_id": schedule_id,
            "schedule_origin.period_start": bson::DateTime::from_chrono(period.period_start),
        })
        .await?;
    let disbursed_count = i64::try_from(disbursed_count)
        .map_err(|_| AppError::Internal("credit schedule grant count overflowed".to_string()))?;
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .update_one(
            doc! { "_id": &period.id, "status": "disbursing" },
            doc! { "$set": {
                "status": "complete",
                "disbursed_count": disbursed_count,
                "lease_expires_at": Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
                "completed_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .update_one(
            doc! { "_id": schedule_id },
            doc! { "$set": { "last_disbursed_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;
    Ok(())
}

async fn release_period_lease(
    db: &mongodb::Database,
    period_id: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .update_one(
            doc! { "_id": period_id, "status": "disbursing" },
            doc! { "$set": {
                "lease_expires_at": Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    Ok(())
}

async fn abandon_elapsed_periods(
    db: &mongodb::Database,
    now: DateTime<Utc>,
    stats: &mut DisbursementStats,
) -> AppResult<()> {
    let periods: Vec<CreditSchedulePeriod> = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find(doc! {
            "status": "disbursing",
            "period_end": { "$lte": bson::DateTime::from_chrono(now) },
        })
        .sort(doc! { "period_end": 1, "_id": 1 })
        .limit(MAX_SCHEDULES_PER_TICK)
        .await?
        .try_collect()
        .await?;
    for period in periods {
        let result = db
            .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
            .update_one(
                doc! { "_id": &period.id, "status": "disbursing" },
                doc! { "$set": {
                    "status": "complete",
                    "lease_expires_at": Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(now),
                    "completed_at": bson::DateTime::from_chrono(now),
                } },
            )
            .await?;
        if result.modified_count == 0 {
            continue;
        }
        db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
            .update_one(
                doc! { "_id": &period.schedule_id },
                doc! { "$inc": { "skipped_periods": 1_i64 } },
            )
            .await?;
        stats.periods_abandoned += 1;
        tracing::warn!(
            schedule_id = %period.schedule_id,
            period_start = %period.period_start,
            "abandoned incomplete elapsed credit schedule period"
        );
    }
    Ok(())
}

fn validate_amount(amount_credits: i64) -> AppResult<i64> {
    if !(1..=MAX_GRANT_CREDITS).contains(&amount_credits) {
        return Err(AppError::ValidationError(format!(
            "amount_credits must be between 1 and {MAX_GRANT_CREDITS}"
        )));
    }
    amount_credits
        .checked_mul(CREDIT_MICROS)
        .ok_or_else(|| AppError::ValidationError("credit schedule amount is too large".to_string()))
}

fn validate_expiry(expiry: &CreditExpiryPolicy) -> AppResult<()> {
    if let CreditExpiryPolicy::AfterDays { days } = expiry
        && !(1..=3_650).contains(days)
    {
        return Err(AppError::ValidationError(
            "after_days expiry must be between 1 and 3650 days".to_string(),
        ));
    }
    Ok(())
}

async fn validate_targets(
    db: &mongodb::Database,
    target_kind: BillingTargetKind,
    targets: &[String],
) -> AppResult<Vec<String>> {
    match target_kind {
        BillingTargetKind::AllUsers => {
            if !targets.is_empty() {
                return Err(AppError::ValidationError(
                    "all-users schedules must not include target_user_ids".to_string(),
                ));
            }
            Ok(Vec::new())
        }
        BillingTargetKind::SelectedUsers => {
            if targets.is_empty() || targets.len() > MAX_SELECTED_USERS {
                return Err(AppError::ValidationError(format!(
                    "selected schedules require 1-{MAX_SELECTED_USERS} target users"
                )));
            }
            super::grants::resolve_recipients(db, target_kind, targets).await
        }
    }
}

fn normalize_reason(reason: Option<String>) -> AppResult<Option<String>> {
    let reason = reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    if reason
        .as_ref()
        .is_some_and(|value| value.len() > MAX_GRANT_REASON_LEN)
    {
        return Err(AppError::ValidationError(format!(
            "reason must not exceed {MAX_GRANT_REASON_LEN} characters"
        )));
    }
    Ok(reason)
}

fn encode<T: serde::Serialize>(value: &T, label: &str) -> AppResult<Bson> {
    bson::to_bson(value)
        .map_err(|error| AppError::Internal(format!("failed to encode {label}: {error}")))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => command.code == 11000,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000
        }
        _ => false,
    }
}
