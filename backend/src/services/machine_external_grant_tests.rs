//! Source-bound grant contracts against isolated MongoDB replica-set databases.
use super::*;
use crate::{
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS},
        api_key_credential::{
            ApiKeyCredential, COLLECTION_NAME as CREDENTIALS, CredentialRevokedReason,
        },
        machine_external_grant::{COLLECTION_NAME as GRANTS, MachineExternalGrant},
        node::{COLLECTION_NAME as NODES, Node, NodeStatus},
        user::{COLLECTION_NAME as USERS, User, UserType},
        user_service::{COLLECTION_NAME as USER_SERVICES, UserService},
    },
    test_utils::{connect_transaction_test_database, test_user, test_user_service},
};
use chrono::{Duration, SecondsFormat, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use nyxid_machine::{MachineProfile, authority::Capabilities};
use std::collections::BTreeMap;
use uuid::Uuid;

struct Fixture {
    db: Database,
    user: String,
    key: ApiKey,
    node: Node,
}

impl Fixture {
    fn selection(&self) -> GrantSelection {
        GrantSelection {
            separation: Some(nyxid_machine::context::Choice::SharedLegacy),
            legacy_warning_acknowledged: true,
            assignments: BTreeMap::from([(
                self.node.id.clone(),
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

    async fn approve(&self) -> MachineExternalGrant {
        let key = self.current_key().await;
        approve_api_key(
            &self.db,
            &self.user,
            &key.id,
            key.state_version,
            &self.selection(),
        )
        .await
        .expect("approve eligible parent key")
    }

    async fn current_key(&self) -> ApiKey {
        self.db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! {"_id": &self.key.id})
            .await
            .unwrap()
            .unwrap()
    }

    async fn grant(&self, id: &str) -> MachineExternalGrant {
        self.db
            .collection::<MachineExternalGrant>(GRANTS)
            .find_one(doc! {"_id": id})
            .await
            .unwrap()
            .unwrap()
    }
}

async fn fixture(label: &str) -> Fixture {
    let db = connect_transaction_test_database(label).await;
    create_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let key: ApiKey = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(),
        "user_id": &user,
        "name": "Shared display label",
        "key_prefix": "test-only",
        "key_hash": "00".repeat(32),
        "scopes": "proxy",
        "is_active": true,
        "state_version": 1_i64,
        "created_at": bson::DateTime::now(),
        "allow_all_nodes": true,
        "allow_all_services": true,
    })
    .unwrap();
    db.collection::<ApiKey>(API_KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    let node = Node {
        id: Uuid::new_v4().to_string(),
        user_id: user.clone(),
        name: "Grant fixture".into(),
        status: NodeStatus::Online,
        auth_token_hash: "11".repeat(32),
        signing_secret_encrypted: None,
        signing_secret_context_version: None,
        signing_secret_hash: "22".repeat(32),
        last_heartbeat_at: None,
        connected_at: None,
        metadata: None,
        metrics: Default::default(),
        connection_owner: None,
        machine: Some(MachineProfile {
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
        }),
        machine_confirm: Default::default(),
        allow_single_user_saved_logins: false,
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.collection::<Node>(NODES)
        .insert_one(&node)
        .await
        .unwrap();
    Fixture {
        db,
        user,
        key,
        node,
    }
}

fn assert_status(grant: &MachineExternalGrant, status: &str) {
    assert_eq!(serde_json::to_value(grant).unwrap()["status"], status);
}

#[test]
fn oauth_binding_tag_matches_database_selector_and_partial_index() {
    let binding = crate::models::machine_external_grant::ExternalGrantBinding::OAuth {
        oauth_client_id: Uuid::new_v4().to_string(),
        consent_id: Uuid::new_v4().to_string(),
        consent_revision: Uuid::new_v4().to_string(),
    };
    let document = bson::to_document(&binding).unwrap();
    assert_eq!(document.get_str("kind").unwrap(), "oauth");
}

async fn admit(
    f: &Fixture,
    source: &VerifiedExternalSource,
    expected: Option<(&str, i64)>,
) -> AppResult<AdmittedGrant> {
    let mut session = f.db.client().start_session().await?;
    session.start_transaction().await?;
    let result = admit_in_session(&f.db, source, &f.node.id, expected, &mut session).await;
    if result.is_ok() {
        session.commit_transaction().await?;
    } else {
        session.abort_transaction().await?;
    }
    result
}

async fn child(f: &Fixture) -> ApiKeyCredential {
    let row: ApiKeyCredential = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(),
        "api_key_id": &f.key.id,
        "user_id": &f.user,
        "secret_hash": "33".repeat(32),
        "secret_prefix": "fixture",
        "label": "Fixture login",
        "login_request_id": Uuid::new_v4().to_string(),
        "is_active": true,
        "created_at": bson::DateTime::now(),
    })
    .unwrap();
    f.db.collection::<ApiKeyCredential>(CREDENTIALS)
        .insert_one(&row)
        .await
        .unwrap();
    row
}

#[tokio::test]
async fn external_grant_defaults_are_explicit_and_expiry_is_bounded() {
    let f = fixture("external_grant_defaults").await;
    let before = Utc::now();
    let grant = f.approve().await;
    let after = Utc::now();
    assert_status(&grant, "active");
    assert_eq!(grant.user_id, f.user);
    assert_eq!(grant.revision, 1);
    assert!(Uuid::parse_str(&grant.id).is_ok());
    assert!(grant.expires_at >= before + Duration::days(30) - Duration::milliseconds(1));
    assert!(grant.expires_at <= after + Duration::days(30));
    let assignment = &grant.policy.assignments[&f.node.id];
    assert_eq!(assignment.mode, "shared_legacy");
    assert_eq!(assignment.saved_login_ids, Some(vec![]));
    assert!(!assignment.legacy);
    assert!(assignment.capabilities.shell && assignment.capabilities.files);
    assert!(!assignment.capabilities.browser);
    assert!(!assignment.capabilities.computer);
    assert!(!assignment.capabilities.developer_browser);
    assert_eq!(
        grant.restrictions[&f.node.id].file_roots,
        vec!["/workspace"]
    );
    assert!(grant.restrictions[&f.node.id].service_ids.is_empty());

    let stored =
        f.db.collection::<bson::Document>(GRANTS)
            .find_one(doc! {"_id": &grant.id})
            .await
            .unwrap()
            .unwrap();
    assert!(stored.get_datetime("expires_at").is_ok());
    assert!(stored.get_datetime("created_at").is_ok());
    assert!(stored.get_datetime("updated_at").is_ok());
}

#[tokio::test]
async fn active_unique_index_preserves_history_and_reauthorization_uses_new_id() {
    let f = fixture("external_grant_unique").await;
    let original = f.approve().await;
    let mut duplicate = original.clone();
    duplicate.id = Uuid::new_v4().to_string();
    assert!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .insert_one(&duplicate)
            .await
            .is_err(),
        "database must enforce active uniqueness"
    );
    let revoked = revoke(&f.db, &f.user, &original.id, original.revision)
        .await
        .unwrap();
    assert_status(&revoked, "revoked");
    assert_eq!(revoked.revision, original.revision + 1);

    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection(),
        )
        .await
        .is_err(),
        "an old approval body cannot recreate authority after revocation"
    );
    assert!(f.current_key().await.state_version > f.key.state_version);

    let replacement = f.approve().await;
    assert_ne!(replacement.id, original.id);
    assert_eq!(replacement.revision, 1);
    assert_status(&f.grant(&original.id).await, "revoked");
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {"status": "active"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn human_handlers_reject_stale_grant_id_after_reauthorization_resets_revision() {
    use crate::handlers::machine_external_grant::{self as handlers, GrantRevoke, KeyGrantPut};
    use axum::{
        Json,
        extract::{Path, State},
    };

    let f = fixture("external_grant_handler_aba").await;
    let old = f.approve().await;
    revoke(&f.db, &f.user, &old.id, old.revision).await.unwrap();
    let current = f.approve().await;
    assert_ne!(current.id, old.id);
    assert_eq!(current.revision, old.revision);
    let state = crate::test_utils::test_app_state(f.db.clone());
    let auth = crate::test_utils::test_auth_user(&f.user);
    let key = f.current_key().await;
    let mut altered = f.selection();
    altered.assignments.get_mut(&f.node.id).unwrap().file_roots =
        vec!["/workspace/narrowed".into()];

    let stale_edit = handlers::put_key(
        State(state.clone()),
        auth.clone(),
        Path(f.key.id.clone()),
        Json(KeyGrantPut {
            expected_grant_id: Some(old.id.clone()),
            expected_revision: Some(old.revision),
            // Use the current source version so only the old grant identity
            // distinguishes this stale edit from a valid edit of the successor.
            expected_state_version: key.state_version,
            selection: altered,
        }),
    )
    .await;
    assert!(matches!(stale_edit, Err(AppError::Conflict(_))));

    let stale_revoke = handlers::revoke_key(
        State(state),
        auth,
        Path(f.key.id.clone()),
        Json(GrantRevoke {
            expected_grant_id: old.id.clone(),
            expected_revision: old.revision,
        }),
    )
    .await;
    assert!(matches!(stale_revoke, Err(AppError::Conflict(_))));
    assert_eq!(
        bson::to_document(&f.grant(&current.id).await).unwrap(),
        bson::to_document(&current).unwrap(),
        "stale edit and revoke must leave the new active contract untouched"
    );
    assert_eq!(f.current_key().await.state_version, key.state_version);
    assert_status(&f.grant(&old.id).await, "revoked");
}

#[tokio::test]
async fn concurrent_approval_has_one_winner_without_orphan_grant() {
    let f = fixture("external_grant_approval_race").await;
    let selection = f.selection();
    let (left, right) = tokio::join!(
        approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection),
        approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {"status": "active"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn approval_requires_current_key_state_but_not_labels_as_identity() {
    let f = fixture("external_grant_approval_cas").await;
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {"$inc": {"state_version": 1_i64}},
        )
        .await
        .unwrap();
    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err(),
        "stale human approval must not approve a changed key"
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let first = f.approve().await;

    let mut other = f.key.clone();
    other.id = Uuid::new_v4().to_string();
    // Same name/platform cannot cause adoption of an existing parent's grant.
    f.db.collection::<ApiKey>(API_KEYS)
        .insert_one(&other)
        .await
        .unwrap();
    let second = approve_api_key(
        &f.db,
        &f.user,
        &other.id,
        other.state_version,
        &f.selection(),
    )
    .await
    .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {"status": "active"})
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn approval_rejects_wrong_owner_restricted_purpose_and_dead_sources() {
    let f = fixture("external_grant_source_eligibility").await;
    let other_user = Uuid::new_v4().to_string();
    f.db.collection::<User>(USERS)
        .insert_one(test_user(&other_user, UserType::Person))
        .await
        .unwrap();
    VerifiedExternalSource::from_verified_api_key(&f.key, None)
        .expect("the General fixture must be an eligible external source");
    type KeyMutation = Box<dyn Fn(&mut ApiKey)>;
    let cases: Vec<(&str, KeyMutation)> = vec![
        (
            "scheduled key",
            Box::new(|key| key.purpose = ApiKeyPurpose::ScheduledInvocation),
        ),
        (
            "permission-bound key",
            Box::new(|key| key.purpose = ApiKeyPurpose::PermissionBound),
        ),
        (
            "public NyxBot key with wide scopes and machine marker",
            Box::new(|key| {
                key.purpose = ApiKeyPurpose::PublicNyxbot;
                key.scopes = "proxy proxy:all llm:proxy account:read admin machine".into();
                key.machine_external_grants_present = true;
            }),
        ),
        (
            "assistant key",
            Box::new(|key| key.assistant_agent_owner_id = Some(Uuid::new_v4().to_string())),
        ),
        ("inactive key", Box::new(|key| key.is_active = false)),
        (
            "expired key",
            Box::new(|key| key.expires_at = Some(Utc::now() - Duration::seconds(1))),
        ),
        (
            "read-only scope",
            Box::new(|key| key.scopes = "account:read mcp:catalog:read".into()),
        ),
    ];
    for (label, mutate) in cases {
        let mut key = f.key.clone();
        key.id = Uuid::new_v4().to_string();
        mutate(&mut key);
        f.db.collection::<ApiKey>(API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        assert!(
            matches!(
                VerifiedExternalSource::from_verified_api_key(&key, None),
                Err(AppError::MachineNotAllowed)
            ),
            "ineligible source constructed: {label}"
        );
        assert!(
            matches!(
                approve_api_key(&f.db, &f.user, &key.id, key.state_version, &f.selection()).await,
                Err(AppError::MachineNotAllowed)
            ),
            "ineligible source accepted: {label}"
        );
    }
    assert!(
        approve_api_key(
            &f.db,
            &other_user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err()
    );
    f.db.collection::<User>(USERS)
        .update_one(doc! {"_id": &f.user}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn approval_rejects_org_owner_foreign_nodes_and_parent_node_ceiling() {
    let f = fixture("external_grant_owner_node").await;
    let original = f.node.clone();
    let mut foreign = original.clone();
    foreign.user_id = Uuid::new_v4().to_string();
    f.db.collection::<Node>(NODES)
        .replace_one(doc! {"_id": &original.id}, &foreign)
        .await
        .unwrap();
    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err()
    );
    f.db.collection::<Node>(NODES)
        .replace_one(doc! {"_id": &original.id}, &original)
        .await
        .unwrap();
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {
                "$set": {"allow_all_nodes": false, "allowed_node_ids": [Uuid::new_v4().to_string()]}
            },
        )
        .await
        .unwrap();
    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err(),
        "node routing ceiling remains authoritative"
    );
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {
                "$set": {"allowed_node_ids": [&original.id]}
            },
        )
        .await
        .unwrap();
    f.db.collection::<User>(USERS)
        .replace_one(doc! {"_id": &f.user}, test_user(&f.user, UserType::Org))
        .await
        .unwrap();
    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err(),
        "external MVP is personal-only"
    );
}

#[tokio::test]
async fn approval_rejects_outside_roots_unavailable_capabilities_and_expired_selection() {
    let f = fixture("external_grant_roots_caps").await;
    for roots in [
        vec![],
        vec!["/etc"],
        vec!["/workspace-other"],
        vec!["/workspace/../etc"],
        vec!["relative"],
    ] {
        let mut selection = f.selection();
        selection
            .assignments
            .get_mut(&f.node.id)
            .unwrap()
            .file_roots = roots.iter().map(|root| (*root).to_string()).collect();
        assert!(
            approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
                .await
                .is_err(),
            "invalid roots accepted: {roots:?}"
        );
    }
    for capability in ["browser", "computer", "developer_browser"] {
        let mut selection = f.selection();
        let capabilities = &mut selection
            .assignments
            .get_mut(&f.node.id)
            .unwrap()
            .capabilities;
        match capability {
            "browser" => capabilities.browser = true,
            "computer" => capabilities.computer = true,
            _ => capabilities.developer_browser = true,
        }
        assert!(
            approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
                .await
                .is_err(),
            "unsupported external capability accepted: {capability}"
        );
    }
    let mut selection = f.selection();
    selection.expires_at = Some(Utc::now() - Duration::seconds(1));
    assert!(
        approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
            .await
            .is_err()
    );
    f.db.collection::<Node>(NODES)
        .update_one(
            doc! {"_id": &f.node.id},
            doc! {
                "$set": {"machine.shell": false}
            },
        )
        .await
        .unwrap();
    assert!(
        approve_api_key(
            &f.db,
            &f.user,
            &f.key.id,
            f.key.state_version,
            &f.selection()
        )
        .await
        .is_err(),
        "grant cannot enable a locally disabled capability"
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn service_selection_requires_owner_resource_and_parent_service_ceiling() {
    let f = fixture("external_grant_services").await;
    let service_id = Uuid::new_v4().to_string();
    let mut selection = f.selection();
    selection
        .assignments
        .get_mut(&f.node.id)
        .unwrap()
        .service_ids = vec![service_id.clone()];
    assert!(
        approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
            .await
            .is_err(),
        "unknown service must not be approved"
    );
    let mut service = test_user_service(
        &service_id,
        &Uuid::new_v4().to_string(),
        "grant-fixture",
        &Uuid::new_v4().to_string(),
        None,
        None,
    );
    f.db.collection::<UserService>(USER_SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    assert!(
        approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
            .await
            .is_err(),
        "foreign owner service must not be approved"
    );
    service.user_id = f.user.clone();
    f.db.collection::<UserService>(USER_SERVICES)
        .replace_one(doc! {"_id": &service_id}, &service)
        .await
        .unwrap();
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {
                "$set": {"allow_all_services": false, "allowed_service_ids": []}
            },
        )
        .await
        .unwrap();
    assert!(
        approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
            .await
            .is_err(),
        "Machine grant must not widen source service ceiling"
    );
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {
                "$set": {"allowed_service_ids": [&service_id]}
            },
        )
        .await
        .unwrap();
    let grant = approve_api_key(&f.db, &f.user, &f.key.id, f.key.state_version, &selection)
        .await
        .unwrap();
    assert_eq!(grant.restrictions[&f.node.id].service_ids, vec![service_id]);
}

#[tokio::test]
async fn grant_edit_uses_own_monotonic_cas_and_revoke_cannot_be_undone() {
    let f = fixture("external_grant_cas").await;
    let original = f.approve().await;
    let mut selection = f.selection();
    selection
        .assignments
        .get_mut(&f.node.id)
        .unwrap()
        .file_roots = vec!["/workspace/project".into()];
    let edited = edit(&f.db, &f.user, &original.id, original.revision, &selection)
        .await
        .unwrap();
    assert_eq!(edited.revision, original.revision + 1);
    assert!(
        edit(
            &f.db,
            &f.user,
            &original.id,
            original.revision,
            &f.selection()
        )
        .await
        .is_err()
    );
    assert!(
        revoke(&f.db, &f.user, &original.id, original.revision)
            .await
            .is_err()
    );
    assert!(
        edit(
            &f.db,
            &Uuid::new_v4().to_string(),
            &original.id,
            edited.revision,
            &selection
        )
        .await
        .is_err()
    );
    assert!(
        revoke(
            &f.db,
            &Uuid::new_v4().to_string(),
            &original.id,
            edited.revision
        )
        .await
        .is_err()
    );

    let revoked = revoke(&f.db, &f.user, &original.id, edited.revision)
        .await
        .unwrap();
    assert_eq!(revoked.revision, edited.revision + 1);
    assert_status(&revoked, "revoked");
    assert!(
        edit(&f.db, &f.user, &original.id, revoked.revision, &selection)
            .await
            .is_err()
    );
    assert_status(&f.grant(&original.id).await, "revoked");
}

#[tokio::test]
async fn source_validity_rejects_snapshots_stale_after_edit_or_revoke() {
    let f = fixture("external_grant_snapshot").await;
    let original = f.approve().await;
    assert!(source_and_grant_valid(&f.db, &original).await.unwrap());

    let mut selection = f.selection();
    selection
        .assignments
        .get_mut(&f.node.id)
        .unwrap()
        .file_roots = vec!["/workspace/project".into()];
    let edited = edit(&f.db, &f.user, &original.id, original.revision, &selection)
        .await
        .unwrap();
    assert!(!source_and_grant_valid(&f.db, &original).await.unwrap());
    assert!(source_and_grant_valid(&f.db, &edited).await.unwrap());

    let revoked = revoke(&f.db, &f.user, &edited.id, edited.revision)
        .await
        .unwrap();
    assert!(!source_and_grant_valid(&f.db, &edited).await.unwrap());
    assert!(!source_and_grant_valid(&f.db, &revoked).await.unwrap());
}

#[tokio::test]
async fn edit_and_revoke_race_cannot_commit_two_changes_at_one_revision() {
    let f = fixture("external_grant_edit_race").await;
    let grant = f.approve().await;
    let mut selection = f.selection();
    selection
        .assignments
        .get_mut(&f.node.id)
        .unwrap()
        .capabilities
        .files = false;
    let (edited, revoked) = tokio::join!(
        edit(&f.db, &f.user, &grant.id, grant.revision, &selection),
        revoke(&f.db, &f.user, &grant.id, grant.revision),
    );
    assert_eq!(
        usize::from(edited.is_ok()) + usize::from(revoked.is_ok()),
        1
    );
    let current = f.grant(&grant.id).await;
    assert_eq!(current.revision, grant.revision + 1);
    if revoked.is_ok() {
        assert_status(&current, "revoked");
    } else {
        assert_status(&current, "active");
        assert!(!current.policy.assignments[&f.node.id].capabilities.files);
    }
}

#[tokio::test]
async fn rotated_successor_cannot_inherit_predecessor_grant() {
    let f = fixture("external_grant_successor").await;
    let original = f.approve().await;
    let mut successor = f.current_key().await;
    successor.id = Uuid::new_v4().to_string();
    successor.rotation_predecessor_id = Some(f.key.id.clone());
    successor.state_version = 1;
    f.db.collection::<ApiKey>(API_KEYS)
        .insert_one(&successor)
        .await
        .unwrap();
    let successor_source = VerifiedExternalSource::from_verified_api_key(&successor, None).unwrap();
    assert!(admit(&f, &successor_source, None).await.is_err());
    assert!(
        admit(
            &f,
            &successor_source,
            Some((&original.id, original.revision))
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {
                "status": "active", "binding.api_key_id": &successor.id,
            })
            .await
            .unwrap(),
        0
    );
    let successor_grant = approve_api_key(
        &f.db,
        &f.user,
        &successor.id,
        successor.state_version,
        &f.selection(),
    )
    .await
    .unwrap();
    assert_ne!(successor_grant.id, original.id);
    assert_eq!(
        serde_json::to_value(&successor_grant).unwrap()["binding"]["api_key_id"],
        successor.id
    );
    assert_eq!(
        admit(&f, &successor_source, None).await.unwrap().grant.id,
        successor_grant.id
    );
}

#[tokio::test]
async fn source_change_and_grant_invalidation_commit_or_abort_together() {
    let f = fixture("external_grant_atomic_source").await;
    let original = f.approve().await;
    let mut sibling_key = f.key.clone();
    sibling_key.id = Uuid::new_v4().to_string();
    f.db.collection::<ApiKey>(API_KEYS)
        .insert_one(&sibling_key)
        .await
        .unwrap();
    let sibling = approve_api_key(
        &f.db,
        &f.user,
        &sibling_key.id,
        sibling_key.state_version,
        &f.selection(),
    )
    .await
    .unwrap();

    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {"$set": {"is_active": false}, "$inc": {"state_version": 1_i64}},
        )
        .session(&mut session)
        .await
        .unwrap();
    assert_eq!(
        invalidate_api_key_in_session(&f.db, &f.user, &f.key.id, "source_disabled", &mut session,)
            .await
            .unwrap(),
        1
    );
    session.abort_transaction().await.unwrap();
    assert!(f.current_key().await.is_active);
    let rolled_back = f.grant(&original.id).await;
    assert_status(&rolled_back, "active");
    assert_eq!(rolled_back.revision, original.revision);

    session.start_transaction().await.unwrap();
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {"$set": {"is_active": false}, "$inc": {"state_version": 1_i64}},
        )
        .session(&mut session)
        .await
        .unwrap();
    assert_eq!(
        invalidate_api_key_in_session(&f.db, &f.user, &f.key.id, "source_disabled", &mut session,)
            .await
            .unwrap(),
        1
    );
    session.commit_transaction().await.unwrap();
    assert!(!f.current_key().await.is_active);
    let invalidated = f.grant(&original.id).await;
    assert_status(&invalidated, "revoked");
    assert_eq!(invalidated.revision, original.revision + 1);
    let unrelated = f.grant(&sibling.id).await;
    assert_status(&unrelated, "active");
    assert_eq!(unrelated.revision, sibling.revision);

    // A durable replay cannot revive the revoked row or advance a sibling.
    session.start_transaction().await.unwrap();
    assert_eq!(
        invalidate_api_key_in_session(&f.db, &f.user, &f.key.id, "source_disabled", &mut session,)
            .await
            .unwrap(),
        0
    );
    session.commit_transaction().await.unwrap();
    assert_eq!(f.grant(&original.id).await.revision, invalidated.revision);
    assert_eq!(f.grant(&sibling.id).await.revision, sibling.revision);
}

#[tokio::test]
async fn repeated_real_key_authentication_preserves_machine_grant() {
    let f = fixture("external_grant_last_used").await;
    let fixture_secret = format!("nyxid_{}", Uuid::new_v4());
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {
                "$set": {"key_hash": crate::crypto::token::hash_token(&fixture_secret)}
            },
        )
        .await
        .unwrap();
    let grant = f.approve().await;
    for _ in 0..3 {
        let (user_id, verified, credential_id) =
            crate::services::key_service::validate_api_key(&f.db, &fixture_secret)
                .await
                .unwrap();
        assert_eq!(user_id, f.user);
        assert!(credential_id.is_none());
        let source = VerifiedExternalSource::from_verified_api_key(&verified, None).unwrap();
        let admitted = admit(&f, &source, Some((&grant.id, grant.revision)))
            .await
            .unwrap();
        assert_eq!(admitted.grant.id, grant.id);
        assert_eq!(admitted.grant.revision, grant.revision);
    }
    let used = f.current_key().await;
    assert!(used.state_version >= f.key.state_version + 3);
    assert!(used.last_used_at.is_some());
    assert_status(&f.grant(&grant.id).await, "active");
}

#[tokio::test]
async fn source_and_grant_expiry_are_checked_live_before_sweeps() {
    let f = fixture("external_grant_live_expiry").await;
    let grant = f.approve().await;
    let source =
        VerifiedExternalSource::from_verified_api_key(&f.current_key().await, None).unwrap();
    assert!(admit(&f, &source, None).await.is_ok());
    f.db.collection::<ApiKey>(API_KEYS).update_one(doc! {"_id": &f.key.id}, doc! {
        "$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}
    }).await.unwrap();
    assert!(
        admit(&f, &source, None).await.is_err(),
        "cached verified source cannot bypass live parent expiry"
    );
    // This is fixture restoration, not a production security mutation.
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! {"_id": &f.key.id},
            doc! {
                "$set": {"expires_at": bson::Bson::Null}
            },
        )
        .await
        .unwrap();
    f.db.collection::<MachineExternalGrant>(GRANTS).update_one(doc! {"_id": &grant.id}, doc! {
        "$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}
    }).await.unwrap();
    assert!(
        admit(&f, &source, None).await.is_err(),
        "grant expiry must not wait for a sweep"
    );
    assert_status(&f.grant(&grant.id).await, "active");
}

#[tokio::test]
async fn independent_validity_distinguishes_live_source_from_expired_or_revoked_grant() {
    let f = fixture("external_grant_independent_validity").await;
    let grant = f.approve().await;

    let valid = evaluate_validity(
        &f.db,
        &f.user,
        &SourceReference::ApiKey {
            api_key_id: f.key.id.clone(),
        },
        Some(&grant),
    )
    .await
    .unwrap();
    assert!(valid.source.valid);
    assert!(valid.grant.valid);
    assert_eq!(valid.source.reason, None);
    assert_eq!(valid.grant.reason, None);

    f.db
        .collection::<MachineExternalGrant>(GRANTS)
        .update_one(
            doc! {"_id": &grant.id},
            doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let expired = evaluate_validity(
        &f.db,
        &f.user,
        &SourceReference::ApiKey {
            api_key_id: f.key.id.clone(),
        },
        Some(&grant),
    )
    .await
    .unwrap();
    assert!(expired.source.valid, "the source remains live");
    assert!(!expired.grant.valid);
    assert_eq!(expired.grant.reason, Some(ValidityReason::Expired));
    assert!(!expired.combined);
}

#[tokio::test]
async fn independent_validity_distinguishes_disabled_source_from_live_grant_metadata() {
    let f = fixture("external_grant_source_validity").await;
    let grant = f.approve().await;
    f.db.collection::<ApiKey>(API_KEYS)
        .update_one(doc! {"_id": &f.key.id}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();

    let state = evaluate_validity(
        &f.db,
        &f.user,
        &SourceReference::ApiKey {
            api_key_id: f.key.id.clone(),
        },
        Some(&grant),
    )
    .await
    .unwrap();
    assert!(!state.source.valid);
    assert_eq!(state.source.reason, Some(ValidityReason::Disabled));
    assert!(
        state.grant.valid,
        "grant metadata is still active and unexpired"
    );
    assert!(!state.combined);
}

#[tokio::test]
async fn independent_validity_reports_revoked_grant_and_live_source() {
    let f = fixture("external_grant_revoked_validity").await;
    let grant = f.approve().await;
    let revoked = revoke(&f.db, &f.user, &grant.id, grant.revision)
        .await
        .unwrap();

    let state = evaluate_validity(
        &f.db,
        &f.user,
        &SourceReference::ApiKey {
            api_key_id: f.key.id.clone(),
        },
        Some(&revoked),
    )
    .await
    .unwrap();
    assert!(state.source.valid);
    assert!(!state.grant.valid);
    assert_eq!(state.grant.reason, Some(ValidityReason::Revoked));
}

#[tokio::test]
async fn independent_validity_reads_expired_missing_and_ineligible_parent_live() {
    let f = fixture("external_grant_parent_validity_states").await;
    let grant = f.approve().await;
    for (update, reason) in [
        (
            doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}},
            ValidityReason::Expired,
        ),
        (
            doc! {"$set": {"expires_at": bson::Bson::Null, "scopes": "account:read"}},
            ValidityReason::Ineligible,
        ),
    ] {
        // Simulate an authoritative source change before its maintenance sweep.
        f.db.collection::<ApiKey>(API_KEYS)
            .update_one(doc! {"_id": &f.key.id}, update)
            .await
            .unwrap();
        let validity = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
        assert_eq!(validity.source.reason, Some(reason));
        assert!(!validity.source.valid);
        assert!(validity.grant.valid, "source state is not grant metadata");
        assert!(!validity.combined);
    }
    f.db.collection::<ApiKey>(API_KEYS)
        .delete_one(doc! {"_id": &f.key.id})
        .await
        .unwrap();
    let missing = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert_eq!(missing.source.reason, Some(ValidityReason::Missing));
    assert!(missing.grant.valid);
    assert!(!missing.combined);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_validity_keeps_parent_source_live_after_one_child_revocation() {
    let f = fixture("external_grant_parent_child_validity").await;
    let grant = f.approve().await;
    let revoked_child = child(&f).await;
    let active_child = child(&f).await;
    let key = f.current_key().await;
    let revoked_source =
        VerifiedExternalSource::from_verified_api_key(&key, Some(&revoked_child.id)).unwrap();
    let active_source =
        VerifiedExternalSource::from_verified_api_key(&key, Some(&active_child.id)).unwrap();
    crate::services::api_key_credential_service::revoke(
        &f.db,
        &revoked_child.id,
        CredentialRevokedReason::WebRevoke,
    )
    .await
    .unwrap();
    let validity = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert!(validity.source.valid && validity.grant.valid && validity.combined);
    assert!(admit(&f, &revoked_source, None).await.is_err());
    assert!(admit(&f, &active_source, None).await.is_ok());
    assert_eq!(f.grant(&grant.id).await.revision, grant.revision);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_validity_rejects_durable_chat_binding_despite_general_key_label() {
    let f = fixture("external_grant_chat_source_validity").await;
    let grant = f.approve().await;
    f.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "credential_api_key_id": &f.key.id,
        })
        .await
        .unwrap();
    let validity = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert!(!validity.source.valid);
    assert_eq!(validity.source.reason, Some(ValidityReason::Ineligible));
    assert!(validity.grant.valid);
    assert!(!validity.combined);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_validity_oauth_reconsent_keeps_live_source_separate_from_old_grant() {
    let f = fixture("external_grant_oauth_independent_validity").await;
    let client = oauth_client(&f).await;
    let (_, old, _) = approve_oauth(&f, &client).await;
    let (_, current, _) = approve_oauth(&f, &client).await;
    let old = f.grant(&old.id).await;
    let validity = evaluate_validity_for_grant(&f.db, &old).await.unwrap();
    assert!(
        validity.source.valid,
        "current client and consent remain live"
    );
    assert!(!validity.grant.valid);
    assert!(!validity.combined);
    let current_validity = evaluate_validity_for_grant(&f.db, &current).await.unwrap();
    assert!(current_validity.source.valid && current_validity.grant.valid);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_validity_oauth_client_and_consent_are_live_not_label_snapshots() {
    let f = fixture("external_grant_oauth_source_states").await;
    let client = oauth_client(&f).await;
    let (consent, grant, _) = approve_oauth(&f, &client).await;
    f.db.collection::<OauthClient>(CLIENTS)
        .update_one(doc! {"_id": &client}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    let disabled = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert_eq!(disabled.source.reason, Some(ValidityReason::Disabled));
    assert!(disabled.grant.valid && !disabled.combined);
    f.db.collection::<OauthClient>(CLIENTS)
        .update_one(doc! {"_id": &client}, doc! {"$set": {"is_active": true}})
        .await
        .unwrap();
    f.db.collection::<Consent>(CONSENTS)
        .update_one(
            doc! {"_id": &consent.id},
            doc! {"$set": {
                "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
            }},
        )
        .await
        .unwrap();
    let expired = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert_eq!(expired.source.reason, Some(ValidityReason::Expired));
    assert!(expired.grant.valid && !expired.combined);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_validity_owner_epoch_remains_changed_after_disable_and_reenable() {
    let f = fixture("external_grant_owner_epoch_validity").await;
    let grant = f.approve().await;
    f.db.collection::<bson::Document>(USERS)
        .update_one(
            doc! {"_id": &f.user},
            doc! {
                "$set": {"is_active": false},
                "$inc": {"machine_external_security_revision": 1_i64},
            },
        )
        .await
        .unwrap();
    let disabled = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert!(!disabled.source.valid);
    assert_eq!(disabled.source.reason, Some(ValidityReason::Disabled));
    assert!(!disabled.grant.valid);
    assert_eq!(disabled.grant.reason, Some(ValidityReason::Changed));
    assert!(!disabled.combined);
    assert_status(&f.grant(&grant.id).await, "active");

    f.db.collection::<bson::Document>(USERS)
        .update_one(doc! {"_id": &f.user}, doc! {"$set": {"is_active": true}})
        .await
        .unwrap();
    let reenabled = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert!(reenabled.source.valid);
    assert_eq!(reenabled.source.reason, None);
    assert!(!reenabled.grant.valid);
    assert_eq!(reenabled.grant.reason, Some(ValidityReason::Changed));
    assert!(
        !reenabled.combined,
        "restoring the owner cannot revive an old grant epoch"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_validity_rejects_empty_oauth_consent_revision() {
    let f = fixture("external_grant_empty_consent_revision").await;
    let client = oauth_client(&f).await;
    let (consent, grant, _) = approve_oauth(&f, &client).await;
    f.db.collection::<bson::Document>(CONSENTS)
        .update_one(doc! {"_id": &consent.id}, doc! {"$set": {"revision": ""}})
        .await
        .unwrap();
    let validity = evaluate_validity_for_grant(&f.db, &grant).await.unwrap();
    assert!(!validity.source.valid);
    assert_eq!(validity.source.reason, Some(ValidityReason::Ineligible));
    assert!(!validity.combined);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn current_source_reads_prefer_active_grant_over_equal_timestamp_history() {
    let f = fixture("external_grant_source_current_ties").await;
    let key_grant = f.approve().await;
    let client = oauth_client(&f).await;
    let (_, oauth_grant, _) = approve_oauth(&f, &client).await;
    for (grant, historical_id) in [
        (&key_grant, "ffffffff-ffff-4fff-bfff-ffffffffffff"),
        (&oauth_grant, "fffffffe-ffff-4fff-bfff-ffffffffffff"),
    ] {
        let mut history = grant.clone();
        history.id = historical_id.into();
        history.status = GrantStatus::Revoked;
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .insert_one(history)
            .await
            .unwrap();
    }
    assert_eq!(
        get_latest_for_key(&f.db, &f.user, &f.key.id)
            .await
            .unwrap()
            .unwrap()
            .id,
        key_grant.id,
    );
    assert_eq!(
        get_latest_for_oauth(&f.db, &f.user, &client)
            .await
            .unwrap()
            .unwrap()
            .id,
        oauth_grant.id,
    );
    assert!(
        get_latest_for_key(&f.db, &f.user, &client)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_latest_for_oauth(&f.db, &f.user, &f.key.id)
            .await
            .unwrap()
            .is_none()
    );
    let stranger = Uuid::new_v4().to_string();
    assert!(
        get_latest_for_key(&f.db, &stranger, &f.key.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_latest_for_oauth(&f.db, &stranger, &client)
            .await
            .unwrap()
            .is_none()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_read_handlers_are_owner_only_and_return_safe_independent_states() {
    use crate::handlers::machine_external_grant as handlers;
    use crate::mw::auth::AuthMethod;
    use axum::extract::{Path, State};
    let f = fixture("external_grant_read_handler_contract").await;
    let grant = f.approve().await;
    let state = crate::test_utils::test_app_state(f.db.clone());
    let auth = crate::test_utils::test_auth_user(&f.user);
    let response = handlers::get_key(State(state.clone()), auth.clone(), Path(f.key.id.clone()))
        .await
        .unwrap()
        .0;
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["grant"]["id"], grant.id);
    assert_eq!(json["source_valid"], true);
    assert_eq!(json["grant_valid"], true);
    assert_eq!(json["source_and_grant_valid"], true);
    assert!(json["source_validity_reason"].is_null());
    assert!(json["grant_validity_reason"].is_null());
    let body = json.to_string();
    for secret_field in [
        "key_hash",
        "secret_hash",
        "key_prefix",
        "client_secret",
        "access_token",
    ] {
        assert!(!body.contains(secret_field));
    }
    for method in [AuthMethod::ApiKey, AuthMethod::Delegated, AuthMethod::Relay] {
        let mut caller = auth.clone();
        caller.auth_method = method;
        assert!(
            handlers::get_key(State(state.clone()), caller, Path(f.key.id.clone()),)
                .await
                .is_err()
        );
    }
    let mut oauth_caller = auth.clone();
    oauth_caller.oauth_client_id = Some(Uuid::new_v4().to_string());
    assert!(
        handlers::get_key(State(state.clone()), oauth_caller, Path(f.key.id.clone()),)
            .await
            .is_err()
    );
    let outsider = crate::test_utils::test_auth_user(&Uuid::new_v4().to_string());
    let foreign = handlers::get_key(State(state.clone()), outsider, Path(f.key.id.clone()))
        .await
        .unwrap()
        .0;
    assert!(foreign.grant.is_none());
    assert!(!foreign.source_valid && !foreign.grant_valid && !foreign.source_and_grant_valid);
    let child = child(&f).await;
    let child_lookup = handlers::get_key(State(state), auth, Path(child.id))
        .await
        .unwrap()
        .0;
    assert!(
        child_lookup.grant.is_none(),
        "a child ID is never a parent grant identity"
    );
    assert!(!child_lookup.source_valid && !child_lookup.grant_valid);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_grant_page_is_bounded_and_stable_for_equal_timestamps() {
    let f = fixture("external_grant_page_equal_timestamps").await;
    let template = f.approve().await;
    let timestamp = template.created_at;
    let mut rows = Vec::new();
    for _ in 0..101 {
        let mut row = template.clone();
        row.id = Uuid::new_v4().to_string();
        // Retained versions may share a source; active versions may not.
        row.status = GrantStatus::Revoked;
        row.created_at = timestamp;
        row.updated_at = timestamp;
        rows.push(row);
    }
    let mut foreign = template.clone();
    foreign.id = Uuid::new_v4().to_string();
    foreign.user_id = Uuid::new_v4().to_string();
    foreign.created_at = timestamp;
    rows.push(foreign);
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_many(rows)
        .await
        .unwrap();

    let first = list_for_machine_page(&f.db, &f.user, &f.node.id, None, 1_000)
        .await
        .unwrap();
    assert_eq!(first.grants.len(), 100);
    assert!(first.next_cursor.is_some());
    assert!(first.grants.iter().all(|row| row.user_id == f.user));
    assert!(
        first
            .grants
            .iter()
            .all(|row| row.policy.assignments.contains_key(&f.node.id))
    );

    let second = list_for_machine_page(
        &f.db,
        &f.user,
        &f.node.id,
        first.next_cursor.as_deref(),
        100,
    )
    .await
    .unwrap();
    assert_eq!(
        second.grants.len(),
        2,
        "template + 101 inserted rows span two pages"
    );
    assert!(second.next_cursor.is_none());
    let ids: std::collections::BTreeSet<_> = first
        .grants
        .iter()
        .chain(second.grants.iter())
        .map(|row| row.id.as_str())
        .collect();
    assert_eq!(ids.len(), 102);
    let ordered: Vec<_> = first
        .grants
        .iter()
        .chain(second.grants.iter())
        .map(|row| (row.created_at, row.id.as_str()))
        .collect();
    assert!(ordered.windows(2).all(|pair| pair[0] > pair[1]));
    let minimum = list_for_machine_page(&f.db, &f.user, &f.node.id, None, 0)
        .await
        .unwrap();
    assert_eq!(minimum.grants.len(), 1);
    assert!(minimum.next_cursor.is_some());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_grant_page_rejects_malformed_cursor_without_widening_scope() {
    let f = fixture("external_grant_page_cursor_validation").await;
    for cursor in [
        "foreign|cursor",
        "2026-10-14T00:00:00.000Z|",
        "2026-10-14T00:00:00.000Z|not-a-uuid",
        "2026-10-14T00:00:00.000Z|00000000-0000-4000-8000-000000000001|extra",
    ] {
        let error = list_for_machine_page(&f.db, &f.user, &f.node.id, Some(cursor), 100)
            .await
            .expect_err("malformed cursor must be rejected");
        assert!(
            matches!(error, AppError::BadRequest(message) if message == "machine_grant_cursor_invalid")
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_grant_page_cursor_cannot_cross_machine_filter() {
    let f = fixture("external_grant_page_cross_machine_cursor").await;
    let template = f.approve().await;
    let other_machine = Uuid::new_v4().to_string();
    let mut other_node = f.node.clone();
    other_node.id = other_machine.clone();
    f.db.collection::<Node>(NODES)
        .insert_one(other_node)
        .await
        .unwrap();
    let mut other = template.clone();
    other.id = Uuid::new_v4().to_string();
    other.status = GrantStatus::Revoked;
    let assignment = template
        .policy
        .assignments
        .values()
        .next()
        .cloned()
        .unwrap();
    let restriction = template.restrictions.values().next().cloned().unwrap();
    other.policy.assignments.clear();
    other
        .policy
        .assignments
        .insert(other_machine.clone(), assignment);
    other.restrictions.clear();
    other
        .restrictions
        .insert(other_machine.clone(), restriction);
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_one(other.clone())
        .await
        .unwrap();

    let other_page = list_for_machine_page(&f.db, &f.user, &other_machine, None, 100)
        .await
        .unwrap();
    assert_eq!(
        other_page
            .grants
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec![other.id.as_str()]
    );
    let cursor = format!(
        "{}|{}",
        other
            .created_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        other.id
    );
    let machine_page = list_for_machine_page(&f.db, &f.user, &f.node.id, Some(&cursor), 100)
        .await
        .unwrap();
    assert!(
        machine_page
            .grants
            .iter()
            .all(|row| row.policy.assignments.contains_key(&f.node.id))
    );
    assert!(machine_page.grants.iter().all(|row| row.id != other.id));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_grant_page_filters_before_limit_and_rechecks_live_machine_owner() {
    let f = fixture("external_grant_page_scope_before_limit").await;
    let template = f.approve().await;
    let foreign_owner = Uuid::new_v4().to_string();
    let other_machine = Uuid::new_v4().to_string();
    let mut noise = Vec::new();
    for index in 0..240 {
        let mut row = template.clone();
        row.id = Uuid::new_v4().to_string();
        row.status = GrantStatus::Revoked;
        row.created_at = template.created_at + Duration::seconds(index + 1);
        if index % 2 == 0 {
            row.user_id = foreign_owner.clone();
        } else {
            let assignment = row.policy.assignments.remove(&f.node.id).unwrap();
            let restriction = row.restrictions.remove(&f.node.id).unwrap();
            row.policy
                .assignments
                .insert(other_machine.clone(), assignment);
            row.restrictions.insert(other_machine.clone(), restriction);
        }
        noise.push(row);
    }
    let foreign_cursor = format!(
        "{}|{}",
        noise[0]
            .created_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        noise[0].id,
    );
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_many(noise)
        .await
        .unwrap();
    for cursor in [None, Some(foreign_cursor.as_str())] {
        let page = list_for_machine_page(&f.db, &f.user, &f.node.id, cursor, 100)
            .await
            .unwrap();
        assert_eq!(
            page.grants.len(),
            1,
            "unrelated rows cannot consume the page"
        );
        assert_eq!(page.grants[0].id, template.id);
        assert!(page.next_cursor.is_none());
    }
    assert!(
        list_for_machine_page(&f.db, &foreign_owner, &f.node.id, None, 100)
            .await
            .is_err(),
        "a matching grant row does not establish Machine ownership"
    );
    f.db.collection::<Node>(NODES)
        .update_one(
            doc! {"_id": &f.node.id},
            doc! {"$set": {"user_id": &foreign_owner}},
        )
        .await
        .unwrap();
    assert!(
        list_for_machine_page(&f.db, &f.user, &f.node.id, Some(&foreign_cursor), 100)
            .await
            .is_err(),
        "an issued cursor cannot retain authority after ownership changes"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_grant_owner_handler_pages_history_and_denies_delegated_or_foreign_callers() {
    use crate::handlers::machine_external_grant::{self as handlers, MachineGrantListQuery};
    use crate::mw::auth::AuthMethod;
    use axum::extract::{Path, Query, State};
    let f = fixture("external_grant_owner_handler_pages").await;
    let template = f.approve().await;
    let mut history = Vec::new();
    for _ in 0..104 {
        let mut row = template.clone();
        row.id = Uuid::new_v4().to_string();
        row.status = GrantStatus::Revoked;
        history.push(row);
    }
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_many(history)
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(f.db.clone());
    let auth = crate::test_utils::test_auth_user(&f.user);
    let first = handlers::list_machine(
        State(state.clone()),
        auth.clone(),
        Path(f.node.id.clone()),
        Query(MachineGrantListQuery {
            cursor: None,
            limit: Some(500),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(first.external_grants.len(), 100);
    assert!(first.next_cursor.is_some());
    let second = handlers::list_machine(
        State(state.clone()),
        auth.clone(),
        Path(f.node.id.clone()),
        Query(MachineGrantListQuery {
            cursor: first.next_cursor.clone(),
            limit: Some(100),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(second.external_grants.len(), 5);
    assert!(second.next_cursor.is_none());
    let ids: std::collections::BTreeSet<_> = first
        .external_grants
        .iter()
        .chain(second.external_grants.iter())
        .map(|row| row.id.as_str())
        .collect();
    assert_eq!(ids.len(), 105);
    let mut delegated = auth;
    delegated.auth_method = AuthMethod::Delegated;
    for caller in [
        delegated,
        crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
    ] {
        assert!(
            handlers::list_machine(
                State(state.clone()),
                caller,
                Path(f.node.id.clone()),
                Query(MachineGrantListQuery {
                    cursor: first.next_cursor.clone(),
                    limit: Some(100)
                }),
            )
            .await
            .is_err()
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn child_revocation_conflicts_with_stale_admission_without_fencing_siblings() {
    let f = fixture("external_grant_child_race").await;
    let grant = f.approve().await;
    let first = child(&f).await;
    let second = child(&f).await;
    let key = f.current_key().await;
    let first_source =
        VerifiedExternalSource::from_verified_api_key(&key, Some(&first.id)).unwrap();
    let second_source =
        VerifiedExternalSource::from_verified_api_key(&key, Some(&second.id)).unwrap();
    assert!(admit(&f, &first_source, None).await.is_ok());
    assert!(admit(&f, &second_source, None).await.is_ok());

    // Pin a snapshot before the independently committed revocation. Admission
    // must write the same durable child row and therefore cannot commit this
    // otherwise-valid stale read after revocation has won.
    let mut stale = f.db.client().start_session().await.unwrap();
    stale.start_transaction().await.unwrap();
    f.db.collection::<ApiKeyCredential>(CREDENTIALS)
        .find_one(doc! {"_id": &first.id})
        .session(&mut stale)
        .await
        .unwrap()
        .unwrap();
    let mut revoking = f.db.client().start_session().await.unwrap();
    revoking.start_transaction().await.unwrap();
    let revoked = crate::services::api_key_credential_service::revoke_in_session(
        &f.db,
        &first.id,
        CredentialRevokedReason::Logout,
        &mut revoking,
    )
    .await
    .unwrap();
    assert!(revoked.is_some());
    revoking.commit_transaction().await.unwrap();
    let result = admit_in_session(&f.db, &first_source, &f.node.id, None, &mut stale).await;
    assert!(
        result.is_err(),
        "stale child admission must conflict with committed child revocation"
    );
    let _ = stale.abort_transaction().await;

    assert!(admit(&f, &first_source, None).await.is_err());
    let sibling = admit(&f, &second_source, Some((&grant.id, grant.revision)))
        .await
        .unwrap();
    assert_eq!(
        sibling.api_key_credential_id.as_deref(),
        Some(second.id.as_str())
    );
    let live_grant = f.grant(&grant.id).await;
    assert_status(&live_grant, "active");
    assert_eq!(live_grant.revision, grant.revision);
    assert!(f.current_key().await.is_active);
}

#[tokio::test]
async fn grant_revoke_conflicts_with_stale_admission_and_new_grant_does_not_adopt_old_work() {
    let f = fixture("external_grant_admission_race").await;
    let grant = f.approve().await;
    let source =
        VerifiedExternalSource::from_verified_api_key(&f.current_key().await, None).unwrap();
    let mut stale = f.db.client().start_session().await.unwrap();
    stale.start_transaction().await.unwrap();
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .find_one(doc! {"_id": &grant.id})
        .session(&mut stale)
        .await
        .unwrap()
        .unwrap();
    revoke(&f.db, &f.user, &grant.id, grant.revision)
        .await
        .unwrap();
    assert!(
        admit_in_session(
            &f.db,
            &source,
            &f.node.id,
            Some((&grant.id, grant.revision)),
            &mut stale
        )
        .await
        .is_err()
    );
    let _ = stale.abort_transaction().await;
    let new_grant = f.approve().await;
    assert_ne!(new_grant.id, grant.id);
    assert!(
        admit(&f, &source, Some((&grant.id, grant.revision)))
            .await
            .is_err()
    );
    assert_eq!(
        admit(&f, &source, None).await.unwrap().grant.id,
        new_grant.id
    );
}

#[tokio::test]
async fn durable_owner_turn_binding_is_never_an_external_fallback() {
    let f = fixture("external_grant_owner_turn").await;
    let grant = f.approve().await;
    let key = f.current_key().await;
    assert!(key.assistant_agent_owner_id.is_none());
    assert!(key.assistant_group_id.is_none());
    let source = VerifiedExternalSource::from_verified_api_key(&key, None).unwrap();
    // The authoritative binding projection is sufficient even if a legacy key
    // has no assistant marker or has had its display fields changed.
    f.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "user_id": &f.user,
            "credential_api_key_id": &f.key.id,
        })
        .await
        .unwrap();
    assert!(
        admit(&f, &source, Some((&grant.id, grant.revision)))
            .await
            .is_err()
    );
    revoke(&f.db, &f.user, &grant.id, grant.revision)
        .await
        .unwrap();
    assert!(
        approve_api_key(&f.db, &f.user, &key.id, key.state_version, &f.selection())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn user_security_epoch_denies_unswept_grant_after_disable_and_reenable() {
    let f = fixture("external_grant_user_epoch").await;
    let mut grant = f.approve().await;
    let original_id = grant.id.clone();
    // The current real key's grant sorts after the bounded first batch.
    grant.id = "ffffffff-ffff-4fff-bfff-ffffffffffff".into();
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .delete_one(doc! {"_id": &original_id})
        .await
        .unwrap();
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let mut other_grants = Vec::new();
    for sequence in 0..1000 {
        let mut other = grant.clone();
        other.id = format!("00000000-0000-4000-8000-{sequence:012x}");
        other.binding = crate::models::machine_external_grant::ExternalGrantBinding::ApiKey {
            api_key_id: Uuid::new_v4().to_string(),
            api_key_state_version_at_approval: 1,
        };
        other_grants.push(other);
    }
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_many(other_grants)
        .await
        .unwrap();
    let source =
        VerifiedExternalSource::from_verified_api_key(&f.current_key().await, None).unwrap();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    f.db.collection::<User>(USERS)
        .update_one(
            doc! {"_id": &f.user},
            doc! {
                "$set": {"is_active": false}
            },
        )
        .session(&mut session)
        .await
        .unwrap();
    assert_eq!(
        user_security_changed_in_session(&f.db, &f.user, "user_disabled", &mut session)
            .await
            .unwrap(),
        1000
    );
    session.commit_transaction().await.unwrap();
    assert!(admit(&f, &source, None).await.is_err());
    let unswept = f.grant(&grant.id).await;
    assert_status(&unswept, "active");
    assert_eq!(unswept.revision, grant.revision);

    // Model a later account re-enable; the durable security epoch remains.
    f.db.collection::<User>(USERS)
        .update_one(
            doc! {"_id": &f.user},
            doc! {
                "$set": {"is_active": true}
            },
        )
        .await
        .unwrap();
    assert!(
        admit(&f, &source, Some((&grant.id, grant.revision)))
            .await
            .is_err(),
        "reenabling the user must not resurrect an unswept old grant"
    );
    assert_eq!(
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .count_documents(doc! {"status": "revoked"})
            .await
            .unwrap(),
        1000
    );

    // A later explicit approval is at the new epoch. Continuing the old
    // bounded sweep must revoke only old-epoch rows, never this fresh grant.
    let mut new_key = f.key.clone();
    new_key.id = Uuid::new_v4().to_string();
    f.db.collection::<ApiKey>(API_KEYS)
        .insert_one(&new_key)
        .await
        .unwrap();
    let current = approve_api_key(
        &f.db,
        &f.user,
        &new_key.id,
        new_key.state_version,
        &f.selection(),
    )
    .await
    .unwrap();
    assert!(current.user_security_revision > grant.user_security_revision);
    session.start_transaction().await.unwrap();
    assert_eq!(
        invalidate_user_in_session(&f.db, &f.user, "user_disabled", 1000, &mut session)
            .await
            .unwrap(),
        1
    );
    session.commit_transaction().await.unwrap();
    assert_status(&f.grant(&grant.id).await, "revoked");
    let preserved = f.grant(&current.id).await;
    assert_status(&preserved, "active");
    assert_eq!(preserved.revision, current.revision);
    let current_source = VerifiedExternalSource::from_verified_api_key(&new_key, None).unwrap();
    assert!(
        admit(&f, &current_source, Some((&current.id, current.revision)))
            .await
            .is_ok()
    );
}

// These fixtures admit through the real source/grant transaction and persist
// typed job/lease rows. They exercise the public outbox consumer without a
// physical Node; they do not claim process-stop or public MCP acceptance.
struct AdmittedWork {
    job_id: String,
    lease_id: String,
    confirmation_id: String,
}

async fn admitted_work(f: &Fixture, source: &VerifiedExternalSource) -> AdmittedWork {
    use crate::models::{machine_access, machine_external_execution, machine_job};
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let admitted = admit_in_session(&f.db, source, &f.node.id, None, &mut session)
        .await
        .unwrap();
    let now = Utc::now();
    let job_id = Uuid::new_v4().to_string();
    let lease_id = Uuid::new_v4().to_string();
    let confirmation_id = Uuid::new_v4().to_string();
    let principal = bson::to_bson(&source.principal()).unwrap();
    let authority = doc! {
        "principal": principal.clone(),
        "oauth_consent": source.protected_consent().map(|c| doc! {
            "consent_id": &c.consent_id, "consent_revision": &c.consent_revision,
        }),
        "context_id": Uuid::new_v4().to_string(), "generation": 1_i64,
        "mode": "shared_legacy", "owner_id": &f.user, "actor_id": &f.user,
        "node_id": &f.node.id, "runtime_id": &f.node.machine.as_ref().unwrap().runtime_id,
        "runtime_generation": bson::Bson::Null, "credential_epoch": bson::Bson::Null,
        "grant_id": &admitted.grant.id, "grant_revision": admitted.grant.revision,
        "assignment_revision": admitted.assignment.revision,
        "lease_id": &lease_id, "job_id": &job_id,
        "expires_at_ms": (now + Duration::seconds(45)).timestamp_millis(),
        "auth_expires_at_ms": admitted.authentication_expires_at.timestamp_millis(),
        "job_expires_at_ms": (now + Duration::minutes(10)).timestamp_millis(),
        "funding_expires_at_ms": bson::Bson::Null,
        "capabilities": bson::to_bson(&admitted.assignment.capabilities).unwrap(),
        "file_roots": &admitted.restrictions.file_roots, "services": [],
    };
    let checked: nyxid_machine::authority::ExternalAuthority =
        bson::from_document(authority.clone()).unwrap();
    assert!(checked.valid(&f.node.id, &checked.runtime_id, now.timestamp_millis()));
    let binding = machine_job::MachineJobBinding::External {
        version: 1,
        principal: source.principal(),
        grant_id: admitted.grant.id.clone(),
        grant_revision: admitted.grant.revision,
        assignment_revision: admitted.assignment.revision,
        protected_consent: source.protected_consent().cloned(),
        api_key_state_version_at_approval: match &admitted.grant.binding {
            ExternalGrantBinding::ApiKey {
                api_key_state_version_at_approval,
                ..
            } => Some(*api_key_state_version_at_approval),
            _ => None,
        },
        authentication_expires_at: admitted.authentication_expires_at,
        service_ceiling: vec![],
        lease_id: lease_id.clone(),
    };
    let job = doc! {
        "_id": &job_id, "user_id": &f.user, "node_id": &f.node.id,
        "runtime_id": &checked.runtime_id, "binding": bson::to_bson(&binding).unwrap(),
        "external_authority": &authority, "state": "running", "services": [],
        "created_at": bson::DateTime::from_chrono(now),
        "expires_at": bson::DateTime::from_chrono(now + Duration::minutes(10)),
    };
    let _: machine_job::MachineJob = bson::from_document(job.clone()).unwrap();
    f.db.collection::<Document>(machine_job::COLLECTION_NAME)
        .insert_one(job)
        .session(&mut session)
        .await
        .unwrap();
    f.db.collection::<machine_external_execution::ExternalLease>(machine_access::LEASES)
        .insert_one(machine_external_execution::ExternalLease {
            id: lease_id.clone(),
            kind: "external".into(),
            node_id: f.node.id.clone(),
            job_id: Some(job_id.clone()),
            external_authority: Box::new(checked),
            expires_at: now + Duration::days(1),
        })
        .session(&mut session)
        .await
        .unwrap();
    // No command, input, bearer or output is retained. Deserialize through the
    // real model so this is a complete confirmation row, not just selector keys.
    let confirmation: machine_external_execution::ExternalConfirmation =
        bson::from_document(doc! {
            "_id": &confirmation_id, "user_id": &f.user, "node_id": &f.node.id,
            "principal": principal, "grant_id": &admitted.grant.id,
            "grant_revision": admitted.grant.revision, "state": "pending",
            "assignment_revision": admitted.assignment.revision,
            "runtime_id": &f.node.machine.as_ref().unwrap().runtime_id,
            "operation": bson::to_bson(&nyxid_machine::Operation::Exec).unwrap(),
            "parameters_digest": "00".repeat(32), "binding_digest": "11".repeat(32),
            "created_at": bson::DateTime::from_chrono(now),
            "expires_at": bson::DateTime::from_chrono(now+Duration::minutes(15)),
        })
        .unwrap();
    f.db.collection::<machine_external_execution::ExternalConfirmation>(
        machine_external_execution::CONFIRMATIONS,
    )
    .insert_one(confirmation)
    .session(&mut session)
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    AdmittedWork {
        job_id,
        lease_id,
        confirmation_id,
    }
}

async fn source_outbox_rows(db: &Database) -> Vec<Document> {
    use futures::TryStreamExt;
    db.collection::<Document>(crate::models::machine_access::OUTBOX)
        .find(doc! {"kind": "external"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn work_row(db: &Database, collection: &str, id: &str) -> Document {
    db.collection::<Document>(collection)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

async fn assert_work(db: &Database, work: &AdmittedWork, state: &str, confirmation: &str) {
    use crate::models::{machine_access, machine_external_execution, machine_job};
    assert_eq!(
        work_row(db, machine_job::COLLECTION_NAME, &work.job_id)
            .await
            .get_str("state")
            .unwrap(),
        state
    );
    assert_eq!(
        work_row(
            db,
            machine_external_execution::CONFIRMATIONS,
            &work.confirmation_id
        )
        .await
        .get_str("state")
        .unwrap(),
        confirmation
    );
    assert_eq!(
        db.collection::<Document>(machine_access::LEASES)
            .count_documents(doc! {"_id": &work.lease_id})
            .await
            .unwrap(),
        u64::from(state == "running")
    );
}

async fn replay_source_outbox(db: &Database, outbox: &Document) {
    use crate::{
        models::machine_access, services::machine_external_service, test_utils::test_app_state,
    };
    let state = test_app_state(db.clone());
    machine_external_service::revoke_outbox(&state, outbox.clone())
        .await
        .unwrap();
    // Crash/replay: the original durable receipt survives after lease expansion
    // but before marking the source record complete. Never manually enqueue.
    db.collection::<Document>(machine_access::OUTBOX)
        .update_one(
            doc! {"_id": outbox.get_str("_id").unwrap()},
            doc! {"$set": {"pending": true, "claim_until": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    machine_external_service::revoke_outbox(&state, outbox.clone())
        .await
        .unwrap();
    machine_external_service::revoke_outbox(&state, outbox.clone())
        .await
        .unwrap();
}

#[tokio::test]
async fn source_outbox_actual_child_revoke_preserves_sibling_view_binding_and_grant() {
    use crate::{
        models::{machine_access, machine_job},
        services::{api_key_credential_service, machine_external_service},
    };
    let f = fixture("srcbox_child").await;
    let grant = f.approve().await;
    let a = child(&f).await;
    let b = child(&f).await;
    let key = f.current_key().await;
    let a_source = VerifiedExternalSource::from_verified_api_key(&key, Some(&a.id)).unwrap();
    let b_source = VerifiedExternalSource::from_verified_api_key(&key, Some(&b.id)).unwrap();
    let a_work = admitted_work(&f, &a_source).await;
    let b_work = admitted_work(&f, &b_source).await;
    let original = work_row(&f.db, machine_job::COLLECTION_NAME, &a_work.job_id).await;
    let page =
        machine_external_service::history(&f.db, &b_source, &f.node.id, &serde_json::json!({}))
            .await
            .unwrap();
    assert_eq!(page["jobs"].as_array().unwrap().len(), 2);
    assert_eq!(
        work_row(&f.db, machine_job::COLLECTION_NAME, &a_work.job_id)
            .await
            .get("binding"),
        original.get("binding")
    );
    let before_revoke = f.grant(&grant.id).await;
    api_key_credential_service::web_revoke(&f.db, &f.user, &f.key.id, &a.id)
        .await
        .unwrap();
    let rows = source_outbox_rows(&f.db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .get_document("selector")
            .unwrap()
            .get_str("api_key_credential_id")
            .unwrap(),
        a.id
    );
    assert_eq!(
        f.grant(&grant.id).await,
        before_revoke,
        "child revocation must not rewrite grant revision or policy"
    );
    replay_source_outbox(&f.db, &rows[0]).await;
    assert_work(&f.db, &a_work, "revoked", "revoked").await;
    assert_work(&f.db, &b_work, "running", "pending").await;
    assert_eq!(
        work_row(&f.db, machine_job::COLLECTION_NAME, &a_work.job_id)
            .await
            .get("binding"),
        original.get("binding")
    );
    assert_eq!(
        f.db.collection::<Document>(machine_access::OUTBOX)
            .count_documents(doc! {"kind": "external_lease"})
            .await
            .unwrap(),
        1
    );
    assert!(admit(&f, &a_source, None).await.is_err());
    assert!(
        admit(&f, &b_source, Some((&grant.id, grant.revision)))
            .await
            .is_ok()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_actual_key_mutation_abort_and_delayed_cancel_preserve_reauthorization() {
    use crate::services::api_key_mutation_service as mutations;
    let f = fixture("srcbox_key").await;
    let original = f.approve().await;
    let source =
        VerifiedExternalSource::from_verified_api_key(&f.current_key().await, None).unwrap();
    let old_work = admitted_work(&f, &source).await;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    mutations::update_one(
        &f.db,
        doc! {"_id": &f.key.id},
        doc! {"$set": {"allow_all_services": false}},
        Some(&mut session),
    )
    .await
    .unwrap();
    assert_eq!(
        f.db.collection::<Document>(crate::models::machine_access::OUTBOX)
            .count_documents(doc! {"kind": "external"})
            .session(&mut session)
            .await
            .unwrap(),
        1
    );
    assert!(source_outbox_rows(&f.db).await.is_empty());
    session.abort_transaction().await.unwrap();
    assert!(source_outbox_rows(&f.db).await.is_empty());
    assert_work(&f.db, &old_work, "running", "pending").await;
    mutations::update_one(
        &f.db,
        doc! {"_id": &f.key.id},
        doc! {"$set": {"allow_all_services": false}},
        None,
    )
    .await
    .unwrap();
    let outbox = source_outbox_rows(&f.db).await;
    assert_eq!(outbox.len(), 1);
    let selector = outbox[0].get_document("selector").unwrap();
    assert_eq!(selector.get_str("kind").unwrap(), "grant");
    assert_eq!(selector.get_str("grant_id").unwrap(), original.id);
    assert_eq!(
        selector.get_i64("through_revision").unwrap(),
        original.revision
    );
    let replacement = f.approve().await;
    assert_ne!(replacement.id, original.id);
    let new_work = admitted_work(&f, &source).await;
    replay_source_outbox(&f.db, &outbox[0]).await;
    assert_work(&f.db, &old_work, "revoked", "revoked").await;
    assert_work(&f.db, &new_work, "running", "pending").await;
    assert_eq!(f.grant(&original.id).await.revision, original.revision + 1);
    assert_eq!(f.grant(&replacement.id).await.revision, 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_actual_grant_edit_and_revoke_conserve_execution_revisions() {
    let f = fixture("srcbox_edit").await;
    let original = f.approve().await;
    let source =
        VerifiedExternalSource::from_verified_api_key(&f.current_key().await, None).unwrap();
    let old_work = admitted_work(&f, &source).await;
    let mut selection = f.selection();
    selection
        .assignments
        .get_mut(&f.node.id)
        .unwrap()
        .file_roots = vec!["/workspace/narrow".into()];
    let changed = edit_api_key(
        &f.db,
        &f.user,
        &original.id,
        original.revision,
        f.current_key().await.state_version,
        &selection,
    )
    .await
    .unwrap();
    assert_eq!(changed.id, original.id);
    assert_eq!(changed.revision, original.revision + 1);
    let next_work = admitted_work(&f, &source).await;
    let rows = source_outbox_rows(&f.db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .get_document("selector")
            .unwrap()
            .get_i64("through_revision")
            .unwrap(),
        original.revision
    );
    replay_source_outbox(&f.db, &rows[0]).await;
    assert_work(&f.db, &old_work, "revoked", "revoked").await;
    assert_work(&f.db, &next_work, "running", "pending").await;
    revoke(&f.db, &f.user, &changed.id, changed.revision)
        .await
        .unwrap();
    let replacement = f.approve().await;
    let replacement_work = admitted_work(&f, &source).await;
    let rows = source_outbox_rows(&f.db).await;
    assert_eq!(rows.len(), 2);
    for row in rows {
        replay_source_outbox(&f.db, &row).await;
    }
    assert_work(&f.db, &next_work, "revoked", "revoked").await;
    assert_work(&f.db, &replacement_work, "running", "pending").await;
    assert_eq!(f.grant(&replacement.id).await.revision, 1);
    f.db.drop().await.unwrap();
}

async fn oauth_client(f: &Fixture) -> String {
    crate::services::oauth_client_service::create_client(
        &f.db,
        "Source outbox fixture",
        &["https://example.invalid/callback".into()],
        "public",
        &f.user,
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

async fn approve_oauth(
    f: &Fixture,
    client: &str,
) -> (Consent, MachineExternalGrant, VerifiedExternalSource) {
    use crate::services::{consent_service, machine_oauth_consent_service as prepared};
    let approval = prepared::MachineConsentApproval {
        request_id: Uuid::new_v4().to_string(),
        signed_request: format!("verified-test-consent-{}", Uuid::new_v4()),
    };
    prepared::prepare(
        &f.db,
        &f.user,
        client,
        &approval.request_id,
        &approval.signed_request,
        &f.selection(),
    )
    .await
    .unwrap();
    let consent = consent_service::grant_consent_with_machine(
        &f.db,
        &f.user,
        client,
        "openid proxy machine offline_access",
        false,
        Some(vec![]),
        Some(&approval),
    )
    .await
    .unwrap();
    let grant = get_active_for_oauth(&f.db, &f.user, client)
        .await
        .unwrap()
        .unwrap();
    // Represents claims already verified by the HTTP authentication adapter;
    // this suite exercises source mutations, not JWT signature verification.
    let claims: Claims = serde_json::from_value(serde_json::json!({
        "sub": f.user, "iss": "source-outbox-fixture", "aud": "nyxid",
        "exp": (Utc::now() + Duration::hours(1)).timestamp(), "iat": Utc::now().timestamp(),
        "jti": Uuid::new_v4().to_string(), "scope": "openid proxy machine offline_access",
        "token_type": "access", "client_id": client,
        "protected_consent": consent_service::protected_reference(&consent).unwrap(),
    }))
    .unwrap();
    let source = VerifiedExternalSource::from_verified_oauth(&claims).unwrap();
    (consent, grant, source)
}

#[tokio::test]
async fn source_outbox_actual_consent_reapproval_cancels_old_version_only() {
    let f = fixture("srcbox_consent").await;
    let client = oauth_client(&f).await;
    let (consent, grant, source) = approve_oauth(&f, &client).await;
    let old_work = admitted_work(&f, &source).await;
    let (renewed_consent, replacement, renewed_source) = approve_oauth(&f, &client).await;
    assert_ne!(consent.revision, renewed_consent.revision);
    assert_ne!(grant.id, replacement.id);
    let new_work = admitted_work(&f, &renewed_source).await;
    assert!(admit(&f, &source, None).await.is_err());
    let outbox = source_outbox_rows(&f.db).await;
    assert_eq!(outbox.len(), 1);
    replay_source_outbox(&f.db, &outbox[0]).await;
    assert_work(&f.db, &old_work, "revoked", "revoked").await;
    assert_work(&f.db, &new_work, "running", "pending").await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_actual_incremental_consent_edit_and_revoke_use_original_grant() {
    use crate::services::consent_service;
    let f = fixture("srcbox_incremental").await;
    let client = oauth_client(&f).await;
    let (consent, original, source) = approve_oauth(&f, &client).await;
    let old_work = admitted_work(&f, &source).await;
    let changed =
        consent_service::replace_incremental_consent(&f.db, &consent, &consent.scopes, vec![])
            .await
            .unwrap();
    // The incremental edit adds no authority, so it is a true no-op and must
    // preserve the live revision. The later Machine re-approval is the
    // authority change that rotates the external grant.
    assert_eq!(changed.revision, consent.revision);
    let (_, replacement, new_source) = approve_oauth(&f, &client).await;
    let new_work = admitted_work(&f, &new_source).await;
    let outbox = source_outbox_rows(&f.db).await;
    assert_eq!(outbox.len(), 1);
    assert_eq!(
        outbox[0]
            .get_document("selector")
            .unwrap()
            .get_str("grant_id")
            .unwrap(),
        original.id
    );
    replay_source_outbox(&f.db, &outbox[0]).await;
    assert_work(&f.db, &old_work, "revoked", "revoked").await;
    assert_work(&f.db, &new_work, "running", "pending").await;
    consent_service::revoke_consent(&f.db, &f.user, &client)
        .await
        .unwrap();
    assert!(admit(&f, &new_source, None).await.is_err());
    let (_, latest, latest_source) = approve_oauth(&f, &client).await;
    let latest_work = admitted_work(&f, &latest_source).await;
    assert_ne!(replacement.id, latest.id);
    let outbox = source_outbox_rows(&f.db).await;
    assert_eq!(outbox.len(), 2);
    for row in outbox {
        replay_source_outbox(&f.db, &row).await;
    }
    assert_work(&f.db, &new_work, "revoked", "revoked").await;
    assert_work(&f.db, &latest_work, "running", "pending").await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_actual_consent_failure_rolls_back_consent_grant_and_outbox() {
    use crate::services::{consent_service, machine_oauth_consent_service::MachineConsentApproval};
    let f = fixture("srcbox_consent_abort").await;
    let client = oauth_client(&f).await;
    let (consent, grant, source) = approve_oauth(&f, &client).await;
    let work = admitted_work(&f, &source).await;
    let before = f.grant(&grant.id).await;
    let result = consent_service::grant_consent_with_machine(
        &f.db,
        &f.user,
        &client,
        &consent.scopes,
        false,
        Some(vec![]),
        Some(&MachineConsentApproval {
            request_id: Uuid::new_v4().to_string(),
            signed_request: "missing-selection".into(),
        }),
    )
    .await;
    assert!(
        result.is_err(),
        "failure follows consent replacement and invalidation inside its transaction"
    );
    assert!(source_outbox_rows(&f.db).await.is_empty());
    assert_eq!(f.grant(&grant.id).await, before);
    let current = consent_service::check_consent(&f.db, &f.user, &client, "machine")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.revision, consent.revision);
    assert_work(&f.db, &work, "running", "pending").await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_actual_client_security_update_preserves_unrelated_client_and_new_epoch() {
    use crate::services::oauth_client_service::{self as clients, AdminUpdateClient};
    let f = fixture("srcbox_client").await;
    let client = oauth_client(&f).await;
    let other = oauth_client(&f).await;
    let (_, grant, source) = approve_oauth(&f, &client).await;
    let (_, other_grant, other_source) = approve_oauth(&f, &other).await;
    let old_work = admitted_work(&f, &source).await;
    let unrelated_work = admitted_work(&f, &other_source).await;
    clients::admin_update_client(
        &f.db,
        &client,
        AdminUpdateClient {
            client_name: Some("Display only"),
            redirect_uris: None,
            allowed_scopes: None,
            broker_capability_enabled: None,
            is_active: None,
        },
    )
    .await
    .unwrap();
    assert!(source_outbox_rows(&f.db).await.is_empty());
    clients::delete_client(&f.db, &client).await.unwrap();
    assert!(admit(&f, &source, None).await.is_err());
    clients::admin_update_client(
        &f.db,
        &client,
        AdminUpdateClient {
            client_name: None,
            redirect_uris: None,
            allowed_scopes: None,
            broker_capability_enabled: None,
            is_active: Some(true),
        },
    )
    .await
    .unwrap();
    let (_, replacement, new_source) = approve_oauth(&f, &client).await;
    assert_ne!(grant.id, replacement.id);
    let new_work = admitted_work(&f, &new_source).await;
    let outbox = source_outbox_rows(&f.db).await;
    assert_eq!(outbox.len(), 1);
    replay_source_outbox(&f.db, &outbox[0]).await;
    assert_work(&f.db, &old_work, "revoked", "revoked").await;
    assert_work(&f.db, &new_work, "running", "pending").await;
    assert_work(&f.db, &unrelated_work, "running", "pending").await;
    assert_eq!(
        f.grant(&other_grant.id).await.revision,
        other_grant.revision
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_expiry_sweep_reaches_parent_consent_and_grant_without_touching_live_grants()
{
    let f = fixture("srcbox_expiry").await;
    let key_grant = f.approve().await;
    let key_source =
        VerifiedExternalSource::from_verified_api_key(&f.current_key().await, None).unwrap();
    let key_work = admitted_work(&f, &key_source).await;
    let client = oauth_client(&f).await;
    let (consent, oauth_grant, oauth_source) = approve_oauth(&f, &client).await;
    let oauth_work = admitted_work(&f, &oauth_source).await;
    let other_client = oauth_client(&f).await;
    let (_, live_grant, live_source) = approve_oauth(&f, &other_client).await;
    let live_work = admitted_work(&f, &live_source).await;
    let past = bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1));
    // Move only fixture clock deadlines; the mutation under test is the expiry
    // producer. Revocation tests use actual update/revoke entrypoints above.
    f.db.collection::<Document>(API_KEYS)
        .update_one(doc! {"_id":&f.key.id}, doc! {"$set":{"expires_at":past}})
        .await
        .unwrap();
    f.db.collection::<Document>(CONSENTS)
        .update_one(doc! {"_id":&consent.id}, doc! {"$set":{"expires_at":past}})
        .await
        .unwrap();
    assert!(admit(&f, &key_source, None).await.is_err());
    assert!(admit(&f, &oauth_source, None).await.is_err());
    let mut after = None;
    let mut revoked = 0;
    loop {
        let batch = sweep_invalid_sources(&f.db, after.as_deref(), 1)
            .await
            .unwrap();
        assert!(batch.examined <= 1);
        revoked += batch.invalidated;
        after = batch.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(revoked, 2);
    assert_eq!(
        f.grant(&key_grant.id).await.revision,
        key_grant.revision + 1
    );
    assert_eq!(
        f.grant(&oauth_grant.id).await.revision,
        oauth_grant.revision + 1
    );
    assert_eq!(f.grant(&live_grant.id).await.revision, live_grant.revision);
    for row in source_outbox_rows(&f.db).await {
        replay_source_outbox(&f.db, &row).await;
    }
    assert_work(&f.db, &key_work, "revoked", "revoked").await;
    assert_work(&f.db, &oauth_work, "revoked", "revoked").await;
    assert_work(&f.db, &live_work, "running", "pending").await;
    f.db.collection::<Document>(GRANTS)
        .update_one(
            doc! {"_id":&live_grant.id},
            doc! {"$set":{"expires_at":past}},
        )
        .await
        .unwrap();
    assert!(admit(&f, &live_source, None).await.is_err());
    let batch = sweep_invalid_sources(&f.db, None, 100).await.unwrap();
    assert_eq!(batch.invalidated, 1);
    let (_, replacement, replacement_source) = approve_oauth(&f, &other_client).await;
    assert_ne!(replacement.id, live_grant.id);
    let new_work = admitted_work(&f, &replacement_source).await;
    for row in source_outbox_rows(&f.db).await {
        replay_source_outbox(&f.db, &row).await;
    }
    assert_work(&f.db, &live_work, "revoked", "revoked").await;
    assert_work(&f.db, &new_work, "running", "pending").await;
    assert_eq!(
        sweep_invalid_sources(&f.db, None, 100)
            .await
            .unwrap()
            .invalidated,
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_child_expiry_is_atomic_bounded_and_precise() {
    use crate::services::machine_external_source_service as sources;
    let f = fixture("srcbox_child_expiry").await;
    let grant = f.approve().await;
    let a = child(&f).await;
    let b = child(&f).await;
    let key = f.current_key().await;
    let a_source = VerifiedExternalSource::from_verified_api_key(&key, Some(&a.id)).unwrap();
    let b_source = VerifiedExternalSource::from_verified_api_key(&key, Some(&b.id)).unwrap();
    let a_work = admitted_work(&f, &a_source).await;
    let b_work = admitted_work(&f, &b_source).await;
    f.db.collection::<Document>(CREDENTIALS).update_one(doc! {"_id":&a.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(admit(&f, &a_source, None).await.is_err());
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert_eq!(
        sources::expire_children_in_session(&f.db, 1, &mut session)
            .await
            .unwrap(),
        1
    );
    assert!(source_outbox_rows(&f.db).await.is_empty());
    session.abort_transaction().await.unwrap();
    assert!(
        !work_row(&f.db, CREDENTIALS, &a.id)
            .await
            .contains_key("machine_external_expiry_enqueued_at")
    );
    assert_eq!(sources::sweep_expired_children(&f.db, 1).await.unwrap(), 1);
    assert_eq!(sources::sweep_expired_children(&f.db, 1).await.unwrap(), 0);
    let rows = source_outbox_rows(&f.db).await;
    assert_eq!(rows.len(), 1);
    replay_source_outbox(&f.db, &rows[0]).await;
    assert_work(&f.db, &a_work, "revoked", "revoked").await;
    assert_work(&f.db, &b_work, "running", "pending").await;
    assert_eq!(f.grant(&grant.id).await.revision, grant.revision);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_client_epoch_sweep_finishes_inline_batch_without_revoking_new_authorization()
{
    use crate::services::oauth_client_service;
    let f = fixture("srcbox_client_batch").await;
    let client = oauth_client(&f).await;
    let (consent, original, source) = approve_oauth(&f, &client).await;
    let old_work = admitted_work(&f, &source).await;
    let mut rows = Vec::new();
    let mut users = Vec::new();
    let mut consents = Vec::new();
    // Keep each source valid before the client mutation. Missing owners or
    // consent rows would let this test pass without exercising client epochs.
    // Two inline 100-row security-edit batches leave a real remainder. Redirect
    // edits preserve consent rows, isolating the epoch check from their removal.
    for _ in 0..205 {
        let mut grant = original.clone();
        grant.id = Uuid::new_v4().to_string();
        grant.user_id = Uuid::new_v4().to_string();
        users.push(test_user(&grant.user_id, UserType::Person));
        let mut other_consent = consent.clone();
        other_consent.id = Uuid::new_v4().to_string();
        other_consent.user_id = grant.user_id.clone();
        if let ExternalGrantBinding::OAuth { consent_id, .. } = &mut grant.binding {
            *consent_id = other_consent.id.clone();
        }
        consents.push(other_consent);
        rows.push(grant);
    }
    f.db.collection::<User>(USERS)
        .insert_many(users)
        .await
        .unwrap();
    f.db.collection::<Consent>(CONSENTS)
        .insert_many(consents)
        .await
        .unwrap();
    let extra_source = rows.last().unwrap().clone();
    f.db.collection::<MachineExternalGrant>(GRANTS)
        .insert_many(rows)
        .await
        .unwrap();
    assert!(source_and_grant_valid(&f.db, &extra_source).await.unwrap());
    oauth_client_service::update_redirect_uris(
        &f.db,
        &client,
        &["https://example.invalid/changed".into()],
    )
    .await
    .unwrap();
    assert_eq!(source_outbox_rows(&f.db).await.len(), 100);
    assert!(admit(&f, &source, None).await.is_err());
    oauth_client_service::update_redirect_uris(
        &f.db,
        &client,
        &["https://example.invalid/callback".into()],
    )
    .await
    .unwrap();
    assert_eq!(source_outbox_rows(&f.db).await.len(), 200);
    let unswept =
        f.db.collection::<MachineExternalGrant>(GRANTS)
            .find_one(doc! {"status":"active"})
            .await
            .unwrap()
            .unwrap();
    assert!(!source_and_grant_valid(&f.db, &unswept).await.unwrap());
    if let ExternalGrantBinding::OAuth { consent_id, .. } = &unswept.binding {
        assert!(
            f.db.collection::<Consent>(CONSENTS)
                .find_one(doc! {"_id":consent_id})
                .await
                .unwrap()
                .is_some()
        );
    }
    let (_, replacement, new_source) = approve_oauth(&f, &client).await;
    let new_work = admitted_work(&f, &new_source).await;
    let remaining = f.db.collection::<MachineExternalGrant>(GRANTS)
        .count_documents(doc! {"status":"active","client_security_revision":{"$lt":replacement.client_security_revision}})
        .await.unwrap();
    assert!((5..=6).contains(&remaining));
    let batch = sweep_invalid_sources(&f.db, None, 100).await.unwrap();
    assert_eq!(batch.invalidated, remaining);
    assert_eq!(source_outbox_rows(&f.db).await.len(), 206);
    assert_eq!(
        sweep_invalid_sources(&f.db, None, 100)
            .await
            .unwrap()
            .invalidated,
        0
    );
    for row in source_outbox_rows(&f.db).await {
        replay_source_outbox(&f.db, &row).await;
    }
    assert_work(&f.db, &old_work, "revoked", "revoked").await;
    assert_work(&f.db, &new_work, "running", "pending").await;
    assert_eq!(f.grant(&replacement.id).await.revision, 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn source_outbox_failure_aborts_actual_oauth_client_security_update() {
    let f = fixture("srcbox_client_abort").await;
    let client = oauth_client(&f).await;
    let (_, grant, _) = approve_oauth(&f, &client).await;
    let before = f.grant(&grant.id).await;
    f.db.create_collection(crate::models::machine_access::OUTBOX)
        .validator(doc! {"test_only_reject_receipts":{"$exists":true}})
        .await
        .unwrap();
    let result = crate::services::oauth_client_service::delete_client(&f.db, &client).await;
    assert!(result.is_err());
    let client_row = work_row(&f.db, CLIENTS, &client).await;
    assert!(client_row.get_bool("is_active").unwrap());
    assert!(!client_row.contains_key("machine_external_security_revision"));
    assert_eq!(f.grant(&grant.id).await, before);
    assert!(source_outbox_rows(&f.db).await.is_empty());
    assert!(
        crate::services::consent_service::check_consent(&f.db, &f.user, &client, "machine")
            .await
            .unwrap()
            .is_some()
    );
    f.db.drop().await.unwrap();
}

#[test]
fn verified_external_source_debug_omits_original_parent_child_and_consent() {
    let owner_reference = SourceReference::ApiKey {
        api_key_id: "sentinel-parent-key-id".into(),
    };
    for debug in [
        format!("{owner_reference:?}"),
        format!("{owner_reference:#?}"),
    ] {
        assert!(!debug.contains("sentinel-parent-key-id"));
        assert!(debug.contains("REDACTED"));
    }
    let source = VerifiedExternalSource {
        user_id: "sentinel-owner-id".into(),
        source: Source::ApiKey {
            api_key_id: "sentinel-parent-key-id".into(),
            credential_id: Some("sentinel-child-key-id".into()),
        },
        expires_at: Utc::now() + Duration::hours(1),
        service_ceiling: Some(vec![]),
        node_ceiling: Some(vec!["sentinel-node-id".into()]),
        discovery_possible: true,
    };
    for debug in [
        format!("{source:?}"),
        format!("{source:#?}"),
        format!("{:?}", source.source),
        format!("{:#?}", source.source),
    ] {
        for sentinel in [
            "sentinel-owner-id",
            "sentinel-parent-key-id",
            "sentinel-child-key-id",
            "sentinel-node-id",
        ] {
            assert!(!debug.contains(sentinel));
        }
        assert!(debug.contains("REDACTED"));
    }
    let oauth = Source::OAuth {
        client_id: "sentinel-oauth-client-id".into(),
        consent: ProtectedConsentRef {
            consent_id: "sentinel-consent-id".into(),
            consent_revision: "sentinel-consent-version".into(),
        },
    };
    for debug in [format!("{oauth:?}"), format!("{oauth:#?}")] {
        assert!(!debug.contains("sentinel"));
    }
}

#[tokio::test]
async fn external_grants_require_explicit_legacy_warning_consent() {
    let f = fixture("external_explicit_legacy").await;
    let mut selection = f.selection();
    selection.separation = None;
    selection.legacy_warning_acknowledged = false;
    let error = validate_selection(&f.db, &f.user, &selection)
        .await
        .unwrap_err();
    assert!(
        matches!(error, crate::errors::AppError::ValidationError(ref text) if text.contains("node tokens") && text.contains("signing secrets") && text.contains("stored credentials") && text.contains("share the host kernel"))
    );
    selection.separation = Some(nyxid_machine::context::Choice::SharedLegacy);
    assert!(
        validate_selection(&f.db, &f.user, &selection)
            .await
            .is_err()
    );
    for choice in [
        nyxid_machine::context::Choice::Automatic,
        nyxid_machine::context::Choice::RequireSeparated,
    ] {
        selection.separation = Some(choice);
        selection.legacy_warning_acknowledged = true;
        let error = validate_selection(&f.db, &f.user, &selection)
            .await
            .unwrap_err();
        assert!(
            matches!(error, crate::errors::AppError::ValidationError(ref text) if text.contains("does not support separated contexts"))
        );
    }
    selection.separation = Some(nyxid_machine::context::Choice::SharedLegacy);
    validate_selection(&f.db, &f.user, &selection)
        .await
        .unwrap();
    f.db.drop().await.unwrap();
}
