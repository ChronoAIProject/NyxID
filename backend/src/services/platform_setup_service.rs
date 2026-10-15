#![allow(clippy::collapsible_if)]

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::{IndexOptions, ReturnDocument},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{credits::Credits, platform_setup::*},
};

pub const TEMPLATE_VERSION: i64 = 1;
pub const MAX_FACT_BYTES: usize = 16 * 1024;
pub const MAX_OWNER_SETUPS: u64 = 10;
pub const MAX_RETAINED_OWNER_SETUPS: u64 = 50;

#[derive(Serialize)]
pub struct TemplateDescription {
    pub id: SetupTemplate,
    pub version: i64,
    pub name: &'static str,
    pub description: &'static str,
    pub runtime_connector_grants: &'static [&'static str],
    pub fields: &'static [&'static str],
}

pub fn templates() -> [TemplateDescription; 2] {
    [
        TemplateDescription {
            id: SetupTemplate::TelegramAssistant,
            version: TEMPLATE_VERSION,
            name: "Telegram assistant",
            description: "A client-owned specialist and Telegram bot, with reviewed public answers and limits.",
            runtime_connector_grants: &[],
            fields: &["name", "timezone", "assistant_instructions"],
        },
        TemplateDescription {
            id: SetupTemplate::RestaurantStarter,
            version: TEMPLATE_VERSION,
            name: "Restaurant starter",
            description: "Published FAQ, a reservation-request link and staff contact. Requests require staff confirmation.",
            runtime_connector_grants: &[],
            fields: &[
                "name",
                "timezone",
                "hours",
                "address",
                "menu_url",
                "reservation_url",
                "contact",
                "policies",
                "faq",
            ],
        },
    ]
}

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

pub fn default_limits() -> RuntimeLimits {
    RuntimeLimits {
        daily_credits: Credits::from_whole(1),
        turns_per_hour: 10,
        concurrent_turns: 1,
        max_input_chars: 4096,
        max_output_tokens: 2048,
    }
}

pub fn validate_limits(limits: &RuntimeLimits) -> AppResult<()> {
    if limits.daily_credits <= Credits::ZERO
        || limits.daily_credits > Credits::from_whole(1000)
        || !(1..=100).contains(&limits.turns_per_hour)
        || !(1..=2).contains(&limits.concurrent_turns)
        || !(256..=8192).contains(&limits.max_input_chars)
        || !(128..=4096).contains(&limits.max_output_tokens)
    {
        return Err(invalid("Runtime limits are outside supported bounds"));
    }
    Ok(())
}

pub fn validate_customer_url(value: &str) -> AppResult<()> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(invalid("Customer URL is too long or malformed"));
    }
    let url =
        reqwest::Url::parse(value).map_err(|_| invalid("Customer URL must be an HTTPS URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("Customer URL needs a public hostname"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || host == "localhost"
        || !host.contains('.')
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().is_ok()
        || url.port().is_some_and(|p| p != 443)
    {
        return Err(invalid(
            "Customer URL must use a public HTTPS hostname without credentials",
        ));
    }
    for (key, value) in url.query_pairs() {
        if [
            "token",
            "access_token",
            "refresh_token",
            "secret",
            "password",
            "key",
            "api_key",
            "signature",
            "code",
            "claim",
        ]
        .contains(&key.to_ascii_lowercase().as_str())
            || value.starts_with("nyxid_ag_")
            || value.starts_with("nyx_")
        {
            return Err(invalid(
                "Customer URL must not contain credentials or private handoff codes",
            ));
        }
    }
    if url.fragment().is_some() {
        return Err(invalid("Customer URL must not contain a fragment"));
    }
    Ok(())
}

pub fn validate_facts(
    template: SetupTemplate,
    facts: &SetupFacts,
    complete: bool,
) -> AppResult<()> {
    let bounded = [
        (&facts.name, 200),
        (&facts.timezone, 100),
        (&facts.hours, 2000),
        (&facts.address, 1000),
        (&facts.contact, 1000),
        (&facts.policies, 4000),
        (&facts.assistant_instructions, 4000),
    ];
    if bounded.iter().any(|(value, cap)| {
        value.len() > *cap
            || value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
    }) || facts.faq.len() > 20
        || facts.faq.iter().any(|entry| {
            entry.question.is_empty()
                || entry.question.len() > 300
                || entry.answer.is_empty()
                || entry.answer.len() > 2000
        })
        || serde_json::to_vec(facts)
            .map_err(|_| invalid("Malformed setup facts"))?
            .len()
            > MAX_FACT_BYTES
    {
        return Err(invalid("Setup facts exceed supported bounds"));
    }
    if !facts.timezone.is_empty() && facts.timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(invalid("Choose a valid IANA timezone"));
    }
    for url in [&facts.menu_url, &facts.reservation_url]
        .into_iter()
        .flatten()
    {
        validate_customer_url(url)?;
    }
    if template == SetupTemplate::RestaurantStarter && !facts.assistant_instructions.is_empty() {
        return Err(invalid(
            "Restaurant starter uses its fixed public-answer policy",
        ));
    }
    if template == SetupTemplate::TelegramAssistant
        && (!facts.hours.is_empty()
            || !facts.address.is_empty()
            || facts.menu_url.is_some()
            || facts.reservation_url.is_some()
            || !facts.contact.is_empty()
            || !facts.policies.is_empty()
            || !facts.faq.is_empty())
    {
        return Err(invalid(
            "Telegram assistant accepts only name, timezone and assistant instructions",
        ));
    }
    if complete
        && (facts.name.trim().is_empty()
            || facts.timezone.is_empty()
            || (template == SetupTemplate::RestaurantStarter
                && (facts.contact.trim().is_empty()
                    || facts.hours.trim().is_empty()
                    || facts.address.trim().is_empty())))
    {
        return Err(invalid(
            "Complete the required template fields before review",
        ));
    }
    Ok(())
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    let collection = db.collection::<PlatformSetup>(COLLECTION_NAME);
    for model in [
        IndexModel::builder()
            .keys(doc! {"idempotency_scope": 1, "idempotency_key": 1})
            .options(IndexOptions::builder().unique(true).build())
            .build(),
        IndexModel::builder()
            .keys(doc! {"owner_user_id": 1, "updated_at": -1, "_id": -1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"state": 1, "expires_at": 1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"purge_after": 1})
            .options(
                IndexOptions::builder()
                    .expire_after(std::time::Duration::ZERO)
                    .build(),
            )
            .build(),
    ] {
        collection.create_index(model).await?;
    }
    Ok(())
}

async fn person(db: &Database, owner: &str) -> AppResult<()> {
    if db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner, "user_type": {"$in": ["person", bson::Bson::Null]}, "is_active": true}).await?.is_none()
    { return Err(AppError::Forbidden("An active personal account is required for client setup".into())); }
    Ok(())
}

pub async fn decrypt_facts(keys: &EncryptionKeys, row: &PlatformSetup) -> AppResult<SetupFacts> {
    let bytes = Zeroizing::new(keys.decrypt(&row.facts_ciphertext).await?);
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Internal("Setup facts are unavailable".into()))
}

pub async fn get_owned(db: &Database, owner: &str, id: &str) -> AppResult<PlatformSetup> {
    person(db, owner).await?;
    let mut row = db
        .collection::<PlatformSetup>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "owner_user_id": owner})
        .await?
        .ok_or_else(|| AppError::NotFound("Setup not found".into()))?;
    if row.expires_at.is_some_and(|t| t <= Utc::now())
        && !matches!(
            row.state,
            SetupState::Active | SetupState::Cancelled | SetupState::Expired
        )
    {
        let updated = db.collection::<PlatformSetup>(COLLECTION_NAME).find_one_and_update(
            doc! {"_id": id, "owner_user_id": owner, "revision": row.revision},
            doc! {"$set": {"state": "expired", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null, "purge_after": bson::DateTime::from_chrono(Utc::now() + Duration::days(30))}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}
        ).return_document(ReturnDocument::After).await?;
        if let Some(updated) = updated {
            row = updated;
        } else {
            return Err(AppError::Conflict("Setup changed; refresh".into()));
        }
    }
    Ok(row)
}

pub async fn list_owned(db: &Database, owner: &str) -> AppResult<Vec<PlatformSetup>> {
    person(db, owner).await?;
    let rows: Vec<PlatformSetup> = db
        .collection::<PlatformSetup>(COLLECTION_NAME)
        .find(doc! {"owner_user_id": owner, "state": {"$nin": ["cancelled", "expired"]}})
        .sort(doc! {"updated_at": -1, "_id": -1})
        .limit(20)
        .await?
        .try_collect()
        .await?;
    let mut current = Vec::with_capacity(rows.len());
    for row in rows {
        let row = get_owned(db, owner, &row.id).await?;
        if !matches!(row.state, SetupState::Cancelled | SetupState::Expired) {
            current.push(row);
        }
    }
    Ok(current)
}

pub async fn list_archived(
    db: &Database,
    owner: &str,
    before: Option<(chrono::DateTime<Utc>, String)>,
) -> AppResult<Vec<PlatformSetup>> {
    person(db, owner).await?;
    let mut filter = doc! {"owner_user_id": owner, "state": {"$in": ["cancelled", "expired"]}};
    if let Some((at, id)) = before {
        filter.insert(
            "$or",
            bson::to_bson(&vec![
                doc! {"updated_at": {"$lt": bson::DateTime::from_chrono(at)}},
                doc! {"updated_at": bson::DateTime::from_chrono(at), "_id": {"$lt": id}},
            ])
            .map_err(|_| invalid("Malformed archive cursor"))?,
        );
    }
    Ok(db
        .collection::<PlatformSetup>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"updated_at": -1, "_id": -1})
        .limit(21)
        .await?
        .try_collect()
        .await?)
}

pub async fn create_owned(
    db: &Database,
    keys: &EncryptionKeys,
    owner: &str,
    request: &str,
    template: SetupTemplate,
    facts: &SetupFacts,
) -> AppResult<PlatformSetup> {
    validate_facts(template, facts, false)?;
    let idempotency =
        uuid::Uuid::parse_str(request).map_err(|_| invalid("Setup request ID must be UUID v4"))?;
    if idempotency.get_version_num() != 4 {
        return Err(invalid("Setup request ID must be UUID v4"));
    }
    person(db, owner).await?;
    let facts_bytes =
        Zeroizing::new(serde_json::to_vec(facts).map_err(|_| invalid("Malformed setup facts"))?);
    let now = Utc::now();
    let row = PlatformSetup {
        id: uuid::Uuid::new_v4().to_string(),
        owner_user_id: Some(owner.into()),
        initiating_actor_id: Some(owner.into()),
        platform_config_id: None,
        binding_id: None,
        binding_generation: None,
        profile_generation: None,
        source: "web".into(),
        idempotency_key: idempotency.to_string(),
        idempotency_scope: format!("person:{owner}"),
        template,
        template_version: TEMPLATE_VERSION,
        revision: 1,
        authority_generation: 1,
        state: SetupState::Collecting,
        facts_ciphertext: keys.encrypt(&facts_bytes).await?,
        limits: default_limits(),
        resources: SetupResources::default(),
        approved_digest: None,
        approved_revision: None,
        approved_by: None,
        approved_at: None,
        published: None,
        delivery_tested: false,
        delivery_test_message_id: None,
        created_at: now,
        updated_at: now,
        expires_at: Some(now + Duration::days(30)),
        purge_after: None,
    };
    let db = db.clone();
    let owner = owner.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = async {
                fence_owner(&db, &owner, session).await?;
                let collection = db.collection::<PlatformSetup>(COLLECTION_NAME);
                if let Some(existing) = collection
                    .find_one(
                        doc! {"owner_user_id": &owner, "idempotency_key": &row.idempotency_key},
                    )
                    .session(&mut *session)
                    .await?
                {
                    if existing.template != template {
                        return Err(AppError::Conflict(
                            "Setup request ID already belongs to a different template".into(),
                        ));
                    }
                    return Ok(existing);
                }
                if collection
                    .count_documents(
                        doc! {"owner_user_id": &owner, "state": {"$nin": ["cancelled", "expired"]}},
                    )
                    .session(&mut *session)
                    .await?
                    >= MAX_OWNER_SETUPS
                {
                    return Err(AppError::RateLimited);
                }
                if collection
                    .count_documents(doc! {"owner_user_id": &owner})
                    .session(&mut *session)
                    .await?
                    >= MAX_RETAINED_OWNER_SETUPS
                    || !super::coordination_service::RateWindowStore::admit_in_session(
                        &db,
                        "onboarding_draft_create",
                        &owner,
                        10,
                        std::time::Duration::from_secs(3600),
                        session,
                    )
                    .await?
                {
                    return Err(AppError::RateLimited);
                }
                collection.insert_one(&row).session(&mut *session).await?;
                Ok(row.clone())
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
}

pub(crate) async fn fence_owner(
    db: &Database,
    owner: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    if db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME).update_one(
        doc! {"_id": owner, "is_active": true, "user_type": {"$in": ["person", bson::Bson::Null]}}, doc! {"$inc": {"onboarding_revision": 1_i64}}
    ).session(session).await?.matched_count != 1 { return Err(AppError::Forbidden("Setup owner is unavailable".into())); }
    Ok(())
}

pub async fn edit_owned(
    db: &Database,
    keys: &EncryptionKeys,
    owner: &str,
    id: &str,
    revision: i64,
    facts: &SetupFacts,
    limits: &RuntimeLimits,
) -> AppResult<PlatformSetup> {
    let row = get_owned(db, owner, id).await?;
    if matches!(row.state, SetupState::Cancelled | SetupState::Expired) {
        return Err(AppError::Conflict("This setup is closed".into()));
    }
    validate_facts(row.template, facts, false)?;
    validate_limits(limits)?;
    let encrypted = keys
        .encrypt(&Zeroizing::new(
            serde_json::to_vec(facts).map_err(|_| invalid("Malformed setup facts"))?,
        ))
        .await?;
    let limits = bson::to_bson(limits).map_err(|_| invalid("Malformed limits"))?;
    mutate_owned(db, owner, id, revision, doc! {"$set": {"facts_ciphertext": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted},
        "limits": limits, "state": "collecting", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
        "approved_by": bson::Bson::Null, "approved_at": bson::Bson::Null, "delivery_tested": false, "delivery_test_message_id": bson::Bson::Null,
        "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}).await
}

pub(crate) async fn mutate_owned(
    db: &Database,
    owner: &str,
    id: &str,
    revision: i64,
    mut update: bson::Document,
) -> AppResult<PlatformSetup> {
    if !update.contains_key("$unset") {
        update.insert("$unset", doc! {});
    }
    if let Some(bson::Bson::Document(unset)) = update.get_mut("$unset") {
        unset.insert("owner_retirement", "");
    }
    let db = db.clone();
    let owner = owner.to_owned();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_owner(&db, &owner, session).await?;
            db.collection::<PlatformSetup>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &id, "owner_user_id": &owner, "revision": revision, "state": {"$nin": ["cancelled", "expired"]}}, update.clone()
            ).return_document(ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Setup revision changed; review the latest facts".into()))
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn pause_or_cancel(
    db: &Database,
    owner: &str,
    id: &str,
    revision: i64,
    cancel: bool,
) -> AppResult<PlatformSetup> {
    get_owned(db, owner, id).await?;
    let row = mutate_owned(db, owner, id, revision, doc! {"$set": {"state": if cancel {"cancelled"} else {"paused"},
        "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null, "delivery_tested": false,
        "updated_at": bson::DateTime::now(), "purge_after": if cancel { bson::Bson::DateTime(bson::DateTime::from_chrono(Utc::now() + Duration::days(30))) } else { bson::Bson::Null }},
        "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}).await?;
    if cancel {
        release_terminal_request(db, id).await?;
    }
    Ok(row)
}

/// Retire the native Telegram creation request of a cancelled or expired
/// setup so it no longer occupies the client's creation slot. Started effects
/// keep their provenance; the generation bump fences in-flight connection work.
pub(crate) async fn release_terminal_request(db: &Database, setup_id: &str) -> AppResult<bool> {
    // A purged setup is as final as a cancelled or expired one.
    let live = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": setup_id, "state": {"$nin": ["cancelled", "expired"]}})
        .projection(doc! {"_id": 1})
        .await?
        .is_some();
    if live {
        return Ok(false);
    }
    let released = db
        .collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME)
        .update_many(
            doc! {"setup_authority.setup_id": setup_id, "active": true,
                "status": {"$in": ["waiting_telegram", "waiting_bot", "waiting_consent", "ready", "provisioning"]}},
            doc! {"$set": {"active": false, "status": "cancelled", "auto_connect": false},
                "$inc": {"revision": 1_i64, "connection_generation": 1_i64}, "$unset": {"consent_hash": ""}},
        )
        .await?;
    Ok(released.modified_count > 0)
}

pub fn approval_digest(row: &PlatformSetup, facts: &SetupFacts) -> AppResult<String> {
    let data = serde_json::json!({"version": 1, "setup_id": row.id, "owner_user_id": row.owner_user_id,
        "template": row.template, "template_version": row.template_version, "revision": row.revision,
        "binding_id": row.binding_id, "binding_generation": row.binding_generation, "profile_generation": row.profile_generation,
        "authority_generation": row.authority_generation, "facts": facts,
        "funding_owner_user_id": row.owner_user_id, "daily_credits": row.limits.daily_credits.to_string(),
        "turns_per_hour": row.limits.turns_per_hour, "concurrent_turns": row.limits.concurrent_turns,
        "max_input_chars": row.limits.max_input_chars, "max_output_tokens": row.limits.max_output_tokens,
        "resources": row.resources, "runtime_connector_grants": [], "published_revision": row.published.as_ref().map(|p| p.revision)});
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&data).map_err(|_| invalid("Malformed approval"))?,
    )))
}

pub async fn request_review(
    db: &Database,
    keys: &EncryptionKeys,
    owner: &str,
    id: &str,
    revision: i64,
) -> AppResult<PlatformSetup> {
    let row = get_owned(db, owner, id).await?;
    if !matches!(
        row.state,
        SetupState::Collecting
            | SetupState::AwaitingApproval
            | SetupState::Paused
            | SetupState::FailedRetryable
    ) {
        return Err(AppError::Conflict(
            "This setup cannot enter review from its current state".into(),
        ));
    }
    let facts = decrypt_facts(keys, &row).await?;
    validate_facts(row.template, &facts, true)?;
    validate_limits(&row.limits)?;
    let mut resources = row.resources.clone();
    resources
        .agent_id
        .get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
    let request = resources
        .creation_request_id
        .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
        .clone();
    resources.channel_bot_id = Some(request);
    if let Some(profile_id) = row.platform_config_id.as_deref() {
        let profile = super::platform_bot_config_service::get(db, profile_id).await?;
        if profile.enabled && Some(profile.generation) == row.profile_generation {
            if let Some(guide) = profile.guide {
                resources.runtime_model_service_id = Some(guide.model_service_id);
                resources.runtime_model_id = Some(guide.model_id);
            }
        }
    }
    mutate_owned(
        db,
        owner,
        id,
        revision,
        doc! {"$set": {"resources": bson::to_bson(&resources).map_err(|_| invalid("Malformed resource plan"))?, "state": "awaiting_approval", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null, "delivery_tested": false, "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}},
    )
    .await
}

pub async fn publish_reviewed(
    db: &Database,
    keys: &EncryptionKeys,
    owner: &str,
    id: &str,
    revision: i64,
) -> AppResult<PlatformSetup> {
    let row = get_owned(db, owner, id).await?;
    if row.revision != revision {
        return Err(AppError::Conflict("Setup revision changed".into()));
    }
    let facts = decrypt_facts(keys, &row).await?;
    validate_facts(row.template, &facts, true)?;
    let content = PublishedSetupContent {
        revision: revision + 1,
        facts,
        published_at: Utc::now(),
    };
    mutate_owned(db, owner, id, revision, doc! {"$set": {"published": bson::to_bson(&content).map_err(|_| invalid("Malformed publication"))?,
        "state": "awaiting_approval", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
        "delivery_tested": false, "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}).await
}

/// Explain effective availability without changing reviewed setup state.
pub(crate) async fn runtime_notice(
    db: &Database,
    row: &PlatformSetup,
) -> AppResult<Option<&'static str>> {
    if row.state != SetupState::Active {
        return Ok(None);
    }
    let installation = db
        .collection::<crate::models::platform_installation::PlatformInstallation>(
            crate::models::platform_installation::COLLECTION_NAME,
        )
        .find_one(doc! {"setup_id": &row.id})
        .await?;
    let Some(installation) = installation else {
        return Ok(Some(
            "Public replies are unavailable. Review this setup to recover its installation.",
        ));
    };
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let assessment = super::platform_provisioning_service::assess_owned_runtime(
        db,
        &installation.authority,
        &mut session,
    )
    .await;
    session.abort_transaction().await?;
    match assessment {
        Ok(()) => Ok(None),
        Err(AppError::Conflict(_) | AppError::Forbidden(_)) => Ok(Some(
            "Public replies are paused because installation authority or delivery changed. Review the latest setup and platform status before resuming.",
        )),
        Err(error) => Err(error),
    }
}

/// Retain installation provenance after account retirement; no child can become
/// an ordinary personal route when its owner or encrypted setup disappears.
pub(crate) async fn retire_owner(db: &Database, owner: &str, deleted: bool) -> AppResult<()> {
    let user = db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner})
        .await?;
    if user
        .as_ref()
        .is_some_and(|user| user.get_bool("is_active").ok() != Some(false))
    {
        return Err(AppError::Conflict(
            "Account was reactivated before retirement completed".into(),
        ));
    }
    // Ordinary account retirement keeps its existing standalone compatibility.
    if db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"owner_user_id": owner})
        .await?
        .is_none()
        && db
            .collection::<bson::Document>(crate::models::platform_binding::COLLECTION_NAME)
            .find_one(doc! {"user_id": owner})
            .await?
            .is_none()
        && db.collection::<bson::Document>(crate::models::platform_binding::REQUESTS).find_one(doc! {"approved_user_id": owner, "state": {"$in": ["pending", "awaiting_chat_confirmation"]}}).await?.is_none()
        && db
            .collection::<bson::Document>(crate::models::platform_installation::COLLECTION_NAME)
            .find_one(doc! {"owner_user_id": owner})
            .await?
            .is_none()
    {
        return Ok(());
    }
    let db = db.clone();
    let owner = owner.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let users = db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME);
            if users.find_one_and_update(doc! {"_id": &owner, "is_active": false},
                doc! {"$inc": {"platform_retirement_revision": 1_i64}}).session(&mut *session).await?.is_none()
                && users.find_one(doc! {"_id": &owner}).session(&mut *session).await?.is_some() {
                return Err(AppError::Conflict("Account was reactivated before retirement completed".into()));
            }

            let mut changes = doc! {"state": if deleted {"cancelled"} else {"paused"},
                "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
                "delivery_tested": false, "updated_at": bson::DateTime::now(), "owner_retirement": if deleted {"deleted"} else {"disabled"}};
            if deleted {
                changes.insert("facts_ciphertext", bson::Bson::Binary(bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: Vec::new()}));
                changes.insert("published", bson::Bson::Null);
                changes.insert("purge_after", bson::DateTime::from_chrono(Utc::now() + Duration::days(30)));
            }
            if !deleted {
                db.collection::<PlatformSetup>(COLLECTION_NAME).update_many(doc! {"owner_user_id": &owner, "state": {"$in": ["cancelled", "expired"]}, "owner_retirement": {"$ne": "disabled"}},
                    doc! {"$set": {"owner_retirement": "disabled"}}).session(&mut *session).await?;
            }
            let filter = if deleted {doc! {"owner_user_id": &owner}} else {
                doc! {"owner_user_id": &owner, "state": {"$nin": ["cancelled", "expired"]}, "owner_retirement": {"$ne": "disabled"}}
            };
            db.collection::<PlatformSetup>(COLLECTION_NAME).update_many(filter,
                doc! {"$set": changes, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}).session(&mut *session).await?;
            db.collection::<bson::Document>(crate::models::platform_binding::COLLECTION_NAME).update_many(
                doc! {"user_id": &owner, "owner_retirement": {"$ne": if deleted {"deleted"} else {"disabled"}}},
                doc! {"$set": {"is_active": false, "updated_at": bson::DateTime::now(), "owner_retirement": if deleted {"deleted"} else {"disabled"}}, "$inc": {"generation": 1_i64, "revocation_generation": 1_i64}}).session(&mut *session).await?;
            db.collection::<bson::Document>(crate::models::platform_binding::REQUESTS).update_many(
                doc! {"approved_user_id": &owner, "state": {"$in": ["pending", "awaiting_chat_confirmation"]}},
                doc! {"$set": {"state": "cancelled"}}).session(&mut *session).await?;
            db.collection::<bson::Document>(crate::models::platform_installation::COLLECTION_NAME).update_many(
                doc! {"owner_user_id": &owner, "owner_retirement": {"$ne": if deleted {"deleted"} else {"disabled"}}},
                doc! {"$set": {"state": if deleted {"owner_deleted"} else {"owner_disabled"}, "updated_at": bson::DateTime::now(), "owner_retirement": if deleted {"deleted"} else {"disabled"}}, "$inc": {"generation": 1_i64}}).session(&mut *session).await?;
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn expire(db: &Database) -> AppResult<()> {
    // Recover interrupted account retirement independently of the admin request.
    let owners = db.collection::<bson::Document>(COLLECTION_NAME).aggregate(vec![
        doc! {"$match": {"owner_user_id": {"$ne": bson::Bson::Null}, "owner_retirement": {"$ne": "deleted"}}},
        doc! {"$unionWith": {"coll": crate::models::platform_installation::COLLECTION_NAME, "pipeline": [{"$match": {"owner_retirement": {"$ne": "deleted"}}}, {"$project": {"owner_user_id": 1_i64, "owner_retirement": 1_i64}}]}},
        doc! {"$unionWith": {"coll": crate::models::platform_binding::COLLECTION_NAME, "pipeline": [{"$match": {"owner_retirement": {"$ne": "deleted"}}}, {"$project": {"owner_user_id": "$user_id", "owner_retirement": 1_i64}}]}},
        doc! {"$unionWith": {"coll": crate::models::platform_binding::REQUESTS, "pipeline": [{"$match": {"approved_user_id": {"$type": "string"}, "state": {"$in": ["pending", "awaiting_chat_confirmation"]}}}, {"$project": {"owner_user_id": "$approved_user_id"}}]}},
        doc! {"$lookup": {"from": crate::models::user::COLLECTION_NAME, "localField": "owner_user_id", "foreignField": "_id", "as": "owner"}},
        doc! {"$match": {"owner.is_active": {"$ne": true}, "$expr": {"$ne": ["$owner_retirement", {"$cond": [{"$eq": [{"$size": "$owner"}, 0_i64]}, "deleted", "disabled"]}]}}},
        doc! {"$group": {"_id": "$owner_user_id", "deleted": {"$first": {"$eq": [{"$size": "$owner"}, 0_i64]}}}},
        doc! {"$sort": {"_id": 1_i64}}, doc! {"$limit": 100_i64},
    ]).await?.try_collect::<Vec<_>>().await?;
    for row in owners {
        let owner = row
            .get_str("_id")
            .map_err(|_| invalid("Invalid setup owner"))?;
        let deleted = row
            .get_bool("deleted")
            .map_err(|_| invalid("Invalid setup owner"))?;
        retire_owner(db, owner, deleted).await?;
    }
    db.collection::<PlatformSetup>(COLLECTION_NAME).update_many(
        doc! {"expires_at": {"$lte": bson::DateTime::now()}, "state": {"$nin": ["active", "cancelled", "expired"]}},
        doc! {"$set": {"state": "expired", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
            "purge_after": bson::DateTime::from_chrono(Utc::now() + Duration::days(30))}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}
    ).await?;
    release_terminal_requests(db).await
}

/// Release native creation requests whose setup ended or was purged while the
/// customer was away, so the creation slot never stays occupied.
async fn release_terminal_requests(db: &Database) -> AppResult<()> {
    let setup_ids: Vec<String> = db
        .collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME)
        .find(doc! {"active": true, "setup_authority.setup_id": {"$type": "string"},
            "status": {"$in": ["waiting_telegram", "waiting_bot", "waiting_consent", "ready", "provisioning"]}})
        .projection(doc! {"setup_authority.setup_id": 1})
        .limit(100)
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .iter()
        .filter_map(|row| {
            row.get_document("setup_authority")
                .ok()
                .and_then(|a| a.get_str("setup_id").ok())
                .map(str::to_owned)
        })
        .collect();
    for setup_id in setup_ids {
        release_terminal_request(db, &setup_id).await?;
    }
    Ok(())
}

pub async fn attach_binding(
    db: &Database,
    owner: &str,
    setup_id: &str,
    revision: i64,
    binding_id: &str,
    binding_generation: i64,
) -> AppResult<PlatformSetup> {
    get_owned(db, owner, setup_id).await?;
    let db = db.clone();
    let owner = owner.to_owned();
    let setup_id = setup_id.to_owned();
    let binding_id = binding_id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_owner(&db, &owner, session).await?;
            let binding = db.collection::<crate::models::platform_binding::PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME)
                .find_one_and_update(doc! {"_id": &binding_id, "user_id": &owner, "generation": binding_generation,
                    "is_active": true, "expires_at": {"$gt": bson::DateTime::now()}}, doc! {"$inc": {"setup_adoption_revision": 1_i64}})
                .return_document(ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::NotFound("Account binding is unavailable".into()))?;
            super::platform_binding_service::fence_profile(&db, &binding.platform_config_id, binding.profile_generation, session).await?;
            db.collection::<PlatformSetup>(COLLECTION_NAME).find_one_and_update(doc! {"_id": &setup_id, "owner_user_id": &owner,
                "revision": revision, "state": {"$nin": ["cancelled", "expired"]}},
                doc! {"$set": {"platform_config_id": &binding.platform_config_id, "binding_id": &binding.id,
                    "binding_generation": binding.generation, "profile_generation": binding.profile_generation,
                    "state": "collecting", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null,
                    "delivery_tested": false, "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}}
            ).return_document(ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Setup revision changed".into()))
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

    #[tokio::test]
    async fn platform_onboarding_cancelled_setup_releases_provisioning_request() {
        let db = connect_transaction_test_database("platform_cancel_releases_request").await;
        let requests =
            db.collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME);
        let setups = db.collection::<bson::Document>(COLLECTION_NAME);
        setups
            .insert_one(doc! {"_id": "setup", "state": "provisioning"})
            .await
            .unwrap();
        requests
            .insert_one(
                doc! {"_id": "request", "active": true, "status": "provisioning",
                "auto_connect": true, "revision": 3_i64, "connection_generation": 2_i64,
                "setup_authority": {"setup_id": "setup"}},
            )
            .await
            .unwrap();
        // A live setup keeps its request.
        assert!(!release_terminal_request(&db, "setup").await.unwrap());
        setups
            .update_one(doc! {"_id": "setup"}, doc! {"$set": {"state": "cancelled"}})
            .await
            .unwrap();
        assert!(release_terminal_request(&db, "setup").await.unwrap());
        let request = requests
            .find_one(doc! {"_id": "request"})
            .await
            .unwrap()
            .unwrap();
        assert!(!request.get_bool("active").unwrap());
        assert_eq!(request.get_str("status").unwrap(), "cancelled");
        assert_eq!(request.get_i64("connection_generation").unwrap(), 3);
        // A setup purged by retention releases its request through the sweep.
        requests
            .insert_one(
                doc! {"_id": "orphan", "active": true, "status": "provisioning",
                "auto_connect": true, "revision": 1_i64, "connection_generation": 1_i64,
                "setup_authority": {"setup_id": "purged"}},
            )
            .await
            .unwrap();
        release_terminal_requests(&db).await.unwrap();
        let orphan = requests
            .find_one(doc! {"_id": "orphan"})
            .await
            .unwrap()
            .unwrap();
        assert!(!orphan.get_bool("active").unwrap());
        assert_eq!(orphan.get_str("status").unwrap(), "cancelled");
        db.drop().await.unwrap();
    }

    fn restaurant() -> SetupFacts {
        SetupFacts {
            name: "Sample restaurant".into(),
            timezone: "Asia/Singapore".into(),
            hours: "12:00–21:00".into(),
            address: "Private draft address".into(),
            contact: "Ask restaurant staff".into(),
            reservation_url: Some("https://forms.example.org/request".into()),
            faq: vec![FaqEntry {
                question: "Are requests confirmed?".into(),
                answer: "Staff must confirm the request.".into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn platform_onboarding_templates_validate_closed_facts_and_customer_links() {
        assert_eq!(templates().len(), 2);
        assert!(
            templates()
                .iter()
                .all(|t| t.runtime_connector_grants.is_empty())
        );
        assert!(validate_facts(SetupTemplate::RestaurantStarter, &restaurant(), true).is_ok());
        assert!(validate_facts(SetupTemplate::TelegramAssistant, &restaurant(), true).is_err());
        for url in [
            "http://forms.example.org/",
            "https://user:secret@forms.example.org/",
            "https://localhost/form",
            "https://127.0.0.1/form",
            "https://forms.example.org/?access_token=private",
            "https://forms.example.org/#secret",
        ] {
            assert!(validate_customer_url(url).is_err());
        }
        let mut facts = restaurant();
        facts.assistant_instructions = "Call arbitrary tools".into();
        assert!(validate_facts(SetupTemplate::RestaurantStarter, &facts, true).is_err());
        assert!(
            serde_json::from_value::<SetupFacts>(
                serde_json::json!({"name":"x", "timezone":"UTC", "user_id":"foreign"})
            )
            .is_err()
        );
        let mut limits = default_limits();
        limits.daily_credits = Credits::ZERO;
        assert!(validate_limits(&limits).is_err());
    }

    async fn fixture(name: &str) -> (crate::AppState, String, String) {
        let db = connect_transaction_test_database(name).await;
        ensure_indexes(&db).await.unwrap();
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        for id in [&a, &b] {
            db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(test_user(id, crate::models::user::UserType::Person))
                .await
                .unwrap();
        }
        (test_app_state(db), a, b)
    }

    #[tokio::test]
    async fn platform_onboarding_setup_isolation_idempotency_encryption_and_revision_race() {
        let (state, a, b) = fixture("platform_setup_revision").await;
        let request = uuid::Uuid::new_v4().to_string();
        let facts = restaurant();
        let row = create_owned(
            &state.db,
            &state.encryption_keys,
            &a,
            &request,
            SetupTemplate::RestaurantStarter,
            &facts,
        )
        .await
        .unwrap();
        let retry = create_owned(
            &state.db,
            &state.encryption_keys,
            &a,
            &request,
            SetupTemplate::RestaurantStarter,
            &facts,
        )
        .await
        .unwrap();
        assert_eq!(row.id, retry.id);
        let other = create_owned(
            &state.db,
            &state.encryption_keys,
            &b,
            &request,
            SetupTemplate::RestaurantStarter,
            &facts,
        )
        .await
        .unwrap();
        assert_ne!(other.id, row.id);
        assert!(matches!(
            get_owned(&state.db, &b, &row.id).await,
            Err(AppError::NotFound(_))
        ));
        assert!(row.published.is_none());
        let stored = state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.get_binary_generic("facts_ciphertext").is_ok());
        assert!(!stored.to_string().contains("Private draft address"));
        assert_eq!(
            decrypt_facts(&state.encryption_keys, &row)
                .await
                .unwrap()
                .address,
            facts.address
        );
        let mut first = facts.clone();
        first.hours = "10:00–20:00".into();
        let mut second = facts.clone();
        second.hours = "11:00–19:00".into();
        let limits = default_limits();
        let (x, y) = tokio::join!(
            edit_owned(
                &state.db,
                &state.encryption_keys,
                &a,
                &row.id,
                1,
                &first,
                &limits
            ),
            edit_owned(
                &state.db,
                &state.encryption_keys,
                &a,
                &row.id,
                1,
                &second,
                &limits
            )
        );
        assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
        let current = get_owned(&state.db, &a, &row.id).await.unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.authority_generation, 2);
        let published = publish_reviewed(
            &state.db,
            &state.encryption_keys,
            &a,
            &row.id,
            current.revision,
        )
        .await
        .unwrap();
        assert_eq!(
            published.published.as_ref().unwrap().revision,
            published.revision
        );
        let old_digest = approval_digest(
            &published,
            &decrypt_facts(&state.encryption_keys, &published)
                .await
                .unwrap(),
        )
        .unwrap();
        let edited = edit_owned(
            &state.db,
            &state.encryption_keys,
            &a,
            &row.id,
            published.revision,
            &facts,
            &limits,
        )
        .await
        .unwrap();
        assert_ne!(approval_digest(&edited, &facts).unwrap(), old_digest);
        assert!(edited.approved_digest.is_none());
        assert!(!edited.delivery_tested);
    }

    #[tokio::test]
    async fn platform_onboarding_cancel_cycles_are_bounded_and_archive_cannot_hide_active() {
        let (state, a, _) = fixture("platform_setup_retention").await;
        let facts = restaurant();
        let active = create_owned(
            &state.db,
            &state.encryption_keys,
            &a,
            &uuid::Uuid::new_v4().to_string(),
            SetupTemplate::RestaurantStarter,
            &facts,
        )
        .await
        .unwrap();
        for _ in 0..9 {
            let row = create_owned(
                &state.db,
                &state.encryption_keys,
                &a,
                &uuid::Uuid::new_v4().to_string(),
                SetupTemplate::RestaurantStarter,
                &facts,
            )
            .await
            .unwrap();
            pause_or_cancel(&state.db, &a, &row.id, row.revision, true)
                .await
                .unwrap();
        }
        assert!(matches!(
            create_owned(
                &state.db,
                &state.encryption_keys,
                &a,
                &uuid::Uuid::new_v4().to_string(),
                SetupTemplate::RestaurantStarter,
                &facts
            )
            .await,
            Err(AppError::RateLimited)
        ));
        // Populate retained archived rows directly to exercise listing independent
        // of admission's intentionally lower hourly limit.
        for index in 0..25 {
            let mut archived = active.clone();
            archived.id = uuid::Uuid::new_v4().to_string();
            archived.idempotency_key = uuid::Uuid::new_v4().to_string();
            archived.state = SetupState::Cancelled;
            archived.updated_at = active.updated_at + Duration::milliseconds(index + 1);
            state
                .db
                .collection::<PlatformSetup>(COLLECTION_NAME)
                .insert_one(archived)
                .await
                .unwrap();
        }
        let listed = list_owned(&state.db, &a).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, active.id);
        let mut first = list_archived(&state.db, &a, None).await.unwrap();
        assert_eq!(first.len(), 21);
        first.truncate(20);
        let last = first.last().unwrap();
        let second = list_archived(&state.db, &a, Some((last.updated_at, last.id.clone())))
            .await
            .unwrap();
        assert!(!second.is_empty());
        assert!(second.iter().all(|r| !first.iter().any(|f| f.id == r.id)));
        state.db.collection::<PlatformSetup>(COLLECTION_NAME).update_one(doc! {"_id": &active.id},doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        assert_eq!(
            get_owned(&state.db, &a, &active.id).await.unwrap().state,
            SetupState::Expired
        );
        assert!(matches!(
            edit_owned(
                &state.db,
                &state.encryption_keys,
                &a,
                &active.id,
                1,
                &facts,
                &default_limits()
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
}

/// Closed guide draft effect, bound to its verified private sender and exact
/// live profile/binding. Anonymous drafts cannot impersonate a shared null owner.
pub(crate) async fn guide_draft(
    state: &crate::AppState,
    authority: &super::platform_inference_service::GuideAuthority,
    template: SetupTemplate,
    facts: &SetupFacts,
) -> AppResult<PlatformSetup> {
    validate_facts(template, facts, false)?;
    let scope = native_scope(state, &authority.profile.id, &authority.sender)?;
    let bytes =
        Zeroizing::new(serde_json::to_vec(facts).map_err(|_| invalid("Malformed setup facts"))?);
    let now = Utc::now();
    let owner = authority
        .binding
        .as_ref()
        .map(|binding| binding.user_id.clone());
    let row = PlatformSetup {
        id: uuid::Uuid::new_v4().to_string(),
        owner_user_id: owner.clone(),
        initiating_actor_id: owner,
        platform_config_id: Some(authority.profile.id.clone()),
        binding_id: authority.binding.as_ref().map(|binding| binding.id.clone()),
        binding_generation: authority.binding.as_ref().map(|binding| binding.generation),
        profile_generation: Some(authority.profile.generation),
        source: "native_guide".into(),
        idempotency_key: authority.update_id.to_string(),
        idempotency_scope: scope.clone(),
        template,
        template_version: TEMPLATE_VERSION,
        revision: 1,
        authority_generation: 1,
        state: if authority.binding.is_some() {
            SetupState::Collecting
        } else {
            SetupState::AwaitingIdentity
        },
        facts_ciphertext: state.encryption_keys.encrypt(&bytes).await?,
        limits: default_limits(),
        resources: SetupResources::default(),
        approved_digest: None,
        approved_revision: None,
        approved_by: None,
        approved_at: None,
        published: None,
        delivery_tested: false,
        delivery_test_message_id: None,
        created_at: now,
        updated_at: now,
        expires_at: Some(
            now + if authority.binding.is_some() {
                Duration::days(30)
            } else {
                Duration::minutes(30)
            },
        ),
        purge_after: if authority.binding.is_none() {
            Some(now + Duration::hours(1))
        } else {
            None
        },
    };
    let db = state.db.clone();
    let authority = authority.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            super::platform_inference_service::fence_public(&db, &authority, session).await?;
            let collection = db.collection::<PlatformSetup>(COLLECTION_NAME);
            if let Some(existing) = collection.find_one(doc! {"idempotency_scope": &scope, "idempotency_key": &row.idempotency_key}).session(&mut *session).await? {
                if existing.owner_user_id != row.owner_user_id || existing.binding_id != row.binding_id || existing.binding_generation != row.binding_generation
                    || existing.platform_config_id != row.platform_config_id || existing.profile_generation != row.profile_generation {
                    return Err(AppError::Forbidden("Draft replay authority changed".into()));
                }
                if collection.update_one(doc! {"_id": &existing.id, "owner_user_id": &row.owner_user_id, "binding_id": &row.binding_id,
                    "binding_generation": row.binding_generation, "platform_config_id": &row.platform_config_id, "profile_generation": row.profile_generation,
                    "revision": existing.revision, "state": {"$nin": ["cancelled", "expired"]}, "$expr": {"$gt": ["$expires_at", "$$NOW"]}},
                    doc! {"$inc": {"guide_replay_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 {
                    return Err(AppError::Conflict("Draft replay is no longer current".into()));
                }
                return Ok(existing);
            }
            if collection.count_documents(doc! {"idempotency_scope": &scope}).session(&mut *session).await? >= 20
                || !super::coordination_service::RateWindowStore::admit_in_session(&db, "platform_native_draft", &scope, 3, std::time::Duration::from_secs(3600), session).await? { return Err(AppError::RateLimited); }
            if let Some(owner) = &row.owner_user_id {
                fence_owner(&db, owner, session).await?;
                if collection.count_documents(doc! {"owner_user_id": owner}).session(&mut *session).await? >= MAX_RETAINED_OWNER_SETUPS
                    || collection.count_documents(doc! {"owner_user_id": owner, "state": {"$nin": ["cancelled", "expired"]}}).session(&mut *session).await? >= MAX_OWNER_SETUPS { return Err(AppError::RateLimited); }
            }
            collection.insert_one(&row).session(&mut *session).await?;
            Ok(row.clone())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn guide_update_current(
    state: &crate::AppState,
    a: &super::platform_inference_service::GuideAuthority,
    captured: &PlatformSetup,
    facts: &SetupFacts,
) -> AppResult<PlatformSetup> {
    validate_facts(captured.template, facts, false)?;
    if !matches!(
        captured.state,
        SetupState::Draft
            | SetupState::Collecting
            | SetupState::AwaitingIdentity
            | SetupState::AwaitingApproval
    ) {
        return Err(AppError::Conflict(
            "Review changes to an approved setup on NyxID's website".into(),
        ));
    }
    let owner = a.binding.as_ref().map(|binding| binding.user_id.clone());
    let scope = if owner.is_none() {
        native_scope(state, &a.profile.id, &a.sender)?
    } else {
        captured.idempotency_scope.clone()
    };
    if captured.idempotency_scope != scope {
        return Err(AppError::Forbidden(
            "This anonymous draft belongs to another sender".into(),
        ));
    }
    let bytes =
        Zeroizing::new(serde_json::to_vec(facts).map_err(|_| invalid("Malformed setup facts"))?);
    let ciphertext = state.encryption_keys.encrypt(&bytes).await?;
    let db = state.db.clone();
    let a = a.clone();
    let captured = captured.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            super::platform_inference_service::fence_public(&db, &a, session).await?;
            let owner = a.binding.as_ref().map(|binding| binding.user_id.clone());
            if captured.owner_user_id != owner || captured.platform_config_id.as_deref() != Some(a.profile.id.as_str())
                || captured.profile_generation != Some(a.profile.generation) || captured.binding_id != a.binding.as_ref().map(|binding| binding.id.clone())
                || captured.binding_generation != a.binding.as_ref().map(|binding| binding.generation) { return Err(AppError::Forbidden("Draft authority changed".into())); }
            db.collection::<PlatformSetup>(COLLECTION_NAME).find_one_and_update(doc! {"_id": &captured.id, "owner_user_id": &owner,
                "idempotency_scope": &scope, "platform_config_id": &a.profile.id, "profile_generation": a.profile.generation,
                "binding_id": a.binding.as_ref().map(|binding| &binding.id), "binding_generation": a.binding.as_ref().map(|binding| binding.generation), "revision": captured.revision, "authority_generation": captured.authority_generation,
                "state": {"$in": ["draft", "collecting", "awaiting_identity", "awaiting_approval"]}, "$expr": {"$gt": ["$expires_at", "$$NOW"]}},
                doc! {"$set": {"facts_ciphertext": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: ciphertext.clone()},
                    "state": if owner.is_some() {"collecting"} else {"awaiting_identity"}, "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null, "delivery_tested": false,
                    "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64, "authority_generation": 1_i64}})
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(|| AppError::Conflict("The draft changed; review its latest facts".into()))
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) fn native_scope(
    state: &crate::AppState,
    profile: &str,
    sender: &str,
) -> AppResult<String> {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<Sha256>::new_from_slice(state.auth_device_hmac_key.as_slice())
        .map_err(|_| invalid("Draft admission unavailable"))?;
    mac.update(format!("platform-draft-v1:{profile}:{sender}").as_bytes());
    Ok(format!(
        "native:{}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

pub(crate) async fn anonymous_for_owner(
    state: &crate::AppState,
    owner: &str,
    binding_id: &str,
    generation: i64,
) -> AppResult<Vec<PlatformSetup>> {
    let binding = state.db.collection::<crate::models::platform_binding::PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME)
        .find_one(doc! {"_id": binding_id, "user_id": owner, "generation": generation, "is_active": true, "$expr": {"$gt": ["$expires_at", "$$NOW"]}}).await?.ok_or_else(|| AppError::Forbidden("Binding is unavailable".into()))?;
    let profile =
        super::platform_bot_config_service::get(&state.db, &binding.platform_config_id).await?;
    let scope = native_scope(state, &profile.id, &binding.telegram_sender_id)?;
    let rows = state.db.collection::<PlatformSetup>(COLLECTION_NAME).find(doc! {"owner_user_id": bson::Bson::Null, "idempotency_scope": scope,
        "platform_config_id": &profile.id, "profile_generation": binding.profile_generation, "state": "awaiting_identity", "$expr": {"$gt": ["$expires_at", "$$NOW"]}})
        .sort(doc! {"updated_at": -1}).limit(20).await?.try_collect().await?;
    super::platform_binding_service::admit_reply(&state.db, &profile, Some(&binding)).await?;
    Ok(rows)
}

pub(crate) async fn adopt_anonymous(
    state: &crate::AppState,
    owner: &str,
    id: &str,
    revision: i64,
    binding_id: &str,
    generation: i64,
) -> AppResult<PlatformSetup> {
    let binding = state
        .db
        .collection::<crate::models::platform_binding::PlatformBinding>(
            crate::models::platform_binding::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id": binding_id, "user_id": owner, "generation": generation, "is_active": true},
        )
        .await?
        .ok_or_else(|| AppError::Forbidden("Binding is unavailable".into()))?;
    let profile =
        super::platform_bot_config_service::get(&state.db, &binding.platform_config_id).await?;
    let scope = native_scope(state, &profile.id, &binding.telegram_sender_id)?;
    let db = state.db.clone();
    let owner = owner.to_owned();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            super::platform_binding_service::fence_reply(&db, &profile, Some(&binding), session).await?;
            let collection = db.collection::<PlatformSetup>(COLLECTION_NAME);
            if let Some(row) = collection.find_one(doc! {"_id": &id, "owner_user_id": &owner, "binding_id": &binding.id, "binding_generation": generation}).session(&mut *session).await? { return Ok(row); }
            if collection.count_documents(doc! {"owner_user_id": &owner}).session(&mut *session).await? >= MAX_RETAINED_OWNER_SETUPS
                || collection.count_documents(doc! {"owner_user_id": &owner, "state": {"$nin": ["cancelled", "expired"]}}).session(&mut *session).await? >= MAX_OWNER_SETUPS { return Err(AppError::RateLimited); }
            collection.find_one_and_update(doc! {"_id": &id, "revision": revision, "owner_user_id": bson::Bson::Null, "idempotency_scope": &scope,
                "platform_config_id": &profile.id, "profile_generation": profile.generation, "state": "awaiting_identity", "$expr": {"$gt": ["$expires_at", "$$NOW"]}},
                vec![doc! {"$set": {"owner_user_id": &owner, "initiating_actor_id": &owner, "binding_id": &binding.id, "binding_generation": generation,
                    "state": "collecting", "approved_digest": bson::Bson::Null, "approved_revision": bson::Bson::Null, "delivery_tested": false,
                    "purge_after": bson::Bson::Null, "expires_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "day", "amount": 30_i64}},
                    "updated_at": "$$NOW", "revision": {"$add": ["$revision", 1_i64]}, "authority_generation": {"$add": ["$authority_generation", 1_i64]}}}])
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(|| AppError::Conflict("Anonymous draft expired or changed".into()))
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn guide_select_current(
    state: &crate::AppState,
    a: &super::platform_inference_service::GuideAuthority,
    captured: &PlatformSetup,
) -> AppResult<()> {
    let binding = a
        .binding
        .as_ref()
        .ok_or_else(|| AppError::Conflict("Link and adopt drafts before selecting a setup".into()))?
        .clone();
    let db = state.db.clone();
    let a = a.clone();
    let captured = captured.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            super::platform_inference_service::fence_public(&db, &a, session).await?;
            if db.collection::<PlatformSetup>(COLLECTION_NAME).update_one(doc! {"_id": &captured.id, "owner_user_id": &binding.user_id,
                "binding_id": &binding.id, "binding_generation": binding.generation, "platform_config_id": &a.profile.id, "profile_generation": a.profile.generation,
                "revision": captured.revision, "state": {"$nin": ["cancelled", "expired"]}}, doc! {"$inc": {"selection_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 {
                return Err(AppError::Conflict("The selected setup changed".into()));
            }
            db.collection::<bson::Document>("platform_guide_selections").update_one(doc! {"binding_id": &binding.id, "generation": binding.generation},
                vec![doc! {"$set": {"_id": {"$ifNull": ["$_id", uuid::Uuid::new_v4().to_string()]}, "binding_id": &binding.id,
                    "generation": binding.generation, "setup_id": &captured.id, "purge_after": {"$dateAdd": {"startDate": "$$NOW", "unit": "day", "amount": 30_i64}}}}])
                .upsert(true).session(&mut *session).await?;
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

#[cfg(test)]
mod guide_scope_tests {
    use super::super::{
        platform_inference_service::{GuideAuthority, PublicPurpose},
        platform_onboarding_test_support,
    };
    use super::*;
    #[tokio::test]
    async fn platform_onboarding_anonymous_write_and_replay_enforce_verified_sender_and_owner() {
        let f = platform_onboarding_test_support::distinct_fixture().await;
        let mut a = GuideAuthority {
            profile: f.profile.clone(),
            binding: None,
            sender: "801".into(),
            update_id: 600,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let facts = SetupFacts {
            name: "Sender A cafe".into(),
            timezone: "UTC".into(),
            ..Default::default()
        };
        let draft = guide_draft(&f.state, &a, SetupTemplate::RestaurantStarter, &facts)
            .await
            .unwrap();
        assert_eq!(
            guide_draft(&f.state, &a, SetupTemplate::RestaurantStarter, &facts)
                .await
                .unwrap()
                .id,
            draft.id
        );
        let mut b = a.clone();
        b.sender = "802".into();
        b.update_id = 601;
        assert!(matches!(
            guide_update_current(&f.state, &b, &draft, &facts).await,
            Err(AppError::Forbidden(_))
        ));
        let unchanged = f
            .state
            .db
            .collection::<PlatformSetup>(COLLECTION_NAME)
            .find_one(doc! {"_id": &draft.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.revision, draft.revision);
        // A valid current binding cannot replay an earlier owner's scope/update row.
        a.sender = "700".into();
        a.update_id = 602;
        a.binding = Some(f.binding.clone());
        let owned = guide_draft(&f.state, &a, SetupTemplate::RestaurantStarter, &facts)
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &owned.id},
                doc! {"$set": {"owner_user_id": f.operator.clone()}},
            )
            .await
            .unwrap();
        assert!(matches!(
            guide_draft(&f.state, &a, SetupTemplate::RestaurantStarter, &facts).await,
            Err(AppError::Forbidden(_))
        ));
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    #[tokio::test]
    async fn platform_onboarding_retirement_recovery_advances_beyond_first_hundred_and_fences_reactivation()
     {
        let f = super::super::platform_onboarding_test_support::fixture().await;
        let db = &f.state.db;
        let mut owners = Vec::new();
        let mut rows = Vec::new();
        let mut ids = (0..101)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect::<Vec<_>>();
        ids.sort();
        for (position, id) in ids.into_iter().enumerate() {
            let mut user = crate::test_utils::test_user(&id, crate::models::user::UserType::Person);
            user.is_active = false;
            owners.push(user);
            rows.push(
                doc! {"_id": uuid::Uuid::new_v4().to_string(), "owner_user_id": &id,
                "idempotency_scope": &id, "idempotency_key": uuid::Uuid::new_v4().to_string(),
                "state": if position < 100 {"cancelled"} else {"collecting"}, "revision": 1_i64, "authority_generation": 1_i64},
            );
        }
        let reactivated = owners[0].id.clone();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many(owners)
            .await
            .unwrap();
        db.collection::<bson::Document>(COLLECTION_NAME)
            .insert_many(rows)
            .await
            .unwrap();
        expire(db).await.unwrap();
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {"owner_retirement": "disabled"})
                .await
                .unwrap(),
            100
        );
        expire(db).await.unwrap();
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {"owner_retirement": "disabled"})
                .await
                .unwrap(),
            101
        );
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {"state": "cancelled", "owner_retirement": "disabled"})
                .await
                .unwrap(),
            100
        );
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &reactivated},
                doc! {"$set": {"is_active": true}},
            )
            .await
            .unwrap();
        assert!(retire_owner(db, &reactivated, false).await.is_err());
        expire(db).await.unwrap();
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {"owner_retirement": "disabled"})
                .await
                .unwrap(),
            101
        );
    }

    #[tokio::test]
    async fn platform_onboarding_account_retirement_retains_child_tombstone_and_clears_deleted_facts()
     {
        let f = super::super::platform_onboarding_test_support::fixture().await;
        let db = &f.state.db;
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": f.setup.owner_user_id.as_deref().unwrap()},
                doc! {"$set": {"is_active": false}},
            )
            .await
            .unwrap();
        let installation = db
            .collection::<crate::models::platform_installation::PlatformInstallation>(
                crate::models::platform_installation::COLLECTION_NAME,
            )
            .find_one(doc! {"setup_id": &f.setup.id})
            .await
            .unwrap()
            .unwrap();
        retire_owner(db, f.setup.owner_user_id.as_deref().unwrap(), false)
            .await
            .unwrap();
        let paused = db
            .collection::<PlatformSetup>(COLLECTION_NAME)
            .find_one(doc! {"_id": &f.setup.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(paused.state, SetupState::Paused);
        assert!(paused.approved_digest.is_none());
        assert!(!paused.facts_ciphertext.is_empty());
        retire_owner(db, f.setup.owner_user_id.as_deref().unwrap(), true)
            .await
            .unwrap();
        let cancelled = db
            .collection::<PlatformSetup>(COLLECTION_NAME)
            .find_one(doc! {"_id": &f.setup.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.state, SetupState::Cancelled);
        assert!(cancelled.facts_ciphertext.is_empty());
        assert!(cancelled.published.is_none());
        let tombstone = db
            .collection::<crate::models::platform_installation::PlatformInstallation>(
                crate::models::platform_installation::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &installation.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tombstone.channel_bot_id, installation.channel_bot_id);
        assert_eq!(tombstone.state, "owner_deleted");
        assert!(tombstone.generation > installation.generation);
        assert!(
            !db.collection::<crate::models::platform_binding::PlatformBinding>(
                crate::models::platform_binding::COLLECTION_NAME
            )
            .find_one(doc! {"_id": &f.binding.id})
            .await
            .unwrap()
            .unwrap()
            .is_active
        );
    }
}

#[cfg(test)]
mod signup_journey_tests {
    use super::*;
    use axum::{
        Json,
        body::{Body, to_bytes},
        extract::{ConnectInfo, State},
        http::{HeaderMap, Request, StatusCode},
    };
    use mongodb::bson::Document;
    use tower::ServiceExt;

    #[tokio::test]
    async fn platform_onboarding_real_registration_policy_login_and_owned_resume() {
        use crate::handlers::auth::{self, LoginRequest, RegisterRequest};
        let mut f = super::super::platform_onboarding_test_support::distinct_fixture().await;
        let peer = ConnectInfo("127.0.0.1:45678".parse().unwrap());
        let email = format!("{}@signup.test", uuid::Uuid::new_v4());
        let password = format!("Test-1!{}", uuid::Uuid::new_v4());
        f.state.config.email_auth_enabled = false;
        assert!(matches!(
            auth::register(
                State(f.state.clone()),
                peer,
                HeaderMap::new(),
                Json(RegisterRequest {
                    email: email.clone(),
                    password: password.clone(),
                    invite_code: None,
                    display_name: None,
                })
            )
            .await,
            Err(AppError::EmailSignupDisabled)
        ));
        f.state.config.email_auth_enabled = true;
        f.state.config.auto_verify_email = true;
        assert!(
            super::super::feature_flag_service::invitation_code_required(&f.state.db)
                .await
                .unwrap()
        );
        assert!(matches!(
            auth::register(
                State(f.state.clone()),
                peer,
                HeaderMap::new(),
                Json(RegisterRequest {
                    email: email.clone(),
                    password: password.clone(),
                    invite_code: None,
                    display_name: None,
                })
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .count_documents(doc! {"email": &email})
                .await
                .unwrap(),
            0
        );
        let invite = super::super::invite_code_service::create_invite_code(
            &f.state.db,
            &f.operator,
            1,
            Some("Local onboarding integration"),
        )
        .await
        .unwrap();
        let Json(registered) = auth::register(
            State(f.state.clone()),
            peer,
            HeaderMap::new(),
            Json(RegisterRequest {
                email: email.clone(),
                password: password.clone(),
                invite_code: Some(invite.code),
                display_name: Some("Registered client".into()),
            }),
        )
        .await
        .unwrap();
        let (_, Json(login)) = auth::login(
            State(f.state.clone()),
            peer,
            HeaderMap::new(),
            Json(LoginRequest {
                email: email.clone(),
                password: password.clone(),
                mfa_code: None,
                client: Some("token".into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(registered.user_id, login.user_id);
        let token = zeroize::Zeroizing::new(login.access_token.unwrap());
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let facts = serde_json::json!({"name": "Registered client bot", "timezone": "UTC", "assistant_instructions": "Public help"});
        let body = serde_json::json!({"request_id": uuid::Uuid::new_v4().to_string(), "template": "telegram-assistant", "facts": facts});
        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/onboarding/setups")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/onboarding/setups")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let response_status = response.status();
        let response_body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
        assert_eq!(
            response_status,
            StatusCode::OK,
            "authenticated setup creation failed: {}",
            String::from_utf8_lossy(&response_body)
        );
        let created: serde_json::Value = serde_json::from_slice(&response_body).unwrap();
        let id = created["id"].as_str().unwrap();
        assert_eq!(
            get_owned(&f.state.db, &registered.user_id, id)
                .await
                .unwrap()
                .owner_user_id
                .as_deref(),
            Some(registered.user_id.as_str())
        );
        assert!(get_owned(&f.state.db, &f.operator, id).await.is_err());
        let (_, Json(login)) = auth::login(
            State(f.state.clone()),
            peer,
            HeaderMap::new(),
            Json(LoginRequest {
                email,
                password: password.clone(),
                mfa_code: None,
                client: Some("token".into()),
            }),
        )
        .await
        .unwrap();
        let next_token = zeroize::Zeroizing::new(login.access_token.unwrap());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/onboarding/setups/{id}"))
                    .header("authorization", format!("Bearer {}", next_token.as_str()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let resumed: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(resumed["id"], created["id"]);
        assert_eq!(resumed["facts"]["name"], "Registered client bot");
        assert!(resumed.get("facts_ciphertext").is_none());
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::platform_installation::COLLECTION_NAME)
                .count_documents(doc! {"owner_user_id": &registered.user_id})
                .await
                .unwrap(),
            0
        );
    }
}
