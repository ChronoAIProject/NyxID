//! Private-chat posting defaults. Execution continues to consult allow_posts only.
use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::{
        COLLECTION_NAME as CHANNELS, NyxbotChannel, THREADS_COLLECTION_NAME as THREADS,
    },
};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};

pub(crate) const APPLY_LIMIT: i64 = 1000;

/// A probe has no verified chat kind yet. Capture the default without granting
/// posting; the first real message consumes it. Legacy placeholders stay off.
pub(crate) fn placeholder(row: &NyxbotChannel, now: bson::DateTime) -> Document {
    doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&row.user_id,"created_at":now,
    "private_chat_posts_pending":row.private_chat_posts.unwrap_or(false)}
}

/// One atomic upsert for every real relay/gateway/native chat. $literal keeps
/// provider strings out of Mongo expressions. Existing explicit choices win.
pub(crate) fn record_update(
    row: &NyxbotChannel,
    private: bool,
    mut values: Document,
    now: bson::DateTime,
) -> Vec<Document> {
    let fresh = doc! {"$eq":[{"$type":"$_id"},"missing"]};
    let allow = doc! {"$cond":[{"$eq":[{"$type":"$allow_posts"},"missing"]},
    {"$and":[private,{"$cond":[fresh.clone(),row.private_chat_posts.unwrap_or(false),
        {"$eq":["$private_chat_posts_pending",true]}]}]},"$allow_posts"]};
    values = values
        .into_iter()
        .map(|(k, v)| (k, bson::Bson::Document(doc! {"$literal":v})))
        .collect();
    for (name, value) in [
        ("_id", bson::Bson::String(uuid::Uuid::new_v4().to_string())),
        ("user_id", bson::Bson::String(row.user_id.clone())),
        ("created_at", bson::Bson::DateTime(now)),
    ] {
        values.insert(
            name,
            doc! {"$cond":[fresh.clone(),{"$literal":value},format!("${name}")]},
        );
    }
    values.insert("allow_posts", allow);
    vec![
        doc! {"$set":values},
        doc! {"$unset":"private_chat_posts_pending"},
    ]
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccessSettings {
    pub private_chats: Option<String>,
    pub private_chat_posts: Option<bool>,
    #[serde(default)]
    pub apply_to_existing: bool,
}
impl AccessSettings {
    pub fn validate(&self) -> AppResult<()> {
        if self
            .private_chats
            .as_deref()
            .is_some_and(|v| !matches!(v, "owner" | "everyone"))
        {
            return Err(AppError::ValidationError(
                "private_chats must be owner or everyone".into(),
            ));
        }
        if self.apply_to_existing && self.private_chat_posts.is_none() {
            return Err(AppError::ValidationError(
                "apply_to_existing requires private_chat_posts".into(),
            ));
        }
        if self.private_chats.is_none() && self.private_chat_posts.is_none() {
            return Err(AppError::ValidationError(
                "Pass private_chats or private_chat_posts".into(),
            ));
        }
        Ok(())
    }
}

/// Bound both the walk and mutation. A cap refusal rolls back the channel too.
pub(crate) async fn update_as(
    db: &Database,
    row: &NyxbotChannel,
    manager: &str,
    settings: &AccessSettings,
) -> AppResult<u64> {
    settings.validate()?;
    let context = (
        db.clone(),
        row.clone(),
        settings.clone(),
        manager.to_owned(),
    );
    super::transaction_service::run_db_only(db, &context, |tx, (db, row, settings, manager)| {
        Box::pin(async move {
            if row.org_specialist.is_some() {
                super::org_channel_service::fence_management(db, tx, row, manager).await?;
            }
            if let Some(org) = row.bot_owner_id.as_deref() {
                super::org_membership_authority::fence_live_member(db, &row.user_id, tx).await?;
                // Contend with role changes/revocation, not just a snapshot read.
                let changed = db
                    .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                    .update_one(
                        doc! {"org_user_id": org, "member_user_id": &row.user_id,
                            "role": "admin", "revoked_at": null},
                        doc! {"$inc": {"channel_settings_revision": 1_i64}},
                    )
                    .session(&mut *tx)
                    .await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Forbidden("org_access_lost".into()));
                }
            }
            let mut set = doc! {"updated_at": bson::DateTime::now()};
            if let Some(access) = &settings.private_chats {
                set.insert("private_chats", access);
            }
            if let Some(posts) = settings.private_chat_posts {
                set.insert("private_chat_posts", posts);
            }
            let mut channel_filter = doc! {
                "_id": &row.id, "user_id": &row.user_id, "bot_owner_id": &row.bot_owner_id,
            };
            // Preserve the ability to narrow private access on a failed link.
            if settings.private_chats.as_deref() == Some("everyone")
                || settings.private_chat_posts == Some(true)
            {
                channel_filter.insert("status", doc! {"$in": ["active", "pending"]});
            }
            let changed = db.collection::<Document>(CHANNELS)
                .update_one(channel_filter, doc! {"$set": set,"$inc":{"execution_policy_revision":1_i64}})
                .session(&mut *tx)
                .await?;
            if changed.matched_count != 1 {
                return Err(AppError::Conflict("Channel authority changed".into()));
            }
            if !settings.apply_to_existing {
                return Ok(0);
            }
            let allow = settings.private_chat_posts.expect("validated setting");
            let mut filter = doc! {
                "channel_id": &row.id, "user_id": &row.user_id, "kind": "private",
                "record_scope": {"$ne": "platform_thread"}, "platform_thread_id": null,
                "partition": {"$not": {"$regex": "^custom:"}}, "allow_posts": {"$ne": allow},
            };
            let threads = db.collection::<Document>(THREADS);
            let mut cursor = threads.find(filter.clone())
                .projection(doc! {"_id": 1})
                .limit(APPLY_LIMIT + 1)
                .max_time(std::time::Duration::from_secs(5))
                .session(&mut *tx)
                .await?;
            let mut ids = Vec::new();
            while let Some(chat) = cursor.next(&mut *tx).await.transpose()? {
                ids.push(chat.get_str("_id")
                    .map_err(|_| AppError::Internal("Invalid chat identity".into()))?
                    .to_owned());
            }
            if ids.len() > APPLY_LIMIT as usize {
                return Err(AppError::ValidationError(format!(
                    "At most {APPLY_LIMIT} private chats can change at once; save the default without apply_to_existing and update existing chats individually"
                )));
            }
            filter.insert("_id", doc! {"$in": ids});
            Ok(threads.update_many(
                    filter,
                    doc! {"$set": {"allow_posts": allow, "updated_at": bson::DateTime::now()},"$inc":{"execution_policy_revision":1_i64}},
                )
                .session(&mut *tx)
                .await?
                .modified_count)
        })
    })
    .await
}

/// CMA detaches its provider binding while the channel and histories remain.
pub(crate) async fn remove_binding(
    db: &Database,
    row: &NyxbotChannel,
    binding: &str,
) -> AppResult<bool> {
    let context = (db.clone(), row.clone(), binding.to_owned());
    super::transaction_service::run_db_only(db,&context,|tx,(db,row,binding)|Box::pin(async move {
        let changed=db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {
            "_id":&row.id,"binding_id":binding,"route_api_key_id":&row.route_api_key_id,
            "transport":&row.transport,"status":&row.status,"agent_api_key_id":&row.agent_api_key_id,
            "pending_agent_api_key_id":&row.pending_agent_api_key_id,
        },doc! {"$inc":{"execution_policy_revision":1_i64},"$unset":{"binding_id":""},
            "$set":{"gateway_no_progress":false,"updated_at":bson::DateTime::now()}}).session(&mut *tx).await?;
        if changed.matched_count!=1 {return Ok(false)}
        Ok(true)
    })).await
}

/// Remove the provider's exact chat, its derived delivery state and feed rows.
/// The parent write serializes this retirement with flagged reply settlement.
/// Per-actor organization lanes share one public channel chat.
pub(crate) async fn remove_thread(
    db: &Database,
    row: &NyxbotChannel,
    partition: &str,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let context = (db.clone(), row.clone(), partition.to_owned());
    super::transaction_service::run_db_only(db, &context, |tx, (db, row, partition)| {
        Box::pin(async move {
            if db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(
                    doc! {
                        "_id":&row.id,"user_id":&row.user_id,"binding_id":&row.binding_id,
                    },
                    doc! {"$inc":{"notification_generation":1_i64}},
                )
                .session(&mut *tx)
                .await?
                .matched_count
                != 1
            {
                return Err(AppError::Conflict("Channel binding changed".into()));
            }
            if let Some(thread) = db
                .collection::<crate::models::nyxbot_channel::NyxbotThread>(THREADS)
                .find_one_and_delete(doc! {"channel_id":&row.id,"partition":partition})
                .session(&mut *tx)
                .await?
            {
                let mut cursor = db
                    .collection::<Document>(crate::models::org_channel::LANES)
                    .find(doc! {"channel_id":&row.id,"partition":partition})
                    .projection(doc! {"conversation_id":1})
                    .session(&mut *tx)
                    .await?;
                let lanes: Vec<Document> = cursor.stream(&mut *tx).try_collect().await?;
                let mut conversations: Vec<String> = lanes
                    .iter()
                    .filter_map(|lane| lane.get_str("conversation_id").ok().map(str::to_owned))
                    .collect();
                conversations.extend(thread.conversation_id);
                super::assistant_notification_service::purge(
                    db,
                    doc! {
                    "source.channel_id":&row.id,"$or":[
                        {"source.conversation_id":{"$in":conversations}},
                        {"source.thread_id":&thread.id},
                        {"source.id":&thread.id},
                    ]},
                    Some(&mut *tx),
                )
                .await?;
                db.collection::<Document>(
                    crate::models::execution_permission_channel::COLLECTION_NAME,
                )
                .delete_many(doc! {"task.channel_source.thread_id":&thread.id})
                .session(&mut *tx)
                .await?;
            }
            if row.gateway_source.is_some() {
                db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME)
                    .delete_many(doc! {"channel_id":&row.id,"partition":partition})
                    .session(tx)
                    .await?;
            }
            Ok(())
        })
    })
    .await
}
