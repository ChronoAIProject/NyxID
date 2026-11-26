use super::*;
use crate::models::credit_schedule_recipient::{
    COLLECTION_NAME as RECIPIENTS, CreditScheduleRecipient,
};
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, MemberScopeSource, OrgRole};
use crate::models::user::UserType;
use crate::services::social_auth_service::{SocialProfile, SocialProvider};
use crate::services::{
    admin_user_service, auth_service, group_service, org_invite_service, org_service,
    social_auth_service,
};
use crate::test_utils::{connect_transaction_test_database, test_user};

tokio::task_local! {
    static AFTER_PERIOD_READ_TIME: DateTime<Utc>;
    static AFTER_CALENDAR_INSERT: (std::sync::Arc<tokio::sync::Notify>, std::sync::Arc<tokio::sync::Notify>);
}

pub(super) fn calendar_check_time(now: DateTime<Utc>) -> DateTime<Utc> {
    AFTER_PERIOD_READ_TIME
        .try_with(|at| now.max(*at))
        .unwrap_or(now)
}

pub(super) async fn after_calendar_insert() {
    let sync = AFTER_CALENDAR_INSERT.try_with(|sync| (sync.0.clone(), sync.1.clone()));
    if let Ok((inserted, resume)) = sync {
        inserted.notify_one();
        resume.notified().await;
    }
}

#[tokio::test]
async fn calendar_hook_does_not_mint_a_window_that_elapsed_during_its_period_read() {
    for expiry in [CreditExpiryPolicy::Never, CreditExpiryPolicy::EndOfPeriod] {
        let (db, now) = setup("eligibility_period_boundary").await;
        let mut request = input(BillingTargetKind::AllUsers, ScheduleTiming::Calendar);
        request.recurrence = ScheduleRecurrence::Daily;
        request.expiry = expiry;
        let schedule = create_schedule(&db, request).await.unwrap();
        disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
            .await
            .unwrap();
        insert_owner(&db, "late", UserType::Person, now).await;
        let window = schedule_period(schedule.recurrence, now);
        AFTER_PERIOD_READ_TIME
            .scope(
                window.end,
                grant_on_eligibility(&db, "late", Eligibility::Account, now, now),
            )
            .await
            .unwrap();
        assert!(grants_for(&db, "late").await.is_empty());
        assert_eq!(
            db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1,
            "a join must not open the next period"
        );
        // A fresh operation in the next claimed period gets exactly its grant.
        let next = window.end + Duration::seconds(1);
        disburse_due(&db, next, MAX_RECIPIENTS_PER_TICK)
            .await
            .unwrap();
        let grants = grants_for(&db, "late").await;
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].id, grant_id(&schedule.id, window.end, "late"));
    }
}

#[tokio::test]
async fn calendar_hook_rechecks_expiry_after_loading_the_claimed_period() {
    let (db, now) = setup("eligibility_expiry_boundary").await;
    let schedule = create_schedule(
        &db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Calendar),
    )
    .await
    .unwrap();
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    insert_owner(&db, "late", UserType::Person, now).await;
    let window = schedule_period(schedule.recurrence, now);
    db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .update_one(
            doc! { "_id": period_id(&schedule.id, window.start) },
            doc! { "$set": { "expires_at": bson::DateTime::from_chrono(now) } },
        )
        .await
        .unwrap();
    grant_on_eligibility(
        &db,
        "late",
        Eligibility::Account,
        now,
        now - Duration::seconds(1),
    )
    .await
    .unwrap();
    assert!(grants_for(&db, "late").await.is_empty());
}

fn input(target_kind: BillingTargetKind, timing: ScheduleTiming) -> CreateScheduleInput {
    CreateScheduleInput {
        amount_credits: 20,
        recurrence: ScheduleRecurrence::Monthly,
        timing,
        expiry: CreditExpiryPolicy::EndOfPeriod,
        target_kind,
        target_user_ids: if target_kind == BillingTargetKind::SelectedUsers {
            vec!["existing".into()]
        } else {
            vec![]
        },
        target_org_ids: vec![],
        target_group_ids: vec![],
        all_services: true,
        service_refs: vec![],
        reason: Some("platform credits".into()),
        created_by: "admin".into(),
        amount: None,
        service_limits: vec![],
        request_id: None,
    }
}

async fn setup(name: &str) -> (mongodb::Database, DateTime<Utc>) {
    let db = connect_transaction_test_database(name).await;
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let now = DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
    let mut user = test_user("existing", UserType::Person);
    user.created_at = now - Duration::days(10);
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    (db, now)
}

async fn grants_for(db: &mongodb::Database, user_id: &str) -> Vec<CreditGrant> {
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "recipient_user_id": user_id })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
async fn every_signup_path_receives_the_current_period_after_the_walk_completed() {
    let (db, now) = setup("signup_calendar_grant").await;
    let schedule = create_schedule(
        &db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Calendar),
    )
    .await
    .unwrap();
    // The month's walk already finished, so its signup snapshot excludes
    // every account created below.
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let window = schedule_period(schedule.recurrence, Utc::now());

    let registered = auth_service::register_user(
        &db,
        "new-signup@example.com",
        "password123",
        None,
        None,
        true,
    )
    .await
    .unwrap()
    .user_id;
    let invite = crate::services::invite_code_service::create_invite_code(&db, "admin", 1, None)
        .await
        .unwrap();
    let reservation = crate::services::invite_code_service::reserve_invite_code(
        &db,
        &invite.code,
        "invited-signup@example.com",
    )
    .await
    .unwrap();
    let invited = auth_service::register_user(
        &db,
        "invited-signup@example.com",
        "password123",
        None,
        Some(&reservation),
        true,
    )
    .await
    .unwrap()
    .user_id;
    let admin_created = admin_user_service::create_user(
        &db,
        "admin-created@example.com",
        "password123",
        None,
        "user",
    )
    .await
    .unwrap()
    .id;
    let social = social_auth_service::find_or_create_user(
        &db,
        &SocialProfile {
            provider: SocialProvider::GitHub,
            provider_id: "gh-new-signup".into(),
            email: "social-signup@example.com".into(),
            display_name: None,
            avatar_url: None,
        },
        true,
    )
    .await
    .unwrap();
    assert!(social.was_newly_created);

    for user_id in [&registered, &invited, &admin_created, &social.user.id] {
        let grants = grants_for(&db, user_id).await;
        assert_eq!(grants.len(), 1, "{user_id} must receive one grant");
        let grant = &grants[0];
        assert_eq!(grant.id, grant_id(&schedule.id, window.start, user_id));
        assert_eq!(grant.amount, Credits::from_whole(20));
        assert_eq!(grant.remaining, Credits::from_whole(20));
        assert_eq!(grant.expires_at, Some(window.end));
        assert_eq!(grant.status, CreditGrantStatus::Active);
        // Journaled, so funding can spend it immediately.
        assert!(grant.issued_ledgered_at.is_some());
    }

    // Retried hooks and later walks converge on the same grant.
    grant_on_eligibility(&db, &registered, Eligibility::Account, now, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        disburse_due(&db, Utc::now(), MAX_RECIPIENTS_PER_TICK)
            .await
            .unwrap()
            .grants_created,
        0
    );
    assert_eq!(grants_for(&db, &registered).await.len(), 1);
    let period = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": period_id(&schedule.id, window.start) })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(period.status, SchedulePeriodStatus::Complete);
    assert_eq!(period.disbursed_count, 5);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .unwrap(),
        5
    );

    // The next period's ordinary walk includes the new account normally.
    let next = window.end + Duration::hours(1);
    disburse_due(&db, next, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let next_window = schedule_period(schedule.recurrence, next);
    assert!(
        grants_for(&db, &registered)
            .await
            .iter()
            .any(|grant| grant.id == grant_id(&schedule.id, next_window.start, &registered))
    );
}

#[tokio::test]
async fn an_unclaimed_period_leaves_the_new_account_to_the_ordinary_walk() {
    let (db, now) = setup("signup_unclaimed_period").await;
    let schedule = create_schedule(
        &db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Calendar),
    )
    .await
    .unwrap();
    let mut user = test_user("fresh", UserType::Person);
    user.created_at = now;
    db.collection::<User>(USERS).insert_one(user).await.unwrap();

    grant_on_eligibility(&db, "fresh", Eligibility::Account, now, now)
        .await
        .unwrap();
    assert!(grants_for(&db, "fresh").await.is_empty());
    assert_eq!(
        db.collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "account creation must never open a period"
    );
    disburse_due(&db, Utc::now(), MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let window = schedule_period(schedule.recurrence, Utc::now());
    assert_eq!(
        grants_for(&db, "fresh").await[0].id,
        grant_id(&schedule.id, window.start, "fresh")
    );
}

#[tokio::test]
async fn paused_and_member_targeted_schedules_grant_nothing_at_signup() {
    let (db, now) = setup("signup_ineligible_schedules").await;
    let paused = create_schedule(
        &db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Calendar),
    )
    .await
    .unwrap();
    create_schedule(
        &db,
        input(BillingTargetKind::SelectedUsers, ScheduleTiming::Calendar),
    )
    .await
    .unwrap();
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    update_schedule(
        &db,
        &paused.id,
        UpdateScheduleInput {
            is_active: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let registered =
        auth_service::register_user(&db, "late@example.com", "password123", None, None, true)
            .await
            .unwrap()
            .user_id;
    assert!(grants_for(&db, &registered).await.is_empty());
}

#[tokio::test]
async fn recipient_schedules_enroll_new_accounts_at_signup() {
    let (db, _) = setup("signup_recipient_enrollment").await;
    let schedule = create_schedule(
        &db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Recipient),
    )
    .await
    .unwrap();
    let registered =
        auth_service::register_user(&db, "cycle@example.com", "password123", None, None, true)
            .await
            .unwrap()
            .user_id;
    let created_at = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &registered })
        .await
        .unwrap()
        .unwrap()
        .created_at;
    let recipient = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(doc! { "schedule_id": &schedule.id, "recipient_user_id": &registered })
        .await
        .unwrap()
        .expect("signup enrolls the account without waiting for discovery");
    assert_eq!(
        recipient.anchor_at.timestamp_millis(),
        created_at.max(schedule.created_at).timestamp_millis()
    );
    let grants = grants_for(&db, &registered).await;
    assert_eq!(grants.len(), 1);
    assert!(grants[0].issued_ledgered_at.is_some());
}

fn member_input(
    target_kind: BillingTargetKind,
    timing: ScheduleTiming,
    ids: &[&str],
) -> CreateScheduleInput {
    let mut input = input(target_kind, timing);
    input.amount_credits = 1000;
    input.recurrence = ScheduleRecurrence::Weekly;
    let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    match target_kind {
        BillingTargetKind::OrgMembers => input.target_org_ids = ids,
        BillingTargetKind::Groups => input.target_group_ids = ids,
        _ => unreachable!("member schedules only"),
    }
    input
}

async fn insert_owner(db: &mongodb::Database, id: &str, kind: UserType, created_at: DateTime<Utc>) {
    let mut user = test_user(id, kind);
    user.created_at = created_at;
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
}

async fn join_org(db: &mongodb::Database, org_id: &str, user_id: &str) {
    org_service::create_membership(
        db,
        org_id,
        user_id,
        OrgRole::Member,
        MemberScopeSource::Inherit,
        None,
    )
    .await
    .unwrap();
}

fn assert_current_grant(
    grants: &[CreditGrant],
    schedule: &CreditSchedule,
    start: DateTime<Utc>,
    user: &str,
) {
    assert_eq!(grants.len(), 1, "{user} must receive exactly one grant");
    assert_eq!(grants[0].id, grant_id(&schedule.id, start, user));
    assert_eq!(grants[0].amount, Credits::from_whole(1000));
    assert!(grants[0].issued_ledgered_at.is_some());
}

#[tokio::test]
async fn organization_joins_receive_the_current_week_once_across_paths_and_rejoins() {
    let (db, now) = setup("eligibility_org_join").await;
    for org in ["org-a", "org-b"] {
        insert_owner(&db, org, UserType::Org, now - Duration::days(30)).await;
    }
    insert_owner(
        &db,
        "old-member",
        UserType::Person,
        now - Duration::days(10),
    )
    .await;
    join_org(&db, "org-a", "old-member").await;
    let schedule = create_schedule(
        &db,
        member_input(
            BillingTargetKind::OrgMembers,
            ScheduleTiming::Calendar,
            &["org-a", "org-b"],
        ),
    )
    .await
    .unwrap();
    // This week's walk finishes before anyone below joins.
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let window = schedule_period(schedule.recurrence, Utc::now());

    // Admin add and invite acceptance both grant this week immediately.
    insert_owner(&db, "added", UserType::Person, now).await;
    join_org(&db, "org-a", "added").await;
    insert_owner(&db, "invited", UserType::Person, now).await;
    insert_owner(&db, "admin-id", UserType::Person, now).await;
    db.collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            "org-b",
            "admin-id",
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let invite = org_invite_service::create_invite(
        &db,
        "org-b",
        "admin-id",
        OrgRole::Viewer,
        MemberScopeSource::Inherit,
        None,
        None,
    )
    .await
    .unwrap();
    org_invite_service::redeem_invite(&db, &invite.nonce, "invited")
        .await
        .unwrap();
    for user in ["added", "invited"] {
        let grants = grants_for(&db, user).await;
        assert_current_grant(&grants, &schedule, window.start, user);
        assert_eq!(grants[0].expires_at, Some(window.end));
    }

    // A second targeted organization and a leave/rejoin converge on one grant.
    join_org(&db, "org-b", "added").await;
    org_service::revoke_membership(&db, "org-a", "added")
        .await
        .unwrap();
    join_org(&db, "org-a", "added").await;
    assert_current_grant(
        &grants_for(&db, "added").await,
        &schedule,
        window.start,
        "added",
    );
    assert_eq!(
        disburse_due(&db, Utc::now(), MAX_RECIPIENTS_PER_TICK)
            .await
            .unwrap()
            .grants_created,
        0
    );

    // The next week's ordinary walk includes the new members normally.
    let next = window.end + Duration::hours(1);
    disburse_due(&db, next, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let next_window = schedule_period(schedule.recurrence, next);
    assert!(
        grants_for(&db, "invited")
            .await
            .iter()
            .any(|grant| grant.id == grant_id(&schedule.id, next_window.start, "invited"))
    );
}

#[tokio::test]
async fn untargeted_inactive_and_paused_organizations_grant_nothing_on_join() {
    let (db, now) = setup("eligibility_org_ineligible").await;
    for org in ["targeted", "other"] {
        insert_owner(&db, org, UserType::Org, now - Duration::days(30)).await;
    }
    let schedule = create_schedule(
        &db,
        member_input(
            BillingTargetKind::OrgMembers,
            ScheduleTiming::Calendar,
            &["targeted"],
        ),
    )
    .await
    .unwrap();
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();

    insert_owner(&db, "elsewhere", UserType::Person, now).await;
    join_org(&db, "other", "elsewhere").await;
    assert!(grants_for(&db, "elsewhere").await.is_empty());

    // A membership whose organization is no longer active is not eligible.
    insert_owner(&db, "stale", UserType::Person, now).await;
    db.collection::<bson::Document>(MEMBERSHIPS)
        .insert_one(doc! { "_id": "stale-membership", "org_user_id": "targeted",
        "member_user_id": "stale", "role": "member", "scope_source": "inherit",
        "allowed_service_ids": Bson::Null, "revoked_at": Bson::Null,
        "created_at": bson::DateTime::now() })
        .await
        .unwrap();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "targeted" },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    grant_on_eligibility(&db, "stale", Eligibility::Org("targeted"), now, Utc::now())
        .await
        .unwrap();
    assert!(grants_for(&db, "stale").await.is_empty());

    // A paused calendar schedule grants nothing to new members.
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "targeted" },
            doc! { "$set": { "is_active": true } },
        )
        .await
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
    insert_owner(&db, "paused", UserType::Person, now).await;
    join_org(&db, "targeted", "paused").await;
    assert!(grants_for(&db, "paused").await.is_empty());
}

#[tokio::test]
async fn group_additions_receive_the_current_period_once() {
    let (db, now) = setup("eligibility_group_join").await;
    db.collection::<Group>(GROUPS)
        .insert_one(Group {
            id: "team".into(),
            name: "Team".into(),
            slug: "team".into(),
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
        member_input(
            BillingTargetKind::Groups,
            ScheduleTiming::Calendar,
            &["team"],
        ),
    )
    .await
    .unwrap();
    // "existing" predates the claim but joins the group only after the walk.
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let window = schedule_period(schedule.recurrence, Utc::now());
    group_service::add_member(&db, "team", "existing")
        .await
        .unwrap();
    assert_current_grant(
        &grants_for(&db, "existing").await,
        &schedule,
        window.start,
        "existing",
    );
    group_service::remove_member(&db, "team", "existing")
        .await
        .unwrap();
    group_service::add_member(&db, "team", "existing")
        .await
        .unwrap();
    assert_current_grant(
        &grants_for(&db, "existing").await,
        &schedule,
        window.start,
        "existing",
    );
}

#[tokio::test]
async fn organization_joins_enroll_recipient_cycles_at_the_join_time() {
    let (db, now) = setup("eligibility_org_recipient").await;
    insert_owner(&db, "org-r", UserType::Org, now - Duration::days(30)).await;
    let schedule = create_schedule(
        &db,
        member_input(
            BillingTargetKind::OrgMembers,
            ScheduleTiming::Recipient,
            &["org-r"],
        ),
    )
    .await
    .unwrap();
    insert_owner(&db, "member", UserType::Person, now).await;
    let membership = org_service::create_membership(
        &db,
        "org-r",
        "member",
        OrgRole::Member,
        MemberScopeSource::Inherit,
        None,
    )
    .await
    .unwrap();
    let recipient = db
        .collection::<CreditScheduleRecipient>(RECIPIENTS)
        .find_one(doc! { "schedule_id": &schedule.id, "recipient_user_id": "member" })
        .await
        .unwrap()
        .expect("joining enrolls the member without waiting for discovery");
    assert_eq!(
        recipient.anchor_at.timestamp_millis(),
        membership
            .created_at
            .max(schedule.created_at)
            .timestamp_millis()
    );
    let grants = grants_for(&db, "member").await;
    assert_eq!(grants.len(), 1);
    assert!(grants[0].issued_ledgered_at.is_some());
}

pub(crate) async fn login_test_schedule(db: &mongodb::Database) -> CreditSchedule {
    create_schedule(
        db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Recipient),
    )
    .await
    .unwrap()
}

pub(crate) async fn assert_login_credits(
    db: &mongodb::Database,
    schedule: &CreditSchedule,
    user_id: &str,
    expected: bool,
) {
    let grants = super::super::super::grants::list_active_for_user(db, user_id, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        grants
            .iter()
            .filter(|grant| {
                grant
                    .schedule_origin
                    .as_ref()
                    .is_some_and(|origin| origin.schedule_id == schedule.id)
            })
            .count(),
        usize::from(expected),
        "spendable scheduled credits for {user_id}"
    );
}

#[tokio::test]
async fn calendar_progress_counts_a_grant_that_seed_already_observed_once() {
    let (db, now) = setup("eligibility_count_race").await;
    let schedule = create_schedule(
        &db,
        input(BillingTargetKind::AllUsers, ScheduleTiming::Calendar),
    )
    .await
    .unwrap();
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    insert_owner(&db, "late", UserType::Person, now).await;

    let inserted = std::sync::Arc::new(tokio::sync::Notify::new());
    let resume = std::sync::Arc::new(tokio::sync::Notify::new());
    let hook = AFTER_CALENDAR_INSERT.scope(
        (inserted.clone(), resume.clone()),
        grant_on_eligibility(&db, "late", Eligibility::Login, now, now),
    );
    let seed = async {
        inserted.notified().await;
        let result = seeding::seed_missing(&db, &schedule.id, Default::default(), Utc::now()).await;
        resume.notify_one();
        result.unwrap()
    };
    let (hook, seeded) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(hook, seed)
    })
    .await
    .unwrap();
    hook.unwrap();
    assert_eq!(seeded.created_count, 0);
    assert_eq!(seeded.existing_count, 2);
    let period = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": period_id(&schedule.id, seeded.period_start) })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(period.disbursed_count, 2);
}

#[tokio::test]
async fn organization_join_follows_frozen_policy_and_exit_preserves_issued_credits() {
    let (db, now) = setup("eligibility_frozen_org").await;
    for id in ["targeted", "unrelated"] {
        insert_owner(&db, id, UserType::Org, now).await;
    }
    let schedule = create_schedule(
        &db,
        member_input(
            BillingTargetKind::OrgMembers,
            ScheduleTiming::Calendar,
            &["targeted"],
        ),
    )
    .await
    .unwrap();
    disburse_due(&db, now, MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let unrelated = login_test_schedule(&db).await;
    update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            target_org_ids: Some(vec!["unrelated".into()]),
            amount_credits: Some(777),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    join_org(&db, "unrelated", "existing").await;
    assert!(grants_for(&db, "existing").await.is_empty());
    join_org(&db, "targeted", "existing").await;
    let window = schedule_period(schedule.recurrence, Utc::now());
    let grants = grants_for(&db, "existing").await;
    assert_current_grant(&grants, &schedule, window.start, "existing");
    assert_login_credits(&db, &unrelated, "existing", false).await;
    assert_eq!(
        db.collection::<CreditScheduleRecipient>(RECIPIENTS)
            .count_documents(doc! { "schedule_id": &unrelated.id })
            .await
            .unwrap(),
        0,
    );

    org_service::revoke_membership(&db, "targeted", "existing")
        .await
        .unwrap();
    assert_current_grant(
        &grants_for(&db, "existing").await,
        &schedule,
        window.start,
        "existing",
    );
    join_org(&db, "targeted", "existing").await;
    assert_current_grant(
        &grants_for(&db, "existing").await,
        &schedule,
        window.start,
        "existing",
    );
}

#[tokio::test]
async fn founder_join_loads_recipient_credits_after_org_commit() {
    let (db, now) = setup("eligibility_founder").await;
    let mut request = member_input(
        BillingTargetKind::OrgMembers,
        ScheduleTiming::Recipient,
        &["future-org"],
    );
    // Authoring validates targets. Simulate a previously configured id to
    // exercise the founder transaction for this reserved organization id.
    request.target_org_ids.clear();
    request.target_kind = BillingTargetKind::AllUsers;
    let schedule = create_schedule(&db, request).await.unwrap();
    db.collection::<bson::Document>(CREDIT_SCHEDULES)
        .update_one(
            doc! { "_id": &schedule.id },
            doc! { "$set": {
                "target_kind": "org_members", "target_org_ids": ["future-org"],
            } },
        )
        .await
        .unwrap();
    let actor_id = Uuid::new_v4().to_string();
    insert_owner(&db, &actor_id, UserType::Person, now).await;
    let actor = crate::services::audit_service::AuditActor {
        user_id: actor_id.clone(),
        ..org_service::audit::fixture_actor()
    };
    org_service::profile::create(&db, "future-org", "Founding org", None, None, &actor)
        .await
        .unwrap();
    assert_login_credits(&db, &schedule, &actor_id, true).await;
}

#[tokio::test]
async fn password_and_mfa_load_credits_only_after_successful_cookie_or_token_login() {
    use crate::handlers::{
        auth::{LoginRequest, login},
        mfa::{MfaLoginVerifyRequest, verify},
    };
    use crate::services::mfa_service;
    use axum::{
        Json,
        extract::{ConnectInfo, State},
        http::{HeaderMap, header},
    };

    let (db, _) = setup("eligibility_password_mfa").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let peer = "127.0.0.1:12345".parse().unwrap();
    let schedule = login_test_schedule(&db).await;
    let password = "fixture-password-1";
    let hash = crate::crypto::password::hash_password(password).unwrap();
    for client in ["web", "token"] {
        for factor in ["password", "inline", "challenge"] {
            let user_id = Uuid::new_v4().to_string();
            let mut user = test_user(&user_id, UserType::Person);
            user.password_hash = Some(hash.clone());
            let email = user.email.clone();
            db.collection::<User>(USERS).insert_one(user).await.unwrap();
            let recovery = if factor == "password" {
                None
            } else {
                let setup = mfa_service::setup_totp(&db, &state.encryption_keys, &user_id, &email)
                    .await
                    .unwrap();
                let totp = totp_rs::TOTP::from_url(setup.qr_code_url).unwrap();
                let codes = mfa_service::verify_totp_setup(
                    &db,
                    &state.encryption_keys,
                    &setup.factor_id,
                    &user_id,
                    &totp.generate_current().unwrap(),
                    None,
                    None,
                )
                .await
                .unwrap();
                db.collection::<User>(USERS)
                    .update_one(
                        doc! { "_id": &user_id },
                        doc! { "$set": { "mfa_enabled": true } },
                    )
                    .await
                    .unwrap();
                Some(codes[0].clone())
            };
            let sign_in = |password: &str, mfa_code| {
                login(
                    State(state.clone()),
                    ConnectInfo(peer),
                    HeaderMap::new(),
                    Json(LoginRequest {
                        email: email.clone(),
                        password: password.into(),
                        mfa_code,
                        client: Some(client.into()),
                    }),
                )
            };
            assert!(sign_in("wrong-password", None).await.is_err());
            assert_login_credits(&db, &schedule, &user_id, false).await;

            let (headers, Json(response)) = match factor {
                "inline" => sign_in(password, recovery).await.unwrap(),
                "challenge" => {
                    let Err(AppError::MfaRequired { session_token }) =
                        sign_in(password, None).await
                    else {
                        panic!("expected a pending MFA challenge")
                    };
                    assert_login_credits(&db, &schedule, &user_id, false).await;
                    verify(
                        State(state.clone()),
                        ConnectInfo(peer),
                        HeaderMap::new(),
                        Json(MfaLoginVerifyRequest {
                            code: zeroize::Zeroizing::new(recovery.unwrap()),
                            mfa_token: Some(zeroize::Zeroizing::new(session_token)),
                            client: Some(client.into()),
                        }),
                    )
                    .await
                    .unwrap()
                }
                _ => sign_in(password, None).await.unwrap(),
            };
            if client == "web" {
                assert!(
                    headers
                        .get_all(header::SET_COOKIE)
                        .iter()
                        .any(|value| { value.to_str().unwrap().starts_with("nyx_session=") })
                );
            }
            assert_eq!(response.access_token.is_some(), client == "token");
            assert_login_credits(&db, &schedule, &user_id, true).await;
        }
    }
}

#[tokio::test]
async fn returning_login_matches_selected_org_and_direct_group_and_never_refills() {
    use crate::handlers::auth::{LoginRequest, login};
    use crate::services::billing::grants;
    use axum::{
        Json,
        extract::{ConnectInfo, State},
        http::HeaderMap,
    };

    let (db, now) = setup("eligibility_returning_user").await;
    let id = Uuid::new_v4().to_string();
    let mut user = test_user(&id, UserType::Person);
    let password = "fixture-password-1";
    user.password_hash = Some(crate::crypto::password::hash_password(password).unwrap());
    user.group_ids = vec!["direct".into()];
    let email = user.email.clone();
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    insert_owner(&db, "targeted", UserType::Org, now).await;
    insert_owner(&db, "inactive", UserType::Org, now).await;
    join_org(&db, "targeted", &id).await;
    join_org(&db, "inactive", &id).await;
    for (group, parent) in [("parent", None), ("direct", Some("parent".into()))] {
        db.collection::<Group>(GROUPS)
            .insert_one(Group {
                id: group.into(),
                name: group.into(),
                slug: group.into(),
                description: None,
                role_ids: vec![],
                parent_group_id: parent,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }
    let mut selected = input(BillingTargetKind::SelectedUsers, ScheduleTiming::Recipient);
    selected.target_user_ids = vec![id.clone()];
    let selected = create_schedule(&db, selected).await.unwrap();
    let mut excluded = Vec::new();
    let mut matched = Vec::new();
    for (kind, target, enabled) in [
        (BillingTargetKind::OrgMembers, "targeted", true),
        (BillingTargetKind::OrgMembers, "inactive", false),
        (BillingTargetKind::Groups, "direct", true),
        (BillingTargetKind::Groups, "parent", false),
    ] {
        let schedule = create_schedule(
            &db,
            member_input(kind, ScheduleTiming::Recipient, &[target]),
        )
        .await
        .unwrap();
        if enabled {
            matched.push(schedule);
        } else {
            excluded.push(schedule);
        }
    }
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "inactive" },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    let paused = login_test_schedule(&db).await;
    update_schedule(
        &db,
        &paused.id,
        UpdateScheduleInput {
            is_active: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let sign_in = || {
        login(
            State(state.clone()),
            ConnectInfo("127.0.0.1:12345".parse().unwrap()),
            HeaderMap::new(),
            Json(LoginRequest {
                email: email.clone(),
                password: password.into(),
                mfa_code: None,
                client: Some("token".into()),
            }),
        )
    };
    let (_, Json(response)) = sign_in().await.unwrap();
    assert_eq!(response.user_id, id);
    assert_login_credits(&db, &selected, &id, true).await;
    for schedule in &matched {
        assert_login_credits(&db, schedule, &id, true).await;
    }
    for schedule in excluded.iter().chain([&paused]) {
        assert_login_credits(&db, schedule, &id, false).await;
    }
    let first = grants_for(&db, &id).await;
    let selected_grant = first
        .iter()
        .find(|grant| grant.schedule_origin.as_ref().unwrap().schedule_id == selected.id)
        .unwrap();
    grants::revoke_grant_immediate(&db, &selected_grant.id)
        .await
        .unwrap();
    let (_, Json(response)) = sign_in().await.unwrap();
    assert_eq!(response.user_id, id);
    assert_login_credits(&db, &selected, &id, false).await;
    assert_eq!(grants_for(&db, &id).await.len(), first.len());
    assert_eq!(
        db.collection::<bson::Document>(CREDIT_SCHEDULE_PERIODS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn signup_credit_issuance_waits_for_exact_accounting_readiness() {
    let (db, _) = setup("eligibility_exact_gate").await;
    let migrations = db.collection::<bson::Document>("billing_migrations");
    migrations
        .delete_one(doc! { "_id": "exact-v2" })
        .await
        .unwrap();
    let schedule = login_test_schedule(&db).await;
    let user_id = auth_service::register_user(
        &db,
        "migration@example.com",
        "password123",
        None,
        None,
        true,
    )
    .await
    .unwrap()
    .user_id;
    assert!(grants_for(&db, &user_id).await.is_empty());
    migrations
        .insert_one(doc! { "_id": "exact-v2", "completed_at": bson::DateTime::now() })
        .await
        .unwrap();
    after_human_login(&db, &user_id).await;
    assert_login_credits(&db, &schedule, &user_id, true).await;
}
