//! Shared query boundaries between completed logins and first-factor challenges.
use bson::{Document, doc};
/// New purpose is authoritative. Absent purpose is accepted only for legacy
/// completed sessions; the old server's MFA sentinel is always denied there.
pub fn authenticated(filter: Document) -> Document {
    doc! {"$and": [filter, {"user_agent": {"$ne": "mfa_pending"}}, {"$or": [
        {"purpose": "authenticated"},
        {"purpose": {"$exists": false}, "user_agent": {"$ne": "mfa_pending"}}
    ]}]}
}
pub fn pending(filter: Document, legacy: bool) -> Document {
    let purpose = if legacy {
        doc! {"purpose": {"$exists": false}, "user_agent": "mfa_pending"}
    } else {
        doc! {"purpose": "mfa_pending"}
    };
    doc! {"$and": [filter, purpose]}
}

/// Invalidate first-factor proofs in the same transaction as the account fence.
/// This also closes request-only browser approvals, which carry login authority
/// even when the user chose not to retain a browser session.
pub(crate) async fn revoke_pending(
    db: &mongodb::Database,
    user_id: &str,
    transaction: &mut mongodb::ClientSession,
) -> crate::errors::AppResult<()> {
    for collection in [crate::models::session::MFA_CHALLENGES, "sessions"] {
        let filter = if collection == "sessions" {
            pending(doc! {"user_id": user_id, "revoked": false}, true)
        } else {
            doc! {"user_id": user_id, "revoked": false}
        };
        db.collection::<Document>(collection)
            .update_many(filter, doc! {"$set": {"revoked": true}})
            .session(&mut *transaction)
            .await?;
    }
    db.collection::<Document>(crate::models::login_approval::COLLECTION_NAME)
        .update_many(
            doc! {"user_id": user_id, "closed": false},
            doc! {"$set": {"closed": true}},
        )
        .session(&mut *transaction)
        .await?;
    Ok(())
}

pub(crate) async fn revoke_all_in_transaction(
    db: &mongodb::Database,
    user_id: &str,
    transaction: &mut mongodb::ClientSession,
) -> crate::errors::AppResult<u64> {
    db.collection::<Document>("users")
        .update_one(
            doc! {"_id": user_id},
            doc! {"$inc": {"mfa_state_version": 1_i64}},
        )
        .session(&mut *transaction)
        .await?;
    let result = db
        .collection::<Document>("sessions")
        .update_many(
            doc! {"user_id": user_id, "revoked": false},
            doc! {"$set": {"revoked": true, "last_active_at": bson::DateTime::now()}},
        )
        .session(&mut *transaction)
        .await?;
    revoke_pending(db, user_id, transaction).await?;
    db.collection::<Document>("refresh_tokens")
        .update_many(
            doc! {"user_id": user_id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *transaction)
        .await?;
    Ok(result.modified_count)
}
