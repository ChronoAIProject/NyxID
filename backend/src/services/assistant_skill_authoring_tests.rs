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
    json!({"agent":agent,"name":"weekly-review","description":"Prepare a weekly review", "skill_md":"# Weekly review\n1. Check outstanding work.\n2. Write a concise report.","files":[{"path":"references/checklist.md","content":"Verify dates before publishing."}]})
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
async fn pending_source_read_times_out_without_leaving_a_draft_or_card() {
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
        0
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
            review::PublicationFailureCode::NyxidRefused
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
    let f = fixture("authoring_expired_confirmation_renewal").await;
    let (row, card) = make(&f, input(&f.chat.agent_id)).await;
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
    let before =
        review::authored_preview_for_card(&f.state, &f.owner, &f.chat.agent_id, &row.id, &card.id)
            .await
            .unwrap();
    assert_eq!(before["failure_code"], "approval_expired");
    assert!(
        before["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("renew"))
    );

    let (renewed, fresh_card) = make(&f, input(&f.chat.agent_id)).await;
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
        "Its keys use nothing else.",
        "When the owner must finish a connect link, channel bot setup or verification outside chat, explain what to do and end your turn.",
        "NyxID resumes you when they finish; never ask them to reply that they are done or connected.",
        "Service approvals wait for the owner in the app, phone or Telegram and continue automatically; report timeouts.",
        "After posting work, end your turn; NyxID wakes you with replies when the group is quiet so you can report back.",
        "Link existing personal or organization bots from nyxid__list_channel_bots to yourself or a specialist with nyxid__connect_channel_bot by id or label.",
        "Create Telegram, Discord, Slack, Lark or other bots with nyxid__channel_bot_setup_link; share its link, never request bot secrets in chat or send the user to Studio.",
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
    for guest in [true, false] {
        let mut chat = f.chat.clone();
        chat.guest = guest;
        if !guest {
            chat.role = crate::models::assistant_conversation::AgentRole::Subagent;
        }
        assert!(
            create(
                &f.state,
                &chat,
                serde_json::from_value(value.clone()).unwrap(),
                &review::UnavailableReader,
            )
            .await
            .is_err()
        );
        assert!(!visible(
            super::super::assistant_account_tools::virtual_service_for(&f.state.db, &chat).await
        ));
    }
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
    crate::handlers::assistant_agent_learning::decide_authored(
        &f.state,
        &crate::test_utils::test_auth_user(&f.owner),
        card,
        allow,
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
    let f = fixture("authoring_publish").await;
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
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::to_bson(&f.row.active_turn).unwrap()}},
        )
        .await
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
    finish(&f, card, false).await.unwrap();
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
    assert!(finish(&f, card, true).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
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
    assert!(finish(&f, card, true).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
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
        "lease_live",
        "base_scripts_not_copied",
    ]
    .into();
    assert_eq!(keys, expected);
    assert_eq!(preview["status"], "pending");
    assert_eq!(preview["files"].as_array().unwrap().len(), 2);
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
        "proposal_id",
        "agent_id",
        "source",
    ]
    .into();
    assert_eq!(keys, expected);
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
