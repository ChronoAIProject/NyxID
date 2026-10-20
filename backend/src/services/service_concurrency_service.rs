//! Per-catalog, per-acting-person concurrency. No policy means no database work.
use std::{future::Future, sync::Arc, time::Duration};

use axum::{body::Body, response::Response};
use futures::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use tokio_util::sync::CancellationToken;

use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::DownstreamService,
        service_concurrency::{COLLECTION_NAME, ServiceConcurrencyPolicy},
    },
};

const TTL_MS: i64 = 60_000;
const RENEW_INTERVAL: Duration = Duration::from_secs(15);
const RENEW_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_TARGETS: usize = 500;
pub const MAX_LIMIT: u32 = 10_000;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let rows = db.collection::<Document>(COLLECTION_NAME);
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"service_id":1,"person_id":1})
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"expires_at":1})
            .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
            .build(),
    )
    .await?;
    db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"member_user_id":1,"revoked_at":1,"org_user_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

pub fn validate(policy: &ServiceConcurrencyPolicy) -> AppResult<()> {
    if policy.users.len() + policy.orgs.len() > MAX_TARGETS {
        return Err(AppError::ValidationError(
            "At most 500 concurrency override targets are allowed".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for row in policy.users.iter().chain(&policy.orgs) {
        if uuid::Uuid::parse_str(&row.id).is_err() || !seen.insert(&row.id) {
            return Err(AppError::ValidationError(
                "Concurrency targets must be unique UUIDs".into(),
            ));
        }
    }
    if std::iter::once(policy.default_limit)
        .chain(policy.users.iter().chain(&policy.orgs).map(|v| v.limit))
        .flatten()
        .any(|v| !(1..=MAX_LIMIT).contains(&v))
    {
        return Err(AppError::ValidationError(
            "Concurrency limits must be 1–10000, or null for unlimited".into(),
        ));
    }
    Ok(())
}

pub async fn validate_targets(db: &Database, policy: &ServiceConcurrencyPolicy) -> AppResult<()> {
    validate(policy)?;
    // Exact type and existence validation, batched and bounded. Service accounts
    // can be targeted by ID through the API, and otherwise receive the default.
    for (targets, kind) in [(&policy.users, "person"), (&policy.orgs, "org")] {
        if targets.is_empty() {
            continue;
        }
        let ids: Vec<_> = targets.iter().map(|v| v.id.as_str()).collect();
        let mut filter = doc! {"_id":{"$in":&ids},"user_type":kind};
        if kind == "person" {
            filter.remove("user_type");
            filter.insert(
                "$or",
                vec![
                    doc! {"user_type":"person"},
                    doc! {"user_type":{"$exists":false}},
                ],
            );
        }
        let mut found = db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .count_documents(filter)
            .await?;
        if kind == "person" {
            found += db
                .collection::<Document>(crate::models::service_account::COLLECTION_NAME)
                .count_documents(doc! {"_id":{"$in":&ids}})
                .await?;
        }
        if found != ids.len() as u64 {
            return Err(AppError::ValidationError(format!(
                "Unknown or incorrectly typed {kind} concurrency target"
            )));
        }
    }
    Ok(())
}

pub async fn set_policy(
    db: &Database,
    service_id: &str,
    policy: Option<&ServiceConcurrencyPolicy>,
) -> AppResult<()> {
    uuid::Uuid::parse_str(service_id)
        .map_err(|_| AppError::ValidationError("Invalid service ID".into()))?;
    if let Some(policy) = policy {
        validate_targets(db, policy).await?;
    }
    let encoded = bson::to_bson(&policy).map_err(|e| AppError::Internal(e.to_string()))?;
    let result = db
        .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":service_id},
            doc! {"$set":{"concurrency_policy":encoded,"updated_at":bson::DateTime::now()}},
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("Service not found".into()));
    }
    Ok(())
}

pub async fn effective_limit(
    db: &Database,
    policy: &ServiceConcurrencyPolicy,
    person: &str,
) -> AppResult<Option<u32>> {
    if let Some(row) = policy.users.iter().find(|v| v.id == person) {
        return Ok(row.limit);
    }
    if policy.orgs.is_empty() {
        return Ok(policy.default_limit);
    }
    let ids: Vec<_> = policy.orgs.iter().map(|v| v.id.as_str()).collect();
    let memberships: Vec<Document> = db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .find(
            doc! {"member_user_id":person,"revoked_at":bson::Bson::Null,"org_user_id":{"$in":ids}},
        )
        .projection(doc! {"org_user_id":1})
        .limit(MAX_TARGETS as i64)
        .await?
        .try_collect()
        .await?;
    let ids: Vec<_> = memberships
        .iter()
        .filter_map(|v| v.get_str("org_user_id").ok())
        .collect();
    if ids.is_empty() {
        return Ok(policy.default_limit);
    }
    let active: Vec<Document> = db
        .collection(crate::models::user::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids},"user_type":"org","is_active":true})
        .projection(doc! {"_id":1})
        .limit(MAX_TARGETS as i64)
        .await?
        .try_collect()
        .await?;
    let mut result = None;
    for row in &policy.orgs {
        if active
            .iter()
            .any(|org| org.get_str("_id").ok() == Some(row.id.as_str()))
        {
            let Some(limit) = row.limit else {
                return Ok(None);
            };
            result = Some(result.unwrap_or(0).max(limit));
        }
    }
    Ok(result.or(policy.default_limit))
}

pub async fn acquire(
    db: &Database,
    service: &DownstreamService,
    person: &str,
) -> AppResult<Option<Lease>> {
    acquire_policy(db, service.concurrency_policy.as_ref(), person).await
}

pub async fn acquire_policy(
    db: &Database,
    policy: Option<&ServiceConcurrencyPolicy>,
    person: &str,
) -> AppResult<Option<Lease>> {
    let Some(policy) = policy else {
        return Ok(None);
    };
    let Some(limit) = effective_limit(db, policy, person).await? else {
        return Ok(None);
    };
    claim(db, &policy.service_id, person, limit).await.map(Some)
}

fn scope(service: &str, person: &str) -> Document {
    doc! {"service_id":service,"person_id":person}
}
fn expiry() -> Document {
    doc! {"$add":["$$NOW",TTL_MS]}
}

fn claim<'a>(
    db: &'a Database,
    service: &'a str,
    person: &'a str,
    limit: u32,
) -> futures::future::BoxFuture<'a, AppResult<Lease>> {
    let (db, service, person) = (db.clone(), service.to_owned(), person.to_owned());
    Box::pin(async move {
        // A disconnect during commit must not discard a committed claim before
        // it gets a guard. The owned task finishes admission and drops its result
        // (releasing the lease) even if its awaiting request has disappeared.
        tokio::spawn(async move { claim_inner(&db, &service, &person, limit).await })
            .await
            .map_err(|_| AppError::Internal("Concurrency claim task failed".into()))?
    })
}

async fn claim_inner(db: &Database, service: &str, person: &str, limit: u32) -> AppResult<Lease> {
    let lease_id = uuid::Uuid::new_v4().to_string();
    let filter = scope(service, person);
    // A real write on the common scope document fences every claim, including
    // rejected claims. The transaction cannot commit a stale snapshot count.
    // Pruning uses server time; TTL deletion is only storage cleanup.
    let pipeline = vec![
        doc! {"$set":{
            "_id":{"$ifNull":["$_id",uuid::Uuid::new_v4().to_string()]},
            "revision":{"$add":[{"$ifNull":["$revision",0]},1]},
            "leases":{"$filter":{"input":{"$ifNull":["$leases",[]]},"as":"slot","cond":{"$gt":["$$slot.expires_at","$$NOW"]}}},
            "expires_at":expiry(),
        }},
        doc! {"$set":{"leases":{"$cond":[{"$lt":[{"$size":"$leases"},i64::from(limit)]},
        {"$concatArrays":["$leases",[{"id":&lease_id,"expires_at":expiry()}]]},"$leases"]}}},
    ];
    let mut tx = db.client().start_session().await?;
    for attempt in 0..4 {
        let claim_db = db.clone();
        let claim_filter = filter.clone();
        let claim_pipeline = pipeline.clone();
        let result = tx
            .start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .and_run2(async move |tx| {
                claim_db
                    .collection::<Document>(COLLECTION_NAME)
                    .find_one_and_update(claim_filter.clone(), claim_pipeline.clone())
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .session(tx)
                    .await
            })
            .await;
        match result {
            Ok(Some(row)) => {
                if !row.get_array("leases").is_ok_and(|rows| {
                    rows.iter().any(|v| {
                        v.as_document().and_then(|v| v.get_str("id").ok()) == Some(&lease_id)
                    })
                }) {
                    return Err(AppError::ServiceConcurrencyLimited);
                }
                let lease = Lease::new(db.clone(), filter, lease_id);
                // A delayed commit acknowledgement must not admit work under an
                // already-expired lease. Confirm ownership before dispatch.
                if !matches!(
                    tokio::time::timeout(
                        RENEW_TIMEOUT,
                        renew(db, &lease.0.filter, &lease.0.lease_id)
                    )
                    .await,
                    Ok(Ok(true))
                ) {
                    return Err(AppError::ServiceConcurrencyLimited);
                }
                return Ok(lease);
            }
            Err(e) if attempt < 3 && super::assistant_team_service::is_duplicate(&e) => continue,
            Err(e) => return Err(e.into()),
            Ok(None) => {
                return Err(AppError::Internal(
                    "Concurrency claim returned no document".into(),
                ));
            }
        }
    }
    unreachable!()
}

async fn renew(db: &Database, filter: &Document, lease_id: &str) -> AppResult<bool> {
    let mut filter = filter.clone();
    filter.insert("$expr", doc! {"$gt":[{"$size":{"$filter":{"input":"$leases","as":"slot","cond":{"$and":[{"$eq":["$$slot.id",lease_id]},{"$gt":["$$slot.expires_at","$$NOW"]}]}}}},0]});
    let result = db.collection::<Document>(COLLECTION_NAME).update_one(filter, vec![doc! {"$set":{
        "expires_at":expiry(),
        "leases":{"$map":{"input":"$leases","as":"slot","in":{"$cond":[{"$eq":["$$slot.id",lease_id]},{"id":lease_id,"expires_at":expiry()},"$$slot"]}}}
    }}]).write_concern(mongodb::options::WriteConcern::majority()).await?;
    Ok(result.matched_count == 1)
}

#[derive(Clone)]
pub struct Lease(Arc<LeaseInner>);
impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lease([redacted])")
    }
}
struct LeaseInner {
    db: Database,
    filter: Document,
    lease_id: String,
    stop: CancellationToken,
    lost: CancellationToken,
}
impl Lease {
    fn new(db: Database, filter: Document, lease_id: String) -> Self {
        let inner = LeaseInner {
            db,
            filter,
            lease_id,
            stop: CancellationToken::new(),
            lost: CancellationToken::new(),
        };
        let (db, filter, lease_id, stop, lost) = (
            inner.db.clone(),
            inner.filter.clone(),
            inner.lease_id.clone(),
            inner.stop.clone(),
            inner.lost.clone(),
        );
        tokio::spawn(async move {
            loop {
                tokio::select! { biased; () = stop.cancelled() => return, () = tokio::time::sleep(RENEW_INTERVAL) => {} }
                tokio::select! { biased;
                    () = stop.cancelled() => return,
                    result = tokio::time::timeout(RENEW_TIMEOUT, renew(&db, &filter, &lease_id)) => {
                        if !matches!(result, Ok(Ok(true))) { lost.cancel(); return; }
                    }
                }
            }
        });
        Self(Arc::new(inner))
    }
    pub async fn cancelled(&self) {
        self.0.lost.cancelled().await;
    }
    pub async fn run<T>(&self, future: impl Future<Output = AppResult<T>>) -> AppResult<T> {
        tokio::select! { biased;
            () = self.0.lost.cancelled() => Err(AppError::ServiceConcurrencyLimited),
            result = future => result,
        }
    }
    /// Couple the existing WS bridge cancellation to loss of service capacity.
    pub fn cancel_on_loss(&self, token: CancellationToken) {
        let (lost, stop) = (self.0.lost.clone(), self.0.stop.clone());
        tokio::spawn(async move {
            tokio::select! { biased; () = lost.cancelled() => token.cancel(), () = stop.cancelled() => {} }
        });
    }
    pub fn hold_response(self, response: Response) -> Response {
        let (parts, mut body) = response.into_parts();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let pump_lease = self.clone();
        // The pump observes loss/disconnect even when a slow client stops polling
        // its response. At most one frame is queued; trailers remain intact.
        tokio::spawn(async move {
            loop {
                let frame = tokio::select! { biased;
                    () = sender.closed() => return,
                    () = pump_lease.cancelled() => {
                        let _ = sender.try_send(Err(std::io::Error::other("Service concurrency lease lost")));
                        return;
                    },
                    frame = body.frame() => frame,
                };
                let Some(frame) = frame else {
                    return;
                };
                let failed = frame.is_err();
                tokio::select! { biased;
                    () = pump_lease.cancelled() => return,
                    result = sender.send(frame.map_err(std::io::Error::other)) => {
                        if result.is_err() || failed { return; }
                    }
                }
            }
        });
        let frames = async_stream::try_stream! {
            let lease = self;
            loop {
                let frame = tokio::select! { biased;
                    () = lease.cancelled() => Err(std::io::Error::other("Service concurrency lease lost")),
                    frame = receiver.recv() => Ok(frame),
                }?;
                let Some(frame) = frame else { break; };
                yield frame?;
            }
        };
        let frames: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<_, std::io::Error>> + Send>,
        > = Box::pin(frames);
        Response::from_parts(parts, Body::new(StreamBody::new(frames)))
    }
}
impl Drop for LeaseInner {
    fn drop(&mut self) {
        self.stop.cancel();
        let (db, filter, lease_id) = (self.db.clone(), self.filter.clone(), self.lease_id.clone());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if db
                    .collection::<Document>(COLLECTION_NAME)
                    .update_one(filter, doc! {"$pull":{"leases":{"id":lease_id}}})
                    .await
                    .is_err()
                {
                    tracing::warn!(
                        "Service concurrency lease release failed; expiry will reclaim it"
                    );
                }
            });
        }
    }
}

#[cfg(test)]
mod tests;
