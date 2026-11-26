use super::*;
use crate::models::user::UserType;
use crate::test_utils::{connect_transaction_test_database, test_user};

async fn setup() -> (mongodb::Database, CreditSchedule, DateTime<Utc>) {
    let db = connect_transaction_test_database("seed_missing_schedule").await;
    let now = DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
    let mut user = test_user("z-paid", UserType::Person);
    user.created_at = now - Duration::days(10);
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    let schedule = create_schedule(
        &db,
        CreateScheduleInput {
            amount_credits: 100,
            recurrence: ScheduleRecurrence::Monthly,
            timing: ScheduleTiming::Calendar,
            expiry: CreditExpiryPolicy::EndOfPeriod,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: vec![],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            reason: Some("Monthly benefit".into()),
            created_by: "admin".into(),
            amount: None,
            service_limits: vec![],
            request_id: None,
        },
    )
    .await
    .unwrap();
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    (db, schedule, now)
}

async fn missed_user(db: &mongodb::Database, id: &str, now: DateTime<Utc>) {
    let mut user = test_user(id, UserType::Person);
    user.created_at = now - Duration::days(30);
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
}

#[tokio::test]
async fn completed_calendar_period_seeds_missed_old_users_once_without_refilling_spent_grants() {
    let (db, schedule, now) = setup().await;
    missed_user(&db, "a-missed", now).await;
    let period = schedule_period(schedule.recurrence, now);
    let paid_id = grant_id(&schedule.id, period.start, "z-paid");
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": &paid_id },
            doc! { "$set": { "status": "consumed", "remaining": Credits::ZERO } },
        )
        .await
        .unwrap();
    // Ordinary calendar scans must retain their existing completed-period rule.
    assert_eq!(
        disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
            .await
            .unwrap()
            .grants_created,
        0
    );
    // Mid-period edits must not rewrite the original period's financial policy.
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            amount_credits: Some(200),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let seeded = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now)
        .await
        .unwrap();
    assert_eq!((seeded.created_count, seeded.existing_count), (1, 1));
    assert_eq!(seeded.pending_activation_count, 0);
    let retry = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now)
        .await
        .unwrap();
    assert_eq!((retry.created_count, retry.existing_count), (0, 2));
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id(&schedule.id, period.start, "a-missed") })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.amount, Credits::from_whole(100));
    assert_eq!(grant.expires_at, Some(period.end));
    assert!(grant.issued_ledgered_at.is_some());
    let spent = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": paid_id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(spent.remaining, Credits::ZERO);
    assert_eq!(spent.status, CreditGrantStatus::Consumed);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .unwrap(),
        2
    );
    let progress = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": period_id(&schedule.id, period.start) })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.status, SchedulePeriodStatus::Complete);
    assert_eq!(progress.disbursed_count, 2);
}

#[tokio::test]
async fn concurrent_seeding_and_lost_progress_never_duplicate_current_grants_or_journals() {
    let (db, schedule, now) = setup().await;
    missed_user(&db, "a-missed", now).await;
    let (one, two) = tokio::join!(
        seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now),
        seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now),
    );
    assert_eq!(one.unwrap().created_count + two.unwrap().created_count, 1);
    let period = schedule_period(schedule.recurrence, now);
    let id = grant_id(&schedule.id, period.start, "a-missed");
    assert_eq!(
        db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .count_documents(doc! { "dedupe_key": format!("grant-issued:{id}") })
            .await
            .unwrap(),
        1
    );
    // Losing calendar progress repeats scans, never financial allocations.
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .delete_one(doc! { "_id": period_id(&schedule.id, period.start) })
        .await
        .unwrap();
    let recovered = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now)
        .await
        .unwrap();
    assert_eq!((recovered.created_count, recovered.existing_count), (0, 2));
}

#[tokio::test]
async fn bounded_seed_pages_skip_inactive_users_and_reject_elapsed_periods_and_paused_schedules() {
    let (db, schedule, now) = setup().await;
    for index in 0..SEED_PAGE_SIZE + 1 {
        missed_user(&db, &format!("a-missed-{index:03}"), now).await;
    }
    let mut inactive = test_user("a-inactive", UserType::Person);
    inactive.is_active = false;
    db.collection::<User>(USERS)
        .insert_one(inactive)
        .await
        .unwrap();
    let first = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now)
        .await
        .unwrap();
    assert_eq!(first.created_count, SEED_PAGE_SIZE);
    assert!(first.next_user_id.is_some());
    let second = seed_missing(
        &db,
        &schedule.id,
        SeedScheduleInput {
            period_start: Some(first.period_start),
            after_user_id: first.next_user_id.clone(),
            ..Default::default()
        },
        now,
    )
    .await
    .unwrap();
    assert_eq!((second.created_count, second.existing_count), (1, 1));
    assert!(second.next_user_id.is_none());
    assert!(matches!(
        seed_missing(
            &db,
            &schedule.id,
            SeedScheduleInput {
                period_start: Some(first.period_start),
                after_user_id: first.next_user_id,
                ..Default::default()
            },
            schedule_period(schedule.recurrence, now).end
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            is_active: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now).await,
        Err(AppError::ValidationError(_))
    ));
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "recipient_user_id": "a-inactive" })
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        seed_missing(
            &db,
            &schedule.id,
            SeedScheduleInput {
                expected_updated_at: Some(now - Duration::days(1)),
                ..Default::default()
            },
            now
        )
        .await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn group_seed_uses_live_membership_including_existing_people_behind_the_calendar_cursor() {
    use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
    let (db, schedule, now) = setup().await;
    db.collection::<Group>(GROUPS)
        .insert_one(Group {
            id: "group".into(),
            name: "Group".into(),
            slug: "group".into(),
            description: None,
            role_ids: vec![],
            parent_group_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            target_kind: Some(BillingTargetKind::Groups),
            target_group_ids: Some(vec!["group".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Open the next calendar period with a group policy and no members.
    let later = schedule_period(schedule.recurrence, now).end;
    disburse_due(&db, later, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    missed_user(&db, "a-member", now).await;
    missed_user(&db, "a-outsider", now).await;
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "a-member" },
            doc! { "$set": { "group_ids": ["group"] } },
        )
        .await
        .unwrap();
    let seeded = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), later)
        .await
        .unwrap();
    assert_eq!((seeded.created_count, seeded.existing_count), (1, 0));
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "recipient_user_id": "a-outsider" })
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn org_seed_excludes_deactivated_organizations_and_preserves_the_seeding_actor() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    use crate::test_utils::test_membership;
    let (db, schedule, now) = setup().await;
    db.collection::<User>(USERS)
        .insert_one(test_user("org", UserType::Org))
        .await
        .unwrap();
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            target_kind: Some(BillingTargetKind::OrgMembers),
            target_org_ids: Some(vec!["org".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let later = schedule_period(schedule.recurrence, now).end;
    disburse_due(&db, later, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    missed_user(&db, "missed-member", now).await;
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(
            "org",
            "missed-member",
            OrgRole::Viewer,
            None,
        ))
        .await
        .unwrap();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "org" },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert_eq!(
        seed_missing(&db, &schedule.id, SeedScheduleInput::default(), later)
            .await
            .unwrap()
            .created_count,
        0
    );
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "org" },
            doc! { "$set": { "is_active": true } },
        )
        .await
        .unwrap();
    let actor = crate::services::audit_service::AuditActor {
        user_id: "seeding-admin".into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    assert_eq!(
        seed_missing(
            &db,
            &schedule.id,
            SeedScheduleInput {
                actor: Some(actor),
                ..Default::default()
            },
            later
        )
        .await
        .unwrap()
        .created_count,
        1
    );
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id(&schedule.id, later, "missed-member") })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.granted_by, "seeding-admin");
    let audit = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! { "event_type": "billing.credit_schedule.seed_requested" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.get_str("user_id").unwrap(), "seeding-admin");
}

#[tokio::test]
async fn selected_seed_obeys_frozen_targets_and_rejects_invalid_cursors_and_expired_benefits() {
    let (db, schedule, now) = setup().await;
    missed_user(&db, "selected", now).await;
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            target_kind: Some(BillingTargetKind::SelectedUsers),
            target_user_ids: Some(vec!["selected".into()]),
            expiry: Some(CreditExpiryPolicy::AfterDays { days: 1 }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let updated = db
        .collection::<CreditSchedule>(CREDIT_SCHEDULES)
        .find_one(doc! { "_id": &schedule.id })
        .await
        .unwrap()
        .unwrap();
    let window = schedule_period(schedule.recurrence, now);
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .delete_one(doc! { "_id": period_id(&schedule.id, window.start) })
        .await
        .unwrap();
    let PeriodClaim::Acquired(period) = claim_period(&db, &updated, window, window.start, true)
        .await
        .unwrap()
    else {
        panic!("fresh period");
    };
    // Hold the ordinary worker's lease: seeding must still use its frozen policy.
    missed_user(&db, "outsider", now).await;
    let seeded = seed_missing(
        &db,
        &schedule.id,
        SeedScheduleInput::default(),
        window.start,
    )
    .await
    .unwrap();
    assert_eq!(seeded.created_count, 1);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "recipient_user_id": "outsider" })
            .await
            .unwrap(),
        0
    );
    for input in [
        SeedScheduleInput {
            after_user_id: Some("selected".into()),
            ..Default::default()
        },
        SeedScheduleInput {
            period_start: Some(window.start),
            after_user_id: Some(" ".into()),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            seed_missing(&db, &schedule.id, input, window.start).await,
            Err(AppError::ValidationError(_))
        ));
    }
    assert!(matches!(
        seed_missing(
            &db,
            &schedule.id,
            SeedScheduleInput::default(),
            period.expires_at.unwrap()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn failed_seed_journal_reports_pending_and_reconciliation_activates_once() {
    let (db, schedule, now) = setup().await;
    missed_user(&db, "a-pending", now).await;
    let ledger = crate::models::billing_ledger::COLLECTION_NAME;
    db.run_command(doc! { "collMod": ledger,
    "validator": { "reject_test_writes": true } })
        .await
        .unwrap();
    let seeded = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now)
        .await
        .unwrap();
    assert_eq!(
        (
            seeded.created_count,
            seeded.existing_count,
            seeded.pending_activation_count
        ),
        (1, 1, 1)
    );
    let id = grant_id(
        &schedule.id,
        schedule_period(schedule.recurrence, now).start,
        "a-pending",
    );
    let pending = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": &id })
        .await
        .unwrap()
        .unwrap();
    assert!(pending.issued_ledgered_at.is_none());
    db.run_command(doc! { "collMod": ledger, "validator": {} })
        .await
        .unwrap();
    assert_eq!(
        super::super::super::grants::recover_unledgered_events(&db, now)
            .await
            .unwrap(),
        1
    );
    let retry = seed_missing(&db, &schedule.id, SeedScheduleInput::default(), now)
        .await
        .unwrap();
    assert_eq!(
        (
            retry.created_count,
            retry.existing_count,
            retry.pending_activation_count
        ),
        (0, 2, 0)
    );
    assert_eq!(
        db.collection::<Document>(ledger)
            .count_documents(doc! { "dedupe_key": format!("grant-issued:{id}") })
            .await
            .unwrap(),
        1
    );
}
