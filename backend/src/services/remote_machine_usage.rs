//! Authenticated, generation-fenced usage observations; never a physical-stop
//! or accounting authority. Raw file names/content are not collected.
use super::node_owner_service::NodeOwnerFence;
use crate::{
    errors::{AppError, AppResult},
    models::remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine},
};
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use nyxid_machine::managed::Usage;

pub async fn record(db: &Database, fence: &NodeOwnerFence, usage: &Usage) -> AppResult<()> {
    let now = Utc::now();
    usage
        .managed_runtime
        .validate()
        .map_err(|_| AppError::MachineAuthorityStale)?;
    if usage.managed_runtime.node_id != fence.node_id
        || usage.runtime_id.is_empty()
        || usage.sampled_at_ms > now.timestamp_millis() + 30_000
        || usage.sampled_at_ms < now.timestamp_millis() - 120_000
    {
        return Err(AppError::MachineAuthorityStale);
    }
    let usage = normalize_usage(usage)?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result:AppResult<()> = async {
        let mut filter=fence.filter();
        filter.insert("is_active",true);
        filter.insert("machine.runtime_id",&usage.runtime_id);
        filter.insert("connection_owner.expires_at",doc!{"$gt":bson::DateTime::from_chrono(now)});
        let node=db.collection::<Document>(crate::models::node::COLLECTION_NAME)
            .update_one(filter,doc!{"$set":{"machine_usage_received_at":bson::DateTime::from_chrono(now)}})
            .session(&mut session).await?;
        if node.matched_count!=1 {return Err(AppError::MachineAuthorityStale);}
        let b=&usage.managed_runtime;
        let changed=db.collection::<Document>(MACHINES).update_one(doc!{
            "_id":&b.node_id,"sandbox_uid":&b.sandbox_uid,"pod_uid":&b.pod_uid,
            "runtime_generation":b.runtime_generation as i64,"credential_epoch":b.credential_epoch as i64,
            "writer_fence":&b.writer_fence,"resource_state":{"$nin":["deleted","deleting"]},
            "$or":[{"usage.sampled_at_ms":{"$lt":usage.sampled_at_ms}},{"usage":null},
                {"usage.managed_runtime.runtime_generation":{"$ne":b.runtime_generation as i64}}]
        },doc!{"$set":{"usage":bson::to_bson(&usage).map_err(|_|AppError::MachineAuthorityStale)?,
            "usage_received_at":bson::DateTime::from_chrono(now)}}).session(&mut session).await?;
        if changed.matched_count!=1 {return Err(AppError::MachineAuthorityStale);}
        Ok(())
    }.await;
    match result {
        Ok(()) => {
            session.commit_transaction().await?;
            Ok(())
        }
        Err(e) => {
            let _ = session.abort_transaction().await;
            Err(e)
        }
    }
}

/// Validate current measurements: workspace is measured, temporary paths are
/// a lower bound without an allocated capacity or available-space promise.
fn normalize_usage(usage: &Usage) -> AppResult<Usage> {
    let invalid = || AppError::ValidationError("Invalid managed usage measurement".into());
    let workspace = &usage.workspace;
    let capacity = workspace
        .capacity_bytes
        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
        .ok_or_else(invalid)?;
    if workspace.used_bytes > capacity
        || workspace.available_bytes.is_none_or(|v| v > capacity)
        || !workspace.complete
        || !matches!(workspace.measurement.as_str(), "filesystem" | "statvfs")
        || usage.temporary.used_bytes > i64::MAX as u64
        || usage.temporary.capacity_bytes.is_some()
        || usage.temporary.available_bytes.is_some()
        || usage.temporary.measurement != "node_paths_lower_bound"
        || usage.temporary.complete
    {
        return Err(invalid());
    }
    let mut normalized = usage.clone();
    normalized.disk_full = workspace.available_bytes == Some(0);
    Ok(normalized)
}

pub struct Observation {
    pub usage: Usage,
    pub received_at: DateTime<Utc>,
    pub stale: bool,
}

pub async fn owner_observation(
    db: &Database,
    owner: &str,
    node: &str,
) -> AppResult<Option<Observation>> {
    let machine = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":node,"owner_id":owner})
        .await?
        .filter(|m| m.resource_state != crate::models::remote_machine::ResourceState::Deleted)
        .ok_or_else(|| AppError::NotFound("Remote Machine not found".into()))?;
    let row = db
        .collection::<Document>(MACHINES)
        .find_one(doc! {"_id":node,"owner_id":owner})
        .projection(doc! {"usage":1,"usage_received_at":1})
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    let Some(usage) = row.get("usage") else {
        return Ok(None);
    };
    let usage: Usage = bson::from_bson(usage.clone())
        .map_err(|_| AppError::Internal("Invalid usage observation".into()))?;
    let usage = normalize_usage(&usage)?;
    let received_at = row
        .get_datetime("usage_received_at")
        .map_err(|_| AppError::Internal("Invalid usage observation time".into()))?
        .to_chrono();
    let now = Utc::now();
    let current_runtime = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":node,"is_active":true,"machine.runtime_id":&usage.runtime_id})
        .projection(doc! {"_id":1})
        .await?
        .is_some();
    let stale = received_at < now - chrono::Duration::seconds(90)
        || received_at > now + chrono::Duration::seconds(30)
        || usage.sampled_at_ms < (now - chrono::Duration::seconds(90)).timestamp_millis()
        || usage.sampled_at_ms > (now + chrono::Duration::seconds(30)).timestamp_millis()
        || usage.managed_runtime.runtime_generation != machine.runtime_generation as u64
        || usage.managed_runtime.credential_epoch != machine.credential_epoch as u64
        || machine.pod_uid.as_deref() != Some(usage.managed_runtime.pod_uid.as_str())
        || machine.sandbox_uid.as_deref() != Some(usage.managed_runtime.sandbox_uid.as_str())
        || machine.writer_fence != usage.managed_runtime.writer_fence
        || !current_runtime;
    Ok(Some(Observation {
        usage,
        received_at,
        stale,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyxid_machine::managed::UsageVolume;

    #[tokio::test]
    async fn observations_require_current_socket_runtime_and_generation_and_remain_owner_only() {
        let (db, _, binding, _) = super::super::remote_machine_bootstrap::tests::fixture().await;
        let owner = db
            .collection::<ManagedMachine>(MACHINES)
            .find_one(doc! {"_id":&binding.node_id})
            .await
            .unwrap()
            .unwrap()
            .owner_id;
        let fence = NodeOwnerFence {
            node_id: binding.node_id.clone(),
            instance_name: "replica-test".into(),
            generation_id: uuid::Uuid::new_v4().to_string(),
            connection_id: uuid::Uuid::new_v4().to_string(),
        };
        let runtime_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc!{"_id":&binding.node_id},doc!{"$set":{
            "is_active":true,"machine.runtime_id":&runtime_id,"connection_owner":{
                "instance_name":&fence.instance_name,"generation_id":&fence.generation_id,"connection_id":&fence.connection_id,
                "expires_at":bson::DateTime::from_chrono(now+chrono::Duration::minutes(1))
            }
        }}).await.unwrap();
        let volume = UsageVolume {
            capacity_bytes: Some(20 * 1024 * 1024 * 1024),
            used_bytes: 1234,
            available_bytes: Some(20000),
            measurement: "filesystem".into(),
            complete: true,
        };
        let mut usage = Usage {
            runtime_id: runtime_id.clone(),
            managed_runtime: binding.clone(),
            sampled_at_ms: now.timestamp_millis(),
            workspace: volume.clone(),
            temporary: UsageVolume {
                capacity_bytes: None,
                used_bytes: 100,
                available_bytes: None,
                measurement: "node_paths_lower_bound".into(),
                complete: false,
            },
            disk_full: false,
        };
        record(&db, &fence, &usage).await.unwrap();
        assert_eq!(
            owner_observation(&db, &owner, &binding.node_id)
                .await
                .unwrap()
                .unwrap()
                .usage
                .temporary
                .capacity_bytes,
            None
        );
        let mut legacy = usage.clone();
        legacy.temporary.used_bytes = 3 * 1024 * 1024 * 1024;
        let unknown_large = normalize_usage(&legacy).unwrap();
        assert_eq!(unknown_large.temporary.capacity_bytes, None);
        assert!(
            !unknown_large.disk_full,
            "partial use above 2 GiB is not a full-disk observation"
        );
        legacy.temporary.capacity_bytes = Some(2147483648);
        legacy.temporary.available_bytes = Some(2147483548);
        legacy.disk_full = true;
        assert!(normalize_usage(&legacy).is_err());
        let mut invalid = usage.clone();
        invalid.workspace.capacity_bytes = None;
        assert!(normalize_usage(&invalid).is_err());
        invalid.workspace = usage.workspace.clone();
        invalid.workspace.available_bytes = Some(invalid.workspace.capacity_bytes.unwrap() + 1);
        assert!(normalize_usage(&invalid).is_err());
        invalid.workspace = usage.workspace.clone();
        invalid.temporary.complete = true;
        assert!(normalize_usage(&invalid).is_err());
        assert!(
            record(&db, &fence, &usage).await.is_err(),
            "out-of-order replay is not a fresh sample"
        );
        let view = owner_observation(&db, &owner, &binding.node_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!view.stale);
        assert!(!view.usage.temporary.complete);
        assert!(
            owner_observation(&db, &uuid::Uuid::new_v4().to_string(), &binding.node_id)
                .await
                .is_err()
        );
        usage.sampled_at_ms += 1;
        usage.managed_runtime.runtime_generation += 1;
        assert!(record(&db, &fence, &usage).await.is_err());
        usage.managed_runtime = binding.clone();
        usage.runtime_id = uuid::Uuid::new_v4().to_string();
        assert!(record(&db, &fence, &usage).await.is_err());
        usage.runtime_id = runtime_id;
        let mut other = fence.clone();
        other.connection_id = uuid::Uuid::new_v4().to_string();
        assert!(record(&db, &other, &usage).await.is_err());
        db.collection::<Document>(MACHINES).update_one(doc!{"_id":&binding.node_id},doc!{"$set":{"usage_received_at":bson::DateTime::from_chrono(now-chrono::Duration::seconds(91))}}).await.unwrap();
        assert!(
            owner_observation(&db, &owner, &binding.node_id)
                .await
                .unwrap()
                .unwrap()
                .stale
        );
        // Fresh delivery cannot refresh an old sample or a replaced Runtime.
        db.collection::<Document>(MACHINES).update_one(doc!{"_id":&binding.node_id},doc!{"$set":{"usage_received_at":bson::DateTime::now(),"usage.sampled_at_ms":(now-chrono::Duration::seconds(119)).timestamp_millis()}}).await.unwrap();
        assert!(
            owner_observation(&db, &owner, &binding.node_id)
                .await
                .unwrap()
                .unwrap()
                .stale
        );
        db.collection::<Document>(MACHINES)
            .update_one(
                doc! {"_id":&binding.node_id},
                doc! {"$set":{"usage.sampled_at_ms":Utc::now().timestamp_millis()}},
            )
            .await
            .unwrap();
        assert!(
            !owner_observation(&db, &owner, &binding.node_id)
                .await
                .unwrap()
                .unwrap()
                .stale
        );
        db.collection::<Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&binding.node_id},
                doc! {"$set":{"machine.runtime_id":uuid::Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
        assert!(
            owner_observation(&db, &owner, &binding.node_id)
                .await
                .unwrap()
                .unwrap()
                .stale
        );
        db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc!{"_id":&binding.node_id},doc!{"$set":{"connection_owner.expires_at":bson::DateTime::from_chrono(now-chrono::Duration::seconds(1))}}).await.unwrap();
        assert!(record(&db, &fence, &usage).await.is_err());
        db.drop().await.unwrap();
    }
}
