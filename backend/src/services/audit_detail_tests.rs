use super::*;
use crate::{
    models::{
        assistant_settings::AccessMode,
        audit_outbox::{AuditApprovalMode, ResourceSetKind, SecurityFlag, SystemAuditActor},
        org_membership::OrgRole,
        service_approval_config::ApprovalMode,
    },
    test_utils::connect_transaction_test_database,
};
use chrono::{TimeZone, Utc};

fn event() -> AuditEvent {
    AuditEvent {
        operation_id: "00000000-0000-4000-8000-000000000001".into(),
        stage: AuditStage::Committed,
        principal: AuditPrincipal::System {
            actor: SystemAuditActor::Maintenance,
        },
        owner: AuditOwner::Platform,
        action: "audit_detail_test",
        target_id: None,
    }
}

fn access(scope: AccessModeScope, old: Option<AccessMode>, new: Option<AccessMode>) -> AuditDetail {
    AuditDetail::AccessModeChange { scope, old, new }
}

fn valid_details() -> Vec<AuditDetail> {
    vec![
        access(
            AccessModeScope::Workspace,
            Some(AccessMode::AskEveryTime),
            Some(AccessMode::Autopilot),
        ),
        access(AccessModeScope::Agent, Some(AccessMode::Nyxmode), None),
        access(AccessModeScope::Agent, None, Some(AccessMode::Nyxmode)),
        AuditDetail::OrgRoleChange {
            old: OrgRole::Viewer,
            new: OrgRole::Admin,
        },
        AuditDetail::ApprovalModeChange {
            old: AuditApprovalMode::WritesOnly,
            new: AuditApprovalMode::Off,
        },
        AuditDetail::ServiceApprovalModeChange {
            old: ApprovalMode::PerRequest,
            new: ApprovalMode::Grant,
        },
        AuditDetail::SecurityFlagChange {
            flag: SecurityFlag::MfaEnabled,
            old: false,
            new: true,
        },
        AuditDetail::ScopeSetChange {
            added: vec![scope_ident("account:read").unwrap()],
            removed: vec![scope_ident("openid").unwrap()],
            truncated: false,
        },
        AuditDetail::ResourceSetChange {
            kind: ResourceSetKind::Services,
            added: ids(0, 32),
            removed: vec![],
            truncated: true,
        },
        AuditDetail::ExpiryChange {
            old: None,
            new: Some(Utc.timestamp_millis_opt(1_800_000_000_000).unwrap()),
        },
        session_revocation_count(0),
        session_revocation_count(MAX_SESSION_REVOCATION_COUNT),
        session_revocation_count(u64::MAX),
    ]
}

fn ids(start: u64, count: u64) -> Vec<String> {
    (start..start + count)
        .map(|n| format!("00000000-0000-4000-8000-{n:012x}"))
        .collect()
}

#[test]
fn audit_detail_all_variants_validate_and_round_trip_json_and_bson_at_bounds() {
    for detail in valid_details() {
        let projected = event().with_detail(detail.clone()).project();
        validate(&projected).unwrap();
        let json = serde_json::to_vec(&projected).unwrap();
        assert_eq!(
            serde_json::from_slice::<SafeAuditEvent>(&json).unwrap(),
            projected
        );
        let bson = bson::to_document(&projected).unwrap();
        assert_eq!(
            bson::from_document::<SafeAuditEvent>(bson).unwrap(),
            projected
        );
        assert_eq!(projected.detail, Some(detail));
    }
}

#[test]
fn audit_detail_access_override_preserves_inheritance_and_rejects_nonchanges() {
    for scope in [AccessModeScope::Workspace, AccessModeScope::Agent] {
        for mode in [
            None,
            Some(AccessMode::Autopilot),
            Some(AccessMode::Nyxmode),
            Some(AccessMode::AskEveryTime),
        ] {
            assert!(validate_detail(&access(scope, mode, mode)).is_err());
        }
    }
    for (old, new) in [
        (None, Some(AccessMode::Nyxmode)),
        (Some(AccessMode::Nyxmode), None),
    ] {
        assert!(validate_detail(&access(AccessModeScope::Workspace, old, new)).is_err());
        validate_detail(&access(AccessModeScope::Agent, old, new)).unwrap();
    }
}

#[test]
fn audit_detail_rejects_unknown_kinds_fields_and_enum_values() {
    for json in [
        r#"{"kind":"provider_payload","body":"secret"}"#,
        r#"{"kind":"security_flag_change","flag":"mfa_enabled","old":true,"new":false,"body":"secret"}"#,
        r#"{"kind":"security_flag_change","flag":"user_authored_flag","old":true,"new":false}"#,
        r#"{"kind":"access_mode_change","scope":"workspace","old":"secret","new":"autopilot"}"#,
        r#"{"kind":"org_role_change","old":"viewer","new":"superadmin"}"#,
        r#"{"kind":"approval_mode_change","old":"all","new":"inherit"}"#,
        r#"{"kind":"service_approval_mode_change","old":"grant","new":"off"}"#,
        r#"{"kind":"resource_set_change","kind_override":"provider","added":[],"removed":[],"truncated":false}"#,
    ] {
        assert!(serde_json::from_str::<AuditDetail>(json).is_err(), "{json}");
    }
}

#[test]
fn audit_detail_rejects_non_v4_and_noncanonical_resource_ids() {
    for id in [
        "00000000-0000-0000-0000-000000000000",
        "00000000-0000-5000-8000-000000000001",
        "https://secret",
        "00000000000040008000000000000001",
    ] {
        let detail = AuditDetail::ResourceSetChange {
            kind: ResourceSetKind::Nodes,
            added: vec![id.into()],
            removed: vec![],
            truncated: false,
        };
        assert!(validate_detail(&detail).is_err());
    }
}

#[test]
fn audit_detail_scope_ident_requires_registry_alphabet_and_byte_bound() {
    for ident in [
        "",
        "openid secret",
        "https://provider/scope",
        "OPENID",
        "email@example.com",
        "proxy:*",
        "not_a_registered_scope",
        &"x".repeat(65),
    ] {
        assert!(scope_ident(ident).is_err());
        // Storage deserialization cannot bypass the producer constructor.
        let detail = AuditDetail::ScopeSetChange {
            added: vec![ScopeIdent(ident.into())],
            removed: vec![],
            truncated: false,
        };
        assert!(validate_detail(&detail).is_err());
    }
    for ident in [
        "openid",
        "urn:nyxid:scope:broker_binding",
        "account:read",
        "llm:proxy",
    ] {
        scope_ident(ident).unwrap();
    }
}

#[test]
fn audit_detail_sets_reject_oversize_unsorted_duplicate_and_overlapping_entries() {
    for truncated in [false, true] {
        for (added, removed) in [
            (ids(0, 33), vec![]),
            (vec![], ids(0, 33)),
            (ids(0, 2).into_iter().rev().collect(), vec![]),
            (vec![ids(0, 1)[0].clone(); 2], vec![]),
            (ids(0, 1), ids(0, 1)),
        ] {
            assert!(
                validate_detail(&AuditDetail::ResourceSetChange {
                    kind: ResourceSetKind::Services,
                    added,
                    removed,
                    truncated
                })
                .is_err()
            );
        }
        let too_many = AuditDetail::ScopeSetChange {
            added: vec![scope_ident("openid").unwrap(); 33],
            removed: vec![],
            truncated,
        };
        assert!(validate_detail(&too_many).is_err());
    }
    validate_detail(&AuditDetail::ResourceSetChange {
        kind: ResourceSetKind::Nodes,
        added: ids(0, 32),
        removed: vec![],
        truncated: false,
    })
    .unwrap();
}

#[test]
fn audit_detail_total_limit_is_enforced_even_when_each_vector_is_bounded() {
    let detail = AuditDetail::ResourceSetChange {
        kind: ResourceSetKind::Agents,
        added: ids(0, 32),
        removed: ids(32, 32),
        truncated: true,
    };
    assert!(serde_json::to_vec(&detail).unwrap().len() > MAX_DETAIL_BYTES);
    assert!(validate_detail(&detail).is_err());
}

#[test]
fn audit_detail_session_count_bounds_and_saturation_are_explicit() {
    assert_eq!(
        session_revocation_count(1_000_001),
        AuditDetail::SessionRevocationCount {
            count: 1_000_000,
            saturated: true
        }
    );
    assert_eq!(
        session_revocation_count(1_000_000),
        AuditDetail::SessionRevocationCount {
            count: 1_000_000,
            saturated: false
        }
    );
    for detail in [
        AuditDetail::SessionRevocationCount {
            count: 1_000_001,
            saturated: false,
        },
        AuditDetail::SessionRevocationCount {
            count: 99,
            saturated: true,
        },
    ] {
        assert!(validate_detail(&detail).is_err());
    }
}

#[test]
fn audit_detail_expiry_rejects_lossy_submillisecond_storage() {
    let detail = AuditDetail::ExpiryChange {
        old: None,
        new: Some(Utc.timestamp_opt(1_800_000_000, 1).unwrap()),
    };
    assert!(validate_detail(&detail).is_err());
}

#[test]
fn audit_detail_no_detail_preserves_exact_preextension_json_and_bson_bytes() {
    let old = r#"{"operation_id":"00000000-0000-4000-8000-000000000001","stage":"committed","principal":{"kind":"system","actor":"maintenance"},"owner":{"kind":"platform"},"action":"audit_detail_test","target_id":null}"#;
    let event = event().safe_projection();
    assert_eq!(serde_json::to_vec(&event).unwrap(), old.as_bytes());
    assert_eq!(serde_json::from_str::<SafeAuditEvent>(old).unwrap(), event);
    let mut old_document = bson::to_document(&event).unwrap();
    assert!(!old_document.contains_key("detail"));
    // Build the pre-extension document independently in the historical order.
    let legacy = doc! {"operation_id": &event.operation_id, "stage": "committed", "principal": {"kind": "system", "actor": "maintenance"}, "owner": {"kind": "platform"}, "action": "audit_detail_test", "target_id": bson::Bson::Null};
    assert_eq!(
        bson::to_vec(&old_document).unwrap(),
        bson::to_vec(&legacy).unwrap()
    );
    old_document.insert("detail", bson::Bson::Null);
    assert_eq!(
        bson::from_document::<SafeAuditEvent>(old_document).unwrap(),
        event
    );
}

#[test]
fn audit_detail_serialization_is_deterministic_and_part_of_immutable_equality() {
    let event = event()
        .with_detail(access(
            AccessModeScope::Agent,
            None,
            Some(AccessMode::Autopilot),
        ))
        .project();
    let expected = r#"{"operation_id":"00000000-0000-4000-8000-000000000001","stage":"committed","principal":{"kind":"system","actor":"maintenance"},"owner":{"kind":"platform"},"action":"audit_detail_test","target_id":null,"detail":{"kind":"access_mode_change","scope":"agent","old":null,"new":"autopilot"}}"#;
    assert_eq!(serde_json::to_string(&event).unwrap(), expected);
    assert_eq!(serde_json::to_string(&event.clone()).unwrap(), expected);
    let mut changed = event.clone();
    changed.detail = Some(access(
        AccessModeScope::Agent,
        Some(AccessMode::AskEveryTime),
        Some(AccessMode::Autopilot),
    ));
    assert_ne!(event, changed);
}

#[test]
fn audit_detail_producers_forward_nested_borrows_from_transaction_contexts() {
    fn project(producer: &impl AuditEventSource) -> SafeAuditEvent {
        producer.project()
    }
    let plain = event();
    let detailed = plain.clone().with_detail(session_revocation_count(3));
    let context = (&plain, &detailed);
    let (plain_ref, detailed_ref) = &context;
    assert_eq!(project(plain_ref), plain.project());
    assert_eq!(project(detailed_ref), detailed.project());
    assert_eq!(project(&plain_ref), plain.project());
    assert_eq!(project(&detailed_ref), detailed.project());
}

#[tokio::test]
async fn audit_detail_enqueue_replay_conflicts_and_materialization_revalidates_storage() {
    let db = connect_transaction_test_database("audit_detail_replay").await;
    let key = crate::services::audit_service::audit_chain_hmac_key().unwrap();
    let id = Uuid::new_v4();
    let plain = event();
    let original = plain.clone().with_detail(session_revocation_count(3));
    let changed = plain.with_detail(session_revocation_count(4));
    let runtime = super::super::audit_runtime::AuditRuntime::new(Default::default(), false);
    for (candidate, conflict) in [(&original, false), (&original, false), (&changed, true)] {
        let mut session = db.client().start_session().await.unwrap();
        session
            .start_transaction()
            .write_concern(super::super::audit_state_service::majority())
            .await
            .unwrap();
        let result = enqueue_admitted_in_session(
            &db,
            &mut session,
            &runtime,
            super::super::audit_runtime::policy::AuditTier::Narrowing,
            &id,
            candidate,
        )
        .await;
        if conflict {
            assert!(matches!(result, Err(AppError::AuditIntegrityConflict)));
            session.abort_transaction().await.unwrap();
        } else {
            result.unwrap();
            super::super::audit_state_service::commit(&mut session)
                .await
                .unwrap();
        }
    }
    assert!(materialize_one(&db, key).await.unwrap());
    let row = db
        .collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"_id": id.to_string()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_value::<SafeAuditEvent>(row.event_data.unwrap()).unwrap(),
        original.project()
    );

    let mut next = event();
    next.operation_id = Uuid::new_v4().to_string();
    let next_id = Uuid::new_v4();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let invalid = next
        .clone()
        .with_detail(AuditDetail::SessionRevocationCount {
            count: 1_000_001,
            saturated: false,
        });
    assert!(matches!(
        enqueue_in_session(&db, &mut session, &next_id.to_string(), &invalid).await,
        Err(AppError::ValidationError(_))
    ));
    session.abort_transaction().await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .write_concern(super::super::audit_state_service::majority())
        .await
        .unwrap();
    enqueue_in_session(
        &db,
        &mut session,
        &next_id.to_string(),
        &next.with_detail(session_revocation_count(1)),
    )
    .await
    .unwrap();
    super::super::audit_state_service::commit(&mut session)
        .await
        .unwrap();
    // A syntactically valid stored detail can still be semantically invalid.
    db.collection::<bson::Document>(PENDING)
        .update_one(
            doc! {"_id": next_id.to_string()},
            doc! {"$set": {"event.detail.count": 1_000_001_i64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        materialize_one(&db, key).await,
        Err(AppError::ValidationError(_))
    ));
    let head = super::super::audit_state_service::load(&db).await.unwrap();
    assert_eq!(head.head_seq, 1);
    assert_eq!(head.pending_count, 2);
    assert!(
        db.collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"_id": next_id.to_string(), "materialized_seq": bson::Bson::Null})
            .await
            .unwrap()
            .is_some()
    );
}
