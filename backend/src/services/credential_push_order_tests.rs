use super::*;
use crate::{models::user::UserType, services::user_api_key_service, test_utils::*};
use bson::Document;
use serde_json::Value;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

pub(super) async fn fixture() -> (crate::AppState, UserApiKey, UserService) {
    let db = connect_transaction_test_database("node_push_order").await;
    fixture_in(db).await
}

async fn fixture_in(db: mongodb::Database) -> (crate::AppState, UserApiKey, UserService) {
    crate::services::node_credential_delivery::ensure_indexes(&db)
        .await
        .unwrap();
    crate::services::coordination_service::ensure_indexes(&db)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let initial = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let key = user_api_key_service::create_api_key(
        &db,
        &state.encryption_keys,
        &owner,
        user_api_key_service::CreateApiKeyParams {
            label: "ordering fixture",
            credential_type: "api_key",
            credential: &initial,
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    let endpoint = test_user_endpoint(
        &uuid::Uuid::new_v4().to_string(),
        &owner,
        "ordering fixture",
        "https://example.invalid",
        None,
        None,
    );
    db.collection::<UserEndpoint>(USER_ENDPOINTS)
        .insert_one(&endpoint)
        .await
        .unwrap();
    let node = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>("nodes")
        .insert_one(doc! {
            "_id": &node, "user_id": &owner, "name": "ordering fixture", "is_active": true,
            "status": "online", "auth_token_hash": "fixture", "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let mut service = test_user_service(
        &uuid::Uuid::new_v4().to_string(),
        &owner,
        "ordering-fixture",
        &endpoint.id,
        None,
        Some(&node),
    );
    service.api_key_id = Some(key.id.clone());
    service.auth_method = "header".into();
    service.auth_key_name = "Authorization".into();
    // Existing records before this upgrade are also eligible for durable delivery.
    db.collection::<UserService>(USER_SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    (state, key, service)
}

pub(super) async fn rotate(state: &crate::AppState, key: &UserApiKey, value: &str) {
    user_api_key_service::update_api_key(
        &state.db,
        &state.encryption_keys,
        &key.user_id,
        &key.id,
        None,
        Some(value),
    )
    .await
    .unwrap();
}

pub(super) async fn next_frame(
    rx: &mut mpsc::Receiver<crate::services::node_ws_manager::NodeOutboundMessage>,
) -> Value {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let crate::services::node_ws_manager::NodeOutboundMessage::Text(frame) = frame else {
        panic!("expected ordinary update");
    };
    serde_json::from_str(&frame).unwrap()
}

#[tokio::test]
async fn node_credential_order_2472_two_replacements_hold_first_push_until_second_arrives() {
    let (state, key, service) = fixture().await;
    let (tx, mut rx) = mpsc::channel(4);
    register_test_node_connection(&state, service.node_id.as_deref().unwrap(), tx).await;
    let first = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let second = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &first).await;
    push_credential_to_node_if_routed(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &key.user_id,
        &key.id,
    )
    .await;
    let held = next_frame(&mut rx).await;
    rotate(&state, &key, &second).await;
    push_credential_to_node_if_routed(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &key.user_id,
        &key.id,
    )
    .await;
    let latest = next_frame(&mut rx).await;
    // Model the protocol consumer; the CLI test exercises the actual persisted store.
    let mut held_generation = 0;
    let mut material = Zeroizing::new(String::new());
    for frame in [&latest, &held] {
        let generation = frame["credential_generation"].as_i64();
        if generation.is_none_or(|generation| generation >= held_generation) {
            held_generation = generation.unwrap_or(held_generation);
            material = Zeroizing::new(frame["header_value"].as_str().unwrap().to_owned());
        }
    }
    assert!(
        material.as_str() == second.as_str(),
        "late arrival regressed the node after two successful saves"
    );
}

#[tokio::test]
async fn node_credential_order_2472_save_before_process_death_leaves_survivor_delivery_intent() {
    let (state, key, service) = fixture().await;
    let replacement = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &replacement).await;
    let db = state.db.clone();
    // No detached push is allowed to start. Drop all originating process state.
    drop(state);
    let survivor = test_app_state(db);
    let desired = survivor
        .db
        .collection::<Document>("node_credential_desired")
        .find_one(doc! {"node_id": &service.node_id, "service_slug": &service.slug})
        .await
        .unwrap();
    assert!(
        desired.is_some(),
        "the saved replacement has no durable intent for a survivor to deliver"
    );
    let node = service.node_id.as_deref().unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    register_test_node_connection(&survivor, node, tx).await;
    survivor.node_ws_manager.record_capabilities(
        node,
        &crate::services::node_ws_manager::NodeCapabilitiesMsg {
            credential_ack_correlation: true,
            ..Default::default()
        },
    );
    let shutdown = crate::services::shutdown::Runtime::default();
    let (result, ()) = tokio::join!(
        crate::services::node_credential_delivery::run_pass(
            &survivor.db,
            &survivor.encryption_keys,
            &survivor.node_ws_manager,
            crate::services::coordination_service::cluster_lease_runtime(),
            &shutdown
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert!(frame["header_value"].as_str().unwrap() == replacement.as_str());
            survivor.node_ws_manager.deliver_credential_ack(
                node,
                frame["request_id"].as_str().unwrap(),
                crate::services::node_ws_manager::CredentialAckOutcome::Versioned(
                    crate::services::node_ws_manager::CredentialVersion {
                        credential_id: frame["credential_id"].as_str().unwrap().into(),
                        credential_generation: frame["credential_generation"].as_i64().unwrap(),
                    },
                ),
            );
        }
    );
    result.unwrap();
    let delivered = survivor
        .db
        .collection::<Document>("node_credential_desired")
        .find_one(doc! {"node_id": node, "service_slug": &service.slug})
        .await
        .unwrap()
        .unwrap();
    assert!(!delivered.get_bool("pending").unwrap());
    assert_eq!(
        delivered.get_i64("generation").unwrap(),
        delivered.get_i64("acknowledged_generation").unwrap()
    );
    assert_eq!(shutdown.tracked_units(), 0);
}

#[tokio::test]
async fn node_credential_order_2472_replacement_request_command_counts() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::{
        collections::BTreeMap,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };
    let enabled = Arc::new(AtomicBool::new(false));
    let counts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let capture_enabled = enabled.clone();
    let capture_counts = counts.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && capture_enabled.load(Ordering::SeqCst)
        {
            *capture_counts
                .lock()
                .unwrap()
                .entry(event.command_name)
                .or_default() += 1;
        }
    });
    let db =
        connect_transaction_test_database_with_command_handler("node_push_counts", handler).await;
    let (state, key, _) = fixture_in(db).await;
    let replacement = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    enabled.store(true, Ordering::SeqCst);
    let result = crate::handlers::user_api_keys_external::update_external_api_key(
        axum::extract::State(state.clone()),
        test_auth_user(&key.user_id),
        axum::extract::Path(key.id.clone()),
        axum::Json(
            crate::handlers::user_api_keys_external::UpdateExternalApiKeyRequest {
                label: None,
                credential: Some(replacement.to_string()),
            },
        ),
    )
    .await;
    // Current-thread runtime: the spawned push cannot run between handler return
    // and this toggle, so these are request commands, excluding detached delivery.
    enabled.store(false, Ordering::SeqCst);
    assert!(result.is_ok());
    let counts = counts.lock().unwrap();
    eprintln!(
        "replacement request commands: total={} by_name={counts:?}",
        counts.values().sum::<usize>()
    );
}
