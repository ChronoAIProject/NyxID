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

fn input(agent: &str) -> Value {
    json!({"agent":agent,"name":"weekly-review","description":"Prepare a weekly review", "skill_md":"# Weekly review\n1. Check outstanding work.\n2. Write a concise report.","files":[{"path":"references/checklist.md","content":"Verify dates before publishing."}]})
}
async fn fixture(name: &str) -> Fixture {
    let f = orchestrator_fixture(name).await;
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
    let result = create(&f.state, &f.chat, serde_json::from_value(value).unwrap())
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
            serde_json::from_value(value.clone()).unwrap()
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
                serde_json::from_value(value.clone()).unwrap()
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":id,"name":p.name,"version":p.version,"skillHash":p.sha256,"description":"Weekly review","isPrivate":true,"createdBy":actor,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}}))).mount(server).await;
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
    let archive = publication::package(&draft, &p.operation_id, &p.name, &p.version).unwrap();
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
        .collection::<bson::Document>(
            crate::models::assistant_agent_learning::ROOTS_COLLECTION_NAME,
        )
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":id,"version":"1.0"});
    revision["skill_md"] = "# Improved weekly review".into();
    let (next, next_card) = make(&f, revision).await;
    assert_eq!(next.publication.as_ref().unwrap().version, "1.1");
    mount_publication(&f, &server, &next, &id, "PUT").await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{"version":"1.0"}]}})),
        )
        .mount(&server)
        .await;
    finish(&f, next_card, true).await.unwrap();
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
