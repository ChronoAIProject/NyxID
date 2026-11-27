use super::*;
use crate::models::platform_native_reply_handle::COLLECTION_NAME as HANDLES;
use bson::doc;

#[tokio::test]
async fn cleanup_is_bounded_and_removes_children_with_the_expired_parent() {
    let db = crate::test_utils::connect_transaction_test_database("native_receipt_cleanup").await;
    let parents = db.collection::<Document>(NATIVE_RECEIPTS);
    let handles = db.collection::<Document>(HANDLES);
    let expired = bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::minutes(1));
    let first = "receipt-a".to_owned();
    let second = "receipt-b".to_owned();
    parents
        .insert_many([
            doc! {"_id": &first, "expires_at": bson::DateTime::from_millis(expired.timestamp_millis() - 1)},
            doc! {"_id": &second, "expires_at": expired, "cleanup_fence": 0_i64},
        ])
        .await
        .unwrap();
    handles
        .insert_many([
            doc! {"_id": uuid::Uuid::new_v4().to_string(), "receipt_id": &first},
            doc! {"_id": uuid::Uuid::new_v4().to_string(), "receipt_id": &first},
            doc! {"_id": uuid::Uuid::new_v4().to_string(), "receipt_id": &second},
        ])
        .await
        .unwrap();

    assert_eq!(cleanup_expired(&db, 1).await.unwrap(), 1);
    assert_eq!(parents.count_documents(doc! {}).await.unwrap(), 1);
    assert_eq!(handles.count_documents(doc! {}).await.unwrap(), 1);
    let first_exists = parents
        .find_one(doc! {"_id": &first})
        .await
        .unwrap()
        .is_some();
    let second_exists = parents
        .find_one(doc! {"_id": &second})
        .await
        .unwrap()
        .is_some();
    assert_ne!(first_exists, second_exists);
    let remaining = if first_exists { &first } else { &second };
    assert!(
        handles
            .find_one(doc! {"receipt_id": remaining})
            .await
            .unwrap()
            .is_some()
    );

    assert_eq!(cleanup_expired(&db, 8).await.unwrap(), 1);
    assert_eq!(parents.count_documents(doc! {}).await.unwrap(), 0);
    assert_eq!(handles.count_documents(doc! {}).await.unwrap(), 0);

    db.drop().await.unwrap();
}
