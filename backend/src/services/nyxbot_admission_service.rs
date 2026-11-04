//! Bounded, content-free channel admission diagnostics. These never grant authority.
use crate::{
    errors::AppResult,
    models::{
        channel_message::{COLLECTION_NAME, NyxbotAdmission},
        channel_thread::ChannelThreadFacts,
        nyxbot_channel::NyxbotChannel,
    },
};
use bson::{Document, doc};
use mongodb::Database;

pub async fn begin(
    db: &Database,
    row: &NyxbotChannel,
    message: &str,
    event: &str,
) -> AppResult<Option<ChannelThreadFacts>> {
    // Only the optional evidence is needed here. Legacy relay metadata may be
    // sparse; diagnostics must not require unrelated message fields to admit it.
    let Some(source) = db.collection::<Document>(COLLECTION_NAME).find_one(doc! {
        "_id":message,"channel_bot_id":&row.channel_bot_id,"direction":"inbound","agent_api_key_id":&row.route_api_key_id,
    }).projection(doc! {"thread_context":1,"nyxbot_admission":1}).await? else { return Ok(None); };
    let facts = source
        .get("thread_context")
        .cloned()
        .and_then(|value| bson::from_bson::<ChannelThreadFacts>(value).ok());
    let discord = facts
        .as_ref()
        .and_then(|facts| facts.discord_gateway.as_ref());
    let diagnostic = NyxbotAdmission {
        disposition: "received".into(),
        bot_mentioned: discord.map(|d| d.bot_mentioned),
        managed_role_mentioned: discord.is_some_and(|d| d.managed_role_mentioned),
        broadcast_roles: discord.and_then(|d| {
            (!d.broadcast_roles || d.broadcast_role_count > 0).then_some(d.broadcast_role_count)
        }),
        broadcast_everyone_or_here: discord.is_some_and(|d| d.broadcast_everyone_or_here),
        address: facts.as_ref().map(|f| f.address).unwrap_or_default(),
        thread_follow_selection: None,
        resolved_thread_id: None,
        turn_id: None,
    };
    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":message,"channel_bot_id":&row.channel_bot_id,"nyxbot_admission":null},
        doc! {"$set":{"nyxbot_admission":bson::to_bson(&diagnostic).expect("admission metadata")}}).await?;
    if source.get_document("nyxbot_admission").is_ok() && facts.is_some() {
        db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":message,"channel_bot_id":&row.channel_bot_id},
            doc! {"$set":{"nyxbot_admission.address":bson::to_bson(&diagnostic.address).expect("address"),
                "nyxbot_admission.bot_mentioned":diagnostic.bot_mentioned,
                "nyxbot_admission.managed_role_mentioned":diagnostic.managed_role_mentioned,
                "nyxbot_admission.broadcast_roles":diagnostic.broadcast_roles.map(i32::from),
                "nyxbot_admission.broadcast_everyone_or_here":diagnostic.broadcast_everyone_or_here}}).await?;
    }
    db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":event,"channel_id":&row.id},
            doc! {"$set":{"admission_message_id":message}},
        )
        .await?;
    Ok(facts)
}

pub async fn selection(
    db: &Database,
    row: &NyxbotChannel,
    source: &str,
    selection: &'static str,
    child: Option<&str>,
) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":source,"channel_bot_id":&row.channel_bot_id,"nyxbot_admission":{"$type":"object"}},
        doc! {"$set":{"nyxbot_admission.thread_follow_selection":selection,"nyxbot_admission.resolved_thread_id":child}}).await?;
    Ok(())
}

pub async fn finish(
    db: &Database,
    row: &NyxbotChannel,
    source: &str,
    disposition: &'static str,
) -> AppResult<()> {
    // A committed turn is stronger evidence than a later response/cleanup error.
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":source,"channel_bot_id":&row.channel_bot_id,
            "nyxbot_admission":{"$type":"object"},"nyxbot_admission.turn_id":null},
            doc! {"$set":{"nyxbot_admission.disposition":disposition}},
        )
        .await?;
    Ok(())
}

/// Called in the existing turn-binding transaction, including queued work.
pub async fn bind_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    event: &Document,
    turn: &str,
) -> AppResult<()> {
    let Ok(source) = event.get_str("admission_message_id") else {
        return Ok(());
    };
    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":source,"nyxbot_admission":{"$type":"object"}}, vec![doc! {"$set":{
        "nyxbot_admission.turn_id":turn,
        "nyxbot_admission.disposition":{"$switch":{"branches":[
            {"case":{"$eq":["$nyxbot_admission.managed_role_mentioned",true]},"then":"addressed_managed_role"},
            {"case":{"$eq":["$nyxbot_admission.bot_mentioned",true]},"then":"addressed_user_mention"},
            {"case":{"$eq":["$nyxbot_admission.thread_follow_selection","child"]},"then":"followed_thread"},
            {"case":{"$eq":["$nyxbot_admission.address","reply_to_bot"]},"then":"addressed_reply"}
        ],"default":"admitted"}}
    }}]).session(&mut *session).await?;
    Ok(())
}
