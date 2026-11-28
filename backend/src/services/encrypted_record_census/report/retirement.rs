//! Retirement consumes the private checkpoint and all its partitions. The public
//! report is a summary and is never an input to a lifecycle transition.
use super::*;
use crate::services::key_lifecycle::retirement::{Target, Transition};

/// One authenticated report, opened with the proposed survivor-only reader.
pub(in crate::services) struct Verified<'a> {
    state: Checkpoint,
    census: Census<'a>,
    store: Store<'a>,
    previous: Document,
    pub baseline_revision: i64,
    dated_blockers: BTreeSet<&'static str>,
}

impl<'a> Verified<'a> {
    pub async fn open(
        db: &'a Database,
        keys: &'a EncryptionKeys,
        target: &'a Target,
        scope: &key_lifecycle::Scope,
        scope_revision: i64,
    ) -> AppResult<Self> {
        let store = Store {
            db,
            keys,
            generation: &target.generation,
        };
        let (previous, mut state) = store.load().await?;
        let (digest, namespaces) = contract()?;
        if state.phase != Phase::Published
            || state.scope.registry_digest != digest
            || state.scope.namespaces != namespaces
            || state.start.generation != target.generation
            || state.ring.revision != scope_revision
            || state.ring.deployment != scope.deployment_id
            || state.ring.restore_epoch != scope.restore_epoch
            || state.ring.deployment_revision != scope.deployment_revision
            || !state.writers
            || state.legacy
            || state
                .ring
                .proofs
                .iter()
                .map(|p| p.key.key_ref.clone())
                .collect::<BTreeSet<_>>()
                != scope.keys
        {
            return Err(refused("report_scope_mismatch"));
        }
        let census = Census::resume(db, keys, &target.generation, state.scope.clone()).await?;
        if state.database_identity
            != census
                .state()
                .await?
                .get_document("database_identity")
                .map_err(|_| malformed())?
                .clone()
            || state.catalog != census.catalog().await?
        {
            return Err(refused("report_scope_mismatch"));
        }
        let end = state.end.as_ref().ok_or_else(malformed)?;
        if Some(census.read_page(end.page).await?.token) != state.cut_token {
            return Err(malformed());
        }
        let mut dated_blockers = BTreeSet::new();
        for hash in &state.published {
            let report: FamilyReport = store.get("family", hash).await?;
            if report.totals.unresolved > 0 {
                dated_blockers.insert("unresolved_rows");
            }
            if report
                .buckets
                .iter()
                .any(|bucket| bucket.references(&target.retiring))
            {
                dated_blockers.insert("non_survivor_key");
            }
        }
        let totals = std::mem::take(&mut state.totals);
        state.walk = Walk::default();
        while !scan::verify(&store, &mut state).await? {}
        if state.totals != totals {
            return Err(malformed());
        }
        let baseline_revision = match state.baseline_lifecycle_revision {
            Some(revision) => revision,
            None => key_lifecycle::retirement::revision_at(db, state.start.time).await?,
        };
        Ok(Self {
            state,
            census,
            store,
            previous,
            baseline_revision,
            dated_blockers,
        })
    }

    pub fn manifest(&self) -> AppResult<String> {
        Ok(hex::encode(Sha256::digest(
            self.previous
                .get_binary_generic("checkpoint")
                .map_err(|_| malformed())?,
        )))
    }
    pub fn registry(&self) -> &str {
        &self.state.scope.registry_digest
    }
    pub fn report_revision(&self) -> i64 {
        self.state.revision
    }
    pub fn expires_at(&self) -> bson::DateTime {
        self.state.expires_at
    }

    pub async fn check_rows(
        &self,
        witness: Option<(&str, &Cut)>,
        target: &Target,
    ) -> AppResult<Option<(Cut, BTreeSet<&'static str>)>> {
        let published = self.state.end.as_ref().ok_or_else(malformed)?;
        let cut = match witness {
            Some((id, prior)) => {
                if self.census.cut(id).await?.is_none() {
                    self.census.capture().await?;
                }
                let Some(cut) = self.census.cut(id).await? else {
                    return Ok(None);
                };
                if prior.generation != cut.generation
                    || prior.page < published.page
                    || cut.page <= prior.page
                    || cut.time <= prior.time
                {
                    return Err(malformed());
                }
                cut
            }
            None => published.clone(),
        };
        // The preceding witnessed cut proves every retained input readable with
        // B. For the next cut, recheck changed source rows and every family
        // whose parent or clock can change their meaning. A parent change
        // covers both its old and its new children, including missing edges.
        let prior = witness.map(|(_, cut)| cut);
        let mut changed = BTreeSet::new();
        let mut dirty: BTreeMap<String, Vec<bson::Bson>> = BTreeMap::new();
        let mut dirty_bytes = 0;
        let mut rescan = prior.is_none();
        for page in prior.map_or(0, |previous| previous.page + 1)..=cut.page {
            let batch = self.census.read_page(page).await?;
            if rescan {
                continue;
            }
            for row in batch.dirty {
                dirty_bytes += bson::to_vec(&row).map_err(|_| malformed())?.len();
                if dirty_bytes > MAX_PAGE_BYTES {
                    dirty.clear();
                    rescan = true;
                    break;
                }
                changed.insert(row.collection.clone());
                dirty.entry(row.collection).or_default().push(row.id);
            }
        }
        let at = self.census.snapshot(&cut).await?;
        let mut blockers = self.dated_blockers.clone();
        for family in observation::families() {
            let parent_changed = family
                .parent_joins()
                .iter()
                .any(|(c, _)| changed.contains(*c))
                || family.rescan_parents().iter().any(|c| changed.contains(*c));
            let clock_changed =
                family.depends_on_clock() && prior.is_some_and(|p| p.time.time != cut.time.time);
            let filter = if rescan || parent_changed || clock_changed {
                doc! {}
            } else if let Some(ids) = dirty.get(family.collection()) {
                doc! {"_id":{"$in":ids}}
            } else {
                continue;
            };
            let mut rows = at.find(family.collection(), filter, doc! {}).await?;
            while let Some(row) = rows.try_next().await? {
                let observed = family.observe_at(self.store.keys, &row, &at).await?;
                if !observed.findings.is_empty() {
                    blockers.insert("unresolved_rows");
                }
                for (_, storage) in observed.instances {
                    match storage {
                        observation::Storage::Encrypted {
                            contract, evidence, ..
                        } => {
                            use crate::crypto::record_field::authenticated::Format;
                            use observation::Contract;
                            if !matches!(
                                (contract, evidence.format),
                                (Contract::OracleOuter, Format::V2)
                                    | (Contract::Contextual, Format::V3)
                            ) {
                                blockers.insert("legacy_reader_required");
                            }
                            let (_, reference) = self.state.ring.attribution(evidence.route);
                            if reference.as_deref() != Some(target.survivor.as_str()) {
                                blockers.insert("non_survivor_key");
                            }
                        }
                        observation::Storage::Plaintext => {
                            blockers.insert("legacy_reader_required");
                        }
                        observation::Storage::Absent | observation::Storage::Sentinel => {}
                    }
                }
            }
        }
        Ok(Some((cut, blockers)))
    }

    pub async fn claim(&self) -> AppResult<String> {
        self.store.claim(&self.previous).await
    }

    pub async fn fence(
        &self,
        tx: &mut mongodb::ClientSession,
        claim: &str,
        marker: Option<&str>,
    ) -> AppResult<()> {
        let state = &self.state;
        let id = self.census.generation();
        let journal = self.census.log().update_one(
            doc! {"_id":id, "report_revision":state.revision,"claim":{"$exists":false},"invalid":{"$exists":false},"catalog":&state.catalog,
                "scope_revision":state.ring.revision, "database_identity":&state.database_identity,
                "$expr":{"$gt":["$expires_at","$$NOW"]}},
            doc! {"$inc":{"retirement_fence":1_i64}}).session(&mut *tx).await?;
        let checked = self.store.collection().update_one(doc! {"_id":id, "claim":claim,
            "checkpoint":self.previous.get("checkpoint"),"$expr":{"$and":[{"$gt":["$lease_until","$$NOW"]},{"$gt":["$expires_at","$$NOW"]}]}},
            doc! {"$unset":{"claim":""},"$set":{"lease_until":bson::DateTime::from_millis(0)}}).session(&mut *tx).await?;
        if journal.matched_count != 1 || checked.matched_count != 1 {
            return Err(refused("retirement_fence_lost"));
        }
        if let Some(marker) = marker {
            self.census
                .log()
                .insert_one(
                    doc! {"_id":self.census.marker_id(marker),"expires_at":state.expires_at},
                )
                .session(tx)
                .await?;
        }
        Ok(())
    }
}

pub(in crate::services) async fn seal(
    db: &Database,
    keys: &EncryptionKeys,
    generation: &str,
    value: &Transition,
) -> AppResult<bson::Binary> {
    Store {
        db,
        keys,
        generation,
    }
    .seal("retirement-v1", value)
    .await
}
pub(in crate::services) async fn open(
    db: &Database,
    keys: &EncryptionKeys,
    generation: &str,
    sealed: &[u8],
) -> AppResult<Transition> {
    Store {
        db,
        keys,
        generation,
    }
    .open("retirement-v1", sealed)
    .await
}

pub(in crate::services) fn current_registry(digest: &str) -> bool {
    contract().is_ok_and(|(current, _)| current == digest)
}
