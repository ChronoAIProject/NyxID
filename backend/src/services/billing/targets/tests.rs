use super::*;
use crate::models::credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant};
use crate::models::group::Group;
use crate::models::org_membership::{OrgMembership, OrgRole};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_allowance::{
    AllowanceRecurrence, COLLECTION_NAME as ALLOWANCES, UsageAllowance,
};
use crate::models::usage_allowance_period::COLLECTION_NAME as PERIODS;
use crate::models::user::{User, UserType};
use crate::services::billing::{allowances, grants, ledger};
use crate::test_utils::{connect_transaction_test_database, test_membership, test_user};
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::{Arc, Mutex};

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

async fn seed(db: &mongodb::Database) {
    let mut users: Vec<_> = ["person", "viewer", "revoked", "inactive", "outsider"]
        .into_iter()
        .map(|id| test_user(id, UserType::Person))
        .collect();
    users.extend([
        test_user("org-a", UserType::Org),
        test_user("org-b", UserType::Org),
    ]);
    for user in &mut users {
        user.group_ids = ids(&["group-a", "group-b"]);
    }
    users
        .iter_mut()
        .find(|user| user.id == "inactive")
        .unwrap()
        .is_active = false;
    users
        .iter_mut()
        .find(|user| user.id == "outsider")
        .unwrap()
        .group_ids = ids(&["child"]);
    db.collection::<User>(USERS)
        .insert_many(users)
        .await
        .unwrap();
    let mut memberships = Vec::new();
    for org in ["org-a", "org-b"] {
        for (id, role) in [
            ("person", OrgRole::Admin),
            ("viewer", OrgRole::Viewer),
            ("revoked", OrgRole::Member),
            ("inactive", OrgRole::Member),
            ("org-a", OrgRole::Member),
        ] {
            let mut membership = test_membership(org, id, role, None);
            if id == "revoked" {
                membership.revoked_at = Some(Utc::now());
            }
            memberships.push(membership);
        }
    }
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(memberships)
        .await
        .unwrap();
    for id in ["group-a", "group-b", "child", "empty"] {
        db.collection::<Group>(GROUPS)
            .insert_one(Group {
                id: id.to_string(),
                name: id.to_string(),
                slug: id.to_string(),
                description: None,
                role_ids: vec![],
                parent_group_id: (id == "child").then(|| "group-a".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
    }
}

#[test]
fn all_kind_list_pairings_and_caps() {
    for kind in [
        BillingTargetKind::AllUsers,
        BillingTargetKind::SelectedUsers,
        BillingTargetKind::OrgMembers,
        BillingTargetKind::Groups,
    ] {
        for mask in 0..8 {
            let lists: [Vec<String>; 3] = std::array::from_fn(|i| {
                if mask & (1 << i) != 0 {
                    ids(&["one"])
                } else {
                    vec![]
                }
            });
            let expected = match kind {
                BillingTargetKind::AllUsers => 0,
                BillingTargetKind::SelectedUsers => 1,
                BillingTargetKind::OrgMembers => 2,
                BillingTargetKind::Groups => 4,
            };
            assert_eq!(
                validate_shape(kind, &lists[0], &lists[1], &lists[2]).is_ok(),
                mask == expected,
                "{kind:?}: {mask}"
            );
        }
    }
    for (index, kind) in [
        BillingTargetKind::SelectedUsers,
        BillingTargetKind::OrgMembers,
        BillingTargetKind::Groups,
    ]
    .into_iter()
    .enumerate()
    {
        for size in [MAX_SELECTED_TARGETS, MAX_SELECTED_TARGETS + 1] {
            let mut lists: [Vec<String>; 3] = Default::default();
            lists[index] = (0..size).map(|i| format!("id-{i}")).collect();
            assert_eq!(
                validate_shape(kind, &lists[0], &lists[1], &lists[2]).is_ok(),
                size == MAX_SELECTED_TARGETS
            );
        }
        for bad in [
            ids(&["same", "same"]),
            ids(&[""]),
            ids(&["  "]),
            ids(&[" person"]),
            ids(&["person "]),
            ids(&["\tperson\n"]),
        ] {
            let mut lists: [Vec<String>; 3] = Default::default();
            lists[index] = bad;
            assert!(validate_shape(kind, &lists[0], &lists[1], &lists[2]).is_err());
        }
    }
}

#[test]
fn kind_updates_require_matching_list_and_clear_old_lists() {
    for (kind, index) in [
        (BillingTargetKind::SelectedUsers, 0),
        (BillingTargetKind::OrgMembers, 1),
        (BillingTargetKind::Groups, 2),
    ] {
        assert!(
            updated_lists(
                BillingTargetKind::AllUsers,
                kind,
                [&[], &[], &[]],
                [None, None, None]
            )
            .is_err()
        );
        let mut requested: [Option<Vec<String>>; 3] = Default::default();
        requested[index] = Some(ids(&["new"]));
        let result = updated_lists(
            BillingTargetKind::AllUsers,
            kind,
            [&[], &[], &[]],
            requested,
        )
        .unwrap();
        assert_eq!(result[index], ids(&["new"]));
        assert_eq!(
            updated_lists(
                kind,
                BillingTargetKind::AllUsers,
                [&result[0], &result[1], &result[2]],
                [None, None, None]
            )
            .unwrap(),
            [Vec::<String>::new(), Vec::new(), Vec::new()]
        );
    }
    assert!(
        updated_lists(
            BillingTargetKind::SelectedUsers,
            BillingTargetKind::Groups,
            [&ids(&["old"]), &[], &[]],
            [Some(ids(&["old"])), None, Some(ids(&["new"]))]
        )
        .is_err()
    );
}

#[tokio::test]
async fn validates_org_type_activity_and_group_existence() {
    let db = connect_transaction_test_database("credit_target_validation").await;
    seed(&db).await;
    for org in ["person", "missing", "inactive"] {
        assert!(
            validate(&db, BillingTargetKind::OrgMembers, &[], &ids(&[org]), &[])
                .await
                .is_err()
        );
    }
    validate(
        &db,
        BillingTargetKind::OrgMembers,
        &[],
        &ids(&["org-a", "org-b"]),
        &[],
    )
    .await
    .unwrap();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "org-b" },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert!(
        validate(
            &db,
            BillingTargetKind::OrgMembers,
            &[],
            &ids(&["org-b"]),
            &[]
        )
        .await
        .is_err()
    );
    validate(&db, BillingTargetKind::Groups, &[], &[], &ids(&["group-a"]))
        .await
        .unwrap();
    assert!(
        validate(&db, BillingTargetKind::Groups, &[], &[], &ids(&["missing"]))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn organization_recipients_dedupe_include_viewers_and_exclude_ineligible_owners() {
    let db = connect_transaction_test_database("credit_target_orgs").await;
    seed(&db).await;
    assert_eq!(
        grants::resolve_recipients(
            &db,
            BillingTargetKind::OrgMembers,
            &[],
            &ids(&["org-a", "org-b"]),
            &[]
        )
        .await
        .unwrap(),
        ids(&["person", "viewer"])
    );
}

#[tokio::test]
async fn group_recipients_dedupe_direct_members_and_exclude_orgs_and_inactive_people() {
    let db = connect_transaction_test_database("credit_target_groups").await;
    seed(&db).await;
    assert_eq!(
        grants::resolve_recipients(
            &db,
            BillingTargetKind::Groups,
            &[],
            &[],
            &ids(&["group-a", "group-b"])
        )
        .await
        .unwrap(),
        ids(&["person", "revoked", "viewer"])
    );
}

fn grant_input(kind: BillingTargetKind) -> grants::IssueCreditGrantInput {
    grants::IssueCreditGrantInput {
        amount_credits: 2,
        target_kind: kind,
        target_user_ids: vec![],
        target_org_ids: if kind == BillingTargetKind::OrgMembers {
            ids(&["org-a", "org-b"])
        } else {
            vec![]
        },
        target_group_ids: if kind == BillingTargetKind::Groups {
            ids(&["group-a", "group-b"])
        } else {
            vec![]
        },
        all_services: true,
        service_refs: vec![],
        expires_at: None,
        reason: None,
        granted_by: "admin".to_string(),
    }
}

#[tokio::test]
async fn issued_member_grants_snapshot_provenance_and_keep_bounded_ledger_activation() {
    let db = connect_transaction_test_database("credit_target_issuance").await;
    seed(&db).await;
    ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    // Expand beyond the inline ledger budget without changing the target cap.
    let users: Vec<_> = (0..55)
        .map(|i| {
            let mut user = test_user(&format!("extra-{i:03}"), UserType::Person);
            user.group_ids = ids(&["group-a", "group-b"]);
            user
        })
        .collect();
    db.collection::<User>(USERS)
        .insert_many(users)
        .await
        .unwrap();
    for kind in [BillingTargetKind::OrgMembers, BillingTargetKind::Groups] {
        let input = grant_input(kind);
        let issued = grants::issue_grants(&db, input.clone()).await.unwrap();
        let expected = if kind == BillingTargetKind::OrgMembers {
            2
        } else {
            58
        };
        assert_eq!(issued.len(), expected);
        assert_eq!(
            issued
                .iter()
                .filter(|g| g.issued_ledgered_at.is_some())
                .count(),
            expected.min(grants::INLINE_ISSUANCE_LEDGER_LIMIT)
        );
        for grant in &issued {
            assert_eq!(grant.batch_id, issued[0].batch_id);
            assert_eq!(grant.target_org_ids, input.target_org_ids);
            assert_eq!(grant.target_group_ids, input.target_group_ids);
        }
    }
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .delete_many(doc! {})
        .await
        .unwrap();
    db.collection::<User>(USERS)
        .update_many(doc! {}, doc! { "$set": { "group_ids": [] } })
        .await
        .unwrap();
    assert_eq!(
        db.collection::<CreditGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        60
    );
    let error = grants::issue_grants(&db, grant_input(BillingTargetKind::OrgMembers))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::ValidationError(message) if message == "credit grant has no eligible user recipients")
    );
    let error = grants::issue_grants(&db, grant_input(BillingTargetKind::Groups))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::ValidationError(message) if message == "credit grant has no eligible user recipients")
    );
}

async fn seed_allowance(db: &mongodb::Database, kind: BillingTargetKind) -> UsageAllowance {
    let allowance = UsageAllowance {
        id: format!("allowance-{kind:?}"),
        service_id: "service".to_string(),
        service_slug: "service".to_string(),
        metric: BillingMetric::Requests,
        quantity: 100,
        recurrence: AllowanceRecurrence::Monthly,
        target_kind: kind,
        target_user_ids: vec![],
        target_org_ids: if kind == BillingTargetKind::OrgMembers {
            ids(&["org-a", "org-b"])
        } else {
            vec![]
        },
        target_group_ids: if kind == BillingTargetKind::Groups {
            ids(&["group-a", "group-b"])
        } else {
            vec![]
        },
        is_active: true,
        created_by: "admin".into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.collection::<UsageAllowance>(ALLOWANCES)
        .insert_one(&allowance)
        .await
        .unwrap();
    allowance
}

async fn assert_matching(db: &mongodb::Database, owner: &str, expected: usize) {
    assert_eq!(
        allowances::list_current_for_user(db, owner, Utc::now())
            .await
            .unwrap()
            .len(),
        expected,
        "display {owner}"
    );
    assert_eq!(
        allowances::applicable_allowances(
            db,
            owner,
            Some("service"),
            None,
            BillingMetric::Requests
        )
        .await
        .unwrap()
        .len(),
        expected,
        "funding {owner}"
    );
    assert!(
        allowances::applicable_allowances(db, owner, Some("service"), None, BillingMetric::Tokens)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn allowances_follow_live_memberships_and_groups_but_never_org_wallets() {
    let db = connect_transaction_test_database("credit_target_live").await;
    seed(&db).await;
    seed_allowance(&db, BillingTargetKind::OrgMembers).await;
    seed_allowance(&db, BillingTargetKind::Groups).await;
    assert_matching(&db, "person", 2).await;
    assert_matching(&db, "viewer", 2).await;
    assert_matching(&db, "org-a", 0).await;
    assert_matching(&db, "inactive", 0).await;
    assert_matching(&db, "outsider", 0).await;
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .update_many(
            doc! { "member_user_id": "person" },
            doc! { "$set": { "revoked_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    assert_matching(&db, "person", 1).await;
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "person" },
            doc! { "$set": { "group_ids": [] } },
        )
        .await
        .unwrap();
    assert_matching(&db, "person", 0).await;
    assert_eq!(
        db.collection::<Document>(PERIODS)
            .count_documents(doc! { "owner_user_id": "person" })
            .await
            .unwrap(),
        2
    );
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership("org-a", "outsider", OrgRole::Viewer, None))
        .await
        .unwrap();
    assert_matching(&db, "outsider", 1).await;
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "outsider" },
            doc! { "$set": { "group_ids": ["group-a"] } },
        )
        .await
        .unwrap();
    assert_matching(&db, "outsider", 2).await;
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": "org-a" },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert_matching(&db, "outsider", 1).await;
}

#[tokio::test]
async fn allowance_partial_target_updates_clear_stale_ids_and_validate_before_write() {
    let db = connect_transaction_test_database("credit_target_update").await;
    seed(&db).await;
    let allowance = seed_allowance(&db, BillingTargetKind::OrgMembers).await;
    assert!(
        allowances::update_allowance(
            &db,
            &allowance.id,
            allowances::UpdateAllowanceInput {
                target_kind: Some(BillingTargetKind::Groups),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let updated = allowances::update_allowance(
        &db,
        &allowance.id,
        allowances::UpdateAllowanceInput {
            target_kind: Some(BillingTargetKind::Groups),
            target_group_ids: Some(ids(&["group-a"])),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(updated.target_org_ids.is_empty());
    assert_eq!(updated.target_group_ids, ids(&["group-a"]));
    let updated = allowances::update_allowance(
        &db,
        &allowance.id,
        allowances::UpdateAllowanceInput {
            quantity: Some(200),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.target_group_ids, ids(&["group-a"]));
    let updated = allowances::update_allowance(
        &db,
        &allowance.id,
        allowances::UpdateAllowanceInput {
            target_kind: Some(BillingTargetKind::AllUsers),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(updated.target_group_ids.is_empty());
}

#[tokio::test]
async fn recipient_indexes_are_idempotent_and_cover_member_query_shapes() {
    let db = connect_transaction_test_database("credit_target_indexes").await;
    crate::db::ensure_indexes(&db).await.unwrap();
    crate::db::ensure_indexes(&db).await.unwrap();
    for (collection, expected) in [
        (ALLOWANCES, doc! { "target_org_ids": 1, "is_active": 1 }),
        (ALLOWANCES, doc! { "target_group_ids": 1, "is_active": 1 }),
        (USERS, doc! { "group_ids": 1, "is_active": 1, "_id": 1 }),
        (MEMBERSHIPS, doc! { "org_user_id": 1, "revoked_at": 1 }),
        (MEMBERSHIPS, doc! { "member_user_id": 1, "revoked_at": 1 }),
    ] {
        let indexes: Vec<_> = db
            .collection::<Document>(collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            indexes.iter().any(|index| index.keys == expected),
            "{collection}: {expected:?}"
        );
    }
}

#[tokio::test]
async fn oversized_member_expansion_fails_before_issuing_any_grants() {
    let db = connect_transaction_test_database("credit_member_expansion_cap").await;
    // Only the recipient query's projected fields are needed for this large
    // fixture. The group is validated by id, not deserialized for expansion.
    db.collection::<Document>(GROUPS)
        .insert_one(doc! { "_id": "group-a" })
        .await
        .unwrap();
    for start in (0..=grants::MAX_MEMBER_RECIPIENTS).step_by(5_000) {
        let end = (start + 5_000).min(grants::MAX_MEMBER_RECIPIENTS + 1);
        db.collection::<Document>(USERS).insert_many((start..end).map(|i| doc! {
            "_id": format!("person-{i:06}"), "is_active": true, "user_type": "person", "group_ids": ["group-a"],
        })).await.unwrap();
    }
    let mut input = grant_input(BillingTargetKind::Groups);
    input.target_group_ids = ids(&["group-a"]);
    let error = grants::issue_grants(&db, input).await.unwrap_err();
    assert!(
        matches!(error, AppError::ValidationError(message) if message.contains("100000 resolved recipients"))
    );
    assert_eq!(
        db.collection::<Document>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

async fn monitored_database(prefix: &str) -> (mongodb::Database, Arc<Mutex<Vec<Document>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let observed = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && matches!(
                event.command_name.as_str(),
                "aggregate" | "find" | "distinct"
            )
        {
            observed.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_test_database_with_command_handler(prefix, handler)
        .await
        .unwrap();
    (db, commands)
}

#[tokio::test]
async fn organization_pages_start_from_memberships_and_ignore_earlier_unrelated_people() {
    let (db, commands) = monitored_database("credit_org_pages").await;
    seed(&db).await;
    db.collection::<User>(USERS)
        .insert_one(test_user("000-unrelated", UserType::Person))
        .await
        .unwrap();
    let orgs = ids(&["org-a", "org-b"]);
    let cutoff = Utc::now();
    commands.lock().unwrap().clear();
    for (cursor, expected) in [
        (None, ids(&["person"])),
        (Some("000-unrelated"), ids(&["person"])),
        (Some("person"), ids(&["viewer"])),
        (Some("viewer"), vec![]),
    ] {
        assert_eq!(
            member_recipients(
                &db,
                BillingTargetKind::OrgMembers,
                &orgs,
                &[],
                Some(cutoff),
                cursor,
                1
            )
            .await
            .unwrap(),
            expected,
        );
    }
    let commands = commands.lock().unwrap();
    assert_eq!(commands.len(), 4);
    assert!(
        commands
            .iter()
            .all(|command| command.get_str("aggregate") == Ok(MEMBERSHIPS))
    );
}

#[tokio::test]
async fn allowance_targets_use_one_aggregation_and_preserve_legacy_owner_clauses() {
    let (db, commands) = monitored_database("credit_allowance_query_count").await;
    seed(&db).await;
    // Legacy person rows may omit user_type and group_ids.
    db.collection::<Document>(USERS)
        .insert_one(doc! {
            "_id": "legacy-person", "is_active": true,
        })
        .await
        .unwrap();
    for (owner, expected_count) in [
        ("person", 4),
        ("viewer", 4),
        ("revoked", 3),
        ("outsider", 3),
        ("org-a", 2),
        ("inactive", 2),
        ("missing", 2),
        ("legacy-person", 2),
    ] {
        commands.lock().unwrap().clear();
        let clauses = allowance_clauses(&db, owner).await.unwrap();
        assert_eq!(clauses.len(), expected_count, "{owner}");
        assert_eq!(clauses[0], doc! { "target_kind": "all_users" });
        assert_eq!(
            clauses[1],
            doc! { "target_kind": "selected_users", "target_user_ids": owner }
        );
        let commands = commands.lock().unwrap();
        assert_eq!(commands.len(), 1, "{owner}: {commands:?}");
        assert_eq!(commands[0].get_str("aggregate"), Ok(USERS));
    }
}

#[tokio::test]
async fn selected_recipient_resolution_checks_existing_owners_once() {
    let (db, commands) = monitored_database("credit_selected_query_count").await;
    seed(&db).await;
    for selected in [
        ids(&["viewer", "org-a", "person"]),
        ids(&["missing"]),
        ids(&["inactive"]),
    ] {
        commands.lock().unwrap().clear();
        validate_shape(BillingTargetKind::SelectedUsers, &selected, &[], &[]).unwrap();
        let result =
            grants::resolve_recipients(&db, BillingTargetKind::SelectedUsers, &selected, &[], &[])
                .await;
        if selected.len() == 3 {
            assert_eq!(result.unwrap(), ids(&["org-a", "person", "viewer"]));
        } else {
            assert!(matches!(result, Err(AppError::ValidationError(_))));
        }
        let commands = commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].get_str("find"), Ok(USERS));
    }
}

#[tokio::test]
async fn allowance_rejects_padded_selected_ids_before_persisting() {
    let db = connect_transaction_test_database("credit_allowance_padded_id").await;
    seed(&db).await;
    let allowance = seed_allowance(&db, BillingTargetKind::AllUsers).await;
    for id in [" person", "person ", ""] {
        let result = allowances::update_allowance(
            &db,
            &allowance.id,
            allowances::UpdateAllowanceInput {
                target_kind: Some(BillingTargetKind::SelectedUsers),
                target_user_ids: Some(ids(&[id])),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }
    let persisted = db
        .collection::<UsageAllowance>(ALLOWANCES)
        .find_one(doc! { "_id": allowance.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.target_kind, BillingTargetKind::AllUsers);
    assert!(persisted.target_user_ids.is_empty());
}
