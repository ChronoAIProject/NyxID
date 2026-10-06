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
    let existing = if let Some(id) = id {
        let row = get(db, actor, id).await?;
        if row.user_id != owner {
            return Err(AppError::Forbidden(
                "A saved login cannot change owners".into(),
            ));
        }
        Some(row)
    } else {
        None
    };
    let username_encrypted = keys.encrypt(input.username.as_bytes()).await?;
    let password_encrypted = match &input.password {
        Some(value) => Some(keys.encrypt(value.as_bytes()).await?),
        None => None,
    };
    let totp_secret_encrypted = match &input.totp_secret {
        Some(value) => Some(keys.encrypt(value.as_bytes()).await?),
        None => None,
    };
    let now = Utc::now();
    let login = SavedLogin {
        id: existing
            .as_ref()
            .map(|row| row.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
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
        last_used_at: existing.and_then(|row| row.last_used_at),
    };
    if id.is_some() {
        let result = db
            .collection::<SavedLogin>(COLLECTION_NAME)
            .replace_one(doc! {"_id":&login.id,"user_id":owner}, &login)
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::NotFound("Saved login not found".into()));
        }
    } else {
        db.collection::<SavedLogin>(COLLECTION_NAME)
            .insert_one(&login)
            .await?;
    }
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
    let bytes = Zeroizing::new(keys.decrypt(ciphertext).await?);
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
