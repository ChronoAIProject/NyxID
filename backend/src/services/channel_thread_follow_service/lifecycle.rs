use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn reserve(
    db: &Database,
    channel: &NyxbotChannel,
    settings: &NyxbotThread,
    parent_id: &str,
    key: &str,
    target: &ThreadReplyTarget,
    sender: &str,
    agent: &str,
    child: &mut Option<NyxbotThread>,
    session: &mut ClientSession,
) -> AppResult<()> {
    let now = Utc::now();
    if child.as_ref().is_some_and(|c| {
        c.follow.follow_state.as_deref() == Some("opening")
            && c.follow.follow_opening_expires_at.is_some_and(|t| t > now)
    }) {
        return Ok(());
    }
    let rows = db.collection::<NyxbotThread>(THREADS);
    let mut filter = doc! {"channel_id": &channel.id,"record_scope":SCOPE,"$or":[
    {"follow_state":"active","follow_expires_at":{"$gt":bson::DateTime::from_chrono(now)}},
    {"follow_state":"opening","follow_opening_expires_at":{"$gt":bson::DateTime::from_chrono(now)}}]};
    if rows
        .count_documents(filter.clone())
        .session(&mut *session)
        .await?
        >= BOT_CAP
    {
        return Err(AppError::Conflict("thread_follow_capacity".into()));
    }
    filter.insert("parent_chat_id", parent_id);
    if rows.count_documents(filter).session(&mut *session).await? >= CHAT_CAP {
        return Err(AppError::Conflict("thread_follow_capacity".into()));
    }
    for (namespace, key, limit) in [
        ("thread_follow_bot", channel.channel_bot_id.as_str(), 100),
        ("thread_follow_chat", parent_id, 10),
    ] {
        if !super::super::coordination_service::RateWindowStore::admit_in_session(
            db,
            namespace,
            key,
            limit,
            std::time::Duration::from_secs(3600),
            session,
        )
        .await?
        {
            return Err(AppError::Conflict("thread_follow_rate_limited".into()));
        }
    }
    rows.update_one(
        doc! {"_id":parent_id,"user_id":&channel.user_id,"channel_id":&channel.id},
        doc! {"$set":{"has_thread_history":true}},
    )
    .session(&mut *session)
    .await?;
    let f = target.facts();
    let mut set = doc! {"follow_state":"opening","follow_opening_expires_at":bson::DateTime::from_chrono(now+Duration::seconds(OPENING_SECONDS)),
    "follow_started_by":sender,"follow_stop_reason":bson::Bson::Null,"follow_error_code":bson::Bson::Null,
    "updated_at":bson::DateTime::from_chrono(now)};
    if child.as_ref().is_none_or(|c| c.conversation_id.is_none()) {
        set.insert(
            "conversation_id",
            format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        );
        set.insert("context_status", "pending");
    }
    *child = rows.find_one_and_update(doc! {"channel_id":&channel.id,"partition":key},
        doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"user_id":&channel.user_id,
            "created_at":bson::DateTime::from_chrono(now),"record_scope":SCOPE,"parent_chat_id":parent_id,
            "settings_chat_id":&settings.id,"kind":&settings.kind,"platform_chat_id":&f.chat_id,
            "thread_identity_version":1,"thread_kind":bson::to_bson(&f.kind).map_err(|_| not_found())?,
            "thread_root_id":&f.root_id,"thread_scope_id":&f.chat_id,
            "binding_generation":settings.follow.binding_generation,"binding_channel_generation":channel_generation(channel,settings),
            "bound_agent_id":agent},"$set":set,"$inc":{"follow_revision":1}})
        .upsert(true).return_document(ReturnDocument::After).session(session).await?;
    Ok(())
}

pub(super) async fn stop_in_session(
    db: &Database,
    child: &mut NyxbotThread,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    if child.follow.follow_state.as_deref() == Some("stopped") {
        return Ok(());
    }
    *child = db
        .collection::<NyxbotThread>(THREADS)
        .find_one_and_update(
            doc! {"_id":&child.id},
            doc! {"$set":{"follow_state":"stopped","follow_stop_reason":reason,
            "follow_stopped_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},
            "$inc":{"follow_revision":1}},
        )
        .return_document(ReturnDocument::After)
        .session(session)
        .await?
        .ok_or_else(not_found)?;
    Ok(())
}

pub async fn stop(
    db: &Database,
    owner: &str,
    channel: &str,
    parent: &str,
    child: &str,
) -> AppResult<NyxbotThread> {
    access(db, owner, channel).await?;
    let db = db.clone();
    let owner = owner.to_owned();
    let channel = channel.to_owned();
    let parent = parent.to_owned();
    let child = child.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db;
            let owner = owner.as_str();
            let channel = channel.as_str();
            let parent = parent.as_str();
            let child = child.as_str();

            let work: AppResult<_> = Box::pin(async {
                fence(db, channel, owner, session).await?;
                let mut row = db.collection::<NyxbotThread>(THREADS).find_one(doc! {"_id":child,
                "user_id":owner,"channel_id":channel,"parent_chat_id":parent,"record_scope":SCOPE})
                .session(&mut *session).await?.ok_or_else(not_found)?;
                stop_in_session(db, &mut row, "owner", session).await?;
                Ok(row)
            })
            .await;
            transactions::transaction_result(work)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// The same transaction as a parent setting write; late starters conflict
/// on the channel fence and reload the live setting/generation.
pub async fn update_settings(
    db: &Database,
    owner: &str,
    channel: &str,
    chat: &str,
    update: bson::Document,
    stop_children: bool,
    reassign: bool,
) -> AppResult<NyxbotThread> {
    let db = db.clone();
    let owner = owner.to_owned();
    let channel = channel.to_owned();
    let chat = chat.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
            let db=&db;
            let owner=owner.as_str();
            let channel=channel.as_str();
            let chat=chat.as_str();

        let work:AppResult<_>=Box::pin(async {
            fence(db,channel,owner,session).await?;
            let row=db.collection::<NyxbotThread>(THREADS).find_one_and_update(
                doc! {"_id":chat,"user_id":owner,"channel_id":channel,"record_scope":{"$ne":SCOPE}},update.clone())
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(not_found)?;
            if stop_children || reassign {
                db.collection::<NyxbotThread>(THREADS).update_many(
                    doc! {"channel_id":channel,"record_scope":SCOPE,"settings_chat_id":chat},
                    doc! {"$set":{"follow_state":"stopped","follow_stop_reason":if reassign {"agent_changed"} else {"settings_off"},
                        "follow_stopped_at":bson::DateTime::now()},"$inc":{"follow_revision":1}})
                    .session(&mut *session).await?;
            }
            Ok(row)
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

/// Finite oldest-due pass; admission enforces expiry even when this lags.
pub async fn sweep(db: &Database) -> AppResult<()> {
    let now = bson::DateTime::now();
    for (status, field) in [
        ("opening", "follow_opening_expires_at"),
        ("active", "follow_expires_at"),
    ] {
        let rows: Vec<NyxbotThread> = db
            .collection(THREADS)
            .find(doc! {"record_scope":SCOPE,"follow_state":status,field:{"$lte":now}})
            .sort(doc! {field:1,"_id":1})
            .limit(50)
            .await?
            .try_collect()
            .await?;
        for row in rows {
            expire(db, &row.id, row.follow.follow_revision, status, field, now).await?;
        }
    }
    Ok(())
}

/// Relink and close inherited bindings atomically. Explicit chat overrides keep
/// their effective agent and generation.
pub async fn relink(db: &Database, owner: &str, channel: &str, agent: &str) -> AppResult<()> {
    let db = db.clone();
    let owner = owner.to_owned();
    let channel = channel.to_owned();
    let agent = agent.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
            let db=&db;
            let owner=owner.as_str();
            let channel=channel.as_str();
            let agent=agent.as_str();

        let work:AppResult<()>=Box::pin(async {
            db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":channel,"user_id":owner},
                doc! {"$set":{"agent_id":agent,"updated_at":bson::DateTime::now()},"$inc":{"follow_capacity_revision":1,"follow_binding_generation":1}})
                .session(&mut *session).await?;
            let parents:Vec<NyxbotThread>=db.collection(THREADS).find(doc! {"channel_id":channel,"user_id":owner,"agent_id":bson::Bson::Null,"record_scope":{"$ne":SCOPE}})
                .session(&mut *session).await?.stream(&mut *session).try_collect().await?;
            let ids:Vec<&str>=parents.iter().map(|r|r.id.as_str()).collect();
            db.collection::<NyxbotThread>(THREADS).update_many(doc! {"_id":{"$in":&ids}},doc! {"$set":{"conversation_id":bson::Bson::Null}}).session(&mut *session).await?;
            db.collection::<NyxbotThread>(THREADS).update_many(doc! {"channel_id":channel,"record_scope":SCOPE,"settings_chat_id":{"$in":&ids}},
                doc! {"$set":{"follow_state":"stopped","follow_stop_reason":"agent_changed","follow_stopped_at":bson::DateTime::now()},"$inc":{"follow_revision":1}})
                .session(&mut *session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn delete_conversation(
    db: &Database,
    owner: &str,
    conversation: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(child) = db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"user_id":owner,"conversation_id":conversation,"record_scope":SCOPE})
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&child.channel_id,"user_id":owner},
            doc! {"$inc":{"follow_capacity_revision":1}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<NyxbotThread>(THREADS).update_one(doc! {"_id":&child.id},doc! {"$set":{
        "conversation_id":bson::Bson::Null,"follow_state":"stopped","follow_stop_reason":"conversation_deleted","follow_stopped_at":bson::DateTime::now()},"$inc":{"follow_revision":1}})
        .session(session).await?;
    Ok(())
}

async fn expire(
    db: &Database,
    id: &str,
    revision: i64,
    status: &str,
    field: &str,
    now: bson::DateTime,
) -> AppResult<()> {
    let db = db.clone();
    let id = id.to_owned();
    let status = status.to_owned();
    let field = field.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<()>=Box::pin(async {
            let filter=doc! {"_id":&id,"follow_revision":revision,"follow_state":&status,&field:{"$lte":now}};
            let Some(row)=db.collection::<NyxbotThread>(THREADS).find_one(filter.clone()).session(&mut *session).await? else{return Ok(());};
            db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.channel_id,"user_id":&row.user_id},doc! {"$inc":{"follow_capacity_revision":1}}).session(&mut *session).await?;
            db.collection::<NyxbotThread>(THREADS).update_one(filter,doc! {"$set":{"follow_state":"expired","follow_stop_reason":"idle"},"$inc":{"follow_revision":1}}).session(&mut *session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn suspend(db: &Database, owner: &str, channel: &str, reason: &str) -> AppResult<()> {
    let db = db.clone();
    let owner = owner.to_owned();
    let channel = channel.to_owned();
    let reason = reason.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<()>=Box::pin(async {
            db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&channel,"user_id":&owner},
                doc! {"$inc":{"follow_capacity_revision":1}}).session(&mut *session).await?;
            db.collection::<NyxbotThread>(THREADS).update_many(doc! {"channel_id":&channel,"user_id":&owner,
                "record_scope":SCOPE,"follow_state":{"$in":["active","opening"]}},
                doc! {"$set":{"follow_state":"unavailable","follow_stop_reason":&reason,"follow_stopped_at":bson::DateTime::now()},"$inc":{"follow_revision":1}})
                .session(&mut *session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn carry_over(
    db: &Database,
    owner: &str,
    from: &str,
    to: &str,
    changed: bool,
) -> AppResult<()> {
    let db = db.clone();
    let owner = owner.to_owned();
    let from = from.to_owned();
    let to = to.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<()>=Box::pin(async {
            let old=db.collection::<NyxbotChannel>(CHANNELS).find_one_and_update(doc! {"_id":&from,"user_id":&owner},
                doc! {"$inc":{"follow_capacity_revision":1}}).session(&mut *session).await?.ok_or_else(not_found)?;
            db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&to,"user_id":&owner},
                doc! {"$set":{"follow_binding_generation":old.follow_binding_generation+i64::from(changed)},"$inc":{"follow_capacity_revision":1}}).session(&mut *session).await?;
            let rows=db.collection::<NyxbotThread>(THREADS);
            if changed {
                rows.update_many(doc! {"channel_id":&from,"user_id":&owner,"agent_id":bson::Bson::Null,"record_scope":{"$ne":SCOPE}},
                    doc! {"$set":{"conversation_id":bson::Bson::Null}}).session(&mut *session).await?;
                rows.update_many(doc! {"channel_id":&from,"user_id":&owner,"record_scope":SCOPE},
                    doc! {"$set":{"follow_state":"stopped","follow_stop_reason":"agent_changed"},"$inc":{"follow_revision":1}}).session(&mut *session).await?;
            }
            let stable=doc! {"$not":{"$regex":"^conv_"}};
            rows.update_many(doc! {"channel_id":&from,"user_id":&owner,"partition":&stable},doc! {"$set":{"channel_id":&to}}).session(&mut *session).await?;
            db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
                .update_many(doc! {"user_id":&owner,"channel.nyxbot_channel_id":&from,"channel.partition":stable},
                    doc! {"$set":{"channel.nyxbot_channel_id":&to}}).session(&mut *session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn release_agent(db: &Database, owner: &str, agent: &str) -> AppResult<()> {
    let db = db.clone();
    let owner = owner.to_owned();
    let agent = agent.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<()>=Box::pin(async {
            let rows=db.collection::<NyxbotThread>(THREADS);
            let parents:Vec<NyxbotThread>=rows.find(doc! {"user_id":&owner,"agent_id":&agent,"record_scope":{"$ne":SCOPE}})
                .session(&mut *session).await?.stream(&mut *session).try_collect().await?;
            let channels:Vec<&str>=parents.iter().map(|p|p.channel_id.as_str()).collect();
            db.collection::<NyxbotChannel>(CHANNELS).update_many(doc! {"_id":{"$in":channels},"user_id":&owner},doc! {"$inc":{"follow_capacity_revision":1}})
                .session(&mut *session).await?;
            rows.update_many(doc! {"_id":{"$in":parents.iter().map(|p|p.id.as_str()).collect::<Vec<_>>()},"user_id":&owner},
                doc! {"$unset":{"agent_id":""},"$set":{"conversation_id":bson::Bson::Null},"$inc":{"binding_generation":1}}).session(&mut *session).await?;
            rows.update_many(doc! {"user_id":&owner,"record_scope":SCOPE,"bound_agent_id":&agent},doc! {"$set":{
                "follow_state":"stopped","follow_stop_reason":"agent_destroyed","follow_stopped_at":bson::DateTime::now()},"$inc":{"follow_revision":1}}).session(&mut *session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}
