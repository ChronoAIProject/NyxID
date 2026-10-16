use super::*;
use crate::models::{assistant_acknowledgement::AssistantAcknowledgement, oracle_pool::OraclePool};
use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

#[tokio::test]
async fn deleted_owner_selection_still_blocks_other_exact_same_catalog_calls() {
    let f = orchestrator_fixture("deleted_exact_choice").await;
    let c = catalog(&f).await;
    let a = connection(&f, &f.owner, "a", Some(&c)).await;
    let b = connection(&f, &f.owner, "b", Some(&c)).await;
    let mut selection = scope(&f.chat).unwrap();
    selection.extend(doc! {
        "_id":Uuid::new_v4().to_string(),"kind":"selection","connection_id":&a,
        "selection_source":"owner","owner_id":&f.owner,"catalog_service_id":&c,
        "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10)),
    });
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .insert_one(selection)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .delete_one(doc! {"_id":&a})
        .await
        .unwrap();
    assert_eq!(
        gate(
            &f,
            &b,
            &f.owner,
            &json!({"kind":"inference","provider":c,"model":"m"}),
            None
        )
        .await
        .refusal
        .unwrap()["error"],
        "service_selection_required"
    );
}

#[tokio::test]
async fn concurrent_outages_are_deduplicated_and_atomically_bounded() {
    let f = orchestrator_fixture("atomic_outage_bound").await;
    let id = connection(&f, &f.owner, "service", None).await;
    let decision = gate(&f, &id, &f.owner, &json!({"path":"/read"}), None).await;
    let admission = decision.admission.as_ref().unwrap();
    let attempts =
        (0..64).map(|_| admission.record_unavailable(&f.state.db, &f.chat, "pre_dispatch"));
    for result in futures::future::join_all(attempts).await {
        result.unwrap();
    }
    let bucket = f
        .state
        .db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"kind":"unavailable_bucket"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bucket.get_array("entries").unwrap().len(), 1);
    let mut admissions = Vec::new();
    for index in 0..48 {
        admissions.push(
            gate(
                &f,
                &id,
                &f.owner,
                &json!({"path":format!("/read/{index}")}),
                None,
            )
            .await
            .admission
            .unwrap(),
        );
    }
    for result in futures::future::join_all(
        admissions
            .iter()
            .map(|admission| admission.record_unavailable(&f.state.db, &f.chat, "pre_dispatch")),
    )
    .await
    {
        result.unwrap();
    }
    let bucket = f
        .state
        .db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"kind":"unavailable_bucket"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bucket.get_array("entries").unwrap().len(), MAX_EVIDENCE);
}

#[tokio::test]
async fn uncertain_write_without_prior_outage_does_not_admit_lower_connection() {
    let f = orchestrator_fixture("unknown_write_no_evidence").await;
    let c = catalog(&f).await;
    let a = connection(&f, &f.owner, "a", Some(&c)).await;
    let b = connection(&f, &f.owner, "b", Some(&c)).await;
    f.state.db.collection::<Document>("service_preferences").insert_one(doc! {"_id":&f.owner,"ordered":[&a,&b],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let op =
        json!({"kind":"inference","method":"POST","path":"/v1/responses","model":"m","provider":c});
    let admission = gate(&f, &a, &f.owner, &op, None).await.admission.unwrap();
    admission
        .transport_failure(
            &f.state.db,
            &f.chat,
            &crate::services::proxy_service::ForwardRequestError::OutcomeUnknown,
        )
        .await
        .unwrap();
    assert_eq!(
        gate(&f, &b, &f.owner, &op, None).await.refusal.unwrap()["error"],
        "service_priority_required"
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(doc! {"kind":"unavailable_bucket"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn held_admission_expires_with_elapsed_key_or_current_turn_stop() {
    for expire in [false, true] {
        let f = orchestrator_fixture("final_live_authority").await;
        let id = connection(&f, &f.owner, "a", None).await;
        if expire {
            f.state.db.collection::<Document>("api_keys").update_one(doc! {"_id":&f.chat.api_key_id},
                doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(3))}}).await.unwrap();
        }
        let admission = gate(&f, &id, &f.owner, &json!({"path":"/read"}), None)
            .await
            .admission
            .unwrap();
        if expire {
            tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
        } else {
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await
                .unwrap();
        }
        assert!(admission.revalidate(&f.state.db, &id, None).await.is_err());
    }
}

#[tokio::test]
async fn scope_ineligible_preferred_candidate_does_not_block_exact_allowed_operation() {
    let f = orchestrator_fixture("operation_scope_candidates").await;
    let c = catalog(&f).await;
    let a = connection(&f, &f.owner, "a", Some(&c)).await;
    let b = connection(&f, &f.owner, "b", Some(&c)).await;
    f.state.db.collection::<Document>("service_preferences").insert_one(doc! {"_id":&f.owner,"ordered":[&a,&b],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let mut scopes = Document::new();
    for (id, path) in [(&a, "/foo"), (&b, "/bar")] {
        scopes.insert(id,doc! {"revision":1_i64,"catalog_service_id":&c,"operations":[{"rule":{"method":"GET","path_template":path}}]});
    }
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allowed_service_ids":[&a,&b],"assistant_operation_scopes":scopes}},
        )
        .await
        .unwrap();
    assert!(
        gate(
            &f,
            &b,
            &f.owner,
            &json!({"kind":"http","method":"GET","path":"/bar"}),
            None
        )
        .await
        .refusal
        .is_none()
    );
}

async fn pool(f: &Fixture, owner: &str, slug: &str) -> String {
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>("oracle_pools")
        .insert_one(doc! {"_id":&id,"user_id":owner,"slug":slug,"name":slug,
        "visibility":"platform","worker_token_hash":"test","max_workers":2,"max_queue_length":10,
        "per_user_max_inflight":10,"task_timeout_secs":300_i64,"is_active":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()})
        .await
        .unwrap();
    // Assert fixture uses the production schema.
    f.state
        .db
        .collection::<OraclePool>("oracle_pools")
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    id
}

#[tokio::test]
async fn oracle_http_person_platform_auto_proceeds_org_auto_asks_and_full_access_proceeds() {
    let f = orchestrator_fixture("oracle_owner_types").await;
    mode(&f, AccessMode::Nyxmode).await;
    let person = Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_utils::test_user(&person, UserType::Person))
        .await
        .unwrap();
    let org = org(&f).await;
    let personal_pool = pool(&f, &person, "person-public").await;
    let org_pool = pool(&f, &org, "org-public").await;
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    let body = || serde_json::from_value(json!({"prompt":"question"})).unwrap();
    let response = crate::handlers::oracle_tasks::submit_task(
        State(f.state.clone()),
        auth.clone(),
        Path(personal_pool),
        Json(body()),
    )
    .await
    .unwrap()
    .into_response();
    assert!(response.status().is_success());
    assert!(
        crate::handlers::oracle_tasks::submit_task(
            State(f.state.clone()),
            auth.clone(),
            Path(org_pool.clone()),
            Json(body())
        )
        .await
        .is_err()
    );
    let cards = f
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .count_documents(doc! {"conversation_id":&f.row.id,"status":"pending"})
        .await
        .unwrap();
    assert_eq!(cards, 1);
    mode(&f, AccessMode::Autopilot).await;
    let response = crate::handlers::oracle_tasks::submit_task(
        State(f.state.clone()),
        auth,
        Path(org_pool),
        Json(body()),
    )
    .await
    .unwrap()
    .into_response();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn denied_exact_card_has_no_retry_or_continue_directive_and_no_outage() {
    let f = orchestrator_fixture("denied_exact_note").await;
    mode(&f, AccessMode::Nyxmode).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "org", None).await;
    let op = json!({"method":"GET","path":"/read"});
    let refusal = gate(&f, &id, &owner, &op, None).await.refusal.unwrap();
    let ack = refusal["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.row.id, ack, false)
        .await
        .unwrap();
    let row = f
        .state
        .db
        .collection::<AssistantAcknowledgement>("assistant_acknowledgements")
        .find_one(doc! {"_id":ack})
        .await
        .unwrap()
        .unwrap();
    let note = acks::decisions_note(&[row]);
    assert!(note.contains("not authorized"));
    assert!(!note.contains("Retry"));
    assert!(!note.contains("continue"));
    assert_eq!(
        gate(&f, &id, &owner, &op, Some(ack)).await.refusal.unwrap()["error"],
        "acknowledgement_invalid"
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(doc! {"kind":"unavailable_bucket"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn model_grant_publication_cannot_bless_a_replacement_generation() {
    let f = orchestrator_fixture("grant_publication_drift").await;
    let owner = org(&f).await;
    let c = catalog(&f).await;
    let id = connection(&f, &owner, "org-model", Some(&c)).await;
    mode(&f, AccessMode::Nyxmode).await;
    let held = target(&f, &id).await;
    let operation = json!({"kind":"raw_inference","method":"POST","path":"/v1/chat/completions", "provider":c,"model":"m","input_admission":"original"});
    let ack = allow(
        &f,
        gate(&f, &id, &owner, &operation, None)
            .await
            .refusal
            .unwrap(),
    )
    .await;
    let decision = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        &id,
        &owner,
        &operation,
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        Some(&ack),
        "Model use",
        false,
        Some(&held),
        false,
        None,
    )
    .await
    .unwrap();
    let admission = decision.admission.unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$inc":{"state_version":1_i64}})
        .await
        .unwrap();
    assert!(
        super::super::http::persist_model_consent(
            &f.state.db,
            &f.chat,
            &admission,
            &held,
            &operation
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(doc! {"kind":"model_consent"})
            .await
            .unwrap(),
        0
    );
    assert!(
        gate(&f, &id, &owner, &operation, None)
            .await
            .refusal
            .is_some()
    );
}

#[tokio::test]
async fn held_node_route_rejects_config_replacement_and_removal() {
    let f = orchestrator_fixture("held_node_config").await;
    let id = connection(&f, &f.owner, "node-service", None).await;
    let held = target(&f, &id).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$set":{"node_id":"old-node"}})
        .await
        .unwrap();
    let route = crate::services::node_routing_service::NodeRoute {
        node_id: "old-node".into(),
        fallback_node_ids: vec![],
    };
    verify_node_route(&f.state.db, &f.chat, &id, Some(&route), &held)
        .await
        .unwrap();
    for replacement in [bson::Bson::String("new-node".into()), bson::Bson::Null] {
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(doc! {"_id":&id}, doc! {"$set":{"node_id":replacement}})
            .await
            .unwrap();
        assert!(
            verify_node_route(&f.state.db, &f.chat, &id, Some(&route), &held)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn exact_upload_identity_hashes_actual_bytes_without_json_expansion() {
    let f = orchestrator_fixture("upload_exact_bytes").await;
    let id = connection(&f, &f.owner, "upload", None).await;
    let held = target(&f, &id).await;
    let bytes = vec![0xff; 1024 * 1024];
    let first = http_operation(&held, "POST", "/upload", None, &bytes);
    assert_eq!(first["body"].as_str().unwrap().len(), 64);
    let mut changed = bytes;
    changed[512] = 0;
    assert_ne!(
        first,
        http_operation(&held, "POST", "/upload", None, &changed)
    );
}

#[tokio::test]
async fn cached_model_consent_cannot_upgrade_an_autopilot_snapshot_to_auto() {
    let f = orchestrator_fixture("cached_model_mode_race").await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "model", None).await;
    mode(&f, AccessMode::Autopilot).await;
    let cached_binding = connection_binding(&f.state.db, &id, &f.chat.api_key_id)
        .await
        .unwrap();
    mode(&f, AccessMode::Nyxmode).await;
    let decision = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        &id,
        &owner,
        &json!({"kind":"raw_inference","model":"m"}),
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        None,
        "model",
        false,
        None,
        true,
        None,
    )
    .await
    .unwrap();
    assert!(
        super::super::http::validate_model_consent_snapshot(
            decision.admission.as_ref(),
            &cached_binding
        )
        .is_err()
    );
}
