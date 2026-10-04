//! Current-policy reads and bounded, transactionally fenced retention cleanup.
use super::{
    api_key_mutation_service as transactions,
    audit_service::{self, AuditActor},
    coordination_service::{LeaseStore, LeaseToken, cluster_lease_runtime},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_attachment::COLLECTION_NAME as ATTACHMENTS,
        assistant_upload_retention::{Policy, SETTINGS_ID, Settings, TOMBSTONES},
        platform_settings::COLLECTION_NAME as SETTINGS,
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
};
use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
    time::Duration as StdDuration,
};

pub const REFRESH_SECONDS: u64 = 5;
const SWEEP_SECONDS: u64 = 60;
const BATCH: i64 = 100;
const LEASE: &str = "assistant-upload-retention";

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub settings: Settings,
    pub refreshed_at: Option<DateTime<Utc>>,
}
pub type Cache = Arc<RwLock<Snapshot>>;

pub fn validate(policy: &Policy) -> AppResult<()> {
    if !(1..=8760).contains(&policy.pending_hours)
        || !(1..=365).contains(&policy.image_days)
        || !(1..=365).contains(&policy.document_days)
        || policy
            .tool_image_days
            .is_some_and(|days| !(1..=365).contains(&days))
    {
        return Err(AppError::BadRequest("Pending uploads must be 1–8760 hours; image, document and tool-image retention must be 1–365 days. Tool images may instead stay with the conversation.".into()));
    }
    Ok(())
}

pub async fn load(db: &Database) -> AppResult<Settings> {
    Ok(db
        .collection::<Settings>(SETTINGS)
        .find_one(doc! {"_id": SETTINGS_ID})
        .await?
        .unwrap_or_default())
}
pub async fn refresh(db: &Database, cache: &Cache) -> AppResult<()> {
    let settings = load(db).await?;
    let mut snapshot = cache.write().unwrap_or_else(|e| e.into_inner());
    if settings.revision >= snapshot.settings.revision {
        *snapshot = Snapshot {
            settings,
            refreshed_at: Some(Utc::now()),
        };
    }
    Ok(())
}

/// None clears the DB override; revisions never go backwards on reset.
pub async fn update(
    db: &Database,
    actor: &AuditActor,
    audit_key: &[u8],
    policy: Option<Policy>,
) -> AppResult<Settings> {
    if let Some(policy) = policy {
        validate(&policy)?;
    }
    ensure_settings(db).await?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let actor = actor.clone();
    let audit_key = zeroize::Zeroizing::new(audit_key.to_vec());
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = Box::pin(async {
                let collection = db.collection::<Settings>(SETTINGS);
                let old = collection
                    .find_one(doc! {"_id": SETTINGS_ID})
                    .session(&mut *session)
                    .await?
                    .unwrap_or_default();
                let new = Settings {
                    policy,
                    revision: old.revision + 1,
                    updated_at: Some(Utc::now()),
                    ..Settings::default()
                };
                collection
                    .replace_one(doc! {"_id": SETTINGS_ID}, &new)
                    .session(&mut *session)
                    .await?;
                Box::pin(audit_service::log_actor_event_in_session(&db, session, &audit_key, &actor,
                "admin_upload_retention_updated", serde_json::json!({
                    "old": old.policy.unwrap_or_default(), "new": new.policy.unwrap_or_default(),
                    "reset": policy.is_none(), "revision": new.revision,
                }))).await?;
                Ok(new)
            })
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

async fn ensure_settings(db: &Database) -> AppResult<()> {
    db.collection::<Document>(SETTINGS).update_one(doc! {"_id": SETTINGS_ID},
        doc! {"$setOnInsert": bson::to_document(&Settings::default()).map_err(|_| AppError::Internal("Retention settings".into()))?})
        .upsert(true).await?;
    Ok(())
}

fn projection() -> Document {
    doc! {"_id":1, "user_id":1, "conversation_id":1, "group_id":1, "origin":1,
    "content_type":1, "message_id":1, "created_at":1, "bound_at":1, "expires_at":1,
    "first_used_turn_id":1, "first_used_conversation_id":1, "first_turn_settled_at":1}
}
fn date(row: &Document, key: &str) -> Option<DateTime<Utc>> {
    row.get_datetime(key).ok().map(|v| v.to_chrono())
}
fn bound(row: &Document) -> bool {
    row.get_str("message_id").is_ok()
}
fn user_upload(row: &Document) -> bool {
    row.get_str("origin").ok() == Some("user_upload")
}
fn bound_at(row: &Document) -> Option<DateTime<Utc>> {
    date(row, "bound_at")
        .or_else(|| date(row, "expires_at").map(|v| v - Duration::days(30)))
        .or_else(|| date(row, "created_at"))
}

fn age_expired(row: &Document, policy: Policy, now: DateTime<Utc>) -> bool {
    let (start, duration) = if row.get_str("origin").ok() == Some("machine_preview") {
        (
            date(row, "created_at"),
            Some(Duration::days(i64::from(policy.document_days.min(30)))),
        )
    } else if !user_upload(row) {
        (
            date(row, "created_at"),
            policy.tool_image_days.map(|v| Duration::days(i64::from(v))),
        )
    } else if !bound(row) {
        (
            date(row, "created_at"),
            Some(Duration::hours(i64::from(policy.pending_hours))),
        )
    } else {
        let days = if row
            .get_str("content_type")
            .unwrap_or_default()
            .starts_with("image/")
        {
            policy.image_days
        } else {
            policy.document_days
        };
        (bound_at(row), Some(Duration::days(i64::from(days))))
    };
    start
        .zip(duration)
        .is_some_and(|(start, duration)| start + duration <= now)
}

/// Also covers pre-policy rows and crashed turns that could not write settlement.
async fn turn_settled(db: &Database, row: &Document, now: DateTime<Utc>) -> AppResult<bool> {
    if date(row, "first_turn_settled_at").is_some() {
        return Ok(true);
    }
    let owner = row.get_str("user_id").unwrap_or_default();
    let mut conversation = row
        .get_str("first_used_conversation_id")
        .unwrap_or_default()
        .to_owned();
    let mut turn = row
        .get_str("first_used_turn_id")
        .unwrap_or_default()
        .to_owned();
    if turn.is_empty() {
        let first = db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .find_one(doc! {"user_id":owner, "attachments.id":row.get_str("_id").unwrap_or_default(), "role":{"$ne":"assistant"}})
            .projection(doc! {"conversation_id":1,"turn_id":1}).sort(doc! {"created_at":1,"_id":1}).await?;
        let Some(first) = first else {
            return Ok(false);
        };
        conversation = first.get_str("conversation_id").unwrap_or_default().into();
        turn = first.get_str("turn_id").unwrap_or_default().into();
    }
    let thread = db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&conversation,"user_id":owner})
        .await?;
    Ok(thread
        .as_ref()
        .and_then(|r| super::assistant_nyxagent::live_turn(r, now))
        .is_none_or(|t| t.turn_id != turn))
}
async fn expired(
    db: &Database,
    row: &Document,
    policy: Policy,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    if age_expired(row, policy, now) {
        return Ok(true);
    }
    if policy.images_delete_after_turn
        && user_upload(row)
        && bound(row)
        && row
            .get_str("content_type")
            .unwrap_or_default()
            .starts_with("image/")
    {
        return Box::pin(turn_settled(db, row, now)).await;
    }
    Ok(false)
}

/// Scope must already be authorized. Live Mongo policy, never the replica cache.
pub async fn require_available(db: &Database, filter: Document) -> AppResult<()> {
    let row = db
        .collection::<Document>(ATTACHMENTS)
        .find_one(filter.clone())
        .projection(projection())
        .await?;
    if let Some(row) = row {
        if Box::pin(expired(
            db,
            &row,
            load(db).await?.policy.unwrap_or_default(),
            Utc::now(),
        ))
        .await?
        {
            return Err(AppError::AssistantAttachmentExpired);
        }
        return Ok(());
    }
    if db
        .collection::<Document>(TOMBSTONES)
        .find_one(filter)
        .await?
        .is_some()
    {
        return Err(AppError::AssistantAttachmentExpired);
    }
    Err(AppError::NotFound("Attachment not found".into()))
}

/// One bounded metadata batch for a transcript page; never load encrypted bodies.
pub async fn expired_ids(db: &Database, user: &str, ids: &[String]) -> AppResult<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let policy = load(db).await?.policy.unwrap_or_default();
    let mut result = HashSet::new();
    for chunk in ids.chunks(200) {
        let filter = doc! {"_id":{"$in":chunk},"user_id":user};
        let rows: Vec<Document> = db
            .collection::<Document>(ATTACHMENTS)
            .find(filter.clone())
            .projection(projection())
            .await?
            .try_collect()
            .await?;
        for row in rows {
            if Box::pin(expired(db, &row, policy, Utc::now())).await? {
                result.insert(row.get_str("_id").unwrap_or_default().into());
            }
        }
        let rows: Vec<Document> = db
            .collection::<Document>(TOMBSTONES)
            .find(filter)
            .projection(doc! {"_id":1})
            .await?
            .try_collect()
            .await?;
        for row in rows {
            result.insert(row.get_str("_id").unwrap_or_default().into());
        }
    }
    Ok(result)
}

/// Mark the first referencing turn, including group turns, in the message transaction.
pub async fn used_in_turn(
    db: &Database,
    session: &mut ClientSession,
    user: &str,
    conversation: &str,
    turn: &str,
    ids: &[String],
) -> AppResult<()> {
    if !ids.is_empty() {
        db.collection::<Document>(ATTACHMENTS).update_many(doc! {"_id":{"$in":ids},"user_id":user,"origin":"user_upload","content_type":{"$regex":"^image/"},"first_used_turn_id":bson::Bson::Null},
            doc! {"$set":{"first_used_turn_id":turn,"first_used_conversation_id":conversation}}).session(&mut *session).await?;
    }
    Ok(())
}

async fn delete_root(db: &Database, session: &mut ClientSession, row: &Document) -> AppResult<()> {
    let id = row
        .get_str("_id")
        .map_err(|_| AppError::Internal("Attachment metadata".into()))?;
    if bound(row) || !user_upload(row) {
        let mut tombstone = doc! {"_id":id, "expired_at":bson::DateTime::now()};
        for key in [
            "user_id",
            "conversation_id",
            "group_id",
            "origin",
            "message_id",
        ] {
            if let Some(value) = row.get(key) {
                tombstone.insert(key, value.clone());
            }
        }
        db.collection::<Document>(TOMBSTONES)
            .replace_one(doc! {"_id":id}, tombstone)
            .upsert(true)
            .session(&mut *session)
            .await?;
    }
    db.collection::<Document>(ATTACHMENTS)
        .delete_many(doc! {"$or":[{"_id":id},{"parent_attachment_id":id}]})
        .session(&mut *session)
        .await?;
    Ok(())
}

/// Settlement and immediate image deletion share the turn's transaction.
pub async fn settled_in_session(
    db: &Database,
    session: &mut ClientSession,
    user: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<()> {
    let filter =
        doc! {"user_id":user,"first_used_conversation_id":conversation,"first_used_turn_id":turn};
    let changed = db
        .collection::<Document>(ATTACHMENTS)
        .update_many(
            filter.clone(),
            doc! {"$set":{"first_turn_settled_at":bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    if changed.matched_count == 0 {
        return Ok(());
    }
    let policy = db
        .collection::<Settings>(SETTINGS)
        .find_one_and_update(doc! {"_id":SETTINGS_ID}, doc! {"$inc":{"sweep_fence":1}})
        .session(&mut *session)
        .await?
        .unwrap_or_default()
        .policy
        .unwrap_or_default();
    if policy.images_delete_after_turn {
        let rows: Vec<Document> = db
            .collection::<Document>(ATTACHMENTS)
            .find(filter)
            .projection(projection())
            .session(&mut *session)
            .await?
            .stream(&mut *session)
            .try_collect()
            .await?;
        for row in rows {
            Box::pin(delete_root(db, session, &row)).await?;
        }
    }
    Ok(())
}

/// Remove only the attachment TTL, never the upload admission-window TTL.
pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    ensure_settings(db).await?;
    let indexes: Vec<_> = db
        .collection::<Document>(ATTACHMENTS)
        .list_indexes()
        .await?
        .try_collect()
        .await?;
    for index in indexes {
        if index
            .options
            .as_ref()
            .and_then(|o| o.expire_after)
            .is_some()
            && index.keys == doc! {"expires_at":1}
            && let Some(name) = index.options.and_then(|o| o.name)
        {
            // Ignore only a concurrent drop, not authorization or storage errors.
            if let Err(error) = db
                .collection::<Document>(ATTACHMENTS)
                .drop_index(name)
                .await
                && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 27 || e.code == 26)
            {
                return Err(error.into());
            }
        }
    }
    db.collection::<Document>(ATTACHMENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"origin":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"attachments.id":1,"created_at":1})
                .build(),
        )
        .await?;
    for key in ["conversation_id", "group_id", "user_id"] {
        db.collection::<Document>(TOMBSTONES)
            .create_index(IndexModel::builder().keys(doc! {key:1}).build())
            .await?;
    }
    Ok(())
}

async fn delete_fenced(db: &Database, token: &LeaseToken, id: &str) -> AppResult<bool> {
    Box::pin(delete_fenced_kind(db, token, id, false)).await
}

async fn delete_fenced_kind(
    db: &Database,
    token: &LeaseToken,
    id: &str,
    orphan: bool,
) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let token = token.clone();
    let id = id.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let operation = Box::pin(async {
            let fenced = db.collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME).update_one(
                doc! {"_id":&token.name,"lease_id":&token.lease_id,"holder.instance_id":&token.holder.instance_id,"holder.generation_id":&token.holder.generation_id,"$expr":{"$gt":["$expires_at","$$NOW"]}},
                doc! {"$inc":{"retention_fence":1}}).session(&mut *session).await?;
            if fenced.matched_count != 1 { return Ok(false); }
            // A concurrent policy update conflicts with this write, so an old
            // snapshot cannot delete after a newer, longer policy commits.
            let settings = db.collection::<Settings>(SETTINGS).find_one_and_update(doc! {"_id":SETTINGS_ID},doc! {"$inc":{"sweep_fence":1}}).session(&mut *session).await?.unwrap_or_default();
            let row = db.collection::<Document>(ATTACHMENTS).find_one(doc! {"_id":&id}).projection(projection()).session(&mut *session).await?;
            if orphan {
                if row.is_some() { return Ok(false); }
                // Old fixed-TTL cleanup could delete a root before its chunks.
                // Upload roots and chunks are inserted atomically, so a missing
                // parent cannot be an in-progress upload.
                db.collection::<Document>(ATTACHMENTS).delete_many(doc! {"parent_attachment_id":&id}).session(&mut *session).await?;
                return Ok(true);
            }
            let Some(row) = row else { return Ok(false); };
            if !Box::pin(expired(&db,&row,settings.policy.unwrap_or_default(),Utc::now())).await? { return Ok(false); }
            Box::pin(delete_root(&db,session,&row)).await?;
            Ok(true)
        }).await;
        transactions::transaction_result(operation)
    }).await.map_err(transactions::map_transaction_error)
}

async fn sweep_batch(db: &Database, token: &LeaseToken) -> AppResult<usize> {
    let settings = load(db).await?;
    let checkpoint = LeaseStore::load_checkpoint(db, token).await?;
    let previous = checkpoint.as_ref().and_then(|c| c.as_document());
    let cursor = previous
        .filter(|v| v.get_i64("revision").ok() == Some(settings.revision))
        .and_then(|v| v.get_str("cursor").ok())
        .unwrap_or_default();
    let rows: Vec<Document> = db
        .collection::<Document>(ATTACHMENTS)
        .find(doc! {"_id":{"$gt":cursor}, "parent_attachment_id":{"$exists":false}})
        .projection(projection())
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .await?
        .try_collect()
        .await?;
    let mut deleted = 0;
    for row in &rows {
        if Box::pin(expired(
            db,
            row,
            settings.policy.unwrap_or_default(),
            Utc::now(),
        ))
        .await?
            && Box::pin(delete_fenced(
                db,
                token,
                row.get_str("_id").unwrap_or_default(),
            ))
            .await?
        {
            deleted += 1;
        }
    }
    let chunk_cursor = previous
        .and_then(|v| v.get_str("chunk_cursor").ok())
        .unwrap_or_default();
    let chunks: Vec<Document> = db
        .collection::<Document>(ATTACHMENTS)
        .find(doc! {"_id":{"$gt":chunk_cursor},"parent_attachment_id":{"$type":"string"}})
        .projection(doc! {"_id":1,"parent_attachment_id":1})
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .await?
        .try_collect()
        .await?;
    let parents: HashSet<&str> = chunks
        .iter()
        .filter_map(|r| r.get_str("parent_attachment_id").ok())
        .collect();
    if !parents.is_empty() {
        let existing: Vec<Document> = db
            .collection::<Document>(ATTACHMENTS)
            .find(doc! {"_id":{"$in":parents.iter().copied().collect::<Vec<_>>()}})
            .projection(doc! {"_id":1})
            .await?
            .try_collect()
            .await?;
        let present: HashSet<&str> = existing
            .iter()
            .filter_map(|r| r.get_str("_id").ok())
            .collect();
        for parent in parents.difference(&present) {
            Box::pin(delete_fenced_kind(db, token, parent, true)).await?;
        }
    }
    let chunk_next = if chunks.len() == BATCH as usize {
        chunks
            .last()
            .and_then(|r| r.get_str("_id").ok())
            .unwrap_or_default()
    } else {
        ""
    };
    let next = if rows.len() == BATCH as usize {
        rows.last()
            .and_then(|r| r.get_str("_id").ok())
            .unwrap_or_default()
    } else {
        ""
    };
    LeaseStore::store_checkpoint(
        db,
        token,
        bson::Bson::Document(
            doc! {"revision":settings.revision,"cursor":next,"chunk_cursor":chunk_next},
        ),
    )
    .await?;
    Ok(deleted)
}

pub async fn sweep(db: &Database) -> AppResult<usize> {
    let runtime = cluster_lease_runtime();
    let Some(token) = runtime.acquire(db, LEASE).await? else {
        return Ok(0);
    };
    let result = runtime
        .run_while_renewed(db, &token, Box::pin(sweep_batch(db, &token)))
        .await;
    let release = LeaseStore::release(db, &token).await;
    match result {
        Some(result) => {
            release?;
            result
        }
        None => Ok(0),
    }
}

pub fn spawn(state: AppState) {
    let refresh_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(StdDuration::from_secs(REFRESH_SECONDS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if refresh(&refresh_state.db, &refresh_state.upload_retention)
                .await
                .is_err()
            {
                tracing::warn!(
                    "Upload retention policy refresh failed; attachment reads still require MongoDB"
                );
            }
        }
    });
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(StdDuration::from_secs(SWEEP_SECONDS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if Box::pin(sweep(&state.db)).await.is_err() {
                tracing::warn!("Upload retention sweep deferred");
            }
        }
    });
}

#[cfg(test)]
#[path = "assistant_upload_retention_tests.rs"]
mod tests;
