//! Explicit adoption and append-only external evidence. Every accepted mutation
//! writes the same singleton and appends its evidence and chained audit atomically.
mod contract;
mod inventory;
mod load_gate;
mod location;
mod proof;
pub(crate) mod retirement;
pub(crate) mod rotation;
use super::{
    audit_service::{self, AuditActor},
    hmac_lifecycle_service, transaction_service,
};
use crate::{
    config::AppConfig,
    errors::{AppError, AppResult},
};
use bson::{Document, doc};
pub(crate) use contract::*;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    options::{IndexOptions, ReturnDocument},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use sha2::{Digest, Sha256};

const HEAD: &str = "encryption_key_lifecycle";
const ENTRIES: &str = "encryption_key_evidence";
const DEPENDENCIES: &str = "encryption_key_dependencies";
const SINGLETON: &str = "ring-v1";

fn invalid() -> AppError {
    AppError::ValidationError("Invalid key lifecycle evidence".into())
}
fn conflict(message: &'static str) -> AppError {
    AppError::Conflict(message.into())
}
fn stale() -> AppError {
    conflict("Key lifecycle revision or scope changed")
}
fn corrupt() -> AppError {
    AppError::Internal("Key lifecycle state unavailable".into())
}
fn encode<T: Serialize>(value: &T) -> AppResult<bson::Bson> {
    bson::to_bson(value).map_err(|_| corrupt())
}
fn field<T: DeserializeOwned>(row: &Document, name: &str) -> AppResult<T> {
    bson::from_bson(row.get(name).cloned().ok_or_else(corrupt)?).map_err(|_| corrupt())
}
fn fresh() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn fingerprint<T: Serialize>(actor: &AuditActor, domain: &str, input: &T) -> AppResult<String> {
    let bytes = serde_json::to_vec(&(1, domain, &actor.user_id, input)).map_err(|_| invalid())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn indexes(db: &Database) -> AppResult<()> {
    if let Err(error) = db.create_collection(HEAD).await
        && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 48)
    {
        return Err(error.into());
    }
    for (collection, key) in [
        (db.collection::<Document>(ENTRIES), "receipt.revision"),
        (db.collection::<Document>(DEPENDENCIES), "opened.revision"),
    ] {
        collection
            .create_index(
                IndexModel::builder()
                    .keys(doc! {key:1})
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
    }
    let deps = db.collection::<Document>(DEPENDENCIES);
    for key in ["parents", "inventory"] {
        deps.create_index(IndexModel::builder().keys(doc! {key:1}).build())
            .await?;
    }
    Ok(())
}

fn repeat(row: Document, fingerprint: &str) -> AppResult<Receipt> {
    if row.get_str("request_digest") != Ok(fingerprint) {
        return Err(conflict("Lifecycle request ID is already in use"));
    }
    field(&row, "receipt")
}

pub(crate) async fn status(db: &Database, config: &AppConfig) -> AppResult<Option<Status>> {
    let db = transaction_service::durable_database(db);
    let Some(row) = location::observed(&db).await? else {
        return Ok(None);
    };
    let keys: Vec<Key> = field(&row, "keys")?;
    Ok(Some(Status {
        revision: field(&row, "revision")?,
        scope: field(&row, "scope")?,
        rotation_open: keys.len() > 1
            || (row.contains_key("published_retirement") && !rotation::closed(&db, &row).await?),
        rolling: row.get_bool("rolling") == Ok(true),
        restore_required: field(&row, "restore_required")?,
        scope_revision: field(&row, "scope_revision")?,
        keys,
        current: field(&row, "current")?,
        independent_hmac_file_verified: hmac_lifecycle_service::independent_file_matches(
            &db,
            config.hmac_keyset_path.as_deref(),
        )
        .await?,
    }))
}

/// Admit a process before database initialization. This path only reads the
/// lifecycle singleton, its proof and database identities, so it is safe to
/// run with the bootstrap database identity before any indexes, migrations or
/// application writes.
/// An unenrolled installation remains observational until an administrator
/// enrolls it. Once enrolled, every process must prove the exact persisted
/// ring and contextual writer policy.
pub(crate) async fn admit_startup(
    db: &Database,
    config: &AppConfig,
) -> AppResult<Option<retirement::Admission>> {
    let application = transaction_service::durable_database(db);
    let Some(enrollment) = application
        .collection::<Document>(HEAD)
        .find_one(doc! {"_id": SINGLETON})
        .projection(doc! {"revision": 1})
        .await?
    else {
        return Ok(None);
    };
    let url = std::env::var("NYXID_BOOTSTRAP_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            conflict("Enrolled lifecycle requires a read-only bootstrap database identity")
        })?;
    let bootstrap =
        crate::db::create_connection_with_url(&url, config.database_max_connections).await?;
    let bootstrap = transaction_service::durable_database(&bootstrap);
    let expected = std::env::var("NYXID_DEPLOYMENT_REVISION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| conflict("Enrolled lifecycle requires a deployment revision"))?;
    let revision = field(&enrollment, "revision")?;
    Ok(Some(
        admit_enrolled(&application, &bootstrap, config, revision, &expected).await?,
    ))
}

async fn admit_enrolled(
    application: &Database,
    bootstrap: &Database,
    config: &AppConfig,
    revision: i64,
    expected: &str,
) -> AppResult<retirement::Admission> {
    let location = location::identity(application).await?;
    if location != location::identity(bootstrap).await? {
        return Err(conflict(
            "Bootstrap identity points to a different database",
        ));
    }
    let row = bootstrap
        .collection::<Document>(HEAD)
        .find_one(doc! {"_id": SINGLETON, "revision": revision})
        .projection(doc! {"scope": 1, "location": 1, "restore_required": 1, "proofs": 1, "published_retirement":1, "survivor_proofs":1, "revision":1, "retirement_generation":1,"managed_proofs":1,"rolling":1,"scope_revision":1,"survivor_lineage":1})
        .await?
        .ok_or_else(stale)?;
    if row.get_bool("restore_required") != Ok(false) {
        return Err(conflict(
            "Key lifecycle admission is blocked by a restore signal",
        ));
    }
    if field::<Document>(&row, "location")? != location {
        return Err(conflict(
            "Key lifecycle admission found a different database identity",
        ));
    }
    let scope: Scope = field(&row, "scope")?;
    if scope.deployment_revision != expected {
        return Err(conflict(
            "Deployment revision is outside the enrolled lifecycle scope",
        ));
    }
    if !config.contextual_record_writes {
        return Err(conflict(
            "Enrolled lifecycle requires contextual record writers",
        ));
    }
    let (proofs, pinned) = if row.contains_key("published_retirement") {
        retirement::admit(bootstrap, config, &row).await?;
        (field(&row, "survivor_proofs")?, true)
    } else {
        let slots = proof::slots(config).await?;
        let pinned = slots.iter().all(|slot| slot.pinned(config));
        let proofs = rotation::admitted(&row, &scope, &slots).await?;
        if row.contains_key("managed_proofs") && (!pinned || config.allow_legacy_record_reads) {
            return Err(stale());
        }
        if row.get_bool("survivor_lineage") == Ok(true) {
            retirement::survivor_dependencies(bootstrap, config).await?;
        }
        (proofs, pinned)
    };
    let primary = proofs
        .iter()
        .find(|p| p.key.role == "primary")
        .ok_or_else(stale)?;
    // A slow provider challenge can straddle publication or restore. Linearize
    // admission after proof, while the version that supplied the proof is current.
    bootstrap
        .collection::<Document>(HEAD)
        .find_one(doc! {"_id":SINGLETON,"revision":revision,"restore_required":false})
        .projection(doc! {"_id":1})
        .await?
        .ok_or_else(stale)?;
    let mut layout = doc! {"_id":SINGLETON,"scope":row.get("scope"),"location":row.get("location"),"proofs":row.get("proofs"),"scope_revision":row.get("scope_revision"),"restore_required":false};
    if let Some(publication) = row.get("published_retirement") {
        layout.insert("published_retirement", publication);
    } else {
        layout.insert("published_retirement", doc! {"$exists":false});
    }
    Ok(retirement::Admission {
        layout,
        cohort: if pinned && !config.allow_legacy_record_reads {
            Some(retirement::cohort(
                &scope,
                &primary.key.key_ref,
                load_gate::executable().await?,
            ))
        } else {
            None
        },
    })
}

/// Exceptional writers call this inside their existing transaction before
/// changing a source that can invalidate a retirement proof.
pub(crate) async fn invalidate_in_transaction(
    db: &Database,
    session: &mut ClientSession,
    reason: &'static str,
) -> AppResult<Option<i64>> {
    let head = db.collection::<Document>(HEAD);
    let Some(row) = head
        .find_one_and_update(
            doc! {"_id": SINGLETON, "restore_required": false, "published_retirement":{"$exists":false},"survivor_lineage":{"$ne":true}},
            doc! {"$inc": {"revision": 1_i64}, "$set": {"restore_required": true, "current": {}}},
        )
        .projection(doc! {"revision": 1})
        .session(&mut *session)
        .await?
    else {
        if head.find_one(doc! {"_id": SINGLETON, "$or":[{"published_retirement":{"$exists":true}},{"survivor_lineage":true}]}).projection(doc!{"_id":1}).session(&mut *session).await?.is_some() {
            return Err(conflict("Published survivor requires isolated recovery"));
        }
        return Ok(None);
    };
    let key = audit_service::audit_chain_hmac_key().ok_or_else(corrupt)?;
    audit_service::log_system_event_in_session(
        db,
        session,
        key,
        "encryption_key_lifecycle.writer_invalidated",
        json!({"revision": field::<i64>(&row, "revision")?, "reason": reason}),
    )
    .await?;
    Ok(Some(field(&row, "revision")?))
}

pub(crate) async fn enroll(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    request: &Enroll,
) -> AppResult<Receipt> {
    request.recording.validate()?;
    contract::label(&request.deployment_revision)?;
    if request.recording.expected_revision != 0 {
        return Err(stale());
    }
    let db = transaction_service::durable_database(db);
    let digest = fingerprint(actor, "enroll", request)?;
    if let Some(head) = location::observed(&db).await? {
        if head.get_bool("restore_required") != Ok(false) {
            return Err(stale());
        }
        if let Some(row) = db
            .collection::<Document>(ENTRIES)
            .find_one(doc! {"_id":&request.recording.request_id})
            .await?
        {
            return repeat(row, &digest);
        }
        return Err(stale());
    }
    let id = fresh();
    let slots = proof::slots(config).await?;
    let proofs = proof::issue(&id, &slots).await?;
    let keys: Vec<_> = proofs.iter().map(|p| p.key.clone()).collect();
    let scope = Scope {
        deployment_id: id,
        deployment_revision: request.deployment_revision.clone(),
        restore_epoch: fresh(),
        keys: keys.iter().map(|k| k.key_ref.clone()).collect(),
    };
    indexes(&db).await?;
    let location = location::identity(&db).await?;
    let new_head = doc! {
        "_id": SINGLETON,
        "schema_version": 1,
        "revision": 0_i64,
        "scope_revision": 1_i64,
        "scope": encode(&scope)?,
        "keys": encode(&keys)?,
        "proofs": encode(&proofs)?,
        "location": location,
        "restore_required": false,
        "current": {},
    };
    let mutation = Mutation {
        db: &db,
        actor: Some(actor),
        recording: &request.recording,
        scope,
        digest,
        action: Action::Enroll(new_head),
    };
    record(&mutation).await
}

pub(crate) async fn append(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    request: &Append,
) -> AppResult<Receipt> {
    request.recording.validate()?;
    if request.scope.keys.is_empty() || request.scope.keys.len() > 4 {
        return Err(invalid());
    }
    let digest = fingerprint(actor, "append", request)?;
    let db = transaction_service::durable_database(db);
    let observed = location::observed(&db).await?.ok_or_else(stale)?;
    if observed.get_bool("restore_required") != Ok(false)
        && !matches!(request.change, Change::AdvanceRestoreEpoch)
    {
        return Err(conflict("A detected restore requires a new epoch"));
    }
    if matches!(request.change, Change::VerifyRing) {
        if let Some(row) = db
            .collection::<Document>(ENTRIES)
            .find_one(doc! {"_id":&request.recording.request_id})
            .await?
        {
            return repeat(row, &digest);
        }
        let head = observed;
        if head.get_i64("revision") != Ok(request.recording.expected_revision) {
            return Err(stale());
        }
        let scope: Scope = field(&head, "scope")?;
        if scope != request.scope {
            return Err(stale());
        }
        active_proofs(&db, config, &head, &scope).await?;
    }
    let location = if matches!(request.change, Change::AdvanceRestoreEpoch) {
        indexes(&db).await?;
        Some(location::identity(&db).await?)
    } else {
        None
    };
    record(&Mutation {
        db: &db,
        actor: Some(actor),
        recording: &request.recording,
        scope: request.scope.clone(),
        digest,
        action: Action::Change(&request.change, location),
    })
    .await
}

enum Action<'a> {
    Enroll(Document),
    Change(&'a Change, Option<Document>),
    Detected,
    Rotation(&'a rotation::Edit<'a>),
}
struct Mutation<'a> {
    db: &'a Database,
    actor: Option<&'a AuditActor>,
    recording: &'a Recording,
    scope: Scope,
    digest: String,
    action: Action<'a>,
}

async fn record(m: &Mutation<'_>) -> AppResult<Receipt> {
    let result = transaction_service::run_db_only(m.db, m, |tx, m| Box::pin(commit(m, tx))).await;
    if result.is_err()
        && let Some(row) =
            m.db.collection::<Document>(ENTRIES)
                .find_one(doc! {"_id":&m.recording.request_id})
                .await?
    {
        return repeat(row, &m.digest);
    }
    result
}

async fn commit(m: &Mutation<'_>, tx: &mut ClientSession) -> AppResult<Receipt> {
    let entries = m.db.collection::<Document>(ENTRIES);
    if let Some(row) = entries
        .find_one(doc! {"_id":&m.recording.request_id})
        .session(&mut *tx)
        .await?
    {
        return repeat(row, &m.digest);
    }
    let head = m.db.collection::<Document>(HEAD);
    if let Action::Enroll(row) = &m.action {
        if head
            .find_one(doc! {"_id":SINGLETON})
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Err(stale());
        }
        head.insert_one(row).session(&mut *tx).await?;
    }
    let row = head
        .find_one_and_update(
            doc! {"_id":SINGLETON,"schema_version":1,"revision":m.recording.expected_revision},
            doc! {"$inc":{"revision":1_i64},"$currentDate":{"recorded_at":true}},
        )
        .projection(doc! {"proofs":0})
        .return_document(ReturnDocument::After)
        .session(&mut *tx)
        .await?
        .ok_or_else(stale)?;
    let ring: Scope = field(&row, "scope")?;
    if m.scope.deployment_id != ring.deployment_id
        || m.scope.deployment_revision != ring.deployment_revision
        || m.scope.restore_epoch != ring.restore_epoch
        || !m.scope.keys.is_subset(&ring.keys)
    {
        return Err(stale());
    }
    let mut receipt = Receipt {
        entry_id: m.recording.request_id.clone(),
        revision: field(&row, "revision")?,
        scope: m.scope.clone(),
        recorded_at_ms: row
            .get_datetime("recorded_at")
            .map_err(|_| corrupt())?
            .timestamp_millis(),
    };
    let assertion = match &m.action {
        Action::Enroll(_) => json!({"kind":"enroll"}),
        Action::Rotation(edit) => {
            edit.apply(m.db, tx, &mut receipt).await?;
            serde_json::to_value(edit.operation).map_err(|_| invalid())?
        }
        Action::Detected => {
            head.update_one(
                doc! {"_id": SINGLETON},
                doc! {"$set": {"restore_required": true, "scope_revision": receipt.revision, "current": {}}},
            ).session(&mut *tx).await?;
            json!({"kind":"restore_detected"})
        }
        Action::Change(change, location) => {
            if let Some(identity) = location {
                head.update_one(
                    doc! {"_id":SINGLETON},
                    doc! {"$set":{"location":identity,"restore_required":false}},
                )
                .session(&mut *tx)
                .await?;
            } else if row.get_bool("restore_required") != Ok(false) {
                return Err(stale());
            }
            inventory::apply(m.db, tx, &row, &ring, &mut receipt, m.recording, change).await?;
            serde_json::to_value(change).map_err(|_| invalid())?
        }
    };
    let evidence = Evidence {
        schema_version: 1,
        receipt: receipt.clone(),
        previous_revision: m.recording.expected_revision,
        owner: m.recording.owner.clone(),
        evidence_ref: m.recording.evidence_ref.clone(),
        issuer: m
            .actor
            .map_or_else(|| "nyxid".into(), |actor| actor.user_id.clone()),
        producer: if m.actor.is_some() { "admin" } else { "system" }.into(),
        input_scope: m.scope.clone(),
        assertion,
    };
    let mut entry = bson::to_document(&evidence).map_err(|_| corrupt())?;
    entry.insert("_id", &receipt.entry_id);
    entry.insert("request_digest", &m.digest);
    if let Action::Rotation(edit) = &m.action
        && let Some(proof) = &edit.closed_proof
    {
        entry.insert("closed_proof", proof);
    }

    entries.insert_one(entry).session(&mut *tx).await?;
    let metadata = json!({
        "entry_id": receipt.entry_id,
        "revision": receipt.revision,
        "kind": evidence.assertion["kind"],
        "evidence_ref": evidence.evidence_ref,
        "restore_epoch": receipt.scope.restore_epoch,
    });
    let event = "encryption_key_lifecycle.recorded";
    let audit_key = audit_service::audit_chain_hmac_key().ok_or_else(corrupt)?;
    if let Some(actor) = m.actor {
        audit_service::log_actor_event_in_session(m.db, tx, audit_key, actor, event, metadata)
            .await?;
    } else {
        audit_service::log_system_event_in_session(m.db, tx, audit_key, event, metadata).await?;
    }
    Ok(receipt)
}

async fn page<T: DeserializeOwned>(
    collection: mongodb::Collection<Document>,
    revision: &str,
    page: Page,
) -> AppResult<Vec<T>> {
    let after = page.after.unwrap_or(0);
    let limit = page.limit.unwrap_or(50);
    if after < 0 || !(1..=100).contains(&limit) {
        return Err(invalid());
    }
    let mut rows = collection
        .find(doc! {revision:{"$gt":after}})
        .sort(doc! {revision:1})
        .limit(limit)
        .await?;
    let mut result = Vec::new();
    while let Some(row) = rows.try_next().await? {
        result.push(bson::from_document(row).map_err(|_| corrupt())?)
    }
    Ok(result)
}
pub(crate) async fn evidence(db: &Database, query: Page) -> AppResult<Vec<Evidence>> {
    page(
        transaction_service::durable_database(db).collection(ENTRIES),
        "receipt.revision",
        query,
    )
    .await
}
pub(crate) async fn dependencies(db: &Database, query: Page) -> AppResult<Vec<Dependency>> {
    page(
        transaction_service::durable_database(db).collection(DEPENDENCIES),
        "opened.revision",
        query,
    )
    .await
}

#[cfg(test)]
mod tests;

/// Revision of the enrolled scope, independent of ordinary evidence edits.
/// An unenrolled database uses zero so PR 7b journals remain readable.
pub(super) async fn census_scope_revision(
    db: &Database,
    deployment_revision: &str,
    epoch: &str,
) -> AppResult<Option<i64>> {
    let Some(row) = location::observed(db).await? else {
        return Ok(Some(0));
    };
    let scope: Scope = field(&row, "scope")?;
    if row.get_bool("restore_required") == Ok(false)
        && scope.deployment_revision == deployment_revision
        && scope.restore_epoch == epoch
    {
        return Ok(Some(field(&row, "scope_revision")?));
    }
    Ok(None)
}

/// An unenrolled observational generation uses private, generation-local
/// references. Enrolled reports carry the same logical references as the ring.
#[derive(Serialize, serde::Deserialize)]
pub(crate) struct CensusRing {
    pub deployment: String,
    pub revision: i64,
    pub deployment_revision: String,
    pub restore_epoch: String,
    pub proofs: Vec<proof::Proof>,
}

async fn active_proofs(
    db: &Database,
    config: &AppConfig,
    head: &Document,
    scope: &Scope,
) -> AppResult<Vec<proof::Proof>> {
    if head.contains_key("published_retirement") {
        retirement::admit(db, config, head).await?;
        field(head, "survivor_proofs")
    } else {
        let slots = proof::slots(config).await?;
        rotation::admitted(head, scope, &slots).await
    }
}

pub(crate) async fn census_ring(db: &Database, config: &AppConfig) -> AppResult<CensusRing> {
    if let Some(row) = location::observed(db).await? {
        if field::<bool>(&row, "restore_required")? {
            return Err(stale());
        }
        let scope: Scope = field(&row, "scope")?;
        let proofs = active_proofs(db, config, &row, &scope).await?;
        Ok(CensusRing {
            deployment: scope.deployment_id,
            revision: field(&row, "scope_revision")?,
            deployment_revision: scope.deployment_revision,
            restore_epoch: scope.restore_epoch,
            proofs,
        })
    } else {
        let deployment = fresh();
        let proofs = proof::issue(&deployment, &proof::slots(config).await?).await?;
        Ok(CensusRing {
            deployment,
            revision: 0,
            deployment_revision: "unenrolled".into(),
            restore_epoch: "unenrolled".into(),
            proofs,
        })
    }
}

impl CensusRing {
    pub(crate) async fn verify(&self, config: &AppConfig) -> AppResult<()> {
        proof::verify(&self.deployment, &proof::slots(config).await?, &self.proofs).await
    }
}

#[cfg(test)]
mod rollout_peer;
