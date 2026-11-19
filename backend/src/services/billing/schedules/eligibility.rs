//! Entitlement when an owner becomes eligible mid-period: account creation,
//! an organization join or a group join. A calendar period's claim snapshot
//! excludes owners and memberships created after it, so the current period is
//! granted here instead of waiting for the next one. Grants keep the ordinary
//! deterministic identity, frozen period policy and journal path, so the
//! calendar walk, admin seeding and a retried hook all converge on one grant.
use super::*;
use crate::models::org_membership::COLLECTION_NAME as MEMBERSHIPS;

#[derive(Clone, Copy, Debug)]
pub enum Eligibility<'a> {
    Account,
    Org(&'a str),
    Group(&'a str),
}

impl Eligibility<'_> {
    fn schedule_target(self) -> bson::Document {
        match self {
            Self::Account => doc! { "target_kind": "all_users" },
            Self::Org(org_id) => doc! { "target_kind": "org_members", "target_org_ids": org_id },
            Self::Group(group_id) => {
                doc! { "target_kind": "groups", "target_group_ids": group_id }
            }
        }
    }

    /// The claimed period's frozen targets decide, never the live schedule.
    fn covered_by(self, period: &CreditSchedulePeriod) -> bool {
        match self {
            Self::Account => period.target_kind == BillingTargetKind::AllUsers,
            Self::Org(org_id) => {
                period.target_kind == BillingTargetKind::OrgMembers
                    && period.target_org_ids.iter().any(|id| id == org_id)
            }
            Self::Group(group_id) => {
                period.target_kind == BillingTargetKind::Groups
                    && period.target_group_ids.iter().any(|id| id == group_id)
            }
        }
    }
}

/// Best effort: the account or membership has already committed and must
/// never fail here. An admin can still grant missed credits for the period.
pub async fn after_user_created(db: &mongodb::Database, user_id: &str, created_at: DateTime<Utc>) {
    after_eligible(db, user_id, Eligibility::Account, created_at).await;
}

pub async fn after_org_joined(
    db: &mongodb::Database,
    org_id: &str,
    user_id: &str,
    joined_at: DateTime<Utc>,
) {
    after_eligible(db, user_id, Eligibility::Org(org_id), joined_at).await;
}

pub async fn after_group_joined(
    db: &mongodb::Database,
    group_id: &str,
    user_id: &str,
    joined_at: DateTime<Utc>,
) {
    after_eligible(db, user_id, Eligibility::Group(group_id), joined_at).await;
}

async fn after_eligible(
    db: &mongodb::Database,
    user_id: &str,
    source: Eligibility<'_>,
    at: DateTime<Utc>,
) {
    if let Err(error) = grant_on_eligibility(db, user_id, source, at, Utc::now()).await {
        tracing::warn!(user_id, ?source, %error, "Scheduled credits for new eligibility deferred");
    }
}

pub async fn grant_on_eligibility(
    db: &mongodb::Database,
    user_id: &str,
    source: Eligibility<'_>,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    // One read serves both timings. Recipient cycles record the anchor even
    // while paused; calendar schedules grant only while active.
    let mut filter = doc! { "$or": [
        { "is_active": true },
        { "timing": "recipient" },
    ] };
    filter.extend(source.schedule_target());
    let schedules: Vec<bson::Document> = db
        .collection::<bson::Document>(CREDIT_SCHEDULES)
        .find(filter)
        .await?
        .try_collect()
        .await?;
    let mut first_error = None;
    let mut calendar = Vec::new();
    for raw in schedules {
        match recovery_batch::decode::<CreditSchedule>(&raw) {
            Ok(schedule) if schedule.timing == ScheduleTiming::Recipient => {
                if let Err(error) = recipients::enroll_member(db, &schedule, user_id, at).await {
                    first_error.get_or_insert(error);
                }
            }
            Ok(schedule) => calendar.push(schedule),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if !calendar.is_empty()
        && let Err(error) = grant_calendar_periods(db, &calendar, user_id, source, now).await
    {
        first_error.get_or_insert(error);
    }
    first_error.map_or(Ok(()), Err)
}

async fn grant_calendar_periods(
    db: &mongodb::Database,
    schedules: &[CreditSchedule],
    user_id: &str,
    source: Eligibility<'_>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    super::super::exact_migration::require_ready(db).await?;
    let durable_db = crate::services::transaction_service::durable_database(db);
    let db = &durable_db;
    if !still_eligible(db, user_id, source).await? {
        return Ok(());
    }
    let mut first_error = None;
    for schedule in schedules {
        if let Err(error) = grant_current_period(db, schedule, user_id, source, now).await {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Re-read live eligibility after commit, matching the walk and seeding:
/// active people in active organizations with a non-revoked membership, or
/// active people holding the group directly.
async fn still_eligible(
    db: &mongodb::Database,
    user_id: &str,
    source: Eligibility<'_>,
) -> AppResult<bool> {
    let mut user = doc! { "_id": user_id, "is_active": true };
    match source {
        Eligibility::Account => {}
        Eligibility::Group(group_id) => {
            user.insert("user_type", doc! { "$ne": "org" });
            user.insert("group_ids", group_id);
        }
        Eligibility::Org(org_id) => {
            user.insert("user_type", doc! { "$ne": "org" });
            let org_active = db
                .collection::<bson::Document>(USERS)
                .count_documents(doc! { "_id": org_id, "is_active": true, "user_type": "org" })
                .await?
                > 0;
            let member = db
                .collection::<bson::Document>(MEMBERSHIPS)
                .count_documents(doc! { "org_user_id": org_id,
                "member_user_id": user_id, "revoked_at": Bson::Null })
                .await?
                > 0;
            if !org_active || !member {
                return Ok(false);
            }
        }
    }
    Ok(db
        .collection::<bson::Document>(USERS)
        .count_documents(user)
        .await?
        > 0)
}

async fn grant_current_period(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    user_id: &str,
    source: Eligibility<'_>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let now = Utc::now().max(now);
    let window = schedule_period(schedule.recurrence, now);
    // An unclaimed period's snapshot will include this owner. Never open a
    // period from an account or membership change.
    let Some(period) = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": period_id(&schedule.id, window.start) })
        .await?
    else {
        return Ok(());
    };
    // Discovery and earlier schedules may have taken us across a boundary.
    // Recheck after the database read, immediately before starting this unit.
    let now = Utc::now().max(now);
    #[cfg(test)]
    let now = tests::calendar_check_time(now);
    if schedule_period(schedule.recurrence, now).start != window.start
        || !source.covered_by(&period)
        || period.expires_at.is_some_and(|expiry| expiry <= now)
    {
        return Ok(());
    }
    let grants = grants_for_recipients(schedule, &period, vec![user_id.to_owned()], now);
    let outcome = insert_grant_chunk(db, &grants).await?;
    // Journal only the persisted grant, including after a concurrent walk or
    // seed inserted it first.
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": &grants[0].id })
        .await?
        .ok_or_else(|| AppError::Internal("Eligibility credit grant disappeared".into()))?;
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
    if outcome.inserted_count > 0 {
        // Display progress only; completion and abandonment recount exactly.
        db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
            .update_one(
                doc! { "_id": &period.id },
                doc! { "$inc": { "disbursed_count": outcome.inserted_count as i64 } },
            )
            .await?;
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
mod tests;
