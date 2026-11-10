use super::*;
use crate::services::{
    assistant_authority_tests::{fixture, orchestrator_fixture},
    assistant_team_service as team,
};
use std::{io::Write, sync::Mutex};

#[test]
fn problem_body_reads_only_the_root_code_and_never_detail() {
    assert_eq!(
        OrnnCode::from_problem_body(
            br#"{"code":"BREAKING_CHANGE_WITHOUT_MAJOR_BUMP","detail":"private skill text"}"#
        ),
        OrnnCode::BreakingChangeWithoutMajorBump
    );
    assert_eq!(
        OrnnCode::from_problem_body(br#"{"error":{"code":"forbidden"},"detail":"private"}"#),
        OrnnCode::Unknown
    );
    assert_eq!(
        OrnnCode::from_problem_body(br#"{"code":"unexpected","detail":"private"}"#),
        OrnnCode::Unknown
    );
    assert_eq!(OrnnCode::from_problem_body(b"not json"), OrnnCode::Unknown);
}

#[test]
fn validation_rules_keep_only_fixed_ids_and_drop_provider_prose() {
    let body = br#"{"data":{"valid":false,"violations":[{"rule":"allowed-root-items","value":"SECRET"},{"rule":"frontmatter.name","message":"PRIVATE"},{"rule":"unknown-rule","detail":"TOKEN"}]},"detail":"DO NOT RETURN"}"#;
    assert_eq!(
        parse_validation_rules(body),
        vec![
            OrnnValidationRule::AllowedRootItems,
            OrnnValidationRule::FrontmatterName,
            OrnnValidationRule::Other
        ]
    );
    assert!(
        !serde_json::to_string(&parse_validation_rules(body))
            .unwrap()
            .contains("SECRET")
    );
    for (path, expected) in [
        (
            "frontmatter.metadata.tool-list.0",
            OrnnValidationRule::FrontmatterToolList,
        ),
        (
            "frontmatter.metadata.runtime-env-var.1",
            OrnnValidationRule::FrontmatterRuntimeEnvVar,
        ),
        (
            "frontmatter.description",
            OrnnValidationRule::FrontmatterDescription,
        ),
        ("frontmatter.PRIVATE_SENTINEL", OrnnValidationRule::Other),
    ] {
        let value =
            serde_json::json!({"errors":[{"code":"frontmatter","path":path,"detail":"SECRET"}]});
        assert_eq!(
            crate::services::assistant_learning_publication::validation_rules(&value),
            vec![expected]
        );
        assert!(
            !serde_json::to_string(
                &crate::services::assistant_learning_publication::validation_rules(&value)
            )
            .unwrap()
            .contains("SENTINEL")
        );
    }
    let problem = br#"{"errors":[{"code":"no-readme-md","path":"README.md","title":"SECRET"}]}"#;
    assert_eq!(
        parse_validation_rules(problem),
        vec![OrnnValidationRule::NoReadmeMd]
    );
}

/// Non-publication readers keep the pre-classification contract exactly:
/// each row is the error the unclassified fetch returned for that case.
#[test]
fn classified_outcomes_keep_native_read_error_contract() {
    type Expect = Box<dyn Fn(&AppError) -> bool>;
    fn forbidden(message: &'static str) -> Expect {
        Box::new(move |error| matches!(error, AppError::Forbidden(m) if m == message))
    }
    fn validation(message: &'static str) -> Expect {
        Box::new(move |error| matches!(error, AppError::ValidationError(m) if m == message))
    }
    const NOT_VISIBLE: &str = "Skill unavailable or not visible through your Ornn access";
    let cases: Vec<(OrnnOutcome, Expect)> = vec![
        (
            OrnnOutcome::LocalRefusal(OrnnLocalRefusal::OperationNotAllowed),
            forbidden("This Ornn operation is not available to agent learning"),
        ),
        (
            OrnnOutcome::LocalRefusal(OrnnLocalRefusal::InvalidPath),
            validation("Invalid skill path"),
        ),
        (
            OrnnOutcome::LocalRefusal(OrnnLocalRefusal::ServiceUnavailable),
            forbidden("Ornn is unavailable; connect your Ornn access and retry"),
        ),
        (
            OrnnOutcome::LocalRefusal(OrnnLocalRefusal::ScopeForbidden("scope detail".into())),
            Box::new(|e| matches!(e, AppError::ApiKeyScopeForbidden(m) if m == "scope detail")),
        ),
        (
            OrnnOutcome::LocalError(AppError::NotFound("Conversation not found".into())),
            Box::new(|e| matches!(e, AppError::NotFound(m) if m == "Conversation not found")),
        ),
        (
            OrnnOutcome::ProxyError,
            forbidden("Skill unavailable through your Ornn access; check the connection and retry"),
        ),
        (
            OrnnOutcome::Response {
                status: 404,
                code: OrnnCode::SkillNotFound,
                rules: vec![],
            },
            forbidden(NOT_VISIBLE),
        ),
        (
            OrnnOutcome::Response {
                status: 403,
                code: OrnnCode::Forbidden,
                rules: vec![],
            },
            forbidden(NOT_VISIBLE),
        ),
        (
            OrnnOutcome::Response {
                status: 409,
                code: OrnnCode::SkillVersionExists,
                rules: vec![],
            },
            forbidden(NOT_VISIBLE),
        ),
        (
            OrnnOutcome::Response {
                status: 503,
                code: OrnnCode::Unknown,
                rules: vec![],
            },
            forbidden(NOT_VISIBLE),
        ),
        (
            OrnnOutcome::Interrupted,
            validation("Ornn response exceeded the skill size limit or was interrupted"),
        ),
        (
            OrnnOutcome::Uncertain,
            Box::new(|e| matches!(e, AppError::ServicePoolInfrastructureUnavailable)),
        ),
    ];
    for (index, (outcome, expected)) in cases.into_iter().enumerate() {
        let error = outcome.into_app_result().unwrap_err();
        assert!(expected(&error), "case {index}: {error:?}");
    }
    assert_eq!(OrnnOutcome::Ok(vec![1]).into_app_result().unwrap(), vec![1]);
}

const ID: &str = "128393f3-d528-4ce2-b197-f1b13cb8fd5b";
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
struct Registry {
    bytes: Vec<u8>,
    calls: Mutex<Vec<String>>,
    deny: bool,
    bad_hash: bool,
    closure: Vec<Value>,
}
impl Registry {
    fn new() -> Self {
        Self {
            bytes: archive(&[
                ("bundle/SKILL.md", b"Use the existing tools.\n"),
                ("bundle/scripts/run.py", b"print('guidance only')"),
            ]),
            calls: Mutex::new(vec![]),
            deny: false,
            bad_hash: false,
            closure: vec![],
        }
    }
    fn reference(&self) -> SkillReference {
        SkillReference {
            source: "ornn".into(),
            skill_id: ID.into(),
            name: "example".into(),
            version: "1.0".into(),
            sha256: hex::encode(Sha256::digest(&self.bytes)),
            dependencies: vec![],
            disabled: false,
        }
    }
}
#[async_trait::async_trait]
impl OrnnReader for Registry {
    async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
        self.calls.lock().unwrap().push(path.into());
        if self.deny {
            return Err(inaccessible());
        }
        if path.ends_with("/download") {
            return Ok(self.bytes.clone());
        }
        if path.contains("/closure") {
            return Ok(serde_json::to_vec(&json!({"data":{"items":self.closure}})).unwrap());
        }
        let hash = if self.bad_hash {
            "0".repeat(64)
        } else {
            self.reference().sha256
        };
        Ok(serde_json::to_vec(&json!({"data":{"guid":ID,"name":"example","description":"Useful\nguidance","version":"1.0","skillHash":hash}})).unwrap())
    }
}
#[tokio::test]
async fn agent_skill_pin_integrity_and_exact_guid_version() {
    let registry = Registry::new();
    let p = preview(&registry, ID, "1.0").await.unwrap();
    assert_eq!(p.reference, registry.reference());
    assert_eq!(p.description, "Useful guidance");
    assert_eq!(p.size_bytes, registry.bytes.len());
    assert!(
        registry
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|p| p.contains(ID))
    );
    let broken = Registry {
        bad_hash: true,
        ..Registry::new()
    };
    assert!(
        matches!(preview(&broken,ID,"1.0").await,Err(AppError::ValidationError(message)) if message.contains("SHA-256"))
    );
    assert!(preview(&registry, ID, "latest").await.is_err());
}

#[tokio::test]
async fn agent_skill_invisible_cannot_be_pinned_and_removal_needs_no_ornn() {
    let f = orchestrator_fixture("agent_skill_visibility").await;
    let registry = Registry::new();
    let selection = Selection {
        expected_revision: 0,
        skills: vec![registry.reference()],
    };
    let denied = Registry {
        deny: true,
        ..Registry::new()
    };
    assert!(
        set(
            &f.state.db,
            &denied,
            &f.owner,
            &f.chat.agent_id,
            &selection,
            true
        )
        .await
        .is_err()
    );
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .skills_revision,
        0
    );
    let result = set(
        &f.state.db,
        &registry,
        &f.owner,
        &f.chat.agent_id,
        &selection,
        true,
    )
    .await
    .unwrap();
    assert_eq!(result.revision, 1);
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::assistant_agent_learning::ROOTS_COLLECTION_NAME,
            )
            .count_documents(mongodb::bson::doc! {})
            .await
            .unwrap(),
        0,
        "ordinary B2 pinning must not touch dormant learning provenance"
    );
    let remove = Selection {
        expected_revision: 1,
        skills: vec![],
    };
    assert!(
        set(
            &f.state.db,
            &denied,
            &f.owner,
            &f.chat.agent_id,
            &remove,
            false
        )
        .await
        .unwrap()
        .skills
        .is_empty()
    );
    assert!(denied.calls.lock().unwrap().len() == 1);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn agent_skill_stale_revision_and_other_owner_refused() {
    let f = fixture("agent_skill_revision").await;
    let registry = Registry::new();
    let selection = Selection {
        expected_revision: 0,
        skills: vec![registry.reference()],
    };
    assert!(
        set(
            &f.state.db,
            &registry,
            &f.owner,
            &f.chat.agent_id,
            &selection,
            false
        )
        .await
        .is_err()
    );
    set(
        &f.state.db,
        &registry,
        &f.owner,
        &f.chat.agent_id,
        &selection,
        true,
    )
    .await
    .unwrap();
    assert!(matches!(
        set(
            &f.state.db,
            &registry,
            &f.owner,
            &f.chat.agent_id,
            &selection,
            true
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert!(
        set(
            &f.state.db,
            &registry,
            &uuid::Uuid::new_v4().to_string(),
            &f.chat.agent_id,
            &selection,
            true
        )
        .await
        .is_err()
    );
    let granted = team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Replace {
            grants: Default::default(),
            guests: Default::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(granted.skills, selection.skills);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn agent_skill_reader_uses_only_attached_pins_and_never_runs_scripts() {
    let f = fixture("agent_skill_reader").await;
    let registry = Registry::new();
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert!(
        read(&registry, &agent, ID, None, "SKILL.md", 0)
            .await
            .is_err()
    );
    assert!(registry.calls.lock().unwrap().is_empty());
    agent.skills = vec![registry.reference()];
    let page = read(&registry, &agent, ID, None, "scripts/run.py", 0)
        .await
        .unwrap();
    assert_eq!(page["content"], "print('guidance only')");
    assert_eq!(page["untrusted_guidance"], true);
    assert!(
        read(
            &registry,
            &agent,
            "another-agent-skill",
            None,
            "SKILL.md",
            0
        )
        .await
        .is_err()
    );
    assert!(
        read(&registry, &agent, ID, Some("not-attached"), "SKILL.md", 0)
            .await
            .is_err()
    );
    assert!(
        read(&registry, &agent, ID, None, "../private", 0)
            .await
            .is_err()
    );
    assert!(
        read(
            &Registry {
                deny: true,
                ..Registry::new()
            },
            &agent,
            ID,
            None,
            "SKILL.md",
            0
        )
        .await
        .is_err()
    );
    let tampered = Registry {
        bytes: archive(&[("SKILL.md", b"changed after pinning")]),
        ..Registry::new()
    };
    assert!(
        matches!(read(&tampered, &agent, ID, None, "SKILL.md", 0).await, Err(AppError::ValidationError(message)) if message.contains("SHA-256"))
    );
    let mut dependency = registry.reference();
    dependency.skill_id = uuid::Uuid::new_v4().to_string();
    dependency.name = "dependency".into();
    agent.skills[0].dependencies.push(SkillPin {
        source: dependency.source,
        skill_id: dependency.skill_id,
        name: dependency.name,
        version: dependency.version,
        sha256: dependency.sha256,
    });
    assert!(
        read(&registry, &agent, ID, Some("dependency"), "SKILL.md", 0)
            .await
            .is_ok()
    );
    agent.skills.clear();
    assert!(
        read(&registry, &agent, ID, None, "SKILL.md", 0)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}
#[test]
fn agent_skill_pages_reconstruct_utf8_within_mcp_budget() {
    let text = "\u{0001}é🙂\\\"\n".repeat(1900);
    let mut offset = 0;
    let mut output = String::new();
    let mut saw_partial = false;
    loop {
        let result = page(&text, ID, "1.0", "SKILL.md", offset).unwrap();
        let wire = json!({"content":[{"type":"text","text":result.to_string()}],"isError":false})
            .to_string();
        assert!(wire.len() < 10_000, "{}", wire.len());
        output.push_str(result["content"].as_str().unwrap());
        if let Some(next) = result["next_offset"].as_u64() {
            saw_partial = true;
            assert_eq!(result["complete"], false);
            assert!(result["continuation_hint"].as_str().is_some());
            assert!(next as usize > offset);
            offset = next as usize;
        } else {
            assert_eq!(result["complete"], true);
            assert!(result["continuation_hint"].is_null());
            break;
        }
    }
    assert_eq!(output, text);
    assert!(saw_partial);
    assert!(page("é", ID, "1.0", "SKILL.md", 1).is_err());
    assert!(page("x", ID, "1.0", "SKILL.md", 10).is_err());
}
#[test]
fn agent_skill_zip_rejects_traversal_duplicates_bombs_and_binary_text() {
    for bad in [
        "../SKILL.md",
        "/SKILL.md",
        "x/../../SKILL.md",
        "x\\SKILL.md",
    ] {
        assert!(package(&archive(&[(bad, b"x")])).is_err());
    }
    let duplicate = archive(&[("SKILL.md", b"a"), ("SKILX.md", b"b")]);
    let duplicate = duplicate
        .windows(8)
        .enumerate()
        .filter_map(|(i, w)| (w == b"SKILX.md").then_some(i))
        .collect::<Vec<_>>()
        .into_iter()
        .fold(duplicate, |mut bytes, i| {
            bytes[i + 4] = b'L';
            bytes
        });
    assert!(package(&duplicate).is_err());
    assert!(package(&archive(&[("SKILL.md", &vec![b'a'; MAX_EXPANDED + 1])])).is_err());
    assert!(package(b"not a zip").is_err());
}
#[tokio::test]
async fn agent_skill_legacy_documents_and_turn_metadata() {
    let f = fixture("agent_skill_legacy").await;
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    let mut doc = bson::to_document(&agent).unwrap();
    for field in ["skills", "skills_revision", "skill_metadata"] {
        doc.remove(field);
    }
    let old: AssistantAgent = bson::from_document(doc).unwrap();
    assert!(old.skills.is_empty());
    assert_eq!(old.skills_revision, 0);
    assert!(instructions(&old).is_empty());
    agent.skills = vec![Registry::new().reference()];
    agent.skill_metadata.insert(
        ID.into(),
        AgentSkillMetadata {
            description: "bounded description".into(),
            size_bytes: 99,
            preload_paths: vec![],
            preload_bytes: 0,
        },
    );
    let prompt = instructions(&agent);
    assert!(prompt.contains("1.0") && prompt.contains("bounded description"));
    assert!(!prompt.contains(&"0".repeat(64)) && !prompt.contains("scripts/run.py"));
    let mut guest = f.row.clone();
    guest.guest_turn = true;
    assert!(
        !crate::services::assistant_nyxagent::base_prompt(&guest, Some(&agent))
            .contains("bounded description")
    );
    f.state.db.drop().await.unwrap();
}
#[test]
fn agent_skill_bounds_reject_duplicate_and_unpinned_refs() {
    let registry = Registry::new();
    assert!(
        validate(&Selection {
            expected_revision: 0,
            skills: vec![registry.reference(); 17]
        })
        .is_err()
    );
    assert!(
        validate(&Selection {
            expected_revision: 0,
            skills: vec![registry.reference(); 2]
        })
        .is_err()
    );
    let mut r = registry.reference();
    r.source = "elsewhere".into();
    assert!(
        validate(&Selection {
            expected_revision: 0,
            skills: vec![r]
        })
        .is_err()
    );
}

#[tokio::test]
async fn agent_skill_concurrent_pin_changes_do_not_overwrite() {
    let f = fixture("agent_skill_concurrent").await;
    let reader = Registry::new();
    let selection = Selection {
        expected_revision: 0,
        skills: vec![reader.reference()],
    };
    let (a, b) = tokio::join!(
        set(
            &f.state.db,
            &reader,
            &f.owner,
            &f.chat.agent_id,
            &selection,
            true
        ),
        set(
            &f.state.db,
            &reader,
            &f.owner,
            &f.chat.agent_id,
            &selection,
            true
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap()
            .skills_revision,
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn agent_skill_cannot_pin_an_invisible_dependency() {
    struct DependencyReader(Registry);
    #[async_trait::async_trait]
    impl OrnnReader for DependencyReader {
        async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
            let dependency_id = "338393f3-d528-4ce2-b197-f1b13cb8fd5b";
            if path.contains(dependency_id) {
                return Err(inaccessible());
            }
            if path.contains("/closure") {
                return Ok(serde_json::to_vec(&json!({"data":{"items":[{"guid":dependency_id,"name":"private-dependency","version":"1.0","skillHash":self.0.reference().sha256}]}})).unwrap());
            }
            self.0.get(path).await
        }
    }
    assert!(
        preview(&DependencyReader(Registry::new()), ID, "1.0")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn agent_skill_version_pages_keep_older_pins_available() {
    struct Versions;
    #[async_trait::async_trait]
    impl OrnnReader for Versions {
        async fn get(&self, _: &str) -> AppResult<Vec<u8>> {
            Ok(serde_json::to_vec(&json!({"data":{"items":(0..27).rev().map(|v| json!({"version":format!("1.{v}"),"skillHash":"a".repeat(64),"isDeprecated":false})).collect::<Vec<_>>()}})).unwrap())
        }
    }
    let first = versions(&Versions, ID, 1).await.unwrap();
    let last = versions(&Versions, ID, 2).await.unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 20);
    assert_eq!(last["items"].as_array().unwrap().len(), 7);
    assert_eq!(last["items"][6]["version"], "1.0");
    assert_eq!(last["total_pages"], 2);
    assert!(first.to_string().len() < 8000);
}

#[tokio::test]
async fn disabled_pins_are_omitted_from_prompt_instructions() {
    let f = fixture("agent_skill_disabled_prompt").await;
    let reader = Registry::new();
    let selection = Selection {
        expected_revision: 0,
        skills: vec![reader.reference()],
    };
    set(
        &f.state.db,
        &reader,
        &f.owner,
        &f.chat.agent_id,
        &selection,
        true,
    )
    .await
    .unwrap();
    let mut toggled = reader.reference();
    toggled.disabled = true;
    let toggled_selection = Selection {
        expected_revision: 1,
        skills: vec![toggled],
    };
    // Disabling is refused until the rollout flag is on, before any write.
    let refused = set(
        &f.state.db,
        &reader,
        &f.owner,
        &f.chat.agent_id,
        &toggled_selection,
        true,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&refused, AppError::Forbidden(m) if m == "Skill pin disabling is not enabled for this rollout"),
        "{refused:?}"
    );
    let unchanged = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(unchanged.skills_revision, 1);
    assert!(!unchanged.skills[0].disabled);
    crate::services::feature_flag_service::set_platform_override(
        &f.state.db,
        crate::services::feature_flag_service::SKILL_PIN_DISABLE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    set(
        &f.state.db,
        &reader,
        &f.owner,
        &f.chat.agent_id,
        &toggled_selection,
        true,
    )
    .await
    .expect("disabling an existing pin must not require a re-preview");
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert!(agent.skills[0].disabled);
    // The change is in the agent's skill history with each pin's state.
    let history = history(&f.state.db, &agent).await.unwrap();
    let latest = &history.items[0];
    assert_eq!(latest.event_type, "assistant_skills_updated");
    assert_eq!(latest.revision, Some(2));
    assert_eq!(latest.skills.len(), 1);
    assert_eq!(latest.skills[0].skill_id, reader.reference().skill_id);
    assert!(latest.skills[0].disabled);
    assert!(!history.items[1].skills[0].disabled);
    let mut disabled = reader.reference();
    disabled.skill_id = "disabled-skill".into();
    disabled.name = "Disabled Skill".into();
    agent.skills = vec![reader.reference(), disabled];
    agent.skills[1].disabled = true;
    let prompt = instructions(&agent);
    assert!(prompt.contains(&reader.reference().name));
    assert!(!prompt.contains("Disabled Skill"));
    for pin in &mut agent.skills {
        pin.disabled = true;
    }
    assert!(instructions(&agent).is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn disabled_pins_are_inaccessible_to_skill_read() {
    let f = fixture("agent_skill_disabled_read").await;
    let reader = Registry::new();
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    agent.skills = vec![reader.reference()];
    assert!(read(&reader, &agent, ID, None, "/", 0).await.is_ok());
    agent.skills[0].disabled = true;
    assert!(matches!(
        read(&reader, &agent, ID, None, "/", 0).await,
        Err(AppError::Forbidden(_))
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn re_enabling_a_disabled_pin_needs_the_owner_card_but_disabling_does_not() {
    let f = fixture("agent_skill_reenable_card").await;
    let reader = Registry::new();
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    agent.skills = vec![reader.reference()];
    let selection = |disabled: bool| {
        let mut pin = reader.reference();
        pin.disabled = disabled;
        Selection {
            expected_revision: 0,
            skills: vec![pin],
        }
    };
    assert!(!needs_confirmation(&agent, &selection(true)));
    assert!(!needs_confirmation(&agent, &selection(false)));
    agent.skills[0].disabled = true;
    assert!(needs_confirmation(&agent, &selection(false)));
    assert!(!needs_confirmation(&agent, &selection(true)));
    let mut other = reader.reference();
    other.skill_id = "other".into();
    assert!(needs_confirmation(
        &agent,
        &Selection {
            expected_revision: 0,
            skills: vec![other],
        }
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_set_agent_skills_attaches_with_receipt_while_auto_asks() {
    use crate::models::assistant_settings::AccessMode;
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_settings_service as settings,
    };
    for (mode, asks) in [(AccessMode::Autopilot, false), (AccessMode::Nyxmode, true)] {
        let f = orchestrator_fixture("mode_skills_widen").await;
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                access_mode: Some(mode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let registry = Registry::new();
        let selection = Selection {
            expected_revision: 0,
            skills: vec![registry.reference()],
        };
        let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        let args = json!({"subagent": agent.id, "selection": selection});
        let result = set_for_chat(
            &f.state.db,
            &registry,
            &f.chat,
            &agent,
            &selection,
            &args,
            "Attach example 1.0 to NyxBot",
        )
        .await
        .unwrap();
        let current = team::agent(&f.state.db, &f.owner, &agent.id).await.unwrap();
        let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        if asks {
            assert_eq!(result.err().unwrap()["error"], "acknowledgement_required");
            assert!(current.skills.is_empty());
            assert_eq!(rows[0].status, "pending");
            assert!(registry.calls.lock().unwrap().is_empty());
        } else {
            assert_eq!(result.unwrap().revision, 1);
            assert_eq!(current.skills, selection.skills);
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].decided_by.as_deref(), Some("access_mode"));
            assert_eq!(
                rows[0].tool_name.as_deref(),
                Some("nyxid__set_agent_skills")
            );
            assert_eq!(rows[0].summary, "Attach example 1.0 to NyxBot");
            assert_eq!(rows[0].confirmation.as_ref().unwrap().risk, "sensitive");
            assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "access_mode");
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn full_access_failed_skill_attach_writes_no_receipt() {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_settings_service as settings,
    };
    let f = orchestrator_fixture("mode_skills_failure").await;
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let registry = Registry {
        deny: true,
        ..Registry::new()
    };
    let selection = Selection {
        expected_revision: 0,
        skills: vec![registry.reference()],
    };
    let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    let args = json!({"subagent": agent.id, "selection": selection});
    assert!(
        set_for_chat(
            &f.state.db,
            &registry,
            &f.chat,
            &agent,
            &selection,
            &args,
            "Attach example 1.0"
        )
        .await
        .is_err()
    );
    assert!(
        acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

const OUTLINE_SKILL: &str = "---\nname: example\ndescription: Example.\nmetadata:\n  category: plain\n---\n# Example\n\n## Overview\nYou get one tidy summary.\n\n## Approach\n1. **Read.** I read the inputs.\n2. **Summarise.** I write the summary.\n   Asks you\n\n## Needs and produces\nNeeds: the inputs.\nProduces: one summary.\n\n## Details\nScripts are text only.\n";

fn outline_registry(skill_md: &str) -> Registry {
    Registry {
        bytes: archive(&[("SKILL.md", skill_md.as_bytes())]),
        ..Registry::new()
    }
}

/// A valid outline at every hard limit, in CJK (three UTF-8 bytes per character).
fn max_cjk_outline() -> String {
    use crate::services::skill_outline as outline;
    let mut skill = format!(
        "## Overview\n{}\n\n## Approach\n",
        "概".repeat(outline::MAX_OVERVIEW)
    );
    for n in 1..=outline::MAX_STEPS {
        skill.push_str(&format!(
            "{n}. **{}** {}\n",
            "题".repeat(outline::MAX_TITLE),
            "步".repeat(outline::MAX_STEP_TEXT)
        ));
    }
    skill.push_str(&format!(
        "\n## Needs and produces\nNeeds: {}\nProduces: {}\n",
        "需".repeat(outline::MAX_NEEDS),
        "产".repeat(outline::MAX_NEEDS)
    ));
    skill
}

/// Pages `fetch` from offset 0 to completion, checking every page's budget.
async fn collect_pages<F, Fut>(mut fetch: F) -> String
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Value>,
{
    let mut offset = 0;
    let mut output = String::new();
    let mut pages = 0;
    loop {
        let page = fetch(offset).await;
        assert!(wire_len(&page) <= 9200, "{}", wire_len(&page));
        assert_eq!(page["untrusted_guidance"], true);
        output.push_str(page["content"].as_str().unwrap());
        pages += 1;
        match page["next_offset"].as_u64() {
            Some(next) => offset = next as usize,
            None => break,
        }
    }
    assert!(
        pages > 1,
        "the maximum CJK outline needs more than one page"
    );
    output
}

#[tokio::test]
async fn preview_outline_is_bounded_untrusted_and_absent_without_template() {
    let registry = outline_registry(OUTLINE_SKILL);
    let verified = preview(&registry, ID, "1.0").await.unwrap();
    // The verified SKILL.md never appears in the plain preview response.
    let plain = json!(verified);
    assert!(plain.get("skill_md").is_none());
    assert!(!plain.to_string().contains("tidy summary"));
    let value = preview_with_outline(&verified, None).unwrap();
    assert_eq!(value["reference"], plain["reference"]);
    assert!(value.get("skill_md").is_none());
    let outline = &value["outline"];
    assert_eq!(outline["untrusted_guidance"], true);
    assert_eq!(outline["complete"], true);
    let content = outline["content"].as_str().unwrap();
    assert!(content.starts_with("Overview:\nYou get one tidy summary."));
    assert!(content.contains("2. Summarise: I write the summary.\n   Asks you"));
    // outline_offset asks for an outline-only page.
    let only = preview_with_outline(&verified, Some(0)).unwrap();
    assert!(only.get("reference").is_none());
    assert_eq!(only["content"], outline["content"]);

    // A legacy skill keeps working; it simply has no outline.
    let legacy = preview(&Registry::new(), ID, "1.0").await.unwrap();
    let value = preview_with_outline(&legacy, None).unwrap();
    assert_eq!(value["outline"]["outline_available"], false);
    assert!(value["outline"].get("content").is_none());
}

#[tokio::test]
async fn preview_with_maximum_metadata_defers_the_outline_within_budget() {
    let mut registry = outline_registry(OUTLINE_SKILL);
    let hash = registry.reference().sha256;
    // Sixteen dependencies with long accepted names leave too little room.
    registry.closure = (0..16)
        .map(|n| json!({"guid": uuid::Uuid::new_v4().to_string(), "name": format!("{n:02}{}", "n".repeat(240)), "version": "1.0", "skillHash": hash}))
        .collect();
    let verified = preview(&registry, ID, "1.0").await.unwrap();
    assert_eq!(verified.reference.dependencies.len(), 16);
    let value = preview_with_outline(&verified, None).unwrap();
    assert!(wire_len(&value) <= 9200, "{}", wire_len(&value));
    assert_eq!(value["outline"]["outline_available"], true);
    assert_eq!(value["outline"]["next_offset"], 0);
    assert!(value["outline"].get("content").is_none());
    let only = preview_with_outline(&verified, Some(0)).unwrap();
    assert!(wire_len(&only) <= 9200);
    assert!(only["content"].as_str().unwrap().starts_with("Overview:"));

    // A seventeenth dependency is refused before any outline is read.
    registry.closure.push(json!({"guid": uuid::Uuid::new_v4().to_string(), "name": "extra", "version": "1.0", "skillHash": hash}));
    assert!(preview(&registry, ID, "1.0").await.is_err());
}

#[tokio::test]
async fn max_size_cjk_outline_pages_through_skill_read_and_preview() {
    let f = fixture("agent_skill_outline_cjk").await;
    let skill = max_cjk_outline();
    let expected = crate::services::skill_outline::render(
        &crate::services::skill_outline::parse(&skill, &[]).expect("valid at the limits"),
    );
    let registry = outline_registry(&skill);
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    agent.skills = vec![registry.reference()];
    let via_read = collect_pages(|offset| {
        let (registry, agent) = (&registry, &agent);
        async move {
            read_outline(registry, agent, ID, None, offset)
                .await
                .unwrap()
        }
    })
    .await;
    assert_eq!(via_read, expected);

    let verified = preview(&registry, ID, "1.0").await.unwrap();
    let first = preview_with_outline(&verified, None).unwrap();
    assert!(wire_len(&first) <= 9200, "{}", wire_len(&first));
    let via_preview = collect_pages(|offset| {
        let verified = &verified;
        async move { preview_with_outline(verified, Some(offset)).unwrap() }
    })
    .await;
    assert_eq!(via_preview, expected);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn outline_reads_only_attached_pins_and_resolves_uses_in_the_root_closure() {
    let f = fixture("agent_skill_outline_reader").await;
    let dependency_id = uuid::Uuid::new_v4().to_string();
    let root_md = "---\nname: example\nmetadata:\n  depends-on:\n    - helper@1.0\n---\n## Overview\nOne result.\n\n## Approach\n1. **Prepare.** I get ready.\n   Uses: helper\n";
    let registry = outline_registry(root_md);
    let mut agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert!(read_outline(&registry, &agent, ID, None, 0).await.is_err());
    assert!(registry.calls.lock().unwrap().is_empty());
    let mut reference = registry.reference();
    reference.dependencies.push(SkillPin {
        source: "ornn".into(),
        skill_id: dependency_id.clone(),
        name: "helper".into(),
        version: "1.0".into(),
        sha256: reference.sha256.clone(),
    });
    agent.skills = vec![reference.clone()];
    let page = read_outline(&registry, &agent, ID, None, 0).await.unwrap();
    assert_eq!(page["path"], "outline");
    assert_eq!(page["untrusted_guidance"], true);
    assert!(page["content"].as_str().unwrap().contains(&format!(
        "Uses: helper (skill {dependency_id}, version 1.0)"
    )));
    // A pinned dependency is readable through its root; same package here.
    assert!(
        read_outline(&registry, &agent, ID, Some("helper"), 0)
            .await
            .is_ok()
    );
    // A dependency that is not pinned with this root is refused.
    assert!(
        read_outline(&registry, &agent, ID, Some("not-pinned"), 0)
            .await
            .is_err()
    );
    // Disabled pins and tampered packages are refused like ordinary reads.
    let tampered = outline_registry("changed after pinning");
    assert!(
        matches!(read_outline(&tampered, &agent, ID, None, 0).await, Err(AppError::ValidationError(message)) if message.contains("SHA-256"))
    );
    agent.skills[0].disabled = true;
    assert!(read_outline(&registry, &agent, ID, None, 0).await.is_err());
    f.state.db.drop().await.unwrap();
}
