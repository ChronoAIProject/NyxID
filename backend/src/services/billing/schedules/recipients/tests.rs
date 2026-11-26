use super::*;
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::user::UserType;
use crate::test_utils::{connect_transaction_test_database, test_user};
use chrono::TimeZone;

tokio::task_local! {
    static DRAIN_AFTER_INSERT: ();
}

pub(super) fn after_insert() {
    if DRAIN_AFTER_INSERT.try_with(|_| ()).is_ok() {
        crate::services::billing::reconcile::drain_for_test();
    }
}

#[test]
fn recipient_windows_preserve_join_time_and_original_month_day() {
    let anchor = Utc.with_ymd_and_hms(2028, 1, 31, 13, 42, 7).unwrap();
    let feb = recipient_period(
        ScheduleRecurrence::Monthly,
        anchor,
        Utc.with_ymd_and_hms(2028, 2, 29, 13, 42, 7).unwrap(),
    )
    .unwrap();
    assert_eq!(
        feb.start,
        Utc.with_ymd_and_hms(2028, 2, 29, 13, 42, 7).unwrap()
    );
    assert_eq!(
        feb.end,
        Utc.with_ymd_and_hms(2028, 3, 31, 13, 42, 7).unwrap()
    );
    let before = recipient_period(
        ScheduleRecurrence::Monthly,
        anchor,
        feb.start - Duration::milliseconds(1),
    )
    .unwrap();
    assert_eq!(before.start, anchor);
    assert_eq!(before.end, feb.start);
    let week = recipient_period(
        ScheduleRecurrence::Weekly,
        anchor,
        anchor + Duration::days(16),
    )
    .unwrap();
    assert_eq!(week.start, anchor + Duration::days(14));
    assert_eq!(week.end, anchor + Duration::days(21));
    let daily = recipient_period(
        ScheduleRecurrence::Daily,
        anchor,
        anchor + Duration::days(3) + Duration::hours(2),
    )
    .unwrap();
    assert_eq!(daily.start, anchor + Duration::days(3));
    assert_eq!(daily.end, anchor + Duration::days(4));
    let non_leap = recipient_period(
        ScheduleRecurrence::Monthly,
        anchor,
        Utc.with_ymd_and_hms(2029, 2, 28, 13, 42, 7).unwrap(),
    )
    .unwrap();
    assert_eq!(
        non_leap.end,
        Utc.with_ymd_and_hms(2029, 3, 31, 13, 42, 7).unwrap()
    );
}

async fn setup() -> (mongodb::Database, CreditSchedule, DateTime<Utc>) {
    let db = connect_transaction_test_database("recipient_credit_cycles").await;
    db.collection::<Document>(RECIPIENTS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "schedule_id": 1, "recipient_user_id": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let now = DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
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
    let schedule = create_schedule(
        &db,
        CreateScheduleInput {
            amount_credits: 100,
            amount: None,
            service_limits: vec![],
            request_id: None,
            recurrence: ScheduleRecurrence::Monthly,
            timing: ScheduleTiming::Recipient,
            expiry: CreditExpiryPolicy::EndOfPeriod,
            target_kind: BillingTargetKind::Groups,
            target_group_ids: vec!["group".into()],
            target_org_ids: vec![],
            target_user_ids: vec![],
            all_services: true,
            service_refs: vec![],
            reason: None,
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();
    let start = DateTime::from_timestamp_millis(schedule.created_at.timestamp_millis()).unwrap()
        + Duration::seconds(1);
    (db, schedule, start)
}

async fn join(db: &mongodb::Database, id: &str, at: DateTime<Utc>) {
    let mut user = test_user(id, UserType::Person);
    user.group_ids = vec!["group".into()];
    user.created_at = at;
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    enroll_group_member(db, "group", id, at).await.unwrap();
}

async fn tick(db: &mongodb::Database, now: DateTime<Utc>) -> DisbursementStats {
    disburse_due(db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap()
}

#[tokio::test]
async fn new_group_members_get_full_grants_and_independent_monthly_renewals() {
    let (db, schedule, start) = setup().await;
    join(&db, "first", start).await;
    assert_eq!(tick(&db, start).await.grants_created, 1);
    let later = start + Duration::days(10);
    join(&db, "new-signup", later).await;
    assert_eq!(tick(&db, later).await.grants_created, 1);
    let renewal = start.checked_add_months(Months::new(1)).unwrap();
    let (one, two) = tokio::join!(tick(&db, renewal), tick(&db, renewal));
    assert_eq!(one.grants_created + two.grants_created, 1);
    assert_eq!(tick(&db, renewal).await.grants_created, 0);
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "schedule_origin.schedule_id": &schedule.id })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(grants.len(), 3);
    assert!(
        grants
            .iter()
            .all(|g| g.amount == Credits::from_whole(100) && g.issued_ledgered_at.is_some())
    );
    let first = grants
        .iter()
        .find(|g| {
            g.recipient_user_id == "first"
                && g.schedule_origin.as_ref().unwrap().period_start == start
        })
        .unwrap();
    assert_eq!(first.expires_at, Some(renewal));
    let second = grants
        .iter()
        .find(|g| g.recipient_user_id == "new-signup")
        .unwrap();
    assert_eq!(second.schedule_origin.as_ref().unwrap().period_start, later);
    assert_eq!(second.expires_at, later.checked_add_months(Months::new(1)));
}

#[tokio::test]
async fn drain_finishes_current_issuance_and_leaves_next_recipient_due() {
    let (db, schedule, start) = setup().await;
    join(&db, "first", start).await;
    join(&db, "second", start + Duration::milliseconds(1)).await;
    let now = start + Duration::seconds(1);
    let stats = crate::services::billing::reconcile::with_drain_for_test(
        DRAIN_AFTER_INSERT.scope((), tick(&db, now)),
    )
    .await;
    assert_eq!(stats.recipients_examined, 1);
    assert_eq!(stats.grants_created, 1);
    assert_eq!(stats.grants_ledgered, 1);
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "schedule_origin.schedule_id": &schedule.id })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].recipient_user_id, "first");
    assert!(grants[0].issued_ledgered_at.is_some());
    for (id, expected) in [
        ("first", start.checked_add_months(Months::new(1)).unwrap()),
        ("second", start + Duration::milliseconds(1)),
    ] {
        let recipient = db
            .collection::<CreditScheduleRecipient>(RECIPIENTS)
            .find_one(recipient_filter(&schedule.id, id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recipient.next_due_at, expected);
    }
    assert_eq!(tick(&db, now).await.grants_created, 1);
    assert_eq!(tick(&db, now).await.grants_created, 0);
    assert_eq!(
        db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .count_documents(doc! { "dedupe_key": { "$in": [
                format!("grant-issued:{}", grant_id(&schedule.id, start, "first")),
                format!("grant-issued:{}", grant_id(&schedule.id, start + Duration::milliseconds(1), "second")),
            ] } })
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn drained_worker_starts_no_discovery_or_issuance() {
    let (db, schedule, start) = setup().await;
    let mut user = test_user("missed-person", UserType::Person);
    user.group_ids = vec!["group".into()];
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    let mut budget = MAX_RECIPIENTS_PER_TICK;
    let mut stats = DisbursementStats::default();
    crate::services::billing::reconcile::with_drain_for_test(async {
        crate::services::billing::reconcile::drain_for_test();
        disburse(&db, &schedule, start, &mut budget, &mut stats)
            .await
            .unwrap();
    })
    .await;
    assert_eq!(budget, MAX_RECIPIENTS_PER_TICK);
    assert_eq!(stats.recipients_examined, 0);
    assert_eq!(
        db.collection::<Document>(RECIPIENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(SCANS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(CREDIT_GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn rejoining_recovers_eligibility_without_resetting_or_double_granting() {
    let (db, schedule, start) = setup().await;
    join(&db, "person", start).await;
    tick(&db, start).await;
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "person" },
            doc! { "$set": { "group_ids": [] } },
        )
        .await
        .unwrap();
    let renewal = start.checked_add_months(Months::new(1)).unwrap();
    assert_eq!(tick(&db, renewal).await.grants_created, 0);
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "person" },
            doc! { "$set": { "group_ids": ["group"] } },
        )
        .await
        .unwrap();
    let rejoin = renewal + Duration::days(4);
    enroll_group_member(&db, "group", "person", rejoin)
        .await
        .unwrap();
    assert_eq!(tick(&db, rejoin).await.grants_created, 1);
    enroll_group_member(&db, "group", "person", rejoin)
        .await
        .unwrap();
    assert_eq!(tick(&db, rejoin).await.grants_created, 0);
    let recipient = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(recipient_filter(&schedule.id, "person"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recipient.anchor_at, start);
    assert_eq!(
        recipient.next_due_at,
        start.checked_add_months(Months::new(2)).unwrap()
    );
}

#[tokio::test]
async fn discovery_repairs_missed_joins_and_pause_skips_elapsed_periods() {
    let (db, schedule, start) = setup().await;
    let mut user = test_user("person", UserType::Person);
    user.group_ids = vec!["group".into()];
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    assert_eq!(tick(&db, start).await.grants_created, 1);
    let recipient = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(recipient_filter(&schedule.id, "person"))
        .await
        .unwrap()
        .unwrap();
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
    let later = recipient
        .anchor_at
        .checked_add_months(Months::new(3))
        .unwrap();
    assert_eq!(tick(&db, later).await.grants_created, 0);
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            is_active: Some(true),
            amount_credits: Some(200),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(tick(&db, later).await.grants_created, 1);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .unwrap(),
        2
    );
    let latest = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id(&schedule.id, later, "person") })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.amount, Credits::from_whole(200));
}

#[tokio::test]
async fn group_service_records_join_anchor_and_retry_keeps_persisted_grant_policy() {
    let (db, schedule, _) = setup().await;
    db.collection::<User>(USERS)
        .insert_one(test_user("person", UserType::Person))
        .await
        .unwrap();
    let before = Utc::now() - Duration::milliseconds(1);
    crate::services::group_service::add_member(&db, "group", "person")
        .await
        .unwrap();
    let recipient = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(recipient_filter(&schedule.id, "person"))
        .await
        .unwrap()
        .unwrap();
    assert!(recipient.anchor_at >= before && recipient.anchor_at <= Utc::now());
    tick(&db, Utc::now()).await;
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
    // Simulate a crash after grant insertion but before scheduling its renewal.
    db.collection::<CreditScheduleRecipient>(RECIPIENTS)
        .update_one(
            doc! { "_id": &recipient.id },
            doc! { "$set": { "next_due_at": bson::DateTime::from_chrono(recipient.anchor_at) } },
        )
        .await
        .unwrap();
    assert_eq!(tick(&db, Utc::now()).await.grants_created, 0);
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "schedule_origin.schedule_id": &schedule.id })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].amount, Credits::from_whole(100));
    assert_eq!(
        db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .count_documents(doc! { "dedupe_key": format!("grant-issued:{}", grants[0].id) })
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn registered_members_receive_spendable_credits_even_after_a_missed_enrollment() {
    use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS};
    use crate::models::service_billing::{BillingMetric, PlatformUsage};
    use crate::models::usage_meter::{BillingLayer, CredentialClass};
    use crate::services::billing::{
        BillingIngress, BillingRouteContext, NodeIntent, funding, meter,
        reservation::LayerReservation,
    };

    let (db, schedule, _) = setup().await;
    for missed_hook in [false, true] {
        let registration = crate::services::auth_service::register_user(
            &db,
            &format!("signup-{missed_hook}@example.com"),
            "test-registration-password",
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert!(registration.actually_created);
        let owner = registration.user_id;
        let grants = db.collection::<CreditGrant>(CREDIT_GRANTS);
        let grant_filter =
            doc! { "recipient_user_id": &owner, "schedule_origin.schedule_id": &schedule.id };
        assert_eq!(
            grants.count_documents(grant_filter.clone()).await.unwrap(),
            0
        );
        if missed_hook {
            // Commit the membership write by itself to model an interrupted hook.
            let joined = db
                .collection::<User>(USERS)
                .update_one(
                    doc! { "_id": &owner },
                    doc! { "$addToSet": { "group_ids": "group" } },
                )
                .write_concern(mongodb::options::WriteConcern::majority())
                .await
                .unwrap();
            assert_eq!(joined.modified_count, 1);
            assert!(
                db.collection::<CreditScheduleRecipient>(RECIPIENTS)
                    .find_one(recipient_filter(&schedule.id, &owner))
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                grants.count_documents(grant_filter.clone()).await.unwrap(),
                0
            );
            let recovered = tick(&db, Utc::now()).await;
            assert_eq!(recovered.grants_created, 1);
            assert_eq!(recovered.grants_ledgered, 1);
        } else {
            crate::services::group_service::add_member(&db, "group", &owner)
                .await
                .unwrap();
        }
        let grant = grants
            .find_one(grant_filter.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grant.amount, Credits::from_whole(100));
        assert!(grant.issued_ledgered_at.is_some());

        // A zero-balance wallet proves the scheduled grant actually funds the
        // request, rather than merely appearing as an active grant in the UI.
        let now = bson::DateTime::now();
        let wallet: BillingWallet = bson::from_document(doc! {
            "_id": Uuid::new_v4().to_string(), "owner_id": &owner,
            "lago_customer_id": "test-customer", "plan_kind": "prepaid",
            "balance_credits": Credits::ZERO, "reserved_credits": Credits::ZERO,
            "collection_state": "good", "balance_synced_at": now,
            "created_at": now, "updated_at": now,
        })
        .unwrap();
        db.collection::<BillingWallet>(WALLETS)
            .insert_one(&wallet)
            .await
            .unwrap();
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.clone(),
            owner.clone(),
            None,
            None,
            Some("test-service".into()),
            Some("test-service".into()),
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            None,
            false,
        )
        .with_platform_metering(true);
        let reservation = funding::reserve_and_open_ordinary(
            &db,
            &ctx,
            &wallet,
            vec![LayerReservation {
                layer: BillingLayer::Platform,
                metric: BillingMetric::Requests,
                lago_metric_code: ctx.platform_lago_metric_code.clone(),
                estimated_quantity: 1,
                credits_per_unit_micros: 25_000_000,
                credits_per_unit_pico: None,
                reserved_credits: Credits::from_whole(25),
                allowance_reservations: vec![],
                grant_reservations: vec![],
                tax_cache_ttl_secs: 900,
                applied_tax: None,
            }],
        )
        .await
        .unwrap();
        assert_eq!(reservation.total_reserved_credits, Credits::ZERO);
        assert_eq!(reservation.layers[0].grant_reservations.len(), 1);
        assert_eq!(
            reservation.layers[0].grant_reservations[0].grant_id,
            grant.id
        );
        assert_eq!(
            reservation.layers[0].grant_reservations[0].amount,
            Credits::from_whole(25)
        );
        let metered = meter::MeteredProxyContext::from_route(&ctx).with_funding(true);
        meter::mark_forwarded(&db, &metered).await.unwrap();
        meter::settle(&db, &metered, PlatformUsage::single_request(0), None, None)
            .await
            .unwrap();
        let spent = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": &grant.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(spent.remaining, Credits::from_whole(75));
        assert_eq!(spent.reserved, Credits::ZERO);
        assert_eq!(tick(&db, Utc::now()).await.grants_created, 0);
        assert_eq!(grants.count_documents(grant_filter).await.unwrap(), 1);
        assert_eq!(
            db.collection::<CreditGrant>(CREDIT_GRANTS)
                .find_one(doc! { "_id": &grant.id })
                .await
                .unwrap()
                .unwrap()
                .remaining,
            Credits::from_whole(75),
        );
    }
}

#[tokio::test]
async fn recipient_discovery_respects_each_target_kind_and_personal_membership() {
    use crate::models::org_membership::{OrgMembership, OrgRole};
    use crate::test_utils::test_membership;
    for kind in [
        BillingTargetKind::AllUsers,
        BillingTargetKind::SelectedUsers,
        BillingTargetKind::OrgMembers,
        BillingTargetKind::Groups,
    ] {
        let (db, schedule, start) = setup().await;
        let mut person = test_user("person", UserType::Person);
        person.group_ids = vec!["group".into()];
        db.collection::<User>(USERS)
            .insert_many([
                person,
                test_user("org", UserType::Org),
                test_user("other", UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership("org", "person", OrgRole::Viewer, None))
            .await
            .unwrap();
        update_schedule(
            &db,
            &schedule.id,
            UpdateScheduleInput {
                target_kind: Some(kind),
                target_user_ids: Some(if kind == BillingTargetKind::SelectedUsers {
                    vec!["person".into(), "org".into()]
                } else {
                    vec![]
                }),
                target_org_ids: Some(if kind == BillingTargetKind::OrgMembers {
                    vec!["org".into()]
                } else {
                    vec![]
                }),
                target_group_ids: Some(if kind == BillingTargetKind::Groups {
                    vec!["group".into()]
                } else {
                    vec![]
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let expected: &[&str] = match kind {
            BillingTargetKind::AllUsers => &["org", "other", "person"],
            BillingTargetKind::SelectedUsers => &["org", "person"],
            BillingTargetKind::OrgMembers | BillingTargetKind::Groups => &["person"],
        };
        assert_eq!(tick(&db, start).await.grants_created, expected.len());
        let recipients = db.collection::<CreditScheduleRecipient>(RECIPIENTS);
        let discovered: Vec<_> = recipients
            .find(doc! { "schedule_id": &schedule.id })
            .sort(doc! { "recipient_user_id": 1 })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(
            discovered
                .iter()
                .map(|recipient| recipient.recipient_user_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        // Discovery can observe eligibility after the supplied first tick time.
        let renewal = discovered
            .iter()
            .map(|recipient| recipient.next_due_at)
            .max()
            .unwrap();
        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": "person" },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();
        assert_eq!(tick(&db, renewal).await.grants_created, expected.len() - 1);
        let deactivated = recipients
            .find_one(recipient_filter(&schedule.id, "person"))
            .await
            .unwrap()
            .unwrap();
        assert!(!deactivated.is_eligible);
    }
}

#[tokio::test]
async fn delayed_worker_does_not_mint_an_already_expired_short_grant() {
    let (db, schedule, start) = setup().await;
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            expiry: Some(CreditExpiryPolicy::AfterDays { days: 1 }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    join(&db, "person", start).await;
    assert_eq!(tick(&db, start + Duration::days(2)).await.grants_created, 0);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        tick(&db, start.checked_add_months(Months::new(1)).unwrap())
            .await
            .grants_created,
        1
    );
}

#[tokio::test]
async fn group_join_is_discovered_even_behind_an_unfinished_cursor() {
    let (db, schedule, start) = setup().await;
    join(&db, "z-existing", start).await;
    let first = disburse_due(&db, start, 1).await.unwrap();
    assert_eq!(first.grants_created, 1);
    assert!(first.budget_exhausted);
    // The existing discovery cursor is z-existing; the new id sorts before it.
    let later = start + Duration::days(2);
    join(&db, "a-new-member", later).await;
    assert_eq!(disburse_due(&db, later, 1).await.unwrap().grants_created, 1);
    assert!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! {
                "_id": grant_id(&schedule.id, later, "a-new-member")
            })
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_rejoin_fences_a_stale_ineligibility_write_even_with_the_same_timestamp() {
    let (db, schedule, start) = setup().await;
    join(&db, "person", start).await;
    let stale = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(recipient_filter(&schedule.id, "person"))
        .await
        .unwrap()
        .unwrap();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "person" },
            doc! { "$set": { "group_ids": [] } },
        )
        .await
        .unwrap();
    assert!(
        eligible_ids(&db, &schedule, &["person".into()])
            .await
            .unwrap()
            .is_empty()
    );
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "person" },
            doc! { "$set": { "group_ids": ["group"] } },
        )
        .await
        .unwrap();
    // The hook runs while the row still says eligible, before the stale write.
    enroll_group_member(&db, "group", "person", start)
        .await
        .unwrap();
    mark_ineligible(&db, &stale, start).await.unwrap();
    let current = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(recipient_filter(&schedule.id, "person"))
        .await
        .unwrap()
        .unwrap();
    assert!(current.is_eligible);
    assert!(current.eligibility_version > stale.eligibility_version);
    assert_eq!(current.anchor_at, stale.anchor_at);
    assert_eq!(tick(&db, start).await.grants_created, 1);
    assert_eq!(tick(&db, start).await.grants_created, 0);
}

#[tokio::test]
async fn joins_while_paused_keep_the_join_anchor_without_issuing_until_resumed() {
    let (db, schedule, start) = setup().await;
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
    join(&db, "person", start).await;
    assert_eq!(tick(&db, start).await.grants_created, 0);
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            is_active: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(tick(&db, start + Duration::days(2)).await.grants_created, 1);
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id(&schedule.id, start, "person") })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.expires_at, start.checked_add_months(Months::new(1)));
}

#[tokio::test]
async fn daily_and_weekly_recipient_grants_renew_once_and_skip_elapsed_windows() {
    for (recurrence, days) in [
        (ScheduleRecurrence::Daily, 1),
        (ScheduleRecurrence::Weekly, 7),
    ] {
        let (db, schedule, start) = setup().await;
        db.collection::<CreditSchedule>(CREDIT_SCHEDULES)
            .update_one(
                doc! { "_id": &schedule.id },
                doc! { "$set": { "recurrence": encode(&recurrence, "recurrence").unwrap() } },
            )
            .await
            .unwrap();
        join(&db, "person", start).await;
        assert_eq!(tick(&db, start).await.grants_created, 1);
        assert_eq!(
            tick(&db, start + Duration::days(days)).await.grants_created,
            1
        );
        assert_eq!(
            tick(&db, start + Duration::days(days)).await.grants_created,
            0
        );
        assert_eq!(
            tick(&db, start + Duration::days(days * 4))
                .await
                .grants_created,
            1
        );
        let grants: Vec<CreditGrant> = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(grants.len(), 3);
        assert!(
            grants
                .iter()
                .all(|grant| grant.amount == Credits::from_whole(100)
                    && grant.issued_ledgered_at.is_some())
        );
        assert!(grants.iter().all(|grant| grant.expires_at
            == Some(grant.schedule_origin.as_ref().unwrap().period_start + Duration::days(days))));
    }
}

#[tokio::test]
async fn pending_recipient_issuance_recovers_without_a_period_row_or_refilling() {
    let (db, schedule, now) = setup().await;
    join(&db, "pending-person", now).await;
    let ledger = crate::models::billing_ledger::COLLECTION_NAME;
    // Simulate a durable grant followed by a rejected journal append.
    db.create_collection(ledger)
        .validator(doc! { "reject_test_writes": true })
        .await
        .unwrap();
    assert_eq!(tick(&db, now).await.grants_created, 1);
    let id = grant_id(&schedule.id, now, "pending-person");
    let pending = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": &id })
        .await
        .unwrap()
        .unwrap();
    assert!(pending.issued_ledgered_at.is_none());
    assert!(
        super::super::super::grants::list_active_for_user(&db, "pending-person", now)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.collection::<Document>(CREDIT_SCHEDULE_PERIODS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let recipient = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(recipient_filter(&schedule.id, "pending-person"))
        .await
        .unwrap()
        .unwrap();
    assert!(recipient.next_due_at > now);
    db.run_command(doc! { "collMod": ledger, "validator": {} })
        .await
        .unwrap();
    assert_eq!(
        super::super::super::grants::recover_unledgered_events(&db, now)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        super::super::super::grants::recover_unledgered_events(&db, now)
            .await
            .unwrap(),
        0
    );
    assert_eq!(tick(&db, now).await.grants_created, 0);
    let recovered = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": &id })
        .await
        .unwrap()
        .unwrap();
    assert!(recovered.issued_ledgered_at.is_some());
    assert_eq!(recovered.remaining, Credits::from_whole(100));
    assert_eq!(
        db.collection::<Document>(ledger)
            .count_documents(doc! { "dedupe_key": format!("grant-issued:{id}") })
            .await
            .unwrap(),
        1
    );
}
