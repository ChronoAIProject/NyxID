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
    access(db, owner, channel).await?;
    list_allowed(
        db,
        owner,
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
    let rows = db.collection::<NyxbotThread>(THREADS);
    let mut filter = doc! {"user_id":owner,"channel_id":{"$in":channels},"record_scope":SCOPE};
    if let Some(parent) = parent {
        if rows.find_one(doc! {"_id":parent,"user_id":owner,"channel_id":{"$in":channels},"record_scope":{"$ne":SCOPE}})
            .await?.is_none() {return Err(not_found());}
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
