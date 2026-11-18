use super::*;
use crate::services::billing::DiagnosticSeverity;

fn sample() -> Inventory {
    Inventory {
        registry_readable: true,
        holdless_lower_bound: Some(55_339),
        ..Default::default()
    }
}

fn add(
    inventory: &mut Inventory,
    request: &str,
    class: &str,
    eligible: bool,
    age: &str,
    kind: &str,
) {
    inventory.rows += 1;
    inventory.requests.insert(request.into());
    let group = inventory
        .groups
        .entry((class.into(), eligible, age.into(), String::new()))
        .or_default();
    group.rows += 1;
    group.requests.insert(request.into());
    let group = inventory
        .holds
        .entry((
            class.into(),
            eligible,
            age.into(),
            kind.into(),
            "output_tokens".into(),
            String::new(),
        ))
        .or_default();
    group.rows += 1;
    group.requests.insert(request.into());
}

#[test]
fn integrity_stuck_request_sentences_describe_eligibility_and_cleanup_without_a_deadline() {
    let mut inventory = sample();
    for n in 0..2 {
        add(
            &mut inventory,
            &format!("request-{n}"),
            "eligible_now",
            true,
            "under_1h",
            "allowance",
        );
    }
    let totals = RecoveryTotals {
        holdless_ended_rows: 4664,
        holdless_ended_requests: 4664,
        ..Default::default()
    };
    let (severity, sentence, remediation) = inventory.presentation(Some(&totals), None);
    assert_eq!(severity, DiagnosticSeverity::Info);
    assert_eq!(
        remediation,
        "No action is needed. This inventory is read-only."
    );
    assert!(
        sentence.starts_with(
            "2 requests have holds across 2 billing rows, including 2 with allowance units."
        ),
        "{sentence}"
    );
    assert!(sentence.contains("The oldest is under an hour old."));
    assert!(sentence.contains("The recovery sweep can release unused holds from eligible reservations in 2 requests after rechecking them."));
    assert!(sentence.contains(
        "At least 55,339 sent request rows hold no funds, so no money is tied up in them."
    ));
    assert!(sentence.contains("Cleanup has closed 4,664 such rows so far (4,664 requests)."));
    assert!(
        !sentence.contains("next pass")
            && !sentence.contains("dead requests")
            && !sentence.contains('=')
    );
}

#[test]
fn integrity_stuck_request_ages_and_overlapping_categories_are_informational() {
    for age in [
        "under_1h",
        "1h_to_24h",
        "1d_to_7d",
        "over_7d",
        "future_timestamp",
    ] {
        let mut inventory = sample();
        add(
            &mut inventory,
            "shared",
            "eligible_now",
            true,
            age,
            "wallet",
        );
        add(
            &mut inventory,
            "shared",
            "owned_by_live_generation",
            false,
            age,
            "allowance",
        );
        add(
            &mut inventory,
            "final",
            "money_protocol_in_progress",
            false,
            age,
            "grant",
        );
        add(
            &mut inventory,
            "voice",
            "voice_live_session",
            false,
            age,
            "wallet",
        );
        add(
            &mut inventory,
            "unknown",
            "legacy_may_be_live",
            false,
            age,
            "grant",
        );
        let (severity, sentence, remediation) =
            inventory.presentation(Some(&RecoveryTotals::default()), None);
        assert_eq!(severity, DiagnosticSeverity::Info);
        assert_eq!(
            remediation,
            "No action is needed. This inventory is read-only."
        );
        assert!(sentence.starts_with("4 requests have holds across 5 billing rows"));
        assert!(sentence.contains("The recovery sweep can release unused holds from eligible reservations in 1 request after rechecking them."));
        assert!(
            sentence.contains(
                "2 requests have reservations that belong to work that is still running."
            )
        );
        assert!(sentence.contains("1 request has reservations being finalized."));
        assert!(sentence.contains("1 request awaits ownership evidence or another recovery path."));
    }
}

#[test]
fn integrity_stuck_request_untrustworthy_inventory_requires_action() {
    let totals = RecoveryTotals::default();
    let mut truncated = sample();
    truncated.truncated = true;
    let mut invalid = sample();
    invalid.invalid = 2;
    let mut registry = sample();
    registry.registry_readable = false;
    let mut count_unreadable = sample();
    count_unreadable.holdless_lower_bound = None;
    for (inventory, expected, check) in [
        (
            truncated,
            "The hold scan is incomplete.",
            "the row limit in Details and the size of the unfinished-request backlog",
        ),
        (
            invalid,
            "2 invalid rows were found.",
            "the stored billing rows for invalid fields or incompatible schemas",
        ),
        (
            registry,
            "Process ownership evidence could not be read.",
            "the process registry for missing, invalid, or unreadable records",
        ),
        (
            count_unreadable,
            "The count of rows with no holds could not be read.",
            "the count query and required indexes for rows with no holds",
        ),
    ] {
        let (severity, sentence, remediation) = inventory.presentation(Some(&totals), None);
        assert_eq!(severity, DiagnosticSeverity::Action);
        assert!(sentence.starts_with(expected), "{sentence}");
        assert_eq!(
            remediation,
            format!(
                "Check {check}. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
            )
        );
    }
    let (severity, sentence, remediation) = sample().presentation(None, None);
    assert_eq!(severity, DiagnosticSeverity::Action);
    assert!(sentence.starts_with("Recovery totals could not be read."));
    assert!(!sentence.contains("Cleanup has closed"));
    assert_eq!(
        remediation,
        "Check the stored recovery counters for invalid or unreadable values. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
    );
}

#[test]
fn integrity_stuck_request_capped_holdless_backlog_is_still_information() {
    let mut inventory = sample();
    inventory.forwarded_count_capped = true;
    let (severity, sentence, remediation) =
        inventory.presentation(Some(&RecoveryTotals::default()), None);
    assert_eq!(severity, DiagnosticSeverity::Info);
    assert_eq!(
        remediation,
        "No action is needed. This inventory is read-only."
    );
    assert!(sentence.contains("At least 55,339"));
}

#[test]
fn integrity_stuck_request_recovery_totals_reject_unreadable_values() {
    assert_eq!(RecoveryTotals::read(None).unwrap().holdless_ended_rows, 0);
    for value in [Bson::String("invalid".into()), Bson::Int64(-1), Bson::Null] {
        let failure = RecoveryTotals::read(Some(doc! {"holdless_ended_rows":value}))
            .err()
            .unwrap();
        assert_eq!(failure.reason, "row_decode_error");
        assert_eq!(failure.stage, "ended_totals");
    }
}

#[test]
fn integrity_stuck_request_summary_is_bounded_with_all_findings() {
    let mut inventory = sample();
    for (index, (class, eligible)) in [
        ("eligible_now", true),
        ("owned_by_live_generation", false),
        ("money_protocol_in_progress", false),
        ("excluded_pool", false),
    ]
    .into_iter()
    .enumerate()
    {
        for kind in ["wallet", "grant", "allowance"] {
            add(
                &mut inventory,
                &format!("request-{index}-{kind}"),
                class,
                eligible,
                "over_7d",
                kind,
            );
        }
    }
    inventory.invalid = 10_000;
    inventory.truncated = true;
    inventory.registry_readable = false;
    let totals = RecoveryTotals {
        ended_requests: i64::MAX,
        ended_rows: i64::MAX,
        holdless_ended_requests: i64::MAX,
        holdless_ended_rows: i64::MAX,
    };
    let (_, sentence, remediation) = inventory.presentation(Some(&totals), None);
    assert!(
        sentence.len() + remediation.len() < 2048,
        "{}: {sentence} {remediation}",
        sentence.len() + remediation.len()
    );
    inventory.holdless_lower_bound = None;
    let failure = ReadFailure {
        reason: "deadline_exceeded",
        stage: "inventory",
        database_code: None,
    };
    for totals in [Some(&totals), None] {
        let (_, sentence, remediation) = inventory.presentation(totals, Some(&failure));
        assert!(sentence.len() + remediation.len() < 2048);
        assert_eq!(remediation.matches("Check database health").count(), 1);
    }
}

#[test]
fn integrity_stuck_request_read_failures_name_the_operator_check() {
    for (reason, check) in [
        (
            "deadline_exceeded",
            "query performance for the scan that reached its deadline",
        ),
        (
            "query_timeout",
            "query performance for the scan that reached its deadline",
        ),
        (
            "database_error",
            "database access and required indexes for the unreadable inventory",
        ),
        (
            "index_unavailable",
            "database access and required indexes for the unreadable inventory",
        ),
        (
            "row_decode_error",
            "the stored billing rows for invalid fields or incompatible schemas",
        ),
    ] {
        let mut inventory = sample();
        inventory.truncated = true;
        let failure = ReadFailure {
            reason,
            stage: "inventory",
            database_code: None,
        };
        let (severity, _, remediation) =
            inventory.presentation(Some(&RecoveryTotals::default()), Some(&failure));
        assert_eq!(severity, DiagnosticSeverity::Action);
        assert_eq!(
            remediation,
            format!(
                "Check {check}. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
            )
        );
    }
}
