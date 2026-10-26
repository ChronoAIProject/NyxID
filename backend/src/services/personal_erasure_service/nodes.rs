//! Operator evidence is used only when a retained owner generation can no
//! longer acknowledge close. A timeout/409 never establishes retirement.
use super::*;
use crate::services::node_owner_service::NodeOwnerFence;

#[derive(serde::Serialize)]
pub(crate) struct DrainMetadata {
    pub node_id: String,
    pub fence: Option<NodeOwnerFence>,
}
pub(crate) async fn pending(
    db: &Database,
    user: &str,
    after: Option<&str>,
) -> AppResult<Vec<DrainMetadata>> {
    use futures::TryStreamExt;
    let db = super::super::transaction_service::durable_database(db);
    let mut filter = doc! {"user_id":user,"personal_erasure_drain_required":true,"personal_erasure_generation_retired":{"$ne":true}};
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt":after});
    }
    let rows: Vec<Document> = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find(filter)
        .projection(doc! {"_id":1,"personal_erasure_drain_owner":1})
        .sort(doc! {"_id":1})
        .limit(64)
        .await?
        .try_collect()
        .await?;
    rows.into_iter()
        .map(|row| {
            let id = row
                .get_str("_id")
                .map_err(|_| AppError::Internal("Invalid node drain identity".into()))?
                .to_owned();
            let owner = row
                .get_document("personal_erasure_drain_owner")
                .ok()
                .map(|owner| {
                    bson::from_document::<crate::models::node::NodeConnectionOwner>(owner.clone())
                })
                .transpose()
                .map_err(|_| AppError::Internal("Invalid retained node generation".into()))?;
            Ok(DrainMetadata {
                fence: owner
                    .as_ref()
                    .map(|owner| NodeOwnerFence::from_owner(&id, owner)),
                node_id: id,
            })
        })
        .collect()
}
pub(crate) async fn attest_generation_retired(
    db: &Database,
    actor: &str,
    user: &str,
    fence: &NodeOwnerFence,
    evidence: &str,
) -> AppResult<()> {
    if evidence.trim().is_empty() || evidence.len() > 1024 || evidence.chars().any(char::is_control)
    {
        return Err(AppError::ValidationError(
            "Bounded deployment retirement evidence is required".into(),
        ));
    }
    let db = super::super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()>=async {
        let node=db.collection::<Document>(crate::models::node::COLLECTION_NAME).find_one(doc! {"_id":&fence.node_id,"user_id":user,"is_active":false,"personal_erasure_drain_required":true,
            "metadata.provisioning_source":{"$ne":"remote_machine"},
            "personal_erasure_drain_owner.instance_name":&fence.instance_name,
            "personal_erasure_drain_owner.generation_id":&fence.generation_id,
            "personal_erasure_drain_owner.connection_id":&fence.connection_id}).session(&mut tx).await?
            .ok_or_else(||AppError::Conflict("Retained node generation changed; reload drain evidence".into()))?;
        // Persist exact generation proof separately from node rows before their
        // local removal. It attests process retirement, never provider erasure.
        let key=super::super::audit_service::audit_chain_hmac_key().ok_or_else(||AppError::Internal("Audit chain unavailable".into()))?;
        super::super::audit_service::log_actor_event_in_session(&db,&mut tx,key,&super::super::audit_service::AuditActor {user_id:actor.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},
            "user.account.erasure_node_generation_retired",serde_json::json!({"target_user_id":user,"node_fence":fence,"evidence_sha256":hex::encode(sha2::Sha256::digest(evidence.as_bytes()))})).await?;
        db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc! {"_id":node.get("_id").unwrap()},doc! {"$set":{"personal_erasure_generation_retired":true}}).session(&mut tx).await?;
        let changed=db.collection::<Document>(JOBS).update_one(doc! {"user_id":user},vec![doc! {"$set":{"status":"pending","retry_at":"$$NOW","lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"completed_at":bson::Bson::Null,
            "domains":{"$map":{"input":"$domains","as":"domain","in":{"$cond":[{"$eq":["$$domain.domain","local_inventory"]},{"$mergeObjects":["$$domain",{"status":"pending","completed_at":bson::Bson::Null,"reason":bson::Bson::Null}]},"$$domain"]}}}}}]).session(&mut tx).await?;
        if changed.matched_count!=1 {return Err(AppError::Conflict("Accepted erasure intent is unavailable".into()));}
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}
use sha2::Digest;
