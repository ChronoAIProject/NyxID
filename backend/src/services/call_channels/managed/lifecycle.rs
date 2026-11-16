//! Rental lifecycle. The request outcome, funding, coverage and authority always
//! share one majority transaction. External proof/freshness stays outside it.
use super::super::owner_admission;
use super::{budgets, conflict, custody, eligibility, resolver, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_adapter::*,
        credits::Credits,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        managed_comms::*,
        service_billing::BillingMetric,
    },
    services::{
        audit_service::{self, AuditActor},
        billing::managed_lifecycle::RentalFunding,
        transaction_service,
    },
};
use chrono::{DateTime, Datelike, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
mod sweep;
#[cfg(test)]
pub(super) use sweep::sweep_with_origin;
pub use sweep::{spawn_sweep, sweep_once};
mod admin;
pub use admin::{LineOperation, LineRevisionInput, RetireInput, resume, retire, suspend};
mod erasure;
pub(crate) use erasure::erasure_guard;
pub use erasure::{erase_personal_fields, prepare_erasure};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    pub request_id: String,
    pub quote_id: String,
    pub line_id: String,
    pub agent_id: String,
    pub voice_tier: VoiceTier,
    #[serde(default)]
    pub standard_voice: Option<StandardVoiceOption>,
    pub enable_sms: bool,
    pub caller_confirmations: bool,
    #[serde(default)]
    pub auto_renew: bool,
    pub accepted_terms_versions: TermsVersions,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinueInput {
    pub request_id: String,
    pub quote_id: String,
    #[serde(default)]
    pub auto_renew: bool,
    pub accepted_terms_versions: TermsVersions,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseInput {
    pub request_id: String,
    pub line_id: String,
    pub claim_id: String,
}
fn uuid(value: &str) -> AppResult<()> {
    if !Uuid::parse_str(value).is_ok_and(|u| u.get_version_num() == 4) {
        return Err(AppError::ValidationError("Expected UUID v4".into()));
    }
    Ok(())
}
fn digest<T: Serialize>(value: &T) -> AppResult<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value).map_err(|_| unavailable())?,
    )))
}
async fn replay(
    db: &Database,
    actor: &str,
    request: &str,
    digest: &str,
) -> AppResult<Option<ManagedLifecycleRequest>> {
    let row = db
        .collection::<ManagedLifecycleRequest>(ManagedLifecycleRequest::COLLECTION_NAME)
        .find_one(doc! {"actor_id":actor,"request_id":request})
        .await?;
    if row.as_ref().is_some_and(|r| r.payload_digest != digest) {
        return Err(conflict(
            "Managed lifecycle request ID has a different payload",
        ));
    }
    Ok(row)
}
async fn replay_in_session(
    db: &Database,
    tx: &mut ClientSession,
    actor: &str,
    request: &str,
    digest: &str,
) -> AppResult<Option<ManagedLifecycleRequest>> {
    let row = db
        .collection::<ManagedLifecycleRequest>(ManagedLifecycleRequest::COLLECTION_NAME)
        .find_one(doc! {"actor_id":actor,"request_id":request})
        .session(tx)
        .await?;
    if row.as_ref().is_some_and(|r| r.payload_digest != digest) {
        return Err(conflict(
            "Managed lifecycle request ID has a different payload",
        ));
    }
    Ok(row)
}
async fn eligibility_for(db: &Database, owner: &str) -> AppResult<ManagedCommsEligibility> {
    super::require_enabled(db, owner).await?;
    owner_admission::check(db, owner).await?;
    if db
        .collection::<ManagedLineErasure>(ManagedLineErasure::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner})
        .await?
        .is_some()
    {
        return Err(AppError::ErasurePreparing);
    }
    let e = eligibility::load(db, owner).await?;
    if e.revoked_at.is_some() || e.accepted_terms.as_ref() != Some(&e.required_terms) {
        return Err(unavailable());
    }
    Ok(e)
}
async fn owner_fence(db: &Database, tx: &mut ClientSession, owner: &str) -> AppResult<()> {
    owner_admission::fence(db, tx, owner).await?;
    if db
        .collection::<ManagedLineErasure>(ManagedLineErasure::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner})
        .session(tx)
        .await?
        .is_some()
    {
        return Err(AppError::ErasurePreparing);
    }
    Ok(())
}
async fn line(db: &Database, id: &str) -> AppResult<ManagedPhoneLine> {
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)
}
pub async fn latest_period(db: &Database, claim: &str) -> AppResult<ManagedLinePeriod> {
    db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .find_one(doc! {"claim_id":claim,"funded":true})
        .sort(doc! {"period_index":-1})
        .await?
        .ok_or_else(unavailable)
}
pub async fn quote(
    state: &AppState,
    owner: &str,
    line_id: &str,
    kind: QuoteKind,
) -> AppResult<ManagedLineQuote> {
    super::require_ready(&state.db).await?;
    throttle(&state.db, owner).await?;
    quote_internal(state, owner, line_id, kind, false).await
}
async fn quote_internal(
    state: &AppState,
    owner: &str,
    line_id: &str,
    kind: QuoteKind,
    automatic: bool,
) -> AppResult<ManagedLineQuote> {
    super::require_ready(&state.db).await?;
    let e = eligibility_for(&state.db, owner).await?;
    let line = line(&state.db, line_id).await?;
    let mut accept_before = Utc::now() + Duration::minutes(15);
    if matches!(kind, QuoteKind::Claim) && line.state != LineState::Available {
        return Err(unavailable());
    }
    if let QuoteKind::Renewal {
        ref claim_id,
        generation,
        period_index,
    } = kind
    {
        let claim = owned_claim(&state.db, owner, claim_id).await?;
        let period = latest_period(&state.db, claim_id).await?;
        if automatic {
            accept_before = period.covered_until;
        }
        if line.current_claim_id.as_deref() != Some(claim_id)
            || claim.managed_claim_generation != generation
            || period.period_index + 1 != period_index
            || claim.state != LineState::Claimed
            || period.covered_until <= Utc::now()
        {
            return Err(unavailable());
        }
    }
    if let QuoteKind::Resume {
        ref claim_id,
        generation,
    } = kind
    {
        let c = owned_claim(&state.db, owner, claim_id).await?;
        if line.current_claim_id.as_deref() != Some(claim_id)
            || c.managed_claim_generation != generation
            || c.suspension != Some(Suspension::Unpaid)
        {
            return Err(unavailable());
        }
    }
    let rental_rate = state
        .billing
        .managed_price(
            &line.rental_billing_service_id,
            ManagedCommsRole::Rental,
            BillingMetric::PhoneNumberDays,
        )
        .await?;
    let mut usage_rates = vec![];
    for (cap, role, metric) in [
        (
            &line.voice,
            ManagedCommsRole::Voice,
            BillingMetric::VoiceSeconds,
        ),
        (&line.sms, ManagedCommsRole::Sms, BillingMetric::Requests),
    ] {
        if let Some(cap) = cap {
            usage_rates.push(
                state
                    .billing
                    .managed_price(&cap.billing_service_id, role, metric)
                    .await?,
            );
        }
    }
    let now = Utc::now();
    let row = ManagedLineQuote {
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        standard_voice_options: {
            let mut options = super::voice::options(&line);
            // Premium is a voice surcharge; SMS-only lines never quote it.
            if line.provider == "telnyx"
                && line.voice.is_some()
                && e.premium_allowed
                && super::synthesis::enabled(&state.db).await?
                && let Some(c) =
                    super::synthesis::config(&state.db, &line.trusted_account_id).await?
                && let Some(p) = c.premium
                && e.required_terms.elevenlabs_eua.as_deref() == Some(&p.oem_terms_version)
            {
                usage_rates.push(
                    state
                        .billing
                        .managed_price(
                            &p.billing_service_id,
                            ManagedCommsRole::Premium,
                            BillingMetric::VoiceSeconds,
                        )
                        .await?,
                );
                options.extend(p.voices);
            }
            options
        },
        line_id: line.id,
        kind,
        rental_rate,
        terms: e.required_terms,
        usage_rates,
        accept_before,
        expires_at: Some(accept_before + Duration::days(1)),
        accepted_at: None,
        cancelled_at: None,
        created_at: now,
        line_revision: line.revision,
        eligibility_revision: e.revision,
        automatic_request_id: automatic.then(|| Uuid::new_v4().to_string()),
    };
    // The quote is immutable price/terms evidence. Acceptance is recorded only
    // with funding, never by trusting client-submitted monetary values.
    if let Err(error) = state
        .db
        .collection::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME)
        .insert_one(&row)
        .await
    {
        if automatic && super::super::binding::duplicate(&error) {
            return state.db.collection::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME)
                .find_one(doc!{"user_id":owner,"line_id":line_id,"kind":bson::to_bson(&row.kind).map_err(|_|unavailable())?,"automatic_request_id":{"$type":"string"}}).await?.ok_or_else(unavailable);
        }
        return Err(error.into());
    }
    Ok(row)
}
async fn owned_claim(db: &Database, owner: &str, id: &str) -> AppResult<ManagedLineClaim> {
    db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"user_id":owner})
        .await?
        .ok_or_else(unavailable)
}
async fn load_quote(db: &Database, owner: &str, id: &str) -> AppResult<ManagedLineQuote> {
    db.collection::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"user_id":owner})
        .await?
        .ok_or_else(unavailable)
}
async fn proof(
    state: &AppState,
    line: &ManagedPhoneLine,
    fixture: Option<&str>,
) -> AppResult<(ProviderAccount, DownstreamService)> {
    super::require_ready(&state.db).await?;
    custody::verify(&state.db, line).await?;
    let account = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&line.trusted_account_id})
        .await?
        .ok_or_else(unavailable)?;
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&account.catalog_service_id,"is_active":true})
        .await?
        .ok_or_else(unavailable)?;
    resolver::fixed_service(&service, &line.provider)?;
    let key =
        crate::services::catalog_credential_record_service::text(&state.encryption_keys, &service)
            .await?;
    let limit = custody::prove(state, line, &account, &service, &key, fixture).await?;
    if limit != line.provider_channel_limit {
        return Err(conflict(
            "Provider channel limit changed; operator revalidation required",
        ));
    }
    Ok((account, service))
}
async fn authority(
    db: &Database,
    tx: &mut ClientSession,
    e: &ManagedCommsEligibility,
    l: &ManagedPhoneLine,
    a: &ProviderAccount,
    s: &DownstreamService,
) -> AppResult<()> {
    super::require_enabled(db, &e.user_id).await?;
    owner_fence(db, tx, &e.user_id).await?;
    eligibility::fence(db, tx, e).await?;
    for (collection, filter) in [
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"_id":&l.id,"fence":l.fence,"revision":l.revision,"state":bson::to_bson(&l.state).map_err(|_|unavailable())?,"current_claim_id":bson::to_bson(&l.current_claim_id).map_err(|_|unavailable())?},
        ),
        (
            ACCOUNTS,
            doc! {"_id":&a.id,"revision":a.revision,"fence":custody::account_fence_value(a.fence),"catalog_service_id":&s.id},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .update_one(
                filter,
                doc! {"$inc":{super::admission::ADMISSION_TOUCH:1_i64}},
            )
            .session(&mut *tx)
            .await?
            .matched_count
            != 1
        {
            return Err(conflict("Managed lifecycle authority changed"));
        }
    }
    if db.collection::<Document>(SERVICES).update_one(doc!{"_id":&s.id,"is_active":true,"managed_comms_role":"credential","updated_at":bson::DateTime::from_chrono(s.updated_at)},doc!{"$inc":{"managed_comms_fence":1_i64}}).session(tx).await?.matched_count!=1{return Err(unavailable());}
    Ok(())
}
pub(super) async fn monthly_hold(
    db: &Database,
    tx: &mut ClientSession,
    e: &ManagedCommsEligibility,
    effect: &str,
    amount: Credits,
    now: DateTime<Utc>,
) -> AppResult<Option<String>> {
    let limit = e
        .monthly_spend_limit
        .unwrap_or(Credits::from_pico(crate::models::credits::MAX_PICO)?);
    let rows = db.collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME);
    let key = format!("managed-spend:{}", e.user_id);
    let window = BudgetWindow::CalendarMonth {
        year: now.year(),
        month: now.month(),
    };
    let mut budget = if let Some(old) = rows
        .find_one(doc! {"budget_key":&key})
        .session(&mut *tx)
        .await?
    {
        old
    } else {
        let b = ManagedBudget {
            id: Uuid::new_v4().to_string(),
            budget_key: key,
            window: window.clone(),
            limit,
            revision: 1,
            fence: 0,
        };
        rows.insert_one(&b).session(&mut *tx).await?;
        b
    };
    if budget.window != window || budget.limit != limit {
        budget.window = window;
        budget.limit = limit;
        budget.revision += 1;
        rows.replace_one(doc! {"_id":&budget.id}, &budget)
            .session(&mut *tx)
            .await?;
    }
    let hold = budgets::hold(db, tx, &budget, effect, amount, now).await?;
    if effect.starts_with("managed-line-rental:") {
        budgets::recognize(db, tx, &hold.id, amount, now).await?;
    }
    Ok(Some(hold.id))
}
#[allow(clippy::too_many_arguments)]
async fn period(
    db: &Database,
    tx: &mut ClientSession,
    funding: &RentalFunding,
    e: &ManagedCommsEligibility,
    claim: &ManagedLineClaim,
    index: i64,
    from: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<ManagedLinePeriod> {
    let request = format!("managed-line-rental:{}:{index}", claim.id);
    let row_id = funding
        .reserve(db, tx, &claim.user_id, &request, now)
        .await?;
    let spend_hold_id =
        monthly_hold(db, tx, e, &request, funding.rate.rate.checked_mul(30)?, now).await?;
    let p = ManagedLinePeriod {
        id: Uuid::new_v4().to_string(),
        claim_id: claim.id.clone(),
        managed_claim_generation: claim.managed_claim_generation,
        period_index: index,
        covered_from: from,
        covered_until: from + Duration::days(30),
        quantity: 30,
        rate: funding.rate.clone(),
        billing_request_id: request,
        usage_row_ids: vec![row_id],
        funded: true,
        spend_hold_id,
        created_at: now,
    };
    db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .insert_one(&p)
        .session(tx)
        .await?;
    Ok(p)
}
fn binding(
    line: &ManagedPhoneLine,
    claim: &ManagedLineClaim,
    account: &ProviderAccount,
    service: &DownstreamService,
    sms: bool,
    now: DateTime<Utc>,
) -> AppResult<Binding> {
    let cap = if sms {
        line.sms.as_ref()
    } else {
        line.voice.as_ref()
    }
    .ok_or_else(unavailable)?;
    let profile = match (line.provider.as_str(), sms) {
        ("twilio", false) => Profile::TwilioRelay,
        ("twilio", true) => Profile::TwilioSms,
        ("telnyx", false) => Profile::TelnyxRelay,
        ("telnyx", true) => Profile::TelnyxSms,
        _ => return Err(unavailable()),
    };
    Ok(Binding {
        id: Uuid::new_v4().to_string(),
        user_id: claim.user_id.clone(),
        profile,
        contract_version: 1,
        revision: 1,
        label: "NyxID phone line".into(),
        agent_id: claim.agent_id.clone(),
        credential: CredentialBinding::ManagedLine {
            line_id: line.id.clone(),
            managed_claim_id: claim.id.clone(),
            managed_claim_generation: claim.managed_claim_generation,
        },
        account_id: account.account_id.clone(),
        resource_id: line.number_resource_id.clone(),
        phone_number: line.phone_number.clone(),
        application_id: cap.application_id.clone(),
        webhook_public_key: account.webhook_public_key.clone(),
        voice: claim
            .standard_voice
            .clone()
            .unwrap_or_else(|| super::voice::default_option(&line.provider))
            .voice,
        language: claim
            .standard_voice
            .clone()
            .unwrap_or_else(|| super::voice::default_option(&line.provider))
            .language,
        enabled: true,
        verified_credential_identity: super::super::session::identity(&[
            "managed-canonical",
            &account.id,
            &service.id,
        ]),
        trusted_account_revision: Some(account.revision),
        verified_at: now,
        created_at: now,
        updated_at: now,
    })
}
async fn throttle(db: &Database, owner: &str) -> AppResult<()> {
    let admission = crate::services::coordination_service::TokenBucketStore::admit_over_period(
        db,
        "managed-line-lifecycle",
        owner,
        10,
        std::time::Duration::from_secs(60),
        10,
    )
    .await?;
    if !admission.allowed {
        return Err(AppError::RateLimited);
    }
    Ok(())
}
async fn finish(state: &AppState, receipt: &ManagedLifecycleRequest) {
    if let Some(id) = &receipt.period_id
        && let Ok(Some(p)) = state
            .db
            .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await
    {
        for row in p.usage_row_ids {
            if state.billing.settle_managed_rental(&row).await.is_err() {
                tracing::warn!(period_id=%id,"Managed rental settlement deferred to billing recovery");
            }
        }
    }
}
pub async fn claim(
    state: &AppState,
    actor: &AuditActor,
    input: ClaimInput,
) -> AppResult<ManagedLifecycleRequest> {
    claim_with_origin(state, actor, input, None).await
}
pub(super) async fn claim_with_origin(
    state: &AppState,
    actor: &AuditActor,
    input: ClaimInput,
    fixture: Option<&str>,
) -> AppResult<ManagedLifecycleRequest> {
    for id in [
        &input.request_id,
        &input.quote_id,
        &input.line_id,
        &input.agent_id,
    ] {
        uuid(id)?;
    }
    let payload = digest(&("claim", &input))?;
    let db = transaction_service::durable_database(&state.db);
    if let Some(r) = replay(&db, &actor.user_id, &input.request_id, &payload).await? {
        finish(state, &r).await;
        return Ok(r);
    }
    super::require_ready(&db).await?;
    throttle(&db, &actor.user_id).await?;
    let e = eligibility_for(&db, &actor.user_id).await?;
    let l = line(&db, &input.line_id).await?;
    let q = load_quote(&db, &actor.user_id, &input.quote_id).await?;
    if q.kind != QuoteKind::Claim
        || q.line_id != l.id
        || l.state != LineState::Available
        || l.available_at.is_some_and(|at| at > Utc::now())
        || q.terms != input.accepted_terms_versions
        || q.terms != e.required_terms
        || q.line_revision != l.revision
        || q.eligibility_revision != e.revision
        || (input.voice_tier == VoiceTier::Premium && !e.premium_allowed)
        || (input.enable_sms
            && (l.sms.is_none()
                || e.sms_registration_status != "approved"
                || l.regulatory
                    .sms_sender_registration
                    .as_ref()
                    .is_none_or(String::is_empty)))
        || (input.caller_confirmations && !input.enable_sms)
    {
        return Err(unavailable());
    }
    let selection = input
        .standard_voice
        .clone()
        .unwrap_or_else(|| super::voice::default_option(&l.provider));
    if input.voice_tier == VoiceTier::Premium {
        if l.provider != "telnyx" || l.voice.is_none() {
            return Err(unavailable());
        }
        super::synthesis::validate_selection(
            &db,
            &l.trusted_account_id,
            &e,
            &input.accepted_terms_versions,
            &selection,
        )
        .await?;
        if !q.standard_voice_options.contains(&selection) {
            return Err(unavailable());
        }
    } else {
        super::voice::validate_option(&l.provider, &selection)?;
    }
    if input.voice_tier == VoiceTier::Standard
        && l.voice.is_some()
        && (!super::voice::options(&l).contains(&selection)
            || !q.standard_voice_options.contains(&selection))
    {
        return Err(conflict(
            "Voice option is not allowed by this quote and line",
        ));
    }
    if l.voice.is_none() && input.standard_voice.is_some() {
        return Err(unavailable());
    }

    let (a, s) = proof(state, &l, fixture).await?;
    let funding = state
        .billing
        .prepare_managed_rental(&actor.user_id, &q.rental_rate)
        .await?;
    let ctx = (
        &db,
        actor,
        &input,
        &payload,
        &e,
        &l,
        &q,
        &a,
        &s,
        &funding,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    let result=transaction_service::run_db_only(&db,&ctx,|tx,c|Box::pin(async move{
        let &(db,actor,input,payload,e,l,q,a,s,funding,key)=c;
        if let Some(r)=replay_in_session(db,tx,&actor.user_id,&input.request_id,payload).await?{return Ok(r);}
        let now=Utc::now();
        if q.accept_before<=now || q.accepted_at.is_some() || q.cancelled_at.is_some(){return Err(conflict("Managed quote expired or already accepted"));}
        authority(db,tx,e,l,a,s).await?;
        if db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).count_documents(doc!{"user_id":&e.user_id,"state":{"$in":["claimed","suspended","releasing"]}}).session(&mut *tx).await? >= e.max_lines as u64{return Err(conflict("Managed line limit reached"));}
        // The owner's specialist must be live and personal, never an org route.
        if db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc!{"_id":&input.agent_id,"user_id":&actor.user_id,"kind":"specialist","destroyed_at":bson::Bson::Null},doc!{"$inc":{"managed_line_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Choose an active specialist owned by this account"));}
        let mut claim=ManagedLineClaim{id:Uuid::new_v4().to_string(),line_id:l.id.clone(),user_id:actor.user_id.clone(),managed_claim_generation:l.next_claim_generation,
            eligibility_revision:e.revision,accepted_terms:q.terms.clone(),agent_id:input.agent_id.clone(),voice_tier:input.voice_tier,standard_voice:l.voice.as_ref().map(|_|input.standard_voice.clone().unwrap_or_else(||super::voice::default_option(&l.provider))),sms_enabled:input.enable_sms,caller_confirmations:input.caller_confirmations,
            auto_renew:input.auto_renew,accepted_rental_rate:q.rental_rate.clone(),state:LineState::Claimed,suspension:None,voice_binding_id:None,sms_binding_id:None,fence:1,covered_from:now,created_at:now,release_requested_at:None,suspended_at:None};
        for sms in [false,true] {if (sms&&!input.enable_sms)||(!sms&&l.voice.is_none()){continue;}let b=binding(l,&claim,a,s,sms,now)?;
            if sms{claim.sms_binding_id=Some(b.id.clone());}else{claim.voice_binding_id=Some(b.id.clone());}
            db.collection::<Binding>(BINDINGS).insert_one(b).session(&mut *tx).await?;
        }
        let p=period(db,tx,funding,e,&claim,0,now,now).await?;
        db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).insert_one(&claim).session(&mut *tx).await?;
        db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&l.id},doc!{"$set":{"state":"claimed","suspension":bson::Bson::Null,"current_claim_id":&claim.id,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"next_claim_generation":1_i64,"fence":1_i64}}).session(&mut *tx).await?;
        if db.collection::<Document>(ManagedLineQuote::COLLECTION_NAME).update_one(doc!{"_id":&q.id,"accepted_at":bson::Bson::Null,"cancelled_at":bson::Bson::Null},doc!{"$set":{"accepted_at":bson::DateTime::from_chrono(now)},"$unset":{"expires_at":""}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        let r=ManagedLifecycleRequest{id:Uuid::new_v4().to_string(),actor_id:actor.user_id.clone(),request_id:input.request_id.clone(),payload_digest:payload.clone(),kind:"claim".into(),line_id:l.id.clone(),claim_id:Some(claim.id),period_id:Some(p.id),created_at:now};
        db.collection::<ManagedLifecycleRequest>(ManagedLifecycleRequest::COLLECTION_NAME).insert_one(&r).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_line_claimed",serde_json::json!({"line_id":l.id,"claim_id":r.claim_id,"period_id":r.period_id})).await?;Ok(r)
    })).await;
    let receipt = match result {
        Ok(r) => r,
        Err(error) => match replay(&db, &actor.user_id, &input.request_id, &payload).await? {
            Some(r) => r,
            None => return Err(error),
        },
    };
    finish(state, &receipt).await;
    Ok(receipt)
}

/// A fresh renewal/resume quote is the complete price and terms consent.
pub async fn continue_coverage(
    state: &AppState,
    actor: &AuditActor,
    input: ContinueInput,
) -> AppResult<ManagedLifecycleRequest> {
    continue_with_origin(state, actor, input, None, false).await
}
pub(super) async fn continue_with_origin(
    state: &AppState,
    actor: &AuditActor,
    input: ContinueInput,
    fixture: Option<&str>,
    automatic: bool,
) -> AppResult<ManagedLifecycleRequest> {
    uuid(&input.request_id)?;
    uuid(&input.quote_id)?;
    let payload = digest(&("continue", &input))?;
    let db = transaction_service::durable_database(&state.db);
    if let Some(r) = replay(&db, &actor.user_id, &input.request_id, &payload).await? {
        finish(state, &r).await;
        return Ok(r);
    }
    super::require_ready(&db).await?;
    let q = load_quote(&db, &actor.user_id, &input.quote_id).await?;
    let (claim_id, generation, renewal) = match &q.kind {
        QuoteKind::Renewal {
            claim_id,
            generation,
            ..
        } => (claim_id, *generation, true),
        QuoteKind::Resume {
            claim_id,
            generation,
        } => (claim_id, *generation, false),
        _ => return Err(unavailable()),
    };
    let c = owned_claim(&db, &actor.user_id, claim_id).await?;
    let e = eligibility_for(&db, &actor.user_id).await?;
    let l = line(&db, &c.line_id).await?;
    let previous = latest_period(&db, &c.id).await?;
    if c.managed_claim_generation != generation
        || l.current_claim_id.as_deref() != Some(&c.id)
        || q.line_id != l.id
        // The single automatic renewal quote is bound to price, terms, claim
        // generation and period. Operational line edits must not strand an
        // accepted auto-renewal; manual quotes still bind the line revision.
        || (!automatic && q.line_revision != l.revision)
        || q.eligibility_revision != e.revision
        || c.eligibility_revision != e.revision
        || c.accepted_terms != e.required_terms
        || q.terms != e.required_terms
        || input.accepted_terms_versions != q.terms
        || (c.voice_tier == VoiceTier::Premium && !e.premium_allowed)
    {
        return Err(unavailable());
    }
    if renewal {
        if c.state != LineState::Claimed
            || l.state != LineState::Claimed
            || c.suspension.is_some()
            || l.suspension.is_some()
            || previous.covered_until <= Utc::now()
            || !matches!(q.kind,QuoteKind::Renewal{period_index,..} if period_index==previous.period_index+1)
        {
            return Err(unavailable());
        }
        if automatic && (!c.auto_renew || c.accepted_rental_rate != q.rental_rate) {
            return Err(conflict("Accept the changed renewal price before funding"));
        }
    } else if c.state != LineState::Suspended
        || l.state != LineState::Suspended
        || c.suspension != Some(Suspension::Unpaid)
        || l.suspension != Some(Suspension::Unpaid)
        || c.suspended_at
            .is_none_or(|at| at + Duration::days(7) <= Utc::now())
    {
        return Err(conflict("Only retained unpaid claims can be resumed"));
    }
    let (a, s) = proof(state, &l, fixture).await?;
    let funding = state
        .billing
        .prepare_managed_rental(&actor.user_id, &q.rental_rate)
        .await?;
    let context = (
        &db,
        actor,
        &input,
        &payload,
        &e,
        &l,
        &c,
        &previous,
        &q,
        &a,
        &s,
        &funding,
        renewal,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    let result=transaction_service::run_db_only(&db,&context,|tx,ctx|Box::pin(async move{
        let &(db,actor,input,payload,e,l,c,previous,q,a,s,funding,renewal,key)=ctx;
        if let Some(r)=replay_in_session(db,tx,&actor.user_id,&input.request_id,payload).await?{return Ok(r);}
        let now=Utc::now();
        if q.accept_before<=now || q.accepted_at.is_some() || q.cancelled_at.is_some() || (renewal && previous.covered_until<=now){return Err(conflict("Managed quote or renewal cutoff expired"));}
        authority(db,tx,e,l,a,s).await?;
        // A renewal keeps the claim and line claimed and changes no admission
        // authority (admission re-reads the period covering now), so it only
        // touches them; resume (suspended -> claimed) advances the authority version.
        let advance=if renewal{super::admission::ADMISSION_TOUCH}else{"fence"};
        let changed=db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"fence":c.fence,"state":bson::to_bson(&c.state).map_err(|_|unavailable())?,"managed_claim_generation":c.managed_claim_generation},doc!{"$inc":{advance:1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(conflict("Managed claim changed during rental admission"));}
        let latest=db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME).find_one(doc!{"claim_id":&c.id,"funded":true}).sort(doc!{"period_index":-1}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if latest.id!=previous.id{return Err(conflict("Managed rental period already advanced"));}
        let from=if renewal{previous.covered_until}else{now};
        let p=period(db,tx,funding,e,c,previous.period_index+1,from,now).await?;
        db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id},doc!{"$set":{"state":"claimed","suspension":bson::Bson::Null,"suspended_at":bson::Bson::Null,"auto_renew":input.auto_renew,"accepted_rental_rate":bson::to_bson(&q.rental_rate).map_err(|_|unavailable())?}}).session(&mut *tx).await?;
        db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&l.id},doc!{"$set":{"state":"claimed","suspension":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{advance:1_i64}}).session(&mut *tx).await?;
        if !renewal {
            db.collection::<Document>(BINDINGS).update_many(doc!{"credential.managed_claim_id":&c.id,"credential.managed_claim_generation":generation},doc!{"$set":{"enabled":true,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
        }
        if db.collection::<Document>(ManagedLineQuote::COLLECTION_NAME).update_one(doc!{"_id":&q.id,"accepted_at":bson::Bson::Null,"cancelled_at":bson::Bson::Null},doc!{"$set":{"accepted_at":bson::DateTime::from_chrono(now)},"$unset":{"expires_at":""}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        let r=ManagedLifecycleRequest{id:Uuid::new_v4().to_string(),actor_id:actor.user_id.clone(),request_id:input.request_id.clone(),payload_digest:payload.clone(),kind:if renewal{"renewal"}else{"resume"}.into(),line_id:l.id.clone(),claim_id:Some(c.id.clone()),period_id:Some(p.id),created_at:now};
        db.collection::<ManagedLifecycleRequest>(ManagedLifecycleRequest::COLLECTION_NAME).insert_one(&r).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,if renewal{"managed_line_renewed"}else{"managed_line_resumed"},serde_json::json!({"line_id":l.id,"claim_id":c.id,"period_id":r.period_id})).await?;
        if renewal {
            // Recheck with the database clock after all funding/audit writes.
            // A worker paused at the cutoff must abort its whole transaction.
            if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"$expr":{"$lt":["$$NOW",bson::DateTime::from_chrono(previous.covered_until)]}},doc!{"$inc":{super::admission::ADMISSION_TOUCH:1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Renewal cutoff reached before commit"));}
        }
        Ok(r)
    })).await;
    let r = match result {
        Ok(r) => r,
        Err(error) => match replay(&db, &actor.user_id, &input.request_id, &payload).await? {
            Some(r) => r,
            None => return Err(error),
        },
    };
    finish(state, &r).await;
    Ok(r)
}

/// Disable, expire socket leases, and fence generations in the same mutation.
/// Provider hangup is attempted only after the mutation commits.
async fn disable(
    db: &Database,
    tx: &mut ClientSession,
    claim: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<Document>(BINDINGS).update_many(doc!{"credential.managed_claim_id":claim,"enabled":true},doc!{"$set":{"enabled":false,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
    db.collection::<Document>(SESSIONS).update_many(doc!{"managed_claim_id":claim,"state":{"$ne":"closed"}},doc!{"$set":{"lease_expires_at":bson::DateTime::from_chrono(now)},"$inc":{"generation":1_i64}}).session(tx).await?;
    Ok(())
}
pub async fn release(
    state: &AppState,
    actor: &AuditActor,
    input: ReleaseInput,
    admin: bool,
) -> AppResult<ManagedLifecycleRequest> {
    uuid(&input.request_id)?;
    uuid(&input.line_id)?;
    uuid(&input.claim_id)?;
    let payload = digest(&("release", &input, admin))?;
    let db = transaction_service::durable_database(&state.db);
    if let Some(r) = replay(&db, &actor.user_id, &input.request_id, &payload).await? {
        drain(state, &input.claim_id).await?;
        return Ok(r);
    }
    throttle(&db, &actor.user_id).await?;
    let context = (
        &db,
        actor,
        &input,
        &payload,
        admin,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    let result = transaction_service::run_db_only(&db, &context, |tx, ctx| {
        Box::pin(async move {
            let &(db, actor, input, payload, admin, key) = ctx;
            if let Some(r) =
                replay_in_session(db, tx, &actor.user_id, &input.request_id, payload).await?
            {
                return Ok(r);
            }
            if admin {
                eligibility::admin_fence(db, tx, actor).await?;
            }
            let c = db
                .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
                .find_one(doc! {"_id":&input.claim_id,"line_id":&input.line_id})
                .session(&mut *tx)
                .await?
                .ok_or_else(unavailable)?;
            if !admin && c.user_id != actor.user_id {
                return Err(AppError::Forbidden(
                    "This claim belongs to another account".into(),
                ));
            }
            // Release is cleanup; flag-off, eligibility loss and expired coverage
            // cannot prevent the owner from stopping further paid periods.
            let now = Utc::now();
            release_in_session(db, tx, &c, now).await?;
            let r = ManagedLifecycleRequest {
                id: Uuid::new_v4().to_string(),
                actor_id: actor.user_id.clone(),
                request_id: input.request_id.clone(),
                payload_digest: payload.clone(),
                kind: "release".into(),
                line_id: input.line_id.clone(),
                claim_id: Some(c.id),
                period_id: None,
                created_at: now,
            };
            db.collection::<ManagedLifecycleRequest>(ManagedLifecycleRequest::COLLECTION_NAME)
                .insert_one(&r)
                .session(&mut *tx)
                .await?;
            audit_service::log_actor_event_in_session(
                db,
                tx,
                key,
                actor,
                "managed_line_release_requested",
                serde_json::json!({"line_id":r.line_id,"claim_id":r.claim_id}),
            )
            .await?;
            Ok(r)
        })
    })
    .await;
    let r = match result {
        Ok(r) => r,
        Err(error) => match replay(&db, &actor.user_id, &input.request_id, &payload).await? {
            Some(r) => r,
            None => return Err(error),
        },
    };
    drain(state, &input.claim_id).await?;
    Ok(r)
}
async fn release_in_session(
    db: &Database,
    tx: &mut ClientSession,
    c: &ManagedLineClaim,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if matches!(
        c.state,
        LineState::Releasing | LineState::Cooldown | LineState::PendingReassignment
    ) {
        return Ok(());
    }
    if !matches!(c.state, LineState::Claimed | LineState::Suspended) {
        return Err(unavailable());
    }
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&c.user_id},
            doc! {"$inc":{"channel_adapter_admission_fence":1_i64}},
        )
        .session(&mut *tx)
        .await?
        .matched_count
        != 1
    {
        return Err(unavailable());
    }
    if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"fence":c.fence},doc!{"$set":{"state":"releasing","auto_renew":false,"release_requested_at":bson::DateTime::from_chrono(now)},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
    if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&c.line_id,"current_claim_id":&c.id,"state":{"$in":["claimed","suspended"]}},doc!{"$set":{"state":"releasing","updated_at":bson::DateTime::from_chrono(now)},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
    disable(db, tx, &c.id, now).await?;
    // Quotes carry no holds. Cancelling an uncommitted renewal never modifies a
    // paid receipt; a concurrent funding transaction contends on claim/line.
    db.collection::<Document>(ManagedLineQuote::COLLECTION_NAME)
        .update_many(
            doc! {"kind.claim_id":&c.id,"accepted_at":bson::Bson::Null},
            doc! {"$set":{"cancelled_at":bson::DateTime::from_chrono(now)}},
        )
        .session(tx)
        .await?;
    Ok(())
}
async fn effects_accounted(db: &Database, tx: &mut ClientSession, claim: &str) -> AppResult<bool> {
    for (collection, filter) in [
        (
            ManagedEffectRoute::COLLECTION_NAME,
            doc! {"managed_claim_id":claim,"financially_finalized":{"$ne":true}},
        ),
        (
            ManagedCallAdmission::COLLECTION_NAME,
            doc! {"managed_claim_id":claim,"$or":[{"financially_finalized":{"$ne":true}},{"slot_held":true}]},
        ),
        (
            SESSIONS,
            doc! {"managed_claim_id":claim,"$or":[{"state":{"$ne":"closed"}},{"billing_finalized":{"$ne":true}}]},
        ),
        (
            ManagedInboundSms::COLLECTION_NAME,
            doc! {"outcomes.claim_id":claim,"financially_finalized":{"$ne":true}},
        ),
        (
            ManagedCallConfirmation::COLLECTION_NAME,
            doc! {"managed_claim_id":claim,"status":{"$in":["prepared","sending","uncertain"]},"terminal_evidence":{"$exists":false}},
        ),
        (
            EVENTS,
            doc! {"managed_claim_id":claim,"billing_finalized":{"$ne":true}},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(false);
        }
    }
    Ok(true)
}
async fn drain(state: &AppState, claim: &str) -> AppResult<()> {
    let rows: Vec<Binding> = state
        .db
        .collection::<Binding>(BINDINGS)
        .find(doc! {"credential.managed_claim_id":claim,"enabled":false})
        .await?
        .try_collect()
        .await?;
    for b in rows {
        super::super::management::close_disabled_binding(state, &b).await?;
    }
    complete_release(&state.db, claim, Utc::now()).await?;
    Ok(())
}
async fn complete_release(db: &Database, claim: &str, now: DateTime<Utc>) -> AppResult<bool> {
    transaction_service::run_db_only(db,&(db,claim,now),|tx,ctx|Box::pin(async move{
        let &(db,claim,now)=ctx;
        let Some(c)=db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find_one(doc!{"_id":claim,"state":"releasing"}).session(&mut *tx).await? else{return Ok(false);};
        if !effects_accounted(db,tx,claim).await?{return Ok(false);}
        if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&c.line_id,"current_claim_id":claim,"state":"releasing"},doc!{"$set":{"state":"cooldown","current_claim_id":bson::Bson::Null,"suspension":bson::Bson::Null,"available_at":bson::DateTime::from_chrono(now+Duration::days(30)),"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"fence":1_i64,"revision":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":claim,"fence":c.fence},doc!{"$set":{"state":"cooldown"},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        db.collection::<Document>(BINDINGS).delete_many(doc!{"credential.managed_claim_id":claim}).session(tx).await?;Ok(true)
    })).await
}

pub async fn approve_reassignment(
    state: &AppState,
    actor: &AuditActor,
    line_id: &str,
    revision: i64,
) -> AppResult<()> {
    super::require_ready(&state.db).await?;
    let ctx = (
        &state.db,
        actor,
        line_id,
        revision,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    transaction_service::run_db_only(&state.db,&ctx,|tx,c|Box::pin(async move{
        let &(db,actor,line_id,revision,key)=c;eligibility::admin_fence(db,tx,actor).await?;
        let now=Utc::now();
        if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":line_id,"revision":revision,"state":"pending_reassignment","current_claim_id":bson::Bson::Null,"available_at":{"$lte":bson::DateTime::from_chrono(now)}},doc!{"$set":{"state":"available","updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64,"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Line is not ready for reassignment approval"));}
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_line_reassignment_approved",serde_json::json!({"line_id":line_id,"revision":revision+1})).await?;Ok(())
    })).await
}
