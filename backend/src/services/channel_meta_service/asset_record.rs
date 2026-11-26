//! Storage for the eligible assets returned by one Meta consent attempt.
use super::{AssetCredential, OAuthSession, SESSIONS, binary, conflict, meta, provider};
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    services::{record_storage, transaction_service},
};
use bson::{Document, doc};
use mongodb::Database;
use std::collections::HashSet;

const FIELD: &str = "assets_encrypted";
const VERSION: &str = "assets_context_version";
pub(in crate::services) const AMBIGUOUS: &str = "meta_consent_assets_ambiguous";

fn ambiguous() -> AppError {
    AppError::Conflict(AMBIGUOUS.into())
}

// JSON distinguishes absent optional identities from every possible string value.
fn attempt(s: &OAuthSession) -> AppResult<String> {
    serde_json::to_string(&(
        s.created_at.timestamp_millis(),
        &s.bot_id,
        &s.reconnect_bot_id,
        s.expected_generation,
    ))
    .map_err(|_| conflict())
}

fn path<'a>(s: &'a OAuthSession, subject: &'a str, attempt: &'a str) -> [&'a str; 7] {
    [
        FIELD,
        &s.actor_id,
        &s.platform,
        &s.app_id,
        subject,
        &s.state_hash,
        attempt,
    ]
}

fn check_assets(s: &OAuthSession, subject: &str, assets: &[AssetCredential]) -> AppResult<()> {
    if !meta::valid_id(subject)
        || assets.is_empty()
        || !matches!(s.platform.as_str(), "instagram" | "messenger")
        || (s.platform == "instagram" && assets.len() != 1)
    {
        return Err(ambiguous());
    }
    if let Some(stored) = &s.grant_identity
        && *stored != provider::grant_identity(&s.platform, &s.app_id, subject)?
    {
        return Err(ambiguous());
    }
    let mut ids = HashSet::new();
    for asset in assets {
        if !meta::valid_id(&asset.id)
            || !ids.insert(asset.id.as_str())
            || asset.grant_subject_id != subject
            || asset.token.is_empty()
            || asset.expires_at.is_some() != (s.platform == "instagram")
        {
            return Err(ambiguous());
        }
    }
    if s.selected_asset_id
        .as_deref()
        .is_some_and(|id| !ids.contains(id))
    {
        return Err(ambiguous());
    }
    Ok(())
}

pub(super) async fn publication(
    keys: &EncryptionKeys,
    session: &OAuthSession,
    subject: &str,
    assets: &[AssetCredential],
) -> AppResult<Document> {
    check_assets(session, subject, assets).map_err(|_| conflict())?;
    let plain = zeroize::Zeroizing::new(serde_json::to_vec(assets).map_err(|_| conflict())?);
    let attempt = attempt(session)?;
    let path = path(session, subject, &attempt);
    let context = RecordContext::new(SESSIONS, &session.id, &session.owner_id, &path)?;
    let ciphertext = record_field::encrypt_context(keys, &context, &plain).await?;
    let mut fields = doc! {
        "status": "assets",
        "grant_subject_id": subject,
        "grant_identity": provider::grant_identity(&session.platform, &session.app_id, subject)?,
        FIELD: binary(ciphertext),
    };
    if keys.contextual_record_writes() {
        fields.insert(VERSION, 3);
        Ok(doc! {"$set": fields})
    } else {
        Ok(doc! {"$set": fields, "$unset": {VERSION: ""}})
    }
}

fn decode(session: &OAuthSession, subject: &str, bytes: &[u8]) -> AppResult<Vec<AssetCredential>> {
    let assets: Vec<AssetCredential> = serde_json::from_slice(bytes).map_err(|_| ambiguous())?;
    check_assets(session, subject, &assets)?;
    Ok(assets)
}

pub(super) async fn open(
    keys: &EncryptionKeys,
    session: &OAuthSession,
    subject: &str,
) -> AppResult<Vec<AssetCredential>> {
    let attempt = attempt(session)?;
    let path = path(session, subject, &attempt);
    let context = RecordContext::new(SESSIONS, &session.id, &session.owner_id, &path)?;
    let bytes = record_field::decrypt_context(
        keys,
        &context,
        session.assets_context_version,
        session.assets_encrypted.as_deref().ok_or_else(conflict)?,
    )
    .await?;
    decode(session, subject, &bytes).map_err(|_| conflict())
}

async fn inspect(
    keys: &EncryptionKeys,
    source: &Document,
    rewrap: bool,
) -> AppResult<Option<Vec<u8>>> {
    let session: OAuthSession = bson::from_document(source.clone()).map_err(|_| ambiguous())?;
    let Some(ciphertext) = session.assets_encrypted.as_deref() else {
        return if session.assets_context_version.is_none()
            && matches!(
                session.status.as_str(),
                "pending"
                    | "exchanging"
                    | "no_eligible_assets"
                    | "completed"
                    | "failed"
                    | "cancelled"
                    | "revoked"
            ) {
            Ok(None)
        } else {
            Err(ambiguous())
        };
    };
    if !matches!(
        (session.status.as_str(), session.selected_asset_id.is_some()),
        ("assets", false) | ("installing", true)
    ) {
        return Err(ambiguous());
    }
    let subject = session.grant_subject_id.as_deref().ok_or_else(ambiguous)?;
    let attempt = attempt(&session)?;
    let path = path(&session, subject, &attempt);
    let context = RecordContext::new(SESSIONS, &session.id, &session.owner_id, &path)?;
    let primary = session.assets_context_version == Some(3)
        && keys
            .authenticate_current_record(ciphertext, &context)
            .await
            .is_ok();
    let plain =
        record_field::decrypt_context(keys, &context, session.assets_context_version, ciphertext)
            .await?;
    decode(&session, subject, &plain)?;
    if primary {
        return Ok(None);
    }
    if !rewrap {
        return Err(conflict());
    }
    let next = keys.encrypt_record(&plain, &context).await?;
    keys.authenticate_current_record(&next, &context).await?;
    Ok(Some(next))
}

pub(in crate::services) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    inspect(keys, source, false).await.map(drop)
}

pub(in crate::services) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let Some(ciphertext) = inspect(keys, source, true).await? else {
        return Ok(false);
    };
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(SESSIONS)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set": {
                FIELD: binary(ciphertext),
                VERSION: 3,
            }},
        )
        .await?
        .matched_count
        == 1)
}
