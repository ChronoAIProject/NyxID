use super::*;
use crate::models::{assistant_agent::AgentKind, catalog_skill_revision::SkillPin};
use crate::services::{
    assistant_authority_tests::{fixture, orchestrator_fixture},
    feature_flag_service as flags,
};
use sha2::{Digest, Sha256};
use std::io::Write;

const ID: &str = "00000000-0000-4000-8000-000000000001";
const DEP: &str = "00000000-0000-4000-8000-000000000002";

struct Registry {
    bytes: Vec<u8>,
    private: Option<Value>,
    dependency_private: Option<Value>,
    bad_hash: bool,
}
impl Registry {
    fn new() -> Self {
        Self::with_markdown("Package content, untrusted guidance")
    }
    fn with_markdown(markdown: &str) -> Self {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("SKILL.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(markdown.as_bytes()).unwrap();
        Self {
            bytes: zip.finish().unwrap().into_inner(),
            private: Some(json!(false)),
            dependency_private: Some(json!(false)),
            bad_hash: false,
        }
    }
    fn reference(&self) -> SkillReference {
        let hash = hex::encode(Sha256::digest(&self.bytes));
        SkillReference {
            source: "ornn".into(),
            skill_id: ID.into(),
            name: "nyxbot".into(),
            version: "1.0".into(),
            sha256: hash.clone(),
            disabled: false,
            dependencies: vec![SkillPin {
                source: "ornn".into(),
                skill_id: DEP.into(),
                name: "dependency".into(),
                version: "1.0".into(),
                sha256: hash,
            }],
        }
    }
    fn value(&self, path: &str) -> AppResult<Vec<u8>> {
        if path.contains("/download") {
            return Ok(self.bytes.clone());
        }
        if path.contains("skill-search") {
            return Ok(serde_json::to_vec(&json!({"data":{"items":[{"guid":ID,"name":"nyxbot","description":"Useful"}],"totalPages":1}})).unwrap());
        }
        if !path.ends_with("version=1.0") {
            return Err(invalid("Unknown skill version"));
        }
        let pin = self.reference();
        let value = if path.contains("/closure") {
            json!({"items":[{"guid":DEP,"name":"dependency","version":"1.0","skillHash":pin.sha256}]})
        } else {
            let dependency = path.contains(DEP);
            let mut value = json!({"guid":if dependency {DEP} else {ID},"name":if dependency {"dependency"} else {"nyxbot"},"version":"1.0","skillHash":if self.bad_hash {"f".repeat(64)} else {pin.sha256},"description":"Useful\nguidance"});
            if let Some(private) = if dependency {
                &self.dependency_private
            } else {
                &self.private
            } {
                value["isPrivate"] = private.clone();
            }
            value
        };
        Ok(serde_json::to_vec(&json!({"data":value})).unwrap())
    }
    async fn mount(&self, server: &wiremock::MockServer) {
        for path in [
            format!("/api/v1/skills/{ID}"),
            format!("/api/v1/skills/{DEP}"),
            format!("/api/v1/skills/{ID}/closure"),
            format!("/api/v1/skills/{ID}/versions/1.0/download"),
            format!("/api/v1/skills/{DEP}/versions/1.0/download"),
        ] {
            let query = if path.ends_with("download") {
                path.clone()
            } else {
                format!("{path}?version=1.0")
            };
            wiremock::Mock::given(wiremock::matchers::path(path))
                .respond_with(
                    wiremock::ResponseTemplate::new(200)
                        .set_body_bytes(self.value(&query).unwrap()),
                )
                .mount(server)
                .await;
        }
    }
}
#[async_trait::async_trait]
impl OrnnReader for Registry {
    async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
        self.value(path)
    }
}

fn agent() -> AssistantAgent {
    bson::from_document(doc! {"_id":"agent","user_id":"person","kind":"nyxbot","name":"nyxbot","created_by":"user","model":"default","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap()
}
fn system(registry: &Registry) -> AssistantSystemSkills {
    AssistantSystemSkills {
        id: DOCUMENT_ID.into(),
        revision: 1,
        skills: vec![SystemSkill {
            reference: registry.reference(),
            description: "Useful guidance".into(),
            size_bytes: registry.bytes.len(),
        }],
        updated_by: "admin".into(),
        updated_at: Utc::now(),
    }
}
fn resolved(system: AssistantSystemSkills) -> Context {
    Context {
        system,
        mode: SystemSkillsForSpecialists::Default,
        nyxbot_overrides: BTreeMap::new(),
    }
}
fn selection(revision: i64, entries: &[(&str, &str)]) -> Selection {
    Selection {
        expected_revision: revision,
        skills: entries
            .iter()
            .map(|(skill, version)| SelectionEntry {
                skill: (*skill).into(),
                version: (*version).into(),
            })
            .collect(),
    }
}
fn actor(person: &str) -> AuditActor {
    AuditActor {
        user_id: person.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}
async fn flag(db: &Database, person: &str, on: bool) {
    flags::set_platform_override(
        db,
        flags::SYSTEM_SKILLS_FLAG_KEY,
        &flags::FlagTarget::User(person.into()),
        on,
        person,
    )
    .await
    .unwrap();
}

#[test]
fn bson_roundtrip_and_legacy_agent_defaults() {
    let row = system(&Registry::new());
    let doc = bson::to_document(&row).unwrap();
    assert!(doc.get_datetime("updated_at").is_ok());
    let decoded: AssistantSystemSkills = bson::from_document(doc).unwrap();
    assert_eq!(decoded.skills[0].reference, row.skills[0].reference);
    let mut agent = agent();
    assert!(agent.system_skill_overrides.is_empty());
    agent.system_skill_overrides.insert(ID.into(), false);
    assert_eq!(
        bson::from_document::<AssistantAgent>(bson::to_document(&agent).unwrap())
            .unwrap()
            .system_skill_overrides,
        BTreeMap::from([(ID.into(), false)])
    );
}

#[test]
fn effective_order_opt_out_and_pin_precedence() {
    let mut row = resolved(system(&Registry::new()));
    let mut second = row.system.skills[0].clone();
    second.reference.skill_id = DEP.into();
    second.reference.name = "other".into();
    row.system.skills.push(second);
    let mut agent = agent();
    assert_eq!(
        effective(&agent, &row)
            .iter()
            .map(|s| s.reference.skill_id.as_str())
            .collect::<Vec<_>>(),
        vec![ID, DEP]
    );
    agent.system_skill_overrides.insert(ID.into(), false);
    assert_eq!(effective(&agent, &row).len(), 1);
    agent.system_skill_overrides.clear();
    for disabled in [false, true] {
        for by_name in [false, true] {
            let mut pin = row.system.skills[0].reference.clone();
            pin.disabled = disabled;
            if by_name {
                pin.skill_id = uuid::Uuid::new_v4().to_string();
            } else {
                pin.name = "renamed".into();
            }
            agent.skills = vec![pin];
            assert_eq!(effective(&agent, &row)[0].reference.skill_id, DEP);
            assert_eq!(effective(&agent, &row).len(), 1);
        }
    }
    agent.destroyed_at = Some(Utc::now());
    assert!(effective(&agent, &row).is_empty());
    assert!(resolve(&agent, Some(&row), ID).is_none());
}

#[test]
fn request_body_refuses_client_hashes_and_unknown_fields() {
    for enabled in [Value::Null, json!(true), json!(false)] {
        assert!(
            serde_json::from_value::<crate::handlers::assistant_system_skills::Toggle>(
                json!({"enabled":enabled})
            )
            .is_ok()
        );
    }
    for invalid in [
        json!({}),
        json!({"enabled":"inherit"}),
        json!({"enabled":null,"other":true}),
    ] {
        assert!(
            serde_json::from_value::<crate::handlers::assistant_system_skills::Toggle>(invalid)
                .is_err()
        );
    }

    for value in [
        json!({"expected_revision":0,"skills":[{"skill":"x","version":"1.0","sha256":"untrusted"}]}),
        json!({"expected_revision":0,"skills":[],"unknown":true}),
    ] {
        assert!(serde_json::from_value::<Selection>(value).is_err());
    }
}

#[test]
fn guidance_has_stable_metadata_without_a_pin_fingerprint() {
    let mut system = resolved(system(&Registry::new()));
    let mut agent = agent();
    let original = instructions(&agent, &system);
    system.system.skills[0].reference.sha256 = "f".repeat(64);
    let changed_root = instructions(&agent, &system);
    assert_eq!(original, changed_root);
    system.system.skills[0].reference.dependencies[0].sha256 = "e".repeat(64);
    assert_eq!(changed_root, instructions(&agent, &system));
    assert!(!original.contains("fingerprint"));
    assert!(!original.contains(&system.system.skills[0].reference.sha256));
    system.system.skills[0].reference.version = "2.0".into();
    assert_ne!(original, instructions(&agent, &system));
    agent.system_skill_overrides.insert(ID.into(), false);
    assert!(instructions(&agent, &system).is_empty());
    system.system.skills[0].reference.sha256 = "d".repeat(64);
    assert!(instructions(&agent, &system).is_empty());
}

#[tokio::test]
async fn public_metadata_requires_exact_identity_and_version() {
    struct Metadata(Value);
    #[async_trait::async_trait]
    impl OrnnReader for Metadata {
        async fn get(&self, _path: &str) -> AppResult<Vec<u8>> {
            Ok(serde_json::to_vec(&json!({"data":self.0})).unwrap())
        }
    }
    let metadata = json!({"guid":ID,"version":"1.0","isPrivate":false});
    public(&Metadata(metadata.clone()), ID, "1.0")
        .await
        .unwrap();
    for (field, value) in [
        ("guid", json!(DEP)),
        ("version", json!("2.0")),
        ("isPrivate", json!(true)),
        ("isPrivate", json!(0)),
    ] {
        let mut changed = metadata.clone();
        changed[field] = value;
        assert!(matches!(
            public(&Metadata(changed), ID, "1.0").await,
            Err(AppError::ValidationError(_))
        ));
    }
}

#[tokio::test]
async fn exact_name_search_is_bounded_and_refuses_ambiguity() {
    use std::sync::atomic::{AtomicU32, Ordering};
    struct Search {
        calls: AtomicU32,
        pages: u32,
        names: Vec<&'static str>,
    }
    #[async_trait::async_trait]
    impl OrnnReader for Search {
        async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
            let page = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            assert!(path.contains("pageSize=20"));
            assert!(path.contains(&format!("page={page}")));
            Ok(serde_json::to_vec(&json!({"data":{
                "items":self.names.iter().enumerate().map(|(index, name)| json!({"guid":format!("{page}-{index}"),"name":name})).collect::<Vec<_>>(),
                "totalPages":self.pages
            }})).unwrap())
        }
    }
    for (pages, names, calls, valid) in [
        (1000, vec![], 5, false),
        (1000, vec!["nyxbot"], 5, false),
        (1, vec![], 1, false),
        (1, vec!["nyxbot", "nyxbot"], 1, false),
        (1, vec!["nyxbot"], 1, true),
    ] {
        let search = Search {
            calls: AtomicU32::new(0),
            pages,
            names,
        };
        let result = resolve_id(&search, "nyxbot").await;
        assert_eq!(result.is_ok(), valid);
        if !valid {
            assert!(
                matches!(result, Err(AppError::ValidationError(message)) if message.contains("use a skill GUID"))
            );
        }
        assert_eq!(search.calls.load(Ordering::SeqCst), calls);
        assert_eq!(resolve_id(&search, ID).await.unwrap(), ID);
        assert_eq!(search.calls.load(Ordering::SeqCst), calls);
    }
}

#[test]
fn status_source_serializes_to_frontend_enum_values() {
    for (source, wire) in [
        (SystemSkillSource::Agent, "agent"),
        (SystemSkillSource::NyxbotDefault, "nyxbot_default"),
        (SystemSkillSource::NyxbotOverride, "nyxbot_override"),
        (SystemSkillSource::Platform, "platform"),
        (SystemSkillSource::Pin, "pin"),
    ] {
        assert_eq!(serde_json::to_value(source).unwrap(), wire);
    }
}

#[tokio::test]
async fn admin_selection_integrity_public_dependencies_and_revision_fence() {
    let f = orchestrator_fixture("system_skills_admin").await;
    let db = &f.state.db;
    let registry = Registry::new();
    let actor = actor(&f.owner);
    assert!(get(db).await.unwrap().is_none());
    assert!(
        set(db, &registry, &actor, &selection(0, &[(ID, "1.0"); 9]))
            .await
            .is_err()
    );
    assert!(
        set(
            db,
            &registry,
            &actor,
            &selection(0, &[(ID, "1.0"), ("nyxbot", "1.0")])
        )
        .await
        .is_err()
    );
    assert!(
        set(db, &registry, &actor, &selection(0, &[(ID, "2.0")]))
            .await
            .is_err()
    );
    for invalid_private in [
        None,
        Some(Value::Null),
        Some(json!(true)),
        Some(json!("false")),
    ] {
        for dependency in [false, true] {
            let mut registry = Registry::new();
            if dependency {
                registry.dependency_private = invalid_private.clone();
            } else {
                registry.private = invalid_private.clone();
            }
            assert!(
                matches!(set(db,&registry,&actor,&selection(0,&[(ID,"1.0")])).await,Err(AppError::ValidationError(message)) if message.contains("confirmed public"))
            );
        }
    }
    let metadata: Value = serde_json::from_slice(
        &registry
            .value(&format!("/api/v1/skills/{ID}?version=1.0"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["data"]["isPrivate"], false);
    assert!(metadata["data"].get("visibility").is_none());
    let row = set(db, &registry, &actor, &selection(0, &[("nyxbot", "1.0")]))
        .await
        .unwrap();
    assert_eq!(row.revision, 1);
    assert_eq!(row.skills[0].reference, registry.reference());
    assert_eq!(row.skills[0].description, "Useful guidance");
    assert!(matches!(
        set(db, &registry, &actor, &selection(0, &[])).await,
        Err(AppError::Conflict(_))
    ));
    let clear = selection(1, &[]);
    let (a, b) = tokio::join!(
        set(db, &registry, &actor, &clear),
        set(db, &registry, &actor, &clear)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(get(db).await.unwrap().unwrap().skills.is_empty());
    let audit: crate::models::audit_log::AuditLog = db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"assistant_system_skills_updated","event_data.revision":1})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        audit.event_data.unwrap(),
        json!({"revision":1,"count":1,"skill_ids":[ID]})
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn reads_dependencies_shadowing_hash_and_destroyed_agents() {
    let registry = Registry::new();
    let system = resolved(system(&registry));
    for kind in [AgentKind::Nyxbot, AgentKind::Specialist] {
        let mut agent = agent();
        agent.kind = kind;
        assert!(agent.grants.service_ids.is_empty());
        for dependency in [None, Some("dependency")] {
            let value = read_resolved(
                &registry,
                &agent,
                Some(&system),
                "nyxbot",
                dependency,
                "SKILL.md",
                0,
            )
            .await
            .unwrap();
            assert_eq!(
                value["skill_id"],
                if dependency.is_some() { DEP } else { ID }
            );
            assert_eq!(value["content"], "Package content, untrusted guidance");
        }
        assert!(
            read_resolved(&registry, &agent, None, ID, None, "SKILL.md", 0)
                .await
                .is_err()
        );
        agent.system_skill_overrides.insert(ID.into(), false);
        assert!(
            read_resolved(&registry, &agent, Some(&system), ID, None, "SKILL.md", 0)
                .await
                .is_err()
        );
        agent.system_skill_overrides.clear();
        let mut pin = registry.reference();
        pin.disabled = true;
        agent.skills.push(pin);
        assert!(
            read_resolved(&registry, &agent, Some(&system), ID, None, "SKILL.md", 0)
                .await
                .is_err()
        );
        agent.skills.clear();
        let mut bad = system.clone();
        bad.system.skills[0].reference.sha256 = "f".repeat(64);
        assert!(matches!(
            read_resolved(&registry, &agent, Some(&bad), ID, None, "SKILL.md", 0).await,
            Err(AppError::SkillRead(
                crate::errors::skill_read::SkillReadReason::PinnedVersionMismatch
            ))
        ));
        agent.destroyed_at = Some(Utc::now());
        assert!(
            read_resolved(&registry, &agent, Some(&system), ID, None, "SKILL.md", 0)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn guidance_without_an_agent_is_empty_for_configured_flag_on_users() {
    let f = orchestrator_fixture("system_skills_no_agent").await;
    flag(&f.state.db, &f.owner, true).await;
    f.state
        .db
        .collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    assert!(
        guidance(&f.state.db, &f.owner, false, None, None, true)
            .await
            .unwrap()
            .is_empty()
    );
    // No inheritance/configuration is needed when there is no agent.
    f.state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":DOCUMENT_ID},
            doc! {"$set":{"skills":"malformed"}},
        )
        .await
        .unwrap();
    assert!(
        guidance(&f.state.db, &f.owner, false, None, None, true)
            .await
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn flag_off_authoring_and_preloaded_guidance_preserve_unbound_legacy_sessions() {
    use super::super::assistant_instruction_context::Prepared;
    let f = super::super::assistant_authority_tests::direct_fixture("system_skills_legacy").await;
    let db = &f.state.db;
    let agent = super::super::assistant_team_service::agent(db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    db.collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    let mut legacy_agent = agent.clone();
    legacy_agent.updated_at = f.row.created_at;
    let mut legacy = f.row.clone();
    legacy.nyxagent_session_id = Some("legacy-session".into());
    for authoring in [
        super::super::assistant_skill_authoring::GUIDANCE,
        super::super::assistant_skill_authoring::SPECIALIST_GUIDANCE,
        "\nPreloaded skills: existing content\n",
    ] {
        let system = guidance(db, &f.owner, false, Some(&legacy_agent), None, false)
            .await
            .unwrap();
        assert!(system.is_empty(), "system-skills flag is off");
        let prepared = Prepared::with_guidance(
            b"test-key",
            &legacy,
            Some(&legacy_agent),
            &[],
            &format!("{authoring}{system}"),
            !system.is_empty(),
        )
        .unwrap();
        assert!(
            !prepared.reset,
            "existing guidance preserves legacy sessions"
        );
        assert!(prepared.instructions().contains(authoring));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn rollout_guidance_response_shortcut_and_reset_once() {
    use super::super::assistant_instruction_context::Prepared;
    let f = super::super::assistant_authority_tests::direct_fixture("system_skills_guidance").await;
    let db = &f.state.db;
    let agent = super::super::assistant_team_service::agent(db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    let registry = Registry::new();
    db.collection(COLLECTION_NAME)
        .insert_one(system(&registry))
        .await
        .unwrap();
    assert!(
        guidance(
            db,
            &f.owner,
            false,
            Some(&agent),
            None,
            enabled(db, &f.owner).await.unwrap()
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(
        serde_json::to_value(response(db, &f.owner, agent.clone()).await.unwrap())
            .unwrap()
            .get("system_skills")
            .is_none()
    );
    flag(db, &f.owner, true).await;
    for kind in [AgentKind::Nyxbot, AgentKind::Specialist] {
        let mut agent = agent.clone();
        agent.kind = kind;
        let text = guidance(
            db,
            &f.owner,
            false,
            Some(&agent),
            None,
            enabled(db, &f.owner).await.unwrap(),
        )
        .await
        .unwrap();
        assert!(
            guidance(db, &f.owner, false, None, None, true)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            text.contains("Built-in skills")
                && text.contains(ID)
                && text.contains("untrusted guidance")
        );
        assert!(
            guidance(db, &f.owner, true, Some(&agent), None, true)
                .await
                .unwrap()
                .is_empty()
        );
        let mut row = f.row.clone();
        row.nyxagent_session_id = Some("session".into());
        agent.updated_at = row.created_at;
        let mut baseline =
            Prepared::with_guidance(b"test-key", &row, Some(&agent), &[], &text, true).unwrap();
        assert!(
            baseline.reset,
            "unbound legacy guidance must be installed once"
        );
        row.nyxagent_session_id = Some("guided-session".into());
        baseline.binding.session_id = "guided-session".into();
        row.nyxagent_instruction_binding = Some(baseline.binding.clone());
        assert!(
            !Prepared::with_guidance(b"test-key", &row, Some(&agent), &[], &text, true)
                .unwrap()
                .reset
        );
        agent.system_skill_overrides.insert(ID.into(), false);
        let changed = guidance(
            db,
            &f.owner,
            false,
            Some(&agent),
            None,
            enabled(db, &f.owner).await.unwrap(),
        )
        .await
        .unwrap();
        let mut reset = Prepared::with_guidance(
            b"test-key",
            &row,
            Some(&agent),
            &[],
            &changed,
            !changed.is_empty(),
        )
        .unwrap();
        assert!(reset.reset);
        row.nyxagent_session_id = Some("opt-out-session".into());
        reset.binding.session_id = "opt-out-session".into();
        row.nyxagent_instruction_binding = Some(reset.binding);
        assert!(
            !Prepared::with_guidance(
                b"test-key",
                &row,
                Some(&agent),
                &[],
                &changed,
                !changed.is_empty()
            )
            .unwrap()
            .reset
        );
    }
    let statuses = response(db, &f.owner, agent.clone())
        .await
        .unwrap()
        .system_skills;
    assert_eq!(statuses.len(), 1);
    assert!(statuses[0].enabled);
    assert!(!statuses[0].shadowed_by_pin);
    let mut shadowed_agent = agent.clone();
    shadowed_agent.skills.push(registry.reference());
    shadowed_agent.skills[0].disabled = true;
    let shadowed = response(db, &f.owner, shadowed_agent).await.unwrap();
    assert!(shadowed.system_skills[0].enabled);
    assert!(shadowed.system_skills[0].shadowed_by_pin);
    for args in [
        json!({"agent":agent.id,"skill":"nyxbot"}),
        json!({"agent":agent.id,"skill":ID,"version":"1.0"}),
        json!({"agent":agent.id,"selection":{"expected_revision":0,"skills":[registry.reference()]}}),
    ] {
        let (value, error) = crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "request_agent_skills",
            &args,
        )
        .await
        .unwrap();
        assert!(!error);
        assert_eq!(value["request_created"], false);
        assert!(value["instructions"].as_str().unwrap().contains("built-in"));
    }
    let (value, error) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "get_agent_skills",
        &json!({"agent":agent.id}),
    )
    .await
    .unwrap();
    assert!(!error);
    assert_eq!(value["system_skills"][0]["skill_id"], ID);
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(doc! {"_id":DOCUMENT_ID}, doc! {"$set":{"skills":[]}})
        .await
        .unwrap();
    assert!(
        guidance(
            db,
            &f.owner,
            false,
            Some(&agent),
            None,
            enabled(db, &f.owner).await.unwrap()
        )
        .await
        .unwrap()
        .is_empty()
    );
    let mut guest = f.chat.clone();
    guest.guest = true;
    assert!(
        crate::handlers::agent_skills::dispatch(
            &f.state,
            &guest,
            "skill_read",
            &json!({"skill":ID})
        )
        .await
        .is_err()
    );
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":DOCUMENT_ID},
            doc! {"$set":{"skills":"malformed"}},
        )
        .await
        .unwrap();
    assert!(
        guidance(
            db,
            &f.owner,
            false,
            Some(&agent),
            None,
            enabled(db, &f.owner).await.unwrap()
        )
        .await
        .is_err()
    );
    assert!(
        guidance(db, &f.owner, true, Some(&agent), None, true)
            .await
            .unwrap()
            .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn owner_toggle_flag_idempotence_unknown_destroyed_audit_and_history() {
    let f = orchestrator_fixture("system_skills_toggle").await;
    let db = &f.state.db;
    db.collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    let actor = actor(&f.owner);
    let id = &f.chat.agent_id;
    assert!(matches!(
        toggle(db, &actor, id, ID, Some(false)).await,
        Err(AppError::Forbidden(_))
    ));
    flag(db, &f.owner, true).await;
    for _ in 0..2 {
        toggle(db, &actor, id, ID, Some(false)).await.unwrap();
    }
    let agent = super::super::assistant_team_service::agent(db, &f.owner, id)
        .await
        .unwrap();
    assert_eq!(
        agent.system_skill_overrides,
        BTreeMap::from([(ID.into(), false)])
    );
    let status = &response(db, &f.owner, agent.clone())
        .await
        .unwrap()
        .system_skills[0];
    assert!(!status.enabled);
    let opt_outs: BTreeMap<_, _> = std::iter::once(ID.to_owned())
        .chain((0..31).map(|_| uuid::Uuid::new_v4().to_string()))
        .map(|id| (id, false))
        .collect();
    db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"system_skill_overrides":bson::to_bson(&opt_outs).unwrap()}},
        )
        .await
        .unwrap();
    toggle(db, &actor, id, ID, Some(false)).await.unwrap();
    let mut additional = system(&Registry::new());
    additional.skills[0].reference.skill_id = DEP.into();
    db.collection::<AssistantSystemSkills>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":DOCUMENT_ID},
            doc! {"$set":{"skills":bson::to_bson(&additional.skills).unwrap()}},
        )
        .await
        .unwrap();
    assert!(matches!(
        toggle(db, &actor, id, DEP, Some(false)).await,
        Err(AppError::Conflict(_))
    ));
    db.collection::<AssistantSystemSkills>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":DOCUMENT_ID},
            doc! {"$set":{"skills":bson::to_bson(&system(&Registry::new()).skills).unwrap()}},
        )
        .await
        .unwrap();
    assert!(matches!(
        toggle(db, &actor, id, DEP, Some(false)).await,
        Err(AppError::ValidationError(_))
    ));
    toggle(db, &actor, id, DEP, None).await.unwrap();
    let history = skills::history(db, &agent).await.unwrap();
    assert!(
        history
            .items
            .iter()
            .any(|row| row.event_type == "assistant_system_skill_toggled"
                && row.skill_id.as_deref() == Some(ID)
                && row.enabled == Some(Some(false)))
    );
    db.collection::<bson::Document>(COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    toggle(db, &actor, id, ID, None).await.unwrap();
    db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert!(toggle(db, &actor, id, ID, Some(true)).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn toggle_org_maintainer_and_nonmember_acl() {
    let f = super::super::org_agent_tests::Fixture::new("system_skills_org_toggle").await;
    let (agent, _) = f.create().await;
    f.state
        .db
        .collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    for person in [&f.admin, &f.member, &f.viewer, &f.outsider] {
        flag(&f.state.db, person, true).await;
    }
    toggle(&f.state.db, &actor(&f.admin), &agent.id, ID, Some(false))
        .await
        .unwrap();
    toggle(&f.state.db, &actor(&f.member), &agent.id, ID, Some(true))
        .await
        .unwrap();
    assert!(matches!(
        toggle(&f.state.db, &actor(&f.viewer), &agent.id, ID, Some(false)).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        toggle(&f.state.db, &actor(&f.outsider), &agent.id, ID, Some(false)).await,
        Err(AppError::NotFound(_))
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_and_toggle_handlers_reject_non_humans_and_non_admins() {
    use crate::{
        handlers::assistant_system_skills as handler, mw::auth::AuthMethod,
        test_utils::test_auth_user,
    };
    use axum::{
        Json,
        extract::{Path, State},
        response::IntoResponse,
    };
    let f = orchestrator_fixture("system_skills_handler_auth").await;
    super::super::role_service::seed_system_roles(&f.state.db)
        .await
        .unwrap();
    let auth = test_auth_user(&f.owner);
    let error = handler::get(State(f.state.clone()), auth.clone())
        .await
        .unwrap_err();
    assert_eq!(error.into_response().status(), 403);
    let error = handler::set(
        State(f.state.clone()),
        auth.clone(),
        Json(selection(0, &[])),
    )
    .await
    .unwrap_err();
    assert_eq!(error.into_response().status(), 403);
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
        AuthMethod::Relay,
    ] {
        let mut auth = auth.clone();
        auth.auth_method = method;
        assert!(
            handler::set(
                State(f.state.clone()),
                auth.clone(),
                Json(selection(0, &[]))
            )
            .await
            .is_err()
        );
        let body = serde_json::from_value(json!({"enabled":true})).unwrap();
        assert!(
            handler::toggle(
                State(f.state.clone()),
                auth,
                Path((f.chat.agent_id.clone(), ID.into())),
                Json(body)
            )
            .await
            .is_err()
        );
    }
    let mut oauth = auth.clone();
    oauth.oauth_client_id = Some("client".into());
    assert!(
        handler::set(State(f.state.clone()), oauth, Json(selection(0, &[])))
            .await
            .is_err()
    );
    let role_ids = super::super::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$addToSet":{"role_ids":role_ids.admin}},
        )
        .await
        .unwrap();
    let _ = handler::set(State(f.state.clone()), auth, Json(selection(0, &[])))
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn built_in_outline_reads_keep_the_pinned_dependency_closure() {
    let f = fixture("system_skills_outline").await;
    flag(&f.state.db, &f.owner, true).await;
    flags::set_platform_override(
        &f.state.db,
        flags::SKILL_RUNS_FLAG_KEY,
        &flags::FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let registry = Registry::with_markdown(
        "---\nname: nyxbot\nmetadata:\n  depends-on:\n    - dependency@1.0\n---\n## Overview\nOne result.\n\n## Approach\n1. **Prepare.** I get ready.\n   Uses: dependency\n",
    );
    f.state
        .db
        .collection(COLLECTION_NAME)
        .insert_one(system(&registry))
        .await
        .unwrap();
    let upstream = wiremock::MockServer::start().await;
    registry.mount(&upstream).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(catalog)
        .await
        .unwrap();
    for dependency in [Value::Null, json!("dependency")] {
        let (page, error) = crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "skill_read",
            &json!({"skill":ID,"dependency":dependency,"section":"outline"}),
        )
        .await
        .unwrap();
        assert!(!error);
        assert_eq!(page["path"], "outline");
        assert_eq!(page["untrusted_guidance"], true);
        assert!(
            page["content"]
                .as_str()
                .unwrap()
                .contains(&format!("Uses: dependency (skill {DEP}, version 1.0)"),)
        );
    }
    let before = upstream.received_requests().await.unwrap().len();
    assert_eq!(before, 2);
    assert!(
        crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "skill_read",
            &json!({"skill":ID,"dependency":"not-pinned","section":"outline"}),
        )
        .await
        .is_err()
    );
    flag(&f.state.db, &f.owner, false).await;
    assert!(
        crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "skill_read",
            &json!({"skill":ID,"section":"outline"}),
        )
        .await
        .is_err()
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);
    let agent =
        super::super::assistant_team_service::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
    assert!(
        agent.skills.is_empty(),
        "built-ins are never persisted as attached pins"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_and_specialist_read_same_proxy_key_without_ornn_grant_and_preview() {
    use crate::handlers::agent_skills;
    use axum::{
        Json,
        extract::{Path, State},
    };
    for specialist in [false, true] {
        let mut f = if specialist {
            Box::pin(fixture("system_skills_specialist_proxy")).await
        } else {
            Box::pin(orchestrator_fixture("system_skills_nyxbot_proxy")).await
        };
        f.state.config.resource_request_proof_enabled = true;
        flag(&f.state.db, &f.owner, true).await;
        let registry = Registry::new();
        f.state
            .db
            .collection(COLLECTION_NAME)
            .insert_one(system(&registry))
            .await
            .unwrap();
        let upstream = wiremock::MockServer::start().await;
        registry.mount(&upstream).await;
        let mut service = crate::test_utils::test_auto_connected_catalog_service();
        service.slug = "ornn-api".into();
        service.base_url = upstream.uri();
        service.identity_propagation_mode = "jwt".into();
        service.introspection_client_ids = Some(vec![uuid::Uuid::new_v4().to_string()]);
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .insert_one(&service)
            .await
            .unwrap();
        let key: crate::models::api_key::ApiKey = f
            .state
            .db
            .collection(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.chat.api_key_id})
            .await
            .unwrap()
            .unwrap();
        if specialist {
            assert!(!key.allow_all_services);
            assert!(!key.allowed_service_ids.contains(&service.id));
        }
        for dep in [Value::Null, json!("dependency")] {
            let (result, error) = agent_skills::dispatch(
                &f.state,
                &f.chat,
                "skill_read",
                &json!({"skill":ID,"dependency":dep}),
            )
            .await
            .unwrap();
            assert!(!error);
            assert_eq!(result["content"], "Package content, untrusted guidance");
        }
        // A connected person's Ornn path must work for the same restricted
        // specialist key, just as the legacy no-credential catalog path does.
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let endpoint = crate::test_utils::test_user_endpoint(
            &endpoint_id,
            &f.owner,
            "Ornn",
            &upstream.uri(),
            None,
            Some(&service.id),
        );
        let mut connection = crate::test_utils::test_user_service(
            &connection_id,
            &f.owner,
            "ornn-api",
            &endpoint_id,
            Some(&service.id),
            None,
        );
        connection.identity_propagation_mode = "jwt".into();
        f.state
            .db
            .collection(crate::models::user_endpoint::COLLECTION_NAME)
            .insert_one(endpoint)
            .await
            .unwrap();
        f.state
            .db
            .collection::<crate::models::user_service::UserService>(
                crate::models::user_service::COLLECTION_NAME,
            )
            .insert_one(&connection)
            .await
            .unwrap();
        if specialist {
            let reader = agent_skills::Reader {
                state: &f.state,
                person: &f.owner,
                thread_key: Some(&f.chat.api_key_id),
                scopes: None,
                chat: Some(std::sync::Arc::new(f.chat.clone())),
            };
            assert!(matches!(
                reader
                    .get(&format!("/api/v1/skills/{ID}/versions/1.0/download"))
                    .await,
                Err(AppError::ApiKeyScopeForbidden(_))
            ));
        }
        // Connected built-ins exercise proof issuance independently of own pins.
        for dependency in [Value::Null, json!(DEP)] {
            let (value, error) = agent_skills::dispatch(
                &f.state,
                &f.chat,
                "skill_read",
                &json!({"skill":ID,"dependency":dependency}),
            )
            .await
            .expect("a connected built-in read retains the signed identity");
            assert!(!error);
            assert_eq!(value["content"], "Package content, untrusted guidance");
        }
        f.state
            .db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"skills":[bson::to_bson(&registry.reference()).unwrap()]}},
            )
            .await
            .unwrap();
        // Pin precedence still uses exactly the same adapter and thread key.
        for dependency in [Value::Null, json!(DEP)] {
            let (value, error) = agent_skills::dispatch(
                &f.state,
                &f.chat,
                "skill_read",
                &json!({"skill":ID,"dependency":dependency}),
            )
            .await
            .expect("an attached pin must be readable without an Ornn service grant");
            assert!(!error);
            assert_eq!(value["content"], "Package content, untrusted guidance");
        }
        let requests = upstream.received_requests().await.unwrap();
        for request in requests.iter().skip(2) {
            assert!(request.headers.contains_key("x-nyxid-identity-token"));
            assert_eq!(
                request
                    .headers
                    .contains_key(super::super::resource_request_proof::HEADER),
                !specialist,
                "proof issuance remains personal NyxBot-only",
            );
        }
        flag(&f.state.db, &f.owner, false).await;
        f.state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":DOCUMENT_ID},
                doc! {"$set":{"skills":"malformed"}},
            )
            .await
            .unwrap();
        assert!(
            agent_skills::dispatch(&f.state, &f.chat, "skill_read", &json!({"skill":ID}))
                .await
                .is_ok()
        );
        let Json(preview) = agent_skills::preview_attached(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            Path((f.chat.agent_id.clone(), ID.into())),
        )
        .await
        .expect("attached preview ignores built-in configuration");
        assert!(preview.readable);
        f.state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":DOCUMENT_ID},
                doc! {"$set":{"skills":bson::to_bson(&system(&registry).skills).unwrap()}},
            )
            .await
            .unwrap();
        flag(&f.state.db, &f.owner, true).await;
        f.state
            .db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(doc! {"_id":&f.chat.agent_id}, doc! {"$set":{"skills":[]}})
            .await
            .unwrap();
        for dependency in [Value::Null, json!(DEP)] {
            let (value, error) = agent_skills::dispatch(
                &f.state,
                &f.chat,
                "skill_read",
                &json!({"skill":ID,"dependency":dependency}),
            )
            .await
            .unwrap();
            assert!(!error);
            assert_eq!(value["content"], "Package content, untrusted guidance");
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if f.state
                    .db
                    .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                    .find_one(
                        doc! {"event_type":"service_request","user_id":&f.owner,"api_key_id":&f.chat.api_key_id,"event_data.user_service_id":&connection_id,"event_data.response_status":200},
                    )
                    .await
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("normal proxy receipt audit");

        let before = upstream.received_requests().await.unwrap().len();
        let scopes = BTreeMap::from([(
            connection_id.clone(),
            crate::models::agent_operation_scope::AgentOperationScope {
                revision: 1,
                catalog_service_id: Some(service.id.clone()),
                operations: vec![],
            },
        )]);
        f.state
            .db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"operation_scopes":bson::to_bson(&scopes).unwrap()}},
            )
            .await
            .unwrap();
        assert!(
            agent_skills::dispatch(&f.state, &f.chat, "skill_read", &json!({"skill":ID}))
                .await
                .is_err()
        );
        assert_eq!(upstream.received_requests().await.unwrap().len(), before);
        f.state
            .db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"operation_scopes":{}}},
            )
            .await
            .unwrap();
        let mut guest = f.chat.clone();
        guest.guest = true;
        assert!(
            agent_skills::dispatch(&f.state, &guest, "skill_read", &json!({"skill":ID}))
                .await
                .is_err()
        );
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.row.id},
                doc! {"$set":{"active_turn.stop_requested":true}},
            )
            .await
            .unwrap();
        assert!(
            agent_skills::dispatch(&f.state, &f.chat, "skill_read", &json!({"skill":ID}))
                .await
                .is_err()
        );
        assert_eq!(upstream.received_requests().await.unwrap().len(), before);
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.row.id},
                doc! {"$set":{"active_turn.stop_requested":false}},
            )
            .await
            .unwrap();
        // The native reader uses main's fixed scope for its exact GETs.
        // Revoking the live key must still stop this lane before egress.
        f.state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.api_key_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert!(
            agent_skills::dispatch(&f.state, &f.chat, "skill_read", &json!({"skill":ID}))
                .await
                .is_err()
        );
        assert_eq!(upstream.received_requests().await.unwrap().len(), before);
        f.state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.api_key_id},
                doc! {"$set":{"is_active":true}},
            )
            .await
            .unwrap();
        let Json(preview) = agent_skills::preview_attached(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            Path((f.chat.agent_id.clone(), ID.into())),
        )
        .await
        .unwrap();
        assert!(preview.readable);
        assert!(preview.excerpt.contains("Package content"));
        let requests = upstream.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|r| r.headers.contains_key("x-nyxid-identity-token"))
        );
        flag(&f.state.db, &f.owner, false).await;
        assert!(
            agent_skills::dispatch(&f.state, &f.chat, "skill_read", &json!({"skill":ID}))
                .await
                .is_err()
        );
        flag(&f.state.db, &f.owner, true).await;
        // The generic service approval is still reached through the exact-pin
        // path, and no package bytes flow before the owner decides.
        super::super::assistant_settings_service::update(
            &f.state.db,
            &f.owner,
            super::super::assistant_settings_service::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        f.state
            .db
            .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
                crate::models::service_approval_config::COLLECTION_NAME,
            )
            .insert_one(
                crate::models::service_approval_config::ServiceApprovalConfig {
                    id: uuid::Uuid::new_v4().to_string(),
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
        let before = upstream.received_requests().await.unwrap().len();
        let state = f.state.clone();
        let chat = f.chat.clone();
        let pending = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            agent_skills::dispatch(&state, &chat, "skill_read", &json!({"skill":ID})).await
        }));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if f.state
                    .db
                    .collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME)
                    .find_one(doc! {"user_id":&f.owner,"service_id":&service.id,"status":"pending"})
                    .await
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                assert!(!pending.is_finished(), "pin read ended before approval");
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("exact pin requires the ordinary service approval");
        assert_eq!(upstream.received_requests().await.unwrap().len(), before);
        drop(pending);
        f.state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.api_key_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert!(
            agent_skills::dispatch(&f.state, &f.chat, "skill_read", &json!({"skill":ID}))
                .await
                .is_err()
        );
        assert_eq!(upstream.received_requests().await.unwrap().len(), before);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_specialist_pinned_reads_use_member_identity_and_recheck_membership() {
    use super::super::{
        assistant_acknowledgement_service as acks, assistant_nyxagent as engine,
        assistant_team_service as team,
    };
    use crate::handlers::agent_skills;
    let mut f = super::super::org_agent_tests::Fixture::new("system_skills_org_proxy").await;
    f.state.config.resource_request_proof_enabled = true;
    let (agent, _) = f.create().await;
    flag(&f.state.db, &f.member, true).await;
    let registry = Registry::new();
    f.state
        .db
        .collection(COLLECTION_NAME)
        .insert_one(system(&registry))
        .await
        .unwrap();
    let upstream = wiremock::MockServer::start().await;
    registry.mount(&upstream).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    catalog.introspection_client_ids = Some(vec![uuid::Uuid::new_v4().to_string()]);
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let endpoint_id = uuid::Uuid::new_v4().to_string();
    let connection_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint_id,
            &f.org,
            "Ornn",
            &upstream.uri(),
            None,
            Some(&catalog.id),
        ))
        .await
        .unwrap();
    let mut connection = crate::test_utils::test_user_service(
        &connection_id,
        &f.org,
        "ornn-api",
        &endpoint_id,
        Some(&catalog.id),
        None,
    );
    connection.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(connection)
        .await
        .unwrap();
    let channel = super::super::notification_service::get_or_create_channel(&f.state.db, &f.member)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::notification_channel::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&channel.id},
            doc! {"$set":{"approval_required":false}},
        )
        .await
        .unwrap();
    let home = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let row = engine::begin_turn(
        &f.state.db,
        &f.member,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            conversation_id: Some(home.id),
            text: "Read pinned guidance".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&f.state.db, &f.member, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let key = f.key(&row).await;
    assert!(!key.allow_all_services);
    assert!(!key.allowed_service_ids.contains(&connection_id));
    assert_eq!(
        key.assistant_agent_owner_id.as_deref(),
        Some(f.org.as_str())
    );
    for attached in [false, true] {
        f.state.db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(doc! {"_id":&agent.id}, doc! {"$set":{"skills":if attached {vec![bson::to_bson(&registry.reference()).unwrap()]} else {vec![]}}}).await.unwrap();
        for dependency in [Value::Null, json!(DEP)] {
            let (value, error) = agent_skills::dispatch(
                &f.state,
                &chat,
                "skill_read",
                &json!({"skill":ID,"dependency":dependency}),
            )
            .await
            .unwrap();
            assert!(!error);
            assert_eq!(value["content"], "Package content, untrusted guidance");
        }
    }
    for request in upstream.received_requests().await.unwrap() {
        assert!(
            !request
                .headers
                .contains_key(super::super::resource_request_proof::HEADER)
        );
        let token = request.headers["x-nyxid-identity-token"].to_str().unwrap();
        let payload = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            token.split('.').nth(1).unwrap(),
        )
        .unwrap();
        let claims: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(claims["sub"], f.member);
    }
    let before = upstream.received_requests().await.unwrap().len();
    f.revoke(&f.member).await;
    assert!(
        agent_skills::dispatch(&f.state, &chat, "skill_read", &json!({"skill":ID}))
            .await
            .is_err()
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);
    f.state.db.drop().await.unwrap();
}

#[test]
fn inheritance_truth_table_and_status_sources() {
    for kind in [AgentKind::Nyxbot, AgentKind::Specialist] {
        for mode in [
            SystemSkillsForSpecialists::Default,
            SystemSkillsForSpecialists::Override,
        ] {
            for nyxbot in [None, Some(false), Some(true)] {
                for own in [None, Some(false), Some(true)] {
                    let mut agent = agent();
                    agent.kind = kind;
                    agent.system_skill_overrides =
                        own.map(|value| (ID.into(), value)).into_iter().collect();
                    let context = Context {
                        system: system(&Registry::new()),
                        mode,
                        nyxbot_overrides: nyxbot
                            .map(|value| (ID.into(), value))
                            .into_iter()
                            .collect(),
                    };
                    let (expected, source) = match kind {
                        AgentKind::Nyxbot => (
                            own.unwrap_or(true),
                            if own.is_some() {
                                SystemSkillSource::Agent
                            } else {
                                SystemSkillSource::Platform
                            },
                        ),
                        AgentKind::Specialist if mode == SystemSkillsForSpecialists::Override => {
                            (nyxbot.unwrap_or(true), SystemSkillSource::NyxbotOverride)
                        }
                        AgentKind::Specialist => (
                            own.or(nyxbot).unwrap_or(true),
                            if own.is_some() {
                                SystemSkillSource::Agent
                            } else {
                                SystemSkillSource::NyxbotDefault
                            },
                        ),
                    };
                    assert_eq!(
                        !effective(&agent, &context).is_empty(),
                        expected,
                        "{kind:?} {mode:?} {nyxbot:?} {own:?}"
                    );
                    let status = &statuses(&agent, &context)[0];
                    assert_eq!(status.r#override, own);
                    assert_eq!(status.enabled, expected);
                    assert_eq!(status.effective, expected);
                    assert_eq!(status.source, source);
                    assert_eq!(resolve(&agent, Some(&context), ID).is_some(), expected);
                    assert_eq!(!instructions(&agent, &context).is_empty(), expected);
                    assert_eq!(
                        already_available(&agent, &context, &json!({"skill":ID})).is_some(),
                        expected
                    );
                    let before = agent.system_skill_overrides.clone();
                    agent
                        .skills
                        .push(context.system.skills[0].reference.clone());
                    let status = &statuses(&agent, &context)[0];
                    assert!(!status.effective);
                    assert_eq!(status.source, SystemSkillSource::Pin);
                    assert_eq!(agent.system_skill_overrides, before);
                }
            }
        }
    }
}

#[tokio::test]
async fn notification_preferences_and_system_skill_mode_survive_independent_settings_updates() {
    use crate::handlers::assistant_team;
    use axum::{Json, extract::State};
    let f = orchestrator_fixture("system_skills_notification_settings").await;
    flag(&f.state.db, &f.owner, true).await;
    let mut legacy = bson::to_document(&AssistantSettings::defaults(&f.owner)).unwrap();
    legacy.remove("notifications");
    legacy.remove("system_skills_for_specialists");
    let defaults: AssistantSettings = bson::from_document(legacy).unwrap();
    assert_eq!(
        defaults.system_skills_for_specialists,
        SystemSkillsForSpecialists::Default
    );
    assert_eq!(defaults.notifications, Default::default());
    for (patch, mode, automation_result, chat_update) in [
        (
            json!({"system_skills_for_specialists":"override","notifications":{"push":{"automation_result":true}}}),
            "override",
            true,
            false,
        ),
        (
            json!({"system_skills_for_specialists":"default"}),
            "default",
            true,
            false,
        ),
        (
            json!({"notifications":{"push":{"chat_update":true}}}),
            "default",
            false,
            true,
        ),
        (
            json!({"system_skills_for_specialists":"override"}),
            "override",
            false,
            true,
        ),
    ] {
        let Json(updated) = assistant_team::update_settings(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            Json(serde_json::from_value(patch).unwrap()),
        )
        .await
        .unwrap();
        let Json(read) = assistant_team::get_settings(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
        )
        .await
        .unwrap();
        for settings in [updated, read] {
            let settings = serde_json::to_value(settings).unwrap();
            assert_eq!(settings["system_skills_for_specialists"], mode);
            assert_eq!(
                settings["notifications"]["push"]["automation_result"],
                automation_result
            );
            assert_eq!(
                settings["notifications"]["push"]["chat_update"],
                chat_update
            );
        }
    }
    let saved = super::super::assistant_settings_service::get(&f.state.db, &f.owner)
        .await
        .unwrap();
    assert_eq!(
        saved.system_skills_for_specialists,
        SystemSkillsForSpecialists::Override
    );
    assert!(saved.notifications.push.chat_update);
    assert!(!saved.notifications.push.automation_result);
    assert!(f.state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"assistant_settings_updated","event_data.system_skills_for_specialists":"override","event_data.previous_system_skills_for_specialists":"default"})
        .await.unwrap().is_some());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn settings_roundtrip_audit_native_exclusion_and_specialist_conflict() {
    use crate::handlers::assistant_team;
    use axum::{Json, extract::State};
    let f = orchestrator_fixture("system_skills_settings").await;
    let db = &f.state.db;
    let legacy = bson::to_document(&AssistantSettings::defaults(&f.owner)).unwrap();
    let mut missing = legacy.clone();
    missing.remove("system_skills_for_specialists");
    let decoded: AssistantSettings = bson::from_document(missing).unwrap();
    assert_eq!(
        decoded.system_skills_for_specialists,
        SystemSkillsForSpecialists::Default
    );
    assert!(legacy.get_datetime("updated_at").is_ok());
    let result = assistant_team::update_settings(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Json(
            serde_json::from_value(
                json!({"system_skills_for_specialists":"override","max_auto_continuations":3}),
            )
            .unwrap(),
        ),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::Forbidden(message)) if message == "System skills are not enabled for this rollout")
    );
    let unchanged = super::super::assistant_settings_service::get(db, &f.owner)
        .await
        .unwrap();
    assert_eq!(
        unchanged.system_skills_for_specialists,
        SystemSkillsForSpecialists::Default
    );
    assert_eq!(
        unchanged.max_auto_continuations,
        decoded.max_auto_continuations
    );
    let Json(ordinary) = assistant_team::update_settings(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Json(serde_json::from_value(json!({"max_auto_continuations":3})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(ordinary).unwrap()["max_auto_continuations"],
        3
    );
    flag(db, &f.owner, true).await;
    for mode in ["override", "default", "override"] {
        let request =
            serde_json::from_value(json!({"system_skills_for_specialists":mode})).unwrap();
        let Json(result) = assistant_team::update_settings(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            Json(request),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap()["system_skills_for_specialists"],
            mode
        );
        let Json(read) = assistant_team::get_settings(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(read).unwrap()["system_skills_for_specialists"],
            mode
        );
    }
    let audit: crate::models::audit_log::AuditLog = db.collection(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"assistant_settings_updated","event_data.system_skills_for_specialists":"override","event_data.previous_system_skills_for_specialists":"default"}).await.unwrap().unwrap();
    assert_eq!(
        audit.event_data.unwrap()["system_skills_for_specialists"],
        "override"
    );
    // Native validation rejects the human-only mode before dispatch.
    let (_, refused) = assistant_team::execute_tool(
        &f.state,
        &f.chat,
        "nyxid__update_settings",
        &json!({"max_auto_continuations":4,"system_skills_for_specialists":"default"}),
    )
    .await;
    assert!(refused);
    let (_, refused) = assistant_team::execute_tool(
        &f.state,
        &f.chat,
        "nyxid__update_settings",
        &json!({"max_auto_continuations":4}),
    )
    .await;
    assert!(!refused);
    assert_eq!(
        super::super::assistant_settings_service::get(db, &f.owner)
            .await
            .unwrap()
            .system_skills_for_specialists,
        SystemSkillsForSpecialists::Override
    );
    flag(db, &f.owner, true).await;
    db.collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"kind":"specialist","system_skill_overrides":{ID:true}}},
        )
        .await
        .unwrap();
    for enabled in [None, Some(false), Some(true)] {
        assert!(
            matches!(toggle(db, &actor(&f.owner), &f.chat.agent_id, ID, enabled).await, Err(AppError::Conflict(message)) if message == "Built-in skills for specialists are managed by NyxBot settings")
        );
    }
    super::super::assistant_settings_service::update(
        db,
        &f.owner,
        super::super::assistant_settings_service::Update {
            system_skills_for_specialists: Some(SystemSkillsForSpecialists::Default),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let response = toggle(db, &actor(&f.owner), &f.chat.agent_id, ID, None)
        .await
        .unwrap();
    assert_eq!(response.system_skills[0].r#override, None);
    assert!(response.system_skills[0].effective);
    assert_eq!(
        response.system_skills[0].source,
        SystemSkillSource::NyxbotDefault
    );
    assert!(db.collection::<crate::models::audit_log::AuditLog>(crate::models::audit_log::COLLECTION_NAME).find_one(doc! {"event_type":"assistant_system_skill_toggled","event_data.enabled":bson::Bson::Null}).await.unwrap().is_some());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn inheritance_reads_are_gated_and_org_choices_follow_acting_member() {
    let f = super::super::org_agent_tests::Fixture::new("system_skills_actor_inheritance").await;
    let (agent, _) = f.create().await;
    let db = &f.state.db;
    // Malformed rows would fail deserialization if an unnecessary read occurred.
    db.collection::<bson::Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.admin},
            doc! {"$set":{"system_skills_for_specialists":"invalid"}},
        )
        .await
        .unwrap();
    assert!(for_person(db, &f.admin, false).await.unwrap().is_none());
    flag(db, &f.admin, true).await;
    assert!(for_person(db, &f.admin, false).await.unwrap().is_none());
    let mut empty = system(&Registry::new());
    empty.skills.clear();
    db.collection::<AssistantSystemSkills>(COLLECTION_NAME)
        .insert_one(&empty)
        .await
        .unwrap();
    assert!(for_person(db, &f.admin, false).await.unwrap().is_none());
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":DOCUMENT_ID},
            doc! {"$set":{"skills":bson::to_bson(&system(&Registry::new()).skills).unwrap()}},
        )
        .await
        .unwrap();
    assert!(for_person(db, &f.admin, false).await.is_err());
    db.collection::<bson::Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(
        db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .delete_one(doc! {"user_id":&f.admin,"kind":"nyxbot"})
            .await
            .unwrap()
            .deleted_count,
        1
    );
    let no_nyxbot = for_person(db, &f.admin, false).await.unwrap().unwrap();
    assert!(no_nyxbot.nyxbot_overrides.is_empty());
    assert_eq!(no_nyxbot.mode, SystemSkillsForSpecialists::Default);
    assert_eq!(effective(&agent, &no_nyxbot).len(), 1);
    for (person, mode, choice) in [
        (&f.admin, SystemSkillsForSpecialists::Override, false),
        (&f.member, SystemSkillsForSpecialists::Default, true),
    ] {
        flag(db, person, true).await;
        super::super::assistant_settings_service::update(
            db,
            person,
            super::super::assistant_settings_service::Update {
                system_skills_for_specialists: Some(mode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut nyxbot = self::agent();
        nyxbot.id = uuid::Uuid::new_v4().to_string();
        nyxbot.user_id = person.clone();
        nyxbot.system_skill_overrides.insert(ID.into(), choice);
        db.collection(crate::models::assistant_agent::COLLECTION_NAME)
            .insert_one(nyxbot)
            .await
            .unwrap();
    }
    let admin = response(db, &f.admin, agent.clone()).await.unwrap();
    assert!(!admin.system_skills[0].effective);
    assert_eq!(
        admin.system_skills[0].source,
        SystemSkillSource::NyxbotOverride
    );
    let member = response(db, &f.member, agent).await.unwrap();
    assert!(member.system_skills[0].effective);
    assert_eq!(
        member.system_skills[0].source,
        SystemSkillSource::NyxbotDefault
    );
    assert_eq!(
        member.system_skills_for_specialists,
        Some(SystemSkillsForSpecialists::Default)
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn saving_own_pins_preserves_built_in_response() {
    let f = orchestrator_fixture("system_skills_set_response").await;
    flag(&f.state.db, &f.owner, true).await;
    f.state
        .db
        .collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    let registry = Registry::new();
    for (revision, pins) in [
        (0, vec![]),
        (0, vec![registry.reference()]),
        (1, vec![registry.reference()]),
    ] {
        let saved = skills::set(
            &f.state.db,
            &registry,
            &f.owner,
            &f.chat.agent_id,
            &skills::Selection {
                expected_revision: revision,
                skills: pins,
            },
            true,
        )
        .await
        .unwrap();
        let agent =
            super::super::assistant_team_service::agent(&f.state.db, &f.owner, &f.chat.agent_id)
                .await
                .unwrap();
        let loaded = response(&f.state.db, &f.owner, agent).await.unwrap();
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::to_value(loaded).unwrap()
        );
    }
    for revision in [1, 2] {
        let saved = super::super::agent_skill_preload::set(
            &f.state.db,
            &registry,
            &f.owner,
            &f.chat.agent_id,
            ID,
            &super::super::agent_skill_preload::Selection {
                expected_revision: revision,
                paths: vec!["SKILL.md".into()],
            },
        )
        .await
        .unwrap();
        let agent =
            super::super::assistant_team_service::agent(&f.state.db, &f.owner, &f.chat.agent_id)
                .await
                .unwrap();
        let loaded = response(&f.state.db, &f.owner, agent).await.unwrap();
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::to_value(loaded).unwrap()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn flag_lookup_failure_preserves_runtime_availability_without_guidance() {
    use crate::services::{
        assistant_nyxagent_runtime as runtime, platform_manager_contract as contract,
    };
    struct PreparedEgress;
    #[async_trait::async_trait]
    impl contract::NyxAgentEgress for PreparedEgress {
        async fn request(
            &self,
            _: &contract::NyxAgentExecutionContext,
            _: &contract::RuntimeCredential,
            request: contract::NyxAgentRequest,
        ) -> AppResult<axum::response::Response> {
            assert_eq!(request.relative_path, "v1/responses");
            let body = request.body.unwrap();
            let instructions = body["instructions"].as_str().unwrap();
            assert!(!instructions.contains(super::super::assistant_skill_authoring::GUIDANCE));
            assert!(!instructions.contains("Built-in skills (selected by NyxID"));
            let completed = json!({
                "type":"response.completed", "sequence_number":0,
                "response":{"id":"resp_11111111111111111111111111111111_22222222222222222222222222222222", "status":"completed",
                    "conversation":{"id":"conv_11111111111111111111111111111111"},
                    "output":[{"type":"message","role":"assistant",
                        "content":[{"type":"output_text","text":"Prepared without optional guidance"}]}]}
            });
            Ok(axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from(format!("data: {completed}\n\n")))
                .unwrap())
        }
    }
    let f = orchestrator_fixture("system_skills_flag_lookup_failure").await;
    let db = &f.state.db;
    let mut credential = super::super::assistant_agent_credential_service::load_or_provision(
        db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap();
    // A malformed configuration would fail preparation if an unknown flag
    // state accidentally enabled system guidance after the lookup error.
    db.collection::<bson::Document>(COLLECTION_NAME)
        .insert_one(doc! {"_id":DOCUMENT_ID,"skills":"malformed"})
        .await
        .unwrap();
    let flags_collection = crate::models::feature_flag_override::COLLECTION_NAME;
    db.collection::<bson::Document>(flags_collection)
        .drop()
        .await
        .unwrap();
    db.collection::<bson::Document>("broken_guidance_flags")
        .insert_one(
            doc! {"flag_key":flags::SYSTEM_SKILLS_FLAG_KEY,"definition_id":null,
            "target_kind":"user","target_key":&f.owner,"fault":"not-an-integer"},
        )
        .await
        .unwrap();
    db.run_command(
        doc! {"create":flags_collection,"viewOn":"broken_guidance_flags",
        "pipeline":[{"$set":{"enabled":{"$toBool":{"$toInt":"$fault"}}}}]},
    )
    .await
    .unwrap();
    assert!(guidance_flags(db, &f.owner, false).await.is_err());
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let mut events = runtime::Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    let mut context = None;
    let result = Box::pin(runtime::execute_owner_turn(
        &f.state,
        &PreparedEgress,
        &runtime::owner_context(&f.row).unwrap(),
        &f.row,
        "Please answer",
        &mut credential,
        None,
        &mut events,
        "reply",
        &mut String::new(),
        &mut context,
    ))
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => panic!(
            "flag lookup failure must not fail preparation: {} at {}",
            error.code, error.stage
        ),
    };
    assert!(result.error.is_none());
    assert_eq!(result.text, "Prepared without optional guidance");
    let instructions = context.unwrap();
    assert!(
        !instructions
            .instructions()
            .contains(super::super::assistant_skill_authoring::GUIDANCE)
    );
    assert!(
        !instructions
            .instructions()
            .contains("Built-in skills (selected by NyxID")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn malformed_platform_configuration_fails_runtime_preparation_before_egress() {
    use crate::services::{
        assistant_nyxagent_runtime as runtime, platform_manager_contract as contract,
    };
    struct NoEgress;
    #[async_trait::async_trait]
    impl contract::NyxAgentEgress for NoEgress {
        async fn request(
            &self,
            _: &contract::NyxAgentExecutionContext,
            _: &contract::RuntimeCredential,
            _: contract::NyxAgentRequest,
        ) -> AppResult<axum::response::Response> {
            panic!("invalid system configuration must fail before any upstream request")
        }
    }
    let f = orchestrator_fixture("system_skills_runtime_prepare").await;
    flag(&f.state.db, &f.owner, true).await;
    f.state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .insert_one(doc! {"_id":DOCUMENT_ID,"skills":"malformed"})
        .await
        .unwrap();
    let mut credential = super::super::assistant_agent_credential_service::load_or_provision(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let mut events = runtime::Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    let result = Box::pin(runtime::execute_owner_turn(
        &f.state,
        &NoEgress,
        &runtime::owner_context(&f.row).unwrap(),
        &f.row,
        "Please answer",
        &mut credential,
        None,
        &mut events,
        "reply",
        &mut String::new(),
        &mut None,
    ))
    .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("preparation must fail"),
    };
    assert_eq!(error.code, "assistant_unavailable");
    assert_eq!(error.stage, "prepare");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn catalog_pin_is_revalidated_after_approval_before_dispatch() {
    catalog_pin_dispatch_check(false).await;
}

#[tokio::test]
async fn catalog_only_node_pin_is_revalidated_after_approval_before_every_dispatch() {
    catalog_pin_dispatch_check(true).await;
}

async fn catalog_pin_dispatch_check(via_node: bool) {
    use crate::handlers::agent_skills;
    use crate::models::service_approval_config::{ApprovalMode, ServiceApprovalConfig};
    for change in [
        "none",
        "removed",
        "pin_changed",
        "opt_out",
        "flag",
        "nyxbot_override",
        "attached",
        "node_retry",
        "http_fallback",
    ] {
        if !via_node && matches!(change, "node_retry" | "http_fallback") {
            continue;
        }
        let f = fixture(&format!("system_skills_catalog_{change}")).await;
        let db = &f.state.db;
        flag(db, &f.owner, true).await;
        let registry = Registry::new();
        db.collection(COLLECTION_NAME)
            .insert_one(system(&registry))
            .await
            .unwrap();
        if change == "attached" {
            db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&f.chat.agent_id},
                    doc! {"$set":{"skills":[bson::to_bson(&registry.reference()).unwrap()]}},
                )
                .await
                .unwrap();
        }
        if change == "nyxbot_override" {
            db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&f.chat.agent_id},
                    doc! {"$set":{format!("system_skill_overrides.{ID}"):true}},
                )
                .await
                .unwrap();
            db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"user_id":&f.owner,"kind":"nyxbot"},
                    doc! {"$set":{format!("system_skill_overrides.{ID}"):false}},
                )
                .await
                .unwrap();
        }
        let upstream = wiremock::MockServer::start().await;
        registry.mount(&upstream).await;
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.slug = "ornn-api".into();
        catalog.base_url = upstream.uri();
        catalog.identity_propagation_mode = "jwt".into();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
        // The legacy node route deliberately has no UserService and therefore
        // no assistant connection admission. Both nodes must remain untouched
        // after revocation; the unchanged control proves this route dispatches.
        let mut node_receivers = Vec::new();
        if via_node {
            for priority in 0..if change == "http_fallback" { 1 } else { 2 } {
                let node_id = uuid::Uuid::new_v4().to_string();
                let node: crate::models::node::Node = bson::from_document(doc! {
                    "_id":&node_id,"user_id":&f.owner,"name":"skill-node",
                    "status":"online","auth_token_hash":"test",
                    "signing_secret_hash":crate::crypto::token::hash_token(&"11".repeat(32)),
                    "signing_secret_encrypted":bson::to_bson(&f.state.encryption_keys.encrypt("11".repeat(32).as_bytes()).await.unwrap()).unwrap(),
                    "is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
                    "last_heartbeat_at":bson::DateTime::now(),"connected_at":bson::DateTime::now(),
                }).unwrap();
                db.collection(crate::models::node::COLLECTION_NAME)
                    .insert_one(node)
                    .await
                    .unwrap();
                db.collection(crate::models::node_service_binding::COLLECTION_NAME)
                    .insert_one(crate::models::node_service_binding::NodeServiceBinding {
                        id: uuid::Uuid::new_v4().to_string(),
                        node_id: node_id.clone(),
                        user_id: f.owner.clone(),
                        service_id: catalog.id.clone(),
                        is_active: true,
                        priority,
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                    })
                    .await
                    .unwrap();
                let (tx, rx) = tokio::sync::mpsc::channel(8);
                crate::test_utils::register_test_node_connection(&f.state, &node_id, tx).await;
                node_receivers.push((node_id, rx));
            }
            assert!(
                super::super::node_routing_service::resolve_node_route(
                    db,
                    &f.owner,
                    &catalog.id,
                    &f.state.node_ws_manager,
                )
                .await
                .unwrap()
                .is_some()
            );
        }
        assert_eq!(
            db.collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
                .count_documents(doc! {"catalog_service_id":&catalog.id})
                .await
                .unwrap(),
            0
        );
        super::super::assistant_settings_service::update(
            db,
            &f.owner,
            super::super::assistant_settings_service::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        db.collection(crate::models::service_approval_config::COLLECTION_NAME)
            .insert_one(ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: catalog.id.clone(),
                service_name: catalog.name.clone(),
                approval_required: true,
                approval_mode: ApprovalMode::PerRequest,
                rules: vec![],
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        let state = f.state.clone();
        let chat = f.chat.clone();
        let pending = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            agent_skills::dispatch(&state, &chat, "skill_read", &json!({"skill":ID})).await
        }));
        let request = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if let Some(request) = db
                    .collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME)
                    .find_one(doc! {"user_id":&f.owner,"service_id":&catalog.id,"status":"pending"})
                    .await
                    .unwrap()
                {
                    break request;
                }
                assert!(
                    !pending.is_finished(),
                    "{change}: read ended before approval"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("catalog read reaches ordinary service approval");
        assert!(upstream.received_requests().await.unwrap().is_empty());
        match change {
            "pin_changed" => {
                db.collection::<bson::Document>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":DOCUMENT_ID},
                        doc! {"$set":{"skills.0.reference.version":"2.0"}},
                    )
                    .await
                    .unwrap();
            }
            "removed" => {
                db.collection::<bson::Document>(COLLECTION_NAME)
                    .update_one(doc! {"_id":DOCUMENT_ID}, doc! {"$set":{"skills":[]}})
                    .await
                    .unwrap();
            }
            "opt_out" => {
                db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&f.chat.agent_id},
                        doc! {"$set":{format!("system_skill_overrides.{ID}"):false}},
                    )
                    .await
                    .unwrap();
            }
            "flag" => flag(db, &f.owner, false).await,
            "nyxbot_override" => {
                super::super::assistant_settings_service::update(
                    db,
                    &f.owner,
                    super::super::assistant_settings_service::Update {
                        system_skills_for_specialists: Some(SystemSkillsForSpecialists::Override),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            }
            "attached" => {
                db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&f.chat.agent_id},
                        doc! {"$set":{"skills.0.disabled":true}},
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        super::super::approval_service::process_decision(
            db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            request.get_str("_id").unwrap(),
            true,
            None,
            None,
            "web",
        )
        .await
        .unwrap();
        let peer = if via_node && matches!(change, "none" | "node_retry" | "http_fallback") {
            let (node_id, mut rx) = node_receivers.remove(0);
            let manager = f.state.node_ws_manager.clone();
            let bytes = registry.bytes.clone();
            let peer_db = db.clone();
            Some(tokio::spawn(async move {
                let Some(crate::services::node_ws_manager::NodeOutboundMessage::Text(message)) =
                    rx.recv().await
                else {
                    panic!("expected node proxy request");
                };
                let value: Value = serde_json::from_str(&message).unwrap();
                assert_eq!(value["type"], "proxy_request");
                if change != "none" {
                    peer_db
                        .collection::<bson::Document>(COLLECTION_NAME)
                        .update_one(doc! {"_id":DOCUMENT_ID}, doc! {"$set":{"skills":[]}})
                        .await
                        .unwrap();
                    manager.deliver_proxy_error(
                        &node_id,
                        value["request_id"].as_str().unwrap(),
                        "Downstream transport lost",
                        502,
                        true,
                        None,
                    );
                    return;
                }
                manager.deliver_proxy_response(
                    &node_id,
                    crate::services::node_ws_manager::NodeProxyResponse {
                        request_id: value["request_id"].as_str().unwrap().into(),
                        status: 200,
                        headers: vec![],
                        body: bytes,
                    },
                );
            }))
        } else {
            None
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), pending)
            .await
            .unwrap()
            .unwrap();
        if change == "none" {
            let (value, error) = result.unwrap();
            assert!(!error);
            assert_eq!(value["content"], "Package content, untrusted guidance");
            assert_eq!(
                upstream.received_requests().await.unwrap().len(),
                usize::from(!via_node)
            );
        } else {
            assert!(result.is_err(), "{change}: stale pin must fail");
            if matches!(change, "node_retry" | "http_fallback") {
                assert!(
                    matches!(result, Err(AppError::SkillRead(_))),
                    "{change}: final binding fence must refuse: {result:?}"
                );
            }
            assert!(
                upstream.received_requests().await.unwrap().is_empty(),
                "{change}: no stale download"
            );
        }
        if let Some(peer) = peer {
            peer.await.unwrap();
        }
        for (_, mut rx) in node_receivers {
            assert!(
                rx.try_recv().is_err(),
                "{change}: no request to a node or fallback"
            );
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pinned_downloads_never_follow_same_host_or_cross_host_redirects() {
    for cross_host in [false, true] {
        let f = orchestrator_fixture("system_skills_redirect").await;
        flag(&f.state.db, &f.owner, true).await;
        let registry = Registry::new();
        f.state
            .db
            .collection(COLLECTION_NAME)
            .insert_one(system(&registry))
            .await
            .unwrap();
        let upstream = wiremock::MockServer::start().await;
        let other = wiremock::MockServer::start().await;
        let destination = if cross_host {
            other.uri().replace("127.0.0.1", "localhost")
        } else {
            upstream.uri()
        };
        for server in [&upstream, &other] {
            wiremock::Mock::given(wiremock::matchers::path("/escaped"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_bytes(registry.bytes.clone()),
                )
                .mount(server)
                .await;
        }
        wiremock::Mock::given(wiremock::matchers::path(format!(
            "/api/v1/skills/{ID}/versions/1.0/download"
        )))
        .respond_with(
            wiremock::ResponseTemplate::new(302)
                .insert_header("location", format!("{destination}/escaped")),
        )
        .mount(&upstream)
        .await;
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.slug = "ornn-api".into();
        catalog.base_url = upstream.uri();
        catalog.identity_propagation_mode = "jwt".into();
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .insert_one(catalog)
            .await
            .unwrap();
        assert!(
            crate::handlers::agent_skills::dispatch(
                &f.state,
                &f.chat,
                "skill_read",
                &json!({"skill":ID})
            )
            .await
            .is_err()
        );
        // Metadata remains owner scoped; enabled preview downloads use the same
        // bound reader and must refuse the same redirect before reaching it.
        for route in [
            format!("/api/v1/skills/{ID}"),
            format!("/api/v1/skills/{ID}/closure"),
        ] {
            let value = registry.value(&format!("{route}?version=1.0")).unwrap();
            wiremock::Mock::given(wiremock::matchers::path(route))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(value))
                .mount(&upstream)
                .await;
        }
        assert!(
            crate::handlers::agent_skills::preview_attached(
                axum::extract::State(f.state.clone()),
                crate::test_utils::test_auth_user(&f.owner),
                axum::extract::Path((f.chat.agent_id.clone(), ID.into())),
            )
            .await
            .is_err()
        );
        let requests = upstream.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.url.path().ends_with("/download"))
                .count(),
            2
        );
        assert!(
            requests
                .iter()
                .all(|request| request.url.path() != "/escaped")
        );
        assert!(requests[0].headers.contains_key("x-nyxid-identity-token"));
        assert_ne!(requests[0].url.path(), "/escaped");
        assert!(other.received_requests().await.unwrap().is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn guidance_snapshot_adds_no_flag_off_turn_reads_and_empty_config_is_one_read() {
    use crate::models::feature_flag_override::{FeatureFlagOverride, FlagTargetKind};
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::<bson::Document>::new()));
    let recorded = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "system_skills_query_counts",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                recorded.lock().unwrap().push(event.command);
            }
        }),
    )
    .await
    .unwrap();
    let mut agent = agent();
    agent.user_id = uuid::Uuid::new_v4().to_string();
    let settings = AssistantSettings::defaults(&agent.user_id);
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &agent.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    db.collection(COLLECTION_NAME)
        .insert_one(system(&Registry::new()))
        .await
        .unwrap();
    // More unrelated system-skill recipients than fit in a find's first batch
    // must not add cursor reads to an ordinary flag-off person's turn.
    db.collection::<FeatureFlagOverride>(crate::models::feature_flag_override::COLLECTION_NAME)
        .insert_many((0..150).map(|_| FeatureFlagOverride {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: None,
            flag_key: flags::SYSTEM_SKILLS_FLAG_KEY.into(),
            definition_id: None,
            target_kind: FlagTargetKind::User,
            target_key: Some(uuid::Uuid::new_v4().to_string()),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: agent.user_id.clone(),
        }))
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    assert!(
        !super::super::assistant_skill_authoring::enabled(&db, &agent.user_id)
            .await
            .unwrap()
    );
    let baseline = commands.lock().unwrap().clone();
    assert_eq!(
        baseline.len(),
        1,
        "main authoring gate performs one bounded find"
    );
    commands.lock().unwrap().clear();
    let (authoring, system_skills) = guidance_flags(&db, &agent.user_id, false).await.unwrap();
    assert!(!authoring && !system_skills);
    assert!(
        guidance(
            &db,
            &agent.user_id,
            false,
            Some(&agent),
            Some(&settings),
            system_skills
        )
        .await
        .unwrap()
        .is_empty()
    );
    let off = commands.lock().unwrap().clone();
    assert_eq!(off.len(), baseline.len(), "flag-off turn adds zero queries");
    assert!(off.iter().all(|command| command.get_str("aggregate")
        == Ok(crate::models::feature_flag_override::COLLECTION_NAME)));
    super::super::security_readiness_service::ready_test_replica(&db).await;
    flag(&db, &agent.user_id, true).await;
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(doc! {"_id":DOCUMENT_ID}, doc! {"$set":{"skills":[]}})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let (_, eligible) = guidance_flags(&db, &agent.user_id, false).await.unwrap();
    assert!(eligible);
    assert_eq!(commands.lock().unwrap().len(), 1, "one shared flag lookup");
    commands.lock().unwrap().clear();
    assert!(
        guidance(
            &db,
            &agent.user_id,
            false,
            Some(&agent),
            Some(&settings),
            eligible
        )
        .await
        .unwrap()
        .is_empty()
    );
    let empty = commands.lock().unwrap().clone();
    assert_eq!(empty.len(), 1, "empty set reads only its indexed _id");
    assert_eq!(empty[0].get_str("find").unwrap(), COLLECTION_NAME);
    assert_eq!(
        empty[0].get_document("filter").unwrap(),
        &doc! {"_id":DOCUMENT_ID}
    );
    commands.lock().unwrap().clear();
    assert!(
        for_person(&db, &agent.user_id, false)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        commands.lock().unwrap().len(),
        1,
        "fresh empty configuration never resolves flags or inheritance"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn flag_off_settings_omit_mode_from_all_response_surfaces() {
    use crate::handlers::assistant_team;
    use axum::{
        Json,
        extract::{Query, State},
    };
    let f = orchestrator_fixture("system_skills_settings_hidden").await;
    let Json(get) = assistant_team::get_settings(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
    )
    .await
    .unwrap();
    assert!(
        serde_json::to_value(get)
            .unwrap()
            .get("system_skills_for_specialists")
            .is_none()
    );
    let Json(put) = assistant_team::update_settings(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Json(serde_json::from_value(json!({"max_auto_continuations":3})).unwrap()),
    )
    .await
    .unwrap();
    assert!(
        serde_json::to_value(put)
            .unwrap()
            .get("system_skills_for_specialists")
            .is_none()
    );
    let Json(list) = assistant_team::list_agents(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Query(serde_json::from_value(json!({})).unwrap()),
    )
    .await
    .unwrap();
    assert!(
        list["limits"]
            .get("system_skills_for_specialists")
            .is_none()
    );
    let (native, error) = assistant_team::execute_tool(
        &f.state,
        &f.chat,
        "nyxid__update_settings",
        &json!({"max_auto_continuations":4}),
    )
    .await;
    assert!(!error);
    assert_eq!(native["max_auto_continuations"], 4);
    assert!(native.get("system_skills_for_specialists").is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn catalog_pin_projected_check_rejects_changed_authority_and_non_exact_requests() {
    use crate::services::{assistant_service_execution as execution, proxy_service};
    let mut f = fixture("system_skills_catalog_projection").await;
    f.auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    let reader = Registry::new();
    flag(&f.state.db, &f.owner, true).await;
    f.state
        .db
        .collection(COLLECTION_NAME)
        .insert_one(system(&reader))
        .await
        .unwrap();
    let agent =
        super::super::assistant_team_service::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
    let context = for_person(&f.state.db, &f.owner, false).await.unwrap();
    read_resolved(&reader, &agent, context.as_ref(), ID, None, "SKILL.md", 0)
        .await
        .unwrap();
    let pin = skills::OrnnSkillRead::new(&agent.id, reader.reference());
    let path = format!("/api/v1/skills/{ID}/versions/1.0/download");
    let projection = execution::HttpProjection {
        skill_read: Some(pin),
        ..Default::default()
    };
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let target = proxy_service::resolve_proxy_target(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &catalog.id,
        crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
    )
    .await
    .unwrap();
    let headers = axum::http::HeaderMap::new();
    let check = |method, path, query, body, pool| {
        execution::check_http_projected(
            &f.state,
            &f.auth,
            None,
            None,
            &target,
            method,
            path,
            query,
            body,
            None,
            pool,
            &headers,
            &projection,
        )
    };
    let decision = check("GET", &path, None, &[], None).await.unwrap();
    assert!(
        decision.admission.is_none(),
        "catalog reads have no connection admission"
    );
    for (method, path, query, body, pool) in [
        ("POST", path.as_str(), None, &[][..], None),
        (
            "GET",
            "/api/v1/skills/other/versions/1.0/download",
            None,
            &[],
            None,
        ),
        ("GET", path.as_str(), Some("extra=true"), &[], None),
        ("GET", path.as_str(), None, b"extra".as_slice(), None),
        ("GET", path.as_str(), None, &[], Some("pool")),
    ] {
        assert!(check(method, path, query, body, pool).await.is_err());
    }
    f.state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{format!("system_skill_overrides.{ID}"):false}},
        )
        .await
        .unwrap();
    assert!(check("GET", &path, None, &[], None).await.is_err());
    assert!(
        execution::HttpProjection::default().skill_read.is_none(),
        "ordinary catalog reads acquire no bound-read authority"
    );
    f.state.db.drop().await.unwrap();
}

async fn read_resolved(
    reader: &impl OrnnReader,
    agent: &AssistantAgent,
    context: Option<&Context>,
    skill: &str,
    dependency: Option<&str>,
    path: &str,
    offset: usize,
) -> AppResult<Value> {
    skills::read(
        reader,
        &read_view(agent, context),
        skill,
        dependency,
        path,
        offset,
    )
    .await
}

#[tokio::test]
async fn attached_preview_reports_dependency_pin_mismatch_before_download_with_system_flag_off() {
    use axum::extract::{Path, State};
    let f = orchestrator_fixture("attached_preview_dep_drift").await;
    flag(&f.state.db, &f.owner, false).await;
    let registry = Registry::new();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills":[bson::to_bson(&registry.reference()).unwrap()]}},
        )
        .await
        .unwrap();
    let upstream = wiremock::MockServer::start().await;
    registry.mount(&upstream).await;
    wiremock::Mock::given(wiremock::matchers::path(format!(
        "/api/v1/skills/{ID}/closure"
    )))
    .respond_with(
        wiremock::ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[{
            "guid":DEP,"name":"dependency","version":"2.0","skillHash":registry.reference().sha256
        }]}})),
    )
    .with_priority(1)
    .mount(&upstream)
    .await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(catalog)
        .await
        .unwrap();
    let error = crate::handlers::agent_skills::preview_attached(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Path((f.chat.agent_id.clone(), ID.into())),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AppError::SkillRead(crate::errors::skill_read::SkillReadReason::PinnedVersionMismatch)
    ));
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "metadata and closure only");
    assert!(
        requests
            .iter()
            .all(|r| !r.url.path().ends_with("/download"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_catalog_reads_ignore_engine_flag_and_retain_mounted_admin_auth_posture() {
    use crate::{crypto::jwt, services::key_service};
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let f = orchestrator_fixture("admin_skills_catalog_engine_off").await;
    let db = &f.state.db;
    super::super::role_service::seed_system_roles(db)
        .await
        .unwrap();
    let roles = super::super::role_service::get_platform_role_ids(db)
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$addToSet":{"role_ids":roles.admin}},
        )
        .await
        .unwrap();
    flags::set_platform_override(
        db,
        "assistant:nyxagent-engine",
        &flags::FlagTarget::User(f.owner.clone()),
        false,
        &f.owner,
    )
    .await
    .unwrap();
    assert!(
        super::super::assistant_nyxagent::require_enabled(db, &f.owner)
            .await
            .is_err()
    );
    let upstream = wiremock::MockServer::start().await;
    let registry = Registry::new();
    registry.mount(&upstream).await;
    for (path, body) in [
        (
            "/api/v1/skill-search".to_owned(),
            json!({"data":{"items":[{"guid":ID,"name":"nyxbot","description":"Useful"}],"totalPages":1}}),
        ),
        (
            format!("/api/v1/skills/{ID}/versions"),
            json!({"data":{"items":[{"version":"1.0","skillHash":registry.reference().sha256,"isDeprecated":false}]}}),
        ),
    ] {
        wiremock::Mock::given(wiremock::matchers::path(path))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&upstream)
            .await;
    }
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    db.collection(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(catalog)
        .await
        .unwrap();
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let app = private.with_state(f.state.clone());
    let paths = [
        "/api/v1/admin/assistant/system-skills/search?q=nyxbot&page=1".to_owned(),
        format!("/api/v1/admin/assistant/system-skills/{ID}/versions?page=1"),
        format!("/api/v1/admin/assistant/system-skills/{ID}/versions/1.0/preview"),
    ];
    let non_admin = uuid::Uuid::new_v4();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &non_admin.to_string(),
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let key = key_service::create_api_key(
        db,
        &f.owner,
        "admin catalog",
        "read admin",
        None,
        None,
        None,
        None,
        Some(true),
        None,
        Some(true),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &uuid::Uuid::parse_str(&f.owner).unwrap(),
        "account:read",
        "test",
        60,
        None,
    )
    .unwrap();
    let (client, _) = super::super::oauth_client_service::create_client(
        db,
        "Admin catalog OAuth boundary",
        &[],
        "public",
        &f.owner,
        "account:read",
        "account:read",
        Default::default(),
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    let consent = super::super::consent_service::grant_consent_with_services(
        db,
        &f.owner,
        &client.id,
        "account:read",
        Some(vec![]),
    )
    .await
    .unwrap();
    assert!(!consent.allow_all_services);
    assert_eq!(consent.allowed_service_ids, Some(vec![]));
    let reference = super::super::consent_service::ensure_issuance_reference(db, &consent)
        .await
        .unwrap();
    let oauth = jwt::generate_oauth_access_token_with_consent(
        &f.state.jwt_keys,
        &f.state.config,
        &uuid::Uuid::parse_str(&f.owner).unwrap(),
        "account:read",
        None,
        None,
        None,
        None,
        Some(jwt::AccessTokenRestrictions {
            resources: &[],
            allowed_service_ids: &[],
            allowed_node_ids: &[],
            allow_all_nodes: false,
        }),
        &client.id,
        Some(&reference),
    )
    .unwrap();
    let claims = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &oauth).unwrap();
    assert_eq!(claims.allow_all_services, Some(false));
    assert_eq!(claims.allowed_service_ids, Some(vec![]));
    assert_eq!(claims.scope, "account:read");
    // Prove this is an accepted admin token, rather than a token rejected by
    // authentication. The non-executing configuration GET keeps its posture.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/assistant/system-skills")
                .header("authorization", format!("Bearer {oauth}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(upstream.received_requests().await.unwrap().is_empty());
    for (token, expected) in [
        (
            jwt::generate_access_token(
                &f.state.jwt_keys,
                &f.state.config,
                &uuid::Uuid::parse_str(&f.owner).unwrap(),
                "read write",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
            200,
        ),
        (
            jwt::generate_access_token(
                &f.state.jwt_keys,
                &f.state.config,
                &non_admin,
                "read write",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
            403,
        ),
        (key.full_key.to_string(), 403),
        (delegated, 403),
        (oauth, 403),
    ] {
        for path in &paths {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert_eq!(
                status.as_u16(),
                expected,
                "{path}: {}",
                String::from_utf8_lossy(&bytes)
            );
            if expected == 200 {
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                if path.ends_with("preview") {
                    assert_eq!(body["reference"]["skill_id"], ID);
                } else {
                    assert_eq!(body["items"].as_array().unwrap().len(), 1);
                }
            }
        }
    }
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 6, "only the admin's GETs reach Ornn");
    for request in requests {
        assert_eq!(request.method, "GET");
        let identity = request.headers["x-nyxid-identity-token"].to_str().unwrap();
        let payload = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            identity.split('.').nth(1).unwrap(),
        )
        .unwrap();
        let claims: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(claims["sub"], f.owner);
    }
    db.drop().await.unwrap();
}
