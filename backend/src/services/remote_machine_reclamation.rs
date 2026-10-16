//! Finality-aware minimization. Display clocks never erase active liabilities.
//! Keep operation/idempotency and Machine tombstones permanently: a delayed
//! request must never allocate resources or charge again.
use crate::{
    errors::{AppError, AppResult},
    models::{
        machine_access::{CONTEXTS, LEASES, OUTBOX as REVOCATIONS},
        machine_external_execution::CONFIRMATIONS,
        machine_runtime_billing::{INTERVAL_COLLECTION as INTERVALS, RUN_COLLECTION as RUNS},
        remote_machine::{
            AuthorizationState, COLLECTION_NAME as MACHINES, ManagedMachine, ResourceState,
        },
        remote_machine_bootstrap::COLLECTION_NAME as BOOTSTRAP,
        remote_machine_operation::{COLLECTION_NAME as OPERATIONS, MachineOperation},
        remote_machine_outbox::COLLECTION_NAME as EFFECTS,
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Bson, Document, doc},
};

const BATCH: i64 = 100;
const FINAL: &str = "final_minimal_tombstone";

/// Every mutation uses IDs selected within the same transaction. Both the
/// selection and post-mutation remainder probe have bounded query time.
async fn batch_ids(
    db: &Database,
    collection: &str,
    filter: Document,
    session: &mut ClientSession,
) -> AppResult<Vec<Bson>> {
    let rows: Vec<Document> = db
        .collection(collection)
        .find(filter)
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .max_time(std::time::Duration::from_secs(3))
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    rows.into_iter()
        .map(|row| {
            row.get("_id")
                .cloned()
                .ok_or_else(|| AppError::Internal("reclamation row identity missing".into()))
        })
        .collect()
}
async fn exists(
    db: &Database,
    collection: &str,
    filter: Document,
    session: &mut ClientSession,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(collection)
        .find_one(filter)
        .projection(doc! {"_id":1})
        .max_time(std::time::Duration::from_secs(3))
        .session(session)
        .await?
        .is_some())
}
async fn delete_batch(
    db: &Database,
    collection: &str,
    filter: Document,
    session: &mut ClientSession,
) -> AppResult<bool> {
    let ids = batch_ids(db, collection, filter.clone(), session).await?;
    if !ids.is_empty() {
        db.collection::<Document>(collection)
            .delete_many(doc! {"$and":[filter.clone(),doc!{"_id":{"$in":ids}}]})
            .session(&mut *session)
            .await?;
    }
    Ok(!exists(db, collection, filter, session).await?)
}
async fn update_batch(
    db: &Database,
    collection: &str,
    filter: Document,
    update: Document,
    session: &mut ClientSession,
) -> AppResult<bool> {
    let ids = batch_ids(db, collection, filter.clone(), session).await?;
    if !ids.is_empty() {
        db.collection::<Document>(collection)
            .update_many(
                doc! {"$and":[filter.clone(),doc!{"_id":{"$in":ids}}]},
                update,
            )
            .session(&mut *session)
            .await?;
    }
    Ok(!exists(db, collection, filter, session).await?)
}

async fn reclaim_in_session(
    db: &Database,
    op: &MachineOperation,
    now: DateTime<Utc>,
    cutoff: bson::DateTime,
    session: &mut ClientSession,
) -> AppResult<&'static str> {
    let Some(machine_id) = op.machine_id.as_deref() else {
        return Ok("machine_identity_missing");
    };
    let machine = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":machine_id,"owner_id":&op.owner_id})
        .session(&mut *session)
        .await?;
    let Some(machine) = machine else {
        return Ok("resource_finality_pending");
    };
    if machine.resource_state != ResourceState::Deleted
        || machine.cleanup_state != "complete"
        || machine.authorization_state != AuthorizationState::Revoked
        || machine.writer_admitted
        || machine
            .cleanup_evidence_digest
            .as_deref()
            .is_none_or(|s| s.is_empty())
    {
        return Ok("resource_finality_pending");
    }
    let node = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":machine_id})
        .session(&mut *session)
        .await?;
    let credentials_final = node.as_ref().is_some_and(|n| {
        n.get_bool("is_active") == Ok(false)
            && n.get("connection_owner").is_none_or(|v| v == &Bson::Null)
            && n.get("signing_secret_encrypted")
                .is_none_or(|v| v == &Bson::Null)
    });
    if !credentials_final {
        return Ok("credential_finality_pending");
    }
    if exists(
        db,
        RUNS,
        doc! {"node_id":machine_id,"state":{"$ne":"closed"}},
        session,
    )
    .await?
        || exists(
            db,
            INTERVALS,
            doc! {"node_id":machine_id,"settlement_complete":{"$ne":true}},
            session,
        )
        .await?
    {
        return Ok("accounting_finality_pending");
    }
    if exists(
        db,
        EFFECTS,
        doc! {"machine_id":machine_id,"state":{"$ne":"applied"}},
        session,
    )
    .await?
        || exists(
            db,
            OPERATIONS,
            doc! {"machine_id":machine_id,"terminal_at":null},
            session,
        )
        .await?
    {
        return Ok("provider_delivery_or_retry_pending");
    }
    if exists(
        db,
        crate::models::machine_job::COLLECTION_NAME,
        doc! {"node_id":machine_id,"finished_at":null},
        session,
    )
    .await?
    {
        return Ok("execution_revocation_pending");
    }
    // Fence the authoritative tombstone before any retained cleanup mutation.
    let changed = db.collection::<Document>(MACHINES).update_one(
        doc!{"_id":machine_id,"revision":machine.revision,"resource_state":"deleted","cleanup_state":"complete"},
        doc!{"$set":{"reclamation_checked_at":bson::DateTime::from_chrono(now)},"$inc":{"reclamation_fence":1_i64}}
    ).session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Ok("resource_finality_pending");
    }

    // An already deleted target cannot ACK. Resolve only this Node's direct
    // deliveries from retained physical cleanup, never a global source selector
    // and never by pretending a Node acknowledgement occurred.
    let direct_drained = update_batch(db,REVOCATIONS,
        doc!{"node_id":machine_id,"pending":true,"kind":{"$ne":"external"}},
        doc!{"$set":{"pending":false,"completed_at":bson::DateTime::from_chrono(now),"completion_evidence":"managed_cleanup_finality","cleanup_evidence_digest":&machine.cleanup_evidence_digest},"$unset":{"claim":"","claim_until":""}},session).await?;
    let leases_drained = delete_batch(
        db,
        LEASES,
        doc! {"node_id":machine_id,"expires_at":{"$lte":bson::DateTime::from_chrono(now)}},
        session,
    )
    .await?;
    if !direct_drained
        || !leases_drained
        || exists(
            db,
            REVOCATIONS,
            doc! {"node_id":machine_id,"pending":true},
            session,
        )
        .await?
        || exists(db, LEASES, doc! {"node_id":machine_id}, session).await?
    {
        return Ok("execution_revocation_pending");
    }

    // All payload classes make bounded progress per pass. Keep the operation
    // pending until every reclaimable batch has drained; full batches never
    // silently finalize and strand the remaining history.
    let mut drained = delete_batch(
        db,
        EFFECTS,
        doc! {"operation_id":&op.id,"machine_id":machine_id,"state":"applied"},
        session,
    )
    .await?;
    drained &= delete_batch(db, BOOTSTRAP, doc! {"binding.node_id":machine_id}, session).await?;
    // The operation display window and all finality gates already passed.
    // Direct delivery payloads need no further age window: legacy receipts
    // have no created_at, and cleanup may only resolve them on this pass.
    drained &= update_batch(db,REVOCATIONS,
        doc!{"node_id":machine_id,"pending":false,"kind":{"$ne":"external"},"minimized_at":null},
        doc!{"$unset":{"external_authority":"","safe_parameters":"","claim":"","claim_until":"","last_error":""},"$set":{"minimized_at":bson::DateTime::from_chrono(now)}},session).await?;
    drained &= delete_batch(db,CONFIRMATIONS,doc!{"node_id":machine_id,"expires_at":{"$lte":cutoff},"state":{"$in":["consumed","declined","expired","revoked"]}},session).await?;
    drained &= delete_batch(db, CONTEXTS, doc! {"node_id":machine_id}, session).await?;
    if !drained {
        return Ok("reclamation_batches_pending");
    }

    db.collection::<Document>(MACHINES).update_one(doc!{"_id":machine_id,"resource_state":"deleted"},
        doc!{"$unset":{"usage":"","usage_received_at":"","platform_storage":"","last_runtime_observation":"","bootstrap_digest":"","bootstrap_delivery_id":"","provider_coverage":""},"$set":{"reclamation_minimized_at":bson::DateTime::from_chrono(now)}})
        .session(session).await?;
    Ok(FINAL)
}

pub async fn sweep(db: &Database, now: DateTime<Utc>) -> AppResult<u64> {
    let cutoff = bson::DateTime::from_chrono(now - Duration::days(90));
    let rows:Vec<MachineOperation> = db.collection(OPERATIONS).find(doc!{
        "terminal_at":{"$lte":cutoff},"reclamation_finalized_at":null,
        "$or":[{"reclamation_checked_at":null},{"reclamation_checked_at":{"$lt":bson::DateTime::from_chrono(now-Duration::minutes(10))}}]
    }).sort(doc!{"reclamation_checked_at":1,"_id":1}).limit(32)
        .max_time(std::time::Duration::from_secs(3)).await?.try_collect().await?;
    let mut minimized = 0;
    for op in rows {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        let result: AppResult<bool> = async {
            let reason = reclaim_in_session(db,&op,now,cutoff,&mut session).await?;
            let finality = reason == FINAL;
            let mut set = doc!{"reclamation_checked_at":bson::DateTime::from_chrono(now),"reclamation_reason":reason,"error_message":Bson::Null};
            if finality {
                set.insert("reclamation_finalized_at",bson::DateTime::from_chrono(now));
                for key in ["effect_id","next_attempt_at","deadline_at"] { set.insert(key,Bson::Null); }
            }
            db.collection::<Document>(OPERATIONS).update_one(doc!{"_id":&op.id,"terminal_at":{"$lte":cutoff}},doc!{"$set":set})
                .session(&mut session).await?;
            Ok(finality)
        }.await;
        match result {
            Ok(done) => {
                session.commit_transaction().await?;
                minimized += u64::from(done);
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                return Err(error);
            }
        }
    }
    Ok(minimized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{
            remote_machine_operation::{LifecycleAction, OperationState},
            user::{COLLECTION_NAME as USERS, User, UserType},
        },
        services::remote_machine_service as service,
        test_utils,
    };
    #[tokio::test]
    async fn reclamation_retains_pending_effects_and_idempotency_then_minimizes_final_rows() {
        let db = test_utils::connect_transaction_test_database("machine_reclamation").await;
        service::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let input = service::CreateMachineInput::fixed(&owner, true, "unconfigured");
        let created = service::create_machine(&db, input.clone()).await.unwrap();
        let now = Utc::now();
        let old = bson::DateTime::from_chrono(now - Duration::days(91));
        db.collection::<Document>(OPERATIONS).update_one(doc!{"_id":&created.operation.id},doc!{"$set":{"state":"succeeded","terminal_at":old,"error_message":"obsolete detail"}}).await.unwrap();
        // Display expiry cannot discard an unclean allocation or pending effect.
        assert_eq!(sweep(&db, now).await.unwrap(), 0);
        let op = db
            .collection::<Document>(OPERATIONS)
            .find_one(doc! {"_id":&created.operation.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            op.get_str("reclamation_reason").unwrap(),
            "resource_finality_pending"
        );
        assert!(op.get_str("request_digest").is_ok());
        let action = service::LifecycleRequest {
            owner_id: owner.clone(),
            machine_id: created.machine.node_id.clone(),
            action: LifecycleAction::Delete,
            expected_revision: Some(created.machine.revision),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            request_body: serde_json::json!({"acknowledge_data_loss":true,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION}),
            now: Some(now),
        };
        let deleted = service::request_action(&db, action).await.unwrap();
        assert!(
            service::finalize_unallocated_delete(&db, &created.machine.node_id)
                .await
                .unwrap()
                .is_some()
        );
        db.collection::<Document>(OPERATIONS).update_many(doc!{"machine_id":&created.machine.node_id},doc!{"$set":{"state":OperationState::Succeeded.as_str(),"terminal_at":old,"reclamation_checked_at":bson::Bson::Null}}).await.unwrap();
        db.collection::<Document>(EFFECTS)
            .update_many(
                doc! {"machine_id":&created.machine.node_id},
                doc! {"$set":{"state":"applied"}},
            )
            .await
            .unwrap();
        let direct_id = uuid::Uuid::new_v4().to_string();
        let legacy_direct_id = uuid::Uuid::new_v4().to_string();
        let global_id = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::machine_access::OUTBOX).insert_many([
            doc!{"_id":&direct_id,"node_id":&created.machine.node_id,"agent_id":uuid::Uuid::new_v4().to_string(),"revision":1_i64,"pending":true,"created_at":old},
            doc!{"_id":&legacy_direct_id,"node_id":&created.machine.node_id,"agent_id":uuid::Uuid::new_v4().to_string(),"revision":1_i64,"pending":true,"last_error":"obsolete delivery detail","safe_parameters":{"command":"fixture"}},
            doc!{"_id":&global_id,"kind":"external","pending":true,"created_at":old,"selector":{"type":"grant","grant_id":uuid::Uuid::new_v4().to_string()}}
        ]).await.unwrap();
        assert!(sweep(&db, now).await.unwrap() >= 1);
        let direct = db
            .collection::<Document>(crate::models::machine_access::OUTBOX)
            .find_one(doc! {"_id":direct_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(direct.get_bool("pending"), Ok(false));
        assert_eq!(
            direct.get_str("completion_evidence"),
            Ok("managed_cleanup_finality")
        );
        assert!(direct.get("acknowledged_at").is_none());
        let legacy = db
            .collection::<Document>(crate::models::machine_access::OUTBOX)
            .find_one(doc! {"_id":legacy_direct_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(legacy.get_bool("pending"), Ok(false));
        assert_eq!(
            legacy.get_str("completion_evidence"),
            Ok("managed_cleanup_finality")
        );
        assert_eq!(
            legacy.get_datetime("completed_at"),
            Ok(&bson::DateTime::from_chrono(now))
        );
        assert!(legacy.get_datetime("minimized_at").is_ok());
        assert!(legacy.get("created_at").is_none());
        assert!(legacy.get("acknowledged_at").is_none());
        assert!(legacy.get("last_error").is_none());
        assert!(legacy.get("safe_parameters").is_none());
        assert!(legacy.get_str("agent_id").is_ok());
        assert_eq!(legacy.get_i64("revision"), Ok(1));
        let global = db
            .collection::<Document>(crate::models::machine_access::OUTBOX)
            .find_one(doc! {"_id":global_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(global.get_bool("pending"), Ok(true));
        let replay = service::create_machine(&db, input).await.unwrap();
        assert_eq!(replay.machine.node_id, created.machine.node_id);
        assert!(replay.replayed);
        assert_eq!(
            replay.machine.resource_state,
            crate::models::remote_machine::ResourceState::Deleted
        );
        assert!(
            db.collection::<Document>(OPERATIONS)
                .find_one(doc! {"_id":deleted.operation.id})
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(sweep(&db, now).await.unwrap(), 0);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn reclamation_batches_over_one_hundred_retain_tombstone_until_all_payloads_drain() {
        let db = test_utils::connect_transaction_test_database("machine_reclamation_batches").await;
        service::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let input = service::CreateMachineInput::fixed(&owner, true, "unconfigured");
        let created = service::create_machine(&db, input.clone()).await.unwrap();
        let now = Utc::now();
        let old = bson::DateTime::from_chrono(now - Duration::days(91));
        let machine_id = &created.machine.node_id;
        let original_effect = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"operation_id":&created.operation.id})
            .await
            .unwrap()
            .unwrap();
        service::request_action(
            &db,
            service::LifecycleRequest {
                owner_id: owner.clone(),
                machine_id: machine_id.clone(),
                action: LifecycleAction::Delete,
                expected_revision: Some(created.machine.revision),
                idempotency_key: uuid::Uuid::new_v4().to_string(),
                request_body: serde_json::json!({"acknowledge_data_loss":true,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION}),
                now: Some(now),
            },
        )
        .await
        .unwrap();
        assert!(
            service::finalize_unallocated_delete(&db, machine_id)
                .await
                .unwrap()
                .is_some()
        );
        // Only the original create has passed its display clock. The newer
        // terminal deletion does not become a second batch driver this pass.
        db.collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&created.operation.id},
                doc! {"$set":{"terminal_at":old}},
            )
            .await
            .unwrap();
        db.collection::<Document>(EFFECTS)
            .update_many(
                doc! {"machine_id":machine_id},
                doc! {"$set":{"state":"applied"}},
            )
            .await
            .unwrap();
        let mut effects = Vec::new();
        for _ in 0..200 {
            let mut effect = original_effect.clone();
            effect.insert("_id", uuid::Uuid::new_v4().to_string());
            effect.insert("kind", "install_provider_deadline");
            effect.insert("effect_digest", uuid::Uuid::new_v4().to_string());
            effect.insert("state", "applied");
            effects.push(effect);
        }
        db.collection::<Document>(EFFECTS)
            .insert_many(effects)
            .await
            .unwrap();
        for collection in [BOOTSTRAP, CONTEXTS, CONFIRMATIONS, REVOCATIONS] {
            // These rows are terminal-retention metadata fixtures only; no
            // fabricated source is admitted and no runtime request is issued.
            let rows = (0..201).map(|_| match collection {
                BOOTSTRAP => doc!{"_id":uuid::Uuid::new_v4().to_string(),"binding":{"node_id":machine_id}},
                CONTEXTS => doc!{"_id":uuid::Uuid::new_v4().to_string(),"node_id":machine_id,"kind":"external","grant_id":uuid::Uuid::new_v4().to_string()},
                CONFIRMATIONS => doc!{"_id":uuid::Uuid::new_v4().to_string(),"node_id":machine_id,"state":"expired","expires_at":old},
                _ => doc!{"_id":uuid::Uuid::new_v4().to_string(),"node_id":machine_id,"kind":"external_lease","pending":false,"created_at":old,"external_authority":{"retention_fixture":true}},
            }).collect::<Vec<_>>();
            db.collection::<Document>(collection)
                .insert_many(rows)
                .await
                .unwrap();
        }
        for (pass, remaining) in [(0_i64, 101_u64), (1, 1)] {
            assert_eq!(
                sweep(&db, now + Duration::minutes(pass * 11))
                    .await
                    .unwrap(),
                0
            );
            let op = db
                .collection::<Document>(OPERATIONS)
                .find_one(doc! {"_id":&created.operation.id})
                .await
                .unwrap()
                .unwrap();
            assert!(op.get("reclamation_finalized_at").is_none());
            assert_eq!(
                op.get_str("reclamation_reason"),
                Ok("reclamation_batches_pending")
            );
            assert!(op.get_str("request_digest").is_ok());
            assert_eq!(
                db.collection::<Document>(EFFECTS)
                    .count_documents(doc! {"operation_id":&created.operation.id})
                    .await
                    .unwrap(),
                remaining
            );
            for (collection, filter) in [
                (BOOTSTRAP, doc! {"binding.node_id":machine_id}),
                (CONTEXTS, doc! {"node_id":machine_id}),
                (CONFIRMATIONS, doc! {"node_id":machine_id}),
                (REVOCATIONS, doc! {"node_id":machine_id,"minimized_at":null}),
            ] {
                assert_eq!(
                    db.collection::<Document>(collection)
                        .count_documents(filter)
                        .await
                        .unwrap(),
                    remaining
                );
            }
        }
        assert_eq!(sweep(&db, now + Duration::minutes(22)).await.unwrap(), 1);
        let op = db
            .collection::<Document>(OPERATIONS)
            .find_one(doc! {"_id":&created.operation.id})
            .await
            .unwrap()
            .unwrap();
        assert!(op.get_datetime("reclamation_finalized_at").is_ok());
        assert_eq!(
            db.collection::<Document>(REVOCATIONS)
                .count_documents(doc! {"node_id":machine_id})
                .await
                .unwrap(),
            201
        );
        assert_eq!(
            db.collection::<Document>(REVOCATIONS)
                .count_documents(doc! {"node_id":machine_id,"external_authority":{"$exists":true}})
                .await
                .unwrap(),
            0
        );
        let replay = service::create_machine(&db, input).await.unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.machine.node_id, *machine_id);
        assert_eq!(replay.machine.resource_state, ResourceState::Deleted);
        assert_eq!(sweep(&db, now + Duration::minutes(33)).await.unwrap(), 0);
        db.drop().await.unwrap();
    }
}
