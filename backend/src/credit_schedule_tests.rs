use std::collections::BTreeSet;

use chrono::{Duration, TimeZone, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

use crate::models::billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as BILLING_LEDGER};
use crate::models::billing_target::BillingTargetKind;
use crate::models::credit_grant::{COLLECTION_NAME as CREDIT_GRANTS, CreditGrant};
use crate::models::credit_schedule::{
    COLLECTION_NAME as CREDIT_SCHEDULES, CreditExpiryPolicy, CreditSchedule, SchedulePeriod,
    ScheduleRecurrence,
};
use crate::models::credit_schedule_period::{
    COLLECTION_NAME as CREDIT_SCHEDULE_PERIODS, CreditSchedulePeriod, SchedulePeriodStatus,
};
use crate::models::usage_allowance::AllowanceRecurrence;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::billing::{allowances, grants, schedules};
use crate::test_utils::{connect_test_database, test_user};

fn schedule_input() -> schedules::CreateScheduleInput {
    schedules::CreateScheduleInput {
        amount_credits: 25,
        recurrence: ScheduleRecurrence::Monthly,
        expiry: CreditExpiryPolicy::EndOfPeriod,
        target_kind: BillingTargetKind::AllUsers,
        target_user_ids: Vec::new(),
        all_services: true,
        service_refs: Vec::new(),
        reason: Some("Monthly platform credits".to_string()),
        created_by: "admin-1".to_string(),
    }
}

async fn seed_owners(db: &mongodb::Database, count: usize) -> Vec<String> {
    let ids: Vec<String> = (0..count)
        .map(|index| format!("owner-{index:04}"))
        .collect();
    let owners: Vec<User> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            test_user(
                id,
                if index % 7 == 0 {
                    UserType::Org
                } else {
                    UserType::Person
                },
            )
        })
        .collect();
    db.collection::<User>(USERS)
        .insert_many(owners)
        .await
        .expect("insert billing owners");
    ids
}

async fn create_schedule(db: &mongodb::Database) -> CreditSchedule {
    schedules::create_schedule(db, schedule_input())
        .await
        .expect("create credit schedule")
}

fn init_ledger_key() {
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
}

#[tokio::test]
async fn disburse_due_is_idempotent_across_replicas_and_crashes() {
    let Some(db) = connect_test_database("credit_schedule_idempotent").await else {
        return;
    };
    init_ledger_key();
    let owner_ids = seed_owners(&db, 225).await;
    let schedule = create_schedule(&db).await;
    let now = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();

    let first = schedules::disburse_due(&db, now, 100)
        .await
        .expect("first partial disbursement");
    assert_eq!(first.grants_created, 100);
    assert!(first.budget_exhausted);

    let period = schedules::schedule_period(ScheduleRecurrence::Monthly, now);
    let period_id = schedules::period_id(&schedule.id, period.start);
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .update_one(
            doc! { "_id": &period_id },
            doc! { "$set": {
                "cursor_user_id": bson::Bson::Null,
                "disbursed_count": 0_i64,
                "lease_expires_at": bson::Bson::Null,
            } },
        )
        .await
        .expect("simulate crash before cursor persistence");

    let resumed = schedules::disburse_due(&db, now, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("resume disbursement after lost lease");
    assert_eq!(resumed.grants_created, 125);
    assert_eq!(resumed.already_disbursed, 100);

    let repeated = schedules::disburse_due(&db, now, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("second replica repeats completed period");
    assert_eq!(repeated.grants_created, 0);

    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "schedule_origin.schedule_id": &schedule.id })
        .await
        .expect("find scheduled grants")
        .try_collect()
        .await
        .expect("collect scheduled grants");
    let recipients: BTreeSet<&str> = grants
        .iter()
        .map(|grant| grant.recipient_user_id.as_str())
        .collect();
    assert_eq!(grants.len(), owner_ids.len());
    assert_eq!(recipients.len(), owner_ids.len());
    assert!(
        grants
            .iter()
            .all(|grant| grant.issued_ledgered_at.is_some())
    );
    assert_eq!(
        db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .count_documents(doc! {
                "event_type": "grant_issued",
                "reference_id": { "$in": grants.iter().map(|grant| &grant.id).collect::<Vec<_>>() },
            })
            .await
            .expect("count scheduled grant ledger entries"),
        owner_ids.len() as u64
    );
    let completed = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": period_id })
        .await
        .expect("find period")
        .expect("period exists");
    assert_eq!(completed.status, SchedulePeriodStatus::Complete);
    assert_eq!(completed.disbursed_count, owner_ids.len() as u64);
}

#[tokio::test]
async fn interrupted_walk_resumes_from_cursor_without_double_minting() {
    let Some(db) = connect_test_database("credit_schedule_cursor_resume").await else {
        return;
    };
    init_ledger_key();
    let owner_ids = seed_owners(&db, 220).await;
    let schedule = create_schedule(&db).await;
    let now = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();

    let first = schedules::disburse_due(&db, now, schedules::DISBURSEMENT_CHUNK)
        .await
        .expect("mint first chunk");
    assert_eq!(first.grants_created, schedules::DISBURSEMENT_CHUNK);
    let period_id = schedules::period_id(
        &schedule.id,
        schedules::schedule_period(schedule.recurrence, now).start,
    );
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .update_one(
            doc! { "_id": &period_id },
            doc! { "$set": { "lease_expires_at": bson::Bson::Null } },
        )
        .await
        .expect("clear efficiency lease");

    let resumed = schedules::disburse_due(&db, now, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("resume from cursor");
    assert_eq!(
        resumed.grants_created,
        owner_ids.len() - schedules::DISBURSEMENT_CHUNK
    );
    assert_eq!(resumed.already_disbursed, 0);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .expect("count grants"),
        owner_ids.len() as u64
    );
}

#[test]
fn schedule_windows_share_allowance_utc_boundaries() {
    let december = Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 0).unwrap();
    let monthly = schedules::schedule_period(ScheduleRecurrence::Monthly, december);
    assert_eq!(
        monthly.end,
        Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
    );
    let allowance_monthly =
        allowances::allowance_window(AllowanceRecurrence::Monthly, december, december);
    assert_eq!(allowance_monthly.start, monthly.start);
    assert_eq!(allowance_monthly.end, Some(monthly.end));

    let wednesday = Utc.with_ymd_and_hms(2026, 8, 26, 9, 0, 0).unwrap();
    let weekly = schedules::schedule_period(ScheduleRecurrence::Weekly, wednesday);
    assert_eq!(
        weekly.start,
        Utc.with_ymd_and_hms(2026, 8, 24, 0, 0, 0).unwrap()
    );
    let allowance_weekly =
        allowances::allowance_window(AllowanceRecurrence::Weekly, wednesday, wednesday);
    assert_eq!(allowance_weekly.start, weekly.start);
    assert_eq!(allowance_weekly.end, Some(weekly.end));
}

#[test]
fn schedule_expiry_policies_resolve_from_the_frozen_period() {
    let created_at = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();
    let period = SchedulePeriod {
        start: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
    };
    assert_eq!(
        schedules::resolve_expiry(&CreditExpiryPolicy::EndOfPeriod, &period, created_at),
        Some(period.end)
    );
    assert_eq!(
        schedules::resolve_expiry(
            &CreditExpiryPolicy::AfterDays { days: 30 },
            &period,
            created_at,
        ),
        Some(created_at + Duration::days(30))
    );
    assert_eq!(
        schedules::resolve_expiry(&CreditExpiryPolicy::Never, &period, created_at),
        None
    );
}

#[tokio::test]
async fn scheduled_grants_use_existing_active_listing_and_expiry_sweep() {
    let Some(db) = connect_test_database("credit_schedule_existing_grant_lifecycle").await else {
        return;
    };
    init_ledger_key();
    let owner_id = seed_owners(&db, 1).await.remove(0);
    let schedule = create_schedule(&db).await;
    let now = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();
    schedules::disburse_due(&db, now, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("mint scheduled grant");

    let active = grants::list_active_for_user(&db, &owner_id, now)
        .await
        .expect("list active scheduled grant");
    assert_eq!(active.len(), 1);
    assert_eq!(
        active[0]
            .schedule_origin
            .as_ref()
            .map(|origin| origin.schedule_id.as_str()),
        Some(schedule.id.as_str())
    );

    let expiry = schedules::schedule_period(schedule.recurrence, now).end;
    assert_eq!(
        grants::expire_due_grants(&db, expiry)
            .await
            .expect("expire scheduled grant"),
        1
    );
    assert!(
        grants::list_active_for_user(&db, &owner_id, expiry)
            .await
            .expect("list after expiry")
            .is_empty()
    );
}

#[tokio::test]
async fn elapsed_incomplete_period_is_abandoned_without_dead_credit_backfill() {
    let Some(db) = connect_test_database("credit_schedule_abandon_elapsed").await else {
        return;
    };
    init_ledger_key();
    seed_owners(&db, 3).await;
    let schedule = create_schedule(&db).await;
    let now = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();
    let old_period = SchedulePeriod {
        start: Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
    };
    let old_id = schedules::period_id(&schedule.id, old_period.start);
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .insert_one(CreditSchedulePeriod {
            id: old_id.clone(),
            schedule_id: schedule.id.clone(),
            period_start: old_period.start,
            period_end: old_period.end,
            status: SchedulePeriodStatus::Disbursing,
            amount_micros: schedule.amount_micros,
            expires_at: Some(old_period.end),
            target_kind: schedule.target_kind,
            target_user_ids: Vec::new(),
            scope: schedule.scope.clone(),
            reason: schedule.reason.clone(),
            cursor_user_id: None,
            disbursed_count: 0,
            lease_expires_at: None,
            created_at: old_period.start,
            updated_at: old_period.start,
            completed_at: None,
        })
        .await
        .expect("insert incomplete elapsed period");

    let stats = schedules::disburse_due(&db, now, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("run current-window-only catch-up");
    assert_eq!(stats.periods_abandoned, 1);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "batch_id": &old_id })
            .await
            .expect("count dead-window grants"),
        0
    );
    let old = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": old_id })
        .await
        .expect("find old period")
        .expect("old period exists");
    assert_eq!(old.status, SchedulePeriodStatus::Complete);
    assert_eq!(
        db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
            .find_one(doc! { "_id": &schedule.id })
            .await
            .expect("find schedule")
            .expect("schedule exists")
            .skipped_periods,
        1
    );
}

#[tokio::test]
async fn recipient_budget_exhaustion_resumes_next_tick_without_loss() {
    let Some(db) = connect_test_database("credit_schedule_budget_resume").await else {
        return;
    };
    init_ledger_key();
    let owner_ids = seed_owners(&db, 205).await;
    let schedule = create_schedule(&db).await;
    let now = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();

    let first = schedules::disburse_due(&db, now, 75)
        .await
        .expect("first bounded tick");
    let second = schedules::disburse_due(&db, now, 75)
        .await
        .expect("second bounded tick");
    let third = schedules::disburse_due(&db, now, 75)
        .await
        .expect("final bounded tick");
    assert!(first.budget_exhausted);
    assert!(second.budget_exhausted);
    assert!(!third.budget_exhausted);
    assert_eq!(
        first.grants_created + second.grants_created + third.grants_created,
        owner_ids.len()
    );
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .expect("count all scheduled grants"),
        owner_ids.len() as u64
    );
}

#[tokio::test]
async fn frozen_period_policy_and_signup_snapshot_survive_mid_walk_edit() {
    let Some(db) = connect_test_database("credit_schedule_frozen_snapshot").await else {
        return;
    };
    init_ledger_key();
    let owner_ids = seed_owners(&db, 205).await;
    let schedule = create_schedule(&db).await;
    let now = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();

    schedules::disburse_due(&db, now, 75)
        .await
        .expect("open and partially disburse period");
    let window = schedules::schedule_period(schedule.recurrence, now);
    let period = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": schedules::period_id(&schedule.id, window.start) })
        .await
        .expect("find claimed period")
        .expect("claimed period exists");

    let mut late_owner = test_user("zz-late-owner", UserType::Person);
    late_owner.created_at = period.created_at + Duration::milliseconds(1);
    late_owner.updated_at = late_owner.created_at;
    db.collection::<User>(USERS)
        .insert_one(late_owner)
        .await
        .expect("insert owner after period snapshot");
    schedules::update_schedule(
        &db,
        &schedule.id,
        schedules::UpdateScheduleInput {
            amount_credits: Some(99),
            expiry: Some(CreditExpiryPolicy::Never),
            reason: Some(Some("Edited policy".to_string())),
            ..Default::default()
        },
    )
    .await
    .expect("edit schedule during period walk");

    schedules::disburse_due(&db, now, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("finish frozen period");
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "schedule_origin.schedule_id": &schedule.id })
        .await
        .expect("find scheduled grants")
        .try_collect()
        .await
        .expect("collect scheduled grants");

    assert_eq!(grants.len(), owner_ids.len());
    assert!(
        grants
            .iter()
            .all(|grant| grant.recipient_user_id != "zz-late-owner")
    );
    assert!(grants.iter().all(|grant| {
        grant.amount_credits == 25
            && grant.amount_micros == 25_000_000
            && grant.expires_at == Some(window.end)
            && grant.reason.as_deref() == Some("Monthly platform credits")
    }));
}

#[tokio::test]
async fn pause_finishes_open_period_but_does_not_open_the_next_period() {
    let Some(db) = connect_test_database("credit_schedule_pause_semantics").await else {
        return;
    };
    init_ledger_key();
    let owner_ids = seed_owners(&db, 205).await;
    let schedule = create_schedule(&db).await;
    let august = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();

    let first = schedules::disburse_due(&db, august, 75)
        .await
        .expect("open schedule period");
    assert_eq!(first.grants_created, 75);
    schedules::update_schedule(
        &db,
        &schedule.id,
        schedules::UpdateScheduleInput {
            is_active: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("pause schedule");

    let resumed = schedules::disburse_due(&db, august, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("finish paused in-flight period");
    assert_eq!(resumed.grants_created, owner_ids.len() - 75);
    let september = Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0).unwrap();
    let next = schedules::disburse_due(&db, september, schedules::MAX_RECIPIENTS_PER_TICK)
        .await
        .expect("skip paused next period");
    assert_eq!(next.grants_created, 0);
    assert_eq!(
        db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
            .count_documents(doc! { "schedule_id": &schedule.id })
            .await
            .expect("count schedule periods"),
        1
    );
}

#[tokio::test]
async fn schedule_origin_index_boots_with_legacy_grants() {
    let Some(db) = connect_test_database("credit_schedule_legacy_index").await else {
        return;
    };
    db.collection::<mongodb::bson::Document>(CREDIT_GRANTS)
        .insert_one(doc! {
            "_id": "legacy-grant",
            "batch_id": "legacy-batch",
            "recipient_user_id": "legacy-owner",
        })
        .await
        .expect("insert legacy grant without schedule origin");

    crate::db::ensure_indexes(&db)
        .await
        .expect("ensure indexes on legacy grant data");
    let indexes: Vec<mongodb::IndexModel> = db
        .collection::<mongodb::bson::Document>(CREDIT_GRANTS)
        .list_indexes()
        .await
        .expect("list credit grant indexes")
        .try_collect()
        .await
        .expect("collect credit grant indexes");
    let origin = indexes
        .iter()
        .find(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some("credit_grants_schedule_origin")
        })
        .expect("schedule origin index exists");
    assert!(
        origin
            .options
            .as_ref()
            .and_then(|options| options.partial_filter_expression.as_ref())
            .is_some()
    );
}
