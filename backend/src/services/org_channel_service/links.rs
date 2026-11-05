//! Two-leg member linking: account-issued code, authenticated private sender,
//! then confirmation in the same member's first-party session. Codes are hashed.
use crate::{
    errors::{AppError, AppResult},
    models::{
        nyxbot_channel::NyxbotChannel,
        org_channel::{CHALLENGES, LINKS, SenderLink},
    },
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use mongodb::Database;
use serde_json::{Value, json};

async fn member(db: &Database, actor: &str, channel: &NyxbotChannel) -> AppResult<()> {
    let binding = channel.org_specialist.as_ref().ok_or_else(super::denied)?;
    super::enabled(db, &channel.user_id).await?;
    super::require_admin(db, &channel.user_id, &binding.org_id).await?;
    if !super::super::org_agent_service::can_use(
        &super::super::org_agent_service::access(db, actor, &binding.org_id).await?,
    ) {
        return Err(super::denied());
    }
    Ok(())
}

pub(crate) async fn issue(db: &Database, actor: &str, channel: &NyxbotChannel) -> AppResult<Value> {
    member(db, actor, channel).await?;
    if !super::super::coordination_service::RateWindowStore::admit(
        db,
        "org-channel-link",
        actor,
        3,
        std::time::Duration::from_secs(60),
    )
    .await?
    .allowed
    {
        return Err(AppError::ValidationError(
            "Please wait before requesting another verification code".into(),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let code = zeroize::Zeroizing::new(format!(
        "nyxorg_{}",
        crate::crypto::token::generate_random_token()
    ));
    let expires = Utc::now() + Duration::minutes(10);
    let binding = channel.org_specialist.as_ref().ok_or_else(super::denied)?;
    db.collection::<Document>(CHALLENGES).insert_one(doc!{"_id":&id,"channel_id":&channel.id,"org_id":&binding.org_id,
        "user_id":actor,"binding_revision":&binding.revision,"code_hash":crate::crypto::token::hash_token(&code),
        "state":"issued","created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(expires)}).await?;
    Ok(json!({"id":id,"code":code.as_str(),"expires_at":expires,"state":"issued"}))
}

/// Returns true only for a recognized challenge. Call only for authenticated,
/// original human messages in a private chat, never for bots/forwarded posts.
pub(crate) async fn receive(
    db: &Database,
    channel: &NyxbotChannel,
    sender: &str,
    text: &str,
    label: Option<&str>,
) -> AppResult<bool> {
    let Some(code) = text
        .split_whitespace()
        .find(|word| word.starts_with("nyxorg_"))
    else {
        return Ok(false);
    };
    if code.len() > 128 {
        return Ok(false);
    }
    let binding = channel.org_specialist.as_ref().ok_or_else(super::denied)?;
    let hash = crate::crypto::token::hash_token(code);
    let row=db.collection::<Document>(CHALLENGES).find_one(doc!{"channel_id":&channel.id,"code_hash":&hash,
        "binding_revision":&binding.revision,"state":"issued","expires_at":{"$gt":bson::DateTime::now()}}).await?;
    let Some(row) = row else { return Ok(false) };
    let actor = row.get_str("user_id").map_err(|_| super::denied())?;
    member(db, actor, channel).await?;
    Ok(db.collection::<Document>(CHALLENGES).update_one(doc!{"_id":row.get_str("_id").map_err(|_|super::denied())?,"state":"issued"},
        doc!{"$set":{"state":"observed","sender_hash":super::sender_hash(&channel.id,sender),
            "sender_label":label.unwrap_or("Chat account").chars().take(80).collect::<String>()},"$unset":{"code_hash":""}}).await?.matched_count==1)
}

pub(crate) async fn status(
    db: &Database,
    actor: &str,
    channel: &NyxbotChannel,
    id: &str,
) -> AppResult<Value> {
    member(db, actor, channel).await?;
    let row = db
        .collection::<Document>(CHALLENGES)
        .find_one(doc! {"_id":id,"user_id":actor,"channel_id":&channel.id,
        "expires_at":{"$gt":bson::DateTime::now()}})
        .await?
        .ok_or_else(super::denied)?;
    Ok(
        json!({"id":id,"state":row.get_str("state").unwrap_or("expired"),"sender_label":row.get_str("sender_label").ok()}),
    )
}

pub(crate) async fn confirm(
    db: &Database,
    actor: &str,
    channel: &NyxbotChannel,
    id: &str,
) -> AppResult<()> {
    member(db, actor, channel).await?;
    channel.org_specialist.as_ref().ok_or_else(super::denied)?;
    let context = (
        db.clone(),
        actor.to_owned(),
        channel.clone(),
        id.to_owned(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    super::super::transaction_service::run_db_only(db,&context,|tx,(db,actor,channel,id,link_id,new_revision)| Box::pin(async move {
        let b=channel.org_specialist.as_ref().ok_or_else(super::denied)?;
        for (person,roles) in [(actor.as_str(),vec!["admin","member"]),(channel.user_id.as_str(),vec!["admin"])] {
            super::live_doc(db,tx,"users",doc!{"_id":person,"is_active":true},true).await?;
            super::live_doc(db,tx,"org_memberships",doc!{"org_user_id":&b.org_id,"member_user_id":person,
                "revoked_at":bson::Bson::Null,"role":{"$in":roles}},true).await?;
        }
        super::fence_management(db,tx,channel,&channel.user_id).await?;
        super::live_doc(db,tx,"nyxbot_channels",doc!{"_id":&channel.id,"status":"active","org_specialist.revision":&b.revision},true).await?;
        let challenge=db.collection::<Document>(CHALLENGES).find_one_and_update(doc!{"_id":id,"user_id":actor,"channel_id":&channel.id,
            "binding_revision":&b.revision,"state":"observed","expires_at":{"$gt":bson::DateTime::now()}},
            doc!{"$set":{"state":"confirmed"}}).session(&mut *tx).await?.ok_or_else(super::denied)?;
        let hash=challenge.get_str("sender_hash").map_err(|_|super::denied())?;
        let existing=db.collection::<SenderLink>(LINKS).find_one(doc!{"channel_id":&channel.id,"sender_hash":hash}).session(&mut *tx).await?;
        if let Some(existing)=existing {
            if existing.user_id!=*actor {return Err(AppError::Conflict("This chat account is already linked to a different person".into()));}
            db.collection::<SenderLink>(LINKS).update_one(doc!{"_id":&existing.id},doc!{"$set":{"active":true,"revision":new_revision,"binding_revision":&b.revision}}).session(tx).await?;
        } else {
            db.collection::<SenderLink>(LINKS).insert_one(SenderLink{id:link_id.clone(),channel_id:channel.id.clone(),org_id:b.org_id.clone(),
                revision:new_revision.clone(),binding_revision:b.revision.clone(),sender_hash:hash.into(),user_id:actor.clone(),active:true,created_at:Utc::now()}).session(tx).await?;
        }
        Ok(())
    })).await?;
    // The consumed challenge is an audit trail, not a source of execution authority.
    Ok(())
}
