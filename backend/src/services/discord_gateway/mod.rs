//! Default-off Discord Gateway ingress. A renewable, database-time lease owns
//! each socket; metadata admission and status/checkpoint writes contend on its
//! exact fence. The socket is dropped before releasing ownership.
mod facts;
pub(crate) use facts::snowflake as valid_message_id;
mod protocol;
#[cfg(test)]
mod tests;
pub(crate) mod transport;

use bson::{Document, doc};
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::coordination_service::{
    LeaseStore, LeaseToken, active_lease_filter, cluster_lease_runtime,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_bot::{COLLECTION_NAME, ChannelBot},
};
use protocol::{Failure, Resume};

const MAX_SESSIONS: usize = 32;
const DISCOVERY_BATCH: i64 = 100;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const API: &str = "https://discord.com/api/v10";
pub(crate) const EVENT_WEBHOOK_NOTICE: &str = "discord_event_webhooks_wrong_endpoint";

/// Called only after the HTTP layer has authenticated the Discord signature.
/// Event Webhooks PING is deliberately refused (400, not its expected 204).
/// A later correct Interactions PING clears only this setup notice.
pub(crate) async fn record_setup_probe(
    db: &mongodb::Database,
    bot_id: &str,
    misplaced: bool,
) -> AppResult<()> {
    let mut filter = doc! {"_id":bot_id,"platform":"discord","is_active":true};
    if !misplaced {
        filter.insert("diagnostics.failure_reason", EVENT_WEBHOOK_NOTICE);
    }
    db.collection::<Document>(COLLECTION_NAME).update_one(filter,
        doc! {"$set":{"diagnostics.failure_reason":if misplaced {bson::Bson::String(EVENT_WEBHOOK_NOTICE.into())} else {bson::Bson::Null}}}).await?;
    Ok(())
}

/// This capability is internal and never deserialized from an HTTP request.
#[derive(Clone)]
pub(crate) struct Fence {
    pub lease: LeaseToken,
    pub bot: ChannelBot,
}
impl Fence {
    fn bot_filter(&self) -> Document {
        doc! {"_id": &self.bot.id, "user_id": &self.bot.user_id,
        "platform":"discord", "is_active":true,
        "status":{"$in":["active","pending_webhook"]},
        "ownership_version": if self.bot.ownership_version == 0 { bson::Bson::Document(doc! {"$in":[bson::Bson::Null,0_i64]}) } else { self.bot.ownership_version.into() },
        "bot_token_encrypted": bson::Binary { subtype:bson::spec::BinarySubtype::Generic, bytes:self.bot.bot_token_encrypted.clone() },
        "platform_bot_id": &self.bot.platform_bot_id}
    }
    /// A write, not a snapshot read: lease replacement, owner changes, deletion
    /// and credential rotation conflict with admission in the same transaction.
    pub(crate) async fn touch(
        &self,
        db: &mongodb::Database,
        tx: &mut mongodb::ClientSession,
    ) -> Result<bool, mongodb::error::Error> {
        if !self.touch_identity(db, tx).await? {
            return Ok(false);
        }
        transport::fence_relay(db, &self.bot.id, tx).await
    }

    async fn touch_identity(
        &self,
        db: &mongodb::Database,
        tx: &mut mongodb::ClientSession,
    ) -> Result<bool, mongodb::error::Error> {
        let lease = db
            .collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
            .update_one(
                active_lease_filter(&self.lease.name, &self.lease.holder, &self.lease.lease_id),
                doc! {"$inc":{"discord_ingress_revision":1_i64}},
            )
            .session(&mut *tx)
            .await?;
        if lease.matched_count != 1 {
            return Ok(false);
        }
        let bot = db
            .collection::<Document>(COLLECTION_NAME)
            .update_one(
                self.bot_filter(),
                doc! {"$inc":{"discord_ingress_revision":1_i64}},
            )
            .session(tx)
            .await?;
        Ok(bot.matched_count == 1)
    }
    pub(crate) async fn record_inbound(&self, db: &mongodb::Database) -> AppResult<()> {
        super::transaction_service::run_db_only(db, &(db.clone(), self.clone()), |tx, (db, fence)| {
            Box::pin(async move {
                if !fence.touch(db, tx).await? { return Err(AppError::ChannelBotInactive("Discord Gateway ownership lost".into())); }
                db.collection::<Document>(COLLECTION_NAME).update_one(fence.bot_filter(),
                    doc! {"$set":{"diagnostics.last_inbound_at":bson::DateTime::now(),"diagnostics.last_inbound_source":"discord_gateway"}}).session(tx).await?;
                Ok(())
            })
        }).await
    }

    pub(crate) async fn current(&self, db: &mongodb::Database) -> AppResult<bool> {
        super::transaction_service::run_db_only(
            db,
            &(db.clone(), self.clone()),
            |tx, (db, fence)| Box::pin(async move { Ok(fence.touch(db, tx).await?) }),
        )
        .await
    }
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    resume: Resume,
    credential_digest: String,
    ownership_version: i64,
    #[serde(default)]
    platform_bot_id: String,
    #[serde(default)]
    owner_id: String,
}
impl Checkpoint {
    fn matches_bot(&self, bot: &ChannelBot) -> bool {
        self.credential_digest == credential_digest(bot)
            && self.ownership_version == bot.ownership_version
            && self.platform_bot_id == bot.platform_bot_id
            && self.owner_id == bot.user_id
    }
}
fn credential_digest(bot: &ChannelBot) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(&bot.bot_token_encrypted))
}

/// All inputs to this URL are fixed or Discord-authenticated; no user URLs.
async fn json(http: &reqwest::Client, token: &str, url: &str) -> Result<Value, Failure> {
    timeout_http(async {
        let response = http
            .get(url)
            .header("Authorization", format!("Bot {token}"))
            .send()
            .await
            .map_err(|_| Failure::Transport)?;
        match response.status().as_u16() {
            401 => return Err(Failure::InvalidToken),
            429 => return Err(Failure::RateLimited),
            200 => {}
            _ => return Err(Failure::Transport),
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Failure::Transport)?;
            if body.len() + chunk.len() > 256 * 1024 {
                return Err(Failure::Protocol);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| Failure::Protocol)
    })
    .await
}
async fn timeout_http<T>(
    work: impl std::future::Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    tokio::time::timeout(HTTP_TIMEOUT, work)
        .await
        .unwrap_or(Err(Failure::Transport))
}

async fn enabled(db: &mongodb::Database, bot: &ChannelBot) -> AppResult<bool> {
    if !super::feature_flag_service::personal_flag_enabled(
        db,
        &bot.user_id,
        super::feature_flag_service::DISCORD_GATEWAY_FLAG_KEY,
    )
    .await?
    {
        return Ok(false);
    }
    super::security_readiness_service::require(
        db,
        super::feature_flag_service::DISCORD_GATEWAY_FLAG_KEY,
    )
    .await?;
    transport::relay_allowed(db, &bot.id).await
}

/// Bounded, rotating scan avoids starvation when earlier bots are already
/// leased on another replica. No new timer/env setting; reuse the poll cadence.
pub async fn run(state: AppState, shutdown: CancellationToken) {
    let mut tasks: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut after = String::new();
    let mut tick = tokio::time::interval(Duration::from_secs(
        if state.config.channel_poll_interval_secs == 0 {
            60
        } else {
            state.config.channel_poll_interval_secs
        },
    ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! { _ = shutdown.cancelled() => break, _ = tick.tick() => {} }
        tasks.retain(|_, task| !task.is_finished());
        let scan = async {
            state.db.collection::<ChannelBot>(COLLECTION_NAME)
                .find(doc! {"platform":"discord","is_active":true,"status":{"$in":["active","pending_webhook"]},"_id":{"$gt":&after}})
                .sort(doc! {"_id":1}).limit(DISCOVERY_BATCH).await?.try_collect::<Vec<_>>().await
        };
        let bots = tokio::select! {
            _ = shutdown.cancelled() => break,
            result = tokio::time::timeout(HTTP_TIMEOUT, scan) => match result {
                Ok(Ok(bots)) => bots,
                _ => { tracing::warn!("Discord Gateway discovery unavailable"); continue; }
            }
        };
        if bots.is_empty() {
            after.clear();
        }
        for bot in bots {
            after = bot.id.clone();
            if shutdown.is_cancelled() {
                break;
            }
            if tasks.contains_key(&bot.id) || !enabled(&state.db, &bot).await.unwrap_or(false) {
                continue;
            }
            let Ok(Some(lease)) = cluster_lease_runtime()
                .acquire(&state.db, &format!("discord-gateway:{}", bot.id))
                .await
            else {
                continue;
            };
            if tasks.len() >= MAX_SESSIONS {
                let fence = Fence { lease, bot };
                let _ = status(
                    &state.db,
                    &fence,
                    "waiting_capacity",
                    Some("discord_gateway_capacity"),
                )
                .await;
                let _ = LeaseStore::release(&state.db, &fence.lease).await;
                continue;
            }
            let task_state = state.clone();
            let cancel = shutdown.child_token();
            tasks.insert(
                bot.id.clone(),
                tokio::spawn(async move {
                    run_owned(&task_state, Fence { lease, bot }, cancel).await;
                }),
            );
        }
    }
    // Cancellation drops sockets and pending callbacks first, then releases
    // only this process generation's leases. DB hangs cannot stall shutdown.
    for task in tasks.into_values() {
        let _ = task.await;
    }
}

async fn guard(state: &AppState, fence: &Fence, mut deadline: tokio::time::Instant) {
    let runtime = cluster_lease_runtime();
    loop {
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + runtime.renew_interval),
        )
        .await;
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        let started = tokio::time::Instant::now();
        let renewal = async {
            if !enabled(&state.db, &fence.bot).await? || !fence.current(&state.db).await? {
                return Ok(false);
            }
            LeaseStore::renew(&state.db, &fence.lease, runtime.ttl).await
        };
        if !matches!(
            tokio::time::timeout_at(deadline, renewal).await,
            Ok(Ok::<_, AppError>(true))
        ) {
            return;
        }
        deadline = started + runtime.ttl.saturating_sub(Duration::from_secs(1));
    }
}

async fn run_owned(state: &AppState, fence: Fence, shutdown: CancellationToken) {
    // Acquisition may have completed slowly or this task may have waited to be
    // scheduled. Prove live ownership before opening any external connection,
    // and measure the conservative local deadline before the renewal request.
    let runtime = cluster_lease_runtime();
    let started = tokio::time::Instant::now();
    let deadline = started + runtime.ttl.saturating_sub(Duration::from_secs(1));
    let ready = tokio::select! {
        biased;
        _ = shutdown.cancelled() => false,
        result = tokio::time::timeout_at(deadline, async {
            if !enabled(&state.db, &fence.bot).await? || !fence.current(&state.db).await? {
                return Ok(false);
            }
            LeaseStore::renew(&state.db, &fence.lease, runtime.ttl).await
        }) => matches!(result, Ok(Ok::<_, AppError>(true))),
    };
    if ready && tokio::time::Instant::now() < deadline {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {},
            _ = guard(state, &fence, deadline) => {},
            _ = session(state, &fence) => {},
        }
    }
    // A lost fence cannot overwrite a successor's status.
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        status(&state.db, &fence, "stopped", None),
    )
    .await;
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        LeaseStore::release(&state.db, &fence.lease),
    )
    .await;
}

async fn status(
    db: &mongodb::Database,
    fence: &Fence,
    state: &str,
    reason: Option<&str>,
) -> AppResult<()> {
    super::transaction_service::run_db_only(db, &(db.clone(), fence.clone(), state.to_owned(), reason.map(str::to_owned)), |tx, (db, fence, state, reason)| {
        Box::pin(async move {
            if !fence.touch_identity(db, tx).await? { return Err(AppError::ChannelBotInactive("Discord Gateway ownership lost".into())); }
            let mut set = doc! {"diagnostics.discord_gateway":{"state":state,"reason":reason,"updated_at":bson::DateTime::now()}};
            if let Some(reason) = reason { set.insert("diagnostics.failure_reason", reason); }
            db.collection::<Document>(COLLECTION_NAME).update_one(fence.bot_filter(),doc! {"$set":set}).session(&mut *tx).await?;
            if reason.is_none() && state == "connected" {
                let mut filter = fence.bot_filter();
                filter.insert("diagnostics.failure_reason", doc! {"$regex":"^discord_gateway_"});
                db.collection::<Document>(COLLECTION_NAME).update_one(filter,doc! {"$set":{"diagnostics.failure_reason":bson::Bson::Null}}).session(tx).await?;
            }
            Ok(())
        })
    }).await
}

async fn checkpoint(db: &mongodb::Database, fence: &Fence, resume: &Resume) -> Result<(), Failure> {
    let checkpoint = Checkpoint {
        resume: resume.clone(),
        credential_digest: credential_digest(&fence.bot),
        ownership_version: fence.bot.ownership_version,
        platform_bot_id: fence.bot.platform_bot_id.clone(),
        owner_id: fence.bot.user_id.clone(),
    };
    let value = bson::to_bson(&checkpoint).map_err(|_| Failure::Ingress)?;
    super::transaction_service::run_db_only(
        db,
        &(db.clone(), fence.clone(), value),
        |tx, (db, fence, value)| {
            Box::pin(async move {
                if !fence.touch(db, tx).await? {
                    return Err(AppError::ChannelBotInactive(
                        "Discord Gateway ownership lost".into(),
                    ));
                }
                db.collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
                    .update_one(
                        active_lease_filter(
                            &fence.lease.name,
                            &fence.lease.holder,
                            &fence.lease.lease_id,
                        ),
                        doc! {"$set":{"checkpoint":value,"record_kind":"checkpoint"}},
                    )
                    .session(tx)
                    .await?;
                Ok(())
            })
        },
    )
    .await
    .map_err(|_| Failure::Ingress)
}

/// Small metadata-only cache. Thread/channel/guild events invalidate it; a
/// bounded TTL also refreshes metadata after a missed or resumed event.
#[derive(Default)]
struct Channels(tokio::sync::Mutex<HashMap<String, (std::time::Instant, Value)>>);
impl Channels {
    async fn get(&self, http: &reqwest::Client, token: &str, chat: &str) -> Result<Value, Failure> {
        {
            let cache = self.0.lock().await;
            if let Some((at, value)) = cache.get(chat)
                && at.elapsed() < Duration::from_secs(60)
            {
                return Ok(value.clone());
            }
        }
        let value = json(http, token, &format!("{API}/channels/{chat}")).await?;
        let value = serde_json::json!({"id":value["id"],"type":value["type"],"parent_id":value["parent_id"],"guild_id":value["guild_id"]});
        let mut cache = self.0.lock().await;
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(chat.into(), (std::time::Instant::now(), value.clone()));
        Ok(value)
    }
}

async fn session(state: &AppState, fence: &Fence) {
    let token =
        match super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, &fence.bot)
            .await
        {
            Ok(token) => Zeroizing::new(token),
            Err(_) => {
                let _ = status(
                    &state.db,
                    fence,
                    "blocked",
                    Some("discord_gateway_credentials_unavailable"),
                )
                .await;
                return;
            }
        };
    let mut resume = LeaseStore::load_checkpoint(&state.db, &fence.lease)
        .await
        .ok()
        .flatten()
        .and_then(|v| bson::from_bson::<Checkpoint>(v).ok())
        .filter(|v| v.matches_bot(&fence.bot))
        .map(|v| v.resume)
        .unwrap_or_default();
    let mut attempts = 0_u32;
    let channels = Channels::default();
    loop {
        let _ = status(&state.db, fence, "connecting", None).await;
        let connected_at = std::time::Instant::now();
        let gateway = if resume.session_id.is_empty() {
            json(&state.http_client, &token, &format!("{API}/gateway/bot")).await
        } else {
            Ok(serde_json::json!({"url":resume.url}))
        };
        let failure = match gateway {
            Ok(info) => {
                // `shards` is a recommendation. An unsharded connection is
                // refused only when Discord reports 4010/4011 during IDENTIFY.
                if resume.session_id.is_empty()
                    && info["session_start_limit"]["remaining"].as_u64() == Some(0)
                {
                    let _ = status(
                        &state.db,
                        fence,
                        "reconnecting",
                        Some(Failure::RateLimited.code()),
                    )
                    .await;
                    let delay = info["session_start_limit"]["reset_after"]
                        .as_u64()
                        .unwrap_or(60_000)
                        .clamp(1_000, 86_400_000);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                } else if let Some(url) = info["url"].as_str() {
                    // One unsharded IDENTIFY per five seconds for this Discord
                    // identity, including lease handover to another replica.
                    if resume.session_id.is_empty() {
                        let gate = LeaseStore::acquire(
                            &state.db,
                            &format!("discord-identify:{}", fence.bot.platform_bot_id),
                            &fence.lease.holder,
                            Duration::from_secs(5),
                        )
                        .await;
                        if !matches!(gate, Ok(Some(_))) {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    }
                    let adapter = super::channel_adapters::discord::DiscordAdapter::default();
                    protocol::connection(url, &token, &fence.bot.platform_bot_id, &mut resume, |cursor, event| {
                        let adapter = &adapter;
                        let token = &token;
                        let channels = &channels;
                        async move {
                            if event["t"].as_str().is_some_and(|name| name.starts_with("THREAD_") || name.starts_with("CHANNEL_") || name.starts_with("GUILD_")) {
                                channels.0.lock().await.clear();
                            }
                            if matches!(event["t"].as_str(), Some("READY" | "RESUMED")) {
                                status(&state.db, fence, "connected", None).await.map_err(|_| Failure::Ingress)?;
                            }
                            if event["t"] == "MESSAGE_CREATE" {
                                let msg = &event["d"];
                                if msg["author"]["bot"] != true && msg["author"]["id"].as_str() != Some(&fence.bot.platform_bot_id)
                                    && msg.get("webhook_id").is_none_or(Value::is_null) && matches!(msg["type"].as_u64(), Some(0 | 19)) {
                                    let chat = msg["channel_id"].as_str().filter(|id| facts::snowflake(id)).ok_or(Failure::Protocol)?;
                                    let channel = channels.get(&state.http_client, token, chat).await?;
                                    if let Some((inbound, facts)) = facts::normalize(&fence.bot, &event, &channel)? {
                                        let result = super::channel_inbound_service::process_discord_gateway_message(state, &fence.bot, adapter, &inbound, fence, &facts).await;
                                        if !matches!(result, Ok(true)) { return Err(Failure::Ingress); }
                                    }
                                }
                            }
                            checkpoint(&state.db, fence, &cursor).await
                        }
                    }).await
                } else {
                    Failure::Protocol
                }
            }
            Err(failure) => failure,
        };
        if matches!(failure, Failure::InvalidSession(false)) {
            let _ = checkpoint(&state.db, fence, &resume).await;
        }
        if connected_at.elapsed() >= Duration::from_secs(60) {
            attempts = 0;
        }
        attempts = attempts.saturating_add(1);
        let _ = status(
            &state.db,
            fence,
            if failure.terminal() {
                "blocked"
            } else {
                "reconnecting"
            },
            Some(failure.code()),
        )
        .await;
        tracing::warn!(bot_id = %fence.bot.id, reason = failure.code(), "Discord Gateway reconnect scheduled");
        tokio::time::sleep(protocol::retry_delay(attempts, failure)).await;
    }
}
