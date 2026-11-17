//! Incurred SMS cost has one immutable outcome per part, even without a tenant.
use super::super::{binding, coordinator, session};
use super::{
    admission, budgets,
    ingress::{self, EventIdentity},
    lifecycle, unavailable,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_adapter::*,
        managed_comms::*,
        usage_meter::{COLLECTION_NAME as USAGE, UsageMeterRow},
    },
    services::transaction_service,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

#[allow(clippy::too_many_arguments)]
pub(super) async fn ingest(
    state: &AppState,
    l: &ManagedPhoneLine,
    a: &ProviderAccount,
    id: &EventIdentity,
    message: &str,
    from: &str,
    to: &str,
    text: &str,
    parts: i64,
) -> AppResult<String> {
    if !(1..=16).contains(&parts) {
        return Err(unavailable());
    }
    let candidate = if id.occurrence_verified && id.deliverable {
        ingress::current(state, l, true, id.occurred_at).await.ok()
    } else {
        None
    };
    let reference = id.reference(l, a, None, Some(message))?;
    let claim = if let Some(b) = &candidate {
        if let CredentialBinding::ManagedLine {
            managed_claim_id,
            managed_claim_generation,
            ..
        } = &b.credential
        {
            Some((managed_claim_id.clone(), *managed_claim_generation))
        } else {
            None
        }
    } else {
        None
    };
    let liability = ManagedInboundSms {
        id: uuid::Uuid::new_v4().to_string(),
        trusted_account_id: a.id.clone(),
        provider_message_id: message.into(),
        line_id: l.id.clone(),
        occurred_at: id.occurred_at,
        parts,
        outcomes: (0..parts)
            .map(|part| SmsPartOutcome::Pending { part })
            .collect(),
        delivery_claimed: false,
        quarantined: candidate.is_none(),
        financially_finalized: false,
        admitted: reference.clone(),
        created_at: Utc::now(),
    };
    let route = ManagedEffectRoute {
        id: uuid::Uuid::new_v4().to_string(),
        trusted_account_id: a.id.clone(),
        provider: l.provider.clone(),
        provider_effect_id: message.into(),
        line_id: l.id.clone(),
        managed_claim_id: claim.as_ref().map(|c| c.0.clone()),
        managed_claim_generation: claim.map(|c| c.1),
        binding_id: candidate.as_ref().map(|b| b.id.clone()),
        effect_id: liability.id.clone(),
        kind: EffectKind::Sms,
        financially_finalized: false,
        admitted: reference,
        covered_from: id.occurred_at,
        covered_until: None,
        created_at: Utc::now(),
    };
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,&liability,&route,l),|tx,c|Box::pin(async move{
        let (db,l,r,line)=*c;
        let old=db.collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME).find_one(doc!{"trusted_account_id":&l.trusted_account_id,"provider_message_id":&l.provider_message_id}).session(&mut *tx).await?;
        if old.is_none(){
            super::costs::register(db,tx,line,&l.id,EffectKind::Sms,&l.admitted,l.created_at).await?;
            db.collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME).insert_one(l).session(&mut *tx).await?;
            db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME).insert_one(r).session(&mut *tx).await?;
        }
        Ok(())
    })).await?;
    let saved = state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"trusted_account_id":&a.id,"provider_message_id":message})
        .await?
        .ok_or_else(unavailable)?;
    recover_one(state, &saved.id).await?;
    let saved = state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":&saved.id})
        .await?
        .ok_or_else(unavailable)?;
    // Content exists only during this authenticated request. Recovery accounts
    // parts, never reconstructs content or automatically replays an agent turn.
    if !saved.quarantined
        && saved.financially_finalized
        && saved
            .outcomes
            .iter()
            .all(|o| matches!(o, SmsPartOutcome::CustomerFunded { .. }))
        && let Some(b) = candidate
    {
        let current = ingress::current(state, l, true, id.occurred_at).await?;
        if current.id != b.id {
            return Ok("<Response/>".into());
        }
        let a = admission::read(&state.db, &b.user_id, &b.credential).await?;
        let mut s = session::new_row(&b, &format!("sms:{message}"), from, to);
        s.id = saved.id.clone();
        s.admitted_reference = Some(saved.admitted.clone());
        s.managed_claim_id = Some(a.claim.id.clone());
        s.managed_claim_generation = Some(a.claim.managed_claim_generation);
        let won = transaction_service::run_db_only(&db, &(&db, &a, &b, &s), |tx, c| {
            Box::pin(async move {
                let (db, a, b, s) = *c;
                admission::fence(db, tx, a, Utc::now()).await?;
                if db
                    .collection::<Binding>(BINDINGS)
                    .update_one(
                        doc! {"_id":&b.id,"enabled":true,"revision":b.revision},
                        doc! {"$inc":{"admission_fence":1_i64}},
                    )
                    .session(&mut *tx)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(unavailable());
                }
                let won = db
                    .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&s.id,"delivery_claimed":false,"financially_finalized":true},
                        doc! {"$set":{"delivery_claimed":true}},
                    )
                    .session(&mut *tx)
                    .await?
                    .modified_count
                    == 1;
                if won {
                    db.collection::<CallSession>(SESSIONS)
                        .insert_one(s)
                        .session(&mut *tx)
                        .await?;
                }
                Ok(won)
            })
        })
        .await?;
        if won {
            let state = state.clone();
            let text = text.to_owned();
            let event = message.to_owned();
            state.shutdown.clone().spawn(async move {
                let sid = s.id.clone();
                let _ = coordinator::sms_turn(state.clone(), b, s, event, text).await;
                let _ = session::finish_noncall(&state, &sid, "sms_turn_finished").await;
            });
        }
    }
    Ok("<Response/>".into())
}
async fn absorb(state: &AppState, id: &str, part: i64) -> AppResult<()> {
    // Exact pending predicate prevents either recovery or a later claimant from
    // billing a previously absorbed part. Other outcomes cannot be overwritten.
    let path = format!("outcomes.{part}");
    let mut filter = doc! {"_id":id};
    filter.insert(format!("{path}.state"), "pending");
    let mut set = doc! {};
    set.insert(
        path,
        bson::to_bson(&SmsPartOutcome::Absorbed { part }).map_err(|_| unavailable())?,
    );
    state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .update_one(filter, doc! {"$set":set})
        .await?;
    Ok(())
}
pub async fn recover_one(state: &AppState, id: &str) -> AppResult<()> {
    let row = state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)?;
    if row.financially_finalized {
        return Ok(());
    }
    let route = state
        .db
        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .find_one(doc! {"effect_id":id,"kind":"sms"})
        .await?
        .ok_or_else(unavailable)?;
    for outcome in &row.outcomes {
        let SmsPartOutcome::Pending { part } = outcome else {
            continue;
        };
        let part = *part;
        let eligible = async {
            if row.quarantined {
                return Err(unavailable());
            }
            let b =
                binding::load(state, route.binding_id.as_deref().ok_or_else(unavailable)?).await?;
            binding::live(state, &b).await?;
            let a = admission::read(&state.db, &b.user_id, &b.credential).await?;
            if route.managed_claim_id.as_deref() != Some(&a.claim.id)
                || route.managed_claim_generation != Some(a.claim.managed_claim_generation)
                || row.occurred_at < a.claim.covered_from
            {
                return Err(unavailable());
            }
            let cap = a.line.sms.as_ref().ok_or_else(unavailable)?;
            let f = state
                .billing
                .prepare_managed_effect(&b.user_id, &cap.billing_service_id, ManagedCommsRole::Sms)
                .await?;
            Ok((a, b, f))
        }
        .await;
        let (a, b, f) = match eligible {
            Ok(v) => v,
            Err(e @ AppError::DatabaseError(_)) => return Err(e),
            Err(_) => {
                absorb(state, id, part).await?;
                continue;
            }
        };
        let request = format!("managed-inbound:{id}:{part}");
        let db = transaction_service::durable_database(&state.db);
        let result = transaction_service::run_db_only(
            &db,
            &(&db, &a, &b, &f, &request, id, part),
            |tx, c| {
                Box::pin(async move {
                    let (db, a, b, f, request, id, part) = *c;
                    let path = format!("outcomes.{part}");
                    let mut filter = doc! {"_id":id};
                    filter.insert(format!("{path}.state"), "pending");
                    let current = db
                        .collection::<bson::Document>(ManagedInboundSms::COLLECTION_NAME)
                        .find_one(filter.clone())
                        .session(&mut *tx)
                        .await?;
                    let Some(current) = current else {
                        return Ok(());
                    };
                    let now = Utc::now();
                    admission::fence(db, tx, a, now).await?;
                    if !current.get_bool("rate_admitted").unwrap_or(false) {
                        ingress::rate(db, tx, &a.line, "sms", a.line.sms_per_minute, now).await?;
                    }
                    f.reserve(db, tx, &b.user_id, request, now).await?;
                    let hold =
                        lifecycle::monthly_hold(db, tx, &a.eligibility, request, f.rate.rate, now)
                            .await?;
                    if let Some(hold) = hold {
                        budgets::recognize(db, tx, &hold, f.rate.rate, now).await?;
                    }
                    let mut set = doc! {"rate_admitted":true};
                    set.insert(
                        path,
                        bson::to_bson(&SmsPartOutcome::CustomerFunded {
                            part,
                            claim_id: a.claim.id.clone(),
                            billing_request_id: request.to_owned(),
                        })
                        .map_err(|_| unavailable())?,
                    );
                    db.collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
                        .update_one(filter, doc! {"$set":set})
                        .session(&mut *tx)
                        .await?;
                    Ok(())
                })
            },
        )
        .await;
        if let Err(e) = result {
            match e {
                AppError::InsufficientCredits
                | AppError::ManagedBudgetExhausted
                | AppError::Conflict(_)
                | AppError::WalletSuspended => {
                    absorb(state, id, part).await?;
                }
                other => return Err(other),
            }
        }
    }
    let row = state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)?;
    for o in &row.outcomes {
        match o {
            SmsPartOutcome::Pending { .. } => return Err(unavailable()),
            SmsPartOutcome::CustomerFunded {
                billing_request_id, ..
            } => {
                let usage = state
                    .db
                    .collection::<UsageMeterRow>(USAGE)
                    .find_one(doc! {"billing_request_id":billing_request_id})
                    .await?
                    .ok_or_else(unavailable)?;
                super::super::billing::settle_admitted(
                    state,
                    billing_request_id,
                    &usage.actor_user_id,
                    0,
                    1,
                )
                .await?;
                if state
                    .db
                    .collection::<UsageMeterRow>(USAGE)
                    .find_one(doc! {"_id":&usage.id,"released":true,"quantity":1_i64})
                    .await?
                    .is_none()
                {
                    return Err(unavailable());
                }
            }
            SmsPartOutcome::Absorbed { .. } => {}
        }
    }
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db, &(&db, id), |tx, c| {
        Box::pin(async move {
            let (db, id) = *c;
            db.collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":id},
                    doc! {"$set":{"financially_finalized":true}},
                )
                .session(&mut *tx)
                .await?;
            db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
                .update_one(
                    doc! {"effect_id":id,"kind":"sms"},
                    doc! {"$set":{"financially_finalized":true}},
                )
                .session(&mut *tx)
                .await?;
            Ok(())
        })
    })
    .await
}
pub async fn sweep(state: &AppState) -> AppResult<()> {
    let rows: Vec<ManagedInboundSms> = state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find(doc! {"financially_finalized":false})
        .sort(doc! {"recovery_checked_at":1,"_id":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        state
            .db
            .collection::<bson::Document>(ManagedInboundSms::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"recovery_checked_at":bson::DateTime::now()}},
            )
            .await?;
        if recover_one(state, &row.id).await.is_err() {
            tracing::warn!(liability_id=%row.id,"Managed inbound SMS accounting pending");
        }
    }
    Ok(())
}

/// Reply sends retain the same exact monthly retail budget as incurred parts.
pub(in crate::services::call_channels) async fn hold_send(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    a: &admission::Admission,
    _s: &CallSession,
    row: &EventReceipt,
) -> AppResult<()> {
    let now = Utc::now();
    ingress::rate(db, tx, &a.line, "sms", a.line.sms_per_minute, now).await?;
    let parts = row
        .reserved_parts
        .filter(|n| (1..=16).contains(n))
        .ok_or_else(unavailable)?;
    let mut rates = Vec::new();
    let mut total = crate::models::credits::Credits::ZERO;
    for part in 0..parts {
        let r=db.collection::<UsageMeterRow>(USAGE).find_one(doc!{"billing_request_id":super::super::sms_delivery::outbound_request_id(row,part)?,"actor_user_id":&a.claim.user_id,"released":false}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        let f = r.funding.as_ref().ok_or_else(unavailable)?;
        let rate = crate::models::credits::Credits::from_pico(
            crate::services::billing::amounts::rate_pico(
                f.credits_per_unit_pico,
                f.credits_per_unit_micros,
            ),
        )?;
        if rate <= crate::models::credits::Credits::ZERO {
            return Err(unavailable());
        }
        total = total.checked_add(rate)?;
        rates.push(rate.decimal());
    }
    let hold = lifecycle::monthly_hold(
        db,
        tx,
        &a.eligibility,
        &format!("managed-sms-send:{}", row.id),
        total,
        now,
    )
    .await?;
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"managed_spend_hold_id":hold,"managed_part_rates":rates}},
        )
        .session(&mut *tx)
        .await?;
    Ok(())
}
pub(in crate::services::call_channels) async fn recognize_send(
    state: &AppState,
    row: &EventReceipt,
) -> AppResult<()> {
    let raw = state
        .db
        .collection::<bson::Document>(EVENTS)
        .find_one(doc! {"_id":&row.id})
        .await?
        .ok_or_else(unavailable)?;
    let Ok(hold) = raw.get_str("managed_spend_hold_id") else {
        return Ok(());
    };
    let actual = if row.status == "rejected_before_dispatch"
        || raw.get_str("terminal_evidence") == Ok(super::reservations::WAIVED)
    {
        0
    } else {
        row.observed_parts.ok_or_else(unavailable)?
    };
    let rates = raw
        .get_array("managed_part_rates")
        .map_err(|_| unavailable())?;
    if actual < 0 || actual as usize > rates.len() {
        return Err(unavailable());
    }
    let mut total = crate::models::credits::Credits::ZERO;
    for rate in rates.iter().take(actual as usize) {
        total = total.checked_add(crate::models::credits::Credits::from_bson(rate.clone(), 1)?)?;
    }
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db, &(&db, hold, total, &row.id), |tx, c| {
        Box::pin(async move {
            let (db, id, total, send) = *c;
            budgets::recognize(db, tx, id, total, Utc::now()).await?;
            db.collection::<bson::Document>(EVENTS)
                .update_one(
                    doc! {"_id":send},
                    doc! {"$set":{"managed_spend_recognized":true}},
                )
                .session(&mut *tx)
                .await?;
            Ok(())
        })
    })
    .await
}
