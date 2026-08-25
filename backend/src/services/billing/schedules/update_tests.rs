use chrono::Duration;
use mongodb::bson::{self, doc};

use super::*;
use crate::models::billing_target::BillingTargetKind;
use crate::models::credit_schedule::{CreditExpiryPolicy, ScheduleRecurrence};
use crate::test_utils::connect_test_database;

#[tokio::test]
async fn stale_schedule_update_returns_conflict_without_overwriting_winner() {
    let Some(db) = connect_test_database("credit_schedule_update_conflict").await else {
        return;
    };
    let schedule = create_schedule(
        &db,
        CreateScheduleInput {
            amount_credits: 25,
            recurrence: ScheduleRecurrence::Monthly,
            expiry: CreditExpiryPolicy::EndOfPeriod,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: Vec::new(),
            all_services: true,
            service_refs: Vec::new(),
            reason: None,
            created_by: "admin-1".to_string(),
        },
    )
    .await
    .expect("create schedule");
    let stale = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find_one(doc! { "_id": &schedule.id })
        .await
        .expect("load schedule")
        .expect("schedule exists");
    let winner_updated_at = stale.updated_at + Duration::seconds(1);
    db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .update_one(
            doc! { "_id": &schedule.id },
            doc! { "$set": {
                "amount_credits": 99_i64,
                "amount_micros": 99_000_000_i64,
                "updated_at": bson::DateTime::from_chrono(winner_updated_at),
            } },
        )
        .await
        .expect("commit winning update");

    let error = update_schedule_with_current(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            is_active: Some(false),
            ..Default::default()
        },
        stale,
    )
    .await
    .expect_err("stale update should conflict");
    assert!(
        matches!(error, AppError::Conflict(ref message) if message.contains("retry")),
        "unexpected error: {error:?}"
    );

    let stored = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find_one(doc! { "_id": &schedule.id })
        .await
        .expect("reload schedule")
        .expect("schedule exists");
    assert_eq!(stored.amount_credits, 99);
    assert_eq!(stored.amount_micros, 99_000_000);
    assert!(stored.is_active);
    assert_eq!(stored.updated_at, winner_updated_at);
}
