//! Account disable/delete keeps old Telegram consent retired before reactivation.
use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};

use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    platform_manager_contract::*,
    platform_personal_binding_service::{
        self as bindings, CapturedPersonalStop, PersonalLifecycleEffects,
    },
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_bot_runtime_controls::{ExactMachineJob, ExactMachineScope},
        platform_personal_binding::{PersonalActiveTurn, PlatformPersonalBinding},
    },
};

struct LocalEffects;
#[async_trait]
impl PersonalLifecycleEffects for LocalEffects {
    async fn capture_stop(
        &self,
        db: &Database,
        session: &mut ClientSession,
        active: &PersonalActiveTurn,
    ) -> AppResult<Option<CapturedPersonalStop>> {
        let origin = bindings::service_origin(&active.origin);
        let turn = TurnRef {
            conversation_id: active.conversation_id.clone(),
            turn_id: active.turn_id.clone(),
        };
        match super::platform_personal_turn_service::capture_exact_stop_in_session(
            db, session, &origin, &turn,
        )
        .await?
        {
            ExactStopOutcome::NoMatchingActiveTurn => Ok(None),
            ExactStopOutcome::Captured(captured) => Ok(Some(CapturedPersonalStop {
                active_turn: active.clone(),
                stop_requested_at: captured.stop_requested_at,
                machine_jobs: captured
                    .machine_jobs
                    .into_iter()
                    .map(|job| ExactMachineJob {
                        node_id: job.node_id,
                        runtime_id: job.runtime_id,
                        job_id: job.job_id,
                    })
                    .collect(),
                machine_scopes: captured
                    .machine_scopes
                    .into_iter()
                    .map(|scope| ExactMachineScope {
                        node_id: scope.node_id,
                        runtime_id: scope.runtime_id,
                    })
                    .collect(),
            })),
        }
    }
    async fn revoke_public_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        binding: &PlatformPersonalBinding,
    ) -> AppResult<()> {
        super::platform_public_agent_service::revoke_in_session(
            db,
            session,
            &PublicRevocationScope::Sender {
                manager: ManagerAuthority {
                    profile_id: binding.platform_config_id.clone(),
                    manager_bot_id: binding.manager_bot_id.clone(),
                    telegram_bot_id: binding.telegram_bot_id,
                    manager_authority_epoch: binding.manager_authority_epoch,
                },
                sender_id: binding.telegram_sender_id,
                chat_id: binding.telegram_chat_id,
            },
            PublicRevocationReason::SenderUnlinked,
        )
        .await?;
        Ok(())
    }
}

const RECOVERY_LIMIT: usize = 32;
const CREDENTIAL_BATCH: i64 = 50;

fn changed() -> AppError {
    AppError::Conflict("Account state changed during consent retirement".into())
}

// Inactive users cannot acquire new consent: review/confirmation write the same
// user document. No personal state preserves ordinary standalone MongoDB use.
async fn has_personal_state(db: &Database, owner: &str) -> AppResult<bool> {
    for (collection, field) in [
        (
            crate::models::platform_personal_binding::COLLECTION_NAME,
            "user_id",
        ),
        (
            crate::models::platform_link_intent::COLLECTION_NAME,
            "claimant_user_id",
        ),
        (
            crate::models::platform_link_intent::CHALLENGES,
            "reviewed_user_id",
        ),
    ] {
        let mut filter = Document::new();
        filter.insert(field, owner);
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id": 1})
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn disable_credentials_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
) -> AppResult<usize> {
    db.collection::<Document>(crate::models::session::COLLECTION_NAME)
        .update_many(
            doc! {"user_id": owner, "revoked": false},
            doc! {"$set": {"revoked": true, "last_active_at": bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(crate::models::refresh_token::COLLECTION_NAME)
        .update_many(
            doc! {"user_id": owner, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *session)
        .await?;
    super::api_key_mutation_service::update_many(
        db,
        doc! {"user_id": owner, "is_active": true},
        doc! {"$set": {"is_active": false}},
        Some(&mut *session),
    )
    .await?;

    let mut cursor = db
        .collection::<Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
        .find(doc! {"user_id": owner})
        .projection(doc! {"api_key_id": 1})
        .sort(doc! {"_id": 1})
        .limit(CREDENTIAL_BATCH)
        .session(&mut *session)
        .await?;
    let mut keys = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        keys.push(row.get_str("api_key_id").map_err(|_| changed())?.to_owned());
    }
    for key in &keys {
        super::assistant_agent_credential_service::invalidate_for_key(
            db,
            owner,
            key,
            Some(&mut *session),
        )
        .await?;
    }
    Ok(keys.len())
}

/// Retire one snapshot while writing the inactive owner. The final transaction
/// revokes ordinary credentials before releasing the reactivation barrier.
async fn retire_batch(db: &Database, owner: &str, deleted: bool) -> AppResult<bool> {
    let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
    let Some(user) = users
        .find_one(doc! {"_id": owner, "is_active": false,
        "platform_personal_retirement_pending": true})
        .await?
    else {
        return Ok(true);
    };
    let deleted = deleted || user.get_bool("platform_personal_deleting").unwrap_or(false);
    super::platform_setup_service::retire_owner(db, owner, deleted).await?;

    if !has_personal_state(db, owner).await? {
        // Match the pre-personal lifecycle for ordinary accounts on standalone
        // MongoDB. Their credential helpers do not require transactions.
        if !deleted {
            super::admin_user_service::revoke_all_user_sessions(db, owner).await?;
            super::api_key_mutation_service::update_many(
                db,
                doc! {"user_id": owner, "is_active": true},
                doc! {"$set": {"is_active": false}},
                None,
            )
            .await?;
            super::assistant_agent_credential_service::invalidate_for_owner(db, owner).await?;
        }
        let completed = users.update_one(doc! {"_id": owner, "is_active": false,
            "platform_personal_retirement_pending": true,
            "platform_personal_effect_revision": user.get("platform_personal_effect_revision").cloned().unwrap_or(bson::Bson::Null)},
            doc! {"$set": {"platform_personal_retirement_pending": false}}).await?;
        return Ok(completed.matched_count == 1);
    }

    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((db, owner, deleted), |session, (db, owner, deleted)| {
            Box::pin(async move {
                let result: AppResult<bool> = async {
                    let Some(current) = db
                        .collection::<Document>(crate::models::user::COLLECTION_NAME)
                        .find_one_and_update(
                            doc! {"_id": *owner, "is_active": false,
                            "platform_personal_retirement_pending": true},
                            doc! {"$inc": {"platform_personal_effect_revision": 1_i64}},
                        )
                        .session(&mut *session)
                        .await?
                    else {
                        return Ok(true);
                    };
                    if current
                        .get_bool("platform_personal_deleting")
                        .unwrap_or(false)
                        && !*deleted
                    {
                        return Err(changed());
                    }
                    let deleted = *deleted;
                    let bindings = bindings::retire_owner_batch_in_session(
                        db,
                        session,
                        owner,
                        deleted,
                        &LocalEffects,
                    )
                    .await?;
                    let credentials = if deleted {
                        0
                    } else {
                        disable_credentials_in_session(db, session, owner).await?
                    };
                    let done = bindings == 0 && credentials == 0;
                    if done {
                        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                            .update_one(
                                doc! {"_id": *owner, "is_active": false,
                                "platform_personal_retirement_pending": true},
                                doc! {"$set": {"platform_personal_retirement_pending": false}},
                            )
                            .session(session)
                            .await?;
                    }
                    Ok(done)
                }
                .await;
                transaction_result(result)
            })
        })
        .await
        .map_err(map_transaction_error)
}

/// Request paths finish every batch before activation or destructive cascade.
/// The deleting marker is monotonic and is never cleared by disable/recovery.
pub async fn retire(db: &Database, owner: &str, deleted: bool) -> AppResult<()> {
    let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
    if users
        .find_one(doc! {"_id": owner, "is_active": false})
        .await?
        .is_none()
    {
        return Err(changed());
    }
    // Legacy inactive accounts have no barrier yet. Do not reopen one that a
    // concurrent worker has already completed.
    users
        .update_one(
            doc! {"_id": owner, "is_active": false,
            "platform_personal_retirement_pending": {"$exists": false}},
            doc! {"$set": {"platform_personal_retirement_pending": true},
            "$inc": {"platform_personal_effect_revision": 1_i64}},
        )
        .await?;
    while !retire_batch(db, owner, deleted).await? {}
    Ok(())
}

/// Visit at most 32 inactive pending owners, one local retirement batch each.
/// Never activates an account or resumes deletion. Failed owners remain pending
/// and move behind other candidates so one failure cannot starve the sweep.
pub async fn recover_pending(db: &Database, limit: usize) -> AppResult<usize> {
    if limit == 0 {
        return Ok(0);
    }
    let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
    let pending: Vec<Document> = users
        .find(doc! {"is_active": false,
        "platform_personal_retirement_pending": true})
        .projection(doc! {"_id": 1})
        .sort(doc! {"platform_personal_retirement_attempted_at": 1, "_id": 1})
        .limit(limit.min(RECOVERY_LIMIT) as i64)
        .await?
        .try_collect()
        .await?;
    let visited = pending.len();
    for user in pending {
        let owner = user.get_str("_id").map_err(|_| changed())?;
        users
            .update_one(
                doc! {"_id": owner, "is_active": false,
                "platform_personal_retirement_pending": true},
                doc! {"$set": {"platform_personal_retirement_attempted_at": bson::DateTime::now()}},
            )
            .await?;
        if let Err(error) = retire_batch(db, owner, false).await {
            tracing::warn!(user_id = owner, error = %error, "Personal account retirement remains pending");
        }
    }
    Ok(visited)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disable_and_reenable_cannot_restore_previous_personal_consent() {
        let (db, update, owner) = bindings::tests::fixture("personal_owner_retirement").await;
        let binding = bindings::tests::binding(&update, &owner);
        db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&binding)
        .await
        .unwrap();
        let admin = uuid::Uuid::new_v4().to_string();
        super::super::admin_user_service::set_user_active(&db, &admin, &owner, false)
            .await
            .unwrap();
        super::super::admin_user_service::set_user_active(&db, &admin, &owner, true)
            .await
            .unwrap();
        let retired = db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":binding.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retired.state,
            crate::models::platform_personal_binding::PersonalBindingState::Revoked
        );
        assert_eq!(
            retired.revocation_reason,
            Some(crate::models::platform_personal_binding::PersonalRevocationReason::OwnerDisabled)
        );
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            bindings::resolve_sender_in_session(&db, &mut session, &update)
                .await
                .unwrap(),
            bindings::ResolvedPersonalSender::Unavailable(UnavailableReason::OwnerUnavailable)
        ));
        session.abort_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
}
