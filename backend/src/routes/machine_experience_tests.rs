//! Production-route coverage for retained Machine presentation/history and
//! write-only Saved login credentials. Every fixture requires a replica set.
use super::build_router_with_state;
use crate::{crypto::jwt, models::user::UserType, test_utils};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use mongodb::bson::{self, Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn fixture(label: &str) -> (crate::AppState, Router, String, String) {
    let db = test_utils::connect_transaction_test_database(label).await;
    let owner = Uuid::new_v4();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_utils::test_user(&owner.to_string(), UserType::Person))
        .await
        .unwrap();
    let state = test_utils::test_app_state(db);
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
    (
        state.clone(),
        private.with_state(state),
        owner.to_string(),
        token,
    )
}

async fn request(
    app: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
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

async fn retained_machine(state: &crate::AppState, owner: &str, status: &str) -> String {
    let id = Uuid::new_v4().to_string();
    let machine =
        crate::services::remote_machine_service::CreateMachineInput::fixed(owner, true, "fixture")
            .preview(&id, chrono::Utc::now());
    let mut source = bson::to_document(&machine).unwrap();
    source.insert("resource_state", status);
    state
        .db
        .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .insert_one(source)
        .await
        .unwrap();
    id
}

async fn reported_node(state: &crate::AppState, owner: &str, id: &str) {
    let profile = nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        shell: true,
        files: true,
        computer: true,
        browser: Some(true),
        computer_ready: true,
        os: "linux".into(),
        arch: "x86_64".into(),
        roots: vec!["/workspace".into()],
        runtime_id: "runtime-private-sentinel".into(),
        separated: Some(nyxid_machine::context::Support {
            available: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    state.db.collection::<Document>(crate::models::node::COLLECTION_NAME).insert_one(doc! {
        "_id":id,"user_id":owner,"name":"machine","is_active":false,"status":"offline",
        "auth_token_hash":"auth-private-sentinel","signing_secret_hash":"signing-private-sentinel",
        "metadata":{"agent_version":"0.41.1","ip_address":"ip-private-sentinel"},
        "machine":bson::to_bson(&profile).unwrap(),"machine_confirm":"all",
        "allow_single_user_saved_logins":false,
        "last_heartbeat_at":bson::DateTime::now(),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
}

#[tokio::test]
async fn remote_machine_presentation_retains_safe_reports_without_online_or_generation_claims() {
    let (state, app, owner, token) = fixture("machine_presentation").await;
    let id = retained_machine(&state, &owner, "paused").await;
    reported_node(&state, &owner, &id).await;
    let (status, value) = request(
        &app,
        &token,
        "GET",
        &format!("/api/v1/remote-machines/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let p = &value["machine"]["presentation"];
    assert_eq!(p["agent_version"], "0.41.1");
    assert_eq!(p["roots"], json!(["/workspace"]));
    assert_eq!(p["owner_confirmation"], "all");
    assert_eq!(p["connected"], false);
    assert_eq!(p["desktop_ready"], false);
    assert_eq!(p["historical"], true);
    assert_eq!(p["configuration_current"], false);
    assert!(!p.to_string().contains("private-sentinel"));
    let (_, listed) = request(&app, &token, "GET", "/api/v1/remote-machines", Value::Null).await;
    assert_eq!(listed["machines"][0]["presentation"], *p);
    assert_eq!(
        request(
            &app,
            &token,
            "GET",
            &format!("/api/v1/nodes/{id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_activity_retained_remote_history_survives_missing_node_without_widening_acl() {
    let (state, app, owner, token) = fixture("machine_history").await;
    let id = retained_machine(&state, &owner, "paused").await;
    reported_node(&state, &owner, &id).await;
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "account:read",
        "test-reader",
        60,
        None,
    )
    .unwrap();
    assert_eq!(
        request(
            &app,
            &delegated,
            "GET",
            &format!("/api/v1/machines/{id}/activity"),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let other = Uuid::new_v4();
    state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_utils::test_user(&other.to_string(), UserType::Person))
        .await
        .unwrap();
    let other_token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &other,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        request(
            &app,
            &other_token,
            "GET",
            &format!("/api/v1/machines/{id}/activity"),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            &other_token,
            "GET",
            &format!("/api/v1/remote-machines/{id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    state.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":&owner,"event_type":"machine_operation",
        "created_at":bson::DateTime::now(),"event_data":{"node_id":&id,"operation":"exec","outcome":"completed",
        "command":"command-private-sentinel","stdout":"output-private-sentinel"}
    }).await.unwrap();
    for lifecycle in ["paused", "deleting", "deleted"] {
        state
            .db
            .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_one(doc! {"_id":&id}, doc! {"$set":{"resource_state":lifecycle}})
            .await
            .unwrap();
        let (status, page) = request(
            &app,
            &token,
            "GET",
            &format!("/api/v1/machines/{id}/activity"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{lifecycle}: {page}");
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert!(!page.to_string().contains("private-sentinel"));
    }
    state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .delete_one(doc! {"_id":&id})
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            &token,
            "GET",
            &format!("/api/v1/machines/{id}/activity"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    state
        .db
        .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"owner_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(
        !request(
            &app,
            &token,
            "GET",
            &format!("/api/v1/machines/{id}/activity"),
            Value::Null
        )
        .await
        .0
        .is_success()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remote_machine_presentation_unknown_binding_and_current_generation_are_distinct() {
    let (state, app, owner, token) = fixture("machine_report_current").await;
    let id = retained_machine(&state, &owner, "ready").await;
    let path = format!("/api/v1/remote-machines/{id}");
    assert!(
        request(&app, &token, "GET", &path, Value::Null).await.1["machine"]["presentation"]
            .is_null()
    );
    reported_node(&state, &owner, &id).await;
    let machines = state
        .db
        .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME);
    let nodes = state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME);
    nodes.update_one(doc! {"_id":&id},doc! {"$set":{"machine":bson::to_bson(&nyxid_machine::MachineProfile::default()).unwrap()}}).await.unwrap();
    assert!(
        request(&app, &token, "GET", &path, Value::Null).await.1["machine"]["presentation"]
            .is_null(),
        "a provisioned placeholder is not a reported profile"
    );
    let mut machine: crate::models::remote_machine::ManagedMachine =
        bson::from_document(machines.find_one(doc! {"_id":&id}).await.unwrap().unwrap()).unwrap();
    machine.runtime_generation = 1;
    machine.writer_admitted = true;
    let mut profile = nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: Uuid::new_v4().to_string(),
        shell: true,
        files: true,
        os: "linux".into(),
        roots: vec!["/workspace".into()],
        separation_choice: Some(machine.configuration.separation_choice),
        commands_isolated: Some(true),
        authority_versions: vec![2],
        managed_configuration: Some(nyxid_machine::managed::ConfigurationReport {
            configuration: machine.configuration.clone(),
            digest: machine.configuration.digest().unwrap(),
            runtime_generation: 1,
        }),
        ..Default::default()
    };
    machine.configuration_evidence = Some(
        crate::models::remote_machine::MachineConfigurationEvidence {
            configuration_revision: machine.configuration.revision,
            configuration_digest: machine.configuration.digest().unwrap(),
            runtime_generation: 1,
            credential_epoch: machine.credential_epoch,
            runtime_id: profile.runtime_id.clone(),
            connection_id: "connection-private-sentinel".into(),
            observed_at: chrono::Utc::now(),
        },
    );
    machines
        .replace_one(doc! {"_id":&id}, bson::to_document(&machine).unwrap())
        .await
        .unwrap();
    nodes.update_one(doc! {"_id":&id},doc! {"$unset":{"machine_confirm":"","allow_single_user_saved_logins":""},
        "$set":{"is_active":true,"status":"online","machine":bson::to_bson(&profile).unwrap(),
        "connection_owner":{"connection_id":"connection-private-sentinel","capabilities_resolved":true,
        "instance_name":"replica","generation_id":"replica-generation","internal_base_url":"http://private-sentinel",
        "claimed_at":bson::DateTime::now(),"renewed_at":bson::DateTime::now(),
        "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(1))}}}).await.unwrap();
    let (status, current) = request(&app, &token, "GET", &path, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    let p = &current["machine"]["presentation"];
    assert_eq!(p["configuration_current"], true);
    assert_eq!(p["connected"], true);
    assert_eq!(
        p["desktop_ready"], false,
        "a current report does not establish a display"
    );
    assert!(p["owner_confirmation"].is_null());
    assert!(p["allow_single_user_saved_logins"].is_null());
    assert!(p["separation_support"].is_null());
    assert!(!p.to_string().contains("private-sentinel"));
    // A ready display does not require a positive separation summary. Keep
    // those independent while still proving the current configuration.
    machine.capabilities = vec![
        "browser".into(),
        "computer".into(),
        "files".into(),
        "shell".into(),
    ];
    machine.configuration.capabilities = machine.capabilities.clone();
    machine.configuration.revision += 1;
    let digest = machine.configuration.digest().unwrap();
    profile.computer = true;
    profile.browser = Some(true);
    profile.computer_ready = true;
    profile.managed_configuration = Some(nyxid_machine::managed::ConfigurationReport {
        configuration: machine.configuration.clone(),
        digest: digest.clone(),
        runtime_generation: 1,
    });
    let evidence = machine.configuration_evidence.as_mut().unwrap();
    evidence.configuration_revision = machine.configuration.revision;
    evidence.configuration_digest = digest;
    machines
        .replace_one(doc! {"_id":&id}, bson::to_document(&machine).unwrap())
        .await
        .unwrap();
    nodes
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"machine":bson::to_bson(&profile).unwrap()}},
        )
        .await
        .unwrap();
    let display = request(&app, &token, "GET", &path, Value::Null).await.1;
    assert_eq!(display["machine"]["presentation"]["desktop_ready"], true);
    assert!(display["machine"]["presentation"]["separation_support"].is_null());
    machines
        .update_one(doc! {"_id":&id}, doc! {"$inc":{"runtime_generation":1}})
        .await
        .unwrap();
    let stale = request(&app, &token, "GET", &path, Value::Null).await.1;
    assert_eq!(
        stale["machine"]["presentation"]["configuration_current"],
        false
    );
    assert_eq!(stale["machine"]["presentation"]["historical"], true);
    nodes
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"user_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(
        request(&app, &token, "GET", &path, Value::Null).await.1["machine"]["presentation"]
            .is_null()
    );
    nodes
        .update_one(doc! {"_id":&id}, doc! {"$set":{"user_id":&owner}})
        .await
        .unwrap();
    machines
        .update_one(doc! {"_id":&id}, doc! {"$set":{"resource_state":"deleted"}})
        .await
        .unwrap();
    assert!(
        request(&app, &token, "GET", &path, Value::Null).await.1["machine"]["presentation"]
            .is_null(),
        "completed deletion does not expose settings"
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_activity_rejects_inactive_self_hosted_unbound_and_erased_remote_history() {
    let (state, app, owner, token) = fixture("machine_history_acl").await;
    let id = retained_machine(&state, &owner, "paused").await;
    reported_node(&state, &owner, &id).await;
    let path = format!("/api/v1/machines/{id}/activity");
    let nodes = state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME);
    nodes
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"user_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&app, &token, "GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    nodes
        .update_one(doc! {"_id":&id}, doc! {"$set":{"user_id":&owner}})
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(crate::services::erasure_admission::COLLECTION)
        .insert_one(doc! {"_id":&owner,"erasure_accepted":true})
        .await
        .unwrap();
    assert!(
        !request(&app, &token, "GET", &path, Value::Null)
            .await
            .0
            .is_success()
    );
    state
        .db
        .collection::<Document>(crate::services::erasure_admission::COLLECTION)
        .delete_one(doc! {"_id":&owner})
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .delete_one(doc! {"_id":&id})
        .await
        .unwrap();
    assert_eq!(
        request(&app, &token, "GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    let missing_id = retained_machine(&state, &owner, "deleted").await;
    assert_eq!(
        request(
            &app,
            &token,
            "GET",
            &format!("/api/v1/machines/{missing_id}/activity"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "a retained machine without its Node or any retained history is unavailable"
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_activity_and_saved_login_keep_live_org_admin_acl() {
    use crate::models::{
        org_membership::{OrgMembership, OrgRole},
        user::User,
    };
    let (state, app, actor, token) = fixture("machine_management_org_acl").await;
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    let membership = test_utils::test_membership(&org, &actor, OrgRole::Admin, None);
    let memberships = state
        .db
        .collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME);
    memberships.insert_one(&membership).await.unwrap();
    let node = retained_machine(&state, &org, "paused").await;
    reported_node(&state, &org, &node).await;
    let history = format!("/api/v1/machines/{node}/activity");
    assert_eq!(
        request(&app, &token, "GET", &history, Value::Null).await.0,
        StatusCode::OK
    );
    let (status, created) = request(
        &app,
        &token,
        "POST",
        &format!("/api/v1/saved-logins?owner_id={org}"),
        login_input(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let path = format!("/api/v1/saved-logins/{}", created["id"].as_str().unwrap());
    assert_eq!(
        request(&app, &token, "PATCH", &path, json!({"label":"denied"}))
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    memberships
        .update_one(doc! {"_id":&membership.id}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert_eq!(
        request(&app, &token, "GET", &history, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &token, "PUT", &path, login_input()).await.0,
        StatusCode::FORBIDDEN
    );
    memberships
        .update_one(
            doc! {"_id":&membership.id},
            doc! {"$set":{"role":"admin","revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&app, &token, "PUT", &path, login_input()).await.0,
        StatusCode::FORBIDDEN
    );
    state.db.drop().await.unwrap();
}

fn login_input() -> Value {
    json!({"label":"Before","allowed_origins":["https://example.com"],"username":"user-private-sentinel",
        "password":"password-private-sentinel","totp_secret":"GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ","confirm_each_sign_in":false})
}

#[tokio::test]
async fn saved_login_metadata_patch_is_removed_and_full_replace_remains_available() {
    let (state, app, _owner, token) = fixture("saved_login_metadata_removed").await;
    let (status, created) =
        request(&app, &token, "POST", "/api/v1/saved-logins", login_input()).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap();
    let collection = state
        .db
        .collection::<Document>(crate::models::saved_login::COLLECTION_NAME);
    let before = collection.find_one(doc! {"_id":id}).await.unwrap().unwrap();
    assert!(!before.contains_key("metadata_revision"));
    assert_eq!(
        request(
            &app,
            &token,
            "PATCH",
            &format!("/api/v1/saved-logins/{id}"),
            json!({"label":"After"}),
        )
        .await
        .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        collection.find_one(doc! {"_id":id}).await.unwrap().unwrap(),
        before
    );
    let (status, replaced) = request(
        &app,
        &token,
        "PUT",
        &format!("/api/v1/saved-logins/{id}"),
        login_input(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert!(replaced.get("metadata_revision").is_none());
    assert_eq!(replaced["label"], "Before");
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn remote_machine_presentation_list_joins_once_and_only_selects_safe_node_inputs() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let recorded = Arc::new(Mutex::new(Vec::<Document>::new()));
    let sink = recorded.clone();
    let db = test_utils::connect_transaction_test_database_with_command_handler(
        "machine_presentation_batch",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                sink.lock().unwrap().push(event.command);
            }
        }),
    )
    .await;
    let owner = Uuid::new_v4().to_string();
    let state = test_utils::test_app_state(db);
    let mut machines = Vec::new();
    for _ in 0..4 {
        let id = retained_machine(&state, &owner, "paused").await;
        reported_node(&state, &owner, &id).await;
        machines.push(
            crate::services::remote_machine_service::managed_machine_for_node(&state.db, &id)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    recorded.lock().unwrap().clear();
    let views = crate::services::remote_machine_configuration::safe_views(&state.db, &machines)
        .await
        .unwrap();
    assert_eq!(views.len(), 4);
    assert!(
        views
            .iter()
            .all(|view| view.presentation.as_ref().unwrap().historical)
    );
    let commands = recorded.lock().unwrap().clone();
    let finds: Vec<_> = commands
        .iter()
        .filter(|command| command.get_str("find") == Ok(crate::models::node::COLLECTION_NAME))
        .collect();
    assert_eq!(finds.len(), 1, "one Node read for the entire page");
    let projection = finds[0].get_document("projection").unwrap();
    assert_eq!(projection.get_i32("metadata.agent_version"), Ok(1));
    assert!(!projection.contains_key("auth_token_hash"));
    assert!(!projection.contains_key("signing_secret_hash"));
    assert!(!projection.contains_key("connection_owner"));
    assert!(
        !serde_json::to_string(&views)
            .unwrap()
            .contains("private-sentinel")
    );
    state.db.drop().await.unwrap();
}
