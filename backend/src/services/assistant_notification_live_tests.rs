use super::*;

async fn next_notification(
    stream: &mut mongodb::change_stream::ChangeStream<ChangeStreamEvent<Document>>,
) -> LiveEvent {
    let change = tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("notification change arrives")
        .expect("change stream remains open")
        .expect("change stream succeeds");
    let full = change.full_document.as_ref().expect("projected document");
    for field in ["source", "preview", "text", "push", "count", "latest"] {
        assert!(
            !full.contains_key(field),
            "MongoDB must remove notification {field} before delivery"
        );
    }
    decode(&change).expect("notification change decodes")
}

#[tokio::test]
async fn notification_stream_projects_content_and_routes_visible_changes_only_to_recipient() {
    let db = crate::test_utils::connect_transaction_test_database("notification_live").await;
    let mut stream = db
        .watch()
        .pipeline(pipeline())
        .full_document(FullDocumentType::UpdateLookup)
        .await
        .unwrap();
    let rows = db.collection::<Document>(crate::models::assistant_notification::COLLECTION_NAME);
    let owner = uuid::Uuid::new_v4().to_string();
    let other_person = uuid::Uuid::new_v4().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let sentinel = uuid::Uuid::new_v4().to_string();
    let live = AssistantLive::new();
    let mut recipient = live.subscribe_owner(&owner).unwrap();
    let mut unrelated = live.subscribe_owner(&other_person).unwrap();
    rows.insert_one(doc! {
        "_id":&id,"user_id":&owner,"unread":true,"count":1_i64,
        "updated_at":bson::DateTime::now(),"source":{"id":"private-source"},
        "preview":"must-never-leave-mongodb","text":"must-never-leave-mongodb",
        "push":{"state":"pending"},
    })
    .await
    .unwrap();
    let expected = LiveEvent::Notification {
        id: id.clone(),
        user_id: owner.clone(),
    };
    let event = next_notification(&mut stream).await;
    assert_eq!(event, expected);
    live.publish(event);
    assert_eq!(recipient.events.try_recv().unwrap(), expected);
    assert!(unrelated.events.try_recv().is_err());

    // Coalescing still refreshes clients when two activities share the same
    // BSON millisecond and MongoDB reports only the increment as changed.
    rows.update_one(doc! {"_id":&id}, doc! {"$inc":{"count":1_i64}})
        .await
        .unwrap();
    assert_eq!(next_notification(&mut stream).await, expected);

    // Reading on one device and resolving a card both refresh other devices.
    rows.update_one(doc! {"_id":&id}, doc! {"$set":{"unread":false}})
        .await
        .unwrap();
    assert_eq!(next_notification(&mut stream).await, expected);
    rows.update_one(
        doc! {"_id":&id},
        doc! {"$set":{"resolved_at":bson::DateTime::now()}},
    )
    .await
    .unwrap();
    assert_eq!(next_notification(&mut stream).await, expected);

    // The sentinel follows push bookkeeping in oplog order. Seeing it next
    // proves a push-only change cannot make clients reload their feed.
    rows.update_one(doc! {"_id":&id}, doc! {"$set":{"push.state":"sending"}})
        .await
        .unwrap();
    rows.insert_one(doc! {"_id":&sentinel,"user_id":&owner})
        .await
        .unwrap();
    assert_eq!(
        next_notification(&mut stream).await,
        LiveEvent::Notification {
            id: sentinel,
            user_id: owner,
        }
    );
    drop(stream);
    db.drop().await.unwrap();
}
