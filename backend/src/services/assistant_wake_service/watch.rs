//! Preparation claims are fenced; expired claims never repeat gateway effects.
use super::*;
use crate::models::assistant_wake::WatchWake;
use crate::models::nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME as WATCHES};
use chrono::{Duration, Utc};

pub const CLAIM_SECS: i64 = 120;

pub fn snapshot(watch: &NyxbotWatch) -> bson::Document {
    let mut filter = doc! {"_id":&watch.id,"user_id":&watch.user_id,"kind":&watch.kind,
    "conversation_id":&watch.conversation_id,"platform":&watch.platform,
    "agent_id":&watch.agent_id,"connect_link_id":&watch.connect_link_id,
    "created_at":bson::DateTime::from_chrono(watch.created_at),
    "expires_at":bson::DateTime::from_chrono(watch.expires_at),
    "report_to":&watch.report_to,"report_provenance":watch.report_provenance as i32};
    if watch.kind == "channel_bot" {
        filter.insert("source_turn_id", watch.source_turn_id.clone());
        filter.insert(
            "source_connect_link_id",
            watch.source_connect_link_id.clone(),
        );
    }
    if watch.report_provenance == 0 {
        filter.insert("report_provenance", doc! {"$in":[0,bson::Bson::Null]});
    }
    filter
}

fn event_id(watch: &NyxbotWatch) -> AppResult<String> {
    let claim = watch
        .wake_claim
        .as_ref()
        .ok_or_else(|| AppError::Internal("Missing watch claim".into()))?;
    Ok(format!("watch:{}:{}", watch.id, claim.id))
}

fn kind(watch: &NyxbotWatch) -> AppResult<&'static str> {
    match watch.kind.as_str() {
        "channel_bot" => Ok("channel_bot_linked"),
        "connect_link" => Ok("connection_finished"),
        "trigger_created" => Ok("trigger_created"),
        _ => Err(AppError::Internal("Unsupported watch claim".into())),
    }
}

async fn reference(
    db: &Database,
    watch: &NyxbotWatch,
    error: Option<String>,
    ready_at: chrono::DateTime<Utc>,
    tx: &mut ClientSession,
    base_url: Option<&str>,
) -> AppResult<WakeReference> {
    let source_status = if watch.kind == "connect_link" {
        document(
            db,
            crate::models::connect_link::COLLECTION_NAME,
            doc! {"_id":&watch.connect_link_id,"user_id":&watch.user_id},
            tx,
        )
        .await?
        .and_then(|d| d.get_str("status").ok().map(str::to_owned))
    } else {
        None
    };
    let channel = if watch.kind == "channel_bot" {
        document(db,crate::models::nyxbot_channel::COLLECTION_NAME,doc! {"user_id":&watch.user_id,"channel_bot_id":&watch.channel_bot_id,"agent_id":&watch.agent_id,"status":"active"},tx).await?
    } else {
        None
    };
    let channel_id = channel
        .as_ref()
        .and_then(|row| row.get_str("_id").ok())
        .map(str::to_owned);
    let expected_callback = channel
        .as_ref()
        .filter(|row| {
            row.get_str("transport") == Ok("direct")
                && row
                    .get_str("platform")
                    .is_ok_and(super::super::channel_adapters::meta::is_meta)
        })
        .and(base_url)
        .zip(channel_id.as_deref())
        .map(|(base, id)| super::super::nyxbot_channel_query_service::direct_callback(base, id));
    Ok(WakeReference::Watch(Box::new(WatchWake {
        watch_id: watch.id.clone(),
        claim_id: watch
            .wake_claim
            .as_ref()
            .ok_or_else(|| AppError::Internal("Missing watch claim".into()))?
            .id
            .clone(),
        ready_at,
        watch_kind: watch.kind.clone(),
        source_id: watch.connect_link_id.clone(),
        bot_id: watch.channel_bot_id.clone(),
        channel_id,
        expected_callback,
        agent_id: watch.agent_id.clone(),
        reason: error,
        source_status,
        source_turn_id: watch.source_turn_id.clone(),
        source_connect_link_id: watch.source_connect_link_id.clone(),
        conversation_id: watch.conversation_id.clone(),
        created_at: watch.created_at,
    })))
}

pub async fn claim(
    db: &Database,
    watch: &NyxbotWatch,
    extra: bson::Document,
) -> AppResult<Option<NyxbotWatch>> {
    let mut set: bson::Document = extra
        .into_iter()
        .map(|(key, value)| (key, bson::Bson::Document(doc! {"$literal":value})))
        .collect();
    set.insert("status", "claimed");
    set.insert(
        "wake_claim",
        doc! {"id":uuid::Uuid::new_v4().to_string(),
        "expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":CLAIM_SECS}}},
    );
    let context = (db.clone(), watch.clone(), set);
    super::super::transaction_service::run_db_only(db, &context, |tx, (db, watch, extra)| {
        Box::pin(async move {
            let mut filter = snapshot(watch);
            filter.insert("status", "pending");
            let Some(claimed) = db
                .collection::<NyxbotWatch>(WATCHES)
                .find_one_and_update(filter, vec![doc! {"$set":extra}])
                .return_document(mongodb::options::ReturnDocument::After)
                .session(&mut *tx)
                .await?
            else {
                return Ok(None);
            };
            let until = claimed
                .wake_claim
                .as_ref()
                .ok_or_else(|| AppError::Internal("Missing watch claim".into()))?
                .expires_at;
            let mut event = super::super::nyxbot_watch_service::watch_event(
                &claimed,
                kind(&claimed)?,
                String::new(),
            );
            event.id = event_id(&claimed)?;
            // A TTL-deleted watch must still report uncertain preparation,
            // rather than claiming that a gateway operation completed.
            let unfinished =
                (claimed.kind == "channel_bot").then(|| "wake_preparation_interrupted".to_owned());
            event.wake = Some(reference(db, &claimed, unfinished, until, tx, None).await?);
            // A deleted destination ends only its notification, never setup.
            // Queue pressure still returns an error and rolls back the claim.
            super::super::assistant_nyxagent::append_events_in_session(
                db,
                &claimed.user_id,
                &claimed.conversation_id,
                &[event],
                tx,
            )
            .await?;
            Ok(Some(claimed))
        })
    })
    .await
}

pub async fn settle(
    db: &Database,
    watch: &NyxbotWatch,
    kind: &str,
    error: Option<&str>,
    base_url: Option<&str>,
) -> AppResult<bool> {
    let context = (
        db.clone(),
        watch.clone(),
        kind.to_owned(),
        error.map(str::to_owned),
        base_url.map(str::to_owned),
    );
    super::super::transaction_service::run_db_only(db, &context, |tx, (db, watch, kind, error, base_url)| {
        Box::pin(async move {
            let mut filter = snapshot(watch);
            filter.insert("status", "claimed");
            filter.insert("wake_claim.id", watch.wake_claim.as_ref().map(|c| c.id.clone()));
            let result = db.collection::<NyxbotWatch>(WATCHES).update_one(
                filter,
                doc! {"$set":{"status":if error.is_some(){"failed"}else{"done"},"last_error":error}},
            ).session(&mut *tx).await?;
            if result.modified_count != 1 {
                return Ok(false);
            }
            let reference = reference(db, watch, error.clone(), watch.created_at, tx, base_url.as_deref()).await?;
            let reference = bson::to_bson(&reference).map_err(|_| AppError::Internal("Invalid wake reference".into()))?;
            // This intent already exists. Never recreate an event which admission
            // consumed after the preparation lease expired.
            db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&watch.conversation_id,"user_id":&watch.user_id,"pending_events.id":event_id(watch)?},
                    update_event(&event_id(watch)?, doc! {"kind":{"$literal":kind},
                        "wake":{"$mergeObjects":[{"$literal":reference},{"ready_at":"$$NOW"}]}}),
                ).session(tx).await?;
            Ok(true)
        })
    }).await
}

/// Expired preparation is settled in the same transaction that consumes its
/// wake. A late original worker then loses the exact claimed-state predicate.
/// A replaced or TTL-deleted source cannot erase the committed wake intent.
pub async fn expire(
    db: &Database,
    owner: &str,
    id: &str,
    claim: &str,
    tx: &mut ClientSession,
) -> AppResult<bool> {
    Ok(db
        .collection::<NyxbotWatch>(WATCHES)
        .update_one(
            doc! {"_id":id,"user_id":owner,"status":"claimed","wake_claim.id":claim,
            "$expr":{"$lte":["$wake_claim.expires_at","$$NOW"]}},
            doc! {"$set":{"status":"failed","last_error":"wake_preparation_interrupted"}},
        )
        .session(tx)
        .await?
        .modified_count
        == 1)
}

/// Mint the owner step only when the wake is admitted, so no code is retained
/// in the pending event. The hash and the consuming turn commit together.
pub async fn verification(
    db: &Database,
    owner: &str,
    channel: &str,
    bot: &str,
    agent: &str,
    tx: &mut ClientSession,
    expected_callback: Option<&str>,
) -> AppResult<String> {
    use crate::models::nyxbot_channel::{COLLECTION_NAME, LINK_CODE_TTL_HOURS, NyxbotChannel};
    use sha2::{Digest, Sha256};
    let rows = db.collection::<NyxbotChannel>(COLLECTION_NAME);
    let filter = doc! {"_id":channel,"user_id":owner,"channel_bot_id":bot,"agent_id":agent,"status":"active","org_specialist":null};
    let Some(row) = rows.find_one(filter.clone()).session(&mut *tx).await? else {
        return Ok("The bot's link changed after setup. Check its current status; do not recreate it automatically.".into());
    };
    if !row.owner_sender_ids.is_empty() {
        return Ok("Their account there is already verified. Tell them to open the bot and press Start (or send a message) to check the first reply. A reply has not been verified yet.".into());
    }
    if row.transport == "direct" && super::super::channel_adapters::meta::is_meta(&row.platform) {
        match super::super::nyxbot_channel_query_service::fence_meta_default(db,&row,expected_callback,tx).await {
            Ok(_) => {},
            Err(AppError::Conflict(_)) | Err(AppError::Forbidden(_)) => return Ok("The inbox route or setup authority changed. Assign its specialist again to repair the default callback, then request a fresh owner code. No code was issued.".into()),
            Err(error) => return Err(error),
        }
    } else {
        match super::super::platform_bot_authority::fence_personal_routes(
            db,
            bot,
            row.bot_owner_id.as_deref().unwrap_or(owner),
            tx,
        )
        .await
        {
            Ok(()) => {}
            Err(error) if error.is_forbidden() => {
                return Ok("The bot's setup authority changed. Check its current status; do not recreate it automatically.".into());
            }
            Err(error) => return Err(error),
        }
    }
    let code = new_link_code();
    let hash = hex::encode(Sha256::digest(code.as_bytes()));
    rows.update_one(filter,doc! {"$set":{"link_code_hash":hash,"link_code_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(LINK_CODE_TTL_HOURS))}}).session(tx).await?;
    Ok(match (row.platform.as_str(), row.bot_username.as_deref()) {
        ("telegram", Some(username)) if !username.is_empty() => format!(
            "Give the user its owner-verification step: https://t.me/{username}?start={}. Open the link and press Start.",
            code.as_str()
        ),
        _ => format!(
            "Give the user its owner-verification step: send this code to the bot once from their own {} account: {}",
            super::super::assistant_nyxagent::identifier(&row.platform),
            code.as_str()
        ),
    })
}

pub fn new_link_code() -> zeroize::Zeroizing<String> {
    use rand::Rng;
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let tail: String = (0..12)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();
    zeroize::Zeroizing::new(format!("nyxlink_{tail}"))
}

/// Renew only preparation: losing a renewal never authorizes another gateway
/// call. The queued reference still recovers an interrupted/uncertain outcome.
pub async fn during<F: std::future::Future>(
    db: &Database,
    watch: &NyxbotWatch,
    future: F,
) -> F::Output {
    if watch.wake_claim.is_none() {
        return future.await;
    }
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs((CLAIM_SECS / 3) as u64));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    tokio::pin!(future);
    loop {
        tokio::select! {
            result=&mut future=>return result,
            _=interval.tick()=> {
                let result = renew(db, watch).await;
                if result.is_err() {tracing::debug!("Assistant watch preparation renewal deferred");}
            }
        }
    }
}

fn update_event(id: &str, fields: bson::Document) -> Vec<bson::Document> {
    vec![
        doc! {"$set":{"pending_events":{"$map":{"input":"$pending_events","as":"event",
        "in":{"$cond":[{"$eq":["$$event.id",{"$literal":id}]},
            {"$mergeObjects":["$$event",fields]},"$$event"]}}}}},
    ]
}

pub(super) async fn renew(db: &Database, watch: &NyxbotWatch) -> AppResult<()> {
    let Some(claim) = watch.wake_claim.as_ref() else {
        return Ok(());
    };
    let context = (
        db.clone(),
        watch.user_id.clone(),
        watch.id.clone(),
        claim.id.clone(),
        watch.conversation_id.clone(),
    );
    super::super::transaction_service::run_db_only(db,&context,|tx,(db,owner,id,claim,conversation)|Box::pin(async move {
        let renewed=db.collection::<NyxbotWatch>(WATCHES).find_one_and_update(
            doc! {"_id":id,"user_id":owner,"status":"claimed","wake_claim.id":claim,
                "$expr":{"$gt":["$wake_claim.expires_at","$$NOW"]}},
            vec![doc! {"$set":{"wake_claim.expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":CLAIM_SECS}}}}],
        ).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?;
        if let Some(claim) = renewed.and_then(|watch|watch.wake_claim) {
            let event_id=format!("watch:{id}:{}",claim.id);
            // Mirror the exact time authored by MongoDB, not a second clock.
            db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(
                doc! {"_id":conversation,"user_id":owner,"pending_events.id":&event_id},
                update_event(&event_id,doc! {"wake":{"$mergeObjects":["$$event.wake",{"ready_at":bson::DateTime::from_chrono(claim.expires_at)}]}}),
            ).session(tx).await?;
        }
        Ok(())
    })).await
}
