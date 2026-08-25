use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use mongodb::bson::doc;
use uuid::Uuid;

use crate::AppState;
use crate::errors::AppError;
use crate::handlers::billing_credits::{
    self, BillingBenefitsQuery, CreditGrantActivationState, GrantListQuery, IssueGrantRequest,
};
use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
use crate::models::credit_grant::{
    COLLECTION_NAME as CREDIT_GRANTS, CreditGrant, CreditGrantStatus,
};
use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_allowance::{
    AllowanceRecurrence, COLLECTION_NAME as USAGE_ALLOWANCES, UsageAllowance,
};
use crate::models::user::{COLLECTION_NAME as USERS, UserType};
use crate::services::billing::reconcile::{BillingReconciler, spawn_reconcile_worker};
use crate::services::feature_flag_service::{self, BILLING_FLAG_KEY, FlagTarget};
use crate::services::role_service;
use crate::test_utils::{
    connect_test_database, test_app_config, test_app_state, test_auth_user, test_membership,
    test_user,
};

struct BenefitFixture {
    state: AppState,
    platform_admin_id: String,
    org_id: String,
    org_admin_id: String,
    org_member_id: String,
    non_member_id: String,
    personal_id: String,
}

async fn setup_benefits(prefix: &str) -> Option<BenefitFixture> {
    let db = connect_test_database(prefix).await?;
    role_service::seed_system_roles(&db).await.ok()?;
    let role_ids = role_service::get_platform_role_ids(&db).await.ok()?;
    let platform_admin_id = Uuid::new_v4().to_string();
    let org_id = Uuid::new_v4().to_string();
    let org_admin_id = Uuid::new_v4().to_string();
    let org_member_id = Uuid::new_v4().to_string();
    let non_member_id = Uuid::new_v4().to_string();
    let personal_id = Uuid::new_v4().to_string();
    let mut platform_admin = test_user(&platform_admin_id, UserType::Person);
    platform_admin.role_ids.push(role_ids.admin);
    db.collection(USERS)
        .insert_many([
            platform_admin,
            test_user(&org_id, UserType::Org),
            test_user(&org_admin_id, UserType::Person),
            test_user(&org_member_id, UserType::Person),
            test_user(&non_member_id, UserType::Person),
            test_user(&personal_id, UserType::Person),
        ])
        .await
        .ok()?;
    db.collection(ORG_MEMBERSHIPS)
        .insert_many([
            test_membership(&org_id, &org_admin_id, OrgRole::Admin, None),
            test_membership(&org_id, &org_member_id, OrgRole::Member, None),
        ])
        .await
        .ok()?;

    Some(BenefitFixture {
        state: test_app_state(db),
        platform_admin_id,
        org_id,
        org_admin_id,
        org_member_id,
        non_member_id,
        personal_id,
    })
}

fn grant(recipient_user_id: &str, issued_ledgered_at: Option<DateTime<Utc>>) -> CreditGrant {
    let now = Utc::now();
    CreditGrant {
        id: Uuid::new_v4().to_string(),
        batch_id: Uuid::new_v4().to_string(),
        schedule_origin: None,
        recipient_user_id: recipient_user_id.to_string(),
        target_kind: BillingTargetKind::SelectedUsers,
        amount_credits: 5,
        amount_micros: 5_000_000,
        remaining_micros: 5_000_000,
        reserved_micros: 0,
        scope: BillingServiceScope {
            all_services: true,
            service_ids: Vec::new(),
            service_slugs: Vec::new(),
        },
        expires_at: None,
        reason: Some("visibility regression".to_string()),
        granted_by: "platform-admin".to_string(),
        status: CreditGrantStatus::Active,
        issued_ledgered_at,
        terminal_ledgered_at: None,
        terminal_amount_micros: 0,
        active_settlement: None,
        created_at: now,
        updated_at: now,
        consumed_at: None,
        expired_at: None,
        revoked_at: None,
    }
}

fn allowance() -> UsageAllowance {
    let now = Utc::now();
    UsageAllowance {
        id: Uuid::new_v4().to_string(),
        service_id: Uuid::new_v4().to_string(),
        service_slug: "visibility-service".to_string(),
        metric: BillingMetric::Requests,
        quantity: 100,
        recurrence: AllowanceRecurrence::Monthly,
        target_kind: BillingTargetKind::AllUsers,
        target_user_ids: Vec::new(),
        is_active: true,
        created_by: "platform-admin".to_string(),
        created_at: now,
        updated_at: now,
    }
}

async fn assert_benefits_visible(state: &AppState, actor_id: &str, owner_id: Option<&str>) {
    let query = BillingBenefitsQuery {
        owner_id: owner_id.map(ToString::to_string),
    };
    let Json(grants) = billing_credits::user_list_grants(
        State(state.clone()),
        test_auth_user(actor_id),
        Query(query),
    )
    .await
    .expect("authorized grant read");
    let Json(allowances) = billing_credits::user_list_allowances(
        State(state.clone()),
        test_auth_user(actor_id),
        Query(BillingBenefitsQuery {
            owner_id: owner_id.map(ToString::to_string),
        }),
    )
    .await
    .expect("authorized allowance read");

    assert_eq!(grants.grants.len(), 1);
    assert_eq!(grants.page, 1);
    assert_eq!(grants.per_page, 1);
    assert_eq!(grants.total, 1);
    assert_eq!(allowances.allowances.len(), 1);
}

#[tokio::test]
async fn benefit_reads_allow_personal_org_admin_and_org_member_but_reject_non_member() {
    let Some(fixture) = setup_benefits("grant_visibility_owner_reads").await else {
        return;
    };
    fixture
        .state
        .db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_many([
            grant(&fixture.org_id, Some(Utc::now())),
            grant(&fixture.personal_id, Some(Utc::now())),
        ])
        .await
        .expect("insert grants");
    fixture
        .state
        .db
        .collection::<UsageAllowance>(USAGE_ALLOWANCES)
        .insert_one(allowance())
        .await
        .expect("insert allowance");

    assert_benefits_visible(&fixture.state, &fixture.org_admin_id, Some(&fixture.org_id)).await;
    assert_benefits_visible(
        &fixture.state,
        &fixture.org_member_id,
        Some(&fixture.org_id),
    )
    .await;
    assert_benefits_visible(&fixture.state, &fixture.personal_id, None).await;

    let grant_error = billing_credits::user_list_grants(
        State(fixture.state.clone()),
        test_auth_user(&fixture.non_member_id),
        Query(BillingBenefitsQuery {
            owner_id: Some(fixture.org_id.clone()),
        }),
    )
    .await
    .expect_err("non-member grant read must fail");
    let allowance_error = billing_credits::user_list_allowances(
        State(fixture.state.clone()),
        test_auth_user(&fixture.non_member_id),
        Query(BillingBenefitsQuery {
            owner_id: Some(fixture.org_id),
        }),
    )
    .await
    .expect_err("non-member allowance read must fail");

    assert!(matches!(grant_error, AppError::Forbidden(_)));
    assert!(matches!(allowance_error, AppError::Forbidden(_)));
}

#[tokio::test]
async fn admin_grant_responses_expose_rollout_and_pending_activation() {
    let Some(fixture) = setup_benefits("grant_visibility_admin_signals").await else {
        return;
    };
    feature_flag_service::set_platform_override(
        &fixture.state.db,
        BILLING_FLAG_KEY,
        &FlagTarget::User(fixture.non_member_id.clone()),
        false,
        &fixture.platform_admin_id,
    )
    .await
    .expect("disable recipient billing rollout");
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));

    let Json(issued) = billing_credits::issue_grant(
        State(fixture.state.clone()),
        test_auth_user(&fixture.platform_admin_id),
        Json(IssueGrantRequest {
            amount_credits: 10,
            target_kind: BillingTargetKind::SelectedUsers,
            target_user_ids: vec![fixture.personal_id.clone(), fixture.non_member_id.clone()],
            all_services: true,
            service_refs: Vec::new(),
            expires_at: None,
            reason: Some("rollout signal".to_string()),
        }),
    )
    .await
    .expect("issue grant batch");

    assert_eq!(issued.created_count, 2);
    assert_eq!(issued.recipients.len(), 2);
    assert!(
        issued
            .recipients
            .iter()
            .find(|recipient| recipient.recipient_user_id == fixture.personal_id)
            .expect("enabled recipient")
            .recipient_billing_enabled
    );
    assert!(
        !issued
            .recipients
            .iter()
            .find(|recipient| recipient.recipient_user_id == fixture.non_member_id)
            .expect("disabled recipient")
            .recipient_billing_enabled
    );

    let pending = grant(&fixture.non_member_id, None);
    let pending_id = pending.id.clone();
    fixture
        .state
        .db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_one(pending)
        .await
        .expect("insert pending grant");
    let Json(listed) = billing_credits::admin_list_grants(
        State(fixture.state.clone()),
        test_auth_user(&fixture.platform_admin_id),
        Query(GrantListQuery {
            recipient_user_id: Some(fixture.non_member_id),
            page: Some(1),
            per_page: Some(50),
        }),
    )
    .await
    .expect("list recipient grants");

    assert_eq!(listed.page, 1);
    assert_eq!(listed.per_page, 50);
    assert_eq!(listed.total, 2);
    let pending = listed
        .grants
        .iter()
        .find(|grant| grant.id == pending_id)
        .expect("pending grant row");
    assert_eq!(pending.recipient_billing_enabled, Some(false));
    assert_eq!(
        pending.activation_state,
        CreditGrantActivationState::PendingActivation
    );

    fixture
        .state
        .db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_one(grant("deleted-recipient", Some(Utc::now())))
        .await
        .expect("insert orphaned historical grant");
    let Json(orphaned) = billing_credits::admin_list_grants(
        State(fixture.state),
        test_auth_user(&fixture.platform_admin_id),
        Query(GrantListQuery {
            recipient_user_id: Some("deleted-recipient".to_string()),
            page: Some(1),
            per_page: Some(50),
        }),
    )
    .await
    .expect("orphaned grant must remain visible");
    assert_eq!(orphaned.grants.len(), 1);
    assert_eq!(orphaned.grants[0].recipient_billing_enabled, Some(false));
}

#[tokio::test]
async fn reconcile_worker_activates_grants_without_lago_or_billing() {
    let Some(db) = connect_test_database("grant_visibility_no_lago_reconcile").await else {
        return;
    };
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    let pending = grant("recipient-no-lago", None);
    let pending_id = pending.id.clone();
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_one(pending)
        .await
        .expect("insert pending grant");

    let config = test_app_config();
    assert!(!config.billing_enabled);
    let reconciler = BillingReconciler::new(db.clone(), None, std::sync::Arc::new(config));
    let worker = spawn_reconcile_worker(reconciler, 3_600).expect("reconcile worker enabled");

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let row = db
                .collection::<CreditGrant>(CREDIT_GRANTS)
                .find_one(doc! { "_id": &pending_id })
                .await
                .expect("find pending grant")
                .expect("pending grant exists");
            if row.issued_ledgered_at.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("reconcile worker should activate the grant without Lago");

    worker.abort();
    assert!(
        worker
            .await
            .expect_err("aborted reconcile worker should stop")
            .is_cancelled()
    );
}
