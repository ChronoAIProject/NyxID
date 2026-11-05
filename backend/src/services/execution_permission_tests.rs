use super::*;
use crate::models::{
    agent_operation_scope::{AgentOperationScope, ScopedOperation},
    downstream_service::ProxyOperationRule,
};
use bson::doc;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}

pub(crate) fn fixture() -> ApprovalGrant {
    let validity = Validity::Persistent;
    ApprovalGrant {
        execution_review: None,
        execution_last_used_at: None,
        execution_use: None,
        id: uuid::Uuid::new_v4().to_string(), user_id: "actor".into(), service_id: "catalog".into(),
        service_name: "Service".into(), requester_type: "api_key".into(), requester_id: "actor".into(), requester_label: None,
        approval_request_id: "human-decision".into(), scope: Some(LEGACY_SCOPE.into()), granted_at: now(),
        expires_at: legacy_expiry(&validity), revoked: false, org_scoped: false,
        execution: Some(ExecutionPermission {version: VERSION, v1: Some(ExecutionContract {
            subject: Subject {actor_user_id:"actor".into(), principal: Principal::Agent("agent".into()), lane: ExecutionLane::Owner},
            binding: ConnectionBinding {connection_id:"connection".into(), owner_user_id:"actor".into(), credential_id:Some("credential".into()), credential_epoch:1, authority_digest:"authority".into(), behavior_digest:None},
            context: ExecutionContext {audience: Audience::Private {user_id:"actor".into()}, conversation: ConversationScope::Exact {conversation_id:"conversation".into()}},
            operations: AgentOperationScope {revision:1, catalog_service_id:Some("catalog".into()), operations:vec![ScopedOperation {
                endpoint_id:Some("endpoint".into()), rule: ProxyOperationRule {method:"POST".into(),path_template:"/documents/{id}".into(), ..Default::default()},
                inputs: Some(serde_json::from_value(serde_json::json!({
                    "path":{"id":{"type":"exact","value":"doc1"}},
                    "query":{}, "body":{"type":"object","properties":{"text":{"type":"string","max_length":12}},"required":["text"]}
                })).unwrap()), ..Default::default()
            }]}, resource_policy:None, validity, satisfies:vec![ExecutionFloor::AccessModeRisk,ExecutionFloor::PersonalServiceApproval], revision:1,
        })}),
    }
}

fn contract(row: &ApprovalGrant) -> &ExecutionContract {
    row.execution.as_ref().unwrap().v1.as_ref().unwrap()
}
fn contract_mut(row: &mut ApprovalGrant) -> &mut ExecutionContract {
    row.execution.as_mut().unwrap().v1.as_mut().unwrap()
}
fn validity(row: &mut ApprovalGrant, value: Validity) {
    row.expires_at = legacy_expiry(&value);
    contract_mut(row).validity = value;
}
fn request<'a>(row: &'a ApprovalGrant, path: &'a CanonicalPath) -> Request<'a> {
    let permission = contract(row);
    Request {
        now: now(),
        subject: &permission.subject,
        binding: &permission.binding,
        audience: &permission.context.audience,
        conversation_id: "conversation",
        operation: PreparedOperation {
            id: "effect",
            digest: "prepared",
            catalog_service_id: Some("catalog"),
            endpoint_id: Some("endpoint"),
            method: "POST",
            path,
            query: None,
            body: br#"{"text":"hello"}"#,
            content_type: Some("application/json"),
        },
        task: None,
        resource: None,
        floors: &[
            ExecutionFloor::AccessModeRisk,
            ExecutionFloor::PersonalServiceApproval,
        ],
        authority: LiveAuthority::Allowed,
        eligible_decider: true,
        claim: ClaimState::Available,
        delegation_ceilings: &[],
    }
}
fn path() -> CanonicalPath {
    CanonicalPath::from_mcp_literal("/documents/doc1").unwrap()
}
fn covered(claim: bool) -> Result<Decision, EvaluationError> {
    Ok(Decision::Covered {
        operation_claim_required: claim,
    })
}

#[test]
fn persistent_and_bson_roundtrip() {
    let row = fixture();
    let path = path();
    assert_eq!(evaluate(&row, &request(&row, &path)), covered(false));
    let document = bson::to_document(&row).unwrap();
    assert!(document.get_datetime("expires_at").is_ok());
    let decoded: ApprovalGrant = bson::from_document(document).unwrap();
    assert_eq!(evaluate(&decoded, &request(&row, &path)), covered(false));
    assert_eq!(row.expires_at.to_rfc3339(), "9999-12-31T23:59:59+00:00");
}

#[test]
fn missing_unknown_and_invalid_contracts_never_cover() {
    let row = fixture();
    let path = path();
    let req = request(&row, &path);
    let mut other = row.clone();
    other.execution = None;
    assert_eq!(
        evaluate(&other, &req),
        Ok(Decision::RequestableGap(Gap::MissingPermission))
    );
    other.execution =
        Some(bson::from_document(doc! {"version":99,"future":{"arbitrary":"data"}}).unwrap());
    assert_eq!(
        evaluate(&other, &req),
        Ok(Decision::Prohibited(Prohibition::UnsupportedVersion))
    );
    other.execution = Some(ExecutionPermission {
        version: 1,
        v1: None,
    });
    assert_eq!(
        evaluate(&other, &req),
        Ok(Decision::Prohibited(Prohibition::InvalidContract))
    );
    other = row.clone();
    other.scope = None;
    assert_eq!(
        evaluate(&other, &req),
        Ok(Decision::Prohibited(Prohibition::InvalidContract))
    );
    other = row.clone();
    other.expires_at = now();
    assert_eq!(
        evaluate(&other, &req),
        Ok(Decision::Prohibited(Prohibition::InvalidContract))
    );
}

#[test]
fn actor_principal_and_lane_isolation() {
    let row = fixture();
    let path = path();
    for subject in [
        Subject {
            actor_user_id: "someone-else".into(),
            ..contract(&row).subject.clone()
        },
        Subject {
            principal: Principal::Agent("other-agent".into()),
            ..contract(&row).subject.clone()
        },
        Subject {
            lane: ExecutionLane::Shared,
            ..contract(&row).subject.clone()
        },
    ] {
        let mut req = request(&row, &path);
        req.subject = &subject;
        assert_eq!(
            evaluate(&row, &req),
            Ok(Decision::RequestableGap(Gap::Subject))
        );
    }
}

#[test]
fn conversation_and_audience_isolation() {
    let row = fixture();
    let path = path();
    let mut req = request(&row, &path);
    req.conversation_id = "other-thread";
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Context))
    );
    let mut explicit = row.clone();
    contract_mut(&mut explicit).context.conversation = ConversationScope::SameAudience;
    assert_eq!(evaluate(&explicit, &req), covered(false));
    let audience = Audience::Group {
        group_id: "group".into(),
        revision: 1,
    };
    req.audience = &audience;
    assert_eq!(
        evaluate(&explicit, &req),
        Ok(Decision::RequestableGap(Gap::Context))
    );
    let mut group = explicit;
    contract_mut(&mut group).context.audience = audience.clone();
    let changed = Audience::Group {
        group_id: "group".into(),
        revision: 2,
    };
    req.audience = &changed;
    assert_eq!(
        evaluate(&group, &req),
        Ok(Decision::RequestableGap(Gap::Context))
    );
}

#[test]
fn exact_connection_epoch_owner_and_behavior_binding() {
    let row = fixture();
    let path = path();
    let original = &contract(&row).binding;
    for changed in [
        ConnectionBinding {
            connection_id: "other".into(),
            ..original.clone()
        },
        ConnectionBinding {
            owner_user_id: "other".into(),
            ..original.clone()
        },
        ConnectionBinding {
            credential_id: Some("replacement".into()),
            ..original.clone()
        },
        ConnectionBinding {
            credential_epoch: 2,
            ..original.clone()
        },
        ConnectionBinding {
            authority_digest: "new-target".into(),
            ..original.clone()
        },
        ConnectionBinding {
            behavior_digest: Some("new-skill-pin".into()),
            ..original.clone()
        },
    ] {
        let mut req = request(&row, &path);
        req.binding = &changed;
        assert_eq!(
            evaluate(&row, &req),
            Ok(Decision::RequestableGap(Gap::Binding))
        );
    }
}

#[test]
fn prepared_inputs_and_resource_ids_cannot_widen() {
    let row = fixture();
    let path = path();
    for body in [
        br#"{"text":"too much input here"}"#.as_slice(),
        br#"{"text":"ok","extra":true}"#,
        br#"{}"#,
    ] {
        let mut req = request(&row, &path);
        req.operation.body = body;
        assert_eq!(
            evaluate(&row, &req),
            Ok(Decision::RequestableGap(Gap::Operation))
        );
    }
    let bad_path = CanonicalPath::from_mcp_literal("/documents/doc2").unwrap();
    assert_eq!(
        evaluate(&row, &request(&row, &bad_path)),
        Ok(Decision::RequestableGap(Gap::Operation))
    );
    let mut req = request(&row, &path);
    req.operation.query = Some("expand=all");
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Operation))
    );
    req = request(&row, &path);
    req.operation.endpoint_id = Some("other-endpoint");
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Operation))
    );
    req = request(&row, &path);
    req.operation.method = "DELETE";
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Operation))
    );
}

#[test]
fn window_boundaries_and_revocation() {
    let mut row = fixture();
    let path = path();
    validity(
        &mut row,
        Validity::Window {
            starts_at: now(),
            ends_at: now() + TimeDelta::hours(1),
        },
    );
    let mut req = request(&row, &path);
    assert_eq!(evaluate(&row, &req), covered(false));
    req.now = now() - TimeDelta::milliseconds(1);
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Validity))
    );
    req.now = now() + TimeDelta::hours(1);
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Validity))
    );
    let mut revoked = row.clone();
    revoked.revoked = true;
    revoked.expires_at = terminal_expiry(now());
    assert_eq!(
        evaluate(&revoked, &req),
        Ok(Decision::Prohibited(Prohibition::Revoked))
    );
    assert_eq!(terminal_expiry(now()), now() + TimeDelta::days(30));
}

#[test]
fn operation_coverage_never_consumes_or_replays_a_claim() {
    let mut row = fixture();
    let path = path();
    validity(
        &mut row,
        Validity::Operation {
            operation_id: "effect".into(),
            prepared_digest: "prepared".into(),
            expires_at: now() + TimeDelta::minutes(10),
        },
    );
    let mut req = request(&row, &path);
    assert_eq!(evaluate(&row, &req), covered(true));
    assert_eq!(evaluate(&row, &req), covered(true));
    req.operation.digest = "different";
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Validity))
    );
    req.operation.digest = "prepared";
    req.claim = ClaimState::Used;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::ClaimUsed))
    );
    req.claim = ClaimState::Uncertain;
    assert_eq!(evaluate(&row, &req), Err(EvaluationError::OutcomeUncertain));
}

#[test]
fn task_reference_alone_is_not_authority_and_new_roots_are_isolated() {
    let mut row = fixture();
    let path = path();
    let task = TaskRef {
        root_kind: TaskRootKind::Message,
        root_id: "root".into(),
    };
    let deadline = now() + TimeDelta::hours(1);
    validity(
        &mut row,
        Validity::Task {
            task: task.clone(),
            hard_deadline: deadline,
        },
    );
    let mut req = request(&row, &path);
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
    req.task = Some(TaskEvidence {
        task: &task,
        actor_user_id: "actor",
        audience: req.audience,
        hard_deadline: deadline,
        active: true,
        admitted_descendant: true,
    });
    assert_eq!(evaluate(&row, &req), covered(false));
    req.operation.id = "recorded-continuation";
    req.operation.digest = "next-prepared-effect";
    assert_eq!(evaluate(&row, &req), covered(false));
    let other_root = TaskRef {
        root_kind: TaskRootKind::Message,
        root_id: "new-request".into(),
    };
    req.task.as_mut().unwrap().task = &other_root;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
    req.task.as_mut().unwrap().task = &task;
    req.task.as_mut().unwrap().admitted_descendant = false;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
    req.task.as_mut().unwrap().admitted_descendant = true;
    req.task.as_mut().unwrap().active = false;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
    req.task.as_mut().unwrap().active = true;
    req.task.as_mut().unwrap().actor_user_id = "other";
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
    let next = TaskRef {
        root_kind: TaskRootKind::Invocation,
        root_id: "root".into(),
    };
    req.task.as_mut().unwrap().actor_user_id = "actor";
    req.task.as_mut().unwrap().task = &next;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
    req.task.as_mut().unwrap().task = &task;
    req.now = deadline;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Task))
    );
}

#[test]
fn only_the_three_approved_floors_can_be_satisfied() {
    let row = fixture();
    let path = path();
    for floor in [
        ExecutionFloor::WebhookAutomation,
        ExecutionFloor::MachineConfirm,
        ExecutionFloor::SavedLoginEachSignIn,
        ExecutionFloor::DelegationWidening,
        ExecutionFloor::OrgServiceApproval,
        ExecutionFloor::OrgInstruction,
        ExecutionFloor::OrgMandatoryReview,
        ExecutionFloor::ExactClaim,
        ExecutionFloor::DurableClaim,
        ExecutionFloor::Guest,
    ] {
        assert!(!satisfiable(floor));
        let floors = [floor];
        let mut req = request(&row, &path);
        req.floors = &floors;
        assert_eq!(
            evaluate(&row, &req),
            Ok(Decision::RequestableGap(Gap::Floor(floor)))
        );
        let mut forged = row.clone();
        contract_mut(&mut forged).satisfies.push(floor);
        assert_eq!(
            evaluate(&forged, &req),
            Ok(Decision::Prohibited(Prohibition::InvalidContract))
        );
    }
}

#[test]
fn shared_consent_only_covers_exact_actor_owned_connection() {
    let mut row = fixture();
    let path = path();
    let p = contract_mut(&mut row);
    p.subject.lane = ExecutionLane::Shared;
    p.binding.behavior_digest = Some("behavior".into());
    p.satisfies = vec![ExecutionFloor::SharedConnectionConsent];
    let mut req = request(&row, &path);
    req.floors = &[ExecutionFloor::SharedConnectionConsent];
    assert_eq!(evaluate(&row, &req), covered(false));
    let mut borrowed = row.clone();
    contract_mut(&mut borrowed).binding.owner_user_id = "agent-owner".into();
    let mut req = request(&borrowed, &path);
    req.floors = &[ExecutionFloor::SharedConnectionConsent];
    assert_eq!(
        evaluate(&borrowed, &req),
        Ok(Decision::Prohibited(Prohibition::Ceiling))
    );
}

#[test]
fn guests_ceiling_denials_and_operational_errors_never_become_consent() {
    let row = fixture();
    let path = path();
    let mut req = request(&row, &path);
    req.authority = LiveAuthority::Unavailable;
    assert_eq!(
        evaluate(&row, &req),
        Err(EvaluationError::AuthorityUnavailable)
    );
    req.authority = LiveAuthority::Denied;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Ceiling))
    );
    let guest = Subject {
        lane: ExecutionLane::Guest,
        ..req.subject.clone()
    };
    req.subject = &guest;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Guest))
    );
    assert_eq!(
        evaluate_candidates(&[], &req),
        Ok((None, Decision::Prohibited(Prohibition::Guest)))
    );
    req = request(&row, &path);
    req.eligible_decider = false;
    req.conversation_id = "other";
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::IneligibleDecider))
    );
}

#[test]
fn delegation_only_intersects_and_does_not_transfer_principal() {
    let row = fixture();
    let path = path();
    let mut req = request(&row, &path);
    let mut parent = contract(&row).operations.clone();
    parent.operations.clear();
    let parents = [parent];
    req.delegation_ceilings = &parents;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Ceiling))
    );
    let parents = [contract(&row).operations.clone()];
    req.delegation_ceilings = &parents;
    assert_eq!(evaluate(&row, &req), covered(false));
    let child = Subject {
        principal: Principal::Agent("child".into()),
        ..req.subject.clone()
    };
    req.subject = &child;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Subject))
    );
}

#[test]
fn reusable_permission_precedes_once_and_scopes_are_not_spliced() {
    let persistent = fixture();
    let path = path();
    let mut once = persistent.clone();
    once.id = "once".into();
    validity(
        &mut once,
        Validity::Operation {
            operation_id: "effect".into(),
            prepared_digest: "prepared".into(),
            expires_at: now() + TimeDelta::minutes(10),
        },
    );
    let candidates = [once, persistent.clone()];
    let req = request(&persistent, &path);
    assert_eq!(
        evaluate_candidates(&candidates, &req).unwrap(),
        (Some(persistent.id.as_str()), covered(false).unwrap())
    );
    let mut left = persistent.clone();
    contract_mut(&mut left).satisfies = vec![ExecutionFloor::AccessModeRisk];
    let mut right = persistent.clone();
    contract_mut(&mut right).satisfies = vec![ExecutionFloor::PersonalServiceApproval];
    assert!(matches!(
        evaluate_candidates(&[left, right], &req).unwrap().1,
        Decision::RequestableGap(_)
    ));
}

#[test]
fn legacy_scope_parser_refuses_execution_marker() {
    let descriptor =
        super::super::operation_descriptor::build_http_descriptor("POST", "/documents/doc1", None);
    assert!(!super::super::approval_policy::grant_scope_covers(
        Some(LEGACY_SCOPE),
        &descriptor
    ));
    assert!(super::super::approval_policy::grant_scope_covers(
        None,
        &descriptor
    ));
}

#[tokio::test]
async fn legacy_personal_and_org_grant_checks_refuse_execution_rows() {
    let db = crate::test_utils::connect_test_database("execution_permission_legacy")
        .await
        .expect("review Mongo required");
    let rows = db.collection::<ApprovalGrant>(crate::models::approval_grant::COLLECTION_NAME);
    let descriptor =
        super::super::operation_descriptor::build_http_descriptor("POST", "/documents/doc1", None);
    for org_scoped in [false, true] {
        let mut row = fixture();
        row.org_scoped = org_scoped;
        rows.insert_one(&row).await.unwrap();
        // Real legacy evaluator, not a mock or an added execution-field filter.
        for org_policy in [false, true] {
            assert!(
                !super::super::approval_service::check_approval(
                    &db,
                    "actor",
                    "catalog",
                    "api_key",
                    "actor",
                    org_policy,
                    &descriptor
                )
                .await
                .unwrap()
            );
        }
        rows.delete_one(doc! {"_id":&row.id}).await.unwrap();
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn execution_listing_and_revocation_do_not_require_grant_mode() {
    use super::super::approval_service as approvals;
    let db = crate::test_utils::connect_test_database("execution_permission_listing")
        .await
        .expect("review Mongo required");
    let row = fixture();
    let rows = db.collection::<ApprovalGrant>(crate::models::approval_grant::COLLECTION_NAME);
    rows.insert_one(&row).await.unwrap();
    assert_eq!(
        approvals::list_grants(&db, "actor", &[], 1, 20)
            .await
            .unwrap()
            .1,
        1
    );
    assert_eq!(
        approvals::list_grants(&db, "outsider", &[], 1, 20)
            .await
            .unwrap()
            .1,
        0
    );
    assert!(
        approvals::revoke_grant(&db, "outsider", &row.id)
            .await
            .is_err()
    );
    let before = DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
    approvals::revoke_grant(&db, "actor", &row.id)
        .await
        .unwrap();
    let revoked = rows.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap();
    assert!(revoked.revoked);
    assert!(revoked.expires_at >= terminal_expiry(before));
    assert!(revoked.expires_at <= terminal_expiry(Utc::now()));
    assert_eq!(
        approvals::list_grants(&db, "actor", &[], 1, 20)
            .await
            .unwrap()
            .1,
        0
    );
    db.drop().await.unwrap();
}

#[test]
fn oauth_refresh_preserves_coverage_but_account_replacement_invalidates() {
    use super::super::{
        execution_authority,
        proxy_service::{ProxyTarget, UserServiceResolution},
    };
    let service = crate::models::downstream_service::test_helpers::dummy_service();
    let mut resolution = UserServiceResolution {
        target: ProxyTarget {
            execution_request: None,
            execution_permission: None,
            credential_deferred: false,
            workspace_destinations_pending: false,
            target_id: None,
            base_url: service.base_url.clone(),
            auth_method: "bearer".into(),
            auth_key_name: "Authorization".into(),
            credential: "first-oauth-token".to_owned().into(),
            service,
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: Some("oauth-account".into()),
        },
        catalog_service_slug: Some("api-google-drive".into()),
        node_id: None,
        user_service_id: "connection".into(),
        has_server_credential: true,
        api_key_id: Some("credential".into()),
        credential_epoch: 1,
        master_credential_revision: None,
        master_credential: false,
        credential_source: Some("byo".into()),
        org_routing: None,
        pool_selection: None,
        is_auto_connected: false,
    };
    let mut row = fixture();
    let path = path();
    contract_mut(&mut row).binding = connection_binding(
        "actor",
        &execution_authority::build_projection(&resolution, None, vec![]),
        None,
    );
    // Exercise the real execution projection with different refreshed token
    // bytes, not a hand-built digest or a weaker connection-only comparison.
    resolution.target.credential = "refreshed-oauth-token".to_owned().into();
    let refreshed = connection_binding(
        "actor",
        &execution_authority::build_projection(&resolution, None, vec![]),
        None,
    );
    let mut req = request(&row, &path);
    req.binding = &refreshed;
    assert_eq!(evaluate(&row, &req), covered(false));
    resolution.credential_epoch += 1;
    let replaced = connection_binding(
        "actor",
        &execution_authority::build_projection(&resolution, None, vec![]),
        None,
    );
    req.binding = &replaced;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Binding))
    );
    resolution.credential_epoch = 1;
    resolution.api_key_id = Some("another-account".into());
    let different = connection_binding(
        "actor",
        &execution_authority::build_projection(&resolution, None, vec![]),
        None,
    );
    req.binding = &different;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::RequestableGap(Gap::Binding))
    );
}

#[test]
fn resource_policy_requires_supported_adapter_and_exact_live_evidence() {
    let mut row = fixture();
    let path = path();
    let policy = nyxid_permissions::drive::example_policy();
    let digest =
        super::super::mcp_service::canonical_sha256(serde_json::to_value(&policy).unwrap());
    contract_mut(&mut row).resource_policy = Some(policy);
    let mut req = request(&row, &path);
    assert_eq!(
        evaluate(&row, &req),
        Err(EvaluationError::ResourceVerificationRequired)
    );
    req.resource = Some(ResourceEvidence {
        policy_digest: &digest,
        prepared_digest: "prepared",
        authority_digest: "authority",
        allowed: true,
    });
    assert_eq!(evaluate(&row, &req), covered(false));
    req.resource.as_mut().unwrap().prepared_digest = "another-operation";
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Resource))
    );
    req.resource.as_mut().unwrap().prepared_digest = "prepared";
    req.resource.as_mut().unwrap().allowed = false;
    assert_eq!(
        evaluate(&row, &req),
        Ok(Decision::Prohibited(Prohibition::Resource))
    );
    let mut unsupported = row.clone();
    contract_mut(&mut unsupported)
        .resource_policy
        .as_mut()
        .unwrap()
        .provider = "unsupported".into();
    assert_eq!(
        evaluate(&unsupported, &req),
        Ok(Decision::Prohibited(Prohibition::InvalidContract))
    );
}

#[test]
fn candidates_preserve_errors_and_prefer_requestable_gaps_to_unrelated_rejections() {
    let row = fixture();
    let path = path();
    let req = request(&row, &path);
    let mut revoked = row.clone();
    revoked.revoked = true;
    let mut gap_row = row.clone();
    contract_mut(&mut gap_row).binding.connection_id = "other".into();
    for candidates in [[revoked.clone(), gap_row.clone()], [gap_row, revoked]] {
        assert_eq!(
            evaluate_candidates(&candidates, &req).unwrap().1,
            Decision::RequestableGap(Gap::Binding)
        );
    }
    let mut resource = row.clone();
    contract_mut(&mut resource).resource_policy = Some(nyxid_permissions::drive::example_policy());
    let mut missing = row.clone();
    missing.execution = None;
    for candidates in [
        [resource.clone(), missing.clone()],
        [missing, resource.clone()],
    ] {
        assert_eq!(
            evaluate_candidates(&candidates, &req),
            Err(EvaluationError::ResourceVerificationRequired)
        );
    }
    assert_eq!(
        evaluate_candidates(&[resource, row.clone()], &req)
            .unwrap()
            .1,
        covered(false).unwrap()
    );
}

#[test]
fn missing_bindings_are_not_wildcards() {
    let row = fixture();
    let path = path();
    let req = request(&row, &path);
    for mutate in [
        (|p: &mut ExecutionContract| p.subject.principal = Principal::Agent(String::new()))
            as fn(&mut ExecutionContract),
        |p| {
            p.context.conversation = ConversationScope::Exact {
                conversation_id: String::new(),
            }
        },
        |p| {
            p.context.audience = Audience::Private {
                user_id: "other".into(),
            }
        },
        |p| {
            p.context.audience = Audience::Group {
                group_id: "group".into(),
                revision: 0,
            }
        },
        |p| p.operations.revision = 0,
    ] {
        let mut invalid = row.clone();
        mutate(contract_mut(&mut invalid));
        assert_eq!(
            evaluate(&invalid, &req),
            Ok(Decision::Prohibited(Prohibition::InvalidContract))
        );
    }
}

#[tokio::test]
async fn execution_org_listing_obeys_owner_admin_and_service_scopes() {
    use super::super::approval_service::{OrgFilterBranch, list_grants};
    let db = crate::test_utils::connect_test_database("execution_permission_org_listing")
        .await
        .expect("review Mongo required");
    let rows = db.collection::<ApprovalGrant>(crate::models::approval_grant::COLLECTION_NAME);
    let mut row = fixture();
    row.user_id = "org".into();
    row.org_scoped = true;
    rows.insert_one(&row).await.unwrap();
    assert_eq!(list_grants(&db, "actor", &[], 1, 20).await.unwrap().1, 0);
    let branch = |scope| OrgFilterBranch {
        org_id: "org".into(),
        service_id_scope: scope,
    };
    assert_eq!(
        list_grants(&db, "actor", &[branch(Some(vec!["other".into()]))], 1, 20)
            .await
            .unwrap()
            .1,
        0
    );
    assert_eq!(
        list_grants(&db, "actor", &[branch(Some(vec!["catalog".into()]))], 1, 20)
            .await
            .unwrap()
            .1,
        1
    );
    assert_eq!(
        list_grants(&db, "actor", &[branch(None)], 1, 20)
            .await
            .unwrap()
            .1,
        1
    );
    db.drop().await.unwrap();
}

#[test]
fn temporary_extension_expires_back_to_persistent_baseline() {
    let persistent = fixture();
    let mut temporary = persistent.clone();
    temporary.id = "temporary".into();
    contract_mut(&mut temporary).operations.operations[0].inputs=Some(serde_json::from_value(serde_json::json!({
        "path":{"id":{"type":"exact","value":"doc2"}},"query":{},
        "body":{"type":"object","properties":{"text":{"type":"string","max_length":12}},"required":["text"]}
    })).unwrap());
    validity(
        &mut temporary,
        Validity::Window {
            starts_at: now(),
            ends_at: now() + TimeDelta::hours(1),
        },
    );
    let baseline_path = path();
    let extra_path = CanonicalPath::from_mcp_literal("/documents/doc2").unwrap();
    let candidates = [persistent.clone(), temporary];
    let mut req = request(&persistent, &extra_path);
    assert_eq!(
        evaluate_candidates(&candidates, &req).unwrap().0,
        Some("temporary")
    );
    req.now = now() + TimeDelta::hours(1);
    assert!(matches!(
        evaluate_candidates(&candidates, &req).unwrap().1,
        Decision::RequestableGap(_)
    ));
    req.operation.path = &baseline_path;
    assert_eq!(
        evaluate_candidates(&candidates, &req).unwrap().0,
        Some(persistent.id.as_str())
    );
}
