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
                    entry: binding.entry.clone(),
                    manager_authority_epoch: binding.manager_authority_epoch,
                },
                sender: binding.sender.clone(),
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
pub(crate) async fn retire_batch(db: &Database, owner: &str, deleted: bool) -> AppResult<bool> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
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
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
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
    async fn no_personal_state_completion_uses_majority_effects_and_evidence() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};
        let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
        let recorded = commands.clone();
        let db = crate::test_utils::connect_test_database_with_command_handler(
            "personal_retirement_durability",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    recorded.lock().unwrap().push(event.command);
                }
            }),
        )
        .await
        .expect("isolated MongoDB required");
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .insert_one(doc! {
                "_id":&owner,"is_active":false,"platform_personal_retirement_pending":true,
                "platform_personal_deleting":true,"platform_personal_effect_revision":1_i64,
            })
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        assert!(retire_batch(&db, &owner, true).await.unwrap());
        // Lost scheduler checkpoint: rerunning reads the committed marker.
        assert!(
            retire_batch(&db.client().database(db.name()), &owner, true)
                .await
                .unwrap()
        );
        let observed = commands.lock().unwrap().clone();
        let reads: Vec<_> = observed.iter().filter(|c| c.contains_key("find")).collect();
        assert!(!reads.is_empty());
        for read in reads {
            assert_eq!(
                read.get_document("readConcern")
                    .unwrap()
                    .get_str("level")
                    .unwrap(),
                "majority"
            );
        }
        let update = observed.iter().find(|c| c.contains_key("update")).unwrap();
        assert_eq!(
            update
                .get_document("writeConcern")
                .unwrap()
                .get_str("w")
                .unwrap(),
            "majority"
        );
        drop(observed);
        db.drop().await.unwrap();
    }

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

    async fn mark_pending(db: &Database, owner: &str, deleting: bool) {
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": owner},
                doc! {"$set": {
                "is_active": false, "platform_personal_retirement_pending": true,
                "platform_personal_deleting": deleting},
                "$inc": {"platform_personal_effect_revision": 1_i64}},
            )
            .await
            .unwrap();
    }

    async fn owner_row(db: &Database, owner: &str) -> Document {
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": owner})
            .await
            .unwrap()
            .unwrap()
    }

    async fn insert_running_binding(
        db: &Database,
        update: &VerifiedManagerUpdate,
        owner: &str,
    ) -> PlatformPersonalBinding {
        use crate::models::platform_personal_binding::{PersonalOriginSource, PersonalTurnProof};
        let mut binding = bindings::tests::binding(update, owner);
        let conversation = format!("nyxa-{}", uuid::Uuid::new_v4().simple());
        let turn = uuid::Uuid::new_v4().to_string();
        let agent = uuid::Uuid::new_v4().to_string();
        let proof = PersonalTurnProof {
            platform_config_id: update.profile_id.clone(),
            manager_bot_id: update.manager_bot_id.clone(),
            entry: update.entry.clone(),
            manager_authority_epoch: 1,
            sender: update.sender.clone(),
            binding_id: binding.id.clone(),
            binding_generation: 1,
            personal_control_revision: 1,
            user_id: owner.into(),
            nyxbot_agent_id: agent.clone(),
            source: PersonalOriginSource::ThreadEvent {
                admission_id: uuid::Uuid::new_v4().to_string(),
            },
        };
        let model = super::super::platform_personal_turn_service::model_origin(
            &bindings::service_origin(&proof),
        );
        let document = doc! {
            "_id": &conversation, "user_id": owner, "title": "Personal conversation",
            "model": "test-model", "credential_api_key_id": "retirement-key", "message_count": 1_i64,
            "platform_thread": bson::to_bson(&model.thread).unwrap(),
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
            "active_turn": {"turn_id": &turn, "started_at": bson::DateTime::now(),
                "platform_origin": bson::to_bson(&model).unwrap(),
                "machine_scopes": [{"node_id": "retirement-node", "runtime_id": "issue-time-runtime"}]},
        };
        let row: crate::models::assistant_conversation::AssistantConversation =
            bson::from_document(document).unwrap();
        db.collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .insert_one(row)
        .await
        .unwrap();
        binding.nyxbot_agent_id = Some(agent);
        binding.conversation_id = Some(conversation.clone());
        binding.active_turn = Some(PersonalActiveTurn {
            conversation_id: conversation,
            turn_id: turn,
            origin: proof,
        });
        db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&binding)
        .await
        .unwrap();
        binding
    }

    #[tokio::test]
    async fn crash_recovery_is_bounded_and_keeps_account_inactive_until_all_batches_finish() {
        let (db, update, owner) = bindings::tests::fixture("personal_retirement_batches").await;
        let rows = db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        );
        let mut batch = Vec::new();
        for sender in 1000..1051 {
            let mut sender_update = update.clone();
            sender_update.sender =
                crate::models::platform_entry_projection::StoredSenderIdentity::TelegramV1 {
                    telegram_sender_id: sender,
                    telegram_chat_id: sender,
                };
            batch.push(bindings::tests::binding(&sender_update, &owner));
        }
        rows.insert_many(batch).await.unwrap();
        db.collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
            .insert_one(
                doc! {"_id": "retirement-intent", "claimant_user_id": &owner, "state": "pending"},
            )
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_link_intent::CHALLENGES)
            .insert_one(doc! {"_id": "retirement-challenge", "reviewed_user_id": &owner, "state": "awaiting_chat_confirmation"}).await.unwrap();
        mark_pending(&db, &owner, false).await;
        assert_eq!(recover_pending(&db, 0).await.unwrap(), 0);
        assert_eq!(
            rows.count_documents(doc! {"state": "active"})
                .await
                .unwrap(),
            51
        );
        assert_eq!(recover_pending(&db, 1).await.unwrap(), 1);
        assert_eq!(
            rows.count_documents(doc! {"state": "active"})
                .await
                .unwrap(),
            1
        );
        assert!(
            owner_row(&db, &owner)
                .await
                .get_bool("platform_personal_retirement_pending")
                .unwrap()
        );
        for collection in [
            crate::models::platform_link_intent::COLLECTION_NAME,
            crate::models::platform_link_intent::CHALLENGES,
        ] {
            let proof = db
                .collection::<Document>(collection)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(proof.get_str("state").unwrap(), "cancelled");
        }
        assert_eq!(recover_pending(&db, 1).await.unwrap(), 1);
        assert_eq!(
            rows.count_documents(doc! {"state": "active"})
                .await
                .unwrap(),
            0
        );
        assert_eq!(recover_pending(&db, 1).await.unwrap(), 1);
        let user = owner_row(&db, &owner).await;
        assert!(
            !user
                .get_bool("platform_personal_retirement_pending")
                .unwrap()
        );
        assert!(!user.get_bool("is_active").unwrap());
        assert_eq!(recover_pending(&db, 1).await.unwrap(), 0);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn failed_retirement_rolls_back_stop_and_consent_and_refuses_reactivation() {
        use crate::models::platform_bot_runtime_controls::{
            PlatformPersonalStopEffect, STOP_EFFECTS,
        };
        let (db, update, owner) = bindings::tests::fixture("personal_retirement_rollback").await;
        let binding = insert_running_binding(&db, &update, &owner).await;
        for collection in [
            crate::models::session::COLLECTION_NAME,
            crate::models::refresh_token::COLLECTION_NAME,
        ] {
            db.collection::<Document>(collection)
                .insert_one(doc! {
                "_id": "old-session", "user_id": &owner, "revoked": false})
                .await
                .unwrap();
        }
        db.collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {
            "_id": "old-key", "user_id": &owner, "is_active": true,
            "state_version": 1_i64, "updated_at": bson::DateTime::now()})
            .await
            .unwrap();
        // A malformed legacy credential makes local cleanup fail after capture.
        let credentials =
            db.collection::<Document>(crate::models::assistant_agent_credential::COLLECTION_NAME);
        credentials
            .insert_one(doc! {"_id": "broken-credential", "user_id": &owner})
            .await
            .unwrap();
        mark_pending(&db, &owner, false).await;
        assert!(retire_batch(&db, &owner, false).await.is_err());
        let saved = db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &binding.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.state,
            crate::models::platform_personal_binding::PersonalBindingState::Active
        );
        assert_eq!(
            db.collection::<Document>(STOP_EFFECTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let turn = db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": binding.conversation_id.as_deref().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !turn
                .get_document("active_turn")
                .unwrap()
                .get_bool("stop_requested")
                .unwrap_or(false)
        );
        for collection in [
            crate::models::session::COLLECTION_NAME,
            crate::models::refresh_token::COLLECTION_NAME,
        ] {
            let session = db
                .collection::<Document>(collection)
                .find_one(doc! {"_id": "old-session"})
                .await
                .unwrap()
                .unwrap();
            assert!(!session.get_bool("revoked").unwrap());
        }
        let key = db
            .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id": "old-key"})
            .await
            .unwrap()
            .unwrap();
        assert!(key.get_bool("is_active").unwrap());
        assert_eq!(key.get_i64("state_version").unwrap(), 1);
        let admin = uuid::Uuid::new_v4().to_string();
        assert!(
            super::super::admin_user_service::set_user_active(&db, &admin, &owner, true)
                .await
                .is_err()
        );
        assert!(!owner_row(&db, &owner).await.get_bool("is_active").unwrap());
        credentials
            .delete_one(doc! {"_id": "broken-credential"})
            .await
            .unwrap();
        super::super::admin_user_service::set_user_active(&db, &admin, &owner, true)
            .await
            .unwrap();
        let stop = db
            .collection::<PlatformPersonalStopEffect>(STOP_EFFECTS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stop.active_turn, binding.active_turn.unwrap());
        assert_eq!(stop.machine_scopes.len(), 1);
        assert_eq!(stop.machine_scopes[0].runtime_id, "issue-time-runtime");
        assert!(!stop.completed);
        assert!(owner_row(&db, &owner).await.get_bool("is_active").unwrap());
        for collection in [
            crate::models::session::COLLECTION_NAME,
            crate::models::refresh_token::COLLECTION_NAME,
        ] {
            let session = db
                .collection::<Document>(collection)
                .find_one(doc! {"_id": "old-session"})
                .await
                .unwrap()
                .unwrap();
            assert!(session.get_bool("revoked").unwrap());
        }
        let key = db
            .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id": "old-key"})
            .await
            .unwrap()
            .unwrap();
        assert!(!key.get_bool("is_active").unwrap());
        assert_eq!(key.get_i64("state_version").unwrap(), 2);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn stale_personal_snapshot_cannot_commit_after_disable_and_reenable() {
        let (db, update, owner) = bindings::tests::fixture("personal_retirement_snapshot").await;
        let binding = bindings::tests::binding(&update, &owner);
        db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&binding)
        .await
        .unwrap();
        let mut stale = db.client().start_session().await.unwrap();
        stale.start_transaction().await.unwrap();
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": &owner})
            .session(&mut stale)
            .await
            .unwrap()
            .unwrap();
        let admin = uuid::Uuid::new_v4().to_string();
        super::super::admin_user_service::set_user_active(&db, &admin, &owner, false)
            .await
            .unwrap();
        super::super::admin_user_service::set_user_active(&db, &admin, &owner, true)
            .await
            .unwrap();
        assert!(
            bindings::fence_binding_in_session(&db, &mut stale, &binding, 1)
                .await
                .is_err()
        );
        let _ = stale.abort_transaction().await;
        // A late recovery snapshot must not revoke newly issued credentials.
        db.collection::<Document>(crate::models::session::COLLECTION_NAME)
            .insert_one(doc! {"_id": "new-session", "user_id": &owner, "revoked": false})
            .await
            .unwrap();
        assert_eq!(recover_pending(&db, 32).await.unwrap(), 0);
        assert!(retire_batch(&db, &owner, false).await.unwrap());
        let session = db
            .collection::<Document>(crate::models::session::COLLECTION_NAME)
            .find_one(doc! {"_id": "new-session"})
            .await
            .unwrap()
            .unwrap();
        assert!(!session.get_bool("revoked").unwrap());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn deletion_recovery_keeps_marker_and_stop_scopes_survive_cascade() {
        use crate::models::platform_bot_runtime_controls::{
            PlatformPersonalStopEffect, STOP_EFFECTS,
        };
        let (db, update, owner) = bindings::tests::fixture("personal_retirement_delete").await;
        let binding = insert_running_binding(&db, &update, &owner).await;
        mark_pending(&db, &owner, true).await;
        assert_eq!(recover_pending(&db, 1).await.unwrap(), 1);
        assert_eq!(recover_pending(&db, 1).await.unwrap(), 1);
        let user = owner_row(&db, &owner).await;
        assert!(user.get_bool("platform_personal_deleting").unwrap());
        assert!(!user.get_bool("is_active").unwrap());
        let admin = uuid::Uuid::new_v4().to_string();
        assert!(matches!(
            super::super::admin_user_service::set_user_active(&db, &admin, &owner, true).await,
            Err(AppError::Conflict(_))
        ));
        let retired = db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &binding.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retired.revocation_reason,
            Some(crate::models::platform_personal_binding::PersonalRevocationReason::OwnerDeleted)
        );
        crate::test_utils::accept_and_process_owner_erasure(
            &db,
            &owner,
            super::super::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION,
        )
        .await
        .unwrap();
        assert!(
            db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id": &owner})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                .find_one(doc! {"_id": binding.conversation_id.as_deref().unwrap()})
                .await
                .unwrap()
                .is_none()
        );
        let stop = db
            .collection::<PlatformPersonalStopEffect>(STOP_EFFECTS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stop.active_turn, binding.active_turn.unwrap());
        assert_eq!(stop.machine_scopes.len(), 1);
        assert_eq!(stop.machine_scopes[0].runtime_id, "issue-time-runtime");
        assert!(!stop.completed);
        db.drop().await.unwrap();
    }
}
