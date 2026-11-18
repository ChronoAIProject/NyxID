use super::*;
use crate::services::billing::DiagnosticSeverity;

fn sample(kind: &'static str, age: usize) -> KindScan {
    let group = || Group {
        rows: 7,
        held: Credits::from_whole(3_328_691),
        resources: (0..4).map(|n| format!("resource-{n}")).collect(),
        ..Default::default()
    };
    KindScan {
        kind,
        inventory: Ok(Inventory {
            rows: 7,
            groups: [(("forwarded".into(), age, "input_tokens".into()), group())].into(),
            totals: [("input_tokens".into(), group())].into(),
            ..Default::default()
        }),
        elapsed_ms: 0,
    }
}

fn render(scans: Vec<KindScan>) -> super::super::super::BillingStartupDiagnostic {
    describe(scans, DateTime::from_timestamp(0, 0).unwrap()).unwrap()
}

#[test]
fn integrity_hold_backlog_sentences_keep_counts_kinds_and_ages() {
    let diagnostic = render(vec![sample("allowance", 0)]);
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Info);
    assert_eq!(
        diagnostic.remediation,
        "No action is needed. This inventory is read-only."
    );
    assert_eq!(
        diagnostic.detail,
        "Allowance units are held in 4 periods across 7 unfinished billing rows. The oldest is under an hour old."
    );
    assert_eq!(diagnostic.summary, "Holds for unfinished requests");
    let technical = diagnostic.technical_details.unwrap();
    assert!(
        technical.contains("allowance total unit=input_tokens rows=7 held=3328691 resources=4")
    );
    assert!(technical.contains("status=forwarded age=under_1h"));
    assert!(!diagnostic.detail.contains('='));
}

#[test]
fn integrity_hold_backlog_age_never_changes_severity() {
    for (age, text) in [
        (0, "under an hour old"),
        (1, "between one hour and one day old"),
        (2, "between one and seven days old"),
        (3, "over seven days old"),
        (4, "timestamps are in the future"),
    ] {
        for kind in ["wallet", "grant", "allowance"] {
            let diagnostic = render(vec![sample(kind, age)]);
            assert_eq!(diagnostic.severity, DiagnosticSeverity::Info);
            assert_eq!(
                diagnostic.remediation,
                "No action is needed. This inventory is read-only."
            );
            assert!(diagnostic.detail.contains(text), "{}", diagnostic.detail);
        }
    }
}

#[test]
fn integrity_hold_backlog_invalid_and_truncated_scans_require_action() {
    let mut invalid = sample("wallet", 0);
    let inventory = invalid.inventory.as_mut().unwrap();
    inventory.rows += 2;
    inventory.invalid = 2;
    let diagnostic = render(vec![invalid]);
    assert_eq!(
        diagnostic.remediation,
        "Check the stored billing rows for invalid fields or incompatible schemas. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
    );
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Action);
    assert!(
        diagnostic
            .detail
            .starts_with("2 wallet rows could not be decoded.")
    );

    let mut truncated = sample("allowance", 0);
    truncated.inventory.as_mut().unwrap().truncated = true;
    let diagnostic = render(vec![truncated]);
    assert_eq!(
        diagnostic.remediation,
        "Check the row limit in Details and the size of the unfinished-request backlog. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
    );
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Action);
    assert!(
        diagnostic
            .detail
            .starts_with("The allowance scan stopped before all rows were read.")
    );
    assert!(
        diagnostic
            .detail
            .contains("Counts cover only decoded rows.")
    );
}

#[test]
fn integrity_hold_backlog_unreadable_scans_preserve_other_kinds() {
    for reason in [
        "deadline_exceeded",
        "query_timeout",
        "snapshot_unavailable",
        "index_unavailable",
        "database_error",
        "database_unavailable",
        "row_decode_error",
    ] {
        let diagnostic = render(vec![
            KindScan {
                kind: "wallet",
                inventory: Err(ReadFailure {
                    reason,
                    stage: "cursor",
                    database_code: None,
                }),
                elapsed_ms: 0,
            },
            sample("allowance", 2),
        ]);
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Action);
        let check = match reason {
            "deadline_exceeded" | "query_timeout" => {
                "query performance for the scan that reached its deadline"
            }
            "row_decode_error" => {
                "the stored billing rows for invalid fields or incompatible schemas"
            }
            _ => "database access and required indexes for the unreadable inventory",
        };
        assert_eq!(
            diagnostic.remediation,
            format!(
                "Check {check}. Check database health and server logs for the stages shown in Details, then reload this page to read the inventory again."
            )
        );
        assert!(
            diagnostic
                .detail
                .starts_with("The wallet inventory could not be read.")
        );
        assert!(
            diagnostic
                .detail
                .contains("Allowance units are held in 4 periods")
        );
        assert!(
            diagnostic
                .technical_details
                .unwrap()
                .contains(&format!("reason={reason}"))
        );
    }
}

#[test]
fn integrity_hold_backlog_empty_and_returned_holds_do_not_require_action() {
    assert!(
        describe(
            vec![KindScan {
                kind: "wallet",
                inventory: Ok(Inventory::default()),
                elapsed_ms: 0
            }],
            DateTime::from_timestamp(0, 0).unwrap()
        )
        .is_none()
    );
    let mut scan = sample("grant", 3);
    let inventory = scan.inventory.as_mut().unwrap();
    for group in inventory
        .groups
        .values_mut()
        .chain(inventory.totals.values_mut())
    {
        group.held = Credits::ZERO;
        group.resources.clear();
    }
    let diagnostic = render(vec![scan]);
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Info);
    assert_eq!(
        diagnostic.remediation,
        "No action is needed. This inventory is read-only."
    );
    assert_eq!(
        diagnostic.detail,
        "7 unfinished grant billing rows have no remaining holds."
    );
}

#[test]
fn integrity_hold_backlog_summary_is_bounded_with_all_findings() {
    let scans = ["wallet", "grant", "allowance"].map(|kind| {
        let mut scan = sample(kind, 3);
        let inventory = scan.inventory.as_mut().unwrap();
        inventory.rows += 10_000;
        inventory.invalid = 10_000;
        inventory.truncated = true;
        scan
    });
    let diagnostic = render(scans.into());
    assert!(diagnostic.detail.len() + diagnostic.remediation.len() < 2048);
    assert_eq!(diagnostic.remediation.matches("the row limit").count(), 1);
    assert_eq!(
        diagnostic
            .remediation
            .matches("stored billing rows")
            .count(),
        1
    );
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Action);
}
