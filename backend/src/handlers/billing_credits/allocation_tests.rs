use super::*;
use crate::models::{credit_grant::COLLECTION_NAME as GRANTS, credits::Credits};
use crate::test_utils::test_auth_user;
use mongodb::bson::Document;

fn body(owner: &str) -> IssueGrantRequest {
    serde_json::from_value(serde_json::json!({"amount":"0.3","target_kind":"selected_users","target_user_ids":[owner],"all_services":true,"request_id":uuid::Uuid::new_v4().to_string()})).unwrap()
}

#[tokio::test]
async fn legacy_missing_grant_revoke_keeps_bad_request_contract_with_either_capability() {
    let (mut state, admin, _, _) = super::tests::setup("allocation_missing_revoke")
        .await
        .expect("real Mongo required");
    for enabled in [false, true] {
        state.config.billing_allocations_enabled = enabled;
        let error = revoke_grant(
            State(state.clone()),
            test_auth_user(&admin),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, AppError::BadRequest(message) if message == "credit grant is not active or has an in-flight settlement"),
            "unexpected legacy revoke response: {error:?}"
        );
    }
    for collection in [GRANTS, "billing_ledger", "usage_meter"] {
        assert_eq!(
            state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_grant_amount_bounds_keep_validation_contract_and_exact_precedence() {
    let (state, admin, _, owner) = super::tests::setup("allocation_legacy_grants")
        .await
        .expect("real Mongo required");
    let message = format!(
        "amount_credits must be between 1 and {}",
        billing::grants::MAX_GRANT_CREDITS
    );
    for amount in [-1, 0, billing::grants::MAX_GRANT_CREDITS + 1] {
        let request: IssueGrantRequest = serde_json::from_value(serde_json::json!({
            "amount_credits":amount,"target_kind":"selected_users",
            "target_user_ids":[owner],"all_services":true,
        }))
        .unwrap();
        let error = issue_grant(State(state.clone()), test_auth_user(&admin), Json(request))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::ValidationError(actual) if actual == &message),
            "unexpected legacy issue response: {error:?}"
        );
    }
    for collection in [GRANTS, "billing_ledger", "credit_allocation_requests"] {
        assert_eq!(
            state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    let request: IssueGrantRequest = serde_json::from_value(serde_json::json!({
        "amount":"0.3", "amount_credits":-1,"target_kind":"selected_users",
        "target_user_ids":[owner],"all_services":true,
    }))
    .unwrap();
    let Json(response) = issue_grant(State(state.clone()), test_auth_user(&admin), Json(request))
        .await
        .unwrap();
    assert_eq!(response.created_count, 1);
    assert_eq!(response.activated_count, 1);
    let stored = state
        .db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {"batch_id": &response.batch_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.amount.to_string(), "0.3");
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_schedule_amount_errors_and_sparse_exact_updates_keep_existing_contract() {
    use crate::handlers::billing_credit_schedules::{
        CreateCreditScheduleRequest, UpdateCreditScheduleRequest, allocate_recurring,
        create_schedule, update_schedule,
    };
    let (mut state, admin, _, owner) = super::tests::setup("allocation_legacy_amounts")
        .await
        .expect("real Mongo required");
    let legacy_message = format!(
        "amount_credits must be between 1 and {}",
        billing::grants::MAX_GRANT_CREDITS
    );
    let payload = serde_json::json!({
        "target_kind":"selected_users", "target_user_ids":[owner],
        "all_services":true, "recurrence":"monthly", "expiry":{"kind":"never"},
    });
    for amount in [-1, 0, billing::grants::MAX_GRANT_CREDITS + 1] {
        let mut invalid = payload.clone();
        invalid["amount_credits"] = serde_json::json!(amount);
        let request: CreateCreditScheduleRequest = serde_json::from_value(invalid).unwrap();
        let error = create_schedule(State(state.clone()), test_auth_user(&admin), Json(request))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::ValidationError(message) if message == &legacy_message),
            "unexpected legacy create response: {error:?}"
        );
    }
    state.config.billing_allocations_enabled = true;
    let mut exact = payload;
    exact["amount"] = serde_json::json!("0.3");
    let request: CreateCreditScheduleRequest = serde_json::from_value(exact).unwrap();
    let Json(schedule) =
        allocate_recurring(State(state.clone()), test_auth_user(&admin), Json(request))
            .await
            .unwrap();
    for amount in [-1, 0, billing::grants::MAX_GRANT_CREDITS + 1] {
        let request: UpdateCreditScheduleRequest =
            serde_json::from_value(serde_json::json!({"amount_credits":amount})).unwrap();
        let error = update_schedule(
            State(state.clone()),
            test_auth_user(&admin),
            Path(schedule.id.clone()),
            Json(request),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, AppError::ValidationError(message) if message == &legacy_message),
            "unexpected legacy update response: {error:?}"
        );
    }
    let request: UpdateCreditScheduleRequest =
        serde_json::from_value(serde_json::json!({"amount":"0", "amount_credits":7})).unwrap();
    let error = update_schedule(
        State(state.clone()),
        test_auth_user(&admin),
        Path(schedule.id.clone()),
        Json(request),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&error, AppError::ValidationError(message) if message == "Allocation amount must be positive and at most 1000000 credits"),
        "unexpected exact update response: {error:?}"
    );
    // Sparse legacy edits preserve an exact stored amount, even when its whole
    // compatibility projection is zero and new authoring has been disabled.
    state.config.billing_allocations_enabled = false;
    let Json(paused) = update_schedule(
        State(state.clone()),
        test_auth_user(&admin),
        Path(schedule.id.clone()),
        Json(UpdateCreditScheduleRequest {
            is_active: Some(false),
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    assert!(!paused.is_active);
    assert_eq!(paused.amount, "0.3");
    assert_eq!(paused.amount_credits, 0);
    let stored = state
        .db
        .collection::<crate::models::credit_schedule::CreditSchedule>("credit_schedules")
        .find_one(doc! {"_id":&schedule.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.amount.to_string(), "0.3");
    assert!(!stored.is_active);
    assert_eq!(
        state
            .db
            .collection::<Document>("credit_schedules")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_only_and_legacy_create_amounts_deserialize_and_reach_both_handlers() {
    use crate::handlers::billing_credit_schedules::{
        CreateCreditScheduleRequest, allocate_recurring,
    };
    let (mut state, admin, _, owner) = super::tests::setup("allocation_create_amounts")
        .await
        .expect("real Mongo required");
    state.config.billing_allocations_enabled = true;
    for (fields, expected) in [
        (serde_json::json!({"amount":"0.3"}), "0.3"),
        (
            serde_json::json!({"amount":"0.000000000001"}),
            "0.000000000001",
        ),
        (serde_json::json!({"amount_credits":7}), "7"),
        (
            serde_json::json!({"amount":"0.3", "amount_credits":99}),
            "0.3",
        ),
    ] {
        let mut payload = serde_json::json!({
            "target_kind":"selected_users", "target_user_ids":[owner],
            "all_services":true,
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let request: IssueGrantRequest = serde_json::from_value(payload.clone()).unwrap();
        let Json(response) = allocate(State(state.clone()), test_auth_user(&admin), Json(request))
            .await
            .unwrap();
        assert_eq!(response.created_count, 1);
        assert_eq!(response.activated_count, 1);
        let grant = state
            .db
            .collection::<CreditGrant>(GRANTS)
            .find_one(doc! { "batch_id": &response.batch_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grant.amount.to_string(), expected);
        assert_eq!(grant.remaining, grant.amount);

        payload["recurrence"] = serde_json::json!("monthly");
        payload["expiry"] = serde_json::json!({"kind":"never"});
        let request: CreateCreditScheduleRequest = serde_json::from_value(payload).unwrap();
        let Json(response) =
            allocate_recurring(State(state.clone()), test_auth_user(&admin), Json(request))
                .await
                .unwrap();
        assert_eq!(response.amount, expected);
        let stored = state
            .db
            .collection::<crate::models::credit_schedule::CreditSchedule>("credit_schedules")
            .find_one(doc! { "_id": &response.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.amount.to_string(), expected);
    }
    assert_eq!(
        state
            .db
            .collection::<Document>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("credit_schedules")
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_nonpositive_and_malformed_create_amounts_never_write_money_or_schedules() {
    use crate::handlers::billing_credit_schedules::{
        CreateCreditScheduleRequest, allocate_recurring,
    };
    let (mut state, admin, _, owner) = super::tests::setup("allocation_invalid_amounts")
        .await
        .expect("real Mongo required");
    state.config.billing_allocations_enabled = true;
    for fields in [
        serde_json::json!({}),
        serde_json::json!({"amount":null}),
        serde_json::json!({"amount":"0"}),
        serde_json::json!({"amount":"-0.3"}),
        serde_json::json!({"amount_credits":0}),
        serde_json::json!({"amount_credits":-1}),
        serde_json::json!({"amount":"0", "amount_credits":5}),
        serde_json::json!({"amount":"-1", "amount_credits":5}),
        serde_json::json!({"amount":"garbage"}),
        serde_json::json!({"amount":"0.0000000000001"}),
    ] {
        let mut payload = serde_json::json!({
            "target_kind":"selected_users", "target_user_ids":[owner], "all_services":true,
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        match serde_json::from_value::<IssueGrantRequest>(payload.clone()) {
            Ok(request) => assert!(
                matches!(
                    allocate(State(state.clone()), test_auth_user(&admin), Json(request)).await,
                    Err(AppError::ValidationError(_))
                ),
                "accepted invalid allocation: {payload}"
            ),
            Err(_) => assert!(
                fields
                    .get("amount")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            ),
        }
        payload["recurrence"] = serde_json::json!("daily");
        payload["expiry"] = serde_json::json!({"kind":"end_of_period"});
        match serde_json::from_value::<CreateCreditScheduleRequest>(payload.clone()) {
            Ok(request) => assert!(
                matches!(
                    allocate_recurring(State(state.clone()), test_auth_user(&admin), Json(request))
                        .await,
                    Err(AppError::ValidationError(_))
                ),
                "accepted invalid recurring allocation: {payload}"
            ),
            Err(_) => assert!(
                fields
                    .get("amount")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            ),
        }
    }
    for collection in [
        GRANTS,
        "credit_schedules",
        "credit_allocation_requests",
        "billing_ledger",
    ] {
        assert_eq!(
            state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "writes to {collection}"
        );
    }
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn disabled_legacy_revoke_never_marks_pending_and_enabled_reclaim_waits_for_holds() {
    let (mut state, admin, _, owner) = super::tests::setup("allocation_rollout_reclaim")
        .await
        .expect("real Mongo required");
    let Json(issued) = issue_grant(
        State(state.clone()),
        test_auth_user(&admin),
        Json(body(&owner)),
    )
    .await
    .unwrap();
    assert_eq!(issued.created_count, 1);
    let grant = state
        .db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<Document>(GRANTS)
        .update_one(
            doc! { "_id": &grant.id },
            doc! { "$set": { "reserved": Credits::from_pico(1).unwrap() } },
        )
        .await
        .unwrap();
    assert!(
        revoke_grant(
            State(state.clone()),
            test_auth_user(&admin),
            Path(grant.id.clone())
        )
        .await
        .is_err()
    );
    assert!(
        reclaim(
            State(state.clone()),
            test_auth_user(&admin),
            Path(grant.id.clone())
        )
        .await
        .is_err()
    );
    assert!(
        state
            .db
            .collection::<CreditGrant>(GRANTS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .revoke_requested_at
            .is_none()
    );
    state.config.billing_allocations_enabled = true;
    let Json(response) = reclaim(
        State(state.clone()),
        test_auth_user(&admin),
        Path(grant.id.clone()),
    )
    .await
    .unwrap();
    assert!(response.revoke_requested_at.is_some());
    assert_eq!(response.held, "0.000000000001");
    assert_eq!(response.remaining, "0.3");
    let Json(replay) = reclaim(
        State(state.clone()),
        test_auth_user(&admin),
        Path(grant.id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(response.revoke_requested_at, replay.revoke_requested_at);
    assert!(
        billing::grants::list_active_for_user(&state.db, &owner, Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    let Json(summary) = allocation_summary(State(state.clone()), test_auth_user(&admin))
        .await
        .unwrap();
    assert_eq!(summary.summary.unreserved, Credits::ZERO);
    assert_eq!(summary.summary.held.to_string(), "0.000000000001");
    // Summary reads preserve financial state; audit writes are allowed.
    for collection in ["usage_allowance_periods", "billing_wallet", "usage_meter"] {
        assert_eq!(
            state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn allocation_routes_retain_admin_manager_operator_and_user_authority() {
    let (mut state, admin, operator, user) = super::tests::setup("allocation_roles")
        .await
        .expect("real Mongo required");
    state.config.billing_allocations_enabled = true;
    for actor in [&operator, &user] {
        assert!(matches!(
            allocate(
                State(state.clone()),
                test_auth_user(actor),
                Json(body(&user))
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    let manager = state
        .db
        .collection::<crate::models::role::Role>("roles")
        .find_one(doc! { "slug": crate::models::role::CREDITS_MANAGER_ROLE_SLUG })
        .await
        .unwrap()
        .unwrap();
    for actor in [&operator, &user] {
        crate::services::role_service::assign_role_to_user(&state.db, actor, &manager.id)
            .await
            .unwrap();
    }
    let Json(issued) = allocate(
        State(state.clone()),
        test_auth_user(&operator),
        Json(body(&user)),
    )
    .await
    .unwrap();
    assert_eq!(issued.created_count, 1);
    assert!(matches!(
        allocate(
            State(state.clone()),
            test_auth_user(&user),
            Json(body(&user))
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(
        allocation_summary(State(state.clone()), test_auth_user(&operator))
            .await
            .is_ok()
    );
    assert!(matches!(
        allocation_summary(State(state.clone()), test_auth_user(&user)).await,
        Err(AppError::Forbidden(_))
    ));
    let grant = state
        .db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let Json(reclaimed) = reclaim(
        State(state.clone()),
        test_auth_user(&operator),
        Path(grant.id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        reclaimed.status,
        crate::models::credit_grant::CreditGrantStatus::Revoked
    );
    // The same operator's allocation authority does not permit recurring writes.
    let schedule = serde_json::from_value(serde_json::json!({"amount_credits":0,"amount":"0.3","target_kind":"all_users","all_services":true,"recurrence":"monthly","expiry":{"kind":"never"}})).unwrap();
    assert!(matches!(
        super::super::billing_credit_schedules::allocate_recurring(
            State(state.clone()),
            test_auth_user(&operator),
            Json(schedule)
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(
        allocation_summary(State(state.clone()), test_auth_user(&admin))
            .await
            .is_ok()
    );
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn response_preserves_raw_rule_reservations_and_exposes_exact_extra_commitment() {
    let (state, admin, _, owner) = super::tests::setup("allocation_response_held")
        .await
        .expect("real Mongo required");
    let Json(issued) = issue_grant(
        State(state.clone()),
        test_auth_user(&admin),
        Json(body(&owner)),
    )
    .await
    .unwrap();
    assert_eq!(issued.created_count, 1);
    let mut grant = state
        .db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    grant.service_limits = vec![crate::models::credit_service_limit::CreditServiceLimit {
        key: "a".into(),
        service_ids: vec!["a".into()],
        service_slugs: vec!["a".into()],
        label: None,
        limit: "0.1".parse().unwrap(),
        consumed: Credits::ZERO,
        reserved: Credits::ZERO,
    }];
    let mut other = grant.service_limits[0].clone();
    other.key = "b".into();
    other.service_ids = vec!["b".into()];
    other.service_slugs = vec!["b".into()];
    grant.service_limits.push(other);
    grant.active_settlement = Some(crate::models::credit_grant::CreditGrantSettlementLock {
        operation_id: "extra".into(),
        usage_row_id: "row".into(),
        service_limit_key: Some("a".into()),
        reserved: Credits::ZERO,
        consume: Credits::from_pico(1).unwrap(),
        applied: false,
        updated_at: Utc::now(),
    });
    let response = grant_response(grant.clone(), None, None).unwrap();
    assert_eq!(response.reserved, "0");
    assert_eq!(response.held, "0.000000000001");
    assert_eq!(response.service_limits[0].reserved, "0");
    assert_eq!(response.service_limits[0].held, "0.000000000001");
    assert_eq!(response.service_limits[1].held, "0");
    grant.active_settlement.as_mut().unwrap().applied = true;
    let response = grant_response(grant, None, None).unwrap();
    assert_eq!(response.held, "0");
    assert_eq!(response.service_limits[0].held, "0");
    state.db.drop().await.unwrap();
}
