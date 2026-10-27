//! Exclusive ingress across relay/gateway transport moves. Existing channel
//! setup and release records are the authority; no CMA ingress design is assumed.
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use std::time::Duration;

use crate::{
    errors::{AppError, AppResult},
    models::{
        coordination::LEASE_COLLECTION_NAME, nyxbot_channel::COLLECTION_NAME as CHANNELS,
        nyxbot_gateway_release::COLLECTION_NAME as RELEASES,
    },
};

fn channel_filter(bot: &str) -> Document {
    doc! {"channel_bot_id":bot,"status":{"$in":["active","pending"]}}
}

fn channel_allows_relay(channel: &Document) -> bool {
    channel.get_str("transport") == Ok("direct")
        && ["pending_agent_api_key_id", "gateway_setup_release_id"]
            .iter()
            .all(|key| {
                channel
                    .get(*key)
                    .is_none_or(|value| matches!(value, bson::Bson::Null))
            })
}

fn unresolved_release(bot: &str) -> Document {
    // A preserved or uncertain remote record may still own a connection.
    // Local transport=direct alone is insufficient after a failed remote DELETE.
    doc! {"channel_bot_id":bot,"$or":[
        {"state":{"$ne":"settled"}},
        {"resolution":{"$nin":["released","missing","cancelled","not_created","confirmed_missing"]}}
    ]}
}

pub(super) async fn relay_allowed(db: &Database, bot: &str) -> AppResult<bool> {
    let channel = db
        .collection::<Document>(CHANNELS)
        .find_one(channel_filter(bot))
        .await?;
    if channel
        .as_ref()
        .is_some_and(|row| !channel_allows_relay(row))
    {
        return Ok(false);
    }
    Ok(db
        .collection::<Document>(RELEASES)
        .find_one(unresolved_release(bot))
        .projection(doc! {"_id":1})
        .await?
        .is_none())
}

/// The caller already writes the bot in this transaction. Connection insertion
/// uses that same bot fence; this channel write also contends with pending setup,
/// transport swaps, fallback and retirement, including existing replicas.
pub(super) async fn fence_relay(
    db: &Database,
    bot: &str,
    tx: &mut ClientSession,
) -> Result<bool, mongodb::error::Error> {
    let channel = db
        .collection::<Document>(CHANNELS)
        .find_one_and_update(
            channel_filter(bot),
            doc! {"$inc":{"discord_ingress_revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if channel
        .as_ref()
        .is_some_and(|row| !channel_allows_relay(row))
    {
        return Ok(false);
    }
    Ok(db
        .collection::<Document>(RELEASES)
        .find_one(unresolved_release(bot))
        .projection(doc! {"_id":1})
        .session(tx)
        .await?
        .is_none())
}

/// Accepted gateway ownership starts a different event stream. A later relay
/// return must IDENTIFY afresh, never replay that transport's intervening work.
/// Failed setup retains the old checkpoint so Discord can recover the pause.
pub(crate) async fn retire_resume(
    db: &Database,
    bot: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(LEASE_COLLECTION_NAME)
        .update_one(
            doc! {"_id":format!("discord-gateway:{bot}")},
            doc! {"$unset":{"checkpoint":""}},
        )
        .session(tx)
        .await?;
    Ok(())
}

/// Setup is durably reserved before this wait. The producer observes that fence,
/// drops its socket, then releases its lease. Do not send a remote create until
/// the local session is gone (or its conservative lease deadline has elapsed).
pub(crate) async fn wait_relay_stopped(db: &Database, bot: &str) -> AppResult<()> {
    let deadline = super::cluster_lease_runtime().ttl + Duration::from_secs(10);
    tokio::time::timeout(deadline, async {
        loop {
            let active = db
                .collection::<Document>(LEASE_COLLECTION_NAME)
                .find_one(doc! {
                    "_id":format!("discord-gateway:{bot}"),
                    "$expr":{"$gt":["$expires_at","$$NOW"]},
                })
                .projection(doc! {"_id":1})
                .await?;
            if active.is_none() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| AppError::Conflict("Discord relay ingress is still draining".into()))?
}
