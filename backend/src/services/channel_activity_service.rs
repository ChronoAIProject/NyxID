use bson::{Document, doc};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use serde::Serialize;

use crate::errors::AppResult;
use crate::models::channel_activity::{ActivityMetadata, NOTIFICATIONS_COLLECTION};
use crate::models::channel_message::{COLLECTION_NAME, ChannelMessage};

pub const RETENTION_DAYS: i64 = 30;

#[derive(Clone, Debug, Serialize)]
pub struct ActivityDescriptor {
    pub kind: &'static str,
    pub label: &'static str,
    pub subscription: &'static str,
    pub subscription_label: &'static str,
    pub description: &'static str,
    pub content_availability: &'static str,
    pub reply_supported: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallbackActivity {
    pub version: u8,
    pub kind: String,
    pub provider_event_type: String,
    pub platform_event_id: String,
    pub content_availability: String,
    pub reply_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
}

impl CallbackActivity {
    pub fn new(metadata: &ActivityMetadata, platform_event_id: &str) -> Self {
        Self {
            version: 1,
            kind: metadata.kind.clone(),
            provider_event_type: metadata.provider_event_type.clone(),
            platform_event_id: platform_event_id.into(),
            content_availability: metadata.content_availability.clone(),
            reply_supported: metadata.reply_supported,
            occurred_at: metadata.occurred_at.map(|date| date.to_rfc3339()),
        }
    }
}

pub fn notification_only(message: &ChannelMessage) -> bool {
    message
        .activity
        .as_ref()
        .is_some_and(|activity| !activity.reply_supported)
}

pub fn collection(message: &ChannelMessage) -> &'static str {
    if notification_only(message) {
        NOTIFICATIONS_COLLECTION
    } else {
        COLLECTION_NAME
    }
}

pub async fn update_callback_status(
    db: &mongodb::Database,
    message: &ChannelMessage,
    status: &str,
) -> AppResult<()> {
    db.collection::<Document>(collection(message))
        .update_one(
            doc! {"_id": &message.id, "user_id": &message.user_id},
            doc! {"$set": {"callback_status": status, "updated_at": bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

#[derive(Debug)]
pub struct ActivityPage {
    pub activities: Vec<ChannelMessage>,
    pub total: u64,
    pub routes: Vec<RouteActivity>,
}

#[derive(Debug)]
pub struct RouteActivity {
    pub conversation_id: String,
    pub count: u64,
    pub last: ChannelMessage,
}

/// Read admission metadata directly: no callback-dependent cache can become stale.
/// Both branches carry the same owner/resource/retention filter before unioning.
pub async fn list(
    db: &mongodb::Database,
    owner: &str,
    bot_id: Option<&str>,
    conversation_id: Option<&str>,
    kind: Option<&str>,
    page: u32,
    per_page: u32,
) -> AppResult<ActivityPage> {
    let mut filter = doc! {
        "user_id": owner, "direction": "inbound",
        "created_at": {"$gte": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(RETENTION_DAYS))},
    };
    if let Some(id) = bot_id {
        filter.insert("channel_bot_id", id);
    }
    if let Some(id) = conversation_id {
        filter.insert("conversation_id", id);
    }
    let mut items = Vec::new();
    if let Some(kind) = kind {
        items.push(if kind == "message" {
            doc! {"$match": {"activity": bson::Bson::Null}}
        } else {
            doc! {"$match": {"activity.kind": kind}}
        });
    }
    let count_pipeline = {
        let mut count = items.clone();
        count.push(doc! {"$count": "total"});
        count
    };
    items.extend([
        doc! {"$sort": {"created_at": -1, "_id": -1}},
        doc! {"$skip": i64::from(page.saturating_sub(1)) * i64::from(per_page)},
        doc! {"$limit": i64::from(per_page)},
    ]);
    let projection = doc! {"$project": {
        "_id": 1, "user_id": 1, "channel_bot_id": 1, "conversation_id": 1,
        "direction": 1, "platform": 1, "platform_conversation_id": 1,
        "platform_message_id": 1, "sender_platform_id": 1, "content_type": 1,
        "callback_status": 1, "created_at": 1, "activity": 1,
    }};
    let pipeline = vec![
        doc! {"$match": &filter},
        projection.clone(),
        doc! {"$unionWith": {"coll": NOTIFICATIONS_COLLECTION, "pipeline": [{"$match": &filter}, projection]}},
        doc! {"$facet": {
            "activities": items,
            "count": count_pipeline,
            "routes": [
                {"$sort": {"created_at": -1, "_id": -1}},
                {"$group": {"_id": "$conversation_id", "count": {"$sum": 1}, "last": {"$first": "$$ROOT"}}},
            ],
        }},
    ];
    let result = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate(pipeline)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_next()
        .await?
        .unwrap_or_default();
    let documents = |key: &str| -> Vec<Document> {
        result
            .get_array(key)
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_document().cloned())
            .collect()
    };
    let activities = documents("activities")
        .into_iter()
        .map(bson::from_document)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            crate::errors::AppError::Internal("Invalid channel activity metadata".into())
        })?;
    let mut routes = vec![];
    for row in documents("routes") {
        if let (Ok(id), Ok(last)) = (row.get_str("_id"), row.get_document("last")) {
            routes.push(RouteActivity {
                conversation_id: id.into(),
                count: number(&row, "count"),
                last: bson::from_document(last.clone()).map_err(|_| {
                    crate::errors::AppError::Internal("Invalid channel activity metadata".into())
                })?,
            });
        }
    }
    Ok(ActivityPage {
        activities,
        routes,
        total: documents("count")
            .first()
            .map_or(0, |row| number(row, "total")),
    })
}

fn number(doc: &Document, key: &str) -> u64 {
    doc.get_i64(key)
        .or_else(|_| doc.get_i32(key).map(i64::from))
        .unwrap_or(0)
        .max(0) as u64
}

pub async fn ensure_indexes(db: &mongodb::Database) -> mongodb::error::Result<()> {
    use mongodb::IndexModel;
    use mongodb::options::IndexOptions;
    for name in [COLLECTION_NAME, NOTIFICATIONS_COLLECTION] {
        let col = db.collection::<Document>(name);
        for key in ["channel_bot_id", "conversation_id"] {
            let mut keys = doc! {"user_id": 1};
            keys.insert(key, 1);
            keys.insert("direction", 1);
            keys.insert("created_at", -1);
            col.create_index(IndexModel::builder().keys(keys).build())
                .await?;
        }
    }
    db.collection::<Document>(NOTIFICATIONS_COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"created_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(30 * 86400))
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub fn occurrence(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn activity_union_preserves_legacy_history_and_filters_owner_route_kind_and_retention() {
        let db = crate::test_utils::connect_transaction_test_database("activity_union").await;
        ensure_indexes(&db).await.unwrap();
        let record = |id: &str, owner: &str, route: &str, age: i64| {
            doc! {
                "_id": id, "user_id": owner, "conversation_id": route, "channel_bot_id": "bot", "platform": "x",
                "direction": "inbound", "content_type": "text", "created_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(age)),
            }
        };
        let messages = db.collection::<Document>(COLLECTION_NAME);
        messages
            .insert_one(record("legacy", "owner", "route", 1))
            .await
            .unwrap();
        messages
            .insert_one(record("old", "owner", "route", 31))
            .await
            .unwrap();
        messages
            .insert_one(record("other", "other-owner", "route", 0))
            .await
            .unwrap();
        messages
            .insert_one(record("other-route", "owner", "other-route", 2))
            .await
            .unwrap();
        let mut outgoing = record("outbound", "owner", "route", 0);
        outgoing.insert("direction", "outbound");
        messages.insert_one(outgoing).await.unwrap();
        let mut notification = record("notification", "owner", "route", 0);
        notification.insert("activity", doc! {"kind":"encrypted_chat", "provider_event_type":"chat.received", "content_availability":"encrypted", "reply_supported":false});
        notification.insert("callback_status", "failed");
        db.collection::<Document>(NOTIFICATIONS_COLLECTION)
            .insert_one(notification)
            .await
            .unwrap();
        let result = list(&db, "owner", Some("bot"), Some("route"), None, 1, 20)
            .await
            .unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.routes[0].count, 2);
        assert_eq!(result.routes[0].last.id, "notification");
        assert_eq!(result.activities[1].id, "legacy");
        assert!(result.activities[1].activity.is_none());
        assert_eq!(
            list(&db, "owner", Some("bot"), None, None, 1, 20)
                .await
                .unwrap()
                .total,
            3
        );
        let filtered = list(
            &db,
            "owner",
            Some("bot"),
            Some("route"),
            Some("message"),
            1,
            20,
        )
        .await
        .unwrap();
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.activities[0].id, "legacy");
        assert_eq!(
            filtered.routes[0].count, 2,
            "route summaries always cover all kinds"
        );
        let page = list(&db, "owner", Some("bot"), Some("route"), None, 2, 1)
            .await
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.activities[0].id, "legacy");
        assert_eq!(
            messages
                .count_documents(doc! {"user_id": "owner", "conversation_id": "route"})
                .await
                .unwrap(),
            3,
            "notifications never enter old message history"
        );
    }
}
