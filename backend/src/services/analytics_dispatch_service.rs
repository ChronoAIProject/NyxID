//! Durable record of requests handed to analytics transport. A crash/timeout
//! leaves uncertainty requiring operator evidence; lease expiry is not proof
//! that an external provider discarded an already submitted request.
use crate::errors::{AppError, AppResult};
use bson::{Document, doc};
use mongodb::{
    Database,
    options::{ReadConcern, WriteConcern},
};
pub const COLLECTION: &str = "telemetry_dispatches";
pub const ACCOUNTS: &str = "telemetry_capture_accounts";

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedDelivery {
    ProviderAccepted,
    NotDelivered,
}
impl VerifiedDelivery {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProviderAccepted => "provider_accepted",
            Self::NotDelivered => "not_delivered",
        }
    }
}

/// Repair one transport obligation on a live account. This records a human's
/// externally verified delivery outcome; it does not attest processing drain or
/// erase provider data. In particular it never advances provider_drained_generation.
/// The caller authenticates/authorizes the operator before entering this service.
#[derive(Clone, Copy)]
pub struct DispatchRepair<'a> {
    pub dispatch: &'a str,
    pub generation: &'a str,
    pub consent_revision: &'a str,
    pub outcome: VerifiedDelivery,
    pub evidence: &'a str,
}
crate::errors::redacted_debug!(DispatchRepair<'_>);
pub async fn reconcile_active_dispatch(
    db: &Database,
    actor: &super::audit_service::AuditActor,
    user: &str,
    repair: DispatchRepair<'_>,
) -> AppResult<()> {
    let DispatchRepair {
        dispatch,
        generation,
        consent_revision,
        outcome: _,
        evidence,
    } = repair;
    if [user, dispatch, generation, consent_revision]
        .iter()
        .any(|id| uuid::Uuid::parse_str(id).is_err())
        || evidence.trim().is_empty()
        || evidence.len() > 1024
        || evidence.chars().any(char::is_control)
    {
        return Err(AppError::ValidationError("Exact dispatch, capture generation, consent revision and bounded evidence are required".into()));
    }
    let db = super::transaction_service::durable_database(db);
    let context = (&db, actor, user, repair);
    super::transaction_service::run_db_only(&db, &context, |tx, context| {
        let &(
            db,
            actor,
            user,
            DispatchRepair {
                dispatch,
                generation,
                consent_revision,
                outcome,
                evidence,
            },
        ) = context;
        Box::pin(async move {
            let result: AppResult<()> = async {
                // This is the same persistent authority as dispatch/withdrawal/erasure.
                // Both concurrent capture and consent changes conflict with this write.
                let fenced = db
                .collection::<Document>("users")
                .update_one(
                    doc! {
                        "_id":user,"is_active":true,"user_type":{"$ne":"org"},
                        "platform_personal_deleting":{"$ne":true},"erasure_accepted":{"$ne":true},
                        "profile_config.analytics.revision":consent_revision
                    },
                    doc! {"$inc":{"analytics_dispatch_sequence":1_i64}},
                )
                .session(&mut *tx)
                .await?;
                if fenced.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "Account or analytics permission changed; reload reconciliation scope"
                            .into(),
                    ));
                }
                let fenced = db.collection::<Document>(ACCOUNTS).update_one(
            doc! {"_id":user,"generation":generation},
            doc! {"$set":{"transport_reconciliation_fence":uuid::Uuid::new_v4().to_string()}}
        ).session(&mut *tx).await?;
                if fenced.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "Capture generation changed; reload reconciliation scope".into(),
                    ));
                }
                let changed = db
                .collection::<Document>(COLLECTION)
                .update_one(
                    doc! {"_id":dispatch,"user_id":user,"status":{"$in":["in_flight","uncertain"]}},
                    doc! {"$set":{"status":"operator_resolved","delivery_outcome":outcome.as_str(),
                    "settled_at":bson::DateTime::now(),"reconciled_generation":generation}},
                )
                .session(&mut *tx)
                .await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "Dispatch is no longer awaiting delivery evidence".into(),
                    ));
                }
                use sha2::{Digest, Sha256};
                let evidence_digest = hex::encode(Sha256::digest(evidence.as_bytes()));
                let metadata = serde_json::json!({"user_id":user,"dispatch_id":dispatch,
            "generation":generation,"consent_revision":consent_revision,
            "delivery_outcome":outcome.as_str(),"evidence_sha256":evidence_digest});
                let key = super::audit_service::audit_chain_hmac_key().ok_or_else(|| {
                    AppError::Internal("Reconciliation audit key is unavailable".into())
                })?;
                super::audit_service::log_actor_event_in_session(
                    db,
                    &mut *tx,
                    key,
                    actor,
                    "analytics.dispatch.delivery_reconciled",
                    metadata,
                )
                .await?;
                Ok(())
            }
            .await;
            result
        })
    })
    .await
}

pub async fn begin(db: &Database, user: &str, revision: &str) -> Option<String> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    tokio::time::timeout(std::time::Duration::from_millis(250), async {
        let mut transaction = db.client().start_session().await?;
        transaction.start_transaction().read_concern(ReadConcern::snapshot()).write_concern(WriteConcern::majority()).await?;
        // Serialize admission with withdrawal and deletion on the source row.
        let result = db.collection::<Document>("users").update_one(doc! {
            "_id":user,"is_active":true,"user_type":{"$ne":"org"},"platform_personal_deleting":{"$ne":true},
            "profile_config.analytics.enabled":true,"profile_config.analytics.revision":revision
        },doc! {"$inc":{"analytics_dispatch_sequence":1_i64}}).session(&mut transaction).await?;
        if result.matched_count != 1 { return Err(AppError::Unauthorized("Analytics permission changed".into())); }
        // The source-row write above serializes concurrent admissions. Bound
        // retained uncertainty per account; losing vendor connectivity must
        // never grow an unbounded per-event history. Uncertainty is reconciled,
        // never expired merely to admit another request.
        if db.collection::<Document>(COLLECTION).count_documents(doc! {
            "user_id":user,"status":{"$in":["in_flight","uncertain"]}
        }).limit(32).session(&mut transaction).await? >= 32 {
            return Err(AppError::Conflict("Analytics dispatch reconciliation is required".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(COLLECTION).insert_one(doc! {"_id":&id,"user_id":user,
            "status":"in_flight","created_at":bson::DateTime::now()}).session(&mut transaction).await?;
        // One bounded account checkpoint replaces an unbounded history of
        // accepted events. A later admission invalidates prior drain evidence.
        db.collection::<Document>(ACCOUNTS).update_one(doc! {"_id":user},
            doc! {"$set":{"generation":&id,"updated_at":bson::DateTime::now()}})
            .upsert(true).session(&mut transaction).await?;
        // If this result is unknown no HTTP is sent. Its durable uncertainty is
        // intentionally retained for erasure reconciliation rather than hidden.
        transaction.commit_transaction().await?;
        Ok::<_,AppError>(id)
    }).await.ok()?.ok()
}
pub async fn settle(db: &Database, id: &str, accepted: bool) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    if accepted {
        // Receipt is confirmed, not processing drained. Retain the account
        // generation until provider-aware erasure reconciliation, not every event.
        db.collection::<Document>(COLLECTION)
            .delete_one(doc! {"_id":id,"status":"in_flight"})
            .write_concern(WriteConcern::majority())
            .await?;
    } else {
        db.collection::<Document>(COLLECTION)
            .update_one(
                doc! {"_id":id,"status":"in_flight"},
                doc! {"$set":{"status":"uncertain","settled_at":bson::DateTime::now()}},
            )
            .write_concern(WriteConcern::majority())
            .await?;
    }
    Ok(())
}
pub async fn has_unsettled(db: &Database, user: &str) -> AppResult<bool> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    Ok(db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"user_id":user,"status":{"$in":["in_flight","uncertain"]}})
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}
pub async fn attest_drained(
    db: &Database,
    id: &str,
    operator: &str,
    evidence: &str,
) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    if [operator, evidence]
        .iter()
        .any(|v| v.trim().is_empty() || v.len() > 1024 || v.chars().any(char::is_control))
    {
        return Err(AppError::ValidationError(
            "Operator identity and bounded evidence are required".into(),
        ));
    }
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let row = db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"_id":id,"status":{"$in":["in_flight","uncertain"]}})
        .session(&mut transaction)
        .await?
        .ok_or_else(|| AppError::Conflict("Dispatch is not awaiting reconciliation".into()))?;
    let user = row
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid dispatch account".into()))?;
    if db
        .collection::<Document>("users")
        .find_one(doc! {"_id":user})
        .session(&mut transaction)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Account deletion has not completed".into(),
        ));
    }
    db.collection::<Document>("telemetry_erasure_reconciliations").insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"dispatch_id":id,"user_id":user,"operator":operator,
        "evidence":evidence,"action":"attest_dispatch_drained","created_at":bson::DateTime::now()
    }).session(&mut transaction).await?;
    db.collection::<Document>(COLLECTION)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"status":"operator_drained","settled_at":bson::DateTime::now()}},
        )
        .session(&mut transaction)
        .await?;
    super::transaction_service::commit(&mut transaction).await
}

/// Transport acknowledgement cannot prove ingestion has drained. This separate
/// account checkpoint is bound to the last admitted dispatch generation.
pub async fn needs_provider_drain(db: &Database, user: &str) -> AppResult<bool> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    Ok(db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id":user,
        "$expr":{"$ne":["$generation",{"$ifNull":["$provider_drained_generation",""]}]}})
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}
/// Fence current transport and provider drain evidence inside an erasure
/// completion transaction. A lease timeout or HTTP acceptance is not evidence.
pub(crate) async fn fence_drained_in_transaction(
    db: &Database,
    user: &str,
    transaction: &mut mongodb::ClientSession,
) -> AppResult<Option<String>> {
    if db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"user_id":user,"status":{"$in":["in_flight","uncertain"]}})
        .session(&mut *transaction)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Capture requests still require settlement or reconciliation".into(),
        ));
    }
    let Some(account) = db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id":user})
        .session(&mut *transaction)
        .await?
    else {
        return Ok(None);
    };
    let generation = account
        .get_str("generation")
        .map_err(|_| AppError::Internal("Invalid capture checkpoint".into()))?;
    if account.get_str("provider_drained_generation").ok() != Some(generation) {
        return Err(AppError::Conflict(
            "Provider processing drain evidence is required".into(),
        ));
    }
    let fenced = db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":user,"generation":generation,"provider_drained_generation":generation},
            doc! {"$set":{"erasure_completion_fence":uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *transaction)
        .await?;
    if fenced.matched_count != 1 {
        return Err(AppError::Conflict("Capture generation changed".into()));
    }
    Ok(Some(generation.to_owned()))
}

pub async fn attest_account_drained(
    db: &Database,
    user: &str,
    operator: &str,
    evidence: &str,
) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    if [operator, evidence]
        .iter()
        .any(|v| v.trim().is_empty() || v.len() > 1024 || v.chars().any(char::is_control))
    {
        return Err(AppError::ValidationError(
            "Operator identity and bounded provider evidence are required".into(),
        ));
    }
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    if db
        .collection::<Document>("users")
        .find_one(doc! {"_id":user})
        .session(&mut transaction)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Account deletion has not completed".into(),
        ));
    }
    if db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"user_id":user,"status":{"$in":["in_flight","uncertain"]}})
        .session(&mut transaction)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Capture requests still require settlement or reconciliation".into(),
        ));
    }
    let account = db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id":user})
        .session(&mut transaction)
        .await?
        .ok_or_else(|| {
            AppError::NotFound("No capture checkpoint exists for this account".into())
        })?;
    let generation = account
        .get_str("generation")
        .map_err(|_| AppError::Internal("Invalid capture checkpoint".into()))?;
    db.collection::<Document>("telemetry_erasure_reconciliations").insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":user,"operator":operator,"evidence":evidence,
        "action":"attest_account_processing_drained","generation":generation,"created_at":bson::DateTime::now()
    }).session(&mut transaction).await?;
    db.collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":user,"generation":generation},
            doc! {"$set":{"provider_drained_generation":generation}},
        )
        .session(&mut transaction)
        .await?;
    super::transaction_service::commit(&mut transaction).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn active_uncertain_cap_repair_resumes_capture_without_granting_erasure_drain() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db =
            crate::test_utils::connect_transaction_test_database("analytics_live_repair").await;
        let user = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &user,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let consent = super::super::analytics_consent_service::set(&db, &user, true)
            .await
            .unwrap();
        let mut ids = Vec::new();
        for _ in 0..32 {
            let id = begin(&db, &user, &consent.revision).await.unwrap();
            settle(&db, &id, false).await.unwrap();
            ids.push(id);
        }
        assert!(begin(&db, &user, &consent.revision).await.is_none());
        let generation = ids.last().unwrap();
        let actor = super::super::audit_service::AuditActor::from_auth_user(
            &crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string()),
        );
        assert!(
            attest_drained(&db, &ids[0], &actor.user_id, "fixture://receipt")
                .await
                .is_err()
        );
        // A stale capture view cannot release a slot, nor can a new consent
        // epoch reuse evidence approved under the previous one.
        assert!(
            reconcile_active_dispatch(
                &db,
                &actor,
                &user,
                crate::services::analytics_dispatch_service::DispatchRepair {
                    dispatch: &ids[0],
                    generation: &ids[0],
                    consent_revision: &consent.revision,
                    outcome: VerifiedDelivery::ProviderAccepted,
                    evidence: "fixture://verified-receipt"
                }
            )
            .await
            .is_err()
        );
        let renewed = super::super::analytics_consent_service::set(&db, &user, true)
            .await
            .unwrap();
        assert!(
            reconcile_active_dispatch(
                &db,
                &actor,
                &user,
                crate::services::analytics_dispatch_service::DispatchRepair {
                    dispatch: &ids[0],
                    generation,
                    consent_revision: &consent.revision,
                    outcome: VerifiedDelivery::ProviderAccepted,
                    evidence: "fixture://verified-receipt"
                }
            )
            .await
            .is_err()
        );
        reconcile_active_dispatch(
            &db,
            &actor,
            &user,
            crate::services::analytics_dispatch_service::DispatchRepair {
                dispatch: &ids[0],
                generation,
                consent_revision: &renewed.revision,
                outcome: VerifiedDelivery::ProviderAccepted,
                evidence: "fixture://verified-receipt",
            },
        )
        .await
        .unwrap();
        assert_eq!(db.collection::<Document>("audit_log").count_documents(
            doc! {"event_type":"analytics.dispatch.delivery_reconciled","user_id":&actor.user_id}
        ).await.unwrap(),1);
        assert!(needs_provider_drain(&db, &user).await.unwrap());
        let resumed = begin(&db, &user, &renewed.revision)
            .await
            .expect("repaired slot resumes opted-in capture");
        assert_ne!(&resumed, generation);
        assert!(
            reconcile_active_dispatch(
                &db,
                &actor,
                &user,
                crate::services::analytics_dispatch_service::DispatchRepair {
                    dispatch: &ids[1],
                    generation,
                    consent_revision: &renewed.revision,
                    outcome: VerifiedDelivery::NotDelivered,
                    evidence: "fixture://transport-ended"
                }
            )
            .await
            .is_err()
        );
        // A late transport callback must not overwrite the audited disposition.
        settle(&db, &ids[0], false).await.unwrap();
        settle(&db, &ids[0], true).await.unwrap();
        let repaired = db
            .collection::<Document>(COLLECTION)
            .find_one(doc! {"_id":&ids[0]})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repaired.get_str("status"), Ok("operator_resolved"));
        db.collection::<Document>("users")
            .update_one(doc! {"_id":&user}, doc! {"$set":{"erasure_accepted":true}})
            .await
            .unwrap();
        assert!(
            reconcile_active_dispatch(
                &db,
                &actor,
                &user,
                crate::services::analytics_dispatch_service::DispatchRepair {
                    dispatch: &ids[1],
                    generation: &resumed,
                    consent_revision: &renewed.revision,
                    outcome: VerifiedDelivery::NotDelivered,
                    evidence: "fixture://transport-ended"
                }
            )
            .await
            .is_err()
        );
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        assert!(
            fence_drained_in_transaction(&db, &user, &mut tx)
                .await
                .is_err()
        );
        tx.abort_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn retained_uncertainty_is_bounded_without_expiring_proof_and_accepted_rows_are_removed()
    {
        let db =
            crate::test_utils::connect_transaction_test_database("analytics_dispatch_bound").await;
        let id = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let permission = crate::services::analytics_consent_service::set(&db, &id, true)
            .await
            .unwrap();
        for _ in 0..40 {
            let dispatch = begin(&db, &id, &permission.revision)
                .await
                .expect("permitted dispatch");
            settle(&db, &dispatch, true).await.unwrap();
        }
        assert_eq!(
            db.collection::<Document>(COLLECTION)
                .count_documents(doc! {"user_id":&id})
                .await
                .unwrap(),
            0
        );
        let mut outstanding = Vec::new();
        for n in 0..32 {
            let dispatch = begin(&db, &id, &permission.revision)
                .await
                .expect("within uncertainty budget");
            if n % 2 == 0 {
                settle(&db, &dispatch, false).await.unwrap();
            }
            outstanding.push(dispatch);
        }
        let independent =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap()
                .database(db.name());
        assert!(
            begin(&independent, &id, &permission.revision)
                .await
                .is_none()
        );
        assert_eq!(
            db.collection::<Document>(COLLECTION)
                .count_documents(doc! {"user_id":&id})
                .await
                .unwrap(),
            32
        );
        assert!(has_unsettled(&db, &id).await.unwrap());
        // Confirming acceptance of one live request frees one slot; uncertain
        // requests remain as proof and cannot be overwritten by later work.
        settle(&db, &outstanding[1], true).await.unwrap();
        assert!(
            begin(&independent, &id, &permission.revision)
                .await
                .is_some()
        );
        assert_eq!(
            db.collection::<Document>(COLLECTION)
                .count_documents(doc! {"user_id":&id,"status":"uncertain"})
                .await
                .unwrap(),
            16
        );
        db.drop().await.unwrap();
    }
}
