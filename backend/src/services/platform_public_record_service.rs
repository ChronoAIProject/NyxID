//! Server-owned public-session credentials and disposable turn payloads. Empty
//! lifecycle fields carry no ciphertext; they never prove an upstream effect
//! absent. Authority checks remain with the public-session services.
use super::record_field_service as fields;
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::{
        platform_public_session::PlatformPublicSession, platform_public_turn::PlatformPublicTurn,
    },
};
use zeroize::Zeroizing;

fn invalid() -> AppError {
    AppError::Internal("Public-session encrypted storage is invalid or unresolved".into())
}

pub(crate) async fn encrypt_session(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    key_id: &str,
    field: &str,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    if !matches!(
        field,
        "credential_ciphertext" | "upstream_binding_ciphertext"
    ) || bytes.is_empty()
    {
        return Err(invalid());
    }
    fields::encrypt_context(
        keys,
        &RecordContext::new(
            crate::models::platform_public_session::COLLECTION_NAME,
            id,
            owner,
            &[field, "api_key", key_id],
        )?,
        bytes,
    )
    .await
}

pub(crate) async fn decrypt_session(
    keys: &EncryptionKeys,
    row: &PlatformPublicSession,
    field: &str,
) -> AppResult<Zeroizing<String>> {
    let ciphertext = match field {
        "credential_ciphertext" => &row.credential_ciphertext,
        "upstream_binding_ciphertext" => &row.upstream_binding_ciphertext,
        _ => return Err(invalid()),
    };
    if ciphertext.is_empty() {
        return Err(invalid());
    }
    let bytes = fields::decrypt_context(
        keys,
        &RecordContext::new(
            crate::models::platform_public_session::COLLECTION_NAME,
            &row.id,
            &row.sponsor_owner_user_id,
            &[field, "api_key", &row.credential_api_key_id],
        )?,
        row.secret_context_versions.get(field).copied(),
        ciphertext,
    )
    .await?;
    let value = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    if value.is_empty() {
        return Err(invalid());
    }
    Ok(value.to_owned().into())
}

pub(crate) struct TurnIdentity<'a> {
    pub id: &'a str,
    pub owner: &'a str,
    pub session: &'a str,
    pub conversation: &'a str,
    pub receipt: &'a str,
    pub claim: &'a str,
}
impl<'a> From<&'a PlatformPublicTurn> for TurnIdentity<'a> {
    fn from(row: &'a PlatformPublicTurn) -> Self {
        Self {
            id: &row.id,
            owner: &row.sponsor_owner_user_id,
            session: &row.public_session_id,
            conversation: &row.conversation_id,
            receipt: &row.receipt_id,
            claim: &row.claim_id,
        }
    }
}

pub(crate) async fn encrypt_turn(
    keys: &EncryptionKeys,
    identity: TurnIdentity<'_>,
    field: &str,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    let TurnIdentity {
        id,
        owner,
        session,
        conversation,
        receipt,
        claim,
    } = identity;
    if !matches!(field, "input_ciphertext" | "reply_ciphertext") {
        return Err(invalid());
    }
    fields::encrypt_context(
        keys,
        &RecordContext::new(
            crate::models::platform_public_turn::COLLECTION_NAME,
            id,
            owner,
            &[
                field,
                "session",
                session,
                "conversation",
                conversation,
                "receipt",
                receipt,
                "claim",
                claim,
            ],
        )?,
        bytes,
    )
    .await
}

pub(crate) async fn decrypt_turn(
    keys: &EncryptionKeys,
    row: &PlatformPublicTurn,
    field: &str,
) -> AppResult<Zeroizing<String>> {
    let ciphertext = match field {
        "input_ciphertext" => &row.input_ciphertext,
        "reply_ciphertext" => &row.reply_ciphertext,
        _ => return Err(invalid()),
    };
    if ciphertext.is_empty() {
        return Err(invalid());
    }
    let bytes = fields::decrypt_context(
        keys,
        &RecordContext::new(
            crate::models::platform_public_turn::COLLECTION_NAME,
            &row.id,
            &row.sponsor_owner_user_id,
            &[
                field,
                "session",
                &row.public_session_id,
                "conversation",
                &row.conversation_id,
                "receipt",
                &row.receipt_id,
                "claim",
                &row.claim_id,
            ],
        )?,
        row.secret_context_versions.get(field).copied(),
        ciphertext,
    )
    .await?;
    Ok(std::str::from_utf8(&bytes)
        .map_err(|_| invalid())?
        .to_owned()
        .into())
}

pub(crate) fn mark_update(
    contextual: bool,
    update: &mut bson::Document,
    field: &str,
) -> AppResult<()> {
    let path = format!("secret_context_versions.{field}");
    let operator = if contextual { "$set" } else { "$unset" };
    if !update.contains_key(operator) {
        update.insert(operator, bson::Document::new());
    }
    update
        .get_document_mut(operator)
        .map_err(|_| invalid())?
        .insert(
            path,
            if contextual {
                bson::Bson::Int32(3)
            } else {
                bson::Bson::Int32(1)
            },
        );
    Ok(())
}

fn validate_versions(source: &bson::Document, fields: &[&str]) -> AppResult<()> {
    for (field, version) in super::record_field_service::stored_versions(source)? {
        if version != 3 || !fields.contains(&field.as_str()) {
            return Err(invalid());
        }
        // Empty terminal fields may retain their last marker. They are
        // validated against actual lifecycle state separately below.
    }
    Ok(())
}

async fn verify_session(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    row: &PlatformPublicSession,
) -> AppResult<()> {
    if row.credential_ciphertext.is_empty() {
        if row.revoked_at.is_none() {
            return Err(invalid());
        }
    } else {
        let credential = decrypt_session(keys, row, "credential_ciphertext").await?;
        let key = db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).find_one(bson::doc!{"_id":&row.credential_api_key_id,"user_id":&row.sponsor_owner_user_id,"key_hash":crate::crypto::token::hash_token(&credential)}).projection(bson::doc!{"_id":1}).await?;
        if key.is_none() {
            return Err(invalid());
        }
    }
    if row.upstream_binding_ciphertext.is_empty() {
        if row.upstream_may_exist {
            return Err(invalid());
        }
    } else {
        let encoded = decrypt_session(keys, row, "upstream_binding_ciphertext").await?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Binding {
            session_id: Option<Zeroizing<String>>,
            response_id: Option<Zeroizing<String>>,
        }
        let binding: Binding = serde_json::from_str(&encoded).map_err(|_| invalid())?;
        validate_binding_handles(
            binding.session_id.as_ref().map(|s| s.as_str()),
            binding.response_id.as_ref().map(|s| s.as_str()),
        )?;
    }
    Ok(())
}
pub(crate) fn validate_binding_handles(
    session: Option<&str>,
    response: Option<&str>,
) -> AppResult<()> {
    if (session.is_none() && response.is_none())
        || session.is_some_and(|value| !valid_handle(value))
        || response.is_some_and(|value| !valid_handle(value))
    {
        return Err(invalid());
    }
    Ok(())
}

fn valid_handle(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Authenticated primary-current evidence. A live unknown upstream obligation
/// blocks readiness independently of whether its local bytes are empty.
pub(crate) async fn verify_current_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &bson::Document,
) -> AppResult<()> {
    if collection == crate::models::platform_public_session::COLLECTION_NAME {
        validate_versions(
            source,
            &["credential_ciphertext", "upstream_binding_ciphertext"],
        )?;
        let row: PlatformPublicSession =
            bson::from_document(source.clone()).map_err(|_| invalid())?;
        verify_session(db, keys, &row).await?;
        for (field, ciphertext) in [
            ("credential_ciphertext", &row.credential_ciphertext),
            (
                "upstream_binding_ciphertext",
                &row.upstream_binding_ciphertext,
            ),
        ] {
            if ciphertext.is_empty() {
                continue;
            }
            if row.secret_context_versions.get(field) != Some(&3) {
                return Err(invalid());
            }
            keys.authenticate_current_record(
                ciphertext,
                &RecordContext::new(
                    collection,
                    &row.id,
                    &row.sponsor_owner_user_id,
                    &[field, "api_key", &row.credential_api_key_id],
                )?,
            )
            .await?;
        }
    } else if collection == crate::models::platform_public_turn::COLLECTION_NAME {
        validate_versions(source, &["input_ciphertext", "reply_ciphertext"])?;
        let row: PlatformPublicTurn = bson::from_document(source.clone()).map_err(|_| invalid())?;
        if row.input_ciphertext.is_empty()
            || (row.reply_ciphertext.is_empty() && row.reply_id.is_some())
        {
            return Err(invalid());
        }
        // Join the authoritative session, preserving the real sponsor boundary.
        if db.collection::<bson::Document>(crate::models::platform_public_session::COLLECTION_NAME).count_documents(bson::doc!{"_id":&row.public_session_id,"sponsor_owner_user_id":&row.sponsor_owner_user_id}).limit(1).await? != 1 { return Err(invalid()); }
        for (field, ciphertext) in [
            ("input_ciphertext", &row.input_ciphertext),
            ("reply_ciphertext", &row.reply_ciphertext),
        ] {
            if ciphertext.is_empty() {
                continue;
            }
            decrypt_turn(keys, &row, field).await?;
            if row.secret_context_versions.get(field) != Some(&3) {
                return Err(invalid());
            }
            keys.authenticate_current_record(
                ciphertext,
                &RecordContext::new(
                    collection,
                    &row.id,
                    &row.sponsor_owner_user_id,
                    &[
                        field,
                        "session",
                        &row.public_session_id,
                        "conversation",
                        &row.conversation_id,
                        "receipt",
                        &row.receipt_id,
                        "claim",
                        &row.claim_id,
                    ],
                )?,
            )
            .await?;
        }
    } else {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer activation is required".into(),
        ));
    }
    if verify_current_snapshot(db, keys, collection, source)
        .await
        .is_ok()
    {
        return Ok(false);
    }
    let mut set = bson::Document::new();
    if collection == crate::models::platform_public_session::COLLECTION_NAME {
        validate_versions(
            source,
            &["credential_ciphertext", "upstream_binding_ciphertext"],
        )?;
        let row: PlatformPublicSession =
            bson::from_document(source.clone()).map_err(|_| invalid())?;
        verify_session(db, keys, &row).await?;
        for (field, ciphertext) in [
            ("credential_ciphertext", &row.credential_ciphertext),
            (
                "upstream_binding_ciphertext",
                &row.upstream_binding_ciphertext,
            ),
        ] {
            if ciphertext.is_empty() {
                continue;
            }
            let plain = decrypt_session(keys, &row, field).await?;
            let bytes = encrypt_session(
                keys,
                &row.id,
                &row.sponsor_owner_user_id,
                &row.credential_api_key_id,
                field,
                plain.as_bytes(),
            )
            .await?;
            set.insert(
                field,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes,
                },
            );
            set.insert(format!("secret_context_versions.{field}"), 3_i32);
        }
    } else if collection == crate::models::platform_public_turn::COLLECTION_NAME {
        validate_versions(source, &["input_ciphertext", "reply_ciphertext"])?;
        let row: PlatformPublicTurn = bson::from_document(source.clone()).map_err(|_| invalid())?;
        if row.input_ciphertext.is_empty()
            || (row.reply_ciphertext.is_empty() && row.reply_id.is_some())
        {
            return Err(invalid());
        }
        for (field, ciphertext) in [
            ("input_ciphertext", &row.input_ciphertext),
            ("reply_ciphertext", &row.reply_ciphertext),
        ] {
            if ciphertext.is_empty() {
                continue;
            }
            let plain = decrypt_turn(keys, &row, field).await?;
            let bytes = encrypt_turn(
                keys,
                TurnIdentity {
                    id: &row.id,
                    owner: &row.sponsor_owner_user_id,
                    session: &row.public_session_id,
                    conversation: &row.conversation_id,
                    receipt: &row.receipt_id,
                    claim: &row.claim_id,
                },
                field,
                plain.as_bytes(),
            )
            .await?;
            set.insert(
                field,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes,
                },
            );
            set.insert(format!("secret_context_versions.{field}"), 3_i32);
        }
    } else {
        return Err(invalid());
    }
    if set.is_empty() {
        return Ok(false);
    }
    let mut candidate = source.clone();
    // Verify the exact candidate through the same typed adapters before CAS.
    let mut versions = super::record_field_service::stored_versions(source)?;
    for (field, value) in &set {
        if let Some(field) = field.strip_prefix("secret_context_versions.") {
            versions.insert(field.to_owned(), 3);
        } else {
            candidate.insert(field, value.clone());
        }
    }
    candidate.insert(
        "secret_context_versions",
        bson::to_bson(&versions).map_err(|_| invalid())?,
    );
    verify_current_snapshot(db, keys, collection, &candidate).await?;
    let db = super::transaction_service::durable_database(db);
    Ok(db
        .collection::<bson::Document>(collection)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            bson::doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1)
}

pub(crate) fn initial_versions(
    keys: &EncryptionKeys,
    field: &str,
) -> std::collections::BTreeMap<String, u8> {
    let mut versions = Default::default();
    super::credential_record_service::mark_version(&mut versions, keys, field);
    versions
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[tokio::test]
    async fn legacy_arrays_and_contextual_public_payloads_preserve_identity_and_terminal_obligations()
     {
        for binding in [
            br#"{"session_id":"session_1","response_id":"response_1"}"#.as_slice(),
            br#"{"session_id":null,"response_id":"response_only_1"}"#.as_slice(),
        ] {
            let db =
                crate::test_utils::connect_transaction_test_database("public_record_migration")
                    .await;
            let mut keys = crate::test_utils::test_encryption_keys();
            let (mut row, mut key, _) =
                super::super::platform_public_credential_service::tests::fixture();
            let raw = "distinctive-public-agent-key";
            key.key_hash = crate::crypto::token::hash_token(raw);
            row.credential_ciphertext = keys.encrypt(raw.as_bytes()).await.unwrap();
            row.upstream_binding_ciphertext = keys.encrypt(binding).await.unwrap();
            row.upstream_may_exist = true;
            db.collection::<crate::models::api_key::ApiKey>(
                crate::models::api_key::COLLECTION_NAME,
            )
            .insert_one(&key)
            .await
            .unwrap();
            let mut source = bson::to_document(&row).unwrap();
            source.remove("secret_context_versions");
            // Historical integer arrays must remain exact in the migration CAS.
            source.insert(
                "credential_ciphertext",
                bson::Bson::Array(
                    row.credential_ciphertext
                        .iter()
                        .map(|byte| bson::Bson::Int32(i32::from(*byte)))
                        .collect(),
                ),
            );
            let sessions = db.collection::<bson::Document>(
                crate::models::platform_public_session::COLLECTION_NAME,
            );
            sessions.insert_one(&source).await.unwrap();
            assert!(
                migrate_snapshot(
                    &db,
                    &keys,
                    crate::models::platform_public_session::COLLECTION_NAME,
                    &source
                )
                .await
                .unwrap()
            );
            let current = sessions
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            verify_current_snapshot(
                &db,
                &keys,
                crate::models::platform_public_session::COLLECTION_NAME,
                &current,
            )
            .await
            .unwrap();
            assert!(
                !migrate_snapshot(
                    &db,
                    &keys,
                    crate::models::platform_public_session::COLLECTION_NAME,
                    &current
                )
                .await
                .unwrap()
            );
            keys.set_allow_legacy_record_reads(false);
            let migrated: PlatformPublicSession = bson::from_document(current.clone()).unwrap();
            assert_eq!(
                decrypt_session(&keys, &migrated, "credential_ciphertext")
                    .await
                    .unwrap()
                    .as_str(),
                raw
            );
            assert_eq!(
                decrypt_session(&keys, &migrated, "upstream_binding_ciphertext")
                    .await
                    .unwrap()
                    .as_bytes(),
                binding
            );
            let mut no_handles = migrated.clone();
            no_handles.upstream_binding_ciphertext = encrypt_session(
                &keys,
                &no_handles.id,
                &no_handles.sponsor_owner_user_id,
                &no_handles.credential_api_key_id,
                "upstream_binding_ciphertext",
                br#"{"session_id":null,"response_id":null}"#,
            )
            .await
            .unwrap();
            assert!(
                verify_current_snapshot(
                    &db,
                    &keys,
                    crate::models::platform_public_session::COLLECTION_NAME,
                    &bson::to_document(&no_handles).unwrap()
                )
                .await
                .is_err()
            );
            let mut swapped = migrated.clone();
            swapped.credential_api_key_id = uuid::Uuid::new_v4().to_string();
            assert!(
                decrypt_session(&keys, &swapped, "credential_ciphertext")
                    .await
                    .is_err()
            );
            let mut unresolved = migrated.clone();
            unresolved.revoked_at = Some(chrono::Utc::now());
            unresolved.credential_ciphertext.clear();
            unresolved.upstream_binding_ciphertext.clear();
            let unknown = bson::to_document(&unresolved).unwrap();
            assert!(
                verify_current_snapshot(
                    &db,
                    &keys,
                    crate::models::platform_public_session::COLLECTION_NAME,
                    &unknown
                )
                .await
                .is_err()
            );
            unresolved.upstream_may_exist = false;
            unresolved.cleanup_complete = true;
            let terminal = bson::to_document(&unresolved).unwrap();
            verify_current_snapshot(
                &db,
                &keys,
                crate::models::platform_public_session::COLLECTION_NAME,
                &terminal,
            )
            .await
            .unwrap();
            assert!(
                !migrate_snapshot(
                    &db,
                    &keys,
                    crate::models::platform_public_session::COLLECTION_NAME,
                    &terminal
                )
                .await
                .unwrap()
            );
            let mut empty_live = unresolved;
            empty_live.revoked_at = None;
            assert!(
                verify_current_snapshot(
                    &db,
                    &keys,
                    crate::models::platform_public_session::COLLECTION_NAME,
                    &bson::to_document(&empty_live).unwrap()
                )
                .await
                .is_err()
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn turn_context_binds_session_conversation_receipt_and_claim_in_staged_and_strict_modes()
    {
        let mut keys = crate::test_utils::test_encryption_keys();
        for contextual in [false, true] {
            keys.set_contextual_record_writes(contextual);
            keys.set_allow_legacy_record_reads(!contextual);
            let mut row: PlatformPublicTurn = bson::from_document(doc!{
                "_id":"turn","public_session_id":"session","conversation_id":"conversation","receipt_id":"receipt","claim_id":"claim",
                "sponsor_owner_user_id":"owner","policy_digest":"policy","credential_api_key_id":"key","input_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
                "reply_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},"max_output_bytes":4096_i32,"expires_at":bson::DateTime::now()
            }).unwrap();
            row.reply_ciphertext = encrypt_turn(
                &keys,
                TurnIdentity {
                    id: &row.id,
                    owner: &row.sponsor_owner_user_id,
                    session: &row.public_session_id,
                    conversation: &row.conversation_id,
                    receipt: &row.receipt_id,
                    claim: &row.claim_id,
                },
                "reply_ciphertext",
                b"distinctive-answer",
            )
            .await
            .unwrap();
            row.secret_context_versions = initial_versions(&keys, "reply_ciphertext");
            assert_eq!(
                decrypt_turn(&keys, &row, "reply_ciphertext")
                    .await
                    .unwrap()
                    .as_str(),
                "distinctive-answer"
            );
            if contextual {
                for field in [
                    "public_session_id",
                    "conversation_id",
                    "receipt_id",
                    "claim_id",
                ] {
                    let mut swapped = row.clone();
                    match field {
                        "public_session_id" => swapped.public_session_id.push('x'),
                        "conversation_id" => swapped.conversation_id.push('x'),
                        "receipt_id" => swapped.receipt_id.push('x'),
                        _ => swapped.claim_id.push('x'),
                    };
                    assert!(
                        decrypt_turn(&keys, &swapped, "reply_ciphertext")
                            .await
                            .is_err()
                    );
                }
            }
        }
    }
}
