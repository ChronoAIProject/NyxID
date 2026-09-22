use super::*;
use serde_json::json;

#[test]
fn schedule_and_frozen_period_dtos_round_trip_target_provenance() {
    for (kind, orgs, groups) in [
        ("org_members", vec!["org"], vec![]),
        ("groups", vec![], vec!["group"]),
    ] {
        let request: CreateCreditScheduleRequest = serde_json::from_value(json!({ "amount_credits": 10, "recurrence": "monthly", "expiry": { "kind": "never" }, "target_kind": kind, "target_org_ids": orgs, "target_group_ids": groups, "all_services": true })).unwrap();
        let patch: UpdateCreditScheduleRequest = serde_json::from_value(
            json!({ "target_kind": kind, "target_org_ids": orgs, "target_group_ids": groups }),
        )
        .unwrap();
        assert_eq!(patch.target_org_ids.as_ref(), Some(&request.target_org_ids));
        assert_eq!(
            patch.target_group_ids.as_ref(),
            Some(&request.target_group_ids)
        );
        let now = Utc::now();
        let schedule = CreditSchedule {
            id: "schedule".into(),
            amount_credits: 10,
            amount_micros: 10_000_000,
            recurrence: request.recurrence,
            expiry: request.expiry,
            target_kind: request.target_kind,
            target_user_ids: request.target_user_ids,
            target_org_ids: request.target_org_ids,
            target_group_ids: request.target_group_ids,
            scope: BillingServiceScope {
                all_services: true,
                ..Default::default()
            },
            reason: None,
            is_active: true,
            created_by: "admin".into(),
            created_at: now,
            updated_at: now,
            last_period_start: None,
            last_disbursed_at: None,
            skipped_periods: 0,
        };
        let period = CreditSchedulePeriod {
            id: "period".into(),
            schedule_id: schedule.id.clone(),
            period_start: now,
            period_end: now + chrono::Duration::days(30),
            status: SchedulePeriodStatus::Disbursing,
            amount_micros: schedule.amount_micros,
            expires_at: None,
            target_kind: schedule.target_kind,
            target_user_ids: vec![],
            target_org_ids: schedule.target_org_ids.clone(),
            target_group_ids: schedule.target_group_ids.clone(),
            scope: schedule.scope.clone(),
            reason: None,
            cursor_user_id: None,
            disbursed_count: 0,
            lease_expires_at: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };
        let response = serde_json::to_value(schedule_response(
            schedule,
            Some(period),
            &HashMap::new(),
            &HashMap::new(),
        ))
        .unwrap();
        for value in [&response, &response["current_period"]] {
            assert_eq!(value["target_kind"], kind);
            assert_eq!(value["target_org_ids"], json!(orgs));
            assert_eq!(value["target_group_ids"], json!(groups));
        }
    }
    let legacy: CreateCreditScheduleRequest = serde_json::from_value(json!({ "amount_credits": 10, "recurrence": "daily", "expiry": { "kind": "never" }, "target_kind": "all_users", "all_services": true })).unwrap();
    assert!(legacy.target_org_ids.is_empty());
    assert!(legacy.target_group_ids.is_empty());
    let patch: UpdateCreditScheduleRequest =
        serde_json::from_value(json!({ "is_active": false })).unwrap();
    assert!(patch.target_org_ids.is_none());
    assert!(patch.target_group_ids.is_none());
}
