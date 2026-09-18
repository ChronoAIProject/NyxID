use super::*;
use crate::models::billing_target::BillingTargetKind;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_allowance::{
    AllowanceRecurrence, COLLECTION_NAME as ALLOWANCES, UsageAllowance,
};
use crate::models::usage_meter::{CredentialClass, UsageStatus};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::test_utils::{connect_transaction_test_database, test_membership, test_user};

#[tokio::test]
async fn member_removal_blocks_new_funding_but_preserves_admitted_reservations() {
    for kind in [BillingTargetKind::OrgMembers, BillingTargetKind::Groups] {
        let db = connect_transaction_test_database("credit_member_funding").await;
        let mut person = test_user("person", UserType::Person);
        person.group_ids = vec!["group".into()];
        db.collection::<User>(USERS)
            .insert_many([person, test_user("org", UserType::Org)])
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership("org", "person", OrgRole::Viewer, None))
            .await
            .unwrap();
        let now = Utc::now();
        db.collection::<UsageAllowance>(ALLOWANCES)
            .insert_one(UsageAllowance {
                id: "allowance".into(),
                service_id: "service".into(),
                service_slug: "service".into(),
                metric: BillingMetric::Requests,
                quantity: 100,
                recurrence: AllowanceRecurrence::Monthly,
                target_kind: kind,
                target_user_ids: vec![],
                target_org_ids: if kind == BillingTargetKind::OrgMembers {
                    vec!["org".into()]
                } else {
                    vec![]
                },
                target_group_ids: if kind == BillingTargetKind::Groups {
                    vec!["group".into()]
                } else {
                    vec![]
                },
                is_active: true,
                created_by: "admin".into(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let allocations = reserve_allowances(
            &db,
            "person",
            Some("service"),
            None,
            BillingMetric::Requests,
            20,
        )
        .await
        .unwrap();
        assert_eq!(allocations.len(), 1);
        assert_eq!(allocations[0].quantity, 20);
        assert!(
            reserve_allowances(
                &db,
                "org",
                Some("service"),
                None,
                BillingMetric::Requests,
                20
            )
            .await
            .unwrap()
            .is_empty()
        );
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .update_many(
                doc! {},
                doc! { "$set": { "revoked_at": bson::DateTime::now() } },
            )
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": "person" },
                doc! { "$set": { "group_ids": [] } },
            )
            .await
            .unwrap();
        assert!(
            reserve_allowances(
                &db,
                "person",
                Some("service"),
                None,
                BillingMetric::Requests,
                20
            )
            .await
            .unwrap()
            .is_empty()
        );
        let row = UsageMeterRow {
            id: "row".into(),
            transaction_id: "transaction".into(),
            billing_request_id: "request".into(),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: "person".into(),
            wallet_id: Some("wallet".into()),
            actor_user_id: "person".into(),
            api_key_id: None,
            service_id: Some("service".into()),
            service_slug: Some("service".into()),
            metric: BillingMetric::Requests,
            lago_metric_code: "platform_service".into(),
            credential_class: CredentialClass::UserOwned,
            model: None,
            token_breakdown: None,
            reserved_credits: 0,
            funding: Some(UsageFunding {
                credits_per_unit_micros: CREDIT_MICROS,
                allowance_reservations: allocations.clone(),
                ..Default::default()
            }),
            quantity: Some(30),
            pending_resale_quantity: None,
            status: UsageStatus::Finalized,
            forwarded: true,
            released: false,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: Some(now),
            expires_at: None,
            last_error: None,
        };
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let result = settle_usage_funding(&db, &row).await.unwrap();
        assert_eq!(result.wallet_charge_credits, 10);
        assert_eq!(result.lago_billable_quantity_micros, 10_000_000);
        let period = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .find_one(doc! { "_id": &allocations[0].period_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(period.reserved_quantity, 0);
        assert_eq!(period.consumed_quantity, 20);
        assert_eq!(settle_usage_funding(&db, &row).await.unwrap(), result);
    }
}
