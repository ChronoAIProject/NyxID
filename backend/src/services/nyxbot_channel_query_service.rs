use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

use crate::{
    errors::AppResult,
    models::nyxbot_channel::{COLLECTION_NAME, NyxbotChannel},
};

/// One current record per bot: a live connection wins over failed attempts;
/// otherwise the latest attempt or disconnection determines visibility.
pub async fn list_current(db: &Database, owner: &str) -> AppResult<Vec<NyxbotChannel>> {
    let rows = db.collection::<NyxbotChannel>(COLLECTION_NAME).aggregate([
        doc! {"$match": {"user_id": owner}},
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
