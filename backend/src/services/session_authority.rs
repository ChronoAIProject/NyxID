//! Shared query boundaries between completed logins and first-factor challenges.
use crate::models::mcp_notification::COLLECTION_NAME as MCP_NOTIFICATIONS;
use crate::models::mcp_session::{MCP_SESSION_COLLECTION, MCP_SESSION_SLOT_COLLECTION};
use crate::models::refresh_token::COLLECTION_NAME as REFRESH_TOKENS;
use crate::models::session::COLLECTION_NAME as SESSIONS;
use crate::models::user::COLLECTION_NAME as USERS;
use bson::{Document, doc};
use futures::TryStreamExt;
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

/// Validate live user authority for MCP, requiring the exact assured parent
/// only when MFA is enabled. Legacy or sid-less authority is then rejected.
pub async fn mfa_assurance(
    db: &mongodb::Database,
    user_id: &str,
    session_id: Option<&str>,
) -> crate::errors::AppResult<bool> {
    use crate::errors::AppError;
    let (user_id, mfa_enabled) = db
        .collection::<crate::models::user::User>(USERS)
        .find_one(doc! {"_id": user_id, "is_active": true})
        .await?
        .ok_or_else(|| AppError::Unauthorized("User account is inactive; sign in again".into()))
        .map(|user| (user.id, user.mfa_enabled))?;
    mfa_assurance_for_active_user(db, &user_id, mfa_enabled, session_id).await
}

/// Apply assurance to an active user already validated in this authentication
/// request. Callers must keep this account snapshot local to the request.
pub(crate) async fn mfa_assurance_for_active_user(
    db: &mongodb::Database,
    user_id: &str,
    mfa_enabled: bool,
    session_id: Option<&str>,
) -> crate::errors::AppResult<bool> {
    use crate::errors::AppError;
    // Non-MFA MCP fallback keeps its historical lifetime independent of the
    // parent login. Only MFA users require live, assured parent authority.
    if !mfa_enabled {
        return Ok(false);
    }
    let live = if let Some(id) = session_id {
        db.collection::<crate::models::session::Session>(SESSIONS)
            .find_one(authenticated(doc! {"_id": id, "user_id": user_id,
            "revoked": false, "expires_at": {"$gt": bson::DateTime::now()}}))
            .await?
    } else {
        None
    };
    let assured = live.as_ref().is_some_and(|row| row.mfa_assured);
    if !assured {
        Box::pin(reject_unassured(
            db,
            user_id,
            session_id,
            "session_authority",
            None,
            None,
        ))
        .await?;
        return Err(AppError::Unauthorized(
            "MFA verification required; sign in again".into(),
        ));
    }
    Ok(assured)
}

/// Retire stale authority once. The conditional session update, refresh/MCP
/// cascade and audit commit together; repeated or concurrent rejections cannot
/// flood the audit chain. Sid-less authority has no durable session identity.
pub async fn reject_unassured(
    db: &mongodb::Database,
    user_id: &str,
    session_id: Option<&str>,
    source: &str,
    ip: Option<&str>,
    ua: Option<&str>,
) -> crate::errors::AppResult<()> {
    let Some(id) = session_id else {
        db.collection::<Document>(REFRESH_TOKENS)
            .update_many(
                doc! {"user_id": user_id, "client_id": uuid::Uuid::nil().to_string(),
                "session_id": null, "revoked": false},
                doc! {"$set": {"revoked": true}},
            )
            .await?;
        return Ok(());
    };
    let context = (
        db.clone(),
        user_id.to_owned(),
        id.to_owned(),
        source.to_owned(),
        ip.map(str::to_owned),
        ua.map(str::to_owned),
    );
    super::transaction_service::run_db_only(db, &context, |tx, c| {
        Box::pin(async move {
            let (db, user, id, source, ip, ua) = c;
            let changed = db
                .collection::<Document>(SESSIONS)
                .update_one(
                    authenticated(doc! {"_id": id, "user_id": user, "revoked": false,
                    "mfa_assured": {"$ne": true}}),
                    doc! {"$set": {"revoked": true}},
                )
                .session(&mut *tx)
                .await?;
            if changed.modified_count == 0 {
                return Ok(());
            }
            db.collection::<Document>(REFRESH_TOKENS)
                .update_many(
                    doc! {"user_id": user, "session_id": id, "revoked": false},
                    doc! {"$set": {"revoked": true}},
                )
                .session(&mut *tx)
                .await?;
            let mcp = db.collection::<Document>(MCP_SESSION_COLLECTION);
            let mut cursor = mcp
                .find(doc! {"user_id": user, "parent_session_id": id,
                "session_fallback_allowed": true})
                .session(&mut *tx)
                .await?;
            let rows: Vec<Document> = cursor.stream(&mut *tx).try_collect().await?;
            let ids: Vec<String> = rows
                .iter()
                .filter_map(|r| r.get_str("_id").ok().map(str::to_owned))
                .collect();
            if !ids.is_empty() {
                mcp.delete_many(doc! {"_id": {"$in": &ids}})
                    .session(&mut *tx)
                    .await?;
                for collection in [MCP_SESSION_SLOT_COLLECTION, MCP_NOTIFICATIONS] {
                    db.collection::<Document>(collection)
                        .delete_many(doc! {"session_id": {"$in": &ids}})
                        .session(&mut *tx)
                        .await?;
                }
            }
            super::audit_service::log_actor_event_in_transaction(
                db,
                tx,
                &super::audit_service::AuditActor {
                    user_id: user.clone(),
                    ip_address: ip.clone(),
                    user_agent: ua.clone(),
                    api_key_id: None,
                    api_key_name: None,
                },
                "mfa_unassured_session_rejected",
                serde_json::json!({"session_id": id, "source": source}),
            )
            .await
        })
    })
    .await
}

/// Invalidate first-factor proofs in the same transaction as the account fence.
/// This also closes request-only browser approvals, which carry login authority
/// even when the user chose not to retain a browser session.
pub(crate) async fn revoke_pending(
    db: &mongodb::Database,
    user_id: &str,
    transaction: &mut mongodb::ClientSession,
) -> crate::errors::AppResult<()> {
    for collection in [crate::models::session::MFA_CHALLENGES, SESSIONS] {
        let filter = if collection == SESSIONS {
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
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id": user_id},
            doc! {"$inc": {"mfa_state_version": 1_i64}},
        )
        .session(&mut *transaction)
        .await?;
    let result = db
        .collection::<Document>(SESSIONS)
        .update_many(
            doc! {"user_id": user_id, "revoked": false},
            doc! {"$set": {"revoked": true, "last_active_at": bson::DateTime::now()}},
        )
        .session(&mut *transaction)
        .await?;
    revoke_pending(db, user_id, transaction).await?;
    db.collection::<Document>(REFRESH_TOKENS)
        .update_many(
            doc! {"user_id": user_id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *transaction)
        .await?;
    Ok(result.modified_count)
}

/// Revoke every completed login except the session that performed a newly
/// confirmed MFA enrollment. Refresh tokens and durable MCP session cascades
/// are fenced in the same transaction so no pre-MFA credential survives the
/// successful enrollment response.
pub(crate) async fn revoke_other_sessions_in_transaction(
    db: &mongodb::Database,
    user_id: &str,
    current_session_id: Option<&str>,
    transaction: &mut mongodb::ClientSession,
) -> crate::errors::AppResult<u64> {
    let mut filter = doc! {"user_id": user_id, "revoked": false};
    if let Some(current) = current_session_id {
        filter.insert("_id", doc! {"$ne": current});
    }
    let mut session_cursor = db
        .collection::<Document>(SESSIONS)
        .find(authenticated(filter.clone()))
        .session(&mut *transaction)
        .await?;
    let sessions: Vec<Document> = session_cursor
        .stream(&mut *transaction)
        .try_collect()
        .await?;
    let ids: Vec<String> = sessions
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect();
    if current_session_id.is_none() && !ids.is_empty() {
        return Err(crate::errors::AppError::Conflict(
            "MFA confirmation requires an identified session; sign in again".into(),
        ));
    }
    let changed = db
        .collection::<Document>(SESSIONS)
        .update_many(
            authenticated(filter),
            doc! {"$set": {"revoked": true, "last_active_at": bson::DateTime::now()}},
        )
        .session(&mut *transaction)
        .await?;

    let refresh_filter = if ids.is_empty() {
        doc! {"user_id": user_id, "revoked": false, "session_id": null}
    } else {
        doc! {"user_id": user_id, "revoked": false, "$or": [
            {"session_id": {"$in": &ids}},
            {"session_id": null},
        ]}
    };
    db.collection::<Document>(REFRESH_TOKENS)
        .update_many(refresh_filter, doc! {"$set": {"revoked": true}})
        .session(&mut *transaction)
        .await?;

    // MCP fallback sessions are bearer-less authority and must not outlive
    // the login they were derived from. Delete the rows and their slot/
    // notification records while the account transition is still fenced.
    let mcp = db.collection::<Document>(MCP_SESSION_COLLECTION);
    let mut mcp_cursor = mcp
        .find(doc! {"user_id": user_id, "session_fallback_allowed": true})
        .session(&mut *transaction)
        .await?;
    let mcp_rows: Vec<Document> = mcp_cursor.stream(&mut *transaction).try_collect().await?;
    let mcp_ids: Vec<String> = mcp_rows
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect();
    if !mcp_ids.is_empty() {
        mcp.delete_many(doc! {"_id": {"$in": &mcp_ids}})
            .session(&mut *transaction)
            .await?;
        db.collection::<Document>(MCP_SESSION_SLOT_COLLECTION)
            .delete_many(doc! {"session_id": {"$in": &mcp_ids}})
            .session(&mut *transaction)
            .await?;
        db.collection::<Document>(MCP_NOTIFICATIONS)
            .delete_many(doc! {"session_id": {"$in": &mcp_ids}})
            .session(&mut *transaction)
            .await?;
    }
    Ok(changed.modified_count)
}
