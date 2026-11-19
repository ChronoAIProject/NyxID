//! Explicit current-period catch-up for calendar schedules. Ordinary calendar
//! walks retain their signup/join cutoff; this admin action uses live eligibility
//! and the same frozen period policy and grant identities as ordinary issuance.
use super::*;
use mongodb::bson::Document;

pub const SEED_PAGE_SIZE: usize = 50;

#[derive(Debug, Default)]
pub struct SeedScheduleInput {
    pub actor: Option<crate::services::audit_service::AuditActor>,
    pub expected_updated_at: Option<DateTime<Utc>>,
    pub period_start: Option<DateTime<Utc>>,
    pub after_user_id: Option<String>,
}

#[derive(Debug)]
pub struct SeedScheduleResult {
    pub period_start: DateTime<Utc>,
    pub created_count: usize,
    pub existing_count: usize,
    pub pending_activation_count: usize,
    pub next_user_id: Option<String>,
}

/// Each request examines at most 50 people. Retrying a page, restarting a scan,
/// or racing the recurring worker cannot replace or refill an existing grant.
pub async fn seed_missing(
    db: &mongodb::Database,
    schedule_id: &str,
    input: SeedScheduleInput,
    now: DateTime<Utc>,
) -> AppResult<SeedScheduleResult> {
    super::super::exact_migration::require_ready(db).await?;
    if input.after_user_id.as_ref().is_some_and(|cursor| {
        input.period_start.is_none()
            || cursor.is_empty()
            || cursor.len() > 256
            || cursor.trim() != cursor
    }) {
        return Err(AppError::ValidationError(
            "A seed cursor requires its period_start and a valid user id".into(),
        ));
    }
    let durable_db = crate::services::transaction_service::durable_database(db);
    let db = &durable_db;
    let schedule = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find_one(doc! { "_id": schedule_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Credit schedule not found".into()))?;
    if input.expected_updated_at.is_some_and(|expected| {
        expected.timestamp_millis() != schedule.updated_at.timestamp_millis()
    }) {
        return Err(AppError::Conflict(
            "Credit schedule changed; refresh before granting missing credits".into(),
        ));
    }
    if !schedule.is_active {
        return Err(AppError::ValidationError(
            "Resume the schedule before granting missing credits".into(),
        ));
    }
    if schedule.timing != ScheduleTiming::Calendar {
        return Err(AppError::ValidationError(
            "Recipient schedules automatically enroll existing eligible users".into(),
        ));
    }
    let window = schedule_period(schedule.recurrence, now);
    if input
        .period_start
        .is_some_and(|start| start != window.start)
    {
        return Err(AppError::Conflict(
            "The seed period has ended; start again for the current period".into(),
        ));
    }
    // Freeze a missing period through the ordinary claim path. An existing
    // period (including one currently being walked) keeps its original policy.
    if let PeriodClaim::Acquired(period) = claim_period(db, &schedule, window, now, true).await? {
        release_period_lease(db, &period.id, now).await?;
    }
    let period = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": period_id(schedule_id, window.start) })
        .await?
        .ok_or_else(|| AppError::Internal("Credit schedule period disappeared".into()))?;
    if period.expires_at.is_some_and(|expiry| expiry <= now) {
        return Err(AppError::Conflict(
            "This period's grant has expired; wait for the next period".into(),
        ));
    }
    let ids = live_recipients(db, &period, input.after_user_id.as_deref()).await?;
    let next_user_id = (ids.len() == SEED_PAGE_SIZE)
        .then(|| ids.last().cloned())
        .flatten();
    let mut result = SeedScheduleResult {
        period_start: window.start,
        created_count: 0,
        existing_count: 0,
        pending_activation_count: 0,
        next_user_id,
    };
    if ids.is_empty() {
        return Ok(result);
    }
    let mut grants = grants_for_recipients(&schedule, &period, ids, now);
    if let Some(actor) = input.actor {
        for grant in &mut grants {
            grant.granted_by.clone_from(&actor.user_id);
        }
        // Persist the acting admin and the intended financial identities before
        // any insert. Partial failures still have an attributable audit trail.
        crate::services::audit_service::log_actor_event(
            db.clone(),
            &actor,
            "billing.credit_schedule.seed_requested",
            Some(serde_json::json!({
                "schedule_id": schedule_id,
                "period_start": window.start,
                "grant_ids": grants.iter().map(|grant| &grant.id).collect::<Vec<_>>(),
            })),
        )
        .await?;
    }
    let outcome = insert_grant_chunk(db, &grants).await?;
    result.created_count = outcome.inserted_count;
    result.existing_count = outcome.duplicate_count;
    let grant_ids: Vec<_> = grants.iter().map(|grant| &grant.id).collect();
    let persisted: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "_id": { "$in": grant_ids } })
        .await?
        .try_collect()
        .await?;
    for grant in persisted {
        if grant.issued_ledgered_at.is_some() {
            continue;
        }
        // A concurrent insert or admin edit must never cause us to journal a
        // recomputed amount. Journal only the persisted financial identity.
        if super::super::ledger::record_grant_event(
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
        } else {
            result.pending_activation_count += 1;
        }
    }
    progress::refresh_count(db, &period).await?;
    if result.created_count > 0 {
        db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
            .update_one(
                doc! { "_id": schedule_id },
                doc! { "$max": { "last_disbursed_at": bson::DateTime::from_chrono(now) } },
            )
            .await?;
    }
    Ok(result)
}

async fn live_recipients(
    db: &mongodb::Database,
    period: &CreditSchedulePeriod,
    cursor: Option<&str>,
) -> AppResult<Vec<String>> {
    if matches!(
        period.target_kind,
        BillingTargetKind::Groups | BillingTargetKind::OrgMembers
    ) {
        return super::super::targets::member_recipients(
            db,
            period.target_kind,
            &period.target_org_ids,
            &period.target_group_ids,
            None,
            cursor,
            SEED_PAGE_SIZE,
        )
        .await;
    }
    let mut filter = doc! { "is_active": true };
    if period.target_kind == BillingTargetKind::SelectedUsers {
        filter.insert("_id", doc! { "$in": &period.target_user_ids });
    }
    if let Some(cursor) = cursor {
        filter.insert("$and", vec![doc! { "_id": { "$gt": cursor } }]);
    }
    let rows: Vec<Document> = db
        .collection::<Document>(USERS)
        .find(filter)
        .projection(doc! { "_id": 1 })
        .sort(doc! { "_id": 1 })
        .limit(SEED_PAGE_SIZE as i64)
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect())
}

#[cfg(test)]
mod tests;
