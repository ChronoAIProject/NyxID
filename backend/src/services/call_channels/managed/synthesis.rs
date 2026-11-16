//! Durable synthesis debits precede every Telnyx text write. No text is retained.
use super::{conflict, custody, eligibility, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{channel_adapter::*, managed_comms::*},
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::{Datelike, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde::Deserialize;

pub const BLOCK: usize = 400;
pub const WRAPUP_RESERVE: i64 = 300;
pub const WRAPUP: &str = "We have reached the call limit. Thank you for calling. Goodbye.";
pub fn wrapup<'a>(language: &str, configured: Option<&'a str>) -> &'a str {
    configured.unwrap_or(match language {
        "zh-CN" => "本次通话已达到时限。感谢您的来电，再见。",
        "zh-TW" => "本次通話已達到時限。感謝您的來電，再見。",
        _ => WRAPUP,
    })
}
fn validate_texts(i: &VoiceAccountInput) -> AppResult<()> {
    for (text, limit) in [
        (&i.greeting, i.per_call_characters - WRAPUP_RESERVE),
        (&i.wrapup, WRAPUP_RESERVE),
    ] {
        if text.as_ref().is_some_and(|s| {
            s.trim().is_empty()
                || s.chars().count() as i64 > limit
                || s.chars().any(char::is_control)
        }) {
            return Err(AppError::ValidationError(
                "Configured speech exceeds its character allowance or contains control characters"
                    .into(),
            ));
        }
    }
    Ok(())
}
const SWITCH: &str = "a04d261f-9857-4ae0-8310-9d6c13dd9113";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceAccountInput {
    pub revision: i64,
    pub per_call_characters: i64,
    pub greeting: Option<String>,
    pub wrapup: Option<String>,
    pub premium: Option<PremiumInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PremiumInput {
    pub api_key_ref: String,
    pub voices: Vec<StandardVoiceOption>,
    pub billing_account_identity: String,
    pub monthly_characters: i64,
    pub plan_tier: String,
    pub oem_terms_version: String,
    pub admin_acknowledged: bool,
    pub billing_service_id: String,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:".contains(&b))
}
pub async fn config(db: &Database, account: &str) -> AppResult<Option<ManagedVoiceAccount>> {
    Ok(db
        .collection::<ManagedVoiceAccount>(ManagedVoiceAccount::COLLECTION_NAME)
        .find_one(doc! {"trusted_account_id":account})
        .await?)
}
pub async fn enabled(db: &Database) -> AppResult<bool> {
    Ok(db
        .collection::<ManagedVoiceSwitch>(ManagedVoiceSwitch::COLLECTION_NAME)
        .find_one(doc! {"_id":SWITCH})
        .await?
        .is_some_and(|s| s.premium_enabled))
}
pub async fn configure(
    state: &AppState,
    actor: &AuditActor,
    account: &str,
    i: VoiceAccountInput,
) -> AppResult<()> {
    configure_with_origin(state, actor, account, i, None).await
}
pub(super) async fn configure_with_origin(
    state: &AppState,
    actor: &AuditActor,
    account: &str,
    i: VoiceAccountInput,
    fixture: Option<&str>,
) -> AppResult<()> {
    super::require_ready(&state.db).await?;
    eligibility::require_admin(&state.db, actor).await?;
    if !(WRAPUP_RESERVE + 400..=1_000_000).contains(&i.per_call_characters) {
        return Err(unavailable());
    }
    validate_texts(&i)?;
    let proved_account = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":account,"provider":"telnyx"})
        .await?
        .ok_or_else(unavailable)?;
    let premium = if let Some(p) = i.premium {
        if !p.admin_acknowledged
            || !["Scale", "Business", "Enterprise"].contains(&p.plan_tier.as_str())
            || !identifier(&p.api_key_ref)
            || !identifier(&p.billing_account_identity)
            || !identifier(&p.oem_terms_version)
            || p.monthly_characters < i.per_call_characters
            || p.voices.is_empty()
            || p.voices.len() > 64
            || uuid::Uuid::parse_str(&p.billing_service_id).is_err()
        {
            return Err(unavailable());
        }
        for (index, v) in p.voices.iter().enumerate() {
            let parts: Vec<_> = v.voice.split('.').collect();
            if parts.len() != 3
                || parts[0] != "ElevenLabs"
                || !parts[1..].iter().all(|s| identifier(s))
                || !["en-US", "zh-CN", "zh-TW"].contains(&v.language.as_str())
                || p.voices[..index].contains(v)
            {
                return Err(unavailable());
            }
        }
        custody::prove_integration_reference(state, &proved_account, &p.api_key_ref, fixture)
            .await?;
        Some(PremiumVoiceConfig {
            api_key_ref: p.api_key_ref,
            voices: p.voices,
            billing_account_identity: p.billing_account_identity,
            monthly_characters: p.monthly_characters,
            plan_tier: p.plan_tier,
            oem_terms_version: p.oem_terms_version,
            acknowledged_by: actor.user_id.clone(),
            acknowledged_at: Utc::now(),
            billing_service_id: p.billing_service_id,
        })
    } else {
        None
    };
    let revision = i.revision;
    let row = ManagedVoiceAccount {
        id: uuid::Uuid::new_v4().to_string(),
        trusted_account_id: account.into(),
        revision: revision + 1,
        per_call_characters: i.per_call_characters,
        greeting: i.greeting,
        wrapup: i.wrapup,
        premium,
    };
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,actor,account,&row,revision,&proved_account,state.audit_chain_hmac_key.as_ref().as_ref()),|tx,c|Box::pin(async move {
        let (db,actor,account,row,revision,proved,key)=*c;
        eligibility::admin_fence(db,tx,actor).await?;
        let a=db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"_id":account,"provider":"telnyx","revision":proved.revision,"fence":custody::account_fence_value(proved.fence),"catalog_service_id":&proved.catalog_service_id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        let old=db.collection::<ManagedVoiceAccount>(ManagedVoiceAccount::COLLECTION_NAME).find_one(doc!{"trusted_account_id":account}).session(&mut *tx).await?;
        if revision<0 || old.as_ref().map_or(0,|r|r.revision)!=revision {return Err(conflict("Voice configuration changed"));}
        if let Some(p)=&row.premium {
            use crate::models::downstream_service::{DownstreamService,COLLECTION_NAME};
            let service=db.collection::<DownstreamService>(COLLECTION_NAME).find_one(doc!{"_id":&p.billing_service_id,"is_active":true}).session(&mut *tx).await?.ok_or_else(unavailable)?;
            custody::dedicated_service(db,tx,&service).await?;
            if service.managed_comms_role.is_some_and(|r|r!=ManagedCommsRole::Premium) || !service.credential_encrypted.is_empty() || service.token_exchange_config.is_some() || service.platform_key.as_ref().is_some_and(|p|p.enabled) {return Err(unavailable());}
            db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc!{"_id":&service.id},doc!{"$set":{"managed_comms_role":"premium"},"$inc":{"managed_comms_fence":1_i64}}).session(&mut *tx).await?;
            // Every Telnyx account using this ElevenLabs identity shares one policy fence.
            let identity=super::super::session::identity(&["elevenlabs-policy",&p.billing_account_identity]);
            let rows=db.collection::<bson::Document>(ManagedCharacterMonth::COLLECTION_NAME);
            rows.update_one(doc!{"identity_key":&identity},doc!{"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"consumed":0_i64},"$set":{"limit":p.monthly_characters},"$inc":{"fence":1_i64}}).upsert(true).session(&mut *tx).await?;

        }
        if db.collection::<ProviderAccount>(ACCOUNTS).update_one(doc!{"_id":account,"revision":a.revision,"fence":custody::account_fence_value(a.fence)},doc!{"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1 {return Err(unavailable());}
        let mut saved=row.clone();if let Some(old)=old {saved.id=old.id;}
        db.collection::<ManagedVoiceAccount>(ManagedVoiceAccount::COLLECTION_NAME).replace_one(doc!{"trusted_account_id":account},&saved).upsert(true).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_voice_account_configured",serde_json::json!({"trusted_account_id":account,"revision":row.revision,"premium":row.premium.is_some()})).await?;Ok(())
    })).await
}
pub async fn set_enabled(state: &AppState, actor: &AuditActor, on: bool) -> AppResult<()> {
    if on {
        super::require_ready(&state.db).await?;
    }
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(
        &db,
        &(&db, actor, on, state.audit_chain_hmac_key.as_ref().as_ref()),
        |tx, c| {
            Box::pin(async move {
                let (db, actor, on, key) = *c;
                eligibility::admin_fence(db, tx, actor).await?;
                db.collection::<bson::Document>(ManagedVoiceSwitch::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":SWITCH},
                        doc! {"$set":{"premium_enabled":on},"$inc":{"fence":1_i64}},
                    )
                    .upsert(true)
                    .session(&mut *tx)
                    .await?;
                audit_service::log_actor_event_in_session(
                    db,
                    tx,
                    key,
                    actor,
                    "managed_premium_switch_changed",
                    serde_json::json!({"enabled":on}),
                )
                .await?;
                Ok(())
            })
        },
    )
    .await
}
pub async fn validate_selection(
    db: &Database,
    account: &str,
    e: &ManagedCommsEligibility,
    terms: &TermsVersions,
    option: &StandardVoiceOption,
) -> AppResult<ManagedVoiceAccount> {
    let c = config(db, account).await?.ok_or_else(unavailable)?;
    let p = c.premium.as_ref().ok_or_else(unavailable)?;
    if !e.premium_allowed
        || terms.elevenlabs_eua.as_deref() != Some(&p.oem_terms_version)
        || !p.voices.contains(option)
        || !enabled(db).await?
    {
        return Err(conflict(
            "Premium voice requires live configuration and accepted End User Agreement",
        ));
    }
    Ok(c)
}
pub async fn initialize(
    db: &Database,
    tx: &mut ClientSession,
    s: &CallSession,
    c: Option<&ManagedVoiceAccount>,
    premium: bool,
) -> AppResult<()> {
    if let Some(c) = c
        && db
            .collection::<ManagedVoiceAccount>(ManagedVoiceAccount::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&c.id,"revision":c.revision},
                doc! {"$inc":{"admission_fence":1_i64}},
            )
            .session(&mut *tx)
            .await?
            .matched_count
            != 1
    {
        return Err(unavailable());
    }
    let p = if premium {
        Some(c.and_then(|c| c.premium.clone()).ok_or_else(unavailable)?)
    } else {
        None
    };
    let reserve_month_id = if let Some(p) = &p {
        Some(month_debit(db, tx, p, WRAPUP_RESERVE).await?)
    } else {
        None
    };
    let row = ManagedCharacterAllowance {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: s.id.clone(),
        limit: c.map_or(12000, |c| c.per_call_characters),
        consumed: 0,
        wrapup_consumed: 0,
        greeting: c.and_then(|c| c.greeting.clone()),
        wrapup: c.and_then(|c| c.wrapup.clone()),
        reserve_month_id,
        reserve_released: false,
        premium: p,
        premium_fallback: false,
        fence: 0,
    };
    db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME)
        .insert_one(row)
        .session(&mut *tx)
        .await?;
    Ok(())
}
async fn month_debit(
    db: &Database,
    tx: &mut ClientSession,
    p: &PremiumVoiceConfig,
    n: i64,
) -> AppResult<String> {
    let now = Utc::now();
    let identity = super::super::session::identity(&[
        "elevenlabs",
        &p.billing_account_identity,
        &now.year().to_string(),
        &now.month().to_string(),
    ]);
    let rows = db.collection::<ManagedCharacterMonth>(ManagedCharacterMonth::COLLECTION_NAME);
    let policy =
        super::super::session::identity(&["elevenlabs-policy", &p.billing_account_identity]);
    let policy_row = db
        .collection::<bson::Document>(ManagedCharacterMonth::COLLECTION_NAME)
        .find_one(doc! {"identity_key":&policy})
        .session(&mut *tx)
        .await?
        .ok_or_else(unavailable)?;
    let limit = policy_row.get_i64("limit").map_err(|_| unavailable())?;
    if db
        .collection::<bson::Document>(ManagedCharacterMonth::COLLECTION_NAME)
        .update_one(
            doc! {"identity_key":&policy,"limit":limit},
            doc! {"$inc":{"fence":1_i64}},
        )
        .session(&mut *tx)
        .await?
        .matched_count
        != 1
    {
        return Err(unavailable());
    }

    let m = rows
        .find_one(doc! {"identity_key":&identity})
        .session(&mut *tx)
        .await?;
    let m = m.unwrap_or(ManagedCharacterMonth {
        id: uuid::Uuid::new_v4().to_string(),
        identity_key: identity.clone(),
        consumed: 0,
        fence: 0,
    });
    if m.consumed.checked_add(n).is_none_or(|v| v > limit) {
        return Err(AppError::ManagedBudgetExhausted);
    }
    rows.update_one(
        doc! {"identity_key":identity,"fence":m.fence},
        doc! {"$setOnInsert":{"_id":&m.id},"$set":{"consumed":m.consumed+n},"$inc":{"fence":1_i64}},
    )
    .upsert(true)
    .session(&mut *tx)
    .await?;
    Ok(m.id)
}
pub async fn allowance(db: &Database, id: &str) -> AppResult<ManagedCharacterAllowance> {
    db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME)
        .find_one(doc! {"session_id":id})
        .await?
        .ok_or_else(unavailable)
}
/// The debit itself is the dispatch barrier. A caller cannot replay its block.
pub async fn debit(
    state: &AppState,
    s: &CallSession,
    text: &str,
    wrapup: bool,
) -> AppResult<ManagedCharacterBlock> {
    debit_capacity(state, s, text.chars().count() as i64, wrapup, false).await
}
async fn debit_capacity(
    state: &AppState,
    s: &CallSession,
    n: i64,
    wrapup: bool,
    partial: bool,
) -> AppResult<ManagedCharacterBlock> {
    super::super::session::fence(state, s).await?;
    if n <= 0 || n > BLOCK as i64 {
        return Err(unavailable());
    }
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,s,n,wrapup,partial),|tx,c|Box::pin(async move {
        let (db,s,n,wrapup,partial)=*c;
        let mut n=n;
        let mut a=db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME).find_one(doc!{"session_id":&s.id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if partial { n=n.min((a.limit-WRAPUP_RESERVE-a.consumed).max(0)); }
        if n<=0 || a.reserve_released || a.consumed+n>a.limit || (!wrapup && a.consumed+n>a.limit-WRAPUP_RESERVE) || (wrapup && a.wrapup_consumed+n>WRAPUP_RESERVE) {return Err(AppError::ManagedBudgetExhausted);}
        if let Some(p)=a.premium.as_ref().filter(|_|!a.premium_fallback) {
            // Even wrap-up is reserved character capacity; kill-off allows only this bounded ending.
            let changed=db.collection::<ManagedVoiceSwitch>(ManagedVoiceSwitch::COLLECTION_NAME).update_one(doc!{"_id":SWITCH,"premium_enabled":true},doc!{"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
            if changed.matched_count!=1 && !wrapup {return Err(AppError::ManagedBudgetExhausted);}
            if p.monthly_characters<n {return Err(AppError::ManagedBudgetExhausted);}
        }
        let now=Utc::now();
        let month_id=if let Some(p)=a.premium.as_ref().filter(|_|!a.premium_fallback && !wrapup) {
            Some(month_debit(db,tx,p,n).await?)
        }else{None};
        if db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"generation":s.generation,"claim_id":&s.claim_id,"binding_revision":s.binding_revision,"state":{"$in":["prepared","active"]},"expires_at":{"$gt":bson::DateTime::from_chrono(now)},"lease_expires_at":{"$gt":bson::DateTime::from_chrono(now)}},doc!{"$inc":{"synthesis_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        a.consumed+=n;if wrapup{a.wrapup_consumed+=n;}a.fence+=1;
        db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME).replace_one(doc!{"_id":&a.id,"fence":a.fence-1},&a).session(&mut *tx).await?;
        let block=ManagedCharacterBlock{id:uuid::Uuid::new_v4().to_string(),session_id:s.id.clone(),writer_id:s.claim_id.clone().unwrap_or_else(||"relay-start".into()),generation:s.generation,characters:n,wrapup,month_id,status:"uncertain".into(),created_at:now};
        db.collection::<ManagedCharacterBlock>(ManagedCharacterBlock::COLLECTION_NAME).insert_one(&block).session(&mut *tx).await?;Ok(block)
    })).await
}
pub async fn written(db: &Database, block: &ManagedCharacterBlock) -> AppResult<()> {
    db.collection::<ManagedCharacterBlock>(ManagedCharacterBlock::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&block.id,"status":"uncertain"},
            doc! {"$set":{"status":"written"}},
        )
        .await?;
    Ok(())
}
/// Called only when the local writer proves send was never invoked. Uncertain
/// socket writes and interrupted playback never qualify for a credit.
pub async fn unwritten(db: &Database, block: &ManagedCharacterBlock) -> AppResult<()> {
    credit_remainder(db, block, block.characters).await
}
async fn credit_remainder(
    db: &Database,
    block: &ManagedCharacterBlock,
    remainder: i64,
) -> AppResult<()> {
    if !(0..=block.characters).contains(&remainder) {
        return Err(unavailable());
    }
    if remainder == 0 {
        return written(db, block).await;
    }
    transaction_service::run_db_only(db, &(db, block, remainder), |tx, c| {
        Box::pin(async move {
            let (db, b, remainder) = *c;
            if db
                .collection::<ManagedCharacterBlock>(ManagedCharacterBlock::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&b.id,"status":"uncertain"},
                    doc! {"$set":{"status":"proven_unwritten","credited_characters":remainder}},
                )
                .session(&mut *tx)
                .await?
                .modified_count
                == 1
            {
                let mut inc = doc! {"consumed":-remainder,"fence":1_i64};
                if b.wrapup {
                    inc.insert("wrapup_consumed", -remainder);
                }
                db.collection::<ManagedCharacterAllowance>(
                    ManagedCharacterAllowance::COLLECTION_NAME,
                )
                .update_one(doc! {"session_id":&b.session_id}, doc! {"$inc":inc})
                .session(&mut *tx)
                .await?;
                // Wrap-up was debited from the admission reserve. If terminal
                // cleanup already released that reserve, credit this now-proven
                // unwritten block directly; otherwise cleanup will release it.
                let allowance = db
                    .collection::<ManagedCharacterAllowance>(
                        ManagedCharacterAllowance::COLLECTION_NAME,
                    )
                    .find_one(doc! {"session_id":&b.session_id})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(unavailable)?;
                let month_id = b.month_id.as_ref().or_else(|| {
                    (b.wrapup && allowance.reserve_released)
                        .then_some(allowance.reserve_month_id.as_ref())
                        .flatten()
                });
                if let Some(id) = month_id {
                    db.collection::<ManagedCharacterMonth>(ManagedCharacterMonth::COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":id},
                            doc! {"$inc":{"consumed":-remainder,"fence":1_i64}},
                        )
                        .session(&mut *tx)
                        .await?;
                }
            }
            Ok(())
        })
    })
    .await
}
/// Only the preflight caller may enter this before a premium relay command.
pub async fn fallback_before_dispatch(state: &AppState, s: &CallSession) -> AppResult<()> {
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(&db,&(&db,s),|tx,c|Box::pin(async move {
        let (db,s)=*c;
        let r=db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME).find_one(doc!{"session_id":&s.id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if r.premium.is_none(){return Err(unavailable());}
        let session=db.collection::<CallSession>(SESSIONS).find_one(doc!{"_id":&s.id,"relay_dispatched":false,"state":"prepared"}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if session.generation!=s.generation {return Err(unavailable());}
        if db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"generation":s.generation,"relay_dispatched":false,"state":"prepared"},doc!{"$inc":{"synthesis_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        let a=db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME).find_one(doc!{"session_id":&s.id,"consumed":0_i64}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if let Some(month)=&a.reserve_month_id {
            db.collection::<ManagedCharacterMonth>(ManagedCharacterMonth::COLLECTION_NAME).update_one(doc!{"_id":month},doc!{"$inc":{"consumed":-WRAPUP_RESERVE,"fence":1_i64}}).session(&mut *tx).await?;
        }
        db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME).update_one(doc!{"_id":&a.id},doc!{"$set":{"premium_fallback":true,"reserve_month_id":bson::Bson::Null,"reserve_released":false},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc!{"_id":&r.id},doc!{"$set":{"premium_fallback":"proven_pre_dispatch"}}).session(&mut *tx).await?;Ok(())
    })).await?;
    let r = super::calls::receipt(state, &s.id).await?;
    if let Some(m) = r.premium {
        super::super::billing::settle_admitted(state, &m.billing_request_id, &s.user_id, 0, 1)
            .await?;
    }
    Ok(())
}

/// The unused wrap-up reserve is provably unwritten. All dispatched blocks,
/// including uncertain ones, remain consumed independently of termination.
pub async fn release_reserve(db: &Database, id: &str) -> AppResult<()> {
    transaction_service::run_db_only(db,&(db,id),|tx,c|Box::pin(async move {
        let (db,id)=*c;
        let Some(a)=db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME).find_one(doc!{"session_id":id,"reserve_released":false}).session(&mut *tx).await? else {return Ok(());};
        if let Some(month)=&a.reserve_month_id {
            db.collection::<ManagedCharacterMonth>(ManagedCharacterMonth::COLLECTION_NAME).update_one(doc!{"_id":month},doc!{"$inc":{"consumed":-(WRAPUP_RESERVE-a.wrapup_consumed),"fence":1_i64}}).session(&mut *tx).await?;
        }
        db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME).update_one(doc!{"_id":&a.id,"fence":a.fence},doc!{"$set":{"reserve_released":true},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;Ok(())
    })).await
}

/// Local proof of unused capacity belongs to the sole socket writer. It is never
/// reconstructed from durable rows: a crash or uncertain send consumes the block.
#[derive(Default)]
pub struct Writer {
    current: Option<(ManagedCharacterBlock, usize)>,
    pub wrapup: String,
}
impl Writer {
    pub async fn capacity(&mut self, state: &AppState, s: &CallSession) -> AppResult<usize> {
        if self.current.as_ref().is_some_and(|(b, _)| {
            b.generation != s.generation || Some(&b.writer_id) != s.claim_id.as_ref()
        }) {
            self.finish(&state.db).await?;
        }
        if self
            .current
            .as_ref()
            .is_none_or(|(_, remaining)| *remaining == 0)
        {
            self.finish(&state.db).await?;
            let block = debit_capacity(state, s, BLOCK as i64, false, true).await?;
            let remaining = block.characters as usize;
            self.current = Some((block, remaining));
        }
        Ok(self.current.as_ref().ok_or_else(unavailable)?.1)
    }
    /// Invoke immediately before handing text to the socket. No await separates
    /// this bookkeeping and send invocation. An uncertain send forgets all proof.
    pub fn attempting(&mut self, n: usize) -> AppResult<()> {
        let (_, remaining) = self.current.as_mut().ok_or_else(unavailable)?;
        *remaining = remaining.checked_sub(n).ok_or_else(unavailable)?;
        Ok(())
    }
    pub fn uncertain(&mut self) {
        self.current = None;
    }
    pub async fn finish(&mut self, db: &Database) -> AppResult<()> {
        if let Some((block, remaining)) = self.current.take() {
            credit_remainder(db, &block, remaining as i64).await?;
        }
        Ok(())
    }
}
