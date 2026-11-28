//! An authorization binds the enrolled B/A layout to its sole surviving primary.
//! Each step writes the singleton and the next watched marker in one transaction.
use super::*;
use crate::{
    crypto::aes::EncryptionKeys,
    services::{
        encrypted_record_census::{Cut, report::retirement as census},
        security_readiness_service as fleet,
    },
};
use serde::Deserialize;
use std::collections::BTreeSet;

mod guards;
mod initialization;
pub(crate) use initialization::{Admission, Initialization};
pub(crate) const CAPABILITY: &str = "encryption:retirement-v1";
pub(crate) const LOAD_BLOCKER: &str = "isolated_load_gate_unmet";

pub(super) fn cohort(scope: &Scope) -> String {
    // Scope contains only opaque identifiers and bounded release metadata.
    hex::encode(Sha256::digest(
        bson::to_vec(scope).expect("serializable enrollment scope"),
    ))
}

// PR 10 verifies isolated-rig evidence before a release can issue this capability.
struct CertifiedRelease;
fn certified_release() -> AppResult<CertifiedRelease> {
    Err(conflict(LOAD_BLOCKER))
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub generation: String,
    pub retiring: String,
    pub survivor: String,
}
impl Target {
    fn validate(&self) -> AppResult<()> {
        for id in [&self.generation, &self.retiring, &self.survivor] {
            contract::uuid(id)?;
        }
        if self.retiring == self.survivor {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub recording: Recording,
    pub target: Target,
    pub report_revision: i64,
    pub expected_step_revision: i64,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Decision,
    Approved,
    Consume,
    Consumed,
    Published,
}

#[derive(Serialize, Deserialize)]
pub(in crate::services) struct Transition {
    phase: Phase,
    target: Target,
    scope: Scope,
    location: Document,
    revision: i64,
    report_revision: i64,
    registry_digest: String,
    manifest: String,
    request_digest: String,
    marker: String,
    witnessed: Cut,
    valid_until_ms: i64,
    evidence_ref: String,
}

#[derive(Serialize)]
pub(crate) struct Evaluation {
    revision: Option<i64>,
    report_revision: Option<i64>,
    as_of: Option<bson::Timestamp>,
    certification_enabled: bool,
    blockers: BTreeSet<&'static str>,
}

#[derive(Serialize)]
pub(crate) struct Progress {
    phase: Phase,
    revision: i64,
    as_of: bson::Timestamp,
}

impl Transition {
    fn survivor_scope(&self) -> Scope {
        Scope {
            keys: [self.target.survivor.clone()].into(),
            ..self.scope.clone()
        }
    }

    fn progress(&self) -> Progress {
        Progress {
            phase: self.phase,
            revision: self.revision,
            as_of: self.witnessed.time,
        }
    }
}

pub(in crate::services) async fn revision_at(
    db: &Database,
    time: bson::Timestamp,
) -> AppResult<i64> {
    let row = db
        .run_cursor_command(
            doc! {"find":HEAD,"filter":{"_id":SINGLETON},"projection":{"revision":1},
            "readConcern":{"level":"snapshot","atClusterTime":time}},
        )
        .await?
        .try_next()
        .await?
        .ok_or_else(stale)?;
    field(&row, "revision")
}

async fn reader(config: &AppConfig, head: &Document, target: &Target) -> AppResult<EncryptionKeys> {
    let scope: Scope = field(head, "scope")?;
    let proofs: Vec<proof::Proof> = field(head, "proofs")?;
    let mut slots = proof::slots(config).await?;
    proof::verify(&scope.deployment_id, &slots, &proofs).await?;
    if !slots.iter().all(|slot| slot.pinned(config))
        || proofs.len() != 2
        || !proofs
            .iter()
            .any(|p| p.key.key_ref == target.retiring && p.key.role != "primary")
        || !proofs
            .iter()
            .any(|p| p.key.key_ref == target.survivor && p.key.role == "primary")
        || scope.keys != [target.survivor.clone(), target.retiring.clone()].into()
    {
        return Err(conflict("layout_mismatch"));
    }
    let i = slots
        .iter()
        .position(proof::Slot::primary)
        .ok_or_else(stale)?;
    Ok(slots.swap_remove(i).strict_reader())
}

pub(crate) async fn evaluate(
    db: &Database,
    config: &AppConfig,
    target: &Target,
) -> AppResult<Evaluation> {
    target.validate()?;
    let db = transaction_service::durable_database(db);
    let mut result = Evaluation {
        revision: None,
        report_revision: None,
        as_of: None,
        certification_enabled: false,
        blockers: BTreeSet::from([LOAD_BLOCKER]),
    };
    let Some(head) = location::observed(&db).await? else {
        result.blockers.insert("unenrolled");
        return Ok(result);
    };
    result.revision = Some(field(&head, "revision")?);
    let scope: Scope = field(&head, "scope")?;
    let mut checks = guards::environment(&db, config, &head).await?;
    let Ok(keys) = reader(config, &head, target).await else {
        checks.blockers.insert("layout_mismatch");
        result.blockers.extend(checks.blockers);
        return Ok(result);
    };
    match census::Verified::open(&db, &keys, target, &scope, field(&head, "scope_revision")?).await
    {
        Ok(report) => {
            result.report_revision = Some(report.report_revision());
            guards::evidence(&db, &head, &scope, report.baseline_revision, &mut checks).await?;
            match report.check_rows(None, target).await {
                Ok(Some((cut, rows))) => {
                    result.as_of = Some(cut.time);
                    checks.blockers.extend(rows);
                }
                _ => {
                    checks.blockers.insert("census_evidence_unavailable");
                }
            }
        }
        Err(_) => {
            checks.blockers.insert("census_evidence_unavailable");
        }
    }
    if super::super::encrypted_record_migration::survivor_checkpoint(&db, &keys)
        .await
        .is_err()
    {
        checks.blockers.insert("migration_checkpoint_unreadable");
    }
    result.blockers.extend(checks.blockers);
    Ok(result)
}

pub(crate) async fn advance(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    request: &Request,
) -> AppResult<Progress> {
    request.recording.validate()?;
    request.target.validate()?;
    let release = certified_release()?;
    step(
        &transaction_service::durable_database(db),
        config,
        actor,
        request,
        &release,
    )
    .await
}

async fn step(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    request: &Request,
    _release: &CertifiedRelease,
) -> AppResult<Progress> {
    let head = location::observed(db).await?.ok_or_else(stale)?;
    let id = &request.recording.request_id;
    let digest = fingerprint(
        actor,
        "retirement",
        &(&request.recording, &request.target, request.report_revision),
    )?;
    let revision: i64 = field(&head, "revision")?;
    if head.contains_key("published_retirement") {
        let mut slots = proof::slots(config).await?;
        let i = slots
            .iter()
            .position(proof::Slot::primary)
            .ok_or_else(stale)?;
        let keys = slots.swap_remove(i).strict_reader();
        let state = publication(db, &keys, &head).await?;
        if state.request_digest == digest {
            return Ok(state.progress());
        }
        return Err(stale());
    }
    let scope: Scope = field(&head, "scope")?;
    let keys = reader(config, &head, &request.target).await?;
    let old: Option<Transition> = if head.get_str("retirement_id") == Ok(id) {
        let bytes = head
            .get_binary_generic("retirement_state")
            .map_err(|_| corrupt())?;
        let state: Transition = census::open(db, &keys, &request.target.generation, bytes).await?;
        if state.request_digest != digest
            || state.scope != scope
            || state.revision != revision
            || state.location != location::identity(db).await?
        {
            return Err(stale());
        }
        if request.expected_step_revision < revision
            && request.expected_step_revision >= request.recording.expected_revision
        {
            return Ok(state.progress());
        }
        Some(state)
    } else {
        // A new request may supersede a grant invalidated by an intervening edit.
        if revision != request.recording.expected_revision
            || head.get_i64("retirement_revision") == Ok(revision)
        {
            return Err(stale());
        }
        None
    };
    if request.expected_step_revision != revision {
        return Err(stale());
    }
    let report = census::Verified::open(
        db,
        &keys,
        &request.target,
        &scope,
        field(&head, "scope_revision")?,
    )
    .await?;
    if report.report_revision() != request.report_revision
        || old.as_ref().is_some_and(|s| {
            s.registry_digest != report.registry()
                || Some(&s.manifest) != report.manifest().ok().as_ref()
        })
    {
        return Err(stale());
    }
    let marker = old.as_ref().map(|s| (s.marker.as_str(), &s.witnessed));
    let Some((cut, rows)) = report.check_rows(marker, &request.target).await? else {
        return old.map(|s| s.progress()).ok_or_else(stale);
    };
    let mut checks = guards::environment(db, config, &head).await?;
    guards::evidence(db, &head, &scope, report.baseline_revision, &mut checks).await?;
    checks.blockers.extend(rows);
    if let Some(reason) = checks.blockers.first() {
        return Err(conflict(reason));
    }
    super::super::encrypted_record_migration::survivor_checkpoint(db, &keys).await?;
    let phase = match old.as_ref().map(|s| s.phase) {
        None => Phase::Decision,
        Some(Phase::Decision) => Phase::Approved,
        Some(Phase::Approved) => Phase::Consume,
        Some(Phase::Consume) => Phase::Consumed,
        Some(Phase::Consumed) => Phase::Published,
        Some(Phase::Published) => return Err(stale()),
    };
    if let Some(old) = &old {
        // The marker commits in the same transaction as its receipt. Mongo's
        // cluster timestamp has second resolution, so use the end of its
        // second as a conservative expiry check after an uncertain commit.
        let witnessed_before = (i64::from(cut.time.time) + 1) * 1000;
        if cut.time <= old.witnessed.time || witnessed_before > old.valid_until_ms {
            return Err(stale());
        }
    }
    let next = Transition {
        phase,
        scope,
        location: location::identity(db).await?,
        target: request.target.clone(),
        revision: revision.checked_add(1).ok_or_else(stale)?,
        report_revision: report.report_revision(),
        registry_digest: report.registry().into(),
        manifest: report.manifest()?,
        request_digest: digest,
        marker: fresh(),
        witnessed: cut,
        valid_until_ms: checks.deadline.min(report.expires_at().timestamp_millis()),
        evidence_ref: request.recording.evidence_ref.clone(),
    };
    let sealed = census::seal(db, &keys, &request.target.generation, &next).await?;
    let update = if phase == Phase::Published {
        let proofs: Vec<proof::Proof> = field(&head, "proofs")?;
        let survivor = proofs
            .into_iter()
            .filter(|p| p.key.key_ref == request.target.survivor)
            .collect::<Vec<_>>();
        let survivor_scope = next.survivor_scope();
        let survivor_key = &survivor.first().ok_or_else(stale)?.key;
        // Empty legacy proof list fences pre-publication admission binaries.
        doc! {"$set":{"published_retirement":sealed,"retirement_generation":&request.target.generation,
        "scope":encode(&survivor_scope)?, "scope_revision":next.revision, "keys":[encode(survivor_key)?],
        "current":{}, "survivor_proofs":encode(&survivor)?,"proofs":[]}, "$inc":{"revision":1_i64},
        "$unset":{"retirement_state":"","retirement_revision":""}}
    } else {
        doc! {"$set":{"retirement_state":sealed,"retirement_revision":next.revision,"retirement_id":id},"$inc":{"revision":1_i64}}
    };
    let expires = bson::DateTime::from_millis(next.valid_until_ms);
    let claim = report.claim().await?;
    #[cfg(test)]
    crate::services::durable_wake_tests::after("retirement_claimed").await;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let audit_key = audit_service::audit_chain_hmac_key().ok_or_else(corrupt)?;
    audit_service::log_actor_event_in_session(db,&mut tx,audit_key,actor,"encryption_key_lifecycle.retirement",
        json!({"phase":phase,"request_id":id,"revision":next.revision,"evidence_ref":next.evidence_ref})).await?;
    let edited = db
        .collection::<Document>(HEAD)
        .update_one(
            doc! {"_id":SINGLETON,"revision":revision,"scope":encode(&next.scope)?,
            "location":&next.location,"restore_required":false,"initializing.0":{"$exists":false},"$expr":{"$lt":["$$NOW",expires]}},
            update,
        )
        .session(&mut tx)
        .await?;
    if edited.matched_count != 1 {
        return Err(stale());
    }
    report
        .fence(
            &mut tx,
            &claim,
            (phase != Phase::Published).then_some(next.marker.as_str()),
        )
        .await?;
    transaction_service::commit(&mut tx).await?;
    #[cfg(test)]
    crate::services::durable_wake_tests::after("retirement_committed").await;
    Ok(next.progress())
}

async fn admission_reader(config: &AppConfig, head: &Document) -> AppResult<EncryptionKeys> {
    let scope: Scope = field(head, "scope")?;
    let proofs: Vec<proof::Proof> = field(head, "survivor_proofs")?;
    let mut slots = proof::slots(config).await?;
    if !slots.iter().all(|slot| slot.pinned(config))
        || !config.contextual_record_writes
        || config.allow_legacy_record_reads
        || proofs.len() != 1
        || proofs[0].key.role != "primary"
    {
        return Err(stale());
    }
    proof::verify(&scope.deployment_id, &slots, &proofs).await?;
    Ok(slots.pop().ok_or_else(stale)?.strict_reader())
}

async fn publication(
    db: &Database,
    keys: &EncryptionKeys,
    head: &Document,
) -> AppResult<Transition> {
    let generation = head
        .get_str("retirement_generation")
        .map_err(|_| corrupt())?;
    let state: Transition = census::open(
        db,
        keys,
        generation,
        head.get_binary_generic("published_retirement")
            .map_err(|_| corrupt())?,
    )
    .await?;
    let proofs: Vec<proof::Proof> = field(head, "survivor_proofs")?;
    if state.phase != Phase::Published
        || state.survivor_scope() != field(head, "scope")?
        || state.location != location::identity(db).await?
        || !census::current_registry(&state.registry_digest)
        || state.target.generation != generation
        || proofs.len() != 1
        || state.target.survivor != proofs[0].key.key_ref
        || state.revision > field(head, "revision")?
        || head.get_bool("restore_required") != Ok(false)
    {
        return Err(stale());
    }
    Ok(state)
}

pub(super) async fn admit(db: &Database, config: &AppConfig, head: &Document) -> AppResult<()> {
    let keys = admission_reader(config, head).await?;
    publication(db, &keys, head).await?;
    if !hmac_lifecycle_service::independent_file_matches(db, config.hmac_keyset_path.as_deref())
        .await?
    {
        return Err(conflict("independent_hmac_unverified"));
    }
    if !guards::native_cleanup(db).await? {
        return Err(conflict("native_parent_cleanup_unverified"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
