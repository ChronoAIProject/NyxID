use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

use crate::crypto::token::{AGENT_KEY_PREFIX, agent_key_display_prefix, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::api_key_credential::{
    ApiKeyCredential, ApiKeyCredentialKind, COLLECTION_NAME, CredentialRevokedReason,
};
use crate::services::{
    api_key_mutation_service as transactions, audit_service,
    machine_external_source_service::{self as machine_sources, SourceInvalidation},
    org_service,
};

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
    super::key_service::require_ordinary_purpose(parent.purpose)?;
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
    let transaction_db = db.clone();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    let row = session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                revoke_in_session(&transaction_db, &id, reason, session).await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(row) = row {
        audit_revocations(db, &[row.credential], reason);
    }
    Ok(())
}

/// The source-row fence and exact child cancellation are already enqueued in
/// the caller's transaction when this receipt is returned. Callers must commit
/// both together. A child revocation never edits its parent or the parent grant.
pub struct RevokedCredential {
    pub credential: ApiKeyCredential,
    // Retain the transaction receipt alongside the revoked credential.
    pub _source_invalidation: SourceInvalidation,
}

pub async fn revoke_in_session(
    db: &Database,
    id: &str,
    reason: CredentialRevokedReason,
    session: &mut ClientSession,
) -> AppResult<Option<RevokedCredential>> {
    let row = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": id, "revoked_at": bson::Bson::Null },
            revocation_update(reason)?,
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let source_invalidation = machine_sources::child_revoked_in_session(db, &row, session).await?;
    let credential = bson::from_document(row)
        .map_err(|_| AppError::Internal("Invalid Agent Key credential source".into()))?;
    Ok(Some(RevokedCredential {
        credential,
        _source_invalidation: source_invalidation,
    }))
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
    }, "$inc": {"machine_external_fence": 1_i64} })
}

pub async fn revoke_children(
    db: &Database,
    key_id: &str,
    reason: CredentialRevokedReason,
    session: Option<&mut ClientSession>,
) -> AppResult<Vec<ApiKeyCredential>> {
    if let Some(session) = session {
        return revoke_children_in_session(db, key_id, reason, session).await;
    }
    let db = db.clone();
    let key_id = key_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                revoke_children_in_session(&db, &key_id, reason, session).await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)
}

async fn revoke_children_in_session(
    db: &Database,
    key_id: &str,
    reason: CredentialRevokedReason,
    session: &mut ClientSession,
) -> AppResult<Vec<ApiKeyCredential>> {
    let collection = db.collection::<ApiKeyCredential>(COLLECTION_NAME);
    let filter = doc! {"api_key_id": key_id, "revoked_at": bson::Bson::Null};
    let mut cursor = collection.find(filter).session(&mut *session).await?;
    let rows: Vec<ApiKeyCredential> = cursor.stream(&mut *session).try_collect().await?;
    let mut revoked = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(result) = revoke_in_session(db, &row.id, reason, &mut *session).await? {
            revoked.push(result.credential);
        }
    }
    Ok(revoked)
}

pub async fn managed_key(
    db: &Database,
    actor: &str,
    key_id: &str,
    write: bool,
) -> AppResult<ApiKey> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": key_id, "purpose": {"$ne": "public_nyxbot"}})
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
