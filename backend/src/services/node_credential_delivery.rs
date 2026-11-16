//! Durable ordinary credential delivery. Material never enters the queue.
use std::{sync::Arc, time::Duration};

use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, options::ReturnDocument};

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{user_api_key::UserApiKey, user_endpoint::UserEndpoint, user_service::UserService},
    services::{
        coordination_service::{ClusterLeaseRuntime, LeaseStore, cluster_lease_runtime},
        credential_push_service::{self, ActorScope},
        node_ws_manager::{CredentialUpdateParams, CredentialVersion, NodeWsManager},
    },
};

pub use crate::models::node_credential_desired::COLLECTION_NAME as COLLECTION;
use crate::models::node_credential_desired::NodeCredentialDesired;
pub const RETRY_LEASE: &str = "node:credential-delivery";
const CLAIM_SECS: i64 = 30;
const OFFLINE_RETRY_BASE_SECS: i64 = 30;
const OFFLINE_RETRY_MAX_SECS: i64 = 600;
const TRANSIENT_RETRY_BASE_SECS: i64 = 5;
const TRANSIENT_RETRY_MAX_SECS: i64 = 60;
const SENT_UNCONFIRMED: &str = "sent_unconfirmed";
const ACKNOWLEDGEMENT_UNSUPPORTED: &str = "acknowledgement_unsupported";
const SERVICE_DISABLED: &str = "service_disabled";
const KEY_INACTIVE: &str = "key_inactive";
const PREPARATION_SECS: i64 = 120;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const PASS_INTERVAL: Duration = Duration::from_secs(5);
const PASS_LIMIT: i64 = 64;

tokio::task_local! {
    static ACTOR_SCOPE: (String, ActorScope);
    static PREPARING: ();
}

pub async fn with_actor<T>(
    actor: String,
    scope: ActorScope,
    future: impl std::future::Future<Output = T>,
) -> T {
    ACTOR_SCOPE.scope((actor, scope), future).await
}

/// The strict handler writes material before its routing/auth proposal. Gate
/// retry atomically with that first write, including death before prepare_update.
pub async fn with_prepared_actor<T>(
    actor: String,
    scope: ActorScope,
    future: impl std::future::Future<Output = T>,
) -> T {
    PREPARING.scope((), with_actor(actor, scope, future)).await
}

#[derive(Debug)]
pub struct SlotCreationRace;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(COLLECTION)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"node_id": 1, "service_slug": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"pending": 1, "retry_at": 1})
                .build(),
        ])
        .await?;
    Ok(())
}

fn changed(before: Option<&Document>, after: Option<&Document>, fields: &[&str]) -> bool {
    fields
        .iter()
        .any(|field| before.and_then(|d| d.get(*field)) != after.and_then(|d| d.get(*field)))
}

/// Called inside the existing service-history transaction, including refreshes.
/// Comparing images avoids writes for usage, labels, approval epochs and other metadata.
pub async fn record_changes(
    db: &Database,
    session: &mut ClientSession,
    name: &str,
    before: &[Document],
    after: &[Document],
) -> mongodb::error::Result<()> {
    let fields = match name {
        "user_api_keys" => &[
            "credential_encrypted",
            "access_token_encrypted",
            "refresh_token_encrypted",
            "credential_type",
            "user_id",
            "status",
        ][..],
        "user_services" => &[
            "node_id",
            "slug",
            "api_key_id",
            "auth_method",
            "auth_key_name",
            "credential_binding",
            "is_active",
        ][..],
        _ => return Ok(()),
    };
    let ids: std::collections::BTreeSet<_> = before
        .iter()
        .chain(after)
        .filter_map(|row| row.get_str("_id").ok())
        .collect();
    for id in ids {
        let old = before
            .iter()
            .find(|row| row.get_str("_id").ok() == Some(id));
        let new = after.iter().find(|row| row.get_str("_id").ok() == Some(id));
        if !changed(old, new, fields) {
            continue;
        }
        let mut services = Vec::new();
        if name == "user_api_keys" {
            let mut cursor = db
                .collection::<UserService>("user_services")
                .find(doc! {"api_key_id": id, "node_id": {"$type": "string"}})
                .session(&mut *session)
                .await?;
            while cursor.advance(&mut *session).await? {
                services.push(cursor.deserialize_current()?);
            }
        } else {
            for row in [new, old].into_iter().flatten() {
                if !row.get_str("node_id").is_ok_and(|node| !node.is_empty()) {
                    continue;
                }
                let svc: UserService = bson::from_document(row.clone())?;
                if !services
                    .iter()
                    .any(|s: &UserService| s.node_id == svc.node_id && s.slug == svc.slug)
                {
                    services.push(svc);
                }
            }
        }
        for svc in services {
            let Some(node) = svc.node_id.as_deref().filter(|node| !node.is_empty()) else {
                continue;
            };
            // Inactive historical rows may share this physical node slot with
            // a newer active service. They cannot redirect or suppress its intent.
            if !svc.is_active
                && let Some(slot) = db
                    .collection::<Document>(COLLECTION)
                    .find_one(doc! {"node_id": node, "service_slug": &svc.slug})
                    .projection(doc! {"service_id": 1})
                    .session(&mut *session)
                    .await?
                && let Ok(other) = slot.get_str("service_id")
                && other != svc.id
                && db
                    .collection::<Document>("user_services")
                    .find_one(
                        doc! {"_id": other, "node_id": node, "slug": &svc.slug, "is_active": true},
                    )
                    .projection(doc! {"_id": 1})
                    .session(&mut *session)
                    .await?
                    .is_some()
            {
                continue;
            }
            if svc.credential_binding.as_deref() == Some("platform")
                && !old.is_some_and(|row| {
                    row.get_str("credential_binding").ok() != Some("platform")
                        && row.get_str("api_key_id").is_ok()
                })
            {
                continue;
            }
            if svc.service_type == "ssh" {
                continue;
            }
            if svc.api_key_id.is_none()
                && !(name == "user_services"
                    && old.is_some_and(|row| row.get_str("api_key_id").is_ok()))
                && db
                    .collection::<Document>(COLLECTION)
                    .find_one(doc! {
                        "node_id": node, "service_slug": &svc.slug, "service_id": &svc.id,
                        "suppressed": {"$ne": true},
                    })
                    .projection(doc! {"_id": 1})
                    .session(&mut *session)
                    .await?
                    .is_none()
            {
                // A no-key route may intentionally use a credential managed only
                // on the node. Only an existing explicit delivery intent can
                // authorize a no-auth placeholder for it.
                continue;
            }
            if name == "user_services"
                && let Some(key_id) = svc
                    .api_key_id
                    .as_deref()
                    .or_else(|| old.and_then(|row| row.get_str("api_key_id").ok()))
            {
                let key = db
                    .collection::<Document>("user_api_keys")
                    .find_one(doc! {"_id": key_id})
                    .projection(doc! {"credential_type": 1})
                    .session(&mut *session)
                    .await?;
                // Key deletion already queued server-held removal. Never turn
                // a local-only binding cleanup into a node credential removal.
                if key.is_none_or(|key| key.get_str("credential_type").ok() == Some("node_managed"))
                {
                    continue;
                }
            }
            let actor = ACTOR_SCOPE.try_with(|(actor, scope)| {
                (scope.permits_node(node) && scope.permits_service(&svc.id)).then(|| actor.clone())
            });
            let local_only = name == "user_api_keys"
                && new
                    .or(old)
                    .is_some_and(|row| row.get_str("credential_type").ok() == Some("node_managed"));
            if local_only
                || matches!(actor, Ok(None))
                || (name == "user_api_keys"
                    && ACTOR_SCOPE.try_with(|_| ()).is_ok()
                    && new.is_some_and(|row| row.get_str("provider_config_id").is_ok()))
            {
                // A pending older intent must not become permission to deliver
                // the newly written material outside this caller's fan-out.
                db.collection::<Document>(COLLECTION).update_one(doc! {"node_id": node, "service_slug": &svc.slug}, vec![doc! {"$set": {
                    "generation": {"$add": ["$generation", 1_i64]}, "pending": false, "suppressed": true,
                    "parked_reason": bson::Bson::Null, "retry_attempts": 0_i64,
                }}]).session(&mut *session).await?;
                continue;
            }
            let actor = actor.ok().flatten().or_else(|| {
                super::service_history::context::current()
                    .and_then(|context| context.actor.person_id)
            });
            advance(
                db,
                session,
                node,
                &svc.slug,
                &svc.id,
                &svc.user_id,
                actor.as_deref(),
            )
            .await?;
        }
    }
    Ok(())
}

async fn advance(
    db: &Database,
    session: &mut ClientSession,
    node: &str,
    slug: &str,
    service: &str,
    owner: &str,
    actor: Option<&str>,
) -> mongodb::error::Result<()> {
    let mut fields = doc! {
                "_id": {"$ifNull": ["$_id", {"$literal": uuid::Uuid::new_v4().to_string()}]},
                "node_id": {"$literal": node}, "service_slug": {"$literal": slug},
                "service_id": {"$literal": service}, "owner_id": {"$literal": owner},
                "actor_user_id": {"$literal": actor},
                "generation": {"$add": [{"$ifNull": ["$generation", 0_i64]}, 1_i64]},
                "acknowledged_generation": {"$ifNull": ["$acknowledged_generation", 0_i64]},
                "pending": true, "suppressed": false, "retry_at": "$$NOW",
                "parked_reason": bson::Bson::Null, "retry_attempts": 0_i64,
    };
    if PREPARING.try_with(|_| ()).is_ok() {
        fields.insert("preparing_token", uuid::Uuid::new_v4().to_string());
        fields.insert(
            "preparing_until",
            doc! {"$add": ["$$NOW", PREPARATION_SECS * 1000]},
        );
    }
    let result = db
        .collection::<Document>(COLLECTION)
        .update_one(
            doc! {"node_id": node, "service_slug": slug},
            vec![doc! {"$set": fields}],
        )
        .upsert(true)
        .session(session)
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(error)
            if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000)
                || matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 11000) =>
        {
            Err(mongodb::error::Error::custom(SlotCreationRace))
        }
        Err(error) => Err(error),
    }
}

fn metadata<'a>(row: &'a Document, field: &str) -> AppResult<&'a str> {
    row.get_str(field)
        .map_err(|_| AppError::Internal("Incomplete credential delivery metadata".into()))
}

fn identity(row: &Document) -> AppResult<CredentialVersion> {
    Ok(CredentialVersion {
        credential_id: row
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid delivery identity".into()))?
            .into(),
        credential_generation: row
            .get_i64("generation")
            .map_err(|_| AppError::Internal("Invalid delivery generation".into()))?,
    })
}

struct Material {
    service: UserService,
    key: Option<UserApiKey>,
    target: Option<String>,
    version: CredentialVersion,
}

enum Delivery {
    Update(Box<Material>),
    Remove,
    Parked(&'static str),
    Retired,
}

/// Snapshot the encrypted source and its generation together. Decryption runs
/// after commit so a remote KMS never holds the delivery transaction open.
async fn current_material(
    db: &Database,
    session: &mut ClientSession,
    row: &Document,
) -> AppResult<Delivery> {
    if db
        .collection::<Document>("nodes")
        .find_one(doc! {"_id": metadata(row, "node_id")?, "is_active": true})
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Ok(Delivery::Retired);
    }
    let svc = db
        .collection::<UserService>("user_services")
        .find_one(
            doc! {"_id": metadata(row, "service_id")?, "user_id": metadata(row, "owner_id")?,
            "node_id": metadata(row, "node_id")?, "slug": metadata(row, "service_slug")?},
        )
        .session(&mut *session)
        .await?;
    let Some(svc) = svc else {
        return Ok(Delivery::Remove);
    };
    if !svc.is_active {
        return Ok(Delivery::Parked(SERVICE_DISABLED));
    }
    if svc.service_type == "ssh" {
        return Ok(Delivery::Retired);
    }
    if svc.credential_binding.as_deref() == Some("platform") {
        return Ok(Delivery::Remove);
    }
    let linked_key = if let Some(key_id) = &svc.api_key_id {
        db.collection::<UserApiKey>("user_api_keys")
            .find_one(doc! {"_id": key_id, "user_id": &svc.user_id})
            .session(&mut *session)
            .await?
    } else {
        None
    };
    // Local-only credentials are outside this protocol, including services whose
    // server metadata says no-auth. Never turn their marker into an erase frame.
    if linked_key
        .as_ref()
        .is_some_and(|key| key.credential_type == "node_managed")
    {
        return Ok(Delivery::Retired);
    }
    let key = if svc.auth_method == "none" {
        None
    } else {
        let Some(key) = linked_key else {
            return Ok(Delivery::Remove);
        };
        if key.status != "active" {
            return Ok(Delivery::Parked(KEY_INACTIVE));
        }
        Some(key)
    };
    let target = db
        .collection::<UserEndpoint>("user_endpoints")
        .find_one(doc! {"_id": &svc.endpoint_id})
        .session(&mut *session)
        .await?
        .map(|ep| ep.url)
        .filter(|url| !url.is_empty());
    Ok(Delivery::Update(Box::new(Material {
        service: svc,
        key,
        target,
        version: identity(row)?,
    })))
}

/// Delivers one current slot under an exact token. Claim expiry permits recovery
/// after process loss; version comparison at the node handles a delayed sender.
pub async fn deliver(
    db: &Database,
    keys: &EncryptionKeys,
    manager: &Arc<NodeWsManager>,
    id: &str,
) -> AppResult<()> {
    let claim = uuid::Uuid::new_v4().to_string();
    let context = (db.clone(), id.to_owned(), claim.clone());
    let snapshot = super::service_history::transaction::run_send(db, &context, |session, context| Box::pin(async move {
        let (db, id, claim) = context;
        let row = db.collection::<Document>(COLLECTION).find_one_and_update(doc! {
            "_id": id, "pending": true,
            "$expr": {"$and": [
                {"$lte": ["$retry_at", "$$NOW"]},
                {"$lte": [{"$ifNull": ["$claim_until", bson::DateTime::from_millis(0)]}, "$$NOW"]},
                {"$lte": [{"$ifNull": ["$preparing_until", bson::DateTime::from_millis(0)]}, "$$NOW"]}
            ]}
        }, vec![doc! {"$set": {
            "claim_token": &claim, "claim_until": {"$add": ["$$NOW", CLAIM_SECS * 1000]},
            // An expired pre-commit push may have reached the node. Order its
            // recovery after that speculative frame, using committed state.
            "generation": {"$add": ["$generation", {"$cond": [{"$ne": [{"$ifNull": ["$preparing_token", bson::Bson::Null]}, bson::Bson::Null]}, 1_i64, 0_i64]}]},
            "preparing_token": bson::Bson::Null,
        }}]).return_document(ReturnDocument::After).session(&mut **session).await?;
        let Some(row) = row else { return Ok(None); };
        let params = super::api_key_mutation_service::transaction_result(current_material(db, session, &row).await)?;
        Ok(Some((row, params)))
    })).await?;
    let Some((row, params)) = snapshot else {
        return Ok(());
    };
    let version = identity(&row)?;
    let node = metadata(&row, "node_id")?;
    let slug = metadata(&row, "service_slug")?;
    let mut offline = false;
    let result = async {
        match params {
            Delivery::Retired => return Ok(Completion::Retired),
            Delivery::Parked(reason) => return Ok(Completion::Parked(reason)),
            _ => {}
        }
        if let Ok(actor) = row.get_str("actor_user_id") {
            super::node_service::ensure_node_writable_by_actor(db, actor, node).await?;
        }
        let info = manager.cluster_session_info(node).await;
        if !info.is_connected {
            offline = true;
            return Err(AppError::NodeOffline(
                "Credential destination is offline".into(),
            ));
        }
        if let Delivery::Update(material) = params {
            let credential = if let Some(key) = &material.key {
                credential_push_service::decrypt_api_key_credential(key, keys)
                    .await
                    .ok_or_else(|| {
                        AppError::NodeCredentialMissing("Delivery material unavailable".into())
                    })?
            } else {
                zeroize::Zeroizing::new(String::new())
            };
            let mut params = credential_push_service::build_credential_params(
                &material.service,
                &credential,
                material.target,
            );
            params.version = Some(material.version);
            if info.capabilities.credential_ack_correlation {
                manager
                    .send_credential_update_and_wait_clustered(node, &params, SEND_TIMEOUT)
                    .await?;
                Ok(Completion::Acknowledged)
            } else {
                manager
                    .send_credential_update_clustered(node, &params)
                    .await?;
                Ok(Completion::SentUnconfirmed)
            }
        } else if info.capabilities.credential_ack_correlation {
            manager
                .send_credential_remove_versioned_clustered(node, slug, &version, SEND_TIMEOUT)
                .await?;
            Ok(Completion::Acknowledged)
        } else {
            Ok(Completion::Parked(ACKNOWLEDGEMENT_UNSUPPORTED))
        }
    }
    .await;
    let acknowledged = matches!(result, Ok(Completion::Acknowledged));
    let unconfirmed = matches!(result, Ok(Completion::SentUnconfirmed));
    let retired = matches!(result, Ok(Completion::Retired));
    let reason = match &result {
        Ok(Completion::SentUnconfirmed) => Some(SENT_UNCONFIRMED),
        Ok(Completion::Parked(reason)) => Some(*reason),
        _ => None,
    };
    let prior_attempts = row.get_i64("retry_attempts").unwrap_or(0).clamp(0, 31);
    let delay = if result.is_err() {
        retry_delay(prior_attempts, offline)
    } else {
        0
    };
    let attempts = if result.is_err() {
        (prior_attempts + 1).min(31)
    } else {
        0
    };
    // A newer source write or reconnect may have re-armed this slot while we
    // sent. Release our claim, but never overwrite that newer scheduling state.
    let current = doc! {"$and": [
        {"$eq": ["$generation", version.credential_generation]},
        {"$eq": [{"$ifNull": ["$retry_revision", 0_i64]}, row.get_i64("retry_revision").unwrap_or(0)]},
    ]};
    db.collection::<Document>(COLLECTION).update_one(doc! {"_id": id, "claim_token": &claim}, vec![doc! {"$set": {
        "acknowledged_generation": {"$max": [{"$ifNull": ["$acknowledged_generation", 0_i64]}, if acknowledged { version.credential_generation } else { 0_i64 }]},
        "sent_unconfirmed_generation": {"$max": [{"$ifNull": ["$sent_unconfirmed_generation", 0_i64]}, if unconfirmed { version.credential_generation } else { 0_i64 }]},
        "pending": {"$cond": [current.clone(), result.is_err(), "$pending"]},
        "parked_reason": {"$cond": [current.clone(), reason, "$parked_reason"]},
        "suppressed": {"$cond": [{"$and": [current.clone(), retired]}, true, "$suppressed"]},
        "retry_attempts": {"$cond": [current.clone(), attempts, "$retry_attempts"]},
        "retry_at": {"$cond": [current, {"$add": ["$$NOW", delay * 1000]}, "$retry_at"]},
        "claim_token": bson::Bson::Null, "claim_until": bson::DateTime::from_millis(0),
    }}]).await?;
    result.map(|_| ())
}

enum Completion {
    Acknowledged,
    SentUnconfirmed,
    Parked(&'static str),
    Retired,
}

fn retry_delay(prior_attempts: i64, offline: bool) -> i64 {
    let (base, maximum) = if offline {
        (OFFLINE_RETRY_BASE_SECS, OFFLINE_RETRY_MAX_SECS)
    } else {
        (TRANSIENT_RETRY_BASE_SECS, TRANSIENT_RETRY_MAX_SECS)
    };
    base.saturating_mul(1_i64 << prior_attempts.clamp(0, 31))
        .min(maximum)
}

pub async fn deliver_slot(
    db: &Database,
    keys: &EncryptionKeys,
    manager: &Arc<NodeWsManager>,
    node: &str,
    slug: &str,
) -> AppResult<()> {
    if let Some(row) = db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"node_id": node, "service_slug": slug})
        .await?
    {
        let version = identity(&row)?;
        deliver(db, keys, manager, &version.credential_id).await?;
    }
    Ok(())
}

pub async fn retry_node(
    db: &Database,
    keys: &EncryptionKeys,
    manager: &Arc<NodeWsManager>,
    node: &str,
) -> AppResult<()> {
    // Source-disabled work stays parked until its source write. Reconnect only
    // wakes transient retries and outcomes that require a new connection/capability.
    db.collection::<Document>(COLLECTION).update_many(doc! {
        "node_id": node, "suppressed": {"$ne": true},
        "$or": [{"pending": true}, {"parked_reason": {"$in": [SENT_UNCONFIRMED, ACKNOWLEDGEMENT_UNSUPPORTED]}}],
    }, vec![doc! {"$set": {
        "pending": true, "parked_reason": bson::Bson::Null, "retry_at": "$$NOW", "retry_attempts": 0_i64,
        "retry_revision": {"$add": [{"$ifNull": ["$retry_revision", 0_i64]}, 1_i64]},
    }}]).await?;
    let rows: Vec<NodeCredentialDesired> = db
        .collection(COLLECTION)
        .find(doc! {"node_id": node, "pending": true})
        .limit(PASS_LIMIT)
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let _ = deliver(db, keys, manager, &row.id).await;
    }
    Ok(())
}

pub async fn run_pass(
    db: &Database,
    keys: &EncryptionKeys,
    manager: &Arc<NodeWsManager>,
    runtime: &ClusterLeaseRuntime,
    shutdown: &super::shutdown::Runtime,
) -> AppResult<()> {
    let Some(_work) = shutdown.background() else {
        return Ok(());
    };
    // This bounded query uses (pending, retry_at). An idle replica performs only
    // this read, and never writes a lease. The claim rechecks due time because
    // another pass can finish between this snapshot and our lease acquisition.
    let rows: Vec<NodeCredentialDesired> = db
        .collection(COLLECTION)
        .find(doc! {"pending": true, "retry_at": {"$lte": bson::DateTime::now()}})
        .sort(doc! {"retry_at": 1})
        .limit(PASS_LIMIT)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let Some(token) = runtime
        .acquire_background(db, RETRY_LEASE, shutdown)
        .await?
    else {
        return Ok(());
    };
    let pass = async {
        for row in rows {
            if shutdown.is_draining() {
                break;
            }
            let _ = deliver(db, keys, manager, &row.id).await;
        }
    };
    runtime.run_while_renewed(db, &token, pass).await;
    // The operation is dropped on lease loss. Outstanding slots retain their
    // exact claims until expiry; late frames remain fenced by node generations.
    LeaseStore::release(db, &token).await?;
    Ok(())
}

pub async fn run_worker(state: crate::AppState) {
    let shutdown = state.shutdown.clone();
    let mut interval = tokio::time::interval(PASS_INTERVAL);
    while let Some(_tick) = shutdown.tick(&mut interval).await {
        if run_pass(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            cluster_lease_runtime(),
            &shutdown,
        )
        .await
        .is_err()
        {
            tracing::warn!("Node credential delivery pass failed; durable intent retained");
        }
    }
}

/// A strict push precedes routing/auth writes. Keep retry away from that
/// preparation; process loss leaves a bounded marker that orders recovery
/// after the speculative frame. No speculative material is stored here.
pub struct Preparation {
    db: Database,
    id: String,
    token: String,
    generation: i64,
}

impl std::fmt::Debug for Preparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Preparation([REDACTED])")
    }
}

impl Preparation {
    pub async fn finish(self) -> AppResult<()> {
        self.db.collection::<Document>(COLLECTION).update_one(doc! {"_id": self.id, "preparing_token": self.token}, vec![doc! {"$set": {
            "generation": {"$add": ["$generation", 1_i64]}, "preparing_token": bson::Bson::Null,
            "preparing_until": bson::DateTime::from_millis(0),
            "parked_reason": bson::Bson::Null, "retry_attempts": 0_i64,
            "pending": {"$cond": [{"$eq": ["$generation", self.generation]}, true, "$pending"]}, "retry_at": "$$NOW",
        }}]).await?;
        Ok(())
    }
}

pub async fn prepare_update(
    db: &Database,
    keys: &EncryptionKeys,
    owner: &str,
    key_id: Option<&str>,
    target: &credential_push_service::StrictPushTarget<'_>,
) -> AppResult<(Preparation, CredentialUpdateParams)> {
    let token = uuid::Uuid::new_v4().to_string();
    let context = (
        db.clone(),
        owner.to_owned(),
        key_id.map(str::to_owned),
        target.target_node_id.to_owned(),
        target.service_slug.to_owned(),
        target.service_id.to_owned(),
        token.clone(),
    );
    let (id, generation, key) = super::service_history::transaction::run_send(db, &context, |session, context| Box::pin(async move {
        let (db, owner, key_id, node, slug, service, token) = context;
        let key = if let Some(key_id) = key_id {
            let key = db.collection::<UserApiKey>("user_api_keys").find_one(doc! {"_id": key_id, "user_id": owner})
                .session(&mut **session).await?
                .ok_or_else(|| mongodb::error::Error::custom("Credential disappeared before delivery"))?;
            Some(key)
        } else { None };
        advance(db, session, node, slug, service, owner, None).await?;
        let row = db.collection::<Document>(COLLECTION).find_one_and_update(doc! {"node_id": node, "service_slug": slug}, vec![doc! {"$set": {
            "preparing_token": &token, "preparing_until": {"$add": ["$$NOW", PREPARATION_SECS * 1000]},
        }}]).return_document(ReturnDocument::After).session(&mut **session).await?
            .ok_or_else(|| mongodb::error::Error::custom("Delivery preparation disappeared"))?;
        let version = super::api_key_mutation_service::transaction_result(identity(&row))?;
        let id = version.credential_id.clone();
        let generation = version.credential_generation;
        Ok((id, generation, key))
    })).await?;
    let material = if let Some(key) = key {
        credential_push_service::decrypt_api_key_credential(&key, keys)
            .await
            .ok_or_else(|| {
                AppError::NodeCredentialMissing("Credential delivery material unavailable".into())
            })?
    } else {
        zeroize::Zeroizing::new(String::new())
    };
    let mut params = credential_push_service::build_credential_params_from_fields(
        target.service_slug,
        target.auth_method,
        target.auth_key_name,
        &material,
        target.target_url.map(str::to_owned),
    );
    params.version = Some(CredentialVersion {
        credential_id: id.clone(),
        credential_generation: generation,
    });
    Ok((
        Preparation {
            db: db.clone(),
            id,
            token,
            generation,
        },
        params,
    ))
}
