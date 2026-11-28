use super::*;

pub(super) struct Checks {
    pub blockers: BTreeSet<&'static str>,
    pub deadline: i64,
}

pub(super) async fn environment(
    db: &Database,
    config: &AppConfig,
    head: &Document,
) -> AppResult<Checks> {
    let mut checks = Checks {
        blockers: BTreeSet::new(),
        deadline: i64::MAX,
    };
    if head.contains_key("initializing") && !head.get_array("initializing").is_ok_and(Vec::is_empty)
    {
        checks.blockers.insert("initialization_in_progress");
    }
    if head.get_bool("restore_required") != Ok(false) {
        checks.blockers.insert("restore_required");
    }
    if !config.contextual_record_writes || config.allow_legacy_record_reads {
        checks.blockers.insert("reader_writer_policy");
    }
    if !hmac_lifecycle_service::independent_file_matches(db, config.hmac_keyset_path.as_deref())
        .await
        .unwrap_or(false)
    {
        checks.blockers.insert("independent_hmac_unverified");
    }
    let ready = fleet::without_diagnostics(fleet::serving_report(db, CAPABILITY)).await;
    let bound = cohort(&field(head, "scope")?);
    // Every newly enrolled generation holds the lifecycle reservation until it
    // publishes its first observation. Expiry of the oldest member also fences
    // the decision at database time, even if proof validation took a long time.
    let age_ms = fleet::MAX_AGE_SECS * 1_000;
    let mut live = db
        .collection::<Document>(crate::models::security_replica::COLLECTION_NAME)
        .find(doc! {"$expr":{"$gte":["$observed_at",{"$subtract":["$$NOW",age_ms]}]}})
        .projection(doc! {"retirement_cohort":1,"observed_at":1})
        .limit(257)
        .await?;
    let mut count = 0;
    while let Some(row) = live.try_next().await? {
        count += 1;
        if row.get_str("retirement_cohort") != Ok(bound.as_str()) {
            checks.blockers.insert("writer_fleet_unverified");
        }
        let seen = row
            .get_datetime("observed_at")
            .map_err(|_| corrupt())?
            .timestamp_millis();
        checks.deadline = checks.deadline.min(seen.saturating_add(age_ms));
    }
    if count == 0
        || count > 256
        || !ready
            .is_ok_and(|r| r.activated && r.ready && r.serving_ready && r.live_generations == count)
    {
        checks.blockers.insert("writer_fleet_unverified");
    }
    if !native_cleanup(db).await.unwrap_or(false) {
        checks.blockers.insert("native_parent_cleanup_unverified");
    }
    Ok(checks)
}

pub(super) async fn native_cleanup(db: &Database) -> AppResult<bool> {
    if !crate::services::platform_entry_migration::native_writes_ready(db).await? {
        return Ok(false);
    }
    let mut indexes = db
        .collection::<Document>("platform_native_entry_receipts")
        .list_indexes()
        .await?;
    let mut indexed = false;
    while let Some(index) = indexes.try_next().await? {
        if index
            .options
            .as_ref()
            .is_some_and(|o| o.expire_after.is_some())
        {
            return Ok(false);
        }
        indexed |= index.keys == doc! {"expires_at":1}
            && index.options.as_ref().is_none_or(|o| {
                o.partial_filter_expression.is_none()
                    && o.hidden != Some(true)
                    && o.sparse != Some(true)
            });
    }
    Ok(indexed)
}

pub(super) const ASSERTIONS: [(&str, Assertion); 4] = [
    ("old_writers_withdrawn", Assertion::OldWritersWithdrawn),
    ("database_access_revoked", Assertion::DatabaseAccessRevoked),
    ("restore_continuity", Assertion::RestoreContinuity),
    (
        "recovery_dependencies_closed",
        Assertion::RecoveryDependenciesClosed,
    ),
];
pub(super) const INVENTORIES: [Inventory; 14] = [
    Inventory::WriterCohort,
    Inventory::ReaderCohort,
    Inventory::IndependentHmac,
    Inventory::MigrationCanary,
    Inventory::DatabaseCredential,
    Inventory::FullBackup,
    Inventory::IncrementalBackup,
    Inventory::Pitr,
    Inventory::Clone,
    Inventory::Archive,
    Inventory::RestoreDrill,
    Inventory::Rollback,
    Inventory::CloudVersion,
    Inventory::Unknown,
];

pub(super) async fn evidence(
    db: &Database,
    head: &Document,
    scope: &Scope,
    baseline: i64,
    checks: &mut Checks,
) -> AppResult<()> {
    let current: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Current>> =
        field(head, "current")?;
    let now = db
        .run_command(doc! {"hello":1})
        .await?
        .get_datetime("localTime")
        .map_err(|_| corrupt())?
        .timestamp_millis();
    let revision = field::<i64>(head, "revision")?;
    // The singleton is the serialization point, but its pointers alone cannot
    // establish who recorded the evidence, its exact scope or its contents.
    let ids: BTreeSet<&str> = current
        .values()
        .flat_map(|m| m.values().map(|c| c.entry_id.as_str()))
        .collect();
    let mut entries = std::collections::BTreeMap::new();
    let mut cursor = db
        .collection::<Document>(ENTRIES)
        .find(doc! {"_id":{"$in":encode(&ids)?}})
        .await?;
    while let Some(raw) = cursor.try_next().await? {
        let id = raw.get_str("_id").map_err(|_| corrupt())?.to_owned();
        let row: Evidence = bson::from_document(raw).map_err(|_| corrupt())?;
        let covered = &row.receipt.scope;
        if row.schema_version == 1
            && row.receipt.entry_id == id
            && row.receipt.revision.checked_sub(1) == Some(row.previous_revision)
            && row.input_scope == *covered
            && covered.keys.is_subset(&scope.keys)
            && covered.deployment_id == scope.deployment_id
            && covered.deployment_revision == scope.deployment_revision
            && covered.restore_epoch == scope.restore_epoch
            && row.receipt.recorded_at_ms <= now
            && contract::label(&row.owner).is_ok()
            && contract::uuid(&row.evidence_ref).is_ok()
            && !row.issuer.is_empty()
            && !row.producer.is_empty()
        {
            entries.insert(id, row);
        }
    }
    let checked = |name: &str, key: &str| {
        let pointer = current.get(name)?.get(key)?;
        let row = entries.get(&pointer.entry_id)?;
        (row.receipt.revision == pointer.revision
            && pointer.revision <= revision
            && row.receipt.scope.keys.contains(key))
        .then_some((pointer, &row.assertion))
    };
    for (label, assertion) in ASSERTIONS {
        let name = format!("assertion_{label}");
        for key in &scope.keys {
            let Some((pointer, value)) = checked(&name, key) else {
                checks.blockers.insert(label);
                continue;
            };
            let required = json!(Change::Attest {
                assertion,
                affirmed: true,
                valid_until_ms: pointer.valid_until_ms
            });
            if pointer.revision <= baseline
                || pointer.disposition != "affirmed"
                || pointer.valid_until_ms.is_some_and(|end| end <= now)
                || value != &required
            {
                checks.blockers.insert(label);
                continue;
            }
            checks.deadline = checks
                .deadline
                .min(pointer.valid_until_ms.unwrap_or(i64::MAX));
        }
    }
    let scope_revision = field::<i64>(head, "scope_revision")?;
    for kind in INVENTORIES {
        let kind = serde_json::to_value(kind).map_err(|_| corrupt())?;
        let name = format!("inventory_{}", kind.as_str().ok_or_else(corrupt)?);
        for key in &scope.keys {
            if !checked(&name,key).is_some_and(|(pointer,value)| {
                pointer.revision>=scope_revision && matches!(pointer.disposition.as_str(),"reviewed"|"reviewed_empty")
                    && *value==json!({"kind":"review_inventory","inventory":kind,"disposition":pointer.disposition})
            }) {
                checks.blockers.insert("inventory_unreviewed");
            }
        }
    }
    let mut unresolved = inventory::unresolved(scope, field(head, "scope_revision")?);
    unresolved.insert("opened.scope.keys", doc! {"$in":encode(&scope.keys)?});
    if db
        .collection::<Document>(DEPENDENCIES)
        .find_one(unresolved)
        .projection(doc! {"_id":1})
        .await?
        .is_some()
    {
        checks.blockers.insert("dependency_open");
    }
    Ok(())
}
