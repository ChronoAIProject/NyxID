//! Persistent MCP identifiers and the fleet-wide storage cutover.
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    error::Result,
};
use sha2::{Digest, Sha256};

use crate::models::{
    mcp_notification::COLLECTION_NAME as NOTIFICATIONS,
    mcp_session::{
        MAX_PER_USER_SESSIONS, MCP_SESSION_COLLECTION as SESSIONS,
        MCP_SESSION_SLOT_COLLECTION as SLOTS, McpSessionRecord, McpSessionSlot,
        is_duplicate_key_error,
    },
    security_capability_activation::{
        COLLECTION_NAME as ROLLOUTS, RECORD_KIND, SecurityCapabilityActivation,
    },
};

pub(crate) const READINESS: &str = "mcp:session-id-hash-v1";
const PREFIX: &str = "mcp1:";
const GRACE: Duration = Duration::days(7);
const PAGE: i64 = 32;

pub(crate) fn hash(wire: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nyxid:mcp-session-id:v1\0");
    digest.update(wire.as_bytes());
    format!("{PREFIX}{:x}", digest.finalize())
}

/// Used only for keys obtained from storage. Wire input always goes through hash().
pub(crate) fn cache_key(stored: &str) -> String {
    if stored.starts_with(PREFIX) {
        stored.to_owned()
    } else {
        hash(stored)
    }
}

pub(crate) fn keys_for_wire(wire: &str) -> Vec<String> {
    let mut keys = vec![hash(wire)];
    if uuid::Uuid::try_parse(wire).is_ok() {
        keys.push(wire.to_owned());
    }
    keys
}

/// Parent keys from inventory can outlive the snapshot where they were selected.
pub(crate) fn keys_for_stored(stored: &str) -> Vec<String> {
    if stored.starts_with(PREFIX) {
        vec![stored.to_owned()]
    } else {
        vec![hash(stored), stored.to_owned()]
    }
}

#[derive(Debug)]
pub(crate) struct ConcurrentMcpClaim;

fn claim_error(error: mongodb::error::Error) -> mongodb::error::Error {
    if is_duplicate_key_error(&error) {
        mongodb::error::Error::custom(ConcurrentMcpClaim)
    } else {
        error
    }
}

async fn activation(
    db: &Database,
    tx: Option<&mut ClientSession>,
) -> Result<Option<DateTime<Utc>>> {
    let markers = db.collection::<SecurityCapabilityActivation>(ROLLOUTS);
    let query = markers.find_one(doc! {"record_kind": RECORD_KIND, "capability": READINESS});
    let row = match tx {
        Some(tx) => query.session(tx).await?,
        None => query.await?,
    };
    Ok(row.map(|row| row.active_since))
}

pub(crate) async fn fence_activation(db: &Database, tx: &mut ClientSession) -> Result<()> {
    db.collection::<Document>(ROLLOUTS)
        .update_one(
            doc! {"_id":"mcp-session-id-hash-v1-issuance"},
            doc! {"$inc":{"revision":1_i64}},
        )
        .upsert(true)
        .session(tx)
        .await
        .map_err(claim_error)?;
    Ok(())
}

pub(crate) async fn issuance_key(
    db: &Database,
    tx: &mut ClientSession,
    wire: &str,
) -> Result<String> {
    if activation(db, Some(tx)).await?.is_some() {
        return Ok(hash(wire));
    }
    Ok(wire.to_owned())
}

pub(crate) async fn live_record(
    db: &Database,
    wire: &str,
    tx: Option<&mut ClientSession>,
) -> Result<Option<McpSessionRecord>> {
    live_by_keys(db, keys_for_wire(wire), tx).await
}

async fn live_by_keys(
    db: &Database,
    keys: Vec<String>,
    mut tx: Option<&mut ClientSession>,
) -> Result<Option<McpSessionRecord>> {
    let records = db.collection::<McpSessionRecord>(SESSIONS);
    // The prefix sorts after every issued UUID. Both candidates share one read,
    // so a migration between two separate lookups cannot look like expiration.
    let query = records
        .find_one(doc! {"_id": {"$in": keys},
        "expires_at":{"$gt": bson::DateTime::now()}})
        .sort(doc! {"_id":-1});
    let row = match tx.as_deref_mut() {
        Some(tx) => query.session(tx).await?,
        None => query.await?,
    };
    let Some(mut row) = row else { return Ok(None) };
    if !row.id.starts_with(PREFIX) {
        row.absolute_expires_at = activation(db, tx).await?.map(|at| at + GRACE);
    }
    Ok((row.absolute_expires_at.is_none_or(|end| Utc::now() < end)).then_some(row))
}

pub(crate) async fn recover(
    db: &Database,
    scanned_id: &str,
    login_alive: bool,
) -> Result<Option<McpSessionRecord>> {
    let context = (db, keys_for_stored(scanned_id), login_alive);
    super::service_history::transaction::run_send(db, &context, |tx, (db, keys, login_alive)| {
        Box::pin(async move {
            let row = live_by_keys(db, keys.clone(), Some(tx)).await?;
            if let Some(record) = row
                && *login_alive
                && recover_slot(db, tx, &record).await?
            {
                return Ok(Some(record));
            }
            db.collection::<Document>(SESSIONS)
                .delete_many(doc! {"_id":{"$in":keys}})
                .session(&mut **tx)
                .await?;
            for name in [SLOTS, NOTIFICATIONS] {
                db.collection::<Document>(name)
                    .delete_many(doc! {"session_id":{"$in":keys}})
                    .session(&mut **tx)
                    .await?;
            }
            Ok(None)
        })
    })
    .await
}

async fn recover_slot(
    db: &Database,
    tx: &mut ClientSession,
    row: &McpSessionRecord,
) -> Result<bool> {
    let slots = db.collection::<McpSessionSlot>(SLOTS);
    let expires = bson::DateTime::from_chrono(
        row.absolute_expires_at
            .map_or(row.expires_at, |end| row.expires_at.min(end)),
    );
    if slots
        .update_one(
            doc! {"session_id": &row.id},
            doc! {"$set":{"expires_at":expires}},
        )
        .session(&mut *tx)
        .await?
        .matched_count
        > 0
    {
        return Ok(true);
    }

    // A missing slot gives migration and backfill no shared slot to lock.
    // Write the parent so both operations use the same version of its identity.
    db.collection::<Document>(SESSIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$inc":{"slot_recovery_revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    let mut cursor = slots
        .find(doc! {"user_id":&row.user_id, "expires_at":{"$gt":bson::DateTime::now()}})
        .session(&mut *tx)
        .await?;
    let occupied: std::collections::HashSet<i32> = cursor
        .stream(&mut *tx)
        .map_ok(|slot| slot.slot)
        .try_collect()
        .await?;
    let Some(slot) = (0..MAX_PER_USER_SESSIONS as i32).find(|n| !occupied.contains(n)) else {
        return Ok(false);
    };
    slots.update_one(doc!{"_id":format!("{}:{slot}",row.user_id), "expires_at":{"$lte":bson::DateTime::now()}},
        doc!{"$set":{"user_id":&row.user_id, "session_id":&row.id, "slot":slot, "expires_at":expires}})
        .upsert(true).session(tx).await.map_err(claim_error)?;
    Ok(true)
}

pub(crate) async fn migrate_batch(db: &Database, after: &mut Option<String>) -> Result<usize> {
    let Some(activated) = activation(db, None).await? else {
        return Ok(0);
    };
    // Legacy UUIDs occupy a contiguous range of the existing _id index.
    let mut range = doc! {"$lt":PREFIX};
    if let Some(last) = after.as_deref() {
        range.insert("$gt", last);
    }
    let ids: Vec<Document> = db
        .collection::<Document>(SESSIONS)
        .find(doc! {"_id":range})
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .limit(PAGE)
        .await?
        .try_collect()
        .await?;
    if ids.is_empty() {
        *after = None;
    }
    let mut moved = 0;
    for entry in ids {
        if super::shutdown::process().is_draining() {
            break;
        }
        let raw = entry
            .get_str("_id")
            .map_err(mongodb::error::Error::custom)?;
        // Resume beyond this key on the next tick even if its transaction fails.
        *after = Some(raw.to_owned());
        let context = (db, raw, activated + GRACE);
        moved += usize::from(
            super::service_history::transaction::run_send(db, &context, |tx, &(db, raw, end)| {
                Box::pin(migrate_one(db, tx, raw, end))
            })
            .await?,
        );
    }
    Ok(moved)
}

async fn migrate_one(
    db: &Database,
    tx: &mut ClientSession,
    raw: &str,
    end: DateTime<Utc>,
) -> Result<bool> {
    let sessions = db.collection::<Document>(SESSIONS);
    let Some(mut row) = sessions
        .find_one(doc! {"_id":raw})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    let key = hash(raw);
    let expires = (*row
        .get_datetime("expires_at")
        .map_err(mongodb::error::Error::custom)?)
    .min(bson::DateTime::from_chrono(end));
    row.insert("_id", &key);
    row.insert("absolute_expires_at", bson::DateTime::from_chrono(end));
    row.insert("expires_at", expires);
    sessions.insert_one(row).session(&mut *tx).await?;
    db.collection::<Document>(SLOTS)
        .update_many(
            doc! {"session_id":raw},
            doc! {"$set":{"session_id": &key, "expires_at": expires}},
        )
        .session(&mut *tx)
        .await?;
    // Stream in sequence order through the existing outbox index. Preserve
    // the complete BSON including fields from other compatible writers.
    let outbox = db.collection::<Document>(NOTIFICATIONS);
    let mut cursor = outbox
        .find(doc! {"session_id":raw})
        .sort(doc! {"sequence":1})
        .batch_size(32)
        .session(&mut *tx)
        .await?;
    while cursor.advance(&mut *tx).await? {
        let mut notification = cursor.deserialize_current()?;
        let sequence = notification
            .get_i64("sequence")
            .or_else(|_| notification.get_i32("sequence").map(i64::from))
            .map_err(mongodb::error::Error::custom)?;
        let expiry = *notification
            .get_datetime("expires_at")
            .map_err(mongodb::error::Error::custom)?;
        notification.insert("expires_at", expiry.min(bson::DateTime::from_chrono(end)));
        notification.insert("_id", format!("{key}:{sequence}"));
        notification.insert("session_id", &key);
        outbox.insert_one(notification).session(&mut *tx).await?;
    }
    outbox
        .delete_many(doc! {"session_id":raw})
        .session(&mut *tx)
        .await?;
    // Deleting the parent fences any concurrent write made since the snapshot.
    sessions.delete_one(doc! {"_id":raw}).session(tx).await?;
    Ok(true)
}

#[cfg(test)]
mod tests;
