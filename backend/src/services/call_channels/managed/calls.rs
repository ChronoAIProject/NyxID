//! One durable admission precedes answer; provider termination and money settle separately.
use super::super::{binding, coordinator, profiles, session};
use super::{
    admission, budgets, conflict, ingress::EventIdentity, lifecycle, unavailable, underwriting,
};
use crate::{
    AppState,
    errors::AppResult,
    models::{
        channel_adapter::*,
        credits::Credits,
        managed_comms::*,
        usage_meter::{COLLECTION_NAME as USAGE, UsageMeterRow},
    },
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

pub(in crate::services::call_channels) async fn prepare(
    state: &AppState,
    b: &Binding,
    call: &str,
    from: &str,
    to: &str,
    identity: Option<&EventIdentity>,
) -> AppResult<(CallSession, bool)> {
    if let Ok(saved) = session::by_call(state, b, call).await {
        if saved.from != from || saved.to != to {
            return Err(unavailable());
        }
        receipt(state, &saved.id).await?;
        return Ok((saved, false));
    }
    super::resolver::read_binding(&state.db, b).await?;
    let a = admission::read(&state.db, &b.user_id, &b.credential).await?;
    let voice_config = super::synthesis::config(&state.db, &a.account.id).await?;
    let premium = if a.claim.voice_tier == VoiceTier::Premium {
        let option = a.claim.standard_voice.as_ref().ok_or_else(unavailable)?;
        super::synthesis::validate_selection(
            &state.db,
            &a.account.id,
            &a.eligibility,
            &a.claim.accepted_terms,
            option,
        )
        .await?;
        let p = voice_config
            .as_ref()
            .and_then(|c| c.premium.as_ref())
            .ok_or_else(unavailable)?;
        Some(
            state
                .billing
                .prepare_managed_effect(
                    &b.user_id,
                    &p.billing_service_id,
                    ManagedCommsRole::Premium,
                )
                .await?,
        )
    } else {
        None
    };
    let identity = identity.ok_or_else(unavailable)?;
    let cap = a.line.voice.as_ref().ok_or_else(unavailable)?;
    let base = state
        .billing
        .prepare_managed_effect(&b.user_id, &cap.billing_service_id, ManagedCommsRole::Voice)
        .await?;
    let underwriting = if a.line.provider == "telnyx" {
        let p = a.account.underwriting.as_ref().ok_or_else(unavailable)?;
        underwriting::validate(p)?;
        if !p.enabled {
            return Err(unavailable());
        }
        Some(p.clone())
    } else {
        None
    };
    let mut row = session::new_row(b, call, from, to);
    row.billing_request_id = format!("managed-call:{}:base", row.id);
    row.managed_claim_id = Some(a.claim.id.clone());
    row.managed_claim_generation = Some(a.claim.managed_claim_generation);
    let reference = identity.reference(&a.line, &a.account, Some(call), None)?;
    row.admitted_reference = Some(reference.clone());
    row.provider_call_leg_id = reference.call_leg_id.clone();
    row.provider_call_session_id = reference.call_session_id.clone();
    let db = transaction_service::durable_database(&state.db);
    let context = (
        &db,
        &a,
        b,
        &row,
        &base,
        &underwriting,
        &reference,
        &premium,
        &voice_config,
    );
    let result = transaction_service::run_db_only(&db, &context, |tx, c| {
        Box::pin(async move {
            let (db, a, b, row, base, underwriting, reference, premium, voice_config) = *c;
            let now = Utc::now();
            admission::fence(db, tx, a, now).await?;
            if db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
                .find_one(doc! {"trusted_account_id":&a.account.id,"provider":&a.line.provider,"provider_effect_id":reference.provider_call_id.as_deref()})
                .session(&mut *tx).await?.is_some() {
                return Err(conflict("Provider effect has already been routed"));
            }
            super::ingress::rate(db, tx, &a.line, "voice", a.line.calls_per_minute, now).await?;
            let limit = a.line.call_slot_limit.min(
                a.line
                    .provider_channel_limit
                    .unwrap_or(a.line.call_slot_limit),
            );
            let held = db
                .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
                .count_documents(doc! {"admitted.line_id":&a.line.id,"slot_held":true})
                .session(&mut *tx)
                .await?;
            if limit <= 0 || held >= limit as u64 {
                return Err(conflict("Managed line call capacity is full"));
            }
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
            let (base_meter, premium_meter) =
                reserve_meters(db, tx, &b.user_id, &row.id, base, premium.as_ref(), now).await?;
            let spend = lifecycle::monthly_hold(
                db,
                tx,
                &a.eligibility,
                &row.billing_request_id,
                base.rate.rate.checked_add(premium.as_ref().map_or(Credits::ZERO, |p|p.rate.rate))?.checked_mul(1800)?,
                now,
            )
            .await?;
            let under = if let Some(p) = underwriting {
                let budget = underwriting::budget(db, tx, &a.account.id, p).await?;
                Some(
                    budgets::hold(
                        db,
                        tx,
                        &budget,
                        &row.billing_request_id,
                        super::custody::worst_case_exposure(p)?,
                        now,
                    )
                    .await?
                    .id,
                )
            } else {
                None
            };
            let r = ManagedCallAdmission {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: row.id.clone(),
                managed_claim_id: a.claim.id.clone(),
                managed_claim_generation: a.claim.managed_claim_generation,
                admitted: reference.clone(),
                base: base_meter,
                premium: premium_meter,
                underwriting_snapshot: underwriting.clone(),
                underwriting_hold_id: under,
                spend_hold_id: spend,
                slot_held: true,
                financially_finalized: false,
                answer_dispatch_anchor: None,
                hangup_deadline: None,
                created_at: now,
            };
            db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
                .insert_one(r)
                .session(&mut *tx)
                .await?;
            if b.profile == Profile::TelnyxRelay {
                super::synthesis::initialize(db, tx, row, voice_config.as_ref(), premium.is_some()).await?;
            }
            super::costs::register(db, tx, &a.line, &row.id, EffectKind::Call, reference, now).await?;
            db.collection::<CallSession>(SESSIONS)
                .insert_one(row)
                .session(&mut *tx)
                .await?;
            let route = ManagedEffectRoute {
                id: uuid::Uuid::new_v4().to_string(),
                trusted_account_id: a.account.id.clone(),
                provider: a.line.provider.clone(),
                provider_effect_id: call.into(),
                line_id: a.line.id.clone(),
                managed_claim_id: Some(a.claim.id.clone()),
                managed_claim_generation: Some(a.claim.managed_claim_generation),
                binding_id: Some(b.id.clone()),
                effect_id: row.id.clone(),
                kind: EffectKind::Call,
                financially_finalized: false,
                admitted: reference.clone(),
                covered_from: a.period.covered_from,
                covered_until: Some(a.period.covered_until),
                created_at: now,
            };
            db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
                .insert_one(route)
                .session(&mut *tx)
                .await?;
            Ok(())
        })
    })
    .await;
    match result {
        Ok(()) => Ok((session::load(state, &row.id).await?, true)),
        Err(e) => {
            // Unknown commit or competing duplicate: resolve the original provider identity.
            if let Ok(saved) = session::by_call(state, b, call).await {
                receipt(state, &saved.id).await?;
                if saved.from == from && saved.to == to {
                    return Ok((saved, false));
                }
            }
            Err(e)
        }
    }
}
/// Both retail meters use one caller transaction.
#[allow(clippy::too_many_arguments)]
pub(super) async fn reserve_meters(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    owner: &str,
    session: &str,
    base: &crate::services::billing::managed_lifecycle::RentalFunding,
    premium: Option<&crate::services::billing::managed_lifecycle::RentalFunding>,
    now: DateTime<Utc>,
) -> AppResult<(MeterAdmission, Option<MeterAdmission>)> {
    let mut meters = Vec::new();
    for (suffix, f) in std::iter::once(("base", base)).chain(premium.map(|p| ("premium", p))) {
        let request = format!("managed-call:{session}:{suffix}");
        let id = f.reserve(db, tx, owner, &request, now).await?;
        meters.push(MeterAdmission {
            billing_request_id: request,
            usage_row_ids: vec![id],
            rate: f.rate.clone(),
            reserved_quantity: 1800,
        });
    }
    let premium = if meters.len() == 2 {
        meters.pop()
    } else {
        None
    };
    Ok((meters.remove(0), premium))
}
pub async fn receipt(state: &AppState, session: &str) -> AppResult<ManagedCallAdmission> {
    state
        .db
        .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
        .find_one(doc! {"session_id":session})
        .await?
        .ok_or_else(|| conflict("Managed call admission receipt is missing; recovery required"))
}
/// The first action that can answer binds the watchdog before network dispatch.
pub async fn anchor(state: &AppState, s: &CallSession) -> AppResult<()> {
    let r = receipt(state, &s.id).await?;
    if r.answer_dispatch_anchor.is_some() {
        return Ok(());
    }
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,&s.id),|tx,c|Box::pin(async move{
        let (db,id)=*c; let now=Utc::now();let deadline=now+Duration::seconds(1740);
        let won=db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc!{"session_id":id,"answer_dispatch_anchor":bson::Bson::Null},doc!{"$set":{"answer_dispatch_anchor":bson::DateTime::from_chrono(now),"hangup_deadline":bson::DateTime::from_chrono(deadline)}}).session(&mut *tx).await?;
        if won.modified_count==1 { db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(deadline)}}).session(&mut *tx).await?; }
        Ok(())
    })).await?;
    arm(state, s);
    Ok(())
}
pub(super) fn arm(state: &AppState, s: &CallSession) {
    let state = state.clone();
    let s = s.clone();
    // The timer waits as infrastructure and only the hangup is claimed work, so
    // it never delays a drain; any replica's sweep is the backstop.
    state.shutdown.clone().spawn_worker(async move {
        if let Ok(r) = receipt(&state, &s.id).await
            && let Some(deadline) = r.hangup_deadline
        {
            if let Ok(delay) = (deadline - Utc::now()).to_std() {
                tokio::select! {
                    biased;
                    () = state.shutdown.intake_stopped() => return,
                    () = tokio::time::sleep(delay) => {}
                }
            }
            let Some(_claim) = state.shutdown.background() else {
                return;
            };
            if let Ok(b) = binding::load(&state, &s.binding_id).await {
                let _ = coordinator::close(&state, &b, &s, false).await;
            }
        }
    });
}
pub(super) fn timestamp_seconds(start: DateTime<Utc>, end: DateTime<Utc>) -> AppResult<i64> {
    let nanos = (end - start)
        .num_nanoseconds()
        .filter(|n| *n >= 0)
        .ok_or_else(unavailable)?;
    nanos
        .checked_add(999_999_999)
        .map(|n| n / 1_000_000_000)
        .filter(|n| *n <= 86400)
        .ok_or_else(unavailable)
}
/// Only trusted managed ingress calls this after checking all three Telnyx IDs.
pub(super) async fn observe(
    state: &AppState,
    s: &CallSession,
    id: &EventIdentity,
    event: &super::super::protocol::Ingress,
) -> AppResult<()> {
    let r = receipt(state, &s.id).await?;
    if r.admitted.provider_call_id.as_deref() != Some(&s.provider_call_id) {
        return Err(unavailable());
    }
    if r.admitted.provider == "telnyx"
        && (id.call_leg_id != r.admitted.call_leg_id
            || id.call_session_id != r.admitted.call_session_id)
    {
        return Err(unavailable());
    }
    match event {
        super::super::protocol::Ingress::CallAnswered { .. } => {
            state.db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc!{"_id":&r.id,"provider_answered_at":bson::Bson::Null},doc!{"$set":{"provider_answered_at":bson::DateTime::from_chrono(id.occurred_at),"provider_answered_at_exact":id.occurred_at.to_rfc3339()}}).await?;
        }
        super::super::protocol::Ingress::CallEnded { at, seconds, .. } => {
            let mut quantity = *seconds;
            if r.admitted.provider == "telnyx" {
                quantity = None;
                if r.underwriting_snapshot.as_ref().is_some_and(|p| {
                    p.timestamp_fixture_reference
                        .as_ref()
                        .is_some_and(|s| !s.is_empty())
                }) {
                    let raw = state
                        .db
                        .collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
                        .find_one(doc! {"_id":&r.id})
                        .await?
                        .ok_or_else(unavailable)?;
                    // BSON dates retain milliseconds only. Preserve the signed
                    // timestamp's full precision before ceiling elapsed seconds.
                    let start = raw
                        .get_str("provider_answered_at_exact")
                        .ok()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|s| s.with_timezone(&Utc));
                    if let (Some(start), Some(end)) = (start, at) {
                        quantity = Some(timestamp_seconds(start, *end)?);
                    }
                }
            }
            let frozen = session::terminal(
                state,
                s,
                at.unwrap_or(id.occurred_at),
                quantity,
                "trusted_managed_provider_terminal",
            )
            .await?;
            if quantity.is_none() {
                let _ = refresh_evidence(state, &frozen).await;
            } else {
                finalize(state, &frozen).await?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(in crate::services::call_channels) fn detail_quantity(
    r: &AdmittedReference,
    v: &serde_json::Value,
) -> AppResult<Option<(DateTime<Utc>, i64)>> {
    let rows = v["data"].as_array().ok_or_else(unavailable)?;
    let matched: Vec<_> = rows
        .iter()
        .filter(|v| {
            v["record_type"] == "call-control"
                && v["call_leg_id"].as_str() == r.call_leg_id.as_deref()
                && v["connection_id"] == r.application_id
        })
        .collect();
    if matched.len() != 1 {
        return Ok(None);
    }
    let parse = |s: &serde_json::Value| {
        s.as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|s| s.with_timezone(&Utc))
    };
    let v = matched[0];
    let (Some(start), Some(end)) = (parse(&v["answered_at"]), parse(&v["finished_at"])) else {
        return Ok(None);
    };
    Ok(Some((end, timestamp_seconds(start, end)?)))
}
pub async fn refresh_evidence(state: &AppState, s: &CallSession) -> AppResult<()> {
    let b = binding::load(state, &s.binding_id).await?;
    let r = receipt(state, &s.id).await?;
    if let Ok(Some((at, n))) = super::super::carrier::managed_terminal_status(state, &b, s).await {
        let saved =
            session::terminal(state, s, at, Some(n), "authenticated_provider_status").await?;
        return finalize(state, &saved).await;
    }
    if r.admitted.provider == "telnyx"
        && r.underwriting_snapshot
            .as_ref()
            .is_some_and(underwriting::detail_enabled)
    {
        let v = super::resolver::request_existing(
            state,
            &b,
            &r.admitted,
            super::resolver::CleanupOperation::ReadDetailRecord,
            Some(&profiles::binding_origin(&b)?),
        )
        .await?;
        if let Some((at, n)) = detail_quantity(&r.admitted, &v)? {
            let saved = session::terminal(
                state,
                s,
                at,
                Some(n),
                "authenticated_provider_detail_record",
            )
            .await?;
            return finalize(state, &saved).await;
        }
    }
    Err(conflict(
        "Provider call quantity pending; reservations retained",
    ))
}
async fn verify_meter(
    state: &AppState,
    m: &MeterAdmission,
    owner: &str,
    settled: bool,
    quantity: i64,
) -> AppResult<()> {
    if m.usage_row_ids.len() != 1
        || m.reserved_quantity != 1800
        || m.rate.metric != crate::models::service_billing::BillingMetric::VoiceSeconds
        || m.rate.rate <= Credits::ZERO
    {
        return Err(unavailable());
    }
    let rows: Vec<UsageMeterRow> = state
        .db
        .collection::<UsageMeterRow>(USAGE)
        .find(doc! {"billing_request_id":&m.billing_request_id})
        .await?
        .try_collect()
        .await?;
    if rows.len() != m.usage_row_ids.len()
        || rows.iter().any(|r| {
            !m.usage_row_ids.contains(&r.id)
                || r.billing_owner_id != owner
                || r.service_id.as_deref() != Some(&m.rate.service_id)
                || r.actor_user_id != owner
                || r.service_slug.as_deref() != Some(&m.rate.service_slug)
                || r.lago_metric_code != m.rate.lago_metric_code
                || r.metric != m.rate.metric
                || !r.forwarded
                || r.funding.as_ref().and_then(|f| f.settlement_rate) != Some(m.rate.rate)
                || (settled
                    && (!r.released
                        || r.quantity != Some(quantity)
                        || r.funding.as_ref().is_none_or(|f| !f.settled)))
        })
    {
        return Err(conflict(
            "Managed call usage evidence incomplete; recovery required",
        ));
    }
    Ok(())
}
pub async fn finalize(state: &AppState, s: &CallSession) -> AppResult<()> {
    if s.ended_at.is_some() {
        super::synthesis::release_reserve(&state.db, &s.id).await?;
    }
    let r = receipt(state, &s.id).await?;
    if r.financially_finalized {
        return Ok(());
    }
    let Some(provider) = s.provider_seconds else {
        return Err(conflict(
            "Provider call quantity pending; reservations retained",
        ));
    };
    if s.ended_at.is_none() {
        return Err(unavailable());
    }
    if !(0..=86400).contains(&provider)
        || r.session_id != s.id
        || s.managed_claim_id.as_deref() != Some(&r.managed_claim_id)
        || s.managed_claim_generation != Some(r.managed_claim_generation)
        || r.admitted.provider_call_id.as_deref() != Some(&s.provider_call_id)
        || r.base.billing_request_id != format!("managed-call:{}:base", s.id)
        || r.premium
            .as_ref()
            .is_some_and(|m| m.billing_request_id != format!("managed-call:{}:premium", s.id))
    {
        return Err(unavailable());
    }
    let quantity = provider.min(1800);
    let fallback = state
        .db
        .collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
        .find_one(doc! {"_id":&r.id,"premium_fallback":"proven_pre_dispatch"})
        .await?
        .is_some();
    // Verify every expected row before either settlement; retry resumes a settled prefix.
    for m in std::iter::once(&r.base).chain(r.premium.iter()) {
        let expected = if fallback
            && r.premium
                .as_ref()
                .is_some_and(|p| p.billing_request_id == m.billing_request_id)
        {
            0
        } else {
            quantity
        };
        verify_meter(state, m, &s.user_id, false, expected).await?;
    }
    for m in std::iter::once(&r.base).chain(r.premium.iter()) {
        let quantity = if fallback
            && r.premium
                .as_ref()
                .is_some_and(|p| p.billing_request_id == m.billing_request_id)
        {
            0
        } else {
            quantity
        };
        super::super::billing::settle_admitted(
            state,
            &m.billing_request_id,
            &s.user_id,
            quantity,
            1,
        )
        .await?;
        verify_meter(state, m, &s.user_id, true, quantity).await?;
    }
    let mut retail = Credits::ZERO;
    for m in std::iter::once(&r.base).chain(r.premium.iter()) {
        if !fallback
            || r.premium
                .as_ref()
                .is_none_or(|p| p.billing_request_id != m.billing_request_id)
        {
            retail = retail.checked_add(m.rate.rate.checked_mul(quantity)?)?;
        }
    }
    let loss = if let Some(p) = &r.underwriting_snapshot {
        underwriting::cost(p, provider - quantity)?
    } else {
        Credits::ZERO
    };
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db, &(&db, &r, retail, loss), |tx, c| {
        Box::pin(async move {
            let (db, r, retail, loss) = *c;
            let now = Utc::now();
            if let Some(id) = &r.spend_hold_id {
                budgets::recognize(db, tx, id, retail, now).await?;
            }
            if let Some(id) = &r.underwriting_hold_id {
                budgets::recognize(db, tx, id, loss, now).await?;
            }
            db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&r.id},
                    doc! {"$set":{"financially_finalized":true,"slot_held":false}},
                )
                .session(&mut *tx)
                .await?;
            db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
                .update_one(
                    doc! {"effect_id":&r.session_id,"kind":"call"},
                    doc! {"$set":{"financially_finalized":true}},
                )
                .session(&mut *tx)
                .await?;
            db.collection::<CallSession>(SESSIONS)
                .update_one(
                    doc! {"_id":&r.session_id},
                    doc! {"$set":{"billing_finalized":true}},
                )
                .session(&mut *tx)
                .await?;
            Ok(())
        })
    })
    .await?;
    if r.underwriting_snapshot
        .as_ref()
        .is_some_and(|p| super::custody::worst_case_exposure(p).is_ok_and(|held| loss > held))
    {
        tracing::error!(session_id=%s.id,"Managed call loss exceeded underwriting hold");
    }
    Ok(())
}
/// Missing-evidence resolution is human-only, exact-effect, immutable and chained-audited.
pub async fn resolve_evidence(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    seconds: i64,
    reference: &str,
) -> AppResult<()> {
    if actor.api_key_id.is_some() {
        return Err(crate::errors::AppError::Forbidden(
            "Human platform Admin evidence resolution required".into(),
        ));
    }
    super::eligibility::require_existing_effect_admin(&state.db, actor).await?;
    if !(0..=86400).contains(&seconds)
        || reference.trim().is_empty()
        || reference.len() > 4096
        || reference.chars().any(char::is_control)
    {
        return Err(unavailable());
    }
    let s = session::load(state, id).await?;
    let managed = s.managed_claim_id.is_some();
    let r = if managed {
        Some(receipt(state, id).await?)
    } else {
        None
    };
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,actor,id,&r,seconds,reference,state.audit_chain_hmac_key.as_ref().as_ref()),|tx,c|Box::pin(async move{
        let (db,actor,id,r,n,reference,key)=*c;
        super::eligibility::admin_fence(db,tx,actor).await?;
        let saved=db.collection::<CallSession>(SESSIONS).find_one(doc!{"_id":id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if !profiles::relay(saved.profile) {return Err(unavailable());}
        if saved.ended_at.is_none(){return Err(conflict("Termination evidence is required before quantity resolution"));}
        let mut previous = saved.admin_evidence_reference.clone();
        if let Some(r)=r {
            let evidence=db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).find_one(doc! {"_id":&r.id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
            previous=previous.or_else(||evidence.get_str("admin_evidence_reference").ok().map(str::to_owned));
        } else {
            let b=db.collection::<Binding>(BINDINGS).find_one(doc! {"_id":&saved.binding_id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
            if saved.managed_claim_id.is_some() || !matches!(b.credential,CredentialBinding::Connection{..}) {return Err(unavailable());}
        }
        if let Some(previous)=previous {
            return if previous==reference && saved.provider_seconds==Some(n) {Ok(())}
                else {Err(conflict("Admin evidence resolution is already frozen"))};
        }
        if saved.provider_seconds.is_some() || saved.billing_finalized || saved.terminal_evidence.as_deref()==Some("provider_evidence_unavailable") {return Err(conflict("Call quantity or fee waiver is already frozen"));}
        if let Some(r)=r {
            db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc!{"_id":&r.id},doc!{"$set":{"admin_evidence_reference":reference}}).session(&mut *tx).await?;
        }
        db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":id},doc!{"$set":{"admin_evidence_reference":reference,"provider_seconds":n,"billable_seconds":n.min(1800),"absorbed_seconds":(n-1800).max(0),"observed_seconds":n.min(1800),"terminated_quantity_pending":false,"terminal_evidence":"admin_provider_evidence"}}).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,if r.is_some(){"managed_call_evidence_resolved"}else{"channel_adapter_call_evidence_resolved"},serde_json::json!({"session_id":id,"provider_seconds":n})).await?;Ok(())
    })).await?;
    coordinator::finalize_call(state, &session::load(state, id).await?).await
}

pub async fn sweep(state: &AppState) -> AppResult<()> {
    let now = bson::DateTime::now();
    let rows:Vec<ManagedCallAdmission>=state.db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME).find(doc!{"financially_finalized":false,"$or":[{"hangup_deadline":{"$lte":now}},{"answer_dispatch_anchor":bson::Bson::Null,"created_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(30))}}]}).sort(doc!{"recovery_checked_at":1,"_id":1}).limit(100).await?.try_collect().await?;
    for r in rows {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        state
            .db
            .collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&r.id},
                doc! {"$set":{"recovery_checked_at":bson::DateTime::now()}},
            )
            .await?;
        if let Ok(s) = session::load(state, &r.session_id).await
            && let Ok(b) = binding::load(state, &s.binding_id).await
        {
            let _ = coordinator::close(state, &b, &s, false).await;
        }
    }
    Ok(())
}
