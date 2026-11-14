use super::order_tests::{fixture, next_frame, rotate};
use super::*;
use crate::services::{
    coordination_service::{ClusterLeaseRuntime, LeaseStore},
    node_credential_delivery as delivery,
    node_ws_manager::{CredentialAckOutcome, CredentialVersion, NodeCapabilitiesMsg},
};
use bson::Document;
use std::time::Duration;
use zeroize::Zeroizing;

async fn attach(
    state: &crate::AppState,
    node: &str,
) -> tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::NodeOutboundMessage> {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    crate::test_utils::register_test_node_connection(state, node, tx).await;
    state.node_ws_manager.record_capabilities(
        node,
        &NodeCapabilitiesMsg {
            credential_ack_correlation: true,
            ..Default::default()
        },
    );
    state.node_ws_manager.mark_status_update_received(node);
    rx
}

fn acknowledge(state: &crate::AppState, node: &str, frame: &serde_json::Value, legacy: bool) {
    let outcome = if legacy {
        CredentialAckOutcome::Ok
    } else {
        CredentialAckOutcome::Versioned(CredentialVersion {
            credential_id: frame["credential_id"].as_str().unwrap().into(),
            credential_generation: frame["credential_generation"].as_i64().unwrap(),
        })
    };
    state.node_ws_manager.deliver_credential_ack(
        node,
        frame["request_id"].as_str().unwrap(),
        outcome,
    );
}

async fn row(state: &crate::AppState, svc: &UserService) -> Document {
    state
        .db
        .collection::<Document>(delivery::COLLECTION)
        .find_one(doc! {"node_id": &svc.node_id, "service_slug": &svc.slug})
        .await
        .unwrap()
        .unwrap()
}

fn runner() -> ClusterLeaseRuntime {
    ClusterLeaseRuntime::new(
        crate::models::coordination::CoordinationHolder {
            instance_id: uuid::Uuid::new_v4().to_string(),
            generation_id: uuid::Uuid::new_v4().to_string(),
        },
        Duration::from_secs(30),
        Duration::from_secs(10),
    )
}

#[tokio::test]
async fn node_credential_order_2472_refresh_advances_delivery_without_changing_approval_epoch() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    let before = row(&state, &svc).await;
    let saved =
        crate::services::user_api_key_service::get_api_key(&state.db, &key.user_id, &key.id)
            .await
            .unwrap();
    let refreshed = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let ciphertext = crate::services::credential_record_service::encrypt(
        &state.encryption_keys,
        USER_API_KEYS,
        &key.id,
        &key.user_id,
        "credential_encrypted",
        refreshed.as_bytes(),
    )
    .await
    .unwrap();
    crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
        .update_one(
            doc! {"_id": &key.id},
            doc! {"$set": {"credential_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic, bytes: ciphertext,
            }}},
        )
        .routine_refresh()
        .await
        .unwrap();
    let after = row(&state, &svc).await;
    assert_eq!(
        after.get_i64("generation").unwrap(),
        before.get_i64("generation").unwrap() + 1
    );
    let installed =
        crate::services::user_api_key_service::get_api_key(&state.db, &key.user_id, &key.id)
            .await
            .unwrap();
    assert_eq!(installed.credential_epoch, saved.credential_epoch);
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert!(frame["header_value"].as_str().unwrap() == refreshed.as_str());
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    assert!(after.keys().all(|field| !field.contains("encrypted")
        && field != "header_value"
        && field != "payload"));
}

#[tokio::test]
async fn node_credential_order_2472_old_node_retry_reads_current_material_and_acks_exact_generation()
 {
    let (state, key, svc) = fixture().await;
    let first = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let latest = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &first).await;
    rotate(&state, &key, &latest).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert!(frame["header_value"].as_str().unwrap() == latest.as_str());
            acknowledge(&state, node, &frame, true);
        }
    );
    result.unwrap();
    let saved = row(&state, &svc).await;
    assert!(!saved.get_bool("pending").unwrap());
    assert_eq!(
        saved.get_i64("acknowledged_generation").unwrap(),
        saved.get_i64("generation").unwrap()
    );
}

#[tokio::test]
async fn node_credential_order_2472_late_ack_cannot_complete_a_newer_desired_generation() {
    let (state, key, svc) = fixture().await;
    let first = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let latest = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &first).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            rotate(&state, &key, &latest).await;
            // A second runner cannot take this slot while its send is unresolved.
            delivery::retry_node(
                &state.db,
                &state.encryption_keys,
                &state.node_ws_manager,
                node,
            )
            .await
            .unwrap();
            assert!(rx.try_recv().is_err());
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    let saved = row(&state, &svc).await;
    assert!(saved.get_bool("pending").unwrap());
    assert!(
        saved.get_i64("acknowledged_generation").unwrap() < saved.get_i64("generation").unwrap()
    );
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert!(frame["header_value"].as_str().unwrap() == latest.as_str());
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
}

#[tokio::test]
async fn node_credential_order_2472_two_runners_drain_finishes_admitted_send_and_releases_lease() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    let a = runner();
    let b = runner();
    let shutdown = crate::services::shutdown::Runtime::default();
    let (result, ()) = tokio::join!(
        delivery::run_pass(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &a,
            &shutdown
        ),
        async {
            let frame = next_frame(&mut rx).await;
            delivery::run_pass(
                &state.db,
                &state.encryption_keys,
                &state.node_ws_manager,
                &b,
                &shutdown,
            )
            .await
            .unwrap();
            assert!(rx.try_recv().is_err());
            shutdown.begin_drain();
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    assert_eq!(shutdown.tracked_units(), 0);
    let token = b
        .acquire(&state.db, delivery::RETRY_LEASE)
        .await
        .unwrap()
        .unwrap();
    assert!(LeaseStore::release(&state.db, &token).await.unwrap());
    rotate(&state, &key, &value).await;
    delivery::run_pass(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &a,
        &shutdown,
    )
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
    assert!(row(&state, &svc).await.get_bool("pending").unwrap());
}

#[tokio::test]
async fn node_credential_order_2472_disabled_service_defers_without_erasing_local_material() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .update_one(doc! {"_id": &svc.id}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    delivery::retry_node(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        node,
    )
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .update_one(doc! {"_id": &svc.id}, doc! {"$set": {"is_active": true}})
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_update");
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
}

#[tokio::test]
async fn node_credential_order_2472_expired_strict_preparation_recovers_committed_state() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    let node = svc.node_id.as_deref().unwrap();
    let (preparation, speculative) = delivery::prepare_update(
        &state.db,
        &state.encryption_keys,
        &key.user_id,
        None,
        &StrictPushTarget {
            service_id: &svc.id,
            target_node_id: node,
            service_slug: &svc.slug,
            auth_method: "none",
            auth_key_name: "",
            target_url: None,
        },
    )
    .await
    .unwrap();
    drop(preparation); // Origin dies after a speculative no-auth send; service write did not commit.
    state
        .db
        .collection::<Document>(delivery::COLLECTION)
        .update_one(
            doc! {"node_id": node},
            doc! {"$set": {"preparing_until": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let mut rx = attach(&state, node).await;
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["injection_method"], "header");
            assert!(frame["header_value"].as_str().unwrap() == value.as_str());
            assert!(
                frame["credential_generation"].as_i64().unwrap()
                    > speculative.version.as_ref().unwrap().credential_generation
            );
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
}

#[tokio::test]
async fn node_credential_order_2472_expired_sender_cannot_release_successor_claim_or_acknowledgement()
 {
    let (state, key, svc) = fixture().await;
    let first = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let latest = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &first).await;
    let node = svc.node_id.clone().unwrap();
    let mut rx = attach(&state, &node).await;
    let sender_state = state.clone();
    let sender_node = node.clone();
    let older = tokio::spawn(async move {
        delivery::retry_node(
            &sender_state.db,
            &sender_state.encryption_keys,
            &sender_state.node_ws_manager,
            &sender_node,
        )
        .await
    });
    let held = next_frame(&mut rx).await;
    rotate(&state, &key, &latest).await;
    state
        .db
        .collection::<Document>(delivery::COLLECTION)
        .update_one(
            doc! {"node_id": &node},
            doc! {"$set": {"claim_until": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &node
        ),
        async {
            let current = next_frame(&mut rx).await;
            assert!(current["header_value"].as_str().unwrap() == latest.as_str());
            acknowledge(&state, &node, &current, false);
        }
    );
    result.unwrap();
    acknowledge(&state, &node, &held, false);
    older.await.unwrap().unwrap();
    let saved = row(&state, &svc).await;
    assert!(!saved.get_bool("pending").unwrap());
    assert_eq!(
        saved.get_i64("acknowledged_generation").unwrap(),
        saved.get_i64("generation").unwrap()
    );
}

#[tokio::test]
async fn node_credential_order_2472_scoped_replacement_cannot_reuse_an_older_broader_pending_intent()
 {
    let (state, key, svc) = fixture().await;
    let first = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let latest = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &first).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let held = next_frame(&mut rx).await;
            delivery::with_actor(
                key.user_id.clone(),
                ActorScope {
                    allow_all_nodes: false,
                    allowed_node_ids: vec![],
                    allow_all_services: false,
                    allowed_service_ids: vec![],
                },
                rotate(&state, &key, &latest),
            )
            .await;
            acknowledge(&state, node, &held, false);
        }
    );
    result.unwrap();
    let saved = row(&state, &svc).await;
    assert!(saved.get_bool("suppressed").unwrap());
    assert!(!saved.get_bool("pending").unwrap());
    delivery::retry_node(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        node,
    )
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn node_credential_order_2472_route_removal_and_rebind_reuse_the_slot_and_increase_generation()
 {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    let initial = row(&state, &svc).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .update_one(
            doc! {"_id": &svc.id},
            doc! {"$set": {"node_id": bson::Bson::Null}},
        )
        .await
        .unwrap();
    let removed = row(&state, &svc).await;
    assert!(removed.get_i64("generation").unwrap() > initial.get_i64("generation").unwrap());
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_remove");
            assert!(frame["credential_id"].as_str().unwrap() == initial.get_str("_id").unwrap());
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .update_one(doc! {"_id": &svc.id}, doc! {"$set": {"node_id": node}})
        .await
        .unwrap();
    let rebound = row(&state, &svc).await;
    assert!(rebound.get_i64("generation").unwrap() > removed.get_i64("generation").unwrap());
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_update");
            assert!(frame["credential_id"].as_str().unwrap() == initial.get_str("_id").unwrap());
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
}

#[tokio::test]
async fn node_credential_order_2472_operational_usage_touch_remains_one_command_without_delivery_work()
 {
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
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "node_push_usage_counts",
        handler,
    )
    .await;
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(USER_API_KEYS)
        .insert_one(doc! {"_id": &id})
        .await
        .unwrap();
    enabled.store(true, Ordering::SeqCst);
    crate::services::service_history::mutation::touch_credential_usage(&db, &id)
        .await
        .unwrap();
    enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        *counts.lock().unwrap(),
        BTreeMap::from([("update".to_owned(), 1)])
    );
    assert_eq!(
        db.collection::<Document>(delivery::COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    eprintln!("operational usage touch commands: total=1 by_name={{update: 1}}; delivery work=0");
}

#[tokio::test]
async fn node_credential_order_2472_deleted_node_retires_retry_without_claiming_receipt() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    state
        .db
        .collection::<Document>("nodes")
        .update_one(doc! {"_id": node}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    delivery::retry_node(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        node,
    )
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
    let saved = row(&state, &svc).await;
    assert!(!saved.get_bool("pending").unwrap());
    assert_eq!(saved.get_i64("acknowledged_generation").unwrap(), 0);
}

#[tokio::test]
async fn node_credential_order_2472_local_only_promotion_and_cleanup_never_remove_node_material() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    crate::services::user_api_key_service::activate_node_managed_api_key(
        &state.db,
        &key.user_id,
        &key.id,
    )
    .await
    .unwrap();
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .update_one(
            doc! {"_id": &svc.id},
            doc! {"$set": {"auth_method": "none"}},
        )
        .await
        .unwrap();
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .update_one(
            doc! {"_id": &svc.id},
            doc! {"$set": {"api_key_id": bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
        .delete_one(doc! {"_id": &key.id})
        .await
        .unwrap();
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .delete_one(doc! {"_id": &svc.id})
        .await
        .unwrap();
    delivery::retry_node(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        node,
    )
    .await
    .unwrap();
    assert!(rx.try_recv().is_err());
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
}

#[tokio::test]
async fn node_credential_order_2472_no_key_route_does_not_create_an_unsolicited_erase_placeholder()
{
    let (state, _key, mut svc) = fixture().await;
    svc.id = uuid::Uuid::new_v4().to_string();
    svc.slug = "local-only-fixture".into();
    svc.api_key_id = None;
    svc.auth_method = "none".into();
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .insert_one(&svc)
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>(delivery::COLLECTION)
            .count_documents(doc! {
                "node_id": &svc.node_id, "service_slug": &svc.slug,
            })
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn node_credential_order_2472_strict_material_commit_gates_retry_before_preparation() {
    let (state, key, svc) = fixture().await;
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    delivery::with_prepared_actor(
        key.user_id.clone(),
        ActorScope::unrestricted(),
        rotate(&state, &key, &value),
    )
    .await;
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    delivery::retry_node(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        node,
    )
    .await
    .unwrap();
    assert!(
        rx.try_recv().is_err(),
        "intermediate routing/auth state escaped preparation"
    );
    let before = row(&state, &svc).await;
    // The writer dies between saving the material and reserving its strict push.
    state
        .db
        .collection::<Document>(delivery::COLLECTION)
        .update_one(
            doc! {"node_id": node},
            doc! {"$set": {"preparing_until": bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert!(frame["header_value"].as_str().unwrap() == value.as_str());
            assert!(
                frame["credential_generation"].as_i64().unwrap()
                    > before.get_i64("generation").unwrap()
            );
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
}

#[tokio::test]
async fn node_credential_order_2472_inactive_sibling_cannot_redirect_or_erase_active_delivery() {
    let (state, key, svc) = fixture().await;
    let mut inactive = svc.clone();
    inactive.id = uuid::Uuid::new_v4().to_string();
    inactive.is_active = false;
    state
        .db
        .collection::<UserService>(USER_SERVICES)
        .insert_one(&inactive)
        .await
        .unwrap();
    let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    rotate(&state, &key, &value).await;
    assert!(row(&state, &svc).await.get_str("service_id").unwrap() == svc.id);
    crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
        .delete_one(doc! {"_id": &inactive.id})
        .await
        .unwrap();
    assert!(row(&state, &svc).await.get_str("service_id").unwrap() == svc.id);
    let node = svc.node_id.as_deref().unwrap();
    let mut rx = attach(&state, node).await;
    let (result, ()) = tokio::join!(
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node
        ),
        async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_update");
            assert!(frame["header_value"].as_str().unwrap() == value.as_str());
            acknowledge(&state, node, &frame, false);
        }
    );
    result.unwrap();
    assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
}

mod retry_policy {
    use super::*;

    async fn pass(state: &crate::AppState) {
        delivery::run_pass(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &runner(),
            &crate::services::shutdown::Runtime::default(),
        )
        .await
        .unwrap();
    }

    async fn make_due(state: &crate::AppState, svc: &UserService) {
        // Advance the persisted retry clock without altering parking or claims.
        state
            .db
            .collection::<Document>(delivery::COLLECTION)
            .update_one(
                doc! {"node_id": &svc.node_id, "service_slug": &svc.slug},
                doc! {"$set": {"retry_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }

    async fn assert_parked(state: &crate::AppState, svc: &UserService, reason: &str) {
        let parked = row(state, svc).await;
        assert!(
            !parked.get_bool("pending").unwrap(),
            "unresolvable delivery remains due"
        );
        assert_eq!(parked.get_str("parked_reason").unwrap(), reason);
        make_due(state, svc).await;
        let before = row(state, svc).await;
        for _ in 0..3 {
            pass(state).await;
        }
        assert!(
            row(state, svc).await == before,
            "a parked slot was attempted again"
        );
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            svc.node_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        assert!(
            row(state, svc).await == before,
            "reconnect incorrectly woke source-disabled work"
        );
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc1_legacy_send_once_per_change_and_reconnect() {
        let (state, key, svc) = fixture().await;
        let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
        rotate(&state, &key, &value).await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        state
            .node_ws_manager
            .record_capabilities(node, &NodeCapabilitiesMsg::default());
        let mut sends = 0;
        for _ in 0..4 {
            make_due(&state, &svc).await;
            pass(&state).await;
            while rx.try_recv().is_ok() {
                sends += 1;
            }
        }
        assert_eq!(
            sends, 1,
            "legacy node is repeatedly sent unchanged material"
        );
        let sent = row(&state, &svc).await;
        assert!(!sent.get_bool("pending").unwrap());
        assert_eq!(sent.get_i64("acknowledged_generation").unwrap(), 0);
        assert_eq!(
            sent.get_i64("sent_unconfirmed_generation").unwrap(),
            sent.get_i64("generation").unwrap()
        );
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node,
        )
        .await
        .unwrap();
        let _ = next_frame(&mut rx).await;
        for _ in 0..3 {
            make_due(&state, &svc).await;
            pass(&state).await;
        }
        assert!(
            rx.try_recv().is_err(),
            "reconnect created a legacy resend loop"
        );
        rotate(&state, &key, &value).await;
        pass(&state).await;
        let _ = next_frame(&mut rx).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc1_unsupported_removal_parks_until_upgraded_reconnect() {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        state
            .node_ws_manager
            .record_capabilities(node, &NodeCapabilitiesMsg::default());
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"node_id": bson::Bson::Null}},
            )
            .await
            .unwrap();
        pass(&state).await;
        let parked = row(&state, &svc).await;
        assert!(
            !parked.get_bool("pending").unwrap(),
            "unsupported removal remains on the timer"
        );
        assert_eq!(
            parked.get_str("parked_reason").unwrap(),
            "acknowledgement_unsupported"
        );
        for _ in 0..3 {
            make_due(&state, &svc).await;
            pass(&state).await;
        }
        assert!(rx.try_recv().is_err());
        state.node_ws_manager.record_capabilities(
            node,
            &NodeCapabilitiesMsg {
                credential_ack_correlation: true,
                ..Default::default()
            },
        );
        let (result, ()) = tokio::join!(
            delivery::retry_node(
                &state.db,
                &state.encryption_keys,
                &state.node_ws_manager,
                node
            ),
            async {
                let frame = next_frame(&mut rx).await;
                assert_eq!(frame["type"], "credential_remove");
                assert!(
                    row(&state, &svc).await.get_bool("pending").unwrap(),
                    "queue success was treated as an acknowledgement"
                );
                acknowledge(&state, node, &frame, false);
            }
        );
        result.unwrap();
        let saved = row(&state, &svc).await;
        assert!(!saved.get_bool("pending").unwrap());
        assert_eq!(
            saved.get_i64("acknowledged_generation").unwrap(),
            saved.get_i64("generation").unwrap()
        );
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc2_disabled_service_parks_and_enable_rearms() {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(doc! {"_id": &svc.id}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        pass(&state).await;
        assert_parked(&state, &svc, "service_disabled").await;
        assert!(rx.try_recv().is_err());
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(doc! {"_id": &svc.id}, doc! {"$set": {"is_active": true}})
            .await
            .unwrap();
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            acknowledge(&state, node, &frame, false);
        });
        assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc2_inactive_key_parks_and_status_change_rearms() {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": {"status": "expired"}})
            .await
            .unwrap();
        pass(&state).await;
        assert_parked(&state, &svc, "key_inactive").await;
        assert!(rx.try_recv().is_err());
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": {"status": "active"}})
            .await
            .unwrap();
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            acknowledge(&state, node, &frame, false);
        });
        assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc2_status_reactivation_rearms_existing_parked_intent() {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": {"status": "expired"}})
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(delivery::COLLECTION)
            .update_one(
                doc! {"node_id": &svc.node_id},
                doc! {"$set": {"pending": false, "parked_reason": "key_inactive"}},
            )
            .await
            .unwrap();
        let before = row(&state, &svc).await;
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": {"status": "active"}})
            .await
            .unwrap();
        let after = row(&state, &svc).await;
        assert!(
            after.get_bool("pending").unwrap(),
            "status-only reactivation did not re-arm delivery"
        );
        assert!(after.get_i64("generation").unwrap() > before.get_i64("generation").unwrap());
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc3_offline_never_decrypts_and_reconnect_resets_backoff() {
        use std::sync::atomic::Ordering::SeqCst;
        let (mut state, key, svc) = fixture().await;
        let provider = provider(&mut state, false, false);
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        let before = provider.unwraps.load(SeqCst);
        let mut delays = Vec::new();
        for _ in 0..7 {
            make_due(&state, &svc).await;
            pass(&state).await;
            let saved = row(&state, &svc).await;
            delays.push(
                (saved.get_datetime("retry_at").unwrap().to_chrono() - chrono::Utc::now())
                    .num_seconds(),
            );
        }
        assert_eq!(
            provider.unwraps.load(SeqCst),
            before,
            "offline passes decrypted material"
        );
        for (actual, expected) in delays.into_iter().zip([30, 60, 120, 240, 480, 600, 600]) {
            assert!(
                (expected - 3..=expected).contains(&actual),
                "unexpected offline retry delay: {actual}"
            );
        }
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        let (result, ()) = tokio::join!(
            delivery::retry_node(
                &state.db,
                &state.encryption_keys,
                &state.node_ws_manager,
                node
            ),
            async {
                let frame = next_frame(&mut rx).await;
                acknowledge(&state, node, &frame, false);
            }
        );
        result.unwrap();
        let saved = row(&state, &svc).await;
        assert!(!saved.get_bool("pending").unwrap());
        assert_eq!(saved.get_i64("retry_attempts").unwrap(), 0);
        assert_eq!(provider.unwraps.load(SeqCst), before + 1);
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc4_idle_pass_is_one_indexed_read() {
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
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "node_delivery_idle",
            handler,
        )
        .await;
        delivery::ensure_indexes(&db).await.unwrap();
        crate::services::coordination_service::ensure_indexes(&db)
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        enabled.store(true, Ordering::SeqCst);
        pass(&state).await;
        enabled.store(false, Ordering::SeqCst);
        let captured = counts.lock().unwrap().clone();
        eprintln!(
            "idle delivery pass commands: total={} by_name={captured:?}",
            captured.values().sum::<usize>()
        );
        assert_eq!(captured, BTreeMap::from([("find".to_owned(), 1)]));
        let plan = db
            .run_command(doc! {"explain": {"find": delivery::COLLECTION,
            "filter": {"pending": true, "retry_at": {"$lte": bson::DateTime::now()}},
            "sort": {"retry_at": 1}, "limit": 64}, "verbosity": "queryPlanner"})
            .await
            .unwrap();
        let plan = plan
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap()
            .to_string();
        assert!(
            plan.contains("IXSCAN"),
            "due-work query did not use an index"
        );
        assert!(!plan.contains("COLLSCAN"));
    }

    use crate::crypto::key_provider::KeyProvider;

    #[derive(Debug)]
    struct ControlledProvider {
        inner: crate::crypto::local_key_provider::LocalKeyProvider,
        fail: std::sync::atomic::AtomicBool,
        hold: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Semaphore,
        unwraps: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl KeyProvider for ControlledProvider {
        async fn wrap_dek(
            &self,
            bytes: &[u8],
        ) -> AppResult<crate::crypto::key_provider::WrappedKey> {
            self.inner.wrap_dek(bytes).await
        }
        async fn unwrap_dek(
            &self,
            wrapped: &crate::crypto::key_provider::WrappedKey,
        ) -> AppResult<Zeroizing<Vec<u8>>> {
            use std::sync::atomic::Ordering::SeqCst;
            self.unwraps.fetch_add(1, SeqCst);
            if self.hold.load(SeqCst) {
                self.entered.notify_one();
                tokio::time::timeout(Duration::from_secs(10), self.release.acquire())
                    .await
                    .map_err(|_| AppError::Internal("test provider barrier timed out".into()))?
                    .map_err(|_| AppError::Internal("test provider barrier closed".into()))?
                    .forget();
            }
            if self.fail.load(SeqCst) {
                return Err(AppError::Internal("test provider unavailable".into()));
            }
            self.inner.unwrap_dek(wrapped).await
        }
        fn current_key_id(&self) -> u8 {
            self.inner.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.inner.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            self.inner.has_previous_key()
        }
    }

    fn provider(state: &mut crate::AppState, fail: bool, hold: bool) -> Arc<ControlledProvider> {
        let provider = Arc::new(ControlledProvider {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::from_config(&state.config),
            fail: fail.into(),
            hold: hold.into(),
            entered: Default::default(),
            release: tokio::sync::Semaphore::new(0),
            unwraps: 0.into(),
        });
        state.encryption_keys = Arc::new(crate::crypto::aes::EncryptionKeys::with_provider(
            provider.clone(),
        ));
        provider
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc3_provider_errors_back_off_and_new_generation_resets() {
        use std::sync::atomic::Ordering::SeqCst;
        let (mut state, key, svc) = fixture().await;
        let provider = provider(&mut state, false, false);
        let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
        rotate(&state, &key, &value).await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        let before = provider.unwraps.load(SeqCst);
        provider.fail.store(true, SeqCst);
        for expected in [5, 10, 20, 40, 60, 60] {
            make_due(&state, &svc).await;
            pass(&state).await;
            let saved = row(&state, &svc).await;
            let delay = (saved.get_datetime("retry_at").unwrap().to_chrono() - chrono::Utc::now())
                .num_seconds();
            assert!(
                (expected - 3..=expected).contains(&delay),
                "unexpected transient retry delay: {delay}"
            );
            assert!(rx.try_recv().is_err());
        }
        assert_eq!(provider.unwraps.load(SeqCst), before + 6);
        let attempts = provider.unwraps.load(SeqCst);
        pass(&state).await;
        assert_eq!(
            provider.unwraps.load(SeqCst),
            attempts,
            "backoff was bypassed"
        );
        rotate(&state, &key, &value).await;
        assert_eq!(
            row(&state, &svc).await.get_i64("retry_attempts").unwrap(),
            0
        );
        provider.fail.store(false, SeqCst);
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            acknowledge(&state, node, &frame, false);
        });
        assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc3_late_failure_cannot_delay_newer_generation() {
        use std::sync::atomic::Ordering::SeqCst;
        let (mut state, key, svc) = fixture().await;
        let writer = state.clone();
        let provider = provider(&mut state, true, true);
        let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
        rotate(&writer, &key, &value).await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        let old_state = state.clone();
        let old_svc = svc.clone();
        let older = tokio::spawn(async move {
            delivery::deliver_slot(
                &old_state.db,
                &old_state.encryption_keys,
                &old_state.node_ws_manager,
                old_svc.node_id.as_deref().unwrap(),
                &old_svc.slug,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), provider.entered.notified())
            .await
            .unwrap();
        rotate(&writer, &key, &value).await;
        let rearmed = row(&state, &svc).await;
        provider.release.add_permits(1);
        assert!(older.await.unwrap().is_err());
        let after = row(&state, &svc).await;
        assert_eq!(after.get_i64("retry_attempts").unwrap(), 0);
        assert_eq!(
            after.get_datetime("retry_at").unwrap(),
            rearmed.get_datetime("retry_at").unwrap()
        );
        assert!(after.get_bool("pending").unwrap());
        provider.fail.store(false, SeqCst);
        provider.hold.store(false, SeqCst);
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            acknowledge(&state, node, &frame, false);
        });
        assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    }

    #[tokio::test]
    async fn node_credential_order_2472_nc1_late_legacy_send_cannot_erase_concurrent_reconnect() {
        use std::sync::atomic::Ordering::SeqCst;
        let (mut state, key, svc) = fixture().await;
        let writer = state.clone();
        let provider = provider(&mut state, false, true);
        rotate(
            &writer,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        state
            .node_ws_manager
            .record_capabilities(node, &NodeCapabilitiesMsg::default());
        let old_state = state.clone();
        let old_svc = svc.clone();
        let older = tokio::spawn(async move {
            delivery::deliver_slot(
                &old_state.db,
                &old_state.encryption_keys,
                &old_state.node_ws_manager,
                old_svc.node_id.as_deref().unwrap(),
                &old_svc.slug,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), provider.entered.notified())
            .await
            .unwrap();
        delivery::retry_node(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            node,
        )
        .await
        .unwrap();
        provider.hold.store(false, SeqCst);
        provider.release.add_permits(1);
        older.await.unwrap().unwrap();
        let _ = next_frame(&mut rx).await;
        assert!(
            row(&state, &svc).await.get_bool("pending").unwrap(),
            "old queue result erased the reconnect wake-up"
        );
        pass(&state).await;
        let _ = next_frame(&mut rx).await;
        assert!(!row(&state, &svc).await.get_bool("pending").unwrap());
    }

    #[tokio::test]
    async fn node_credential_order_2472_decision_platform_skips_material_and_switch_queues_remove()
    {
        let (state, key, svc) = fixture().await;
        state
            .db
            .collection::<Document>(USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"credential_binding": "platform"}},
            )
            .await
            .unwrap();
        let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
        rotate(&state, &key, &value).await;
        assert_eq!(
            state
                .db
                .collection::<Document>(delivery::COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"credential_binding": bson::Bson::Null}},
            )
            .await
            .unwrap();
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_update");
            acknowledge(&state, node, &frame, false);
        });
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"credential_binding": "platform"}},
            )
            .await
            .unwrap();
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_remove");
            acknowledge(&state, node, &frame, false);
        });
    }

    #[tokio::test]
    async fn node_credential_order_2472_decision_ssh_never_creates_ordinary_intent() {
        let (state, key, svc) = fixture().await;
        state
            .db
            .collection::<Document>(USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"service_type": "ssh"}},
            )
            .await
            .unwrap();
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"auth_method": "none"}},
            )
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .collection::<Document>(delivery::COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn node_credential_order_2472_decision_provider_linked_actor_suppresses_but_refresh_rearms()
     {
        let (state, key, svc) = fixture().await;
        let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
        rotate(&state, &key, &value).await;
        state
            .db
            .collection::<Document>(USER_API_KEYS)
            .update_one(
                doc! {"_id": &key.id},
                doc! {"$set": {"provider_config_id": uuid::Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
        delivery::with_actor(
            key.user_id.clone(),
            ActorScope::unrestricted(),
            rotate(&state, &key, &value),
        )
        .await;
        let suppressed = row(&state, &svc).await;
        assert!(suppressed.get_bool("suppressed").unwrap());
        assert!(!suppressed.get_bool("pending").unwrap());
        let ciphertext = crate::services::credential_record_service::encrypt(
            &state.encryption_keys,
            USER_API_KEYS,
            &key.id,
            &key.user_id,
            "credential_encrypted",
            value.as_bytes(),
        )
        .await
        .unwrap();
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS).update_one(doc! {"_id": &key.id},
            doc! {"$set": {"credential_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: ciphertext}}})
            .routine_refresh().await.unwrap();
        let rearmed = row(&state, &svc).await;
        assert!(!rearmed.get_bool("suppressed").unwrap());
        assert!(rearmed.get_bool("pending").unwrap());
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            acknowledge(&state, node, &frame, false);
        });
    }

    #[tokio::test]
    async fn node_credential_order_2472_decision_node_and_service_scope_each_suppress() {
        for scope in [
            ActorScope {
                allow_all_nodes: false,
                allowed_node_ids: vec![],
                allow_all_services: true,
                allowed_service_ids: vec![],
            },
            ActorScope {
                allow_all_nodes: true,
                allowed_node_ids: vec![],
                allow_all_services: false,
                allowed_service_ids: vec![],
            },
        ] {
            let (state, key, svc) = fixture().await;
            let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
            rotate(&state, &key, &value).await;
            let node = svc.node_id.as_deref().unwrap();
            let mut rx = attach(&state, node).await;
            delivery::with_actor(key.user_id.clone(), scope, rotate(&state, &key, &value)).await;
            pass(&state).await;
            delivery::retry_node(
                &state.db,
                &state.encryption_keys,
                &state.node_ws_manager,
                node,
            )
            .await
            .unwrap();
            assert!(rx.try_recv().is_err());
            let saved = row(&state, &svc).await;
            assert!(saved.get_bool("suppressed").unwrap());
            assert!(!saved.get_bool("pending").unwrap());
        }
    }

    #[tokio::test]
    async fn node_credential_order_2472_decision_existing_no_key_intent_can_reconcile_explicit_no_auth()
     {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(
                doc! {"_id": &svc.id},
                doc! {"$set": {"api_key_id": bson::Bson::Null, "auth_method": "none"}},
            )
            .await
            .unwrap();
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["injection_method"], "none");
            acknowledge(&state, node, &frame, false);
        });
        // With no old key reference, an existing explicit slot still authorizes
        // reconciling a later route edit; a fresh no-key route does not.
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .update_one(doc! {"_id": &svc.id}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        pass(&state).await;
        assert_eq!(
            row(&state, &svc).await.get_str("parked_reason").unwrap(),
            "service_disabled"
        );
    }

    #[tokio::test]
    async fn node_credential_order_2472_decision_deleted_key_cleanup_preserves_queued_removal() {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
            .delete_one(doc! {"_id": &key.id})
            .await
            .unwrap();
        let before = row(&state, &svc).await;
        crate::services::service_history::collection::<UserService>(&state.db, USER_SERVICES)
            .delete_one(doc! {"_id": &svc.id})
            .await
            .unwrap();
        assert!(
            row(&state, &svc).await == before,
            "binding cleanup redirected the queued removal"
        );
        let node = svc.node_id.as_deref().unwrap();
        let mut rx = attach(&state, node).await;
        tokio::join!(pass(&state), async {
            let frame = next_frame(&mut rx).await;
            assert_eq!(frame["type"], "credential_remove");
            acknowledge(&state, node, &frame, false);
        });
    }
    #[tokio::test]
    async fn node_credential_order_2472_decision_metadata_edits_do_not_rearm_delivery() {
        let (state, key, svc) = fixture().await;
        rotate(
            &state,
            &key,
            &Zeroizing::new(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        let before = row(&state, &svc).await;
        crate::services::service_history::collection::<UserApiKey>(&state.db, USER_API_KEYS)
            .update_one(
                doc! {"_id": &key.id},
                doc! {"$set": {"label": "renamed fixture"}},
            )
            .await
            .unwrap();
        assert!(
            row(&state, &svc).await == before,
            "label edit changed delivery intent"
        );
        crate::services::service_history::collection::<UserEndpoint>(&state.db, USER_ENDPOINTS)
            .update_one(
                doc! {"_id": &svc.endpoint_id},
                doc! {"$set": {"label": "renamed endpoint"}},
            )
            .await
            .unwrap();
        assert!(
            row(&state, &svc).await == before,
            "endpoint metadata changed delivery intent"
        );
    }
}
