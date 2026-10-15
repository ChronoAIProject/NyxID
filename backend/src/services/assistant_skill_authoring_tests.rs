use super::*;
use crate::{
    models::{
        assistant_acknowledgement::AssistantAcknowledgement, assistant_agent::AssistantAgent,
    },
    services::{
        assistant_authority_tests::{Fixture, orchestrator_fixture},
        assistant_learning_publication as publication,
        feature_flag_service::FlagTarget,
    },
};
use futures::TryStreamExt;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[path = "assistant_skill_authoring_validation_tests.rs"]
mod validation_regressions;

#[path = "assistant_learning_recovery_tests.rs"]
mod recovery;

fn input(agent: &str) -> Value {
    json!({"scope":{"status":"complete","omissions":[]},"agent":agent,"name":"weekly-review","description":"Prepare a weekly review", "skill_md":"# Weekly review\n1. Check outstanding work.\n2. Write a concise report.","files":[{"path":"references/checklist.md","content":"Verify dates before publishing."}]})
}

/// The pre-review format check NyxID sends while drafting (its package's
/// operation ID is the fixed `preflight`), as opposed to publication traffic.
pub(super) fn preflight_request(request: &wiremock::Request) -> bool {
    const MARKER: &[u8] = br#"operation-id: "preflight""#;
    request.url.path() == "/api/v1/skill-format/validate"
        && request
            .body
            .windows(MARKER.len())
            .any(|window| window == MARKER)
}

/// Ornn requests other than drafting's pre-review format check.
async fn ornn_requests(server: &MockServer) -> Vec<wiremock::Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| !preflight_request(request))
        .collect()
}

async fn pending_skill_event_count(
    db: &mongodb::Database,
    conversation_id: &str,
    proposal_id: &str,
) -> usize {
    db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": conversation_id})
        .await
        .unwrap()
        .and_then(|row| row.get_array("pending_events").ok().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|event| {
            event
                .as_document()
                .and_then(|event| {
                    (event.get_str("kind").ok() == Some("skill_publication")
                        && event
                            .get_str("text")
                            .is_ok_and(|text| text.contains(proposal_id)))
                    .then_some(())
                })
                .is_some()
        })
        .count()
}

async fn fixture(name: &str) -> Fixture {
    let f = orchestrator_fixture(name).await;
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    feature_flag_service::set_platform_override(
        &f.state.db,
        learning::FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    f
}
async fn make(
    f: &Fixture,
    value: Value,
) -> (AssistantAgentLearningProposal, AssistantAcknowledgement) {
    make_with_access(f, value, false).await
}

async fn make_with_access(
    f: &Fixture,
    value: Value,
    owner_reader: bool,
) -> (AssistantAgentLearningProposal, AssistantAcknowledgement) {
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: if owner_reader {
            None
        } else {
            Some(&f.chat.api_key_id)
        },
        scopes: None,
        chat: (!owner_reader).then(|| std::sync::Arc::new(f.chat.clone())),
    };
    let result = create(
        &f.state,
        &f.chat,
        serde_json::from_value(value).unwrap(),
        &reader,
        &reader,
    )
    .await
    .unwrap();
    let proposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":result["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let card = f
        .state
        .db
        .collection(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":result["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    (proposal, card)
}

#[tokio::test]
async fn creation_preflights_once_and_review_binding_does_not_repeat_it() {
    let f = fixture("authoring_single_preflight").await;
    let server = setup_ornn(&f).await;
    let (row, _) = make(&f, input(&f.chat.agent_id)).await;
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| preflight_request(request))
            .count(),
        1
    );
    let before = server.received_requests().await.unwrap().len();
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    review::approval_binding_typed(
        &f.state,
        &f.owner,
        &row.agent_id,
        &row.id,
        row.revision,
        0,
        &reader,
    )
    .await
    .map_err(|failure| failure.error)
    .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_source_read_keeps_one_retryable_proposal_without_a_card() {
    let f = fixture("authoring_snapshot_approval_timeout").await;
    let server = setup_ornn(&f).await;
    let skill_id = Uuid::new_v4().to_string();
    let pin = crate::models::catalog_skill_revision::SkillReference {
        source: "ornn".into(),
        skill_id: skill_id.clone(),
        name: "weekly-review".into(),
        version: "1.0".into(),
        sha256: "a".repeat(64),
        dependencies: vec![],
        disabled: false,
    };
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills":bson::to_bson(&vec![pin]).unwrap()}},
        )
        .await
        .unwrap();
    let service = f
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! {"slug":"ornn-api"})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: service.id.clone(),
                service_name: service.name.clone(),
                approval_required: true,
                approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                rules: vec![],
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    let mut value = input(&f.chat.agent_id);
    value["base_skill"] = json!({"skill_id":skill_id,"version":"1.0"});
    let result = create(
        &f.state,
        &f.chat,
        serde_json::from_value(value).unwrap(),
        &reader,
        &reader,
    )
    .await;
    assert!(
        matches!(result, Err(AppError::Conflict(message)) if message.starts_with("source_skill_unavailable:"))
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {"source":"authored"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&f.owner,"authored_skill":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&f.owner,"service_id":&service.id,"status":"pending"})
            .await
            .unwrap(),
        1
    );
    assert!(ornn_requests(&server).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn renewed_confirmation_stays_in_original_conversation_and_operation() {
    let f = fixture("authoring_renewal_same_conversation").await;
    let (row, original) = make(&f, input(&f.chat.agent_id)).await;
    f.state.db.collection::<mongodb::bson::Document>(
        crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&original.id},
            doc! {"$set":{"status":"expired","expires_at":mongodb::bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::seconds(1))}},
        )
        .await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"failure_code":"approval_expired"}},
        )
        .await
        .unwrap();
    let expired_preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &original.id)
            .await
            .unwrap();
    assert!(
        expired_preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("renew"))
    );
    let body = serde_json::from_value(json!({
        "revision":row.revision,
        "agent_skills_revision":row.agent_skills_revision,
        "renewal_of":original.id,
    }))
    .unwrap();
    let axum::Json(result) = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(body),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "confirmation_required");
    let new_id = result["acknowledgement"]["acknowledgement_id"]
        .as_str()
        .unwrap();
    let renewed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":new_id})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(renewed.id, original.id);
    assert_eq!(renewed.conversation_id, original.conversation_id);
    assert_eq!(renewed.api_key_id, original.api_key_id);
    assert_eq!(renewed.arguments_digest, original.arguments_digest);
    let latest = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        latest.publication.unwrap().operation_id,
        row.publication.unwrap().operation_id
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn renewed_card_at_live_revision_publishes_through_decide_authored() {
    let f = fixture("authoring_renewed_card_publishes").await;
    let server = setup_ornn(&f).await;
    let (row, original) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id},
            doc! {"$inc":{"skills_revision":1_i64}},
        )
        .await
        .unwrap();
    let axum::Json(result) = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({"revision":row.revision,
            "agent_skills_revision":row.agent_skills_revision+1,"renewal_of":original.id}))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let renewed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":result["acknowledgement"]["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &renewed.id)
            .await
            .unwrap();
    assert_eq!(preview["skills_revision"], row.agent_skills_revision);
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("publish"))
    );
    let skill_id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &row, &skill_id, "POST").await;
    assert_eq!(finish(&f, renewed, true).await.unwrap().status, "used");
    let pinned = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pinned.status, "pinned");
    assert_eq!(
        pinned.publication.unwrap().skills_revision,
        row.agent_skills_revision + 1
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn old_pending_card_dismisses_without_rejecting_started_renewal() {
    let f = fixture("authoring_old_card_dismiss_started_renewal").await;
    let (row, old) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id},
            doc! {"$inc":{"skills_revision":1_i64}},
        )
        .await
        .unwrap();
    let axum::Json(result) = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({"revision":row.revision,
            "agent_skills_revision":row.agent_skills_revision+1,"renewal_of":old.id}))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let renewed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":result["acknowledgement"]["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    // The renewed card approved and was consumed by the started attempt.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&renewed.id},
            doc! {"$set":{"status":"used","decided_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{
                "status":"publication_failed","publication.started":true,
                "publication.approved_by":&f.owner,"publication.acknowledgement_id":&renewed.id,
                "publication.approval_digest":renewed.arguments_digest.as_ref().unwrap(),
                "publication.skills_revision":row.agent_skills_revision+1,
                "publication.attempt":1_i64,"failure_code":"version_conflict"
            }},
        )
        .await
        .unwrap();
    let approving =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &renewed.id)
            .await
            .unwrap();
    assert_eq!(approving["actions"], json!(["check"]));
    let before = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &old.id)
            .await
            .unwrap();
    assert_eq!(preview["actions"], json!(["dismiss_card"]));
    let denied = finish(&f, old, false).await.unwrap();
    assert_eq!(denied.status, "denied");
    let after = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, before.status);
    assert_eq!(after.failure_code, before.failure_code);
    assert_eq!(
        after.publication.unwrap().operation_id,
        before.publication.unwrap().operation_id
    );
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &denied.id)
            .await
            .unwrap();
    assert_eq!(preview["state"], "dismissed");
    // Past its expiry the consumed approving card still checks; the sweep
    // never expires a used card.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&renewed.id},
            doc! {"$set":{
                "expires_at":bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))
            }},
        )
        .await
        .unwrap();
    let expired_preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &renewed.id)
            .await
            .unwrap();
    assert_eq!(expired_preview["actions"], json!(["check"]));
    let axum::Json(reconfirmation) = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({"revision":row.revision,
            "agent_skills_revision":row.agent_skills_revision+1,"renewal_of":renewed.id}))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let new_id = reconfirmation["acknowledgement"]["acknowledgement_id"]
        .as_str()
        .unwrap();
    let check_preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, new_id)
            .await
            .unwrap();
    assert!(
        check_preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn active_publication_refuses_reject_and_edit() {
    let f = fixture("authoring_active_lease_refuses_mutation").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let publication = row.publication.as_ref().unwrap();
    let target = format!("{}:{}", Uuid::new_v4(), publication.version);
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    let barriers = f.state.db.collection::<bson::Document>(
        crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
    );
    barriers
        .insert_one(doc! {
            "_id":&target,"agent_id":&row.agent_id,"owner_id":&row.owner_id,
            "proposal_id":&row.id,"operation_id":&publication.operation_id,
            "package_sha256":&publication.sha256,"state":"reserved",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    let edited: Value = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&row.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    for (status, started, uncertain, verified, expiry) in [
        (
            "pending",
            false,
            false,
            false,
            Utc::now() + chrono::Duration::minutes(2),
        ),
        (
            "publication_failed",
            true,
            false,
            false,
            Utc::now() - chrono::Duration::seconds(1),
        ),
        (
            "pending",
            false,
            true,
            false,
            Utc::now() - chrono::Duration::seconds(1),
        ),
        (
            "publication_failed",
            false,
            false,
            true,
            Utc::now() - chrono::Duration::seconds(1),
        ),
    ] {
        proposals
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{
                    "status":status,"publication.started":started,
                    "publication.uncertain_dispatch":uncertain,
                    "publication.verified_at":if verified { bson::Bson::DateTime(bson::DateTime::now()) } else { bson::Bson::Null },
                    "publication.lease_id":"held-worker",
                    "publication.lease_expires_at":bson::DateTime::from_chrono(expiry),
                    "publication.target_kind":"update",
                    "publication.target_skill_id":target.split(':').next().unwrap(),
                }},
            )
            .await
            .unwrap();
        let before_row = proposals
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        let before_barrier = barriers
            .find_one(doc! {"_id":&target})
            .await
            .unwrap()
            .unwrap();
        let preview =
            review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
                .await
                .unwrap();
        assert!(
            !preview["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("discard")),
            "started={started}"
        );
        assert!(matches!(
            review::reject(
                &f.state,
                &f.owner,
                &row.agent_id,
                &row.id,
                row.revision,
                "rejected"
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            review::edit(
                &f.state,
                &f.owner,
                &row.agent_id,
                &row.id,
                row.revision,
                &edited
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            proposals
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap(),
            before_row
        );
        assert_eq!(
            barriers
                .find_one(doc! {"_id":&target})
                .await
                .unwrap()
                .unwrap(),
            before_barrier
        );
    }
    proposals.update_one(doc! {"_id":&row.id}, doc! {"$set":{
        "status":"publication_failed","publication.started":false,
        "publication.uncertain_dispatch":false,"publication.verified_at":bson::Bson::Null,
        "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1)),
    }}).await.unwrap();
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        !preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("discard"))
    );
    review::edit(
        &f.state,
        &f.owner,
        &row.agent_id,
        &row.id,
        row.revision,
        &edited,
    )
    .await
    .unwrap();
    let edited_row = proposals
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited_row.get_i64("revision").unwrap(), row.revision + 1);
    assert!(
        barriers
            .find_one(doc! {"_id":&target})
            .await
            .unwrap()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lease_expiry_uses_started_boundary_for_retry_mode() {
    let f = fixture("authoring_started_crash_boundary").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let digest = card.arguments_digest.clone().unwrap();
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    proposals.update_one(doc! {"_id":&row.id}, doc! {"$set":{
        "status":"publishing","publication.approved_by":&f.owner,
        "publication.acknowledgement_id":&card.id,"publication.approval_digest":&digest,
        "publication.lease_id":"lost-worker",
        "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1)),
    }}).await.unwrap();
    let before =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        before["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("publish"))
    );
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.started":true,"publication.last_stage":"publish"}},
        )
        .await
        .unwrap();
    let after =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        after["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    assert!(
        !after["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("discard"))
    );
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{
                "status":"publication_failed","failure_code":"approval_expired"
            }},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$set":{
                "expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))
            }},
        )
        .await
        .unwrap();
    let expired =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        expired["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    assert!(expired["failure_code"].is_null());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pre_claim_failure_is_bound_to_its_card_and_draft() {
    let f = fixture("authoring_preclaim_isolation").await;
    let (first, card) = make(&f, input(&f.chat.agent_id)).await;
    let mut second_input = input(&f.chat.agent_id);
    second_input["name"] = "another-review".into();
    let (second, _) = make(&f, second_input).await;
    let reference = card.authored_skill.as_ref().unwrap();
    let mut wrong = card.clone();
    wrong.arguments_digest = Some("wrong".into());
    assert!(
        review::record_pre_claim_failure(
            &f.state,
            &f.owner,
            reference,
            &wrong,
            0,
            review::PublicationFailureCode::NyxidRefused,
            &[],
        )
        .await
        .is_err()
    );
    review::record_pre_claim_failure(
        &f.state,
        &f.owner,
        reference,
        &card,
        0,
        review::PublicationFailureCode::NyxidRefused,
        &[],
    )
    .await
    .unwrap();
    let proposals = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
    assert_eq!(
        proposals
            .find_one(doc! {"_id":&first.id})
            .await
            .unwrap()
            .unwrap()
            .failure_code
            .as_deref(),
        Some("nyxid_refused")
    );
    assert_eq!(
        proposals
            .find_one(doc! {"_id":&second.id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "pending"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn expired_confirmation_renews_the_same_operation_and_restores_write_retry() {
    for allow_first in [false, true] {
        let f = fixture("authoring_expired_confirmation_renewal").await;
        let (row, card) = make(&f, input(&f.chat.agent_id)).await;
        if allow_first {
            let allowed =
                acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
                    .await
                    .unwrap();
            assert_eq!(allowed.status, "allowed");
        }
        let original = row.publication.as_ref().unwrap();
        f.state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"status":"publication_failed","failure_code":"approval_expired"}},
            )
            .await
            .unwrap();
        f.state.db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .update_one(doc! {"_id":&card.id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}})
            .await.unwrap();
        let unswept = f
            .state
            .db
            .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .find_one(doc! {"_id":&card.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            unswept.status,
            if allow_first { "allowed" } else { "pending" }
        );
        let listed = review::list(&f.state, &f.owner, &row.agent_id, false)
            .await
            .unwrap();
        let item = listed.iter().find(|item| item.id == row.id).unwrap();
        assert_eq!(item.card_status.as_deref(), Some("expired"));
        let repeated = draft_in_chat(&f, &f.chat).await.unwrap();
        assert_eq!(repeated["proposal_id"], row.id);
        assert_eq!(repeated["acknowledgement_id"], card.id);
        assert_eq!(repeated["status"], "expired");
        assert_eq!(
            f.state
                .db
                .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
                .count_documents(doc! {"authored_skill.proposal_id":&row.id})
                .await
                .unwrap(),
            1
        );
        let before = review::authored_preview_for_card(
            &f.state,
            &f.owner,
            &f.chat.agent_id,
            &row.id,
            &card.id,
        )
        .await
        .unwrap();
        assert_eq!(before["failure_code"], "approval_expired");
        assert!(
            before["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("renew"))
        );

        let repeated = draft_in_chat(&f, &f.chat).await.unwrap();
        assert_eq!(repeated["proposal_id"], row.id);
        assert_eq!(repeated["acknowledgement_id"], card.id);
        assert_eq!(repeated["status"], "expired");
        // Only the owner's explicit renewal raises another confirmation card.
        let body = serde_json::from_value(json!({
            "revision":row.revision, "agent_skills_revision":row.agent_skills_revision,
            "renewal_of":card.id,
        }))
        .unwrap();
        let axum::Json(result) = crate::handlers::assistant_agent_learning::approve(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((row.agent_id.clone(), row.id.clone())),
            axum::Json(body),
        )
        .await
        .unwrap();
        let fresh_card = f
            .state
            .db
            .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .find_one(
                doc! {"_id":result["acknowledgement"]["acknowledgement_id"].as_str().unwrap()},
            )
            .await
            .unwrap()
            .unwrap();
        let renewed = stored(&f, &row.id).await;
        assert_eq!(renewed.id, row.id);
        assert_ne!(fresh_card.id, card.id);
        assert_eq!(
            renewed.publication.as_ref().unwrap().operation_id,
            original.operation_id
        );
        assert_eq!(
            renewed.publication.as_ref().unwrap().sha256,
            original.sha256
        );
        let after = review::authored_preview_for_card(
            &f.state,
            &f.owner,
            &f.chat.agent_id,
            &row.id,
            &fresh_card.id,
        )
        .await
        .unwrap();
        assert!(after["failure_code"].is_null());
        assert!(
            after["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("publish"))
        );
        f.state.db.drop().await.unwrap();
    }
}

#[test]
fn authoring_reuses_text_validation_and_rejects_credential_shapes() {
    let raw = input("agent");
    let base = GeneratedProposal {
        schema_version: 1,
        kind: "new".into(),
        name: raw["name"].as_str().unwrap().into(),
        description: raw["description"].as_str().unwrap().into(),
        skill_md: raw["skill_md"].as_str().unwrap().into(),
        files: vec![],
        base_skill: None,
        rationale: String::new(),
        safety_notes: String::new(),
    };
    assert!(validate_body(&base).is_ok());
    for name in [
        "Bad Name",
        "../bad",
        "bad--name",
        "-bad",
        "bad-",
        "bad/name",
        "",
    ] {
        let mut draft = base.clone();
        draft.name = name.into();
        assert!(validate_body(&draft).is_err(), "{name}");
    }
    for path in [
        "../secrets.txt",
        "/tmp/x.md",
        "a//b.md",
        "a/./b.md",
        "a\\b.md",
        "run.sh",
        "SKILL.md",
    ] {
        let mut draft = base.clone();
        draft.files.push(GeneratedFile {
            path: path.into(),
            content: "text".into(),
        });
        assert!(validate_body(&draft).is_err(), "{path}");
    }
    for content in [
        "a".repeat(8001),
        "sk-0123456789abcdefghijklmnop".into(),
        "password: fixture".into(),
        "nyxid_ag_fixture".into(),
        "ghp_fixture".into(),
        "api_key=fixture".into(),
        "text\0binary".into(),
    ] {
        let mut draft = base.clone();
        draft.skill_md = content;
        assert!(validate_body(&draft).is_err());
    }
    let mut duplicate = base.clone();
    duplicate.files = vec![
        GeneratedFile {
            path: "A.md".into(),
            content: "one".into(),
        },
        GeneratedFile {
            path: "a.md".into(),
            content: "two".into(),
        },
    ];
    assert!(validate_body(&duplicate).is_err());
    for key in ["url", "owner", "permissions", "script", "destination"] {
        let mut bad = raw.clone();
        bad[key] = "untrusted".into();
        assert!(serde_json::from_value::<DraftInput>(bad).is_err());
    }
}

#[test]
fn authoring_guidance_uses_three_parts_and_existing_skills_first() {
    let prompt = crate::services::assistant_nyxagent::SYSTEM_PROMPT;
    for text in [
        "description defines role/scope",
        "persona defines tone/style",
        "Ornn skills",
        "checklists",
        "output templates",
        "multi-step workflows",
        "search_agent_skills",
        "preview_agent_skill",
        "set_agent_skills",
        "Ornn Playground",
        "sandboxes",
        "machines",
        "raw Ornn upload APIs",
    ] {
        assert!(GUIDANCE.contains(text), "{text}");
    }
    assert!(prompt.ends_with(" Description=role/scope; persona=tone; skills=procedures."));
    for preserved in [
        "For uncovered settings (agent-key creation, security, profile, billing, organizations), give the exact page with nyxid__settings_link.",
        "the owner's account and connected services.",
        "When the owner must finish a connect link, channel bot setup or verification outside chat, explain what to do and end your turn.",
        "NyxID resumes you when they finish; never ask them to reply that they are done or connected.",
        "Service approvals wait for the owner in the app, phone or Telegram and continue automatically; report timeouts.",
        "After posting work, end your turn; NyxID wakes you with replies when the group is quiet so you can report back.",
        "Link existing personal or organization bots from nyxid__list_channel_bots to yourself or a specialist with nyxid__connect_channel_bot by id or label.",
        "Create Telegram, Discord, Slack, Lark or other bots with nyxid__channel_bot_setup_link; share its link, never request bot secrets in chat or send the user to Studio.",
        "and ask the owner when unsure.",
        "When work is specialized or parallel, or the user asks for an agent",
        "Never tell the user to create an agent themselves. An agent key is different: a raw API key the user creates on its settings page.",
    ] {
        assert!(prompt.contains(preserved), "{preserved}");
    }
    assert!(!prompt.contains("draft_agent_skill"));
    assert!(GUIDANCE.contains("draft_agent_skill"));
    for name in ["spawn_subagent", "update_subagent"] {
        let description = crate::services::assistant_team_tools::description(name);
        for part in [
            "description",
            "persona",
            "Ornn skills",
            "set_agent_skills",
            "Never package or publish skills through Ornn Playground, sandboxes, machines or raw Ornn upload APIs.",
        ] {
            assert!(description.contains(part));
        }
    }
}

#[tokio::test]
async fn authoring_flag_guest_specialist_and_discovery_gates() {
    let f = orchestrator_fixture("authoring_gates").await;
    let value = input(&f.chat.agent_id);
    assert!(
        create(
            &f.state,
            &f.chat,
            serde_json::from_value(value.clone()).unwrap(),
            &review::UnavailableReader,
            &review::UnavailableReader
        )
        .await
        .is_err()
    );
    let visible = |service: crate::services::mcp_service::McpToolService| {
        service.endpoints.iter().any(|e| e.name == TOOL_NAME)
    };
    assert!(!visible(
        super::super::assistant_account_tools::virtual_service_for(&f.state.db, &f.chat).await
    ));
    feature_flag_service::set_platform_override(
        &f.state.db,
        learning::FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    assert!(visible(
        super::super::assistant_account_tools::virtual_service_for(&f.state.db, &f.chat).await
    ));
    let mut guest = f.chat.clone();
    guest.guest = true;
    assert!(
        create(
            &f.state,
            &guest,
            serde_json::from_value(value.clone()).unwrap(),
            &review::UnavailableReader,
            &review::UnavailableReader
        )
        .await
        .is_err()
    );
    assert!(!visible(
        super::super::assistant_account_tools::virtual_service_for(&f.state.db, &guest).await
    ));
    // A specialist sees the tool but drafts only for its own agent.
    let mut specialist = f.chat.clone();
    specialist.role = crate::models::assistant_conversation::AgentRole::Subagent;
    specialist.agent_id = Uuid::new_v4().to_string();
    assert!(visible(
        super::super::assistant_account_tools::virtual_service_for(&f.state.db, &specialist).await
    ));
    let error = create(
        &f.state,
        &specialist,
        serde_json::from_value(value.clone()).unwrap(),
        &review::UnavailableReader,
        &review::UnavailableReader,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&error, AppError::Forbidden(message) if message.contains("only for their own agent")),
        "{error:?}"
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authored_draft_is_encrypted_deduplicated_and_card_shows_complete_package() {
    let f = fixture("authoring_card").await;
    let value = input(&f.chat.agent_id);
    let (row, card) = make(&f, value.clone()).await;
    assert_eq!(row.source, ProposalSource::Authored);
    assert!(row.evidence.is_empty());
    assert!(!String::from_utf8_lossy(&row.body_encrypted).contains("Weekly review"));
    assert!(!format!("{row:?}").contains("Weekly review"));
    let legacy = {
        let mut doc = bson::to_document(&row).unwrap();
        doc.remove("source");
        bson::from_document::<AssistantAgentLearningProposal>(doc).unwrap()
    };
    assert_eq!(legacy.source, ProposalSource::Learned);
    assert!(acks::refusal(&card).get("confirm_phrase").is_none());
    assert!(
        acks::decide_reply(
            &f.state.db,
            &f.owner,
            std::slice::from_ref(&f.row.id),
            &format!("yes {}", acks::confirm_code(&card.id)),
            None
        )
        .await
        .unwrap()
        .is_none()
    );
    let raw_card = bson::to_document(&card).unwrap().to_string();
    assert!(!raw_card.contains("Weekly review"));
    let preview = review::authored_preview(&f.state, &f.owner, &row.agent_id, &row.id)
        .await
        .unwrap();
    assert_eq!(preview["files"].as_array().unwrap().len(), 2);
    assert!(
        preview["files"][0]["content"]
            .as_str()
            .unwrap()
            .contains(value["skill_md"].as_str().unwrap())
    );
    assert_eq!(preview["files"][1]["content"], value["files"][0]["content"]);
    let (retry, retry_card) = make(&f, value).await;
    assert_eq!(retry.id, row.id);
    assert_eq!(retry_card.id, card.id);
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(
                crate::models::assistant_agent_learning::CONFIG_COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

async fn mount_version(
    server: &MockServer,
    actor: &str,
    id: &str,
    p: &crate::models::assistant_agent_learning::LearningPublication,
    bytes: Vec<u8>,
) {
    Mock::given(method("GET")).and(path(format!("/api/v1/skills/{id}"))).and(query_param("version", &p.version))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":id,"name":p.name,"version":p.version,"skillHash":p.sha256,"description":"Weekly review","isPrivate":true,"createdBy":actor,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[],"metadata":{"category":"plain"}}}))).mount(server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .and(query_param("version", &p.version))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{id}/versions/{}/download",
            p.version
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(server)
        .await;
}
async fn setup_ornn(f: &Fixture) -> MockServer {
    let server = MockServer::start().await;
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.slug = "ornn-api".into();
    service.base_url = server.uri();
    service.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    server
}
async fn mount_publication(
    f: &Fixture,
    server: &MockServer,
    row: &AssistantAgentLearningProposal,
    skill_id: &str,
    verb: &str,
) {
    let p = row.publication.as_ref().unwrap();
    let bytes = f
        .state
        .encryption_keys
        .decrypt(&row.body_encrypted)
        .await
        .unwrap();
    let draft: GeneratedProposal = serde_json::from_slice(&bytes).unwrap();
    let snapshot: Option<publication::InterfaceSnapshot> = match &p.interface_encrypted {
        Some(encrypted) => Some(
            serde_json::from_slice(&f.state.encryption_keys.decrypt(encrypted).await.unwrap())
                .unwrap(),
        ),
        None => None,
    };
    let archive = publication::package_with_snapshot(
        &draft,
        &p.operation_id,
        &p.name,
        &p.version,
        snapshot.as_ref(),
    )
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(server)
        .await;
    Mock::given(method(verb))
        .and(path(if verb == "PUT" {
            format!("/api/v1/skills/{skill_id}")
        } else {
            "/api/v1/skills".into()
        }))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id}})))
        .expect(1)
        .mount(server)
        .await;
    mount_version(server, &f.owner, skill_id, p, archive).await;
}
async fn finish(
    f: &Fixture,
    card: AssistantAcknowledgement,
    allow: bool,
) -> AppResult<AssistantAcknowledgement> {
    crate::handlers::assistant_agent_learning::decide_authored_with_scope(
        &f.state,
        &crate::test_utils::test_auth_user(&f.owner),
        card,
        allow,
        false,
    )
    .await
}

async fn crash_revision(
    name: &str,
) -> (
    Fixture,
    MockServer,
    AssistantAgentLearningProposal,
    AssistantAcknowledgement,
    String,
) {
    let f = fixture(name).await;
    let server = setup_ornn(&f).await;
    let (base, base_card) = make(&f, input(&f.chat.agent_id)).await;
    let skill_id = Uuid::new_v4().to_string();
    let base_p = base.publication.as_ref().unwrap();
    let base_draft: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&base.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let base_bytes = publication::package_with_snapshot(
        &base_draft,
        &base_p.operation_id,
        &base_p.name,
        &base_p.version,
        None,
    )
    .unwrap();
    mount_publication(&f, &server, &base, &skill_id, "POST").await;
    finish(&f, base_card, true).await.unwrap();
    server.verify().await;
    server.reset().await;
    mount_version(&server, &f.owner, &skill_id, base_p, base_bytes).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":skill_id,"version":"1.0"});
    revision["skill_md"] = "# Crash window revision".into();
    let (row, card) = make_with_access(&f, revision, true).await;
    assert_eq!(
        row.publication.as_ref().unwrap().target_kind.as_deref(),
        Some("update")
    );
    server.reset().await;
    mount_version(
        &server,
        &f.owner,
        &skill_id,
        base_p,
        publication::package_with_snapshot(
            &base_draft,
            &base_p.operation_id,
            &base_p.name,
            &base_p.version,
            None,
        )
        .unwrap(),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    (f, server, row, card, skill_id)
}

async fn abort_at_stage(
    f: &Fixture,
    server: &MockServer,
    card: &AssistantAcknowledgement,
    row: &AssistantAgentLearningProposal,
    stage: &str,
    request_method: &str,
    request_path: &str,
) {
    {
        let future = finish(f, card.clone(), true);
        tokio::pin!(future);
        let mut reached = false;
        for _ in 0..400 {
            tokio::select! {
                result = &mut future => panic!("publication completed before crash: {result:?}"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            }
            let current = f
                .state
                .db
                .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            let requests = server.received_requests().await.unwrap();
            if current.status == "publishing"
                && current.publication.as_ref().unwrap().last_stage.as_deref() == Some(stage)
                && requests
                    .iter()
                    .any(|r| r.method.as_str() == request_method && r.url.path() == request_path)
            {
                reached = true;
                break;
            }
        }
        assert!(reached, "publication did not reach {stage}");
    }
    let claimed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(claimed.publication.as_ref().unwrap().attempt > 0);
    assert!(
        claimed
            .publication
            .as_ref()
            .unwrap()
            .lease_expires_at
            .is_some()
    );
}

async fn expire_crashed_lease(f: &Fixture, row: &AssistantAgentLearningProposal) {
    f.state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{
            "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1)),
        }}).await.unwrap();
}

async fn allowed_card(f: &Fixture, card: &AssistantAcknowledgement) -> AssistantAcknowledgement {
    let allowed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&card.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(allowed.status, "used");
    allowed
}

async fn publication_bytes(f: &Fixture, row: &AssistantAgentLearningProposal) -> Vec<u8> {
    let p = row.publication.as_ref().unwrap();
    let body: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&row.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let snapshot: publication::InterfaceSnapshot = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(p.interface_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    publication::package_with_snapshot(&body, &p.operation_id, &p.name, &p.version, Some(&snapshot))
        .unwrap()
}

#[tokio::test]
async fn crash_window_a_before_started_allows_rewrite() {
    let (f, server, row, card, skill_id) = crash_revision("authoring_crash_window_a").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(30))
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .with_priority(10)
        .mount(&server)
        .await;
    abort_at_stage(
        &f,
        &server,
        &card,
        &row,
        "format_validate",
        "POST",
        "/api/v1/skill-format/validate",
    )
    .await;
    expire_crashed_lease(&f, &row).await;
    let current = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    let p = current.publication.as_ref().unwrap();
    assert!(!p.started);
    assert!(!p.uncertain_dispatch);
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("retry"))
    );
    let target = format!("{skill_id}:{}", p.version);
    let barrier = f
        .state
        .db
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&target})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(barrier.get_str("operation_id").unwrap(), p.operation_id);
    assert_eq!(barrier.get_str("state").unwrap(), "reserved");
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id}})))
        .expect(1)
        .mount(&server)
        .await;
    mount_version(
        &server,
        &f.owner,
        &skill_id,
        row.publication.as_ref().unwrap(),
        publication_bytes(&f, &row).await,
    )
    .await;
    assert_eq!(
        finish(&f, allowed_card(&f, &card).await, true)
            .await
            .unwrap()
            .status,
        "used"
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "PUT"
                && r.url.path() == format!("/api/v1/skills/{skill_id}"))
            .count(),
        1
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crash_window_b_after_started_before_dispatch_is_check_only() {
    let (f, server, row, card, skill_id) = crash_revision("authoring_crash_window_b").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(30))
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    abort_at_stage(
        &f,
        &server,
        &card,
        &row,
        "format_validate",
        "POST",
        "/api/v1/skill-format/validate",
    )
    .await;
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{
                "publication.started":true,"publication.last_stage":"publish",
            }},
        )
        .await
        .unwrap();
    let target = format!("{skill_id}:{}", row.publication.as_ref().unwrap().version);
    f.state
        .db
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
        )
        .update_one(doc! {"_id":&target}, doc! {"$set":{"state":"uncertain"}})
        .await
        .unwrap();
    expire_crashed_lease(&f, &row).await;
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    assert!(
        finish(&f, allowed_card(&f, &card).await, true)
            .await
            .is_err()
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "PUT")
            .count(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crash_window_c_after_dispatch_is_check_only() {
    let (f, server, row, card, skill_id) = crash_revision("authoring_crash_window_c").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(30))
                .set_body_json(json!({"data":{"guid":skill_id}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    abort_at_stage(
        &f,
        &server,
        &card,
        &row,
        "publish",
        "PUT",
        &format!("/api/v1/skills/{skill_id}"),
    )
    .await;
    expire_crashed_lease(&f, &row).await;
    let crashed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(crashed.publication.as_ref().unwrap().started);
    assert!(crashed.publication.as_ref().unwrap().verified_at.is_none());
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    mount_version(
        &server,
        &f.owner,
        &skill_id,
        row.publication.as_ref().unwrap(),
        publication_bytes(&f, &row).await,
    )
    .await;
    assert_eq!(
        finish(&f, allowed_card(&f, &card).await, true)
            .await
            .unwrap()
            .status,
        "used"
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "PUT"
                && r.url.path() == format!("/api/v1/skills/{skill_id}"))
            .count(),
        1
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn base_verify_integrity_failures_keep_source_review_code() {
    for (case, owner, shared, hash) in [
        ("owner", "other-owner", json!([]), None),
        ("acl", "owner", json!(["another-user"]), None),
        ("hash", "owner", json!([]), Some("f".repeat(64))),
        ("zip", "owner", json!([]), None),
    ] {
        let (f, server, row, card, skill_id) =
            crash_revision(&format!("authoring_base_verify_{case}")).await;
        let owner = if owner == "owner" {
            f.owner.clone()
        } else {
            owner.to_owned()
        };
        let draft: GeneratedProposal = serde_json::from_slice(
            &f.state
                .encryption_keys
                .decrypt(&row.body_encrypted)
                .await
                .unwrap(),
        )
        .unwrap();
        let base_hash = draft.base_skill.as_ref().unwrap().sha256.clone();
        Mock::given(method("POST"))
            .and(path("/api/v1/skill-format/validate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/skills/{skill_id}")))
            .and(query_param("version", "1.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
                "guid":skill_id,"name":"weekly-review","version":"1.0",
                "skillHash":hash.unwrap_or(base_hash),
                "description":"Prepare a weekly review","isPrivate":true,"createdBy":owner,
                "sharedWithUsers":shared,"sharedWithOrgs":[],"grants":[],"metadata":{"category":"plain"}
            }})))
            .with_priority(1).mount(&server).await;
        if case == "zip" {
            Mock::given(method("GET"))
                .and(path(format!(
                    "/api/v1/skills/{skill_id}/versions/1.0/download"
                )))
                .respond_with(
                    ResponseTemplate::new(200).set_body_bytes(b"different zip bytes".to_vec()),
                )
                .with_priority(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/skills/{skill_id}")))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        assert!(finish(&f, card.clone(), true).await.is_err());
        let failed = f
            .state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            failed.failure_code.as_deref(),
            Some("base_verify_failed"),
            "{case}"
        );
        let preview =
            review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
                .await
                .unwrap();
        assert!(
            preview["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("discard"))
        );
        server.verify().await;
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn validator_read_status_replaces_previous_attempt_status() {
    let (f, server, row, card, skill_id) = crash_revision("authoring_validator_status_fence").await;
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{
                "status":"publication_failed","publication.last_registry_status":403_i32,
                "publication.last_stage":"publish",
            }},
        )
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"code":"unavailable"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    assert!(finish(&f, card.clone(), true).await.is_err());
    let failed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.failure_code.as_deref(), Some("ornn_unavailable"));
    assert_eq!(
        failed.publication.as_ref().unwrap().last_registry_status,
        Some(503)
    );
    assert_eq!(
        failed.publication.as_ref().unwrap().last_stage.as_deref(),
        Some("format_validate")
    );
    let audit = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(
            doc! {"event_type":"assistant_learning_publication_deferred",
            "event_data.proposal_id":&row.id},
        )
        .await
        .unwrap()
        .unwrap();
    let status = audit
        .get_document("event_data")
        .unwrap()
        .get("registry_status")
        .unwrap();
    assert!(matches!(
        status,
        bson::Bson::Int32(503) | bson::Bson::Int64(503)
    ));
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn validator_refusal_persists_its_http_status() {
    let (f, server, row, card, skill_id) = crash_revision("authoring_validator_422").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({"code":"validation_failed"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    assert!(finish(&f, card.clone(), true).await.is_err());
    let failed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed.failure_code.as_deref(),
        Some("ornn_validation_failed")
    );
    assert_eq!(failed.publication.unwrap().last_registry_status, Some(422));
    let audit = f.state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"assistant_learning_publication_deferred","event_data.proposal_id":&row.id})
        .await.unwrap().unwrap();
    assert!(matches!(
        audit
            .get_document("event_data")
            .unwrap()
            .get("registry_status"),
        Some(bson::Bson::Int32(422) | bson::Bson::Int64(422))
    ));
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn version_conflict_saga_reconciles_without_second_put() {
    for (case, code, outcome) in [
        ("exists_match", "SKILL_VERSION_EXISTS", "matched"),
        (
            "not_incremented_match",
            "VERSION_NOT_INCREMENTED",
            "matched",
        ),
        ("exists_mismatch", "SKILL_VERSION_EXISTS", "mismatch"),
        (
            "exists_zip_mismatch",
            "SKILL_VERSION_EXISTS",
            "zip_mismatch",
        ),
        (
            "not_incremented_transport",
            "VERSION_NOT_INCREMENTED",
            "transport",
        ),
    ] {
        let (f, server, row, card, skill_id) =
            crash_revision(&format!("authoring_conflict_{case}")).await;
        let p = row.publication.as_ref().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v1/skill-format/validate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/skills/{skill_id}")))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"code":code})))
            .expect(1)
            .mount(&server)
            .await;
        match outcome {
            "matched" => {
                mount_version(
                    &server,
                    &f.owner,
                    &skill_id,
                    p,
                    publication_bytes(&f, &row).await,
                )
                .await
            }
            "mismatch" => {
                Mock::given(method("GET"))
                    .and(path(format!("/api/v1/skills/{skill_id}")))
                    .and(query_param("version", &p.version))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
                        "guid":skill_id,"name":p.name,"version":p.version,"skillHash":"f".repeat(64),
                        "isPrivate":true,"createdBy":f.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]
                    }}))).mount(&server).await;
            }
            "zip_mismatch" => {
                mount_version(
                    &server,
                    &f.owner,
                    &skill_id,
                    p,
                    b"different zip bytes".to_vec(),
                )
                .await;
            }
            "transport" => {
                Mock::given(method("GET"))
                    .and(path(format!("/api/v1/skills/{skill_id}")))
                    .and(query_param("version", &p.version))
                    .respond_with(
                        ResponseTemplate::new(503).set_body_json(json!({"code":"unavailable"})),
                    )
                    .mount(&server)
                    .await;
            }
            _ => unreachable!(),
        }
        let first = finish(&f, card.clone(), true).await;
        let after = f
            .state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        if outcome == "matched" {
            assert_eq!(first.unwrap().status, "used", "{case}");
            assert_eq!(after.status, "pinned", "{case}");
        } else {
            assert!(first.is_err(), "{case}");
            assert_eq!(
                after.failure_code.as_deref(),
                Some(if outcome == "mismatch" || outcome == "zip_mismatch" {
                    "version_conflict"
                } else {
                    "publish_uncertain"
                }),
                "{case}"
            );
            assert_eq!(
                after.publication.as_ref().unwrap().last_registry_status,
                Some(if outcome == "transport" { 503 } else { 409 }),
                "{case}"
            );
            if outcome == "transport" {
                let event = f.state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                    .find_one(doc! {"event_type":"assistant_learning_publication_deferred","event_data.proposal_id":&row.id})
                    .await.unwrap().unwrap();
                assert!(matches!(
                    event
                        .get_document("event_data")
                        .unwrap()
                        .get("registry_status"),
                    Some(bson::Bson::Int32(503) | bson::Bson::Int64(503))
                ));
            }
            let barrier = f
                .state
                .db
                .collection::<bson::Document>(
                    crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
                )
                .find_one(doc! {"_id":format!("{skill_id}:{}",p.version)})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(barrier.get_str("state").unwrap(), "uncertain");
        }
        let _ = finish(&f, allowed_card(&f, &card).await, true).await;
        if outcome == "transport" {
            let retried = f
                .state
                .db
                .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(retried.publication.unwrap().last_registry_status, Some(503));
        }
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method.as_str() == "PUT"
                    && r.url.path() == format!("/api/v1/skills/{skill_id}"))
                .count(),
            1,
            "{case}"
        );
        server.verify().await;
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn post_write_zip_hash_mismatch_keeps_check_only_barrier() {
    let (f, server, row, card, skill_id) =
        crash_revision("authoring_post_write_zip_mismatch").await;
    let p = row.publication.as_ref().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id}})))
        .expect(1)
        .mount(&server)
        .await;
    mount_version(
        &server,
        &f.owner,
        &skill_id,
        p,
        b"different zip bytes".to_vec(),
    )
    .await;
    assert!(finish(&f, card.clone(), true).await.is_err());
    let failed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.failure_code.as_deref(), Some("version_conflict"));
    assert!(failed.publication.as_ref().unwrap().started);
    assert_eq!(
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap()["actions"],
        json!(["check"])
    );
    let barrier = f
        .state
        .db
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":format!("{skill_id}:{}",p.version)})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(barrier.get_str("state").unwrap(), "uncertain");
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authored_approval_uses_signed_person_and_pins_exact_post_then_put_version() {
    let mut f = fixture("authoring_publish").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &row, &id, "POST").await;
    // The model's initiating turn has ended; human review must still work.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let decided = finish(&f, card, true).await.unwrap();
    assert_eq!(decided.status, "used");
    let agent = f
        .state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.skills.len(), 1);
    assert_eq!(agent.skills[0].skill_id, id);
    assert_eq!(agent.skills[0].version, "1.0");
    assert_eq!(
        agent.skills[0].sha256,
        row.publication.as_ref().unwrap().sha256
    );
    let stored = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "pinned");
    assert!(stored.body_encrypted.is_empty());
    // A repeated human request cannot publish twice.
    finish(&f, decided, true).await.unwrap();
    for request in server.received_requests().await.unwrap() {
        let token = request
            .headers
            .get("x-nyxid-identity-token")
            .unwrap()
            .to_str()
            .unwrap();
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            token.split('.').nth(1).unwrap(),
        )
        .unwrap();
        let claims: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(claims["sub"], f.owner);
    }
    server.verify().await;
    // Remove L1 provenance: authored revisions may improve other owned B2 pins too.
    // A new owner turn can improve the pin while the completion event runs separately.
    f.row = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &f.owner,
        "Improve the weekly review skill",
    )
    .await;
    f.chat = crate::services::assistant_acknowledgement_service::for_key(
        &f.state.db,
        &f.owner,
        Some(&f.row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::ROOTS_COLLECTION_NAME,
        )
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    revision["skill_md"] = "# Improved weekly review".into();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let (next, next_card) = make_with_access(&f, revision, true).await;
    assert_eq!(next.publication.as_ref().unwrap().version, "1.1");
    mount_publication(&f, &server, &next, &id, "PUT").await;
    let service = f
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! {"slug":"ornn-api"})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: service.id.clone(),
                service_name: service.name.clone(),
                approval_required: true,
                approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                rules: vec![],
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    finish(&f, next_card, true).await.unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME,)
            .count_documents(doc! {"user_id":&f.owner,"service_id":&service.id})
            .await
            .unwrap(),
        0
    );
    let agent = f
        .state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.skills.len(), 1);
    assert_eq!(agent.skills[0].version, "1.1");
    assert_eq!(
        agent.skills[0].sha256,
        next.publication.as_ref().unwrap().sha256
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn two_authored_drafts_cannot_publish_the_same_update_target_after_uncertainty() {
    let f = fixture("authoring_target_barrier").await;
    let server = setup_ornn(&f).await;
    let (base, base_card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &base, &id, "POST").await;
    finish(&f, base_card, true).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let mut first = input(&f.chat.agent_id);
    first["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    first["skill_md"] = "# First revision".into();
    let mut second = first.clone();
    second["skill_md"] = "# Second revision".into();
    let (first_row, first_card) = make_with_access(&f, first, true).await;
    let (second_row, second_card) = make_with_access(&f, second, true).await;
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&second_row.id},
            doc! {"$unset":{"publication.target_kind":"","publication.target_skill_id":""}},
        )
        .await
        .unwrap();
    assert_ne!(
        first_row.publication.as_ref().unwrap().operation_id,
        second_row.publication.as_ref().unwrap().operation_id
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let first_card_id = first_card.id.clone();
    assert!(finish(&f, first_card.clone(), true).await.is_err());
    let held = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&first_row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(held.publication.as_ref().unwrap().uncertain_dispatch);
    assert!(finish(&f, second_card, true).await.is_err());
    let blocked = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&second_row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocked.failure_code.as_deref(), Some("target_busy"));
    assert_eq!(
        blocked.publication.as_ref().unwrap().target_kind.as_deref(),
        Some("update")
    );
    assert_eq!(
        blocked
            .publication
            .as_ref()
            .unwrap()
            .target_skill_id
            .as_deref(),
        Some(id.as_str())
    );
    let barrier = f
        .state
        .db
        .collection::<crate::models::assistant_agent_learning::LearningPublicationTarget>(
            crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":format!("{id}:1.1")})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        barrier.operation_id,
        held.publication.as_ref().unwrap().operation_id
    );
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&first_row.id},
            doc! {"$set":{"failure_code":"version_conflict"}},
        )
        .await
        .unwrap();
    let preview = review::authored_preview_for_card(
        &f.state,
        &f.owner,
        &first_row.agent_id,
        &first_row.id,
        &first_card.id,
    )
    .await
    .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    assert!(
        !preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("discard"))
    );
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    assert!(
        review::approval_binding(
            &f.state,
            &f.owner,
            &first_row.agent_id,
            &first_row.id,
            first_row.revision,
            first_row.agent_skills_revision,
            &reader,
        )
        .await
        .is_ok()
    );
    let allowed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&first_card_id})
        .await
        .unwrap()
        .unwrap();
    assert!(finish(&f, allowed.clone(), true).await.is_err());
    let p = held.publication.as_ref().unwrap();
    let draft: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&held.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let snapshot: publication::InterfaceSnapshot = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(p.interface_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    let archive = publication::package_with_snapshot(
        &draft,
        &p.operation_id,
        &p.name,
        &p.version,
        Some(&snapshot),
    )
    .unwrap();
    mount_version(&server, &f.owner, &id, p, archive).await;
    let retry = finish(&f, allowed, true).await.unwrap();
    assert_eq!(retry.status, "used");
    let pinned = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&first_row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pinned.status, "pinned");
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn interface_refusal_releases_barrier_and_requires_a_revised_draft() {
    let f = fixture("authoring_interface_refusal_releases").await;
    let server = setup_ornn(&f).await;
    let (base, base_card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    let base_p = base.publication.as_ref().unwrap();
    let base_draft: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&base.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let base_bytes = publication::package_with_snapshot(
        &base_draft,
        &base_p.operation_id,
        &base_p.name,
        &base_p.version,
        None,
    )
    .unwrap();
    mount_publication(&f, &server, &base, &id, "POST").await;
    finish(&f, base_card, true).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let mut first = input(&f.chat.agent_id);
    first["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    first["skill_md"] = "# First revision".into();
    let mut second = first.clone();
    second["skill_md"] = "# Revised guidance".into();
    let (first_row, first_card) = make_with_access(&f, first, true).await;
    let (second_row, second_card) = make_with_access(&f, second, true).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(json!({"code":"BREAKING_CHANGE_WITHOUT_MAJOR_BUMP"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(finish(&f, first_card.clone(), true).await.is_err());
    let refused = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&first_row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        refused.failure_code.as_deref(),
        Some("ornn_interface_change_requires_major")
    );
    assert!(!refused.publication.unwrap().started);
    assert!(
        f.state
            .db
            .collection::<crate::models::assistant_agent_learning::LearningPublicationTarget>(
                crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME
            )
            .find_one(doc! {"_id":format!("{id}:1.1")})
            .await
            .unwrap()
            .is_none()
    );
    let preview = review::authored_preview_for_card(
        &f.state,
        &f.owner,
        &f.chat.agent_id,
        &first_row.id,
        &first_card.id,
    )
    .await
    .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("discard"))
    );
    server.verify().await;
    server.reset().await;
    mount_version(&server, &f.owner, &id, base_p, base_bytes).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    mount_publication(&f, &server, &second_row, &id, "PUT").await;
    assert_eq!(finish(&f, second_card, true).await.unwrap().status, "used");
    let claimed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&second_row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.status, "pinned");
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn released_target_is_claimed_by_second_draft_and_old_retry_reports_target_busy() {
    let f = fixture("authoring_released_target_reclaimed").await;
    let server = setup_ornn(&f).await;
    let (base, base_card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    let base_p = base.publication.as_ref().unwrap();
    let base_draft: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&base.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let base_bytes = publication::package_with_snapshot(
        &base_draft,
        &base_p.operation_id,
        &base_p.name,
        &base_p.version,
        None,
    )
    .unwrap();
    mount_publication(&f, &server, &base, &id, "POST").await;
    finish(&f, base_card, true).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let mut first_input = input(&f.chat.agent_id);
    first_input["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    first_input["skill_md"] = "# First attempt".into();
    let mut second_input = first_input.clone();
    second_input["skill_md"] = "# Another attempt".into();
    let (first, first_card) = make_with_access(&f, first_input, true).await;
    let (second, second_card) = make_with_access(&f, second_input, true).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"code":"forbidden"})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(finish(&f, first_card.clone(), true).await.is_err());
    let failed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&first.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.failure_code.as_deref(), Some("ornn_write_forbidden"));
    assert!(!failed.publication.unwrap().started);
    let targets = f
        .state
        .db
        .collection::<crate::models::assistant_agent_learning::LearningPublicationTarget>(
            crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
        );
    assert!(
        targets
            .find_one(doc! {"_id":format!("{id}:1.1")})
            .await
            .unwrap()
            .is_none()
    );
    server.verify().await;
    server.reset().await;
    mount_version(&server, &f.owner, &id, base_p, base_bytes).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(finish(&f, second_card, true).await.is_err());
    let holder = targets
        .find_one(doc! {"_id":format!("{id}:1.1")})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        holder.operation_id,
        second.publication.unwrap().operation_id
    );
    let allowed_first = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&first_card.id})
        .await
        .unwrap()
        .unwrap();
    let busy = review::authored_preview_for_card(
        &f.state,
        &f.owner,
        &first.agent_id,
        &first.id,
        &first_card.id,
    )
    .await
    .unwrap();
    // The holder is the only blocker: the stored refusal stays, the card shows
    // the computed target_busy, and no retry is offered.
    assert_eq!(busy["failure_code"], "target_busy");
    assert!(busy["failure_stage"].is_null());
    assert_eq!(busy["failure_rules"], json!([]));
    let original_preview = review::authored_preview(&f.state, &f.owner, &first.agent_id, &first.id)
        .await
        .unwrap();
    assert_eq!(original_preview["failure_code"], "ornn_write_forbidden");
    assert_eq!(original_preview["failure_stage"], "publish");
    assert_eq!(busy["actions"], json!(["discard"]));
    assert!(matches!(
        finish(&f, allowed_first, true).await,
        Err(AppError::Conflict(message))
            if message == "This confirmation cannot publish right now; see the card for the reason"
    ));
    let blocked = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&first.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        blocked.failure_code.as_deref(),
        Some("ornn_write_forbidden")
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn validator_access_failure_releases_pre_dispatch_reservation() {
    let f = fixture("authoring_predispatch_target_release").await;
    let server = setup_ornn(&f).await;
    let (base, base_card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &base, &id, "POST").await;
    finish(&f, base_card, true).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    let (row, card) = make_with_access(&f, revision, true).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"code":"forbidden"})))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    assert!(finish(&f, card.clone(), true).await.is_err());
    let failed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.failure_code.as_deref(), Some("ornn_read_forbidden"));
    assert!(!failed.publication.unwrap().started);
    assert!(
        f.state
            .db
            .collection::<crate::models::assistant_agent_learning::LearningPublicationTarget>(
                crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME
            )
            .find_one(doc! {"_id":format!("{id}:1.1")})
            .await
            .unwrap()
            .is_none()
    );
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("retry"))
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn verified_checkpoint_survives_pin_failure_without_republishing() {
    use crate::services::agent_skill_service::{OrnnOutcome, OrnnReader};
    use std::sync::atomic::{AtomicBool, Ordering};
    struct PinRaceReader<'a> {
        inner: crate::handlers::agent_skills::Reader<'a>,
        db: mongodb::Database,
        agent_id: String,
        changed: AtomicBool,
    }
    #[async_trait::async_trait]
    impl OrnnReader for PinRaceReader<'_> {
        async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
            self.classified(http::Method::GET, path, Vec::new())
                .await
                .into_app_result()
        }
        async fn classified(&self, method: http::Method, path: &str, body: Vec<u8>) -> OrnnOutcome {
            let result = self.inner.classified(method, path, body).await;
            if path.ends_with("/download") && !self.changed.swap(true, Ordering::SeqCst) {
                self.db
                    .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&self.agent_id},
                        doc! {"$inc":{"skills_revision":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            result
        }
    }
    let f = fixture("authoring_verified_checkpoint_pin_race").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &row, &id, "POST").await;
    let allowed = acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
        .await
        .unwrap();
    let p = row.publication.as_ref().unwrap();
    let binding = publication::binding(&row, p, p.skills_revision);
    let inner = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    let race = PinRaceReader {
        inner,
        db: f.state.db.clone(),
        agent_id: row.agent_id.clone(),
        changed: AtomicBool::new(false),
    };
    assert!(
        review::approve(
            &f.state,
            &f.chat,
            &row.agent_id,
            &row.id,
            &allowed.id,
            &binding,
            &race
        )
        .await
        .is_err()
    );
    let failed = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, "published_unpinned");
    assert_eq!(failed.failure_code.as_deref(), Some("pin_conflict"));
    assert!(failed.publication.as_ref().unwrap().verified_at.is_some());
    let fresh_preview = review::authored_preview(&f.state, &f.owner, &row.agent_id, &row.id)
        .await
        .unwrap();
    assert_eq!(
        fresh_preview["current_skills_revision"],
        row.agent_skills_revision + 1
    );
    let axum::Json(renewal) = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({
                "revision":row.revision,
                "agent_skills_revision":row.agent_skills_revision+1,
                "renewal_of":allowed.id,
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let new_id = renewal["acknowledgement"]["acknowledgement_id"]
        .as_str()
        .unwrap();
    let renewed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":new_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        renewed.authored_skill.as_ref().unwrap().skills_revision,
        row.agent_skills_revision + 1
    );
    assert_eq!(renewed.conversation_id, allowed.conversation_id);
    let card_preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &renewed.id)
            .await
            .unwrap();
    assert!(
        card_preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    assert_eq!(finish(&f, renewed, true).await.unwrap().status, "used");
    let pinned = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pinned.status, "pinned");
    assert_eq!(
        pinned.publication.as_ref().unwrap().operation_id,
        p.operation_id
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/api/v1/skills")
            .count(),
        1
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_authored_card_can_be_denied_after_unrelated_skill_pin() {
    let f = fixture("authoring_pending_deny_after_revision").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id},
            doc! {"$inc":{"skills_revision":1_i64}},
        )
        .await
        .unwrap();
    finish(&f, card, false).await.unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "rejected"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn renewal_refuses_changed_source_skill() {
    let (f, _server, row, card, _) = crash_revision("authoring_renewal_base_changed").await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id},
            doc! {"$inc":{"skills_revision":1_i64},"$set":{"skills":[]}},
        )
        .await
        .unwrap();
    let renewal = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({
                "revision":row.revision,"agent_skills_revision":row.agent_skills_revision+1,
                "renewal_of":card.id,
            }))
            .unwrap(),
        ),
    )
    .await;
    assert!(matches!(renewal, Err(AppError::Conflict(message)) if message == "base_skill_changed"));
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(preview["failure_code"], "base_changed");
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("deny"))
    );
    assert_eq!(
        pending_skill_event_count(&f.state.db, &card.conversation_id, &row.id).await,
        1
    );
    let retry = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({
                "revision":row.revision,"agent_skills_revision":row.agent_skills_revision+1,
                "renewal_of":card.id,
            }))
            .unwrap(),
        ),
    )
    .await;
    assert!(retry.is_err());
    assert_eq!(
        pending_skill_event_count(&f.state.db, &card.conversation_id, &row.id).await,
        1
    );
    assert_ne!(stored(&f, &row.id).await.status, "published");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn duplicate_confirmation_keeps_substantive_refusal() {
    let f = fixture("authoring_refusal_not_overwritten").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{
                "status":"publication_failed",
                "failure_code":"ornn_interface_change_requires_major",
                "publication.last_stage":"publish",
            }},
        )
        .await
        .unwrap();
    assert!(finish(&f, card.clone(), true).await.is_err());
    let after = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.failure_code.as_deref(),
        Some("ornn_interface_change_requires_major")
    );
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("deny"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authored_denial_and_expiry_never_publish() {
    let f = fixture("authoring_no_publish").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let before = server.received_requests().await.unwrap().len();
    finish(&f, card, false).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    let stored = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "rejected");
    assert!(stored.body_encrypted.is_empty());
    let mut value = input(&f.chat.agent_id);
    value["name"] = "other-review".into();
    let (_, mut card) = make(&f, value).await;
    card.expires_at = Utc::now() - chrono::Duration::seconds(1);
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(card.expires_at)}},
        )
        .await
        .unwrap();
    let before = server.received_requests().await.unwrap().len();
    assert!(finish(&f, card, true).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authored_changed_content_refuses_old_card_before_ornn() {
    let f = fixture("authoring_changed").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id}, doc! {"$inc":{"revision":1_i64}})
        .await
        .unwrap();
    let before = server.received_requests().await.unwrap().len();
    assert!(finish(&f, card, true).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    f.state.db.drop().await.unwrap();
}

const WITHHELD: &str = "This confirmation cannot publish right now; see the card for the reason";

async fn stored(f: &Fixture, id: &str) -> AssistantAgentLearningProposal {
    f.state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn card_preview(
    f: &Fixture,
    row: &AssistantAgentLearningProposal,
    card: &AssistantAcknowledgement,
) -> Value {
    review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
        .await
        .unwrap()
}

#[tokio::test]
async fn allowed_card_binding_refusal_keeps_substantive_failure_and_audits_clicked_card() {
    let f = fixture("authoring_allowed_binding_refusal").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let allowed = acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
        .await
        .unwrap();
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"publication_failed","failure_code":"ornn_write_forbidden",
            "publication.last_stage":"publish"}},
        )
        .await
        .unwrap();
    assert_eq!(
        card_preview(&f, &row, &allowed).await["actions"],
        json!(["retry", "discard"])
    );
    // The binding refuses before any claim while publication is not admitted.
    f.state
        .db
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::MIGRATIONS_COLLECTION_NAME,
        )
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(finish(&f, allowed.clone(), true).await.is_err());
    let kept = stored(&f, &row.id).await;
    assert_eq!(kept.failure_code.as_deref(), Some("ornn_write_forbidden"));
    assert_eq!(
        kept.publication.unwrap().last_stage.as_deref(),
        Some("publish")
    );
    let audits = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let pre_claim = doc! {"event_type":"assistant_learning_publication_deferred",
    "event_data.proposal_id":&row.id,"event_data.stage":"pre_claim"};
    assert_eq!(audits.count_documents(pre_claim.clone()).await.unwrap(), 0);
    // A transient code is replaced, and the audit names the clicked card.
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"failure_code":"ornn_unavailable","publication.last_stage":"format_validate"}},
        )
        .await
        .unwrap();
    assert!(finish(&f, allowed.clone(), true).await.is_err());
    assert_eq!(
        stored(&f, &row.id).await.failure_code.as_deref(),
        Some("nyxid_refused")
    );
    let audit = audits.find_one(pre_claim).await.unwrap().unwrap();
    assert_eq!(
        audit
            .get_document("event_data")
            .unwrap()
            .get_str("clicked_acknowledgement_id")
            .unwrap(),
        allowed.id
    );
    // Decisions the server withholds are audited with metadata only.
    assert!(matches!(
        finish(&f, allowed.clone(), false).await,
        Err(AppError::Conflict(message)) if message == WITHHELD
    ));
    let withheld = audits
        .find_one(
            doc! {"event_type":"assistant_learning_publication_decision_withheld",
            "event_data.acknowledgement_id":&allowed.id},
        )
        .await
        .unwrap()
        .unwrap();
    let data = withheld.get_document("event_data").unwrap();
    assert_eq!(data.get_str("decision").unwrap(), "deny");
    assert_eq!(data.get_str("card_status").unwrap(), "allowed");
    assert_eq!(data.get_str("proposal_id").unwrap(), row.id);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn live_pending_card_reports_target_busy_for_a_classified_second_draft() {
    let f = fixture("authoring_classified_target_busy").await;
    let server = setup_ornn(&f).await;
    let (base, base_card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &base, &id, "POST").await;
    finish(&f, base_card, true).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let mut first_input = input(&f.chat.agent_id);
    first_input["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    first_input["skill_md"] = "# First revision".into();
    let mut second_input = first_input.clone();
    second_input["skill_md"] = "# Second revision".into();
    let (first, _) = make_with_access(&f, first_input, true).await;
    let (second, second_card) = make_with_access(&f, second_input, true).await;
    let held = first.publication.as_ref().unwrap();
    assert_eq!(
        second.publication.as_ref().unwrap().target_kind.as_deref(),
        Some("update")
    );
    // The first draft's dispatched attempt holds the version.
    let now = Utc::now();
    f.state
        .db
        .collection(crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME)
        .insert_one(
            crate::models::assistant_agent_learning::LearningPublicationTarget {
                id: format!("{id}:{}", held.version),
                agent_id: first.agent_id.clone(),
                owner_id: first.owner_id.clone(),
                proposal_id: first.id.clone(),
                operation_id: held.operation_id.clone(),
                package_sha256: held.sha256.clone(),
                state: "uncertain".into(),
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap().len();
    let preview = card_preview(&f, &second, &second_card).await;
    assert_eq!(preview["failure_code"], "target_busy");
    assert_eq!(preview["actions"], json!(["deny"]));
    assert!(matches!(
        finish(&f, second_card.clone(), true).await,
        Err(AppError::Conflict(message)) if message == WITHHELD
    ));
    let unchanged = stored(&f, &second.id).await;
    assert_eq!(unchanged.status, "pending");
    assert_eq!(unchanged.failure_code, None);
    assert_eq!(server.received_requests().await.unwrap().len(), requests);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn base_change_replaces_a_transient_failure_and_stops_the_retry_loop() {
    let (f, _server, row, card, _) = crash_revision("authoring_base_change_after_transient").await;
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"publication_failed","failure_code":"ornn_unavailable",
            "publication.last_stage":"format_validate"}},
        )
        .await
        .unwrap();
    // Replacing the source pin advances the agent's skills revision.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id},
            doc! {"$set":{"skills":[]},"$inc":{"skills_revision":1_i64}},
        )
        .await
        .unwrap();
    let before = card_preview(&f, &row, &card).await;
    assert_eq!(before["failure_code"], "ornn_unavailable");
    assert_eq!(before["actions"], json!(["deny", "renew"]));
    let renewal = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({"revision":row.revision,
                "agent_skills_revision":row.agent_skills_revision+1,"renewal_of":card.id}))
            .unwrap(),
        ),
    )
    .await;
    assert!(matches!(renewal, Err(AppError::Conflict(message)) if message == "base_skill_changed"));
    let changed = stored(&f, &row.id).await;
    assert_eq!(changed.status, "publication_failed");
    assert_eq!(changed.failure_code.as_deref(), Some("base_changed"));
    let after = card_preview(&f, &row, &card).await;
    assert_eq!(after["failure_code"], "base_changed");
    assert_eq!(after["actions"], json!(["deny"]));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn base_change_during_an_attempt_is_recorded_before_dispatch() {
    use crate::services::agent_skill_service::{OrnnOutcome, OrnnReader};
    struct BaseSwapReader<'a> {
        inner: crate::handlers::agent_skills::Reader<'a>,
        db: mongodb::Database,
        agent_id: String,
    }
    #[async_trait::async_trait]
    impl OrnnReader for BaseSwapReader<'_> {
        async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
            self.classified(http::Method::GET, path, Vec::new())
                .await
                .into_app_result()
        }
        async fn classified(&self, method: http::Method, path: &str, body: Vec<u8>) -> OrnnOutcome {
            let validator = method == http::Method::POST && path == "/api/v1/skill-format/validate";
            let result = self.inner.classified(method, path, body).await;
            if validator {
                self.db
                    .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(doc! {"_id":&self.agent_id}, doc! {"$set":{"skills":[]}})
                    .await
                    .unwrap();
            }
            result
        }
    }
    let (f, server, row, card, skill_id) =
        crash_revision("authoring_base_change_mid_attempt").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id}})))
        .expect(0)
        .mount(&server)
        .await;
    let allowed = acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
        .await
        .unwrap();
    let p = row.publication.as_ref().unwrap();
    let binding = publication::binding(&row, p, p.skills_revision);
    let reader = BaseSwapReader {
        inner: crate::handlers::agent_skills::Reader {
            state: &f.state,
            person: &f.owner,
            thread_key: None,
            scopes: None,
            chat: None,
        },
        db: f.state.db.clone(),
        agent_id: row.agent_id.clone(),
    };
    let error = review::approve(
        &f.state,
        &f.chat,
        &row.agent_id,
        &row.id,
        &allowed.id,
        &binding,
        &reader,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::Conflict(message) if message == "base_skill_changed"));
    let changed = stored(&f, &row.id).await;
    assert_eq!(changed.status, "publication_failed");
    assert_eq!(changed.failure_code.as_deref(), Some("base_changed"));
    let changed_p = changed.publication.unwrap();
    assert_eq!(changed_p.last_stage.as_deref(), Some("base_verify"));
    assert!(!changed_p.started);
    assert!(changed_p.lease_expires_at.is_none());
    assert!(
        f.state
            .db
            .collection::<bson::Document>(
                crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME
            )
            .find_one(doc! {"_id":format!("{skill_id}:{}", p.version)})
            .await
            .unwrap()
            .is_none()
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn card_actions_follow_unswept_expiry_and_withhold_renewal_during_a_lease() {
    let f = fixture("authoring_actions_expiry_lease").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    // Past expiry but not yet swept: still `pending`, yet it can no longer decide.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}},
        )
        .await
        .unwrap();
    assert_eq!(
        card_preview(&f, &row, &card).await["actions"],
        json!(["discard", "renew"])
    );
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"publishing","publication.lease_id":"live-attempt",
                "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::seconds(60))}},
        )
        .await
        .unwrap();
    let leased = card_preview(&f, &row, &card).await;
    assert_eq!(leased["lease_live"], true);
    assert_eq!(leased["actions"], json!([]));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn preview_without_a_card_keeps_the_original_response() {
    let f = fixture("authoring_preview_without_card").await;
    let (row, _) = make(&f, input(&f.chat.agent_id)).await;
    let axum::Json(preview) = crate::handlers::assistant_agent_learning::authored_preview(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::extract::Query(serde_json::from_value(json!({"_":"cache-buster"})).unwrap()),
    )
    .await
    .unwrap();
    let keys: std::collections::BTreeSet<&str> = preview
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    // The original fields plus additive diagnostics; no card actions or state.
    let expected: std::collections::BTreeSet<&str> = [
        "id",
        "agent_id",
        "agent_name",
        "revision",
        "skills_revision",
        "current_skills_revision",
        "status",
        "name",
        "version",
        "files",
        "failure_code",
        "failure_details_unavailable",
        "failure_rules",
        "failure_stage",
        "scope",
        "scope_unavailable",
        "coverage",
        "coverage_omitted",
        "lease_live",
        "base_scripts_not_copied",
    ]
    .into();
    assert_eq!(keys, expected);
    assert_eq!(preview["status"], "pending");
    assert_eq!(preview["files"].as_array().unwrap().len(), 2);
    assert_eq!(preview["failure_details_unavailable"], false);
    assert_eq!(preview["failure_rules"], json!([]));
    assert!(preview["failure_stage"].is_null());
    assert_eq!(preview["scope"]["status"], "complete");
    assert_eq!(preview["scope_unavailable"], false);
    assert_eq!(preview["coverage"], json!([]));
    assert_eq!(preview["coverage_omitted"], 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn draft_tool_result_keeps_its_shape() {
    let f = fixture("authoring_draft_result_shape").await;
    let result = create(
        &f.state,
        &f.chat,
        parse_input(&input(&f.chat.agent_id)).unwrap(),
        &review::UnavailableReader,
        &review::UnavailableReader,
    )
    .await
    .unwrap();
    let keys: std::collections::BTreeSet<&str> = result
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected: std::collections::BTreeSet<&str> = [
        "error",
        "kind",
        "acknowledgement_id",
        "service_slug",
        "service_name",
        "summary",
        "decider",
        "instructions",
        "review_url",
        "proposal_id",
        "agent_id",
        "source",
    ]
    .into();
    assert_eq!(keys, expected);
    let review_url = url::Url::parse(result["review_url"].as_str().unwrap()).unwrap();
    assert!(matches!(review_url.scheme(), "http" | "https"));
    assert!(review_url.host_str().is_some());
    assert_eq!(result["error"], "acknowledgement_required");
    assert_eq!(result["kind"], "action");
    assert_eq!(result["source"], "authored");
    assert_eq!(result["agent_id"], f.chat.agent_id.as_str());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn operator_release_frees_an_uncertain_target_only_for_the_exact_operation() {
    let (f, _server, row, card, skill_id) = crash_revision("authoring_operator_release").await;
    let p = row.publication.as_ref().unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(doc! {"_id":&card.id}, doc! {"$set":{"status":"used"}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"publication_failed","failure_code":"publish_uncertain",
            "publication.started":true,"publication.uncertain_dispatch":true,
            "publication.approved_by":&f.owner,"publication.acknowledgement_id":&card.id,
            "publication.approval_digest":card.arguments_digest.as_ref().unwrap(),
            "publication.attempt":1_i64,"publication.last_stage":"publish"}},
        )
        .await
        .unwrap();
    let target = format!("{skill_id}:{}", p.version);
    let now = Utc::now();
    let targets = f
        .state
        .db
        .collection::<crate::models::assistant_agent_learning::LearningPublicationTarget>(
            crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME,
        );
    targets
        .insert_one(
            crate::models::assistant_agent_learning::LearningPublicationTarget {
                id: target.clone(),
                agent_id: row.agent_id.clone(),
                owner_id: row.owner_id.clone(),
                proposal_id: row.id.clone(),
                operation_id: p.operation_id.clone(),
                package_sha256: p.sha256.clone(),
                state: "uncertain".into(),
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    let used = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&card.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        card_preview(&f, &row, &used).await["actions"],
        json!(["check"])
    );
    // The owner's session is not an operator.
    assert!(
        crate::handlers::assistant_agent_learning::admin_release_target(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path(row.id.clone()),
            axum::Json(
                serde_json::from_value(
                    json!({"operation_id":p.operation_id,"evidence_ref":"INC-1812"})
                )
                .unwrap()
            ),
        )
        .await
        .is_err()
    );
    let operator = crate::services::audit_service::AuditActor {
        user_id: "operator".into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    assert!(matches!(
        review::operator_release_target(&f.state, &operator, &row.id, &p.operation_id, "free text")
            .await,
        Err(AppError::ValidationError(_))
    ));
    assert!(matches!(
        review::operator_release_target(
            &f.state,
            &operator,
            &row.id,
            "another-operation",
            "INC-1812"
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(
        targets
            .find_one(doc! {"_id":&target})
            .await
            .unwrap()
            .is_some()
    );
    let released =
        review::operator_release_target(&f.state, &operator, &row.id, &p.operation_id, "INC-1812")
            .await
            .unwrap();
    assert_eq!(released["status"], "released");
    assert!(
        targets
            .find_one(doc! {"_id":&target})
            .await
            .unwrap()
            .is_none()
    );
    let after = stored(&f, &row.id).await;
    assert_eq!(after.status, "publication_failed");
    assert_eq!(after.failure_code.as_deref(), Some("operator_released"));
    let after_p = after.publication.unwrap();
    assert!(!after_p.started && !after_p.uncertain_dispatch);
    assert_eq!(after_p.last_stage.as_deref(), Some("operator_released"));
    let audit = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(
            doc! {"event_type":"assistant_learning_publication_target_released",
            "event_data.proposal_id":&row.id},
        )
        .await
        .unwrap()
        .unwrap();
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("evidence_ref").unwrap(), "INC-1812");
    assert_eq!(data.get_str("previous_state").unwrap(), "uncertain");
    // The same reviewed package may now be retried in place, or discarded.
    assert_eq!(
        card_preview(&f, &row, &used).await["actions"],
        json!(["retry", "discard"])
    );
    assert!(
        review::operator_release_target(&f.state, &operator, &row.id, &p.operation_id, "INC-1812")
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn check_of_a_missing_version_stays_uncertain_and_later_pins() {
    let (f, server, row, card, skill_id) = crash_revision("authoring_check_missing_version").await;
    let version = row.publication.as_ref().unwrap().version.clone();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .and(query_param("version", version.as_str()))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            json!({"code":"skill_version_not_found","status":404,"detail":"private detail"}),
        ))
        .mount(&server)
        .await;
    assert!(finish(&f, card.clone(), true).await.is_err());
    let used = allowed_card(&f, &card).await;
    // Ornn's "version not found" after an uncertain dispatch is not proof
    // that the request cannot still land, and never the source-skill message.
    assert!(finish(&f, used.clone(), true).await.is_err());
    let checked = stored(&f, &row.id).await;
    assert_eq!(checked.failure_code.as_deref(), Some("publish_uncertain"));
    let checked_p = checked.publication.unwrap();
    assert!(checked_p.uncertain_dispatch);
    assert_eq!(checked_p.last_registry_status, Some(404));
    assert_eq!(checked_p.last_stage.as_deref(), Some("reconcile"));
    assert!(
        card_preview(&f, &row, &used).await["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    server.verify().await;
    server.reset().await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(0)
        .mount(&server)
        .await;
    mount_version(
        &server,
        &f.owner,
        &skill_id,
        row.publication.as_ref().unwrap(),
        publication_bytes(&f, &row).await,
    )
    .await;
    assert_eq!(finish(&f, used, true).await.unwrap().status, "used");
    assert_eq!(stored(&f, &row.id).await.status, "pinned");
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn http_approve_never_executes_or_raises_authored_cards() {
    let f = fixture("authoring_http_approve_refusals").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    for (body, expected) in [
        (
            json!({"acknowledgement_id":card.id}),
            "Confirm this authored skill from its conversation card",
        ),
        (
            json!({}),
            "Confirm this authored skill from its conversation card",
        ),
        (
            json!({"acknowledgement_id":card.id,"renewal_of":card.id}),
            "renewal_of cannot be combined with acknowledgement_id",
        ),
    ] {
        let result = crate::handlers::assistant_agent_learning::approve(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((row.agent_id.clone(), row.id.clone())),
            axum::Json(serde_json::from_value(body).unwrap()),
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(message)) if message == expected));
    }
    let cards = f.state.db.collection::<AssistantAcknowledgement>(
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
    );
    let unchanged = cards
        .find_one(doc! {"_id":&card.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.status, "pending");
    assert!(unchanged.decided_at.is_none());
    assert_eq!(
        cards
            .count_documents(doc! {"authored_skill.proposal_id":&row.id})
            .await
            .unwrap(),
        1
    );
    let proposal = stored(&f, &row.id).await;
    assert_eq!(proposal.status, "pending");
    assert_eq!(proposal.failure_code, None);
    let p = proposal.publication.unwrap();
    assert_eq!(p.attempt, 0);
    assert!(p.approved_by.is_none() && !p.started);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxid_side_failure_after_base_verify_is_refused_not_registry_unavailable() {
    use crate::services::agent_skill_service::{OrnnOutcome, OrnnReader};
    struct RevisionBumpReader<'a> {
        inner: crate::handlers::agent_skills::Reader<'a>,
        db: mongodb::Database,
        agent_id: String,
        versions: String,
    }
    #[async_trait::async_trait]
    impl OrnnReader for RevisionBumpReader<'_> {
        async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
            self.classified(http::Method::GET, path, Vec::new())
                .await
                .into_app_result()
        }
        async fn classified(&self, method: http::Method, path: &str, body: Vec<u8>) -> OrnnOutcome {
            let versions = path == self.versions;
            let result = self.inner.classified(method, path, body).await;
            if versions {
                // Another pin lands while this attempt verifies the base.
                self.db
                    .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&self.agent_id},
                        doc! {"$inc":{"skills_revision":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            result
        }
    }
    let (f, server, row, card, skill_id) = crash_revision("authoring_nyxid_side_failure").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(0)
        .mount(&server)
        .await;
    let allowed = acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
        .await
        .unwrap();
    let p = row.publication.as_ref().unwrap();
    let reader = RevisionBumpReader {
        inner: crate::handlers::agent_skills::Reader {
            state: &f.state,
            person: &f.owner,
            thread_key: None,
            scopes: None,
            chat: None,
        },
        db: f.state.db.clone(),
        agent_id: row.agent_id.clone(),
        versions: format!("/api/v1/skills/{skill_id}/versions"),
    };
    assert!(
        review::approve(
            &f.state,
            &f.chat,
            &row.agent_id,
            &row.id,
            &allowed.id,
            &publication::binding(&row, p, p.skills_revision),
            &reader,
        )
        .await
        .is_err()
    );
    let refused = stored(&f, &row.id).await;
    assert_eq!(refused.status, "publication_failed");
    assert_eq!(refused.failure_code.as_deref(), Some("nyxid_refused"));
    let refused_p = refused.publication.unwrap();
    assert_eq!(refused_p.last_stage.as_deref(), Some("base_verify"));
    assert!(!refused_p.started && refused_p.lease_expires_at.is_none());
    assert!(
        f.state
            .db
            .collection::<bson::Document>(
                crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME
            )
            .find_one(doc! {"_id":format!("{skill_id}:{}", p.version)})
            .await
            .unwrap()
            .is_none()
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn verified_version_is_not_attached_after_a_base_change_and_check_stops() {
    use crate::services::agent_skill_service::{OrnnOutcome, OrnnReader};
    struct BaseSwapOnVerify<'a> {
        inner: crate::handlers::agent_skills::Reader<'a>,
        db: mongodb::Database,
        agent_id: String,
        download: String,
    }
    #[async_trait::async_trait]
    impl OrnnReader for BaseSwapOnVerify<'_> {
        async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
            self.classified(http::Method::GET, path, Vec::new())
                .await
                .into_app_result()
        }
        async fn classified(&self, method: http::Method, path: &str, body: Vec<u8>) -> OrnnOutcome {
            let download = path == self.download;
            let result = self.inner.classified(method, path, body).await;
            if download {
                // The owner replaces the source pin while the new version verifies.
                self.db
                    .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&self.agent_id},
                        doc! {"$set":{"skills":[]},"$inc":{"skills_revision":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            result
        }
    }
    let (f, server, row, card, skill_id) = crash_revision("authoring_verified_base_change").await;
    let p = row.publication.as_ref().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id}})))
        .expect(1)
        .mount(&server)
        .await;
    mount_version(
        &server,
        &f.owner,
        &skill_id,
        p,
        publication_bytes(&f, &row).await,
    )
    .await;
    let allowed = acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
        .await
        .unwrap();
    let reader = BaseSwapOnVerify {
        inner: crate::handlers::agent_skills::Reader {
            state: &f.state,
            person: &f.owner,
            thread_key: None,
            scopes: None,
            chat: None,
        },
        db: f.state.db.clone(),
        agent_id: row.agent_id.clone(),
        download: format!("/api/v1/skills/{skill_id}/versions/{}/download", p.version),
    };
    assert!(
        review::approve(
            &f.state,
            &f.chat,
            &row.agent_id,
            &row.id,
            &allowed.id,
            &publication::binding(&row, p, p.skills_revision),
            &reader,
        )
        .await
        .is_err()
    );
    let raced = stored(&f, &row.id).await;
    assert_eq!(raced.status, "published_unpinned");
    assert_eq!(raced.failure_code.as_deref(), Some("pin_conflict"));
    // A fresh confirmation at the live revision may still check the
    // dispatched operation even though its source pin is gone.
    let axum::Json(renewal) = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({"revision":row.revision,
                "agent_skills_revision":row.agent_skills_revision+1,"renewal_of":allowed.id}))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let renewed = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":renewal["acknowledgement"]["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(
        card_preview(&f, &row, &renewed).await["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("check"))
    );
    assert!(finish(&f, renewed.clone(), true).await.is_err());
    let settled = stored(&f, &row.id).await;
    assert_eq!(settled.status, "published_unpinned");
    assert_eq!(settled.failure_code.as_deref(), Some("base_changed"));
    let used = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&renewed.id})
        .await
        .unwrap()
        .unwrap();
    // Checking again would only repeat the refusal, so it is not offered.
    let preview = card_preview(&f, &row, &used).await;
    assert_eq!(preview["failure_code"], "base_changed");
    assert_eq!(preview["actions"], json!([]));
    assert!(matches!(
        finish(&f, used, true).await,
        Err(AppError::Conflict(message)) if message == WITHHELD
    ));
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn inactive_registry_service_at_validation_is_a_neutral_refusal() {
    let (f, server, row, card, skill_id) =
        crash_revision("authoring_inactive_registry_validate").await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(503))
        .expect(0)
        .mount(&server)
        .await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_many(doc! {"slug":"ornn-api"}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(finish(&f, card.clone(), true).await.is_err());
    let refused = stored(&f, &row.id).await;
    assert_eq!(refused.status, "publication_failed");
    assert_eq!(refused.failure_code.as_deref(), Some("nyxid_refused"));
    let refused_p = refused.publication.unwrap();
    assert_eq!(refused_p.last_stage.as_deref(), Some("format_validate"));
    assert_eq!(refused_p.last_registry_status, None);
    assert!(!refused_p.started && !refused_p.uncertain_dispatch);
    assert!(
        f.state
            .db
            .collection::<bson::Document>(
                crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME
            )
            .find_one(
                doc! {"_id":format!("{skill_id}:{}", row.publication.as_ref().unwrap().version)}
            )
            .await
            .unwrap()
            .is_none()
    );
    let used = allowed_card(&f, &card).await;
    assert!(
        card_preview(&f, &row, &used).await["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("retry"))
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn panel_link_to_a_group_member_thread_opens_its_authored_card() {
    let f = fixture("authoring_card_in_member_thread").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    // NyxBot drafted inside a group, so its hidden member thread holds the card.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.conversation_id},
            doc! {"$set":{"group_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    let listed = review::list(&f.state, &f.owner, &row.agent_id, false)
        .await
        .unwrap();
    let item = listed.iter().find(|item| item.id == row.id).unwrap();
    assert_eq!(
        item.card_conversation_id.as_deref(),
        Some(card.conversation_id.as_str())
    );
    // `?c=<conversation>` loads this history; the thread page renders the full
    // authored card (files and actions) for any acknowledgement in it.
    let axum::Json(history) = crate::handlers::assistant_nyxagent::history(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path(card.conversation_id.clone()),
        axum::extract::Query(Default::default()),
    )
    .await
    .unwrap();
    let history = serde_json::to_value(history).unwrap();
    let shown = history["acknowledgements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ack| ack["id"] == card.id.as_str())
        .unwrap();
    assert_eq!(shown["authored_skill"]["proposal_id"], row.id.as_str());
    assert_eq!(shown["status"], "pending");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authored_review_links_self_id_listing_and_cross_turn_reuse() {
    let mut f = fixture("authoring_review_links").await;
    let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
    let url = super::super::assistant_links::AssistantPage::ReviewCard {
        conversation: &card.conversation_id,
        card: &card.id,
    }
    .url(&f.state.config.frontend_url);
    assert!(acks::refusal(&card).get("review_url").is_none());
    assert!(url.starts_with("http://") || url.starts_with("https://"));
    assert_eq!(
        acks::refusal_with_frontend(&card, &f.state.config.frontend_url)["review_url"],
        url
    );
    assert!(
        acks::refusal(&card)["instructions"]
            .as_str()
            .unwrap()
            .contains("chat reply")
    );
    let (status, error) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "learning_status",
        &json!({"agent":f.chat.agent_id}),
    )
    .await
    .unwrap();
    assert!(!error);
    assert!(status["can_maintain"].as_bool().unwrap());
    let (listed, error) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "learning_list_proposals",
        &json!({"agent":f.chat.agent_id}),
    )
    .await
    .unwrap();
    assert!(!error);
    let item = &listed["proposals"][0];
    assert_eq!(item["acknowledgement_id"], card.id);
    assert_eq!(item["card_status"], "pending");
    assert_eq!(item["review_url"], url);
    assert!(item["card_expires_at"].is_string());
    learning::configure(&f.state.db, &f.owner, &f.chat.agent_id, true, 5)
        .await
        .unwrap();
    let (_, error) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "learning_run_now",
        &json!({"agent":f.chat.agent_id}),
    )
    .await
    .unwrap();
    assert!(!error);
    // Exact links can include cards outside the normal recent history, but never another owner/thread.
    let mut cards = Vec::new();
    acks::include_card(
        &f.state.db,
        &f.owner,
        &card.conversation_id,
        &card.id,
        &mut cards,
    )
    .await
    .unwrap();
    assert_eq!(cards.len(), 1);
    let mut hidden = Vec::new();
    acks::include_card(
        &f.state.db,
        &Uuid::new_v4().to_string(),
        &card.conversation_id,
        &card.id,
        &mut hidden,
    )
    .await
    .unwrap();
    acks::include_card(
        &f.state.db,
        &f.owner,
        "another-thread",
        &card.id,
        &mut hidden,
    )
    .await
    .unwrap();
    assert!(hidden.is_empty());
    // A later turn is the same draft, never another proposal or approval card.
    let turn = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.turn_id":&turn}},
        )
        .await
        .unwrap();
    f.chat.turn_id = Some(turn);
    let (again, same_card) = make(&f, input(&f.chat.agent_id)).await;
    assert_eq!(again.id, proposal.id);
    assert_eq!(same_card.id, card.id);
    f.state.db.collection::<bson::Document>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .update_one(doc! {"_id":&card.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}).await.unwrap();
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    let expired = create(
        &f.state,
        &f.chat,
        serde_json::from_value(input(&f.chat.agent_id)).unwrap(),
        &reader,
        &reader,
    )
    .await
    .unwrap();
    assert_eq!(expired["status"], "expired");
    assert_eq!(expired["review_url"], url);
    assert!(
        expired["instructions"]
            .as_str()
            .unwrap()
            .contains("Request a new confirmation")
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .count_documents(doc! {"authored_skill.proposal_id":&proposal.id})
            .await
            .unwrap(),
        1
    );
    let mut changed = input(&f.chat.agent_id);
    changed["skill_md"] = "# A revised procedure".into();
    let (different, _) = make(&f, changed).await;
    assert_ne!(different.id, proposal.id);
    f.state.db.drop().await.unwrap();
}

async fn another_authoring_chat(f: &Fixture) -> ChatAuthority {
    let row = super::super::assistant_authority_tests::new_orchestrator(
        &f.state,
        &f.owner,
        "Prepare the requested skill",
    )
    .await;
    assert_ne!(row.id, f.row.id);
    acks::for_key(&f.state.db, &f.owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap()
}

async fn draft_in_chat(f: &Fixture, chat: &ChatAuthority) -> AppResult<Value> {
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    create(
        &f.state,
        chat,
        parse_input(&input(&f.chat.agent_id)).unwrap(),
        &reader,
        &reader,
    )
    .await
}

#[tokio::test]
async fn concurrent_threads_share_one_draft_and_expired_review_origin() {
    let f = fixture("authoring_concurrent_threads").await;
    let other = another_authoring_chat(&f).await;
    let barrier = tokio::sync::Barrier::new(2);
    let run = async |chat: &ChatAuthority| {
        barrier.wait().await;
        draft_in_chat(&f, chat).await.unwrap()
    };
    let (first, second) = tokio::join!(run(&f.chat), run(&other));
    assert_eq!(first["proposal_id"], second["proposal_id"]);
    assert_eq!(first["acknowledgement_id"], second["acknowledgement_id"]);
    assert_eq!(first["review_url"], second["review_url"]);
    let cards = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME);
    let card = cards
        .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(
        first["review_url"]
            .as_str()
            .unwrap()
            .contains(&card.conversation_id)
    );
    cards.update_one(doc! {"_id":&card.id}, doc! {"$set":{"status":"expired", "expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}).await.unwrap();
    let (first, second) = tokio::join!(run(&f.chat), run(&other));
    for result in [first, second] {
        assert_eq!(result["status"], "expired");
        assert_eq!(result["acknowledgement_id"], card.id);
    }
    assert_eq!(
        cards
            .count_documents(doc! {"authored_skill":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn allowed_unstarted_drafts_join_the_original_publication_without_another_card() {
    let f = fixture("authoring_allowed_join").await;
    let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
    let reservation = claims::reserve(&f.state, &f.chat, &proposal).await.unwrap();
    let other = another_authoring_chat(&f).await;
    let allowed = acks::decide(&f.state.db, &f.owner, &card.conversation_id, &card.id, true)
        .await
        .unwrap();
    assert_eq!(allowed.status, "allowed");
    let (first, second) = tokio::join!(draft_in_chat(&f, &f.chat), draft_in_chat(&f, &other));
    for result in [first.unwrap(), second.unwrap()] {
        assert_eq!(result["status"], "waiting_publication");
        assert_eq!(result["proposal_id"], proposal.id);
        assert_eq!(result["acknowledgement_id"], card.id);
        assert!(
            result["review_url"]
                .as_str()
                .unwrap()
                .contains(&card.conversation_id)
        );
    }
    // A pending card allowed while another worker prepares is joined too.
    let binding = publication::binding(
        &proposal,
        proposal.publication.as_ref().unwrap(),
        proposal.agent_skills_revision,
    );
    let finalized = claims::finalize(&f.state, &reservation, &binding)
        .await
        .unwrap();
    assert_eq!(finalized.id, card.id);
    assert_eq!(finalized.status, "allowed");
    let after = stored(&f, &proposal.id).await;
    assert_eq!(
        bson::to_bson(&after.publication).unwrap(),
        bson::to_bson(&proposal.publication).unwrap()
    );
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .count_documents(doc! {"authored_skill.proposal_id":&proposal.id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn interrupted_prepared_draft_recovers_same_proposal_and_original_turn_identity() {
    let f = fixture("authoring_orphan_recovery").await;
    let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
    let cards = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME);
    cards.delete_one(doc! {"_id":&card.id}).await.unwrap();
    // Crash after publication preparation but before card insertion, followed
    // by another turn in the origin conversation and a retry in another chat.
    f.state.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.row.id},doc! {"$set":{
            "active_turn.turn_id":Uuid::new_v4().to_string(),"active_turn.trigger_run_id":"unrelated-trigger",
            "active_turn.voice_request_id":"unrelated-voice"}}).await.unwrap();
    let other = another_authoring_chat(&f).await;
    let (first, second) = tokio::join!(draft_in_chat(&f, &other), draft_in_chat(&f, &other));
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first["proposal_id"], proposal.id);
    assert_eq!(first["acknowledgement_id"], second["acknowledgement_id"]);
    assert!(first["review_url"].as_str().unwrap().contains(&f.row.id));
    let recovered = cards
        .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.requested_turn_id, card.requested_turn_id);
    assert_eq!(recovered.trigger_run_id, card.trigger_run_id);
    assert_eq!(recovered.voice_request_id, card.voice_request_id);
    assert_eq!(
        stored(&f, &proposal.id)
            .await
            .publication
            .unwrap()
            .operation_id,
        proposal.publication.unwrap().operation_id
    );
    assert_eq!(
        cards
            .count_documents(doc! {"authored_skill.proposal_id":&proposal.id})
            .await
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn obsolete_review_origins_and_started_publications_never_reuse_cards() {
    for change in [
        "revoked",
        "replaced",
        "deleted",
        "started",
        "uncertain",
        "verified",
        "lease",
    ] {
        let f = fixture("authoring_nonreuse_fences").await;
        let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
        let other = another_authoring_chat(&f).await;
        match change {
            "revoked" => {
                f.state
                    .db
                    .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&f.chat.api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "replaced" => {
                // Use the real replacement path: the encrypted credential is
                // rebound and predecessor cards expire in the same transaction.
                crate::services::key_service::rotate_api_key(
                    &f.state.db,
                    &f.state.encryption_keys,
                    &f.owner,
                    &f.chat.api_key_id,
                )
                .await
                .unwrap();
                assert!(matches!(
                    card_authority(&f.state.db, &f.owner, &card).await,
                    Err(AppError::Forbidden(_))
                ));
            }
            "deleted" => {
                f.state
                    .db
                    .collection::<bson::Document>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .delete_one(doc! {"_id":&f.row.id})
                    .await
                    .unwrap();
            }
            _ => {
                let update = match change {
                    "started" => doc! {"publication.started":true},
                    "uncertain" => doc! {"publication.uncertain_dispatch":true},
                    "verified" => doc! {"publication.verified_at":bson::DateTime::now()},
                    "lease" => {
                        doc! {"publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(5))}
                    }
                    _ => unreachable!(),
                };
                f.state
                    .db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .update_one(doc! {"_id":&proposal.id}, doc! {"$set":update})
                    .await
                    .unwrap();
            }
        }
        let result = draft_in_chat(&f, &other).await.unwrap();
        assert_ne!(result["proposal_id"], proposal.id, "{change}");
        assert_ne!(result["acknowledgement_id"], card.id, "{change}");
        assert!(
            result["review_url"]
                .as_str()
                .unwrap()
                .contains(&other.conversation_id)
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn stale_preparation_cannot_finalize_after_origin_replacement() {
    let f = fixture("authoring_generation_fence").await;
    let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .delete_one(doc! {"_id":&card.id})
        .await
        .unwrap();
    let stale = claims::reserve(&f.state, &f.chat, &proposal).await.unwrap();
    let binding = publication::binding(
        &proposal,
        proposal.publication.as_ref().unwrap(),
        proposal.agent_skills_revision,
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    let other = another_authoring_chat(&f).await;
    let current = draft_in_chat(&f, &other).await.unwrap();
    assert_ne!(current["proposal_id"], proposal.id);
    assert!(matches!(
        claims::finalize(&f.state, &stale, &binding).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .count_documents(doc! {"authored_skill.proposal_id":&proposal.id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_orphan_without_an_origin_refuses_duplicate_creation() {
    let f = fixture("authoring_legacy_orphan").await;
    let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME,
        )
        .delete_many(doc! {})
        .await
        .unwrap();
    f.state
        .db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .delete_one(doc! {"_id":&card.id})
        .await
        .unwrap();
    let other = another_authoring_chat(&f).await;
    assert!(
        matches!(draft_in_chat(&f, &other).await, Err(AppError::Conflict(message)) if message.contains("original review conversation"))
    );
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {"fingerprint":&proposal.fingerprint})
            .await
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn destroyed_or_deleted_target_fences_stale_reservation_and_finalization() {
    for deleted in [false, true] {
        let f = fixture("authoring_target_fence").await;
        let (proposal, card) = make(&f, input(&f.chat.agent_id)).await;
        f.state
            .db
            .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .delete_one(doc! {"_id":&card.id})
            .await
            .unwrap();
        let stale = claims::reserve(&f.state, &f.chat, &proposal).await.unwrap();
        let binding = publication::binding(
            &proposal,
            proposal.publication.as_ref().unwrap(),
            proposal.agent_skills_revision,
        );
        let agents = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME);
        if deleted {
            agents
                .delete_one(doc! {"_id":&proposal.agent_id})
                .await
                .unwrap();
        } else {
            agents
                .update_one(
                    doc! {"_id":&proposal.agent_id},
                    doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
                )
                .await
                .unwrap();
        }
        let claims = f.state.db.collection::<bson::Document>(
            crate::models::assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME,
        );
        claims
            .delete_many(doc! {"agent_id":&proposal.agent_id})
            .await
            .unwrap();
        assert!(matches!(
            claims::reserve(&f.state, &f.chat, &proposal).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            claims::finalize(&f.state, &stale, &binding).await,
            Err(AppError::NotFound(_))
        ));
        assert_eq!(claims.count_documents(doc! {}).await.unwrap(), 0);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn owner_edit_starts_an_improve_draft_from_the_exact_pinned_package() {
    let f = fixture("authoring_owner_improve").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &row, &id, "POST").await;
    finish(&f, card, true).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    // An unknown pin is not found and creates nothing.
    let missing = create_improve_draft(
        &f.state,
        &f.chat,
        &f.chat.agent_id,
        &Uuid::new_v4().to_string(),
        &reader,
        &reader,
    )
    .await
    .unwrap_err();
    assert!(matches!(&missing, AppError::NotFound(m) if m == "Attached skill not found"));
    let result = create_improve_draft(&f.state, &f.chat, &f.chat.agent_id, &id, &reader, &reader)
        .await
        .unwrap();
    assert_eq!(result["error"], "acknowledgement_required", "{result}");
    assert!(result["acknowledgement_id"].as_str().is_some());
    let proposal: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":result["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proposal.source, ProposalSource::Authored);
    assert_eq!(proposal.status, "pending");
    assert!(proposal.scope_encrypted.is_none());
    assert!(proposal.scope_digest.is_none());
    let encoded = f
        .state
        .encryption_keys
        .decrypt(&proposal.body_encrypted)
        .await
        .unwrap();
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(f.state.audit_chain_hmac_key.as_slice()).unwrap();
    mac.update(b"nyxid-authored-skill-v1\0");
    mac.update(proposal.agent_id.as_bytes());
    mac.update(&encoded);
    assert_eq!(
        proposal.fingerprint,
        hex::encode(mac.finalize().into_bytes())
    );
    let body: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&proposal.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body.kind, "improve");
    assert_eq!(body.name, "weekly-review");
    let base = body.base_skill.as_ref().unwrap();
    assert_eq!(base.skill_id, id);
    assert_eq!(base.version, "1.0");
    assert_eq!(base.sha256, row.publication.as_ref().unwrap().sha256);
    // The editable guidance is the published body without its frontmatter,
    // and the package's other text files come along.
    assert_eq!(
        body.skill_md,
        input(&f.chat.agent_id)["skill_md"].as_str().unwrap()
    );
    assert_eq!(body.files.len(), 1);
    assert_eq!(body.files[0].path, "references/checklist.md");
    assert_eq!(body.files[0].content, "Verify dates before publishing.");
    assert!(!body.description.is_empty());
    // Repeating the same edit reuses the pending draft.
    let again = create_improve_draft(&f.state, &f.chat, &f.chat.agent_id, &id, &reader, &reader)
        .await
        .unwrap();
    assert_eq!(again["proposal_id"], result["proposal_id"]);
    let mut edited = body.clone();
    edited.skill_md.push_str("\nCheck the revised schedule.");
    review::edit(
        &f.state,
        &f.owner,
        &proposal.agent_id,
        &proposal.id,
        proposal.revision,
        &serde_json::to_value(&edited).unwrap(),
    )
    .await
    .unwrap();
    let saved: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&proposal.id})
        .await
        .unwrap()
        .unwrap();
    let encoded = f
        .state
        .encryption_keys
        .decrypt(&saved.body_encrypted)
        .await
        .unwrap();
    let expected =
        crate::services::assistant_action_receipts::fingerprint_sensitive_material(&format!(
            "{}:{}:{}",
            saved.agent_id,
            saved.model_contract,
            String::from_utf8_lossy(&encoded)
        ));
    assert_eq!(saved.fingerprint, expected);
    assert!(saved.scope_encrypted.is_none() && saved.scope_digest.is_none());
    // "Test" reads the pin the way the agent does; a disabled pin is unreadable.
    let owner = f.owner.clone();
    let test = |state: crate::AppState, agent: String, skill: String| {
        let owner = owner.clone();
        async move {
            crate::handlers::agent_skills::preview_attached(
                axum::extract::State(state),
                crate::test_utils::test_auth_user(&owner),
                axum::extract::Path((agent, skill)),
            )
            .await
            .map(|axum::Json(body)| body)
        }
    };
    let live = test(f.state.clone(), f.chat.agent_id.clone(), id.clone())
        .await
        .unwrap();
    assert!(live.readable);
    assert!(!live.disabled);
    assert!(live.excerpt.contains("Check outstanding work."), "{live:?}");
    assert_eq!(live.version, "1.0");
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.disabled":true}},
        )
        .await
        .unwrap();
    let off = test(f.state.clone(), f.chat.agent_id.clone(), id.clone())
        .await
        .unwrap();
    assert!(off.disabled);
    assert!(!off.readable);
    assert!(off.excerpt.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn editing_an_authored_draft_raises_a_fresh_card_that_publishes_the_edit() {
    let f = fixture("authoring_edit_new_card").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let draft = json!({"schema_version":1,"kind":"new","name":"weekly-review",
        "description":"Prepare a weekly review","skill_md":"# Weekly review\nEdited by the owner.",
        "files":[],"base_skill":null,"rationale":"","safety_notes":""});
    let axum::Json(edited) = crate::handlers::assistant_agent_learning::edit(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.chat.agent_id.clone(), row.id.clone())),
        axum::Json(serde_json::from_value(json!({"revision":0,"draft":draft})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(edited["status"], "updated");
    assert_eq!(edited["conversation_id"], f.chat.conversation_id);
    let fresh_id = edited["acknowledgement_id"].as_str().unwrap().to_owned();
    assert_ne!(fresh_id, card.id);
    let fresh: AssistantAcknowledgement = f
        .state
        .db
        .collection(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":&fresh_id})
        .await
        .unwrap()
        .unwrap();
    let reference = fresh.authored_skill.as_ref().unwrap();
    assert_eq!(reference.proposal_id, row.id);
    assert_eq!(reference.revision, 1);
    // The fresh card publishes the edited revision.
    let saved: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.revision, 1);
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &saved, &id, "POST").await;
    let decided = finish(&f, fresh, true).await.unwrap();
    assert_eq!(decided.status, "used");
    let agent: AssistantAgent = f
        .state
        .db
        .collection(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.skills.len(), 1);
    assert_eq!(agent.skills[0].skill_id, id);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ornn_format_verdict_refuses_the_draft_before_review() {
    let f = fixture("authoring_ornn_preflight").await;
    let server = setup_ornn(&f).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"valid":false,
            "violations":[{"rule":"allowed-root-items","message":"Root item \"private-notes.md\" is not allowed."}]}})))
        .expect(1)
        .mount(&server)
        .await;
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    let error = create(
        &f.state,
        &f.chat,
        serde_json::from_value(input(&f.chat.agent_id)).unwrap(),
        &reader,
        &reader,
    )
    .await
    .unwrap_err();
    let AppError::SkillDraftValidation(diagnostic) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(diagnostic.rule, "ornn_format_rejected");
    assert_eq!(diagnostic.field, "files");
    assert_eq!(diagnostic.ornn_rules, ["allowed-root-items"]);
    // Ornn's message (which can quote filenames) never reaches the author.
    assert!(!format!("{:?}", error.response_body().details).contains("private-notes"));
    for collection in [
        PROPOSALS_COLLECTION_NAME,
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn top_level_files_are_reviewed_and_packaged_under_references() {
    let f = fixture("authoring_reference_paths").await;
    let server = setup_ornn(&f).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    let mut value = input(&f.chat.agent_id);
    value["files"] = json!([
        {"path":"faq.md","content":"Opening hours are 9 to 5."},
        {"path":"assets/notes.txt","content":"Plain notes."}
    ]);
    let (row, _card) = make(&f, value).await;
    let draft: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&row.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let paths: Vec<_> = draft.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["references/faq.md", "assets/notes.txt"]);
    let archive =
        publication::package_with_snapshot(&draft, "op", "weekly-review", "1.0", None).unwrap();
    let names: Vec<_> = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .unwrap()
        .file_names()
        .map(str::to_owned)
        .collect();
    assert!(names.contains(&"weekly-review/references/faq.md".to_owned()));
    assert!(names.contains(&"weekly-review/assets/notes.txt".to_owned()));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ornn_field_rules_are_checked_when_authoring() {
    let f = fixture("authoring_ornn_field_rules").await;
    for (field, value, rule) in [
        (
            "description",
            "Answers for <VIP> customers",
            "angle_brackets",
        ),
        ("name", "claude-helper", "reserved_name"),
    ] {
        let mut draft = input(&f.chat.agent_id);
        draft[field] = json!(value);
        let error = create(
            &f.state,
            &f.chat,
            serde_json::from_value(draft).unwrap(),
            &review::UnavailableReader,
            &review::UnavailableReader,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, AppError::SkillDraftValidation(d) if d.rule == rule && d.field == field),
            "{error:?}"
        );
    }
    f.state.db.drop().await.unwrap();
}

/// A specialist drafts its own skill in the default (denied) access mode; the
/// owner's approval publishes and pins it to that specialist.
#[tokio::test]
async fn specialist_drafts_and_owner_approval_publishes_its_own_skill() {
    let f =
        crate::services::assistant_authority_tests::fixture("authoring_specialist_publish").await;
    assert!(!f.chat.is_orchestrator());
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"specialist_access_requests":"denied"}},
        )
        .await
        .unwrap();
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    feature_flag_service::set_platform_override(
        &f.state.db,
        learning::FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let server = setup_ornn(&f).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"name": "Research \"quoted\"\nteam"}},
        )
        .await
        .unwrap();
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    assert_eq!(row.agent_id, f.chat.agent_id);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(preflight_request),
        "the specialist's draft gets Ornn's pre-review check"
    );
    let id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &row, &id, "POST").await;
    assert_eq!(finish(&f, card, true).await.unwrap().status, "used");
    let agent = f
        .state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert!(agent.skills.iter().any(|pin| pin.skill_id == id));
    let thread = f
        .state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &f.row.id})
        .await
        .unwrap()
        .unwrap();
    let event = thread
        .pending_events
        .iter()
        .find(|e| e.kind == "skill_publication")
        .unwrap();
    assert!(
        event
            .text
            .contains(&super::super::assistant_nyxagent::identifier(&agent.name))
    );
    assert!(!event.text.contains(&agent.name));
    assert!(event.text.contains(&format!("agent_id {}", agent.id)));
    assert!(!event.text.contains(['"', '\n']));
    assert!(event.text.contains("1.0"));
    assert!(event.text.contains(acks::DECISION_NARRATION));

    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_source_read_timeout_keeps_retryable_reservation_without_a_card() {
    let f = fixture("authoring_snapshot_approval_timeout").await;
    let server = setup_ornn(&f).await;
    let skill_id = Uuid::new_v4().to_string();
    let pin = crate::models::catalog_skill_revision::SkillReference {
        disabled: false,
        source: "ornn".into(),
        skill_id: skill_id.clone(),
        name: "weekly-review".into(),
        version: "1.0".into(),
        sha256: "a".repeat(64),
        dependencies: vec![],
    };
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills":bson::to_bson(&vec![pin]).unwrap()}},
        )
        .await
        .unwrap();
    let service = f
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! {"slug":"ornn-api"})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: service.id.clone(),
                service_name: service.name.clone(),
                approval_required: true,
                approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                rules: vec![],
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    let mut value = input(&f.chat.agent_id);
    value["base_skill"] = json!({"skill_id":skill_id,"version":"1.0"});
    let result = create(
        &f.state,
        &f.chat,
        serde_json::from_value(value).unwrap(),
        &reader,
        &reader,
    )
    .await;
    assert!(
        matches!(result, Err(AppError::Conflict(message)) if message.starts_with("source_skill_unavailable:"))
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {"source":"authored"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&f.owner,"authored_skill":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&f.owner,"service_id":&service.id,"status":"pending"})
            .await
            .unwrap(),
        1
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn preview_marks_only_missing_format_details_and_sanitizes_stored_rules() {
    let f = fixture("authoring_preview_format_details").await;
    let (row, _) = make(&f, input(&f.chat.agent_id)).await;
    for (code, rules, unavailable) in [
        ("ornn_validation_failed", vec![], true),
        ("ornn_package_rejected", vec![], true),
        ("ornn_validation_failed", vec!["allowed_root_items"], false),
        (
            "ornn_validation_failed",
            vec!["nyxid_ag_sentinel_secret", "unknown_rule"],
            true,
        ),
        ("base_changed", vec![], false),
        ("base_verify_failed", vec![], false),
        ("base_interface_incompatible", vec![], false),
        ("ornn_interface_change_requires_major", vec![], false),
        ("ornn_dependency_rejected", vec![], false),
        ("ornn_name_taken", vec![], false),
    ] {
        f.state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"failure_code": code, "failure_rules": &rules, "publication.last_stage": "format_validate"}},
            )
            .await
            .unwrap();
        let preview = review::authored_preview(&f.state, &f.owner, &row.agent_id, &row.id)
            .await
            .unwrap();
        assert_eq!(
            preview["failure_details_unavailable"], unavailable,
            "{code}"
        );
        assert!(!preview.to_string().contains("sentinel_secret"));
        assert!(!preview.to_string().contains("unknown_rule"));
        assert_eq!(preview["failure_stage"], "format_validate");
    }
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"publication.last_stage": "nyxid_ag_sentinel_secret"}},
        )
        .await
        .unwrap();
    let preview = review::authored_preview(&f.state, &f.owner, &row.agent_id, &row.id)
        .await
        .unwrap();
    assert!(preview["failure_stage"].is_null());
    assert!(!preview.to_string().contains("sentinel_secret"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn scope_identity_acceptance_revision_and_durable_repair_cap() {
    let f = fixture("authoring_scope_cap").await;
    let mut value = input(&f.chat.agent_id);
    value["scope"] = json!({"status":"reduced","omissions":["An older source was omitted."]});
    let result = create(
        &f.state,
        &f.chat,
        parse_input(&value).unwrap(),
        &review::UnavailableReader,
        &review::UnavailableReader,
    )
    .await
    .unwrap();
    let id = result["proposal_id"].as_str().unwrap();
    let row: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let p = row.publication.as_ref().unwrap();
    let binding = publication::binding(&row, p, p.skills_revision);
    // An old replica recomputes the legacy shape, which cannot consume this card.
    let mut old_row = row.clone();
    old_row.scope_digest = None;
    old_row.scope_encrypted = None;
    let old_binding = publication::binding(&old_row, p, p.skills_revision);
    assert!(old_binding.get("scope_digest").is_none());
    assert_ne!(
        acks::arguments_digest(&old_binding),
        acks::arguments_digest(&binding)
    );
    let card_id = result["acknowledgement_id"].as_str().unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":card_id},
            doc! {"$set":{"status":"allowed","decider":"user","decided_by":"user"}},
        )
        .await
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        !acks::consume_action_in_session(
            &f.state.db,
            &f.chat,
            card_id,
            review::TOOL,
            &old_binding,
            &mut session
        )
        .await
        .unwrap()
    );
    assert!(
        acks::consume_action_in_session(
            &f.state.db,
            &f.chat,
            card_id,
            review::TOOL,
            &binding,
            &mut session
        )
        .await
        .unwrap()
    );
    session.abort_transaction().await.unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(doc! {"_id":card_id},doc! {"$set":{"status":"pending","decider":bson::Bson::Null,"decided_by":bson::Bson::Null}}).await.unwrap();
    assert!(
        review::accept_scope(&f.state, &f.owner, &row.agent_id, id, &binding, false)
            .await
            .is_err()
    );
    review::accept_scope(&f.state, &f.owner, &row.agent_id, id, &binding, true)
        .await
        .unwrap();
    review::accept_scope(&f.state, &f.owner, &row.agent_id, id, &binding, false)
        .await
        .unwrap();
    let accepted: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        accepted
            .publication
            .unwrap()
            .reduced_scope_acceptance
            .unwrap()
            .omission_count,
        1
    );
    // A renewed binding cannot reuse consent, but explicit consent can replace it.
    let current_skills_revision = p.skills_revision + 1;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id},
            doc! {"$set":{"skills_revision":current_skills_revision}},
        )
        .await
        .unwrap();
    let renewed_binding = publication::binding(&row, p, current_skills_revision);
    assert!(
        review::accept_scope(
            &f.state,
            &f.owner,
            &row.agent_id,
            id,
            &renewed_binding,
            false
        )
        .await
        .is_err()
    );
    review::accept_scope(
        &f.state,
        &f.owner,
        &row.agent_id,
        id,
        &renewed_binding,
        true,
    )
    .await
    .unwrap();
    let renewed: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        renewed
            .publication
            .unwrap()
            .reduced_scope_acceptance
            .unwrap()
            .approval_digest,
        acks::arguments_digest(&renewed_binding)
    );
    value["scope"] = json!({"status":"complete","omissions":[]});
    let complete = create(
        &f.state,
        &f.chat,
        parse_input(&value).unwrap(),
        &review::UnavailableReader,
        &review::UnavailableReader,
    )
    .await
    .unwrap();
    assert_ne!(complete["proposal_id"], result["proposal_id"]);
    let body: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&row.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    review::edit(
        &f.state,
        &f.owner,
        &row.agent_id,
        id,
        row.revision,
        &serde_json::to_value(&body).unwrap(),
    )
    .await
    .unwrap();
    let edited: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert!(edited.publication.is_none());
    assert_eq!(edited.revision, row.revision + 1);
    let prepared = review::approval_binding_typed(
        &f.state,
        &f.owner,
        &row.agent_id,
        id,
        edited.revision,
        current_skills_revision,
        &review::UnavailableReader,
    )
    .await
    .map_err(|failure| failure.error)
    .unwrap();
    assert!(
        review::accept_scope(&f.state, &f.owner, &row.agent_id, id, &prepared, false)
            .await
            .is_err()
    );
    let (a, b, c) = tokio::join!(
        record_format_rejection(&f.state.db, &f.chat),
        record_format_rejection(&f.state.db, &f.chat),
        record_format_rejection(&f.state.db, &f.chat)
    );
    let mut counts = vec![a.unwrap(), b.unwrap(), c.unwrap()];
    counts.sort();
    assert_eq!(counts, vec![1, 2, 3]);
    // Legacy conversation replacement cannot touch a counter in its own collection.
    let conversation = super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .replace_one(doc! {"_id":&f.row.id}, conversation)
        .await
        .unwrap();
    assert!(require_repair_budget(&f.state.db, &f.chat).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn reduced_scope_publication_requires_consent_and_pins_with_metadata_audit() {
    let f = fixture("authoring_reduced_publication").await;
    let server = setup_ornn(&f).await;
    let mut value = input(&f.chat.agent_id);
    value["scope"] =
        json!({"status":"reduced","omissions":["The historical appendix was omitted."]});
    let (row, card) = make(&f, value).await;
    let skill_id = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &row, &skill_id, "POST").await;
    assert!(finish(&f, card.clone(), true).await.is_err());
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() != "/api/v1/skills")
    );
    let card: AssistantAcknowledgement = f
        .state
        .db
        .collection(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":&card.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.status, "allowed");
    crate::handlers::assistant_agent_learning::decide_authored_with_scope(
        &f.state,
        &crate::test_utils::test_auth_user(&f.owner),
        card.clone(),
        true,
        true,
    )
    .await
    .unwrap();
    let stored: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "pinned");
    assert_eq!(
        stored
            .publication
            .unwrap()
            .reduced_scope_acceptance
            .unwrap()
            .omission_count,
        1
    );
    let audits: Vec<bson::Document> = f
        .state
        .db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"assistant_skill_reduced_scope_accepted","event_data.proposal_id":&row.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(audits.len(), 1);
    assert!(
        !serde_json::to_string(&audits)
            .unwrap()
            .contains("historical appendix")
    );
    let thread = super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let notifications: Vec<_> = thread
        .pending_events
        .iter()
        .filter(|event| event.kind == "skill_publication")
        .collect();
    assert_eq!(notifications.len(), 1);
    assert!(
        notifications[0]
            .text
            .contains("published and attached; not yet tested")
    );
    assert!(
        crate::handlers::assistant_agent_learning::decide_authored_with_scope(
            &f.state,
            &crate::test_utils::test_auth_user(&f.owner),
            card,
            true,
            true
        )
        .await
        .is_err()
    );
    let after = super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(
        after
            .pending_events
            .iter()
            .filter(|event| event.kind == "skill_publication")
            .count(),
        1
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ornn_preflight_rules_are_safe_no_card_and_count_toward_repair_cap() {
    let f = fixture("authoring_preflight_cap").await;
    let server = setup_ornn(&f).await;
    Mock::given(method("POST")).and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"valid":false,"violations":[{"rule":"frontmatter.description","message":"PRIVATE_SENTINEL"},{"rule":"no-readme-md","value":"SECRET_SENTINEL"}]}})))
        .expect(3).mount(&server).await;
    for attempt in 1..=3 {
        let reader = crate::handlers::agent_skills::Reader {
            state: &f.state,
            person: &f.owner,
            thread_key: Some(&f.chat.api_key_id),
            scopes: None,
            chat: Some(std::sync::Arc::new(f.chat.clone())),
        };
        let error = create(
            &f.state,
            &f.chat,
            parse_input(&input(&f.chat.agent_id)).unwrap(),
            &reader,
            &reader,
        )
        .await
        .unwrap_err();
        let AppError::SkillDraftValidation(diagnostic) = error else {
            panic!("Expected caller-safe validation");
        };
        assert_eq!(
            diagnostic.rule,
            if attempt == 3 {
                "repair_cap_report_remaining_rules_to_owner"
            } else {
                "ornn_format_rejected"
            }
        );
        assert!(
            diagnostic
                .ornn_rules
                .iter()
                .any(|rule| rule == "frontmatter.description")
        );
        assert!(
            diagnostic
                .ornn_rules
                .iter()
                .any(|rule| rule == "no-readme-md")
        );
        assert!(
            serde_json::to_value(&diagnostic)
                .unwrap()
                .get("rules")
                .is_none()
        );
        let text = serde_json::to_string(&diagnostic).unwrap();
        assert!(!text.contains("SENTINEL"));
    }
    assert!(
        crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "draft_agent_skill",
            &input(&f.chat.agent_id)
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {"source":"authored"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {"authored_skill":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn complete_draft_card_observes_a_fully_read_omitted_upload() {
    use super::super::assistant_upload_service as uploads;
    let f = fixture("authoring_coverage_observation").await;
    let attachment = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.chat.conversation_id,
        "source.txt",
        b"Full source text".to_vec(),
    )
    .await
    .unwrap();
    let message: crate::models::assistant_message::AssistantMessage = f
        .state
        .db
        .collection(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"conversation_id":&f.chat.conversation_id,"role":"user"})
        .await
        .unwrap()
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let metadata = uploads::bind(
        &f.state.db,
        &f.owner,
        &f.chat.conversation_id,
        &message.id,
        std::slice::from_ref(&attachment.id),
        &mut session,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&message.id},
            doc! {"$set":{"attachments":bson::to_bson(&metadata).unwrap()}},
        )
        .session(&mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let page = uploads::read(
        &f.state.db,
        &f.state.encryption_keys,
        &f.chat,
        &json!({"attachment_id":attachment.id}),
    )
    .await
    .unwrap();
    assert_eq!(page["complete"], true);
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(preview["scope"]["status"], "complete");
    assert_eq!(preview["coverage"][0]["id"], attachment.id);
    assert_eq!(preview["coverage"][0]["percent"], 100);
    assert_eq!(preview["coverage"][0]["lower_bound"], false);
    let collection = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    let stored = collection
        .find_one(doc! {"_id":&attachment.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.get_i64("extracted_chars").unwrap(),
        "Full source text".chars().count() as i64
    );
    collection
        .update_one(
            doc! {"_id":&attachment.id},
            doc! {"$unset":{"extracted_chars":""}},
        )
        .await
        .unwrap();
    let legacy =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(legacy["coverage"][0]["percent"], 100);
    assert_eq!(
        collection
            .find_one(doc! {"_id":&attachment.id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("extracted_chars")
            .unwrap(),
        "Full source text".chars().count() as i64
    );
    collection.update_one(doc! {"_id":&attachment.id}, doc! {"$set":{"text_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0,1,2]}}}).await.unwrap();
    let cached =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(cached["coverage"][0]["percent"], 100);

    assert!(preview["files"].as_array().unwrap().iter().all(|file| {
        !file["content"]
            .as_str()
            .unwrap()
            .contains("Full source text")
    }));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn repair_budget_counts_only_package_format_rejections() {
    let f = fixture("authoring_format_only_cap").await;
    let valid = input(&f.chat.agent_id);
    let mut size = valid.clone();
    size["skill_md"] = "x".repeat(7501).into();
    let mut credential = valid.clone();
    credential["skill_md"] = "nyxid_ag_private_material".into();
    let mut scope = valid.clone();
    scope["scope"]["status"] = "invalid".into();
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("scope");
    let skill = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.chat.agent_id}, doc! {"$set":{"skills":[{"source":"ornn","skill_id":&skill,"name":"other-name","version":"1.0","sha256":"a".repeat(64),"dependencies":[]}]}}).await.unwrap();
    let mut base = valid.clone();
    base["base_skill"] = json!({"skill_id":skill,"version":"1.0"});
    for value in [size, credential, scope, missing, base] {
        let error =
            crate::handlers::agent_skills::dispatch(&f.state, &f.chat, "draft_agent_skill", &value)
                .await
                .unwrap_err();
        assert!(matches!(error, AppError::SkillDraftValidation(_)));
    }
    let attempts = f.state.db.collection::<bson::Document>(
        crate::models::assistant_skill_draft_attempt::COLLECTION_NAME,
    );
    assert_eq!(attempts.count_documents(doc! {}).await.unwrap(), 0);
    let format = valid;
    let server = setup_ornn(&f).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"valid":false,"violations":[{"rule":"allowed-root-items"}]}}),
        ))
        .with_priority(1)
        .expect(3)
        .mount(&server)
        .await;
    for attempt in 1..=3 {
        let error = crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "draft_agent_skill",
            &format,
        )
        .await
        .unwrap_err();
        let AppError::SkillDraftValidation(diagnostic) = error else {
            panic!("Expected validation")
        };
        assert_eq!(
            diagnostic.rule,
            if attempt == 3 {
                "repair_cap_report_remaining_rules_to_owner"
            } else {
                "ornn_format_rejected"
            }
        );
        assert_eq!(
            attempts
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .get_i32("rejections")
                .unwrap(),
            attempt
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn unavailable_scope_preview_distinguishes_decryption_parse_and_legacy_rows() {
    let f = fixture("authoring_scope_unavailable").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    for bytes in [
        vec![0, 1, 2],
        f.state
            .encryption_keys
            .encrypt(b"invalid JSON")
            .await
            .unwrap(),
    ] {
        f.state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .update_one(doc! {"_id":&row.id}, doc! {"$set":{"scope_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes}}}).await.unwrap();
        let preview =
            review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
                .await
                .unwrap();
        assert_eq!(preview["scope_unavailable"], true);
        assert!(preview["scope"].is_null());
    }
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$unset":{"scope_encrypted":"","scope_digest":""}},
        )
        .await
        .unwrap();
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(preview["scope_unavailable"], false);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn relocated_authored_package_records_postapproval_failure_and_clears_rules_after_edit() {
    let f = fixture("authoring_legacy_root_preclaim").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    let mut body: GeneratedProposal = serde_json::from_slice(
        &f.state
            .encryption_keys
            .decrypt(&row.body_encrypted)
            .await
            .unwrap(),
    )
    .unwrap();
    let server = setup_ornn(&f).await;
    // The root path is relocated before review, preserving main's released contract.
    assert_eq!(
        publication::package_path("legacy.md"),
        "references/legacy.md"
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"valid":false,"violations":[{"rule":"allowed-root-items"}]}}),
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    let preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert!(
        preview["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "references/checklist.md")
    );
    assert!(matches!(
        finish(&f, card.clone(), true).await,
        Err(AppError::Conflict(code)) if code == "ornn_validation_failed"
    ));
    let failed = stored(&f, &row.id).await;
    assert_eq!(
        failed.failure_code.as_deref(),
        Some("ornn_validation_failed")
    );
    assert_eq!(failed.failure_rules, vec!["allowed_root_items"]);
    let failed_preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(failed_preview["failure_stage"], "format_validate");
    assert_eq!(
        failed.publication.unwrap().last_stage.as_deref(),
        Some("format_validate")
    );
    // A retry cannot create a second repair notification for the same transition.
    assert!(finish(&f, card, true).await.is_err());
    let thread = super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let events: Vec<_> = thread
        .pending_events
        .iter()
        .filter(|event| event.kind == "skill_publication")
        .collect();
    assert_eq!(events.len(), 1);
    assert!(events[0].text.contains("allowed_root_items"));

    body.files[0].path = "references/corrected.md".into();
    review::edit(
        &f.state,
        &f.owner,
        &row.agent_id,
        &row.id,
        row.revision,
        &serde_json::to_value(&body).unwrap(),
    )
    .await
    .unwrap();
    let edited = stored(&f, &row.id).await;
    assert!(edited.failure_code.is_none());
    assert!(edited.failure_rules.is_empty());
    assert!(edited.publication.is_none());
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    let binding = review::approval_binding_typed(
        &f.state,
        &f.owner,
        &row.agent_id,
        &row.id,
        edited.revision,
        edited.agent_skills_revision,
        &reader,
    )
    .await
    .map_err(|failure| failure.error)
    .unwrap();
    let new_card = acks::request(
        &f.state.db,
        &f.chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some(review::TOOL),
            arguments: Some(&binding),
            summary: "Review corrected draft",
            platform: false,
        },
    )
    .await
    .unwrap();
    let preview = review::authored_preview(&f.state, &f.owner, &row.agent_id, &row.id)
        .await
        .unwrap();
    let card_preview =
        review::authored_preview_for_card(&f.state, &f.owner, &row.agent_id, &row.id, &new_card.id)
            .await
            .unwrap();
    for value in [preview, card_preview] {
        assert!(value["failure_code"].is_null());
        assert_eq!(value["failure_rules"], json!([]));
        assert!(value["failure_stage"].is_null());
    }
    let listed = review::list(&f.state, &f.owner, &row.agent_id, true)
        .await
        .unwrap();
    let item = listed.iter().find(|item| item.id == row.id).unwrap();
    assert!(item.failure_code.is_none());
    assert!(item.failure_rules.is_empty());
    assert!(item.failure_stage.is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn format_failure_preserves_rules_in_pre_claim_transition() {
    let f = fixture("authoring_preflight_binding_rules").await;
    let server = setup_ornn(&f).await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
    // Exercise the preflight binding used before an operation is prepared.
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication":bson::Bson::Null}},
        )
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"valid":false,"violations":[{"rule":"frontmatter.metadata.tag"}]}}),
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    let error = publication::validate(&reader, b"package")
        .await
        .unwrap_err();
    let failure = review::BindingFailure::from_publication(error);
    assert_eq!(
        failure.code,
        Some(review::PublicationFailureCode::OrnnValidationFailed)
    );
    assert_eq!(failure.rules, vec!["frontmatter_tag"]);
    // Restore the bound legacy operation and persist the same typed failure.
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication":bson::to_bson(&row.publication).unwrap()}},
        )
        .await
        .unwrap();
    review::record_pre_claim_failure(
        &f.state,
        &f.owner,
        card.authored_skill.as_ref().unwrap(),
        &card,
        0,
        failure.code.unwrap(),
        &failure.rules,
    )
    .await
    .unwrap();
    let failed = stored(&f, &row.id).await;
    assert_eq!(failed.failure_rules, vec!["frontmatter_tag"]);
    let thread = super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert!(
        thread.pending_events.iter().any(
            |event| event.kind == "skill_publication" && event.text.contains("frontmatter_tag")
        )
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_authored_skill_publishes_and_pins_with_receipt_while_auto_reviews() {
    use crate::models::assistant_settings::AccessMode;
    for (mode, asks, reduced) in [
        (AccessMode::Autopilot, false, false),
        (AccessMode::Nyxmode, true, false),
        (AccessMode::Autopilot, false, true),
        (AccessMode::Nyxmode, true, true),
        (AccessMode::AskEveryTime, true, true),
    ] {
        let f = fixture("mode_authored_publication").await;
        crate::services::assistant_settings_service::update(
            &f.state.db,
            &f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(mode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let server = setup_ornn(&f).await;
        let skill_id = Uuid::new_v4().to_string();
        let archive = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let uploaded = archive.clone();
        let published_id = skill_id.clone();
        Mock::given(method("POST"))
            .and(path("/api/v1/skill-format/validate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/skills"))
            .respond_with(move |request: &wiremock::Request| {
                *uploaded.lock().unwrap() = request.body.clone();
                ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":published_id}}))
            })
            .mount(&server)
            .await;
        let downloaded = archive.clone();
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/skills/{skill_id}/versions/1.0/download"
            )))
            .respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_bytes(downloaded.lock().unwrap().clone())
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/skills/{skill_id}/closure")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
            .mount(&server)
            .await;
        let verified = archive.clone();
        let returned_id = skill_id.clone();
        let owner = f.owner.clone();
        Mock::given(method("GET")).and(path(format!("/api/v1/skills/{skill_id}"))).respond_with(move |_:&wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":returned_id,"name":"weekly-review","version":"1.0","skillHash":publication::hash(&verified.lock().unwrap()),"description":"Weekly review","isPrivate":true,"createdBy":owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[],"metadata":{"category":"plain"}}}))
        }).mount(&server).await;
        let reader = crate::handlers::agent_skills::Reader {
            state: &f.state,
            person: &f.owner,
            thread_key: Some(&f.chat.api_key_id),
            scopes: None,
            chat: Some(std::sync::Arc::new(f.chat.clone())),
        };
        let mut value = input(&f.chat.agent_id);
        if reduced {
            value["scope"] = json!({"status":"reduced","omissions":["SOURCE_SENTINEL_ALPHA historical imports", "SOURCE_SENTINEL_BETA archived notes"]});
        }
        let result = Box::pin(create(
            &f.state,
            &f.chat,
            parse_input(&value).unwrap(),
            &reader,
            &reader,
        ))
        .await
        .unwrap();
        let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let agent = f
            .state
            .db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.chat.agent_id})
            .await
            .unwrap()
            .unwrap();
        if asks {
            assert_eq!(result["error"], "acknowledgement_required");
            assert!(result["review_url"].is_string());
            assert_eq!(rows[0].status, "pending");
            assert!(agent.skills.is_empty());
            assert!(archive.lock().unwrap().is_empty());
        } else {
            assert_eq!(result["status"], "pinned");
            assert!(result.get("confirm_phrase").is_none());
            assert!(result.get("review_url").is_none());
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].decided_by.as_deref(), Some("access_mode"));
            assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "access_mode");
            assert!(rows[0].summary.contains("weekly-review 1.0"));
            assert!(rows[0].summary.contains(&agent.name));
            assert!(!rows[0].summary.contains("http"));
            if reduced {
                assert!(rows[0].summary.contains("2 declared omission(s)"));
                let receipt = serde_json::to_string(&rows[0]).unwrap();
                assert!(!receipt.contains("SOURCE_SENTINEL_ALPHA"));
                assert!(!receipt.contains("SOURCE_SENTINEL_BETA"));
                for omission in value["scope"]["omissions"].as_array().unwrap() {
                    assert!(!receipt.contains(omission.as_str().unwrap()));
                }
                let audits: Vec<bson::Document> = f
                    .state
                    .db
                    .collection(crate::models::audit_log::COLLECTION_NAME)
                    .find(doc! {})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                let thread =
                    super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
                        .await
                        .unwrap();
                let plaintext = format!(
                    "{}{}",
                    serde_json::to_string(&audits).unwrap(),
                    serde_json::to_string(&thread.pending_events).unwrap()
                );
                assert!(!plaintext.contains("SOURCE_SENTINEL_ALPHA"));
                assert!(!plaintext.contains("SOURCE_SENTINEL_BETA"));
                let scope_preview = review::authored_preview(
                    &f.state,
                    &f.owner,
                    &agent.id,
                    result["proposal_id"].as_str().unwrap(),
                )
                .await
                .unwrap();
                assert_eq!(scope_preview["scope"], value["scope"]);
                assert!(
                    rows[0]
                        .summary
                        .contains("separate consent bypassed by Full access")
                );
            } else {
                assert!(!rows[0].summary.contains("Reduced scope"));
            }
            assert_eq!(rows[0].link.as_deref(), result["proposal_url"].as_str());
            let dto = serde_json::to_value(
                crate::handlers::assistant_nyxagent::AcknowledgementResponse::from(rows[0].clone()),
            )
            .unwrap();
            assert_eq!(dto["link"], result["proposal_url"]);
            let mut legacy = bson::to_document(&rows[0]).unwrap();
            legacy.remove("link");
            let legacy: crate::models::assistant_acknowledgement::AssistantAcknowledgement =
                bson::from_document(legacy).unwrap();
            assert!(legacy.link.is_none());
            assert!(!bson::to_document(&legacy).unwrap().contains_key("link"));
            let legacy_dto = serde_json::to_value(
                crate::handlers::assistant_nyxagent::AcknowledgementResponse::from(legacy),
            )
            .unwrap();
            assert!(legacy_dto.get("link").is_none());
            assert_eq!(agent.skills.len(), 1);
            assert_eq!(agent.skills[0].skill_id, skill_id);
            assert_eq!(
                agent.skills[0].sha256,
                publication::hash(&archive.lock().unwrap())
            );
            let proposal = f
                .state
                .db
                .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":result["proposal_id"].as_str().unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(proposal.status, "pinned");
            let publication = proposal.publication.as_ref().unwrap();
            assert!(publication.reduced_scope_acceptance.is_none());
            assert_eq!(publication.approved_by.as_deref(), Some(f.owner.as_str()));
            let items = review::list(&f.state, &f.owner, &agent.id, true)
                .await
                .unwrap();
            assert!(items[0].auto_allowed);
            assert_eq!(items[0].draft.as_ref().unwrap().name, "weekly-review");
            review::reject(
                &f.state,
                &f.owner,
                &agent.id,
                &proposal.id,
                proposal.revision,
                "rejected",
            )
            .await
            .unwrap();
            let detached = f
                .state
                .db
                .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .find_one(doc! {"_id": &agent.id})
                .await
                .unwrap()
                .unwrap();
            assert!(detached.skills.is_empty());
        }
        server.verify().await;
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn automatic_format_failure_repairs_original_drafting_conversation_without_a_card() {
    let f = fixture("automatic_format_repair").await;
    let server = setup_ornn(&f).await;
    let (row, _) = make(&f, input(&f.chat.agent_id)).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .delete_many(doc! {"authored_skill.proposal_id": &row.id})
        .await
        .unwrap();
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"valid": false, "violations": [{"rule": "no-readme-md"}]}
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: Some(&f.chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(f.chat.clone())),
    };
    let binding = publication::binding(
        &row,
        row.publication.as_ref().unwrap(),
        row.agent_skills_revision,
    );
    for _ in 0..2 {
        assert!(
            review::approve_automatically(
                &f.state,
                &f.chat,
                &row.agent_id,
                &row.id,
                &binding,
                &crate::services::agent_skill_service::OwnerPublicationReader(&reader),
            )
            .await
            .is_err()
        );
    }
    let failed: AssistantAgentLearningProposal = f
        .state
        .db
        .collection(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed.failure_code.as_deref(),
        Some("ornn_validation_failed")
    );
    assert_eq!(failed.failure_rules, ["no_readme_md"]);
    assert_eq!(
        pending_skill_event_count(&f.state.db, &f.row.id, &row.id).await,
        1
    );
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| {
                request.method.as_str() == "POST" && request.url.path() == "/api/v1/skills"
            })
    );
    f.state.db.drop().await.unwrap();
}
