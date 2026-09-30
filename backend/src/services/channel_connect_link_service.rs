//! Hosted channel setup requests. Service connect links retain their own state machine.
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::{ClientSession, Database, options::ReturnDocument};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    connect_link_service, developer_webhook_service::DeveloperWebhookDispatcher, org_service,
    webhook_delivery_service,
};
use crate::crypto::{aes::EncryptionKeys, token::generate_random_token};
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::models::channel_connect_link::{
    COLLECTION_NAME as LINKS, ChannelConnectLink as Link, LinkStatus, Receiver,
};

const TOKEN_PREFIX: &str = "nyx_bcl_";
const CLAIM_SECONDS: i64 = 120;
const GRACE_SECONDS: i64 = 1800;
const MAX_CYCLES: u32 = 5;

pub struct CreateInput {
    pub owner: String,
    pub actor: String,
    pub platform: String,
    pub label: String,
    pub requested_by: Option<String>,
    pub app_id: Option<String>,
    pub callback_url: Option<String>,
    pub webhook_url: Option<String>,
    pub expires_in: Option<i64>,
}

pub struct Created {
    pub link: Link,
    pub token: Zeroizing<String>,
    pub signing_secret: Option<Zeroizing<String>>,
    pub signing_key_id: Option<String>,
}

#[derive(Clone)]
pub struct Claim {
    pub link: Link,
    pub id: String,
}

fn missing() -> AppError {
    AppError::NotFound("Channel connect link not found".into())
}
fn busy() -> AppError {
    AppError::Conflict("Channel setup is in progress; retry shortly".into())
}
fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn status_name(status: LinkStatus) -> &'static str {
    match status {
        LinkStatus::Pending => "pending",
        LinkStatus::Completed => "completed",
        LinkStatus::Cancelled => "cancelled",
        LinkStatus::Expired => "expired",
    }
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    let rows = db.collection::<Link>(LINKS);
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"token_hash": 1})
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    for keys in [
        doc! {"user_id": 1, "created_at": -1},
        doc! {"status": 1, "expires_at": 1},
        doc! {"delivery_status": 1, "delivery_after": 1},
        doc! {"bot_id": 1},
        doc! {"telegram_request_id": 1},
    ] {
        rows.create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    Ok(())
}

pub async fn create(
    db: &Database,
    keys: &EncryptionKeys,
    input: CreateInput,
) -> AppResult<Created> {
    if !org_service::resolve_owner_access(db, &input.actor, &input.owner)
        .await?
        .can_write()
    {
        return Err(missing());
    }
    let label = input.label.trim();
    if label.is_empty() || label.len() > 128 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 128 characters".into(),
        ));
    }
    if input.requested_by.as_ref().is_some_and(|v| v.len() > 128) {
        return Err(AppError::ValidationError(
            "requested_by must be at most 128 characters".into(),
        ));
    }
    if input.app_id.is_some() && input.webhook_url.is_some() {
        return Err(AppError::ValidationError(
            "App callers use their registered connection webhook".into(),
        ));
    }
    let (callback_url, app) = connect_link_service::resolve_requesting_app(
        db,
        input.app_id.as_deref(),
        input.callback_url.as_deref(),
    )
    .await?;
    let mut signing_secret = None;
    let mut signing_key_id = None;
    let receiver = if let Some(app) = &app {
        Some(Receiver::App {
            app_id: app.id.clone(),
        })
    } else if let Some(url) = &input.webhook_url {
        let url = webhook_delivery_service::validate_webhook_url(url, "webhook_url").await?;
        let secret = Zeroizing::new(format!("nyx_bwh_{}", generate_random_token()));
        let key_id = webhook_delivery_service::generate_signing_key_id();
        let encrypted = keys.encrypt(secret.as_bytes()).await?;
        signing_secret = Some(secret);
        signing_key_id = Some(key_id.clone());
        Some(Receiver::Direct {
            url,
            secret_encrypted: encrypted,
            key_id,
        })
    } else {
        None
    };
    let token = Zeroizing::new(format!("{TOKEN_PREFIX}{}", generate_random_token()));
    let now = Utc::now();
    let link = Link {
        id: Uuid::new_v4().to_string(),
        user_id: input.owner,
        created_by_user_id: input.actor,
        platform: input.platform,
        label: label.into(),
        requested_by: app.map(|a| a.client_name).or(input.requested_by),
        token_hash: hash(&token),
        callback_url,
        receiver,
        status: LinkStatus::Pending,
        flow: None,
        bot_id: None,
        connection_id: None,
        telegram_request_id: None,
        claim_id: None,
        claim_until: None,
        created_at: now,
        expires_at: now + Duration::seconds(input.expires_in.unwrap_or(900).clamp(60, 3600)),
        terminal_at: None,
        last_error: None,
        event_id: None,
        event_data: None,
        delivery_status: None,
        delivery_attempts: 0,
        delivery_after: None,
    };
    db.collection::<Link>(LINKS).insert_one(&link).await?;
    Ok(Created {
        link,
        token,
        signing_secret,
        signing_key_id,
    })
}

pub async fn get(db: &Database, id: &str) -> AppResult<Link> {
    db.collection::<Link>(LINKS)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or_else(missing)
}

pub async fn by_token(db: &Database, token: &str) -> AppResult<Link> {
    if !token.starts_with(TOKEN_PREFIX) || token.len() > 160 {
        return Err(missing());
    }
    db.collection::<Link>(LINKS)
        .find_one(doc! {"token_hash": hash(token)})
        .await?
        .ok_or_else(missing)
}

pub async fn authorize(db: &Database, actor: &str, link: &Link) -> AppResult<()> {
    if !org_service::resolve_owner_access(db, actor, &link.user_id)
        .await?
        .can_write()
    {
        return Err(missing());
    }
    Ok(())
}

pub async fn telegram_requires_original_actor(
    db: &Database,
    actor: &str,
    link: &Link,
) -> AppResult<bool> {
    let Some(id) = link
        .telegram_request_id
        .as_deref()
        .filter(|_| link.status == LinkStatus::Pending)
    else {
        return Ok(false);
    };
    use crate::models::telegram_bot_request::{COLLECTION_NAME, TelegramBotRequest};
    Ok(db
        .collection::<TelegramBotRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "owner_user_id": &link.user_id})
        .await?
        .is_some_and(|request| request.actor_user_id != actor))
}

fn claim_free(now: chrono::DateTime<Utc>) -> Document {
    doc! {"$or": [{"claim_until": null}, {"claim_until": {"$lte": bson::DateTime::from_chrono(now)}}]}
}

pub async fn claim(db: &Database, actor: &str, token: &str, flow: &str) -> AppResult<Claim> {
    let link = by_token(db, token).await?;
    authorize(db, actor, &link).await?;
    reconcile(db, &link).await?;
    let link = get(db, &link.id).await?;
    if link.status != LinkStatus::Pending {
        return Err(AppError::Conflict(format!(
            "Channel connect link is {}",
            status_name(link.status)
        )));
    }
    if link.flow.as_deref().is_some_and(|saved| saved != flow)
        && (link.bot_id.is_some()
            || link.connection_id.is_some()
            || link.telegram_request_id.is_some())
    {
        return Err(AppError::Conflict(
            "Resume the setup method already started for this link".into(),
        ));
    }
    let now = Utc::now();
    let mut filter = doc! {"_id": &link.id, "status": "pending", "$and": [{"$or": [
        {"flow": {"$in": [bson::Bson::Null, bson::Bson::String(flow.into())]}},
        {"bot_id": null, "connection_id": null, "telegram_request_id": null}
    ]}]};
    filter.extend(claim_free(now));
    let id = Uuid::new_v4().to_string();
    let link = db.collection::<Link>(LINKS).find_one_and_update(filter, doc! {"$set": {"claim_id": &id, "claim_until": bson::DateTime::from_chrono(now + Duration::seconds(CLAIM_SECONDS)), "flow": flow, "last_error": bson::Bson::Null}}).return_document(ReturnDocument::After).await?.ok_or_else(busy)?;
    Ok(Claim { link, id })
}

/// Renew the fenced claim while provider I/O runs. Dropping the caller does not
/// drop this future: HTTP handlers run it in an owned task.
pub async fn with_claim<T>(
    db: &Database,
    claim: &Claim,
    work: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    let mut renew = tokio::time::interval(std::time::Duration::from_secs(30));
    renew.tick().await;
    tokio::pin!(work);
    let mut lease_until = claim.link.claim_until.ok_or_else(busy)?;
    loop {
        tokio::select! {
            result = &mut work => return result,
            _ = renew.tick() => {
                let now = Utc::now();
                if now >= lease_until { return Err(busy()); }
                let result = db.collection::<Link>(LINKS).update_one(
                    doc! {"_id": &claim.link.id, "claim_id": &claim.id, "status": "pending", "claim_until": {"$gt": bson::DateTime::from_chrono(now)}},
                    doc! {"$set": {"claim_until": bson::DateTime::from_chrono(now + Duration::seconds(CLAIM_SECONDS))}},
                ).await;
                match result {
                    Ok(result) if result.matched_count == 1 => lease_until = now + Duration::seconds(CLAIM_SECONDS),
                    Ok(_) => return Err(busy()),
                    Err(error) => tracing::warn!(channel_connect_link_id = %claim.link.id, %error, "Channel setup claim renewal will retry within its lease"),
                }
            }
        }
    }
}

pub async fn release(db: &Database, claim: &Claim, failed: bool) -> AppResult<()> {
    // The method is durable only once a bot or provider request is bound. All
    // setup methods save that binding before configuring a remote bot.
    let mut set = doc! {"claim_id": bson::Bson::Null, "claim_until": bson::Bson::Null};
    if failed {
        set.insert("last_error", "setup_incomplete");
    }
    set.insert("flow", doc! {"$cond": [
        {"$and": [{"$eq": [{"$ifNull": ["$bot_id", null]}, null]}, {"$eq": [{"$ifNull": ["$connection_id", null]}, null]}, {"$eq": [{"$ifNull": ["$telegram_request_id", null]}, null]}]},
        bson::Bson::Null, "$flow"
    ]});
    db.collection::<Link>(LINKS)
        .update_one(
            doc! {"_id": &claim.link.id, "claim_id": &claim.id, "status": "pending"},
            vec![doc! {"$set": set}],
        )
        .await?;
    Ok(())
}

pub async fn pin_connection(db: &Database, claim: &Claim, connection_id: &str) -> AppResult<()> {
    let result = db
        .collection::<Link>(LINKS)
        .update_one(
            doc! {"_id": &claim.link.id, "claim_id": &claim.id, "status": "pending", "claim_until": {"$gt": bson::DateTime::now()}},
            doc! {"$set": {"connection_id": connection_id}},
        )
        .await?;
    if result.matched_count != 1 {
        return Err(busy());
    }
    Ok(())
}

pub async fn insert_telegram_request(
    db: &Database,
    claim: &Claim,
    request: &crate::models::telegram_bot_request::TelegramBotRequest,
) -> AppResult<()> {
    use super::api_key_mutation_service::{map_transaction_error, transaction_result};
    if request.id != claim.link.id
        || request.owner_user_id != claim.link.user_id
        || request.label != claim.link.label
        || claim.link.platform != "telegram-new"
    {
        return Err(missing());
    }
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let claim = claim.clone();
    let request = request.clone();
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let saved = db.collection::<Link>(LINKS).update_one(
                doc! {"_id": &claim.link.id, "claim_id": &claim.id, "status": "pending", "claim_until": {"$gt": bson::DateTime::now()}},
                doc! {"$set": {"telegram_request_id": &claim.link.id}},
            ).session(&mut *session).await?;
            if saved.matched_count != 1 { return Err(busy()); }
            db.collection::<crate::models::telegram_bot_request::TelegramBotRequest>(crate::models::telegram_bot_request::COLLECTION_NAME).insert_one(&request).session(session).await?;
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn associate(
    db: &Database,
    session: &mut ClientSession,
    claim: &Claim,
    bot: &ChannelBot,
) -> AppResult<()> {
    if bot.user_id != claim.link.user_id || bot.platform != claim.link.platform {
        return Err(missing());
    }
    let result = db.collection::<Link>(LINKS).update_one(doc! {"_id": &claim.link.id, "status": "pending", "claim_id": &claim.id, "claim_until": {"$gt": bson::DateTime::now()}, "bot_id": null}, doc! {"$set": {"bot_id": &bot.id}}).session(session).await?;
    if result.matched_count != 1 {
        return Err(busy());
    }
    Ok(())
}

pub async fn associate_telegram(
    db: &Database,
    session: &mut ClientSession,
    bot: &ChannelBot,
) -> AppResult<()> {
    if let Some(link) = db
        .collection::<Link>(LINKS)
        .find_one(doc! {"telegram_request_id": &bot.id})
        .session(&mut *session)
        .await?
    {
        if link.user_id != bot.user_id || link.platform != bot.platform {
            return Err(busy());
        }
        // Link expiry ends notification tracking, not the actor's Telegram consent.
        if link.status != LinkStatus::Pending {
            return Ok(());
        }
        db.collection::<Link>(LINKS)
            .update_one(
                doc! {"_id": link.id, "status": "pending"},
                doc! {"$set": {"bot_id": &bot.id}},
            )
            .session(session)
            .await?;
    }
    Ok(())
}

pub fn callback_url(link: &Link) -> AppResult<Option<String>> {
    if link.status == LinkStatus::Pending {
        return Ok(None);
    }
    let Some(raw) = &link.callback_url else {
        return Ok(None);
    };
    let mut url = url::Url::parse(raw)
        .map_err(|_| AppError::Internal("Invalid stored callback URL".into()))?;
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k != "status" && k != "channel_connect_link_id" && k != "bot_id")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(pairs)
        .append_pair("status", status_name(link.status))
        .append_pair("channel_connect_link_id", &link.id);
    if let Some(bot_id) = &link.bot_id {
        url.query_pairs_mut().append_pair("bot_id", bot_id);
    }
    Ok(Some(url.into()))
}

fn terminal_update(link: &Link, status: LinkStatus, bot: Option<&ChannelBot>) -> Document {
    let now = Utc::now();
    let data = doc! {
        "user_id": &link.user_id, "channel_connect_link_id": &link.id,
        "platform": &link.platform, "status": status_name(status),
        "bot_id": &link.bot_id, "bot_status": bot.map(|b| b.status.as_str()),
        "webhook_registered": bot.map(|b| b.webhook_registered),
        "is_active": bot.map(|b| b.is_active),
        "last_error": if status == LinkStatus::Completed { None } else { link.last_error.as_deref() },
        "expires_at": link.expires_at.to_rfc3339(),
    };
    doc! {"$set": {
        "last_error": if status == LinkStatus::Completed { None } else { link.last_error.as_deref() },
        "status": status_name(status), "terminal_at": bson::DateTime::from_chrono(now),
        "claim_id": bson::Bson::Null, "claim_until": bson::Bson::Null,
        "event_id": Uuid::new_v4().to_string(), "event_data": data,
        "delivery_status": if link.receiver.is_some() { "pending" } else { "none" },
        "delivery_after": bson::DateTime::from_chrono(now),
    }}
}

fn ready(link: &Link, bot: &ChannelBot) -> bool {
    if !bot.is_active
        || bot.user_id != link.user_id
        || bot.platform != link.platform
        || !matches!(bot.status.as_str(), "active" | "pending_webhook")
    {
        return false;
    }
    match link.flow.as_deref() {
        Some("manual") => true,
        Some("telegram") => bot.status == "active" && bot.webhook_registered,
        Some("managed") => match &bot.managed_setup {
            Some(setup) => {
                setup.subscription == "subscribed"
                    && setup.webhook_override == "configured"
                    && matches!(
                        setup.registration.as_str(),
                        "registered" | "already_registered" | "coexistence"
                    )
            }
            None => {
                bot.webhook_registered
                    || (bot.credential_source == "connection"
                        && bot.status == "active"
                        && bot.poll_cursor.is_some())
            }
        },
        _ => false,
    }
}

pub async fn complete(db: &Database, id: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let id = id.to_string();
    use super::api_key_mutation_service::{map_transaction_error, transaction_result};
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let Some(link) = db.collection::<Link>(LINKS).find_one(doc! {"_id": &id, "status": "pending"}).session(&mut *session).await? else { return Ok(()); };
            let Some(bot_id) = &link.bot_id else { return Err(busy()); };
            let bot = db.collection::<ChannelBot>(BOTS).find_one(doc! {"_id": bot_id, "user_id": &link.user_id, "platform": &link.platform, "is_active": true}).session(&mut *session).await?.ok_or_else(missing)?;
            if !ready(&link, &bot) { return Err(AppError::Conflict("Channel setup has not completed".into())); }
            // A write fence makes concurrent disable/delete/credential updates
            // conflict with the completion snapshot instead of racing a read.
            db.collection::<Document>(BOTS).update_one(doc! {"_id": bot_id}, doc! {"$inc": {"channel_connect_revision": 1_i64}}).session(&mut *session).await?;
            db.collection::<Link>(LINKS).update_one(doc! {"_id": &id, "status": "pending"}, terminal_update(&link, LinkStatus::Completed, Some(&bot))).session(&mut *session).await?;
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub async fn cancel(db: &Database, link: &Link) -> AppResult<Link> {
    reconcile(db, link).await?;
    let current = get(db, &link.id).await?;
    let link = &current;
    if link.status != LinkStatus::Pending {
        return Ok(link.clone());
    }
    // Once an external setup has started, cancellation cannot pretend its effects were undone.
    if link.bot_id.is_some() || link.telegram_request_id.is_some() || link.connection_id.is_some() {
        return Err(AppError::Conflict(
            "Setup has started; finish or recover this connection".into(),
        ));
    }
    let mut filter = doc! {"_id": &link.id, "status": "pending", "bot_id": null, "telegram_request_id": null, "connection_id": null, "expires_at": {"$gt": bson::DateTime::now()}};
    filter.extend(claim_free(Utc::now()));
    db.collection::<Link>(LINKS)
        .find_one_and_update(filter, terminal_update(link, LinkStatus::Cancelled, None))
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(busy)
}

pub async fn reconcile(db: &Database, link: &Link) -> AppResult<()> {
    if link.status != LinkStatus::Pending {
        return Ok(());
    }
    let now = Utc::now();
    if link.claim_until.is_some_and(|until| until > now) {
        return Ok(());
    }
    let mut telegram_connected = false;
    if let Some(id) = &link.telegram_request_id {
        use crate::models::telegram_bot_request::{
            COLLECTION_NAME as REQUESTS, TelegramBotRequest, TelegramRequestStatus as S,
        };
        if let Some(request) = db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"_id": id, "owner_user_id": &link.user_id})
            .await?
        {
            if matches!(request.status, S::Cancelled | S::Expired) && link.bot_id.is_none() {
                let status = if request.status == S::Cancelled {
                    LinkStatus::Cancelled
                } else {
                    LinkStatus::Expired
                };
                let mut filter = doc! {"_id": &link.id, "status": "pending", "bot_id": null};
                filter.extend(claim_free(now));
                db.collection::<Link>(LINKS)
                    .update_one(filter, terminal_update(link, status, None))
                    .await?;
                return Ok(());
            }
            telegram_connected = request.status == S::Connected;
        }
    }
    if let Some(id) = &link.bot_id {
        let bot = db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id": id, "user_id": &link.user_id})
            .await?;
        if bot.as_ref().is_some_and(|bot| ready(link, bot))
            && (link.flow.as_deref() != Some("telegram") || telegram_connected)
        {
            match complete(db, &link.id).await {
                Ok(()) => return Ok(()),
                Err(AppError::NotFound(_) | AppError::Conflict(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let pinned =
        link.bot_id.is_some() || link.connection_id.is_some() || link.telegram_request_id.is_some();
    let deadline = link.expires_at + Duration::seconds(if pinned { GRACE_SECONDS } else { 0 });
    if now >= deadline {
        let mut filter = doc! {"_id": &link.id, "status": "pending", "bot_id": &link.bot_id, "connection_id": &link.connection_id, "telegram_request_id": &link.telegram_request_id};
        filter.extend(claim_free(now));
        db.collection::<Link>(LINKS)
            .update_one(filter, terminal_update(link, LinkStatus::Expired, None))
            .await?;
    }
    Ok(())
}

pub async fn dispatch(
    db: &Database,
    keys: &EncryptionKeys,
    dispatcher: &DeveloperWebhookDispatcher,
    id: &str,
) -> AppResult<()> {
    let now = Utc::now();
    let Some(link) = db.collection::<Link>(LINKS).find_one_and_update(
        doc! {"_id": id, "delivery_status": "pending", "delivery_attempts": {"$lt": MAX_CYCLES}, "delivery_after": {"$lte": bson::DateTime::from_chrono(now)}},
        doc! {"$set": {"delivery_after": bson::DateTime::from_chrono(now + Duration::minutes(5))}, "$inc": {"delivery_attempts": 1}},
    ).return_document(ReturnDocument::After).await? else { return Ok(()); };
    let event_id = link
        .event_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("Missing channel event ID".into()))?;
    let event_type = format!("channel_connect.{}", status_name(link.status));
    let data = serde_json::to_value(&link.event_data)
        .map_err(|_| AppError::Internal("Invalid channel event".into()))?;
    let result = match &link.receiver {
        Some(Receiver::App { app_id }) => {
            dispatcher
                .deliver_for_app_hardened(
                    db,
                    app_id,
                    event_id,
                    &event_type,
                    data,
                    link.terminal_at.ok_or_else(|| {
                        AppError::Internal("Missing channel terminal time".into())
                    })?,
                )
                .await
        }
        Some(Receiver::Direct { url, .. }) => {
            async {
                let client = super::channel_media_service::media_client(url, None, None)
                    .await
                    .map_err(|_| delivery_failure("destination_unavailable"))?;
                deliver_direct(&client, keys, &link).await
            }
            .await
        }
        None => Ok(()),
    };
    let status = if result.is_ok() {
        "delivered"
    } else if link.delivery_attempts >= MAX_CYCLES {
        "abandoned"
    } else {
        "pending"
    };
    db.collection::<Link>(LINKS)
        .update_one(
            doc! {"_id": id, "event_id": event_id, "delivery_attempts": link.delivery_attempts},
            doc! {"$set": {"delivery_status": status}},
        )
        .await?;
    if let Err(failure) = result {
        tracing::warn!(
            channel_connect_link_id = id,
            event_id,
            reason = failure.reason,
            delivery_status = status,
            "Channel setup webhook delivery failed"
        );
    }
    Ok(())
}

async fn deliver_direct(
    client: &reqwest::Client,
    keys: &EncryptionKeys,
    link: &Link,
) -> Result<(), webhook_delivery_service::DeliveryFailure> {
    let Some(Receiver::Direct {
        url,
        secret_encrypted,
        key_id,
    }) = &link.receiver
    else {
        return Err(delivery_failure("missing_receiver"));
    };
    let event_id = link
        .event_id
        .as_deref()
        .ok_or_else(|| delivery_failure("missing_event"))?;
    let event_type = format!("channel_connect.{}", status_name(link.status));
    let secret = Zeroizing::new(
        keys.decrypt(secret_encrypted)
            .await
            .map_err(|_| delivery_failure("secret_unavailable"))?,
    );
    let body = serde_json::to_vec(&serde_json::json!({"event_id": event_id, "event_type": event_type, "occurred_at": link.terminal_at, "data": link.event_data})).map_err(|_| delivery_failure("serialization_failed"))?;
    if body.len() > 16 * 1024 {
        return Err(delivery_failure("body_too_large"));
    }
    webhook_delivery_service::deliver_signed_body(
        client,
        url,
        &secret,
        &event_type,
        event_id,
        &body,
        webhook_delivery_service::SignatureContract::Timestamped,
        Some(key_id),
    )
    .await
}

fn delivery_failure(reason: &'static str) -> webhook_delivery_service::DeliveryFailure {
    webhook_delivery_service::DeliveryFailure {
        attempts: 0,
        reason,
        last_status: None,
    }
}

pub async fn sweep(
    db: &Database,
    keys: &EncryptionKeys,
    dispatcher: &DeveloperWebhookDispatcher,
) -> AppResult<()> {
    // Walk a fixed-time snapshot by ID so a stuck old setup cannot starve later
    // links. Network deliveries are bounded independently of connector links.
    let now = bson::DateTime::now();
    let mut after = String::new();
    loop {
        let links: Vec<Link> = db
            .collection::<Link>(LINKS)
            .find(doc! {"_id": {"$gt": &after}, "$or": [
                {"status": "pending", "expires_at": {"$lte": now}},
                {"status": "pending", "bot_id": {"$type": "string"}},
                {"delivery_status": "pending", "delivery_after": {"$lte": now}},
            ]})
            .sort(doc! {"_id": 1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        let Some(last) = links.last() else {
            break;
        };
        after = last.id.clone();
        futures::stream::iter(links).for_each_concurrent(8, |link| async move {
            let result: AppResult<()> = async {
                reconcile(db, &link).await?;
                if link.delivery_attempts >= MAX_CYCLES && link.delivery_after.is_some_and(|after| after <= Utc::now()) {
                    db.collection::<Link>(LINKS).update_one(doc! {"_id": &link.id, "delivery_status": "pending", "delivery_attempts": {"$gte": MAX_CYCLES}}, doc! {"$set": {"delivery_status": "abandoned"}}).await?;
                } else { dispatch(db, keys, dispatcher, &link.id).await?; }
                Ok(())
            }.await;
            if let Err(error) = result { tracing::warn!(channel_connect_link_id = %link.id, %error, "Channel setup recovery failed"); }
        }).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_transaction_test_database, test_encryption_keys};

    async fn fixture(db: &Database) -> Created {
        let actor = Uuid::new_v4().to_string();
        create(
            db,
            &test_encryption_keys(),
            CreateInput {
                owner: actor.clone(),
                actor,
                platform: "discord".into(),
                label: "Support".into(),
                requested_by: Some("Test app".into()),
                app_id: None,
                callback_url: Some(
                    "https://example.com/return?state=original&status=spoof&bot_id=spoof".into(),
                ),
                webhook_url: None,
                expires_in: None,
            },
        )
        .await
        .unwrap()
    }

    fn bot(link: &Link) -> ChannelBot {
        bson::from_document(doc! {
            "_id": &link.id, "user_id": &link.user_id, "platform": &link.platform,
            "label": &link.label, "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1, 2, 3] },
            "platform_bot_id": Uuid::new_v4().to_string(), "platform_bot_username": "support",
            "webhook_registered": false, "webhook_secret_hash": "never-in-event",
            "status": "pending_webhook", "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        }).unwrap()
    }

    #[tokio::test]
    async fn channel_link_persists_hash_and_bson_dates_and_hides_private_preview() {
        let db = connect_transaction_test_database("channel_link_storage").await;
        ensure_indexes(&db).await.unwrap();
        let created = fixture(&db).await;
        let stored = db
            .collection::<Document>(LINKS)
            .find_one(doc! {"_id": &created.link.id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.get_datetime("created_at").is_ok());
        assert!(stored.get_datetime("expires_at").is_ok());
        assert!(!format!("{stored:?}").contains(created.token.as_str()));
        assert!(!format!("{:?}", created.link).contains(&created.link.token_hash));
        assert_eq!(
            by_token(&db, &created.token).await.unwrap().id,
            created.link.id
        );
        assert!(
            authorize(&db, &Uuid::new_v4().to_string(), &created.link)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn channel_link_claim_is_exclusive_and_stale_writer_cannot_insert_a_bot() {
        let db = connect_transaction_test_database("channel_link_claim").await;
        let c = fixture(&db).await;
        let (a, b) = tokio::join!(
            claim(&db, &c.link.user_id, &c.token, "manual"),
            claim(&db, &c.link.user_id, &c.token, "manual")
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let first = a.or(b).unwrap();
        assert!(cancel(&db, &c.link).await.is_err());
        db.collection::<Link>(LINKS).update_one(doc! {"_id": &c.link.id}, doc! {"$set": {"claim_until": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        let next = claim(&db, &c.link.user_id, &c.token, "manual")
            .await
            .unwrap();
        let bot = bot(&c.link);
        assert!(
            super::super::channel_bot_service::insert_registered_bot_linked(
                &db,
                &bot,
                10,
                None,
                Some(&first)
            )
            .await
            .is_err()
        );
        assert_eq!(
            db.collection::<ChannelBot>(BOTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        super::super::channel_bot_service::insert_registered_bot_linked(
            &db,
            &bot,
            10,
            None,
            Some(&next),
        )
        .await
        .unwrap();
        assert_eq!(get(&db, &c.link.id).await.unwrap().bot_id, Some(bot.id));
        assert!(
            cancel(&db, &get(&db, &c.link.id).await.unwrap())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn channel_link_completion_atomically_reserves_one_credential_free_event() {
        let db = connect_transaction_test_database("channel_link_complete").await;
        let c = fixture(&db).await;
        let claimed = claim(&db, &c.link.user_id, &c.token, "manual")
            .await
            .unwrap();
        super::super::channel_bot_service::insert_registered_bot_linked(
            &db,
            &bot(&c.link),
            10,
            None,
            Some(&claimed),
        )
        .await
        .unwrap();
        let (a, b) = tokio::join!(complete(&db, &c.link.id), complete(&db, &c.link.id));
        a.unwrap();
        b.unwrap();
        let saved = get(&db, &c.link.id).await.unwrap();
        assert_eq!(saved.status, LinkStatus::Completed);
        let event_id = saved.event_id.clone().unwrap();
        assert!(saved.terminal_at.is_some());
        assert!(saved.claim_id.is_none());
        let data = saved.event_data.as_ref().unwrap();
        assert_eq!(data.get_str("bot_id").unwrap(), c.link.id);
        assert_eq!(data.get_str("bot_status").unwrap(), "pending_webhook");
        assert!(!data.get_bool("webhook_registered").unwrap());
        let text = serde_json::to_string(data).unwrap();
        for forbidden in ["token", "secret", "connection_id", "callback_url"] {
            assert!(!text.contains(forbidden));
        }
        complete(&db, &c.link.id).await.unwrap();
        assert_eq!(
            get(&db, &c.link.id).await.unwrap().event_id.as_deref(),
            Some(event_id.as_str())
        );
        let url = url::Url::parse(&callback_url(&saved).unwrap().unwrap()).unwrap();
        let pairs = url.query_pairs().collect::<Vec<_>>();
        assert_eq!(pairs.iter().filter(|(k, _)| k == "status").count(), 1);
        assert!(pairs.iter().any(|(k, v)| k == "state" && v == "original"));
        assert!(!url.as_str().contains("spoof"));
        assert!(
            claim(&db, &c.link.user_id, &c.token, "manual")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn channel_link_cancel_and_expiry_reserve_events_without_polling_or_starvation() {
        let db = connect_transaction_test_database("channel_link_sweep").await;
        let c = fixture(&db).await;
        let cancelled = cancel(&db, &c.link).await.unwrap();
        assert_eq!(cancelled.status, LinkStatus::Cancelled);
        assert!(cancelled.event_id.is_some());
        assert!(
            claim(&db, &c.link.user_id, &c.token, "manual")
                .await
                .is_err()
        );
        let mut blocked = Vec::new();
        for i in 0..101 {
            let mut link = c.link.clone();
            link.id = format!("00000000-0000-4000-8000-{i:012}");
            link.expires_at = Utc::now() - Duration::hours(1);
            link.claim_until = Some(Utc::now() + Duration::minutes(10));
            blocked.push(link);
        }
        let mut expired = c.link.clone();
        expired.id = "ffffffff-ffff-4fff-8fff-ffffffffffff".into();
        expired.expires_at = Utc::now() - Duration::seconds(1);
        blocked.push(expired.clone());
        db.collection::<Link>(LINKS)
            .insert_many(&blocked)
            .await
            .unwrap();
        let keys = std::sync::Arc::new(test_encryption_keys());
        let dispatcher = DeveloperWebhookDispatcher::new(reqwest::Client::new(), keys.clone());
        sweep(&db, &keys, &dispatcher).await.unwrap();
        let saved = get(&db, &expired.id).await.unwrap();
        assert_eq!(saved.status, LinkStatus::Expired);
        assert!(saved.event_id.is_some());
        assert_eq!(
            get(&db, &blocked[0].id).await.unwrap().status,
            LinkStatus::Pending
        );
    }

    #[tokio::test]
    async fn channel_link_delivery_is_leased_and_stops_after_bounded_retries() {
        let db = connect_transaction_test_database("channel_link_delivery").await;
        let mut c = fixture(&db).await;
        let keys = std::sync::Arc::new(test_encryption_keys());
        // Simulate a previously public destination becoming private at delivery.
        c.link.receiver = Some(Receiver::Direct {
            url: "https://127.0.0.1/callback".into(),
            secret_encrypted: keys.encrypt(b"test-signing-secret").await.unwrap(),
            key_id: "test-key-id".into(),
        });
        db.collection::<Link>(LINKS)
            .replace_one(doc! {"_id": &c.link.id}, &c.link)
            .await
            .unwrap();
        let terminal = cancel(&db, &c.link).await.unwrap();
        let dispatcher = DeveloperWebhookDispatcher::new(reqwest::Client::new(), keys.clone());
        let (a, b) = tokio::join!(
            dispatch(&db, &keys, &dispatcher, &c.link.id),
            dispatch(&db, &keys, &dispatcher, &c.link.id)
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(get(&db, &c.link.id).await.unwrap().delivery_attempts, 1);
        for _ in 1..MAX_CYCLES {
            db.collection::<Link>(LINKS).update_one(doc! {"_id": &c.link.id}, doc! {"$set": {"delivery_after": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
            dispatch(&db, &keys, &dispatcher, &c.link.id).await.unwrap();
        }
        let saved = get(&db, &c.link.id).await.unwrap();
        assert_eq!(saved.delivery_attempts, MAX_CYCLES);
        assert_eq!(saved.delivery_status.as_deref(), Some("abandoned"));
        assert_eq!(saved.event_id, terminal.event_id);
        assert_eq!(saved.event_data, terminal.event_data);
    }

    #[tokio::test]
    async fn channel_link_rejects_private_webhooks_before_inserting() {
        let db = connect_transaction_test_database("channel_link_urls").await;
        for url in [
            "http://example.com/hook",
            "https://127.0.0.1/hook",
            "https://169.254.169.254/",
        ] {
            assert!(
                create(
                    &db,
                    &test_encryption_keys(),
                    CreateInput {
                        owner: "actor".into(),
                        actor: "actor".into(),
                        platform: "discord".into(),
                        label: "Support".into(),
                        requested_by: None,
                        app_id: None,
                        callback_url: None,
                        webhook_url: Some(url.into()),
                        expires_in: None,
                    }
                )
                .await
                .is_err()
            );
        }
        assert_eq!(
            db.collection::<Link>(LINKS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn channel_link_failed_preflight_can_change_method_but_bound_setup_cannot() {
        let db = connect_transaction_test_database("channel_link_method").await;
        let c = fixture(&db).await;
        let manual = claim(&db, &c.link.user_id, &c.token, "manual")
            .await
            .unwrap();
        db.collection::<Document>(LINKS)
            .update_one(
                doc! {"_id": &c.link.id},
                doc! {"$unset": {"bot_id": "", "connection_id": "", "telegram_request_id": ""}},
            )
            .await
            .unwrap();
        release(&db, &manual, true).await.unwrap();
        assert!(get(&db, &c.link.id).await.unwrap().flow.is_none());
        let managed = claim(&db, &c.link.user_id, &c.token, "managed")
            .await
            .unwrap();
        assert!(managed.link.last_error.is_none());
        pin_connection(&db, &managed, "provider-connection")
            .await
            .unwrap();
        release(&db, &managed, true).await.unwrap();
        assert!(
            claim(&db, &c.link.user_id, &c.token, "manual")
                .await
                .is_err()
        );
        assert!(
            claim(&db, &c.link.user_id, &c.token, "managed")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn channel_link_crashed_unpinned_claim_can_change_method() {
        let db = connect_transaction_test_database("channel_link_crashed_claim").await;
        let c = fixture(&db).await;
        let stale = claim(&db, &c.link.user_id, &c.token, "manual")
            .await
            .unwrap();
        assert!(
            claim(&db, &c.link.user_id, &c.token, "managed")
                .await
                .is_err()
        );
        db.collection::<Link>(LINKS).update_one(doc! {"_id": &c.link.id}, doc! {"$set": {"claim_until": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        assert!(pin_connection(&db, &stale, "too-late").await.is_err());
        let next = claim(&db, &c.link.user_id, &c.token, "managed")
            .await
            .unwrap();
        release(&db, &stale, true).await.unwrap();
        assert_eq!(
            get(&db, &c.link.id).await.unwrap().claim_id.as_deref(),
            Some(next.id.as_str())
        );
    }

    #[tokio::test]
    async fn channel_link_managed_ready_accepts_supported_success_variants_only() {
        let db = connect_transaction_test_database("channel_link_ready_states").await;
        let mut c = fixture(&db).await;
        c.link.flow = Some("managed".into());
        let mut bot = bot(&c.link);
        for registration in [
            "registered",
            "already_registered",
            "coexistence",
            "failed",
            "pending",
        ] {
            for subscription in ["subscribed", "failed", "pending"] {
                for webhook_override in ["configured", "failed", "pending"] {
                    bot.managed_setup = Some(crate::models::channel_bot::ManagedBotSetup {
                        registration: registration.into(),
                        subscription: subscription.into(),
                        webhook_override: webhook_override.into(),
                        ..Default::default()
                    });
                    assert_eq!(
                        ready(&c.link, &bot),
                        matches!(
                            registration,
                            "registered" | "already_registered" | "coexistence"
                        ) && subscription == "subscribed"
                            && webhook_override == "configured",
                        "{registration}/{subscription}/{webhook_override}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn channel_link_direct_delivery_signs_stable_terminal_snapshot() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(204))
            .expect(2)
            .mount(&server)
            .await;
        let db = connect_transaction_test_database("channel_link_signed_callback").await;
        let c = fixture(&db).await;
        let keys = test_encryption_keys();
        let secret = "nyx_bwh_test_only";
        let mut terminal = cancel(&db, &c.link).await.unwrap();
        terminal.receiver = Some(Receiver::Direct {
            url: server.uri(),
            secret_encrypted: keys.encrypt(secret.as_bytes()).await.unwrap(),
            key_id: "test-key-id".into(),
        });
        // Exercise the production serializer, decryption and signed sender with a local receiver.
        // Destination validation is covered separately; production constructs a DNS-pinned client.
        for _ in 0..2 {
            deliver_direct(&reqwest::Client::new(), &keys, &terminal)
                .await
                .unwrap();
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].body, requests[1].body);
        for request in requests {
            let timestamp = request.headers["x-nyxid-timestamp"].to_str().unwrap();
            let signature = webhook_delivery_service::compute_timestamped_signature(
                secret.as_bytes(),
                timestamp,
                &request.body,
            );
            assert_eq!(
                request.headers["x-nyxid-signature"],
                format!("sha256={signature}")
            );
            assert_eq!(
                request.headers["x-nyxid-event"],
                "channel_connect.cancelled"
            );
            assert_eq!(
                request.headers["x-nyxid-delivery-id"],
                terminal.event_id.as_deref().unwrap()
            );
            assert_eq!(request.headers["x-nyxid-key-id"], "test-key-id");
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["occurred_at"], serde_json::json!(terminal.terminal_at));
            assert_eq!(body["data"]["channel_connect_link_id"], c.link.id);
            assert!(!String::from_utf8(request.body).unwrap().contains(secret));
        }
    }

    #[tokio::test]
    async fn channel_link_failed_managed_setup_stays_recoverable_until_required_steps_succeed() {
        let db = connect_transaction_test_database("channel_link_managed_ready").await;
        let c = fixture(&db).await;
        let claimed = claim(&db, &c.link.user_id, &c.token, "managed")
            .await
            .unwrap();
        let mut bot = bot(&c.link);
        bot.managed_setup = Some(crate::models::channel_bot::ManagedBotSetup {
            subscription: "failed".into(),
            webhook_override: "pending".into(),
            registration: "registered".into(),
            ..Default::default()
        });
        super::super::channel_bot_service::insert_registered_bot_linked(
            &db,
            &bot,
            10,
            None,
            Some(&claimed),
        )
        .await
        .unwrap();
        assert!(complete(&db, &c.link.id).await.is_err());
        release(&db, &claimed, true).await.unwrap();
        reconcile(&db, &get(&db, &c.link.id).await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            get(&db, &c.link.id).await.unwrap().status,
            LinkStatus::Pending
        );
        db.collection::<ChannelBot>(BOTS).update_one(doc! {"_id": &bot.id}, doc! {"$set": {"managed_setup.subscription": "subscribed", "managed_setup.webhook_override": "configured", "managed_setup.registration": "already_registered"}}).await.unwrap();
        reconcile(&db, &get(&db, &c.link.id).await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            get(&db, &c.link.id).await.unwrap().status,
            LinkStatus::Completed
        );
    }

    #[tokio::test]
    async fn channel_link_cancel_after_deadline_reports_expired() {
        let db = connect_transaction_test_database("channel_link_cancel_expired").await;
        let c = fixture(&db).await;
        db.collection::<Link>(LINKS).update_one(doc! {"_id": &c.link.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        let result = cancel(&db, &get(&db, &c.link.id).await.unwrap())
            .await
            .unwrap();
        assert_eq!(result.status, LinkStatus::Expired);
        assert_eq!(
            result.event_data.unwrap().get_str("status").unwrap(),
            "expired"
        );
    }

    #[tokio::test]
    async fn channel_link_deleted_telegram_bot_does_not_block_expiry() {
        let db = connect_transaction_test_database("channel_link_deleted_bot").await;
        let mut c = fixture(&db).await;
        c.link.platform = "telegram-new".into();
        c.link.flow = Some("telegram".into());
        c.link.telegram_request_id = Some(c.link.id.clone());
        c.link.bot_id = Some(c.link.id.clone());
        c.link.expires_at = Utc::now() - Duration::hours(1);
        db.collection::<Link>(LINKS)
            .replace_one(doc! {"_id": &c.link.id}, &c.link)
            .await
            .unwrap();
        let mut bot = bot(&c.link);
        bot.is_active = false;
        db.collection::<ChannelBot>(BOTS)
            .insert_one(bot)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME).insert_one(doc! {
            "_id": &c.link.id, "actor_user_id": &c.link.user_id, "owner_user_id": &c.link.user_id,
            "manager_bot_id": 100_i64, "observation_id": "obs", "label": "Support", "destination": "personal",
            "status": "connected", "active": false, "revision": 1_i64, "challenge_hash": "hash",
            "expires_at": bson::DateTime::now(), "created_at": bson::DateTime::now(),
        }).await.unwrap();
        reconcile(&db, &c.link).await.unwrap();
        assert_eq!(
            get(&db, &c.link.id).await.unwrap().status,
            LinkStatus::Expired
        );
    }

    #[tokio::test]
    async fn channel_link_stalled_telegram_expires_and_preserves_late_worker_consent() {
        let db = connect_transaction_test_database("channel_link_stalled_telegram").await;
        for state in ["ready", "provisioning", "expired", "cancelled"] {
            let mut c = fixture(&db).await;
            c.link.platform = "telegram-new".into();
            c.link.flow = Some("telegram".into());
            c.link.telegram_request_id = Some(c.link.id.clone());
            let running = matches!(state, "ready" | "provisioning");
            c.link.bot_id = running.then(|| c.link.id.clone());
            c.link.expires_at = Utc::now() + Duration::minutes(if running { -31 } else { 60 });
            db.collection::<Link>(LINKS)
                .replace_one(doc! {"_id": &c.link.id}, &c.link)
                .await
                .unwrap();
            let bot = bot(&c.link);
            if running {
                db.collection::<ChannelBot>(BOTS)
                    .insert_one(&bot)
                    .await
                    .unwrap();
            }
            db.collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME).insert_one(doc! {
                "_id": &c.link.id, "actor_user_id": &c.link.user_id, "owner_user_id": &c.link.user_id,
                "manager_bot_id": 100_i64, "observation_id": "obs", "label": "Support", "destination": "personal",
                "status": state, "active": running, "revision": 1_i64, "challenge_hash": "hash",
                "expires_at": bson::DateTime::now(), "created_at": bson::DateTime::now(),
            }).await.unwrap();
            assert!(
                !telegram_requires_original_actor(&db, &c.link.user_id, &c.link)
                    .await
                    .unwrap()
            );
            assert!(
                telegram_requires_original_actor(&db, "another-admin", &c.link)
                    .await
                    .unwrap()
            );
            reconcile(&db, &c.link).await.unwrap();
            let terminal = get(&db, &c.link.id).await.unwrap();
            assert_eq!(
                terminal.status,
                if state == "cancelled" {
                    LinkStatus::Cancelled
                } else {
                    LinkStatus::Expired
                }
            );
            if running {
                assert_eq!(
                    terminal
                        .event_data
                        .as_ref()
                        .unwrap()
                        .get_str("bot_id")
                        .unwrap(),
                    bot.id
                );
            }
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            associate_telegram(&db, &mut session, &bot).await.unwrap();
            session.commit_transaction().await.unwrap();
            complete(&db, &c.link.id).await.unwrap();
            let after = get(&db, &c.link.id).await.unwrap();
            assert_eq!(after.event_id, terminal.event_id);
            assert_eq!(after.status, terminal.status);
        }
    }
}
