//! Restartable orphan scans use the same server-time claim and token-fenced
//! checkpoint protocol as personal_erasure_service::worker. Candidate pages
//! contain identifiers only; no document bodies enter the durable report.
use super::*;
use mongodb::options::{ReadConcern, ReturnDocument, WriteConcern};

pub const JOBS: &str = "personal_erasure_orphan_sweep_jobs";
pub const PAGES: &str = "personal_erasure_orphan_sweep_pages";
const SCAN_VALUES: usize = 512;
const LIVE_BATCH: usize = 128;
const LEASE_SECONDS: i64 = 120;
const RETRY_CAP: u32 = 8;
const RETENTION_DAYS: i64 = 7;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepStatus {
    pub job_id: String,
    pub status: String,
    pub phase: String,
    pub report: Option<OrphanSweepReport>,
    pub failure: Option<String>,
    pub noop_count: usize,
}

#[derive(Clone, Serialize, Deserialize)]
struct Job {
    #[serde(rename = "_id")]
    id: String,
    requested_by: String,
    apply: bool,
    expected_digest: Option<String>,
    #[serde(default)]
    source_job_id: Option<String>,
    #[serde(default)]
    consecutive_failures: u32,
    #[serde(default)]
    noop_count: usize,
    status: String,
    phase: String,
    scan_spec: usize,
    specs_digest: String,
    last_id: Bson,
    owner_offset: usize,
    next_page: u64,
    apply_page: u64,
    report: Option<OrphanSweepReport>,
    failure: Option<String>,
    lease_token: Option<String>,
    lease_until: Option<bson::DateTime>,
    created_at: bson::DateTime,
    updated_at: bson::DateTime,
}

impl Job {
    fn status(self) -> SweepStatus {
        SweepStatus {
            job_id: self.id,
            status: self.status,
            phase: self.phase,
            report: self.report,
            failure: self.failure,
            noop_count: self.noop_count,
        }
    }
    fn fence(&self) -> Document {
        doc! {"_id": &self.id, "status": "pending", "lease_token": &self.lease_token,
        "$expr": {"$gt": ["$lease_until", "$$NOW"]}}
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Candidate {
    owner_id: String,
    row_id: Bson,
}

#[derive(Clone, Serialize, Deserialize)]
struct CandidatePage {
    #[serde(rename = "_id")]
    id: String,
    collection: String,
    field: String,
    parent: Option<String>,
    rows: Vec<Candidate>,
}

#[derive(Clone, Copy)]
struct ScanSpec {
    collection: &'static str,
    field: &'static str,
    parent: Option<&'static str>,
}

fn specs() -> Vec<ScanSpec> {
    let mut specs = Vec::new();
    for entry in personal_data_registry::entries() {
        match entry.erasure {
            Erasure::JoinedChild { parent } => {
                if let Some((field, parent)) = joined_child_spec(entry.collection, parent) {
                    specs.push(ScanSpec {
                        collection: entry.collection,
                        field,
                        parent: Some(parent),
                    });
                }
            }
            Erasure::LocalInventory | Erasure::Redacted => {
                if let Some(fields) = owner_fields(entry) {
                    specs.extend(fields.iter().map(|field| ScanSpec {
                        collection: entry.collection,
                        field,
                        parent: None,
                    }));
                }
            }
            _ => {}
        }
    }
    specs.sort_by_key(|spec| (spec.collection, spec.field));
    specs
}

fn page_id(job: &str, page: u64) -> String {
    format!("{job}:{page:020}")
}
fn page_range(job: &str) -> Document {
    doc! {"_id": {"$gte": format!("{job}:"), "$lt": format!("{job};")}}
}

pub async fn start(
    db: &mongodb::Database,
    actor: &str,
    apply: bool,
    expected: Option<&str>,
) -> AppResult<SweepStatus> {
    if apply && expected.is_none() {
        return Err(AppError::ValidationError(
            "Apply requires the dry-run digest".into(),
        ));
    }
    // Pin the reviewed pages. A live scan is not an authorization artifact.
    let reviewed = if apply {
        Some(
            db.collection::<Job>(JOBS)
                .find_one(doc! {"apply":false,"status":"complete","report.digest":expected})
                .sort(doc! {"updated_at":-1,"_id":-1})
                .await?
                .ok_or_else(|| {
                    AppError::Conflict("Reviewed dry-run is unavailable; run a new dry-run".into())
                })?,
        )
    } else {
        None
    };
    let mut report = reviewed.as_ref().and_then(|job| job.report.clone());
    if let Some(report) = &mut report {
        report.dry_run = false;
    }
    let job = Job {
        id: Uuid::new_v4().to_string(),
        requested_by: actor.into(),
        apply,
        expected_digest: expected.map(str::to_owned),
        source_job_id: reviewed.as_ref().map(|job| job.id.clone()),
        consecutive_failures: 0,
        noop_count: 0,
        status: "pending".into(),
        phase: if apply { "apply" } else { "scan" }.into(),
        scan_spec: 0,
        specs_digest: reviewed
            .as_ref()
            .map(|job| job.specs_digest.clone())
            .unwrap_or(specs_digest()?),
        last_id: Bson::Null,
        owner_offset: 0,
        next_page: reviewed.as_ref().map_or(0, |job| job.next_page),
        apply_page: 0,
        report,
        failure: None,
        lease_token: None,
        lease_until: None,
        created_at: bson::DateTime::now(),
        updated_at: bson::DateTime::now(),
    };
    if let Some(source) = &job.source_job_id {
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::majority())
            .await?;
        let result: AppResult<()> = async {
            if db
                .collection::<Document>(JOBS)
                .update_one(
                    doc! {"_id":source,"status":"complete"},
                    doc! {"$inc":{"apply_references":1}},
                )
                .session(&mut tx)
                .await?
                .matched_count
                != 1
            {
                return Err(AppError::Conflict("Reviewed dry-run expired".into()));
            }
            db.collection::<Job>(JOBS)
                .insert_one(&job)
                .session(&mut tx)
                .await?;
            Ok(())
        }
        .await;
        super::super::org_membership_authority::finish(tx, result).await?;
    } else {
        db.collection::<Job>(JOBS).insert_one(&job).await?;
    }
    Ok(job.status())
}

pub async fn status(db: &mongodb::Database, id: &str) -> AppResult<SweepStatus> {
    db.collection::<Job>(JOBS)
        .find_one(doc! {"_id": id})
        .await?
        .map(Job::status)
        .ok_or_else(|| AppError::NotFound("Orphan sweep not found".into()))
}

async fn claim(db: &mongodb::Database) -> AppResult<Option<Job>> {
    Ok(db.collection::<Job>(JOBS).find_one_and_update(
        doc! {"status": "pending", "$expr": {"$lte": [{"$ifNull": ["$lease_until", bson::DateTime::from_millis(0)]}, "$$NOW"]}},
        vec![doc! {"$set": {"lease_token": Uuid::new_v4().to_string(), "updated_at": "$$NOW",
            "lease_until": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": LEASE_SECONDS}}}}],
    ).sort(doc! {"updated_at": 1, "_id": 1}).return_document(ReturnDocument::After)
        .write_concern(WriteConcern::majority()).await?)
}

async fn save(db: &mongodb::Database, job: &Job, pages: Vec<CandidatePage>) -> AppResult<()> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        // Writing the claimed job conflicts with a successor claim. All page
        // inserts and cursor advancement share the same majority transaction.
        if db
            .collection::<Document>(JOBS)
            .update_one(job.fence(), doc! {"$inc": {"checkpoint_revision": 1_i64}})
            .session(&mut tx)
            .await?
            .matched_count
            != 1
        {
            return Err(AppError::Conflict("Orphan sweep lease expired".into()));
        }
        if !pages.is_empty() {
            db.collection::<CandidatePage>(PAGES)
                .insert_many(pages)
                .session(&mut tx)
                .await?;
        }
        let mut stored = bson::to_document(job)
            .map_err(|_| AppError::Internal("Orphan sweep checkpoint encoding failed".into()))?;
        stored.remove("_id");
        stored.remove("updated_at");
        stored.insert("lease_token", Bson::Null);
        stored.insert("lease_until", Bson::Null);
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &job.id},
                doc! {"$set": stored, "$currentDate": {"updated_at": true}},
            )
            .session(&mut tx)
            .await?;
        Ok(())
    }
    .await;
    super::super::org_membership_authority::finish(tx, result).await
}

async fn live_ids(
    db: &mongodb::Database,
    spec: ScanSpec,
    rows: &[Candidate],
) -> AppResult<HashSet<String>> {
    let mut live = HashSet::new();
    let owners = rows
        .iter()
        .map(|row| row.owner_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for chunk in owners.chunks(LIVE_BATCH) {
        if let Some(parent) = spec.parent {
            let mut cursor = db
                .collection::<Document>(parent)
                .find(doc! {"_id": {"$in": chunk}})
                .projection(doc! {"_id": 1})
                .batch_size(LIVE_BATCH as u32)
                .max_time(AGGREGATE_TIMEOUT)
                .await?;
            while let Some(row) = cursor.try_next().await? {
                if let Ok(id) = row.get_str("_id") {
                    live.insert(id.to_owned());
                }
            }
        } else {
            for (collection, field) in [
                (crate::models::user::COLLECTION_NAME, "_id"),
                (crate::models::service_account::COLLECTION_NAME, "_id"),
                (crate::models::personal_erasure::COLLECTION_NAME, "user_id"),
            ] {
                let mut cursor = db
                    .collection::<Document>(collection)
                    .find(doc! {field: {"$in": chunk}})
                    .projection(doc! {field: 1})
                    .batch_size(LIVE_BATCH as u32)
                    .max_time(AGGREGATE_TIMEOUT)
                    .await?;
                while let Some(row) = cursor.try_next().await? {
                    if let Ok(id) = row.get_str(field) {
                        live.insert(id.to_owned());
                    }
                }
            }
        }
    }
    Ok(live)
}

fn values(spec: ScanSpec, row: &Document) -> Vec<String> {
    let mut values = if spec.collection == "channel_send_claims" && spec.field == "_id" {
        row.get_str("_id")
            .ok()
            .and_then(|id| id.split_once(':'))
            .map(|(id, _)| vec![id.to_owned()])
            .unwrap_or_default()
    } else if spec.collection == "invite_codes" && spec.field == "usages.user_id" {
        row.get_array("usages")
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|usage| {
                let usage = usage.as_document()?;
                usage
                    .contains_key("email")
                    .then(|| usage.get_str("user_id").ok())
                    .flatten()
                    .map(str::to_owned)
            })
            .collect()
    } else {
        owner_values(row, spec.field)
    };
    values.retain(|value| {
        if spec.parent.is_some() {
            !value.is_empty() && value.len() <= 256
        } else {
            candidate_owner_id(value)
        }
    });
    values.sort();
    values.dedup();
    values
}

async fn scan_page(db: &mongodb::Database, job: &mut Job) -> AppResult<Vec<CandidatePage>> {
    let specs = specs();
    let Some(spec) = specs.get(job.scan_spec).copied() else {
        job.phase = "summarize".into();
        return Ok(Vec::new());
    };
    let mut filter = doc! {spec.field: {"$exists": true}};
    if job.last_id != Bson::Null {
        let comparison = if job.owner_offset > 0 { "$gte" } else { "$gt" };
        // A persisted id is data, including ids beginning with '$'.
        filter.insert("_id", doc! {comparison: job.last_id.clone()});
    }
    if let Some(attribution) = personal_data_registry::attribution(spec.collection, spec.field) {
        filter = doc! {"$and": [filter, super::super::personal_attribution_service::pending_filter(attribution, None)]};
    }
    if spec.collection == "invite_codes" && spec.field == "created_by" {
        filter = doc! {"$and": [filter, {"note": {"$exists": true, "$ne": Bson::Null}}]};
    }
    let mut projection = doc! {"_id": 1, spec.field: 1};
    if spec.collection == "invite_codes" {
        projection = doc! {"_id": 1, "usages.user_id": 1, "usages.email": 1};
    }
    let mut cursor = db
        .collection::<Document>(spec.collection)
        .find(filter)
        .sort(doc! {"_id": 1})
        .projection(projection)
        .limit(SCAN_VALUES as i64)
        .batch_size(LIVE_BATCH as u32)
        .max_time(AGGREGATE_TIMEOUT)
        .await?;
    let mut rows = Vec::new();
    let mut observed = 0;
    let mut exhausted = true;
    while let Some(row) = cursor.try_next().await? {
        observed += 1;
        let id = row
            .get("_id")
            .cloned()
            .ok_or_else(|| AppError::Internal("Orphan scan row has no identity".into()))?;
        let owners = values(spec, &row);
        let offset = if id == job.last_id {
            job.owner_offset
        } else {
            0
        };
        for (index, owner) in owners.iter().enumerate().skip(offset) {
            rows.push(Candidate {
                owner_id: owner.clone(),
                row_id: id.clone(),
            });
            job.last_id = id.clone();
            job.owner_offset = index + 1;
            if rows.len() == SCAN_VALUES {
                exhausted = false;
                break;
            }
        }
        if job.owner_offset >= owners.len() || owners.is_empty() {
            job.last_id = id;
            job.owner_offset = 0;
        }
        if !exhausted {
            break;
        }
    }
    if observed == SCAN_VALUES {
        exhausted = false;
    }
    let live = live_ids(db, spec, &rows).await?;
    rows.retain(|row| !live.contains(&row.owner_id));
    let mut pages = Vec::new();
    for rows in rows.chunks(APPLY_BATCH) {
        pages.push(CandidatePage {
            id: page_id(&job.id, job.next_page),
            collection: spec.collection.into(),
            field: spec.field.into(),
            parent: spec.parent.map(str::to_owned),
            rows: rows.to_vec(),
        });
        job.next_page += 1;
    }
    if exhausted {
        job.scan_spec += 1;
        job.last_id = Bson::Null;
        job.owner_offset = 0;
    }
    Ok(pages)
}

async fn renew(db: &mongodb::Database, job: &Job) -> AppResult<()> {
    let result = db.collection::<Document>(JOBS).update_one(job.fence(), vec![doc! {"$set": {
        "lease_until": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": LEASE_SECONDS}}
    }}]).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("Orphan sweep lease expired".into()));
    }
    Ok(())
}

fn specs_digest() -> AppResult<String> {
    let values = specs()
        .into_iter()
        .map(
            |spec| doc! {"collection": spec.collection, "field": spec.field, "parent": spec.parent},
        )
        .collect::<Vec<_>>();
    let encoded = bson::to_vec(&doc! {"specs": values})
        .map_err(|_| AppError::Internal("Orphan specification encoding failed".into()))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

async fn summarize(db: &mongodb::Database, job: &mut Job) -> AppResult<()> {
    let mut cursor = db
        .collection::<CandidatePage>(PAGES)
        .find(page_range(&job.id))
        .sort(doc! {"_id": 1})
        .batch_size(1)
        .max_time(AGGREGATE_TIMEOUT)
        .await?;
    let mut hash = Sha256::new();
    let mut reports = BTreeMap::<String, OrphanCollectionReport>::new();
    let mut count = 0;
    let mut pages = 0;
    while let Some(page) = cursor.try_next().await? {
        if pages % 128 == 0 {
            renew(db, job).await?;
        }
        pages += 1;
        let report =
            reports
                .entry(page.collection.clone())
                .or_insert_with(|| OrphanCollectionReport {
                    collection: page.collection.clone(),
                    candidate_count: 0,
                    owner_ids: Vec::new(),
                    owner_ids_truncated: false,
                });
        for row in page.rows {
            let bytes = bson::to_vec(
                &doc! {"collection": &page.collection, "field": &page.field, "parent": &page.parent,
                "row_id": row.row_id, "owner_id": &row.owner_id},
            )
            .map_err(|_| AppError::Internal("Orphan candidate encoding failed".into()))?;
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
            count += 1;
            report.candidate_count += 1;
            if !report.owner_ids.contains(&row.owner_id) {
                if report.owner_ids.len() < OWNER_ID_REPORT_CAP {
                    report.owner_ids.push(row.owner_id);
                } else {
                    report.owner_ids_truncated = true;
                }
            }
        }
    }
    let digest = hex::encode(hash.finalize());
    job.report = Some(OrphanSweepReport {
        dry_run: !job.apply,
        digest: digest.clone(),
        collections: reports.into_values().collect(),
        candidate_count: count,
    });
    job.status = "complete".into();
    job.phase = "complete".into();
    Ok(())
}

async fn audit_apply(db: &mongodb::Database, job: &Job, started: bool) -> AppResult<()> {
    let marker = if started {
        "apply_started"
    } else {
        "apply_completed"
    };
    let event = if started {
        "user.account.erasure_orphan_sweep_started"
    } else {
        "user.account.erasure_orphan_sweep"
    };
    let report = job
        .report
        .as_ref()
        .ok_or_else(|| AppError::Internal("Orphan sweep has no report".into()))?;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        let mut filter = job.fence();
        filter.insert(marker, doc! {"$ne": true});
        if db.collection::<Document>(JOBS).update_one(filter, doc! {"$set": {marker: true}}).session(&mut tx).await?.matched_count == 0 {
            if db.collection::<Document>(JOBS).find_one(job.fence()).session(&mut tx).await?.is_none() {
                return Err(AppError::Conflict("Orphan sweep lease expired".into()));
            }
            return Ok(());
        }
        let key = super::super::audit_service::audit_chain_hmac_key().ok_or_else(|| AppError::Internal("Audit key unavailable".into()))?;
        super::super::audit_service::log_actor_event_in_session(db, &mut tx, key,
            &super::super::audit_service::AuditActor {user_id: job.requested_by.clone(), ip_address: None, user_agent: None, api_key_id: None, api_key_name: None},
            event, serde_json::json!({"job_id":job.id, "digest":report.digest, "candidate_count":report.candidate_count,
                "collections":report.collections.iter().map(|row| serde_json::json!({"collection":row.collection, "candidate_count":row.candidate_count})).collect::<Vec<_>>()})
        ).await?;
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

async fn apply_page(db: &mongodb::Database, job: &mut Job) -> AppResult<()> {
    audit_apply(db, job, true).await?;
    if job.apply_page >= job.next_page {
        audit_apply(db, job, false).await?;
        job.status = "complete".into();
        job.phase = "complete".into();
        return Ok(());
    }
    let page = db
        .collection::<CandidatePage>(PAGES)
        .find_one(
            doc! {"_id": page_id(job.source_job_id.as_deref().unwrap_or(&job.id), job.apply_page)},
        )
        .await?
        .ok_or_else(|| AppError::Conflict("Orphan candidate page is unavailable".into()))?;
    let entry = personal_data_registry::find(&page.collection)
        .ok_or_else(|| AppError::Conflict("Orphan collection classification changed".into()))?;
    let spec = specs()
        .into_iter()
        .find(|spec| {
            spec.collection == page.collection
                && spec.field == page.field
                && spec.parent == page.parent.as_deref()
        })
        .ok_or_else(|| AppError::Conflict("Orphan ownership classification changed".into()))?;
    let live = live_ids(db, spec, &page.rows).await?;
    for row in page.rows {
        if live.contains(&row.owner_id) {
            job.noop_count += 1;
            continue;
        }
        if db
            .collection::<Document>(JOBS)
            .find_one(job.fence())
            .projection(doc! {"_id": 1})
            .await?
            .is_none()
        {
            return Err(AppError::Conflict("Orphan sweep lease expired".into()));
        }
        let present = apply_group(
            db,
            &OrphanGroup {
                collection: entry.collection,
                field: spec.field,
                owner_id: row.owner_id,
                row_ids: vec![row.row_id],
                disposition: entry.erasure,
                joined_parent: spec.parent,
            },
            &job.fence(),
        )
        .await?;
        if !present {
            job.noop_count += 1;
        }
    }
    job.apply_page += 1;
    Ok(())
}

/// Finished reports/pages remain reviewable for at least seven days. Delete at
/// most 128 pages per pass, and never purge a dry-run referenced by pending apply.
async fn purge_finished(db: &mongodb::Database) -> AppResult<()> {
    let cutoff =
        bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS));
    let mut jobs = db
        .collection::<Job>(JOBS)
        .find(doc! {"status":{"$in":["complete","failed"]},"updated_at":{"$lt":cutoff}})
        .limit(16)
        .batch_size(16)
        .max_time(AGGREGATE_TIMEOUT)
        .await?;
    while let Some(job) = jobs.try_next().await? {
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::majority())
            .await?;
        let result: AppResult<()> = async {
            // Compete with apply admission through the source-job document.
            if db
                .collection::<Document>(JOBS)
                .update_one(
                    doc! {"_id":&job.id,"updated_at":{"$lt":cutoff}},
                    doc! {"$inc":{"purge_revision":1}},
                )
                .session(&mut tx)
                .await?
                .matched_count
                == 0
            {
                return Ok(());
            }
            if db
                .collection::<Document>(JOBS)
                .find_one(doc! {"source_job_id":&job.id,"status":"pending"})
                .session(&mut tx)
                .await?
                .is_some()
            {
                return Ok(());
            }
            let mut cursor = db
                .collection::<Document>(PAGES)
                .find(page_range(&job.id))
                .projection(doc! {"_id":1})
                .limit(128)
                .session(&mut tx)
                .await?;
            let mut ids = Vec::new();
            while let Some(row) = cursor.next(&mut tx).await.transpose()? {
                ids.push(row.get("_id").cloned().unwrap_or(Bson::Null));
            }
            if !ids.is_empty() {
                db.collection::<Document>(PAGES)
                    .delete_many(doc! {"_id":{"$in":ids}})
                    .session(&mut tx)
                    .await?;
            }
            if db
                .collection::<Document>(PAGES)
                .find_one(page_range(&job.id))
                .session(&mut tx)
                .await?
                .is_none()
            {
                db.collection::<Document>(JOBS)
                    .delete_one(doc! {"_id":&job.id})
                    .session(&mut tx)
                    .await?;
            }
            Ok(())
        }
        .await;
        super::super::org_membership_authority::finish(tx, result).await?;
    }
    Ok(())
}

pub async fn run_once(db: &mongodb::Database) -> AppResult<bool> {
    purge_finished(db).await?;
    let Some(mut job) = claim(db).await? else {
        return Ok(false);
    };
    // Jobs accepted by an older writer cannot authorize a fresh live scan.
    if job.apply && job.source_job_id.is_none() {
        if let Some(source) = db
            .collection::<Job>(JOBS)
            .find_one(doc! {"apply":false,"status":"complete","report.digest":&job.expected_digest})
            .await?
        {
            job.source_job_id = Some(source.id);
            job.specs_digest = source.specs_digest;
            job.next_page = source.next_page;
            job.apply_page = 0;
            job.phase = "apply".into();
            job.report = source.report;
            if let Some(report) = &mut job.report {
                report.dry_run = false;
            }
        } else {
            job.status = "failed".into();
            job.failure = Some("reviewed_pages_unavailable".into());
        }
        save(db, &job, Vec::new()).await?;
        return Ok(true);
    }
    if job.specs_digest != specs_digest()? {
        job.status = "failed".into();
        job.failure = Some("ownership_specification_changed".into());
        save(db, &job, Vec::new()).await?;
        return Ok(true);
    }
    let result: AppResult<Vec<CandidatePage>> = async {
        match job.phase.as_str() {
            "scan" => scan_page(db, &mut job).await,
            "summarize" => {
                summarize(db, &mut job).await?;
                Ok(Vec::new())
            }
            "apply" => {
                apply_page(db, &mut job).await?;
                Ok(Vec::new())
            }
            _ => Err(AppError::Conflict(
                "Orphan sweep phase is unsupported".into(),
            )),
        }
    }
    .await;
    let pages = match result {
        Ok(pages) => {
            job.consecutive_failures = 0;
            pages
        }
        Err(_) => {
            // Don't checkpoint a partially advanced cursor after a failed page.
            let failures = job.consecutive_failures.saturating_add(1);
            job = db
                .collection::<Job>(JOBS)
                .find_one(job.fence())
                .await?
                .ok_or_else(|| AppError::Conflict("Orphan sweep lease expired".into()))?;
            job.consecutive_failures = failures;
            if failures >= RETRY_CAP {
                job.status = "failed".into();
                job.failure = Some("page_retry_limit_exceeded".into());
            }
            Vec::new()
        }
    };
    save(db, &job, pages).await?;
    Ok(true)
}

#[cfg(test)]
pub async fn finish(db: &mongodb::Database, id: &str) -> AppResult<OrphanSweepReport> {
    for _ in 0..100_000 {
        let status = status(db, id).await?;
        match status.status.as_str() {
            "complete" => {
                return status
                    .report
                    .ok_or_else(|| AppError::Internal("Orphan report missing".into()));
            }
            "failed" => return Err(AppError::Conflict(status.failure.unwrap_or_default())),
            _ => {
                if !run_once(db).await? {
                    return Err(AppError::Conflict("Orphan worker is leased".into()));
                }
            }
        }
    }
    Err(AppError::Internal("Orphan worker did not converge".into()))
}

/// Shutdown-aware worker; claim/checkpoint follow personal_erasure_service::worker.
pub fn spawn_worker(state: crate::AppState) -> tokio::task::JoinHandle<()> {
    state.shutdown.clone().spawn_worker(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let Some(_work) = state.shutdown.tick(&mut timer).await else {
                break;
            };
            for _ in 0..8 {
                match run_once(&state.db).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => {
                        tracing::warn!(
                            "Orphan sweep pass remains pending; durable lease will retry"
                        );
                        break;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_consumes_reviewed_pages_and_counts_vanished_candidates() {
        let db =
            crate::test_utils::connect_transaction_test_database("orphan_reviewed_pages").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let owner = Uuid::new_v4().to_string();
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(doc! {"_id":"expired","user_id":&owner})
            .await
            .unwrap();
        let dry = scan(&db).await.unwrap();
        db.collection::<Document>("assistant_wire_logs")
            .delete_one(doc! {"_id":"expired"})
            .await
            .unwrap();
        // New orphans were never reviewed and must not enter this apply.
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(doc! {"_id":"new","user_id":&owner})
            .await
            .unwrap();
        let apply = start(&db, "operator", true, Some(&dry.digest))
            .await
            .unwrap();
        let report = finish(&db, &apply.job_id).await.unwrap();
        assert_eq!(report.candidate_count, 1);
        assert_eq!(status(&db, &apply.job_id).await.unwrap().noop_count, 1);
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id":"new"})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn repeated_digest_retries_rows_after_remote_cleanup_clears() {
        let db =
            crate::test_utils::connect_transaction_test_database("orphan_blocked_reapply").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let owner = Uuid::new_v4().to_string();
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(doc! {"_id":"held", "user_id":&owner})
            .await
            .unwrap();
        // Remote cleanup is lifecycle state, so it does not alter the reviewed
        // local-inventory digest when the blocker clears.
        db.collection::<Document>("platform_bot_cleanup")
            .insert_one(doc! {"_id":"remote", "owner_user_id":&owner, "completed":false})
            .await
            .unwrap();
        let dry = scan(&db).await.unwrap();
        let first = start(&db, "operator", true, Some(&dry.digest))
            .await
            .unwrap();
        finish(&db, &first.job_id).await.unwrap();
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id":"held"})
                .await
                .unwrap()
                .is_some()
        );
        assert!(db.collection::<Document>("audit_log").find_one(doc! {"event_type":"user.account.erasure_orphan_sweep", "event_data.digest":&dry.digest}).await.unwrap().is_some());
        db.collection::<Document>("platform_bot_cleanup")
            .update_one(doc! {"_id":"remote"}, doc! {"$set":{"completed":true}})
            .await
            .unwrap();
        let next = scan(&db).await.unwrap();
        assert_eq!(next.digest, dry.digest);
        let second = start(&db, "operator", true, Some(&next.digest))
            .await
            .unwrap();
        assert_eq!(second.status, "pending");
        assert_eq!(
            finish(&db, &second.job_id).await.unwrap().candidate_count,
            1
        );
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id":"held"})
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn persistent_page_errors_fail_after_the_cap_and_finished_pages_are_purged() {
        let db = crate::test_utils::connect_transaction_test_database("orphan_retry_purge").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(doc! {"_id":"row","user_id":Uuid::new_v4().to_string()})
            .await
            .unwrap();
        let dry = start(&db, "operator", false, None).await.unwrap();
        let report = finish(&db, &dry.job_id).await.unwrap();
        let apply = start(&db, "operator", true, Some(&report.digest))
            .await
            .unwrap();
        let reviewed_page = db
            .collection::<CandidatePage>(PAGES)
            .find_one(page_range(&dry.job_id))
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>(PAGES)
            .delete_many(page_range(&dry.job_id))
            .await
            .unwrap();
        for _ in 0..RETRY_CAP {
            assert!(run_once(&db).await.unwrap());
        }
        let failed = status(&db, &apply.job_id).await.unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.failure.as_deref(), Some("page_retry_limit_exceeded"));
        // Restore the saved identifier page so purge must actually remove data,
        // rather than merely observing the already-empty page collection.
        db.collection::<CandidatePage>(PAGES)
            .insert_one(reviewed_page)
            .await
            .unwrap();
        assert_eq!(
            db.collection::<Document>(PAGES)
                .count_documents(page_range(&dry.job_id))
                .await
                .unwrap(),
            1
        );
        let aged = bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::days(8));
        db.collection::<Document>(JOBS)
            .update_many(doc! {}, doc! {"$set":{"updated_at":aged}})
            .await
            .unwrap();
        purge_finished(&db).await.unwrap();
        assert_eq!(
            db.collection::<Document>(JOBS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(PAGES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn dollar_prefixed_ids_and_checkpoint_values_remain_literal() {
        let db = crate::test_utils::connect_transaction_test_database("orphan_literal").await;
        let owner = Uuid::new_v4().to_string();
        db.collection::<Document>("assistant_wire_logs")
            .insert_many([
                doc! {"_id": "$first", "user_id": &owner},
                doc! {"_id": "$second", "user_id": &owner},
            ])
            .await
            .unwrap();
        start(&db, "$operator", false, None).await.unwrap();
        let mut job = claim(&db).await.unwrap().unwrap();
        job.scan_spec = specs()
            .iter()
            .position(|s| s.collection == "assistant_wire_logs")
            .unwrap();
        job.last_id = "$first".into();
        job.failure = Some("$checkpoint".into());
        let pages = scan_page(&db, &mut job).await.unwrap();
        assert_eq!(
            pages
                .iter()
                .flat_map(|p| &p.rows)
                .map(|r| r.row_id.clone())
                .collect::<Vec<_>>(),
            vec![Bson::from("$second")]
        );
        save(&db, &job, pages).await.unwrap();
        let stored = db
            .collection::<Job>(JOBS)
            .find_one(doc! {"_id": &job.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.requested_by, "$operator");
        assert_eq!(stored.failure.as_deref(), Some("$checkpoint"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn checkpoints_resume_and_expired_claims_cannot_commit() {
        let db = crate::test_utils::connect_transaction_test_database("orphan_job_resume").await;
        let owner = Uuid::new_v4().to_string();
        db.collection::<Document>("assistant_wire_logs")
            .insert_many((0..600).map(|n| {
                doc! {
                    "_id": format!("row-{n:04}"), "user_id": &owner
                }
            }))
            .await
            .unwrap();
        let started = start(&db, "operator", false, None).await.unwrap();
        let mut stale = claim(&db).await.unwrap().unwrap();
        let pages = scan_page(&db, &mut stale).await.unwrap();
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &stale.id},
                doc! {"$set": {"lease_until": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(matches!(
            save(&db, &stale, pages).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<Document>(PAGES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        // A successor resumes the durable cursor, rather than trusting the
        // abandoned process's in-memory rows or lease token.
        let report = finish(&db, &started.job_id).await.unwrap();
        assert_eq!(report.candidate_count, 600);
        assert_eq!(scan(&db).await.unwrap().digest, report.digest);
        assert!(matches!(
            apply(&db, "operator", "wrong").await,
            Err(AppError::Conflict(_))
        ));
        apply(&db, "operator", &report.digest).await.unwrap();
        assert_eq!(
            db.collection::<Document>("assistant_wire_logs")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            apply(&db, "operator", &report.digest)
                .await
                .unwrap()
                .candidate_count,
            600
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn scan_streams_four_hundred_thousand_owners_beyond_one_bson_query() {
        let db = crate::test_utils::connect_transaction_test_database("orphan_job_scale").await;
        // 400,000 UUIDs exceed MongoDB's 16 MiB command limit. No such
        // command is sent: this models the old implementation's query only.
        let mut legacy = Vec::new();
        for base in (0..400_000_u128).step_by(1000) {
            let mut rows = Vec::new();
            let mut people = Vec::new();
            for n in base..base + 1000 {
                let owner = Uuid::from_u128(n + 1).to_string();
                legacy.push(owner.clone());
                rows.push(doc! {"_id": format!("row-{n:06}"), "user_id": &owner});
                // Both live-heavy and orphan-heavy pages occur in the same
                // scan. Retained reports cap IDs without losing exact counts.
                if n < 200_000 {
                    people.push(doc! {"_id": &owner});
                }
            }
            db.collection::<Document>("assistant_wire_logs")
                .insert_many(rows)
                .await
                .unwrap();
            if !people.is_empty() {
                db.collection::<Document>("users")
                    .insert_many(people)
                    .await
                    .unwrap();
            }
        }
        assert!(bson::to_vec(&doc! {"_id": {"$in": &legacy}}).unwrap().len() > 16 * 1024 * 1024);
        drop(legacy);
        let started = start(&db, "operator", false, None).await.unwrap();
        let report = finish(&db, &started.job_id).await.unwrap();
        assert_eq!(report.candidate_count, 200_000);
        let rows = report
            .collections
            .iter()
            .find(|row| row.collection == "assistant_wire_logs")
            .unwrap();
        assert_eq!(rows.owner_ids.len(), OWNER_ID_REPORT_CAP);
        assert!(rows.owner_ids_truncated);
        let mut pages = db
            .collection::<CandidatePage>(PAGES)
            .find(page_range(&started.job_id))
            .batch_size(1)
            .await
            .unwrap();
        let mut count = 0;
        while let Some(page) = pages.try_next().await.unwrap() {
            assert!(page.rows.len() <= APPLY_BATCH);
            count += page.rows.len();
            for row in page.rows {
                assert!(Uuid::parse_str(&row.owner_id).unwrap().as_u128() > 200_000);
            }
        }
        assert_eq!(count, 200_000);
        const { assert!(SCAN_VALUES <= 512 && LIVE_BATCH <= 128) };
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn mutation_transactions_recheck_liveness_parent_guards_lease_and_audit_order() {
        let db =
            crate::test_utils::connect_transaction_test_database("orphan_mutation_fences").await;
        let dry = scan(&db).await.unwrap();
        let started = start(&db, "operator", true, Some(&dry.digest))
            .await
            .unwrap();
        let mut job = claim(&db).await.unwrap().unwrap();
        job.report = Some(OrphanSweepReport {
            dry_run: false,
            digest: "reviewed-digest".into(),
            collections: Vec::new(),
            candidate_count: 1,
        });
        audit_apply(&db, &job, true).await.unwrap();
        audit_apply(&db, &job, true).await.unwrap();
        let event = db
            .collection::<Document>("audit_log")
            .find_one(doc! {"event_type":"user.account.erasure_orphan_sweep_started"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event.get_document("event_data").unwrap().get_str("digest"),
            Ok("reviewed-digest")
        );
        assert_eq!(
            db.collection::<Document>("audit_log")
                .count_documents(doc! {"event_type":"user.account.erasure_orphan_sweep_started"})
                .await
                .unwrap(),
            1
        );
        for live_collection in ["users", "service_accounts", "personal_erasure_jobs"] {
            let owner = Uuid::new_v4().to_string();
            let row = doc! {"_id":format!("held-{live_collection}"),"user_id":&owner};
            db.collection::<Document>("assistant_wire_logs")
                .insert_one(row.clone())
                .await
                .unwrap();
            let mut live = doc! {"_id": &owner};
            if live_collection == "personal_erasure_jobs" {
                live.insert("user_id", &owner);
            }
            db.collection::<Document>(live_collection)
                .insert_one(live)
                .await
                .unwrap();
            let group = OrphanGroup {
                collection: "assistant_wire_logs",
                field: "user_id",
                owner_id: owner,
                row_ids: vec![row.get("_id").unwrap().clone()],
                disposition: Erasure::LocalInventory,
                joined_parent: None,
            };
            apply_group(&db, &group, &job.fence()).await.unwrap();
            assert_eq!(
                db.collection::<Document>("assistant_wire_logs")
                    .find_one(doc! {"_id":row.get("_id").unwrap()})
                    .await
                    .unwrap(),
                Some(row)
            );
        }
        let parent = Uuid::new_v4().to_string();
        let child = doc! {"_id":"joined-child","installation_id":&parent};
        db.collection::<Document>("platform_delivery_tests")
            .insert_one(child.clone())
            .await
            .unwrap();
        db.collection::<Document>("platform_bot_installations")
            .insert_one(doc! {"_id":&parent})
            .await
            .unwrap();
        let joined = OrphanGroup {
            collection: "platform_delivery_tests",
            field: "installation_id",
            owner_id: parent,
            row_ids: vec!["joined-child".into()],
            disposition: Erasure::JoinedChild {
                parent: "platform_bot_installations",
            },
            joined_parent: Some("platform_bot_installations"),
        };
        apply_group(&db, &joined, &job.fence()).await.unwrap();
        assert_eq!(
            db.collection::<Document>("platform_delivery_tests")
                .find_one(doc! {"_id":"joined-child"})
                .await
                .unwrap(),
            Some(child)
        );
        let owner = Uuid::new_v4().to_string();
        let held =
            doc! {"_id":"claimed-link","user_id":&owner,"completion_claim_id":"external-effect"};
        db.collection::<Document>("connect_links")
            .insert_one(held.clone())
            .await
            .unwrap();
        apply_group(
            &db,
            &OrphanGroup {
                collection: "connect_links",
                field: "user_id",
                owner_id: owner.clone(),
                row_ids: vec!["claimed-link".into()],
                disposition: Erasure::LocalInventory,
                joined_parent: None,
            },
            &job.fence(),
        )
        .await
        .unwrap();
        assert_eq!(
            db.collection::<Document>("connect_links")
                .find_one(doc! {"_id":"claimed-link"})
                .await
                .unwrap(),
            Some(held)
        );
        let row = doc! {"_id":"removable","user_id":&owner};
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(row.clone())
            .await
            .unwrap();
        let group = OrphanGroup {
            collection: "assistant_wire_logs",
            field: "user_id",
            owner_id: owner,
            row_ids: vec!["removable".into()],
            disposition: Erasure::LocalInventory,
            joined_parent: None,
        };
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id":&started.job_id},
                doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(matches!(
            apply_group(&db, &group, &job.fence()).await,
            Err(AppError::Conflict(_))
        ));
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id":"removable"})
                .await
                .unwrap()
                .is_some()
        );
        let mut successor = claim(&db).await.unwrap().unwrap();
        successor.report = job.report;
        apply_group(&db, &group, &successor.fence()).await.unwrap();
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id":"removable"})
                .await
                .unwrap()
                .is_none()
        );
        audit_apply(&db, &successor, false).await.unwrap();
        let completion = db
            .collection::<Document>("audit_log")
            .find_one(doc! {"event_type":"user.account.erasure_orphan_sweep"})
            .await
            .unwrap()
            .unwrap();
        assert!(event.get_i64("seq").unwrap() < completion.get_i64("seq").unwrap());
        db.drop().await.unwrap();
    }
}
