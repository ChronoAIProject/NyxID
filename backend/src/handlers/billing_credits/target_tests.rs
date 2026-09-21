use super::*;
use crate::models::credit_grant::CreditGrantStatus;
use crate::models::service_billing::BillingMetric;
use serde_json::json;

#[test]
fn grant_and_allowance_target_dtos_preserve_new_fields_and_legacy_defaults() {
    for (kind, orgs, groups) in [
        ("all_users", vec![], vec![]),
        ("selected_users", vec![], vec![]),
        ("org_members", vec!["org"], vec![]),
        ("groups", vec![], vec!["group"]),
    ] {
        let target =
            json!({ "target_kind": kind, "target_org_ids": orgs, "target_group_ids": groups });
        let mut grant_json = target.clone();
        grant_json["amount_credits"] = json!(1);
        grant_json["all_services"] = json!(true);
        let grant: IssueGrantRequest = serde_json::from_value(grant_json).unwrap();
        assert!(grant.target_user_ids.is_empty());
        assert_eq!(grant.target_org_ids, orgs);
        assert_eq!(grant.target_group_ids, groups);
        let mut allowance_json = target.clone();
        allowance_json["quantity"] = json!(100);
        allowance_json["service_ref"] = json!("service");
        allowance_json["recurrence"] = json!("monthly");
        let request: CreateAllowanceRequest = serde_json::from_value(allowance_json).unwrap();
        let patch: UpdateAllowanceRequest = serde_json::from_value(target).unwrap();
        assert_eq!(patch.target_org_ids.as_ref(), Some(&request.target_org_ids));
        assert_eq!(
            patch.target_group_ids.as_ref(),
            Some(&request.target_group_ids)
        );
        let now = Utc::now();
        let allowance = UsageAllowance {
            bundle_id: None,
            id: "allowance".into(),
            service_id: "service".into(),
            service_slug: "service".into(),
            metric: BillingMetric::Requests,
            quantity: request.quantity.flatten().unwrap(),
            recurrence: request.recurrence.flatten().unwrap(),
            target_kind: request.target_kind,
            target_user_ids: request.target_user_ids,
            target_org_ids: request.target_org_ids,
            target_group_ids: request.target_group_ids,
            is_active: true,
            created_by: "admin".into(),
            created_at: now,
            updated_at: now,
        };
        let mut legacy = bson::to_document(&allowance).unwrap();
        legacy.remove("target_org_ids");
        legacy.remove("target_group_ids");
        let legacy: UsageAllowance = bson::from_document(legacy).unwrap();
        assert!(legacy.target_org_ids.is_empty());
        assert!(legacy.target_group_ids.is_empty());
        let response = serde_json::to_value(allowance_response(allowance)).unwrap();
        assert_eq!(response["target_org_ids"], json!(orgs));
        assert_eq!(response["target_group_ids"], json!(groups));
        let grant = CreditGrant {
            id: "grant".into(),
            batch_id: "batch".into(),
            schedule_origin: None,
            recipient_user_id: "person".into(),
            target_kind: grant.target_kind,
            target_org_ids: grant.target_org_ids,
            target_group_ids: grant.target_group_ids,
            amount_credits: 1,
            amount_micros: 1_000_000,
            remaining_micros: 1_000_000,
            reserved_micros: 0,
            scope: BillingServiceScope {
                all_services: true,
                ..Default::default()
            },
            expires_at: None,
            reason: None,
            granted_by: "admin".into(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: Some(now),
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now,
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        };
        let mut legacy = bson::to_document(&grant).unwrap();
        legacy.remove("target_org_ids");
        legacy.remove("target_group_ids");
        let legacy: CreditGrant = bson::from_document(legacy).unwrap();
        assert!(legacy.target_org_ids.is_empty());
        assert!(legacy.target_group_ids.is_empty());
        let response = serde_json::to_value(grant_response(grant.clone(), None, None)).unwrap();
        assert_eq!(response["target_kind"], kind);
        assert_eq!(response["target_org_ids"], json!(orgs));
        assert_eq!(response["target_group_ids"], json!(groups));
        let response = serde_json::to_value(IssueGrantResponse {
            batch_id: grant.batch_id,
            target_kind: grant.target_kind,
            target_org_ids: grant.target_org_ids,
            target_group_ids: grant.target_group_ids,
            created_count: 1,
            activated_count: 1,
            pending_activation_count: 0,
            recipients: vec![],
        })
        .unwrap();
        assert_eq!(response["target_kind"], kind);
        assert_eq!(response["target_org_ids"], json!(orgs));
        assert_eq!(response["target_group_ids"], json!(groups));
    }
    let legacy: IssueGrantRequest = serde_json::from_value(
        json!({ "amount_credits": 1, "all_services": true, "target_kind": "all_users" }),
    )
    .unwrap();
    assert!(legacy.target_org_ids.is_empty());
    assert!(legacy.target_group_ids.is_empty());
    let legacy: CreateAllowanceRequest = serde_json::from_value(json!({ "service_ref": "service", "quantity": 1, "recurrence": "daily", "target_kind": "all_users" })).unwrap();
    assert!(legacy.target_org_ids.is_empty());
    assert!(legacy.target_group_ids.is_empty());
    let patch: UpdateAllowanceRequest = serde_json::from_value(json!({ "quantity": 2 })).unwrap();
    assert!(patch.target_org_ids.is_none());
    assert!(patch.target_group_ids.is_none());
}
