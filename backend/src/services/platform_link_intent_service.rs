//! Personal proofs: 24-hour bearer intents followed by fresh 10-minute reviews.
use super::platform_entry_scope as scope;
use super::platform_manager_contract::{EntryIdentity, SenderIdentity};
use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    platform_manager_contract::{ManagerChatKind, PublicExecutionContext, VerifiedManagerUpdate},
    platform_personal_binding_service as bindings,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_bot_runtime_controls::PlatformBotRuntimeControls,
        platform_link_intent::*,
        platform_personal_binding::{
            PersonalBindingState, PersonalRevocationReason, PlatformPersonalBinding,
        },
    },
};
use chrono::Duration;
#[cfg(test)]
use chrono::Utc;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};
use rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

pub struct IntentIssued {
    pub intent_id: String,
    #[cfg(test)]
    pub intent: PlatformLinkIntent,
    pub token: Option<Zeroizing<String>>,
}
pub struct ChallengeApproved {
    pub challenge: PlatformLinkChallenge,
    pub confirmation_command: Option<Zeroizing<String>>,
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    super::platform_entry_migration::ensure_indexes(db).await?;
    let intents = db.collection::<PlatformLinkIntent>(COLLECTION_NAME);
    for index in [
        IndexModel::builder().keys(doc! {"token_lookup_hmac":1}).options(IndexOptions::builder().unique(true).build()).build(),
        IndexModel::builder().keys(doc! {"claimant_user_id":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"platform_config_id":1,"manager_authority_epoch":1,"telegram_sender_id":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"expires_at":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"purge_after":1}).options(IndexOptions::builder().expire_after(std::time::Duration::ZERO).build()).build(),
    ]{intents.create_index(index).await?;}
    let challenges = db.collection::<PlatformLinkChallenge>(CHALLENGES);
    for index in [
        IndexModel::builder().keys(doc! {"code_lookup_hmac":1}).options(IndexOptions::builder().unique(true).partial_filter_expression(doc! {"code_lookup_hmac":{"$type":"string"}}).build()).build(),
        IndexModel::builder().keys(doc! {"intent_id":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"reviewed_user_id":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"platform_config_id":1,"manager_authority_epoch":1,"telegram_sender_id":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"expires_at":1,"state":1}).build(),
        IndexModel::builder().keys(doc! {"purge_after":1}).options(IndexOptions::builder().expire_after(std::time::Duration::ZERO).build()).build(),
    ]{challenges.create_index(index).await?;}
    Ok(())
}

fn hash(key: &[u8], domain: &str, parts: &[&str]) -> String {
    let mut material = Zeroizing::new(Vec::new());
    for part in std::iter::once(domain).chain(parts.iter().copied()) {
        material.extend_from_slice(&(part.len() as u64).to_be_bytes());
        material.extend_from_slice(part.as_bytes());
    }
    super::auth_device_service::hmac_hex(key, &material)
}
fn equal(a: &str, b: &str) -> bool {
    crate::crypto::token::constant_time_eq(a.as_bytes(), b.as_bytes())
}
fn token_lookup(key: &[u8], token: &str) -> AppResult<String> {
    if token.len() != 72
        || !token.starts_with("nyx_pli_")
        || !token[8..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bindings::unavailable());
    }
    Ok(hash(key, "platform-personal-intent-lookup:v1", &[token]))
}
fn identity_hash(
    key: &[u8],
    legacy_domain: &str,
    entry: &EntryIdentity,
    sender: &SenderIdentity,
    prefix: &[&str],
    suffix: &[&str],
) -> String {
    let identity = scope::hmac_identity(entry, sender);
    let domain = match entry {
        EntryIdentity::TelegramV1 { .. } => legacy_domain.to_owned(),
        EntryIdentity::NativeV2 { .. } => format!(
            "{}:v2",
            legacy_domain.strip_suffix(":v1").expect("versioned domain")
        ),
    };
    let parts = prefix
        .iter()
        .copied()
        .chain(identity.iter().map(String::as_str))
        .chain(suffix.iter().copied())
        .collect::<Vec<_>>();
    hash(key, &domain, &parts)
}
fn intent_hash(key: &[u8], row: &PlatformLinkIntent, token: &str) -> String {
    identity_hash(
        key,
        "platform-personal-intent:v1",
        &row.entry,
        &row.sender,
        &[
            &row.purpose,
            &row.id,
            &row.platform_config_id,
            &row.manager_bot_id,
            &row.manager_authority_epoch.to_string(),
        ],
        &[token],
    )
}
fn normalize_code(code: &str) -> AppResult<Zeroizing<String>> {
    if code.len() > 32 {
        return Err(bindings::unavailable());
    }
    let code = Zeroizing::new(code.trim().to_ascii_uppercase());
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    if code.len() != 11
        || !code.starts_with("P-")
        || code.as_bytes()[6] != b'-'
        || !code.as_bytes()[2..6]
            .iter()
            .chain(&code.as_bytes()[7..11])
            .all(|b| ALPHABET.contains(b))
    {
        return Err(bindings::unavailable());
    }
    Ok(code)
}
fn code_lookup(key: &[u8], code: &str) -> AppResult<String> {
    Ok(hash(
        key,
        "platform-personal-challenge-lookup:v1",
        &[normalize_code(code)?.as_str()],
    ))
}
fn challenge_hash(key: &[u8], row: &PlatformLinkChallenge, code: &str) -> AppResult<String> {
    Ok(identity_hash(
        key,
        "platform-personal-challenge:v1",
        &row.entry,
        &row.sender,
        &[
            &row.purpose,
            &row.id,
            &row.intent_id,
            &row.platform_config_id,
            &row.manager_bot_id,
            &row.manager_authority_epoch.to_string(),
        ],
        &[
            &row.reviewed_user_id,
            &row.review_digest,
            normalize_code(code)?.as_str(),
        ],
    ))
}
fn review_hash(key: &[u8], row: &PlatformLinkChallenge) -> AppResult<String> {
    let previous =
        serde_json::to_string(&row.previous_binding).map_err(|_| bindings::conflict())?;
    Ok(identity_hash(
        key,
        "platform-personal-consent:v1",
        &row.entry,
        &row.sender,
        &[
            &row.purpose,
            &row.id,
            &row.intent_id,
            &row.platform_config_id,
            &row.manager_bot_id,
            &row.manager_authority_epoch.to_string(),
        ],
        &[
            &row.reviewed_user_id,
            &row.reviewed_access_mode,
            &row.consent_version.to_string(),
            &previous,
        ],
    ))
}
fn proof_command(update: &VerifiedManagerUpdate, forwarded: bool) -> AppResult<()> {
    if forwarded
        || update.command_eligibility
            != super::platform_manager_contract::ManagerCommandEligibility::OriginalPrivateMessage
        || update.chat_kind != ManagerChatKind::Private
        || scope::validate(&update.entry, &update.manager_bot_id, Some(&update.sender)).is_err()
    {
        return Err(AppError::Forbidden(
            "Personal controls require the initiating private chat".into(),
        ));
    }
    Ok(())
}
fn confirmation_rate_key(update: &VerifiedManagerUpdate) -> String {
    match &update.sender {
        SenderIdentity::TelegramV1 {
            telegram_sender_id,
            telegram_chat_id,
        } => format!(
            "confirm:{}:{}:{}",
            update.profile_id, telegram_sender_id, telegram_chat_id
        ),
        SenderIdentity::NativeV2 { .. } => format!(
            "confirm:{}:v2:{}",
            update.profile_id,
            super::mcp_service::canonical_sha256(
                serde_json::json!({"entry":update.entry,"sender":update.sender})
            )
        ),
    }
}
fn live() -> Document {
    doc! {"$gt":["$expires_at","$$NOW"]}
}

pub async fn begin_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    update: &VerifiedManagerUpdate,
    forwarded: bool,
    initiation_ref: &str,
) -> AppResult<IntentIssued> {
    proof_command(update, forwarded)?;
    if initiation_ref.is_empty() || initiation_ref.len() > 128 {
        return Err(bindings::unavailable());
    }
    let controls = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &update.profile_id,
        &update.manager_bot_id,
        &update.entry,
        Some(&update.observation_id),
    )
    .await?
    .ok_or_else(bindings::unavailable)?;
    mint_intent_in_session(
        db,
        session,
        key,
        IntentMint {
            controls: &controls,
            sender: &update.sender,
            reference: initiation_ref,
        },
    )
    .await
}

/// Public input can request a bearer intent, never approve personal consent.
/// The caller owns commit; an error requires aborting all accompanying effects.
pub(crate) async fn begin_public_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    context: &PublicExecutionContext,
) -> AppResult<IntentIssued> {
    let authority = fence_public_authority_in_session(db, session, key, context).await?;
    mint_intent_in_session(
        db,
        session,
        key,
        IntentMint {
            controls: &authority.controls,
            sender: &authority.sender,
            reference: &authority.reference,
        },
    )
    .await
}

/// Validate a public signup bearer for one transport attempt, returning only its ID.
/// The caller owns commit and must abort on error; this neither consumes consent
/// nor grants permission to retry delivery.
pub(crate) async fn fence_public_intent_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    context: &PublicExecutionContext,
    token: &str,
) -> AppResult<String> {
    let lookup = token_lookup(key, token)?;
    let authority = fence_public_authority_in_session(db, session, key, context).await?;
    let controls = &authority.controls;
    let rows = db.collection::<PlatformLinkIntent>(COLLECTION_NAME);
    let mut filter = doc! {
        "token_lookup_hmac": lookup, "purpose": PURPOSE,
        "platform_config_id": &controls.platform_config_id,
        "manager_bot_id": &controls.manager_bot_id,
        "manager_authority_epoch": controls.manager_authority_epoch,
        "initiation_ref": &authority.reference, "state": "pending", "$expr": live(),
    };
    filter.extend(scope::entry_fields(&controls.entry));
    filter.extend(scope::sender_fields(&authority.sender, false));
    scope::exclude_other_version(&mut filter, &controls.entry);
    let row = rows
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
        .ok_or_else(bindings::unavailable)?;
    if !equal(&row.token_hmac, &intent_hash(key, &row, token)) {
        return Err(bindings::unavailable());
    }
    filter.insert("_id", &row.id);
    filter.insert("token_hmac", &row.token_hmac);
    // A real conditional write conflicts with concurrent cancellation/consumption.
    // This internal counter is not delivery state; the transport owns dedupe.
    if rows
        .update_one(filter, doc! {"$inc": {"public_delivery_fence": 1_i64}})
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(bindings::unavailable());
    }
    Ok(row.id)
}

struct PublicIntentAuthority {
    controls: PlatformBotRuntimeControls,
    sender: SenderIdentity,
    reference: String,
}

async fn fence_public_authority_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    context: &PublicExecutionContext,
) -> AppResult<PublicIntentAuthority> {
    use crate::models::{
        platform_manager_ingress::{COLLECTION_NAME as RECEIPTS, PlatformManagerIngress},
        platform_public_session::COLLECTION_NAME as PUBLIC_SESSIONS,
    };
    let ingress = &context.ingress;
    let manager = &ingress.manager;
    // A public receipt was admitted only from verified private native ingress.
    // Match its own identity too: the budget fence checks execution linkage but
    // deliberately is not a constructor of private-message provenance.
    let mut receipt_filter = doc! {
        "_id": &ingress.ingress.receipt_id, "claim_id": &ingress.ingress.claim_id,
        "profile_id": &manager.profile_id, "manager_bot_id": &manager.manager_bot_id,
        "manager_authority_epoch": manager.manager_authority_epoch,
        "audience.kind": "public", "audience.public_policy_revision": ingress.public_policy_revision,
        "phase": "executing", "settled_at": bson::Bson::Null,
        "execution.public_session_id": &context.public_session_id,
        "execution.conversation_id": &context.turn.conversation_id, "execution.turn_id": &context.turn.turn_id,
        "execution.payer_user_id": &context.sponsor_owner_user_id,
        "execution.public_policy_digest": &context.policy_digest,
    };
    receipt_filter.extend(scope::entry_fields(&manager.entry));
    receipt_filter.extend(scope::sender_fields(&ingress.sender, true));
    scope::exclude_other_version(&mut receipt_filter, &manager.entry);
    let receipt = db
        .collection::<PlatformManagerIngress>(RECEIPTS)
        .find_one(receipt_filter)
        .session(&mut *session)
        .await?
        .ok_or_else(bindings::unavailable)?;
    if scope::validate(
        &receipt.entry,
        &receipt.manager_bot_id,
        Some(&receipt.sender),
    )
    .is_err()
        || receipt.observation_id.is_empty()
    {
        return Err(bindings::unavailable());
    }
    let stored = db.collection::<Document>(PUBLIC_SESSIONS).find_one(doc! {
        "_id": &context.public_session_id,
        "active_turn.receipt_id": &receipt.id, "active_turn.claim_id": &receipt.claim_id,
        "active_turn.conversation_id": &context.turn.conversation_id, "active_turn.turn_id": &context.turn.turn_id,
        "$expr": {"$and": [
            {"$gt": [bson::DateTime::from_chrono(context.expires_at), "$$NOW"]},
            {"$lte": [bson::DateTime::from_chrono(context.expires_at), "$expires_at"]},
            {"$lte": [bson::DateTime::from_chrono(context.expires_at), "$active_turn.expires_at"]},
        ]},
    }).projection(doc! {"credential_api_key_id": 1}).session(&mut *session).await?.ok_or_else(bindings::unavailable)?;
    let key_id = stored
        .get_str("credential_api_key_id")
        .map_err(|_| bindings::unavailable())?;
    // Includes current controls, sponsor, exact key, revocation scopes and
    // receipt write fences; it joins this transaction without any network work.
    super::platform_public_budget_service::fence_context(db, session, context, key_id).await?;
    let controls = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &receipt.profile_id,
        &receipt.manager_bot_id,
        &receipt.entry,
        Some(&receipt.observation_id),
    )
    .await?
    .ok_or_else(bindings::unavailable)?;
    if !controls.personal_enabled
        || controls.manager_authority_epoch != receipt.manager_authority_epoch
    {
        return Err(bindings::unavailable());
    }
    let reference = format!(
        "public:{}",
        hash(
            key,
            "platform-personal-public-initiation:v1",
            &[
                &receipt.id,
                &receipt.claim_id,
                &context.public_session_id,
                &context.turn.conversation_id,
                &context.turn.turn_id,
            ]
        )
    );
    Ok(PublicIntentAuthority {
        controls,
        sender: receipt.sender,
        reference,
    })
}

struct IntentMint<'a> {
    controls: &'a PlatformBotRuntimeControls,
    sender: &'a SenderIdentity,
    reference: &'a str,
}

async fn mint_intent_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    subject: IntentMint<'_>,
) -> AppResult<IntentIssued> {
    let controls = subject.controls;
    let initiation_ref = subject.reference;
    if scope::validate(
        &controls.entry,
        &controls.manager_bot_id,
        Some(subject.sender),
    )
    .is_err()
        || initiation_ref.is_empty()
        || initiation_ref.len() > 128
        || controls.retired
        || controls.emergency_stopped
    {
        return Err(bindings::unavailable());
    }
    let rows = db.collection::<PlatformLinkIntent>(COLLECTION_NAME);
    if let Some(_existing) = rows.find_one(doc! {"platform_config_id":&controls.platform_config_id,"manager_authority_epoch":controls.manager_authority_epoch,scope::actor_key(subject.sender,false):scope::actor_value(subject.sender),"initiation_ref":initiation_ref}).session(&mut *session).await? {
        return Ok(IntentIssued {
            intent_id: _existing.id.clone(),
            #[cfg(test)]
            intent: _existing,
            token: None,
        });
    }
    if !super::coordination_service::RateWindowStore::admit_in_session(
        db,
        "personal_intent_sender",
        &match subject.sender {
            SenderIdentity::TelegramV1 {
                telegram_sender_id, ..
            } => format!("{}:{}", controls.platform_config_id, telegram_sender_id),
            SenderIdentity::NativeV2 { .. } => format!(
                "{}:v2:{}",
                controls.platform_config_id,
                super::mcp_service::canonical_sha256(
                    serde_json::json!({"entry":controls.entry,"sender":subject.sender})
                )
            ),
        },
        10,
        std::time::Duration::from_secs(3600),
        session,
    )
    .await?
    {
        return Err(AppError::RateLimited);
    }
    let mut bytes = Zeroizing::new([0_u8; 32]);
    OsRng.fill_bytes(bytes.as_mut());
    let token = Zeroizing::new(format!("nyx_pli_{}", hex::encode(bytes.as_ref())));
    let now = controls.updated_at;
    let mut row = PlatformLinkIntent {
        id: uuid::Uuid::new_v4().to_string(),
        purpose: PURPOSE.into(),
        platform_config_id: controls.platform_config_id.clone(),
        manager_bot_id: controls.manager_bot_id.clone(),
        entry: controls.entry.clone(),
        manager_authority_epoch: controls.manager_authority_epoch,
        sender: subject.sender.clone(),
        initiation_ref: initiation_ref.into(),
        token_lookup_hmac: token_lookup(key, &token)?,
        token_hmac: String::new(),
        state: IntentState::Pending,
        claimant_user_id: None,
        current_challenge_id: None,
        completion: None,
        created_at: now,
        expires_at: now + Duration::hours(24),
        purge_after: now + Duration::days(31),
    };
    row.token_hmac = intent_hash(key, &row, &token);
    rows.insert_one(&row).session(session).await?;
    Ok(IntentIssued {
        intent_id: row.id.clone(),
        #[cfg(test)]
        intent: row,
        token: Some(token),
    })
}

pub async fn preview(db: &Database, key: &[u8], token: &str) -> AppResult<PlatformLinkIntent> {
    let row=db.collection::<PlatformLinkIntent>(COLLECTION_NAME).find_one(doc! {"token_lookup_hmac":token_lookup(key,token)?,"purpose":PURPOSE,"state":"pending","$expr":live()}).await?.ok_or_else(bindings::unavailable)?;
    if !equal(&row.token_hmac, &intent_hash(key, &row, token)) {
        return Err(bindings::unavailable());
    }
    let controls = super::platform_bot_runtime_control_service::get(db, &row.platform_config_id)
        .await?
        .ok_or_else(bindings::unavailable)?;
    if controls.retired || controls.manager_authority_epoch != row.manager_authority_epoch {
        return Err(bindings::unavailable());
    }
    Ok(row)
}

/// The committed effect identifies an intent; the ephemeral bearer must prove
/// that exact row too. Never return token material or accept a different intent.
pub(crate) async fn command_token_matches_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    intent_id: &str,
    token: &str,
) -> AppResult<bool> {
    let lookup = match token_lookup(key, token) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let Some(row) = db.collection::<PlatformLinkIntent>(COLLECTION_NAME)
        .find_one(doc! {"_id":intent_id,"token_lookup_hmac":lookup,"purpose":PURPOSE,"state":"pending","$expr":live()})
        .session(session).await? else { return Ok(false); };
    Ok(equal(&row.token_hmac, &intent_hash(key, &row, token)))
}

/// Independent of the decision transaction: rejected guesses retain their cost.
async fn admit_decision(db: &Database, actor: &str) -> AppResult<()> {
    if !super::coordination_service::RateWindowStore::admit(
        db,
        "personal_proof_decisions",
        actor,
        30,
        std::time::Duration::from_secs(3600),
    )
    .await?
    .allowed
    {
        return Err(AppError::RateLimited);
    }
    let rows = db.collection::<Document>("platform_personal_decision_spacing");
    match rows
        .update_one(
            doc! {"_id":actor},
            doc! {"$setOnInsert":{"last_decision_at":bson::DateTime::from_millis(0)}},
        )
        .upsert(true)
        .await
    {
        Ok(_) => {}
        Err(e) if super::assistant_team_service::is_duplicate(&e) => {}
        Err(e) => return Err(e.into()),
    }
    if rows.update_one(doc! {"_id":actor,"$expr":{"$lte":["$last_decision_at",{"$subtract":["$$NOW",750_i64]}]}},vec![doc! {"$set":{"last_decision_at":"$$NOW"}}]).await?.matched_count!=1 {return Err(AppError::RateLimited);}
    Ok(())
}

fn reviewed(row: &PlatformPersonalBinding) -> ReviewedPersonalBinding {
    ReviewedPersonalBinding {
        binding_id: row.id.clone(),
        generation: row.generation,
        user_id: row.user_id.clone(),
        state: row.state,
    }
}
async fn access_mode(db: &Database, actor: &str) -> AppResult<String> {
    let settings = super::assistant_settings_service::get(db, actor).await?;
    Ok(match settings.access_mode {
        crate::models::assistant_settings::AccessMode::Autopilot => "autopilot",
        crate::models::assistant_settings::AccessMode::Nyxmode => "nyxmode",
        crate::models::assistant_settings::AccessMode::AskEveryTime => "ask_every_time",
    }
    .into())
}

async fn fence_access_mode(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    expected: &str,
) -> AppResult<bool> {
    use crate::models::assistant_settings::{AccessMode, AssistantSettings, COLLECTION_NAME};
    let rows = db.collection::<Document>(COLLECTION_NAME);
    let raw = rows
        .find_one_and_update(
            doc! {"_id":actor},
            doc! {"$inc":{"personal_consent_revision":1_i64}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *session)
        .await?;
    let mode = if let Some(raw) = raw {
        let has_mode = raw.contains_key("access_mode");
        let row: AssistantSettings = bson::from_document(raw)
            .map_err(|_| AppError::Internal("Assistant settings unavailable".into()))?;
        if !has_mode && row.skip_destructive_confirmation {
            AccessMode::Autopilot
        } else {
            row.access_mode
        }
    } else {
        // Materialize existing defaults so a concurrent settings insert conflicts
        // with consent instead of changing the policy behind the review.
        db.collection::<AssistantSettings>(COLLECTION_NAME)
            .insert_one(AssistantSettings::defaults(actor))
            .session(session)
            .await?;
        AccessMode::Nyxmode
    };
    Ok(expected
        == match mode {
            AccessMode::Autopilot => "autopilot",
            AccessMode::Nyxmode => "nyxmode",
            AccessMode::AskEveryTime => "ask_every_time",
        })
}

pub async fn review(
    db: &Database,
    key: &[u8],
    actor: &str,
    token: &str,
) -> AppResult<PlatformLinkChallenge> {
    admit_decision(db, actor).await?;
    let lookup = token_lookup(key, token)?;
    let mode = access_mode(db, actor).await?;
    let db = db.clone();
    let actor = actor.to_owned();
    let key = zeroize::Zeroizing::new(key.to_vec());
    let token = Zeroizing::new(token.to_owned());
    let challenge_id = uuid::Uuid::new_v4().to_string();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result:AppResult<_>=async {
            if !bindings::fence_person(&db,session,&actor).await? {return Err(bindings::unavailable());}
            if !fence_access_mode(&db,session,&actor,&mode).await? {return Err(bindings::conflict());}
            let intents=db.collection::<PlatformLinkIntent>(COLLECTION_NAME);
            let intent=intents.find_one(doc! {"token_lookup_hmac":&lookup,"purpose":PURPOSE,"state":"pending","$expr":live(),"$or":[{"claimant_user_id":bson::Bson::Null},{"claimant_user_id":&actor}]}).session(&mut *session).await?.ok_or_else(bindings::unavailable)?;
            if !equal(&intent.token_hmac,&intent_hash(&key,&intent,&token)) {return Err(bindings::unavailable());}
            let controls=super::platform_bot_runtime_control_service::fence_manager_in_session(&db,session,&intent.platform_config_id,&intent.manager_bot_id,&intent.entry,None).await?.ok_or_else(bindings::unavailable)?;
            if controls.retired || controls.emergency_stopped || controls.manager_authority_epoch!=intent.manager_authority_epoch {return Err(bindings::unavailable());}
            let previous=bindings::latest_for_sender(&db,session,&intent.platform_config_id,intent.manager_authority_epoch,&intent.sender).await?;
            let now=controls.updated_at;
            let mut row=PlatformLinkChallenge {id:challenge_id.clone(),intent_id:intent.id.clone(),purpose:PURPOSE.into(),platform_config_id:intent.platform_config_id.clone(),manager_bot_id:intent.manager_bot_id.clone(),entry:intent.entry.clone(),manager_authority_epoch:intent.manager_authority_epoch,sender:intent.sender.clone(),reviewed_user_id:actor.clone(),reviewed_access_mode:mode.clone(),consent_version:CONSENT_VERSION,review_digest:String::new(),previous_binding:previous.as_ref().map(reviewed),state:ChallengeState::Review,code_lookup_hmac:None,code_hmac:None,completion:None,created_at:now,expires_at:std::cmp::min(now+Duration::minutes(10),intent.expires_at),purge_after:intent.purge_after};
            row.review_digest=review_hash(&key,&row)?;
            db.collection::<PlatformLinkChallenge>(CHALLENGES).update_many(doc! {"intent_id":&intent.id,"state":{"$in":["review","awaiting_chat_confirmation"]}},doc! {"$set":{"state":"cancelled"}}).session(&mut *session).await?;
            intents.update_one(doc! {"_id":&intent.id,"state":"pending"},doc! {"$set":{"claimant_user_id":&actor,"current_challenge_id":&row.id}}).session(&mut *session).await?;
            db.collection::<PlatformLinkChallenge>(CHALLENGES).insert_one(&row).session(&mut *session).await?;
            Ok(row)
        }.await;transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn approve(
    db: &Database,
    key: &[u8],
    actor: &str,
    challenge_id: &str,
    review_digest: &str,
    replace_existing: bool,
) -> AppResult<ChallengeApproved> {
    admit_decision(db, actor).await?;
    let mut bytes = Zeroizing::new([0_u8; 8]);
    OsRng.fill_bytes(bytes.as_mut());
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let chars = Zeroizing::new(
        bytes
            .iter()
            .map(|b| ALPHABET[usize::from(b & 31)] as char)
            .collect::<String>(),
    );
    let code = Zeroizing::new(format!("P-{}-{}", &chars[..4], &chars[4..]));
    let db = db.clone();
    let actor = actor.to_owned();
    let key = zeroize::Zeroizing::new(key.to_vec());
    let id = challenge_id.to_owned();
    let digest = review_digest.to_owned();
    let mode = access_mode(&db, &actor).await?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result:AppResult<_>=async {
            if !bindings::fence_person(&db,session,&actor).await? {return Err(bindings::unavailable());}
            let challenges=db.collection::<PlatformLinkChallenge>(CHALLENGES);
            let mut row=challenges.find_one(doc! {"_id":&id,"reviewed_user_id":&actor,"state":{"$in":["review","awaiting_chat_confirmation"]},"$expr":live()}).session(&mut *session).await?.ok_or_else(bindings::unavailable)?;
            if !equal(&row.review_digest,&digest) || !equal(&row.review_digest,&review_hash(&key,&row)?) || row.reviewed_access_mode!=mode {return Err(bindings::conflict());}
            if !fence_access_mode(&db,session,&actor,&row.reviewed_access_mode).await? {return Err(bindings::conflict());}
            if row.previous_binding.as_ref().is_some_and(|b|b.state==PersonalBindingState::Active && b.user_id!=actor) && !replace_existing {return Err(AppError::Conflict("Explicit account replacement acknowledgement required".into()));}
            let intent=db.collection::<PlatformLinkIntent>(COLLECTION_NAME).find_one(doc! {"_id":&row.intent_id,"claimant_user_id":&actor,"current_challenge_id":&row.id,"state":"pending","$expr":live()}).session(&mut *session).await?.ok_or_else(bindings::unavailable)?;
            let controls=super::platform_bot_runtime_control_service::fence_manager_in_session(&db,session,&row.platform_config_id,&row.manager_bot_id,&row.entry,None).await?.ok_or_else(bindings::unavailable)?;
            if controls.retired || controls.emergency_stopped || controls.manager_authority_epoch!=row.manager_authority_epoch {return Err(bindings::unavailable());}
            if bindings::latest_for_sender(&db,session,&row.platform_config_id,row.manager_authority_epoch,&row.sender).await?.as_ref().map(reviewed)!=row.previous_binding {return Err(bindings::conflict());}
            if row.state==ChallengeState::AwaitingChatConfirmation {return Ok(ChallengeApproved {challenge:row,confirmation_command:None});}
            row.state=ChallengeState::AwaitingChatConfirmation;row.code_lookup_hmac=Some(code_lookup(&key,&code)?);row.code_hmac=Some(challenge_hash(&key,&row,&code)?);
            challenges.replace_one(doc! {"_id":&row.id,"state":"review"},&row).session(&mut *session).await?;
            let _=intent;
            let command=super::platform_chat_presentation::ChatPresentation::for_entry(&row.entry)?.confirmation(code.as_str())?;
            Ok(ChallengeApproved {challenge:row,confirmation_command:Some(command)})
        }.await;transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn status(
    db: &Database,
    actor: &str,
    challenge_id: &str,
) -> AppResult<PlatformLinkChallenge> {
    let mut row = db
        .collection::<PlatformLinkChallenge>(CHALLENGES)
        .find_one(doc! {"_id":challenge_id,"reviewed_user_id":actor})
        .await?
        .ok_or_else(bindings::unavailable)?;
    if matches!(
        row.state,
        ChallengeState::Review | ChallengeState::AwaitingChatConfirmation
    ) && db
        .collection::<Document>(CHALLENGES)
        .find_one(doc! {"_id":challenge_id,"$expr":{"$lte":["$expires_at","$$NOW"]}})
        .await?
        .is_some()
    {
        row.state = ChallengeState::Expired;
    }
    Ok(row)
}

pub async fn deny(db: &Database, actor: &str, challenge_id: &str) -> AppResult<()> {
    admit_decision(db, actor).await?;
    let db = db.clone();
    let actor = actor.to_owned();
    let id = challenge_id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result:AppResult<_>=async {
            if !bindings::fence_person(&db,session,&actor).await? {return Err(bindings::unavailable());}
            let row=db.collection::<PlatformLinkChallenge>(CHALLENGES).find_one(doc! {"_id":&id,"reviewed_user_id":&actor,"state":{"$in":["review","awaiting_chat_confirmation","denied"]}}).session(&mut *session).await?.ok_or_else(bindings::unavailable)?;
            if row.state==ChallengeState::Denied {return Ok(());}
            if db.collection::<PlatformLinkIntent>(COLLECTION_NAME).update_one(doc! {"_id":&row.intent_id,"claimant_user_id":&actor,"current_challenge_id":&id,"state":"pending"},doc! {"$set":{"state":"cancelled"}}).session(&mut *session).await?.matched_count!=1 {return Err(bindings::conflict());}
            db.collection::<PlatformLinkChallenge>(CHALLENGES).update_one(doc! {"_id":&id},doc! {"$set":{"state":"denied"}}).session(&mut *session).await?;
            Ok(())
        }.await;transaction_result(result)
    }).await.map_err(map_transaction_error)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Frozen transaction seam carries independent proof and lifecycle authorities"
)]
pub async fn confirm_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    audit_key: &[u8],
    update: &VerifiedManagerUpdate,
    forwarded: bool,
    code: &str,
    effects: &dyn bindings::PersonalLifecycleEffects,
) -> AppResult<PersonalBindingCompletion> {
    proof_command(update, forwarded)?;
    // Separate session: a failed/aborted proof transaction must not refund guesses.
    // The key is authenticated sender context, never the guessed code or account.
    admit_decision(db, &confirmation_rate_key(update)).await?;
    let mut filter = doc! {"code_lookup_hmac":code_lookup(key,code)?,"purpose":PURPOSE,"platform_config_id":&update.profile_id,"manager_bot_id":&update.manager_bot_id};
    filter.extend(scope::entry_fields(&update.entry));
    filter.extend(scope::sender_fields(&update.sender, false));
    scope::exclude_other_version(&mut filter, &update.entry);
    let mut row = db
        .collection::<PlatformLinkChallenge>(CHALLENGES)
        .find_one(filter)
        .session(&mut *session)
        .await?
        .ok_or_else(bindings::unavailable)?;
    let expected_code_hmac = challenge_hash(key, &row, code)?;
    if !row
        .code_hmac
        .as_deref()
        .is_some_and(|h| equal(h, &expected_code_hmac))
    {
        return Err(bindings::unavailable());
    }
    let controls = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &row.platform_config_id,
        &row.manager_bot_id,
        &row.entry,
        Some(&update.observation_id),
    )
    .await?
    .ok_or_else(bindings::unavailable)?;
    if row.state == ChallengeState::Confirmed {
        if controls.retired
            || controls.emergency_stopped
            || !controls.personal_enabled
            || controls.manager_authority_epoch != row.manager_authority_epoch
            || !bindings::fence_person(db, session, &row.reviewed_user_id).await?
            || !fence_access_mode(
                db,
                session,
                &row.reviewed_user_id,
                &row.reviewed_access_mode,
            )
            .await?
        {
            return Err(bindings::unavailable());
        }
        let completion = row.completion.as_ref().ok_or_else(bindings::unavailable)?;
        let binding = bindings::latest_for_sender(
            db,
            session,
            &row.platform_config_id,
            row.manager_authority_epoch,
            &row.sender,
        )
        .await?
        .ok_or_else(bindings::unavailable)?;
        if binding.state != PersonalBindingState::Active
            || binding.id != completion.binding_id
            || binding.generation != completion.binding_generation
            || binding.user_id != row.reviewed_user_id
            || binding.sender != row.sender
        {
            return Err(bindings::unavailable());
        }
        return Ok(completion.clone());
    }
    if controls.retired
        || controls.emergency_stopped
        || !controls.personal_enabled
        || controls.manager_authority_epoch != row.manager_authority_epoch
        || row.state != ChallengeState::AwaitingChatConfirmation
        || row.expires_at <= controls.updated_at
        || !fence_access_mode(
            db,
            session,
            &row.reviewed_user_id,
            &row.reviewed_access_mode,
        )
        .await?
        || !bindings::fence_person(db, session, &row.reviewed_user_id).await?
    {
        return Err(bindings::unavailable());
    }
    let intent=db.collection::<PlatformLinkIntent>(COLLECTION_NAME).find_one(doc! {"_id":&row.intent_id,"purpose":PURPOSE,"claimant_user_id":&row.reviewed_user_id,"current_challenge_id":&row.id,"state":"pending","$expr":live()}).session(&mut *session).await?.ok_or_else(bindings::unavailable)?;
    let previous = bindings::latest_for_sender(
        db,
        session,
        &row.platform_config_id,
        row.manager_authority_epoch,
        &row.sender,
    )
    .await?;
    if previous.as_ref().map(reviewed) != row.previous_binding {
        return Err(bindings::conflict());
    }
    let now = controls.updated_at;
    let binding = if let Some(previous) = previous
        .as_ref()
        .filter(|b| b.state == PersonalBindingState::Active && b.user_id == row.reviewed_user_id)
    {
        previous.clone()
    } else {
        if let Some(previous) = &previous {
            bindings::revoke_binding_in_session(
                db,
                session,
                previous,
                PersonalRevocationReason::Replaced,
                effects,
                Some(&intent.id),
            )
            .await?;
        }
        let binding = PlatformPersonalBinding {
            id: uuid::Uuid::new_v4().to_string(),
            platform_config_id: row.platform_config_id.clone(),
            manager_bot_id: row.manager_bot_id.clone(),
            entry: row.entry.clone(),
            manager_authority_epoch: row.manager_authority_epoch,
            sender: row.sender.clone(),
            user_id: row.reviewed_user_id.clone(),
            generation: previous
                .as_ref()
                .map_or(Some(1), |b| b.generation.checked_add(1))
                .ok_or_else(bindings::conflict)?,
            state: PersonalBindingState::Active,
            revocation_reason: None,
            consent_version: row.consent_version,
            review_digest: row.review_digest.clone(),
            reviewed_access_mode: row.reviewed_access_mode.clone(),
            nyxbot_agent_id: None,
            conversation_id: None,
            active_turn: None,
            effect_revision: 0,
            created_at: now,
            consented_at: now,
            updated_at: now,
            revoked_at: None,
            logged_out_at: None,
        };
        db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&binding)
        .session(&mut *session)
        .await?;
        binding
    };
    effects.revoke_public_sender(db, session, &binding).await?;
    bindings::cancel_sender_proofs_in_session(db, session, &binding, Some(&intent.id)).await?;
    let completion = PersonalBindingCompletion {
        binding_id: binding.id.clone(),
        binding_generation: binding.generation,
    };
    row.state = ChallengeState::Confirmed;
    row.completion = Some(completion.clone());
    db.collection::<PlatformLinkChallenge>(CHALLENGES)
        .replace_one(
            doc! {"_id":&row.id,"state":"awaiting_chat_confirmation"},
            &row,
        )
        .session(&mut *session)
        .await?;
    db.collection::<PlatformLinkIntent>(COLLECTION_NAME).update_one(doc! {"_id":&intent.id,"state":"pending"},doc! {"$set":{"state":"consumed","completion":bson::to_bson(&completion).map_err(|_|bindings::conflict())?}}).session(&mut *session).await?;
    let actor = super::audit_service::AuditActor {
        user_id: binding.user_id.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    super::audit_service::log_actor_event_in_session(db,session,audit_key,&actor,"platform_personal_consent_confirmed",serde_json::json!({"binding_id":binding.id,"generation":binding.generation,"consent_version":binding.consent_version})).await?;
    Ok(completion)
}

/// Native transport entry points require original-private command provenance.
/// The transport owns the ingress transaction and must preserve its dedupe claim.
pub async fn begin_original_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    command: &super::platform_manager_contract::OriginalManagerCommand,
    initiation_ref: &str,
) -> AppResult<IntentIssued> {
    begin_in_session(db, session, key, command.update(), false, initiation_ref).await
}

pub async fn confirm_original_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    audit_key: &[u8],
    command: &super::platform_manager_contract::OriginalManagerCommand,
    effects: &dyn bindings::PersonalLifecycleEffects,
) -> AppResult<PersonalBindingCompletion> {
    let mut words = command.text().split_whitespace();
    if words
        .next()
        .is_none_or(|w| w.split('@').next() != Some("/confirm"))
    {
        return Err(bindings::unavailable());
    }
    let code = words.next().ok_or_else(bindings::unavailable)?;
    if words.next().is_some() {
        return Err(bindings::unavailable());
    }
    confirm_in_session(
        db,
        session,
        key,
        audit_key,
        command.update(),
        false,
        code,
        effects,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::platform_personal_binding_service::tests::{binding, fixture};
    const KEY: &[u8] = b"personal-proof-test-key";

    async fn issue(db: &Database, update: &VerifiedManagerUpdate, reference: &str) -> IntentIssued {
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let issued = begin_in_session(db, &mut session, KEY, update, false, reference)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        issued
    }
    async fn elapsed_spacing(db: &Database, owner: &str) {
        db.collection::<Document>("platform_personal_decision_spacing")
            .update_one(
                doc! {"_id":owner},
                doc! {"$set":{"last_decision_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn long_signup_gets_fresh_bounded_challenge_without_agent_or_raw_code_storage() {
        let (db, update, owner) = fixture("personal_long_signup").await;
        let issued = issue(&db, &update, "update:1").await;
        let token = issued.token.unwrap();
        db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&issued.intent.id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(20))}}).await.unwrap();
        let challenge = review(&db, KEY, &owner, &token).await.unwrap();
        assert!(challenge.expires_at > Utc::now() + Duration::minutes(9));
        assert!(challenge.expires_at <= challenge.created_at + Duration::minutes(10));
        assert!(challenge.expires_at <= issued.intent.expires_at);
        assert_eq!(
            db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        elapsed_spacing(&db, &owner).await;
        let approved = approve(
            &db,
            KEY,
            &owner,
            &challenge.id,
            &challenge.review_digest,
            false,
        )
        .await
        .unwrap();
        let command = approved.confirmation_command.unwrap();
        assert!(command.starts_with("/confirm P-"));
        let stored = db
            .collection::<Document>(CHALLENGES)
            .find_one(doc! {"_id":&challenge.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!format!("{stored:?}").contains(command.as_str()));
        elapsed_spacing(&db, &owner).await;
        assert!(
            approve(
                &db,
                KEY,
                &owner,
                &challenge.id,
                &challenge.review_digest,
                false
            )
            .await
            .unwrap()
            .confirmation_command
            .is_none()
        );
        assert_eq!(
            status(&db, &owner, &challenge.id).await.unwrap().state,
            ChallengeState::AwaitingChatConfirmation
        );
        let duplicate = issue(&db, &update, "update:1").await;
        assert!(duplicate.token.is_none());
        assert_eq!(duplicate.intent.id, issued.intent.id);
        db.drop().await.unwrap();
    }

    #[test]
    fn manual_codes_are_purpose_specific_and_context_bound() {
        assert!(normalize_code("1234-5678").is_err());
        assert!(normalize_code("P-1234-5678").is_ok());
        assert!(normalize_code("P-OOOO-OOOO").is_err());
        let personal = code_lookup(KEY, "P-1234-5678").unwrap();
        let onboarding =
            crate::services::platform_binding_service::code_hash(KEY, "1234-5678").unwrap();
        assert_ne!(personal, onboarding);
        assert_ne!(
            hash(KEY, "platform-personal-intent:v1", &["a", "bc"]),
            hash(KEY, "platform-personal-intent:v1", &["ab", "c"])
        );
    }

    #[tokio::test]
    async fn fresh_review_cancels_old_challenge_and_foreign_account_cannot_claim() {
        let (db, update, owner) = fixture("personal_fresh_review").await;
        let issued = issue(&db, &update, "update:2").await;
        let token = issued.token.unwrap();
        let first = review(&db, KEY, &owner, &token).await.unwrap();
        elapsed_spacing(&db, &owner).await;
        let next = review(&db, KEY, &owner, &token).await.unwrap();
        assert_ne!(first.id, next.id);
        assert_eq!(
            status(&db, &owner, &first.id).await.unwrap().state,
            ChallengeState::Cancelled
        );
        let other = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &other,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        assert!(review(&db, KEY, &other, &token).await.is_err());
        elapsed_spacing(&db, &owner).await;
        assert!(deny(&db, &owner, &first.id).await.is_err());
        let current = db
            .collection::<PlatformLinkIntent>(COLLECTION_NAME)
            .find_one(doc! {"_id":&issued.intent.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.state, IntentState::Pending);
        assert_eq!(
            current.current_challenge_id.as_deref(),
            Some(next.id.as_str())
        );
        db.drop().await.unwrap();
    }

    struct PublicUnavailable;
    #[async_trait::async_trait]
    impl bindings::PersonalLifecycleEffects for PublicUnavailable {
        async fn capture_stop(
            &self,
            _: &Database,
            _: &mut ClientSession,
            active: &crate::models::platform_personal_binding::PersonalActiveTurn,
        ) -> AppResult<Option<bindings::CapturedPersonalStop>> {
            Ok(Some(bindings::CapturedPersonalStop {
                active_turn: active.clone(),
                stop_requested_at: Utc::now(),
                machine_jobs: vec![],
                machine_scopes: vec![
                    crate::models::platform_bot_runtime_controls::ExactMachineScope {
                        node_id: "test-node".into(),
                        runtime_id: "captured-runtime".into(),
                    },
                ],
            }))
        }
        async fn revoke_public_sender(
            &self,
            _: &Database,
            _: &mut ClientSession,
            _: &PlatformPersonalBinding,
        ) -> AppResult<()> {
            Err(AppError::Conflict(
                "injected public revocation failure".into(),
            ))
        }
    }
    struct LocalEffects;
    #[async_trait::async_trait]
    impl bindings::PersonalLifecycleEffects for LocalEffects {
        async fn capture_stop(
            &self,
            _: &Database,
            _: &mut ClientSession,
            active: &crate::models::platform_personal_binding::PersonalActiveTurn,
        ) -> AppResult<Option<bindings::CapturedPersonalStop>> {
            Ok(Some(bindings::CapturedPersonalStop {
                active_turn: active.clone(),
                stop_requested_at: Utc::now(),
                machine_jobs: vec![],
                machine_scopes: vec![
                    crate::models::platform_bot_runtime_controls::ExactMachineScope {
                        node_id: "test-node".into(),
                        runtime_id: "captured-runtime".into(),
                    },
                ],
            }))
        }
        async fn revoke_public_sender(
            &self,
            _: &Database,
            _: &mut ClientSession,
            _: &PlatformPersonalBinding,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn replacement_requires_ack_and_public_revoke_failure_rolls_back_all_consent_writes() {
        let (db, update, owner) = fixture("personal_replace_atomic").await;
        let other = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &other,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let mut old = binding(&update, &other);
        let agent = uuid::Uuid::new_v4().to_string();
        let origin = super::bindings::stored_origin(
            &crate::services::platform_manager_contract::PlatformTurnOrigin {
                authority: super::bindings::authority(&old, 1),
                nyxbot_agent_id: agent.clone(),
                source: crate::services::platform_manager_contract::PlatformOriginSource::Ingress(
                    crate::services::platform_manager_contract::IngressClaim {
                        receipt_id: "old-receipt".into(),
                        claim_id: "old-claim".into(),
                    },
                ),
            },
        );
        old.nyxbot_agent_id = Some(agent);
        old.conversation_id = Some("old-thread".into());
        old.active_turn = Some(
            crate::models::platform_personal_binding::PersonalActiveTurn {
                conversation_id: "old-thread".into(),
                turn_id: "old-turn".into(),
                origin,
            },
        );
        db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&old)
        .await
        .unwrap();
        let issued = issue(&db, &update, "update:3").await;
        let reviewed = review(&db, KEY, &owner, &issued.token.unwrap())
            .await
            .unwrap();
        elapsed_spacing(&db, &owner).await;
        assert!(
            approve(
                &db,
                KEY,
                &owner,
                &reviewed.id,
                &reviewed.review_digest,
                false
            )
            .await
            .is_err()
        );
        elapsed_spacing(&db, &owner).await;
        let command = approve(
            &db,
            KEY,
            &owner,
            &reviewed.id,
            &reviewed.review_digest,
            true,
        )
        .await
        .unwrap()
        .confirmation_command
        .unwrap();
        let code = command.strip_prefix("/confirm ").unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            confirm_in_session(
                &db,
                &mut session,
                KEY,
                b"audit-test-key",
                &update,
                false,
                code,
                &PublicUnavailable
            )
            .await
            .is_err()
        );
        session.abort_transaction().await.unwrap();
        elapsed_spacing(
            &db,
            &format!(
                "confirm:{}:{}:{}",
                update.profile_id,
                crate::services::platform_entry_scope::legacy_sender_id(&update.sender),
                crate::services::platform_entry_scope::legacy_chat_id(&update.sender)
            ),
        )
        .await;
        let stop_rows = db
            .collection::<crate::models::platform_bot_runtime_controls::PlatformPersonalStopEffect>(
                crate::models::platform_bot_runtime_controls::STOP_EFFECTS,
            );
        assert_eq!(stop_rows.count_documents(doc! {}).await.unwrap(), 0);
        let rows = db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        );
        assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 1);
        assert_eq!(
            rows.find_one(doc! {"_id":&old.id})
                .await
                .unwrap()
                .unwrap()
                .state,
            PersonalBindingState::Active
        );
        assert_eq!(
            status(&db, &owner, &reviewed.id).await.unwrap().state,
            ChallengeState::AwaitingChatConfirmation
        );
        session.start_transaction().await.unwrap();
        let completed = confirm_in_session(
            &db,
            &mut session,
            KEY,
            b"audit-test-key",
            &update,
            false,
            code,
            &LocalEffects,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        assert_ne!(completed.binding_id, old.id);
        assert_eq!(completed.binding_generation, 2);
        let stop = stop_rows.find_one(doc! {}).await.unwrap().unwrap();
        assert_eq!(stop.active_turn, old.active_turn.clone().unwrap());
        assert!(!stop.completed);
        assert_eq!(stop_rows.count_documents(doc! {}).await.unwrap(), 1);
        elapsed_spacing(
            &db,
            &format!(
                "confirm:{}:{}:{}",
                update.profile_id,
                crate::services::platform_entry_scope::legacy_sender_id(&update.sender),
                crate::services::platform_entry_scope::legacy_chat_id(&update.sender)
            ),
        )
        .await;
        session.start_transaction().await.unwrap();
        let replay = confirm_in_session(
            &db,
            &mut session,
            KEY,
            b"audit-test-key",
            &update,
            false,
            code,
            &LocalEffects,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        assert_eq!(replay, completed);
        assert_eq!(
            rows.count_documents(doc! {"state":"active"}).await.unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    async fn public_fixture(
        name: &str,
    ) -> (Database, VerifiedManagerUpdate, PublicExecutionContext) {
        use crate::models::{
            credits::Credits,
            platform_bot_runtime_controls::PublicRuntimePolicy,
            platform_manager_ingress::{COLLECTION_NAME as RECEIPTS, IngressExecution},
            platform_public_session::{COLLECTION_NAME as PUBLIC_SESSIONS, PlatformPublicSession},
        };
        use crate::services::platform_manager_contract::*;
        let (db, mut update, _) = fixture(name).await;
        update.command_eligibility = ManagerCommandEligibility::Ineligible;
        super::super::platform_public_agent_service::ensure_indexes(&db)
            .await
            .unwrap();
        super::super::platform_manager_ingress_service::ensure_indexes(&db)
            .await
            .unwrap();
        let mut public = super::super::platform_public_agent_service::tests::fixture(
            &db,
            crate::services::platform_entry_scope::legacy_sender_id(&update.sender),
            false,
        )
        .await;
        public.profile_id = update.profile_id.clone();
        public.manager_bot_id = update.manager_bot_id.clone();
        public.entry = update.entry.clone();
        public.manager_authority_epoch = 1;
        db.collection::<PlatformPublicSession>(PUBLIC_SESSIONS)
            .replace_one(doc! {"_id": &public.id}, &public)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .insert_one(
                doc! {"_id": &public.sponsor_owner_user_id, "user_type": "org", "is_active": true},
            )
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id": &update.profile_id},
                doc! {"$set": {
                "public_enabled": true, "personal_enabled": true,
                "public_policy": bson::to_bson(&PublicRuntimePolicy {
                    sponsor_owner_user_id: public.sponsor_owner_user_id.clone(),
                    nyxagent_profile: public.nyxagent_profile.clone(),
                    total_daily_credits: Credits::from_pico(10).unwrap(),
                    sender_daily_credits: Credits::from_pico(10).unwrap(),
                }).unwrap()}},
            )
            .await
            .unwrap();
        let turn = public.active_turn.as_ref().unwrap();
        let context = PublicExecutionContext {
            proof_record_id: "test-proof".into(),
            ingress: PublicIngress {
                ingress: IngressClaim {
                    receipt_id: turn.receipt_id.clone(),
                    claim_id: turn.claim_id.clone(),
                },
                manager: ManagerAuthority {
                    profile_id: public.profile_id.clone(),
                    manager_bot_id: public.manager_bot_id.clone(),
                    entry: public.entry.clone(),
                    manager_authority_epoch: public.manager_authority_epoch,
                },
                sender: public.sender.clone(),
                public_policy_revision: public.public_policy_revision,
            },
            public_session_id: public.id.clone(),
            turn: TurnRef {
                conversation_id: turn.conversation_id.clone(),
                turn_id: turn.turn_id.clone(),
            },
            sponsor_owner_user_id: public.sponsor_owner_user_id.clone(),
            policy_digest: public.policy_digest.clone(),
            nyxagent_profile: public.nyxagent_profile.clone(),
            model_operations: vec![],
            limits: PublicExecutionLimits {
                max_duration_secs: 60,
                max_model_calls: 2,
                max_tool_calls: 3,
                max_continuations: 0,
                max_input_tokens: 20,
                max_output_tokens: 10,
                max_input_bytes: 100,
                max_context_bytes: 100,
                max_output_bytes: 100,
            },
            expires_at: turn.expires_at,
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        super::super::platform_manager_ingress_service::record_admission_in_session(
            &db,
            &mut session,
            &update,
            &context.ingress.manager,
            &context.ingress.ingress,
            &ManagerAudience::Public(context.ingress.clone()),
        )
        .await
        .unwrap();
        db.collection::<Document>(RECEIPTS).update_one(doc! {"_id": &turn.receipt_id}, doc! {"$set": {
            "phase": "executing", "execution": bson::to_bson(&IngressExecution {
                conversation_id: turn.conversation_id.clone(), turn_id: turn.turn_id.clone(),
                public_session_id: Some(public.id.clone()), payer_user_id: public.sponsor_owner_user_id.clone(),
                public_policy_digest: Some(public.policy_digest.clone()),
            }).unwrap()}}).session(&mut session).await.unwrap();
        session.commit_transaction().await.unwrap();
        (db, update, context)
    }

    async fn issue_public(db: &Database, context: &PublicExecutionContext) -> IntentIssued {
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let issued = begin_public_in_session(db, &mut session, KEY, context)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        issued
    }

    async fn refused_public(db: &Database, context: &PublicExecutionContext) {
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            begin_public_in_session(db, &mut session, KEY, context)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn public_signup_requires_personal_linking_to_be_enabled() {
        let (db, _, context) = public_fixture("public_signup_personal_paused").await;
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&context.ingress.manager.profile_id},
                doc! {"$set":{"personal_enabled":false}},
            )
            .await
            .unwrap();
        refused_public(&db, &context).await;
    }

    #[tokio::test]
    async fn public_forwarded_signup_issues_only_an_intent_and_preserves_command_provenance() {
        let (db, update, context) = public_fixture("public_signup_forwarded").await;
        let issued = issue_public(&db, &context).await;
        let token = issued.token.as_ref().unwrap();
        assert_eq!(
            crate::services::platform_entry_scope::legacy_sender_id(&issued.intent.sender),
            crate::services::platform_entry_scope::legacy_sender_id(&update.sender)
        );
        assert_eq!(
            crate::services::platform_entry_scope::legacy_chat_id(&issued.intent.sender),
            crate::services::platform_entry_scope::legacy_chat_id(&update.sender)
        );
        assert_eq!(
            issued.intent.manager_authority_epoch,
            context.ingress.manager.manager_authority_epoch
        );
        assert_eq!(
            issued.intent.expires_at - issued.intent.created_at,
            Duration::hours(24)
        );
        assert_eq!(issued.intent.state, IntentState::Pending);
        assert!(issued.intent.claimant_user_id.is_none());
        assert!(issued.intent.completion.is_none());
        assert!(issued.intent.current_challenge_id.is_none());
        let stored = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &issued.intent.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!format!("{stored:?}").contains(token.as_str()));
        let receipt = db
            .collection::<Document>(crate::models::platform_manager_ingress::COLLECTION_NAME)
            .find_one(doc! {"_id": &context.ingress.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert!(!receipt.get_bool("original_private_command").unwrap());
        assert_eq!(
            receipt
                .get_document("audience")
                .unwrap()
                .get_str("kind")
                .unwrap(),
            "public"
        );
        for collection in [
            CHALLENGES,
            crate::models::platform_personal_binding::COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        // Neither the public path nor the presence of a valid intent upgrades a forwarded update.
        assert!(matches!(
            begin_in_session(&db, &mut session, KEY, &update, false, "original-ref").await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            confirm_in_session(
                &db,
                &mut session,
                KEY,
                KEY,
                &update,
                false,
                "P-1234-5678",
                &LocalEffects
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        let mut original = update.clone();
        original.command_eligibility = super::super::platform_manager_contract::ManagerCommandEligibility::OriginalPrivateMessage;
        assert!(matches!(
            begin_in_session(&db, &mut session, KEY, &original, true, "forwarded-ref").await,
            Err(AppError::Forbidden(_))
        ));
        session.abort_transaction().await.unwrap();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn public_intent_and_sender_rate_rollback_with_caller_and_duplicate_returns_no_token() {
        let (db, _, context) = public_fixture("public_signup_transaction").await;
        let rates =
            db.collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME);
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let rolled_back = begin_public_in_session(&db, &mut session, KEY, &context)
            .await
            .unwrap();
        let old_token = rolled_back.token.unwrap();
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            rates
                .count_documents(doc! {"namespace": "personal_intent_sender"})
                .await
                .unwrap(),
            0
        );
        assert!(preview(&db, KEY, &old_token).await.is_err());
        let issued = issue_public(&db, &context).await;
        assert_ne!(issued.intent.id, rolled_back.intent.id);
        let duplicate = issue_public(&db, &context).await;
        assert_eq!(duplicate.intent.id, issued.intent.id);
        assert!(duplicate.token.is_none());
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            rates
                .find_one(doc! {"namespace": "personal_intent_sender"})
                .await
                .unwrap()
                .unwrap()
                .get_i64("count")
                .unwrap(),
            1
        );
        assert_eq!(
            preview(&db, KEY, issued.token.as_ref().unwrap())
                .await
                .unwrap()
                .id,
            issued.intent.id
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn public_delivery_intent_fence_rolls_back_and_preserves_pending_consent() {
        let (db, _, context) = public_fixture("public_delivery_intent_transaction").await;
        let issued = issue_public(&db, &context).await;
        let token = issued.token.as_ref().unwrap();
        let rows = db.collection::<Document>(COLLECTION_NAME);
        let filter = doc! {"_id": &issued.intent.id};
        let original = rows.find_one(filter.clone()).await.unwrap().unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert_eq!(
            fence_public_intent_in_session(&db, &mut session, KEY, &context, token)
                .await
                .unwrap(),
            issued.intent.id
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(
            rows.find_one(filter.clone()).await.unwrap().unwrap(),
            original
        );
        session.start_transaction().await.unwrap();
        assert_eq!(
            fence_public_intent_in_session(&db, &mut session, KEY, &context, token)
                .await
                .unwrap(),
            issued.intent.id
        );
        session.commit_transaction().await.unwrap();
        let stored = rows.find_one(filter).await.unwrap().unwrap();
        assert_eq!(stored.get_str("state").unwrap(), "pending");
        assert_eq!(stored.get_i64("public_delivery_fence").unwrap(), 1);
        assert_eq!(stored.get("claimant_user_id"), Some(&bson::Bson::Null));
        assert_eq!(stored.get("current_challenge_id"), Some(&bson::Bson::Null));
        assert_eq!(stored.get("completion"), Some(&bson::Bson::Null));
        assert!(!format!("{stored:?}").contains(token.as_str()));
        db.drop().await.unwrap();
    }

    async fn refused_public_intent(db: &Database, context: &PublicExecutionContext, token: &str) {
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            fence_public_intent_in_session(db, &mut session, KEY, context, token)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
    }

    #[tokio::test]
    async fn public_delivery_intent_fence_rejects_foreign_or_nonpending_bearer_and_stale_context() {
        use crate::models::{
            platform_manager_ingress::COLLECTION_NAME as RECEIPTS,
            platform_public_session::COLLECTION_NAME as SESSIONS,
        };
        let (db, _, context) = public_fixture("public_delivery_intent_identity").await;
        let issued = issue_public(&db, &context).await;
        let token = issued.token.as_ref().unwrap();
        let filter = doc! {"_id": &issued.intent.id};
        let intents = db.collection::<Document>(COLLECTION_NAME);
        let original = intents.find_one(filter.clone()).await.unwrap().unwrap();
        for changes in [
            doc! {"purpose": "another-purpose"},
            doc! {"platform_config_id": "another-profile"},
            doc! {"manager_bot_id": "another-manager"},
            doc! {"telegram_bot_id": crate::services::platform_entry_scope::legacy_bot_id(&context.ingress.manager.entry) + 1},
            doc! {"manager_authority_epoch": context.ingress.manager.manager_authority_epoch + 1},
            doc! {"telegram_sender_id": crate::services::platform_entry_scope::legacy_sender_id(&context.ingress.sender) + 1},
            doc! {"telegram_chat_id": crate::services::platform_entry_scope::legacy_chat_id(&context.ingress.sender) + 1},
            doc! {"initiation_ref": "ordinary-original-command"},
            doc! {"token_lookup_hmac": "invalid-lookup"},
            doc! {"token_hmac": "invalid-proof"},
            doc! {"state": "cancelled"},
            doc! {"state": "consumed"},
            doc! {"expires_at": bson::DateTime::from_millis(0)},
        ] {
            intents
                .update_one(filter.clone(), doc! {"$set": changes})
                .await
                .unwrap();
            refused_public_intent(&db, &context, token).await;
            intents
                .replace_one(filter.clone(), &original)
                .await
                .unwrap();
        }
        refused_public_intent(&db, &context, "invalid-bearer").await;
        let mut stale = context.clone();
        stale.turn.turn_id = "another-turn".into();
        refused_public_intent(&db, &stale, token).await;
        for (collection, row_filter, changes) in [
            (
                RECEIPTS,
                doc! {"_id": &context.ingress.ingress.receipt_id},
                doc! {"phase": "settled"},
            ),
            (
                RECEIPTS,
                doc! {"_id": &context.ingress.ingress.receipt_id},
                doc! {"observation_id": "stale-observation"},
            ),
            (
                SESSIONS,
                doc! {"_id": &context.public_session_id},
                doc! {"revoked_at": bson::DateTime::now()},
            ),
            (
                SESSIONS,
                doc! {"_id": &context.public_session_id},
                doc! {"active_turn.stop_requested": true},
            ),
        ] {
            let rows = db.collection::<Document>(collection);
            let before = rows.find_one(row_filter.clone()).await.unwrap().unwrap();
            rows.update_one(row_filter.clone(), doc! {"$set": changes})
                .await
                .unwrap();
            refused_public_intent(&db, &context, token).await;
            rows.replace_one(row_filter, &before).await.unwrap();
        }
        assert_eq!(intents.find_one(filter).await.unwrap().unwrap(), original);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn public_delivery_intent_fence_conflicts_with_cancellation_after_snapshot() {
        let (db, _, context) = public_fixture("public_delivery_intent_cancel_race").await;
        let issued = issue_public(&db, &context).await;
        let token = issued.token.as_ref().unwrap();
        let rows = db.collection::<Document>(COLLECTION_NAME);
        let filter = doc! {"_id": &issued.intent.id};
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            rows.find_one(filter.clone())
                .session(&mut session)
                .await
                .unwrap()
                .is_some()
        );
        rows.update_one(filter.clone(), doc! {"$set": {"state": "cancelled"}})
            .await
            .unwrap();
        assert!(
            fence_public_intent_in_session(&db, &mut session, KEY, &context, token)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        let current = rows.find_one(filter).await.unwrap().unwrap();
        assert_eq!(current.get_str("state").unwrap(), "cancelled");
        assert!(!current.contains_key("public_delivery_fence"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn public_intent_rejects_substituted_context_and_receipt_identity() {
        let (db, _, context) = public_fixture("public_signup_identity").await;
        let mut cases = Vec::new();
        let mut changed = context.clone();
        changed.ingress.ingress.claim_id = "wrong-claim".into();
        cases.push(changed);
        let mut changed = context.clone();
        changed.ingress.ingress.receipt_id = "wrong-receipt".into();
        cases.push(changed);
        let mut changed = context.clone();
        changed.public_session_id = "wrong-session".into();
        cases.push(changed);
        let mut changed = context.clone();
        changed.turn.turn_id = "wrong-turn".into();
        cases.push(changed);
        let mut changed = context.clone();
        changed.turn.conversation_id = "wrong-conversation".into();
        cases.push(changed);
        let mut changed = context.clone();
        if let SenderIdentity::TelegramV1 {
            telegram_sender_id,
            telegram_chat_id,
        } = &mut changed.ingress.sender
        {
            *telegram_sender_id += 1;
            *telegram_chat_id += 1;
        } else {
            panic!("legacy fixture");
        }
        cases.push(changed);
        let mut changed = context.clone();
        changed.ingress.manager.profile_id = "wrong-profile".into();
        cases.push(changed);
        let mut changed = context.clone();
        changed.expires_at = Utc::now() - Duration::seconds(1);
        cases.push(changed);
        let mut changed = context.clone();
        changed.expires_at += Duration::days(1);
        cases.push(changed);
        for changed in cases {
            refused_public(&db, &changed).await;
        }
        let receipts =
            db.collection::<Document>(crate::models::platform_manager_ingress::COLLECTION_NAME);
        let filter = doc! {"_id": &context.ingress.ingress.receipt_id};
        let original = receipts.find_one(filter.clone()).await.unwrap().unwrap();
        for changes in [
            doc! {"sender_id": crate::services::platform_entry_scope::legacy_sender_id(&context.ingress.sender) + 1},
            doc! {"chat_id": -123_i64},
            doc! {"manager_bot_id": "different-manager"},
            doc! {"telegram_bot_id": crate::services::platform_entry_scope::legacy_bot_id(&context.ingress.manager.entry) + 1},
            doc! {"manager_authority_epoch": context.ingress.manager.manager_authority_epoch + 1},
            doc! {"audience.kind": "control"},
            doc! {"phase": "settled"},
            doc! {"audience.public_policy_revision": context.ingress.public_policy_revision + 1},
            doc! {"execution.public_session_id": "different-session"},
            doc! {"execution.payer_user_id": "different-sponsor"},
            doc! {"observation_id": "stale-observation"},
        ] {
            receipts
                .update_one(filter.clone(), doc! {"$set": changes})
                .await
                .unwrap();
            refused_public(&db, &context).await;
            receipts
                .replace_one(filter.clone(), &original)
                .await
                .unwrap();
        }
        // Even mutually matching forged zero sender IDs cannot establish a private sender.
        receipts
            .update_one(
                filter.clone(),
                doc! {"$set": {"sender_id": 0_i64, "chat_id": 0_i64}},
            )
            .await
            .unwrap();
        let mut zero = context.clone();
        zero.ingress.sender = SenderIdentity::TelegramV1 {
            telegram_sender_id: 0,
            telegram_chat_id: 0,
        };
        refused_public(&db, &zero).await;
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn public_intent_obeys_live_session_controls_sponsor_key_and_revocation_scopes() {
        use crate::models::{
            platform_bot_runtime_controls::COLLECTION_NAME as CONTROLS,
            platform_public_session::COLLECTION_NAME as PUBLIC_SESSIONS,
        };
        use crate::services::platform_manager_contract::{
            PublicRevocationReason, PublicRevocationScope,
        };
        let (db, _, context) = public_fixture("public_signup_fences").await;
        let sessions = db.collection::<Document>(PUBLIC_SESSIONS);
        let session_filter = doc! {"_id": &context.public_session_id};
        let original = sessions
            .find_one(session_filter.clone())
            .await
            .unwrap()
            .unwrap();
        let key_id = original.get_str("credential_api_key_id").unwrap();
        for (collection, filter, changes) in [
            (
                PUBLIC_SESSIONS,
                session_filter.clone(),
                doc! {"expires_at": bson::DateTime::from_millis(0)},
            ),
            (
                PUBLIC_SESSIONS,
                session_filter.clone(),
                doc! {"revoked_at": bson::DateTime::now()},
            ),
            (
                PUBLIC_SESSIONS,
                session_filter.clone(),
                doc! {"active_turn.stop_requested": true},
            ),
            (
                PUBLIC_SESSIONS,
                session_filter.clone(),
                doc! {"active_turn.expires_at": bson::DateTime::from_millis(0)},
            ),
            (
                PUBLIC_SESSIONS,
                session_filter.clone(),
                doc! {"sender_id": crate::services::platform_entry_scope::legacy_sender_id(&context.ingress.sender) + 1},
            ),
            (
                PUBLIC_SESSIONS,
                session_filter.clone(),
                doc! {"active_turn.claim_id": "different-claim"},
            ),
            (
                CONTROLS,
                doc! {"platform_config_id": &context.ingress.manager.profile_id},
                doc! {"public_enabled": false},
            ),
            (
                CONTROLS,
                doc! {"platform_config_id": &context.ingress.manager.profile_id},
                doc! {"emergency_stopped": true},
            ),
            (
                CONTROLS,
                doc! {"platform_config_id": &context.ingress.manager.profile_id},
                doc! {"manager_authority_epoch": 2_i64},
            ),
            (
                crate::models::user::COLLECTION_NAME,
                doc! {"_id": &context.sponsor_owner_user_id},
                doc! {"is_active": false},
            ),
            (
                crate::models::api_key::COLLECTION_NAME,
                doc! {"_id": key_id},
                doc! {"is_active": false},
            ),
        ] {
            let rows = db.collection::<Document>(collection);
            let before = rows.find_one(filter.clone()).await.unwrap().unwrap();
            rows.update_one(filter.clone(), doc! {"$set": changes})
                .await
                .unwrap();
            refused_public(&db, &context).await;
            rows.replace_one(filter, &before).await.unwrap();
        }
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        super::super::platform_public_agent_service::revoke_in_session(
            &db,
            &mut session,
            &PublicRevocationScope::Sender {
                manager: context.ingress.manager.clone(),
                sender: context.ingress.sender.clone(),
            },
            PublicRevocationReason::SenderUnlinked,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        // Simulate an old row awaiting drain: the durable scope alone must deny it.
        sessions
            .replace_one(session_filter, &original)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(doc! {"_id": key_id}, doc! {"$set": {"is_active": true}})
            .await
            .unwrap();
        refused_public(&db, &context).await;
        db.drop().await.unwrap();
    }
}
