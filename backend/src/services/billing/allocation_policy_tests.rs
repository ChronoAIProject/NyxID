use super::*;
use crate::models::{
    billing_target::{BillingServiceScope, BillingTargetKind},
    credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant},
    credit_schedule::{CreditExpiryPolicy, ScheduleRecurrence},
    credit_service_limit::CreditServiceLimitInput,
    credits::Credits,
    user::{User, UserType},
};
use crate::test_utils::{
    connect_transaction_test_database, test_auto_connected_catalog_service, test_user,
};
use chrono::{Duration, Utc};
use mongodb::bson::{self, Document, doc};

async fn seed(label: &str) -> mongodb::Database {
    let db = connect_transaction_test_database(label).await;
    ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    db.collection::<User>("users")
        .insert_many([
            test_user("person", UserType::Person),
            test_user("other", UserType::Person),
        ])
        .await
        .unwrap();
    for (id, slug) in [("svc-a", "service-a"), ("svc-b", "service-b")] {
        let mut service = test_auto_connected_catalog_service();
        service.id = id.into();
        service.slug = slug.into();
        service.is_active = true;
        db.collection("downstream_services")
            .insert_one(service)
            .await
            .unwrap();
    }
    db
}
fn rule(refs: &[&str], amount: &str) -> CreditServiceLimitInput {
    CreditServiceLimitInput {
        service_refs: refs.iter().map(|v| (*v).into()).collect(),
        amount: amount.parse().unwrap(),
        label: Some("LLM".into()),
    }
}
fn input() -> grants::IssueCreditGrantInput {
    grants::IssueCreditGrantInput {
        amount_credits: 0,
        amount: Some("0.00000000001".parse().unwrap()),
        target_kind: BillingTargetKind::AllUsers,
        target_user_ids: vec![],
        target_org_ids: vec![],
        target_group_ids: vec![],
        all_services: true,
        service_refs: vec![],
        service_limits: vec![rule(&["svc-a", "service-b"], "0.000000000003")],
        expires_at: None,
        reason: None,
        granted_by: "admin".into(),
        request_id: Some(uuid::Uuid::new_v4().to_string()),
    }
}

#[tokio::test]
async fn legacy_creates_preserve_v4_identity_and_success_without_post_write_reads() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};

    let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
    let observed = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            observed.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "allocation_legacy_write_response",
        handler,
    )
    .await;
    db.collection::<User>("users")
        .insert_one(test_user("person", UserType::Person))
        .await
        .unwrap();
    // An actual database failure at the post-issuance marker must retain the
    // successful legacy response and defer activation, as before allocations.
    db.create_collection(GRANTS)
        .validator(doc! { "$or": [
            { "issued_ledgered_at": { "$exists": false } },
            { "issued_ledgered_at": { "$type": "null" } },
        ] })
        .await
        .unwrap();
    let mut policy = input();
    policy.request_id = None;
    policy.service_limits.clear();
    commands.lock().unwrap().clear();
    let issued = grants::issue_grants(&db, policy).await.unwrap();
    assert_eq!(issued.len(), 1);
    assert_eq!(
        uuid::Uuid::parse_str(&issued[0].id)
            .unwrap()
            .get_version_num(),
        4
    );
    assert_eq!(issued[0].amount, Credits::from_pico(10).unwrap());
    assert!(issued[0].issued_ledgered_at.is_none());
    let grant_commands = commands.lock().unwrap().clone();
    assert_eq!(
        grant_commands
            .iter()
            .filter(|c| c.get_str("insert").ok() == Some(GRANTS))
            .count(),
        1
    );
    assert!(
        !grant_commands
            .iter()
            .any(|c| c.get_str("find").ok() == Some(GRANTS))
    );
    assert!(
        grant_commands
            .iter()
            .any(|c| c.get_str("update").ok() == Some(GRANTS))
    );
    let stored = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {"_id":&issued[0].id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.remaining, issued[0].remaining);
    assert!(stored.issued_ledgered_at.is_none());
    assert!(
        grants::list_active_for_user(&db, "person", Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    db.run_command(doc! {"collMod": GRANTS, "validator": {}})
        .await
        .unwrap();
    grants::recover_unledgered_events(&db, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        grants::list_active_for_user(&db, "person", Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {
                "dedupe_key":format!("grant-issued:{}", issued[0].id)
            })
            .await
            .unwrap(),
        1
    );

    commands.lock().unwrap().clear();
    let schedule = schedules::create_schedule(
        &db,
        schedules::CreateScheduleInput {
            amount_credits: 7,
            amount: None,
            request_id: None,
            recurrence: ScheduleRecurrence::Monthly,
            expiry: CreditExpiryPolicy::Never,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: vec![],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            service_limits: vec![],
            reason: None,
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        uuid::Uuid::parse_str(&schedule.id)
            .unwrap()
            .get_version_num(),
        4
    );
    assert_eq!(schedule.amount, Credits::from_whole(7));
    let schedule_commands = commands.lock().unwrap().clone();
    assert_eq!(
        schedule_commands
            .iter()
            .filter(|c| c.get_str("insert").ok() == Some("credit_schedules"))
            .count(),
        1
    );
    assert!(
        !schedule_commands
            .iter()
            .any(|c| c.get_str("find").ok() == Some("credit_schedules"))
    );
    assert_eq!(
        db.collection::<Document>("credit_schedules")
            .count_documents(doc! {"_id":&schedule.id})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn streaming_summary_exceeds_i64_picos_and_excludes_units_and_future_policy_value() {
    let db = seed("allocation_summary_all_history").await;
    db.collection::<User>("users")
        .insert_many(
            (0..61)
                .map(|i| test_user(&format!("recipient-{i}"), UserType::Person))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    let mut policy = input();
    policy.amount = Some("999999.999999999999".parse().unwrap());
    policy.service_limits.clear();
    let issued = grants::issue_grants(&db, policy).await.unwrap();
    assert_eq!(issued.len(), 63);
    assert_eq!(
        issued
            .iter()
            .filter(|g| g.issued_ledgered_at.is_some())
            .count(),
        50
    );
    db.collection::<Document>("credit_schedules")
        .insert_one(doc! {
            "_id": "future-policy", "amount": Credits::from_whole(1000000), "is_active": true,
            "created_at": bson::DateTime::from_chrono(Utc::now() + Duration::days(1)),
        })
        .await
        .unwrap();
    db.collection::<Document>("usage_allowances")
        .insert_one(doc! {
            "_id": "included-units", "quantity": i64::MAX, "is_active": true,
        })
        .await
        .unwrap();
    let summary = summaries::allocations(&db, None, Utc::now()).await.unwrap();
    let each: Credits = "999999.999999999999".parse().unwrap();
    assert_eq!(summary.remaining, each.checked_mul(63).unwrap());
    assert!(summary.remaining.pico() > i128::from(i64::MAX));
    assert_eq!(summary.unreserved, each.checked_mul(50).unwrap());
    assert_eq!(summary.pending_activation, each.checked_mul(13).unwrap());
    assert_eq!(summary.held, Credits::ZERO);
    assert_eq!(summary.restricted, Credits::ZERO);
    assert_eq!(
        db.collection::<Document>("usage_allowance_periods")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn exact_submission_replay_freezes_recipients_and_recovers_partial_concurrent_batch() {
    let db = seed("allocation_submission_replay").await;
    let policy = input();
    let (a, b) = tokio::join!(
        grants::issue_grants(&db, policy.clone()),
        grants::issue_grants(&db, policy.clone())
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.len(), 2);
    assert_eq!(
        a.iter().map(|g| &g.id).collect::<Vec<_>>(),
        b.iter().map(|g| &g.id).collect::<Vec<_>>()
    );
    db.collection::<User>("users")
        .insert_one(test_user("late", UserType::Person))
        .await
        .unwrap();
    db.collection::<Document>(GRANTS)
        .delete_one(doc! { "_id": &a[0].id })
        .await
        .unwrap();
    db.collection::<Document>(GRANTS)
        .update_one(
            doc! { "_id": &a[1].id },
            doc! { "$set": { "remaining": Credits::from_pico(1).unwrap() } },
        )
        .await
        .unwrap();
    let replay = grants::issue_grants(&db, policy.clone()).await.unwrap();
    assert_eq!(replay.len(), 2);
    assert!(replay.iter().all(|g| g.recipient_user_id != "late"));
    assert_eq!(
        replay.iter().find(|g| g.id == a[1].id).unwrap().remaining,
        Credits::from_pico(1).unwrap()
    );
    assert_eq!(
        db.collection::<Document>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    let mut conflict = policy.clone();
    conflict.amount = Some("1".parse().unwrap());
    assert!(matches!(
        grants::issue_grants(&db, conflict).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    let mut another_actor = policy;
    another_actor.granted_by = "other-admin".into();
    assert_eq!(
        grants::issue_grants(&db, another_actor)
            .await
            .unwrap()
            .len(),
        3
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn rules_reject_alias_overlap_unknown_and_contradictory_scope_and_bound_realizable() {
    let db = seed("allocation_policy_validation").await;
    let all = BillingServiceScope {
        all_services: true,
        ..Default::default()
    };
    assert!(
        service_limits::resolve(
            &db,
            &all,
            Credits::from_whole(1),
            &[rule(&["svc-a"], "0.2"), rule(&["service-a"], "0.3")]
        )
        .await
        .is_err()
    );
    let mut collision = test_auto_connected_catalog_service();
    collision.id = "collision".into();
    collision.slug = "svc-a".into();
    db.collection("downstream_services")
        .insert_one(collision)
        .await
        .unwrap();
    assert!(
        grants::resolve_service_scope(&db, false, &["svc-a".into(), "missing".into()])
            .await
            .is_err()
    );
    let id_first = grants::resolve_service_scope(&db, false, &["svc-a".into()])
        .await
        .unwrap();
    assert_eq!(id_first.service_ids, ["svc-a"]);
    assert!(
        service_limits::resolve(
            &db,
            &all,
            Credits::from_whole(1),
            &[rule(&["unknown"], "0.2")]
        )
        .await
        .is_err()
    );
    let scoped = grants::resolve_service_scope(&db, false, &["svc-a".into()])
        .await
        .unwrap();
    assert!(
        service_limits::resolve(
            &db,
            &scoped,
            Credits::from_whole(1),
            &[rule(&["svc-b"], "0.2")]
        )
        .await
        .is_err()
    );
    let mut policy = input();
    policy.all_services = false;
    policy.service_refs = vec!["svc-a".into(), "svc-b".into()];
    let rows = grants::issue_grants(&db, policy).await.unwrap();
    let summary = summaries::allocations(&db, None, Utc::now()).await.unwrap();
    assert_eq!(summary.remaining.to_string(), "0.00000000002");
    assert_eq!(summary.unreserved.to_string(), "0.000000000006");
    assert_eq!(summary.restricted, summary.unreserved);
    // Exact keys win over obsolete legacy aliases, absent policies default safe.
    let mut old = bson::to_document(&rows[0]).unwrap();
    old.remove("service_limits");
    old.remove("revoke_requested_at");
    old.remove("revoke_requested_by");
    old.insert("remaining_micros", 999_i64);
    let decoded: CreditGrant = bson::from_document(old.clone()).unwrap();
    assert_eq!(decoded.remaining, rows[0].remaining);
    assert!(decoded.service_limits.is_empty());
    assert!(decoded.revoke_requested_at.is_none());
    old.remove("remaining");
    old.insert("remaining_micros", 1_i64);
    assert_eq!(
        bson::from_document::<CreditGrant>(old).unwrap().remaining,
        Credits::from_micros(1)
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn recurring_exact_policy_snapshot_and_sparse_pause_survive_retired_catalog() {
    let db = seed("allocation_schedule_snapshot").await;
    let create = schedules::CreateScheduleInput {
        amount_credits: 0,
        amount: Some("0.3".parse().unwrap()),
        recurrence: ScheduleRecurrence::Monthly,
        expiry: CreditExpiryPolicy::Never,
        target_kind: BillingTargetKind::AllUsers,
        target_user_ids: vec![],
        target_org_ids: vec![],
        target_group_ids: vec![],
        all_services: true,
        service_refs: vec![],
        service_limits: vec![rule(&["svc-a"], "0.1")],
        reason: None,
        created_by: "admin".into(),
        request_id: Some(uuid::Uuid::new_v4().to_string()),
    };
    let schedule = schedules::create_schedule(&db, create.clone())
        .await
        .unwrap();
    assert_eq!(
        schedules::create_schedule(&db, create).await.unwrap().id,
        schedule.id
    );
    let now = Utc::now();
    schedules::disburse_due(&db, now, 5000).await.unwrap();
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(GRANTS)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(grants.len(), 2);
    assert!(
        grants.iter().all(
            |g| g.amount.to_string() == "0.3" && g.service_limits[0].limit.to_string() == "0.1"
        )
    );
    schedules::update_schedule(
        &db,
        &schedule.id,
        schedules::UpdateScheduleInput {
            amount: Some("0.4".parse().unwrap()),
            service_limits: Some(vec![rule(&["svc-a"], "0.2")]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db.collection::<Document>("downstream_services")
        .delete_many(doc! {})
        .await
        .unwrap();
    let paused = schedules::update_schedule(
        &db,
        &schedule.id,
        schedules::UpdateScheduleInput {
            is_active: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!paused.is_active);
    assert_eq!(paused.amount.to_string(), "0.4");
    assert_eq!(paused.amount_credits, 0);
    assert_eq!(paused.service_limits[0].limit.to_string(), "0.2");
    let frozen = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "_id": &grants[0].id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frozen.amount.to_string(), "0.3");
    assert_eq!(frozen.service_limits[0].limit.to_string(), "0.1");
    // Expiry, rather than reclaim, retains its own terminal amount and identity.
    db.collection::<Document>(GRANTS).update_one(doc! { "_id": &frozen.id }, doc! { "$set": { "expires_at": bson::DateTime::from_chrono(now - Duration::seconds(1)) } }).await.unwrap();
    assert_eq!(grants::expire_due_grants(&db, now).await.unwrap(), 1);
    let expired = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "_id": &frozen.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.terminal_amount, frozen.remaining);
    // The next window freezes edited policy and starts independent counters.
    schedules::update_schedule(
        &db,
        &schedule.id,
        schedules::UpdateScheduleInput {
            is_active: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    schedules::disburse_due(&db, now + Duration::days(40), 5000)
        .await
        .unwrap();
    let next = db.collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "schedule_origin.schedule_id": &schedule.id, "amount": "0.4".parse::<Credits>().unwrap() })
        .await.unwrap().unwrap();
    assert_ne!(next.schedule_origin, frozen.schedule_origin);
    assert_eq!(next.service_limits[0].limit.to_string(), "0.2");
    assert_eq!(next.service_limits[0].consumed, Credits::ZERO);
    assert_eq!(next.service_limits[0].reserved, Credits::ZERO);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn member_submission_replays_frozen_people_after_org_and_group_membership_changes() {
    use crate::models::{
        group::Group,
        org_membership::{OrgMembership, OrgRole},
    };
    use crate::test_utils::test_membership;
    for kind in [BillingTargetKind::OrgMembers, BillingTargetKind::Groups] {
        let db = seed("allocation_member_replay").await;
        db.collection::<User>("users")
            .insert_one(test_user("org", UserType::Org))
            .await
            .unwrap();
        let now = Utc::now();
        db.collection::<Group>("groups")
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
        db.collection::<OrgMembership>("org_memberships")
            .insert_one(test_membership("org", "person", OrgRole::Viewer, None))
            .await
            .unwrap();
        db.collection::<Document>("users")
            .update_one(
                doc! { "_id": "person" },
                doc! { "$set": { "group_ids": ["group"] } },
            )
            .await
            .unwrap();
        let mut policy = input();
        policy.target_kind = kind;
        if kind == BillingTargetKind::OrgMembers {
            policy.target_org_ids = vec!["org".into()];
        } else {
            policy.target_group_ids = vec!["group".into()];
        }
        let issued = grants::issue_grants(&db, policy.clone()).await.unwrap();
        assert_eq!(issued.len(), 1);
        assert_eq!(issued[0].recipient_user_id, "person");
        db.collection::<Document>("org_memberships")
            .delete_many(doc! {})
            .await
            .unwrap();
        db.collection::<OrgMembership>("org_memberships")
            .insert_one(test_membership("org", "other", OrgRole::Member, None))
            .await
            .unwrap();
        db.collection::<Document>("users")
            .update_one(
                doc! { "_id": "person" },
                doc! { "$set": { "group_ids": [] } },
            )
            .await
            .unwrap();
        db.collection::<Document>("users")
            .update_one(
                doc! { "_id": "other" },
                doc! { "$set": { "group_ids": ["group"] } },
            )
            .await
            .unwrap();
        grants::request_revoke_grant(&db, &issued[0].id, Some("admin"))
            .await
            .unwrap();
        let replay = grants::issue_grants(&db, policy).await.unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].id, issued[0].id);
        assert_eq!(replay[0].remaining, Credits::ZERO);
        assert_eq!(
            replay[0].status,
            crate::models::credit_grant::CreditGrantStatus::Revoked
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn legacy_policy_and_nested_allocation_bson_keep_exact_precedence_and_safe_defaults() {
    use crate::models::{
        credit_grant::CreditGrantSettlementLock,
        credit_schedule::CreditSchedule,
        credit_schedule_period::CreditSchedulePeriod,
        usage_meter::{GrantConsumptionAllocation, GrantReservationAllocation},
    };
    let db = seed("allocation_legacy_nested_bson").await;
    let now = bson::DateTime::now();
    let pico = Credits::from_pico(1).unwrap();
    let scope = doc! { "all_services": true, "service_ids": [], "service_slugs": [] };
    let documents = [
        (
            "credit_schedules",
            doc! { "_id": "old-schedule", "amount_credits": 1_i64, "amount_micros": 1_000_000_i64,
            "recurrence": "daily", "expiry": { "kind": "never" }, "target_kind": "all_users",
            "scope": &scope, "is_active": true, "created_by": "admin", "created_at": now, "updated_at": now },
        ),
        (
            "credit_schedule_periods",
            doc! { "_id": "old-period", "schedule_id": "old-schedule",
            "period_start": now, "period_end": now, "status": "complete", "amount_micros": 1_i64,
            "target_kind": "all_users", "scope": &scope, "created_at": now, "updated_at": now },
        ),
    ];
    for (collection, document) in documents {
        db.collection::<Document>(collection)
            .insert_one(document)
            .await
            .unwrap();
    }
    let old = db
        .collection::<CreditSchedule>("credit_schedules")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.amount, Credits::from_whole(1));
    assert!(old.service_limits.is_empty());
    let old = db
        .collection::<CreditSchedulePeriod>("credit_schedule_periods")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.amount, Credits::from_micros(1));
    assert!(old.service_limits.is_empty());
    for collection in ["credit_schedules", "credit_schedule_periods"] {
        db.collection::<Document>(collection)
            .update_one(
                doc! {},
                doc! { "$set": { "amount": pico, "amount_micros": 999_i64 } },
            )
            .await
            .unwrap();
    }
    assert_eq!(
        db.collection::<CreditSchedule>("credit_schedules")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .amount,
        pico
    );
    assert_eq!(
        db.collection::<CreditSchedulePeriod>("credit_schedule_periods")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .amount,
        pico
    );
    for exact in [false, true] {
        let mut allocation = doc! { "grant_id": "grant", "amount_micros": 1_i64 };
        if exact {
            allocation.insert("amount", pico);
        }
        let expected = if exact { pico } else { Credits::from_micros(1) };
        let reservation: GrantReservationAllocation =
            bson::from_document(allocation.clone()).unwrap();
        assert_eq!(reservation.amount, expected);
        assert!(reservation.service_limit_key.is_none());
        allocation.insert("operation_id", "operation");
        let consumed: GrantConsumptionAllocation = bson::from_document(allocation).unwrap();
        assert_eq!(consumed.amount, expected);
        assert!(consumed.service_limit_key.is_none());
        let mut lock = doc! { "operation_id": "op", "usage_row_id": "row", "reserved_micros": 1_i64,
        "consume_micros": 1_i64, "applied": false, "updated_at": now };
        if exact {
            lock.insert("reserved", pico);
            lock.insert("consume", pico);
        }
        let lock: CreditGrantSettlementLock = bson::from_document(lock).unwrap();
        assert_eq!(lock.consume, expected);
        assert_eq!(lock.reserved, expected);
        assert!(lock.service_limit_key.is_none());
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn takeover_lookup_indexes_install_on_existing_collections_and_use_ixscan() {
    let db = connect_transaction_test_database("allocation_recovery_indexes").await;
    for collection in [GRANTS, "usage_allowance_periods"] {
        let rows = (0..100).map(|i| doc! { "_id": format!("{collection}-{i}"), "allowance_id": format!("allowance-{i}"), "owner_user_id": "owner", "period_start": bson::DateTime::now() }).collect::<Vec<_>>();
        db.collection::<Document>(collection)
            .insert_many(rows)
            .await
            .unwrap();
        db.collection::<Document>(collection)
            .update_one(
                doc! { "_id": format!("{collection}-0") },
                doc! { "$set": { "active_settlement.usage_row_id": "recover-this" } },
            )
            .await
            .unwrap();
    }
    crate::db::ensure_indexes(&db).await.unwrap();
    crate::db::ensure_indexes(&db).await.unwrap();
    for (collection, name) in [
        (GRANTS, "credit_grants_settlement_usage"),
        (
            "usage_allowance_periods",
            "usage_allowance_periods_settlement_usage",
        ),
    ] {
        let indexes = db
            .collection::<Document>(collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        let index = indexes
            .iter()
            .find(|i| i.options.as_ref().and_then(|o| o.name.as_deref()) == Some(name))
            .unwrap();
        assert_eq!(index.options.as_ref().unwrap().sparse, Some(true));
        assert_ne!(index.options.as_ref().unwrap().unique, Some(true));
        let explain = db.run_command(doc! { "explain": { "find": collection, "filter": { "active_settlement.usage_row_id": "recover-this" } }, "verbosity": "queryPlanner" }).await.unwrap();
        let plan = explain
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap();
        let encoded = serde_json::to_string(plan).unwrap();
        assert!(
            encoded.contains("IXSCAN") && encoded.contains(name),
            "{encoded}"
        );
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            100
        );
    }
    db.drop().await.unwrap();
}
