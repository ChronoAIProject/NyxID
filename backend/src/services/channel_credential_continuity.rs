//! A retained pin identifies one credential generation. Its authentication covers
//! the exact producer envelopes while the pin's dependencies survive a rewrap.
use crate::{
    errors::{AppError, AppResult},
    models::channel_bot::{ChannelBot, CredentialContinuity, RetainedCredentialPin},
};
use bson::{Bson, Document, doc};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) const TOKEN: &str = "bot_token_encrypted";
pub(super) const APP: &str = "app_secret_encrypted";
pub(super) const VERIFIED: [&str; 4] = [
    TOKEN,
    APP,
    "lark_verification_token_encrypted",
    "lark_encrypt_key_encrypted",
];
const FIELDS: [&str; 6] = [
    TOKEN,
    APP,
    VERIFIED[2],
    VERIFIED[3],
    "registration_pin_encrypted",
    "webhook_secret_encrypted",
];

fn invalid() -> AppError {
    AppError::Conflict("Bot credentials or verification changed. Run verification again.".into())
}

fn digest(material: &impl Serialize) -> AppResult<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(material).map_err(|_| invalid())?,
    )))
}

fn authentication(bot: &ChannelBot, evidence: &CredentialContinuity) -> AppResult<Hmac<Sha256>> {
    let version = if evidence.shared_producers.is_empty() {
        1
    } else {
        2
    };
    if evidence.version != version || evidence.generations.len() != FIELDS.len() {
        return Err(invalid());
    }
    let mut fields = Vec::new();
    for field in FIELDS {
        let generation = evidence.generations.get(field).ok_or_else(invalid)?;
        let bytes = super::channel_registration::stored_secret(bot, field)?;
        fields.push((
            field,
            generation,
            bot.secret_context_versions.get(field),
            bytes.map(Sha256::digest).map(hex::encode),
        ));
    }
    let material = serde_json::to_vec(&(
        1,
        "channel_bots",
        &bot.id,
        &bot.user_id,
        bot.ownership_version,
        fields,
        &evidence.pins,
    ))
    .map_err(|_| invalid())?;
    if evidence.version == 2 {
        let mut mac = continuity_mac(b"nyxid-channel-credential-continuity:v2", &material)?;
        mac.update(&serde_json::to_vec(&evidence.shared_producers).map_err(|_| invalid())?);
        return Ok(mac);
    }
    continuity_mac(b"nyxid-channel-credential-continuity:v1", &material)
}

pub(super) fn continuity_mac(domain: &[u8], material: &[u8]) -> AppResult<Hmac<Sha256>> {
    let key = super::audit_service::audit_chain_hmac_key().ok_or_else(invalid)?;
    let mut derive = Hmac::<Sha256>::new_from_slice(key).map_err(|_| invalid())?;
    derive.update(domain);
    let subkey = zeroize::Zeroizing::new(<[u8; 32]>::from(derive.finalize().into_bytes()));
    let mut mac = Hmac::<Sha256>::new_from_slice(subkey.as_slice()).map_err(|_| invalid())?;
    mac.update(material);
    Ok(mac)
}

pub(super) fn verified(bot: &ChannelBot) -> AppResult<Option<&CredentialContinuity>> {
    let Some(evidence) = &bot.credential_continuity else {
        return Ok(None);
    };
    let tag = hex::decode(&evidence.authentication).map_err(|_| invalid())?;
    authentication(bot, evidence)?
        .verify_slice(&tag)
        .map_err(|_| invalid())?;
    Ok(Some(evidence))
}

pub(super) fn dependencies(
    bot: &ChannelBot,
    purpose: &str,
    fields: &[&str],
    other: &impl Serialize,
    evidence: &CredentialContinuity,
) -> AppResult<String> {
    let gens = fields
        .iter()
        .map(|f| evidence.generations.get(*f).ok_or_else(invalid))
        .collect::<AppResult<Vec<_>>>()?;
    digest(&(
        "nyxid-channel-credential-generation:v1",
        purpose,
        &bot.id,
        &bot.user_id,
        bot.ownership_version,
        &bot.platform,
        &bot.platform_bot_id,
        gens,
        other,
    ))
}

pub(super) fn select(
    evidence: &CredentialContinuity,
    purpose: &str,
    dependencies: String,
) -> String {
    evidence
        .pins
        .get(purpose)
        .filter(|pin| pin.dependencies == dependencies)
        .map(|pin| pin.value.clone())
        .unwrap_or(dependencies)
}

pub(super) fn field_pin(bot: &ChannelBot, field: &str) -> AppResult<String> {
    let bytes = super::channel_registration::stored_secret(bot, field)?.ok_or_else(invalid)?;
    if let Some(evidence) = verified(bot)? {
        return Ok(select(
            evidence,
            field,
            dependencies(bot, field, &[field], &(), evidence)?,
        ));
    }
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn physical_filter(bot: &ChannelBot) -> AppResult<Document> {
    let mut filter = doc! {"credential_continuity":bson::to_bson(&bot.credential_continuity).map_err(|_|invalid())?,
    "ownership_version":if bot.ownership_version==0 { Bson::Document(doc!{"$in":[0_i64,Bson::Null]}) } else {bot.ownership_version.into()}};
    for field in FIELDS {
        if bot.credential_continuity.is_some() {
            filter.insert(
                field,
                super::channel_registration::stored_secret(bot, field)?.map(|bytes| bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: bytes.to_vec(),
                }),
            );
        }
        filter.insert(
            format!("secret_context_versions.{field}"),
            bot.secret_context_versions
                .get(field)
                .map(|n| i32::from(*n)),
        );
    }
    Ok(filter)
}

fn sign(bot: &ChannelBot, evidence: &mut CredentialContinuity) -> AppResult<()> {
    evidence.authentication = hex::encode(authentication(bot, evidence)?.finalize().into_bytes());
    Ok(())
}

/// Prepared rows can still be updated with compatible legacy envelopes after writer rollback.
pub(super) fn replacement(
    bot: &ChannelBot,
    set: &mut Document,
    unset: &mut Document,
) -> AppResult<()> {
    let Some(previous) = verified(bot)? else {
        return Ok(());
    };
    let mut evidence = previous.clone();
    let mut target = bson::to_document(bot).map_err(|_| invalid())?;
    for field in FIELDS {
        if !set.contains_key(field) && !unset.contains_key(field) {
            continue;
        }
        if let Some(value) = set.get(field) {
            target.insert(field, value.clone());
        } else {
            target.remove(field);
        }
        evidence
            .generations
            .insert(field.into(), uuid::Uuid::new_v4().to_string());
        if let Ok(versions) = target.get_document_mut("secret_context_versions") {
            versions.remove(field);
        }
        unset.insert(format!("secret_context_versions.{field}"), "");
    }
    let updated = bson::from_document(target).map_err(|_| invalid())?;
    sign(&updated, &mut evidence)?;
    set.insert(
        "credential_continuity",
        bson::to_bson(&evidence).map_err(|_| invalid())?,
    );
    Ok(())
}

/// Capture the values issued by the current readers before changing their dependencies.
pub(super) fn capture(
    entry: &super::platform_entry_credentials::EntryCredentialSnapshot,
    sources: Option<BTreeMap<String, String>>,
) -> AppResult<CredentialContinuity> {
    let verify = entry.channel_snapshot();
    let bot = verify.bot();
    let mut evidence = match verified(bot)? {
        Some(previous) => previous.clone(),
        None => {
            let mut evidence = CredentialContinuity {
                version: 1,
                generations: FIELDS
                    .iter()
                    .map(|name| (name.to_string(), uuid::Uuid::new_v4().to_string()))
                    .collect(),
                pins: BTreeMap::new(),
                shared_producers: BTreeMap::new(),
                authentication: String::new(),
            };
            for name in [TOKEN, APP] {
                if super::channel_registration::stored_secret(bot, name)?.is_some() {
                    evidence.pins.insert(
                        name.into(),
                        RetainedCredentialPin {
                            dependencies: dependencies(bot, name, &[name], &(), &evidence)?,
                            value: field_pin(bot, name)?,
                        },
                    );
                }
            }
            evidence
        }
    };
    let shared = sources.is_some();
    if let Some(sources) = sources {
        evidence.version = 2;
        evidence.shared_producers = sources;
    }
    if shared || bot.credential_continuity.is_none() {
        let verification = verify.fingerprint()?;
        let native = entry.fingerprint()?;
        let verification_key = verify.pin_name(shared);
        let native_key = entry.pin_name(shared);
        let vd = verify.dependencies(&evidence, verification_key)?;
        let nd = entry.pin_dependencies(&evidence, &verification, native_key)?;
        evidence.pins.insert(
            verification_key.into(),
            RetainedCredentialPin {
                dependencies: vd,
                value: verification,
            },
        );
        evidence.pins.insert(
            native_key.into(),
            RetainedCredentialPin {
                dependencies: nd,
                value: native,
            },
        );
    }
    sign(bot, &mut evidence)?;
    Ok(evidence)
}

/// Run inside the exact-source write transaction. The registry keeps channel_bots
/// pending until all business writers and owner transfer can seal contextual rows.
pub(super) async fn publish_rewrap(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    source: &Document,
    mut set: Document,
) -> AppResult<bool> {
    use super::platform_entry_credentials::EntryCredentialSnapshot;
    if !keys.contextual_record_writes()
        || set
            .keys()
            .any(|field| field != "secret_context_versions" && !FIELDS.contains(&field.as_str()))
    {
        return Err(invalid());
    }
    let bot: ChannelBot = bson::from_document(source.clone()).map_err(|_| invalid())?;
    let cache =
        std::sync::Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    let adapter = super::channel_adapters::resolve_adapter(&bot.platform, &cache)?;
    let entry = EntryCredentialSnapshot::load(db, adapter.as_ref(), &bot).await?;
    let db = super::transaction_service::durable_database(db);
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let result = async {
        let mut target = source.clone();
        target.extend(set.clone());
        let updated: ChannelBot = bson::from_document(target).map_err(|_| invalid())?;
        for name in FIELDS {
            if name == TOKEN
                && bot.bot_token_encrypted.is_empty()
                && matches!(
                    bot.credential_source.as_str(),
                    "connection" | "telegram_manager"
                )
                && !bot.secret_context_versions.contains_key(name)
                && !set.contains_key(name)
            {
                continue;
            }
            let before = super::channel_registration::open_secret(keys, &bot, name).await?;
            if set.contains_key(name)
                && before != super::channel_registration::open_secret(keys, &updated, name).await?
            {
                return Err(invalid());
            }
        }
        let mut evidence = capture(&entry, None)?;
        sign(&updated, &mut evidence)?;
        set.insert(
            "credential_continuity",
            bson::to_bson(&evidence).map_err(|_| invalid())?,
        );
        let changed = db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                super::record_storage::exact_document_predicate(source),
                doc! {"$set":set},
            )
            .session(&mut session)
            .await?
            .matched_count
            == 1;
        if changed && !entry.fence_sources_for_rewrap(&db, &mut session).await? {
            return Err(invalid());
        }
        Ok(changed)
    }
    .await;
    super::org_membership_authority::finish(session, result).await
}

pub(super) async fn transfer(
    bot: &ChannelBot,
    destination: &str,
    keys: Option<&crate::crypto::aes::EncryptionKeys>,
) -> AppResult<Document> {
    let mut evidence = verified(bot)?.cloned().ok_or_else(invalid)?;
    let keys = keys.ok_or_else(|| {
        AppError::Conflict("Credential encryption is required for this transfer".into())
    })?;
    let mut target = bson::to_document(bot).map_err(|_| invalid())?;
    target.insert("user_id", destination);
    target.insert(
        "ownership_version",
        bot.ownership_version.checked_add(1).ok_or_else(invalid)?,
    );
    let mut set = Document::new();
    let mut versions = Document::new();
    for name in FIELDS {
        evidence
            .generations
            .insert(name.into(), uuid::Uuid::new_v4().to_string());
        if name == TOKEN
            && bot.bot_token_encrypted.is_empty()
            && matches!(
                bot.credential_source.as_str(),
                "connection" | "telegram_manager"
            )
            && !bot.secret_context_versions.contains_key(name)
        {
            continue;
        }
        if let Some(plain) = super::channel_registration::open_secret(keys, bot, name).await? {
            let encrypted = crate::crypto::record_field::encrypt(
                keys,
                "channel_bots",
                &bot.id,
                destination,
                name,
                &plain,
            )
            .await?;
            set.insert(
                name,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
            );
            if keys.contextual_record_writes() {
                versions.insert(name, 3);
            }
        }
    }
    set.insert("secret_context_versions", versions);
    target.extend(set.clone());
    evidence.pins.clear();
    sign(
        &bson::from_document(target).map_err(|_| invalid())?,
        &mut evidence,
    )?;
    set.insert(
        "credential_continuity",
        bson::to_bson(&evidence).map_err(|_| invalid())?,
    );
    Ok(set)
}

#[cfg(test)]
pub(super) async fn rewrap_fixture(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    bot: &ChannelBot,
    fields: &[&str],
) -> ChannelBot {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let coll = crate::models::channel_bot::COLLECTION_NAME;
    let rows = db.collection::<Document>(coll);
    let source = rows.find_one(doc! {"_id":&bot.id}).await.unwrap().unwrap();
    assert!(
        super::record_field_service::migrate_snapshot(
            db,
            keys,
            coll,
            &source,
            &bot.user_id,
            fields
        )
        .await
        .unwrap()
    );
    super::channel_bot_service::get_bot(db, &bot.id)
        .await
        .unwrap()
}
