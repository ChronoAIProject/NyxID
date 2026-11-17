//! Exact capacity holds survive window boundaries until evidence resolves them.
use super::{conflict, unavailable};
use crate::{
    errors::{AppError, AppResult},
    models::{credits::Credits, managed_comms::*},
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Used by the M1 budget primitive; production consumers arrive in M2/M4"
    )
)]
fn start(window: &BudgetWindow, now: DateTime<Utc>) -> AppResult<DateTime<Utc>> {
    match *window {
        BudgetWindow::RollingThirtyDays => Ok(now - Duration::days(30)),
        BudgetWindow::CalendarMonth { year, month } => Utc
            .with_ymd_and_hms(year, month, 1, 0, 0, 0)
            .single()
            .ok_or_else(unavailable),
    }
}
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Used by the M1 budget primitive; production consumers arrive in M2/M4"
    )
)]
fn counts(b: &ManagedBudget, entry: &ManagedBudgetEntry, now: DateTime<Utc>) -> AppResult<bool> {
    Ok(match b.window {
        BudgetWindow::RollingThirtyDays => entry
            .recognized_at
            .is_some_and(|at| at > now - Duration::days(30)),
        BudgetWindow::CalendarMonth { year, month } => {
            entry.admitted_at.year() == year && entry.admitted_at.month() == month
        }
    })
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M2 lifecycle and M4 call admission consume this M1 transaction primitive"
    )
)]
pub async fn hold(
    db: &Database,
    tx: &mut ClientSession,
    budget: &ManagedBudget,
    effect_key: &str,
    candidate: Credits,
    now: DateTime<Utc>,
) -> AppResult<ManagedBudgetEntry> {
    super::require_ready(db).await?;
    if matches!(budget.window, BudgetWindow::CalendarMonth {year,month} if now.year()!=year || now.month()!=month)
    {
        return Err(conflict(
            "Managed spend budget does not cover this admission month",
        ));
    }
    if candidate < Credits::ZERO || effect_key.is_empty() || budget.limit < Credits::ZERO {
        return Err(unavailable());
    }
    let budgets = db.collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME);
    let changed = budgets
        .update_one(
            doc! {"_id":&budget.id,"revision":budget.revision,"limit":budget.limit.decimal()},
            doc! {"$inc":{"fence":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(conflict("Managed budget changed"));
    }
    let entries = db.collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME);
    if let Some(old) = entries
        .find_one(doc! {"budget_id":&budget.id,"effect_key":effect_key})
        .session(&mut *tx)
        .await?
    {
        if old.held != candidate {
            return Err(conflict("Managed budget request payload changed"));
        }
        return Ok(old);
    }
    // The budget row write serializes this read and insertion across replicas.
    // Entries are independently retained; no unresolved hold expires with a window.
    let cutoff = start(&budget.window, now)?;
    let mut cursor = entries.find(doc!{"budget_id":&budget.id,"$or":[{"recognized":bson::Bson::Null},{"recognized_at":{"$gt":bson::DateTime::from_chrono(cutoff)}},{"admitted_at":{"$gte":bson::DateTime::from_chrono(cutoff)}}]}).session(&mut *tx).await?;
    let mut used = Credits::ZERO;
    while let Some(entry) = cursor.next(&mut *tx).await.transpose()? {
        used = used.checked_add(match entry.recognized {
            None => entry.held,
            Some(v) if counts(budget, &entry, now)? => v,
            _ => Credits::ZERO,
        })?;
    }
    if used.checked_add(candidate)? > budget.limit {
        return Err(AppError::ManagedBudgetExhausted);
    }
    let entry = ManagedBudgetEntry {
        id: uuid::Uuid::new_v4().to_string(),
        budget_id: budget.id.clone(),
        effect_key: effect_key.into(),
        held: candidate,
        recognized: None,
        admitted_at: now,
        recognized_at: None,
    };
    entries.insert_one(&entry).session(&mut *tx).await?;
    Ok(entry)
}

/// A loss above its original hold is recognized in full; the next hold refuses.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M2/M4 settlements consume this M1 transaction primitive"
    )
)]
pub async fn recognize(
    db: &Database,
    tx: &mut ClientSession,
    entry_id: &str,
    amount: Credits,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if amount < Credits::ZERO {
        return Err(unavailable());
    }
    let entries = db.collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME);
    let old = entries
        .find_one(doc! {"_id":entry_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(unavailable)?;
    if let Some(previous) = old.recognized {
        return if previous == amount {
            Ok(())
        } else {
            Err(conflict("Managed budget evidence changed"))
        };
    }
    let changed = db
        .collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
        .update_one(doc! {"_id":&old.budget_id}, doc! {"$inc":{"fence":1_i64}})
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(unavailable());
    }
    let changed = entries.update_one(doc!{"_id":entry_id,"recognized":bson::Bson::Null},doc!{"$set":{"recognized":amount.decimal(),"recognized_at":bson::DateTime::from_chrono(now)}}).session(&mut *tx).await?;
    if changed.matched_count != 1 {
        return Err(conflict("Managed budget settlement changed"));
    }
    Ok(())
}
