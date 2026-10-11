use crate::models::{assistant_group::{GroupMessage, MESSAGES_COLLECTION_NAME}, assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGE_COLLECTION}, assistant_message_feedback::{AssistantMessageFeedback, COLLECTION_NAME}};
use mongodb::{bson::doc, Database};

pub async fn conversation_message(db: &Database, user_id: &str, conversation_id: &str, message_id: &str) -> mongodb::error::Result<Option<AssistantMessage>> {
    db.collection::<AssistantMessage>(MESSAGE_COLLECTION)
        .find_one(doc! {"_id": message_id, "conversation_id": conversation_id, "user_id": user_id})
        .await
}

pub async fn group_message(db: &Database, group_id: &str, message_id: &str) -> mongodb::error::Result<Option<GroupMessage>> {
    db.collection::<GroupMessage>(MESSAGES_COLLECTION_NAME)
        .find_one(doc! {"_id": message_id, "group_id": group_id})
        .await
}

pub async fn feedback_for_messages(db: &Database, user_id: &str, message_ids: &[String]) -> mongodb::error::Result<Vec<AssistantMessageFeedback>> {
    db.collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find(doc! {"user_id": user_id, "message_id": {"$in": message_ids}})
        .await?.try_collect().await
}

use futures::TryStreamExt;
