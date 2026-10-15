//! Transactional source fences for external Machine authority.
//!
//! These are projections of existing key/child source rows, not principals or
//! credentials. Source mutations join the existing typed machine_revocations
//! outbox in their supplied transaction. No legacy Assistant revocation is
//! written with invented chat identifiers.

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Bson, Document, doc},
    options::ReturnDocument,
};

use crate::{
    errors::{AppError, AppResult},
    models::api_key_credential::COLLECTION_NAME as CREDENTIALS,
};

/// Reuse the grant model's typed selector contract. A child selector must
/// never be widened to its parent or grant.
pub use crate::models::machine_external_grant::ExternalRevocationSelector as SourceSelector;

/// A fence already written on the authoritative source row in the caller's
/// transaction. This is not an asynchronously discovered revocation intent.
#[derive(Clone)]
pub struct SourceInvalidation {
    pub selector: SourceSelector,
    pub source_fence: i64,
    pub reason: &'static str,
}

/// Evidence returned after a live child read/write fence. The authenticated
/// caller supplies these identifiers; request bodies are not an authority.
pub struct ChildSourceFence {
    // Preserve the checked source revision even when only expiry is consumed.
    pub _source_fence: i64,
    pub expires_at: Option<DateTime<Utc>>,
}

fn source_string(row: &Document, field: &str) -> AppResult<String> {
    row.get_str(field)
        .ok()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::Internal("Machine source identity is invalid".into()))
}

pub(crate) fn source_fence(row: &Document) -> AppResult<i64> {
    let fence = match row.get("machine_external_fence") {
        None => 0,
        Some(Bson::Int64(value)) => *value,
        Some(Bson::Int32(value)) => i64::from(*value),
        _ => return Err(AppError::Internal("Machine source fence is invalid".into())),
    };
    if fence < 0 {
        return Err(AppError::Internal("Machine source fence is invalid".into()));
    }
    Ok(fence)
}

/// Called after `api_key_mutation_service` writes source authority and fence.
/// `previous` was read in this same transaction. The persistent presence hint
/// only avoids Machine reads for keys that have never had a Machine grant; it
/// is set at grant approval and is never authorization or cleared on revoke.
pub(crate) async fn api_key_security_changed_in_session(
    db: &Database,
    previous: &Document,
    session: &mut ClientSession,
) -> AppResult<SourceInvalidation> {
    let user_id = source_string(previous, "user_id")?;
    let api_key_id = source_string(previous, "_id")?;
    let source_fence = source_fence(previous)?
        .checked_add(1)
        .ok_or_else(|| AppError::Internal("Machine source fence exhausted".into()))?;
    let grants_present = match previous.get("machine_external_grants_present") {
        None | Some(Bson::Boolean(false)) => false,
        Some(Bson::Boolean(true)) => true,
        _ => {
            return Err(AppError::Internal(
                "Machine source presence hint is invalid".into(),
            ));
        }
    };
    if grants_present {
        super::machine_external_grant_service::invalidate_api_key_in_session(
            db,
            &user_id,
            &api_key_id,
            "api_key_security_changed",
            session,
        )
        .await?;
    }
    let invalidation = SourceInvalidation {
        selector: SourceSelector::ApiKeyPrincipal {
            user_id,
            api_key_id,
        },
        source_fence,
        reason: "api_key_security_changed",
    };
    // invalidate_api_key_in_session has already enqueued each affected grant's
    // OLD execution revision in this transaction. Never enqueue this broader
    // diagnostic principal receipt: the consumer does not compare source_fence
    // and delayed delivery could cancel a later explicit reauthorization.
    Ok(invalidation)
}

/// Called with the updated child row by `revoke_in_session` and
/// `revoke_children_in_session`. Its fence and revoked state were written
/// together. It does not touch the parent or any Machine grant.
pub(crate) async fn child_revoked_in_session(
    db: &Database,
    updated: &Document,
    session: &mut ClientSession,
) -> AppResult<SourceInvalidation> {
    let source_fence = source_fence(updated)?;
    if source_fence == 0
        || updated.get_bool("is_active").unwrap_or(true)
        || !matches!(updated.get("revoked_at"), Some(Bson::DateTime(_)))
    {
        return Err(AppError::Internal(
            "Machine child revocation fence is invalid".into(),
        ));
    }
    let invalidation = SourceInvalidation {
        selector: SourceSelector::Child {
            user_id: source_string(updated, "user_id")?,
            api_key_id: source_string(updated, "api_key_id")?,
            api_key_credential_id: source_string(updated, "_id")?,
        },
        source_fence,
        reason: "api_key_credential_revoked",
    };
    // Child IDs are immutable, single-issue credential identities. Revocation
    // is terminal, so delayed exact-child delivery cannot match a later login.
    super::machine_external_service::enqueue_external_in_session(
        db,
        &invalidation.selector,
        invalidation.source_fence,
        invalidation.reason,
        session,
    )
    .await?;
    Ok(invalidation)
}

/// Bounded expiry producer for Machine maintenance. Expiry remains enforced by
/// every live child admission regardless of this sweep. The marker is only an
/// idempotent projection on the existing child source, not another outbox or a
/// credential lifecycle state. Child expiry is immutable after issuance.
pub async fn sweep_expired_children(db: &Database, limit: i64) -> AppResult<u64> {
    let db = db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            super::api_key_mutation_service::transaction_result(
                expire_children_in_session(&db, limit, session).await,
            )
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

pub async fn expire_children_in_session(
    db: &Database,
    limit: i64,
    session: &mut ClientSession,
) -> AppResult<u64> {
    let filter = doc! {
        "is_active":true, "revoked_at":Bson::Null,
        "expires_at":{"$type":"date","$lte":bson::DateTime::now()},
        "machine_external_expiry_enqueued_at":Bson::Null,
    };
    let mut cursor = db
        .collection::<Document>(CREDENTIALS)
        .find(filter.clone())
        .projection(doc! {"_id":1})
        .sort(doc! {"expires_at":1,"_id":1})
        .limit(limit.clamp(1, 1000))
        .session(&mut *session)
        .await?;
    let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let mut count = 0;
    for row in rows {
        let mut exact = filter.clone();
        exact.insert("_id", source_string(&row, "_id")?);
        let updated = db.collection::<Document>(CREDENTIALS).find_one_and_update(exact,
            doc! {"$set":{"machine_external_expiry_enqueued_at":bson::DateTime::now()},"$inc":{"machine_external_fence":1_i64}})
            .return_document(ReturnDocument::After).session(&mut *session).await?
            .ok_or(AppError::MachineAuthorityStale)?;
        let fence = source_fence(&updated)?;
        if fence == 0 {
            return Err(AppError::MachineAuthorityStale);
        }
        super::machine_external_service::enqueue_external_in_session(
            db,
            &SourceSelector::Child {
                user_id: source_string(&updated, "user_id")?,
                api_key_id: source_string(&updated, "api_key_id")?,
                api_key_credential_id: source_string(&updated, "_id")?,
            },
            fence,
            "api_key_credential_expired",
            session,
        )
        .await?;
        count += 1;
    }
    Ok(count)
}

/// Join this to Machine admission/renewal's transaction. Its source-row write
/// conflicts with revoke/expiry updates; expiry is checked immediately, without
/// waiting for a cleanup sweep. The parent is fenced separately by admission.
pub async fn fence_child_in_session(
    db: &Database,
    user_id: &str,
    api_key_id: &str,
    credential_id: &str,
    session: &mut ClientSession,
) -> AppResult<ChildSourceFence> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let row = db
        .collection::<Document>(CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": credential_id, "api_key_id": api_key_id, "user_id": user_id,
                "is_active": true, "revoked_at": Bson::Null,
                "$or": [{"expires_at": Bson::Null}, {"expires_at": {"$gt": now}}],
            },
            doc! {"$inc": {"machine_external_fence": 1_i64}},
        )
        .projection(doc! {"machine_external_fence": 1, "expires_at": 1})
        .return_document(ReturnDocument::After)
        .session(session)
        .await?
        .ok_or(AppError::MachinePermissionRevoked)?;
    let expires_at = match row.get("expires_at") {
        None | Some(Bson::Null) => None,
        Some(Bson::DateTime(value)) => Some(value.to_chrono()),
        _ => return Err(AppError::MachinePermissionRevoked),
    };
    Ok(ChildSourceFence {
        _source_fence: source_fence(&row)?,
        expires_at,
    })
}

#[cfg(test)]
#[path = "machine_external_source_tests.rs"]
mod tests;
