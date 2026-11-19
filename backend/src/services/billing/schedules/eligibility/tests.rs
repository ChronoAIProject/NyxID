use super::*;
use crate::models::credit_schedule_recipient::{
    COLLECTION_NAME as RECIPIENTS, CreditScheduleRecipient,
};
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::org_membership::{MemberScopeSource, OrgRole};
use crate::models::user::UserType;
use crate::services::social_auth_service::{SocialProfile, SocialProvider};
use crate::services::{
    admin_user_service, auth_service, group_service, org_invite_service, org_service,
    social_auth_service,
};
use crate::test_utils::{connect_transaction_test_database, test_user};

tokio::task_local! {
    static AFTER_PERIOD_READ_TIME: DateTime<Utc>;
}

pub(super) fn calendar_check_time(now: DateTime<Utc>) -> DateTime<Utc> {
    AFTER_PERIOD_READ_TIME
        .try_with(|at| now.max(*at))
        .unwrap_or(now)
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
                grant_current_period(&db, &schedule, "late", Eligibility::Account, now),
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
    grant_current_period(
        &db,
        &schedule,
        "late",
        Eligibility::Account,
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

    for user_id in [&registered, &admin_created, &social.user.id] {
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
    assert_eq!(period.disbursed_count, 4);
    assert_eq!(
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .count_documents(doc! { "schedule_origin.schedule_id": &schedule.id })
            .await
            .unwrap(),
        4
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
    disburse_due(&db, Utc::now(), MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
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
    disburse_due(&db, Utc::now(), MAX_RECIPIENTS_PER_TICK)
        .await
        .unwrap();
    let grants = grants_for(&db, "member").await;
    assert_eq!(grants.len(), 1);
    assert!(grants[0].issued_ledgered_at.is_some());
}
