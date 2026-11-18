use super::*;

#[allow(clippy::too_many_arguments)]
pub async fn list_children(
    db: &Database,
    owner: &str,
    channel: &str,
    parent: Option<&str>,
    state: &str,
    cursor: Option<&str>,
    limit: u32,
) -> AppResult<(Vec<NyxbotThread>, Option<String>)> {
    let row = access(db, owner, channel).await?;
    list_allowed(
        db,
        &row.user_id,
        &[channel.to_owned()],
        parent,
        state,
        cursor,
        limit,
    )
    .await
}

/// Channel IDs come from the existing live owner/org-authorized channel list.
pub async fn list_allowed(
    db: &Database,
    owner: &str,
    channels: &[String],
    parent: Option<&str>,
    state: &str,
    cursor: Option<&str>,
    limit: u32,
) -> AppResult<(Vec<NyxbotThread>, Option<String>)> {
    if !matches!(state, "active" | "all") || !(1..=50).contains(&limit) {
        return Err(AppError::ValidationError("Invalid thread page".into()));
    }
    for channel in channels {
        access(db, owner, channel).await?;
    }
    let rows = db.collection::<NyxbotThread>(THREADS);
    let mut filter = doc! {"channel_id":{"$in":channels},"record_scope":SCOPE};
    if let Some(parent) = parent {
        if rows
            .find_one(
                doc! {"_id":parent,"channel_id":{"$in":channels},"record_scope":{"$ne":SCOPE}},
            )
            .await?
            .is_none()
        {
            return Err(not_found());
        }
        filter.insert("parent_chat_id", parent);
    }
    if state == "active" {
        filter.insert("follow_state", "active");
        filter.insert("follow_expires_at", doc! {"$gt":bson::DateTime::now()});
    }
    if let Some(cursor) = cursor {
        let invalid = || AppError::ValidationError("Invalid thread cursor".into());
        if cursor.len() > 100 {
            return Err(invalid());
        }
        let (stamp, id) = cursor.split_once(':').ok_or_else(invalid)?;
        let stamp = stamp.parse::<i64>().map_err(|_| invalid())?;
        uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
        let time = bson::DateTime::from_millis(stamp);
        filter.insert(
            "$or",
            bson::bson! ([{"created_at":{"$lt":time}},{"created_at":time,"_id":{"$lt":id}}]),
        );
    }
    let mut page: Vec<NyxbotThread> = rows
        .find(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(i64::from(limit) + 1)
        .await?
        .try_collect()
        .await?;
    let more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let cursor = more
        .then(|| {
            page.last()
                .map(|r| format!("{}:{}", r.created_at.timestamp_millis(), r.id))
        })
        .flatten();
    Ok((page, cursor))
}

pub async fn counts(
    db: &Database,
    owner: &str,
    parents: &[String],
) -> AppResult<std::collections::HashMap<String, i64>> {
    let rows: Vec<bson::Document> = db
        .collection::<NyxbotThread>(THREADS)
        .aggregate([
            doc! {"$match":{"user_id":owner,"record_scope":SCOPE,"parent_chat_id":{"$in":parents},
            "follow_state":"active","follow_expires_at":{"$gt":bson::DateTime::now()}}},
            doc! {"$group":{"_id":"$parent_chat_id","count":{"$sum":1}}},
            doc! {"$limit":parents.len() as i64 + 1},
        ])
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some((
                r.get_str("_id").ok()?.into(),
                r.get_i32("count")
                    .map(i64::from)
                    .or_else(|_| r.get_i64("count"))
                    .ok()?,
            ))
        })
        .collect())
}

pub async fn materialized_conversations(
    db: &Database,
    owner: &str,
    children: &[NyxbotThread],
) -> AppResult<std::collections::HashSet<String>> {
    let ids: Vec<&str> = children
        .iter()
        .filter_map(|r| r.conversation_id.as_deref())
        .collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<bson::Document> = db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(doc! {"user_id":owner,"_id":{"$in":ids}})
        .projection(doc! {"_id":1})
        .limit(children.len() as i64 + 1)
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.get_str("_id").ok().map(str::to_owned))
        .collect())
}

/// Owner lists may link only to that viewer's current org lane. A child's
/// reserved personal-thread ID is never another member's execution identity.
pub(crate) async fn actor_conversations(
    db: &Database,
    viewer: &str,
    children: &[NyxbotThread],
    channels: &[NyxbotChannel],
) -> AppResult<std::collections::HashMap<String, String>> {
    // Each full lane key uses the existing unique index. Never scan a viewer's
    // conversation history; only materialize at most one lane per page child.
    let mut candidates: Vec<_> = children
        .iter()
        .filter_map(|child| {
            let channel = channels.iter().find(|c| c.id == child.channel_id)?;
            let org = channel.org_specialist.as_ref()?;
            Some((child, channel, org, child.follow.bound_agent_id.as_deref()?))
        })
        .collect();
    if candidates.is_empty() {
        return Ok(Default::default());
    }
    let settings_ids: Vec<_> = candidates
        .iter()
        .filter_map(|(child, _, _, _)| child.follow.settings_chat_id.as_deref())
        .collect();
    let settings: Vec<NyxbotThread> = db
        .collection(THREADS)
        .find(doc! {"_id":{"$in":settings_ids},"record_scope":{"$ne":SCOPE}})
        .hint(mongodb::options::Hint::Name("_id_".into()))
        .limit(candidates.len() as i64)
        .await?
        .try_collect()
        .await?;
    candidates.retain(|(child, channel, _, agent)| {
        settings.iter().any(|parent| {
            Some(parent.id.as_str()) == child.follow.settings_chat_id.as_deref()
                && parent.channel_id == channel.id
                && parent.follow.binding_generation == child.follow.binding_generation
                && channel_generation(channel, parent) == child.follow.binding_channel_generation
                && parent.agent_id.as_deref().or(channel.agent_id.as_deref()) == Some(*agent)
        })
    });
    if candidates.is_empty() {
        return Ok(Default::default());
    }
    let agents: Vec<_> = candidates
        .iter()
        .map(|(_, _, org, agent)| {
            doc! {
                "_id":*agent, "user_id":&org.org_id, "destroyed_at":bson::Bson::Null
            }
        })
        .collect();
    let live: Vec<bson::Document> = db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .find(doc! {"$or":agents})
        .projection(doc! {"_id":1})
        .limit(candidates.len() as i64)
        .await?
        .try_collect()
        .await?;
    let filters: Vec<_> = candidates
        .iter()
        .filter(|(_, _, _, agent)| live.iter().any(|a| a.get_str("_id").ok() == Some(*agent)))
        .map(|(child, channel, org, agent)| {
            doc! {
                "channel_id":&channel.id,"partition":&child.partition,"actor_id":viewer,
                "agent_id":*agent,"binding_revision":&org.revision,
                "guest_revision":bson::Bson::Null,"sender_hash":""
            }
        })
        .collect();
    if filters.is_empty() {
        return Ok(Default::default());
    }
    let lanes: Vec<bson::Document> = db
        .collection::<bson::Document>(crate::models::org_channel::LANES)
        .find(doc! {"$or":filters})
        .projection(doc! {"conversation_id":1})
        .limit(candidates.len() as i64)
        .await?
        .try_collect()
        .await?;
    let ids: Vec<_> = lanes
        .iter()
        .filter_map(|lane| lane.get_str("conversation_id").ok())
        .collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let filters: Vec<_> = candidates.iter().map(|(child, channel, org, agent)| doc! {
        "channel.nyxbot_channel_id":&channel.id,"channel.partition":&child.partition,
        "channel.thread.child_id":&child.id,"channel.thread.generation":child.follow.binding_generation,
        "channel.org_execution.actor_id":viewer,"channel.org_execution.revision":&org.revision,
        "channel.org_execution.guest":false,"agent_id":*agent
    }).collect();
    let conversations: Vec<bson::Document> = db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(doc!{"_id":{"$in":ids},"user_id":viewer,"guest_model_setup":{"$ne":true},"guest_turn":false,"$or":filters})
        .hint(mongodb::options::Hint::Name("_id_".into()))
        .projection(doc!{"_id":1,"channel.thread.child_id":1})
        .limit(candidates.len() as i64).await?.try_collect().await?;
    Ok(conversations
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get_document("channel")
                    .ok()?
                    .get_document("thread")
                    .ok()?
                    .get_str("child_id")
                    .ok()?
                    .to_owned(),
                row.get_str("_id").ok()?.to_owned(),
            ))
        })
        .collect())
}
