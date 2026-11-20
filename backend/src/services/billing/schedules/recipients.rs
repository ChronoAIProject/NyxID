//! Recipient cycles discover new members continuously and renew independently.
//! Recipient anchors are durable; grants and journal dedupe remain the existing
//! schedule/period/person identity, including after a crash or a rejoin.
use super::*;
use chrono::{Datelike, Months};
use mongodb::bson::Document;

use crate::models::credit_schedule_recipient::{
    COLLECTION_NAME as RECIPIENTS, CreditScheduleRecipient, CreditScheduleRecipientScan,
    SCAN_COLLECTION_NAME as SCANS,
};
use crate::models::org_membership::COLLECTION_NAME as MEMBERSHIPS;
use crate::services::billing::reconcile::continue_reconcile;

#[cfg(test)]
tokio::task_local! { pub(crate) static STALL_BEFORE_POSTING: (); }

pub const READINESS: &str = "billing:recipient-schedules-v1";

fn recipient_filter(schedule_id: &str, user_id: &str) -> Document {
    doc! { "schedule_id": schedule_id, "recipient_user_id": user_id }
}

/// Always calculate from the original anchor, so Jan 31 -> Feb 28 -> Mar 31.
/// A delayed worker issues only the current window, never elapsed credits.
pub fn recipient_period(
    recurrence: ScheduleRecurrence,
    anchor: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<SchedulePeriod> {
    let now = now.max(anchor);
    let overflow = || AppError::Internal("Credit recipient period overflowed".into());
    let (start, end) = match recurrence {
        ScheduleRecurrence::Daily | ScheduleRecurrence::Weekly => {
            let days = if recurrence == ScheduleRecurrence::Daily {
                1
            } else {
                7
            };
            let span = Duration::days(days);
            let elapsed = (now - anchor).num_milliseconds() / span.num_milliseconds();
            let start = anchor
                .checked_add_signed(Duration::milliseconds(
                    elapsed
                        .checked_mul(span.num_milliseconds())
                        .ok_or_else(overflow)?,
                ))
                .ok_or_else(overflow)?;
            (start, start.checked_add_signed(span).ok_or_else(overflow)?)
        }
        ScheduleRecurrence::Monthly => {
            let mut months = u32::try_from(
                (now.year() - anchor.year()) * 12 + now.month() as i32 - anchor.month() as i32,
            )
            .map_err(|_| overflow())?;
            let mut start = anchor
                .checked_add_months(Months::new(months))
                .ok_or_else(overflow)?;
            if start > now {
                months = months.checked_sub(1).ok_or_else(overflow)?;
                start = anchor
                    .checked_add_months(Months::new(months))
                    .ok_or_else(overflow)?;
            }
            let end = anchor
                .checked_add_months(Months::new(months.checked_add(1).ok_or_else(overflow)?))
                .ok_or_else(overflow)?;
            (start, end)
        }
    };
    Ok(SchedulePeriod { start, end })
}

async fn enroll(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    user_id: &str,
    anchor: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let result = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .update_one(
            recipient_filter(&schedule.id, user_id),
            doc! { "$setOnInsert": {
                "_id": Uuid::new_v4().to_string(),
                "schedule_id": &schedule.id, "recipient_user_id": user_id,
                "anchor_at": bson::DateTime::from_chrono(anchor),
                "next_due_at": bson::DateTime::from_chrono(anchor),
                "is_eligible": true,
                "eligibility_version": 0_i64,
                "created_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .upsert(true)
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(error) if is_duplicate_key_error(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
pub async fn enroll_group_member(
    db: &mongodb::Database,
    group_id: &str,
    user_id: &str,
    joined_at: DateTime<Utc>,
) -> AppResult<()> {
    let schedules: Vec<CreditSchedule> = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find(doc! { "timing": "recipient",
        "target_kind": "groups", "target_group_ids": group_id })
        .await?
        .try_collect()
        .await?;
    for schedule in schedules {
        enroll_member(db, &schedule, user_id, joined_at).await?;
    }
    Ok(())
}

/// Capture the eligibility time (account creation, organization or group
/// join) instead of waiting for the next discovery page, including while the
/// schedule is paused. A rejoin keeps its original anchor and restores
/// eligibility. Discovery also recovers missed enrollments.
pub(super) async fn enroll_member(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    user_id: &str,
    joined_at: DateTime<Utc>,
) -> AppResult<()> {
    let durable_db = crate::services::transaction_service::durable_database(db);
    let db = &durable_db;
    enroll(
        db,
        schedule,
        user_id,
        joined_at.max(schedule.created_at),
        joined_at,
    )
    .await?;
    restore_eligibility(db, &schedule.id, &[user_id.to_owned()], joined_at, true).await
}

async fn restore_eligibility(
    db: &mongodb::Database,
    schedule_id: &str,
    ids: &[String],
    now: DateTime<Utc>,
    joined: bool,
) -> AppResult<()> {
    let mut filter = doc! { "schedule_id": schedule_id, "recipient_user_id": { "$in": ids } };
    if !joined {
        filter.insert("is_eligible", false);
    }
    // A join must fence a stale worker even when this row is still eligible.
    // Preserve a future renewal when an already-eligible member is touched.
    db.collection::<CreditScheduleRecipient>(RECIPIENTS)
        .update_many(
            filter,
            vec![doc! { "$set": { "is_eligible": true,
            "next_due_at": { "$cond": [ { "$eq": ["$is_eligible", false] },
                bson::DateTime::from_chrono(now), "$next_due_at" ] },
            "eligibility_version": { "$add": [ { "$ifNull": ["$eligibility_version", 0] }, 1 ] },
            "updated_at": bson::DateTime::from_chrono(now) } }],
        )
        .await?;
    Ok(())
}

async fn mark_ineligible(
    db: &mongodb::Database,
    recipient: &CreditScheduleRecipient,
    now: DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<CreditScheduleRecipient>(RECIPIENTS)
        .update_one(
            doc! { "_id": &recipient.id, "is_eligible": true,
                "next_due_at": bson::DateTime::from_chrono(recipient.next_due_at),
                "$expr": { "$eq": [ { "$ifNull": ["$eligibility_version", 0] }, recipient.eligibility_version ] } },
            doc! { "$set": { "is_eligible": false,
                "updated_at": bson::DateTime::from_chrono(now) },
                "$inc": { "eligibility_version": 1_i64 } },
        )
        .await?;
    Ok(())
}

/// Check live eligibility in batches, including active organizations and people.
async fn eligible_ids(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    ids: &[String],
) -> AppResult<HashSet<String>> {
    let mut filter = doc! { "_id": { "$in": ids }, "is_active": true };
    match schedule.target_kind {
        BillingTargetKind::AllUsers => {}
        BillingTargetKind::SelectedUsers => {
            filter.insert(
                "_id",
                doc! { "$in": ids.iter()
                .filter(|id| schedule.target_user_ids.contains(id)).collect::<Vec<_>>() },
            );
        }
        BillingTargetKind::Groups => {
            filter.insert("user_type", doc! { "$ne": "org" });
            filter.insert("group_ids", doc! { "$in": &schedule.target_group_ids });
        }
        BillingTargetKind::OrgMembers => {
            let orgs: Vec<Document> = db
                .collection::<Document>(USERS)
                .find(doc! { "_id": { "$in": &schedule.target_org_ids },
                "is_active": true, "user_type": "org" })
                .projection(doc! { "_id": 1 })
                .await?
                .try_collect()
                .await?;
            let org_ids: Vec<_> = orgs
                .iter()
                .filter_map(|row| row.get_str("_id").ok())
                .collect();
            let members = db
                .collection::<Document>(MEMBERSHIPS)
                .distinct(
                    "member_user_id",
                    doc! { "org_user_id": { "$in": org_ids },
                    "member_user_id": { "$in": ids }, "revoked_at": Bson::Null },
                )
                .await?;
            filter.insert("_id", doc! { "$in": members });
            filter.insert("user_type", doc! { "$ne": "org" });
        }
    }
    let rows: Vec<Document> = db
        .collection::<Document>(USERS)
        .find(filter)
        .projection(doc! { "_id": 1 })
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect())
}

async fn discover(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    now: DateTime<Utc>,
    limit: usize,
) -> AppResult<()> {
    let scan = db
        .collection::<CreditScheduleRecipientScan>(SCANS)
        .find_one(doc! { "_id": &schedule.id })
        .await?;
    let cursor = scan
        .as_ref()
        .and_then(|scan| scan.cursor_user_id.as_deref());
    let ids = match schedule.target_kind {
        BillingTargetKind::Groups | BillingTargetKind::OrgMembers => {
            super::super::targets::member_recipients(
                db,
                schedule.target_kind,
                &schedule.target_org_ids,
                &schedule.target_group_ids,
                None,
                cursor,
                limit,
            )
            .await?
        }
        _ => {
            let mut filter = doc! { "is_active": true };
            if let Some(cursor) = cursor {
                filter.insert("_id", doc! { "$gt": cursor });
            }
            if schedule.target_kind == BillingTargetKind::SelectedUsers {
                filter.insert(
                    "$and",
                    vec![doc! { "_id": { "$in": &schedule.target_user_ids } }],
                );
            }
            let rows: Vec<Document> = db
                .collection::<Document>(USERS)
                .find(filter)
                .projection(doc! { "_id": 1 })
                .sort(doc! { "_id": 1 })
                .limit(limit as i64)
                .await?
                .try_collect()
                .await?;
            rows.into_iter()
                .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
                .collect()
        }
    };
    let live = eligible_ids(db, schedule, &ids).await?;
    let existing: Vec<CreditScheduleRecipient> = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find(doc! { "schedule_id": &schedule.id, "recipient_user_id": { "$in": &ids } })
        .await?
        .try_collect()
        .await?;
    let existing_ids: HashSet<_> = existing
        .iter()
        .map(|row| row.recipient_user_id.as_str())
        .collect();
    if !continue_reconcile() {
        return Ok(());
    }
    let observed_at = Utc::now().max(now);
    // Freeze new anchors in one bounded batch. A simultaneous group join or
    // discovery walk may win individual ids; their original anchors must win.
    let new_recipients: Vec<_> = ids
        .iter()
        .filter(|id| live.contains(*id) && !existing_ids.contains(id.as_str()))
        .map(|id| CreditScheduleRecipient {
            id: Uuid::new_v4().to_string(),
            schedule_id: schedule.id.clone(),
            recipient_user_id: id.clone(),
            anchor_at: observed_at,
            next_due_at: observed_at,
            is_eligible: true,
            eligibility_version: 0,
            created_at: observed_at,
            updated_at: observed_at,
        })
        .collect();
    if !new_recipients.is_empty()
        && let Err(error) = db
            .collection::<CreditScheduleRecipient>(RECIPIENTS)
            .insert_many(new_recipients)
            .ordered(false)
            .await
    {
        let mongodb::error::ErrorKind::InsertMany(failure) = error.kind.as_ref() else {
            return Err(error.into());
        };
        if failure.write_concern_error.is_some()
            || failure.write_errors.as_ref().is_none_or(|errors| {
                errors.is_empty() || errors.iter().any(|item| item.code != 11000)
            })
        {
            return Err(error.into());
        }
    }
    restore_eligibility(
        db,
        &schedule.id,
        &live.into_iter().collect::<Vec<_>>(),
        observed_at,
        false,
    )
    .await?;
    let next = if ids.len() == limit {
        ids.last().cloned()
    } else {
        None
    };
    db.collection::<CreditScheduleRecipientScan>(SCANS)
        .update_one(
            doc! { "_id": &schedule.id },
            doc! { "$set": { "cursor_user_id": next } },
        )
        .upsert(true)
        .await?;
    Ok(())
}

pub(super) async fn disburse(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    now: DateTime<Utc>,
    budget: &mut usize,
    stats: &mut DisbursementStats,
) -> AppResult<()> {
    if !schedule.is_active || *budget == 0 || !continue_reconcile() {
        return Ok(());
    }
    // An anchor must survive failover before any grant can use its identity.
    let durable_db = crate::services::transaction_service::durable_database(db);
    let db = &durable_db;
    discover(db, schedule, now, (*budget).min(MAX_RECIPIENTS_PER_TICK)).await?;
    if !continue_reconcile() {
        return Ok(());
    }
    let now = Utc::now().max(now);
    let due: Vec<CreditScheduleRecipient> = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find(doc! { "schedule_id": &schedule.id, "is_eligible": true,
        "next_due_at": { "$lte": bson::DateTime::from_chrono(now) } })
        .sort(doc! { "next_due_at": 1, "_id": 1 })
        .limit(*budget as i64)
        .await?
        .try_collect()
        .await?;
    if !continue_reconcile() {
        return Ok(());
    }
    let ids: Vec<_> = due
        .iter()
        .map(|row| row.recipient_user_id.clone())
        .collect();
    let live = eligible_ids(db, schedule, &ids).await?;
    for recipient in due {
        // Finish each insert/journal/renewal protocol before observing a drain.
        if !continue_reconcile() {
            break;
        }
        *budget -= 1;
        stats.recipients_examined += 1;
        // A full batch can run for minutes; resolve each window and expiry
        // against the current time so an elapsed window is never minted.
        let now = Utc::now().max(now);
        if !live.contains(&recipient.recipient_user_id) {
            mark_ineligible(db, &recipient, now).await?;
            continue;
        }
        issue_recipient_window(db, schedule, &recipient, now, stats).await?;
    }
    Ok(())
}

/// Capture a committed signup or join even while paused. Login only writes
/// when it discovers a missing or ineligible row.
pub(super) async fn load_member(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    user_id: &str,
    existing: Option<CreditScheduleRecipient>,
    joined: bool,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<Option<CreditScheduleRecipient>> {
    let row = match existing {
        Some(row) if row.is_eligible && !joined => Some(row),
        _ => {
            enroll_member(db, schedule, user_id, at).await?;
            if !schedule.is_active {
                return Ok(None);
            }
            db.collection::<CreditScheduleRecipient>(RECIPIENTS)
                .find_one(recipient_filter(&schedule.id, user_id))
                .await?
        }
    };
    Ok(row.filter(|row| schedule.is_active && row.is_eligible && row.next_due_at <= now))
}

pub(super) async fn issue_recipient_window(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    recipient: &CreditScheduleRecipient,
    now: DateTime<Utc>,
    stats: &mut DisbursementStats,
) -> AppResult<()> {
    let window = recipient_period(schedule.recurrence, recipient.anchor_at, now)?;
    let expires_at = resolve_expiry(&schedule.expiry, &window, window.start);
    if expires_at.is_some_and(|expiry| expiry <= now) {
        // A short expiry can elapse before a delayed worker runs. Do not
        // mint already-expired money; resume at this recipient's next cycle.
        db.collection::<CreditScheduleRecipient>(RECIPIENTS)
            .update_one(
                doc! { "_id": &recipient.id },
                doc! { "$max": { "next_due_at": bson::DateTime::from_chrono(window.end) } },
            )
            .await?;
        return Ok(());
    }
    let period = CreditSchedulePeriod {
        id: period_id(&schedule.id, window.start),
        schedule_id: schedule.id.clone(),
        period_start: window.start,
        period_end: window.end,
        status: SchedulePeriodStatus::Disbursing,
        amount: schedule.amount,
        expires_at,
        target_kind: schedule.target_kind,
        target_user_ids: vec![],
        target_org_ids: schedule.target_org_ids.clone(),
        target_group_ids: schedule.target_group_ids.clone(),
        scope: schedule.scope.clone(),
        service_limits: schedule.service_limits.clone(),
        reason: schedule.reason.clone(),
        cursor_user_id: None,
        disbursed_count: 0,
        lease_expires_at: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    let grants = grants_for_recipients(
        schedule,
        &period,
        vec![recipient.recipient_user_id.clone()],
        now,
    );
    let outcome = insert_grant_chunk(db, &grants).await?;
    #[cfg(test)]
    {
        tests::after_insert();
        if STALL_BEFORE_POSTING.try_with(|_| ()).is_ok() {
            std::future::pending::<()>().await;
        }
    }
    stats.grants_created += outcome.inserted_count;
    stats.already_disbursed += outcome.duplicate_count;
    // Journal the persisted snapshot, including after a concurrent insert or
    // a crash before the posting. Never journal an edited candidate amount.
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": &grants[0].id })
        .await?
        .ok_or_else(|| AppError::Internal("Recipient credit grant disappeared".into()))?;
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
        stats.grants_ledgered += 1;
    }
    // A stale worker cannot move next_due backwards after a newer renewal.
    db.collection::<CreditScheduleRecipient>(RECIPIENTS)
        .update_one(
            doc! { "_id": &recipient.id },
            doc! { "$max": { "next_due_at": bson::DateTime::from_chrono(window.end) },
            "$set": { "updated_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .update_one(
            doc! { "_id": &schedule.id },
            doc! { "$max": { "last_disbursed_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
