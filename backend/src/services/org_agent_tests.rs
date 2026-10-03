use super::{
    assistant_acknowledgement_service as acks, assistant_nyxagent as engine,
    assistant_team_service as team, feature_flag_service as flags, org_agent_service as org_agents,
};
use crate::{
    models::{
        api_key::ApiKey,
        assistant_agent::{AgentGrants, AssistantAgent},
        assistant_conversation::AssistantConversation,
        org_membership::OrgRole,
        user::UserType,
    },
    test_utils::*,
};
use mongodb::bson::{self, doc};
use uuid::Uuid;

pub(crate) struct Fixture {
    pub(crate) state: crate::AppState,
    pub(crate) admin: String,
    pub(crate) member: String,
    pub(crate) viewer: String,
    pub(crate) outsider: String,
    pub(crate) org: String,
}

#[tokio::test]
async fn org_agent_upload_drafts_keep_member_identity_privacy_and_live_access() {
    use super::assistant_upload_service::owner_scope;
    use crate::handlers::assistant_uploads::{self, Draft};
    use axum::{Json, extract::State};

    let f = Fixture::new("org_agent_upload_drafts").await;
    let (agent, _) = f.create().await;
    flags::set_platform_override(
        &f.state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let mut threads = Vec::new();
    for actor in [&f.admin, &f.member] {
        let Json(response) = Box::pin(assistant_uploads::draft(
            State(f.state.clone()),
            test_auth_user(actor),
            Json(Draft {
                agent_id: Some(agent.id.clone()),
            }),
        ))
        .await
        .unwrap();
        let row = engine::get(&f.state.db, actor, response["id"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(&row.user_id, actor);
        assert_eq!(row.agent_owner_id.as_deref(), Some(f.org.as_str()));
        let key = f.key(&row).await;
        assert_eq!(&key.user_id, actor);
        assert_eq!(
            key.assistant_agent_owner_id.as_deref(),
            Some(f.org.as_str())
        );
        owner_scope(&f.state.db, actor, &row.id).await.unwrap();
        threads.push(row.id);
    }
    assert_ne!(threads[0], threads[1]);
    assert!(
        owner_scope(&f.state.db, &f.member, &threads[0])
            .await
            .is_err()
    );
    assert!(
        owner_scope(&f.state.db, &f.admin, &threads[1])
            .await
            .is_err()
    );
    f.revoke(&f.member).await;
    assert!(
        owner_scope(&f.state.db, &f.member, &threads[1])
            .await
            .is_err()
    );
    for actor in [&f.member, &f.viewer, &f.outsider] {
        assert!(
            Box::pin(assistant_uploads::draft(
                State(f.state.clone()),
                test_auth_user(actor),
                Json(Draft {
                    agent_id: Some(agent.id.clone())
                }),
            ))
            .await
            .is_err()
        );
    }
    f.state.db.drop().await.unwrap();
}

impl Fixture {
    pub(crate) async fn new(name: &str) -> Self {
        let db = connect_transaction_test_database(name).await;
        Self::with_db(db).await
    }
    pub(crate) async fn observed(
        name: &str,
    ) -> (
        Self,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let memberships = Arc::new(AtomicUsize::new(0));
        let reads = Arc::new(AtomicUsize::new(0));
        let (member_count, read_count) = (memberships.clone(), reads.clone());
        let handler = EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && let Ok(collection) = event.command.get_str("find")
            {
                read_count.fetch_add(1, Ordering::SeqCst);
                if collection == crate::models::org_membership::COLLECTION_NAME {
                    member_count.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let db = connect_test_database_with_command_handler(name, handler)
            .await
            .unwrap();
        (Self::with_db(db).await, memberships, reads)
    }
    async fn with_db(db: mongodb::Database) -> Self {
        engine::ensure_indexes(&db).await.unwrap();
        let ids: Vec<_> = (0..5).map(|_| Uuid::new_v4().to_string()).collect();
        for (i, id) in ids.iter().enumerate() {
            let mut user = test_user(
                id,
                if i == 4 {
                    UserType::Org
                } else {
                    UserType::Person
                },
            );
            if i == 4 {
                user.slug = Some("org-agent-team".into());
                user.display_name = Some("Agent Team".into());
            }
            db.collection(crate::models::user::COLLECTION_NAME)
                .insert_one(user)
                .await
                .unwrap();
        }
        for (i, role) in [OrgRole::Admin, OrgRole::Member, OrgRole::Viewer]
            .into_iter()
            .enumerate()
        {
            db.collection(crate::models::org_membership::COLLECTION_NAME)
                .insert_one(test_membership(&ids[4], &ids[i], role, None))
                .await
                .unwrap();
        }
        Self {
            state: test_app_state(db),
            admin: ids[0].clone(),
            member: ids[1].clone(),
            viewer: ids[2].clone(),
            outsider: ids[3].clone(),
            org: ids[4].clone(),
        }
    }
    async fn flag(&self, enabled: bool) {
        flags::set_platform_override(
            &self.state.db,
            "assistant:org-agents",
            &flags::FlagTarget::Global,
            enabled,
            &self.admin,
        )
        .await
        .unwrap();
    }
    fn request(&self, name: &str) -> team::CreateRequest {
        team::CreateRequest {
            name: name.into(),
            description: "Organization specialist".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            machines: None,
            logins: None,
            specialty: None,
            created_by: "user",
        }
    }
    pub(crate) async fn create(&self) -> (AssistantAgent, AssistantConversation) {
        self.flag(true).await;
        team::create_specialist_for(
            &self.state.db,
            &self.state.encryption_keys,
            &self.admin,
            &self.org,
            self.request("researcher"),
        )
        .await
        .unwrap()
        .unwrap()
    }
    async fn service(&self, owner: &str, slug: &str) -> String {
        let endpoint = Uuid::new_v4().to_string();
        let id = Uuid::new_v4().to_string();
        self.state
            .db
            .collection(crate::models::user_endpoint::COLLECTION_NAME)
            .insert_one(test_user_endpoint(
                &endpoint,
                owner,
                slug,
                "https://example.invalid",
                None,
                None,
            ))
            .await
            .unwrap();
        self.state
            .db
            .collection(crate::models::user_service::COLLECTION_NAME)
            .insert_one(test_user_service(&id, owner, slug, &endpoint, None, None))
            .await
            .unwrap();
        id
    }
    async fn key(&self, row: &AssistantConversation) -> ApiKey {
        self.state
            .db
            .collection(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id": &row.credential_api_key_id})
            .await
            .unwrap()
            .unwrap()
    }
    pub(crate) async fn revoke(&self, person: &str) {
        self.state
            .db
            .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"org_user_id": &self.org, "member_user_id": person},
                doc! {"$set": {"revoked_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn org_agent_auth_resolves_access_once_and_reuses_only_within_request() {
    use std::sync::atomic::Ordering;
    let (f, memberships, _) = Fixture::observed("org_agent_request_snapshot").await;
    let (agent, row) = f.create().await;
    let upload = super::assistant_upload_service::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &row.id,
        "report.txt",
        b"Member attachment".to_vec(),
    )
    .await
    .unwrap();
    engine::begin_turn(
        &f.state.db,
        &f.admin,
        &engine::TurnRequest {
            attachment_ids: vec![upload.id.clone()],
            conversation_id: Some(row.id.clone()),
            agent_id: None,
            text: "Read attachment".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let service = f.service(&f.org, "allowed").await;
    team::set_grants(
        &f.state.db,
        &f.admin,
        &agent.id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let key = f.key(&row).await;
    memberships.store(0, Ordering::SeqCst);
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    assert_eq!(auth.allowed_service_ids, vec![service.clone()]);
    let chat = acks::for_key_with_access(
        &f.state.db,
        &f.admin,
        Some(&key.id),
        auth.org_agent_access.as_ref(),
    )
    .await
    .unwrap()
    .unwrap();
    org_agents::chat_agent(&f.state.db, &chat).await.unwrap();
    super::machine_service::visible_nodes(&f.state.db, &chat)
        .await
        .unwrap();
    assert!(
        acks::service_gate(&f.state.db, &chat, &service, "allowed", "Allowed", false)
            .await
            .unwrap()
            .is_none()
    );
    org_agents::authorize_execution(&f.state.db, &auth, Some(&service))
        .await
        .unwrap();
    let attachment = super::assistant_upload_service::read(
        &f.state.db,
        &f.state.encryption_keys,
        &chat,
        &serde_json::json!({"attachment_id": upload.id}),
    )
    .await
    .unwrap();
    assert_eq!(attachment["text"], "Member attachment");
    assert_eq!(
        memberships.load(Ordering::SeqCst),
        1,
        "auth, chat, grants, attachments and final execution share one access resolution"
    );

    f.revoke(&f.admin).await;
    org_agents::authorize_execution(&f.state.db, &auth, Some(&service))
        .await
        .unwrap();
    assert_eq!(
        memberships.load(Ordering::SeqCst),
        1,
        "an admitted request retains its snapshot"
    );
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    assert_eq!(
        memberships.load(Ordering::SeqCst),
        2,
        "a new authentication rechecks membership"
    );

    let mut missing = auth.clone();
    missing.org_agent_access = None;
    assert!(
        org_agents::authorize_execution(&f.state.db, &missing, Some(&service))
            .await
            .is_err()
    );
    let mut foreign_actor = auth.clone();
    foreign_actor.user_id = Uuid::parse_str(&f.outsider).unwrap();
    assert!(
        org_agents::authorize_execution(&f.state.db, &foreign_actor, Some(&service))
            .await
            .is_err()
    );
    let mut foreign_owner = auth.clone();
    foreign_owner.assistant_agent_owner_id = Some(Uuid::new_v4().to_string());
    assert!(
        org_agents::authorize_execution(&f.state.db, &foreign_owner, Some(&service))
            .await
            .is_err()
    );
    for owner in [None, Some(f.admin.clone())] {
        let mut unbound = auth.clone();
        unbound.assistant_agent_owner_id = owner;
        assert!(
            org_agents::authorize_execution(&f.state.db, &unbound, Some(&service))
                .await
                .is_err()
        );
    }
    assert_eq!(memberships.load(Ordering::SeqCst), 2);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_ordinary_key_auth_keeps_no_read_fast_path() {
    use std::sync::atomic::Ordering;
    let (f, memberships, reads) = Fixture::observed("org_agent_ordinary_fast_path").await;
    let (_, row) = f.create().await;
    let mut key = f.key(&row).await;
    key.assistant_agent_owner_id = None;
    key.allow_auto_connected_services = false;
    memberships.store(0, Ordering::SeqCst);
    reads.store(0, Ordering::SeqCst);
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    assert!(auth.org_agent_access.is_none());
    org_agents::authorize_execution(&f.state.db, &auth, None)
        .await
        .unwrap();
    assert_eq!(memberships.load(Ordering::SeqCst), 0);
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "only the existing active-person read; no org, service or node reads"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_creation_gate_and_role_matrix() {
    let f = Fixture::new("org_agent_roles").await;
    assert!(
        team::create_specialist_for(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &f.org,
            f.request("disabled")
        )
        .await
        .is_err()
    );
    f.flag(true).await;
    for actor in [&f.admin, &f.member] {
        let (agent, row) = team::create_specialist_for(
            &f.state.db,
            &f.state.encryption_keys,
            actor,
            &f.org,
            f.request(if actor == &f.admin {
                "admin-agent"
            } else {
                "member-agent"
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(agent.user_id, f.org);
        assert_eq!(row.user_id, *actor);
        assert!(agent.home_conversation_id.is_none());
        assert!(
            org_agents::require_use(&f.state.db, &f.member, &agent)
                .await
                .is_ok()
        );
        assert!(
            org_agents::require_maintain(&f.state.db, &f.member, &agent)
                .await
                .is_ok()
        );
        assert!(team::agent(&f.state.db, &f.viewer, &agent.id).await.is_ok());
        assert!(
            org_agents::require_use(&f.state.db, &f.viewer, &agent)
                .await
                .is_err()
        );
        assert!(
            team::maintained_agent(&f.state.db, &f.viewer, &agent.id)
                .await
                .is_err()
        );
        assert!(
            team::agent(&f.state.db, &f.outsider, &agent.id)
                .await
                .is_err()
        );
    }
    for actor in [&f.viewer, &f.outsider] {
        assert!(
            team::create_specialist_for(
                &f.state.db,
                &f.state.encryption_keys,
                actor,
                &f.org,
                f.request("denied")
            )
            .await
            .is_err()
        );
    }
    assert!(team::ensure_nyxbot(&f.state.db, &f.org).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_threads_are_private_and_memory_is_shared() {
    let f = Fixture::new("org_agent_privacy").await;
    let (agent, first) = f.create().await;
    let second = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(second.user_id, f.member);
    assert!(
        engine::get(&f.state.db, &f.member, &first.id)
            .await
            .is_err()
    );
    assert!(
        engine::get(&f.state.db, &f.admin, &second.id)
            .await
            .is_err()
    );
    for (person, row) in [(&f.admin, &first), (&f.member, &second)] {
        let rows = team::threads_for(&f.state.db, person, &agent, 100)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, row.id);
        let summary = team::summaries(&f.state.db, person, false, false, 0)
            .await
            .unwrap();
        assert_eq!(
            summary[0].home_conversation_id.as_deref(),
            Some(row.id.as_str())
        );
    }
    let note = team::remember(
        &f.state.db,
        &f.member,
        &agent.id,
        "Public team convention",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        team::agent(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .memory[0]
            .id,
        note.id
    );
    assert!(
        team::remember(&f.state.db, &f.viewer, &agent.id, "denied", None)
            .await
            .is_err()
    );
    team::forget(&f.state.db, &f.admin, &agent.id, &note.id)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_revocation_refuses_key_tool_turn_and_queued_work() {
    let f = Fixture::new("org_agent_revoke").await;
    let (agent, _) = f.create().await;
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let key = f.key(&row).await;
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    assert_eq!(auth.user_id.to_string(), f.member);
    assert_eq!(
        auth.assistant_agent_owner_id.as_deref(),
        Some(f.org.as_str())
    );
    f.flag(false).await; // creation flag never disables membership enforcement
    f.revoke(&f.member).await;
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    assert!(
        acks::for_key(&f.state.db, &f.member, Some(&key.id))
            .await
            .is_err()
    );
    assert!(
        engine::begin_turn(
            &f.state.db,
            &f.member,
            &engine::TurnRequest {
                attachment_ids: Vec::new(),
                conversation_id: Some(row.id.clone()),
                agent_id: None,
                text: "Next request".into(),
                model: None,
                access_mode: None
            },
            &f.state.encryption_keys
        )
        .await
        .is_err()
    );
    assert!(
        engine::begin_turn(
            &f.state.db,
            &f.member,
            &engine::TurnRequest {
                attachment_ids: Vec::new(),
                conversation_id: None,
                agent_id: Some(agent.id),
                text: "New request".into(),
                model: None,
                access_mode: None
            },
            &f.state.encryption_keys
        )
        .await
        .is_err()
    );
    assert!(engine::get(&f.state.db, &f.member, &row.id).await.is_err());
    assert!(
        engine::rename(&f.state.db, &f.member, &row.id, "Forbidden")
            .await
            .is_err()
    );
    assert!(
        engine::list(&f.state.db, &f.member, 100, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_demotion_and_inactive_owner_revoke_existing_keys() {
    let f = Fixture::new("org_agent_demote").await;
    let (agent, row) = f.create().await;
    let key = f.key(&row).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &f.admin,
            "conversation_id": &row.id, "turn_id": Uuid::new_v4().to_string(),
            "seq": 1_i64, "role": "user",
            "status": "completed", "text": "Private member context",
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    assert!(
        team::direct_chats_note(
            &f.state.db,
            &f.admin,
            chrono::Utc::now() - chrono::Duration::minutes(1)
        )
        .await
        .unwrap()
        .contains("Private member context")
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &f.org, "member_user_id": &f.admin},
            doc! {"$set": {"role": "viewer"}},
        )
        .await
        .unwrap();
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    assert!(
        team::threads_for(&f.state.db, &f.admin, &agent, 100)
            .await
            .is_err()
    );
    assert!(
        team::direct_chats_note(
            &f.state.db,
            &f.admin,
            chrono::Utc::now() - chrono::Duration::minutes(1)
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(
        team::update_agent(
            &f.state.db,
            &f.admin,
            &agent.id,
            None,
            Some("No"),
            Default::default()
        )
        .await
        .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &f.org}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert!(
        team::agent(&f.state.db, &f.member, &agent.id)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_grants_refuse_personal_cross_org_and_account_resources() {
    let f = Fixture::new("org_agent_grants").await;
    let (agent, _) = f.create().await;
    let other_org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&other_org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .insert_one(test_membership(&other_org, &f.member, OrgRole::Admin, None))
        .await
        .unwrap();
    for owner in [&f.admin, &f.member, &f.outsider, &other_org] {
        let id = f.service(owner, "personal").await;
        assert!(
            team::set_grants(
                &f.state.db,
                &f.member,
                &agent.id,
                team::GrantChange::Add(AgentGrants {
                    service_ids: vec![id],
                    ..Default::default()
                })
            )
            .await
            .is_err()
        );
    }
    assert!(
        team::set_grants(
            &f.state.db,
            &f.admin,
            &agent.id,
            team::GrantChange::Add(AgentGrants {
                account_read: true,
                ..Default::default()
            })
        )
        .await
        .is_err()
    );
    let id = f.service(&f.org, "org-service").await;
    team::set_grants(
        &f.state.db,
        &f.member,
        &agent.id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_grants_scopes_sync_across_person_owned_keys() {
    let f = Fixture::new("org_agent_sync").await;
    let (agent, first) = f.create().await;
    let second = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let service = f.service(&f.org, "org-service").await;
    team::set_grants(
        &f.state.db,
        &f.member,
        &agent.id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    set_agent_operation_scopes_enabled(&f.state.db, &f.admin, true).await;
    let selection = crate::models::agent_operation_scope::OperationSelection {
        expected_revision: 0,
        all_operations: false,
        endpoint_ids: vec![],
        rules: vec![],
    };
    super::agent_operation_scope_service::set(
        &f.state.db,
        &f.member,
        &agent.id,
        &service,
        &selection,
        false,
    )
    .await
    .unwrap();
    for row in [&first, &second] {
        let key = f.key(row).await;
        assert_eq!(key.user_id, row.user_id);
        assert_eq!(key.allowed_service_ids, vec![service.clone()]);
        assert!(
            key.assistant_operation_scopes[&service]
                .operations
                .is_empty()
        );
    }
    team::destroy(&f.state.db, &f.member, &agent.id)
        .await
        .unwrap();
    for row in [&first, &second] {
        assert!(!f.key(row).await.is_active);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_live_service_intersection_and_final_routing_guard() {
    let f = Fixture::new("org_agent_intersection").await;
    let (agent, first) = f.create().await;
    let allowed = f.service(&f.org, "allowed").await;
    let denied = f.service(&f.org, "denied").await;
    let personal = f.service(&f.admin, "personal").await;
    team::set_grants(
        &f.state.db,
        &f.admin,
        &agent.id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![allowed.clone(), denied.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &f.org, "member_user_id": &f.admin},
            doc! {"$set": {"allowed_service_ids": [&allowed]}},
        )
        .await
        .unwrap();
    let key = f.key(&first).await;
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    assert_eq!(auth.allowed_service_ids, vec![allowed.clone()]);
    assert!(
        org_agents::authorize_execution(&f.state.db, &auth, Some(&allowed))
            .await
            .is_ok()
    );
    for id in [Some(denied.as_str()), Some(personal.as_str()), None] {
        assert!(
            org_agents::authorize_execution(&f.state.db, &auth, id)
                .await
                .is_err()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_nyxbot_explicit_org_resolution_and_specialist_refusal() {
    let f = Fixture::new("org_agent_tools").await;
    let (agent, _) = f.create().await;
    for selector in [&f.org, "org-agent-team", "Agent Team"] {
        let args = serde_json::json!({"org": selector, "agent": "researcher"});
        let resolved = org_agents::tool_arguments(&f.state.db, &f.member, true, &args)
            .await
            .unwrap();
        assert_eq!(resolved["agent"], agent.id);
        assert_eq!(resolved["org"], f.org);
        assert!(
            org_agents::tool_arguments(&f.state.db, &f.viewer, true, &args)
                .await
                .is_err()
        );
        assert!(
            org_agents::tool_arguments(&f.state.db, &f.outsider, true, &args)
                .await
                .is_err()
        );
        assert!(
            org_agents::tool_arguments(&f.state.db, &f.member, false, &args)
                .await
                .is_err()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_member_key_cannot_escape_agent_grants() {
    let f = Fixture::new("org_agent_key_edits").await;
    let (_, row) = f.create().await;
    assert!(
        super::key_service::update_api_key_scope_with_expected_state_version(
            &f.state.db,
            &f.admin,
            Some(&f.admin),
            &row.credential_api_key_id,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_platform_billing_uses_acting_person() {
    let f = Fixture::new("org_agent_billing").await;
    let (agent, _) = f.create().await;
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let key = f.key(&row).await;
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    let payer = super::billing::owner_resolver::BillingOwnerResolver::new(f.state.db.clone())
        .resolve_for_execution(
            &auth.user_id.to_string(),
            &f.org,
            crate::models::usage_meter::CredentialClass::NyxidManagedMaster,
        )
        .await
        .unwrap();
    assert_eq!(payer.owner_id, f.member);
    f.state.db.drop().await.unwrap();
}

struct NoOrnn;
#[async_trait::async_trait]
impl super::agent_skill_service::OrnnReader for NoOrnn {
    async fn get(&self, _: &str) -> crate::errors::AppResult<Vec<u8>> {
        panic!("removing an existing pin must not fetch external content")
    }
}

#[tokio::test]
async fn org_agent_skills_removal_acl_and_mandatory_add_card() {
    let f = Fixture::new("org_agent_skills").await;
    let (agent, _) = f.create().await;
    let reference = crate::models::catalog_skill_revision::SkillReference {
        source: "ornn".into(),
        skill_id: Uuid::new_v4().to_string(),
        name: "Team guide".into(),
        version: "1.0".into(),
        sha256: "a".repeat(64),
        dependencies: vec![],
    };
    let selection = super::agent_skill_service::Selection {
        expected_revision: 0,
        skills: vec![reference.clone()],
    };
    // Addition cannot bypass the one-use human card even for a maintainer.
    assert!(
        super::agent_skill_service::set(
            &f.state.db,
            &NoOrnn,
            &f.member,
            &agent.id,
            &selection,
            false
        )
        .await
        .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"skills": bson::to_bson(&vec![reference]).unwrap()}},
        )
        .await
        .unwrap();
    let removal = super::agent_skill_service::Selection {
        expected_revision: 0,
        skills: vec![],
    };
    assert!(
        super::agent_skill_service::set(
            &f.state.db,
            &NoOrnn,
            &f.viewer,
            &agent.id,
            &removal,
            false
        )
        .await
        .is_err()
    );
    let result = super::agent_skill_service::set(
        &f.state.db,
        &NoOrnn,
        &f.member,
        &agent.id,
        &removal,
        false,
    )
    .await
    .unwrap();
    assert!(result.skills.is_empty());
    assert_eq!(result.revision, 1);
    f.revoke(&f.member).await;
    assert!(
        super::agent_skill_service::set(
            &f.state.db,
            &NoOrnn,
            &f.member,
            &agent.id,
            &removal,
            false
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_guest_refusal_and_legacy_personal_compatibility() {
    let f = Fixture::new("org_agent_legacy").await;
    let (org_agent, row) = f.create().await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"guest_turn": true}})
        .await
        .unwrap();
    assert!(
        acks::for_key(&f.state.db, &f.admin, Some(&row.credential_api_key_id))
            .await
            .is_err()
    );
    let (personal, thread) = team::create_specialist_for(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &f.admin,
        f.request("personal"),
    )
    .await
    .unwrap()
    .unwrap();
    let mut legacy = bson::to_document(&personal).unwrap();
    for field in [
        "skills",
        "skills_revision",
        "skill_metadata",
        "operation_scopes",
        "operation_scope_revisions",
    ] {
        legacy.remove(field);
    }
    let restored: AssistantAgent = bson::from_document(legacy).unwrap();
    assert!(restored.skills.is_empty());
    assert!(restored.operation_scopes.is_empty());
    assert!(
        team::agent(&f.state.db, &f.member, &personal.id)
            .await
            .is_err()
    );
    f.revoke(&f.admin).await;
    let key = f.key(&thread).await;
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_ok()
    );
    assert!(
        team::agent(&f.state.db, &f.admin, &org_agent.id)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_machine_gateway_checks_member_and_node_owner_live() {
    use crate::models::node::{Node, NodeStatus};
    use std::sync::atomic::Ordering;
    let (f, memberships, _) = Fixture::observed("org_agent_machine").await;
    let (agent, _) = f.create().await;
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let (_, token, _) = super::node_service::create_registration_token(
        &f.state.db,
        &f.org,
        "org-machine",
        100,
        300,
    )
    .await
    .unwrap();
    let (mut node, _, _) =
        super::node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .unwrap();
    node.status = NodeStatus::Online;
    node.machine = Some(nyxid_machine::MachineProfile {
        version: 1,
        runtime_id: Uuid::new_v4().to_string(),
        shell: true,
        ..Default::default()
    });
    f.state
        .db
        .collection::<Node>(crate::models::node::COLLECTION_NAME)
        .replace_one(doc! {"_id": &node.id}, &node)
        .await
        .unwrap();
    team::set_grants(
        &f.state.db,
        &f.member,
        &agent.id,
        team::GrantChange::Machine {
            base: Box::new(team::GrantChange::Add(Default::default())),
            machines: Some(vec![node.id.clone()]),
            logins: None,
            mode: team::MachineGrantMode::Add,
        },
    )
    .await
    .unwrap();
    let chat = acks::for_key(&f.state.db, &f.member, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        super::machine_service::visible_nodes(&f.state.db, &chat)
            .await
            .unwrap()
            .len(),
        1
    );
    let job = super::machine_service::issue_job(&f.state.db, &chat, &node, 120, vec![])
        .await
        .unwrap();
    memberships.store(0, Ordering::SeqCst);
    let auth = crate::handlers::machine_gateway::job_auth(&f.state, &job)
        .await
        .unwrap();
    assert_eq!(
        memberships.load(Ordering::SeqCst),
        1,
        "machine gateway authentication shares its access resolution"
    );
    assert_eq!(auth.user_id.to_string(), f.member);
    assert_eq!(
        auth.assistant_agent_owner_id.as_deref(),
        Some(f.org.as_str())
    );
    f.state
        .db
        .collection::<Node>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &node.id},
            doc! {"$set": {"user_id": &f.member}},
        )
        .await
        .unwrap();
    assert!(
        crate::handlers::machine_gateway::job_auth(&f.state, &job)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Node>(crate::models::node::COLLECTION_NAME)
        .update_one(doc! {"_id": &node.id}, doc! {"$set": {"user_id": &f.org}})
        .await
        .unwrap();
    f.revoke(&f.member).await;
    assert!(
        crate::handlers::machine_gateway::job_auth(&f.state, &job)
            .await
            .is_err()
    );
    assert!(
        super::machine_service::visible_nodes(&f.state.db, &chat)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_nyxbot_widening_card_belongs_to_requesting_member() {
    use serde_json::json;
    let f = Fixture::new("org_agent_cards").await;
    let (agent, _) = f.create().await;
    let service = f.service(&f.org, "allowed").await;
    team::set_grants(
        &f.state.db,
        &f.member,
        &agent.id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    set_agent_operation_scopes_enabled(&f.state.db, &f.admin, true).await;
    super::agent_operation_scope_service::set(
        &f.state.db,
        &f.member,
        &agent.id,
        &service,
        &crate::models::agent_operation_scope::OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            rules: vec![],
        },
        false,
    )
    .await
    .unwrap();
    super::assistant_settings_service::update(
        &f.state.db,
        &f.member,
        super::assistant_settings_service::Update {
            skip_destructive_confirmation: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let nyxbot = team::ensure_nyxbot(&f.state.db, &f.member).await.unwrap();
    let home = team::home_thread(&f.state.db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let chat = acks::for_key(&f.state.db, &f.member, Some(&home.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let mut args = json!({"org": "org-agent-team", "subagent": agent.id, "service_id": service, "selection": {"expected_revision": 1, "all_operations": true}});
    let (card, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__set_agent_operations",
        &args,
    ))
    .await;
    assert!(error);
    assert_eq!(card["decider"], "user");
    let id = card["acknowledgement_id"].as_str().unwrap();
    assert!(
        acks::decide(&f.state.db, &f.admin, &home.id, id, true)
            .await
            .is_err()
    );
    acks::decide(&f.state.db, &f.member, &home.id, id, true)
        .await
        .unwrap();
    args["acknowledgement_id"] = json!(id);
    let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__set_agent_operations",
        &args,
    ))
    .await;
    assert!(!error);
    let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__set_agent_operations",
        &args,
    ))
    .await;
    assert!(error);
    f.revoke(&f.member).await;
    let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__list_subagents",
        &json!({"org": f.org}),
    ))
    .await;
    assert!(error);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_skills_card_pin_read_and_live_access() {
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::io::{Cursor, Write};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };
    let f = Fixture::new("org_agent_skill_native").await;
    let (agent, row) = f.create().await;
    let upstream = MockServer::start().await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("SKILL.md", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"Pinned team guidance").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let id = Uuid::new_v4().to_string();
    let pin = crate::models::catalog_skill_revision::SkillReference {
        source: "ornn".into(),
        skill_id: id.clone(),
        name: "team-guide".into(),
        version: "1.0".into(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        dependencies: vec![],
    };
    Mock::given(method("GET")).and(path(format!("/api/v1/skills/{id}"))).and(query_param("version","1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":id,"name":pin.name,"version":"1.0","description":"Team guidance","skillHash":pin.sha256}}))).mount(&upstream).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&upstream)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions/1.0/download")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&upstream)
        .await;
    let nyxbot = team::ensure_nyxbot(&f.state.db, &f.member).await.unwrap();
    let home = team::home_thread(&f.state.db, &f.state.encryption_keys, &nyxbot)
        .await
        .unwrap();
    let managing = acks::for_key(&f.state.db, &f.member, Some(&home.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    super::assistant_settings_service::update(
        &f.state.db,
        &f.member,
        super::assistant_settings_service::Update {
            skip_destructive_confirmation: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut args = json!({"org": f.org, "agent": agent.id, "selection": {"expected_revision": 0, "skills": [pin]}});
    let (card, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &managing,
        "nyxid__set_agent_skills",
        &args,
    ))
    .await;
    assert!(error);
    assert_eq!(card["decider"], "user");
    let ack = card["acknowledgement_id"].as_str().unwrap();
    assert!(
        acks::decide(&f.state.db, &f.admin, &home.id, ack, true)
            .await
            .is_err()
    );
    acks::decide(&f.state.db, &f.member, &home.id, ack, true)
        .await
        .unwrap();
    args["acknowledgement_id"] = json!(ack);
    let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &managing,
        "nyxid__set_agent_skills",
        &args,
    ))
    .await;
    assert!(!error);
    let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &managing,
        "nyxid__set_agent_skills",
        &args,
    ))
    .await;
    assert!(error);
    let chat = acks::for_key(&f.state.db, &f.admin, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    for (tool, arguments) in [
        ("nyxid__get_agent_skills", json!({"agent": agent.name})),
        (
            "nyxid__get_agent_operations",
            json!({"subagent": agent.name}),
        ),
    ] {
        let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
            &f.state, &chat, tool, &arguments,
        ))
        .await;
        assert!(!error, "a specialist can read its own settings by name");
    }
    let (content, error) = Box::pin(crate::handlers::agent_skills::dispatch(
        &f.state,
        &chat,
        "skill_read",
        &json!({"skill": id}),
    ))
    .await
    .unwrap();
    assert!(!error);
    assert_eq!(content["content"], "Pinned team guidance");
    assert!(content["untrusted_guidance"].as_bool().unwrap());
    assert!(
        Box::pin(crate::handlers::agent_skills::dispatch(
            &f.state,
            &managing,
            "skill_read",
            &json!({"skill": id})
        ))
        .await
        .is_err()
    );
    let (_, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__set_agent_skills",
        &args,
    ))
    .await;
    assert!(error);
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &f.org, "member_user_id": &f.admin},
            doc! {"$set": {"role": "viewer"}},
        )
        .await
        .unwrap();
    assert!(
        Box::pin(crate::handlers::agent_skills::dispatch(
            &f.state,
            &chat,
            "skill_read",
            &json!({"skill": id})
        ))
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_rotation_preserves_binding_and_live_revocation() {
    let f = Fixture::new("org_agent_rotation").await;
    let (agent, _) = f.create().await;
    let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let rotated = super::key_service::rotate_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &row.credential_api_key_id,
    )
    .await
    .unwrap();
    let key = f
        .state
        .db
        .collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": &rotated.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        key.assistant_agent_owner_id.as_deref(),
        Some(f.org.as_str())
    );
    assert_eq!(key.user_id, f.member);
    assert!(
        acks::for_key(&f.state.db, &f.member, Some(&key.id))
            .await
            .unwrap()
            .is_some()
    );
    f.revoke(&f.member).await;
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}
