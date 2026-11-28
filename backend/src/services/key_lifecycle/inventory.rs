use super::*;
use std::collections::BTreeSet;

fn name(value: impl Serialize) -> AppResult<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(corrupt)
}

pub(super) fn unresolved(scope: &Scope, scope_revision: i64) -> Document {
    doc! {"$or":[{"resolution":bson::Bson::Null},{"last_record.revision":{"$lt":scope_revision}},{"last_record.scope.restore_epoch":{"$ne":&scope.restore_epoch}},
    {"last_record.scope.deployment_revision":{"$ne":&scope.deployment_revision}}]}
}

fn current_key(kind: &str, value: impl Serialize) -> AppResult<String> {
    Ok(format!("{kind}_{}", name(value)?))
}

fn set_current(
    update: &mut Document,
    field: &str,
    receipt: &Receipt,
    disposition: String,
    expiry: Option<i64>,
) -> AppResult<()> {
    let mut values = Document::new();
    let current = encode(&Current {
        entry_id: receipt.entry_id.clone(),
        revision: receipt.revision,
        disposition,
        valid_until_ms: expiry,
    })?;
    for key in &receipt.scope.keys {
        values.insert(format!("current.{field}.{key}"), current.clone());
    }
    update.insert("$set", values);
    Ok(())
}

fn invalidations(kind: Inventory, scope: &Scope, clear_inventory: bool) -> AppResult<Document> {
    let assertion = match kind {
        Inventory::WriterCohort => Assertion::OldWritersWithdrawn,
        Inventory::DatabaseCredential => Assertion::DatabaseAccessRevoked,
        _ => Assertion::RecoveryDependenciesClosed,
    };
    let inventory_field = current_key("inventory", kind)?;
    let assertion_field = current_key("assertion", assertion)?;
    let mut unset = Document::new();
    for key in &scope.keys {
        if clear_inventory {
            unset.insert(format!("current.{inventory_field}.{key}"), "");
        }
        unset.insert(format!("current.{assertion_field}.{key}"), "");
    }
    Ok(unset)
}

async fn open(
    db: &Database,
    tx: &mut ClientSession,
    receipt: &Receipt,
    owner: &str,
    kind: Inventory,
    parents: &BTreeSet<String>,
    formats: &BTreeSet<String>,
) -> AppResult<()> {
    if parents.len() > 16 || formats.is_empty() || formats.len() > 16 {
        return Err(invalid());
    }
    if matches!(kind, Inventory::IncrementalBackup | Inventory::Pitr) && parents.is_empty() {
        return Err(invalid());
    }
    for format in formats {
        contract::label(format)?;
    }
    let deps = db.collection::<Document>(DEPENDENCIES);
    for parent in parents {
        contract::uuid(parent)?;
        let row = deps
            .find_one(doc! {"_id":parent})
            .session(&mut *tx)
            .await?
            .ok_or_else(|| conflict("Dependency parent unavailable"))?;
        let dep: Dependency = bson::from_document(row).map_err(|_| corrupt())?;
        if !dep.opened.scope.keys.is_subset(&receipt.scope.keys) || !dep.formats.is_subset(formats)
        {
            return Err(conflict(
                "Dependency parent keys or formats are outside this scope",
            ));
        }
    }
    let dependency = Dependency {
        dependency_id: receipt.entry_id.clone(),
        inventory: kind,
        opened: receipt.clone(),
        last_record: receipt.clone(),
        parents: parents.clone(),
        formats: formats.clone(),
        owner: owner.into(),
        resolution: None,
    };
    let mut row = bson::to_document(&dependency).map_err(|_| corrupt())?;
    row.insert("_id", &receipt.entry_id);
    deps.insert_one(row).session(tx).await?;
    Ok(())
}

async fn resolve_dependency(
    db: &Database,
    tx: &mut ClientSession,
    id: &str,
    resolution: Option<Resolution>,
    receipt: &Receipt,
    scope_revision: i64,
) -> AppResult<Inventory> {
    contract::uuid(id)?;
    let deps = db.collection::<Document>(DEPENDENCIES);
    let row = deps
        .find_one(doc! {"_id":id})
        .session(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Unknown dependency"))?;
    let dep: Dependency = bson::from_document(row).map_err(|_| corrupt())?;
    if dep.opened.scope.keys != receipt.scope.keys
        || resolution.is_some_and(|r| !dep.inventory.permits(r))
    {
        return Err(invalid());
    }
    let mut children = unresolved(&receipt.scope, scope_revision);
    children.insert("parents", id);
    if resolution.is_some()
        && deps
            .find_one(children)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
    {
        return Err(conflict("Restorable dependents require closure evidence"));
    }
    deps.update_one(
        doc! {"_id":id},
        doc! {"$set":{"resolution":encode(&resolution)?,"last_record":encode(receipt)?}},
    )
    .session(tx)
    .await?;
    Ok(dep.inventory)
}

pub(super) async fn apply(
    db: &Database,
    tx: &mut ClientSession,
    row: &Document,
    ring: &Scope,
    receipt: &mut Receipt,
    recording: &Recording,
    change: &Change,
) -> AppResult<()> {
    let mut update = Document::new();
    let scope_revision = field(row, "scope_revision")?;
    match change {
        Change::OpenDependency {
            inventory,
            parents,
            formats,
        } => {
            if *inventory == Inventory::Unknown && receipt.scope.keys != ring.keys {
                return Err(invalid());
            }
            open(
                db,
                tx,
                receipt,
                &recording.owner,
                *inventory,
                parents,
                formats,
            )
            .await?;
            update.insert("$unset", invalidations(*inventory, &receipt.scope, true)?);
        }
        Change::ReopenDependency { dependency_id } => {
            let kind =
                resolve_dependency(db, tx, dependency_id, None, receipt, scope_revision).await?;
            update.insert("$unset", invalidations(kind, &receipt.scope, true)?);
        }
        Change::CloseDependency {
            dependency_id,
            resolution,
        } => {
            let kind = resolve_dependency(
                db,
                tx,
                dependency_id,
                Some(*resolution),
                receipt,
                scope_revision,
            )
            .await?;
            update.insert("$unset", invalidations(kind, &receipt.scope, true)?);
        }
        Change::ReviewInventory {
            inventory,
            disposition,
        } => {
            if matches!(disposition, Review::ReviewedEmpty) {
                let mut filter = unresolved(&receipt.scope, scope_revision);
                filter.insert("inventory", name(inventory)?);
                filter.insert(
                    "opened.scope.keys",
                    doc! {"$in":encode(&receipt.scope.keys)?},
                );
                if db
                    .collection::<Document>(DEPENDENCIES)
                    .find_one(filter)
                    .projection(doc! {"_id":1})
                    .session(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Err(conflict("Inventory contains unresolved dependencies"));
                }
            }
            let field = current_key("inventory", inventory)?;
            set_current(&mut update, &field, receipt, name(disposition)?, None)?;
            // A changed inventory supersedes the previous aggregate assertion.
            update.insert("$unset", invalidations(*inventory, &receipt.scope, false)?);
        }
        Change::Attest {
            assertion,
            affirmed,
            valid_until_ms,
        } => {
            if valid_until_ms.is_some_and(|end| end <= receipt.recorded_at_ms) {
                return Err(invalid());
            }
            if *affirmed
                && let Some(kind) = match assertion {
                    Assertion::OldWritersWithdrawn => Some(Inventory::WriterCohort),
                    Assertion::DatabaseAccessRevoked => Some(Inventory::DatabaseCredential),
                    _ => None,
                }
            {
                let mut filter = unresolved(&receipt.scope, scope_revision);
                filter.insert("inventory", name(kind)?);
                filter.insert(
                    "opened.scope.keys",
                    doc! {"$in":encode(&receipt.scope.keys)?},
                );
                if db
                    .collection::<Document>(DEPENDENCIES)
                    .find_one(filter)
                    .projection(doc! {"_id":1})
                    .session(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Err(conflict("An open dependency contradicts this assertion"));
                }
            }
            if *affirmed && matches!(assertion, Assertion::RecoveryDependenciesClosed) {
                check_recovery(db, tx, row, &receipt.scope).await?;
            }
            let field = current_key("assertion", assertion)?;
            set_current(
                &mut update,
                &field,
                receipt,
                if *affirmed { "affirmed" } else { "withdrawn" }.into(),
                *valid_until_ms,
            )?;
        }
        Change::AdvanceRestoreEpoch | Change::SetDeploymentRevision { .. } => {
            if &receipt.scope != ring {
                return Err(stale());
            }
            match change {
                Change::SetDeploymentRevision {
                    deployment_revision,
                } => {
                    contract::label(deployment_revision)?;
                    if deployment_revision == &ring.deployment_revision {
                        return Err(invalid());
                    }
                    receipt.scope.deployment_revision = deployment_revision.clone();
                }
                _ => {
                    receipt.scope.restore_epoch = fresh();
                    update.insert("$unset", doc! {"initializing":""});
                }
            }
            update.insert("$set",doc!{"scope":encode(&receipt.scope)?,"scope_revision":receipt.revision,"current":doc!{}});
        }
        Change::VerifyRing => return Ok(()),
    }
    db.collection::<Document>(HEAD)
        .update_one(doc! {"_id":SINGLETON,"revision":receipt.revision}, update)
        .session(tx)
        .await?;
    Ok(())
}

async fn check_recovery(
    db: &Database,
    tx: &mut ClientSession,
    row: &Document,
    scope: &Scope,
) -> AppResult<()> {
    use Inventory::*;
    let current = row.get_document("current").map_err(|_| corrupt())?;
    let recovery = [
        FullBackup,
        IncrementalBackup,
        Pitr,
        Clone,
        Archive,
        RestoreDrill,
        Rollback,
        Unknown,
    ];
    for kind in recovery {
        let field = current_key("inventory", kind)?;
        for key in &scope.keys {
            let disposition = current
                .get_document(&field)
                .and_then(|m| m.get_document(key))
                .and_then(|c| c.get_str("disposition"));
            if !matches!(disposition, Ok("reviewed" | "reviewed_empty")) {
                return Err(conflict("Recovery inventory has not been reviewed"));
            }
        }
    }
    let mut filter = unresolved(scope, field(row, "scope_revision")?);
    filter.insert("opened.scope.keys", doc! {"$in":encode(&scope.keys)?});
    filter.insert("inventory", doc! {"$in":encode(&recovery)?});
    if db
        .collection::<Document>(DEPENDENCIES)
        .find_one(filter)
        .projection(doc! {"_id":1})
        .session(tx)
        .await?
        .is_some()
    {
        return Err(conflict("Recovery dependencies require closure evidence"));
    }
    Ok(())
}
