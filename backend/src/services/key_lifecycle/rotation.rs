use super::*;
use serde::Deserialize;
use std::{io::Read, path::Path};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub recording: Recording,
    pub scope: Scope,
    pub transition: Operation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    Prepare { proposal_id: String },
    BeginRoll,
    SelectWriter { primary: String },
    Close { generation: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    proposal_id: String,
    provider: String,
    previous: Zeroizing<String>,
}

pub(super) fn canonical(head: &Document) -> AppResult<Vec<proof::Proof>> {
    field(
        head,
        if head.contains_key("managed_proofs") {
            "managed_proofs"
        } else {
            "proofs"
        },
    )
}

fn reversed(mut proofs: Vec<proof::Proof>) -> AppResult<Vec<proof::Proof>> {
    if proofs.len() != 2
        || proofs[0].key.provider != proofs[1].key.provider
        || proofs[0].key.role != "primary"
        || proofs[1].key.role != "previous"
    {
        return Err(conflict("dual_readable_layout_required"));
    }
    proofs.swap(0, 1);
    proofs[0].key.role = "primary".into();
    proofs[1].key.role = "previous".into();
    Ok(proofs)
}

pub(super) async fn admitted(
    head: &Document,
    scope: &Scope,
    slots: &[proof::Slot],
) -> AppResult<Vec<proof::Proof>> {
    let proofs = canonical(head)?;
    if head.get_bool("rolling") == Ok(true) {
        let primary = slots.iter().find(|s| s.primary()).ok_or_else(stale)?;
        if proof::identify(&scope.deployment_id, primary, &proofs)
            .await?
            .role
            != "primary"
        {
            let alternate = reversed(proofs)?;
            proof::verify(&scope.deployment_id, slots, &alternate).await?;
            return Ok(alternate);
        }
    }
    proof::verify(&scope.deployment_id, slots, &proofs).await?;
    Ok(proofs)
}

fn load(path: &Path, id: &str, config: &AppConfig) -> AppResult<Proposal> {
    let error = || conflict("rotation_proposal_unavailable");
    let file = std::fs::File::open(path).map_err(|_| error())?;
    let metadata = file.metadata().map_err(|_| error())?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err(error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(error());
        }
    }
    let mut raw = Zeroizing::new(Vec::with_capacity(4097));
    file.take(4097).read_to_end(&mut raw).map_err(|_| error())?;
    if raw.len() > 4096 {
        return Err(error());
    }
    let p: Proposal = serde_json::from_slice(&raw).map_err(|_| error())?;
    if p.proposal_id != id || p.provider != config.key_provider || p.previous.is_empty() {
        return Err(error());
    }
    Ok(p)
}

async fn prepare(
    db: &Database,
    config: &AppConfig,
    request: &Request,
    mut proofs: Vec<proof::Proof>,
    id: &str,
    path: Option<&Path>,
) -> AppResult<Vec<proof::Proof>> {
    contract::uuid(id)?;
    if proofs.len() != 1 || proofs[0].key.role != "primary" {
        return Err(conflict("rotation_already_open"));
    }
    let proposal = load(
        path.ok_or_else(|| conflict("rotation_proposal_unavailable"))?,
        id,
        config,
    )?;
    let slots = proof::proposed_slots(config, &proposal.previous).await?;
    let [primary, next] = slots.as_slice() else {
        return Err(invalid());
    };
    if primary.selector == next.selector {
        return Err(conflict("rotation_key_collision_or_alias"));
    }
    let deployment = &request.scope.deployment_id;
    let mut history = db
        .collection::<Document>(ENTRIES)
        .find(doc! {"closed_proof":{"$exists":true}, "input_scope.deployment_id":deployment})
        .projection(doc! {"closed_proof":1})
        .await?;
    while let Some(row) = history.try_next().await? {
        let retired: proof::Proof = field(&row, "closed_proof")?;
        if proof::recognizes(deployment, next, &retired).await? {
            return Err(conflict("retired_key_reuse"));
        }
    }
    proofs.extend(proof::issue(deployment, std::slice::from_ref(next)).await?);
    proof::verify(deployment, &slots, &proofs).await?;
    Ok(proofs)
}

// PR 9 instances can still append scoped evidence after survivor publication.
// Check the immutable log as well as the pointer written by the close transaction.
pub(super) async fn closed(db: &Database, head: &Document) -> AppResult<bool> {
    let Ok(at) = head.get_i64("rotation_close_revision") else {
        return Ok(false);
    };
    let Some(entry) = db
        .collection::<Evidence>(ENTRIES)
        .find_one(doc! {"receipt.revision":at})
        .await?
    else {
        return Ok(false);
    };
    if entry.input_scope != field(head, "scope")?
        || entry.receipt.scope != entry.input_scope
        || entry.assertion
            != json!({"kind":"close","generation":head.get_str("retirement_generation").map_err(|_|corrupt())?})
    {
        return Ok(false);
    }
    Ok(db
        .collection::<Document>(ENTRIES)
        .find_one(doc! {"receipt.revision":{"$gt":at},"assertion.kind":{"$ne":"verify_ring"}})
        .projection(doc! {"_id":1})
        .await?
        .is_none())
}

pub(crate) async fn submit(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    request: &Request,
) -> AppResult<Receipt> {
    let path = std::env::var_os("NYXID_ROTATION_PROPOSAL_FILE").map(std::path::PathBuf::from);
    submit_at(db, config, actor, request, path.as_deref()).await
}

pub(super) async fn submit_at(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    request: &Request,
    path: Option<&Path>,
) -> AppResult<Receipt> {
    request.recording.validate()?;
    let db = transaction_service::durable_database(db);
    let head = location::observed(&db).await?.ok_or_else(stale)?;
    if head.get_bool("restore_required") != Ok(false) {
        return Err(stale());
    }
    let digest = fingerprint(actor, "rotation", request)?;
    if let Some(prior) = db
        .collection::<Document>(ENTRIES)
        .find_one(doc! {"_id":&request.recording.request_id})
        .await?
    {
        return repeat(prior, &digest);
    }
    if request.scope != field(&head, "scope")?
        || head.get_i64("revision") != Ok(request.recording.expected_revision)
        || head.get_array("initializing").is_ok_and(|a| !a.is_empty())
    {
        return Err(stale());
    }
    let published = head.contains_key("published_retirement");
    if !config.contextual_record_writes
        || config.allow_legacy_record_reads
        || !hmac_lifecycle_service::independent_file_matches(
            &db,
            config.hmac_keyset_path.as_deref(),
        )
        .await?
    {
        return Err(conflict("strict_reader_and_independent_hmac_required"));
    }
    let slots = proof::slots(config).await?;
    if !slots.iter().all(|slot| slot.pinned(config)) {
        return Err(conflict("unpinned_provider"));
    }
    let scope = &request.scope;
    let proofs = active_proofs(&db, config, &head, scope).await?;
    let mut deadline = i64::MAX;
    let mut closing = false;
    let mut rolling = false;
    let proofs = match &request.transition {
        Operation::Prepare { proposal_id } => {
            if published && !closed(&db, &head).await? {
                return Err(conflict("rotation_already_open"));
            }
            if published {
                deadline = retirement::guards::fleet_deadline(
                    &db,
                    &retirement::layout_cohort(&head).await?,
                )
                .await?;
            }
            prepare(&db, config, request, proofs, proposal_id, path).await?
        }
        Operation::BeginRoll => {
            if published || head.get_bool("rolling") == Ok(true) {
                return Err(stale());
            }
            reversed(canonical(&head)?)?;
            deadline =
                retirement::guards::fleet_deadline(&db, &retirement::layout_cohort(&head).await?)
                    .await?;
            rolling = true;
            proofs
        }
        Operation::SelectWriter { primary } => {
            contract::uuid(primary)?;
            if published
                || head.get_bool("rolling") != Ok(true)
                || proofs.first().map(|p| &p.key.key_ref) != Some(primary)
            {
                return Err(stale());
            }
            proofs
        }
        Operation::Close { generation } => {
            if !published
                || head.get_str("retirement_generation") != Ok(generation)
                || closed(&db, &head).await?
            {
                return Err(stale());
            }
            deadline =
                retirement::guards::fleet_deadline(&db, &retirement::layout_cohort(&head).await?)
                    .await?;
            closing = true;
            proofs
        }
    };
    let keys: Vec<_> = proofs.iter().map(|p| p.key.clone()).collect();
    let next = Scope {
        keys: keys.iter().map(|k| k.key_ref.clone()).collect(),
        ..scope.clone()
    };
    let revision = request.recording.expected_revision + 1;
    let update = if closing {
        doc! {"$set":{"rotation_close_revision":revision}}
    } else {
        doc! {"$set":{"scope":encode(&next)?,"scope_revision":revision,"keys":encode(&keys)?,
        "managed_proofs":encode(&proofs)?,"proofs":[],"rolling":rolling,"current":{},
        "survivor_lineage":published || head.get_bool("survivor_lineage")==Ok(true)},
        "$unset":{"retirement_id":"","retirement_state":"","retirement_revision":"","rotation_close_revision":"",
        "published_retirement":"","retirement_generation":"","survivor_proofs":"","retired_proof":"","load_protocol":"","load_result":""}}
    };
    let closed_proof = if closing {
        Some(field(&head, "retired_proof")?)
    } else {
        None
    };
    let edit = Edit {
        next,
        update,
        deadline,
        closed_proof,
        operation: &request.transition,
    };
    record(&Mutation {
        db: &db,
        actor: Some(actor),
        recording: &request.recording,
        scope: scope.clone(),
        digest,
        action: Action::Rotation(&edit),
    })
    .await
}

pub(super) struct Edit<'a> {
    next: Scope,
    update: Document,
    deadline: i64,
    pub(super) closed_proof: Option<bson::Bson>,
    pub(super) operation: &'a Operation,
}

impl Edit<'_> {
    pub(super) async fn apply(
        &self,
        db: &Database,
        tx: &mut ClientSession,
        receipt: &mut Receipt,
    ) -> AppResult<()> {
        let expiry = bson::DateTime::from_millis(self.deadline);
        if matches!(self.operation, Operation::Close { .. }) {
            let open = doc! {"inventory":{"$in":["writer_cohort","reader_cohort","rollback","database_credential"]},
            "opened.scope.keys":{"$in":encode(&receipt.scope.keys)?},"resolution":bson::Bson::Null};
            if db
                .collection::<Document>(DEPENDENCIES)
                .find_one(open)
                .session(&mut *tx)
                .await?
                .is_some()
            {
                return Err(conflict("dependency_open"));
            }
        }
        let result = db.collection::<Document>(HEAD).update_one(doc!{"_id":SINGLETON,"revision":receipt.revision,
            "restore_required":false,"initializing.0":{"$exists":false},"$expr":{"$lt":["$$NOW",expiry]}}, self.update.clone()).session(tx).await?;
        if result.matched_count != 1 {
            return Err(stale());
        }
        receipt.scope = self.next.clone();
        Ok(())
    }
}
