//! PREPARED/UNEXECUTED actual turn-admission coverage and retention fences.
use super::*;

fn coverage(row: &crate::models::assistant_conversation::AssistantConversation) -> Document {
    doc! {"kind":"model_control_recorder_admission","user_id":&row.user_id,"conversation_id":&row.id,
    "api_key_id":&row.credential_api_key_id,"turn_id":&row.active_turn.as_ref().unwrap().turn_id}
}

#[tokio::test]
async fn actual_new_turn_recorder_coverage_is_atomic_revision_bound_and_never_backfilled() {
    let c = Controlled::new("p2_recorder_admission").await;
    let markers =
        c.f.state
            .db
            .collection::<Document>(super::super::super::assistant_service_execution::COLLECTION);
    // The real fixture calls begin_turn while issuance is still default-off.
    let marker = markers.find_one(coverage(&c.f.row)).await.unwrap().unwrap();
    assert_eq!(marker.get_str("revision").unwrap(), READINESS_CAPABILITY);
    assert!(!marker.contains_key("expires_at"));
    assert_eq!(
        uuid::Uuid::parse_str(marker.get_str("_id").unwrap())
            .unwrap()
            .get_version_num(),
        4
    );
    assert!(!marker.contains_key("input") && !marker.contains_key("tools"));
    let replicas =
        c.f.state
            .db
            .collection::<crate::models::security_replica::SecurityReplica>("security_replicas");
    let mut old = replicas
        .find_one(doc! {"_id":"test-process"})
        .await
        .unwrap()
        .unwrap();
    old.id = uuid::Uuid::new_v4().to_string();
    old.generation_id = old.id.clone();
    old.capabilities
        .retain(|capability| capability != READINESS_CAPABILITY);
    old.capabilities
        .push("assistant:model-control-protocol-v1".into());
    replicas.insert_one(&old).await.unwrap();
    let unready = crate::services::assistant_authority_tests::new_orchestrator(
        &c.f.state,
        &c.f.owner,
        "ordinary admission with old recorder writer",
    )
    .await;
    assert!(
        markers
            .find_one(coverage(&unready))
            .await
            .unwrap()
            .is_none()
    );
    replicas.delete_one(doc! {"_id":old.id}).await.unwrap();
    let chat = acks::for_key(
        &c.f.state.db,
        &c.f.owner,
        Some(&unready.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let token = issue(
        &c.f.state,
        IssueRequest {
            chat: &chat,
            runtime_catalog_id: &c.target.service.id,
            outer_body: &json!({"input":"ordinary admission with old recorder writer"}),
            negotiated_v1: true,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let (_, private, _) = persistence::authenticated(&c.f.state, &chat, &token.token, None)
        .await
        .unwrap();
    assert!(
        private.preconsent.ineligible,
        "current readiness cannot certify an earlier turn"
    );
    assert!(
        markers
            .find_one(coverage(&unready))
            .await
            .unwrap()
            .is_none()
    );
    let fresh = crate::services::assistant_authority_tests::new_orchestrator(
        &c.f.state,
        &c.f.owner,
        "new ready recorder admission",
    )
    .await;
    assert!(markers.find_one(coverage(&fresh)).await.unwrap().is_some());
    assert!(markers.find_one(doc! {"kind":"model_control_recorder_admission","conversation_id":&fresh.id,"api_key_id":&c.f.row.credential_api_key_id}).await.unwrap().is_none());
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn simultaneous_issue_reloads_under_fence_and_negative_marker_cannot_leave_pristine_sibling()
{
    for ordering in ["marker_first", "issue_first", "concurrent"] {
        let mut c = Controlled::configured("p2_recorder_concurrent_issue", false).await;
        persistence::ensure_indexes(&c.f.state.db).await.unwrap();
        c.f.state.config.assistant_model_control_v1_enabled = true;
        let marker = || {
            read_observation::unobserved_http(
                &c.f.state,
                &c.f.chat,
                Some(&c.connection),
                &c.target,
                "responses",
            )
        };
        if ordering == "marker_first" {
            marker().await.unwrap();
        }
        let body = json!({"input":"Please manage my account","store":true});
        let issue_one = || {
            issue(
                &c.f.state,
                IssueRequest {
                    chat: &c.f.chat,
                    runtime_catalog_id: &c.target.service.id,
                    outer_body: &body,
                    negotiated_v1: true,
                },
            )
        };
        let (entered, mut arrivals) = tokio::sync::mpsc::channel(2);
        let release = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let issuers = async {
            tokio::join!(
                issuance::BEFORE_ISSUE_TRANSACTION
                    .scope((entered.clone(), release.clone()), issue_one()),
                issuance::BEFORE_ISSUE_TRANSACTION.scope((entered, release.clone()), issue_one())
            )
        };
        let synchronize = async {
            for _ in 0..2 {
                tokio::time::timeout(std::time::Duration::from_secs(15), arrivals.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
            release.add_permits(2);
            if ordering == "concurrent" {
                marker().await.unwrap();
            }
        };
        let ((a, b), ()) = tokio::join!(issuers, synchronize);
        let a = a.unwrap().unwrap();
        let b = b.unwrap().unwrap();
        assert_eq!(a.token.expose(), b.token.expose());
        if ordering == "issue_first" {
            marker().await.unwrap();
        }
        let (root, private, _) = persistence::authenticated(&c.f.state, &c.f.chat, &a.token, None)
            .await
            .unwrap();
        assert!(private.preconsent.seen && private.preconsent.ineligible);
        assert_eq!(private.controls.len(), 1);
        assert_eq!(c.f.state.db.collection::<ModelControlRoot>(COLLECTION_NAME).count_documents(doc! {"user_id":&c.f.owner,"conversation_id":&c.f.row.id,"turn_ids":&c.f.chat.turn_id}).await.unwrap(),1);
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>(crate::services::assistant_service_execution::COLLECTION)
                .count_documents(read_observation::marker_scope(&c.f.chat).unwrap())
                .await
                .unwrap(),
            1
        );
        assert_eq!(root.phase, "unactivated");
        assert!(private.steps.is_empty());
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn negative_recorder_marker_has_no_ttl_and_real_conversation_erasure_removes_it() {
    let c = Controlled::new("p2_recorder_retention").await;
    read_observation::unobserved_http(
        &c.f.state,
        &c.f.chat,
        Some(&c.connection),
        &c.target,
        "responses",
    )
    .await
    .unwrap();
    let markers =
        c.f.state
            .db
            .collection::<Document>(crate::services::assistant_service_execution::COLLECTION);
    let marker = markers
        .find_one(read_observation::marker_scope(&c.f.chat).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!marker.contains_key("expires_at"));
    let mut other = c.f.chat.clone();
    other.api_key_id = uuid::Uuid::new_v4().to_string();
    assert!(
        markers
            .find_one(read_observation::marker_scope(&other).unwrap())
            .await
            .unwrap()
            .is_none()
    );
    other = c.f.chat.clone();
    other.turn_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(
        markers
            .find_one(read_observation::marker_scope(&other).unwrap())
            .await
            .unwrap()
            .is_none()
    );
    engine::delete(&c.f.state.db, &c.f.owner, &c.f.row.id)
        .await
        .unwrap();
    assert_eq!(
        markers
            .count_documents(doc! {"user_id":&c.f.owner,"conversation_id":&c.f.row.id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<ModelControlRoot>(COLLECTION_NAME)
            .count_documents(doc! {"user_id":&c.f.owner,"conversation_id":&c.f.row.id})
            .await
            .unwrap(),
        0
    );
    assert!(
        issue(
            &c.f.state,
            IssueRequest {
                chat: &c.f.chat,
                runtime_catalog_id: &c.target.service.id,
                outer_body: &json!({"input":"stale after erasure"}),
                negotiated_v1: true
            }
        )
        .await
        .is_err()
    );
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn allowed_continuation_never_adds_coverage_or_replaces_original_root() {
    let mut c = Controlled::new("p2_recorder_allow_lineage").await;
    let original = persistence::authenticated(&c.f.state, &c.f.chat, &c.token, None)
        .await
        .unwrap()
        .0;
    let marker =
        c.f.state
            .db
            .collection::<Document>(super::super::super::assistant_service_execution::COLLECTION)
            .find_one(coverage(&c.f.row))
            .await
            .unwrap()
            .unwrap();
    c.allow_resume().await;
    let (root, private, _) = persistence::authenticated(&c.f.state, &c.f.chat, &c.token, None)
        .await
        .unwrap();
    assert_eq!(root.id, original.id);
    assert_eq!(root.phase, "allowed");
    assert_eq!(private.controls.len(), 2);
    let markers =
        c.f.state
            .db
            .collection::<Document>(super::super::super::assistant_service_execution::COLLECTION);
    assert!(
        markers
            .find_one(coverage(&c.f.row))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        markers
            .count_documents(
                doc! {"kind":"model_control_recorder_admission","conversation_id":&c.f.row.id}
            )
            .await
            .unwrap(),
        1
    );
    let retained = markers
        .find_one(doc! {"_id":marker.get_str("_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained, marker);
    assert!(!retained.contains_key("expires_at"));
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn genuine_recorder_marker_write_failure_aborts_turn_admission_and_retry_starts_covered_turn()
{
    let c = Controlled::new("p2_recorder_marker_db_failure").await;
    let collection = crate::services::assistant_service_execution::COLLECTION;
    let before =
        c.f.state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&c.f.owner})
            .await
            .unwrap();
    // A genuine Mongo validator failure, rather than an injected return value.
    c.f.state.db.run_command(doc! {"collMod":collection,"validator":{"kind":{"$ne":"model_control_recorder_admission"}},"validationLevel":"strict"}).await.unwrap();
    let start = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: None,
        text: "ordinary initial admission must atomically retain coverage".into(),
        model: None,
        access_mode: None,
    };
    assert!(
        engine::begin_turn(
            &c.f.state.db,
            &c.f.owner,
            &start,
            &c.f.state.encryption_keys
        )
        .await
        .is_err()
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&c.f.owner})
            .await
            .unwrap(),
        before
    );
    c.f.state
        .db
        .run_command(doc! {"collMod":collection,"validator":{}})
        .await
        .unwrap();
    let row = engine::begin_turn(
        &c.f.state.db,
        &c.f.owner,
        &start,
        &c.f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(
        c.f.state
            .db
            .collection::<Document>(collection)
            .find_one(coverage(&row))
            .await
            .unwrap()
            .is_some()
    );
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn actual_owner_erasure_removes_non_ttl_recorder_markers_and_prevents_stale_recreation() {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let c = Controlled::new("p2_recorder_owner_erasure").await;
    read_observation::unobserved_http(
        &c.f.state,
        &c.f.chat,
        Some(&c.connection),
        &c.target,
        "responses",
    )
    .await
    .unwrap();
    let markers =
        c.f.state
            .db
            .collection::<Document>(crate::services::assistant_service_execution::COLLECTION);
    assert!(
        markers
            .find_one(coverage(&c.f.row))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        markers
            .find_one(read_observation::marker_scope(&c.f.chat).unwrap())
            .await
            .unwrap()
            .is_some()
    );
    crate::services::personal_erasure_service::ensure_indexes(&c.f.state.db)
        .await
        .unwrap();
    crate::services::personal_erasure_service::accept(&c.f.state.db, &c.f.owner, &c.f.owner, None)
        .await
        .unwrap();
    for _ in 0..32 {
        c.f.state
            .db
            .collection::<Document>("personal_erasure_jobs")
            .update_one(
                doc! {"user_id":&c.f.owner},
                doc! {"$set":{"retry_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        crate::services::personal_erasure_service::process_one(&c.f.state)
            .await
            .unwrap();
        if markers.count_documents(doc! {"user_id":&c.f.owner,"kind":{"$in":["model_control_recorder_admission","model_control_unobserved"]}}).await.unwrap()==0 {break;}
    }
    assert_eq!(markers.count_documents(doc! {"user_id":&c.f.owner,"kind":{"$in":["model_control_recorder_admission","model_control_unobserved"]}}).await.unwrap(),0);
    assert!(
        read_observation::unobserved_http(
            &c.f.state,
            &c.f.chat,
            Some(&c.connection),
            &c.target,
            "responses"
        )
        .await
        .is_err()
    );
    assert!(
        issue(
            &c.f.state,
            IssueRequest {
                chat: &c.f.chat,
                runtime_catalog_id: &c.target.service.id,
                outer_body: &json!({"input":"stale"}),
                negotiated_v1: true
            }
        )
        .await
        .is_err()
    );
    assert_eq!(markers.count_documents(doc! {"user_id":&c.f.owner,"kind":{"$in":["model_control_recorder_admission","model_control_unobserved"]}}).await.unwrap(),0);
    c.f.state.db.drop().await.unwrap();
}
