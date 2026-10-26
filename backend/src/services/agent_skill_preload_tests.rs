use super::*;
use crate::models::{
    assistant_agent::{AgentKind, AgentSkillMetadata},
    catalog_skill_revision::SkillReference,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    sync::atomic::{AtomicUsize, Ordering},
};

const ID: &str = "128393f3-d528-4ce2-b197-f1b13cb8fd5b";

struct Registry {
    bytes: Vec<u8>,
    calls: AtomicUsize,
    deny: bool,
}
impl Registry {
    fn new(entries: &[(&str, &[u8])]) -> Self {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        Self {
            bytes: zip.finish().unwrap().into_inner(),
            calls: AtomicUsize::new(0),
            deny: false,
        }
    }
    fn agent(&self) -> AssistantAgent {
        let now = chrono::Utc::now();
        AssistantAgent {
            channel_checking_notice: false,
            id: "a".into(),
            user_id: "owner".into(),
            kind: AgentKind::Specialist,
            name: "example".into(),
            description: String::new(),
            specialty: None,
            grants: Default::default(),
            guest_access: Default::default(),
            operation_scopes: Default::default(),
            skills: vec![SkillReference {
                source: "ornn".into(),
                skill_id: ID.into(),
                name: "example".into(),
                version: "1.0".into(),
                sha256: hex::encode(Sha256::digest(&self.bytes)),
                dependencies: vec![],
                disabled: false,
            }],
            skills_revision: 1,
            skill_metadata: BTreeMap::from([(
                ID.into(),
                AgentSkillMetadata {
                    preload_paths: vec!["SKILL.md".into(), "references/menu.md".into()],
                    ..Default::default()
                },
            )]),
            operation_scope_revisions: Default::default(),
            machine_node_ids: vec![],
            machine_access: None,
            saved_login_ids: vec![],
            created_by: "user".into(),
            model: "nyxagent/channel".into(),
            access_mode_override: None,
            home_conversation_id: None,
            memory: vec![],
            display_name: None,
            persona: None,
            destroyed_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}
#[async_trait::async_trait]
impl OrnnReader for Registry {
    async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(path, format!("/api/v1/skills/{ID}/versions/1.0/download"));
        if self.deny {
            return Err(AppError::Forbidden("PRIVATE PROVIDER DETAIL".into()));
        }
        Ok(self.bytes.clone())
    }
}

#[tokio::test]
async fn preload_reads_complete_selected_files_once_and_excludes_other_content() {
    let menu = "菜: $10\n".repeat(2000);
    let registry = Registry::new(&[
        ("SKILL.md", b"Core facts"),
        ("references/menu.md", menu.as_bytes()),
        ("references/private.md", b"NOT SELECTED"),
    ]);
    let prompt = instructions(&registry, &registry.agent(), false).await;
    assert!(
        prompt.contains("Core facts") && prompt.contains(&serde_json::to_string(&menu).unwrap())
    );
    assert!(!prompt.contains("NOT SELECTED"));
    assert_eq!(registry.calls.load(Ordering::Relaxed), 1);
    assert!(prompt.contains("untrusted") && prompt.contains("without calling skill_read"));
}

#[tokio::test]
async fn guests_disabled_removed_destroyed_and_default_skills_perform_no_reads() {
    let registry = Registry::new(&[("SKILL.md", b"secret")]);
    let mut agent = registry.agent();
    assert!(instructions(&registry, &agent, true).await.is_empty());
    agent.skills[0].disabled = true;
    assert!(instructions(&registry, &agent, false).await.is_empty());
    agent.skills[0].disabled = false;
    agent.destroyed_at = Some(chrono::Utc::now());
    assert!(instructions(&registry, &agent, false).await.is_empty());
    agent.destroyed_at = None;
    agent.skill_metadata.clear();
    assert!(instructions(&registry, &agent, false).await.is_empty());
    agent.skills.clear();
    assert!(instructions(&registry, &agent, false).await.is_empty());
    assert_eq!(registry.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn denied_tampered_missing_binary_and_oversized_preloads_disclose_no_partial_content() {
    for variant in 0..5 {
        let mut registry = Registry::new(&[
            ("SKILL.md", b"PRIVATE PARTIAL"),
            ("references/menu.md", b"menu"),
        ]);
        let agent = registry.agent();
        match variant {
            0 => registry.deny = true,
            1 => registry.bytes = Registry::new(&[("SKILL.md", b"tampered")]).bytes,
            2 => registry.bytes = Registry::new(&[("SKILL.md", b"PRIVATE PARTIAL")]).bytes,
            3 => {
                registry.bytes = Registry::new(&[
                    ("SKILL.md", b"PRIVATE PARTIAL"),
                    ("references/menu.md", &[0xff]),
                ])
                .bytes
            }
            _ => {
                registry.bytes = Registry::new(&[
                    ("SKILL.md", b"PRIVATE PARTIAL"),
                    ("references/menu.md", &vec![b'a'; MAX_PRELOAD_BYTES]),
                ])
                .bytes
            }
        }
        let mut agent = agent;
        if variant >= 2 {
            agent.skills[0].sha256 = hex::encode(Sha256::digest(&registry.bytes));
        }
        let prompt = instructions(&registry, &agent, false).await;
        assert!(prompt.contains("\"unavailable\":true"));
        assert!(!prompt.contains("PRIVATE") && !prompt.contains("tampered"));
        assert_eq!(registry.calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn preload_path_and_byte_limits_are_explicit_and_atomic() {
    for paths in [
        vec!["../SKILL.md"],
        vec!["/SKILL.md"],
        vec!["SKILL.md", "SKILL.md"],
        vec!["references/menu.md"],
        vec!["SKILL.md", "scripts/run.py"],
        vec!["SKILL.md", "a\\b.md"],
    ] {
        assert!(validate_paths(&paths.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
    }
    let files = BTreeMap::from([("SKILL.md".into(), vec![b'a'; MAX_PRELOAD_BYTES])]);
    assert_eq!(
        selected_files(&files, &["SKILL.md".into()], MAX_PRELOAD_BYTES)
            .unwrap()
            .1,
        MAX_PRELOAD_BYTES
    );
    assert!(selected_files(&files, &["SKILL.md".into()], MAX_PRELOAD_BYTES - 1).is_err());
    assert!(
        selected_files(
            &BTreeMap::from([("SKILL.md".into(), b"a\0b".to_vec())]),
            &["SKILL.md".into()],
            100
        )
        .is_err()
    );
}

#[tokio::test]
async fn saved_preload_is_revision_fenced_and_removable_without_registry_access() {
    let fixture = crate::services::assistant_authority_tests::fixture("skill_preload_save").await;
    let registry = Registry::new(&[("SKILL.md", b"core"), ("references/menu.md", b"menu")]);
    let mut agent = registry.agent();
    agent.id = fixture.chat.agent_id.clone();
    agent.user_id = fixture.owner.clone();
    agent.skill_metadata.clear();
    fixture
        .state
        .db
        .collection::<AssistantAgent>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&agent.id}, &agent)
        .await
        .unwrap();
    let selection = Selection {
        expected_revision: 1,
        paths: vec!["SKILL.md".into(), "references/menu.md".into()],
    };
    let saved = set(
        &fixture.state.db,
        &registry,
        &fixture.owner,
        &agent.id,
        ID,
        &selection,
    )
    .await
    .unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.metadata[ID].preload_bytes, 8);
    // The files reach stable instructions without a model tool call. Reusing
    // the same files preserves the session; removal resets its fingerprint.
    let loaded = assistant_team_service::agent(&fixture.state.db, &fixture.owner, &agent.id)
        .await
        .unwrap();
    let guidance = instructions(&registry, &loaded, false).await;
    let mut row = fixture.row.clone();
    row.nyxagent_session_id = None;
    let prepared = crate::services::assistant_instruction_context::Prepared::with_guidance(
        &[3; 32],
        &row,
        Some(&loaded),
        &[],
        &guidance,
    )
    .unwrap();
    assert!(prepared.instructions().contains("\"SKILL.md\":\"core\""));
    assert!(
        prepared
            .instructions()
            .contains("\"references/menu.md\":\"menu\"")
    );
    let mut binding = prepared.binding.clone();
    binding.session_id = "preloaded-session".into();
    row.nyxagent_session_id = Some(binding.session_id.clone());
    row.nyxagent_instruction_binding = Some(binding);
    assert!(
        !crate::services::assistant_instruction_context::Prepared::with_guidance(
            &[3; 32],
            &row,
            Some(&loaded),
            &[],
            &guidance,
        )
        .unwrap()
        .reset
    );
    assert!(
        crate::services::assistant_instruction_context::Prepared::with_guidance(
            &[3; 32],
            &row,
            Some(&loaded),
            &[],
            "",
        )
        .unwrap()
        .reset
    );
    assert!(matches!(
        set(
            &fixture.state.db,
            &registry,
            &fixture.owner,
            &agent.id,
            ID,
            &selection
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert!(
        set(
            &fixture.state.db,
            &registry,
            "other",
            &agent.id,
            ID,
            &Selection {
                expected_revision: 2,
                paths: vec![]
            }
        )
        .await
        .is_err()
    );
    let denied = Registry {
        deny: true,
        ..registry
    };
    let cleared = set(
        &fixture.state.db,
        &denied,
        &fixture.owner,
        &agent.id,
        ID,
        &Selection {
            expected_revision: 2,
            paths: vec![],
        },
    )
    .await
    .unwrap();
    assert_eq!(cleared.revision, 3);
    assert!(cleared.metadata[ID].preload_paths.is_empty());
    assert_eq!(denied.calls.load(Ordering::Relaxed), 2);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn preload_route_rejects_non_human_credentials_before_reading_a_skill() {
    use axum::{
        Json,
        extract::{Path, State},
    };
    let state = crate::test_utils::test_app_state_no_db().await;
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::Delegated,
        crate::mw::auth::AuthMethod::ServiceAccount,
    ] {
        let mut auth = crate::test_utils::test_auth_user("12345678-1234-4123-8123-123456789012");
        auth.auth_method = method;
        let result = crate::handlers::agent_skills::set_preload(
            State(state.clone()),
            auth,
            Path(("agent".into(), ID.into())),
            Json(Selection {
                expected_revision: 0,
                paths: vec!["SKILL.md".into()],
            }),
        )
        .await;
        assert!(result.is_err());
    }
}
