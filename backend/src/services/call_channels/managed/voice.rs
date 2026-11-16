//! Standard synthesis options are a platform allowlist, never a merchant voice ID.
use super::{conflict, eligibility, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{channel_adapter::*, managed_comms::*},
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::Utc;
use mongodb::bson::{self, Document, doc};
use serde::Deserialize;

pub fn default_option(provider: &str) -> StandardVoiceOption {
    StandardVoiceOption {
        voice: if provider == "twilio" {
            "en-US-Standard-A"
        } else {
            "Telnyx.Ultra.Callie"
        }
        .into(),
        language: "en-US".into(),
    }
}
pub fn options(line: &ManagedPhoneLine) -> Vec<StandardVoiceOption> {
    if line.voice.is_none() {
        return vec![];
    }
    if line.standard_voice_options.is_empty() {
        vec![default_option(&line.provider)]
    } else {
        line.standard_voice_options.clone()
    }
}
pub fn validate_option(provider: &str, option: &StandardVoiceOption) -> AppResult<()> {
    // Fixed standard families only. Provider/integration secrets and premium
    // ElevenLabs identifiers cannot enter the standard-price path.
    let allowed = match provider {
        "twilio" => [
            ("en-US-Standard-A", "en-US"),
            ("en-GB-Standard-A", "en-GB"),
            ("en-AU-Standard-A", "en-AU"),
            ("cmn-CN-Standard-A", "cmn-CN"),
            ("cmn-TW-Standard-A", "cmn-TW"),
        ]
        .contains(&(option.voice.as_str(), option.language.as_str())),
        "telnyx" => {
            option.voice == "Telnyx.Ultra.Callie"
                && ["en-US", "zh-CN", "zh-TW"].contains(&option.language.as_str())
        }
        _ => false,
    };
    if !allowed {
        return Err(AppError::ValidationError(
            "Unsupported standard voice and language pair".into(),
        ));
    }
    Ok(())
}
pub fn validate_options(
    provider: &str,
    voice: bool,
    choices: &[StandardVoiceOption],
) -> AppResult<()> {
    if choices.len() > 16 || (!voice && !choices.is_empty()) {
        return Err(unavailable());
    }
    for (index, option) in choices.iter().enumerate() {
        validate_option(provider, option)?;
        if choices[..index].contains(option) {
            return Err(conflict("Duplicate standard voice option"));
        }
    }
    if !choices.is_empty() && !choices.contains(&default_option(provider)) {
        return Err(conflict("Standard voice options must include the default"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowlistInput {
    pub revision: i64,
    pub options: Vec<StandardVoiceOption>,
}
pub async fn configure(
    state: &AppState,
    actor: &AuditActor,
    line_id: &str,
    input: AllowlistInput,
) -> AppResult<()> {
    super::require_ready(&state.db).await?;
    transaction_service::run_db_only(&state.db,&(&state.db,actor,line_id,&input,state.audit_chain_hmac_key.as_ref().as_ref()),|tx,ctx|Box::pin(async move {
        let &(db,actor,line_id,input,key)=ctx;
        eligibility::admin_fence(db,tx,actor).await?;
        let line=db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).find_one(doc!{"_id":line_id,"revision":input.revision}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        validate_options(&line.provider,line.voice.is_some(),&input.options)?;
        let choices=if input.options.is_empty(){vec![default_option(&line.provider)]}else{input.options.clone()};
        if let Some(claim)=&line.current_claim_id {
            let c=db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find_one(doc!{"_id":claim}).session(&mut *tx).await?.ok_or_else(unavailable)?;
            if c.voice_tier==VoiceTier::Standard && !choices.contains(&c.standard_voice.unwrap_or_else(||default_option(&line.provider))) { return Err(conflict("The claimed voice must remain allowed")); }
        }
        if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":line_id,"revision":input.revision,"fence":line.fence},doc!{"$set":{"standard_voice_options":bson::to_bson(&input.options).map_err(|_|unavailable())?,"updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64,"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1 {return Err(unavailable());}
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_line_standard_voice_options_changed",serde_json::json!({"line_id":line_id,"revision":input.revision+1,"option_count":input.options.len()})).await?;
        Ok(())
    })).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionInput {
    pub claim_id: String,
    pub binding_revision: i64,
    pub option: StandardVoiceOption,
}
pub async fn select(
    state: &AppState,
    actor: &AuditActor,
    line_id: &str,
    input: SelectionInput,
) -> AppResult<()> {
    super::require_enabled(&state.db, &actor.user_id).await?;
    transaction_service::run_db_only(&state.db,&(&state.db,actor,line_id,&input,state.audit_chain_hmac_key.as_ref().as_ref()),|tx,ctx|Box::pin(async move {
        let &(db,actor,line_id,input,key)=ctx;
        super::super::owner_admission::fence(db,tx,&actor.user_id).await?;
        let c=db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME).find_one(doc!{"_id":&input.claim_id,"line_id":line_id,"user_id":&actor.user_id,"state":{"$in":["claimed","suspended"]}}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        let line=db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).find_one(doc!{"_id":line_id,"current_claim_id":&c.id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        let premium=input.option.voice.starts_with("ElevenLabs.");
        if premium {
            let e=super::eligibility::load(db,&actor.user_id).await?;
            super::eligibility::fence(db,tx,&e).await?;
            if e.revision!=c.eligibility_revision || line.provider!="telnyx" {return Err(unavailable());}
            super::synthesis::validate_selection(db,&line.trusted_account_id,&e,&c.accepted_terms,&input.option).await?;
            let account=db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"_id":&line.trusted_account_id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
            if db.collection::<ProviderAccount>(ACCOUNTS).update_one(doc!{"_id":&account.id,"fence":super::custody::account_fence_value(account.fence)},doc!{"$inc":{super::admission::ADMISSION_TOUCH:1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        }else{
            validate_option(&line.provider,&input.option)?;
            if !options(&line).contains(&input.option) {return Err(conflict("Voice option is not allowed on this line"));}
        }
        let binding=c.voice_binding_id.as_ref().ok_or_else(unavailable)?;
        // Admission also writes this binding; the idle check and revision write
        // therefore serialize with calls admitted on another replica.
        if db.collection::<Document>(SESSIONS).find_one(doc!{"binding_id":binding,"state":{"$ne":"closed"}}).session(&mut *tx).await?.is_some() {return Err(conflict("End the live call before changing voice or language"));}
        let now=Utc::now();
        if db.collection::<Document>(BINDINGS).update_one(doc!{"_id":binding,"user_id":&actor.user_id,"revision":input.binding_revision},doc!{"$set":{"voice":&input.option.voice,"language":&input.option.language,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64}}).session(&mut *tx).await?.matched_count!=1 {return Err(conflict("Binding changed"));}
        if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"fence":c.fence},doc!{"$set":{"standard_voice":bson::to_bson(&input.option).map_err(|_|unavailable())?,"voice_tier":if premium{"premium"}else{"standard"}},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":line_id,"fence":line.fence,"revision":line.revision},doc!{"$inc":{super::admission::ADMISSION_TOUCH:1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_line_standard_voice_changed",serde_json::json!({"line_id":line_id,"claim_id":c.id,"binding_id":binding,"binding_revision":input.binding_revision+1})).await?;
        Ok(())
    })).await
}
