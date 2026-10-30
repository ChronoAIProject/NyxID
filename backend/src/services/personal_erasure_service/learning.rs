//! Publication dispatch is an external-effect boundary. Refuse acceptance
//! while existing owner-authorized recovery still needs the account or its
//! Ornn credentials. Never publish, infer absence, or release a target here.
use super::*;
use crate::models::assistant_agent_learning as learning;

pub(super) const REASON: &str = "learning_publication_reconciliation_required";

pub(super) async fn clear_in_session(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<bool> {
    // started also covers the crash after the durable dispatch marker but
    // before the response checkpoint. A verified operation is settled; its
    // landed target reservation remains a minimal external-effect receipt.
    let unresolved = doc! {"owner_id":owner,"$or":[
        {"publication.verified_at":bson::Bson::Null,"$or":[
            {"publication.started":true},{"publication.uncertain_dispatch":true}]},
        {"$expr":{"$gt":[{"$ifNull":["$publication.lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}}
    ]};
    if db
        .collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
        .find_one(unresolved)
        .projection(doc! {"_id":1})
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    Ok(db
        .collection::<Document>(learning::PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"owner_id":owner,"state":{"$nin":["reserved","landed"]}})
        .projection(doc! {"_id":1})
        .session(tx)
        .await?
        .is_none())
}

pub(super) async fn clear(db: &Database, owner: &str) -> AppResult<bool> {
    let durable = super::super::transaction_service::durable_database(db);
    let mut tx = durable.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result = clear_in_session(&durable, owner, &mut tx).await;
    super::super::org_membership_authority::finish(tx, result).await
}

/// Release only a reservation whose exact proposal proves no dispatch and no
/// live worker. Landed or orphan reservations remain minimal target locks;
/// absence of a proposal must never be inferred to mean external absence.
pub(super) async fn release_unused_targets(
    db: &Database,
    owner: &str,
    agent: &str,
) -> AppResult<bool> {
    use futures::TryStreamExt;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<bool>=async {
        if db.collection::<Document>(super::super::erasure_admission::COLLECTION)
            .update_one(doc! {"_id":owner,"erasure_accepted":true},doc! {"$inc":{"cleanup_revision":1_i64}})
            .session(&mut tx).await?.matched_count!=1 {
            return Err(AppError::Conflict("Personal cleanup barrier is unavailable".into()));
        }
        let mut cursor=db.collection::<Document>(learning::PUBLICATION_TARGETS_COLLECTION_NAME)
            .find(doc! {"agent_id":agent,"owner_id":owner,"state":"reserved"}).limit(32).session(&mut tx).await?;
        let targets:Vec<Document>=cursor.stream(&mut tx).try_collect().await?;
        let mut changed=false;
        for target in targets {
            let proposal=target.get_str("proposal_id").map_err(|_|AppError::Internal("Invalid publication reservation".into()))?;
            let operation=target.get_str("operation_id").map_err(|_|AppError::Internal("Invalid publication reservation".into()))?;
            let sha=target.get_str("package_sha256").map_err(|_|AppError::Internal("Invalid publication reservation".into()))?;
            if db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME).find_one(doc! {
                "_id":proposal,"agent_id":agent,"owner_id":owner,"publication.operation_id":operation,"publication.sha256":sha,
                "publication.started":{"$ne":true},"publication.uncertain_dispatch":{"$ne":true},"publication.verified_at":bson::Bson::Null,
                "$expr":{"$lte":[{"$ifNull":["$publication.lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}
            }).session(&mut tx).await?.is_some() {
                changed|=db.collection::<Document>(learning::PUBLICATION_TARGETS_COLLECTION_NAME)
                    .delete_one(doc! {"_id":target.get("_id").cloned().unwrap(),"agent_id":agent,"owner_id":owner,"state":"reserved","operation_id":operation,"package_sha256":sha})
                    .session(&mut tx).await?.deleted_count==1;
            }
        }
        Ok(changed)
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn interrupted_publication_rejects_acceptance_without_closing_recovery_or_shared_skills()
    {
        for boundary in [
            "started",
            "uncertain_dispatch",
            "live_lease",
            "uncertain_target",
        ] {
            let (db, owner) = super::super::tests::fixture().await;
            let mut publication =
                doc! {"operation_id":"held-ornn-operation","verified_at":bson::Bson::Null};
            match boundary {
                "started" | "uncertain_dispatch" => {
                    publication.insert(boundary, true);
                }
                "live_lease" => {
                    publication.insert(
                        "lease_expires_at",
                        bson::DateTime::from_chrono(
                            chrono::Utc::now() + chrono::Duration::hours(1),
                        ),
                    );
                }
                _ => {}
            }
            let draft = doc! {"_id":"held-proposal","owner_id":&owner,"agent_id":"personal-agent",
            "publication":publication,"body_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![31,41,59]},"status":"publishing"};
            db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
                .insert_one(&draft)
                .await
                .unwrap();
            let target = doc! {"_id":"shared-skill:1.1","owner_id":&owner,"agent_id":"personal-agent",
            "proposal_id":"held-proposal","operation_id":"held-ornn-operation","state":"uncertain"};
            if boundary == "uncertain_target" {
                db.collection::<Document>(learning::PUBLICATION_TARGETS_COLLECTION_NAME)
                    .insert_one(&target)
                    .await
                    .unwrap();
            }
            let shared = doc! {"_id":"unrelated-proposal","owner_id":"org-owner","agent_id":"org-agent",
            "publication":{"started":true,"operation_id":"org-effect"},"status":"publishing"};
            db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
                .insert_one(&shared)
                .await
                .unwrap();
            let credential = doc! {"_id":"ornn-recovery-credential","user_id":&owner,"provider_config_id":"ornn",
            "api_key_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![2,7,18]}};
            db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
                .insert_one(&credential)
                .await
                .unwrap();
            let before = db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                super::super::accept(&db, &owner, &owner, None).await,
                Err(AppError::Conflict(_))
            ));
            super::super::tests::assert_account_preserved(
                db.collection::<Document>(USERS)
                    .find_one(doc! {"_id":&owner})
                    .await
                    .unwrap()
                    .unwrap(),
                before,
            );
            assert!(super::super::status(&db, &owner).await.unwrap().is_none());
            assert_eq!(
                db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
                    .find_one(doc! {"_id":"held-proposal"})
                    .await
                    .unwrap()
                    .unwrap(),
                draft
            );
            assert_eq!(
                db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
                    .find_one(doc! {"_id":"ornn-recovery-credential"})
                    .await
                    .unwrap()
                    .unwrap(),
                credential
            );
            assert_eq!(
                db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
                    .find_one(doc! {"_id":"unrelated-proposal"})
                    .await
                    .unwrap()
                    .unwrap(),
                shared
            );
            if boundary == "uncertain_target" {
                assert_eq!(
                    db.collection::<Document>(learning::PUBLICATION_TARGETS_COLLECTION_NAME)
                        .find_one(doc! {"_id":"shared-skill:1.1"})
                        .await
                        .unwrap()
                        .unwrap(),
                    target
                );
            }
            // Existing reconciliation (or definitive operator release) settles
            // this exact operation before acceptance. No publication is sent.
            db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME).update_one(doc! {"_id":"held-proposal"},doc! {"$set":{"publication.started":false,"publication.uncertain_dispatch":false,"publication.lease_expires_at":bson::Bson::Null}}).await.unwrap();
            db.collection::<Document>(learning::PUBLICATION_TARGETS_COLLECTION_NAME)
                .delete_one(doc! {"_id":"shared-skill:1.1"})
                .await
                .unwrap();
            assert!(
                super::super::accept(&db, &owner, &owner, None)
                    .await
                    .is_ok()
            );
            db.drop().await.unwrap();
        }
    }
}
