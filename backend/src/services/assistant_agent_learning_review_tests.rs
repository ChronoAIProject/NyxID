use super::*;
use crate::models::{
    assistant_acknowledgement::AssistantAcknowledgement,
    assistant_agent_learning::{
        AssistantAgentLearning, AssistantAgentLearningProposal, AssistantAgentLearningRun,
        AssistantAgentLearningSkillRoot, LearningEvidence, LearningPublication,
    },
    assistant_message::AssistantMessage,
    catalog_skill_revision::{SkillPin, SkillReference},
    org_membership::OrgRole,
    user::UserType,
};
use crate::services::{
    assistant_authority_tests::orchestrator_fixture,
    feature_flag_service::{self, FlagTarget},
};
use crate::test_utils::{test_membership, test_user};
use chrono::Utc;
use mongodb::bson::doc;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn proposal(started: bool) -> AssistantAgentLearningProposal {
    let now = Utc::now();
    AssistantAgentLearningProposal {
        source: Default::default(),
        id: Uuid::new_v4().to_string(),
        agent_id: "agent".into(),
        owner_id: "owner".into(),
        run_id: "run".into(),
        status: "pending".into(),
        revision: 0,
        config_revision: 0,
        agent_skills_revision: 0,
        fingerprint: "fingerprint".into(),
        input_digest: String::new(),
        model_contract: "agent-learning-v1".into(),
        publication: Some(LearningPublication {
            operation_id: "operation".into(),
            name: "skill-operation".into(),
            version: "1.0".into(),
            sha256: "a".repeat(64),
            skill_id: None,
            started,
            approved_by: Some("owner".into()),
            approval_digest: Some("digest".into()),
            acknowledgement_id: Some("card".into()),
            skills_revision: 0,
            lease_id: Some("lease".into()),
            lease_expires_at: Some(now),
            ..Default::default()
        }),
        failure_code: None,
        evidence: Vec::new(),
        body_encrypted: Vec::new(),
        body_bytes: 0,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn durable_failure_status_requires_verified_checkpoint() {
    let mut row = proposal(true);
    assert_eq!(failure_status(&row), "publication_failed");
    row.publication.as_mut().unwrap().verified_at = Some(Utc::now());
    assert_eq!(failure_status(&row), "published_unpinned");
}

#[test]
fn release_predicate_requires_no_dispatch_checkpoint_or_live_lease() {
    let mut p = proposal(false).publication.unwrap();
    p.lease_expires_at = None;
    assert!(non_effective(&p, Utc::now()));
    p.started = true;
    assert!(!non_effective(&p, Utc::now()));
    p.started = false;
    p.uncertain_dispatch = true;
    assert!(!non_effective(&p, Utc::now()));
    p.uncertain_dispatch = false;
    p.verified_at = Some(Utc::now());
    assert!(!non_effective(&p, Utc::now()));
    p.verified_at = None;
    p.lease_expires_at = Some(Utc::now() + Duration::seconds(30));
    assert!(!non_effective(&p, Utc::now()));
}

#[test]
fn recovery_legacy_predicate_needs_an_update_without_snapshot() {
    let mut p = proposal(false).publication.unwrap();
    // Unclassified pre-#1828 rows are updates only when their draft has a base.
    assert!(!legacy_package(&p, None));
    assert!(!legacy_package(&p, Some(false)));
    assert!(legacy_package(&p, Some(true)));
    p.target_kind = Some("create".into());
    assert!(!legacy_package(&p, Some(true)));
    p.target_kind = Some("update".into());
    assert!(legacy_package(&p, None));
    p.package_format = PACKAGE_FORMAT_SNAPSHOT;
    assert!(!legacy_package(&p, None));
    p.package_format = 0;
    p.interface_encrypted = Some(vec![1]);
    assert!(!legacy_package(&p, None));
}

#[tokio::test]
async fn stale_base_changed_binding_cannot_overwrite_new_attempt_or_refusal() {
    let fixture = orchestrator_fixture("learning_stale_base_binding").await;
    let mut observed = proposal(false);
    observed.owner_id = fixture.owner.clone();
    observed.publication.as_mut().unwrap().lease_expires_at = None;
    let proposals = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
    proposals.insert_one(&observed).await.unwrap();
    let raw = fixture
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);

    raw.update_one(doc! {"_id":&observed.id}, doc! {"$set":{
        "publication.attempt":1_i64,"status":"publishing",
        "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now() + Duration::minutes(1))
    }}).await.unwrap();
    record_base_changed_if_current(&fixture.state.db, &observed)
        .await
        .unwrap();
    let live = proposals
        .find_one(doc! {"_id":&observed.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.status, "publishing");
    assert_eq!(live.publication.unwrap().attempt, 1);
    assert_eq!(live.failure_code, None);

    raw.update_one(
        doc! {"_id":&observed.id},
        doc! {"$set":{
            "status":"published_unpinned","publication.lease_expires_at":bson::Bson::Null,
            "failure_code":"verify_failed"
        }},
    )
    .await
    .unwrap();
    record_base_changed_if_current(&fixture.state.db, &observed)
        .await
        .unwrap();
    let live = proposals
        .find_one(doc! {"_id":&observed.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.status, "published_unpinned");
    assert_eq!(live.failure_code.as_deref(), Some("verify_failed"));

    raw.update_one(
        doc! {"_id":&observed.id},
        doc! {"$set":{
            "status":"publication_failed","publication.attempt":0_i64,
            "failure_code":"ornn_validation_failed","publication.last_stage":"format_validate"
        }},
    )
    .await
    .unwrap();
    record_base_changed_if_current(&fixture.state.db, &observed)
        .await
        .unwrap();
    let live = proposals
        .find_one(doc! {"_id":&observed.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.failure_code.as_deref(), Some("ornn_validation_failed"));
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn definitive_refusal_releases_target_for_revised_draft() {
    let fixture = orchestrator_fixture("learning_refusal_releases_target").await;
    let mut row = proposal(true);
    row.status = "publishing".into();
    row.owner_id = fixture.owner.clone();
    let target_skill_id = Uuid::new_v4().to_string();
    let p = row.publication.as_mut().unwrap();
    p.target_kind = Some("update".into());
    p.target_skill_id = Some(target_skill_id.clone());
    p.version = "1.1".into();
    p.lease_expires_at = Some(Utc::now() + Duration::seconds(60));
    let p = p.clone();
    let target = target_id(&p).unwrap();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    let targets = fixture
        .state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME);
    let now = Utc::now();
    targets
        .insert_one(LearningPublicationTarget {
            id: target.clone(),
            agent_id: row.agent_id.clone(),
            owner_id: row.owner_id.clone(),
            proposal_id: row.id.clone(),
            operation_id: p.operation_id.clone(),
            package_sha256: p.sha256.clone(),
            state: "uncertain".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    defer(
        &fixture.state,
        &fixture.owner,
        &row,
        &p,
        DeferredFailure {
            code: publication::FailureCode::OrnnInterfaceChangeRequiresMajor,
            stage: PublicationStage::Publish,
            status: Some(409),
            definitive: true,
        },
    )
    .await
    .unwrap();
    assert!(
        targets
            .find_one(doc! {"_id":&target})
            .await
            .unwrap()
            .is_none()
    );
    let stored = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.publication.unwrap().started);
    targets
        .insert_one(LearningPublicationTarget {
            id: target.clone(),
            agent_id: row.agent_id.clone(),
            owner_id: row.owner_id.clone(),
            proposal_id: Uuid::new_v4().to_string(),
            operation_id: Uuid::new_v4().to_string(),
            package_sha256: "b".repeat(64),
            state: "reserved".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let holder = targets
        .find_one(doc! {"_id":&target})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(holder.operation_id, p.operation_id);
    assert_ne!(holder.package_sha256, p.sha256);
    fixture.state.db.drop().await.unwrap();
}

#[test]
fn approval_replay_is_bound_to_exact_card_and_operation() {
    let mut row = proposal(false);
    let publication = row.publication.as_ref().unwrap();
    assert!(resumes_publication(publication, "owner", "card", "digest"));
    assert!(!resumes_publication(
        publication,
        "owner",
        "other",
        "digest"
    ));
    assert!(!resumes_publication(
        publication,
        "owner",
        "card",
        "changed"
    ));
    row.publication.as_mut().unwrap().operation_id = "other".into();
    assert!(resumes_publication(
        row.publication.as_ref().unwrap(),
        "owner",
        "card",
        "digest"
    ));
}

#[test]
fn approval_binding_changes_after_edit_or_skill_revision() {
    let row = proposal(false);
    let publication = row.publication.as_ref().unwrap();
    let original = publication::binding(&row, publication, 4);
    let mut edited = row.clone();
    edited.revision += 1;
    assert_ne!(original, publication::binding(&edited, publication, 4));
    assert_ne!(original, publication::binding(&row, publication, 5));
}

#[tokio::test]
async fn migration_withholds_marker_for_unrecoverable_started_operation() {
    let fixture = orchestrator_fixture("learning_migration_unrecoverable").await;
    let row = proposal(true);
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    assert!(publication_ready(&fixture.state.db).await.is_err());
    assert!(
        fixture
            .state
            .db
            .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME)
            .find_one(doc! {"_id":MIGRATION_ID})
            .await
            .unwrap()
            .is_none()
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn migration_reserves_started_update_even_if_invalidated_and_is_idempotent() {
    let fixture = orchestrator_fixture("learning_migration_started_update").await;
    let mut row = proposal(true);
    let target = Uuid::new_v4().to_string();
    row.status = "invalidated".into();
    let p = row.publication.as_mut().unwrap();
    p.target_kind = Some("update".into());
    p.target_skill_id = Some(target.clone());
    p.version = "1.1".into();
    let operation_id = p.operation_id.clone();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    publication_ready(&fixture.state.db).await.unwrap();
    let barrier = fixture
        .state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":format!("{target}:1.1")})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(barrier.operation_id, operation_id);
    assert_eq!(barrier.state, "uncertain");
    let durable = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(durable.publication.unwrap().uncertain_dispatch);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn migration_uses_original_encrypted_base_after_current_pin_changes() {
    let fixture = orchestrator_fixture("learning_migration_original_base").await;
    let original_id = Uuid::new_v4().to_string();
    let replacement_id = Uuid::new_v4().to_string();
    let body = json!({"schema_version":1,"kind":"improve","name":"original","description":"safe guidance",
        "skill_md":"# Guidance","files":[],"base_skill":{"source":"ornn","skill_id":original_id,
        "name":"original","version":"1.0","sha256":"a".repeat(64)},"rationale":"","safety_notes":""});
    let mut row = proposal(true);
    row.publication.as_mut().unwrap().version = "1.1".into();
    row.body_encrypted = fixture
        .state
        .encryption_keys
        .encrypt(&serde_json::to_vec(&body).unwrap())
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&fixture.chat.agent_id},
            doc! {"$set":{"skills":[{"source":"ornn","skill_id":&replacement_id,"name":"replacement",
            "version":"2.0","sha256":"b".repeat(64),"dependencies":[]} ]}},
        )
        .await
        .unwrap();
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    publication_ready(&fixture.state.db).await.unwrap();
    let barrier = fixture
        .state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":format!("{original_id}:1.1")})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(barrier.operation_id, row.publication.unwrap().operation_id);
    assert!(
        fixture
            .state
            .db
            .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
            .find_one(doc! {"_id":format!("{replacement_id}:1.1")})
            .await
            .unwrap()
            .is_none()
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn migration_withholds_marker_when_operation_bound_targets_disagree() {
    let fixture = orchestrator_fixture("learning_migration_target_disagreement").await;
    let mut row = proposal(true);
    let p = row.publication.as_mut().unwrap();
    p.target_kind = Some("update".into());
    p.target_skill_id = Some(Uuid::new_v4().to_string());
    p.skill_id = Some(Uuid::new_v4().to_string());
    let barrier_id = format!("{}:{}", p.target_skill_id.as_deref().unwrap(), p.version);
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    assert!(publication_ready(&fixture.state.db).await.is_err());
    assert!(
        fixture
            .state
            .db
            .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
            .find_one(doc! {"_id":barrier_id})
            .await
            .unwrap()
            .is_none()
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn definitive_refusal_clears_only_a_never_uncertain_attempt_and_fences_stale_holder() {
    let fixture = orchestrator_fixture("learning_refusal_fence").await;
    let mut row = proposal(true);
    row.status = "publishing".into();
    let p = row.publication.as_mut().unwrap();
    p.attempt = 1;
    p.lease_expires_at = Some(Utc::now() + Duration::minutes(1));
    let first = p.clone();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    defer(
        &fixture.state,
        &fixture.owner,
        &row,
        &first,
        DeferredFailure {
            code: publication::FailureCode::OrnnWriteForbidden,
            stage: PublicationStage::Publish,
            status: Some(403),
            definitive: true,
        },
    )
    .await
    .unwrap();
    let collection = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
    let stored = collection
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.publication.as_ref().unwrap().started);
    assert_eq!(stored.failure_code.as_deref(), Some("ornn_write_forbidden"));
    collection.update_one(doc! {"_id":&row.id}, doc! {"$set":{"status":"publishing","publication.started":true,"publication.uncertain_dispatch":true,"publication.attempt":2_i64,"publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1))}}).await.unwrap();
    assert!(
        defer(
            &fixture.state,
            &fixture.owner,
            &row,
            &first,
            DeferredFailure {
                code: publication::FailureCode::OrnnValidationFailed,
                stage: PublicationStage::Publish,
                status: Some(400),
                definitive: true
            }
        )
        .await
        .is_err()
    );
    let second = collection
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.failure_code.as_deref(), Some("ornn_write_forbidden"));
    let current = second.publication.as_ref().unwrap().clone();
    defer(
        &fixture.state,
        &fixture.owner,
        &row,
        &current,
        DeferredFailure {
            code: publication::FailureCode::OrnnWriteForbidden,
            stage: PublicationStage::Publish,
            status: Some(403),
            definitive: true,
        },
    )
    .await
    .unwrap();
    let after = collection
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(after.publication.as_ref().unwrap().started);
    assert!(after.publication.as_ref().unwrap().uncertain_dispatch);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn started_boundary_and_target_state_commit_together_before_egress() {
    let fixture = orchestrator_fixture("learning_started_boundary").await;
    let mut row = proposal(false);
    row.status = "publishing".into();
    let target_skill = Uuid::new_v4().to_string();
    let p = row.publication.as_mut().unwrap();
    p.attempt = 1;
    p.lease_expires_at = Some(Utc::now() + Duration::minutes(1));
    p.target_kind = Some("update".into());
    p.target_skill_id = Some(target_skill.clone());
    let p = p.clone();
    let now = Utc::now();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .insert_one(LearningPublicationTarget {
            id: format!("{target_skill}:{}", p.version),
            agent_id: row.agent_id.clone(),
            owner_id: row.owner_id.clone(),
            proposal_id: row.id.clone(),
            operation_id: p.operation_id.clone(),
            package_sha256: p.sha256.clone(),
            state: "reserved".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    assert!(!p.started);
    mark_started(&fixture.state, &row, &p).await.unwrap();
    let durable = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(durable.publication.as_ref().unwrap().started);
    assert!(!non_effective(
        durable.publication.as_ref().unwrap(),
        Utc::now()
    ));
    let target = fixture
        .state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":format!("{target_skill}:{}", p.version)})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.state, "uncertain");
    defer(
        &fixture.state,
        &fixture.owner,
        &row,
        &p,
        DeferredFailure {
            code: publication::FailureCode::OrnnWriteForbidden,
            stage: PublicationStage::Publish,
            status: Some(403),
            definitive: true,
        },
    )
    .await
    .unwrap();
    let durable = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(non_effective(
        durable.publication.as_ref().unwrap(),
        Utc::now()
    ));
    let target = fixture
        .state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":format!("{target_skill}:{}", p.version)})
        .await
        .unwrap();
    assert!(target.is_none());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_evidence_never_deletes_an_effective_operation_body() {
    let fixture = orchestrator_fixture("learning_effective_body_retained").await;
    let mut agent = fixture
        .state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":&fixture.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    agent.destroyed_at = Some(Utc::now());
    let mut row = proposal(true);
    row.source = ProposalSource::Authored;
    row.agent_id = agent.id.clone();
    row.owner_id = fixture.owner.clone();
    row.body_encrypted = vec![1, 2, 3];
    row.body_bytes = 3;
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    assert!(
        require_current(&fixture.state.db, &agent, &row)
            .await
            .is_err()
    );
    let stored = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.body_encrypted, vec![1, 2, 3]);
    assert_eq!(stored.status, "pending");
    fixture.state.db.drop().await.unwrap();
}

#[test]
fn approval_refuses_non_owner_guest_and_non_orchestrator() {
    let mut chat = crate::services::assistant_acknowledgement_service::ChatAuthority {
        org_agent_access: None,
        turn_id: Some("turn".into()),
        turn_stopped: false,
        turn_live: true,
        machine_node_ids: Vec::new(),
        saved_login_ids: Vec::new(),
        conversation_id: "conversation".into(),
        user_id: "owner".into(),
        api_key_id: "key".into(),
        role: crate::models::assistant_conversation::AgentRole::Orchestrator,
        agent_id: "agent".into(),
        agent_name: "NyxBot".into(),
        guest: false,
        confirmation_policy: None,
    };
    let agent = crate::models::assistant_agent::AssistantAgent {
        id: "agent".into(),
        user_id: "owner".into(),
        kind: crate::models::assistant_agent::AgentKind::Nyxbot,
        name: "NyxBot".into(),
        description: String::new(),
        specialty: None,
        grants: Default::default(),
        guest_access: Default::default(),
        operation_scopes: Default::default(),
        skills: Vec::new(),
        skills_revision: 0,
        skill_metadata: Default::default(),
        operation_scope_revisions: Default::default(),
        machine_node_ids: Vec::new(),
        machine_access: None,
        saved_login_ids: Vec::new(),
        created_by: "user".into(),
        model: "default".into(),
        home_conversation_id: None,
        memory: Vec::new(),
        display_name: None,
        persona: None,
        destroyed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    assert!(approval_actor_allowed(&agent, "owner", &chat));
    assert!(!approval_actor_allowed(&agent, "other", &chat));
    chat.guest = true;
    assert!(!approval_actor_allowed(&agent, "owner", &chat));
    chat.guest = false;
    chat.role = crate::models::assistant_conversation::AgentRole::Subagent;
    assert!(!approval_actor_allowed(&agent, "owner", &chat));
}

async fn org_review_fixture(
    name: &str,
) -> (
    crate::services::assistant_authority_tests::Fixture,
    crate::models::assistant_agent::AssistantAgent,
    String,
    String,
) {
    let fixture = orchestrator_fixture(name).await;
    let org = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    fixture
        .state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_many([
            test_user(&org, UserType::Org),
            test_user(&member, UserType::Person),
        ])
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .insert_many([
            test_membership(&org, &fixture.owner, OrgRole::Admin, None),
            test_membership(&org, &member, OrgRole::Member, None),
        ])
        .await
        .unwrap();
    feature_flag_service::set_platform_override(
        &fixture.state.db,
        "assistant:org-agents",
        &FlagTarget::Global,
        true,
        &fixture.owner,
    )
    .await
    .unwrap();
    feature_flag_service::set_platform_override(
        &fixture.state.db,
        learning::FLAG_KEY,
        &FlagTarget::Global,
        true,
        &fixture.owner,
    )
    .await
    .unwrap();
    let (agent, _) = crate::services::assistant_team_service::create_specialist_for(
        &fixture.state.db,
        &fixture.state.encryption_keys,
        &fixture.owner,
        &org,
        crate::services::assistant_team_service::CreateRequest {
            machines: None,
            logins: None,
            name: "learning-worker".into(),
            description: "Learning worker".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let mut row = proposal(false);
    row.agent_id = agent.id.clone();
    row.owner_id = org;
    let proposal_id = row.id.clone();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    (fixture, agent, proposal_id, member)
}

#[tokio::test]
async fn approval_binding_and_approve_refuse_wrong_access() {
    let (fixture, agent, proposal_id, member) =
        org_review_fixture("learning_review_access_refusals").await;
    let binding = approval_binding(
        &fixture.state,
        &member,
        &agent.id,
        &proposal_id,
        0,
        0,
        &UnavailableReader,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(binding, AppError::Conflict(message) if message == "owner_binding_unavailable")
    );

    let mut guest = fixture.chat.clone();
    guest.user_id = member.clone();
    guest.agent_id = agent.id.clone();
    guest.guest = true;
    let guest_error = approve(
        &fixture.state,
        &guest,
        &agent.id,
        &proposal_id,
        "card",
        &json!({}),
        &TestReader,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(guest_error, AppError::Forbidden(message) if message == "owner_binding_unavailable")
    );

    guest.guest = false;
    guest.role = crate::models::assistant_conversation::AgentRole::Subagent;
    let role_error = approve(
        &fixture.state,
        &guest,
        &agent.id,
        &proposal_id,
        "card",
        &json!({}),
        &TestReader,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(role_error, AppError::Forbidden(message) if message == "owner_binding_unavailable")
    );

    let other = Uuid::new_v4().to_string();
    fixture
        .state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&other, UserType::Person))
        .await
        .unwrap();
    let other_agent =
        crate::services::assistant_team_service::ensure_nyxbot(&fixture.state.db, &other)
            .await
            .unwrap();
    let mut other_row = proposal(false);
    other_row.agent_id = other_agent.id.clone();
    other_row.owner_id = other;
    let other_id = other_row.id.clone();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(other_row)
        .await
        .unwrap();
    let other_error = approval_binding(
        &fixture.state,
        &fixture.owner,
        &other_agent.id,
        &other_id,
        0,
        0,
        &UnavailableReader,
    )
    .await
    .unwrap_err();
    assert!(matches!(other_error, AppError::NotFound(_)));
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn flag_off_review_gate_does_not_create_learning_rows() {
    let fixture = orchestrator_fixture("learning_review_flag_off").await;
    assert!(gate(&fixture.state.db, &fixture.owner).await.is_err());
    assert_eq!(
        fixture
            .state
            .db
            .collection::<mongodb::bson::Document>(CONFIG_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .state
            .db
            .collection::<mongodb::bson::Document>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn reject_records_fingerprint_and_suppresses_duplicate() {
    let fixture = orchestrator_fixture("learning_review_reject").await;
    feature_flag_service::set_platform_override(
        &fixture.state.db,
        learning::FLAG_KEY,
        &FlagTarget::Global,
        true,
        &fixture.owner,
    )
    .await
    .unwrap();
    let id = Uuid::new_v4().to_string();
    let mut row = proposal(false);
    row.id = id.clone();
    row.agent_id = fixture.chat.agent_id.clone();
    row.owner_id = fixture.owner.clone();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    reject(
        &fixture.state,
        &fixture.owner,
        &row.agent_id,
        &id,
        0,
        "not_useful",
    )
    .await
    .unwrap();
    let stored = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "rejected");
    let rejection = fixture
        .state
        .db
        .collection::<AssistantAgentLearningRejection>(REJECTIONS_COLLECTION_NAME)
        .find_one(doc! {"agent_id": &row.agent_id, "fingerprint": &row.fingerprint})
        .await
        .unwrap();
    assert!(rejection.is_some());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_root_provenance_is_active_even_when_legacy_active_is_false() {
    let fixture = orchestrator_fixture("learning_review_root_fence").await;
    let reference = SkillReference {
        source: "ornn".into(),
        skill_id: Uuid::new_v4().to_string(),
        name: "base".into(),
        version: "1.0".into(),
        sha256: "b".repeat(64),
        dependencies: Vec::new(),
    };
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &fixture.chat.agent_id},
            doc! {"$set": {"skills": mongodb::bson::to_bson(&vec![reference.clone()]).unwrap(), "skills_revision": 1}},
        )
        .await
        .unwrap();
    let now = Utc::now();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningSkillRoot>(ROOTS_COLLECTION_NAME)
        .insert_one(AssistantAgentLearningSkillRoot {
            id: Uuid::new_v4().to_string(),
            agent_id: fixture.chat.agent_id.clone(),
            owner_id: fixture.owner.clone(),
            skill_id: reference.skill_id.clone(),
            version: reference.version.clone(),
            sha256: reference.sha256.clone(),
            operation_id: "operation".into(),
            active: false,
            proposal_id: "proposal".into(),
            config_revision: 0,
            agent_skills_revision: 1,
            created_at: now,
        })
        .await
        .unwrap();
    let agent = crate::services::assistant_team_service::agent(
        &fixture.state.db,
        &fixture.owner,
        &fixture.chat.agent_id,
    )
    .await
    .unwrap();
    let draft = learning::GeneratedProposal {
        schema_version: 1,
        kind: "improve".into(),
        name: "improved".into(),
        description: "description".into(),
        skill_md: "guidance".into(),
        files: Vec::new(),
        base_skill: Some(SkillPin {
            source: reference.source.clone(),
            skill_id: reference.skill_id.clone(),
            name: reference.name.clone(),
            version: reference.version.clone(),
            sha256: reference.sha256.clone(),
        }),
        rationale: String::new(),
        safety_notes: String::new(),
    };
    validate_base(&fixture.state.db, &agent, &draft)
        .await
        .unwrap();
    fixture.state.db.drop().await.unwrap();
}

struct WireReader {
    base: String,
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl OrnnReader for WireReader {
    async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
        Ok(self
            .client
            .get(format!("{}{}", self.base, path))
            .send()
            .await
            .map_err(|_| AppError::ServicePoolInfrastructureUnavailable)?
            .bytes()
            .await
            .map_err(|_| AppError::ServicePoolInfrastructureUnavailable)?
            .to_vec())
    }

    async fn request(&self, method: http::Method, path: &str, body: Vec<u8>) -> AppResult<Vec<u8>> {
        Ok(self
            .client
            .request(method, format!("{}{}", self.base, path))
            .body(body)
            .send()
            .await
            .map_err(|_| AppError::ServicePoolInfrastructureUnavailable)?
            .bytes()
            .await
            .map_err(|_| AppError::ServicePoolInfrastructureUnavailable)?
            .to_vec())
    }

    async fn classified(
        &self,
        method: http::Method,
        path: &str,
        body: Vec<u8>,
    ) -> super::super::agent_skill_service::OrnnOutcome {
        use super::super::agent_skill_service::{OrnnCode, OrnnOutcome};
        let response = self
            .client
            .request(method, format!("{}{}", self.base, path))
            .body(body)
            .send()
            .await;
        let Ok(response) = response else {
            return OrnnOutcome::Uncertain;
        };
        let status = response.status().as_u16();
        let Ok(bytes) = response.bytes().await else {
            return OrnnOutcome::Uncertain;
        };
        if (200..300).contains(&status) {
            return OrnnOutcome::Ok(bytes.to_vec());
        }
        let code = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v["code"].as_str().map(OrnnCode::parse))
            .unwrap_or(OrnnCode::Unknown);
        OrnnOutcome::Response { status, code }
    }
}

fn archive() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("SKILL.md", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"# Guidance\n").unwrap();
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn wiremock_publish_failure_is_ambiguous_and_reconcile_accepts_one_match() {
    let server = MockServer::start().await;
    let bytes = archive();
    let hash = hex::encode(Sha256::digest(&bytes));
    let id = Uuid::new_v4().to_string();
    let metadata = json!({"data":{"guid":id,"name":"skill-operation","version":"1.0","skillHash":hash,"description":"guidance","isPrivate":true,"createdBy":"owner","sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}});
    Mock::given(method("POST"))
        .and(path("/api/v1/skills"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let reader = WireReader {
        base: server.uri(),
        client: reqwest::Client::new(),
    };
    let publication = LearningPublication {
        sha256: hash.clone(),
        name: "skill-operation".into(),
        version: "1.0".into(),
        operation_id: Uuid::new_v4().to_string(),
        skill_id: None,
        started: true,
        approved_by: None,
        approval_digest: None,
        acknowledgement_id: None,
        skills_revision: 0,
        lease_id: None,
        lease_expires_at: None,
        ..Default::default()
    };
    assert!(matches!(
        publication::publish_classified(&reader, None, bytes.clone()).await,
        publication::PublishOutcome::Uncertain { .. }
    ));
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skills"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/skill-search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[],"totalPages":0}})),
        )
        .mount(&server)
        .await;
    assert!(matches!(
        publication::reconcile(&reader, "owner", &publication, None).await,
        Err(publication::PublicationError::Ambiguous(_))
    ));
    let second_id = Uuid::new_v4().to_string();
    server.reset().await;
    Mock::given(method("GET")).and(path("/api/v1/skill-search")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"guid":id,"name":"skill-operation"},{"guid":second_id,"name":"skill-operation"}],"totalPages":1}}))).mount(&server).await;
    for candidate in [&id, &second_id] {
        Mock::given(method("GET")).and(path(format!("/api/v1/skills/{candidate}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":candidate,"name":"skill-operation","version":"1.0","skillHash":hash,"description":"guidance","isPrivate":true,"createdBy":"owner","sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}}))).mount(&server).await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/skills/{candidate}/closure")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/skills/{candidate}/versions/1.0/download"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;
    }
    assert!(matches!(
        publication::reconcile(&reader, "owner", &publication, None).await,
        Err(publication::PublicationError::Ambiguous(_))
    ));
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skills"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/skill-search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"items":[{"guid":id,"name":"skill-operation"}],"totalPages":1}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions/1.0/download")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    assert_eq!(
        publication::reconcile(&reader, "owner", &publication, None)
            .await
            .unwrap(),
        id
    );
    server.verify().await;
}

#[tokio::test]
async fn version_exists_reconciles_exact_zip_and_refuses_mismatch_without_another_put() {
    let server = MockServer::start().await;
    let id = Uuid::new_v4().to_string();
    let bytes = archive();
    let hash = hex::encode(Sha256::digest(&bytes));
    let base = SkillPin {
        source: "ornn".into(),
        skill_id: id.clone(),
        name: "same-name".into(),
        version: "1.0".into(),
        sha256: "b".repeat(64),
    };
    let publication = LearningPublication {
        operation_id: Uuid::new_v4().to_string(),
        name: "same-name".into(),
        version: "1.1".into(),
        sha256: hash.clone(),
        target_kind: Some("update".into()),
        target_skill_id: Some(id.clone()),
        started: true,
        ..Default::default()
    };
    let reader = WireReader {
        base: server.uri(),
        client: reqwest::Client::new(),
    };
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(json!({"code":"SKILL_VERSION_EXISTS"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        publication::publish_classified(&reader, Some(&base), bytes.clone()).await,
        publication::PublishOutcome::VersionConflict { status: 409 }
    ));
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
            "guid":id,"name":"same-name","version":"1.1","skillHash":hash,"isPrivate":true,
            "createdBy":"owner","sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions/1.1/download")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    assert_eq!(
        publication::reconcile(&reader, "owner", &publication, Some(&base))
            .await
            .unwrap(),
        id
    );
    server.verify().await;
    server.reset().await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path(format!("/api/v1/skills/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
            "guid":id,"name":"same-name","version":"1.1","skillHash":"c".repeat(64),
            "isPrivate":true,"createdBy":"owner","sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}})))
        .mount(&server).await;
    assert!(matches!(
        publication::reconcile(&reader, "owner", &publication, Some(&base)).await,
        Err(publication::PublicationError::Integrity)
    ));
    server.verify().await;
}

struct StartedLearned {
    fixture: crate::services::assistant_authority_tests::Fixture,
    agent: AssistantAgent,
    id: String,
    generated: learning::GeneratedProposal,
    binding: Value,
    card: String,
    publication: LearningPublication,
}

struct SeededLearned {
    fixture: crate::services::assistant_authority_tests::Fixture,
    agent: AssistantAgent,
    id: String,
    generated: learning::GeneratedProposal,
}

/// A pending learned create proposal with current evidence, in the NyxBot
/// conversation of an orchestrator fixture, after the startup migration.
async fn seed_learned_proposal(name: &str) -> SeededLearned {
    let fixture = orchestrator_fixture(name).await;
    feature_flag_service::set_platform_override(
        &fixture.state.db,
        learning::FLAG_KEY,
        &FlagTarget::Global,
        true,
        &fixture.owner,
    )
    .await
    .unwrap();
    let agent = crate::services::assistant_team_service::agent(
        &fixture.state.db,
        &fixture.owner,
        &fixture.chat.agent_id,
    )
    .await
    .unwrap();
    let now = Utc::now();
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &fixture.row.id},
            doc! {"$set": {"agent_id": &agent.id, "learning_epoch": 1_i64, "automation_thread": false, "channel": mongodb::bson::Bson::Null, "group_id": mongodb::bson::Bson::Null, "guest_turn": false, "active_turn": mongodb::bson::Bson::Null}},
        )
        .await
        .unwrap();
    let turn_id = Uuid::new_v4().to_string();
    for (seq, role, text) in [
        (100_i64, "user", "teach me"),
        (101_i64, "assistant", "done"),
    ] {
        fixture
            .state
            .db
            .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(AssistantMessage {
                steering: None,
                voice: None,
                execution_pending: false,
                id: Uuid::new_v4().to_string(),
                conversation_id: fixture.row.id.clone(),
                user_id: fixture.owner.clone(),
                seq,
                turn_id: turn_id.clone(),
                role: role.into(),
                text: text.into(),
                status: "completed".into(),
                error_code: None,
                created_at: now + chrono::Duration::seconds(seq),
                activities: Vec::new(),
                attachments: Vec::new(),
                origin: Some(crate::models::assistant_conversation::TurnOrigin::User),
                via: None,
            })
            .await
            .unwrap();
    }
    let evidence = LearningEvidence {
        label: "teaching task".into(),
        conversation_id: fixture.row.id.clone(),
        turn_id: turn_id.clone(),
        principal_id: fixture.owner.clone(),
        consent_revision: None,
        created_at: now,
    };
    fixture
        .state
        .db
        .collection::<AssistantAgentLearning>(CONFIG_COLLECTION_NAME)
        .insert_one(AssistantAgentLearning {
            agent_id: agent.id.clone(),
            owner_id: fixture.owner.clone(),
            enabled: true,
            threshold: 15,
            learning_epoch: 1,
            config_revision: 0,
            enabled_by: Some(fixture.owner.clone()),
            last_success_cursor: None,
            last_success_run_id: Some("run".into()),
            eligible_count: 1,
            last_run_at: None,
            last_success_at: Some(now),
            last_error_code: None,
            lease_owner: None,
            lease_expires_at: None,
            fence: 0,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningRun>(RUNS_COLLECTION_NAME)
        .insert_one(AssistantAgentLearningRun {
            id: "run".into(),
            agent_id: agent.id.clone(),
            owner_id: fixture.owner.clone(),
            actor_id: fixture.owner.clone(),
            mode: "manual".into(),
            config_revision: 0,
            learning_epoch: 1,
            cursor_before: None,
            candidate_watermark: None,
            evidence: vec![evidence.clone()],
            input_digest: Some("input".into()),
            status: "succeeded".into(),
            attempt: 1,
            lease_owner: None,
            lease_expires_at: None,
            fence: 0,
            error_code: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let generated = learning::GeneratedProposal {
        schema_version: 1,
        kind: "new".into(),
        name: "resumable".into(),
        description: "safe guidance".into(),
        skill_md: "# Guidance".into(),
        files: Vec::new(),
        base_skill: None,
        rationale: "reusable".into(),
        safety_notes: "review".into(),
    };
    let body = serde_json::to_vec(&generated).unwrap();
    let id = Uuid::new_v4().to_string();
    let encrypted = fixture.state.encryption_keys.encrypt(&body).await.unwrap();
    fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .insert_one(AssistantAgentLearningProposal {
            source: Default::default(),
            id: id.clone(),
            agent_id: agent.id.clone(),
            owner_id: fixture.owner.clone(),
            run_id: "run".into(),
            status: "pending".into(),
            revision: 0,
            config_revision: 0,
            agent_skills_revision: agent.skills_revision,
            fingerprint: "fingerprint".into(),
            input_digest: "input".into(),
            model_contract: "agent-learning-v1".into(),
            publication: None,
            failure_code: None,
            evidence: vec![evidence],
            body_encrypted: encrypted,
            body_bytes: body.len() as i64,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let seeded_run = fixture
        .state
        .db
        .collection::<AssistantAgentLearningRun>(RUNS_COLLECTION_NAME)
        .find_one(doc! {"_id":"run"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        learning::all_evidence_current(&fixture.state.db, &seeded_run, &agent)
            .await
            .unwrap()
    );
    migrate_publication_targets(&fixture.state, false)
        .await
        .unwrap();
    SeededLearned {
        fixture,
        agent,
        id,
        generated,
    }
}

/// A learned create whose approved attempt dispatched with an unknown outcome.
async fn started_learned_publication(name: &str) -> StartedLearned {
    let SeededLearned {
        fixture,
        agent,
        id,
        generated,
    } = seed_learned_proposal(name).await;
    let now = Utc::now();
    let binding = approval_binding(
        &fixture.state,
        &fixture.owner,
        &agent.id,
        &id,
        0,
        agent.skills_revision,
        &UnavailableReader,
    )
    .await
    .unwrap();
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(doc! {"_id": &id}, doc! {"$set": {"revision": 1_i64}})
        .await
        .unwrap();
    let edited_error = approve(
        &fixture.state,
        &fixture.chat,
        &agent.id,
        &id,
        "unused-card",
        &binding,
        &TestReader,
    )
    .await
    .unwrap_err();
    assert!(matches!(edited_error, AppError::Conflict(message) if message.contains("changed")));
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(doc! {"_id": &id}, doc! {"$set": {"revision": 0_i64}})
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"skills_revision": 1_i64}},
        )
        .await
        .unwrap();
    let skills_error = approve(
        &fixture.state,
        &fixture.chat,
        &agent.id,
        &id,
        "unused-card",
        &binding,
        &TestReader,
    )
    .await
    .unwrap_err();
    assert!(matches!(skills_error, AppError::Conflict(message) if message.contains("changed")));
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"skills_revision": 0_i64}},
        )
        .await
        .unwrap();
    let card = Uuid::new_v4().to_string();
    fixture
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .insert_one(AssistantAcknowledgement {
            authored_skill: None,
            machine_context: None,
            id: card.clone(),
            conversation_id: fixture.chat.conversation_id.clone(),
            user_id: fixture.owner.clone(),
            api_key_id: fixture.chat.api_key_id.clone(),
            kind: "action".into(),
            service_id: None,
            service_slug: None,
            service_name: None,
            platform: false,
            tool_name: Some(TOOL.into()),
            arguments_digest: Some(
                crate::services::assistant_acknowledgement_service::arguments_digest(&binding),
            ),
            operation_selection: None,
            operation_contract_digest: None,
            skill_selection: None,
            summary: "Publish a learning skill".into(),
            status: "allowed".into(),
            requested_turn_id: None,
            voice_request_id: None,
            continuation_receipt_id: None,
            trigger_run_id: None,
            created_at: now,
            decided_at: Some(now),
            expires_at: now + chrono::Duration::hours(1),
            decider: "user".into(),
            request_excerpt: None,
            decided_by: Some("user".into()),
            reason: None,
        })
        .await
        .unwrap();
    let digest = crate::services::assistant_acknowledgement_service::arguments_digest(&binding);
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"publication.approved_by": &fixture.owner, "publication.acknowledgement_id": &card, "publication.approval_digest": &digest}},
        )
        .await
        .unwrap();
    let reader = TestReader;
    let first = approve(
        &fixture.state,
        &fixture.chat,
        &agent.id,
        &id,
        &card,
        &binding,
        &reader,
    )
    .await;
    assert!(first.is_err());
    let stored = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "publication_failed");
    assert!(stored.publication.as_ref().is_some_and(|p| p.started));
    assert_eq!(
        fixture
            .state
            .db
            .collection::<AssistantAcknowledgement>(
                crate::models::assistant_acknowledgement::COLLECTION_NAME
            )
            .find_one(doc! {"_id": &card})
            .await
            .unwrap()
            .unwrap()
            .status,
        "allowed"
    );

    let publication = stored.publication.clone().unwrap();
    StartedLearned {
        fixture,
        agent,
        id,
        generated,
        binding,
        card,
        publication,
    }
}

#[tokio::test]
async fn approval_started_failure_uses_durable_boundary_and_expired_lease_resumes() {
    let StartedLearned {
        fixture,
        agent,
        id,
        generated,
        binding,
        card,
        publication,
    } = started_learned_publication("learning_review_approval_resume").await;
    fixture
        .state
        .db
        .collection::<mongodb::bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"status": "publishing", "publication.lease_expires_at": mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let skill_id = Uuid::new_v4().to_string();
    let bytes = publication::package_with_snapshot(
        &generated,
        &publication.operation_id,
        &publication.name,
        &publication.version,
        None,
    )
    .unwrap();
    assert_eq!(publication::hash(&bytes), publication.sha256);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skills"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/skill-search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"items":[{"guid":skill_id,"name":publication.name}],"totalPages":1}}),
        ))
        .mount(&server)
        .await;
    let metadata = json!({"data":{"guid":skill_id,"name":publication.name,"version":publication.version,"skillHash":publication.sha256,"description":"safe guidance","isPrivate":true,"createdBy":fixture.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}});
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{skill_id}/versions/{}/download",
            publication.version
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    let reader = WireReader {
        base: server.uri(),
        client: reqwest::Client::new(),
    };
    let resumed = approve(
        &fixture.state,
        &fixture.chat,
        &agent.id,
        &id,
        &card,
        &binding,
        &reader,
    )
    .await
    .unwrap();
    assert_eq!(resumed["skill_id"], skill_id);
    let final_row = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_row.status, "pinned");
    assert_eq!(
        final_row.publication.unwrap().skill_id.as_deref(),
        Some(skill_id.as_str())
    );
    server.verify().await;
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_learned_evidence_checks_a_dispatched_version_but_never_attaches_it() {
    let StartedLearned {
        fixture,
        agent,
        id,
        generated,
        binding,
        card,
        publication,
    } = started_learned_publication("learning_stale_evidence_check_only").await;
    // Consent is withdrawn after the request may have reached Ornn.
    fixture
        .state
        .db
        .collection::<bson::Document>(CONFIG_COLLECTION_NAME)
        .update_one(doc! {"_id":&agent.id}, doc! {"$set":{"enabled":false}})
        .await
        .unwrap();
    let listed = list(&fixture.state, &fixture.owner, &agent.id, true)
        .await
        .unwrap();
    let item = listed.iter().find(|item| item.id == id).unwrap();
    assert!(!item.evidence_available);
    assert!(item.draft.is_some());
    fixture
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"status": "publishing", "publication.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let skill_id = Uuid::new_v4().to_string();
    let bytes = publication::package_with_snapshot(
        &generated,
        &publication.operation_id,
        &publication.name,
        &publication.version,
        None,
    )
    .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skills"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/skill-search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"items":[{"guid":skill_id,"name":publication.name}],"totalPages":1}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id,"name":publication.name,"version":publication.version,"skillHash":publication.sha256,"description":"safe guidance","isPrivate":true,"createdBy":fixture.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{skill_id}/versions/{}/download",
            publication.version
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    let reader = WireReader {
        base: server.uri(),
        client: reqwest::Client::new(),
    };
    let error = approve(
        &fixture.state,
        &fixture.chat,
        &agent.id,
        &id,
        &card,
        &binding,
        &reader,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::Conflict(message)
        if message == "Learning evidence or consent is no longer available; the verified version was not attached"));
    let stored = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "published_unpinned");
    assert_eq!(stored.failure_code.as_deref(), Some("evidence_unavailable"));
    assert!(!stored.body_encrypted.is_empty());
    let p = stored.publication.as_ref().unwrap();
    assert!(p.verified_at.is_some());
    assert_eq!(p.skill_id.as_deref(), Some(skill_id.as_str()));
    assert_eq!(p.last_stage.as_deref(), Some("pin"));
    assert!(p.lease_expires_at.is_none());
    let unchanged = fixture
        .state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.skills_revision, agent.skills_revision);
    assert!(unchanged.skills.iter().all(|pin| pin.skill_id != skill_id));
    assert_eq!(
        fixture
            .state
            .db
            .collection::<bson::Document>(ROOTS_COLLECTION_NAME)
            .count_documents(doc! {"proposal_id":&id})
            .await
            .unwrap(),
        0
    );
    let listed = list(&fixture.state, &fixture.owner, &agent.id, false)
        .await
        .unwrap();
    let item = listed.iter().find(|item| item.id == id).unwrap();
    assert_eq!(item.status, "published_unpinned");
    assert_eq!(item.failure_code.as_deref(), Some("evidence_unavailable"));
    assert!(!item.evidence_available);
    // Settled: neither a new confirmation nor the approving card can check again.
    let settled = |error: AppError| {
        matches!(error, AppError::Conflict(message)
            if message == "NyxID verified this version but did not attach it; it stays private in Ornn")
    };
    assert!(settled(
        approval_binding(
            &fixture.state,
            &fixture.owner,
            &agent.id,
            &id,
            0,
            agent.skills_revision,
            &UnavailableReader,
        )
        .await
        .unwrap_err()
    ));
    assert!(settled(
        approve(
            &fixture.state,
            &fixture.chat,
            &agent.id,
            &id,
            &card,
            &binding,
            &reader
        )
        .await
        .unwrap_err()
    ));
    server.verify().await;
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn approval_card_is_consumed_once_and_replay_cannot_consume_again() {
    let fixture = orchestrator_fixture("learning_review_card_once").await;
    let binding = json!({"agent_id":"agent","operation_id":"operation","revision":0});
    let card = Uuid::new_v4().to_string();
    let now = Utc::now();
    fixture
        .state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .insert_one(AssistantAcknowledgement {
            authored_skill: None,
            machine_context: None,
            id: card.clone(),
            conversation_id: fixture.chat.conversation_id.clone(),
            user_id: fixture.owner.clone(),
            api_key_id: fixture.chat.api_key_id.clone(),
            kind: "action".into(),
            service_id: None,
            service_slug: None,
            service_name: None,
            platform: false,
            tool_name: Some(TOOL.into()),
            arguments_digest: Some(
                crate::services::assistant_acknowledgement_service::arguments_digest(&binding),
            ),
            operation_selection: None,
            operation_contract_digest: None,
            skill_selection: None,
            summary: "Publish a learning skill".into(),
            status: "allowed".into(),
            requested_turn_id: None,
            voice_request_id: None,
            continuation_receipt_id: None,
            trigger_run_id: None,
            created_at: now,
            decided_at: Some(now),
            expires_at: now + chrono::Duration::hours(1),
            decider: "user".into(),
            request_excerpt: None,
            decided_by: Some("user".into()),
            reason: None,
        })
        .await
        .unwrap();
    let mut session = fixture.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        crate::services::assistant_acknowledgement_service::consume_action_in_session(
            &fixture.state.db,
            &fixture.chat,
            &card,
            TOOL,
            &binding,
            &mut session,
        )
        .await
        .unwrap()
    );
    session.commit_transaction().await.unwrap();
    let mut replay = fixture.state.db.client().start_session().await.unwrap();
    replay.start_transaction().await.unwrap();
    assert!(
        !crate::services::assistant_acknowledgement_service::consume_action_in_session(
            &fixture.state.db,
            &fixture.chat,
            &card,
            TOOL,
            &binding,
            &mut replay,
        )
        .await
        .unwrap()
    );
    replay.abort_transaction().await.unwrap();
    fixture.state.db.drop().await.unwrap();
}

struct TestReader;

#[async_trait::async_trait]
impl OrnnReader for TestReader {
    async fn get(&self, _path: &str) -> AppResult<Vec<u8>> {
        Err(AppError::ServicePoolInfrastructureUnavailable)
    }

    async fn request(
        &self,
        _method: http::Method,
        path: &str,
        _body: Vec<u8>,
    ) -> AppResult<Vec<u8>> {
        if path == "/api/v1/skill-format/validate" {
            return Ok(br#"{"data":{"valid":true,"violations":[]}}"#.to_vec());
        }
        Err(AppError::ServicePoolInfrastructureUnavailable)
    }
}

#[tokio::test]
async fn authored_org_skill_refuses_personal_fallback() {
    let (f, agent, _, _) = org_review_fixture("authored_org_refusal").await;
    let input = serde_json::from_value(json!({"agent":agent.id,"name":"team-review","description":"Review team work","skill_md":"# Review"})).unwrap();
    let error = crate::services::assistant_skill_authoring::create(
        &f.state,
        &f.chat,
        input,
        &UnavailableReader,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(message) if message.contains("owner_binding_unavailable") && message.contains("maintainer"))
    );
    f.state.db.drop().await.unwrap();
}

async fn http_approve(
    fixture: &crate::services::assistant_authority_tests::Fixture,
    agent: &str,
    id: &str,
    body: Value,
) -> AppResult<Value> {
    crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(fixture.state.clone()),
        crate::test_utils::test_auth_user(&fixture.owner),
        axum::extract::Path((agent.to_owned(), id.to_owned())),
        axum::Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .map(|axum::Json(value)| value)
}

#[tokio::test]
async fn http_approve_decides_audits_and_publishes_only_the_learning_card() {
    let SeededLearned {
        fixture,
        agent,
        id,
        generated,
    } = seed_learned_proposal("learning_http_approve_card").await;
    let server = MockServer::start().await;
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.slug = "ornn-api".into();
    service.base_url = server.uri();
    service.identity_propagation_mode = "jwt".into();
    fixture
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&service)
        .await
        .unwrap();
    let requested = http_approve(&fixture, &agent.id, &id, json!({}))
        .await
        .unwrap();
    assert_eq!(requested["status"], "confirmation_required");
    let card_id = requested["acknowledgement"]["acknowledgement_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let cards = fixture.state.db.collection::<AssistantAcknowledgement>(
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
    );
    let card = cards
        .find_one(doc! {"_id":&card_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.status, "pending");
    // Other pending cards the owner holds in the same conversation, even with
    // this exact binding digest, are never decided through this route.
    let mut service_card = card.clone();
    service_card.id = Uuid::new_v4().to_string();
    service_card.kind = "service".into();
    service_card.tool_name = None;
    service_card.arguments_digest = None;
    service_card.service_id = Some(service.id.clone());
    let mut authored_card = card.clone();
    authored_card.id = Uuid::new_v4().to_string();
    authored_card.authored_skill = Some(
        crate::models::assistant_acknowledgement::AuthoredSkillReview {
            agent_id: agent.id.clone(),
            proposal_id: id.clone(),
            revision: 0,
            skills_revision: agent.skills_revision,
        },
    );
    cards.insert_one(&service_card).await.unwrap();
    cards.insert_one(&authored_card).await.unwrap();
    for other in [&service_card, &authored_card] {
        assert!(matches!(
            http_approve(&fixture, &agent.id, &id, json!({"acknowledgement_id":other.id})).await,
            Err(AppError::Conflict(message))
                if message == "Learning approval card is missing, expired, used or stale"
        ));
        let unchanged = cards
            .find_one(doc! {"_id":&other.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.status, "pending");
        assert!(unchanged.decided_at.is_none());
    }
    let audits = fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let decided = doc! {"event_type":"assistant_acknowledgement_decided"};
    assert_eq!(audits.count_documents(decided.clone()).await.unwrap(), 0);
    let pending = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, "pending");
    let p = pending.publication.clone().unwrap();
    assert!(!p.started && p.approved_by.is_none());
    let skill_id = Uuid::new_v4().to_string();
    let bytes =
        publication::package_with_snapshot(&generated, &p.operation_id, &p.name, &p.version, None)
            .unwrap();
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id}})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id,"name":p.name,"version":p.version,"skillHash":p.sha256,"description":"safe guidance","isPrivate":true,"createdBy":fixture.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{skill_id}/versions/{}/download",
            p.version
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    let published = http_approve(
        &fixture,
        &agent.id,
        &id,
        json!({"acknowledgement_id":card_id}),
    )
    .await
    .unwrap();
    assert_eq!(published["status"], "pinned");
    assert_eq!(published["skill_id"], skill_id.as_str());
    assert_eq!(
        cards
            .find_one(doc! {"_id":&card_id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "used"
    );
    let audit = audits.find_one(decided.clone()).await.unwrap().unwrap();
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("decision").unwrap(), "allow");
    assert_eq!(data.get_str("tool_name").unwrap(), TOOL);
    assert_eq!(audits.count_documents(decided).await.unwrap(), 1);
    for other in [&service_card, &authored_card] {
        assert_eq!(
            cards
                .find_one(doc! {"_id":&other.id})
                .await
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
    }
    server.verify().await;
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn migration_retries_undecryptable_drafts_without_reporting_them_unresolved() {
    let fixture = orchestrator_fixture("learning_migration_decrypt_retry").await;
    let mut row = proposal(true);
    row.body_encrypted = vec![9; 64];
    let proposals = fixture
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
    proposals.insert_one(&row).await.unwrap();
    let unresolved = doc! {"event_type":"assistant_learning_migration_unresolved"};
    let audits = fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        migrate_publication_targets(&fixture.state, false)
            .await
            .unwrap(),
        MigrationOutcome::Retry
    );
    assert!(publication_ready(&fixture.state.db).await.is_err());
    assert_eq!(audits.count_documents(unresolved.clone()).await.unwrap(), 0);
    // The bounded reporting pass also tells operators, and still retries.
    assert_eq!(
        migrate_publication_targets(&fixture.state, true)
            .await
            .unwrap(),
        MigrationOutcome::Retry
    );
    let reported = audits.find_one(unresolved.clone()).await.unwrap().unwrap();
    let data = reported.get_document("event_data").unwrap();
    assert_eq!(data.get_str("reason").unwrap(), "undecryptable");
    assert_eq!(data.get_str("proposal_id").unwrap(), row.id);
    assert_eq!(audits.count_documents(unresolved.clone()).await.unwrap(), 1);
    // A draft that decrypts but does not decode needs an operator instead.
    let encrypted = fixture
        .state
        .encryption_keys
        .encrypt(b"not a draft")
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"body_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}},
        )
        .await
        .unwrap();
    assert_eq!(
        migrate_publication_targets(&fixture.state, false)
            .await
            .unwrap(),
        MigrationOutcome::Unresolved
    );
    assert!(publication_ready(&fixture.state.db).await.is_err());
    assert_eq!(
        audits
            .count_documents(
                doc! {"event_type":"assistant_learning_migration_unresolved",
                "event_data.reason":"unclassified"}
            )
            .await
            .unwrap(),
        1
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_learned_legacy_update_is_rebuilt_from_the_panel() {
    let SeededLearned {
        fixture: f,
        agent,
        id,
        mut generated,
    } = seed_learned_proposal("recovery_learned_rebuild").await;
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
    // A tool-based base attached through L1 provenance.
    let base_bytes =
        publication::package_with_snapshot(&generated, "base-operation", "resumable", "1.0", None)
            .unwrap();
    let base = SkillPin {
        source: "ornn".into(),
        skill_id: Uuid::new_v4().to_string(),
        name: "resumable".into(),
        version: "1.0".into(),
        sha256: publication::hash(&base_bytes),
    };
    let reference = SkillReference {
        source: base.source.clone(),
        skill_id: base.skill_id.clone(),
        name: base.name.clone(),
        version: base.version.clone(),
        sha256: base.sha256.clone(),
        dependencies: vec![],
    };
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"skills":bson::to_bson(&vec![reference]).unwrap()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<AssistantAgentLearningSkillRoot>(ROOTS_COLLECTION_NAME)
        .insert_one(AssistantAgentLearningSkillRoot {
            id: Uuid::new_v4().to_string(),
            agent_id: agent.id.clone(),
            owner_id: f.owner.clone(),
            skill_id: base.skill_id.clone(),
            version: base.version.clone(),
            sha256: base.sha256.clone(),
            operation_id: "base-operation".into(),
            active: true,
            proposal_id: "base-proposal".into(),
            config_revision: 0,
            agent_skills_revision: agent.skills_revision,
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    let skill_id = &base.skill_id;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}")))
        .and(wiremock::matchers::query_param("version", "1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":skill_id,
            "name":"resumable","version":"1.0","skillHash":base.sha256,"description":"safe guidance",
            "isPrivate":true,"createdBy":f.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[],
            "metadata":{"category":"tool-based","tools":[{"tool":"search_web"}]}}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{skill_id}/versions/1.0/download"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(base_bytes.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{skill_id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    // The learned improvement as a pre-#1828 server stored it.
    generated.kind = "improve".into();
    generated.base_skill = Some(base.clone());
    generated.skill_md = "# Guidance\nSearch first.".into();
    let body = serde_json::to_vec(&generated).unwrap();
    let encrypted = f.state.encryption_keys.encrypt(&body).await.unwrap();
    let legacy = publication::package_with_snapshot(
        &generated,
        "legacy-operation",
        "resumable",
        "1.1",
        None,
    )
    .unwrap();
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    proposals
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"body_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
            "body_bytes":body.len() as i64,"publication":{"operation_id":"legacy-operation","name":"resumable",
            "version":"1.1","sha256":publication::hash(&legacy),"skill_id":bson::Bson::Null,"started":false,
            "approved_by":bson::Bson::Null,"approval_digest":bson::Bson::Null,"acknowledgement_id":bson::Bson::Null,
            "skills_revision":agent.skills_revision,"lease_id":bson::Bson::Null,"lease_expires_at":bson::Bson::Null}}},
        )
        .await
        .unwrap();
    let listed = list(&f.state, &f.owner, &agent.id, true).await.unwrap();
    assert_eq!(listed[0].failure_code.as_deref(), Some(LEGACY_PACKAGE));
    // The panel cannot raise a confirmation for the old package.
    assert!(matches!(
        approval_binding(&f.state, &f.owner, &agent.id, &id, 0, agent.skills_revision, &UnavailableReader).await,
        Err(AppError::Conflict(message)) if message == LEGACY_PACKAGE));
    let acknowledgements = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME);
    let cards = || acknowledgements.count_documents(doc! {"user_id":&f.owner,"tool_name":TOOL});
    let cards_before = cards().await.unwrap();
    let rebuild = |body: Value| {
        crate::handlers::assistant_agent_learning::reprepare(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((agent.id.clone(), id.clone())),
            axum::Json(serde_json::from_value(body).unwrap()),
        )
    };
    let axum::Json(result) = rebuild(json!({})).await.unwrap();
    assert_eq!(result["status"], "reprepared");
    let rebuilt = f
        .state
        .db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    let p = rebuilt.publication.clone().unwrap();
    assert_eq!((rebuilt.revision, rebuilt.status.as_str()), (1, "pending"));
    assert_ne!(p.operation_id, "legacy-operation");
    assert_eq!((p.package_format, p.version.as_str()), (2, "1.1"));
    assert!(p.interface_encrypted.is_some() && !p.review_card_pending);
    // Learned drafts are confirmed from the panel as usual: no card yet.
    assert_eq!(cards().await.unwrap(), cards_before);
    let listed = list(&f.state, &f.owner, &agent.id, true).await.unwrap();
    assert_eq!(listed[0].failure_code, None);
    let binding = approval_binding(
        &f.state,
        &f.owner,
        &agent.id,
        &id,
        1,
        agent.skills_revision,
        &UnavailableReader,
    )
    .await
    .unwrap();
    assert_eq!(binding["operation_id"], p.operation_id);
    assert_eq!(binding["package_sha256"], p.sha256);
    assert!(rebuild(json!({})).await.is_err());
    for request in server.received_requests().await.unwrap() {
        assert!(request.method.as_str() == "GET");
        let token = request
            .headers
            .get("x-nyxid-identity-token")
            .unwrap()
            .to_str()
            .unwrap();
        let claims: Value = serde_json::from_slice(
            &base64::Engine::decode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                token.split('.').nth(1).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["sub"], f.owner);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_stale_release_candidate_queries_use_indexes() {
    let fixture = orchestrator_fixture("recovery_stale_release_indexes").await;
    crate::db::ensure_indexes(&fixture.state.db).await.unwrap();
    let (aged, unstamped) = recovery::stale_candidate_filters(bson::DateTime::now());
    for filter in [aged, unstamped] {
        let explained = fixture
            .state
            .db
            .run_command(
                doc! {"explain":{"find":PROPOSALS_COLLECTION_NAME,"filter":filter},
                "verbosity":"queryPlanner"},
            )
            .await
            .unwrap();
        let plan = explained
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap()
            .to_string();
        // Both `$or` branches (started, uncertain_dispatch) are index scans.
        assert!(!plan.contains("COLLSCAN"), "{plan}");
        assert!(
            plan.contains("publication.uncertain_dispatch_1_status_1"),
            "{plan}"
        );
        assert!(plan.contains("publication.started_1_status_1"), "{plan}");
    }
    fixture.state.db.drop().await.unwrap();
}
