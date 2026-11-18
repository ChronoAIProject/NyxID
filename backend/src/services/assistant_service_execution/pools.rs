use super::*;

/// A pending/decided pool card pins its actual chosen member durably. A balance
/// tick on another request or replica cannot silently change the approved key.
pub async fn pinned_pool_member(
    db: &Database,
    nodes: &crate::services::node_ws_manager::NodeWsManager,
    chat: Option<&acks::ChatAuthority>,
    pool: &str,
    acknowledgement: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(chat) = chat else {
        return Ok(None);
    };
    if !chat.turn_live || chat.turn_stopped {
        return Ok(None);
    }
    if is_shared_chat(chat) {
        let mut shared_filter = scope(chat)?;
        shared_filter.extend(doc! {"kind": "shared_pool_selection", "operation.pool_id": pool});
        if let Some(pinned) = db
            .collection::<Document>(COLLECTION)
            .find_one(shared_filter)
            .await?
        {
            return Ok(Some(
                pinned
                    .get_str("connection_id")
                    .map_err(|_| AppError::AssistantTurnRequired)?
                    .into(),
            ));
        }
    }
    let mut filter = scope(chat)?;
    filter.extend(doc! {"kind": {"$in": ["selection", "card_binding"]}, "operation.pool_id": pool});
    let rows: Vec<Document> = db
        .collection(COLLECTION)
        .find(filter)
        .await?
        .try_collect()
        .await?;
    let mut latest = None;
    for row in rows {
        let card = if row.get_str("kind") == Ok("card_binding") {
            row.get_str("_id").ok()
        } else {
            row.get_str("acknowledgement_id").ok()
        };
        let Some(card) = card else {
            continue;
        };
        if acknowledgement.is_some_and(|requested| requested != card) {
            continue;
        }
        let status = db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {
                "_id":card,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
                "api_key_id":&chat.api_key_id,"expires_at":{"$gt":bson::DateTime::now()},
            })
            .await?;
        let Some(status) = status else {
            continue;
        };
        let created = status
            .get_datetime("created_at")
            .copied()
            .map_err(|_| AppError::Conflict("Pool confirmation lost its creation time".into()))?;
        if latest.as_ref().is_none_or(|(date, _, _)| created > *date) {
            latest = Some((created, row, status));
        }
    }
    if let Some((_, row, status)) = latest {
        if !matches!(status.get_str("status"), Ok("pending" | "allowed" | "used")) {
            return Err(AppError::Forbidden(
                "Pool connection use was not authorized".into(),
            ));
        }
        if let Ok(operation) = row.get_document("operation")
            && let Ok(revision) = operation.get_i64("pool_revision")
                && db.collection::<Document>("service_pools").find_one(doc! {
                    "_id":pool,"is_active":true,"config_revision":revision,
                    "members":{"$elemMatch":{"user_service_id":row.get_str("connection_id").ok(),"enabled":true}},
                }).await?.is_none() { return Err(AppError::Conflict("Confirmed service pool changed".into())); }
        let id = row
            .get_str("connection_id")
            .map_err(|_| AppError::Conflict("Pool confirmation lost its member".into()))?;
        if row.get_str("binding")
            != Ok(connection_binding(db, id, &chat.api_key_id).await?.as_str())
        {
            return Err(AppError::Conflict("Confirmed pool member changed".into()));
        }
        if let Some(service) =
            crate::services::user_service_service::find_user_service_by_id(db, id).await?
            && let Ok(operation) = row.get_document("operation")
        {
            let operation: Value = bson::from_document(operation.clone())
                .map_err(|_| AppError::Conflict("Invalid pool confirmation".into()))?;
            if verified_unavailable(db, nodes, chat, &service, &operation).await? {
                return Ok(None);
            }
        }
        return Ok(Some(id.to_owned()));
    }
    if acknowledgement.is_some() {
        return Err(AppError::Conflict(
            "Pool confirmation expired or belongs to another operation".into(),
        ));
    }
    Ok(None)
}

/// Freeze the first concrete account before prompting; retries and pool health
/// changes cannot select another person's credential behind the consent card.
pub(super) async fn pin_shared_pool(
    db: &Database,
    chat: &acks::ChatAuthority,
    connection: &str,
    binding: &str,
    operation: &Value,
) -> AppResult<()> {
    let pool = operation["pool_id"]
        .as_str()
        .ok_or(AppError::AssistantTurnRequired)?;
    let id = digest(json!([
        "shared-pool.v1",
        chat.user_id,
        chat.conversation_id,
        chat.turn_id,
        pool
    ]));
    let mut row = scope(chat)?;
    row.extend(doc! {"_id": uuid::Uuid::new_v4().to_string(), "shared_pool_scope": &id, "kind": "shared_pool_selection", "connection_id": connection,
        "binding": binding, "operation": bson::to_bson(operation).map_err(|_| AppError::AssistantTurnRequired)?,
        "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(2))});
    let rows = db.collection::<Document>(COLLECTION);
    if let Err(error) = rows
        .update_one(doc! {"shared_pool_scope": &id}, doc! {"$setOnInsert": row})
        .upsert(true)
        .await
        && !crate::services::service_pool_service::is_duplicate_key(&error)
    {
        return Err(error.into());
    }
    if rows
        .find_one(doc! {"shared_pool_scope": &id, "connection_id": connection, "binding": binding})
        .await?
        .is_none()
    {
        return Err(AppError::Forbidden("The pool's exact selected connection changed; start a new turn to choose another account".into()));
    }
    Ok(())
}
