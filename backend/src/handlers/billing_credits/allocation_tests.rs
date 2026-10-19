use super::*;
use crate::models::{credit_grant::COLLECTION_NAME as GRANTS, credits::Credits};
use crate::test_utils::test_auth_user;
use mongodb::bson::Document;

fn body(owner: &str) -> IssueGrantRequest {
    serde_json::from_value(serde_json::json!({"amount_credits":0,"amount":"0.3","target_kind":"selected_users","target_user_ids":[owner],"all_services":true,"request_id":uuid::Uuid::new_v4().to_string()})).unwrap()
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
