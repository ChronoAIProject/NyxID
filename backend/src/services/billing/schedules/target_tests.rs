use super::*;
use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
use crate::models::user::UserType;
use crate::test_utils::{connect_transaction_test_database, test_membership, test_user};

async fn exercise_member_pages(kind: BillingTargetKind) {
    let db = connect_transaction_test_database("credit_schedule_member_pages").await;
    let before = Utc::now() - Duration::days(1);
    db.collection::<User>(USERS)
        .insert_many([
            test_user("org-a", UserType::Org),
            test_user("org-b", UserType::Org),
        ])
        .await
        .unwrap();
    for id in ["group-a", "group-b"] {
        db.collection::<Group>(GROUPS)
            .insert_one(Group {
                id: id.into(),
                name: id.into(),
                slug: id.into(),
                description: None,
                role_ids: vec![],
                parent_group_id: None,
                created_at: before,
                updated_at: before,
            })
            .await
            .unwrap();
    }
    let expected: Vec<_> = (0..405).map(|i| format!("person-{i:04}")).collect();
    let users: Vec<_> = expected
        .iter()
        .map(|id| {
            let mut user = test_user(id, UserType::Person);
            user.created_at = before;
            user.group_ids = vec!["group-a".into(), "group-b".into()];
            user
        })
        .collect();
    db.collection::<User>(USERS)
        .insert_many(users)
        .await
        .unwrap();
    let memberships: Vec<_> = expected
        .iter()
        .flat_map(|id| {
            ["org-a", "org-b"].map(|org| {
                let mut member = test_membership(org, id, OrgRole::Viewer, None);
                member.created_at = before;
                member
            })
        })
        .collect();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(memberships)
        .await
        .unwrap();
    let mut schedule = create_schedule(
        &db,
        CreateScheduleInput {
            amount_credits: 10,
            recurrence: ScheduleRecurrence::Monthly,
            expiry: CreditExpiryPolicy::EndOfPeriod,
            target_kind: kind,
            target_user_ids: vec![],
            target_org_ids: if kind == BillingTargetKind::OrgMembers {
                vec!["org-a".into(), "org-b".into()]
            } else {
                vec![]
            },
            target_group_ids: if kind == BillingTargetKind::Groups {
                vec!["group-a".into(), "group-b".into()]
            } else {
                vec![]
            },
            all_services: true,
            service_refs: vec![],
            reason: None,
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();
    let now = Utc::now();
    let PeriodClaim::Acquired(mut period) = claim_period(
        &db,
        &schedule,
        schedule_period(schedule.recurrence, now),
        now,
        true,
    )
    .await
    .unwrap() else {
        panic!("claim");
    };
    // Later joins of old people and later signups must not enter an org walk.
    // Group membership has no join timestamp, so its cutoff is user.created_at.
    let mut late_user = test_user("person-late", UserType::Person);
    late_user.created_at = if kind == BillingTargetKind::OrgMembers {
        before
    } else {
        period.created_at + Duration::seconds(1)
    };
    late_user.group_ids = vec!["group-a".into()];
    db.collection::<User>(USERS)
        .insert_one(late_user)
        .await
        .unwrap();
    let mut late_member = test_membership("org-a", "person-late", OrgRole::Admin, None);
    late_member.created_at = period.created_at + Duration::seconds(1);
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(late_member)
        .await
        .unwrap();
    let first = next_recipients(&db, &period, DISBURSEMENT_CHUNK)
        .await
        .unwrap();
    assert_eq!(first, expected[..200]);
    period.cursor_user_id = first.last().cloned();
    // A policy edit must not change the already claimed period.
    schedule = update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            target_kind: Some(BillingTargetKind::SelectedUsers),
            target_user_ids: Some(vec!["org-a".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(schedule.target_org_ids.is_empty());
    assert!(schedule.target_group_ids.is_empty());
    let second = next_recipients(&db, &period, DISBURSEMENT_CHUNK)
        .await
        .unwrap();
    assert_eq!(second, expected[200..400]);
    period.cursor_user_id = second.last().cloned();
    let third = next_recipients(&db, &period, DISBURSEMENT_CHUNK)
        .await
        .unwrap();
    assert_eq!(third, expected[400..]);
    period.cursor_user_id = third.last().cloned();
    assert!(
        next_recipients(&db, &period, DISBURSEMENT_CHUNK)
            .await
            .unwrap()
            .is_empty()
    );
    let grants = grants_for_recipients(&schedule, &period, expected.clone(), now);
    assert_eq!(
        grants.iter().map(|g| &g.id).collect::<HashSet<_>>().len(),
        expected.len()
    );
    for grant in grants {
        assert_eq!(
            grant.id,
            grant_id(&schedule.id, period.period_start, &grant.recipient_user_id)
        );
        assert_eq!(grant.target_kind, kind);
        assert_eq!(grant.target_org_ids, period.target_org_ids);
        assert_eq!(grant.target_group_ids, period.target_group_ids);
    }
    assert!(
        update_schedule(
            &db,
            &schedule.id,
            UpdateScheduleInput {
                target_kind: Some(kind),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let updated = update_schedule(
        &db,
        &schedule.id,
        UpdateScheduleInput {
            target_kind: Some(kind),
            target_org_ids: Some(period.target_org_ids.clone()),
            target_group_ids: Some(period.target_group_ids.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(updated.target_user_ids.is_empty());
    assert_eq!(updated.target_org_ids, period.target_org_ids);
    assert_eq!(updated.target_group_ids, period.target_group_ids);
    let frozen = db
        .collection::<CreditSchedulePeriod>(CREDIT_SCHEDULE_PERIODS)
        .find_one(doc! { "_id": &period.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frozen.target_kind, kind);
    assert_eq!(frozen.target_org_ids, period.target_org_ids);
    assert_eq!(frozen.target_group_ids, period.target_group_ids);
    // Old stored documents deserialize with empty provenance vectors.
    let mut stored = bson::to_document(&schedule).unwrap();
    stored.remove("target_org_ids");
    stored.remove("target_group_ids");
    let legacy: CreditSchedule = bson::from_document(stored).unwrap();
    assert!(legacy.target_org_ids.is_empty());
    assert!(legacy.target_group_ids.is_empty());
    let mut stored = bson::to_document(&period).unwrap();
    stored.remove("target_org_ids");
    stored.remove("target_group_ids");
    let legacy: CreditSchedulePeriod = bson::from_document(stored).unwrap();
    assert!(legacy.target_org_ids.is_empty());
    assert!(legacy.target_group_ids.is_empty());
}

#[tokio::test]
async fn org_member_cursor_pages_freeze_policy_and_exclude_later_joins() {
    exercise_member_pages(BillingTargetKind::OrgMembers).await;
}

#[tokio::test]
async fn group_member_cursor_pages_freeze_policy_and_exclude_later_signups() {
    exercise_member_pages(BillingTargetKind::Groups).await;
}

#[tokio::test]
async fn selected_schedule_creation_validates_users_in_one_query() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};

    let commands = Arc::new(Mutex::new(Vec::new()));
    let observed = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && ["aggregate", "find", "count", "distinct"]
                .iter()
                .any(|name| event.command.get_str(name) == Ok(USERS))
        {
            observed.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "credit_schedule_selected_query_count",
        handler,
    )
    .await
    .unwrap();
    db.collection::<User>(USERS)
        .insert_many([
            test_user("person", UserType::Person),
            test_user("org", UserType::Org),
        ])
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let schedule = create_schedule(
        &db,
        CreateScheduleInput {
            amount_credits: 10,
            recurrence: ScheduleRecurrence::Monthly,
            expiry: CreditExpiryPolicy::Never,
            target_kind: BillingTargetKind::SelectedUsers,
            target_user_ids: vec!["person".into(), "org".into()],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            reason: None,
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(schedule.target_user_ids, vec!["org", "person"]);
    assert_eq!(commands.lock().unwrap().len(), 1);
}
