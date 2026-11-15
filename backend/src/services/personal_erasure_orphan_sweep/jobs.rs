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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepStatus {
    pub job_id: String,
    pub status: String,
    pub phase: String,
    pub report: Option<OrphanSweepReport>,
    pub failure: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Job {
    #[serde(rename = "_id")]
    id: String,
    requested_by: String,
    apply: bool,
    expected_digest: Option<String>,
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
    let job = Job {
        id: Uuid::new_v4().to_string(),
        requested_by: actor.into(),
        apply,
        expected_digest: expected.map(str::to_owned),
        status: "pending".into(),
        phase: "scan".into(),
        scan_spec: 0,
        specs_digest: specs_digest()?,
        last_id: Bson::Null,
        owner_offset: 0,
        next_page: 0,
        apply_page: 0,
        report: None,
        failure: None,
        lease_token: None,
        lease_until: None,
        created_at: bson::DateTime::now(),
        updated_at: bson::DateTime::now(),
    };
    db.collection::<Job>(JOBS).insert_one(&job).await?;
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
                vec![
                    doc! {"$set": stored},
                    doc! {"$set": {"updated_at": "$$NOW"}},
                ],
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
        filter.insert(
            "$expr",
            doc! {comparison: [Bson::String("$_id".into()), job.last_id.clone()]},
        );
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
    if !job.apply {
        job.status = "complete".into();
        job.phase = "complete".into();
    } else if job.expected_digest.as_deref() == Some(&digest) {
        job.phase = "apply".into();
    } else if count == 0
        && was_applied(db, job.expected_digest.as_deref().unwrap_or_default()).await?
    {
        job.report.as_mut().unwrap().digest = job.expected_digest.clone().unwrap_or_default();
        job.status = "complete".into();
        job.phase = "complete".into();
    } else {
        job.status = "failed".into();
        job.failure = Some("candidate_set_changed".into());
    }
    Ok(())
}

async fn apply_page(db: &mongodb::Database, job: &mut Job) -> AppResult<()> {
    if job.apply_page >= job.next_page {
        let report = job
            .report
            .as_ref()
            .ok_or_else(|| AppError::Internal("Orphan sweep has no report".into()))?;
        super::super::audit_service::log_actor_event(db.clone(), &super::super::audit_service::AuditActor {
            user_id: job.requested_by.clone(), ip_address: None, user_agent: None, api_key_id: None, api_key_name: None,
        }, "user.account.erasure_orphan_sweep", Some(serde_json::json!({"digest": report.digest, "candidate_count": report.candidate_count,
            "collections": report.collections.iter().map(|row| serde_json::json!({"collection": row.collection, "candidate_count": row.candidate_count})).collect::<Vec<_>>() })))
            .await.map_err(|_| AppError::Internal("Orphan sweep audit append failed".into()))?;
        job.status = "complete".into();
        job.phase = "complete".into();
        return Ok(());
    }
    let page = db
        .collection::<CandidatePage>(PAGES)
        .find_one(doc! {"_id": page_id(&job.id, job.apply_page)})
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
        apply_group(
            db,
            &OrphanGroup {
                collection: entry.collection,
                field: spec.field,
                owner_id: row.owner_id,
                row_ids: vec![row.row_id],
                disposition: entry.erasure,
                joined_parent: spec.parent,
            },
        )
        .await?;
    }
    job.apply_page += 1;
    Ok(())
}

pub async fn run_once(db: &mongodb::Database) -> AppResult<bool> {
    let Some(mut job) = claim(db).await? else {
        return Ok(false);
    };
    if job.specs_digest != specs_digest()? {
        job.status = "failed".into();
        job.failure = Some("ownership_specification_changed".into());
        save(db, &job, Vec::new()).await?;
        return Ok(true);
    }
    let pages = match job.phase.as_str() {
        "scan" => scan_page(db, &mut job).await?,
        "summarize" => {
            summarize(db, &mut job).await?;
            Vec::new()
        }
        "apply" => {
            apply_page(db, &mut job).await?;
            Vec::new()
        }
        _ => {
            return Err(AppError::Conflict(
                "Orphan sweep phase is unsupported".into(),
            ));
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
            0
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
}
