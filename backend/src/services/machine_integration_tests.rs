//! Machine authority tests exercise the real MongoDB, chat grants and signed
//! dispatch path. The node transport is a deterministic in-process peer here;
//! the production Linux node and browser are exercised by the container test.
use super::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::{Fixture, fixture, orchestrator_fixture},
    machine_service as machines, node_service, saved_login_service as logins,
};
use crate::{
    errors::AppError,
    handlers::machine_tools::call,
    models::{
        node::{Node, NodeStatus},
        user::UserType,
    },
    test_utils::{test_membership, test_user},
};
use mongodb::bson::{self, doc};
use nyxid_machine::{Confirmation, MachineProfile, Operation};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) async fn node(f: &Fixture, owner: &str) -> Node {
    let (_, token, _) =
        node_service::create_registration_token(&f.state.db, owner, "test-machine", 100, 300)
            .await
            .unwrap();
    let (mut node, _, _) =
        node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .unwrap();
    node.status = NodeStatus::Online;
    node.machine = Some(MachineProfile {
        version: 1,
        runtime_id: Uuid::new_v4().to_string(),
        shell: true,
        files: true,
        computer: true,
        computer_tools: vec!["get_window_state".into(), "click".into()],
        computer_ready: true,
        saved_login_ready: true,
        browser_tools: true,
        ..Default::default()
    });
    save_node(f, &node).await;
    node
}
async fn save_node(f: &Fixture, node: &Node) {
    f.state
        .db
        .collection::<Node>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{
            "status":node.status.as_str(),"machine":bson::to_bson(&node.machine).unwrap(),
            "machine_confirm":bson::to_bson(&node.machine_confirm).unwrap(),
            "allow_single_user_saved_logins":node.allow_single_user_saved_logins}},
        )
        .await
        .unwrap();
}

pub(crate) async fn peer(
    f: &Fixture,
    node: &Node,
    response: Value,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Receiver<nyxid_machine::Request>,
) {
    use super::node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage};
    let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
    let manager = f.state.node_ws_manager.clone();
    crate::test_utils::register_test_node_connection(&f.state, &node.id, sender).await;
    let caps: NodeCapabilitiesMsg =
        serde_json::from_value(json!({"machine":node.machine})).unwrap();
    manager.record_capabilities(&node.id, &caps);
    let key =
        node_service::get_node_signing_secret(&f.state.db, &f.state.encryption_keys, &node.id)
            .await
            .unwrap();
    let id = node.id.clone();
    let (seen, requests) = tokio::sync::mpsc::channel(32);
    let task = tokio::spawn(async move {
        let mut replay = nyxid_machine::signing::ReplayGuard::default();
        while let Some(message) = receiver.recv().await {
            let NodeOutboundMessage::Text(text) = message else {
                continue;
            };
            let request: nyxid_machine::Request = serde_json::from_str(&text).unwrap();
            replay
                .verify(&request, &id, &key, chrono::Utc::now().timestamp())
                .unwrap();
            manager.deliver_machine_result(
                &id,
                nyxid_machine::Response {
                    request_id: request.request_id.clone(),
                    result: response.clone(),
                },
            );
            let _ = seen.try_send(request);
        }
    });
    (task, requests)
}

#[tokio::test]
async fn machine_authority_owner_guest_org_membership_offline_and_capabilities() {
    let f = orchestrator_fixture("machine_authority_matrix").await;
    let mut own = node(&f, &f.owner).await;
    let other = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&other, UserType::Person))
        .await
        .unwrap();
    let foreign = node(&f, &other).await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let shared = node(&f, &org).await;
    let members = f
        .state
        .db
        .collection(crate::models::org_membership::COLLECTION_NAME);
    members
        .insert_one(test_membership(
            &org,
            &f.owner,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let rows = machines::visible_nodes(&f.state.db, &f.chat).await.unwrap();
    assert!(rows.iter().any(|n| n.id == own.id));
    assert!(rows.iter().any(|n| n.id == shared.id));
    assert!(!rows.iter().any(|n| n.id == foreign.id));
    members
        .update_one(doc! {"org_user_id":&org}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert!(
        !machines::visible_nodes(&f.state.db, &f.chat)
            .await
            .unwrap()
            .iter()
            .any(|n| n.id == shared.id)
    );
    let mut guest = f.chat.clone();
    guest.guest = true;
    assert!(matches!(
        call(&f.state, &guest, "nyx__machine_list", json!({})).await,
        Err(AppError::MachineNotAllowed)
    ));
    assert!(
        call(
            &f.state,
            &f.chat,
            "nyx__machine_exec",
            json!({"machine":foreign.id,"command":"true"})
        )
        .await
        .is_err()
    );
    own.status = NodeStatus::Offline;
    save_node(&f, &own).await;
    assert!(matches!(
        call(
            &f.state,
            &f.chat,
            "nyx__machine_exec",
            json!({"machine":own.id,"command":"true"})
        )
        .await,
        Err(AppError::NodeOffline(_))
    ));
    own.status = NodeStatus::Online;
    own.machine.as_mut().unwrap().shell = false;
    save_node(&f, &own).await;
    assert!(matches!(
        call(
            &f.state,
            &f.chat,
            "nyx__machine_exec",
            json!({"machine":own.id,"command":"true"})
        )
        .await,
        Err(AppError::MachineCapabilityDisabled)
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_specialist_permission_is_explicit_durable_and_revocable() {
    use super::assistant_team_service::{self as team, GrantChange, MachineGrantMode};
    let f = fixture("machine_specialist_permission").await;
    let node = node(&f, &f.owner).await;
    let result = call(
        &f.state,
        &f.chat,
        "nyx__machine_read_file",
        json!({"machine":node.id,"path":"file"}),
    )
    .await
    .unwrap();
    let id = result["acknowledgement_id"].as_str().unwrap();
    let row = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("kind").unwrap(), "machine");
    assert_eq!(row.get_str("decider").unwrap(), "orchestrator");
    assert!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .machine_node_ids
            .is_empty()
    );
    acks::decide_as(
        &f.state.db,
        &f.owner,
        None,
        id,
        true,
        acks::Decider::Nyxbot,
        None,
    )
    .await
    .unwrap();
    let live = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(live.machine_node_ids.contains(&node.id));
    let (task, mut requests) = peer(&f, &node, json!({"content":"safe","has_more":false})).await;
    let result = call(
        &f.state,
        &live,
        "nyx__machine_read_file",
        json!({"machine":node.id,"path":"file"}),
    )
    .await
    .unwrap();
    assert_eq!(result["content"], "safe");
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::ReadFile
    );
    // Deleted resources must still be removable by UUID.
    f.state
        .db
        .collection::<Node>(crate::models::node::COLLECTION_NAME)
        .delete_one(doc! {"_id":&node.id})
        .await
        .unwrap();
    let change = machines::resolve_grant_change(
        &f.state.db,
        &f.owner,
        Some(vec![node.id.clone()]),
        None,
        GrantChange::Remove(Default::default()),
        MachineGrantMode::Remove,
    )
    .await
    .unwrap();
    let changed = team::set_grants(&f.state.db, &f.owner, &f.chat.agent_id, change)
        .await
        .unwrap();
    assert!(changed.machine_node_ids.is_empty());
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_confirmation_is_bound_to_parameters_and_consumed_once() {
    let f = orchestrator_fixture("machine_confirmation").await;
    let mut node = node(&f, &f.owner).await;
    node.machine_confirm = Confirmation::Changes;
    save_node(&f, &node).await;
    let (task, mut requests) = peer(&f, &node, json!({"sha256":"safe"})).await;
    let args = json!({"machine":node.id,"path":"file","content":"example","mode":"create"});
    let card = call(&f.state, &f.chat, "nyx__machine_write_file", args.clone())
        .await
        .unwrap();
    let id = card["acknowledgement_id"].as_str().unwrap();
    assert!(requests.try_recv().is_err());
    acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
        .await
        .unwrap();
    let mut changed = args.clone();
    changed["acknowledgement_id"] = json!(id);
    changed["content"] = json!("different");
    let result = call(&f.state, &f.chat, "nyx__machine_write_file", changed).await;
    assert!(result.is_err() || result.unwrap().get("acknowledgement_id").is_some());
    assert!(requests.try_recv().is_err());
    let mut approved = args;
    approved["acknowledgement_id"] = json!(id);
    assert_eq!(
        call(
            &f.state,
            &f.chat,
            "nyx__machine_write_file",
            approved.clone()
        )
        .await
        .unwrap()["sha256"],
        "safe"
    );
    requests.recv().await.unwrap();
    let repeated = call(&f.state, &f.chat, "nyx__machine_write_file", approved).await;
    assert!(repeated.is_err() || repeated.unwrap().get("acknowledgement_id").is_some());
    assert!(requests.try_recv().is_err());
    task.abort();
    f.state.db.drop().await.unwrap();
}

fn login_input() -> logins::Input {
    serde_json::from_value(json!({"label":"Test site","allowed_origins":["https://example.com"],"username":"synthetic-user-73591","password":"synthetic-password-82641","totp_secret":"GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"})).unwrap()
}

#[tokio::test]
async fn machine_saved_login_specialist_grants_confirmation_and_live_org_access() {
    use super::assistant_team_service::{self as team, GrantChange, MachineGrantMode};
    let f = fixture("machine_login_grant_confirmation").await;
    let mut node = node(&f, &f.owner).await;
    node.machine.as_mut().unwrap().browser_isolated = true;
    save_node(&f, &node).await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let memberships = f
        .state
        .db
        .collection(crate::models::org_membership::COLLECTION_NAME);
    memberships
        .insert_one(test_membership(
            &org,
            &f.owner,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let mut input = login_input();
    input.confirm_each_sign_in = true;
    let login = logins::put(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &org,
        None,
        input,
    )
    .await
    .unwrap();
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        GrantChange::Machine {
            base: Box::new(GrantChange::Add(Default::default())),
            machines: Some(vec![node.id.clone()]),
            logins: None,
            mode: MachineGrantMode::Add,
        },
    )
    .await
    .unwrap();
    let live = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        call(&f.state, &live, "nyx__saved_logins", json!({}))
            .await
            .unwrap()["logins"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let args = json!({"machine":node.id,"login":login.id,"field":"password"});
    let refusal = call(&f.state, &live, "nyx__machine_fill_login", args.clone())
        .await
        .unwrap();
    let id = refusal["acknowledgement_id"].as_str().unwrap();
    let card = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.get_str("kind").unwrap(), "saved_login");
    assert_eq!(card.get_str("decider").unwrap(), "orchestrator");
    acks::decide_as(
        &f.state.db,
        &f.owner,
        None,
        id,
        true,
        acks::Decider::Nyxbot,
        None,
    )
    .await
    .unwrap();
    let live = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(live.saved_login_ids.contains(&login.id));
    let (task, mut received) = peer(
        &f,
        &node,
        json!({"status":"filled","origin":"https://example.com"}),
    )
    .await;
    let confirm = call(&f.state, &live, "nyx__machine_fill_login", args.clone())
        .await
        .unwrap();
    let confirmation = confirm["acknowledgement_id"].as_str().unwrap();
    assert!(received.try_recv().is_err());
    assert!(
        acks::decide_as(
            &f.state.db,
            &f.owner,
            None,
            confirmation,
            true,
            acks::Decider::Nyxbot,
            None
        )
        .await
        .is_err(),
        "only the human may confirm secret use"
    );
    acks::decide(&f.state.db, &f.owner, &f.row.id, confirmation, true)
        .await
        .unwrap();
    let mut approved = args.clone();
    approved["acknowledgement_id"] = json!(confirmation);
    let filled = call(&f.state, &live, "nyx__machine_fill_login", approved)
        .await
        .unwrap();
    assert_eq!(filled["filled"], "password");
    assert!(!filled.to_string().contains("synthetic-password"));
    assert_eq!(
        received.recv().await.unwrap().operation,
        Operation::FillLogin
    );
    memberships
        .update_one(doc! {"org_user_id":&org}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert!(matches!(
        call(&f.state, &live, "nyx__machine_fill_login", args).await,
        Err(AppError::MachineLoginNotFound)
    ));
    assert!(received.try_recv().is_err());
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_lookups_are_batched_and_gateway_binding_is_one_indexed_read() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let f = orchestrator_fixture("machine_query_fixture").await;
    let node = node(&f, &f.owner).await;
    let commands = Arc::new(Mutex::new(Vec::<bson::Document>::new()));
    let recorded = commands.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            recorded.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "machine_query_budget",
        handler,
    )
    .await
    .unwrap();
    let nodes: Vec<_> = (0..64)
        .map(|_| {
            let mut row = node.clone();
            row.id = Uuid::new_v4().to_string();
            row
        })
        .collect();
    db.collection::<Node>(crate::models::node::COLLECTION_NAME)
        .insert_many(nodes)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    assert_eq!(
        machines::visible_nodes(&db, &f.chat).await.unwrap().len(),
        64
    );
    let reads = commands.lock().unwrap().clone();
    assert_eq!(
        reads.iter().filter(|c| c.contains_key("find")).count(),
        2,
        "one membership read and one node read, independent of node count"
    );
    let job = machines::issue_job(&db, &f.chat, &node, 120, Vec::new())
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    machines::gateway_job(&db, &node.id, &job.runtime_id, &f.row.id, &job.id)
        .await
        .unwrap();
    let reads = commands.lock().unwrap().clone();
    assert_eq!(reads.len(), 1);
    assert_eq!(
        reads[0]
            .get_document("filter")
            .unwrap()
            .get_str("_id")
            .unwrap(),
        job.id
    );
    db.drop().await.unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_catalog_discovery_projects_one_batch_of_referenced_and_allowed_ids() {
    use crate::services::{assistant_authority_tests::connected, machine_gateway_service};
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let f = orchestrator_fixture("machine_catalog_batch").await;
    let connection = connected(
        &f.state.db,
        &f.owner,
        "declared-model",
        "https://model.invalid",
    )
    .await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.inference = Some(crate::models::downstream_service::ServiceInference {
        wire_protocol: crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
        model_list: false,
        realtime: false,
    });
    let catalog_id = catalog.id.clone();
    f.state
        .db
        .collection("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! { "_id": &connection },
            doc! { "$set": { "catalog_service_id": &catalog_id } },
        )
        .await
        .unwrap();
    let allowed_catalog = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>("api_keys").update_one(
        doc! { "_id": &f.chat.api_key_id },
        doc! { "$set": { "allow_all_services": false, "allowed_service_ids": [&connection], "allowed_platform_service_ids": [&allowed_catalog] } },
    ).await.unwrap();
    // Unrelated active entries need not even have fields used by this listing.
    f.state.db.collection::<bson::Document>("downstream_services").insert_many(
        (0..64).map(|_| doc! { "_id": Uuid::new_v4().to_string(), "is_active": true, "description": "unrelated" }),
    ).await.unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let recorded = commands.clone();
    let monitor = crate::test_utils::connect_test_database_with_command_handler(
        "machine_catalog_monitor",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                recorded.lock().unwrap().push(event.command);
            }
        }),
    )
    .await
    .unwrap();
    let db = monitor.client().database(f.state.db.name());
    commands.lock().unwrap().clear();
    let rows = machine_gateway_service::services(&db, &f.owner, &f.chat.api_key_id)
        .await
        .unwrap();
    assert!(
        rows.iter()
            .any(|row| row.id == connection && row.inference.is_some())
    );
    let reads: Vec<_> = commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| {
            command.get_str("find") == Ok("downstream_services")
                && command
                    .get_document("projection")
                    .is_ok_and(|projection| projection.contains_key("credential_present"))
        })
        .cloned()
        .collect();
    assert_eq!(reads.len(), 1);
    let ids = reads[0]
        .get_document("filter")
        .unwrap()
        .get_document("_id")
        .unwrap()
        .get_array("$in")
        .unwrap();
    assert_eq!(ids.len(), 2); // referenced catalog + explicitly allowed catalog
    assert!(ids.contains(&bson::Bson::String(catalog_id)));
    assert!(ids.contains(&bson::Bson::String(allowed_catalog)));
    let projection = reads[0].get_document("projection").unwrap();
    for field in [
        "description",
        "base_url",
        "credential_encrypted",
        "billing",
        "token_exchange_config",
    ] {
        assert!(!projection.contains_key(field));
    }
    assert!(projection.contains_key("credential_present"));
    monitor.drop().await.unwrap();
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn machine_saved_logins_are_encrypted_write_only_human_only_and_owner_scoped() {
    use crate::handlers::{
        login_client_context::require_first_party_human, saved_logins::Metadata,
    };
    let f = orchestrator_fixture("machine_login_storage").await;
    let login = logins::put(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.owner,
        None,
        login_input(),
    )
    .await
    .unwrap();
    let stored = f
        .state
        .db
        .collection::<bson::Document>(crate::models::saved_login::COLLECTION_NAME)
        .find_one(doc! {"_id":&login.id})
        .await
        .unwrap()
        .unwrap();
    let visible = call(&f.state, &f.chat, "nyx__saved_logins", json!({}))
        .await
        .unwrap();
    let api = serde_json::to_string(&Metadata::from(login.clone())).unwrap();
    for value in [
        "synthetic-user-73591",
        "synthetic-password-82641",
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
    ] {
        assert!(!format!("{stored:?}{login:?}{api}{visible}").contains(value));
    }
    assert_eq!(
        logins::materialize(&f.state.encryption_keys, &login, "one_time_code", 59)
            .await
            .unwrap()
            .as_str(),
        "287082"
    );
    assert!(require_first_party_human(&f.auth).is_err());
    assert!(require_first_party_human(&crate::test_utils::test_auth_user(&f.owner)).is_ok());
    assert!(
        logins::get(&f.state.db, &Uuid::new_v4().to_string(), &login.id)
            .await
            .is_err()
    );
    let replacement = logins::put(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.owner,
        Some(&login.id),
        login_input(),
    )
    .await
    .unwrap();
    assert_ne!(replacement.password_encrypted, login.password_encrypted);
    logins::delete(&f.state.db, &f.owner, &login.id)
        .await
        .unwrap();
    assert!(logins::get(&f.state.db, &f.owner, &login.id).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_saved_login_single_user_opt_in_and_no_secret_result() {
    let f = orchestrator_fixture("machine_login_opt_in").await;
    let mut node = node(&f, &f.owner).await;
    let login = logins::put(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.owner,
        None,
        login_input(),
    )
    .await
    .unwrap();
    let (task, mut requests) = peer(
        &f,
        &node,
        json!({"status":"filled","field":"password","origin":"https://example.com"}),
    )
    .await;
    let args = json!({"machine":node.id,"login":login.id,"field":"password"});
    let denied = call(&f.state, &f.chat, "nyx__machine_fill_login", args.clone())
        .await
        .unwrap();
    assert_eq!(denied["error"]["code"], 12409);
    assert!(denied.to_string().contains("single-user"));
    assert!(requests.try_recv().is_err());
    node.allow_single_user_saved_logins = true;
    save_node(&f, &node).await;
    let filled = call(&f.state, &f.chat, "nyx__machine_fill_login", args)
        .await
        .unwrap();
    assert_eq!(
        filled,
        json!({"filled":"password","login":"Test site","origin":"https://example.com"})
    );
    let request = requests.recv().await.unwrap();
    assert_eq!(request.parameters["value"], "synthetic-password-82641");
    assert!(!format!("{request:?}{filled}").contains("synthetic-password-82641"));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let count = f
                .state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(
                    doc! {"event_type":{"$in":["machine_login_filled","machine_operation"]}},
                )
                .await
                .unwrap();
            if count >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("machine audit events must be durable before the secret sweep");
    // This is the only transport channel containing the value, with the node's
    // signature already verified by the peer. It is absent from durable rows.
    for collection in [
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
        crate::models::assistant_conversation::COLLECTION_NAME,
        crate::models::audit_log::COLLECTION_NAME,
    ] {
        use futures::TryStreamExt;
        let records: Vec<bson::Document> = f
            .state
            .db
            .collection(collection)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(!format!("{records:?}").contains("synthetic-password-82641"));
    }
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_gateway_binding_rejects_foreign_runtime_conversation_expiry_and_finished_jobs() {
    let f = orchestrator_fixture("machine_gateway_binding").await;
    let node = node(&f, &f.owner).await;
    let job = machines::issue_job(&f.state.db, &f.chat, &node, 120, Vec::new())
        .await
        .unwrap();
    assert!(
        machines::gateway_job(&f.state.db, &node.id, &job.runtime_id, &f.row.id, &job.id)
            .await
            .is_ok()
    );
    for (node_id, runtime, conversation, id) in [
        (
            "other-node",
            job.runtime_id.as_str(),
            f.row.id.as_str(),
            job.id.as_str(),
        ),
        (
            node.id.as_str(),
            "other-runtime",
            f.row.id.as_str(),
            job.id.as_str(),
        ),
        (
            node.id.as_str(),
            job.runtime_id.as_str(),
            "other-chat",
            job.id.as_str(),
        ),
        (
            node.id.as_str(),
            job.runtime_id.as_str(),
            f.row.id.as_str(),
            "node-invented-job",
        ),
    ] {
        assert!(
            machines::gateway_job(&f.state.db, node_id, runtime, conversation, id)
                .await
                .is_err()
        );
    }
    f.state.db.collection::<bson::Document>(crate::models::machine_job::COLLECTION_NAME)
        .update_one(doc!{"_id":&job.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert!(
        machines::gateway_job(&f.state.db, &node.id, &job.runtime_id, &f.row.id, &job.id)
            .await
            .is_err()
    );
    let job = machines::issue_job(&f.state.db, &f.chat, &node, 120, Vec::new())
        .await
        .unwrap();
    machines::finish(&f.state.db, &job.id).await.unwrap();
    assert!(
        machines::gateway_job(&f.state.db, &node.id, &job.runtime_id, &f.row.id, &job.id)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_setup_pairing_races_delivery_hashes_and_expiry() {
    use super::machine_setup_service as setup;
    use crate::models::machine_setup::{COLLECTION_NAME, Choices, MachineSetup};
    let f = orchestrator_fixture("machine_setup_races").await;
    let key = b"test-machine-pairing-hmac";
    let pair = setup::initiate(
        &f.state.db,
        key,
        "test-host",
        "linux",
        "127.0.0.1",
        vec!["shell".into()],
    )
    .await
    .unwrap();
    let row = setup::by_code(&f.state.db, key, &pair.code).await.unwrap();
    let stored = bson::to_document(&row).unwrap().to_string();
    assert!(!stored.contains(pair.device.as_str()));
    assert!(!stored.contains(&pair.code));
    assert!(
        setup::poll(&f.state.db, key, &pair.device, 100)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        setup::poll(&f.state.db, key, &pair.device, 100).await,
        Err(AppError::AuthDeviceCodeSlowDown)
    ));
    let (approve, deny) = tokio::join!(
        setup::decide(&f.state.db, &f.owner, &row.id, true, Some(&f.row.id)),
        setup::decide(&f.state.db, &f.owner, &row.id, false, Some(&f.row.id))
    );
    assert_ne!(approve.is_ok(), deny.is_ok());
    let final_row = setup::get(&f.state.db, &f.owner, &row.id).await.unwrap();
    assert!(matches!(final_row.status.as_str(), "approved" | "declined"));
    let link = setup::create_link(
        &f.state.db,
        &f.owner,
        Some(&f.row.id),
        Choices {
            automatic_updates: None,
            owner_id: None,
            name: "test-machine".into(),
            location: "docker".into(),
            capabilities: vec!["shell".into(), "files".into()],
            grant_to: None,
        },
    )
    .await
    .unwrap();
    let (first, second) = tokio::join!(
        setup::mint(&f.state.db, &f.owner, &link.id, None, 100, "review"),
        setup::mint(&f.state.db, &f.owner, &link.id, None, 100, "review")
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let token = first.or(second).unwrap();
    let stored = setup::get(&f.state.db, &f.owner, &link.id).await.unwrap();
    assert!(
        !bson::to_document(&stored)
            .unwrap()
            .to_string()
            .contains(token.as_str())
    );
    let (registered, _, _) =
        node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .unwrap();
    assert_eq!(registered.id, link.id);
    assert!(
        node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .is_err()
    );
    f.state.db.collection::<MachineSetup>(COLLECTION_NAME).update_one(doc!{"_id":&row.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert!(matches!(
        setup::by_code(&f.state.db, key, &pair.code).await,
        Err(AppError::AuthDeviceCodeExpired)
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_page_setup_grant_is_atomic_and_never_restored_on_reconnect() {
    use super::{assistant_team_service as team, machine_setup_service as setup};
    let f = fixture("machine_page_setup_grant").await;
    let machine = node(&f, &f.owner).await;
    let mut intent = setup::create_link(
        &f.state.db,
        &f.owner,
        None,
        crate::models::machine_setup::Choices {
            automatic_updates: None,
            owner_id: None,
            name: "setup-machine".into(),
            location: "docker".into(),
            capabilities: vec!["shell".into(), "files".into()],
            grant_to: Some(f.chat.agent_id.clone()),
        },
    )
    .await
    .unwrap();
    let setups = f
        .state
        .db
        .collection::<crate::models::machine_setup::MachineSetup>(
            crate::models::machine_setup::COLLECTION_NAME,
        );
    setups.delete_one(doc! {"_id":&intent.id}).await.unwrap();
    intent.id = machine.id.clone();
    intent.status = "waiting".into();
    setups.insert_one(&intent).await.unwrap();
    let (one, two) = tokio::join!(
        setup::complete_page_setup(&f.state.db, &machine.id),
        setup::complete_page_setup(&f.state.db, &machine.id)
    );
    one.unwrap();
    two.unwrap();
    let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(agent.machine_node_ids, vec![machine.id.clone()]);
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Machine {
            base: Box::new(team::GrantChange::Add(Default::default())),
            machines: Some(vec![machine.id.clone()]),
            logins: None,
            mode: team::MachineGrantMode::Remove,
        },
    )
    .await
    .unwrap();
    setup::complete_page_setup(&f.state.db, &machine.id)
        .await
        .unwrap();
    assert!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .machine_node_ids
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_watch_settlement_queues_exactly_one_durable_secret_free_event() {
    use crate::models::nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME};
    let f = orchestrator_fixture("machine_durable_watch").await;
    let intent = super::machine_setup_service::create_link(
        &f.state.db,
        &f.owner,
        Some(&f.row.id),
        crate::models::machine_setup::Choices {
            automatic_updates: None,
            owner_id: None,
            name: "watched".into(),
            location: "docker".into(),
            capabilities: vec!["shell".into()],
            grant_to: None,
        },
    )
    .await
    .unwrap();
    let watch = f
        .state
        .db
        .collection::<NyxbotWatch>(WATCHES_COLLECTION_NAME)
        .find_one(doc! {"connect_link_id":&intent.id})
        .await
        .unwrap()
        .unwrap();
    let started = std::time::Instant::now();
    let (one, two) = tokio::join!(
        machines::settle_watch(
            &f.state.db,
            &watch,
            "machine_setup_finished",
            "Machine connected".into(),
            None
        ),
        machines::settle_watch(
            &f.state.db,
            &watch,
            "machine_setup_finished",
            "Machine connected".into(),
            None
        ),
    );
    assert_ne!(one.unwrap(), two.unwrap());
    let conversation = super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(conversation.pending_events.len(), 1);
    let serialized = serde_json::to_string(&conversation.pending_events).unwrap();
    for forbidden in ["nyx_nreg_", "nyx_nauth_", "device_hmac", "code_hmac"] {
        assert!(!serialized.contains(forbidden));
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_owner_takeover_blocks_tools_and_handback_wakes_with_note() {
    use super::machine_desktop_service as desktop;
    let f = orchestrator_fixture("machine_owner_handback").await;
    let node = node(&f, &f.owner).await;
    let row = desktop::open(&f.state.db, &f.owner, &node.id, Some(&f.row.id))
        .await
        .unwrap();
    let row = desktop::take(&f.state.db, &row, "owner-tab").await.unwrap();
    let row = desktop::controlled(&f.state.db, &row, "owner-tab")
        .await
        .unwrap();
    crate::handlers::machine_desktop::watch(&f.state, &row)
        .await
        .unwrap();
    for (tool, arguments) in [
        (
            "nyx__machine_exec",
            json!({"machine":node.id,"command":"true"}),
        ),
        (
            "nyx__machine_read_file",
            json!({"machine":node.id,"path":"."}),
        ),
        (
            "nyx__machine_computer",
            json!({"machine":node.id,"tool":"get_window_state","arguments":{}}),
        ),
    ] {
        assert!(matches!(
            call(&f.state, &f.chat, tool, arguments).await,
            Err(AppError::MachineOwnerInControl)
        ));
    }
    let row = desktop::release(
        &f.state.db,
        &row,
        "owner-tab",
        "signed in; continue the draft",
    )
    .await
    .unwrap();
    desktop::returned(&f.state.db, &row).await.unwrap();
    crate::handlers::nyxbot::process_watches(&f.state)
        .await
        .unwrap();
    crate::handlers::nyxbot::process_watches(&f.state)
        .await
        .unwrap();
    let conversation = super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let events = serde_json::to_value(&conversation.pending_events).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert!(events.to_string().contains("machine_control_returned"));
    assert!(events.to_string().contains("signed in; continue the draft"));
    desktop::agent_allowed(&f.state.db, &node.id).await.unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "repeatable machine dispatch benchmark; run alone with --ignored --nocapture"]
async fn machine_exec_dispatch_performance() {
    let f = orchestrator_fixture("machine_exec_performance").await;
    let node = node(&f, &f.owner).await;
    let (task, _) = peer(
        &f,
        &node,
        json!({"exit_code":0,"stdout":"","stderr":"","duration_ms":0,"truncated":false}),
    )
    .await;
    let mut samples = Vec::new();
    for iteration in 0..110 {
        let start = std::time::Instant::now();
        let result = call(
            &f.state,
            &f.chat,
            "nyx__machine_exec",
            json!({"machine":node.id,"command":"true"}),
        )
        .await
        .unwrap();
        assert_eq!(result["exit_code"], 0);
        if iteration >= 10 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "machine_exec in-process signed dispatch overhead, 100 samples: p50={:.3} ms p95={:.3} ms",
        samples[49], samples[94]
    );
    assert!(
        samples[94] <= 50.0,
        "machine dispatch exceeds the 50 ms budget"
    );
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_setup_change_stream_wakes_thread_without_sweep() {
    use super::machine_setup_service as setup;
    use std::time::{Duration, Instant};
    let f = orchestrator_fixture("machine_setup_live_wake").await;
    let intent = setup::create_link(
        &f.state.db,
        &f.owner,
        Some(&f.row.id),
        crate::models::machine_setup::Choices {
            automatic_updates: None,
            owner_id: None,
            name: "live-machine".into(),
            location: "docker".into(),
            capabilities: vec!["shell".into()],
            grant_to: None,
        },
    )
    .await
    .unwrap();
    let token = setup::mint(&f.state.db, &f.owner, &intent.id, None, 100, "review")
        .await
        .unwrap();
    let live = f.state.assistant_live.clone();
    let db = f.state.db.clone();
    let runner = tokio::spawn(async move { live.run(db).await });
    let mut open = f.state.assistant_live.watch_open();
    tokio::time::timeout(
        Duration::from_secs(
            if std::env::var("NYXID_MACHINE_STRICT_BENCHMARK").as_deref() == Ok("1") {
                10
            } else {
                60
            },
        ),
        async {
            while !*open.borrow_and_update() {
                open.changed().await.unwrap();
            }
        },
    )
    .await
    .unwrap();
    crate::handlers::nyxbot::spawn_live_dispatch(f.state.clone());
    let (mut node, _, _) =
        node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .unwrap();
    node.status = NodeStatus::Online;
    node.machine = Some(MachineProfile {
        version: 1,
        shell: true,
        runtime_id: Uuid::new_v4().to_string(),
        ..Default::default()
    });
    let start = Instant::now();
    save_node(&f, &node).await;
    let conversation = tokio::time::timeout(
        Duration::from_secs(
            if std::env::var("NYXID_MACHINE_STRICT_BENCHMARK").as_deref() == Ok("1") {
                10
            } else {
                60
            },
        ),
        async {
            loop {
                let row = super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
                    .await
                    .unwrap();
                if !row.pending_events.is_empty() {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        },
    )
    .await
    .expect("setup must wake through the change stream within ten seconds");
    println!(
        "Machine capability report to durable NyxBot wake: {:.2} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let events = serde_json::to_string(&conversation.pending_events).unwrap();
    assert!(events.contains("machine_setup_finished"));
    for secret in [&*token, "nyx_nreg_", "nyx_nauth_"] {
        assert!(!events.contains(secret));
    }
    runner.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_setup_tools_and_owner_cards_never_contain_registration_credentials() {
    use crate::handlers::machine_setup::{link_tool, pair_tool};
    let f = orchestrator_fixture("machine_setup_model_boundary").await;
    let (link, _) = link_tool(
        &f.state,
        &f.chat,
        &json!({"where":"docker","capabilities":["shell","files"]}),
    )
    .await
    .unwrap();
    assert!(
        link["url"]
            .as_str()
            .unwrap()
            .contains("/assistant/machines/new?setup=")
    );
    let pair = super::machine_setup_service::initiate(
        &f.state.db,
        f.state.auth_device_hmac_key.as_slice(),
        "test-host",
        "linux",
        "127.0.0.1",
        vec!["shell".into()],
    )
    .await
    .unwrap();
    let (card, _) = pair_tool(&f.state, &f.chat, &json!({"code":pair.code}))
        .await
        .unwrap();
    let id = card["acknowledgement_id"].as_str().unwrap();
    let row = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("decider").unwrap(), "user");
    let body = format!("{link}{card}{row:?}");
    assert!(body.contains("test-host") && body.contains("linux") && body.contains("127.0.0.1"));
    for forbidden in [
        pair.device.as_str(),
        "nyx_nreg_",
        "nyx_nauth_",
        "signing_secret",
    ] {
        assert!(!body.contains(forbidden));
    }
    assert!(
        acks::decide_as(
            &f.state.db,
            &f.owner,
            None,
            id,
            true,
            acks::Decider::Nyxbot,
            None
        )
        .await
        .is_err()
    );
    let mut guest = f.chat.clone();
    guest.guest = true;
    assert!(
        link_tool(&f.state, &guest, &json!({"where":"docker"}))
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn unsupported_computer_tools_return_advertised_names_and_browser_guidance() {
    let f = orchestrator_fixture("machine_unsupported_tool_help").await;
    let node = node(&f, &f.owner).await;
    let result = call(
        &f.state,
        &f.chat,
        "nyx__machine_computer",
        json!({"machine":node.id,"tool":"screenshot","arguments":{}}),
    )
    .await
    .unwrap();
    assert_eq!(result["error"]["code"], 12416);
    assert_eq!(
        result["error"]["computer_tools"],
        json!(["get_window_state", "click"])
    );
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nyx__machine_browser action=snapshot")
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_screenshots_use_owner_attachments_with_magic_and_turn_limits() {
    use base64::Engine;
    let f = orchestrator_fixture("machine_image_attachments").await;
    let node = node(&f, &f.owner).await;
    let pixels =
        base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nsynthetic-pixels");
    let (task, _) = peer(
        &f,
        &node,
        json!({"content":[{"type":"image","mimeType":"image/png","data":pixels}]}),
    )
    .await;
    for _ in 0..9 {
        let result = call(
            &f.state,
            &f.chat,
            "nyx__machine_computer",
            json!({"machine":node.id,"tool":"get_window_state","arguments":{}}),
        )
        .await
        .unwrap();
        assert!(!result.to_string().contains(&pixels));
        assert!(result["content"][0]["text"].is_string());
    }
    let attachments = f
        .state
        .db
        .collection::<crate::models::assistant_attachment::AssistantAttachment>(
            crate::models::assistant_attachment::COLLECTION_NAME,
        );
    assert_eq!(
        attachments
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        8
    );
    let attachment = attachments
        .find_one(doc! {"conversation_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        super::assistant_nyxagent::read_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &Uuid::new_v4().to_string(),
            &attachment.id
        )
        .await
        .is_err()
    );
    assert!(
        super::assistant_nyxagent::read_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &Uuid::new_v4().to_string(),
            &f.row.id,
            &attachment.id
        )
        .await
        .is_err()
    );
    task.abort();
    let (task,_)=peer(&f,&node,json!({"content":[{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(b"not an image")}]})).await;
    assert!(
        call(
            &f.state,
            &f.chat,
            "nyx__machine_computer",
            json!({"machine":node.id,"tool":"get_window_state","arguments":{}})
        )
        .await
        .is_err()
    );
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stop_fences_issued_machine_turns_cancels_signed_jobs_and_allows_a_new_turn() {
    let f = orchestrator_fixture("machine_stop_turn").await;
    let node = node(&f, &f.owner).await;
    let (task, mut requests) = peer(&f, &node, json!({"status":"running"})).await;
    call(
        &f.state,
        &f.chat,
        "nyx__machine_exec",
        json!({"machine":node.id,"command":"sleep 30","services":[],"background":true}),
    )
    .await
    .unwrap();
    let issued = requests.recv().await.unwrap();
    assert_eq!(
        issued.parameters["turn_id"],
        f.chat.turn_id.as_deref().unwrap()
    );
    let started = std::time::Instant::now();
    crate::handlers::machine_cancel::conversation(&f.state, &f.owner, &f.row.id)
        .await
        .unwrap();
    let stop_budget = if std::env::var("NYXID_MACHINE_STRICT_BENCHMARK").as_deref() == Ok("1") {
        1
    } else {
        5
    };
    println!(
        "signed Stop dispatch: {:.3} ms",
        started.elapsed().as_secs_f64() * 1000.
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(stop_budget));
    let cancel = requests.recv().await.unwrap();
    assert_eq!(cancel.operation, Operation::Cancel);
    assert_eq!(cancel.parameters["conversation_id"], f.row.id);
    assert_eq!(
        cancel.parameters["turn_id"],
        f.chat.turn_id.as_deref().unwrap()
    );
    assert!(matches!(
        call(
            &f.state,
            &f.chat,
            "nyx__machine_exec",
            json!({"machine":node.id,"command":"true","services":[]})
        )
        .await,
        Err(AppError::MachineTurnStopped)
    ));
    assert_eq!(
        f.state
            .db
            .collection::<crate::models::machine_job::MachineJob>(
                crate::models::machine_job::COLLECTION_NAME
            )
            .count_documents(doc! {"node_id":&node.id,"state":"running"})
            .await
            .unwrap(),
        0
    );
    super::assistant_nyxagent::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &super::assistant_nyxagent::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(super::assistant_nyxagent::TurnError::new("cancelled")),
        },
    )
    .await
    .unwrap();
    let next = super::assistant_nyxagent::begin_turn(
        &f.state.db,
        &f.owner,
        &super::assistant_nyxagent::TurnRequest {
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: Some(f.row.id.clone()),
            text: "Resume".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&f.state.db, &f.owner, Some(&next.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(chat.turn_id.as_deref(), f.chat.turn_id.as_deref());
    call(
        &f.state,
        &chat,
        "nyx__machine_exec",
        json!({"machine":node.id,"command":"true","services":[]}),
    )
    .await
    .unwrap();
    assert_eq!(
        requests.recv().await.unwrap().parameters["turn_id"],
        chat.turn_id.as_deref().unwrap()
    );
    crate::handlers::machine_cancel::machine(&f.state, &node.id)
        .await
        .unwrap();
    let all = requests.recv().await.unwrap();
    assert_eq!(all.operation, Operation::Cancel);
    assert_eq!(all.parameters["all"], true);
    assert_eq!(
        all.parameters["scopes"][0]["turn_id"],
        chat.turn_id.as_deref().unwrap()
    );
    task.abort();
}

#[tokio::test]
async fn legacy_machine_browser_offers_the_guided_update_without_dispatch_timeout() {
    let f = orchestrator_fixture("machine_old_browser_update").await;
    let mut legacy = node(&f, &f.owner).await;
    legacy.machine.as_mut().unwrap().browser_tools = false;
    save_node(&f, &legacy).await;
    let result = call(
        &f.state,
        &f.chat,
        "nyx__machine_browser",
        json!({"machine":legacy.id,"action":"snapshot"}),
    )
    .await
    .unwrap();
    assert_eq!(result["error"]["code"], 12416);
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nyxid__machine_update")
    );
    f.state.db.drop().await.unwrap();
}
