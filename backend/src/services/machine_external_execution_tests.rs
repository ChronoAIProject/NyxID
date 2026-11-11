//! Authenticated source -> transactional grant/job/lease -> signed Node dispatch.
//! The peer verifies production signatures but does not claim Linux execution.
#[path = "machine_external_gateway_policy_tests.rs"]
mod gateway_policy;

use super::*;
use crate::{
    crypto::token::{agent_key_display_prefix, hash_token},
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        node::NodeStatus,
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{
        key_service,
        machine_external_grant_service::{GrantAssignment, GrantSelection},
        node_service,
        node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage},
    },
    test_utils::{
        connect_transaction_test_database, register_test_node_connection, test_app_state, test_user,
    },
};
use nyxid_machine::{MachineProfile, authority::Capabilities};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

struct Fixture {
    state: AppState,
    owner: String,
    key: ApiKey,
    node: Node,
    secret: String,
    grant_id: String,
}
async fn fixture(name: &str) -> Fixture {
    let db = connect_transaction_test_database(name).await;
    super::super::machine_access_service::ensure_indexes(&db)
        .await
        .unwrap();
    ensure_indexes(&db).await.unwrap();
    grants::create_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let secret = crate::services::api_key_credential_service::generate_secret().to_string();
    let key:ApiKey=bson::from_document(doc!{"_id":Uuid::new_v4().to_string(),"user_id":&owner,"name":"External integration","key_prefix":agent_key_display_prefix(&secret).unwrap(),"key_hash":hash_token(&secret),"scopes":"proxy","is_active":true,"state_version":1_i64,"created_at":bson::DateTime::now(),"allow_all_nodes":true,"allow_all_services":true}).unwrap();
    db.collection::<ApiKey>(KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    let state = test_app_state(db);
    let (_, token, _) =
        node_service::create_registration_token(&state.db, &owner, "external-machine", 100, 300)
            .await
            .unwrap();
    let (mut node, _, _) =
        node_service::register_node(&state.db, &state.encryption_keys, &token, None)
            .await
            .unwrap();
    node.status = NodeStatus::Online;
    node.machine = Some(MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: Uuid::new_v4().to_string(),
        shell: true,
        files: true,
        browser: Some(false),
        authority_versions: vec![2, 3],
        roots: vec!["/workspace".into()],
        os: "linux".into(),
        arch: "aarch64".into(),
        ..Default::default()
    });
    state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&node.id}, &node)
        .await
        .unwrap();
    let grant = grants::approve_api_key(
        &state.db,
        &owner,
        &key.id,
        key.state_version,
        &GrantSelection {
            separation: Some(nyxid_machine::context::Choice::SharedLegacy),
            legacy_warning_acknowledged: true,
            assignments: BTreeMap::from([(
                node.id.clone(),
                GrantAssignment {
                    capabilities: Capabilities {
                        shell: true,
                        files: true,
                        ..Default::default()
                    },
                    file_roots: vec!["/workspace".into()],
                    service_ids: vec![],
                },
            )]),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    Fixture {
        state,
        owner,
        key,
        node,
        secret,
        grant_id: grant.id,
    }
}
async fn source(f: &Fixture, secret: &str) -> VerifiedExternalSource {
    crate::handlers::mcp_transport::external_machine_source_for_test(&f.state, secret)
        .await
        .unwrap()
}

async fn child(f: &Fixture) -> (String, String) {
    let id = Uuid::new_v4().to_string();
    let secret = crate::services::api_key_credential_service::generate_secret().to_string();
    f.state.db.collection::<Document>(crate::models::api_key_credential::COLLECTION_NAME).insert_one(doc!{"_id":&id,"api_key_id":&f.key.id,"user_id":&f.owner,"secret_hash":hash_token(&secret),"secret_prefix":agent_key_display_prefix(&secret).unwrap(),"label":"test child","login_request_id":Uuid::new_v4().to_string(),"is_active":true,"created_at":bson::DateTime::now()}).await.unwrap();
    (id, secret)
}
async fn peer(
    f: &Fixture,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::Receiver<nyxid_machine::Request>,
) {
    let (tx, mut rx) = mpsc::channel(32);
    register_test_node_connection(&f.state, &f.node.id, tx).await;
    let manager = f.state.node_ws_manager.clone();
    manager.record_capabilities(
        &f.node.id,
        &serde_json::from_value::<NodeCapabilitiesMsg>(json!({"machine":f.node.machine})).unwrap(),
    );
    let secret =
        node_service::get_node_signing_secret(&f.state.db, &f.state.encryption_keys, &f.node.id)
            .await
            .unwrap();
    let id = f.node.id.clone();
    let runtime = f.node.machine.as_ref().unwrap().runtime_id.clone();
    let (seen, requests) = mpsc::channel(32);
    let task = tokio::spawn(async move {
        let mut replay = nyxid_machine::signing::ReplayGuard::default();
        while let Some(NodeOutboundMessage::Text(text)) = rx.recv().await {
            let request: nyxid_machine::Request = serde_json::from_str(&text).unwrap();
            replay
                .verify(&request, &id, &secret, Utc::now().timestamp())
                .unwrap();
            assert!(request.authority.is_none());
            if request.version == 1 {
                assert!(request.external_authority.is_none());
                let proof = &request.parameters["owner_job_access"];
                assert_eq!(proof["node_id"], id);
                assert_eq!(proof["runtime_id"], runtime);
                assert_eq!(proof["job_id"], request.parameters["job_id"]);
                assert!(proof["expires_at_ms"].as_i64().unwrap() > Utc::now().timestamp_millis());
                assert!(matches!(
                    request.operation,
                    Operation::Job | Operation::JobCancel
                ));
            } else {
                let authority = request
                    .external_authority
                    .as_ref()
                    .expect("external v3 authority");
                assert_eq!(request.version, 3);
                if request.operation == Operation::AuthorityRevoke {
                    assert!(authority.valid_stop(&id, &runtime, Utc::now().timestamp_millis()));
                } else {
                    assert!(authority.valid(&id, &runtime, Utc::now().timestamp_millis()));
                }
            }
            let response = match request.operation {
                Operation::Exec => {
                    json!({"job_id":request.parameters["job_id"],"status":"running"})
                }
                Operation::AuthorityRenew | Operation::AuthorityRevoke => json!({"accepted":true}),
                Operation::Job => {
                    json!({"job_id":request.parameters["job_id"],"status":"running","stdout":"bounded test output"})
                }
                Operation::JobCancel => {
                    json!({"job_id":request.parameters["job_id"],"cancel_requested":true})
                }
                _ => json!({"ok":true}),
            };
            manager.deliver_machine_result(
                &id,
                nyxid_machine::Response {
                    request_id: request.request_id.clone(),
                    result: response,
                },
            );
            let _ = seen.try_send(request);
        }
    });
    (task, requests)
}
async fn execute(f: &Fixture, source: &VerifiedExternalSource) -> String {
    let result = dispatch_authorized(
        &f.state,
        source,
        "nyx__machine_exec",
        &mut json!({"machine":f.node.id,"command":"printf tested","background":true}),
    )
    .await
    .unwrap();
    result["job_id"].as_str().unwrap().to_owned()
}
async fn job(f: &Fixture, id: &str) -> MachineJob {
    f.state
        .db
        .collection::<MachineJob>(machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn owner_grants_uses_managed_allocation_before_node_and_rejects_conflicting_owners() {
    use crate::models::remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine};
    let f = fixture("external_owner_grants_allocation").await;
    let managed = super::super::remote_machine_service::CreateMachineInput::fixed(
        &f.owner,
        true,
        "test-image",
    )
    .preview(&f.node.id, Utc::now());
    f.state
        .db
        .collection::<ManagedMachine>(MACHINES)
        .insert_one(&managed)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.node.id})
        .await
        .unwrap();
    let visible = owner_grants(&f.state.db, &f.owner, &f.node.id)
        .await
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, f.grant_id);
    let stranger = Uuid::new_v4().to_string();
    assert!(
        owner_grants(&f.state.db, &stranger, &f.node.id)
            .await
            .is_err()
    );
    assert!(
        owner_grants(&f.state.db, &f.owner, &Uuid::new_v4().to_string())
            .await
            .is_err()
    );
    let mut conflicting_node = f.node.clone();
    conflicting_node.user_id = stranger.clone();
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .insert_one(&conflicting_node)
        .await
        .unwrap();
    assert!(
        owner_grants(&f.state.db, &f.owner, &f.node.id)
            .await
            .is_err()
    );
    assert!(
        owner_grants(&f.state.db, &stranger, &f.node.id)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.node.id}, &f.node)
        .await
        .unwrap();
    assert_eq!(
        owner_grants(&f.state.db, &f.owner, &f.node.id)
            .await
            .unwrap()
            .len(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_real_key_auth_grant_and_signed_dispatch_persist_no_assistant_identity() {
    let f = fixture("external_dispatch").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    let request = requests.recv().await.unwrap();
    let authority = request.external_authority.unwrap();
    assert_eq!(authority.grant_id, f.grant_id);
    assert_eq!(authority.file_roots, vec!["/workspace"]);
    assert!(authority.services.is_empty());
    assert_eq!(authority.runtime_generation, None);
    let raw = f
        .state
        .db
        .collection::<Document>(machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    for legacy in ["conversation_id", "api_key_id", "agent_id"] {
        assert!(!raw.contains_key(legacy), "external must omit {legacy}");
    }
    assert_eq!(
        raw.get_document("binding")
            .unwrap()
            .get_str("kind")
            .unwrap(),
        "external"
    );
    assert_eq!(raw.get("report_to"), Some(&bson::Bson::Null));
    assert_eq!(raw.get_i32("report_provenance").unwrap(), 0);
    let persisted = job(&f, &id).await;
    assert!(persisted.report_to.is_none());
    assert_eq!(persisted.report_provenance, 0);
    let page = history(&f.state.db, &source, &f.node.id, &json!({}))
        .await
        .unwrap();
    assert_eq!(page["jobs"][0]["job_id"], id);
    assert!(page["jobs"][0].get("stdout").is_none());
    let auth = gateway_auth(&f.state.db, &job(&f, &id).await)
        .await
        .unwrap();
    assert_eq!(auth.api_key_id.as_deref(), Some(f.key.id.as_str()));
    assert!(auth.assistant_chat.is_none());
    assert!(!auth.allow_all_services);
    assert!(auth.allowed_service_ids.is_empty());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_real_oauth_jwt_mcp_auth_preserves_consent_and_client_through_dispatch() {
    use crate::{
        crypto::jwt,
        services::{
            consent_service, machine_oauth_consent_service as prepared, oauth_client_service,
        },
    };
    let f = fixture("external_oauth_dispatch").await;
    let client = oauth_client_service::create_client(
        &f.state.db,
        "External OAuth integration",
        &["https://example.invalid/callback".into()],
        "public",
        &f.owner,
        "",
        "openid proxy machine offline_access",
        crate::models::oauth_client::ScopeProvenance::Explicit,
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap()
    .0;
    let selection = GrantSelection {
        separation: Some(nyxid_machine::context::Choice::SharedLegacy),
        legacy_warning_acknowledged: true,
        assignments: BTreeMap::from([(
            f.node.id.clone(),
            GrantAssignment {
                capabilities: Capabilities {
                    shell: true,
                    files: true,
                    ..Default::default()
                },
                file_roots: vec!["/workspace".into()],
                service_ids: vec![],
            },
        )]),
        expires_at: None,
    };
    let approval = prepared::MachineConsentApproval {
        request_id: Uuid::new_v4().to_string(),
        signed_request: format!("verified-local-consent-{}", Uuid::new_v4()),
    };
    prepared::prepare(
        &f.state.db,
        &f.owner,
        &client.id,
        &approval.request_id,
        &approval.signed_request,
        &selection,
    )
    .await
    .unwrap();
    let consent = consent_service::grant_consent_with_machine(
        &f.state.db,
        &f.owner,
        &client.id,
        "openid proxy machine offline_access",
        false,
        Some(vec![]),
        Some(&approval),
    )
    .await
    .unwrap();
    let reference = consent_service::protected_reference(&consent).unwrap();
    let mint = |resources: &[String]| {
        jwt::generate_oauth_access_token_with_consent(
            &f.state.jwt_keys,
            &f.state.config,
            &Uuid::parse_str(&f.owner).unwrap(),
            "openid proxy machine offline_access",
            None,
            Some(600),
            None,
            None,
            Some(jwt::AccessTokenRestrictions {
                resources,
                allowed_service_ids: &[],
                allowed_node_ids: std::slice::from_ref(&f.node.id),
                allow_all_nodes: false,
            }),
            &client.id,
            Some(&reference),
        )
        .unwrap()
    };
    // Real OAuth issuance encodes omitted RFC8707 resource as Some([]).
    // That is not a service-only audience restriction. An explicit MCP
    // audience also works; a nonempty service-only audience must not.
    let token = mint(&[]);
    let explicit_mcp = mint(&[super::super::oauth_resource_service::mcp_resource_uri(
        &f.state.config,
    )]);
    assert!(
        crate::handlers::mcp_transport::external_machine_oauth_source_for_test(
            &f.state,
            &explicit_mcp,
        )
        .await
        .is_ok()
    );
    let service_only = mint(&[
        super::super::oauth_resource_service::user_service_resource_uri(
            &f.state.config,
            "unrelated-service",
        ),
    ]);
    assert!(
        crate::handlers::mcp_transport::external_machine_oauth_source_for_test(
            &f.state,
            &service_only,
        )
        .await
        .is_err()
    );
    let source =
        crate::handlers::mcp_transport::external_machine_oauth_source_for_test(&f.state, &token)
            .await
            .unwrap();
    assert_eq!(
        source.principal(),
        ExternalPrincipalRef::OAuth {
            user_id: f.owner.clone(),
            oauth_client_id: client.id.clone()
        }
    );
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    let wire = requests.recv().await.unwrap().external_authority.unwrap();
    assert_eq!(wire.principal, wire_principal(&source.principal()));
    assert_eq!(
        wire.oauth_consent.as_ref().unwrap().consent_id,
        reference.consent_id
    );
    let row = f
        .state
        .db
        .collection::<Document>(machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.contains_key("api_key_id"));
    assert!(!row.contains_key("conversation_id"));
    let auth = gateway_auth(&f.state.db, &job(&f, &id).await)
        .await
        .unwrap();
    assert_eq!(auth.oauth_client_id.as_deref(), Some(client.id.as_str()));
    assert!(auth.api_key_id.is_none());
    assert!(auth.assistant_chat.is_none());
    // Recovery uses only the persisted binding, so a cache-empty replica or a
    // server restart needs no synthetic token and cannot extend authentication.
    let persisted = job(&f, &id).await;
    let recovered = source_for_job(&f.state.db, &persisted).await.unwrap();
    assert_eq!(recovered.principal(), source.principal());
    assert_eq!(recovered.protected_consent(), Some(&reference));
    assert_eq!(
        recovered.authentication_expires_at(),
        external_binding(&persisted).unwrap().4
    );
    renew_job_with_source(&f.state, &recovered, &persisted)
        .await
        .unwrap();
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRenew
    );
    assert_eq!(job(&f, &id).await.binding, persisted.binding);
    let mut expired = persisted.clone();
    if let Some(MachineJobBinding::External {
        authentication_expires_at,
        ..
    }) = &mut expired.binding
    {
        *authentication_expires_at = Utc::now() - Duration::seconds(1);
    }
    assert!(source_for_job(&f.state.db, &expired).await.is_err());
    // Revocation is live after reconstruction, independently of any sweep.
    consent_service::revoke_consent(&f.state.db, &f.owner, &client.id)
        .await
        .unwrap();
    assert!(gateway_auth(&f.state.db, &persisted).await.is_err());
    assert!(
        renew_job_with_source(&f.state, &recovered, &persisted)
            .await
            .is_err()
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_admission_reloads_confirmation_policy_and_rejects_oversized_writes_before_rows() {
    use base64::Engine;
    let f = fixture("external_admission_live").await;
    let source = source(&f, &f.secret).await;
    f.state
        .db
        .collection::<Document>(node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.node.id},
            doc! {"$set":{"machine_confirm":"all"}},
        )
        .await
        .unwrap();
    let admission = admit_request(
        &f.state,
        &source,
        &f.node,
        Operation::Exec,
        &json!({"command":"true"}),
        None,
    )
    .await
    .unwrap();
    assert!(
        admission.pending.is_some(),
        "stale pre-transaction Node must not bypass owner's latest policy"
    );
    assert!(admission.job.is_none());
    let existing_confirmations = f
        .state
        .db
        .collection::<Document>(execution::CONFIRMATIONS)
        .count_documents(doc! {})
        .await
        .unwrap();
    let too_large = vec![b'x'; nyxid_machine::managed::MANAGED_FILE_WRITE_BYTES + 1];
    let mut args = json!({"machine":f.node.id,"path":"/workspace/large","content":base64::engine::general_purpose::STANDARD.encode(&too_large),"encoding":"base64"});
    assert!(matches!(
        dispatch_authorized(&f.state, &source, "nyx__machine_write_file", &mut args).await,
        Err(AppError::MachineLimitExceeded)
    ));
    assert_eq!(
        f.state
            .db
            .collection::<Document>(execution::CONFIRMATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        existing_confirmations
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(machine_job::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_sibling_shared_view_never_rebinds_or_renews_original_child() {
    let f = fixture("external_child_view").await;
    let (a_id, a_secret) = child(&f).await;
    let (_, b_secret) = child(&f).await;
    let a = source(&f, &a_secret).await;
    let b = source(&f, &b_secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &a).await;
    let _ = requests.recv().await;
    let original = job(&f, &id).await;
    let original_binding = original.binding.clone();
    assert!(
        renew_job_with_source(&f.state, &b, &original)
            .await
            .is_err()
    );
    let result = dispatch_authorized(
        &f.state,
        &b,
        "nyx__machine_job",
        &mut json!({"machine":f.node.id,"job_id":id}),
    )
    .await
    .unwrap();
    assert_eq!(result["stdout"], "bounded test output");
    let request = requests.recv().await.unwrap();
    assert_eq!(request.operation, Operation::Job);
    assert_ne!(
        request.external_authority.unwrap().principal,
        wire_principal(&a.principal())
    );
    assert_eq!(job(&f, &id).await.binding, original_binding);
    let leases: Vec<ExternalLease> = f
        .state
        .db
        .collection(access::LEASES)
        .find(doc! {"job_id":&id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(
        leases[0].external_authority.principal,
        wire_principal(&a.principal())
    );
    let revision = f
        .state
        .db
        .collection::<Document>(crate::models::machine_external_grant::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.grant_id})
        .await
        .unwrap()
        .unwrap()
        .get_i64("revision")
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let revoked = crate::services::api_key_credential_service::revoke_in_session(
        &f.state.db,
        &a_id,
        crate::models::api_key_credential::CredentialRevokedReason::WebRevoke,
        &mut session,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        matches!(revoked._source_invalidation.selector,ExternalRevocationSelector::Child{ref api_key_credential_id,..} if api_key_credential_id==&a_id)
    );
    session.commit_transaction().await.unwrap();
    assert!(
        gateway_auth(&f.state.db, &job(&f, &id).await)
            .await
            .is_err()
    );
    let outbox = f
        .state
        .db
        .collection::<Document>(access::OUTBOX)
        .find_one(doc! {"kind":"external","pending":true})
        .await
        .unwrap()
        .unwrap();
    revoke_outbox(&f.state, outbox).await.unwrap();
    let revoked_job = job(&f, &id).await;
    assert_eq!(revoked_job.state, "revoked");
    assert_eq!(
        revoked_job.cancellation_requested_by,
        Some(MachineCancellationRequester::System {
            reason: MachineSystemCancellationReason::SourceRevoked,
        })
    );
    assert!(revoked_job.cancellation_requested_at.is_some());
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::machine_external_grant::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.grant_id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("revision")
            .unwrap(),
        revision
    );
    assert!(
        !history(&f.state.db, &b, &f.node.id, &json!({}))
            .await
            .unwrap()["jobs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        key_service::validate_api_key(&f.state.db, &a_secret)
            .await
            .is_err()
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_confirmation_owner_only_bound_and_consumed_once() {
    let mut f = fixture("external_confirmation").await;
    f.node.machine_confirm = nyxid_machine::Confirmation::All;
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.node.id}, &f.node)
        .await
        .unwrap();
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let args = json!({"machine":f.node.id,"command":"printf tested","background":true});
    let pending = dispatch_authorized(&f.state, &source, "nyx__machine_exec", &mut args.clone())
        .await
        .unwrap();
    assert_eq!(pending["status"], "confirmation_pending");
    assert!(requests.try_recv().is_err());
    let id = pending["confirmation_id"].as_str().unwrap();
    let confirmations = owner_confirmations(&f.state.db, &f.owner, Some(&f.node.id))
        .await
        .unwrap();
    assert_eq!(
        confirmations["confirmations"][0]["review_parameters_available"],
        true
    );
    assert_eq!(
        confirmations["confirmations"][0]["review_content_available"],
        false
    );
    assert!(!confirmations.to_string().contains("printf tested"));
    assert!(
        owner_decide(&f.state.db, &Uuid::new_v4().to_string(), id, true)
            .await
            .is_err()
    );
    owner_decide(&f.state.db, &f.owner, id, true).await.unwrap();
    let mut wrong = args.clone();
    wrong["command"] = json!("different");
    wrong["acknowledgement_id"] = json!(id);
    assert!(
        dispatch_authorized(&f.state, &source, "nyx__machine_exec", &mut wrong)
            .await
            .is_err()
    );
    let mut allowed = args.clone();
    allowed["acknowledgement_id"] = json!(id);
    let result = dispatch_authorized(&f.state, &source, "nyx__machine_exec", &mut allowed.clone())
        .await
        .unwrap();
    assert!(result["job_id"].is_string());
    let _ = requests.recv().await;
    let replay = dispatch_authorized(&f.state, &source, "nyx__machine_exec", &mut allowed)
        .await
        .unwrap();
    assert_eq!(replay["status"], "already_consumed");
    assert!(requests.try_recv().is_err());
    let next = dispatch_authorized(&f.state, &source, "nyx__machine_exec", &mut args.clone())
        .await
        .unwrap();
    assert_eq!(next["status"], "confirmation_pending");
    assert_ne!(
        next["confirmation_id"], pending["confirmation_id"],
        "a new identical operation requires a new one-shot approval after prior consumption"
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(machine_job::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_history_survives_runtime_replacement_but_output_never_forwards() {
    let f = fixture("external_history").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    let _ = requests.recv().await;
    finish_terminal(
        &f.state.db,
        &id,
        &f.node.id,
        &f.node.machine.as_ref().unwrap().runtime_id,
        "interrupted",
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>(node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.node.id},
            doc! {"$set":{"machine.runtime_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    let output = dispatch_authorized(
        &f.state,
        &source,
        "nyx__machine_job",
        &mut json!({"machine":f.node.id,"job_id":id}),
    )
    .await
    .unwrap();
    assert_eq!(output["output_unavailable_reason"], "runtime_replaced");
    assert!(requests.try_recv().is_err());
    let row = job(&f, &id).await;
    assert!(row.retain_until.unwrap() > Utc::now() + Duration::days(29));
    assert!(
        history(
            &f.state.db,
            &source,
            &f.node.id,
            &json!({"state":"interrupted","limit":1})
        )
        .await
        .unwrap()["jobs"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
    // Late Node completion cannot overwrite a terminal safety state.
    finish_terminal(&f.state.db, &id, &f.node.id, &row.runtime_id, "finished")
        .await
        .unwrap();
    assert_eq!(job(&f, &id).await.state, "interrupted");
    finish_observed(
        &f.state.db,
        &id,
        &f.node.id,
        &row.runtime_id,
        &json!({"exit_code":0,"stop_reason":"completed","duration_ms":123}),
    )
    .await
    .unwrap();
    let retained = job(&f, &id).await;
    assert_eq!(retained.state, "interrupted");
    assert!(retained.exit_code.is_none());
    f.state
        .db
        .collection::<Document>(node::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.node.id})
        .await
        .unwrap();
    let after_delete = owner_history(&f.state.db, &f.owner, &f.node.id, &json!({}))
        .await
        .unwrap();
    assert_eq!(after_delete["jobs"][0]["job_id"], id);
    assert_eq!(after_delete["jobs"][0]["output_query_available"], false);
    assert!(
        owner_output(&f.state, &f.owner, &f.node.id, &id, 0, 0)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Document>(machine_job::COLLECTION_NAME)
        .update_one(doc! {"_id":&id}, doc! {"$unset":{"retain_until":""}})
        .await
        .unwrap();
    maintenance(&f.state).await.unwrap();
    let migrated = job(&f, &id).await;
    assert!(migrated.retain_until.is_some());
    assert_eq!(migrated.finished_at, retained.finished_at);
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[test]
fn external_selectors_keep_children_narrow_and_cursor_order_stable() {
    let selector = ExternalRevocationSelector::Child {
        user_id: Uuid::new_v4().to_string(),
        api_key_id: Uuid::new_v4().to_string(),
        api_key_credential_id: Uuid::new_v4().to_string(),
    };
    let filter = selector_filter(&selector, "binding.");
    assert!(filter.contains_key("binding.principal.api_key_credential_id"));
    assert!(!filter.contains_key("binding.grant_id"));
    let cursor = format!("{}:{}", Utc::now().timestamp_millis(), Uuid::new_v4());
    assert!(page_filter(&json!({"cursor":cursor,"limit":50})).is_ok());
    assert!(page_filter(&json!({"limit":51})).is_err());
    assert!(page_filter(&json!({"cursor":"not-a-cursor"})).is_err());
}

#[tokio::test]
async fn external_http_mcp_denies_without_machine_grant() {
    use axum::{Extension, Router, body::Body, http::Request, routing::post};
    use tower::ServiceExt;
    let f = fixture("external_mcp_no_grant").await;
    grants::revoke(&f.state.db, &f.owner, &f.grant_id, 1)
        .await
        .unwrap();
    let (peer, mut requests) = peer(&f).await;
    let app = Router::new()
        .route("/mcp", post(crate::handlers::mcp_transport::mcp_post))
        .layer(Extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Mcp,
            ),
        ))
        .with_state(f.state.clone());
    let request=Request::builder().method("POST").uri("/mcp").header("content-type","application/json").header("accept","application/json, text/event-stream").header("x-api-key",&f.secret).body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nyx__machine_exec","arguments":{"machine":f.node.id,"command":"true"}}}).to_string())).unwrap();
    let response = app.oneshot(request).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        response["result"]["isError"] == true || response.get("error").is_some(),
        "no-grant source must reject the actual HTTP call"
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(machine_job::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn owner_management_uses_narrow_human_proof_after_source_revocation() {
    use axum::{
        extract::{Path, Query, State},
        response::IntoResponse,
    };
    let f = fixture("external_owner_manage").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    let _ = requests.recv().await;
    let grant = grants::get_active_for_key(&f.state.db, &f.owner, &f.key.id)
        .await
        .unwrap()
        .unwrap();
    grants::revoke(&f.state.db, &f.owner, &grant.id, grant.revision)
        .await
        .unwrap();
    assert!(
        gateway_auth(&f.state.db, &job(&f, &id).await)
            .await
            .is_err()
    );
    let mut denied = crate::test_utils::test_auth_user(&f.owner);
    denied.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    denied.api_key_id = Some(f.key.id.clone());
    assert!(
        crate::handlers::machine_external_execution::output(
            State(f.state.clone()),
            denied,
            Path((f.node.id.clone(), id.clone())),
            Query(crate::handlers::machine_external_execution::OutputQuery {
                output_offset: None,
                stderr_offset: None
            })
        )
        .await
        .is_err()
    );
    let response = crate::handlers::machine_external_execution::output(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Path((f.node.id.clone(), id.clone())),
        Query(crate::handlers::machine_external_execution::OutputQuery {
            output_offset: Some(0),
            stderr_offset: Some(0),
        }),
    )
    .await
    .unwrap()
    .into_response();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let request = requests.recv().await.unwrap();
    assert_eq!(request.version, 1);
    assert!(request.external_authority.is_none());
    assert!(request.parameters.get("conversation_id").is_none());
    let result = owner_cancel(&f.state, &f.owner, &f.node.id, &id)
        .await
        .unwrap();
    assert_eq!(result["status"], "cancelled");
    assert!(result.get("stdout").is_none());
    let request = requests.recv().await.unwrap();
    assert_eq!(request.operation, Operation::JobCancel);
    let cancelled = job(&f, &id).await;
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(
        cancelled.cancellation_requested_by,
        Some(MachineCancellationRequester::HumanOwner {
            user_id: f.owner.clone(),
        })
    );
    assert!(cancelled.cancellation_requested_at.is_some());
    let metadata = job_metadata(&cancelled);
    assert_eq!(metadata["cancellation_requested_by"]["kind"], "human_owner");
    assert_eq!(metadata["cancellation_requested_by"]["user_id"], f.owner);
    assert!(metadata["cancellation_requested_at"].is_string());
    owner_cancel(&f.state, &f.owner, &f.node.id, &id)
        .await
        .unwrap();
    assert_eq!(
        job(&f, &id).await.cancellation_requested_at,
        cancelled.cancellation_requested_at,
        "a repeated cancellation must retain the original request time"
    );
    assert!(
        owner_output(&f.state, &Uuid::new_v4().to_string(), &f.node.id, &id, 0, 0)
            .await
            .is_err()
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_narrow_cancel_survives_grant_revoke_but_never_revoked_source() {
    let f = fixture("external_narrow_stop").await;
    let (child_id, child_secret) = child(&f).await;
    let source = source(&f, &child_secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    let _ = requests.recv().await;
    let grant = grants::get_active_for_key(&f.state.db, &f.owner, &f.key.id)
        .await
        .unwrap()
        .unwrap();
    grants::revoke(&f.state.db, &f.owner, &grant.id, grant.revision)
        .await
        .unwrap();
    let result = dispatch_authorized(
        &f.state,
        &source,
        "nyx__machine_job_cancel",
        &mut json!({"machine":f.node.id,"job_id":id}),
    )
    .await
    .unwrap();
    assert_eq!(result["cancellation_acknowledged"], true);
    assert_eq!(result["stop_acknowledged"], false);
    assert!(result.get("stdout").is_none());
    let stop = requests.recv().await.unwrap();
    assert_eq!(stop.operation, Operation::AuthorityRevoke);
    let cancelled = job(&f, &id).await;
    assert_eq!(
        cancelled.cancellation_requested_by,
        Some(MachineCancellationRequester::External {
            principal: source.principal(),
        })
    );
    assert!(cancelled.cancellation_requested_at.is_some());
    let metadata = job_metadata(&cancelled);
    assert_eq!(metadata["cancellation_requested_by"]["kind"], "external");
    assert_eq!(
        metadata["cancellation_requested_by"]["principal"]["api_key_credential_id"],
        child_id
    );
    assert!(metadata["cancellation_requested_at"].is_string());
    assert!(
        dispatch_authorized(
            &f.state,
            &source,
            "nyx__machine_job",
            &mut json!({"machine":f.node.id,"job_id":id})
        )
        .await
        .is_err()
    );
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    crate::services::api_key_credential_service::revoke_in_session(
        &f.state.db,
        &child_id,
        crate::models::api_key_credential::CredentialRevokedReason::WebRevoke,
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    assert!(
        dispatch_authorized(
            &f.state,
            &source,
            "nyx__machine_job_cancel",
            &mut json!({"machine":f.node.id,"job_id":id})
        )
        .await
        .is_err()
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_sibling_cancel_after_grant_revoke_keeps_original_job_principal() {
    let f = fixture("external_sibling_narrow_stop").await;
    let (_, original_secret) = child(&f).await;
    let (_, sibling_secret) = child(&f).await;
    let original_secret = zeroize::Zeroizing::new(original_secret);
    let sibling_secret = zeroize::Zeroizing::new(sibling_secret);
    let original_source = source(&f, &original_secret).await;
    let sibling_source = source(&f, &sibling_secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &original_source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let original = job(&f, &id).await;
    let grant = grants::get_active_for_key(&f.state.db, &f.owner, &f.key.id)
        .await
        .unwrap()
        .unwrap();
    grants::revoke(&f.state.db, &f.owner, &grant.id, grant.revision)
        .await
        .unwrap();
    let result = dispatch(
        &f.state,
        &sibling_source,
        "nyx__machine_job_cancel",
        json!({"machine":f.node.id,"job_id":id}),
    )
    .await
    .unwrap();
    assert_eq!(result["cancellation_acknowledged"], true);
    assert_eq!(result["stop_acknowledged"], false);
    assert!(result.get("stdout").is_none());
    let stopped = requests.recv().await.unwrap();
    assert_eq!(stopped.operation, Operation::AuthorityRevoke);
    let authority = stopped.external_authority.unwrap();
    assert!(authority.principal == wire_principal(&original_source.principal()));
    assert!(authority.principal != wire_principal(&sibling_source.principal()));
    assert_eq!(authority.lease_id, external_binding(&original).unwrap().6);
    let cancelled = job(&f, &id).await;
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.binding, original.binding);
    assert_eq!(
        cancelled.cancellation_requested_by,
        Some(MachineCancellationRequester::External {
            principal: sibling_source.principal()
        })
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_confirmation_checks_original_child_expiry_before_sweep() {
    let mut f = fixture("external_confirm_live_child").await;
    f.node.machine_confirm = nyxid_machine::Confirmation::All;
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.node.id}, &f.node)
        .await
        .unwrap();
    let (child_id, child_secret) = child(&f).await;
    let caller = source(&f, &child_secret).await;
    let (peer, mut requests) = peer(&f).await;
    let pending = dispatch_authorized(
        &f.state,
        &caller,
        "nyx__machine_exec",
        &mut json!({"machine":f.node.id,"command":"true","background":true}),
    )
    .await
    .unwrap();
    let id = pending["confirmation_id"].as_str().unwrap();
    let row = f
        .state
        .db
        .collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.principal, caller.principal());
    assert!(
        row.authentication_expires_at
            .is_some_and(|at| at > Utc::now())
    );
    // Natural source expiry has no mutation producer: decision must enforce it
    // immediately, without waiting for a sweep to invalidate the pending row.
    f.state.db.collection::<Document>(crate::models::api_key_credential::COLLECTION_NAME)
        .update_one(doc! {"_id":&child_id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}})
        .await.unwrap();
    assert!(owner_decide(&f.state.db, &f.owner, id, true).await.is_err());
    let after = f
        .state
        .db
        .collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.state, "pending");
    assert!(after.decided_at.is_none());
    assert!(requests.try_recv().is_err());
    assert_eq!(
        f.state
            .db
            .collection::<Document>(machine_job::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_confirmation_approval_never_extends_original_authentication() {
    let mut f = fixture("external_confirm_auth_bound").await;
    f.node.machine_confirm = nyxid_machine::Confirmation::All;
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.node.id}, &f.node)
        .await
        .unwrap();
    let (child_id, child_secret) = child(&f).await;
    let auth_until = bson::DateTime::from_chrono(Utc::now() + Duration::minutes(2));
    f.state
        .db
        .collection::<Document>(crate::models::api_key_credential::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&child_id},
            doc! {"$set":{"expires_at":auth_until}},
        )
        .await
        .unwrap();
    let caller = source(&f, &child_secret).await;
    let pending = dispatch_authorized(
        &f.state,
        &caller,
        "nyx__machine_exec",
        &mut json!({"machine":f.node.id,"command":"true","background":true}),
    )
    .await
    .unwrap();
    let id = pending["confirmation_id"].as_str().unwrap();
    let approved = owner_decide(&f.state.db, &f.owner, id, true).await.unwrap();
    assert_eq!(approved["state"], "approved");
    let row = f
        .state
        .db
        .collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.authentication_expires_at, Some(auth_until.to_chrono()));
    assert!(row.expires_at <= auth_until.to_chrono());
    // A persisted original authentication cutoff is independently enforced even
    // if the pending display window has not reached its own deadline.
    f.state.db.collection::<Document>(execution::CONFIRMATIONS)
        .update_one(doc! {"_id":id}, doc! {"$set":{"state":"pending","authentication_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1)),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10))}})
        .await.unwrap();
    assert!(owner_decide(&f.state.db, &f.owner, id, true).await.is_err());
    assert!(
        owner_confirmations(&f.state.db, &f.owner, None)
            .await
            .unwrap()["confirmations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_global_confirmation_pages_keep_equal_timestamp_rows_and_owner_scope() {
    let mut f = fixture("external_confirm_pages").await;
    f.node.machine_confirm = nyxid_machine::Confirmation::All;
    f.state
        .db
        .collection::<Node>(node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.node.id}, &f.node)
        .await
        .unwrap();
    let caller = source(&f, &f.secret).await;
    let pending = dispatch_authorized(
        &f.state,
        &caller,
        "nyx__machine_exec",
        &mut json!({"machine":f.node.id,"command":"true","background":true}),
    )
    .await
    .unwrap();
    let original = f
        .state
        .db
        .collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
        .find_one(doc! {"_id":pending["confirmation_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let mut rows = Vec::new();
    let mut expected = std::collections::BTreeSet::from([original.id.clone()]);
    for _ in 0..104 {
        let mut row = original.clone();
        row.id = Uuid::new_v4().to_string();
        expected.insert(row.id.clone());
        rows.push(row);
    }
    let mut other = original.clone();
    other.id = Uuid::new_v4().to_string();
    other.user_id = Uuid::new_v4().to_string();
    other.created_at += Duration::seconds(1);
    rows.push(other);
    let mut expired = original.clone();
    expired.id = Uuid::new_v4().to_string();
    expired.authentication_expires_at = Some(Utc::now() - Duration::seconds(1));
    rows.push(expired);
    f.state
        .db
        .collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
        .insert_many(rows)
        .await
        .unwrap();
    let first = owner_confirmations_page(&f.state.db, &f.owner, None, None, Some(100))
        .await
        .unwrap();
    assert_eq!(first["confirmations"].as_array().unwrap().len(), 100);
    let cursor = first["next_cursor"].as_str().expect("continuation");
    let second = owner_confirmations_page(&f.state.db, &f.owner, None, Some(cursor), Some(100))
        .await
        .unwrap();
    assert_eq!(second["confirmations"].as_array().unwrap().len(), 5);
    assert!(second["next_cursor"].is_null());
    let found: std::collections::BTreeSet<String> = first["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["confirmations"].as_array().unwrap())
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(found, expected);
    assert!(
        owner_confirmations_page(
            &f.state.db,
            &f.owner,
            Some(&Uuid::new_v4().to_string()),
            None,
            None
        )
        .await
        .unwrap()["confirmations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        owner_confirmations_page(&f.state.db, &f.owner, None, Some("bad|cursor"), None)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_discovery_hint_is_loaded_with_key_and_does_not_grant_authority() {
    let f = fixture("external_discovery_hint").await;
    assert!(
        !f.key.machine_external_grants_present,
        "original pre-approval snapshot"
    );
    let before = VerifiedExternalSource::from_verified_api_key(&f.key, None).unwrap();
    assert!(!before.discovery_possible());
    let current = source(&f, &f.secret).await;
    assert!(
        current.discovery_possible(),
        "authentication returns sticky marker without a discovery lookup"
    );
    // The hint is only for discovery: explicit calls use live admission, even
    // when a already-loaded request predates the owner's approval.
    assert_eq!(visible(&f.state.db, &before).await.unwrap().len(), 1);
    grants::revoke(&f.state.db, &f.owner, &f.grant_id, 1)
        .await
        .unwrap();
    let after = source(&f, &f.secret).await;
    assert!(
        after.discovery_possible(),
        "revocation does not clear the sticky hint"
    );
    assert!(visible(&f.state.db, &after).await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

// A scoped authorization caller pauses only after its transaction establishes a
// read snapshot. The competing real caller then commits its fence writes before
// the first caller attempts its own writes. MongoDB must reject the stale snapshot;
// this is not a synthetic database error, timing delay, or authorization bypass.
// Task-local installation plus the exact database/job pair prevents parallel
// tests and unrelated calls from observing this hook.
#[derive(Clone)]
struct GatewaySnapshotHook {
    database: String,
    job_id: String,
    reached: std::sync::Arc<tokio::sync::Semaphore>,
    resume: std::sync::Arc<tokio::sync::Semaphore>,
    attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

tokio::task_local! {
    static GATEWAY_SNAPSHOT_HOOK: GatewaySnapshotHook;
    static CANCEL_SNAPSHOT_HOOK: GatewaySnapshotHook;
}

impl GatewaySnapshotHook {
    fn new(db: &Database, job: &MachineJob) -> Self {
        Self {
            database: db.name().to_owned(),
            job_id: job.id.clone(),
            reached: std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
            resume: std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
            attempts: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    async fn wait_for_snapshot(&self) {
        self.reached.acquire().await.unwrap().forget();
    }

    fn release(&self) {
        self.resume.add_permits(1);
    }

    fn attempts(&self) -> usize {
        self.attempts.load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub(super) async fn gateway_snapshot_loaded(db: &Database, job_id: &str) {
    transaction_snapshot_loaded(
        GATEWAY_SNAPSHOT_HOOK.try_with(Clone::clone).ok(),
        db,
        job_id,
    )
    .await;
}

pub(super) async fn cancel_snapshot_loaded(db: &Database, job_id: &str) {
    transaction_snapshot_loaded(CANCEL_SNAPSHOT_HOOK.try_with(Clone::clone).ok(), db, job_id).await;
}

async fn transaction_snapshot_loaded(
    hook: Option<GatewaySnapshotHook>,
    db: &Database,
    job_id: &str,
) {
    let Some(hook) = hook else {
        return;
    };
    if hook.database != db.name() || hook.job_id != job_id {
        return;
    }
    if hook
        .attempts
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        == 0
    {
        hook.reached.add_permits(1);
        hook.resume.acquire().await.unwrap().forget();
    }
}

async fn gateway_oauth_source(f: &Fixture) -> VerifiedExternalSource {
    use crate::{
        crypto::jwt,
        services::{
            consent_service, machine_oauth_consent_service as prepared, oauth_client_service,
        },
    };
    let (client, _) = oauth_client_service::create_client(
        &f.state.db,
        "Gateway contention OAuth",
        &["https://example.invalid/callback".into()],
        "public",
        &f.owner,
        "",
        "openid proxy machine offline_access",
        crate::models::oauth_client::ScopeProvenance::Explicit,
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    let selection = GrantSelection {
        separation: Some(nyxid_machine::context::Choice::SharedLegacy),
        legacy_warning_acknowledged: true,
        assignments: BTreeMap::from([(
            f.node.id.clone(),
            GrantAssignment {
                capabilities: Capabilities {
                    shell: true,
                    files: true,
                    ..Default::default()
                },
                file_roots: vec!["/workspace".into()],
                service_ids: vec![],
            },
        )]),
        expires_at: None,
    };
    let approval = prepared::MachineConsentApproval {
        request_id: Uuid::new_v4().to_string(),
        signed_request: format!("verified-local-contention-{}", Uuid::new_v4()),
    };
    prepared::prepare(
        &f.state.db,
        &f.owner,
        &client.id,
        &approval.request_id,
        &approval.signed_request,
        &selection,
    )
    .await
    .unwrap();
    let consent = consent_service::grant_consent_with_machine(
        &f.state.db,
        &f.owner,
        &client.id,
        "openid proxy machine offline_access",
        false,
        Some(vec![]),
        Some(&approval),
    )
    .await
    .unwrap();
    let reference = consent_service::protected_reference(&consent).unwrap();
    let token = jwt::generate_oauth_access_token_with_consent(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.owner).unwrap(),
        "openid proxy machine offline_access",
        None,
        Some(600),
        None,
        None,
        Some(jwt::AccessTokenRestrictions {
            resources: &[],
            allowed_service_ids: &[],
            allowed_node_ids: std::slice::from_ref(&f.node.id),
            allow_all_nodes: false,
        }),
        &client.id,
        Some(&reference),
    )
    .unwrap();
    crate::handlers::mcp_transport::external_machine_oauth_source_for_test(&f.state, &token)
        .await
        .unwrap()
}

#[tokio::test]
async fn external_cancel_retries_renewal_and_reloads_current_transport_authority() {
    let f = fixture("external_cancel_renewal_contention").await;
    let source = gateway_oauth_source(&f).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let original = job(&f, &id).await;
    let ExternalPrincipalRef::OAuth {
        oauth_client_id, ..
    } = source.principal()
    else {
        panic!("OAuth source required");
    };
    let token = zeroize::Zeroizing::new(
        crate::crypto::jwt::generate_oauth_access_token_with_consent(
            &f.state.jwt_keys,
            &f.state.config,
            &Uuid::parse_str(&f.owner).unwrap(),
            "openid proxy machine offline_access",
            None,
            Some(1200),
            None,
            None,
            Some(crate::crypto::jwt::AccessTokenRestrictions {
                resources: &[],
                allowed_service_ids: &[],
                allowed_node_ids: std::slice::from_ref(&f.node.id),
                allow_all_nodes: false,
            }),
            &oauth_client_id,
            source.protected_consent(),
        )
        .unwrap(),
    );
    let refreshed =
        crate::handlers::mcp_transport::external_machine_oauth_source_for_test(&f.state, &token)
            .await
            .unwrap();
    let hook = GatewaySnapshotHook::new(&f.state.db, &original);
    let cancelling = CANCEL_SNAPSHOT_HOOK.scope(
        hook.clone(),
        dispatch(
            &f.state,
            &source,
            "nyx__machine_job_cancel",
            json!({"machine":f.node.id,"job_id":id}),
        ),
    );
    let renewing = async {
        hook.wait_for_snapshot().await;
        renew_job_with_source(&f.state, &refreshed, &original)
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(cancelling, renewing)
    })
    .await
    .expect("cancellation and renewal must finish without timing sleeps");
    let result = result.expect("current source cancellation must retry the real snapshot conflict");
    assert!(hook.attempts() >= 2, "a real stale snapshot must retry");
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["cancellation_acknowledged"], true);
    assert_eq!(result["stop_acknowledged"], false);
    assert!(result.get("stdout").is_none());
    let renewed = requests.recv().await.unwrap();
    assert_eq!(renewed.operation, Operation::AuthorityRenew);
    let renewed = renewed.external_authority.unwrap();
    assert!(renewed.auth_expires_at_ms > source.authentication_expires_at().timestamp_millis());
    let stopped = requests.recv().await.unwrap();
    assert_eq!(stopped.operation, Operation::AuthorityRevoke);
    let stopped = stopped.external_authority.unwrap();
    assert_eq!(stopped.auth_expires_at_ms, renewed.auth_expires_at_ms);
    assert_eq!(stopped.principal, renewed.principal);
    assert_eq!(stopped.oauth_consent, renewed.oauth_consent);
    assert_eq!(stopped.lease_id, renewed.lease_id);
    assert_eq!(stopped.job_id.as_deref(), Some(id.as_str()));
    assert_eq!(stopped.node_id, original.node_id);
    assert_eq!(stopped.runtime_id, original.runtime_id);
    assert!(
        requests.try_recv().is_err(),
        "retries must not send Node effects"
    );
    let cancelled = job(&f, &id).await;
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(
        cancelled.cancellation_requested_by,
        Some(MachineCancellationRequester::External {
            principal: source.principal()
        })
    );
    assert!(cancelled.cancellation_requested_at.is_some());
    assert_eq!(cancelled.finished_at, cancelled.cancellation_requested_at);
    assert_eq!(
        f.state
            .db
            .collection::<Document>(access::LEASES)
            .count_documents(doc! {"_id":&stopped.lease_id})
            .await
            .unwrap(),
        0
    );
    let rows: Vec<Document> = f
        .state
        .db
        .collection(access::OUTBOX)
        .find(doc! {"kind":"external_lease","lease_id":&stopped.lease_id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "one committed cancellation outbox row");
    assert_eq!(rows[0].get_str("reason").unwrap(), "job_cancelled");
    let queued: ExternalAuthority =
        bson::from_bson(rows[0].get("external_authority").unwrap().clone()).unwrap();
    assert_eq!(queued.auth_expires_at_ms, renewed.auth_expires_at_ms);
    assert_eq!(queued.lease_id, renewed.lease_id);
    assert_eq!(
        f.state
            .db
            .collection::<Document>(machine_job::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1,
        "cancellation retries never create or replay execution"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_cancel_retry_rechecks_revoked_child_before_any_stop_effect() {
    let f = fixture("external_cancel_child_revocation_contention").await;
    let (child_id, secret) = child(&f).await;
    let secret = zeroize::Zeroizing::new(secret);
    let source = source(&f, &secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let original = job(&f, &id).await;
    let hook = GatewaySnapshotHook::new(&f.state.db, &original);
    let cancelling = CANCEL_SNAPSHOT_HOOK.scope(
        hook.clone(),
        dispatch(
            &f.state,
            &source,
            "nyx__machine_job_cancel",
            json!({"machine":f.node.id,"job_id":id}),
        ),
    );
    let revoking = async {
        hook.wait_for_snapshot().await;
        renew_job_with_source(&f.state, &source, &original)
            .await
            .unwrap();
        let mut session = f.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        crate::services::api_key_credential_service::revoke_in_session(
            &f.state.db,
            &child_id,
            crate::models::api_key_credential::CredentialRevokedReason::WebRevoke,
            &mut session,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(cancelling, revoking)
    })
    .await
    .expect("revocation and cancellation must finish without timing sleeps");
    assert!(hook.attempts() >= 2, "a real stale snapshot must retry");
    assert!(
        matches!(result, Err(AppError::MachinePermissionRevoked)),
        "retry must reject the now-revoked child"
    );
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRenew
    );
    assert!(
        requests.try_recv().is_err(),
        "denied cancellation sends no Node effect"
    );
    let unchanged = job(&f, &id).await;
    assert_eq!(unchanged.state, "running");
    assert!(unchanged.cancellation_requested_at.is_none());
    let lease = external_binding(&original).unwrap().6;
    assert_eq!(
        f.state
            .db
            .collection::<Document>(access::LEASES)
            .count_documents(doc! {"_id":lease})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(access::OUTBOX)
            .count_documents(doc! {"kind":"external_lease","lease_id":lease})
            .await
            .unwrap(),
        0,
        "failed attempts leave no cancellation outbox"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_retries_real_oauth_renewal_contention_without_replaying_effects() {
    let f = fixture("external_gateway_renewal_contention").await;
    let source = gateway_oauth_source(&f).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let persisted = job(&f, &id).await;
    let original_binding = persisted.binding.clone();
    let hook = GatewaySnapshotHook::new(&f.state.db, &persisted);
    let gateway = GATEWAY_SNAPSHOT_HOOK.scope(hook.clone(), gateway_auth(&f.state.db, &persisted));
    let renew = async {
        hook.wait_for_snapshot().await;
        renew_job_with_source(&f.state, &source, &persisted)
            .await
            .unwrap();
        hook.release();
    };
    let (auth, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(gateway, renew)
    })
    .await
    .expect("gateway and renewal must complete without timing sleeps");
    let auth = auth.expect("still-valid OAuth gateway must retry the owner-fence conflict");
    let ExternalPrincipalRef::OAuth {
        oauth_client_id, ..
    } = source.principal()
    else {
        panic!("OAuth fixture must use its verified original source");
    };
    assert_eq!(
        auth.oauth_client_id.as_deref(),
        Some(oauth_client_id.as_str())
    );
    assert!(auth.api_key_id.is_none());
    assert!(auth.assistant_chat.is_none());
    assert!(hook.attempts() >= 2, "a real stale snapshot must retry");
    assert_eq!(job(&f, &id).await.binding, original_binding);
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRenew,
    );
    assert!(
        requests.try_recv().is_err(),
        "gateway retries send no Node effect"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_retries_real_admission_contention_without_duplicate_jobs() {
    let f = fixture("external_gateway_admission_contention").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let first_id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let persisted = job(&f, &first_id).await;
    let hook = GatewaySnapshotHook::new(&f.state.db, &persisted);
    let gateway = GATEWAY_SNAPSHOT_HOOK.scope(hook.clone(), gateway_auth(&f.state.db, &persisted));
    let admission = async {
        hook.wait_for_snapshot().await;
        let second_id = execute(&f, &source).await;
        hook.release();
        second_id
    };
    let (auth, second_id) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(gateway, admission)
    })
    .await
    .expect("gateway and admission must complete without timing sleeps");
    let auth = auth.expect("still-valid gateway must retry concurrent admission fences");
    assert_eq!(auth.api_key_id.as_deref(), Some(f.key.id.as_str()));
    assert_ne!(second_id, first_id);
    assert!(hook.attempts() >= 2, "a real stale snapshot must retry");
    for collection in [machine_job::COLLECTION_NAME, access::LEASES] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2,
            "each successful admission creates exactly one {collection} row",
        );
    }
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    assert!(
        requests.try_recv().is_err(),
        "gateway retries send no Node effect"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_retry_rechecks_owner_grant_revocation() {
    let f = fixture("external_gateway_revoke_contention").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let persisted = job(&f, &id).await;
    let hook = GatewaySnapshotHook::new(&f.state.db, &persisted);
    let gateway = GATEWAY_SNAPSHOT_HOOK.scope(hook.clone(), gateway_auth(&f.state.db, &persisted));
    let revoke = async {
        hook.wait_for_snapshot().await;
        grants::revoke(&f.state.db, &f.owner, &f.grant_id, 1)
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(gateway, revoke)
    })
    .await
    .expect("gateway and revocation must complete without timing sleeps");
    assert!(
        matches!(result, Err(AppError::MachinePermissionRevoked)),
        "retry must return current grant denial, never transient database_error: {result:?}",
    );
    assert!(
        hook.attempts() >= 2,
        "revocation must invalidate the first snapshot"
    );
    assert!(
        matches!(
            renew_job_with_source(&f.state, &source, &persisted).await,
            Err(AppError::MachinePermissionRevoked)
        ),
        "renewal cannot restore the revoked original grant",
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_retry_reloads_persisted_authentication_expiry() {
    let f = fixture("external_gateway_expiry_contention").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    // The argument intentionally remains valid. Only a reload inside the retry
    // sees the expired persisted execution source; no stale caller snapshot can
    // extend it. Backdating models an already-reached cutoff without a sleep.
    let persisted = job(&f, &id).await;
    let hook = GatewaySnapshotHook::new(&f.state.db, &persisted);
    let gateway = GATEWAY_SNAPSHOT_HOOK.scope(hook.clone(), gateway_auth(&f.state.db, &persisted));
    let expire = async {
        hook.wait_for_snapshot().await;
        f.state
            .db
            .collection::<Document>(machine_job::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"binding.authentication_expires_at":bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}},
            )
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(gateway, expire)
    })
    .await
    .expect("gateway and cutoff mutation must complete without timing sleeps");
    assert!(
        matches!(result, Err(AppError::MachinePermissionRevoked)),
        "retry must return current expiry denial, never stale success or database_error: {result:?}",
    );
    assert!(
        hook.attempts() >= 2,
        "the changed job must invalidate the first snapshot"
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_retry_rechecks_original_child_revocation() {
    let f = fixture("external_gateway_child_contention").await;
    let (child_id, secret) = child(&f).await;
    let source = source(&f, &secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let persisted = job(&f, &id).await;
    let hook = GatewaySnapshotHook::new(&f.state.db, &persisted);
    let gateway = GATEWAY_SNAPSHOT_HOOK.scope(hook.clone(), gateway_auth(&f.state.db, &persisted));
    let revoke = async {
        hook.wait_for_snapshot().await;
        let mut session = f.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        crate::services::api_key_credential_service::revoke_in_session(
            &f.state.db,
            &child_id,
            crate::models::api_key_credential::CredentialRevokedReason::WebRevoke,
            &mut session,
        )
        .await
        .unwrap()
        .expect("live original child must be revoked by the production producer");
        session.commit_transaction().await.unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(gateway, revoke)
    })
    .await
    .expect("gateway and child revocation must complete without timing sleeps");
    assert!(
        matches!(result, Err(AppError::MachinePermissionRevoked)),
        "retry must deny the original revoked child rather than use its live parent: {result:?}",
    );
    assert!(
        hook.attempts() >= 2,
        "child fence must invalidate the first snapshot"
    );
    assert_eq!(job(&f, &id).await.binding, persisted.binding);
    assert!(
        grants::get_active_for_key(&f.state.db, &f.owner, &f.key.id)
            .await
            .unwrap()
            .is_some(),
        "child revocation must not revoke its parent grant",
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_retry_reloads_current_node_runtime() {
    let f = fixture("external_gateway_runtime_contention").await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let persisted = job(&f, &id).await;
    let hook = GatewaySnapshotHook::new(&f.state.db, &persisted);
    let gateway = GATEWAY_SNAPSHOT_HOOK.scope(hook.clone(), gateway_auth(&f.state.db, &persisted));
    let replace_runtime = async {
        hook.wait_for_snapshot().await;
        f.state
            .db
            .collection::<Document>(node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.node.id},
                doc! {"$set":{"machine.runtime_id":Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(gateway, replace_runtime)
    })
    .await
    .expect("gateway and runtime replacement must complete without timing sleeps");
    assert!(
        matches!(result, Err(AppError::MachinePermissionRevoked)),
        "retry must refuse old-runtime work instead of reusing a cached Node: {result:?}",
    );
    assert!(
        hook.attempts() >= 2,
        "Node fence must invalidate the first snapshot"
    );
    assert_eq!(job(&f, &id).await.runtime_id, persisted.runtime_id);
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

// These races pause the actual mounted service consumer at its first source
// snapshot; competing production calls commit, rather than injecting errors or
// relying on sleeps. Only the tested future inherits the task-local barrier.
#[tokio::test]
async fn external_poll_retry_preserves_original_oauth_source_during_renewal() {
    let f = fixture("poll_retry_oauth_renewal").await;
    let source = gateway_oauth_source(&f).await;
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    let persisted = job(&f, &id).await;
    let hook = grants::SourceSnapshotHook::new(&f.state.db, &f.owner);
    let poll = async {
        dispatch_authorized(
            &f.state,
            &source,
            "nyx__machine_job",
            &mut json!({"machine":f.node.id,"job_id":id}),
        )
        .await
    };
    let competing = async {
        hook.wait().await;
        renew_job_with_source(&f.state, &source, &persisted)
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(
            grants::SOURCE_SNAPSHOT_HOOK.scope(hook.clone(), poll),
            competing
        )
    })
    .await
    .expect("poll and renewal complete without sleeps");
    let result = result.expect("valid original OAuth poll retries source contention");
    assert_eq!(result["stdout"], "bounded test output");
    assert!(hook.reads() >= 2);
    assert_eq!(job(&f, &id).await.binding, persisted.binding);
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRenew
    );
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRenew
    );
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Job);
    assert!(
        requests.try_recv().is_err(),
        "read retry cannot replay Node effects"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_poll_retry_denies_original_oauth_consent_revocation() {
    let f = fixture("poll_retry_oauth_revoke").await;
    let source = gateway_oauth_source(&f).await;
    let ExternalPrincipalRef::OAuth {
        oauth_client_id, ..
    } = source.principal()
    else {
        panic!()
    };
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    requests.recv().await.unwrap();
    let hook = grants::SourceSnapshotHook::new(&f.state.db, &f.owner);
    let poll = async {
        dispatch_authorized(
            &f.state,
            &source,
            "nyx__machine_job",
            &mut json!({"machine":f.node.id,"job_id":id}),
        )
        .await
    };
    let competing = async {
        hook.wait().await;
        crate::services::consent_service::revoke_consent(&f.state.db, &f.owner, &oauth_client_id)
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(
            grants::SOURCE_SNAPSHOT_HOOK.scope(hook.clone(), poll),
            competing
        )
    })
    .await
    .expect("poll and revoke complete without sleeps");
    assert!(
        matches!(result, Err(AppError::MachinePermissionRevoked)),
        "specific live denial, not database_error: {result:?}"
    );
    assert!(
        requests.try_recv().is_err(),
        "revoked poll sends neither renewal nor output request"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

async fn external_read_retry_during_admission(list_machines: bool) {
    let f = fixture(if list_machines {
        "discovery_retry_admission"
    } else {
        "history_retry_admission"
    })
    .await;
    let source = source(&f, &f.secret).await;
    let (peer, mut requests) = peer(&f).await;
    let first = execute(&f, &source).await;
    requests.recv().await.unwrap();
    let hook = grants::SourceSnapshotHook::new(&f.state.db, &f.owner);
    let read = async {
        dispatch_authorized(
            &f.state,
            &source,
            if list_machines {
                "nyx__machine_list"
            } else {
                "nyx__machine_job"
            },
            &mut json!({"machine":f.node.id}),
        )
        .await
    };
    let competing = async {
        hook.wait().await;
        let id = execute(&f, &source).await;
        hook.release();
        id
    };
    let (result, second) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(
            grants::SOURCE_SNAPSHOT_HOOK.scope(hook.clone(), read),
            competing
        )
    })
    .await
    .expect("read and admission complete without sleeps");
    let result = result.expect("live listing retries source write conflict");
    assert_ne!(first, second);
    assert!(hook.reads() >= 2);
    assert_eq!(
        result[if list_machines { "machines" } else { "jobs" }]
            .as_array()
            .unwrap()
            .len(),
        if list_machines { 1 } else { 2 }
    );
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_discovery_retry_during_real_admission() {
    external_read_retry_during_admission(true).await;
}
#[tokio::test]
async fn external_history_retry_during_real_admission() {
    external_read_retry_during_admission(false).await;
}

fn retry_selection(f: &Fixture) -> GrantSelection {
    GrantSelection {
        separation: Some(nyxid_machine::context::Choice::SharedLegacy),
        legacy_warning_acknowledged: true,
        assignments: BTreeMap::from([(
            f.node.id.clone(),
            GrantAssignment {
                capabilities: Capabilities {
                    shell: true,
                    files: true,
                    ..Default::default()
                },
                file_roots: vec!["/workspace".into()],
                service_ids: vec![],
            },
        )]),
        expires_at: None,
    }
}

async fn retry_client(f: &Fixture) -> String {
    crate::services::oauth_client_service::create_client(
        &f.state.db,
        "Preparation contention",
        &["https://example.invalid/callback".into()],
        "public",
        &f.owner,
        "",
        "openid proxy machine offline_access",
        crate::models::oauth_client::ScopeProvenance::Explicit,
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap()
    .0
    .id
}

#[tokio::test]
async fn oauth_prepare_retry_survives_real_admission_with_original_absent_consent() {
    use crate::services::machine_oauth_consent_service as prepared;
    let f = fixture("prepare_retry_admission").await;
    let source = source(&f, &f.secret).await;
    let client = retry_client(&f).await;
    let request = Uuid::new_v4().to_string();
    let selection = retry_selection(&f);
    let (peer, mut requests) = peer(&f).await;
    let hook = grants::SourceSnapshotHook::new(&f.state.db, &f.owner);
    let preparing = prepared::prepare(
        &f.state.db,
        &f.owner,
        &client,
        &request,
        "synthetic-signed-selection",
        &selection,
    );
    let competing = async {
        hook.wait().await;
        execute(&f, &source).await;
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(
            grants::SOURCE_SNAPSHOT_HOOK.scope(hook.clone(), preparing),
            competing
        )
    })
    .await
    .expect("preparation and admission complete without sleeps");
    result.expect("ordinary owner-fence increment does not change consent baseline");
    let saved = f
        .state
        .db
        .collection::<crate::models::machine_oauth_consent_request::MachineOauthConsentRequest>(
            crate::models::machine_oauth_consent_request::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&request})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.previous_consent_fingerprint.is_none());
    assert!(hook.reads() >= 2);
    assert_eq!(requests.recv().await.unwrap().operation, Operation::Exec);
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn oauth_prepare_retry_survives_real_renewal_with_original_consent() {
    use crate::services::{consent_service, machine_oauth_consent_service as prepared};
    let f = fixture("prepare_retry_renewal").await;
    let source = gateway_oauth_source(&f).await;
    let ExternalPrincipalRef::OAuth {
        oauth_client_id: client,
        ..
    } = source.principal()
    else {
        panic!()
    };
    let previous = consent_service::check_consent(&f.state.db, &f.owner, &client, "machine")
        .await
        .unwrap()
        .unwrap();
    let (peer, mut requests) = peer(&f).await;
    let id = execute(&f, &source).await;
    requests.recv().await.unwrap();
    let persisted = job(&f, &id).await;
    let request = Uuid::new_v4().to_string();
    let selection = retry_selection(&f);
    let hook = grants::SourceSnapshotHook::new(&f.state.db, &f.owner);
    let preparing = prepared::prepare(
        &f.state.db,
        &f.owner,
        &client,
        &request,
        "synthetic-signed-selection",
        &selection,
    );
    let competing = async {
        hook.wait().await;
        renew_job_with_source(&f.state, &source, &persisted)
            .await
            .unwrap();
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(
            grants::SOURCE_SNAPSHOT_HOOK.scope(hook.clone(), preparing),
            competing
        )
    })
    .await
    .expect("preparation and renewal complete without sleeps");
    result.expect("renewal fence is not new consent");
    let saved = f
        .state
        .db
        .collection::<crate::models::machine_oauth_consent_request::MachineOauthConsentRequest>(
            crate::models::machine_oauth_consent_request::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&request})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.previous_consent_fingerprint,
        Some(consent_service::fingerprint(&previous).unwrap())
    );
    assert_eq!(job(&f, &id).await.binding, persisted.binding);
    assert!(hook.reads() >= 2);
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRenew
    );
    assert!(requests.try_recv().is_err());
    peer.abort();
    f.state.db.drop().await.unwrap();
}

async fn oauth_prepare_retry_rejects_changed_baseline(change: &str) {
    use crate::services::{
        consent_service, machine_oauth_consent_service as prepared, oauth_client_service,
    };
    let f = fixture(&format!("prepare_retry_{change}")).await;
    let client = if change == "absent_aba" {
        retry_client(&f).await
    } else {
        let source = gateway_oauth_source(&f).await;
        let ExternalPrincipalRef::OAuth {
            oauth_client_id, ..
        } = source.principal()
        else {
            panic!()
        };
        oauth_client_id
    };
    let request = Uuid::new_v4().to_string();
    let selection = retry_selection(&f);
    let hook = grants::SourceSnapshotHook::new(&f.state.db, &f.owner);
    let preparing = prepared::prepare(
        &f.state.db,
        &f.owner,
        &client,
        &request,
        "synthetic-stale-selection",
        &selection,
    );
    let competing = async {
        hook.wait().await;
        match change {
            "revoke" => {
                consent_service::revoke_consent(&f.state.db, &f.owner, &client)
                    .await
                    .unwrap();
            }
            "client_security" => {
                oauth_client_service::update_client_for_creator(
                    &f.state.db,
                    &client,
                    &f.owner,
                    None,
                    Some(&["https://example.invalid/new-callback".into()]),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
            }
            "absent_aba" => {
                consent_service::grant_consent(&f.state.db, &f.owner, &client, "openid proxy")
                    .await
                    .unwrap();
                consent_service::revoke_consent(&f.state.db, &f.owner, &client)
                    .await
                    .unwrap();
            }
            _ => panic!("unknown change"),
        }
        hook.release();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(
            grants::SOURCE_SNAPSHOT_HOOK.scope(hook.clone(), preparing),
            competing
        )
    })
    .await
    .expect("preparation and security mutation complete without sleeps");
    assert!(
        matches!(result, Err(AppError::Conflict(_))),
        "stale selection must return consent conflict rather than rebase or database_error: {result:?}"
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::machine_oauth_consent_request::COLLECTION_NAME)
            .count_documents(doc! {"_id":&request})
            .await
            .unwrap(),
        0
    );
    assert!(
        grants::get_active_for_key(&f.state.db, &f.owner, &f.key.id)
            .await
            .unwrap()
            .is_some(),
        "selection staleness must not revoke unrelated grants"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn oauth_prepare_retry_denies_real_consent_revocation() {
    oauth_prepare_retry_rejects_changed_baseline("revoke").await;
}
#[tokio::test]
async fn oauth_prepare_retry_denies_real_client_security_change() {
    oauth_prepare_retry_rejects_changed_baseline("client_security").await;
}
#[tokio::test]
async fn oauth_prepare_retry_denies_absent_approved_revoked_aba() {
    oauth_prepare_retry_rejects_changed_baseline("absent_aba").await;
}
