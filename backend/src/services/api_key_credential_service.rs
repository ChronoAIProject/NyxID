use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

use crate::crypto::token::{AGENT_KEY_PREFIX, agent_key_display_prefix, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::api_key_credential::{
    ApiKeyCredential, ApiKeyCredentialKind, COLLECTION_NAME, CredentialRevokedReason,
};
use crate::services::{audit_service, org_service};

pub fn credential_expiry(
    parent: Option<DateTime<Utc>>,
    requested: Option<DateTime<Utc>>,
) -> AppResult<Option<DateTime<Utc>>> {
    let expires = requested.or(parent);
    if expires.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(AppError::ValidationError(
            "credential_expires_at must be in the future".into(),
        ));
    }
    if let (Some(parent), Some(child)) = (parent, expires)
        && child > parent
    {
        return Err(AppError::ValidationError(
            "credential_expires_at cannot exceed the key expiry".into(),
        ));
    }
    Ok(expires)
}

pub fn generate_secret() -> Zeroizing<String> {
    let mut random = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(random.as_mut());
    Zeroizing::new(format!(
        "{AGENT_KEY_PREFIX}{}",
        hex::encode(random.as_slice())
    ))
}

#[allow(clippy::too_many_arguments)]
pub async fn issue(
    db: &Database,
    parent: &ApiKey,
    id: &str,
    login_request_id: &str,
    label: &str,
    expires_at: Option<DateTime<Utc>>,
    secret: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let secret_prefix = agent_key_display_prefix(secret)
        .ok_or_else(|| AppError::ValidationError("Invalid Agent Key credential prefix".into()))?;
    let expires_at = credential_expiry(parent.expires_at, expires_at)?;
    let row = ApiKeyCredential {
        id: id.into(),
        api_key_id: parent.id.clone(),
        user_id: parent.user_id.clone(),
        secret_hash: hash_token(secret),
        secret_prefix: secret_prefix.into(),
        kind: ApiKeyCredentialKind::AgentKeyLogin,
        label: label.into(),
        login_request_id: login_request_id.into(),
        is_active: false,
        revoked_at: None,
        revoked_reason: None,
        expires_at,
        last_used_at: None,
        created_at: Utc::now(),
    };
    db.collection::<ApiKeyCredential>(COLLECTION_NAME)
        .insert_one(row)
        .session(session)
        .await?;
    Ok(())
}

pub async fn revoke(db: &Database, id: &str, reason: CredentialRevokedReason) -> AppResult<()> {
    let row = db
        .collection::<ApiKeyCredential>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": id, "revoked_at": bson::Bson::Null },
            revocation_update(reason)?,
        )
        .await?;
    if let Some(row) = row {
        audit_revocations(db, &[row], reason);
    }
    Ok(())
}

pub fn audit_revocations(
    db: &Database,
    rows: &[ApiKeyCredential],
    reason: CredentialRevokedReason,
) {
    for row in rows {
        audit_service::log_async(
            db.clone(),
            Some(row.user_id.clone()),
            "agent_key_credential_revoked".into(),
            Some(
                serde_json::json!({"api_key_id": row.api_key_id, "credential_id": row.id, "reason": reason}),
            ),
            None,
            None,
            None,
            None,
        );
    }
}

pub fn revocation_update(reason: CredentialRevokedReason) -> AppResult<bson::Document> {
    Ok(doc! { "$set": {
        "is_active": false,
        "revoked_at": bson::DateTime::from_chrono(Utc::now()),
        "revoked_reason": bson::to_bson(&reason).map_err(|e| AppError::Internal(e.to_string()))?,
    } })
}

pub async fn revoke_children(
    db: &Database,
    key_id: &str,
    reason: CredentialRevokedReason,
    session: Option<&mut ClientSession>,
) -> AppResult<Vec<ApiKeyCredential>> {
    let collection = db.collection::<ApiKeyCredential>(COLLECTION_NAME);
    let filter = doc! {"api_key_id": key_id, "revoked_at": bson::Bson::Null};
    let mut session = session;
    let rows = if let Some(session) = session.as_deref_mut() {
        let mut cursor = collection
            .find(filter.clone())
            .session(&mut *session)
            .await?;
        cursor.stream(&mut *session).try_collect().await?
    } else {
        collection.find(filter.clone()).await?.try_collect().await?
    };
    let operation = collection.update_many(filter, revocation_update(reason)?);
    if let Some(session) = session {
        operation.session(session).await?;
    } else {
        operation.await?;
    }
    Ok(rows)
}

pub async fn managed_key(
    db: &Database,
    actor: &str,
    key_id: &str,
    write: bool,
) -> AppResult<ApiKey> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": key_id})
        .await?
        .ok_or(AppError::AgentKeyCredentialNotFound)?;
    let access = org_service::resolve_owner_access(db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::AgentKeyCredentialNotFound);
    }
    if write && !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this API key".into(),
        ));
    }
    Ok(key)
}

pub async fn list(db: &Database, actor: &str, key_id: &str) -> AppResult<Vec<ApiKeyCredential>> {
    managed_key(db, actor, key_id, false).await?;
    Ok(db
        .collection::<ApiKeyCredential>(COLLECTION_NAME)
        .find(doc! {"api_key_id": key_id})
        .sort(doc! {"created_at": -1})
        .await?
        .try_collect()
        .await?)
}

pub async fn web_revoke(
    db: &Database,
    actor: &str,
    key_id: &str,
    credential_id: &str,
) -> AppResult<()> {
    managed_key(db, actor, key_id, true).await?;
    db.collection::<ApiKeyCredential>(COLLECTION_NAME)
        .find_one(doc! {"_id": credential_id, "api_key_id": key_id})
        .await?
        .ok_or(AppError::AgentKeyCredentialNotFound)?;
    revoke(db, credential_id, CredentialRevokedReason::WebRevoke).await
}
