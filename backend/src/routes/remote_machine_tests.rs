//! Combined production router/index integration on an isolated replica-set DB.
use super::build_router_with_state;
use crate::{
    crypto::jwt,
    models::user::{COLLECTION_NAME as USERS, User, UserType},
    test_utils,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    key: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let response = app
        .clone()
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn remote_machine_full_router_indexes_unconfigured_admission_replay_and_owner_boundaries() {
    let db = test_utils::connect_transaction_test_database("remote_router").await;
    use futures::TryStreamExt;
    let jobs =
        db.collection::<mongodb::bson::Document>(crate::models::machine_job::COLLECTION_NAME);
    jobs.create_index(
        mongodb::IndexModel::builder()
            .keys(mongodb::bson::doc! {"expires_at":1})
            .options(
                mongodb::options::IndexOptions::builder()
                    .expire_after(std::time::Duration::from_secs(3600))
                    .build(),
            )
            .build(),
    )
    .await
    .unwrap();
    crate::db::ensure_indexes(&db).await.unwrap();
    crate::db::ensure_indexes(&db).await.unwrap();
    // Main seeds platform roles before serving admin routes. Exercise that
    // real prerequisite so an ordinary owner receives authorization denial,
    // rather than an uninitialized-platform error from the isolated fixture.
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let indexes: Vec<mongodb::IndexModel> = jobs
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        !indexes
            .iter()
            .any(|i| i.keys == mongodb::bson::doc! {"expires_at":1}
                && i.options.as_ref().is_some_and(|o| o.expire_after.is_some()))
    );
    assert!(indexes.iter().any(|i| {
        i.keys == mongodb::bson::doc! {"retain_until":1}
            && i.options
                .as_ref()
                .is_some_and(|o| o.expire_after == Some(std::time::Duration::ZERO))
    }));
    let user = Uuid::new_v4();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&user.to_string(), UserType::Person))
        .await
        .unwrap();
    super::remote_machine_flag_tests::set_enabled(&db, &user.to_string(), true).await;
    let state = test_utils::test_app_state(db.clone());
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &user,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state.clone());
    let (status, entitlement) = request(
        &app,
        "GET",
        "/api/v1/remote-machine-entitlement",
        &token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{entitlement}");
    let body = json!({"free_slot":true,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true});
    let (status, created) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        body.clone(),
        Some("local-router-create"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "unconfigured platform must not accept: {created}"
    );
    let canonical = json!({"free_slot":true,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":null,"accept_account_balance":false});
    let mut input = crate::services::remote_machine_service::CreateMachineInput::fixed(
        &user.to_string(),
        true,
        "unconfigured",
    );
    input.idempotency_key = "local-router-create".into();
    input.request_body = canonical;
    let created = crate::services::remote_machine_service::create_machine(&db, input)
        .await
        .unwrap();
    let (status, replayed) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        body,
        Some("local-router-create"),
    )
    .await;
    assert!(status.is_success(), "{status}: {replayed}");
    assert_eq!(
        created.machine.node_id,
        replayed["machine"]["node_id"].as_str().unwrap()
    );
    assert_eq!(replayed["replayed"], true);
    let node = &created.machine.node_id;
    for suffix in [
        "",
        "/usage",
        "/jobs",
        "/grants",
        "/operations",
        "/billing/runs",
    ] {
        let (status, _) = request(
            &app,
            "GET",
            &format!("/api/v1/remote-machines/{node}{suffix}"),
            &token,
            Value::Null,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "owner Machine read must succeed");
    }
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &user,
        "account:read",
        "test-reader",
        60,
        None,
    )
    .unwrap();
    for path in [
        format!("/api/v1/remote-machines/{node}"),
        format!("/api/v1/remote-machines/{node}/jobs"),
        format!("/api/v1/remote-machines/{node}/grants"),
        "/api/v1/machine-confirmations".into(),
        format!("/api/v1/users/me/consents/{}/machine-grant", Uuid::new_v4()),
    ] {
        let (status, _) = request(&app, "GET", &path, &delegated, Value::Null, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "delegated Machine read must be denied"
        );
    }
    let key = crate::services::key_service::create_api_key(
        &db,
        &user.to_string(),
        "Machine execution key",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(true),
        Some(true),
        Some(false),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap();
    let (status, value) = request(
        &app,
        "GET",
        "/api/v1/remote-machines",
        &key.full_key,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    let (status, value) = request(
        &app,
        "GET",
        "/api/v1/admin/billing/runtime",
        &token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    // An exhausted delete remains operable by its human owner even after new
    // Machine admission is disabled. Delegated/API keys never renew it.
    use mongodb::bson::{Document, doc};
    let deleted = crate::services::remote_machine_service::request_action(
        &db,
        crate::services::remote_machine_service::LifecycleRequest {
            owner_id: user.to_string(),
            machine_id: node.clone(),
            action: crate::models::remote_machine_operation::LifecycleAction::Delete,
            expected_revision: Some(created.machine.revision),
            idempotency_key: "router-delete".into(),
            request_body: json!({"action":"delete","revision":created.machine.revision,
                "policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data",
                "acknowledge_shared_environment":true,"acknowledge_data_loss":true}),
            now: None,
        },
    )
    .await
    .unwrap();
    db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&deleted.operation.id},
            doc! {"$set":{"retry_count":15_639_i64}},
        )
        .await
        .unwrap();
    super::remote_machine_flag_tests::set_enabled(&db, &user.to_string(), false).await;
    let retry_path = format!(
        "/api/v1/remote-machine-operations/{}/retry-delete",
        deleted.operation.id
    );
    let retry_body =
        json!({"expected_revision":deleted.machine.revision,"expected_retry_count":15_639});
    for credential in [&delegated, &key.full_key] {
        let (status, _) = request(
            &app,
            "POST",
            &retry_path,
            credential,
            retry_body.clone(),
            Some("manual-round"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, _) = request(&app, "POST", &retry_path, &token, retry_body.clone(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for _ in 0..2 {
        let (status, response) = request(
            &app,
            "POST",
            &retry_path,
            &token,
            retry_body.clone(),
            Some("manual-round"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{response}");
        assert_eq!(response["operation"]["id"], deleted.operation.id);
        assert_eq!(response["operation"]["retry_count"], 15_639);
        assert_eq!(response["operation"]["delete_retry_attempts"], 0);
        assert_eq!(response["operation"]["delete_retry_stopped"], false);
        assert!(
            response["operation"].get("delete_retry").is_none(),
            "private retry records are never serialized"
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mounted_machine_grants_pages_equal_timestamp_history_and_denies_delegated() {
    use crate::models::machine_external_grant::{COLLECTION_NAME as GRANTS, MachineExternalGrant};
    use mongodb::bson::{self, doc};
    let db = test_utils::connect_transaction_test_database("mounted_grant_pages").await;
    crate::services::machine_external_grant_service::create_indexes(&db)
        .await
        .unwrap();
    let owner = Uuid::new_v4();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&owner.to_string(), UserType::Person))
        .await
        .unwrap();
    let node:crate::models::node::Node = bson::from_document(doc!{
        "_id":Uuid::new_v4().to_string(),"user_id":owner.to_string(),"name":"Route page fixture","status":"offline",
        "auth_token_hash":"00".repeat(32),"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).unwrap();
    db.collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .insert_one(&node)
        .await
        .unwrap();
    let state = test_utils::test_app_state(db.clone());
    let bearer = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "account:read",
        "test-reader",
        60,
        None,
    )
    .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);
    let at = bson::DateTime::now();
    // Historical revoked contracts need no live source; the API must return
    // their bounded metadata without inventing currently valid source states.
    let mut rows = vec![];
    for _ in 0..105 {
        let mut assignments = doc! {};
        assignments.insert(&node.id, doc!{"capabilities":{"shell":true,"files":true},"mode":"shared_legacy","revision":1_i64,"legacy":false,"saved_login_ids":[]});
        let mut restrictions = doc! {};
        restrictions.insert(
            &node.id,
            doc! {"file_roots":["/workspace"],"service_ids":[]},
        );
        let row:MachineExternalGrant=bson::from_document(doc!{
            "_id":Uuid::new_v4().to_string(),"user_id":owner.to_string(),
            "binding":{"kind":"api_key","api_key_id":Uuid::new_v4().to_string(),"api_key_state_version_at_approval":1_i64},
            "policy":{"version":2_i32,"revision":1_i64,"assignments":assignments},"restrictions":restrictions,
            "revision":1_i64,"admission_fence":0_i64,"user_security_revision":0_i64,"client_security_revision":bson::Bson::Null,"revoked_reason":"fixture","status":"revoked","created_at":at,"updated_at":at,
            "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(1))
        }).unwrap();
        rows.push(row);
    }
    db.collection::<MachineExternalGrant>(GRANTS)
        .insert_many(rows)
        .await
        .unwrap();
    let path = format!("/api/v1/remote-machines/{}/grants?limit=100", node.id);
    let (status, first) = request(&app, "GET", &path, &bearer, Value::Null, None).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["external_grants"].as_array().unwrap().len(), 100);
    let cursor = first["next_cursor"]
        .as_str()
        .expect("mounted paginated handler");
    let cursor = url::form_urlencoded::byte_serialize(cursor.as_bytes()).collect::<String>();
    let (status, second) = request(
        &app,
        "GET",
        &format!("{path}&cursor={cursor}"),
        &bearer,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["external_grants"].as_array().unwrap().len(), 5);
    assert!(second["next_cursor"].is_null());
    let ids: std::collections::BTreeSet<_> = first["external_grants"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["external_grants"].as_array().unwrap())
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 105);
    let (status, _) = request(
        &app,
        "GET",
        &format!("{path}&cursor={cursor}"),
        &delegated,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_create_http_validates_and_preserves_choices_through_idempotent_replay() {
    use crate::services::{
        remote_machine_manager::PlatformReadiness, remote_machine_service as lifecycle,
    };
    use mongodb::bson::doc;
    let db = test_utils::connect_transaction_test_database("hosted_create_http").await;
    lifecycle::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&owner.to_string(), UserType::Person))
        .await
        .unwrap();
    super::remote_machine_flag_tests::set_enabled(&db, &owner.to_string(), true).await;
    let mut state = test_utils::test_app_state(db.clone());
    state.local_machine_platform = Some(std::sync::Arc::new(test_utils::LocalMachinePlatform {
        profile: lifecycle::fixed_profile(format!("registry.test/full@sha256:{}", "a".repeat(64))),
        readiness: PlatformReadiness {
            configuration_ready: true,
            manager_started: true,
            namespace: "nyxid-machine",
            blockers: vec![],
            physical_acceptance: "native_state_only_no_physical_guarantee",
            lifecycle_basis: "agent_sandbox_native_observation",
        },
    }));
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);
    let body = json!({"name":"hosted-workstation","capabilities":["browser","computer","files"],"specialist_id":null,
        "free_slot":true,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true});
    for invalid in [
        json!({"capabilities":["computer"]}),
        json!({"name":"invalid name"}),
        json!({"owner_id":Uuid::new_v4().to_string()}),
        json!({"profile_version":"test-foreign-profile"}),
        json!({"profile_version":null}),
        json!({"cpu_millis":4000}),
        json!({"memory_mib":8192}),
        json!({"workspace_gib":50}),
    ] {
        let mut candidate = body.clone();
        for (k, v) in invalid.as_object().unwrap() {
            candidate[k] = v.clone();
        }
        let expected = if invalid.get("owner_id").is_some()
            || invalid.get("cpu_millis").is_some()
            || invalid.get("memory_mib").is_some()
            || invalid.get("workspace_gib").is_some()
            || invalid.get("profile_version").is_some_and(Value::is_null)
        {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::BAD_REQUEST
        };
        let (status, value) = request(
            &app,
            "POST",
            "/api/v1/remote-machines",
            &token,
            candidate,
            Some("hosted-create-request"),
        )
        .await;
        assert_eq!(status, expected, "{invalid}: {value}");
    }
    let (status, created) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        body.clone(),
        Some("hosted-create-request"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{created}");
    assert_eq!(created["machine"]["name"], "hosted-workstation");
    assert_eq!(
        created["machine"]["capabilities"],
        json!(["browser", "computer", "files"])
    );
    let id = created["machine"]["node_id"].as_str().unwrap();
    assert_ne!(
        created["machine"]["sandbox_name"],
        created["machine"]["name"]
    );
    let mut reordered = body.clone();
    reordered["capabilities"] = json!(["files", "computer", "browser"]);
    reordered["profile_version"] = json!("remote-machine");
    let (status, replay) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        reordered,
        Some("hosted-create-request"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{replay}");
    assert_eq!(replay["machine"]["node_id"], id);
    assert_eq!(replay["replayed"], true);
    let mut changed = body;
    changed["capabilities"] = json!(["files"]);
    let (status, _) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        changed,
        Some("hosted-create-request"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, read) = request(
        &app,
        "GET",
        &format!("/api/v1/remote-machines/{id}"),
        &token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["machine"]["name"], "hosted-workstation");
    assert_eq!(
        db.collection::<mongodb::bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn obsolete_create_contract_rejects_even_with_saved_request_and_keeps_telemetry() {
    use crate::models::remote_machine::ManagedMachine;
    use crate::services::remote_machine_service as machine;
    use mongodb::bson::doc;
    use sha2::{Digest, Sha256};
    let db = test_utils::connect_transaction_test_database("remote_historical_replay").await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&user.to_string(), UserType::Person))
        .await
        .unwrap();
    super::remote_machine_flag_tests::set_enabled(&db, &user.to_string(), true).await;
    let state = test_utils::test_app_state(db.clone());
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &user,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);
    let mut input =
        machine::CreateMachineInput::fixed(&user.to_string(), true, "old-image@sha256:abc");
    input.idempotency_key = "historically-accepted".into();
    // Materialize the old durable shape directly; do not add a legacy allocation
    // path merely to manufacture historical records for this regression.
    let created = machine::create_machine(&db, input.clone()).await.unwrap();
    let legacy_body = json!({"free_slot":true,"policy_version":"remote-machine-mvp-v1","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":null,"accept_account_balance":false});
    // Production stores the digest of recursively sorted JSON. Feature
    // unification enables serde_json/preserve_order in this test binary, so
    // hashing the json! insertion order would fabricate an invalid old row.
    let mut canonical_legacy_body = legacy_body.clone();
    canonical_legacy_body.sort_all_objects();
    let legacy_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical_legacy_body).unwrap(),
    ));
    db.collection::<mongodb::bson::Document>(
        crate::models::remote_machine_operation::COLLECTION_NAME,
    )
    .update_one(
        doc! {"_id":&created.operation.id},
        doc! {"$set":{"request_digest":&legacy_digest}},
    )
    .await
    .unwrap();
    db.collection::<mongodb::bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(doc!{"_id":&created.machine.node_id}, doc!{"$set":{"profile.profile_version":"remote-shell-files-v1","profile.ephemeral_gib":2_i32,"policy_version":"remote-machine-mvp-v1"}}).await.unwrap();
    let (status, replay) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        legacy_body.clone(),
        Some("historically-accepted"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{replay}");
    let mut changed = legacy_body.clone();
    changed["policy_version"] = json!(crate::models::remote_machine::POLICY_VERSION);
    let (status, _) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        changed,
        Some("historically-accepted"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = request(
        &app,
        "POST",
        "/api/v1/remote-machines",
        &token,
        legacy_body.clone(),
        Some("new-obsolete-request"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    input.request_body = legacy_body;
    input.profile.profile_version = "remote-shell-files-v1".into();
    input.profile.ephemeral_gib = Some(2);
    assert!(machine::create_machine(&db, input).await.is_err());
    assert_eq!(
        db.collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let stored = db
        .collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME)
        .find_one(doc! {"_id":&created.machine.node_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.profile.ephemeral_gib, Some(2));
    let operation = db
        .collection::<mongodb::bson::Document>(
            crate::models::remote_machine_operation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&created.operation.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.get_str("request_digest").unwrap(), legacy_digest);
    // A retained, inactive runtime sample remains visibly stale, with sourced
    // partial usage but no invented allocation in the mounted HTTP response.
    let sampled_at = chrono::Utc::now();
    let usage = nyxid_machine::managed::Usage {
        runtime_id: Uuid::new_v4().to_string(),
        managed_runtime: nyxid_machine::managed::Binding {
            node_id: stored.node_id.clone(),
            sandbox_uid: Uuid::new_v4().to_string(),
            pod_uid: Uuid::new_v4().to_string(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: stored.writer_fence.clone(),
            operation_id: created.operation.id.clone(),
            delivery_id: Uuid::new_v4().to_string(),
        },
        sampled_at_ms: sampled_at.timestamp_millis(),
        workspace: nyxid_machine::managed::UsageVolume {
            capacity_bytes: Some(20 * 1024 * 1024 * 1024),
            used_bytes: 12,
            available_bytes: Some(1024),
            measurement: "statvfs".into(),
            complete: true,
        },
        temporary: nyxid_machine::managed::UsageVolume {
            capacity_bytes: None,
            used_bytes: 34,
            available_bytes: None,
            measurement: "node_paths_lower_bound".into(),
            complete: false,
        },
        disk_full: false,
    };
    db.collection::<mongodb::bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(doc!{"_id":&stored.node_id}, doc!{"$set":{"usage":mongodb::bson::to_bson(&usage).unwrap(),"usage_received_at":mongodb::bson::DateTime::from_chrono(sampled_at)}}).await.unwrap();
    let (status, usage) = request(
        &app,
        "GET",
        &format!("/api/v1/remote-machines/{}/usage", stored.node_id),
        &token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["available"], true);
    assert_eq!(usage["stale"], true);
    assert_eq!(usage["temporary"]["capacity_bytes"], Value::Null);
    assert_eq!(usage["temporary"]["available_bytes"], Value::Null);
    assert_eq!(usage["temporary"]["used_bytes"], 34);
    assert_eq!(usage["temporary"]["complete"], false);
    assert_eq!(usage["temporary"]["measurement"], "node_paths_lower_bound");
    assert!(usage["sampled_at"].is_string());
    assert_eq!(
        usage["workspace"]["capacity_bytes"],
        20_u64 * 1024 * 1024 * 1024
    );
    db.drop().await.unwrap();
}
