use super::*;
use crate::models::coordination::CoordinationHolder;
use crate::services::coordination_service::{LeaseStore, LeaseToken};

fn system_actor(owner: &str) -> AuditActor {
    AuditActor {
        user_id: owner.into(),
        ip_address: None,
        user_agent: Some("managed-line-renewal".into()),
        api_key_id: None,
        api_key_name: None,
    }
}
pub(super) async fn notice(
    db: &Database,
    tx: &mut ClientSession,
    c: &ManagedLineClaim,
    key: String,
    kind: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if db
        .collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
        .find_one(doc! {"key":&key})
        .session(&mut *tx)
        .await?
        .is_none()
    {
        db.collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
            .insert_one(ManagedLineNotice {
                id: Uuid::new_v4().to_string(),
                claim_id: c.id.clone(),
                user_id: c.user_id.clone(),
                line_id: c.line_id.clone(),
                key,
                kind: kind.into(),
                dispatched_at: None,
                created_at: now,
            })
            .session(tx)
            .await?;
    }
    Ok(())
}
async fn suspend_unfunded(db: &Database, claim: &str) -> AppResult<()> {
    transaction_service::run_db_only(db,&(db,claim),|tx,ctx|Box::pin(async move{
        let &(db,claim)=ctx;
        let Some(c)=db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find_one(doc!{"_id":claim,"state":"claimed"}).session(&mut *tx).await? else{return Ok(());};
        let now=Utc::now();
        // Contend with every renewal before deciding coverage is absent. A
        // delayed/unknown commit cannot be mistaken for an unfunded period.
        if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":claim,"fence":c.fence,"state":"claimed"},doc!{"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        if db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME).find_one(doc!{"claim_id":claim,"funded":true,"covered_from":{"$lte":bson::DateTime::from_chrono(now)},"covered_until":{"$gt":bson::DateTime::from_chrono(now)}}).session(&mut *tx).await?.is_some(){return Ok(());}
        let last=db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME).find_one(doc!{"claim_id":claim,"funded":true}).sort(doc!{"period_index":-1}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if last.covered_until>now{return Err(unavailable());}
        db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":claim},doc!{"$set":{"state":"suspended","suspension":"unpaid","suspended_at":bson::DateTime::from_chrono(last.covered_until)}}).session(&mut *tx).await?;
        if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&c.line_id,"current_claim_id":claim,"state":"claimed"},doc!{"$set":{"state":"suspended","suspension":"unpaid","updated_at":bson::DateTime::from_chrono(now)},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        disable(db,tx,claim,now).await?;notice(db,tx,&c,format!("unpaid:{}:{}",c.id,last.period_index),"unpaid",now).await?;Ok(())
    })).await
}
async fn prepare_renewal(
    state: &AppState,
    c: &ManagedLineClaim,
    period: &ManagedLinePeriod,
) -> AppResult<ManagedLineQuote> {
    let kind = QuoteKind::Renewal {
        claim_id: c.id.clone(),
        generation: c.managed_claim_generation,
        period_index: period.period_index + 1,
    };
    // One automatic quote covers the whole renewal window. A changed price
    // requires merchant consent through a separately requested manual quote;
    // sweep retries never mint successors every fifteen minutes.
    let old=state.db.collection::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME).find_one(doc!{"kind.kind":"renewal","kind.claim_id":&c.id,"kind.generation":c.managed_claim_generation,"kind.period_index":period.period_index+1,"automatic_request_id":{"$type":"string"}}).await?;
    if let Some(q) = old {
        return Ok(q);
    }
    quote_internal(state, &c.user_id, &c.line_id, kind, true).await
}
async fn process_claim(
    state: &AppState,
    c: ManagedLineClaim,
    fixture: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now();
    if c.state == LineState::Releasing {
        return drain(state, &c.id).await;
    }
    if c.state == LineState::Suspended {
        if c.suspension == Some(Suspension::Unpaid)
            && c.suspended_at
                .is_some_and(|at| at + Duration::days(7) <= now)
        {
            transaction_service::run_db_only(&state.db,&(&state.db,&c),|tx,ctx|Box::pin(async move{
                let &(db,c)=ctx;
                let current=db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find_one(doc!{"_id":&c.id,"state":"suspended","suspension":"unpaid","suspended_at":{"$lte":bson::DateTime::from_chrono(Utc::now()-Duration::days(7))}}).session(&mut *tx).await?.ok_or_else(unavailable)?;
                release_in_session(db,tx,&current,Utc::now()).await
            })).await?;
        }
        return drain(state, &c.id).await;
    }
    let period = latest_period(&state.db, &c.id).await?;
    if period.covered_until <= now {
        suspend_unfunded(&state.db, &c.id).await?;
        return drain(state, &c.id).await;
    }
    if period.covered_until > now + Duration::hours(72) {
        return Ok(());
    }
    transaction_service::run_db_only(
        &state.db,
        &(&state.db, &c, period.period_index),
        |tx, ctx| {
            Box::pin(async move {
                let &(db, c, index) = ctx;
                notice(
                    db,
                    tx,
                    c,
                    format!("renewal:{}:{}", c.id, index + 1),
                    "renewal",
                    Utc::now(),
                )
                .await
            })
        },
    )
    .await?;
    if !c.auto_renew {
        return Ok(());
    }
    // Do not create any quote while automatic consent no longer covers the
    // current terms/rate, or the flag/readiness disallows a new funded period.
    let e = eligibility_for(&state.db, &c.user_id).await?;
    let rate = state
        .billing
        .managed_price(
            &c.accepted_rental_rate.service_id,
            ManagedCommsRole::Rental,
            BillingMetric::PhoneNumberDays,
        )
        .await?;
    if c.accepted_terms != e.required_terms || c.accepted_rental_rate != rate {
        return Ok(());
    }
    let q = prepare_renewal(state, &c, &period).await?;
    if c.accepted_rental_rate == q.rental_rate && c.accepted_terms == q.terms {
        let request = q.automatic_request_id.clone().ok_or_else(unavailable)?;
        continue_with_origin(
            state,
            &system_actor(&c.user_id),
            ContinueInput {
                request_id: request,
                quote_id: q.id,
                auto_renew: true,
                accepted_terms_versions: q.terms,
            },
            fixture,
            true,
        )
        .await?;
    }
    Ok(())
}
async fn notifications(state: &AppState) -> AppResult<()> {
    let rows: Vec<ManagedLineNotice> = state
        .db
        .collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
        .find(doc! {"dispatched_at":bson::Bson::Null})
        .sort(doc! {"created_at":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        if state
            .db
            .collection::<Document>(ManagedLineErasure::COLLECTION_NAME)
            .find_one(doc! {"user_id":&row.user_id})
            .await?
            .is_some()
        {
            continue;
        }
        // Claim once before delivery; transport uncertainty never duplicates a
        // reminder. Durable notices are also visible on the line detail API.
        if state
            .db
            .collection::<Document>(ManagedLineNotice::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id,"dispatched_at":bson::Bson::Null},
                doc! {"$set":{"dispatched_at":bson::DateTime::now()}},
            )
            .await?
            .matched_count
            != 1
        {
            continue;
        }
        let message = if row.kind == "unpaid" {
            "Your phone line is suspended because its paid coverage ended. Resume within seven days to retain it."
        } else {
            "Your phone line renews within 72 hours. Review its rental quote and funding before the coverage cutoff."
        };
        let _ = crate::services::notification_service::send_managed_line_notification(
            &state.db,
            &state.config,
            &state.http_client,
            state.fcm_auth.as_deref(),
            state.apns_auth.as_deref(),
            &row.user_id,
            &row.line_id,
            message,
        )
        .await;
    }
    Ok(())
}
pub async fn sweep_once(state: &AppState) -> AppResult<u64> {
    sweep_with_origin(state, None).await
}
pub(crate) async fn sweep_with_origin(state: &AppState, fixture: Option<&str>) -> AppResult<u64> {
    // An untouched installation pays only a cheap indexed existence read and
    // does not require managed readiness merely to run ordinary billing.
    if state
        .db
        .collection::<Document>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {})
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(0);
    }
    let Some(_pass) = state.shutdown.background() else {
        return Ok(0);
    };
    let holder = CoordinationHolder {
        instance_id: format!("managed-lines:{}", std::process::id()),
        generation_id: Uuid::new_v4().to_string(),
    };
    let Some(lease) = LeaseStore::acquire(
        &state.db,
        "managed-line-lifecycle",
        &holder,
        std::time::Duration::from_secs(120),
    )
    .await?
    else {
        return Ok(0);
    };
    let result = leased_pass(state, &lease, fixture).await;
    let _ = LeaseStore::release(&state.db, &lease).await;
    result
}
async fn leased_pass(
    state: &AppState,
    lease: &LeaseToken,
    fixture: Option<&str>,
) -> AppResult<u64> {
    let after = LeaseStore::load_checkpoint(&state.db, lease)
        .await?
        .and_then(|v| v.as_str().map(str::to_owned));
    let mut filter = doc! {"state":{"$in":["claimed","suspended","releasing"]}};
    if let Some(after) = after.as_deref() {
        filter.insert("_id", doc! {"$gt":after});
    }
    let claims: Vec<ManagedLineClaim> = state
        .db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let mut processed = 0;
    for claim in &claims {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        if !LeaseStore::renew(&state.db, lease, std::time::Duration::from_secs(120)).await? {
            return Err(conflict("Managed lifecycle sweep lease changed"));
        }
        if process_claim(state, claim.clone(), fixture).await.is_err() {
            tracing::warn!(claim_id=%claim.id,"Managed lifecycle processing deferred; coverage remains closed without funding");
        }
        processed += 1;
    }
    // A drain stops between claims; the next holder resumes after the last
    // processed claim (or the previous checkpoint when none was processed).
    let drained = processed < claims.len();
    let resume = if drained || claims.len() == 100 {
        processed
            .checked_sub(1)
            .and_then(|index| claims.get(index))
            .map(|claim| claim.id.clone())
            .or(after)
    } else {
        None
    };
    let checkpoint = resume.map_or(bson::Bson::Null, bson::Bson::String);
    if !LeaseStore::store_checkpoint(&state.db, lease, checkpoint).await? {
        return Err(unavailable());
    }
    if drained {
        return Ok(processed as u64);
    }
    let count = claims.len() as u64;
    state.db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_many(doc!{"state":"cooldown","available_at":{"$lte":bson::DateTime::now()},"current_claim_id":bson::Bson::Null},doc!{"$set":{"state":"pending_reassignment","updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64,"fence":1_i64}}).await?;
    notifications(state).await?;
    Ok(count)
}
pub fn spawn_sweep(state: AppState) {
    state.shutdown.clone().spawn_worker(async move {
        let interval = state.config.billing_reconcile_interval_secs.max(1);
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval));
        loop {
            let Some(_work) = state.shutdown.tick(&mut tick).await else {
                break;
            };
            if super::sweep_once(&state).await.is_err() {
                tracing::warn!("Managed lifecycle sweep deferred");
            }
        }
    });
}
