//! Exact daily guest exposure, held in the same transaction as billing funding.
//! A day is the UTC admission day, even if settlement crosses midnight. Holds
//! survive crashes; terminal release/settlement uses the usage ID exactly once.
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};

use super::GuestPayer;
use crate::{
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        org_channel::{BUDGETS, HOLDS},
        usage_meter::UsageMeterRow,
    },
};

pub(crate) async fn require_wallet(db: &Database, org: &str) -> AppResult<()> {
    let wallet = crate::services::billing::provisioning::get_wallet(db, org)
        .await?
        .ok_or_else(|| {
            AppError::BillingNotConfigured(
                "Provision the organization's billing wallet before enabling channel guests".into(),
            )
        })?;
    if wallet
        .lago_subscription_id
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err(AppError::BillingNotConfigured(
            "The organization's billing wallet needs an active subscription".into(),
        ));
    }
    if wallet.is_suspended() {
        return Err(AppError::WalletSuspended);
    }
    Ok(())
}

pub(crate) async fn require_payer(db: &Database, payer: &GuestPayer) -> AppResult<()> {
    super::validate(db, &payer.origin, &payer.org_id, &payer.agent_id).await?;
    Ok(())
}

pub(crate) fn day_key(channel: &str, now: chrono::DateTime<Utc>) -> String {
    format!("{channel}:{}", now.format("%Y-%m-%d"))
}

fn money(doc: &Document, key: &str) -> AppResult<Credits> {
    Credits::from_bson(
        doc.get(key).cloned().unwrap_or(bson::Bson::Int64(0)),
        crate::models::credits::SCALE,
    )
    .map_err(Into::into)
}

pub(crate) async fn reserve(
    db: &Database,
    session: &mut ClientSession,
    payer: &GuestPayer,
    rows: &[UsageMeterRow],
    layers: &[crate::services::billing::reservation::LayerReservation],
) -> AppResult<()> {
    let access = Box::pin(super::validate_in_session(
        db,
        session,
        &payer.origin,
        &payer.org_id,
        &payer.agent_id,
        true,
    ))
    .await?;
    let policy = &access
        .channel
        .org_specialist
        .as_ref()
        .ok_or_else(super::denied)?
        .guests;
    let cap = policy.daily_credits.ok_or_else(super::denied)?;
    let admitted_at = Utc::now();
    let day = day_key(&payer.channel_id, admitted_at);
    db.collection::<Document>(BUDGETS).update_one(doc!{"_id":&day},doc!{"$setOnInsert":{
        "channel_id":&payer.channel_id,"org_id":&payer.org_id,"day":admitted_at.format("%Y-%m-%d").to_string(),
        "reserved_credits":Credits::ZERO,"spent_credits":Credits::ZERO,"created_at":bson::DateTime::now()}})
        .upsert(true).session(&mut *session).await?;
    for row in rows {
        if row.billing_owner_id != payer.org_id {
            return Err(super::denied());
        }
        if db
            .collection::<Document>(HOLDS)
            .find_one(doc! {"_id":&row.id})
            .session(&mut *session)
            .await?
            .is_some()
        {
            continue;
        }
        let Some(funding) = &row.funding else {
            continue;
        };
        let layer = layers
            .iter()
            .find(|l| l.layer == row.layer && l.lago_metric_code == row.lago_metric_code)
            .ok_or_else(super::denied)?;
        // Reuse the actual funding reservation: gross wallet share already
        // rounds tax upward, while allowance/grant funding is untaxed. The
        // exposure cap counts all consumed value, including those benefits.
        let rate = crate::services::billing::amounts::rate_pico(
            funding.credits_per_unit_pico,
            funding.credits_per_unit_micros,
        );
        let mut held = layer.reserved_credits;
        for grant in &layer.grant_reservations {
            held = held.checked_add(grant.amount)?;
        }
        for allowance in &layer.allowance_reservations {
            held = held.checked_add(crate::services::billing::amounts::cost(
                rate,
                allowance.quantity,
            )?)?;
        }
        let changed = db.collection::<Document>(BUDGETS).update_one(doc!{"_id":&day,
            "$expr":{"$lte":[{"$add":["$spent_credits","$reserved_credits",held.decimal()]},cap.decimal()]}},
            doc!{"$inc":{"reserved_credits":held},"$set":{"updated_at":bson::DateTime::now()}}).session(&mut *session).await?;
        if changed.matched_count != 1 {
            return Err(AppError::Forbidden(
                "org_channel_guest_budget_exhausted".into(),
            ));
        }
        db.collection::<Document>(HOLDS).insert_one(doc!{"_id":&row.id,"budget_id":&day,"channel_id":&payer.channel_id,
            "org_id":&payer.org_id,"sender_hash":&payer.sender_hash,"binding_revision":&payer.revision,
            "reserved_credits":held,"state":"reserved","created_at":bson::DateTime::now()}).session(&mut *session).await?;
    }
    Ok(())
}

pub(crate) async fn settle(db: &Database, usage_id: &str, actual: Credits) -> AppResult<()> {
    // Legacy/other lanes do not acquire a transaction or budget write.
    if db
        .collection::<Document>(HOLDS)
        .find_one(doc! {"_id":usage_id,"state":"reserved"})
        .await?
        .is_none()
    {
        return Ok(());
    }
    let ctx = (db.clone(), usage_id.to_owned(), actual);
    crate::services::transaction_service::run_db_only(db,&ctx,|tx,(db,id,actual)| Box::pin(async move {
        let Some(hold)=db.collection::<Document>(HOLDS).find_one_and_update(doc!{"_id":id,"state":"reserved"},
            doc!{"$set":{"state":"settled","spent_credits":*actual,"updated_at":bson::DateTime::now()}}).session(&mut *tx).await? else { return Ok(()); };
        let held=money(&hold,"reserved_credits")?;
        let budget=hold.get_str("budget_id").map_err(|_| super::denied())?;
        let updated=db.collection::<Document>(BUDGETS).update_one(doc!{"_id":budget,"reserved_credits":{"$gte":held.decimal()}},
            doc!{"$inc":{"reserved_credits":held.checked_neg()?,"spent_credits":*actual},"$set":{"updated_at":bson::DateTime::now()}}).session(tx).await?;
        if updated.matched_count != 1 { return Err(AppError::BillingProviderUnavailable("Organization guest budget settlement is pending".into())); }
        Ok(())
    })).await
}

pub(crate) async fn today(db: &Database, channel: &str) -> AppResult<(Credits, Credits)> {
    match db
        .collection::<Document>(BUDGETS)
        .find_one(doc! {"_id":day_key(channel,Utc::now())})
        .await?
    {
        Some(row) => Ok((
            money(&row, "spent_credits")?,
            money(&row, "reserved_credits")?,
        )),
        None => Ok((Credits::ZERO, Credits::ZERO)),
    }
}
