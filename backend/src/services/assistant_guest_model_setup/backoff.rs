//! Retry state belongs to the durable setup claim, not a replica or a guest.
use super::*;

const MAX_FAILURES: i64 = 7; // 60, 120, 240, 480, 960, 1920, 3600 seconds.

/// One atomic terminal transition, shared by completion, timeout and recovery.
/// Missing fields are the legacy short-retry state. A reset during an active
/// attempt takes effect at settlement without allowing a second concurrent turn.
pub(super) fn terminal(mut set: Document, unset: Document, success: bool) -> Vec<Document> {
    let failures = if success {
        bson::Bson::Int64(0)
    } else {
        bson::Bson::Document(doc! {"$cond":[
            {"$eq":["$retry_reset",true]},0_i64,
            {"$min":[MAX_FAILURES,{"$add":[{"$ifNull":["$model_failures",0_i64]},
                {"$cond":[{"$eq":["$model_dispatched",true]},1,0]}]}]}
        ]})
    };
    set.insert("model_failures", failures);
    // Treat every supplied value as data, including route strings.
    let mut values = Document::new();
    for (key, value) in set {
        values.insert(
            key.clone(),
            if key == "model_failures" {
                value
            } else {
                bson::Bson::Document(doc! {"$literal":value})
            },
        );
    }
    let exponent = doc! {"$cond": [
        {"$eq": ["$model_dispatched", true]},
        {"$max": [0, {"$subtract": ["$model_failures", 1]}]}, 0
    ]};
    let delay = doc! {"$cond": [
        {"$eq": ["$retry_reset", true]}, 0,
        {"$min": [3_600_000_i64, {"$multiply": [RETRY * 1000, {"$pow": [2, exponent]}]}]}
    ]};
    let mut pipeline = vec![
        doc! {"$set": values},
        doc! {"$set": {
            "until": {"$add": ["$$NOW", delay]}
        }},
    ];
    if !unset.is_empty() {
        pipeline.push(doc! {"$unset":unset.keys().cloned().collect::<Vec<_>>()});
    }
    pipeline
}

/// Only explicit owner setup actions call this. In particular, guest mapping
/// retries (`force`) never reset a failed generation's delay.
pub(crate) async fn reset(
    db: &mongodb::Database,
    actor: &str,
    agent: &str,
    profile: Option<&str>,
) -> AppResult<()> {
    let mut filter = doc! {"kind":KIND,"user_id":actor,"agent_id":agent};
    if let Some(profile) = profile {
        filter.insert("profile", profile);
    }
    db.collection::<Document>(execution::COLLECTION)
        .update_many(
            filter,
            vec![doc! {"$set":{
                "model_failures":0_i64,"retry_reset":true,
                "until":{"$cond":[{"$eq":["$status","failed"]},"$$NOW","$until"]}
            }}],
        )
        .await?;
    Ok(())
}

/// Fingerprint configuration, excluding access timestamps, refreshed token
/// bytes, OAuth health status and counters. Only the resulting digest is retained. Target IDs come
/// from the admitted owner callback, never from guest input.
pub(super) async fn fingerprint(
    db: &mongodb::Database,
    agent: &AssistantAgent,
    target: Option<&Document>,
) -> AppResult<String> {
    let mut value = json!({"model":agent.model,"settings":agent.updated_at});
    if let Some(target) = target {
        let catalog = target.get_str("catalog").unwrap_or_default();
        value["catalog"] = json!(db.collection::<Document>("downstream_services").find_one(doc!{"_id":catalog})
            .projection(doc!{"_id":1,"slug":1,"is_active":1,"base_url":1,"auth_method":1,"auth_key_name":1,"auth_scheme":1,
                "credential_encrypted":1,"provider_config_id":1,"platform_key":1,"requires_user_credential":1,
                "default_request_headers":1,"default_request_header_bindings":1,"inference":1}).await?);
        if let Ok(connection) = target.get_str("connection") {
            let mut connection = db.collection::<Document>("user_services").find_one(doc!{"_id":connection})
                .projection(doc!{"_id":1,"slug":1,"is_active":1,"endpoint_id":1,"api_key_id":1,"credential_binding":1,
                    "default_request_headers":1,"auth_method":1,"auth_key_name":1,"catalog_service_id":1,
                    "node_id":1,"node_priority":1,"deleted_at":1}).await?;
            if let Some(row) = connection.as_mut() {
                if row.get_str("credential_binding") == Ok("platform") {
                    row.remove("api_key_id");
                }
                if let Ok(key) = row.get_str("api_key_id") {
                    value["credential"] = json!(db.collection::<Document>("user_api_keys").find_one(doc!{"_id":key})
                        .projection(doc!{"_id":1,"credential_epoch":1,"provider_config_id":1,"connection_id":1,"credential_type":1}).await?);
                }
                if let Ok(endpoint) = row.get_str("endpoint_id") {
                    value["endpoint"] = json!(
                        db.collection::<Document>("user_endpoints")
                            .find_one(doc! {"_id":endpoint})
                            .projection(doc! {"_id":1,"url":1,"catalog_service_id":1})
                            .await?
                    );
                }
            }
            value["connection"] = json!(connection);
        }
    }
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&value)
            .map_err(|_| AppError::Internal("Setup configuration unavailable".into()))?,
    )))
}

/// A changed owner configuration unlocks a failed generation exactly once.
/// This CAS cannot reset or shorten a newer worker's active claim.
pub(super) async fn refresh(
    db: &mongodb::Database,
    agent: &AssistantAgent,
    record: &Document,
) -> AppResult<()> {
    if record.get_str("status") != Ok("failed") {
        return Ok(());
    }
    let Some(old) = record.get_str("retry_config").ok() else {
        return Ok(());
    };
    let current = fingerprint(db, agent, record.get_document("retry_target").ok()).await?;
    if current != old {
        db.collection::<Document>(execution::COLLECTION).update_one(
            doc!{"_id":record.get_str("_id").unwrap_or_default(),"claim":record.get_str("claim").unwrap_or_default(),
                "status":"failed","retry_config":old},
            doc!{"$set":{"model_failures":0_i64,"until":bson::DateTime::now(),"retry_config":current}},
        ).await?;
    }
    Ok(())
}

/// Called by the final model egress guard, after authority and funding checks.
/// Persist before attempting the send: an uncertain network outcome or a crash
/// must not turn a potentially paid attempt into a free rapid retry.
pub(crate) async fn dispatched(db: &mongodb::Database, conversation: &str) -> AppResult<()> {
    let Some(record) = db.collection::<Document>(execution::COLLECTION).find_one(
        doc!{"kind":KIND,"conversation_id":conversation,"status":"setting_up","until":{"$gt":bson::DateTime::now()}}
    ).await? else { return Err(AppError::AssistantTurnRequired); };
    if record.get_bool("model_dispatched") == Ok(true) {
        return Ok(());
    }
    let agent = team::agent(
        db,
        record.get_str("user_id").unwrap_or_default(),
        record.get_str("agent_id").unwrap_or_default(),
    )
    .await?;
    let row = engine::get(
        db,
        record.get_str("user_id").unwrap_or_default(),
        conversation,
    )
    .await?;
    let route = row
        .owner_model_route
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let target = doc! {"catalog":&route.catalog_service_id,"connection":&route.connection_id};
    let fingerprint = fingerprint(db, &agent, Some(&target)).await?;
    let changed = db.collection::<Document>(execution::COLLECTION).update_one(
        doc!{"_id":record.get_str("_id").unwrap_or_default(),"claim":record.get_str("claim").unwrap_or_default(),
            "status":"setting_up","until":{"$gt":bson::DateTime::now()}},
        doc!{"$set":{"model_dispatched":true,"retry_target":target,"retry_config":fingerprint}},
    ).await?;
    if changed.matched_count != 1 {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(())
}

#[cfg(test)]
#[path = "backoff_tests.rs"]
mod tests;
