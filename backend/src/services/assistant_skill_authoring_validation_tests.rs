use super::*;

const WORKFLOW: &str = include_str!("../../tests/fixtures/skills/community-workflow-zh.md");

fn body() -> GeneratedProposal {
    serde_json::from_value(json!({"schema_version":1,"kind":"new","name":"community-workflow",
        "description":"公开社区检索与验收流程","skill_md":"# Workflow\nRead, verify, then answer."})).unwrap()
}

fn details(error: AppError) -> Value {
    let response = error.response_body();
    assert_eq!(response.error_code, 1008);
    assert_eq!(response.error, "validation_error");
    let result = crate::services::assistant_account_tools::error_result(error);
    assert!(result.is_error);
    assert_eq!(result.value["details"], response.details.unwrap());
    result.value["details"].clone()
}

#[test]
fn authored_chinese_workflow_passes_with_full_character_budget() {
    assert!((5_000..=7_000).contains(&WORKFLOW.chars().count()));
    let mut draft = body();
    draft.skill_md = WORKFLOW.into();
    let bytes = validate_body(&draft).unwrap();
    assert!(bytes.len() > 8_000);
    assert!(bytes.len() <= validation::MAX_BYTES);
    assert_eq!(
        decode_body(std::str::from_utf8(&bytes).unwrap())
            .unwrap()
            .skill_md,
        WORKFLOW
    );
    // Same content is still outside the learned L1 byte limit.
    assert!(learning::validate_generated(std::str::from_utf8(&bytes).unwrap()).is_err());

    // The maximum four-byte-script draft fits too, including JSON overhead.
    draft.skill_md.clear();
    let overhead = serde_json::to_string(&draft).unwrap().chars().count();
    draft.skill_md = "𠮷".repeat(validation::MAX_CHARS - overhead);
    let encoded = validate_body(&draft).unwrap();
    assert!(encoded.len() > 28_000 && encoded.len() <= 30_000);
}

#[test]
fn authored_examples_do_not_inherit_transcript_or_telemetry_redaction() {
    for example in [
        "Read https://example.org/docs?q=community&version=2.10.3",
        "Contact helper@example.org for this fictional example.",
        "Example 550e8400-e29b-41d4-a716-446655440000",
        "A basic workflow with tokenization and version judgement.",
        "Example docs/community/retrieval/acceptance-checklist.md",
        "Example checksum abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    ] {
        let mut draft = body();
        draft.skill_md = example.into();
        validate_body(&draft).unwrap();
        // Every example above reproduces a false positive in the old path.
        assert!(learning::validate_generated(&serde_json::to_string(&draft).unwrap()).is_err());
    }
    for example in [
        "Version v2.10.3",
        "普通中文流程与路径 docs/example.md",
        "Use nyxid__search_agent_skills and nyx__search_tools",
        "https://example.org/?key=topic",
        "ssh://git@github.com/example/public-repository.git",
    ] {
        let mut draft = body();
        draft.skill_md = example.into();
        validate_body(&draft).unwrap();
    }
}

#[test]
fn authored_credentials_report_exact_field_and_line_without_echo() {
    for credential in [
        "sk-0123456789abcdefghijklmnop",
        "ghp_fixture",
        "password: fixture",
        "api_key=fixture",
        "这里nyxid_ag_fixture后面",
        "Bearer abcdef0123456789abcdef",
        "-----BEGIN RSA PRIVATE KEY-----\nfixture\n-----END RSA PRIVATE KEY-----",
        "https://reader:fixture@example.org/docs",
        "https://example.org/?access_token=fixture",
        "https://example.org/?%74oken=fixture",
        "https://example.org/?X-Amz-Signature=fixture",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.fixture",
    ] {
        let mut draft = body();
        draft.files = (0..3)
            .map(|i| GeneratedFile {
                path: format!("notes/{i}.md"),
                content: "safe".into(),
            })
            .collect();
        draft.files[2].content = format!("{}{}", "公开流程\n".repeat(13), credential);
        let error = validate_body(&draft).unwrap_err();
        assert!(!format!("{error:?}").contains(credential));
        assert_eq!(
            details(error),
            json!({"rule":"credential_shape","field":"files[2].content","line":14})
        );
    }
    for field in [
        "name",
        "description",
        "skill_md",
        "rationale",
        "safety_notes",
        "files[0].path",
        "files[0].content",
    ] {
        let mut draft = body();
        let mut value = serde_json::to_value(&draft).unwrap();
        if field.starts_with("files") {
            value["files"] = json!([{"path":"notes.md","content":"safe"}]);
            value["files"][0][if field.ends_with("path") {
                "path"
            } else {
                "content"
            }] = "ghp_fixture".into();
        } else {
            value[field] = "ghp_fixture".into();
        }
        draft = serde_json::from_value(value).unwrap();
        assert_eq!(
            details(validate_body(&draft).unwrap_err()),
            json!({"rule":"credential_shape","field":field,"line":1})
        );
    }
}

#[test]
fn authored_size_and_structure_diagnostics_are_safe_and_actionable() {
    for (field, maximum) in [
        ("name", 64),
        ("description", 400),
        ("skill_md", 7_500),
        ("rationale", 1_000),
        ("safety_notes", 1_000),
    ] {
        let mut value = serde_json::to_value(body()).unwrap();
        value[field] = "中".repeat(maximum + 1).into();
        let draft = serde_json::from_value(value).unwrap();
        assert_eq!(
            details(validate_body(&draft).unwrap_err()),
            json!({"rule":"too_large","field":field,"limit":maximum,"actual":maximum+1,"unit":"characters"})
        );
    }
    let mut draft = body();
    draft.skill_md = "中".repeat(7_499);
    let actual = serde_json::to_string(&draft).unwrap().chars().count();
    assert_eq!(
        details(validate_body(&draft).unwrap_err()),
        json!({"rule":"too_large","field":"draft","limit":7500,"actual":actual,"unit":"characters"})
    );
    for (path, rule) in [
        ("../private-location.md", "unsafe_path"),
        ("SKILL.md", "duplicate_path"),
    ] {
        let mut draft = body();
        draft.files = vec![GeneratedFile {
            path: path.into(),
            content: "text".into(),
        }];
        assert_eq!(
            details(validate_body(&draft).unwrap_err()),
            json!({"rule":rule,"field":"files[0].path"})
        );
    }
    let mut value = input("agent");
    value["files"][0]["content"] = json!(42);
    assert_eq!(
        details(parse_input(&value).err().unwrap()),
        json!({"rule":"invalid_shape","field":"files[0].content"})
    );
    value["never-echo-this-unknown-key"] = "private value".into();
    assert_eq!(
        details(parse_input(&value).err().unwrap()),
        json!({"rule":"unknown_field","field":"draft"})
    );
}

#[test]
fn authored_file_and_byte_caps_have_positioned_diagnostics() {
    let mut draft = body();
    draft.files = vec![GeneratedFile {
        path: "流程.md".into(),
        content: "中".repeat(2_000),
    }];
    validate_body(&draft).unwrap();
    draft.files[0].content.push('中');
    assert_eq!(
        details(validate_body(&draft).unwrap_err()),
        json!({"rule":"too_large","field":"files[0].content","unit":"characters","limit":2000,"actual":2001})
    );
    draft.files[0].content.clear();
    draft.files[0].path = format!("{}.md", "中".repeat(158));
    assert_eq!(
        details(validate_body(&draft).unwrap_err()),
        json!({"rule":"too_large","field":"files[0].path","unit":"characters","limit":160,"actual":161})
    );
    draft.files = (0..9)
        .map(|i| GeneratedFile {
            path: format!("{i}.md"),
            content: String::new(),
        })
        .collect();
    assert_eq!(
        details(validate_body(&draft).unwrap_err()),
        json!({"rule":"too_large","field":"files","unit":"items","limit":8,"actual":9})
    );
    let error = decode_body(&"中".repeat(10_001)).err().unwrap();
    assert_eq!(
        details(error),
        json!({"rule":"too_large","field":"draft","unit":"bytes","limit":30000,"actual":30003})
    );
}

#[tokio::test]
async fn authored_refusal_creates_no_proposal_or_card() {
    let f = fixture("authoring_refusal_diagnostic").await;
    let mut value = input(&f.chat.agent_id);
    value["files"][0]["content"] = "Public checklist\nhttps://reader:fixture@example.org/".into();
    let error = create(
        &f.state,
        &f.chat,
        parse_input(&value).unwrap(),
        &review::UnavailableReader,
    )
    .await
    .unwrap_err();
    assert_eq!(
        details(error),
        json!({"rule":"credential_shape","field":"files[0].content","line":2})
    );
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
async fn authored_chinese_draft_survives_review_and_publication() {
    Box::pin(async {
        let f = fixture("authoring_chinese").await;
        let server = setup_ornn(&f).await;
        let mut value = input(&f.chat.agent_id);
        value["skill_md"] = WORKFLOW.into();
        let (row, card) = make(&f, value).await;
        let preview = review::authored_preview(&f.state, &f.owner, &row.agent_id, &row.id)
            .await
            .unwrap();
        assert!(
            preview["files"][0]["content"]
                .as_str()
                .unwrap()
                .contains(WORKFLOW)
        );
        let id = Uuid::new_v4().to_string();
        mount_publication(&f, &server, &row, &id, "POST").await;
        assert_eq!(finish(&f, card, true).await.unwrap().status, "used");
        server.verify().await;
        let agent: AssistantAgent = f
            .state
            .db
            .collection(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one(doc! {"_id":&row.agent_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(agent.skills[0].skill_id, id);
        assert_eq!(agent.skills[0].sha256, row.publication.unwrap().sha256);
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn authored_chinese_edit_survives_review_and_publication() {
    Box::pin(async {
        let f = fixture("authoring_chinese_edit").await;
        let server = setup_ornn(&f).await;
        let mut value = input(&f.chat.agent_id);
        value["name"] = "edited-review".into();
        let (row, _) = make(&f, value).await;
        let mut edited = body();
        edited.name = "edited-review".into();
        edited.skill_md = WORKFLOW.into();
        review::edit(
            &f.state,
            &f.owner,
            &row.agent_id,
            &row.id,
            0,
            &serde_json::to_value(&edited).unwrap(),
        )
        .await
        .unwrap();
        let binding = review::approval_binding(
            &f.state,
            &f.owner,
            &row.agent_id,
            &row.id,
            1,
            row.agent_skills_revision,
            &review::UnavailableReader,
        )
        .await
        .unwrap();
        let card = acks::request(
            &f.state.db,
            &f.chat,
            acks::Request {
                kind: "action",
                service: None,
                tool: Some(review::TOOL),
                arguments: Some(&binding),
                summary: "Review edited draft",
                platform: false,
            },
        )
        .await
        .unwrap();
        let row: AssistantAgentLearningProposal = f
            .state
            .db
            .collection(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        let id = Uuid::new_v4().to_string();
        mount_publication(&f, &server, &row, &id, "POST").await;
        assert_eq!(finish(&f, card, true).await.unwrap().status, "used");
        f.state.db.drop().await.unwrap();
    })
    .await;
}
