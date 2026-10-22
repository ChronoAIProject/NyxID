//! Reconnect history is claimed only by a committed activation.
use super::{api_key_mutation_service as transactions, nyxbot_gateway_release_service as releases};
use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::{COLLECTION_NAME as CHANNELS, LINK_CODE_TTL_HOURS, NyxbotChannel},
};
use chrono::Utc;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::json;

fn canonical(platform: &str) -> &str {
    if platform == "telegram-new" {
        "telegram"
    } else {
        platform
    }
}
fn owner(row: &NyxbotChannel) -> &str {
    row.bot_owner_id.as_deref().unwrap_or(&row.user_id)
}
fn conflict() -> AppError {
    AppError::Conflict("Channel changed during activation".into())
}

async fn restore(
    db: &Database,
    row: &NyxbotChannel,
    explicit: Option<&str>,
    default_agent: bool,
    session: &mut ClientSession,
) -> AppResult<()> {
    let channels = db.collection::<NyxbotChannel>(CHANNELS);
    let mut filter = doc! {"user_id":&row.user_id,"channel_bot_id":&row.channel_bot_id,"status":"disconnected","_id":{"$ne":&row.id}};
    if let Some(id) = explicit {
        filter.insert("_id", id);
    }
    let previous = channels
        .find_one(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .session(&mut *session)
        .await?;
    let Some(previous) = previous else {
        return Ok(());
    };
    // Deliberately inspect only the newest predecessor. An incompatible newest
    // record is a boundary, not permission to recover an older history.
    // A disconnected preparation never reached activation, which stores route_id.
    if previous.route_id.is_none()
        || canonical(&previous.platform) != canonical(&row.platform)
        || owner(&previous) != owner(row)
        || previous.last_error.as_deref() == Some("platform_conversion")
    {
        return Ok(());
    }
    if previous.continuity_to_channel_id.is_some() {
        if explicit.is_some() {
            return Err(conflict());
        }
        return Ok(());
    }
    if db
        .collection::<bson::Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
        .find_one(doc! {"resource_id":&previous.id,"kind":"gateway"})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(());
    }
    if channels.update_one(doc! {"_id":&previous.id,"status":"disconnected","continuity_to_channel_id":bson::Bson::Null},
        doc! {"$set":{"continuity_to_channel_id":&row.id,"updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
    let changed =
        previous.agent_id != row.agent_id && !(previous.agent_id.is_none() && default_agent);
    let carried = super::channel_thread_follow_service::carry_over_in_session(
        db,
        &row.user_id,
        &previous.id,
        &row.id,
        changed,
        session,
    )
    .await?;
    let mut owner_sender_ids = row.owner_sender_ids.clone();
    for sender in &previous.owner_sender_ids {
        if !owner_sender_ids.contains(sender) {
            owner_sender_ids.push(sender.clone());
        }
    }
    channels.update_one(doc! {"_id":&row.id},doc! {"$set":{"owner_sender_ids":owner_sender_ids,"private_chats":&previous.private_chats,
        "continuity_from_channel_id":&previous.id,"continuity_restored_at":bson::DateTime::now()}}).session(&mut *session).await?;
    releases::audit(
        db,
        session,
        &row.user_id,
        "nyxbot_channel_continuity_restored",
        json!({"channel_id":row.id,"predecessor_id":previous.id,"agent_changed":changed,
            "moved":carried.moved,"kept_conflicting":carried.kept_conflicting}),
    )
    .await
}

pub(crate) async fn activate(
    db: &Database,
    owner_id: &str,
    id: &str,
    route: &str,
    explicit: Option<&str>,
    default_agent: bool,
    link_hash: &str,
) -> AppResult<()> {
    let db = db.clone();
    let owner_id = owner_id.to_owned();
    let id = id.to_owned();
    let route = route.to_owned();
    let explicit = explicit.map(str::to_owned);
    let link_hash = link_hash.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let row=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"_id":&id,"user_id":&owner_id,"status":"pending"})
                .session(&mut *session).await?.ok_or_else(conflict)?;
            super::platform_bot_authority::fence_personal_routes(&db,&row.channel_bot_id,owner(&row),session).await?;
            if db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"channel_bot_id":&row.channel_bot_id,"status":"active","_id":{"$ne":&row.id}})
                .session(&mut *session).await?.is_some() {return Err(conflict());}
            // Serialize with route retirement and key invalidation.
            super::channel_routing_service::fence_route_key(&db,&row.route_api_key_id,session).await?;
            if let Some(key)=row.agent_api_key_id.as_deref() {super::channel_routing_service::fence_route_key(&db,key,session).await?;}
            let routed=db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).update_one(
                doc! {"_id":&route,"user_id":owner(&row),"channel_bot_id":&row.channel_bot_id,"agent_api_key_id":&row.route_api_key_id,"is_active":true},
                doc! {"$set":{"updated_at":bson::DateTime::now()}}).session(&mut *session).await?;
            if routed.matched_count!=1 {return Err(conflict());}
            restore(&db,&row,explicit.as_deref(),default_agent,session).await?;
            releases::adopt_in_session(&db,&row,session).await?;
            let now=Utc::now();
            let groups_due=if row.transport=="gateway" {bson::Bson::DateTime(bson::DateTime::from_chrono(now))} else {bson::Bson::Null};
            if db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&id,"status":"pending","gateway_setup_release_id":&row.gateway_setup_release_id},
                doc! {"$set":{"status":"active","route_id":&route,"link_code_hash":&link_hash,
                    "link_code_expires_at":bson::DateTime::from_chrono(now+chrono::Duration::hours(LINK_CODE_TTL_HOURS)),
                    "gateway_groups_retry_at":groups_due,"updated_at":bson::DateTime::from_chrono(now)},
                    "$unset":{"gateway_setup_release_id":""}}).session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            Ok(())
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

/// A healthy connection written by an older replica may still lack a completed
/// restoration. Explicit reconnect checks that boundary before returning it.
pub(crate) async fn restore_existing(
    db: &Database,
    id: &str,
    owner_id: &str,
    default_agent: bool,
    link_hash: &str,
) -> AppResult<()> {
    let db = db.clone();
    let id = id.to_owned();
    let owner_id = owner_id.to_owned();
    let link_hash = link_hash.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let row=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"_id":&id,"user_id":&owner_id,"status":"active"}).session(&mut *session).await?
                .ok_or_else(conflict)?;
            super::platform_bot_authority::fence_personal_routes(&db,&row.channel_bot_id,owner(&row),session).await?;
            if row.continuity_from_channel_id.is_none() {restore(&db,&row,None,default_agent,session).await?;}
            db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&id,"status":"active"},doc! {"$set":{"link_code_hash":&link_hash,
                "link_code_expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(LINK_CODE_TTL_HOURS)),"gateway_groups_retry_at":if row.transport=="gateway" {bson::Bson::DateTime(bson::DateTime::now())} else {bson::Bson::Null}}}).session(&mut *session).await?;
            Ok(())
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}
