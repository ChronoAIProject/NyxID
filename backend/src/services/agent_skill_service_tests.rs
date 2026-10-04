use super::*;
use crate::services::{
    assistant_authority_tests::{fixture, orchestrator_fixture},
    assistant_team_service as team,
};
use std::{io::Write, sync::Mutex};
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
            return Ok(serde_json::to_vec(&json!({"data":{"items":[]}})).unwrap());
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
    loop {
        let result = page(&text, ID, "1.0", "SKILL.md", offset).unwrap();
        let wire = json!({"content":[{"type":"text","text":result.to_string()}],"isError":false})
            .to_string();
        assert!(wire.len() < 10_000, "{}", wire.len());
        output.push_str(result["content"].as_str().unwrap());
        if let Some(next) = result["next_offset"].as_u64() {
            assert!(next as usize > offset);
            offset = next as usize;
        } else {
            break;
        }
    }
    assert_eq!(output, text);
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
