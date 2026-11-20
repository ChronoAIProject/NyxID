//! Post-commit scheduled credit issuance scoped to one active person. Calendar
//! periods use their frozen policy. Recipient cycles use the live schedule and
//! their durable anchor. Both share ordinary deterministic grants and postings.
use super::*;
use crate::models::credit_schedule_recipient::{
    COLLECTION_NAME as RECIPIENTS, CreditScheduleRecipient,
};
use crate::services::billing::{exact_migration, targets};

/// Bound the wait after an account or session has already committed. Durable
/// grants and postings can be retried by their ordinary deterministic IDs.
const FOREGROUND_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
enum Eligibility<'a> {
    Account,
    Login,
    Org(&'a str),
    Group(&'a str),
}

impl Eligibility<'_> {
    fn clauses(self, targets: &targets::ActivePersonTargets, user_id: &str) -> Vec<bson::Document> {
        match self {
            Self::Account | Self::Login => targets.clauses(user_id),
            Self::Org(id) if targets.org_ids.iter().any(|org| org == id) => {
                vec![doc! { "target_kind": "org_members", "target_org_ids": id }]
            }
            Self::Group(id) if targets.group_ids.iter().any(|group| group == id) => {
                vec![doc! { "target_kind": "groups", "target_group_ids": id }]
            }
            Self::Org(_) | Self::Group(_) => Vec::new(),
        }
    }
}

pub async fn after_user_created(db: &mongodb::Database, user_id: &str, created_at: DateTime<Utc>) {
    after_eligible(db, user_id, Eligibility::Account, created_at).await;
}

pub async fn after_human_login(db: &mongodb::Database, user_id: &str) {
    after_eligible(db, user_id, Eligibility::Login, Utc::now()).await;
}

pub async fn after_org_joined(
    db: &mongodb::Database,
    org_id: &str,
    user_id: &str,
    joined_at: DateTime<Utc>,
) {
    after_eligible(db, user_id, Eligibility::Org(org_id), joined_at).await;
}

/// A committed exit only changes the live membership predicate. Existing
/// grants and recipient anchors are financial history. A later join uses the
/// same anchor and current-window identity.
pub fn after_org_exited(_db: &mongodb::Database, _org_id: &str, _user_id: &str) {}

pub async fn after_group_joined(
    db: &mongodb::Database,
    group_id: &str,
    user_id: &str,
    joined_at: DateTime<Utc>,
) {
    after_eligible(db, user_id, Eligibility::Group(group_id), joined_at).await;
}

/// Billing is best effort after the account, session or membership commits.
async fn after_eligible(
    db: &mongodb::Database,
    user_id: &str,
    source: Eligibility<'_>,
    at: DateTime<Utc>,
) {
    let work = grant_on_eligibility(db, user_id, source, at, Utc::now());
    match tokio::time::timeout(FOREGROUND_BUDGET, work).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(user_id, ?source, %error, "Scheduled credits deferred"),
        Err(_) => tracing::warn!(user_id, ?source, "Scheduled credit loading timed out"),
    }
}

async fn grant_on_eligibility(
    db: &mongodb::Database,
    user_id: &str,
    source: Eligibility<'_>,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let durable_db = crate::services::transaction_service::durable_database(db);
    let db = &durable_db;
    let Some(targets) = targets::active_person_targets(db, user_id).await? else {
        return Ok(());
    };
    let clauses = source.clauses(&targets, user_id);
    if clauses.is_empty() {
        return Ok(());
    }
    let (recipient_result, calendar_result) = tokio::join!(
        load_recipient_schedules(db, user_id, &clauses, source, at, now),
        load_calendar_periods(db, user_id, &clauses, now),
    );
    recipient_result.and(calendar_result)
}

async fn load_recipient_schedules(
    db: &mongodb::Database,
    user_id: &str,
    clauses: &[bson::Document],
    source: Eligibility<'_>,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let mut filter = doc! { "timing": "recipient", "$or": clauses };
    let from_login = matches!(source, Eligibility::Login);
    if from_login {
        filter.insert("is_active", true);
    }
    let schedules: Vec<bson::Document> = db
        .collection::<bson::Document>(CREDIT_SCHEDULES)
        .find(filter)
        .await?
        .try_collect()
        .await?;
    if schedules.is_empty() {
        return Ok(());
    }
    let schedule_ids: Vec<_> = schedules
        .iter()
        .filter_map(|row| row.get_str("_id").ok())
        .collect();
    let existing: Vec<CreditScheduleRecipient> = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find(doc! { "recipient_user_id": user_id, "schedule_id": { "$in": schedule_ids } })
        .await?
        .try_collect()
        .await?;
    let mut by_schedule: std::collections::HashMap<_, _> = existing
        .into_iter()
        .map(|row| (row.schedule_id.clone(), row))
        .collect();
    let mut readiness = None;
    let mut failures = Failures::default();
    for raw in schedules {
        failures
            .unit(async {
                let schedule: CreditSchedule = recovery_batch::decode(&raw)?;
                let row = by_schedule.remove(&schedule.id);
                let now = Utc::now().max(now);
                let row =
                    recipients::load_member(db, &schedule, user_id, row, !from_login, at, now)
                        .await?;
                if let Some(row) = row {
                    let ready = match readiness {
                        Some(ready) => ready,
                        None => {
                            let ready = exact_migration::ready(db).await?;
                            readiness = Some(ready);
                            ready
                        }
                    };
                    if ready {
                        recipients::issue_recipient_window(
                            db,
                            &schedule,
                            &row,
                            Utc::now().max(now),
                            &mut DisbursementStats::default(),
                        )
                        .await?;
                    }
                }
                Ok(())
            })
            .await;
    }
    failures.finish(())
}

async fn load_calendar_periods(
    db: &mongodb::Database,
    user_id: &str,
    clauses: &[bson::Document],
    now: DateTime<Utc>,
) -> AppResult<()> {
    let now = Utc::now().max(now);
    // Only the worker and admin seeding open calendar periods. Read the
    // frozen targets first so a live schedule edit cannot hide today's grant.
    let periods: Vec<bson::Document> = db
        .collection::<bson::Document>(CREDIT_SCHEDULE_PERIODS)
        .find(doc! {
            "period_start": { "$lte": bson::DateTime::from_chrono(now) },
            "period_end": { "$gt": bson::DateTime::from_chrono(now) },
            "$or": clauses,
            "$and": [{ "$or": [
                { "expires_at": Bson::Null },
                { "expires_at": { "$gt": bson::DateTime::from_chrono(now) } },
            ] }],
        })
        .await?
        .try_collect()
        .await?;
    if periods.is_empty() {
        return Ok(());
    }
    let schedule_ids: Vec<_> = periods
        .iter()
        .filter_map(|row| row.get_str("schedule_id").ok())
        .collect();
    let schedules: Vec<bson::Document> = db
        .collection::<bson::Document>(CREDIT_SCHEDULES)
        .find(doc! { "_id": { "$in": schedule_ids }, "is_active": true,
        "timing": { "$ne": "recipient" } })
        .await?
        .try_collect()
        .await?;
    let schedules: std::collections::HashMap<_, _> = schedules
        .iter()
        .filter_map(|raw| Some((raw.get_str("_id").ok()?, raw)))
        .collect();
    let mut candidates = Vec::new();
    let mut failures = Failures::default();
    for raw in periods {
        failures
            .unit(async {
                let period: CreditSchedulePeriod = recovery_batch::decode(&raw)?;
                if let Some(raw) = schedules.get(period.schedule_id.as_str()) {
                    let schedule: CreditSchedule = recovery_batch::decode(raw)?;
                    candidates.push((schedule, period));
                }
                Ok(())
            })
            .await;
    }
    if candidates.is_empty() {
        return failures.finish(());
    }

    let ids: Vec<_> = candidates
        .iter()
        .map(|(schedule, period)| grant_id(&schedule.id, period.period_start, user_id))
        .collect();
    let mut existing: std::collections::HashMap<_, _> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "_id": { "$in": &ids } })
        .await?
        .map_ok(|grant| (grant.id.clone(), grant))
        .try_collect()
        .await?;

    let mut readiness = None;
    for ((schedule, period), id) in candidates.into_iter().zip(ids) {
        let persisted = existing.remove(&id);
        if persisted
            .as_ref()
            .is_some_and(|grant| grant.issued_ledgered_at.is_some())
        {
            continue;
        }
        failures
            .unit(async {
                let ready = match readiness {
                    Some(ready) => ready,
                    None => {
                        let ready = exact_migration::ready(db).await?;
                        readiness = Some(ready);
                        ready
                    }
                };
                if ready {
                    grant_current_period(db, &schedule, &period, user_id, persisted, now).await?;
                }
                Ok(())
            })
            .await;
    }
    failures.finish(())
}

async fn grant_current_period(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    period: &CreditSchedulePeriod,
    user_id: &str,
    persisted: Option<CreditGrant>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    // Earlier units or reads may have crossed the period or expiry boundary.
    let now = Utc::now().max(now);
    #[cfg(test)]
    let now = tests::calendar_check_time(now);
    if schedule_period(schedule.recurrence, now).start != period.period_start
        || period.expires_at.is_some_and(|expiry| expiry <= now)
    {
        return Ok(());
    }
    let (grant, inserted) = if let Some(grant) = persisted {
        (grant, false)
    } else {
        let grants = grants_for_recipients(schedule, period, vec![user_id.to_owned()], now);
        let outcome = insert_grant_chunk(db, &grants).await?;
        let grant = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": &grants[0].id })
            .await?
            .ok_or_else(|| AppError::Internal("Eligibility credit grant disappeared".into()))?;
        (grant, outcome.inserted_count > 0)
    };
    // Journal the persisted snapshot even when a walk or seed won the insert.
    if grant.issued_ledgered_at.is_none()
        && super::super::ledger::record_grant_event(
            db,
            BillingLedgerEventType::GrantIssued,
            &grant.recipient_user_id,
            &grant.id,
            grant.amount,
            None,
            format!("grant-issued:{}", grant.id),
        )
        .await
    {
        super::super::grants::mark_issued_ledgered(db, &grant.id, now).await?;
    }
    if inserted {
        #[cfg(test)]
        tests::after_calendar_insert().await;
        progress::refresh_count(db, period).await?;
        db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
            .update_one(
                doc! { "_id": &schedule.id },
                doc! { "$max": { "last_disbursed_at": bson::DateTime::from_chrono(now) } },
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
