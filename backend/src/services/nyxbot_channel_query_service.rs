use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::{COLLECTION_NAME, NyxbotChannel},
};

/// One current record per bot: a live connection wins over failed attempts;
/// otherwise the latest attempt or disconnection determines visibility.
pub async fn list_current(db: &Database, owner: &str) -> AppResult<Vec<NyxbotChannel>> {
    list_matching(db, doc! {"user_id":owner}).await
}

pub(crate) async fn list_matching(
    db: &Database,
    filter: bson::Document,
) -> AppResult<Vec<NyxbotChannel>> {
    let rows = db.collection::<NyxbotChannel>(COLLECTION_NAME).aggregate([
        doc! {"$match": filter},
        doc! {"$set": {"live_rank": {"$cond": [{"$in": ["$status", ["pending", "active"]]}, 1, 0]}}},
        doc! {"$sort": {"channel_bot_id": 1, "live_rank": -1, "created_at": -1, "_id": -1}},
        doc! {"$group": {"_id": "$channel_bot_id", "channel": {"$first": "$$ROOT"}}},
        doc! {"$replaceRoot": {"newRoot": "$channel"}},
        doc! {"$unset": "live_rank"},
        doc! {"$match": {"status": {"$ne": "disconnected"}}},
        doc! {"$sort": {"created_at": -1, "_id": -1}},
        doc! {"$limit": 50},
    ]).await?;
    Ok(rows.with_type::<NyxbotChannel>().try_collect().await?)
}

pub(crate) async fn active_for_bot(
    db: &Database,
    owner: &str,
    bot_id: &str,
) -> AppResult<Option<NyxbotChannel>> {
    Ok(db
        .collection::<NyxbotChannel>(COLLECTION_NAME)
        .find_one(
            doc! {"user_id":owner,"channel_bot_id":bot_id,"status":{"$in":["pending","active"]}},
        )
        .sort(doc! {"created_at":-1,"_id":-1})
        .await?)
}

/// Connect and its read-only preview must choose the same reusable binding.
pub(crate) async fn reusable(
    db: &Database,
    actor: &str,
    bot_owner: &str,
    row: &NyxbotChannel,
) -> AppResult<bool> {
    let owner = row.bot_owner_id.as_deref().unwrap_or(&row.user_id);
    let keys_alive = key_alive(db, owner, &row.route_api_key_id).await?
        && match row.agent_api_key_id.as_deref() {
            Some(key) => key_alive(db, actor, key).await?,
            None => true,
        };
    let route_alive = match row.route_id.as_deref() {
        Some(id) => {
            db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .count_documents(doc! {"_id":id,"user_id":bot_owner,"is_active":true})
                .await?
                > 0
        }
        None => false,
    };
    Ok(row.status == "active"
        && keys_alive
        && route_alive
        && row.delivery_status.as_deref() != Some("failing")
        && owner == bot_owner)
}

pub(crate) fn direct_callback(base_url: &str, channel_id: &str) -> String {
    format!(
        "{}/api/v1/nyxbot/relay/{channel_id}",
        base_url.trim_end_matches('/')
    )
}

/// All direct Meta setup paths promise one dedicated default callback.
/// Explicit per-chat overrides and other platforms retain their policies.
pub(crate) async fn reusable_meta(
    db: &Database,
    actor: &str,
    owner: &str,
    row: &NyxbotChannel,
    base_url: &str,
) -> AppResult<bool> {
    if row.transport != "direct" || !super::channel_adapters::meta::is_meta(&row.platform) {
        return reusable(db, actor, owner, row).await;
    }
    if row.bot_owner_id.as_deref().unwrap_or(&row.user_id) != owner {
        return Ok(false);
    }
    meta_default_matches(db, row, &direct_callback(base_url, &row.id), None).await
}

pub(crate) async fn meta_default_matches(
    db: &Database,
    row: &NyxbotChannel,
    callback: &str,
    tx: Option<&mut mongodb::ClientSession>,
) -> AppResult<bool> {
    if row.status != "active" || row.delivery_status.as_deref() == Some("failing") {
        return Ok(false);
    }
    meta_route_matches(db, row, callback, tx).await
}

/// The same committed route/key proof is used by reuse, repair and activation.
/// Pending activation supplies its candidate route ID without pretending that
/// the channel has already committed its active lifecycle state.
pub(crate) async fn meta_route_matches(
    db: &Database,
    row: &NyxbotChannel,
    callback: &str,
    mut tx: Option<&mut mongodb::ClientSession>,
) -> AppResult<bool> {
    let owner = row.bot_owner_id.as_deref().unwrap_or(&row.user_id);
    let keys =
        db.collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME);
    let key = keys.find_one(
        doc! {"_id": &row.route_api_key_id, "user_id": owner, "is_active": true,
        "purpose": {"$nin": ["public_nyxbot", "shared_entry_transport"]}},
    );
    let key = if let Some(tx) = tx.as_deref_mut() {
        key.session(tx).await?
    } else {
        key.await?
    };
    let Some(key) = key else { return Ok(false) };
    if super::channel_routing_service::ensure_route_agent(&key, owner).is_err()
        || key.callback_url.as_deref() != Some(callback)
    {
        return Ok(false);
    }
    if let Some(id) = &row.agent_api_key_id {
        let agent = keys.find_one(doc! {"_id": id, "user_id": &row.user_id, "is_active": true,
        "purpose": {"$nin": ["public_nyxbot", "shared_entry_transport"]}});
        let agent = if let Some(tx) = tx.as_deref_mut() {
            agent.session(tx).await?
        } else {
            agent.await?
        };
        if agent.is_none_or(|key| {
            key.expires_at
                .is_some_and(|expiry| expiry <= chrono::Utc::now())
        }) {
            return Ok(false);
        }
    }
    let Some(route) = row.route_id.as_deref() else {
        return Ok(false);
    };
    let routes =
        db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME);
    let found = routes
        .find_one(
            doc! {"_id": route, "user_id": owner, "channel_bot_id": &row.channel_bot_id,
            "platform": &row.platform, "is_active": true, "default_agent": true,
            "retired_by_transfer": {"$ne": true}, "agent_api_key_id": &row.route_api_key_id},
        )
        .projection(doc! {"_id": 1});
    let found = if let Some(tx) = tx.as_deref_mut() {
        found.session(tx).await?
    } else {
        found.await?
    };
    if found.is_none() {
        return Ok(false);
    }
    let defaults = routes
        .count_documents(doc! {
            "channel_bot_id": &row.channel_bot_id, "user_id": owner,
            "default_agent": true, "is_active": true, "retired_by_transfer": {"$ne": true},
        })
        .limit(2);
    let count = if let Some(tx) = tx {
        defaults.session(tx).await?
    } else {
        defaults.await?
    };
    Ok(count == 1)
}

/// Caller holds the bot write fence, shared with ordinary route mutations.
/// A rebound default is an owner-selected competitor even if it reused the
/// original route ID. Non-default per-conversation routes are independent.
pub(crate) async fn require_no_competing_meta_default(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    row: &NyxbotChannel,
) -> AppResult<()> {
    let owner = row.bot_owner_id.as_deref().unwrap_or(&row.user_id);
    let mut identity = vec![
        bson::bson!({"user_id": {"$ne": owner}}),
        bson::bson!({"agent_api_key_id": {"$ne": &row.route_api_key_id}}),
    ];
    if let Some(route) = &row.route_id {
        identity.push(bson::bson!({"_id": {"$ne": route}}));
    }
    // A crash may leave route_id unset, but the dedicated key still identifies
    // that incomplete route. Protect any other active unscoped wildcard too:
    // it occupies the unique tuple even when it is not a default or owner route.
    let filter = doc! {"channel_bot_id": &row.channel_bot_id, "is_active": true,
    "$and": [
        {"$or": [
            {"user_id": owner, "default_agent": true, "retired_by_transfer": {"$ne": true}},
            {"platform_conversation_id": "*", "platform_sender_id": null}
        ]},
        {"$or": identity}
    ]};
    let other = db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .find_one(filter)
        .projection(doc! {"_id": 1})
        .session(tx)
        .await?;
    if other.is_some() {
        return Err(AppError::Conflict(
            "This bot has another default or unscoped wildcard route. Review its routing before assigning a specialist"
                .into(),
        ));
    }
    Ok(())
}

async fn key_alive(db: &Database, owner: &str, key: &str) -> AppResult<bool> {
    match super::key_service::get_api_key(db, owner, key).await {
        Ok(key) => Ok(key.is_active),
        Err(AppError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::Document;
    use uuid::Uuid;

    fn channel(owner: &str, bot: &str, status: &str, at: i64) -> Document {
        doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": owner, "channel_bot_id": bot,
            "platform": "telegram", "bot_label": "Test bot", "transport": "gateway",
            "status": status, "route_api_key_id": Uuid::new_v4().to_string(),
            "created_at": bson::DateTime::from_millis(at), "updated_at": bson::DateTime::from_millis(at),
        }
    }

    #[tokio::test]
    async fn current_channels_rank_live_then_latest_before_limiting_and_order_ties() {
        let db = crate::test_utils::connect_transaction_test_database("channel_current").await;
        for (keys, name) in [
            (doc! {"user_id": 1, "channel_bot_id": 1}, "legacy_owner_bot"),
            (
                doc! {"channel_bot_id": 1, "user_id": 1},
                "reverse_owner_bot",
            ),
        ] {
            db.collection::<Document>(COLLECTION_NAME)
                .create_index(
                    mongodb::IndexModel::builder()
                        .keys(keys)
                        .options(
                            mongodb::options::IndexOptions::builder()
                                .name(name.to_owned())
                                .unique(false)
                                .build(),
                        )
                        .build(),
                )
                .await
                .unwrap();
        }
        crate::services::assistant_nyxagent::ensure_indexes(&db)
            .await
            .unwrap();
        crate::services::assistant_nyxagent::ensure_indexes(&db)
            .await
            .unwrap();
        let owner = Uuid::new_v4().to_string();
        let live_bot = Uuid::new_v4().to_string();
        let failed_bot = Uuid::new_v4().to_string();
        let disconnected_bot = Uuid::new_v4().to_string();
        let live = channel(&owner, &live_bot, "active", 5);
        let pending = channel(&owner, &Uuid::new_v4().to_string(), "pending", 6);
        let tie_a = channel(&owner, &failed_bot, "failed", 10);
        let tie_b = channel(&owner, &failed_bot, "failed", 10);
        let tie_winner = [tie_a.clone(), tie_b.clone()]
            .into_iter()
            .max_by(|a, b| a.get_str("_id").unwrap().cmp(b.get_str("_id").unwrap()))
            .unwrap();
        let mut rows = vec![
            live.clone(),
            pending.clone(),
            tie_a,
            tie_b,
            channel(&owner, &disconnected_bot, "failed", 20),
            channel(&owner, &disconnected_bot, "disconnected", 21),
            channel(&Uuid::new_v4().to_string(), &live_bot, "active", 1000),
        ];
        for i in 100..180 {
            rows.push(channel(&owner, &live_bot, "failed", i));
        }
        db.collection::<Document>(COLLECTION_NAME)
            .insert_many(rows)
            .await
            .unwrap();
        let result = list_current(&db, &owner).await.unwrap();
        assert_eq!(
            result.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec![
                tie_winner.get_str("_id").unwrap(),
                pending.get_str("_id").unwrap(),
                live.get_str("_id").unwrap()
            ]
        );
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            87
        );
        let mut rows = Vec::new();
        for _ in 0..60 {
            rows.push(channel(&owner, &Uuid::new_v4().to_string(), "failed", 500));
        }
        let mut expected: Vec<_> = rows
            .iter()
            .map(|row| row.get_str("_id").unwrap().to_owned())
            .collect();
        expected.sort_by(|a, b| b.cmp(a));
        expected.truncate(50);
        db.collection::<Document>(COLLECTION_NAME)
            .insert_many(rows)
            .await
            .unwrap();
        assert_eq!(
            list_current(&db, &owner)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            expected
        );
        let indexes: Vec<_> = db
            .collection::<Document>(COLLECTION_NAME)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let owner_bot_indexes: Vec<_> = indexes
            .iter()
            .filter(|index| {
                index
                    .keys
                    .keys()
                    .map(String::as_str)
                    .take(2)
                    .eq(["user_id", "channel_bot_id"])
            })
            .collect();
        assert_eq!(owner_bot_indexes.len(), 1);
        assert_eq!(
            owner_bot_indexes[0].keys,
            doc! {"user_id":1,"channel_bot_id":1,"created_at":-1,"_id":-1}
        );
        assert!(
            owner_bot_indexes[0]
                .options
                .as_ref()
                .is_none_or(|options| options.unique != Some(true))
        );
        assert!(indexes.iter().any(|index| {
            index
                .keys
                .iter()
                .eq(doc! {"channel_bot_id": 1, "user_id": 1}.iter())
        }));
        db.drop().await.unwrap();
    }
}
