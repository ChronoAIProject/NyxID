use bson::doc;
use chrono::{Duration, Utc};
use futures::{StreamExt, stream};
use mongodb::options::ReturnDocument;

use super::telegram_new_service::TelegramNewService;
use crate::errors::AppResult;
use crate::models::telegram_bot_request::{COLLECTION_NAME, TelegramBotRequest};

impl TelegramNewService<'_> {
    pub async fn complete_pending_creations(&self) -> AppResult<()> {
        self.expire().await?;
        let requests = self.db.collection::<TelegramBotRequest>(COLLECTION_NAME);
        let mut claimed = Vec::new();
        for _ in 0..25 {
            let Some(request) = requests.find_one_and_update(
                doc! {
                    "auto_connect": true, "active": true,
                    "status": {"$in": ["ready", "provisioning"]},
                    "$or": [{"next_connection_attempt_at": null}, {"next_connection_attempt_at": {"$lte": bson::DateTime::now()}}],
                },
                doc! {
                    "$set": {"next_connection_attempt_at": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(60))},
                    "$inc": {"connection_attempts": 1_i64},
                },
            ).sort(doc! {"next_connection_attempt_at": 1, "created_at": 1})
                .return_document(ReturnDocument::After).await? else { break };
            claimed.push(request);
        }
        stream::iter(claimed).for_each_concurrent(8, |request| {
            let requests = &requests;
            async move {
            let Some(bot_id) = request.telegram_bot_id else { return };
            if self.connect(&request.actor_user_id, &request.id, bot_id, request.revision).await.is_err() {
                let delay = 2_i64.pow(request.connection_attempts.min(5));
                let _ = requests.update_one(
                    doc! {"_id": &request.id, "auto_connect": true, "active": true, "status": {"$in": ["ready", "provisioning"]}},
                    doc! {"$set": {
                        "next_connection_attempt_at": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(delay)),
                        "connection_error": "Telegram setup is taking longer than usual. We are retrying automatically.",
                    }},
                ).await;
                tracing::warn!(request_id = %request.id, "Telegram creation connection will retry");
            }
            }
        }).await;
        Ok(())
    }
}
