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
fn durable_failure_status_uses_started_boundary() {
    assert_eq!(failure_status(&proposal(false)), "published_unpinned");
    assert_eq!(failure_status(&proposal(true)), "publication_failed");
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
    (fixture, agent, proposal_id, member)
}

#[tokio::test]
async fn approval_binding_and_approve_refuse_wrong_access() {
    let (fixture, agent, proposal_id, member) =
        org_review_fixture("learning_review_access_refusals").await;
    let binding = approval_binding(&fixture.state, &member, &agent.id, &proposal_id, 0, 0)
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
    };
    assert!(
        matches!(publication::publish(&reader, None, bytes.clone()).await, Err(AppError::Conflict(message)) if message.contains("ambiguous"))
    );
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
    assert!(
        matches!(publication::reconcile(&reader, "owner", &publication, None).await, Err(AppError::Conflict(message)) if message.contains("ambiguous"))
    );
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
    assert!(
        matches!(publication::reconcile(&reader, "owner", &publication, None).await, Err(AppError::Conflict(message)) if message.contains("ambiguous"))
    );
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
async fn approval_started_failure_uses_durable_boundary_and_expired_lease_resumes() {
    let fixture = orchestrator_fixture("learning_review_approval_resume").await;
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
    let binding = approval_binding(
        &fixture.state,
        &fixture.owner,
        &agent.id,
        &id,
        0,
        agent.skills_revision,
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
    let bytes = publication::package(
        &generated,
        &publication.operation_id,
        &publication.name,
        &publication.version,
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
