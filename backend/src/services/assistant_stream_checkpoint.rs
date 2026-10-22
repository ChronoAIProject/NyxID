//! Coalesced, turn-fenced recovery of text already made safe by the relay.
use std::time::Duration;

use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::Value;
use tokio::sync::watch;
use tokio_util::task::AbortOnDropHandle;

use crate::models::assistant_conversation::{COLLECTION_NAME, StreamReply};

pub(crate) fn observe(reply: &mut StreamReply, event: &Value) {
    let cursor = event["cursor"].as_i64().unwrap_or(0);
    if event["block_id"].as_str() != Some(&reply.block_id) || cursor <= reply.cursor {
        return;
    }
    match event["event"].as_str() {
        Some("block.delta") => {
            if let Some(text) = event["text"].as_str() {
                reply.text.push_str(text);
            }
        }
        Some("block.completed") => {
            if let Some(text) = event["block"]["text"].as_str() {
                reply.text = text.into();
            }
        }
        _ => return,
    }
    reply.cursor = cursor;
}

pub(crate) fn start(
    db: Database,
    conversation: &str,
    user: &str,
    turn: &str,
    message_id: String,
    block_id: String,
) -> (watch::Sender<StreamReply>, AbortOnDropHandle<()>) {
    let (sender, mut receiver) = watch::channel(StreamReply {
        message_id,
        block_id,
        cursor: 0,
        text: String::new(),
    });
    let conversation = conversation.to_owned();
    let user = user.to_owned();
    let turn = turn.to_owned();
    let task = AbortOnDropHandle::new(tokio::spawn(async move {
        let collection = db.collection::<bson::Document>(COLLECTION_NAME);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut saved_cursor = 0;
        loop {
            tick.tick().await;
            let reply = receiver.borrow_and_update().clone();
            if reply.cursor <= saved_cursor {
                continue;
            }
            let Ok(value) = bson::to_bson(&reply) else {
                continue;
            };
            let write = collection.update_one(
                doc! {"_id": &conversation, "user_id": &user, "active_turn.turn_id": &turn,
                "active_turn.stop_requested": false,
                "$or": [
                    {"active_turn.reply.cursor": {"$exists": false}},
                    {"active_turn.reply.cursor": {"$lt": reply.cursor}}
                ]},
                doc! {"$set": {"active_turn.reply": value}},
            );
            match tokio::time::timeout(
                Duration::from_secs(2),
                std::future::IntoFuture::into_future(write),
            )
            .await
            {
                Ok(Ok(result)) if result.matched_count == 0 => break,
                Ok(Ok(_)) => saved_cursor = reply.cursor,
                // Keep the latest text and retry; checkpoint outages cannot stall tokens.
                _ => {}
            }
        }
    }));
    (sender, task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checkpoints_recover_unicode_and_authoritative_text_without_duplicate_deltas() {
        let mut reply = StreamReply {
            message_id: "message".into(),
            block_id: "block".into(),
            cursor: 0,
            text: String::new(),
        };
        for event in [
            json!({"event":"block.delta", "block_id":"block", "cursor":4, "text":"Hello 界"}),
            json!({"event":"block.delta", "block_id":"other", "cursor":5, "text":"wrong block"}),
            json!({"event":"block.delta", "block_id":"block", "cursor":4, "text":"duplicate"}),
        ] {
            observe(&mut reply, &event);
        }
        assert_eq!(reply.text, "Hello 界");
        observe(
            &mut reply,
            &json!({"event":"block.completed", "block_id":"block", "cursor":6, "block":{"text":"Revised answer 🌏"}}),
        );
        assert_eq!(reply.text, "Revised answer 🌏");
        assert_eq!(reply.cursor, 6);
    }

    #[tokio::test]
    async fn checkpoints_are_durable_coalesced_and_cannot_recreate_a_settled_turn() {
        let Some(db) = crate::test_utils::connect_test_database("stream_checkpoint").await else {
            return;
        };
        let collection = db.collection::<bson::Document>(COLLECTION_NAME);
        collection.insert_one(doc! {"_id":"conversation", "user_id":"owner", "active_turn":{"turn_id":"turn", "stop_requested":false}}).await.unwrap();
        let (sender, task) = start(
            db.clone(),
            "conversation",
            "owner",
            "turn",
            "message".into(),
            "block".into(),
        );
        for cursor in 1..=100 {
            sender.send_modify(|reply| observe(reply, &json!({"event":"block.delta", "block_id":"block", "cursor":cursor, "text":"界"})));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let row = collection
                    .find_one(doc! {"_id":"conversation"})
                    .await
                    .unwrap()
                    .unwrap();
                if let Ok(reply) = row
                    .get_document("active_turn")
                    .unwrap()
                    .get_document("reply")
                {
                    assert_eq!(reply.get_i64("cursor").unwrap(), 100);
                    assert_eq!(reply.get_str("text").unwrap(), "界".repeat(100));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        collection
            .update_one(
                doc! {"_id":"conversation"},
                doc! {"$set":{"active_turn":{"turn_id":"next", "stop_requested":false}}},
            )
            .await
            .unwrap();
        sender.send_modify(|reply| {
            observe(
                reply,
                &json!({"event":"block.delta", "block_id":"block", "cursor":101, "text":"late"}),
            )
        });
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        let row = collection
            .find_one(doc! {"_id":"conversation"})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !row.get_document("active_turn")
                .unwrap()
                .contains_key("reply")
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn checkpoints_reject_wrong_owners_and_stopped_turns() {
        let Some(db) = crate::test_utils::connect_test_database("checkpoint_fence").await else {
            return;
        };
        let collection = db.collection::<bson::Document>(COLLECTION_NAME);
        for (id, owner, stopped) in [
            ("wrong-owner", "someone-else", false),
            ("stopped", "owner", true),
        ] {
            collection.insert_one(doc! {"_id":id, "user_id":owner, "active_turn":{"turn_id":"turn", "stop_requested":stopped}}).await.unwrap();
            let (sender, task) = start(
                db.clone(),
                id,
                "owner",
                "turn",
                "message".into(),
                "block".into(),
            );
            sender.send_modify(|reply| {
                reply.cursor = 1;
                reply.text = "safe text".into();
            });
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap();
            let row = collection.find_one(doc! {"_id":id}).await.unwrap().unwrap();
            assert!(
                !row.get_document("active_turn")
                    .unwrap()
                    .contains_key("reply")
            );
        }
        db.drop().await.unwrap();
    }
}
