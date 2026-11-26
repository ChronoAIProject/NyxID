use super::*;

#[tokio::test]
async fn stuck_request_exit_dead_ordinary_returns_hold_once() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    f.db.collection::<bson::Document>("security_replicas").insert_one(doc!{"_id":"survivor","generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}).await.unwrap();
    f.db.collection::<bson::Document>(USAGE_METER).update_many(doc!{},doc!{"$set":{
        "created_at":bson::DateTime::from_millis(0),"forwarded_owner":{"generation_id":"dead","marked_at":bson::DateTime::from_millis(0),"ingress":"proxy"}
    }}).await.unwrap();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    let row = f.row().await;
    let held = wallet(&f.db, &f.owner).await.reserved_credits;
    f.db.drop().await.unwrap();
    assert!(
        row.released,
        "a dead generation's ordinary unknown request must finish"
    );
    assert_eq!(row.quantity, Some(0));
    assert_eq!(held, Credits::ZERO);
}
