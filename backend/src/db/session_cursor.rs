use mongodb::{ClientSession, SessionCursor, error::Result};
use serde::de::DeserializeOwned;

/// MongoDB 3.7's session stream can panic on drop during getMore. Direct
/// advance keeps the buffer in the cursor. Discard the cursor after cancellation.
pub(crate) async fn next_document<T: DeserializeOwned + Send>(
    cursor: &mut SessionCursor<T>,
    session: &mut ClientSession,
) -> Result<Option<T>> {
    if cursor.advance(session).await? {
        Ok(Some(cursor.deserialize_current()?))
    } else {
        Ok(None)
    }
}

pub(crate) async fn read_documents<T: DeserializeOwned + Send>(
    cursor: &mut SessionCursor<T>,
    session: &mut ClientSession,
) -> Result<Vec<T>> {
    let mut rows = Vec::new();
    while let Some(row) = next_document(cursor, session).await? {
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::{
        bson::{Document, doc},
        event::{EventHandler, command::CommandEvent},
        options::CursorType,
    };
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn cancelling_next_document_during_get_more_releases_the_session() {
        let started = Arc::new(Notify::new());
        let seen = started.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "session_cursor_drop",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event
                    && event.command_name == "getMore"
                    && event.command.get_str("collection") == Ok("events")
                {
                    seen.notify_one();
                }
            }),
        )
        .await;
        // An idle tailable cursor keeps getMore pending on this collection alone.
        db.create_collection("events")
            .capped(true)
            .size(4096)
            .await
            .unwrap();
        let events = db.collection::<Document>("events");
        events.insert_one(doc! {"_id": 1}).await.unwrap();
        let mut session = db.client().start_session().await.unwrap();
        let mut cursor = events
            .find(doc! {})
            .cursor_type(CursorType::TailableAwait)
            .max_await_time(Duration::from_secs(30))
            .session(&mut session)
            .await
            .unwrap();
        assert_eq!(
            next_document(&mut cursor, &mut session)
                .await
                .unwrap()
                .unwrap()
                .get_i32("_id"),
            Ok(1)
        );
        tokio::select! {
            started = tokio::time::timeout(Duration::from_secs(10), started.notified()) => started.expect("getMore started"),
            _ = next_document(&mut cursor, &mut session) => panic!("idle cursor ended before cancellation"),
        }
        drop(cursor);
        assert_eq!(
            events
                .count_documents(doc! {})
                .session(&mut session)
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }
}
