use super::*;
use crate::{
    models::{
        node::{COLLECTION_NAME as NODES, Node},
        remote_machine::{AuthorizationState, NativeRuntimeObservation, ResourceState},
        user::UserType,
    },
    services::remote_machine_service as lifecycle,
    test_utils::{connect_transaction_test_database, test_encryption_keys, test_user},
};

struct TestWorkloadIdentity;
#[async_trait::async_trait]
impl BootstrapTicketVerifier for TestWorkloadIdentity {
    async fn verify(
        &self,
        ticket: &Value,
        binding: &Binding,
    ) -> AppResult<VerifiedBootstrapTicket> {
        if ticket["test_workload_identity"] != "authenticated_fixture" {
            return Err(AppError::Unauthorized(
                "test workload identity rejects ticket".into(),
            ));
        }
        Ok(VerifiedBootstrapTicket::from_provider_verified_claims(
            binding.clone(),
            hex::decode(ticket["recipient_key_digest"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
            Utc::now() + chrono::Duration::minutes(5),
        ))
    }
}

fn sample_machine() -> (ManagedMachine, Binding) {
    let id = uuid::Uuid::new_v4().to_string();
    let owner = uuid::Uuid::new_v4().to_string();
    let mut machine =
        lifecycle::CreateMachineInput::fixed(&owner, true, "test-image").preview(&id, Utc::now());
    let binding = Binding {
        node_id: id,
        sandbox_uid: uuid::Uuid::new_v4().to_string(),
        pod_uid: uuid::Uuid::new_v4().to_string(),
        runtime_generation: 1,
        credential_epoch: 1,
        writer_fence: uuid::Uuid::new_v4().to_string(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        delivery_id: uuid::Uuid::new_v4().to_string(),
    };
    machine.sandbox_uid = Some(binding.sandbox_uid.clone());
    machine.pod_uid = Some(binding.pod_uid.clone());
    machine.pvc_uid = Some(uuid::Uuid::new_v4().to_string());
    machine.active_operation_id = Some(binding.operation_id.clone());
    machine.runtime_generation = 1;
    machine.credential_epoch = 1;
    machine.writer_fence = binding.writer_fence.clone();
    machine.resource_state = ResourceState::Ready;
    machine.writer_admitted = true;
    machine.authorization_state = AuthorizationState::Active;
    let now = chrono::DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
    let cutoff = now + chrono::Duration::minutes(2);
    machine.native_run_not_after = Some(cutoff);
    machine.native_observation = Some(NativeRuntimeObservation {
        sandbox_uid: binding.sandbox_uid.clone(),
        pod_uid: Some(binding.pod_uid.clone()),
        runtime_generation: 1,
        credential_epoch: 1,
        writer_fence: binding.writer_fence.clone(),
        operation_id: binding.operation_id.clone(),
        sandbox_generation: 1,
        sandbox_resource_version: "native-bootstrap-fixture-rv".into(),
        observed_at: now,
        running: true,
        suspended: false,
        stop_confirmed: false,
        run_not_after: Some(cutoff),
        evidence_ref: "kubernetes-native-bootstrap-fixture".into(),
    });
    (machine, binding)
}

pub(crate) async fn fixture() -> (
    Database,
    EncryptionKeys,
    Binding,
    nyxid_machine::bootstrap::RecipientKey,
) {
    let db = connect_transaction_test_database("managed_bootstrap").await;
    ensure_indexes(&db).await.unwrap();
    let (machine, binding) = sample_machine();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&machine.owner_id, UserType::Person))
        .await
        .unwrap();
    db.collection::<ManagedMachine>(MACHINES)
        .insert_one(&machine)
        .await
        .unwrap();
    let node = Node {
        id: machine.node_id.clone(),
        user_id: machine.owner_id.clone(),
        name: machine.sandbox_name.clone(),
        status: crate::models::node::NodeStatus::Offline,
        auth_token_hash: "reserved-auth".into(),
        signing_secret_encrypted: None,
        signing_secret_context_version: None,
        signing_secret_hash: "reserved-signing".into(),
        last_heartbeat_at: None,
        connected_at: None,
        metadata: Some(crate::models::node::NodeMetadata {
            agent_version: None,
            os: Some("linux".into()),
            arch: None,
            ip_address: None,
            provisioning_source: Some("remote_machine".into()),
        }),
        metrics: Default::default(),
        connection_owner: None,
        machine: Some(Default::default()),
        machine_confirm: Default::default(),
        allow_single_user_saved_logins: false,
        is_active: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.collection::<Node>(NODES).insert_one(node).await.unwrap();
    (
        db,
        test_encryption_keys(),
        binding,
        nyxid_machine::bootstrap::generate_recipient().unwrap(),
    )
}

fn request(binding: &Binding, keypair: &nyxid_machine::bootstrap::RecipientKey) -> BootstrapInput {
    BootstrapInput {
        binding: binding.clone(),
        ticket: serde_json::json!({"test_workload_identity":"authenticated_fixture", "recipient_key_digest": hex::encode(nyxid_machine::bootstrap::recipient_key_digest(&keypair.public_key_b64u().unwrap()).unwrap())}),
        recipient_public_key: keypair.public_key_b64u().unwrap(),
    }
}

#[test]
fn binding_checks_every_identity_and_runtime_fence() {
    let (machine, binding) = sample_machine();
    validate_binding(&machine, &binding).unwrap();
    for field in [
        "node_id",
        "sandbox_uid",
        "pod_uid",
        "runtime_generation",
        "credential_epoch",
        "writer_fence",
        "operation_id",
    ] {
        let mut changed = serde_json::to_value(&binding).unwrap();
        changed[field] = if field.ends_with("generation") || field.ends_with("epoch") {
            serde_json::json!(2)
        } else {
            serde_json::json!(uuid::Uuid::new_v4().to_string())
        };
        let changed: Binding = serde_json::from_value(changed).unwrap();
        assert!(validate_binding(&machine, &changed).is_err(), "{field}");
    }
    for profile in ["remote-shell-files-v1", "remote-shell-files-v2"] {
        let mut obsolete = machine.clone();
        obsolete.profile.profile_version = profile.into();
        assert!(validate_binding(&obsolete, &binding).is_err());
    }
    let mut delivered = machine.clone();
    delivered.bootstrap_delivery_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(validate_binding(&delivered, &binding).is_err());
}

#[test]
fn ready_flags_without_current_native_observation_never_admit_writer() {
    let (mut machine, binding) = sample_machine();
    machine
        .native_observation
        .as_mut()
        .unwrap()
        .evidence_ref
        .clear();
    assert!(validate_binding(&machine, &binding).is_err());
    machine.native_observation.as_mut().unwrap().evidence_ref = "native-fixture".into();
    machine
        .native_observation
        .as_mut()
        .unwrap()
        .credential_epoch += 1;
    assert!(validate_binding(&machine, &binding).is_err());
    machine.native_observation = None;
    assert!(validate_binding(&machine, &binding).is_err());
}

#[test]
fn native_admission_can_bootstrap_before_wss_ready_but_never_without_writer_fence() {
    let (mut machine, binding) = sample_machine();
    for state in [
        ResourceState::Provisioning,
        ResourceState::Resuming,
        ResourceState::Reconciling,
    ] {
        machine.resource_state = state;
        assert!(validate_binding(&machine, &binding).is_ok());
    }
    machine.writer_admitted = false;
    assert!(validate_binding(&machine, &binding).is_err());
}

#[tokio::test]
async fn unconfigured_identity_verifier_rejects_caller_claims_without_database_access() {
    let (_, binding) = sample_machine();
    let result = UnavailableBootstrapTicketVerifier
        .verify(&serde_json::json!({"verified":true}), &binding)
        .await;
    assert!(matches!(result, Err(AppError::NodeOffline(_))));
}

#[tokio::test]
async fn lost_response_replays_one_sealed_delivery_and_stable_node_credentials() {
    let (db, keys, binding, keypair) = fixture().await;
    let first = bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    assert!(!first.replayed);
    let second = bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    assert!(second.replayed);
    assert_eq!(first.sealed, second.sealed);
    let plaintext = nyxid_machine::bootstrap::open(&first.sealed, &keypair, &binding).unwrap();
    let secrets: Value = serde_json::from_slice(&plaintext).unwrap();
    let auth = secrets["auth_token"].as_str().unwrap();
    let signing = secrets["signing_secret"].as_str().unwrap();
    let node = node_service::validate_auth_token(&db, auth).await.unwrap();
    assert_eq!(node.id, binding.node_id);
    assert_eq!(node.signing_secret_hash, hash_token(signing));
    assert_eq!(
        node_service::signing_secret_from_node(&keys, &node)
            .await
            .unwrap()
            .as_slice(),
        hex::decode(signing).unwrap().as_slice()
    );
    assert_eq!(
        db.collection::<bson::Document>(NODES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<bson::Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let row = db
        .collection::<ManagedBootstrapDelivery>(DELIVERIES)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let stored = bson::to_document(&row).unwrap().to_string();
    let debug = format!("{row:?}");
    for serialized in [stored, debug] {
        assert!(!serialized.contains(auth));
        assert!(!serialized.contains(signing));
    }
    let audit = db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"managed_machine_credentials_delivered"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        audit
            .get("user_id")
            .is_none_or(|value| matches!(value, bson::Bson::Null))
    );
    assert!(audit.contains_key("seq"));
    assert!(audit.contains_key("entry_hash"));
    let audit_text = audit.to_string();
    assert!(!audit_text.contains(auth));
    assert!(!audit_text.contains(signing));
    assert!(!audit_text.contains(&keypair.public_key_b64u().unwrap()));
    let mut wrong_binding = binding.clone();
    wrong_binding.delivery_id = uuid::Uuid::new_v4().to_string();
    assert!(nyxid_machine::bootstrap::open(&first.sealed, &keypair, &wrong_binding).is_err());
}

#[tokio::test]
async fn concurrent_bootstrap_retries_issue_exactly_one_credential_set() {
    let (db, keys, binding, keypair) = fixture().await;
    let (a, b) = tokio::join!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        ),
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.sealed, b.sealed);
    assert_ne!(a.replayed, b.replayed);
    assert_eq!(
        db.collection::<bson::Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn changed_recipient_delivery_generation_and_pause_cannot_replay_credentials() {
    let (db, keys, binding, keypair) = fixture().await;
    bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    let other = nyxid_machine::bootstrap::generate_recipient().unwrap();
    assert!(
        bootstrap(&db, &keys, &TestWorkloadIdentity, request(&binding, &other))
            .await
            .is_err()
    );
    let mut changed = binding.clone();
    changed.delivery_id = uuid::Uuid::new_v4().to_string();
    assert!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&changed, &keypair)
        )
        .await
        .is_err()
    );
    changed = binding.clone();
    changed.runtime_generation += 1;
    assert!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&changed, &keypair)
        )
        .await
        .is_err()
    );
    db.collection::<ManagedMachine>(MACHINES)
        .update_one(
            doc! {"_id": &binding.node_id},
            doc! {"$set":{"resource_state":"stopping"},"$inc":{"revision":1_i64}},
        )
        .await
        .unwrap();
    assert!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn missing_reserved_node_rolls_back_delivery_and_machine_activation() {
    let (db, keys, binding, keypair) = fixture().await;
    db.collection::<Node>(NODES)
        .delete_one(doc! {"_id": &binding.node_id})
        .await
        .unwrap();
    assert!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
        .await
        .is_err()
    );
    let machine = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id": &binding.node_id})
        .await
        .unwrap()
        .unwrap();
    assert!(machine.bootstrap_delivery_id.is_none());
    assert_eq!(
        db.collection::<bson::Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn expired_envelope_requires_rekey_and_never_deletes_workspace_or_identity() {
    let (db, keys, binding, keypair) = fixture().await;
    bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    // A fresh workload identity ticket cannot extend the original sealed delivery.
    db.collection::<bson::Document>(DELIVERIES)
        .update_one(
            doc! {"_id":&binding.delivery_id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}},
        )
        .await
        .unwrap();
    assert!(matches!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    // Exercise TTL-after-deletion behavior without waiting for a server sweep.
    db.collection::<bson::Document>(DELIVERIES)
        .delete_one(doc! {"_id":&binding.delivery_id})
        .await
        .unwrap();
    assert!(matches!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let machine = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":&binding.node_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        machine.sandbox_uid.as_deref(),
        Some(binding.sandbox_uid.as_str())
    );
    assert_eq!(
        machine.bootstrap_delivery_id.as_deref(),
        Some(binding.delivery_id.as_str())
    );
    assert_eq!(
        db.collection::<Node>(NODES)
            .count_documents(doc! {"_id":&binding.node_id})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn owner_restriction_or_deactivation_blocks_new_and_replayed_bootstrap() {
    let (db, keys, binding, keypair) = fixture().await;
    let users = db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME);
    users
        .update_one(doc! {}, doc! {"$set":{"machine_restricted":true}})
        .await
        .unwrap();
    assert!(matches!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        db.collection::<bson::Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    users
        .update_one(doc! {}, doc! {"$set":{"machine_restricted":false}})
        .await
        .unwrap();
    bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    users
        .update_one(doc! {}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(matches!(
        bootstrap(
            &db,
            &keys,
            &TestWorkloadIdentity,
            request(&binding, &keypair)
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        db.collection::<bson::Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn workload_ticket_recipient_is_checked_before_first_activation() {
    let (db, keys, binding, keypair) = fixture().await;
    let mut input = request(&binding, &keypair);
    let other = nyxid_machine::bootstrap::generate_recipient().unwrap();
    input.recipient_public_key = other.public_key_b64u().unwrap();
    assert!(matches!(
        bootstrap(&db, &keys, &TestWorkloadIdentity, input).await,
        Err(AppError::Unauthorized(_))
    ));
    assert_eq!(
        db.collection::<bson::Document>(DELIVERIES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id": &binding.node_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!node.is_active);
    assert_eq!(node.auth_token_hash, "reserved-auth");
    let result = bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    assert!(!result.replayed);
}

#[tokio::test]
async fn sealed_bootstrap_carries_selected_browser_computer_capability_ceiling() {
    let (db, keys, binding, keypair) = fixture().await;
    let capabilities = vec!["browser", "computer", "files"];
    db.collection::<mongodb::bson::Document>(MACHINES)
        .update_one(
            doc! {"_id":&binding.node_id},
            doc! {"$set":{"capabilities":&capabilities}},
        )
        .await
        .unwrap();
    let result = bootstrap(
        &db,
        &keys,
        &TestWorkloadIdentity,
        request(&binding, &keypair),
    )
    .await
    .unwrap();
    let plaintext = nyxid_machine::bootstrap::open(&result.sealed, &keypair, &binding).unwrap();
    let payload: Value = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(payload["capabilities"], serde_json::json!(capabilities));
    assert_eq!(result.binding, binding);
    db.drop().await.unwrap();
}
