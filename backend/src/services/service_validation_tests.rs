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

#[tokio::test]
async fn validation_db_joins_attempt_reuses_freshness_and_never_changes_status() {
    let Some(mut f) = fixture("validation_join", true).await else {
        return;
    };
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
    assert!(matches!(
        validate(&f.state, f.caller.clone(), &f.service.id, true).await,
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
    let audit = f
        .state
        .db
        .collection::<Document>("audit_log")
        .find_one(doc! { "event_type": "service_validation_checked" })
        .await
        .unwrap()
        .unwrap();
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
    // A routing change invalidates evidence and receives its own digest cooldown.
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
    assert!(matches!(
        first.await.unwrap(),
        Err(AppError::ServiceValidationUnavailable)
    ));
    let record = f
        .state
        .db
        .collection::<ServiceValidationRecord>(COLLECTION_NAME)
        .find_one(doc! { "user_service_id": &f.service.id })
        .await
        .unwrap()
        .unwrap();
    assert!(!record.completed);
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
    let CallerContext::Human { session } = &f.caller.context else {
        unreachable!()
    };
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
