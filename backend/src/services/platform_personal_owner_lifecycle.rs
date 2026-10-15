//! Account disable/delete keeps old Telegram consent retired before reactivation.
use async_trait::async_trait;
use mongodb::{
    ClientSession, Database,
    bson::{Document, doc},
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

/// Each batch writes the inactive user before its binding snapshot. Reactivation
/// conflicts with that fence and can commit only after pending retirement clears.
pub async fn retire(db: &Database, owner: &str, deleted: bool) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    loop {
        let count=session.start_transaction().and_run((db,owner,deleted),|session,(db,owner,deleted)|Box::pin(async move {
            let result:AppResult<usize>=async {
                if db.collection::<Document>(crate::models::user::COLLECTION_NAME).update_one(
                    doc!{"_id":*owner,"is_active":false},
                    doc!{"$set":{"platform_personal_retirement_pending":true},"$inc":{"platform_personal_effect_revision":1_i64}},
                ).session(&mut *session).await?.matched_count!=1 {
                    return Err(AppError::Conflict("Account state changed during consent retirement".into()));
                }
                let count=bindings::retire_owner_batch_in_session(db,session,owner,*deleted,&LocalEffects).await?;
                if count==0 {
                    db.collection::<Document>(crate::models::user::COLLECTION_NAME).update_one(
                        doc!{"_id":*owner,"is_active":false},
                        doc!{"$set":{"platform_personal_retirement_pending":false}},
                    ).session(session).await?;
                }
                Ok(count)
            }.await;
            transaction_result(result)
        })).await.map_err(map_transaction_error)?;
        if count == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disable_and_reenable_cannot_restore_previous_personal_consent() {
        let (db, update, owner) = bindings::tests::fixture("personal_owner_retirement").await;
        let binding = bindings::tests::binding(&update, &owner);
        db.collection(crate::models::platform_personal_binding::COLLECTION_NAME)
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
