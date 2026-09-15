use super::*;
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::models::user::{COLLECTION_NAME as USERS, UserType};
use crate::services::node_ws_manager::{
    NodeCapabilitiesMsg, NodeOutboundMessage, NodeProxyResponse,
};
use crate::test_utils::*;
use mongodb::bson::{self, Document};
use tokio::sync::mpsc;

struct Fixture {
    state: AppState,
    caller: ValidationCaller,
    service: UserService,
    key: UserApiKey,
    node_id: String,
    outbound: mpsc::Receiver<NodeOutboundMessage>,
}

async fn fixture(prefix: &str, capable: bool) -> Option<Fixture> {
    let db = connect_test_database(prefix).await?;
    coordination_service::ensure_indexes(&db).await.unwrap();
    crate::db::ensure_service_validation_indexes(&db)
        .await
        .unwrap();
    let mut state = test_app_state(db.clone());
    // Mirror startup so chained-audit assertions also hold in focused test runs.
    static AUDIT_INIT: std::sync::Once = std::sync::Once::new();
    AUDIT_INIT.call_once(|| {
        crate::services::audit_service::init_audit_chain_hmac_key(
            state.audit_chain_hmac_key.as_ref().clone(),
        );
    });
    state.config.node_hmac_signing_enabled = false;
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let catalog = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    db.collection::<Document>("downstream_services").insert_one(doc! {
        "_id": &catalog, "name": "GitHub", "slug": "api-github", "base_url": "https://api.github.com",
        "auth_method": "bearer", "auth_key_name": "Authorization", "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "is_active": true, "created_by": &owner, "created_at": now, "updated_at": now,
    }).await.unwrap();
    let node_id = uuid::Uuid::new_v4().to_string();
    let node: Node = bson::from_document(doc! {
        "_id": &node_id, "user_id": &owner, "name": "Validation test node", "status": "online",
        "auth_token_hash": "test-hash", "signing_secret_hash": "test-hash", "is_active": true,
        "created_at": now, "updated_at": now,
    })
    .unwrap();
    db.collection::<Node>(NODES).insert_one(node).await.unwrap();
    let (tx, outbound) = mpsc::channel(64);
    register_test_node_connection(&state, &node_id, tx).await;
    state.node_ws_manager.record_capabilities(
        &node_id,
        &NodeCapabilitiesMsg {
            no_redirect_proxy: capable,
            credential_revisions: Some(std::collections::BTreeMap::from([(
                "api-github".into(),
                "a".repeat(64),
            )])),
            ..Default::default()
        },
    );
    let endpoint = test_user_endpoint(
        &uuid::Uuid::new_v4().to_string(),
        &owner,
        "GitHub",
        "https://api.github.com",
        None,
        Some(&catalog),
    );
    db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(&endpoint)
        .await
        .unwrap();
    let mut key: UserApiKey = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": &owner, "label": "validation fixture",
        "credential_type": "api_key", "status": "active", "created_at": now, "updated_at": now,
    })
    .unwrap();
    key.credential_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"fixture-token")
            .await
            .unwrap(),
    );
    db.collection::<UserApiKey>(USER_API_KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    let mut service = test_user_service(
        &uuid::Uuid::new_v4().to_string(),
        &owner,
        "api-github",
        &endpoint.id,
        Some(&catalog),
        Some(&node_id),
    );
    service.auth_method = "bearer".into();
    service.auth_key_name = "Authorization".into();
    service.api_key_id = Some(key.id.clone());
    db.collection::<UserService>(USER_SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    let caller = ValidationCaller {
        session_id: None,
        user_id: owner,
        context: CallerContext::Human {
            session: uuid::Uuid::new_v4().to_string(),
        },
        allow_all_services: true,
        allowed_service_ids: vec![],
        allow_all_nodes: true,
        allowed_node_ids: vec![],
    };
    Some(Fixture {
        state,
        caller,
        service,
        key,
        node_id,
        outbound,
    })
}

fn begin(f: &Fixture, force: bool) -> tokio::task::JoinHandle<AppResult<ServiceValidationRecord>> {
    let (state, caller, id) = (f.state.clone(), f.caller.clone(), f.service.id.clone());
    tokio::spawn(async move { validate(&state, caller, &id, force).await })
}

async fn request(f: &mut Fixture) -> serde_json::Value {
    let NodeOutboundMessage::Text(frame) =
        tokio::time::timeout(Duration::from_secs(3), f.outbound.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("expected proxy request")
    };
    let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(value["type"], "proxy_request");
    assert_eq!(value["follow_redirects"], false);
    assert_eq!(value["path"], "user");
    value
}

fn respond(
    f: &Fixture,
    frame: &serde_json::Value,
    status: u16,
    body: &[u8],
    headers: Vec<(String, String)>,
) {
    f.state.node_ws_manager.deliver_proxy_response(
        &f.node_id,
        NodeProxyResponse {
            request_id: frame["request_id"].as_str().unwrap().into(),
            status,
            headers,
            body: body.to_vec(),
        },
    );
}

async fn validation_audit(db: &mongodb::Database) -> Document {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(audit) = db
                .collection::<Document>("audit_log")
                .find_one(doc! { "event_type": "service_validation_checked" })
                .await
                .unwrap()
            {
                return audit;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the settled observation must be audited")
}

#[tokio::test]
async fn validation_db_joins_attempt_reuses_freshness_and_never_changes_status() {
    let Some(mut f) = fixture("validation_join", true).await else {
        return;
    };
    let (checking, lease, admission) = pending_attempt(&f).await;
    let state = f.state.clone();
    let caller = f.caller.clone();
    // Settle while retaining the attempt lease to expose the cleanup window
    // deterministically. A new force request must wait, then apply cooldown.
    let worker = tokio::spawn(async move {
        let dispatched = AtomicBool::new(false);
        observe(
            &state,
            &caller,
            validator_profiles::for_slug("api-github"),
            checking,
            &admission,
            &dispatched,
        )
        .await
        .unwrap_or_else(|failure| panic!("observation failed: {}", failure.reason_code()));
        assert!(dispatched.load(Ordering::Relaxed));
        admission
    });
    let first = begin(&f, false);
    let frame = request(&mut f).await;
    let second = begin(&f, true);
    tokio::time::sleep(Duration::from_millis(100)).await;
    respond(
        &f,
        &frame,
        401,
        br#"{"message":"Bad credentials fixture-token"}"#,
        vec![],
    );
    let a = first.await.unwrap().unwrap();
    let b = second.await.unwrap().unwrap();
    assert_eq!(a.attempt_id, b.attempt_id);
    assert_eq!(a.outcome, ValidationOutcome::CredentialRejected);
    assert_eq!(
        validate(&f.state, f.caller.clone(), &f.service.id, false)
            .await
            .unwrap()
            .attempt_id,
        a.attempt_id
    );
    let admission = worker.await.unwrap();
    let forced = begin(&f, true);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !forced.is_finished(),
        "a new check must not join a settled attempt"
    );
    release_admission(&f.state.db, &admission).await;
    LeaseStore::release(&f.state.db, &lease).await.unwrap();
    assert!(matches!(
        forced.await.unwrap(),
        Err(AppError::ServiceValidationRateLimited)
    ));
    assert!(f.outbound.try_recv().is_err());
    let key = f
        .state
        .db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": &f.key.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key.status, "active");
    assert_eq!(key.credential_epoch, 1);
    let audit = validation_audit(&f.state.db).await;
    let encoded = serde_json::to_string(&audit).unwrap();
    assert!(!encoded.contains("fixture-token"));
    assert!(!encoded.contains("Bad credentials"));
    assert!(audit.contains_key("seq"));
}

#[tokio::test]
async fn validation_db_node_upgrade_required_and_offline_do_not_probe() {
    let Some(mut f) = fixture("validation_node_gate", false).await else {
        return;
    };
    let record = validate(&f.state, f.caller.clone(), &f.service.id, false)
        .await
        .unwrap();
    assert_eq!(record.outcome, ValidationOutcome::Unsupported);
    assert_eq!(record.reason_code, "node_agent_upgrade_required");
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(record.valid_until, record.checked_at);
    // Capability changes do not affect the execution digest. The next explicit
    // check must still see the upgrade and send a probe without force.
    f.state.node_ws_manager.record_capabilities(
        &f.node_id,
        &NodeCapabilitiesMsg {
            no_redirect_proxy: true,
            credential_revisions: Some(std::collections::BTreeMap::from([(
                "api-github".into(),
                "a".repeat(64),
            )])),
            ..Default::default()
        },
    );
    let upgraded = begin(&f, false);
    let frame = request(&mut f).await;
    respond(&f, &frame, 200, br#"{"id":123,"login":"fixture"}"#, vec![]);
    let upgraded = upgraded.await.unwrap().unwrap();
    assert_ne!(upgraded.attempt_id, record.attempt_id);
    assert_eq!(upgraded.outcome, ValidationOutcome::Authenticated);
    assert!(upgraded.valid_until > upgraded.checked_at);
    // Routing edits do not create another provider budget. Advance only the
    // lease clock before testing offline routing after the successful probe.
    f.state.db.collection::<Document>(LEASE_COLLECTION_NAME).update_many(
        doc! { "_id": { "$regex": "^service-validation-cooldown:" } },
        doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1)) } },
    ).await.unwrap();
    let offline = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<UserService>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "node_id": offline } },
        )
        .await
        .unwrap();
    let record = validate(&f.state, f.caller.clone(), &f.service.id, false)
        .await
        .unwrap();
    assert_eq!(record.outcome, ValidationOutcome::TransportUnknown);
    assert_eq!(record.valid_until, record.checked_at);
    let retried = validate(&f.state, f.caller.clone(), &f.service.id, false)
        .await
        .unwrap();
    assert_eq!(retried.outcome, ValidationOutcome::TransportUnknown);
    assert_ne!(retried.attempt_id, record.attempt_id);
    assert!(f.outbound.try_recv().is_err());
}

#[tokio::test]
async fn validation_db_stale_rejection_after_refresh_is_discarded() {
    let Some(mut f) = fixture("validation_refresh", true).await else {
        return;
    };
    let first = begin(&f, false);
    let frame = request(&mut f).await;
    // Simulate the coordinated refresh winner publishing different token material
    // without an epoch bump while an old observation is in flight.
    let refreshed = f
        .state
        .encryption_keys
        .encrypt(b"new-fixture-token")
        .await
        .unwrap();
    f.state.db.collection::<UserApiKey>(USER_API_KEYS).update_one(doc! { "_id": &f.key.id }, doc! { "$set": { "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: refreshed } } }).await.unwrap();
    respond(&f, &frame, 401, br#"{"message":"Bad credentials"}"#, vec![]);
    let observation = first.await.unwrap().unwrap();
    assert_eq!(observation.outcome, ValidationOutcome::TransportUnknown);
    assert_eq!(observation.reason_code, "attempt_superseded");
    let record = f
        .state
        .db
        .collection::<ServiceValidationRecord>(COLLECTION_NAME)
        .find_one(doc! { "user_service_id": &f.service.id })
        .await
        .unwrap()
        .unwrap();
    assert!(record.completed);
    assert_eq!(record.reason_code, "attempt_superseded");
    assert_eq!(record.outcome, ValidationOutcome::TransportUnknown);
    let audit = validation_audit(&f.state.db).await;
    assert!(
        serde_json::to_string(&audit)
            .unwrap()
            .contains("attempt_superseded")
    );
}

#[tokio::test]
async fn validation_db_freshness_binds_profile_execution_and_credential_revision() {
    let Some(mut f) = fixture("validation_fresh", true).await else {
        return;
    };
    let run = begin(&f, false);
    let frame = request(&mut f).await;
    respond(&f, &frame, 200, br#"{"login":"octocat"}"#, vec![]);
    let record = run.await.unwrap().unwrap();
    let live = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    assert!(fresh(&record, &live, 1));
    assert!(!fresh(&record, &live, 2));
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "_id": &f.service.endpoint_id },
            doc! { "$set": { "url": "https://api.github.com/changed" } },
        )
        .await
        .unwrap();
    let changed = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    assert!(!fresh(&record, &changed, 1));
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "_id": &f.service.endpoint_id },
            doc! { "$set": { "url": "https://api.github.com" } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<UserApiKey>(USER_API_KEYS)
        .update_one(
            doc! { "_id": &f.key.id },
            doc! { "$set": { "token_scopes": "changed_scope" } },
        )
        .await
        .unwrap();
    let changed = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    assert!(!fresh(&record, &changed, 1));
    let mut pending = record.clone();
    pending.completed = false;
    pending.attempt_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<ServiceValidationRecord>(COLLECTION_NAME)
        .replace_one(doc! { "_id": &record.id }, &pending)
        .await
        .unwrap();
    assert!(!finish_observation(&f.state.db, &record).await.unwrap());
    let mut winner = pending.clone();
    winner.completed = true;
    assert!(finish_observation(&f.state.db, &winner).await.unwrap());
    let mut duplicate = winner;
    duplicate.id = uuid::Uuid::new_v4().to_string();
    assert!(
        f.state
            .db
            .collection::<ServiceValidationRecord>(COLLECTION_NAME)
            .insert_one(duplicate)
            .await
            .is_err()
    );
    let encoded = bson::to_document(&record).unwrap();
    assert!(matches!(
        encoded.get("checked_at"),
        Some(bson::Bson::DateTime(_))
    ));
    assert!(matches!(
        encoded.get("valid_until"),
        Some(bson::Bson::DateTime(_))
    ));
}

#[tokio::test]
async fn validation_db_retry_after_cannot_be_shortened_by_renewal() {
    let Some(mut f) = fixture("validation_retry", true).await else {
        return;
    };
    let run = begin(&f, false);
    let frame = request(&mut f).await;
    respond(
        &f,
        &frame,
        429,
        b"{}",
        vec![("retry-after".into(), "600".into())],
    );
    let record = run.await.unwrap().unwrap();
    assert_eq!(
        record.outcome,
        ValidationOutcome::RateLimited {
            retry_after: Some(Duration::from_secs(600))
        }
    );
    let lease = f
        .state
        .db
        .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
        .find_one(doc! { "_id": { "$regex": "^service-validation-cooldown:" } })
        .await
        .unwrap()
        .unwrap();
    let token = LeaseToken {
        name: lease.id,
        holder: lease.holder,
        lease_id: lease.lease_id,
    };
    assert!(
        extend_cooldown(&f.state.db, &token, MIN_PROBE_INTERVAL)
            .await
            .unwrap()
    );
    let latest = f
        .state
        .db
        .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
        .find_one(doc! { "_id": &token.name })
        .await
        .unwrap()
        .unwrap();
    assert!(latest.expires_at >= lease.expires_at);
    assert!(latest.expires_at > Utc::now() + chrono::Duration::seconds(590));
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "custom_user_agent": "edited-agent" } },
        )
        .await
        .unwrap();
    assert!(matches!(
        validate(&f.state, f.caller.clone(), &f.service.id, true).await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    let mut alias = f.service.clone();
    alias.id = uuid::Uuid::new_v4().to_string();
    alias.slug = "second-alias".into();
    f.state
        .db
        .collection::<UserService>(USER_SERVICES)
        .insert_one(&alias)
        .await
        .unwrap();
    assert!(matches!(
        validate(&f.state, f.caller.clone(), &alias.id, true).await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    assert!(f.outbound.try_recv().is_err());
}

#[tokio::test]
async fn validation_db_shared_deployment_and_session_admission() {
    let Some(f) = fixture("validation_slots", true).await else {
        return;
    };
    let holder = &coordination_service::cluster_lease_runtime().holder;
    let mut slots = vec![];
    for _ in 0..32 {
        slots.push(
            SlotStore::acquire(
                &f.state.db,
                "service-validation-deployment",
                "deployment",
                32,
                holder,
                LEASE_TTL,
            )
            .await
            .unwrap()
            .unwrap(),
        );
    }
    assert!(matches!(
        validate(&f.state, f.caller.clone(), &f.service.id, true).await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    for slot in slots {
        SlotStore::release(&f.state.db, &slot).await.unwrap();
    }
    let session = f.caller.session_id.as_deref().unwrap_or(&f.caller.user_id);
    for _ in 0..2 {
        SlotStore::acquire(
            &f.state.db,
            "service-validation-session",
            session,
            2,
            holder,
            LEASE_TTL,
        )
        .await
        .unwrap()
        .unwrap();
    }
    assert!(matches!(
        validate(&f.state, f.caller.clone(), &f.service.id, true).await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn validation_db_acl_read_disclosure_then_actual_proxy_permission() {
    use crate::handlers::keys::{ValidateKeyRequest, validate_key};
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
    use crate::services::billing::route_inventory::BillingRoutePolicy;
    use axum::{
        Extension, Json,
        extract::{Path, State},
    };
    let Some(mut f) = fixture("validation_acl", true).await else {
        return;
    };
    let org = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection::<UserService>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "user_id": &org } },
        )
        .await
        .unwrap();
    let member = test_membership(&org, &f.caller.user_id, OrgRole::Viewer, None);
    f.state
        .db
        .collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(&member)
        .await
        .unwrap();
    let call = || {
        validate_key(
            State(f.state.clone()),
            test_auth_user(&f.caller.user_id),
            Path(f.service.id.clone()),
            Extension(BillingRoutePolicy::Exempt("service_validation")),
            Json(ValidateKeyRequest::default()),
        )
    };
    assert!(matches!(
        call().await,
        Err(AppError::OrgRoleInsufficient(_))
    ));
    f.state
        .db
        .collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! { "_id": &member.id },
            doc! { "$set": { "role": "member" } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<UserService>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "admin_only": true } },
        )
        .await
        .unwrap();
    assert!(matches!(
        call().await,
        Err(AppError::OrgRoleInsufficient(_))
    ));
    let outsider = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&outsider, UserType::Person))
        .await
        .unwrap();
    assert!(matches!(
        validate_key(
            State(f.state.clone()),
            test_auth_user(&outsider),
            Path(f.service.id.clone()),
            Extension(BillingRoutePolicy::Exempt("service_validation")),
            Json(ValidateKeyRequest::default())
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(f.outbound.try_recv().is_err());
}

#[tokio::test]
async fn validation_db_node_and_ssh_credentials_are_unsupported() {
    let Some(mut f) = fixture("validation_types", true).await else {
        return;
    };
    for kind in ["node_managed", "ssh_certificate"] {
        f.state
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &f.key.id },
                doc! { "$set": { "credential_type": kind }, "$inc": { "credential_epoch": 1 } },
            )
            .await
            .unwrap();
        let record = validate(&f.state, f.caller.clone(), &f.service.id, false)
            .await
            .unwrap();
        assert_eq!(record.outcome, ValidationOutcome::Unsupported);
        assert!(f.outbound.try_recv().is_err());
    }
}

#[tokio::test]
async fn validation_db_streamed_node_body_is_bounded_and_timed_out() {
    for slow in [false, true] {
        let Some(mut f) = fixture("validation_stream", true).await else {
            return;
        };
        let run = begin(&f, false);
        let frame = request(&mut f).await;
        let id = frame["request_id"].as_str().unwrap();
        assert!(
            f.state
                .node_ws_manager
                .deliver_stream_start(&f.node_id, id, 200, vec![])
        );
        if !slow {
            f.state
                .node_ws_manager
                .deliver_stream_chunk(&f.node_id, id, vec![b'x'; 32 * 1024 + 1]);
        }
        let record = run.await.unwrap().unwrap();
        assert_eq!(record.outcome, ValidationOutcome::TransportUnknown);
        assert_eq!(
            record.reason_code,
            if slow {
                "transport_unknown"
            } else {
                "response_too_large"
            }
        );
    }
}

#[tokio::test]
async fn validation_db_nonhuman_callers_are_rejected_before_resource_lookup() {
    use crate::handlers::keys::{ValidateKeyRequest, validate_key};
    use crate::mw::auth::AuthMethod;
    use crate::services::billing::route_inventory::BillingRoutePolicy;
    use axum::{
        Extension, Json,
        extract::{Path, State},
    };
    let Some(f) = fixture("validation_human", true).await else {
        return;
    };
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
    ] {
        let mut caller = test_auth_user(&f.caller.user_id);
        caller.auth_method = method;
        assert!(matches!(
            validate_key(
                State(f.state.clone()),
                caller,
                Path("unknown".into()),
                Extension(BillingRoutePolicy::Exempt("service_validation")),
                Json(ValidateKeyRequest::default())
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

async fn pending_attempt(f: &Fixture) -> (ServiceValidationRecord, LeaseToken, Admission) {
    let profile = validator_profiles::for_slug("api-github").unwrap();
    let live = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    let name = format!("service-validation:{}:{}", f.service.id, profile.id);
    let lease = LeaseStore::acquire(
        &f.state.db,
        &name,
        &coordination_service::cluster_lease_runtime().holder,
        LEASE_TTL,
    )
    .await
    .unwrap()
    .unwrap();
    let (record, admission) = start_attempt(&f.state, &f.caller, live, Some(profile), None, &lease)
        .await
        .unwrap();
    (record, lease, admission)
}

#[tokio::test]
async fn validation_db_aborted_attempts_settle_and_release_unsent_cooldown() {
    for reason in [
        "attempt_superseded",
        "credential_unavailable",
        "lease_lost",
        "internal_error",
    ] {
        let Some(mut f) = fixture("validation_abort", true).await else {
            return;
        };
        if reason == "credential_unavailable" {
            let encrypted = f.state.encryption_keys.encrypt(b"").await.unwrap();
            f.state
                .db
                .collection::<Document>(USER_API_KEYS)
                .update_one(
                    doc! { "_id": &f.key.id },
                    doc! { "$set": { "credential_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted,
                    } } },
                )
                .await
                .unwrap();
            f.state
                .db
                .collection::<Document>(USER_SERVICES)
                .update_one(
                    doc! { "_id": &f.service.id },
                    doc! { "$set": { "node_id": null } },
                )
                .await
                .unwrap();
        } else if reason == "internal_error" {
            // Corrupt ciphertext is an internal fault, not provider rejection.
            f.state
                .db
                .collection::<Document>(USER_API_KEYS)
                .update_one(
                    doc! { "_id": &f.key.id },
                    doc! { "$set": { "credential_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic, bytes: vec![42; 64],
                    } } },
                )
                .await
                .unwrap();
        }
        if reason == "internal_error" {
            f.state
                .db
                .collection::<Document>(USER_SERVICES)
                .update_one(
                    doc! { "_id": &f.service.id },
                    doc! { "$set": { "node_id": null } },
                )
                .await
                .unwrap();
        }
        let (record, lease, admission) = pending_attempt(&f).await;
        let cooldown = admission.cooldown.as_ref().unwrap().name.clone();
        if reason == "attempt_superseded" {
            f.state
                .db
                .collection::<Document>(USER_API_KEYS)
                .update_one(
                    doc! { "_id": &f.key.id },
                    doc! { "$set": { "token_scopes": "changed" } },
                )
                .await
                .unwrap();
        } else if reason == "lease_lost" {
            LeaseStore::release(&f.state.db, &lease).await.unwrap();
        }
        let started = tokio::time::Instant::now();
        run_attempt(
            f.state.clone(),
            f.caller.clone(),
            validator_profiles::for_slug("api-github"),
            record.clone(),
            lease,
            admission,
        )
        .await;
        let settled = completed_observation(
            &f.state,
            &f.caller,
            &f.service.id,
            "github_user_v1",
            1,
            Some(&record.attempt_id),
        )
        .await
        .unwrap()
        .expect("the settled observation must reach the handler");
        assert!(settled.completed);
        assert_eq!(settled.reason_code, reason);
        assert_eq!(settled.outcome, ValidationOutcome::TransportUnknown);
        assert!(settled.valid_until <= settled.checked_at);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(f.outbound.try_recv().is_err());
        let audit = f
            .state
            .db
            .collection::<Document>("audit_log")
            .find_one(doc! { "event_type": "service_validation_checked" })
            .await
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&audit).unwrap();
        assert!(serialized.contains(reason));
        assert!(!serialized.contains("fixture-token"));
        // Immediate reacquisition proves internal aborts consume no provider budget.
        assert!(
            LeaseStore::acquire(
                &f.state.db,
                &cooldown,
                &coordination_service::cluster_lease_runtime().holder,
                MIN_PROBE_INTERVAL
            )
            .await
            .unwrap()
            .is_some()
        );
    }
}

#[tokio::test]
async fn validation_db_poll_exits_when_attempt_lease_disappears() {
    let Some(f) = fixture("validation_poll_lost", true).await else {
        return;
    };
    let (record, lease, admission) = pending_attempt(&f).await;
    let state = f.state.clone();
    let caller = f.caller.clone();
    let service_id = f.service.id.clone();
    let lease_name = lease.name.clone();
    let attempt = record.attempt_id.clone();
    let waiter = tokio::spawn(async move {
        poll_attempt(
            &state,
            &caller,
            &service_id,
            "github_user_v1",
            1,
            &lease_name,
            &attempt,
        )
        .await
    });
    tokio::task::yield_now().await;
    LeaseStore::release(&f.state.db, &lease).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(AppError::ServiceValidationUnavailable)
    ));
    release_cooldown(&f.state.db, &admission).await;
    release_admission(&f.state.db, &admission).await;
}

#[tokio::test]
async fn validation_db_abort_cannot_overwrite_replacement_attempt() {
    let Some(f) = fixture("validation_abort_fence", true).await else {
        return;
    };
    let (record, lease, admission) = pending_attempt(&f).await;
    let replacement = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &record.id },
            doc! { "$set": { "attempt_id": &replacement } },
        )
        .await
        .unwrap();
    LeaseStore::release(&f.state.db, &lease).await.unwrap();
    run_attempt(
        f.state.clone(),
        f.caller.clone(),
        validator_profiles::for_slug("api-github"),
        record.clone(),
        lease,
        admission,
    )
    .await;
    let current = f
        .state
        .db
        .collection::<ServiceValidationRecord>(COLLECTION_NAME)
        .find_one(doc! { "_id": &record.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.attempt_id, replacement);
    assert!(!current.completed);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("audit_log")
            .count_documents(doc! { "event_type": "service_validation_checked" })
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn validation_db_probe_preserves_last_used_at() {
    let Some(mut f) = fixture("validation_usage", true).await else {
        return;
    };
    // Binding routing exercises the shared materializer's normal credential
    // branch, where proxy use schedules touch_last_used (explicit nodes return early).
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "node_id": null } },
        )
        .await
        .unwrap();
    let live = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<Document>("node_service_bindings")
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "node_id": &f.node_id,
            "user_id": &f.service.user_id, "service_id": &live.resolution.target.service.id,
            "is_active": true, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let last_used = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(1));
    f.state
        .db
        .collection::<Document>(USER_API_KEYS)
        .update_one(
            doc! { "_id": &f.key.id },
            doc! { "$set": { "last_used_at": last_used } },
        )
        .await
        .unwrap();
    let run = begin(&f, false);
    let frame = request(&mut f).await;
    respond(&f, &frame, 200, br#"{"login":"octocat"}"#, vec![]);
    assert_eq!(
        run.await.unwrap().unwrap().outcome,
        ValidationOutcome::Authenticated
    );
    // Allow any wrongly spawned usage-touch task to complete before reading.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let key = f
        .state
        .db
        .collection::<Document>(USER_API_KEYS)
        .find_one(doc! { "_id": &f.key.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key.get_datetime("last_used_at").unwrap(), &last_used);
}

#[tokio::test]
async fn validation_db_org_member_probes_org_node_without_node_management_acl() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
    for shared_node_owner in [true, false] {
        let Some(mut f) = fixture("validation_org_node", true).await else {
            return;
        };
        let org = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        f.state
            .db
            .collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(
                &org,
                &f.caller.user_id,
                OrgRole::Member,
                None,
            ))
            .await
            .unwrap();
        for (collection, id) in [
            (USER_SERVICES, &f.service.id),
            (USER_API_KEYS, &f.key.id),
            ("user_endpoints", &f.service.endpoint_id),
        ] {
            f.state
                .db
                .collection::<Document>(collection)
                .update_one(doc! { "_id": id }, doc! { "$set": { "user_id": &org } })
                .await
                .unwrap();
        }
        let node_owner = if shared_node_owner {
            org
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            f.state
                .db
                .collection::<crate::models::user::User>(USERS)
                .insert_one(test_user(&id, UserType::Org))
                .await
                .unwrap();
            id
        };
        f.state
            .db
            .collection::<Document>(NODES)
            .update_one(
                doc! { "_id": &f.node_id },
                doc! { "$set": { "user_id": node_owner } },
            )
            .await
            .unwrap();
        if !shared_node_owner {
            assert!(crate::services::node_service::get_node(
                &f.state.db, &f.caller.user_id, &f.node_id,
            ).await.is_err());
        }
        let run = begin(&f, false);
        let frame = request(&mut f).await;
        respond(&f, &frame, 200, br#"{"login":"octocat"}"#, vec![]);
        assert_eq!(
            run.await.unwrap().unwrap().outcome,
            ValidationOutcome::Authenticated
        );
    }
}

#[tokio::test]
async fn validation_db_handler_returns_settled_credential_abort() {
    use crate::handlers::keys::{ValidateKeyRequest, validate_key};
    use crate::services::billing::route_inventory::BillingRoutePolicy;
    use axum::{
        Extension, Json,
        extract::{Path, State},
    };
    let Some(mut f) = fixture("validation_handler_abort", true).await else {
        return;
    };
    let encrypted = f.state.encryption_keys.encrypt(b"").await.unwrap();
    f.state
        .db
        .collection::<Document>(USER_API_KEYS)
        .update_one(
            doc! { "_id": &f.key.id },
            doc! { "$set": { "credential_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted,
            } } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "node_id": null } },
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let Json(response) = validate_key(
            State(f.state.clone()),
            test_auth_user(&f.caller.user_id),
            Path(f.service.id.clone()),
            Extension(BillingRoutePolicy::Exempt("service_validation")),
            Json(ValidateKeyRequest::default()),
        )
        .await
        .expect("internal abort is a 200 observation and remains immediately retryable");
        assert_eq!(response.outcome, "transport_unknown");
        assert_eq!(response.reason_code, "credential_unavailable");
    }
    assert!(f.outbound.try_recv().is_err());
}

#[tokio::test]
async fn validation_db_openid_token_cannot_probe_despite_matching_allowlist() {
    use crate::handlers::keys::{ValidateKeyRequest, validate_key};
    use crate::services::billing::route_inventory::BillingRoutePolicy;
    use axum::{
        Extension, Json,
        extract::{Path, State},
    };
    let Some(mut f) = fixture("validation_token_scope", true).await else {
        return;
    };
    let mut auth = test_auth_user(&f.caller.user_id);
    auth.auth_method = crate::mw::auth::AuthMethod::AccessToken;
    auth.scope = "openid profile".into();
    auth.allow_all_services = false;
    auth.allowed_service_ids = vec![f.service.id.clone()];
    assert!(matches!(
        validate_key(
            State(f.state.clone()),
            auth,
            Path(f.service.id.clone()),
            Extension(BillingRoutePolicy::Exempt("service_validation")),
            Json(ValidateKeyRequest::default())
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn validation_db_app_admission_separates_humans_and_caps_app_slots() {
    let Some(mut f) = fixture("validation_app_slots", true).await else {
        return;
    };
    let app_id = uuid::Uuid::new_v4().to_string();
    f.caller.context = CallerContext::App {
        client_id: app_id.clone(),
    };
    let mut other = f.caller.clone();
    other.user_id = uuid::Uuid::new_v4().to_string();
    let a = acquire_admission(&f.state, &other).await.unwrap();
    let b = acquire_admission(&f.state, &other).await.unwrap();
    assert!(matches!(
        acquire_admission(&f.state, &other).await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    let c = acquire_admission(&f.state, &f.caller).await.unwrap();
    release_admission(&f.state.db, &a).await;
    release_admission(&f.state.db, &b).await;
    release_admission(&f.state.db, &c).await;
    // Sixteen distinct humans can check concurrently; the seventeenth is
    // bounded by the app limit before reaching the deployment's 32 slots.
    let mut held = vec![];
    for _ in 0..16 {
        other.user_id = uuid::Uuid::new_v4().to_string();
        held.push(acquire_admission(&f.state, &other).await.unwrap());
    }
    assert!(matches!(
        acquire_admission(&f.state, &f.caller).await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    for admission in held {
        release_admission(&f.state.db, &admission).await;
    }
    // Two sessions belonging to one human each have their own two slots.
    f.caller.session_id = Some(uuid::Uuid::new_v4().to_string());
    let a = acquire_admission(&f.state, &f.caller).await.unwrap();
    let b = acquire_admission(&f.state, &f.caller).await.unwrap();
    f.caller.session_id = Some(uuid::Uuid::new_v4().to_string());
    let c = acquire_admission(&f.state, &f.caller).await.unwrap();
    for admission in [a, b, c] {
        release_admission(&f.state.db, &admission).await;
    }
}

#[tokio::test]
async fn validation_db_node_revision_change_invalidates_local_and_remote_evidence() {
    let Some(mut f) = fixture("validation_node_revisions", true).await else {
        return;
    };
    let run = begin(&f, false);
    let frame = request(&mut f).await;
    respond(&f, &frame, 200, br#"{"login":"octocat"}"#, vec![]);
    let record = run.await.unwrap().unwrap();
    let live = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    assert!(fresh(&record, &live, 1));
    let revisions = std::collections::BTreeMap::from([("api-github".into(), "b".repeat(64))]);
    f.state.node_ws_manager.record_capabilities(
        &f.node_id,
        &NodeCapabilitiesMsg {
            no_redirect_proxy: true,
            credential_revisions: Some(revisions.clone()),
            ..Default::default()
        },
    );
    let changed = snapshot(&f.state, &f.caller, &f.service.id).await.unwrap();
    assert!(!fresh(&record, &changed, 1));
    let owner = f
        .state
        .db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": &f.node_id })
        .await
        .unwrap()
        .unwrap()
        .connection_owner
        .unwrap();
    let fence = super::super::node_owner_service::NodeOwnerFence::from_owner(&f.node_id, &owner);
    assert!(
        super::super::node_owner_service::record_capabilities(
            &f.state.db,
            &fence,
            f.state
                .node_ws_manager
                .session_info(&f.node_id)
                .capabilities,
            true,
            Some(&revisions)
        )
        .await
        .unwrap()
    );
    let remote = test_app_state(f.state.db.clone()).node_ws_manager;
    let (_, binding) = node_routing_service::validation_route(
        &f.state.db,
        &remote,
        Some(&f.node_id),
        &[],
        "api-github",
        None,
    )
    .await
    .unwrap();
    assert_eq!(binding, changed.node_credential);
    assert!(!evidence_is_fresh(
        &record,
        &live.digest,
        live.revision.as_deref(),
        binding.as_ref(),
        1,
        Utc::now()
    ));
    let mut stale_fence = fence.clone();
    stale_fence.connection_id = uuid::Uuid::new_v4().to_string();
    assert!(
        !super::super::node_owner_service::record_capabilities(
            &f.state.db,
            &stale_fence,
            Default::default(),
            true,
            None
        )
        .await
        .unwrap()
    );
    assert_eq!(
        node_routing_service::validation_route(
            &f.state.db,
            &remote,
            Some(&f.node_id),
            &[],
            "api-github",
            None
        )
        .await
        .unwrap()
        .1,
        binding
    );
}

#[tokio::test]
async fn validation_db_node_without_revisions_has_no_reuse_window() {
    let Some(mut f) = fixture("validation_node_legacy_revision", true).await else {
        return;
    };
    f.state.node_ws_manager.record_capabilities(
        &f.node_id,
        &NodeCapabilitiesMsg {
            no_redirect_proxy: true,
            ..Default::default()
        },
    );
    let run = begin(&f, false);
    let frame = request(&mut f).await;
    respond(&f, &frame, 200, br#"{"login":"octocat"}"#, vec![]);
    let record = run.await.unwrap().unwrap();
    assert_eq!(record.outcome, ValidationOutcome::Authenticated);
    assert_eq!(record.valid_until, record.checked_at);
    assert!(record.node_credential.unwrap().revision.is_none());
}

#[tokio::test]
async fn validation_db_node_telegram_path_contains_no_server_credential() {
    let Some(mut f) = fixture("validation_telegram_path", true).await else {
        return;
    };
    f.state.db.collection::<Document>("downstream_services").update_one(
        doc! { "_id": &f.service.catalog_service_id },
        doc! { "$set": { "slug": "api-telegram-bot", "base_url": "https://api.telegram.org", "auth_method": "path", "auth_key_name": "bot" } },
    ).await.unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "_id": &f.service.endpoint_id },
            doc! { "$set": { "url": "https://api.telegram.org" } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "auth_method": "path", "auth_key_name": "bot" } },
        )
        .await
        .unwrap();
    let run = begin(&f, false);
    let NodeOutboundMessage::Text(frame) =
        tokio::time::timeout(Duration::from_secs(3), f.outbound.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!();
    };
    assert!(!frame.contains("fixture-token"));
    let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(frame["path"], "getMe");
    assert!(frame["query"].is_null());
    respond(
        &f,
        &frame,
        200,
        br#"{"ok":true,"result":{"id":123,"is_bot":true}}"#,
        vec![],
    );
    assert_eq!(
        run.await.unwrap().unwrap().outcome,
        ValidationOutcome::Authenticated
    );
}
