use super::super::assistant_cma_gateway::{CmaEvent, CmaEventSnapshot};
use super::*;

fn event(
    sequence: i64,
    kind: &str,
    response: Option<&str>,
    turn: Option<i64>,
    approval: Option<&str>,
) -> CmaEvent {
    serde_json::from_value(serde_json::json!({
        "sequence":sequence, "type":kind, "response_id":response,
        "turn_sequence":turn, "approval_id":approval
    }))
    .unwrap()
}

fn page(events: Vec<CmaEvent>, next: Option<&str>) -> CmaEventSnapshot {
    CmaEventSnapshot {
        events,
        next_cursor: next.map(str::to_owned),
        through_cursor: Some("head".into()),
        incarnation: Some("epoch1".into()),
    }
}

#[test]
fn ordered_approval_inherits_only_the_bound_turn_across_pages() {
    let mut scan = prepare_approval_scan(None, 25, 38);
    apply_approval_page(
        &mut scan,
        "resp_one",
        &page(
            vec![
                event(23, "approval_requested", None, None, Some("unbound")),
                event(25, "turn_started", Some("resp_one"), Some(25), None),
            ],
            Some("page2"),
        ),
    )
    .unwrap();
    apply_approval_page(
        &mut scan,
        "resp_one",
        &page(
            vec![
                event(33, "approval_requested", None, None, Some("write")),
                event(36, "approval_resolved", None, None, Some("write")),
                event(44, "approval_requested", None, None, Some("readback")),
            ],
            None,
        ),
    )
    .unwrap();
    assert_eq!(scan.pending.len(), 1);
    assert_eq!(scan.pending[0].approval_id, "readback");
}

#[test]
fn ordered_approval_rejects_closed_unrelated_and_malformed_turns() {
    for boundary in [
        "turn_completed",
        "turn_failed",
        "turn_interrupted",
        "turn_started",
    ] {
        let mut scan = prepare_approval_scan(None, 25, 38);
        apply_approval_page(
            &mut scan,
            "resp_one",
            &page(
                vec![
                    event(25, "turn_started", Some("resp_one"), Some(25), None),
                    event(33, "approval_requested", None, None, Some("old")),
                    event(36, boundary, Some("resp_other"), Some(36), None),
                    event(44, "approval_requested", None, None, Some("unrelated")),
                ],
                None,
            ),
        )
        .unwrap();
        assert!(scan.pending.is_empty(), "{boundary}");
    }
    let mut scan = prepare_approval_scan(None, 25, 38);
    apply_approval_page(
        &mut scan,
        "resp_one",
        &page(
            vec![
                event(24, "turn_started", Some("resp_one"), Some(25), None),
                event(33, "approval_requested", None, None, Some("malformed")),
            ],
            None,
        ),
    )
    .unwrap();
    assert!(scan.pending.is_empty());
}

#[test]
fn ordered_approval_rejects_reordered_history_and_incarnation_changes() {
    let mut scan = prepare_approval_scan(None, 25, 38);
    let first = page(
        vec![event(25, "turn_started", Some("resp_one"), Some(25), None)],
        Some("p2"),
    );
    apply_approval_page(&mut scan, "resp_one", &first).unwrap();
    assert!(
        apply_approval_page(
            &mut scan.clone(),
            "resp_one",
            &page(
                vec![event(24, "approval_requested", None, None, Some("stale"))],
                None
            )
        )
        .is_err()
    );
    let mut changed = page(Vec::new(), None);
    changed.incarnation = Some("epoch2".into());
    assert!(apply_approval_page(&mut scan, "resp_one", &changed).is_err());
}

#[test]
fn ordered_approval_rescan_preserves_notification_without_inheriting_old_turns() {
    let snapshot = page(
        vec![
            event(1, "turn_started", Some("resp_old"), Some(1), None),
            event(4, "turn_completed", None, Some(1), None),
            event(25, "turn_started", Some("resp_one"), Some(25), None),
            event(33, "approval_requested", None, None, Some("write")),
        ],
        None,
    );
    let mut scan = prepare_approval_scan(None, 25, 38);
    apply_approval_page(&mut scan, "resp_one", &snapshot).unwrap();
    assert_eq!(scan.pending.len(), 1);
    scan.pending[0].notified = true;
    let mut refreshed = prepare_approval_scan(Some(&scan), 25, 39);
    apply_approval_page(&mut refreshed, "resp_one", &snapshot).unwrap();
    assert_eq!(refreshed.pending.len(), 1);
    assert!(refreshed.pending[0].notified);
}

#[test]
fn ordered_approval_restarts_legacy_scans_without_an_ordered_anchor() {
    for complete in [false, true] {
        let legacy = serde_json::from_value(serde_json::json!({
            "revision":38,"turn_sequence":25,"incarnation":"epoch1",
            "cursor": if complete {None} else {Some("page2")},
            "complete":complete,"pending":[{"approval_id":"write","notified":true}]
        }))
        .unwrap();
        let scan = prepare_approval_scan(Some(&legacy), 25, 38);
        assert!(!scan.complete);
        assert!(scan.cursor.is_none());
        assert!(scan.pending[0].notified);
    }
}
