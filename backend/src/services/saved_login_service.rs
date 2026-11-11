//! Write-only website credentials. All access uses the polymorphic owner ACL.
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Deserialize;
use totp_rs::{Algorithm, Secret, TOTP};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::saved_login::{COLLECTION_NAME, SavedLogin},
    services::org_service,
};

#[derive(Deserialize)]
pub struct Input {
    pub label: String,
    pub allowed_origins: Vec<String>,
    pub username: Zeroizing<String>,
    pub password: Option<Zeroizing<String>>,
    pub totp_secret: Option<Zeroizing<String>>,
    #[serde(default)]
    pub confirm_each_sign_in: bool,
}
impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SavedLoginInput { [REDACTED] }")
    }
}

pub async fn authorize(db: &Database, actor: &str, owner: &str) -> AppResult<()> {
    if !org_service::resolve_owner_access(db, actor, owner)
        .await?
        .can_write()
    {
        return Err(AppError::Forbidden(
            "Saved logins require owner or organization admin access".into(),
        ));
    }
    Ok(())
}

fn invalid() -> AppError {
    AppError::ValidationError(
        "Invalid saved login: use a label, exact HTTPS origins and bounded credential values"
            .into(),
    )
}

pub fn validate(input: &Input) -> AppResult<()> {
    if input.label.trim().is_empty()
        || input.label.len() > 100
        || input.allowed_origins.is_empty()
        || input.allowed_origins.len() > 16
        || input.username.is_empty()
        || input.username.len() > 1024
        || input
            .password
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 16384)
        || input
            .totp_secret
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 4096)
    {
        return Err(invalid());
    }
    for origin in &input.allowed_origins {
        let url = url::Url::parse(origin).map_err(|_| invalid())?;
        if url.scheme() != "https"
            || url.host_str().is_none_or(|host| host.contains('*'))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.origin().ascii_serialization() != *origin
        {
            return Err(invalid());
        }
    }
    if let Some(secret) = &input.totp_secret {
        let mut totp = parse_totp(secret)?;
        totp.secret.zeroize();
    }
    Ok(())
}

fn parse_totp(value: &str) -> AppResult<TOTP> {
    if value.starts_with("otpauth://") {
        TOTP::from_url(value).map_err(|_| invalid())
    } else {
        let secret = Secret::Encoded(value.to_owned())
            .to_bytes()
            .map_err(|_| invalid())?;
        TOTP::new(Algorithm::SHA1, 6, 1, 30, secret, None, String::new()).map_err(|_| invalid())
    }
}

pub fn one_time_code(value: &str, time: u64) -> AppResult<Zeroizing<String>> {
    let mut totp = parse_totp(value)?;
    let code = Zeroizing::new(totp.generate(time));
    totp.secret.zeroize();
    Ok(code)
}

pub async fn list(db: &Database, actor: &str, owner: &str) -> AppResult<Vec<SavedLogin>> {
    authorize(db, actor, owner).await?;
    Ok(db
        .collection::<SavedLogin>(COLLECTION_NAME)
        .find(doc! {"user_id":owner})
        .sort(doc! {"label":1,"_id":1})
        .limit(500)
        .await?
        .try_collect()
        .await?)
}

pub async fn get(db: &Database, actor: &str, id: &str) -> AppResult<SavedLogin> {
    let login = db
        .collection::<SavedLogin>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("Saved login not found".into()))?;
    authorize(db, actor, &login.user_id).await?;
    Ok(login)
}

pub async fn put(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &str,
    owner: &str,
    id: Option<&str>,
    input: Input,
) -> AppResult<SavedLogin> {
    authorize(db, actor, owner).await?;
    validate(&input)?;
    let existing_source = if let Some(id) = id {
        Some(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":id})
                .await?
                .ok_or_else(|| AppError::NotFound("Saved login not found".into()))?,
        )
    } else {
        None
    };
    let existing = if let Some(source) = &existing_source {
        let row: SavedLogin = bson::from_document(source.clone())
            .map_err(|_| AppError::Internal("Invalid saved login storage".into()))?;
        if row.user_id != owner {
            return Err(AppError::Forbidden(
                "A saved login cannot change owners".into(),
            ));
        }
        Some(row)
    } else {
        None
    };
    let record_id = existing
        .as_ref()
        .map(|row| row.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let secret_context_version = keys.contextual_record_writes().then_some(3);
    let username_encrypted = encrypt_field(
        keys,
        &record_id,
        owner,
        "username_encrypted",
        input.username.as_bytes(),
    )
    .await?;
    let password_encrypted = match &input.password {
        Some(value) => Some(
            encrypt_field(
                keys,
                &record_id,
                owner,
                "password_encrypted",
                value.as_bytes(),
            )
            .await?,
        ),
        None => None,
    };
    let totp_secret_encrypted = match &input.totp_secret {
        Some(value) => Some(
            encrypt_field(
                keys,
                &record_id,
                owner,
                "totp_secret_encrypted",
                value.as_bytes(),
            )
            .await?,
        ),
        None => None,
    };
    let now = Utc::now();
    let login = SavedLogin {
        id: record_id,
        secret_context_version,
        user_id: owner.into(),
        label: input.label.trim().into(),
        allowed_origins: input.allowed_origins,
        username_encrypted,
        password_encrypted,
        totp_secret_encrypted,
        username_hint: "••••••".into(),
        confirm_each_sign_in: input.confirm_each_sign_in,
        created_at: existing.as_ref().map(|row| row.created_at).unwrap_or(now),
        updated_at: now,
        last_used_at: existing.as_ref().and_then(|row| row.last_used_at),
    };
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        super::erasure_admission::fence_in_session(db, actor, &mut session).await?;
        if owner != actor {
            super::erasure_admission::fence_in_session(db, owner, &mut session).await?;
        }
        if id.is_some() {
            let result = db
                .collection::<SavedLogin>(COLLECTION_NAME)
                .replace_one(
                    secret_predicate(existing_source.as_ref().ok_or_else(invalid)?),
                    &login,
                )
                .session(&mut session)
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::NotFound("Saved login not found".into()));
            }
        } else {
            db.collection::<SavedLogin>(COLLECTION_NAME)
                .insert_one(&login)
                .session(&mut session)
                .await?;
        }
        Ok(())
    }
    .await;
    super::org_membership_authority::finish(session, result).await?;
    Ok(login)
}

pub async fn delete(db: &Database, actor: &str, id: &str) -> AppResult<()> {
    let login = get(db, actor, id).await?;
    let db = db.clone();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(async {
                db.collection::<SavedLogin>(COLLECTION_NAME)
                    .delete_one(doc! {"_id":&id,"user_id":&login.user_id})
                    .session(&mut *session)
                    .await?;
                Box::pin(super::machine_access_service::quarantine_login_in_session(
                    &db, &id, session,
                ))
                .await?;
                db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_many(
                        doc! {"saved_login_ids":&id},
                        doc! {"$pull":{"saved_login_ids":&id}},
                    )
                    .session(&mut *session)
                    .await?;
                db.collection::<bson::Document>(
                    crate::models::assistant_acknowledgement::COLLECTION_NAME,
                )
                .update_many(
                    doc! {"kind":"saved_login","service_id":&id,"status":"pending"},
                    doc! {"$set":{"status":"expired"}},
                )
                .session(&mut *session)
                .await?;
                Ok(())
            })
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

async fn encrypt_field(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    field: &str,
    value: &[u8],
) -> AppResult<Vec<u8>> {
    if keys.contextual_record_writes() {
        keys.encrypt_record(
            value,
            &crate::crypto::aes::RecordContext::new(COLLECTION_NAME, id, owner, &[field])?,
        )
        .await
    } else {
        // Compatible-reader stage retains the existing encrypted format. No
        // new plaintext storage is introduced before the drained writer cutover.
        keys.encrypt(value).await
    }
}
async fn decrypt_field(
    keys: &EncryptionKeys,
    login: &SavedLogin,
    field: &str,
    value: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let plaintext = match login.secret_context_version {
        Some(3) => {
            keys.decrypt_record(
                value,
                &crate::crypto::aes::RecordContext::new(
                    COLLECTION_NAME,
                    &login.id,
                    &login.user_id,
                    &[field],
                )?,
            )
            .await?
        }
        None if keys.allow_legacy_record_reads() => Zeroizing::new(keys.decrypt(value).await?),
        _ => {
            return Err(AppError::Internal(
                "Saved login format is unavailable".into(),
            ));
        }
    };
    validate_plaintext(field, &plaintext)?;
    Ok(plaintext)
}
fn validate_plaintext(field: &str, value: &[u8]) -> AppResult<()> {
    let text = std::str::from_utf8(value)
        .map_err(|_| AppError::Internal("Saved login encoding unavailable".into()))?;
    let max = match field {
        "username_encrypted" => 1024,
        "password_encrypted" => 16384,
        "totp_secret_encrypted" => 4096,
        _ => return Err(invalid()),
    };
    if text.is_empty() || text.len() > max {
        return Err(AppError::Internal("Saved login secret is invalid".into()));
    }
    if field == "totp_secret_encrypted" {
        let mut totp = parse_totp(text)?;
        totp.secret.zeroize();
    }
    Ok(())
}
fn secret_predicate(source: &bson::Document) -> bson::Document {
    super::record_storage::source_predicate(
        source,
        &[
            "user_id",
            "username_encrypted",
            "password_encrypted",
            "totp_secret_encrypted",
            "secret_context_version",
            "updated_at",
        ],
    )
}
/// The runner supplies the authoritative snapshot. Every field is authenticated
/// and validated before a single exact-source CAS; no credential epoch changes.
pub(crate) async fn migrate_secret_snapshot(
    db: &Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let login: SavedLogin = bson::from_document(source.clone())
        .map_err(|_| AppError::Internal("Invalid saved login storage".into()))?;
    let mut set = doc! {"secret_context_version":3_i32};
    for (field, ciphertext) in [
        ("username_encrypted", Some(&login.username_encrypted)),
        ("password_encrypted", login.password_encrypted.as_ref()),
        (
            "totp_secret_encrypted",
            login.totp_secret_encrypted.as_ref(),
        ),
    ] {
        if let Some(ciphertext) = ciphertext {
            let plaintext = decrypt_field(keys, &login, field, ciphertext).await?;
            let target = encrypt_field(keys, &login.id, &login.user_id, field, &plaintext).await?;
            keys.authenticate_current_record(
                &target,
                &crate::crypto::aes::RecordContext::new(
                    COLLECTION_NAME,
                    &login.id,
                    &login.user_id,
                    &[field],
                )?,
            )
            .await?;
            set.insert(
                field,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: target,
                },
            );
        }
    }
    let durable = super::transaction_service::durable_database(db);
    let mut tx = super::erasure_admission::begin_write(&durable, &login.user_id).await?;
    let result = durable
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(secret_predicate(source), doc! {"$set":set})
        .session(&mut tx)
        .await
        .map(|result| result.matched_count == 1)
        .map_err(AppError::from);
    super::org_membership_authority::finish(tx, result).await
}
pub(crate) async fn verify_current_secrets(
    keys: &EncryptionKeys,
    login: &SavedLogin,
) -> AppResult<()> {
    if login.secret_context_version != Some(3) {
        return Err(AppError::Conflict(
            "Saved login requires contextual migration".into(),
        ));
    }
    for (field, ciphertext) in [
        ("username_encrypted", Some(&login.username_encrypted)),
        ("password_encrypted", login.password_encrypted.as_ref()),
        (
            "totp_secret_encrypted",
            login.totp_secret_encrypted.as_ref(),
        ),
    ] {
        if let Some(ciphertext) = ciphertext {
            keys.authenticate_current_record(
                ciphertext,
                &crate::crypto::aes::RecordContext::new(
                    COLLECTION_NAME,
                    &login.id,
                    &login.user_id,
                    &[field],
                )?,
            )
            .await?;
            drop(decrypt_field(keys, login, field, ciphertext).await?);
        }
    }
    Ok(())
}

pub async fn materialize(
    keys: &EncryptionKeys,
    login: &SavedLogin,
    field: &str,
    time: u64,
) -> AppResult<Zeroizing<String>> {
    let ciphertext = match field {
        "username" => &login.username_encrypted,
        "password" => login.password_encrypted.as_ref().ok_or_else(invalid)?,
        "one_time_code" => login.totp_secret_encrypted.as_ref().ok_or_else(invalid)?,
        _ => return Err(invalid()),
    };
    let storage_field = match field {
        "username" => "username_encrypted",
        "password" => "password_encrypted",
        _ => "totp_secret_encrypted",
    };
    let bytes = decrypt_field(keys, login, storage_field, ciphertext).await?;
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| AppError::Internal("Saved login encoding unavailable".into()))?;
    if field == "one_time_code" {
        one_time_code(value, time)
    } else {
        Ok(Zeroizing::new(value.to_owned()))
    }
}

pub async fn record_use(db: &Database, id: &str) -> AppResult<()> {
    db.collection::<SavedLogin>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"last_used_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

pub async fn available(db: &Database, actor: &str) -> AppResult<Vec<SavedLogin>> {
    let owners = super::machine_service::usable_owners(db, actor).await?;
    Ok(db
        .collection::<SavedLogin>(COLLECTION_NAME)
        .find(doc! {"user_id":{"$in":owners}})
        .sort(doc! {"label":1,"_id":1})
        .limit(500)
        .await?
        .try_collect()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(username: &str) -> Input {
        Input {
            label: "Context fixture".into(),
            allowed_origins: vec!["https://example.com".into()],
            username: Zeroizing::new(username.into()),
            password: Some(Zeroizing::new("distinctive-password".into())),
            totp_secret: Some(Zeroizing::new("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into())),
            confirm_each_sign_in: true,
        }
    }

    #[tokio::test]
    async fn contextual_login_rejects_row_owner_field_swaps_and_authenticated_invalid_values() {
        let db = crate::test_utils::connect_transaction_test_database("saved_login_context").await;
        let keys = crate::test_utils::test_encryption_keys();
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let login = put(
            &db,
            &keys,
            &owner,
            &owner,
            None,
            input("distinctive-username"),
        )
        .await
        .unwrap();
        assert_eq!(login.secret_context_version, Some(3));
        assert_eq!(
            materialize(&keys, &login, "username", 59)
                .await
                .unwrap()
                .as_str(),
            "distinctive-username"
        );
        assert_eq!(
            materialize(&keys, &login, "one_time_code", 59)
                .await
                .unwrap()
                .as_str(),
            "287082"
        );
        verify_current_secrets(&keys, &login).await.unwrap();
        let mut swapped = login.clone();
        swapped.id = uuid::Uuid::new_v4().to_string();
        assert!(materialize(&keys, &swapped, "username", 59).await.is_err());
        swapped = login.clone();
        swapped.user_id = "other-owner".into();
        assert!(materialize(&keys, &swapped, "username", 59).await.is_err());
        swapped = login.clone();
        swapped.username_encrypted = login.password_encrypted.clone().unwrap();
        assert!(materialize(&keys, &swapped, "username", 59).await.is_err());
        for invalid in [b"".as_slice(), &[0xff], &vec![b'a'; 1025]] {
            let mut malformed = login.clone();
            malformed.username_encrypted =
                encrypt_field(&keys, &login.id, &owner, "username_encrypted", invalid)
                    .await
                    .unwrap();
            assert!(verify_current_secrets(&keys, &malformed).await.is_err());
            assert!(
                migrate_secret_snapshot(&db, &keys, &bson::to_document(&malformed).unwrap())
                    .await
                    .is_err()
            );
        }
        let mut malformed = login.clone();
        malformed.totp_secret_encrypted = Some(
            encrypt_field(
                &keys,
                &login.id,
                &owner,
                "totp_secret_encrypted",
                b"invalid-base32!",
            )
            .await
            .unwrap(),
        );
        assert!(verify_current_secrets(&keys, &malformed).await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn staged_writer_legacy_migration_and_stale_snapshot_preserve_successor() {
        let db =
            crate::test_utils::connect_transaction_test_database("saved_login_migration").await;
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(false);
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let legacy = put(&db, &keys, &owner, &owner, None, input("legacy-username"))
            .await
            .unwrap();
        assert_eq!(legacy.secret_context_version, None);
        // A compatible old reader receives the exact prior encrypted format.
        assert_eq!(
            keys.decrypt(&legacy.username_encrypted).await.unwrap(),
            b"legacy-username"
        );
        assert!(
            migrate_secret_snapshot(&db, &keys, &bson::to_document(&legacy).unwrap())
                .await
                .is_err()
        );
        keys.set_contextual_record_writes(true);
        assert!(
            migrate_secret_snapshot(&db, &keys, &bson::to_document(&legacy).unwrap())
                .await
                .unwrap()
        );
        let migrated = get(&db, &owner, &legacy.id).await.unwrap();
        verify_current_secrets(&keys, &migrated).await.unwrap();
        assert_eq!(
            migrated.created_at.timestamp_millis(),
            legacy.created_at.timestamp_millis()
        );
        assert_eq!(
            migrated.updated_at.timestamp_millis(),
            legacy.updated_at.timestamp_millis()
        );
        assert_eq!(migrated.allowed_origins, legacy.allowed_origins);
        keys.set_allow_legacy_record_reads(false);
        assert!(materialize(&keys, &legacy, "username", 59).await.is_err());
        assert_eq!(
            materialize(&keys, &migrated, "username", 59)
                .await
                .unwrap()
                .as_str(),
            "legacy-username"
        );
        let successor = put(
            &db,
            &keys,
            &owner,
            &owner,
            Some(&migrated.id),
            input("successor-username"),
        )
        .await
        .unwrap();
        assert!(
            !migrate_secret_snapshot(&db, &keys, &bson::to_document(&migrated).unwrap())
                .await
                .unwrap()
        );
        let live = get(&db, &owner, &migrated.id).await.unwrap();
        assert_eq!(live.username_encrypted, successor.username_encrypted);
        assert_eq!(
            materialize(&keys, &live, "username", 59)
                .await
                .unwrap()
                .as_str(),
            "successor-username"
        );
        // Current-format authentication is terminal, including a mutable downgrade marker.
        let mut corrupted = live.clone();
        *corrupted.username_encrypted.last_mut().unwrap() ^= 1;
        assert!(
            materialize(&keys, &corrupted, "username", 59)
                .await
                .is_err()
        );
        corrupted.secret_context_version = None;
        assert!(
            materialize(&keys, &corrupted, "username", 59)
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn legacy_array_fields_support_raw_snapshot_migration_and_explicit_replacement() {
        let db = crate::test_utils::connect_transaction_test_database("saved_login_arrays").await;
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(false);
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        for migrate in [false, true] {
            let legacy = put(&db, &keys, &owner, &owner, None, input("array-username"))
                .await
                .unwrap();
            let mut source = db
                .collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":&legacy.id})
                .await
                .unwrap()
                .unwrap();
            source.remove("secret_context_version");
            for field in [
                "username_encrypted",
                "password_encrypted",
                "totp_secret_encrypted",
            ] {
                let bytes = source.get_binary_generic(field).unwrap().clone();
                source.insert(
                    field,
                    bytes
                        .into_iter()
                        .map(|b| bson::Bson::Int32(b.into()))
                        .collect::<Vec<_>>(),
                );
            }
            db.collection::<bson::Document>(COLLECTION_NAME)
                .replace_one(doc! {"_id":&legacy.id}, &source)
                .await
                .unwrap();
            keys.set_contextual_record_writes(true);
            if migrate {
                assert!(migrate_secret_snapshot(&db, &keys, &source).await.unwrap());
                assert!(!migrate_secret_snapshot(&db, &keys, &source).await.unwrap());
                let live = get(&db, &owner, &legacy.id).await.unwrap();
                verify_current_secrets(&keys, &live).await.unwrap();
                assert_eq!(
                    materialize(&keys, &live, "username", 59)
                        .await
                        .unwrap()
                        .as_str(),
                    "array-username"
                );
            } else {
                // Authorized replacement also compares the exact historical
                // representation rather than reconstructing Binary from it.
                let replacement = put(
                    &db,
                    &keys,
                    &owner,
                    &owner,
                    Some(&legacy.id),
                    input("replacement-username"),
                )
                .await
                .unwrap();
                verify_current_secrets(&keys, &replacement).await.unwrap();
                assert_eq!(
                    materialize(&keys, &replacement, "username", 59)
                        .await
                        .unwrap()
                        .as_str(),
                    "replacement-username"
                );
            }
            keys.set_contextual_record_writes(false);
        }
        db.drop().await.unwrap();
    }

    #[test]
    fn exact_https_origins_only_and_debug_redacts_all_values() {
        let mut input = Input {
            label: "Test".into(),
            allowed_origins: vec!["https://example.com".into()],
            username: Zeroizing::new("synthetic-username".into()),
            password: Some(Zeroizing::new("synthetic-password".into())),
            totp_secret: None,
            confirm_each_sign_in: false,
        };
        assert!(validate(&input).is_ok());
        assert!(!format!("{input:?}").contains("synthetic"));
        for origin in [
            "http://example.com",
            "https://example.com/path",
            "https://example.com/",
            "https://a@example.com",
            "https://example.com?x=1",
            "https://example.com#f",
            "https://*.example.com",
        ] {
            input.allowed_origins = vec![origin.into()];
            assert!(validate(&input).is_err(), "{origin}");
        }
    }
    #[test]
    fn rfc6238_vector_and_uri_parameters() {
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        assert_eq!(one_time_code(secret, 59).unwrap().as_str(), "287082");
        let uri = format!("otpauth://totp/test?secret={secret}&algorithm=SHA1&digits=8&period=30");
        assert_eq!(one_time_code(&uri, 59).unwrap().as_str(), "94287082");
    }
}
