//! Only the outer server envelope is observable. Preserve the established
//! pool/profile/generation/revision AAD; the inner recipient-sealed browser
//! payload never becomes an OAuth-token holder or server-readable plaintext.
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        oracle_login_profile::OracleLoginProfile, oracle_login_snapshot::OracleLoginSnapshot,
    },
};
use bson::{Document, doc};
use mongodb::Database;

const FIELD: &str = "encrypted_envelope";

fn invalid() -> AppError {
    AppError::Internal("Oracle login outer envelope requires authenticated repair".into())
}

struct Outer {
    aad: String,
    ciphertext: Vec<u8>,
    size: u64,
    expired_profile: bool,
}

fn generated_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .is_ok_and(|id| id.get_version_num() == 4 && id.hyphenated().to_string() == value)
}

fn outer(collection: &str, source: &Document) -> AppResult<Outer> {
    let result = match collection {
        "oracle_login_profiles" => {
            let row: OracleLoginProfile =
                bson::from_document(source.clone()).map_err(|_| invalid())?;
            // Pool/profile/generation are canonical v4 server-generated IDs.
            // Revision also accepts the UUID publication ID syntax already
            // accepted by refresh_in_session (including non-v4 UUIDs). It is
            // the final variable AAD component, after three fixed domains.
            if !generated_id(&row.id)
                || !generated_id(&row.pool_id)
                || !generated_id(&row.generation)
                || uuid::Uuid::parse_str(&row.revision).is_err()
                || row.format_version != 1
            {
                return Err(invalid());
            }
            Outer {
                aad: super::oracle_login_profile_service::aad(&row),
                ciphertext: row.encrypted_envelope,
                size: row.envelope_size,
                expired_profile: row.expires_at <= chrono::Utc::now(),
            }
        }
        "oracle_login_snapshots" => {
            let row: OracleLoginSnapshot =
                bson::from_document(source.clone()).map_err(|_| invalid())?;
            if !generated_id(&row.id) || !generated_id(&row.pool_id) || row.format_version != 1 {
                return Err(invalid());
            }
            Outer {
                aad: super::oracle_login_snapshot_service::snapshot_aad(
                    &row.pool_id,
                    &row.id,
                    row.format_version,
                ),
                ciphertext: row.encrypted_envelope,
                size: row.envelope_size,
                expired_profile: false,
            }
        }
        _ => return Err(invalid()),
    };
    if result.ciphertext.is_empty() {
        // The real profile expiry worker clears both fields atomically. An
        // active/partially-cleared row is unresolved, never ordinary absence.
        if !result.expired_profile || result.size != 0 {
            return Err(invalid());
        }
    } else if result.size == 0
        || result.size > super::oracle_login_snapshot_service::MAX_LOGIN_SNAPSHOT_BYTES as u64
        || result.ciphertext.len()
            > super::oracle_login_snapshot_service::MAX_LOGIN_SNAPSHOT_BYTES
                + crate::crypto::aes::MAX_WRAPPED_DEK_SIZE
                + 128
    {
        return Err(invalid());
    }
    Ok(result)
}

pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    let row = outer(collection, source)?;
    if row.ciphertext.is_empty() {
        return Ok(());
    }
    // Rewrap authenticates the data and original AAD even for a current KEK,
    // and detects fallback-provider use despite colliding key selectors.
    let current = keys
        .rewrap_with_aad(&row.ciphertext, row.aad.as_bytes())
        .await?;
    if current != row.ciphertext {
        return Err(invalid());
    }
    let sealed = zeroize::Zeroizing::new(
        keys.decrypt_with_aad(&row.ciphertext, row.aad.as_bytes())
            .await?,
    );
    if sealed.len() as u64 != row.size {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn decrypt(
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
    let row = outer(collection, source)?;
    if row.ciphertext.is_empty() {
        return Err(invalid());
    }
    let sealed = zeroize::Zeroizing::new(
        keys.decrypt_with_aad(&row.ciphertext, row.aad.as_bytes())
            .await?,
    );
    if sealed.len() as u64 != row.size {
        return Err(invalid());
    }
    Ok(sealed)
}

pub(crate) async fn migrate_snapshot(
    db: &Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<bool> {
    let row = outer(collection, source)?;
    if row.ciphertext.is_empty() {
        return Ok(false);
    }
    let sealed = zeroize::Zeroizing::new(
        keys.decrypt_with_aad(&row.ciphertext, row.aad.as_bytes())
            .await?,
    );
    if sealed.len() as u64 != row.size {
        return Err(invalid());
    }
    let encrypted = if row.ciphertext.first() == Some(&2) {
        keys.rewrap_with_aad(&row.ciphertext, row.aad.as_bytes())
            .await?
    } else {
        // Historical outer formats keep the same established binding too.
        keys.encrypt_with_aad(&sealed, row.aad.as_bytes()).await?
    };
    let mut candidate = source.clone();
    candidate.insert(
        FIELD,
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: encrypted.clone(),
        },
    );
    verify_current_snapshot(keys, collection, &candidate).await?;
    if encrypted == row.ciphertext {
        return Ok(false);
    }
    let db = super::transaction_service::durable_database(db);
    let changed = db
        .collection::<Document>(collection)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set": {FIELD: candidate.get(FIELD).ok_or_else(invalid)?}},
        )
        .await?
        .matched_count
        == 1;
    if let Some(winner) = db
        .collection::<Document>(collection)
        .find_one(doc! {"_id":source.get_str("_id").map_err(|_| invalid())?})
        .await?
    {
        verify_current_snapshot(keys, collection, &winner).await?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(current: &str, previous: Option<&str>) -> EncryptionKeys {
        let mut config = crate::test_utils::test_app_config();
        config.encryption_key = Some(current.repeat(32));
        config.encryption_key_previous = previous.map(|key| key.repeat(32));
        EncryptionKeys::from_config(&config)
    }

    #[tokio::test]
    async fn actual_fetch_survives_outer_rewrap_and_rejects_all_binding_swaps() {
        let db = crate::test_utils::connect_transaction_test_database("oracle_outer_rewrap").await;
        let old = keys("aa", None);
        let rotated = keys("bb", Some("aa"));
        let mut current = keys("bb", None);
        current.set_allow_legacy_record_reads(false);
        let sealed = b"recipient-sealed-envelope-not-readable-cookie-material";
        for collection in ["oracle_login_snapshots", "oracle_login_profiles"] {
            let id = uuid::Uuid::new_v4().to_string();
            let pool = uuid::Uuid::new_v4().to_string();
            let now = bson::DateTime::now();
            let mut row = doc! {
                "_id":&id,"pool_id":&pool,"format_version":1_i32,
                "encrypted_envelope":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1]},
                "envelope_size":sealed.len() as i64,"created_at":now,
                "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1)),
            };
            if collection == "oracle_login_profiles" {
                row.extend(doc! {"name":"test profile","generation":uuid::Uuid::new_v4().to_string(),
                    "revision":uuid::Uuid::new_v4().to_string(),"worker_token_hash":"public-test-verifier",
                    "bindings":[],"updated_at":now});
            } else {
                row.insert("created_by_user_id", uuid::Uuid::new_v4().to_string());
            }
            let binding = outer(collection, &row).unwrap();
            row.insert(
                FIELD,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: old
                        .encrypt_with_aad(sealed, binding.aad.as_bytes())
                        .await
                        .unwrap(),
                },
            );
            row.insert("future_metadata", doc! {"preserve":true});
            let rows = db.collection::<Document>(collection);
            rows.insert_one(&row).await.unwrap();
            let fetch = async |encryption: &EncryptionKeys| {
                if collection == "oracle_login_snapshots" {
                    super::super::oracle_login_snapshot_service::fetch_for_worker(
                        &db, encryption, &pool, &id,
                    )
                    .await
                    .map(|payload| payload.sealed_envelope)
                } else {
                    let stored = db
                        .collection::<OracleLoginProfile>(collection)
                        .find_one(doc! {"_id":&id})
                        .await
                        .unwrap()
                        .unwrap();
                    super::super::oracle_login_profile_service::decrypt(encryption, &stored).await
                }
            };
            assert_eq!(fetch(&old).await.unwrap().as_slice(), sealed);
            assert!(
                verify_current_snapshot(&rotated, collection, &row)
                    .await
                    .is_err()
            );
            assert!(
                migrate_snapshot(&db, &rotated, collection, &row)
                    .await
                    .unwrap()
            );
            let winner = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
            verify_current_snapshot(&current, collection, &winner)
                .await
                .unwrap();
            assert_eq!(fetch(&current).await.unwrap().as_slice(), sealed);
            assert_eq!(
                winner
                    .get_document("future_metadata")
                    .unwrap()
                    .get_bool("preserve"),
                Ok(true)
            );
            assert!(
                !migrate_snapshot(&db, &current, collection, &winner)
                    .await
                    .unwrap()
            );
            for field in ["_id", "pool_id", "generation", "revision"] {
                if winner.contains_key(field) {
                    let mut swapped = winner.clone();
                    swapped.insert(field, uuid::Uuid::new_v4().to_string());
                    assert!(
                        verify_current_snapshot(&current, collection, &swapped)
                            .await
                            .is_err()
                    );
                    assert!(decrypt(&current, collection, &swapped).await.is_err());
                }
            }
            let mut malformed = winner.clone();
            malformed.insert("pool_id", format!("{pool}:extra"));
            assert!(
                verify_current_snapshot(&current, collection, &malformed)
                    .await
                    .is_err()
            );
            // Attribution changes do not change the actual pool authority.
            let mut attribution = winner.clone();
            if collection == "oracle_login_snapshots" {
                attribution.insert("created_by_user_id", uuid::Uuid::new_v4().to_string());
                verify_current_snapshot(&current, collection, &attribution)
                    .await
                    .unwrap();
            }
            // A stale source must not overwrite a concurrent field addition.
            rows.update_one(doc! {"_id":&id}, doc! {"$set":{"concurrent_metadata":true}})
                .await
                .unwrap();
            let next = keys("cc", Some("bb"));
            assert!(
                migrate_snapshot(&db, &next, collection, &winner)
                    .await
                    .is_err()
            );
            let fresh = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
            assert_eq!(fresh.get_bool("concurrent_metadata"), Ok(true));
            assert_eq!(fresh.get(FIELD), winner.get(FIELD));
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn expired_profile_tombstone_is_absent_but_partial_clear_and_snapshot_are_unresolved() {
        let keys = crate::test_utils::test_encryption_keys();
        let now = bson::DateTime::now();
        let row = doc! {"_id":uuid::Uuid::new_v4().to_string(),"pool_id":uuid::Uuid::new_v4().to_string(),
        "generation":uuid::Uuid::new_v4().to_string(),"revision":uuid::Uuid::new_v4().to_string(),
        "name":"expired","worker_token_hash":"verifier","format_version":1_i32,"bindings":[],
        "created_at":now,"updated_at":now,"expires_at":bson::DateTime::from_millis(0),
        "envelope_size":0_i64,"encrypted_envelope":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]}};
        verify_current_snapshot(&keys, "oracle_login_profiles", &row)
            .await
            .unwrap();
        for (field, value) in [
            ("envelope_size", bson::Bson::Int64(1)),
            (
                "expires_at",
                bson::Bson::DateTime(bson::DateTime::from_chrono(
                    chrono::Utc::now() + chrono::Duration::days(1),
                )),
            ),
        ] {
            let mut invalid = row.clone();
            invalid.insert(field, value);
            assert!(
                verify_current_snapshot(&keys, "oracle_login_profiles", &invalid)
                    .await
                    .is_err()
            );
        }
        let mut snapshot = row;
        snapshot.insert("created_by_user_id", uuid::Uuid::new_v4().to_string());
        assert!(
            verify_current_snapshot(&keys, "oracle_login_snapshots", &snapshot)
                .await
                .is_err()
        );
    }
}
