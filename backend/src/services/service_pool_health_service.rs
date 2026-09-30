use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_pool::{CooldownPolicy, ServicePool};
use crate::models::service_pool_member_health::{
    COLLECTION_NAME as HEALTH, OBSERVATION_COLLECTION_NAME as OBSERVATIONS,
    ServicePoolHealthObservation, ServicePoolMemberHealth,
};
use crate::services::pool_failover::cooldown_for;

const SERVICE_POOLS: &str = "service_pools";
pub const HEALTH_RETENTION_MS: u64 = 60 * 60 * 1_000;
const HEALTH_RETENTION: Duration = Duration::milliseconds(HEALTH_RETENTION_MS as i64);

#[derive(Clone, PartialEq, Eq)]
pub struct HealthScope {
    pub pool_id: String,
    pub user_service_id: String,
    pub owner_id: String,
    pub pool_config_revision: i64,
    pub credential_identity: String,
    pub credential_epoch: i64,
    pub destination_fingerprint: String,
    pub config_fingerprint: String,
    pub model: Option<String>,
}

impl std::fmt::Debug for HealthScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthScope")
            .field("pool_id", &self.pool_id)
            .field("user_service_id", &self.user_service_id)
            .field("owner_id", &self.owner_id)
            .field("pool_config_revision", &self.pool_config_revision)
            .field("credential_identity", &"[REDACTED]")
            .field("credential_epoch", &self.credential_epoch)
            .field("destination_fingerprint", &"[REDACTED]")
            .field("config_fingerprint", &"[REDACTED]")
            .field("model", &self.model)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationTicket {
    pub observation_id: String,
    pub observation_sequence: i64,
    pub scope: HealthScope,
    pub pool_reset_generation: i64,
    pub member_reset_generation: i64,
    pub health_instance_id: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn scope_from_resolution(
    db: &mongodb::Database,
    pool_id: &str,
    owner_id: &str,
    pool_config_revision: i64,
    resolution: &crate::services::proxy_service::UserServiceResolution,
    model: Option<String>,
    override_identity: Option<&crate::services::proxy_service::AgentCredentialOverrideIdentity>,
    method: &str,
    path: &str,
) -> AppResult<HealthScope> {
    use sha2::{Digest, Sha256};
    let override_projection = override_identity.map(|identity| {
        crate::services::execution_authority::OverrideCredentialIdentity {
            api_key_id: identity.api_key_id.clone(),
            credential_epoch: identity.credential_epoch,
        }
    });
    let nodes = crate::services::node_routing_service::list_configured_binding_node_ids(
        db,
        owner_id,
        &resolution.target.service.id,
    )
    .await?;
    let mut projection = crate::services::execution_authority::build_projection(
        resolution,
        override_projection.as_ref(),
        nodes,
    );
    let canonical = crate::services::proxy_authorization::CanonicalPath::from_rest_decoded(path)?;
    let (native_path, target_id) = crate::services::destination_routing::select_target(
        &resolution.target.service,
        method,
        &canonical,
    )?;
    if let Some(id) = &target_id {
        projection.destination_base_url = resolution.target.service.destination_targets[id].clone();
    }
    let destination_fingerprint = format!(
        "{:x}",
        Sha256::digest(projection.destination_base_url.as_bytes())
    );
    let credential_identity = projection
        .credential
        .override_api_key_id
        .clone()
        .or(projection.credential.api_key_id.clone())
        .or_else(|| resolution.master_credential_revision.clone())
        .unwrap_or_else(|| {
            format!(
                "master:{}:{}",
                resolution
                    .catalog_service_slug
                    .as_deref()
                    .unwrap_or(&resolution.user_service_id),
                crate::services::execution_authority::digest(&projection)
            )
        });
    let config = serde_json::to_vec(&serde_json::json!({
        "authority": crate::services::execution_authority::digest(&projection),
        "method": method, "native_path": native_path, "target_id": target_id,
        "inference": resolution.target.service.inference,
        "catalog_transport": resolution.catalog_service_slug,
    }))
    .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(HealthScope {
        pool_id: pool_id.to_string(),
        user_service_id: resolution.user_service_id.clone(),
        owner_id: owner_id.to_string(),
        pool_config_revision,
        credential_identity,
        credential_epoch: projection
            .credential
            .override_epoch
            .or(Some(projection.credential.credential_epoch))
            .unwrap_or(1),
        destination_fingerprint,
        config_fingerprint: format!("{:x}", Sha256::digest(&config)),
        model,
    })
}

fn scope_filter(
    scope: &HealthScope,
    pool_reset_generation: i64,
    member_reset_generation: i64,
) -> mongodb::bson::Document {
    doc! {
        "pool_id": &scope.pool_id,
        "user_service_id": &scope.user_service_id,
        "owner_id": &scope.owner_id,
        "pool_config_revision": scope.pool_config_revision,
        "credential_identity": &scope.credential_identity,
        "credential_epoch": scope.credential_epoch,
        "destination_fingerprint": &scope.destination_fingerprint,
        "config_fingerprint": &scope.config_fingerprint,
        "model": &scope.model,
        "pool_reset_generation": pool_reset_generation,
        "member_reset_generation": member_reset_generation,
    }
}

async fn current_pool(
    db: &mongodb::Database,
    scope: &HealthScope,
) -> AppResult<Option<ServicePool>> {
    Ok(db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one(doc! {
            "_id": &scope.pool_id,
            "user_id": &scope.owner_id,
            "is_active": true,
            "config_revision": scope.pool_config_revision,
        })
        .await?)
}

fn member_reset_generation(pool: &ServicePool, member_id: &str) -> Option<i64> {
    pool.members
        .iter()
        .find(|member| member.user_service_id == member_id && member.enabled)
        .map(|member| member.health_reset_generation)
}

async fn current_fences(
    db: &mongodb::Database,
    scope: &HealthScope,
) -> AppResult<Option<(i64, i64)>> {
    let Some(pool) = current_pool(db, scope).await? else {
        return Ok(None);
    };
    Ok(member_reset_generation(&pool, &scope.user_service_id)
        .map(|member| (pool.health_reset_generation, member)))
}

/// Issue a ticket by atomically incrementing the pool's durable observation
/// sequence. The exact scoped health row is created before any credential is
/// materialized. Its UUID fences TTL deletion/re-insertion (ABA).
pub async fn issue_observation_ticket(
    db: &mongodb::Database,
    scope: HealthScope,
) -> AppResult<ObservationTicket> {
    let now = Utc::now();
    let pools = db.collection::<ServicePool>(SERVICE_POOLS);
    let pool = pools
        .find_one_and_update(
            doc! {
                "_id": &scope.pool_id,
                "user_id": &scope.owner_id,
                "is_active": true,
                "config_revision": scope.pool_config_revision,
                "members": { "$elemMatch": {
                    "user_service_id": &scope.user_service_id,
                    "enabled": { "$ne": false },
                }},
            },
            doc! {
                "$inc": { "health_observation_sequence": 1_i64 },
            },
        )
        .with_options(
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::Conflict("pool membership changed before dispatch".to_string()))?;
    let member_reset_generation = member_reset_generation(&pool, &scope.user_service_id)
        .ok_or_else(|| AppError::Conflict("pool membership changed before dispatch".to_string()))?;
    let pool_reset_generation = pool.health_reset_generation;
    let observation_sequence = pool.health_observation_sequence;
    let observation_id = Uuid::new_v4().to_string();
    let health = db
        .collection::<ServicePoolMemberHealth>(HEALTH)
        .find_one_and_update(
            scope_filter(&scope, pool_reset_generation, member_reset_generation),
            doc! {
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "pool_id": &scope.pool_id,
                    "user_service_id": &scope.user_service_id,
                    "owner_id": &scope.owner_id,
                    "pool_config_revision": scope.pool_config_revision,
                    "credential_identity": &scope.credential_identity,
                    "credential_epoch": scope.credential_epoch,
                    "destination_fingerprint": &scope.destination_fingerprint,
                    "config_fingerprint": &scope.config_fingerprint,
                    "model": &scope.model,
                    "consecutive_failures": 0_i64,
                    "cooldown_until": bson::Bson::Null,
                    "last_failure_class": bson::Bson::Null,
                    "last_status": bson::Bson::Null,
                    "observation_id": bson::Bson::Null,
                    "observation_sequence": 0_i64,
                    "observation_outcome": bson::Bson::Null,
                    "last_failure_at": bson::DateTime::from_chrono(now),
                },
                "$max": { "generation": observation_sequence },
                "$set": {
                    "pool_reset_generation": pool_reset_generation,
                    "member_reset_generation": member_reset_generation,
                    "expires_at": bson::DateTime::from_chrono(now + HEALTH_RETENTION),
                    "updated_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .with_options(
            mongodb::options::FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::Internal("health row was not created".to_string()))?;
    db.collection::<ServicePoolHealthObservation>(OBSERVATIONS)
        .insert_one(ServicePoolHealthObservation {
            id: observation_id.clone(),
            pool_id: scope.pool_id.clone(),
            user_service_id: scope.user_service_id.clone(),
            pool_config_revision: scope.pool_config_revision,
            pool_reset_generation,
            member_reset_generation,
            observation_sequence,
            health_instance_id: health.id.clone(),
            completed: false,
            outcome: None,
            failure_class: None,
            status: None,
            expires_at: now + HEALTH_RETENTION,
        })
        .await?;
    Ok(ObservationTicket {
        observation_id,
        observation_sequence,
        scope,
        pool_reset_generation,
        member_reset_generation,
        health_instance_id: health.id,
    })
}

pub async fn load_for_scope(
    db: &mongodb::Database,
    scope: &HealthScope,
) -> AppResult<Option<ServicePoolMemberHealth>> {
    let Some((pool_reset, member_reset)) = current_fences(db, scope).await? else {
        return Ok(None);
    };
    Ok(db
        .collection::<ServicePoolMemberHealth>(HEALTH)
        .find_one(scope_filter(scope, pool_reset, member_reset))
        .await?)
}

/// Commit a ticket's terminal observation and its health effect together. Ticket
/// rows expire independently of the hot health row and outcomes never recreate
/// them, so expiry cannot make an ancient ticket applicable again.
async fn finish_observation(
    db: &mongodb::Database,
    ticket: &ObservationTicket,
    failure: Option<(
        &CooldownPolicy,
        &str,
        Option<i32>,
        Option<std::time::Duration>,
    )>,
) -> AppResult<(bool, Option<ServicePoolMemberHealth>)> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let ticket = ticket.clone();
    let failure = failure.map(|(policy, class, status, retry_after)| {
        (policy.clone(), class.to_owned(), status, retry_after)
    });
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<(bool, Option<ServicePoolMemberHealth>)> = async {
                let now = Utc::now();
                let observations = db.collection::<ServicePoolHealthObservation>(OBSERVATIONS);
                let health = db.collection::<ServicePoolMemberHealth>(HEALTH);
                let mut filter = scope_filter(
                    &ticket.scope,
                    ticket.pool_reset_generation,
                    ticket.member_reset_generation,
                );
                filter.insert("_id", &ticket.health_instance_id);
                let current = health
                    .find_one(filter.clone())
                    .session(&mut *session)
                    .await?;
                let Some(current) = current else {
                    return Ok((false, None));
                };
                let claimed = observations.update_one(doc! {
                "_id": &ticket.observation_id,
                "health_instance_id": &ticket.health_instance_id,
                "completed": false,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            }, doc! { "$set": {
                "completed": true,
                "outcome": if failure.is_some() { "failure" } else { "success" },
                "failure_class": failure.as_ref().map(|(_, class, _, _)| class.as_str()),
                "status": failure.as_ref().and_then(|(_, _, status, _)| *status),
            }}).session(&mut *session).await?;
                if claimed.modified_count == 0 {
                    return Ok((false, Some(current)));
                }
                // Reset/removal cannot be undone: outcomes target the immutable
                // parent revisions, and routing only reads the current revisions.
                let pool = db
                    .collection::<ServicePool>(SERVICE_POOLS)
                    .find_one(doc! {
                        "_id": &ticket.scope.pool_id,
                        "user_id": &ticket.scope.owner_id,
                        "is_active": true,
                        "config_revision": ticket.scope.pool_config_revision,
                    })
                    .session(&mut *session)
                    .await?;
                if !pool.as_ref().is_some_and(|pool| {
                    pool.health_reset_generation == ticket.pool_reset_generation
                        && member_reset_generation(pool, &ticket.scope.user_service_id)
                            == Some(ticket.member_reset_generation)
                }) {
                    return Ok((false, None));
                }
                let mut set = doc! {
                    "updated_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(now + HEALTH_RETENTION),
                    "generation": current.generation.saturating_add(1),
                };
                if let Some((policy, class, status, retry_after)) = &failure {
                    let failures = current.consecutive_failures.saturating_add(1);
                    let proposed = if failures >= policy.failures_to_open.max(1) {
                        now.checked_add_signed(
                            Duration::from_std(cooldown_for(policy, failures, *retry_after))
                                .unwrap_or_default(),
                        )
                    } else {
                        None
                    };
                    let cooldown = match (current.cooldown_until, proposed) {
                        (Some(old), Some(new)) => Some(old.max(new)),
                        (Some(old), None) => Some(old),
                        (None, proposed) => proposed,
                    };
                    set.insert("consecutive_failures", i64::from(failures));
                    set.insert(
                        "cooldown_until",
                        cooldown
                            .map(|time| bson::Bson::DateTime(bson::DateTime::from_chrono(time)))
                            .unwrap_or(bson::Bson::Null),
                    );
                    if current.observation_sequence <= ticket.observation_sequence {
                        set.extend(doc! {
                            "observation_id": &ticket.observation_id,
                            "observation_sequence": ticket.observation_sequence,
                            "observation_outcome": "failure",
                            "last_failure_class": class,
                            "last_status": *status,
                            "last_failure_at": bson::DateTime::from_chrono(now),
                        });
                    }
                } else {
                    if current.observation_sequence > ticket.observation_sequence {
                        return Ok((false, Some(current)));
                    }
                    set.extend(doc! {
                        "consecutive_failures": 0_i64,
                        "cooldown_until": bson::Bson::Null,
                        "last_failure_class": bson::Bson::Null,
                        "last_status": bson::Bson::Null,
                        "observation_id": &ticket.observation_id,
                        "observation_sequence": ticket.observation_sequence,
                        "observation_outcome": "success",
                    });
                }
                let updated = health
                    .find_one_and_update(filter, doc! { "$set": set })
                    .return_document(mongodb::options::ReturnDocument::After)
                    .session(&mut *session)
                    .await?;
                Ok((updated.is_some(), updated))
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn record_failure(
    db: &mongodb::Database,
    ticket: &ObservationTicket,
    policy: &CooldownPolicy,
    failure_class: &str,
    status: Option<i32>,
    retry_after: Option<std::time::Duration>,
) -> AppResult<Option<ServicePoolMemberHealth>> {
    Ok(finish_observation(
        db,
        ticket,
        Some((policy, failure_class, status, retry_after)),
    )
    .await?
    .1)
}

pub async fn record_success(db: &mongodb::Database, ticket: &ObservationTicket) -> AppResult<bool> {
    Ok(finish_observation(db, ticket, None).await?.0)
}

pub async fn reset_pool(
    db: &mongodb::Database,
    pool_id: &str,
    owner_id: &str,
    user_service_id: Option<&str>,
) -> AppResult<u64> {
    let pools = db.collection::<ServicePool>(SERVICE_POOLS);
    let now = Utc::now();
    let modified_count = if let Some(member_id) = user_service_id {
        if pools
            .find_one(doc! {
                "_id": pool_id,
                "user_id": owner_id,
                "members.user_service_id": member_id,
            })
            .await?
            .is_none()
        {
            return Err(AppError::ServicePoolMemberInvalid(
                "service pool member not found".to_string(),
            ));
        }
        let pipeline = vec![doc! {
            "$set": {
                "members": {
                    "$map": {
                        "input": "$members",
                        "as": "member",
                        "in": {
                            "$cond": [
                                { "$eq": ["$$member.user_service_id", member_id] },
                                { "$mergeObjects": [
                                    "$$member",
                                    { "health_reset_generation": {
                                        "$add": [
                                            { "$ifNull": ["$$member.health_reset_generation", 0_i64] },
                                            1_i64
                                        ]
                                    }}
                                ]},
                                "$$member"
                            ]
                        }
                    }
                },
                "updated_at": bson::DateTime::from_chrono(now),
            }
        }];
        let changed = pools
            .update_one(
                doc! {
                    "_id": pool_id,
                    "user_id": owner_id,
                    "members.user_service_id": member_id,
                },
                pipeline,
            )
            .await?;
        if changed.matched_count == 0 {
            return Err(AppError::ServicePoolMemberInvalid(
                "service pool member not found".to_string(),
            ));
        }
        changed.modified_count
    } else {
        let changed = pools
            .update_one(
                doc! { "_id": pool_id, "user_id": owner_id },
                doc! {
                    "$inc": { "health_reset_generation": 1_i64 },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) },
                },
            )
            .await?;
        if changed.matched_count == 0 {
            return Err(AppError::ServicePoolNotFound(pool_id.to_string()));
        }
        changed.modified_count
    };
    // Existing rows retain their immutable fence and expire naturally. Current
    // reads select only the new generation, so a reset racing a post-reset
    // failure cannot clear or roll back that new row.
    Ok(modified_count)
}

#[cfg(test)]
pub fn is_cooling(health: &ServicePoolMemberHealth, now: chrono::DateTime<Utc>) -> bool {
    health.cooldown_until.is_some_and(|until| until > now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_redact_secret_identity_in_debug() {
        let scope = HealthScope {
            pool_id: "p".into(),
            user_service_id: "s".into(),
            owner_id: "o".into(),
            pool_config_revision: 3,
            credential_identity: "secret-key-fingerprint".into(),
            credential_epoch: 1,
            destination_fingerprint: "url-fingerprint".into(),
            config_fingerprint: "config-fingerprint".into(),
            model: Some("m".into()),
        };
        let debug = format!("{scope:?}");
        assert!(!debug.contains("secret-key-fingerprint"));
        assert!(debug.contains("REDACTED"));
    }
}
