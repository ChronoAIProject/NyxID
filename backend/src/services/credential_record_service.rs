//! Exact field identities shared by credential readers, revocation preparation
//! and storage migration. Callers supply trusted record/owner identities after
//! their ordinary ACL checks; no identity comes from the envelope header.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::{
        provider_config::ProviderConfig, user_api_key::UserApiKey,
        user_provider_credentials::UserProviderCredentials, user_provider_token::UserProviderToken,
    },
};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

type DecryptFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + 'a>>;

pub(crate) const VERSIONS: &str = "secret_context_versions";
pub(crate) const PLATFORM_OWNER: &str = "platform";
pub(crate) struct CredentialRecord<'a> {
    collection: &'static str,
    id: &'a str,
    owner: &'a str,
    versions: &'a BTreeMap<String, u8>,
}
fn invalid() -> AppError {
    AppError::Internal("Credential field authentication or storage version is invalid".into())
}
fn fields(collection: &str) -> AppResult<&'static [&'static str]> {
    match collection {
        crate::models::provider_config::COLLECTION_NAME
        | crate::models::user_provider_credentials::COLLECTION_NAME => {
            Ok(&["client_id_encrypted", "client_secret_encrypted"])
        }
        crate::models::user_provider_token::COLLECTION_NAME => Ok(&[
            "access_token_encrypted",
            "refresh_token_encrypted",
            "api_key_encrypted",
        ]),
        crate::models::user_api_key::COLLECTION_NAME => Ok(&[
            "credential_encrypted",
            "access_token_encrypted",
            "refresh_token_encrypted",
            "user_oauth_client_id_encrypted",
            "user_oauth_client_secret_encrypted",
        ]),
        _ => Err(invalid()),
    }
}
impl<'a> CredentialRecord<'a> {
    /// Trusted snapshot identity captured after ACL/epoch validation. Decrypt
    /// the captured generation without replacing it with a later live row.
    pub(crate) fn captured_key(
        id: &'a str,
        owner: &'a str,
        versions: &'a BTreeMap<String, u8>,
    ) -> Self {
        Self {
            collection: crate::models::user_api_key::COLLECTION_NAME,
            id,
            owner,
            versions,
        }
    }

    pub(crate) fn provider(row: &'a ProviderConfig) -> Self {
        Self {
            collection: crate::models::provider_config::COLLECTION_NAME,
            id: &row.id,
            owner: PLATFORM_OWNER,
            versions: &row.secret_context_versions,
        }
    }
    pub(crate) fn user_credentials(row: &'a UserProviderCredentials) -> Self {
        Self {
            collection: crate::models::user_provider_credentials::COLLECTION_NAME,
            id: &row.id,
            owner: &row.user_id,
            versions: &row.secret_context_versions,
        }
    }
    pub(crate) fn key(row: &'a UserApiKey) -> Self {
        Self {
            collection: crate::models::user_api_key::COLLECTION_NAME,
            id: &row.id,
            owner: &row.user_id,
            versions: &row.secret_context_versions,
        }
    }
    pub(crate) fn token(row: &'a UserProviderToken) -> Self {
        Self {
            collection: crate::models::user_provider_token::COLLECTION_NAME,
            id: &row.id,
            owner: &row.user_id,
            versions: &row.secret_context_versions,
        }
    }
    pub(crate) fn decrypt<'b>(
        &'b self,
        keys: &'b EncryptionKeys,
        field: &'b str,
        ciphertext: &'b [u8],
    ) -> DecryptFuture<'b> {
        Box::pin(async move {
            if !fields(self.collection)?.contains(&field) {
                return Err(invalid());
            }
            match self.versions.get(field) {
                Some(3) => {
                    keys.decrypt_record(
                        ciphertext,
                        &RecordContext::new(self.collection, self.id, self.owner, &[field])?,
                    )
                    .await
                }
                None if keys.allow_legacy_record_reads() => {
                    keys.decrypt(ciphertext).await.map(Zeroizing::new)
                }
                _ => Err(invalid()),
            }
        })
    }
    pub(crate) async fn text(
        &self,
        keys: &EncryptionKeys,
        field: &str,
        ciphertext: &[u8],
    ) -> AppResult<Zeroizing<String>> {
        let raw = self.decrypt(keys, field, ciphertext).await?;
        let value = std::str::from_utf8(&raw).map_err(|_| invalid())?;
        Ok(Zeroizing::new(value.to_owned()))
    }
}

/// Before writer cutover, preserve exactly the legacy encrypted format. There
/// is no plaintext compatibility write. Partial rotations update only the
/// corresponding version entry, leaving every untouched field's dispatch intact.
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    collection: &'static str,
    id: &str,
    owner: &str,
    field: &str,
    plaintext: &[u8],
) -> AppResult<Vec<u8>> {
    if !fields(collection)?.contains(&field) {
        return Err(invalid());
    }
    if keys.contextual_record_writes() {
        keys.encrypt_record(
            plaintext,
            &RecordContext::new(collection, id, owner, &[field])?,
        )
        .await
    } else {
        keys.encrypt(plaintext).await
    }
}
pub(crate) fn mark_version(
    versions: &mut BTreeMap<String, u8>,
    keys: &EncryptionKeys,
    field: &str,
) {
    if keys.contextual_record_writes() {
        versions.insert(field.into(), 3);
    } else {
        versions.remove(field);
    }
}
/// Classic updates may use a dotted version entry; aggregation pipelines must
/// merge a literal versions object instead. Callers retain their exact-source CAS.
pub(crate) fn mark_classic_update(
    set: &mut bson::Document,
    unset: &mut bson::Document,
    keys: &EncryptionKeys,
    field: &str,
) {
    let path = format!("{VERSIONS}.{field}");
    if keys.contextual_record_writes() {
        set.insert(path, 3_i32);
    } else {
        unset.insert(path, "");
    }
}

/// A storage copy authenticates the producer identity and encrypts the receiver's
/// exact field. Ciphertext is never copied between records or field names.
pub(crate) struct FieldDestination<'a> {
    pub collection: &'static str,
    pub record_id: &'a str,
    pub owner_id: &'a str,
    pub field: &'a str,
}

pub(crate) async fn rebind_optional(
    keys: &EncryptionKeys,
    source: &CredentialRecord<'_>,
    source_field: &str,
    ciphertext: Option<&[u8]>,
    target: FieldDestination<'_>,
    target_versions: &mut BTreeMap<String, u8>,
) -> AppResult<Option<Vec<u8>>> {
    let Some(ciphertext) = ciphertext else {
        target_versions.remove(target.field);
        return Ok(None);
    };
    let plaintext = source.decrypt(keys, source_field, ciphertext).await?;
    if plaintext.is_empty() && optional_client_secret(target.collection, target.field) {
        target_versions.remove(target.field);
        return Ok(None);
    }
    let encrypted = encrypt(
        keys,
        target.collection,
        target.record_id,
        target.owner_id,
        target.field,
        &plaintext,
    )
    .await?;
    mark_version(target_versions, keys, target.field);
    Ok(Some(encrypted))
}

pub(crate) async fn rebind_key_owner(
    keys: &EncryptionKeys,
    row: &UserApiKey,
    target_owner: &str,
) -> AppResult<bson::Document> {
    let source = CredentialRecord::key(row);
    let mut versions = row.secret_context_versions.clone();
    let mut set = bson::Document::new();
    for (field, value) in [
        ("credential_encrypted", row.credential_encrypted.as_deref()),
        (
            "access_token_encrypted",
            row.access_token_encrypted.as_deref(),
        ),
        (
            "refresh_token_encrypted",
            row.refresh_token_encrypted.as_deref(),
        ),
        (
            "user_oauth_client_id_encrypted",
            row.user_oauth_client_id_encrypted.as_deref(),
        ),
        (
            "user_oauth_client_secret_encrypted",
            row.user_oauth_client_secret_encrypted.as_deref(),
        ),
    ] {
        if let Some(value) = rebind_optional(
            keys,
            &source,
            field,
            value,
            FieldDestination {
                collection: crate::models::user_api_key::COLLECTION_NAME,
                record_id: &row.id,
                owner_id: target_owner,
                field,
            },
            &mut versions,
        )
        .await?
        {
            set.insert(
                field,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: value,
                },
            );
        }
    }
    set.insert(VERSIONS, bson::to_bson(&versions).map_err(|_| invalid())?);
    Ok(set)
}

/// Raw storage dispatch for bounded preparation/migration. It uses precisely
/// the same record decoder as typed live readers and preserves the original
/// BSON separately for CAS; no model serialization supplies source evidence.
pub(crate) async fn decrypt_stored_field(
    keys: &EncryptionKeys,
    collection: &'static str,
    row: &bson::Document,
    field: &str,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let id = row.get_str("_id").map_err(|_| invalid())?;
    let owner = if collection == crate::models::provider_config::COLLECTION_NAME {
        PLATFORM_OWNER
    } else {
        row.get_str("user_id").map_err(|_| invalid())?
    };
    let versions = match row.get(VERSIONS) {
        None => BTreeMap::new(),
        Some(value) => bson::from_bson(value.clone()).map_err(|_| invalid())?,
    };
    CredentialRecord {
        collection,
        id,
        owner,
        versions: &versions,
    }
    .decrypt(keys, field, ciphertext)
    .await
}

/// Preserve exact storage shape, including legacy integer arrays and absent
/// version markers. Deserializing and serializing a model is not CAS evidence.
pub(crate) fn exact_source_filter(
    collection: &str,
    row: &bson::Document,
) -> AppResult<bson::Document> {
    let mut clauses = Vec::new();
    for field in [
        "_id",
        "user_id",
        "provider_config_id",
        "credential_user_id",
        "connection_id",
        "credential_epoch",
        "state_version",
        "updated_at",
        "status",
        "oauth_attempt_nonce",
        "token_type",
        "credential_type",
        "token_scopes",
        "expires_at",
        "gateway_url",
        "credential_source",
        "oauth_app_observation",
        "source_id",
        "metadata",
        VERSIONS,
    ]
    .into_iter()
    .chain(fields(collection)?.iter().copied())
    {
        let predicate = match row.get(field) {
            Some(value) => bson::doc! {"$exists":true,"$eq":value},
            None => bson::doc! {"$exists":false},
        };
        let mut clause = bson::Document::new();
        clause.insert(field, predicate);
        clauses.push(clause);
    }
    Ok(bson::doc! {"$and":clauses})
}

/// Reconnects retain the record ID and exact old refresh-field marker when a
/// provider omits refresh. New material is encrypted for that same destination.
pub(crate) async fn oauth_token_fields(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    previous: Option<&UserProviderToken>,
    access: &str,
    refresh: Option<&str>,
) -> AppResult<(BTreeMap<String, u8>, Vec<u8>, Option<Vec<u8>>)> {
    let mut versions = previous
        .map(|row| row.secret_context_versions.clone())
        .unwrap_or_default();
    versions.remove("api_key_encrypted");
    let access = encrypt(
        keys,
        crate::models::user_provider_token::COLLECTION_NAME,
        id,
        owner,
        "access_token_encrypted",
        access.as_bytes(),
    )
    .await?;
    mark_version(&mut versions, keys, "access_token_encrypted");
    let refresh = match refresh.filter(|value| !value.is_empty()) {
        Some(value) => {
            let encrypted = encrypt(
                keys,
                crate::models::user_provider_token::COLLECTION_NAME,
                id,
                owner,
                "refresh_token_encrypted",
                value.as_bytes(),
            )
            .await?;
            mark_version(&mut versions, keys, "refresh_token_encrypted");
            Some(encrypted)
        }
        None => previous.and_then(|row| row.refresh_token_encrypted.clone()),
    };
    Ok((versions, access, refresh))
}

fn stored_versions(row: &bson::Document) -> AppResult<BTreeMap<String, u8>> {
    match row.get(VERSIONS) {
        None => Ok(BTreeMap::new()),
        Some(value) => bson::from_bson(value.clone()).map_err(|_| invalid()),
    }
}
fn stored_bytes(value: &bson::Bson) -> AppResult<Option<Vec<u8>>> {
    #[derive(serde::Deserialize)]
    struct Bytes {
        #[serde(with = "crate::models::bson_bytes::optional")]
        value: Option<Vec<u8>>,
    }
    bson::from_document::<Bytes>(bson::doc! {"value":value})
        .map(|value| value.value)
        .map_err(|_| invalid())
}
fn storage_identity<'a>(
    collection: &str,
    row: &'a bson::Document,
) -> AppResult<(&'a str, &'a str)> {
    let id = row.get_str("_id").map_err(|_| invalid())?;
    let owner = if collection == crate::models::provider_config::COLLECTION_NAME {
        PLATFORM_OWNER
    } else {
        row.get_str("user_id").map_err(|_| invalid())?
    };
    if id.is_empty() || owner.is_empty() {
        return Err(invalid());
    }
    Ok((id, owner))
}
fn optional_client_secret(collection: &str, field: &str) -> bool {
    matches!(
        collection,
        crate::models::provider_config::COLLECTION_NAME
            | crate::models::user_provider_credentials::COLLECTION_NAME
    ) && field == "client_secret_encrypted"
        || collection == crate::models::user_api_key::COLLECTION_NAME
            && field == "user_oauth_client_secret_encrypted"
}

fn validate_field_value(
    collection: &str,
    row: &bson::Document,
    field: &str,
    bytes: &[u8],
) -> AppResult<()> {
    // All four model families store textual credentials. Historical OAuth
    // placeholder rows may contain an encrypted empty primary credential;
    // it is never interpreted as an access token.
    std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let placeholder = collection == crate::models::user_api_key::COLLECTION_NAME
        && field == "credential_encrypted"
        && row.get_str("credential_type") == Ok("oauth2");
    // Empty optional OAuth client secrets were accepted historically for
    // public clients. Authenticate them before normalizing to absence.
    if bytes.is_empty() && !placeholder && !optional_client_secret(collection, field) {
        return Err(invalid());
    }
    Ok(())
}

/// Readiness authenticates every present field with only the selected primary
/// provider. A version marker or a decodable model is not completion evidence.
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    collection: &'static str,
    row: &bson::Document,
) -> AppResult<()> {
    let allowed = fields(collection)?;
    let (id, owner) = storage_identity(collection, row)?;
    let versions = stored_versions(row)?;
    for (field, version) in &versions {
        if *version != 3
            || !allowed.contains(&field.as_str())
            || row
                .get(field)
                .is_none_or(|value| value == &bson::Bson::Null)
        {
            return Err(invalid());
        }
    }
    for field in allowed {
        let Some(value) = row.get(*field) else {
            continue;
        };
        let Some(ciphertext) = stored_bytes(value)? else {
            continue;
        };
        if versions.get(*field) != Some(&3) {
            return Err(invalid());
        }
        let path = [*field];
        let context = RecordContext::new(collection, id, owner, &path)?;
        keys.authenticate_current_record(&ciphertext, &context)
            .await?;
        let plaintext = keys.decrypt_record(&ciphertext, &context).await?;
        validate_field_value(collection, row, field, &plaintext)?;
    }
    Ok(())
}

/// Convert one exact raw BSON generation without replacing authority, epochs,
/// lineage or routine timestamps. Rewrap invalidates prepared storage evidence
/// in the same owner-fenced transaction; plaintext fingerprints are recomputed
/// by the bounded preparation worker through this same field decoder.
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    collection: &'static str,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    // A rerun against authenticated primary-current storage is verification,
    // not another rotation. In particular, do not invalidate erasure dependency
    // preparation merely because a readiness sweep visited this row again.
    if verify_current_snapshot(keys, collection, source)
        .await
        .is_ok()
    {
        return Ok(false);
    }
    let (id, owner) = storage_identity(collection, source)?;
    let allowed = fields(collection)?;
    let mut versions = stored_versions(source)?;
    if versions
        .iter()
        .any(|(field, version)| *version != 3 || !allowed.contains(&field.as_str()))
    {
        return Err(invalid());
    }
    let mut target = source.clone();
    let mut set = bson::Document::new();
    for field in allowed {
        let Some(value) = source.get(*field) else {
            continue;
        };
        let Some(ciphertext) = stored_bytes(value)? else {
            if versions.contains_key(*field) {
                return Err(invalid());
            }
            continue;
        };
        let plaintext = decrypt_stored_field(keys, collection, source, field, &ciphertext).await?;
        validate_field_value(collection, source, field, &plaintext)?;
        if plaintext.is_empty() && optional_client_secret(collection, field) {
            target.insert(*field, bson::Bson::Null);
            set.insert(*field, bson::Bson::Null);
            versions.remove(*field);
            continue;
        }
        let path = [*field];
        let context = RecordContext::new(collection, id, owner, &path)?;
        let encrypted = keys.encrypt_record(&plaintext, &context).await?;
        let value = bson::Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: encrypted,
        });
        target.insert(*field, value.clone());
        set.insert(*field, value);
        versions.insert((*field).to_owned(), 3);
    }
    let version_bson = bson::to_bson(&versions).map_err(|_| invalid())?;
    target.insert(VERSIONS, version_bson.clone());
    set.insert(VERSIONS, version_bson);
    verify_current_snapshot(keys, collection, &target).await?;
    let db = super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<bool> = async {
        // Read/CAS before the owner revision: an obsolete migration attempt
        // never invalidates otherwise current dependency preparation.
        let changed = db
            .collection::<bson::Document>(collection)
            .update_one(
                exact_source_filter(collection, source)?,
                bson::doc! {"$set":set},
            )
            .session(&mut tx)
            .await?;
        if changed.matched_count != 1 {
            return Ok(false);
        }
        if owner != PLATFORM_OWNER {
            super::erasure_admission::credential_change_in_session(&db, owner, &mut tx).await?;
        }
        Ok(true)
    }
    .await;
    super::org_membership_authority::finish(tx, result).await
}

#[cfg(test)]
mod tests;
