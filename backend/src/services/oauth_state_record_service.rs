//! Encoded OAuth-state fields share one trusted identity across live reads,
//! migration and readiness. Legacy hex remains encrypted during reader staging;
//! contextual fields carry an explicit prefix and never fall back after failure.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::oauth_state::{COLLECTION_NAME, OAuthState},
};
use bson::{Document, doc};
use zeroize::Zeroizing;

const PREFIX: &str = "v3:";
const FIELDS: &[&str] = &[
    "code_verifier",
    "device_code_encrypted",
    "user_code_encrypted",
];
fn invalid() -> AppError {
    AppError::Internal("OAuth state field authentication or encoding is invalid".into())
}
fn path<'a>(state: &'a OAuthState, field: &'a str) -> [&'a str; 11] {
    [
        field,
        "initiator",
        &state.user_id,
        "provider",
        &state.provider_config_id,
        "credential_owner",
        if state.credential_user_id.is_some() {
            "present"
        } else {
            "absent"
        },
        state.credential_user_id.as_deref().unwrap_or("absent"),
        "connection",
        if state.connection_id.is_some() {
            "present"
        } else {
            "absent"
        },
        state.connection_id.as_deref().unwrap_or("absent"),
    ]
}
fn context<'a>(state: &'a OAuthState, path: &'a [&'a str]) -> AppResult<RecordContext<'a>> {
    if !path.first().is_some_and(|field| FIELDS.contains(field)) {
        return Err(invalid());
    }
    // Destination, initiator, provider, app owner and connection are distinct
    // authenticated components. Presence tags keep absent values unambiguous.
    RecordContext::new(
        COLLECTION_NAME,
        &state.id,
        state.target_user_id.as_deref().unwrap_or(&state.user_id),
        path,
    )
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    state: &OAuthState,
    field: &str,
    plaintext: &[u8],
) -> AppResult<String> {
    validate(plaintext)?;
    let path = path(state, field);
    if keys.contextual_record_writes() {
        Ok(format!(
            "{PREFIX}{}",
            hex::encode(
                keys.encrypt_record(plaintext, &context(state, &path)?)
                    .await?
            )
        ))
    } else {
        if !FIELDS.contains(&field) {
            return Err(invalid());
        }
        Ok(hex::encode(keys.encrypt(plaintext).await?))
    }
}
fn validate(bytes: &[u8]) -> AppResult<()> {
    if bytes.is_empty() || std::str::from_utf8(bytes).is_err() {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) async fn decrypt(
    keys: &EncryptionKeys,
    state: &OAuthState,
    field: &str,
    encoded: &str,
) -> AppResult<Zeroizing<String>> {
    let path = path(state, field);
    let bytes = if let Some(value) = encoded.strip_prefix(PREFIX) {
        let ciphertext = hex::decode(value).map_err(|_| invalid())?;
        keys.decrypt_record(&ciphertext, &context(state, &path)?)
            .await?
    } else {
        if !FIELDS.contains(&field) || !keys.allow_legacy_record_reads() {
            return Err(invalid());
        }
        let ciphertext = hex::decode(encoded).map_err(|_| invalid())?;
        Zeroizing::new(keys.decrypt(&ciphertext).await?)
    };
    validate(&bytes)?;
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes)
            .map_err(|_| invalid())?
            .to_owned(),
    ))
}
fn encoded<'a>(state: &'a OAuthState, field: &str) -> Option<&'a str> {
    match field {
        "code_verifier" => state.code_verifier.as_deref(),
        "device_code_encrypted" => state.device_code_encrypted.as_deref(),
        "user_code_encrypted" => state.user_code_encrypted.as_deref(),
        _ => None,
    }
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    raw: &Document,
) -> AppResult<()> {
    let state: OAuthState = bson::from_document(raw.clone()).map_err(|_| invalid())?;
    for field in FIELDS {
        if let Some(value) = encoded(&state, field) {
            let ciphertext = hex::decode(value.strip_prefix(PREFIX).ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
            let path = path(&state, field);
            keys.authenticate_current_record(&ciphertext, &context(&state, &path)?)
                .await?;
            decrypt(keys, &state, field, value).await?;
        }
    }
    Ok(())
}
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    raw: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify_current_snapshot(keys, raw).await.is_ok() {
        return Ok(false);
    }
    let state: OAuthState = bson::from_document(raw.clone()).map_err(|_| invalid())?;
    let mut target = raw.clone();
    let mut set = Document::new();
    for field in FIELDS {
        if let Some(value) = encoded(&state, field) {
            let plaintext = decrypt(keys, &state, field, value).await?;
            let value = encrypt(keys, &state, field, plaintext.as_bytes()).await?;
            target.insert(*field, &value);
            set.insert(*field, value);
        }
    }
    verify_current_snapshot(keys, &target).await?;
    let filter = super::record_storage::source_predicate(
        raw,
        &[
            "user_id",
            "provider_config_id",
            "target_user_id",
            "credential_user_id",
            "connection_id",
            "code_verifier",
            "device_code_encrypted",
            "user_code_encrypted",
            "consumed",
            "expires_at",
            "attempt_nonce",
            "flow_kind",
            "connect_link_id",
            "authorization_nonce",
            "last_polled_at",
            "poll_interval",
        ],
    );
    let db = super::transaction_service::durable_database(db);
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(filter, doc! {"$set":set})
        .await?
        .matched_count
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn encoded_state_staging_strict_swaps_exact_cas_and_idempotent_readiness() {
        let db = crate::test_utils::connect_transaction_test_database("oauth_state_record_adapter")
            .await;
        let mut keys = crate::test_utils::test_encryption_keys();
        let mut state: OAuthState = bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":"initiator","target_user_id":"service-account",
            "provider_config_id":"provider","credential_user_id":"app-owner","connection_id":"connection",
            "expires_at":bson::DateTime::now(),"created_at":bson::DateTime::now()
        }).unwrap();
        keys.set_contextual_record_writes(false);
        state.code_verifier = Some(
            encrypt(&keys, &state, "code_verifier", b"distinctive-pkce-verifier")
                .await
                .unwrap(),
        );
        state.device_code_encrypted = Some(
            encrypt(
                &keys,
                &state,
                "device_code_encrypted",
                b"distinctive-device-code",
            )
            .await
            .unwrap(),
        );
        state.user_code_encrypted = Some(
            encrypt(
                &keys,
                &state,
                "user_code_encrypted",
                b"distinctive-user-code",
            )
            .await
            .unwrap(),
        );
        assert!(!state.code_verifier.as_ref().unwrap().starts_with(PREFIX));
        let source = bson::to_document(&state).unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .insert_one(&source)
            .await
            .unwrap();
        keys.set_contextual_record_writes(true);
        assert!(migrate_snapshot(&db, &keys, &source).await.unwrap());
        assert!(!migrate_snapshot(&db, &keys, &source).await.unwrap());
        let current = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&state.id})
            .await
            .unwrap()
            .unwrap();
        verify_current_snapshot(&keys, &current).await.unwrap();
        assert!(!migrate_snapshot(&db, &keys, &current).await.unwrap());
        keys.set_allow_legacy_record_reads(false);
        let current_state: OAuthState = bson::from_document(current.clone()).unwrap();
        assert_eq!(
            decrypt(
                &keys,
                &current_state,
                "code_verifier",
                current_state.code_verifier.as_deref().unwrap()
            )
            .await
            .unwrap()
            .as_str(),
            "distinctive-pkce-verifier"
        );
        for (field, value) in [
            ("_id", "other-state"),
            ("user_id", "other-initiator"),
            ("target_user_id", "other-target"),
            ("provider_config_id", "other-provider"),
            ("credential_user_id", "other-app-owner"),
            ("connection_id", "other-connection"),
        ] {
            let mut swapped = current.clone();
            swapped.insert(field, value);
            assert!(verify_current_snapshot(&keys, &swapped).await.is_err());
        }
        let mut corrupt = current.clone();
        corrupt.insert("code_verifier", "v3:00");
        assert!(verify_current_snapshot(&keys, &corrupt).await.is_err());
        assert!(
            decrypt(
                &keys,
                &state,
                "code_verifier",
                state.code_verifier.as_deref().unwrap()
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }
}
