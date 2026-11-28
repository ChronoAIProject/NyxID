//! One bounded, authenticated snapshot walk. Only a published checkpoint
//! exposes family and finding partitions to readers.
use super::{
    replay::{Dependents, Observations, Snapshot},
    *,
};
use crate::{
    config::AppConfig,
    services::{
        encrypted_record_migration::observation::{self, Family, manifest},
        key_lifecycle::{self, CensusRing},
    },
};
use counts::{FamilyReport, Receipt, Totals};
use futures::StreamExt;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use store::Store;

mod counts;
mod read;
mod scan;
mod store;
pub(crate) use read::{Page, Query, Status, families, findings, status};

pub(super) const REPORTS: &str = "encrypted_record_census_reports";
const ROWS: usize = 16;
const DIRTY_LIMIT: usize = 32;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Baseline,
    Capture,
    Replay,
    Reconcile,
    Verify,
    Published,
}

#[derive(Serialize, Deserialize, Default)]
struct Walk {
    family: usize,
    cursor: Option<Document>,
    link: Option<String>,
    started: bool,
    delta: usize,
    summary: FamilyReport,
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    revision: i64,
    expires_at: bson::DateTime,
    scope: Scope,
    ring: CensusRing,
    catalog: Document,
    database_identity: Document,
    writers: bool,
    legacy: bool,
    seed: bson::Binary,
    phase: Phase,
    start: Cut,
    marker: Option<String>,
    end: Option<Cut>,
    cut_token: Option<ResumeToken>,
    journal_page: i64,
    baseline: Vec<String>,
    published: Vec<String>,
    walk: Walk,
    dirty: BTreeMap<String, Row>,
    rescan: bool,
    totals: Totals,
}

impl Checkpoint {
    fn token(&self, row: &Row) -> AppResult<String> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.seed.bytes).map_err(|_| malformed())?;
        mac.update(&bson::to_vec(row).map_err(|_| malformed())?);
        Ok(hex::encode(mac.finalize().into_bytes()))
    }
}

#[derive(Serialize, Deserialize)]
struct Partition {
    previous: Option<String>,
    rows: Vec<Receipt>,
}

fn contract() -> AppResult<(String, BTreeSet<String>)> {
    let families: Vec<_> = observation::families().collect();
    let names = families
        .iter()
        .flat_map(|f| {
            std::iter::once(f.collection())
                .chain(f.parent_joins().iter().map(|j| j.0))
                .chain(f.rescan_parents().iter().copied())
        })
        .map(str::to_owned)
        .collect();
    let declarations: Vec<_> = families.iter().map(Family::declaration).collect();
    if manifest::coverage(&declarations, &names) != manifest::ManifestCoverage::Complete {
        return Err(refused("Census registry coverage incomplete"));
    }
    let contract = serde_json::to_vec(&(
        1,
        PROTOCOL,
        &declarations,
        families
            .iter()
            .map(Family::depends_on_clock)
            .collect::<Vec<_>>(),
    ))
    .map_err(|_| malformed())?;
    Ok((hex::encode(Sha256::digest(contract)), names))
}

pub(crate) async fn start(
    db: &Database,
    keys: &EncryptionKeys,
    config: &AppConfig,
) -> AppResult<Status> {
    let db = transaction_service::durable_database(db);
    db.collection::<Document>(REPORTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                .build(),
        )
        .await?;
    let ring = key_lifecycle::census_ring(&db, config).await?;
    let (registry_digest, namespaces) = contract()?;
    let scope = Scope {
        registry_digest,
        namespaces,
        deployment_revision: ring.deployment_revision.clone(),
        restore_epoch: ring.restore_epoch.clone(),
    };
    let (census, start) = Census::start(&db, keys, scope.clone()).await?;
    let head = census.state().await?;
    if head.get_i64("scope_revision") != Ok(ring.revision) {
        return Err(refused("Enrollment changed during census setup"));
    }
    let store = Store {
        db: &db,
        keys,
        generation: census.generation(),
    };
    let mut seed = vec![0; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let state = Checkpoint {
        revision: 0,
        expires_at: *head.get_datetime("expires_at").map_err(|_| malformed())?,
        ring,
        scope,
        start,
        phase: Phase::Baseline,
        catalog: head
            .get_document("catalog")
            .map_err(|_| malformed())?
            .clone(),
        database_identity: head
            .get_document("database_identity")
            .map_err(|_| malformed())?
            .clone(),
        writers: keys.contextual_record_writes(),
        legacy: keys.allow_legacy_record_reads(),
        seed: bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: seed,
        },
        marker: None,
        end: None,
        cut_token: None,
        journal_page: 0,
        baseline: vec![],
        published: vec![],
        walk: Walk::default(),
        dirty: BTreeMap::new(),
        rescan: false,
        totals: Totals::default(),
    };
    let checkpoint = store.seal("checkpoint", &state).await?;
    store.collection().insert_one(doc!{"_id":store.generation,"revision":0_i64,"checkpoint":checkpoint,"expires_at":state.expires_at,"lease_until":bson::DateTime::from_millis(0)}).await?;
    Ok(Status::from_state(&state, true, None))
}

pub(crate) async fn advance(
    db: &Database,
    keys: &EncryptionKeys,
    config: &AppConfig,
    id: &str,
) -> AppResult<Status> {
    let db = transaction_service::durable_database(db);
    let store = Store {
        db: &db,
        keys,
        generation: id,
    };
    let (doc, mut state) = store.load().await?;
    if state.phase == Phase::Published {
        return Ok(Status::from_state(
            &state,
            contract().is_ok_and(|(digest, _)| state.scope.registry_digest == digest),
            None,
        ));
    }
    let (digest, namespaces) = contract()?;
    if state.scope.registry_digest != digest
        || state.scope.namespaces != namespaces
        || state.writers != keys.contextual_record_writes()
        || state.legacy != keys.allow_legacy_record_reads()
    {
        return Err(refused("Census registry or policy changed"));
    }
    state.ring.verify(config).await?;
    let census = Census::resume(&db, keys, id, state.scope.clone()).await?;
    let claim = store.claim(&doc).await?;
    let work = async {
        run_page(&census, &store, &mut state).await?;
        census.state().await?;
        store.commit(&census, &claim, &doc, &mut state).await
    }
    .await;
    if let Err(error) = work {
        store
            .collection()
            .update_one(
                doc! {"_id":id,"claim":claim},
                doc! {"$unset":{"claim":""},"$set":{"lease_until":bson::DateTime::from_millis(0)}},
            )
            .await?;
        return Err(error);
    }
    Ok(Status::from_state(&state, true, None))
}

async fn run_page(census: &Census<'_>, store: &Store<'_>, state: &mut Checkpoint) -> AppResult<()> {
    match state.phase {
        Phase::Baseline => {
            census.capture().await?;
            let start = state.start.clone();
            let at = census.snapshot(&start).await?;
            if scan::walk(store, state, &at, false).await? {
                state.marker = Some(census.request_cut().await?);
                state.phase = Phase::Capture;
            }
        }
        Phase::Capture => {
            census.capture().await?;
            if let Some(end) = census
                .cut(state.marker.as_deref().ok_or_else(malformed)?)
                .await?
            {
                let batch = census.read_page(end.page).await?;
                state.cut_token = Some(batch.token);
                state.end = Some(end);
                state.phase = Phase::Replay;
            }
        }
        Phase::Replay => {
            let end = state.end.clone().ok_or_else(malformed)?;
            // Each interval page contributes its closure at the same W. The
            // union is idempotent even for repeated updates to one document.
            let page = state.journal_page.checked_add(1).ok_or_else(malformed)?;
            if state.rescan {
                census.read_page(page).await?;
            } else {
                let start = state.start.clone();
                let mut plan = scan::Plan { store, state };
                census
                    .reconcile_range(&start, &end, page..=page, &mut plan)
                    .await?;
            }
            state.journal_page = page;
            if page < end.page {
                return Ok(());
            }
            state.phase = Phase::Reconcile;
        }
        Phase::Reconcile => {
            let cut = state.end.clone().ok_or_else(malformed)?;
            let at = census.snapshot(&cut).await?;
            if scan::walk(store, state, &at, true).await? {
                state.phase = Phase::Verify;
            }
        }
        Phase::Verify => {
            if scan::verify(store, state).await? {
                state.phase = Phase::Published;
            }
        }
        Phase::Published => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
